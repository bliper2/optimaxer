//! Bounded local inventory snapshots. These records describe observed configuration; they are not backups.

use super::{backend::System, sysmon};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_SNAPSHOTS: usize = 30;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub captured_at: String,
    pub system_drive: String,
    pub operating_system: String,
    pub processor: String,
    pub graphics: Vec<String>,
    pub logical_threads: u32,
    pub memory_used: Option<u64>,
    pub memory_total: Option<u64>,
    pub drive_used: Option<u64>,
    pub drive_total: Option<u64>,
    pub uptime_seconds: u64,
}

fn path() -> PathBuf {
    super::state::state_dir().join("snapshots.json")
}

pub fn load() -> Result<Vec<SystemSnapshot>, String> {
    load_from(&path())
}

pub fn capture(sys: &dyn System) -> SystemSnapshot {
    let info = sysmon::read_info(sys);
    let (memory_used, memory_total) =
        sysmon::read_memory().map_or((None, None), |(used, total)| (Some(used), Some(total)));
    let (drive_used, drive_total) =
        sysmon::read_disk().map_or((None, None), |(used, total)| (Some(used), Some(total)));
    SystemSnapshot {
        captured_at: format!("{}Z", super::now_iso()),
        system_drive: std::env::var("SystemDrive").unwrap_or_default(),
        operating_system: info.os,
        processor: info.cpu,
        graphics: info.gpus,
        logical_threads: info.threads,
        memory_used,
        memory_total,
        drive_used,
        drive_total,
        uptime_seconds: sysmon::uptime_secs(),
    }
}

pub fn append(snapshot: SystemSnapshot) -> Result<Vec<SystemSnapshot>, String> {
    append_to(&path(), snapshot)
}

fn load_from(path: &Path) -> Result<Vec<SystemSnapshot>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let size = fs::metadata(path)
        .map_err(|error| format!("Could not inspect system snapshots: {error}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "System snapshots exceed the {MAX_FILE_BYTES}-byte safety limit"
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read system snapshots: {error}"))?;
    let snapshots: Vec<SystemSnapshot> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("System snapshots are unreadable: {error}"))?;
    Ok(snapshots
        .into_iter()
        .rev()
        .take(MAX_SNAPSHOTS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect())
}

fn append_to(path: &Path, snapshot: SystemSnapshot) -> Result<Vec<SystemSnapshot>, String> {
    let mut snapshots = load_from(path)?;
    snapshots.push(snapshot);
    if snapshots.len() > MAX_SNAPSHOTS {
        snapshots.drain(..snapshots.len() - MAX_SNAPSHOTS);
    }
    let bytes = serde_json::to_vec_pretty(&snapshots)
        .map_err(|error| format!("Could not encode system snapshots: {error}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("System snapshots would exceed the 2 MiB safety limit".into());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "System snapshot path has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create system snapshot directory: {error}"))?;
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)
        .map_err(|error| format!("Could not stage system snapshots: {error}"))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not write system snapshots: {error}"));
    }
    drop(file);
    if let Err(error) = replace_file(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not save system snapshots: {error}"));
    }
    Ok(snapshots)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let flags = windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
        | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    // SAFETY: both paths are NUL-terminated UTF-16 strings and flags request atomic replacement.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            flags,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::{RegData, fake::FakeSystem};

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("optimaxer-snapshots-{name}-{}", std::process::id()))
    }

    fn snapshot(index: usize) -> SystemSnapshot {
        SystemSnapshot {
            captured_at: format!("2026-10-30T00:00:{index:02}Z"),
            ..SystemSnapshot::default()
        }
    }

    #[test]
    fn snapshot_append_round_trips_and_keeps_only_the_newest_entries() {
        let path = temp("bounded").join("snapshots.json");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        for index in 0..MAX_SNAPSHOTS + 3 {
            append_to(&path, snapshot(index)).unwrap();
        }
        let entries = load_from(&path).unwrap();
        assert_eq!(entries.len(), MAX_SNAPSHOTS);
        assert_eq!(entries.first().unwrap().captured_at, "2026-10-30T00:00:03Z");
        assert_eq!(entries.last().unwrap().captured_at, "2026-10-30T00:00:32Z");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_snapshot_store_is_preserved_and_blocks_append() {
        let path = temp("corrupt").join("snapshots.json");
        let _ = fs::remove_dir_all(path.parent().unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not valid json").unwrap();
        let error = append_to(&path, snapshot(1)).unwrap_err();
        assert!(error.contains("unreadable"));
        assert_eq!(fs::read(&path).unwrap(), b"not valid json");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn snapshot_serialization_keeps_unavailable_measurements_explicit() {
        let decoded: SystemSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&snapshot(0)).unwrap()).unwrap();
        assert_eq!(decoded, snapshot(0));
        assert_eq!(decoded.memory_total, None);
        assert_eq!(decoded.drive_total, None);
    }

    #[test]
    fn capture_combines_system_inventory_with_available_read_only_capacity() {
        let sys = FakeSystem::default();
        let current_version = r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        sys.put(
            current_version,
            "ProductName",
            RegData::Str("Windows 10 Pro".into()),
        );
        sys.put(
            current_version,
            "CurrentBuildNumber",
            RegData::Str("26100".into()),
        );
        sys.put(
            current_version,
            "DisplayVersion",
            RegData::Str("24H2".into()),
        );
        sys.put(
            r"HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "ProcessorNameString",
            RegData::Str("Test processor".into()),
        );
        let before = sys.values.borrow().clone();

        let captured = capture(&sys);
        assert!(captured.captured_at.ends_with('Z'));
        assert_eq!(
            captured.operating_system,
            "Windows 11 Pro 24H2 (build 26100)"
        );
        assert_eq!(captured.processor, "Test processor");
        assert_eq!(
            captured.memory_total,
            sysmon::read_memory().map(|(_, total)| total)
        );
        assert_eq!(
            captured.drive_total,
            sysmon::read_disk().map(|(_, total)| total)
        );
        assert!(captured.uptime_seconds > 0);
        assert_eq!(
            *sys.values.borrow(),
            before,
            "capture must not modify registry state"
        );
    }
}
