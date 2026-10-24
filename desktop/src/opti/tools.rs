//! Tools page: restore point, free RAM, TRIM, system-file checks, Windows Update reset, icon and Store cache
//! resets, performance counters and a registry backup. Each tool reports what it did in plain lines.

use super::backend::{RegData, System};
use std::path::Path;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::ProcessStatus::{EmptyWorkingSet, EnumProcesses};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    RestorePoint,
    FreeRam,
    Trim,
    Sfc,
    Dism,
    ResetUpdate,
    IconCache,
    StoreCache,
    PerfCounters,
    RegBackup,
}

pub struct ToolInfo {
    pub tool: Tool,
    pub title: &'static str,
    pub desc: &'static str,
    /// Changes the system (as opposed to only reading it), so the UI asks before running it.
    pub changes_system: bool,
}

pub const TOOLS: [ToolInfo; 10] = [
    ToolInfo { tool: Tool::RestorePoint, title: "Create restore point", desc: "Snapshot system settings so you can roll back from Windows System Restore.", changes_system: true },
    ToolInfo { tool: Tool::FreeRam, title: "Free RAM now", desc: "Clears every RAM cache Windows allows: process and system working sets, file cache, modified and standby lists, identical pages and the registry cache.", changes_system: true },
    ToolInfo { tool: Tool::Trim, title: "Trim SSDs", desc: "Runs TRIM on every SSD. Takes seconds.", changes_system: true },
    ToolInfo { tool: Tool::Sfc, title: "Scan system files (SFC)", desc: "Repairs corrupted Windows system files. Takes several minutes.", changes_system: true },
    ToolInfo { tool: Tool::Dism, title: "Repair Windows image (DISM)", desc: "RestoreHealth: fixes the component store. Needs internet. Run before SFC if SFC fails.", changes_system: true },
    ToolInfo { tool: Tool::ResetUpdate, title: "Reset Windows Update", desc: "Clears the update cache and re-registers update services. Fixes stuck updates.", changes_system: true },
    ToolInfo { tool: Tool::IconCache, title: "Rebuild icon cache", desc: "Fixes blank or wrong icons and thumbnails. Restarts Explorer.", changes_system: true },
    ToolInfo { tool: Tool::StoreCache, title: "Reset Microsoft Store cache", desc: "Runs wsreset to fix Store download problems.", changes_system: true },
    ToolInfo { tool: Tool::PerfCounters, title: "Rebuild performance counters", desc: "Runs lodctr /r and winmgmt /resyncperf. Fixes empty Task Manager graphs and slow WMI.", changes_system: true },
    ToolInfo { tool: Tool::RegBackup, title: "Export registry backup", desc: "Saves .reg exports of the policy, Explorer, memory and network keys that Optimaxer edits.", changes_system: false },
];

const RESTORE_KEY: &str = r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore";

fn tail(out: &str, n: usize, skip: impl Fn(&str) -> bool) -> Vec<String> {
    let lines: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty() && !skip(l)).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

/// System Restore point via Checkpoint-Computer (one PowerShell call; there is no console tool for it). The 24-hour
/// limit is lifted for the call and put back afterwards.
pub fn create_restore_point(sys: &dyn System, description: &str, log: &mut dyn FnMut(&str)) -> bool {
    let old = sys.reg_get(RESTORE_KEY, "SystemRestorePointCreationFrequency");
    let _ = sys.reg_set(RESTORE_KEY, "SystemRestorePointCreationFrequency", &RegData::Dword(0));
    let script = format!("Enable-ComputerRestore -Drive \"$env:SystemDrive\\\" -ErrorAction SilentlyContinue; Checkpoint-Computer -Description '{}' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop", description.replace('\'', ""));
    let out = sys.run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", &script]);
    match old {
        Some(v) => {
            let _ = sys.reg_set(RESTORE_KEY, "SystemRestorePointCreationFrequency", &v);
        }
        None => {
            let _ = sys.reg_delete_value(RESTORE_KEY, "SystemRestorePointCreationFrequency");
        }
    }
    if out.code == 0 {
        log(&format!("Restore point created: {description}"));
        true
    } else {
        log(&format!("WARN Restore point failed: {}", out.text.trim().lines().next().unwrap_or("unknown error")));
        false
    }
}

fn available_ram() -> u64 {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: dwLength is set and `m` is a valid out-pointer.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 { 0 } else { m.ullAvailPhys }
}

/// Trims the working set of every process this user may touch. Returns how many were trimmed.
fn trim_working_sets() -> u32 {
    let mut pids = vec![0u32; 4096];
    let mut bytes = 0u32;
    // SAFETY: buffer size in bytes is passed; `bytes` receives the amount written.
    if unsafe { EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes) } == 0 {
        return 0;
    }
    let me = std::process::id();
    let mut n = 0;
    for pid in pids.iter().take(bytes as usize / 4).copied().filter(|p| *p > 4 && *p != me) {
        // SAFETY: handle is closed on every path.
        unsafe {
            let h = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                continue;
            }
            if EmptyWorkingSet(h) != 0 {
                n += 1;
            }
            CloseHandle(h);
        }
    }
    n
}

// region memory caches
// ntdll and the privilege calls are declared here: windows-sys has no binding for NtSetSystemInformation.
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetSystemInformation(class: i32, info: *mut core::ffi::c_void, len: u32) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetSystemFileCacheSize(min: isize, max: isize, flags: u32) -> i32;
    fn GetCurrentProcess() -> *mut core::ffi::c_void;
    fn GetLastError() -> u32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: *mut core::ffi::c_void, access: u32, token: *mut *mut core::ffi::c_void) -> i32;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut i64) -> i32;
    fn AdjustTokenPrivileges(token: *mut core::ffi::c_void, disable_all: i32, new: *const TokenPrivilege, len: u32, prev: *mut core::ffi::c_void, ret: *mut u32) -> i32;
}

#[repr(C, packed(4))]
struct TokenPrivilege {
    count: u32,
    luid: i64,
    attributes: u32,
}

const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;
const SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION: i32 = 130;
const SYSTEM_REGISTRY_RECONCILIATION_INFORMATION: i32 = 155;

fn enable_privilege(name: &str) -> bool {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: all out-pointers are valid locals; the token handle is closed before returning.
    unsafe {
        let mut tok = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), 0x28, &mut tok) == 0 {
            return false;
        }
        let mut luid = 0i64;
        let ok = LookupPrivilegeValueW(std::ptr::null(), wide.as_ptr(), &mut luid) != 0 && {
            let tp = TokenPrivilege { count: 1, luid, attributes: 2 };
            AdjustTokenPrivileges(tok, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0 && GetLastError() == 0
        };
        CloseHandle(tok);
        ok
    }
}

fn memory_list_command(command: i32) -> i32 {
    let mut c = command;
    // SAFETY: a 4-byte command for SystemMemoryListInformation.
    unsafe { NtSetSystemInformation(SYSTEM_MEMORY_LIST_INFORMATION, (&mut c as *mut i32).cast(), 4) }
}

/// Every RAM cache Windows lets a program empty, in RAMMap's order. One line per step. Windows refills the file
/// cache as programs read files again.
pub fn clear_all_caches() -> Vec<String> {
    let line = |step: &str, status: i32| if status == 0 { format!("ok {step}") } else { format!("failed {step} (0x{status:08X})") };
    let prof = enable_privilege("SeProfileSingleProcessPrivilege");
    let quota = enable_privilege("SeIncreaseQuotaPrivilege");
    let mut out = vec![format!("ok process working sets ({} processes)", trim_working_sets())];
    out.push(line("system working set", if prof { memory_list_command(2) } else { -1 }));
    // SAFETY: -1/-1 asks Windows to flush the system file cache.
    let fc = quota && unsafe { SetSystemFileCacheSize(-1, -1, 0) } != 0;
    out.push(if fc { "ok system file cache".into() } else { format!("failed system file cache ({})", unsafe { GetLastError() }) });
    out.push(line("modified page list", if prof { memory_list_command(3) } else { -1 }));
    out.push(line("standby list", if prof { memory_list_command(4) } else { -1 }));
    out.push(line("low-priority standby list", if prof { memory_list_command(5) } else { -1 }));
    // MEMORY_COMBINE_INFORMATION_EX: handle, pages combined, flags
    let mut combine = [0u64; 3];
    let st = if prof { unsafe { NtSetSystemInformation(SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION, combine.as_mut_ptr().cast(), 24) } } else { -1 };
    out.push(if st == 0 { format!("ok combined identical pages ({} pages)", combine[1]) } else { line("combine identical pages", st) });
    // SAFETY: this class takes no buffer.
    out.push(line("registry cache", unsafe { NtSetSystemInformation(SYSTEM_REGISTRY_RECONCILIATION_INFORMATION, std::ptr::null_mut(), 0) }));
    out
}
// endregion

const REG_BACKUP_KEYS: [&str; 7] = [
    r"HKLM\SOFTWARE\Policies",
    r"HKCU\SOFTWARE\Policies",
    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer",
    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
    r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management",
    r"HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl",
    r"HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters",
];

pub fn run(sys: &dyn System, tool: Tool, backups: &Path, log: &mut dyn FnMut(&str)) {
    match tool {
        Tool::RestorePoint => {
            create_restore_point(sys, "Optimaxer - manual", log);
        }
        Tool::FreeRam => {
            let before = available_ram();
            for l in clear_all_caches() {
                log(&format!("  {}{l}", if l.starts_with("ok ") { "" } else { "WARN " }));
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            let after = available_ram();
            log(&format!(
                "RAM caches cleared. Free RAM {} -> {} (+{}). Windows refills the file cache as you open files again.",
                crate::model::format_size(Some(before)),
                crate::model::format_size(Some(after)),
                crate::model::format_size(Some(after.saturating_sub(before)))
            ));
        }
        Tool::Trim => {
            let out = sys.run("defrag.exe", &["/C", "/L"]);
            for l in tail(&out.text, 6, |_| false) {
                log(&format!("  {l}"));
            }
            log("TRIM finished (hard disks are skipped by /L).");
        }
        Tool::Sfc => {
            let out = sys.run("sfc.exe", &["/scannow"]);
            for l in tail(&out.text, 6, |l| l.contains('%')) {
                log(&format!("  {l}"));
            }
        }
        Tool::Dism => {
            let out = sys.run("Dism.exe", &["/Online", "/Cleanup-Image", "/RestoreHealth"]);
            for l in tail(&out.text, 5, |l| l.starts_with('[')) {
                log(&format!("  {l}"));
            }
        }
        Tool::ResetUpdate => {
            let svcs = ["wuauserv", "bits", "cryptsvc", "msiserver"];
            for s in svcs {
                sys.run("net.exe", &["stop", s]);
            }
            let win = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
            for d in [format!(r"{win}\SoftwareDistribution"), format!(r"{win}\System32\catroot2")] {
                let bak = format!("{d}.bak");
                if Path::new(&bak).exists() {
                    let _ = std::fs::remove_dir_all(&bak);
                }
                if Path::new(&d).exists() {
                    let _ = std::fs::rename(&d, &bak);
                }
            }
            for s in svcs {
                sys.run("net.exe", &["start", s]);
            }
            log("Windows Update components reset. Check for updates again.");
        }
        Tool::IconCache => {
            sys.run("taskkill.exe", &["/F", "/IM", "explorer.exe"]);
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                let _ = std::fs::remove_file(format!(r"{local}\IconCache.db"));
                for pat in ["iconcache_*.db", "thumbcache_*.db"] {
                    for f in super::cleaner::expand(&format!(r"{local}\Microsoft\Windows\Explorer\{pat}")) {
                        let _ = std::fs::remove_file(f);
                    }
                }
            }
            sys.run("cmd.exe", &["/c", "start", "", "explorer.exe"]);
            log("Icon cache rebuilt.");
        }
        Tool::StoreCache => {
            sys.run("wsreset.exe", &[]);
            log("Store cache reset.");
        }
        Tool::PerfCounters => {
            sys.run("lodctr.exe", &["/r"]);
            sys.run("lodctr.exe", &["/r"]);
            sys.run("winmgmt.exe", &["/resyncperf"]);
            log("Performance counters rebuilt. Restart apps that read them (Task Manager, monitoring tools).");
        }
        Tool::RegBackup => {
            let dir = backups.join(format!("registry-{}", super::now_stamp()));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                log(&format!("WARN could not create {}: {e}", dir.display()));
                return;
            }
            for k in REG_BACKUP_KEYS {
                let file = dir.join(format!("{}.reg", k.replace(['\\', ' '], "_")));
                sys.run("reg.exe", &["export", k, &file.to_string_lossy(), "/y"]);
                log(&format!("  exported {k}"));
            }
            log(&format!("Registry backup saved to {}", dir.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    fn restore_point_lifts_and_restores_the_frequency_limit() {
        let sys = FakeSystem::default();
        sys.put(RESTORE_KEY, "SystemRestorePointCreationFrequency", RegData::Dword(1440));
        let mut lines = Vec::new();
        assert!(create_restore_point(&sys, "Optimaxer - test", &mut |l| lines.push(l.to_string())));
        assert!(sys.ran("Checkpoint-Computer"));
        assert_eq!(sys.reg_get(RESTORE_KEY, "SystemRestorePointCreationFrequency"), Some(RegData::Dword(1440)));
    }

    #[test]
    fn restore_point_removes_the_value_it_added() {
        let sys = FakeSystem::default();
        create_restore_point(&sys, "x", &mut |_| {});
        assert_eq!(sys.reg_get(RESTORE_KEY, "SystemRestorePointCreationFrequency"), None);
    }

    #[test]
    fn quotes_cannot_break_out_of_the_restore_point_command() {
        let sys = FakeSystem::default();
        create_restore_point(&sys, "a'; Remove-Item C:\\x; '", &mut |_| {});
        let cmd = sys.commands.borrow()[0].clone();
        assert!(!cmd.contains("a';"), "single quotes are stripped from the description: {cmd}");
    }

    #[test]
    fn registry_backup_exports_every_key() {
        let sys = FakeSystem::default();
        let dir = std::env::temp_dir().join(format!("opti-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        run(&sys, Tool::RegBackup, &dir, &mut |_| {});
        assert_eq!(sys.commands.borrow().iter().filter(|c| c.starts_with("reg.exe export")).count(), 7);
    }

    #[test]
    fn only_read_only_tools_skip_confirmation() {
        let ro: Vec<_> = TOOLS.iter().filter(|t| !t.changes_system).map(|t| t.tool).collect();
        assert_eq!(ro, vec![Tool::RegBackup]);
    }

    #[test]
    #[ignore = "empties this PC's memory caches; run as administrator"]
    fn live_clear_all_caches_runs_every_step() {
        let lines = clear_all_caches();
        println!("{}", lines.join("
"));
        assert_eq!(lines.len(), 8);
        assert!(lines.iter().all(|l| l.starts_with("ok ")), "{lines:?}");
    }

    #[test]
    fn tail_keeps_the_last_lines_and_filters_progress() {
        let out = "a\nVerification 40%\nb\nc\n";
        assert_eq!(tail(out, 2, |l| l.contains('%')), vec!["b", "c"]);
    }
}
