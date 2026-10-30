//! Read-only process data for the Performance page. Fields that Windows denies remain unavailable.

use std::collections::HashMap;
use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::prelude::OsStrExt;
use std::path::{Path, PathBuf};
use std::slice;
use std::time::Instant;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, GetLastError, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows_sys::Win32::System::ProcessStatus::{
    EnumProcesses, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    QueryFullProcessImageNameW,
};

const INITIAL_PROCESS_CAPACITY: usize = 1024;
const MAX_PROCESSES: usize = 16_384;
const MAX_VERSION_INFO_SIZE: u32 = 8 * 1024 * 1024;
const PROCESS_QUERY_ACCESS: u32 = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ;

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub image_path: Option<PathBuf>,
    /// Working set in bytes, when Windows permits the query.
    pub working_set: Option<u64>,
    /// Approximate percentage of total logical CPU capacity since the previous scan.
    pub cpu_percent: Option<f32>,
}

pub type CpuTimes = HashMap<u32, (u64, Instant)>;

#[repr(C)]
#[derive(Clone, Copy)]
struct Translation {
    language: u16,
    code_page: u16,
}

struct ProcessHandle(HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this handle came from OpenProcess and is owned by this wrapper.
        unsafe { CloseHandle(self.0) };
    }
}

/// Enumerate process IDs and collect accessible image path, working-set and CPU-time readings.
/// The caller runs this on a worker thread because opening every process can take measurable time.
pub fn scan(
    previous: &CpuTimes,
    logical_processors: u32,
) -> Result<(Vec<ProcessInfo>, CpuTimes), String> {
    let pids = process_ids()?;
    let sampled_at = Instant::now();
    let mut current = CpuTimes::with_capacity(pids.len());
    let mut processes = Vec::with_capacity(pids.len());
    for pid in pids {
        if let Some(info) =
            read_process(pid, previous, &mut current, sampled_at, logical_processors)
        {
            processes.push(info);
        }
    }
    processes.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.pid.cmp(&right.pid))
    });
    Ok((processes, current))
}

/// Read the executable's embedded CompanyName field. This is descriptive file metadata, not a
/// signature check, and is deliberately queried only when the user inspects a process.
pub fn company_name(path: &Path) -> Result<Option<String>, String> {
    let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `path_wide` is a valid NUL-terminated UTF-16 path for the duration of the call.
    let size = unsafe { GetFileVersionInfoSizeW(path_wide.as_ptr(), std::ptr::null_mut()) };
    if size == 0 {
        // Windows reports missing version resources through GetLastError. Some providers leave
        // the error code clear, which also means that no company field is available.
        let error = unsafe { GetLastError() };
        if error == 0 || matches!(error, 1812..=1815) {
            return Ok(None);
        }
        return Err(format!(
            "Windows could not read executable version metadata: {}",
            std::io::Error::from_raw_os_error(error as i32)
        ));
    }
    if size > MAX_VERSION_INFO_SIZE {
        return Err("Executable version metadata exceeds the 8 MiB read limit.".into());
    }

    let mut buffer = vec![0u8; size as usize];
    // SAFETY: buffer has the exact capacity reported by GetFileVersionInfoSizeW.
    if unsafe { GetFileVersionInfoW(path_wide.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) } == 0
    {
        let error = unsafe { GetLastError() };
        return Err(format!(
            "Windows could not read executable version metadata: {}",
            std::io::Error::from_raw_os_error(error as i32)
        ));
    }

    company_name_from_version_info(&buffer)
}

fn company_name_from_version_info(buffer: &[u8]) -> Result<Option<String>, String> {
    let translation_path: Vec<u16> = "\\VarFileInfo\\Translation"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut translations_ptr: *mut c_void = std::ptr::null_mut();
    let mut translations_len = 0u32;
    // SAFETY: the version block and output pointers remain valid for the call.
    let found = unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            translation_path.as_ptr(),
            &mut translations_ptr,
            &mut translations_len,
        )
    };
    if found == 0 || translations_ptr.is_null() {
        return Ok(None);
    }

    let translation_count = translations_len as usize / size_of::<Translation>();
    if translation_count == 0 || translation_count > 64 {
        return Ok(None);
    }
    // SAFETY: VerQueryValueW returns a view into `buffer`; length is in bytes and the result is
    // aligned according to the Windows version-resource format.
    let translations =
        unsafe { slice::from_raw_parts(translations_ptr.cast::<Translation>(), translation_count) };
    for translation in translations {
        let query = format!(
            "\\StringFileInfo\\{:04x}{:04x}\\CompanyName",
            translation.language, translation.code_page
        );
        let query_wide: Vec<u16> = query.encode_utf16().chain(Some(0)).collect();
        let mut value_ptr: *mut c_void = std::ptr::null_mut();
        let mut value_len = 0u32;
        // SAFETY: the query is NUL-terminated and the version block stays alive through the call.
        let found = unsafe {
            VerQueryValueW(
                buffer.as_ptr().cast(),
                query_wide.as_ptr(),
                &mut value_ptr,
                &mut value_len,
            )
        };
        if found == 0 || value_ptr.is_null() || value_len == 0 || value_len > 4096 {
            continue;
        }
        // SAFETY: VerQueryValueW supplies `value_len` UTF-16 code units within `buffer`.
        let value = unsafe { slice::from_raw_parts(value_ptr.cast::<u16>(), value_len as usize) };
        let company = String::from_utf16_lossy(value)
            .trim_matches('\0')
            .trim()
            .to_string();
        if !company.is_empty() {
            return Ok(Some(company));
        }
    }
    Ok(None)
}

fn process_ids() -> Result<Vec<u32>, String> {
    let mut ids = vec![0u32; INITIAL_PROCESS_CAPACITY];
    loop {
        let mut bytes = 0u32;
        // SAFETY: ids is writable for the byte count supplied, and bytes receives the written size.
        if unsafe {
            EnumProcesses(
                ids.as_mut_ptr(),
                (ids.len() * size_of::<u32>()) as u32,
                &mut bytes,
            )
        } == 0
        {
            return Err(format!(
                "Could not enumerate processes: {}",
                std::io::Error::last_os_error()
            ));
        }
        let count = bytes as usize / size_of::<u32>();
        if count < ids.len() {
            ids.truncate(count);
            return Ok(ids);
        }
        if ids.len() >= MAX_PROCESSES {
            return Err(format!(
                "The process list exceeds the supported limit of {MAX_PROCESSES}."
            ));
        }
        ids.resize((ids.len() * 2).min(MAX_PROCESSES), 0);
    }
}

fn read_process(
    pid: u32,
    previous: &CpuTimes,
    current: &mut CpuTimes,
    sampled_at: Instant,
    logical_processors: u32,
) -> Option<ProcessInfo> {
    if pid == 0 {
        return Some(ProcessInfo {
            pid,
            name: "System Idle Process".into(),
            image_path: None,
            working_set: None,
            cpu_percent: None,
        });
    }
    // SAFETY: OpenProcess takes scalar arguments and returns an owned handle or null.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_ACCESS, 0, pid) };
    if raw.is_null() {
        return Some(ProcessInfo {
            pid,
            name: format!("Process {pid}"),
            image_path: None,
            working_set: None,
            cpu_percent: None,
        });
    }
    let handle = ProcessHandle(raw);
    let image_path = image_path(handle.0);
    let name = image_path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("Process {pid}"));
    let working_set = working_set(handle.0);
    let cpu_percent = process_cpu(
        handle.0,
        pid,
        previous,
        current,
        sampled_at,
        logical_processors,
    );
    Some(ProcessInfo {
        pid,
        name,
        image_path,
        working_set,
        cpu_percent,
    })
}

fn image_path(handle: HANDLE) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    // SAFETY: the writable UTF-16 buffer and its size are valid for the duration of the call.
    if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return None;
    }
    buffer.truncate(length as usize);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

fn working_set(handle: HANDLE) -> Option<u64> {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..PROCESS_MEMORY_COUNTERS::default()
    };
    // SAFETY: counters is initialized with the required structure size and writable in full.
    (unsafe { GetProcessMemoryInfo(handle, &mut counters, counters.cb) } != 0)
        .then_some(counters.WorkingSetSize as u64)
}

fn process_cpu(
    handle: HANDLE,
    pid: u32,
    previous: &CpuTimes,
    current: &mut CpuTimes,
    sampled_at: Instant,
    logical_processors: u32,
) -> Option<f32> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: all four FILETIME values are valid writable out-pointers.
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return None;
    }
    let cpu_time = filetime(&kernel).saturating_add(filetime(&user));
    current.insert(pid, (cpu_time, sampled_at));
    let (old_cpu, old_at) = previous.get(&pid).copied()?;
    let elapsed = sampled_at.duration_since(old_at).as_secs_f64();
    process_cpu_percent(old_cpu, cpu_time, elapsed, logical_processors)
}

fn filetime(value: &FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

pub fn process_cpu_percent(
    previous: u64,
    current: u64,
    elapsed_seconds: f64,
    logical_processors: u32,
) -> Option<f32> {
    if elapsed_seconds <= 0.0 || !elapsed_seconds.is_finite() || logical_processors == 0 {
        return None;
    }
    let cpu_delta = current.checked_sub(previous)?;
    let capacity = elapsed_seconds * 10_000_000.0 * f64::from(logical_processors);
    Some((100.0 * cpu_delta as f64 / capacity).clamp(0.0, 100.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_cpu_uses_elapsed_time_and_total_logical_capacity() {
        assert_eq!(process_cpu_percent(0, 10_000_000, 1.0, 1), Some(100.0));
        assert_eq!(process_cpu_percent(0, 10_000_000, 1.0, 4), Some(25.0));
        assert_eq!(process_cpu_percent(10, 5, 1.0, 4), None);
        assert_eq!(process_cpu_percent(0, 10, 0.0, 4), None);
        assert_eq!(process_cpu_percent(0, 10, 1.0, 0), None);
    }

    #[test]
    fn cpu_percent_is_capped_to_total_machine_capacity() {
        assert_eq!(process_cpu_percent(0, 80_000_000, 1.0, 4), Some(100.0));
    }

    #[test]
    fn scan_reads_current_process_without_elevation() {
        let cores = std::thread::available_parallelism()
            .map(|count| count.get() as u32)
            .unwrap_or(1);
        let (first, first_times) = scan(&CpuTimes::new(), cores).unwrap();
        let current_pid = std::process::id();
        let current = first
            .iter()
            .find(|process| process.pid == current_pid)
            .unwrap();
        assert!(current.image_path.is_some());
        assert!(current.working_set.is_some());

        let (second, _) = scan(&first_times, cores).unwrap();
        assert!(
            second
                .iter()
                .find(|process| process.pid == current_pid)
                .unwrap()
                .cpu_percent
                .is_some()
        );
    }

    #[test]
    fn company_name_reads_embedded_windows_version_metadata() {
        let system_root = std::env::var_os("WINDIR").expect("Windows sets WINDIR");
        let image = PathBuf::from(system_root)
            .join("System32")
            .join("kernel32.dll");
        assert_eq!(
            company_name(&image).unwrap().as_deref(),
            Some("Microsoft Corporation")
        );
    }

    #[test]
    fn company_name_reports_a_missing_executable() {
        let image = std::env::temp_dir().join(format!(
            "optimaxer-missing-process-image-{}.exe",
            std::process::id()
        ));
        assert!(!image.exists());
        assert!(company_name(&image).is_err());
    }
}
