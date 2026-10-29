//! Bounded history of verified tweak apply and undo batches.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 100;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OPERATIONS: usize = 100;
const MAX_OPERATION_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_OPERATION_DETAILS: usize = 16;
const MAX_OPERATION_LINE_CHARS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Apply,
    Undo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub id: String,
    pub name: String,
    pub verified: bool,
    pub undo_available: bool,
    /// Exact apply preview lines. Undo entries describe restoration without guessing old values.
    #[serde(default)]
    pub details: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// UTC timestamp in ISO 8601 form.
    pub time: String,
    pub action: Action,
    pub changes: Vec<Change>,
}

/// Output captured from a confirmed, non-tweak system action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationEntry {
    /// UTC timestamp in ISO 8601 form.
    pub time: String,
    pub title: String,
    pub details: Vec<String>,
}

fn path() -> PathBuf {
    crate::opti::state::state_dir().join("history.json")
}

fn operation_path() -> PathBuf {
    crate::opti::state::state_dir().join("operations.json")
}

pub fn load() -> Result<Vec<Entry>, String> {
    load_from(&path())
}

fn load_from(path: &Path) -> Result<Vec<Entry>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let size = fs::metadata(path)
        .map_err(|e| format!("Could not inspect tweak history: {e}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "Tweak history exceeds the {MAX_FILE_BYTES}-byte safety limit"
        ));
    }
    let bytes = fs::read(path).map_err(|e| format!("Could not read tweak history: {e}"))?;
    let entries: Vec<Entry> =
        serde_json::from_slice(&bytes).map_err(|e| format!("Tweak history is unreadable: {e}"))?;
    Ok(entries
        .into_iter()
        .rev()
        .take(MAX_ENTRIES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect())
}

pub fn append(entry: Entry) -> Result<Vec<Entry>, String> {
    append_to(&path(), entry)
}

pub fn load_operations() -> Result<Vec<OperationEntry>, String> {
    load_operations_from(&operation_path())
}

pub fn append_operation(entry: OperationEntry) -> Result<Vec<OperationEntry>, String> {
    append_operation_to(&operation_path(), entry)
}

fn load_operations_from(path: &Path) -> Result<Vec<OperationEntry>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let size = fs::metadata(path)
        .map_err(|error| format!("Could not inspect operation history: {error}"))?
        .len();
    if size > MAX_OPERATION_FILE_BYTES {
        return Err(format!(
            "Operation history exceeds the {MAX_OPERATION_FILE_BYTES}-byte safety limit"
        ));
    }
    let bytes = fs::read(path).map_err(|error| format!("Could not read operation history: {error}"))?;
    let entries: Vec<OperationEntry> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Operation history is unreadable: {error}"))?;
    Ok(entries
        .into_iter()
        .rev()
        .take(MAX_OPERATIONS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect())
}

fn append_operation_to(
    path: &Path,
    mut entry: OperationEntry,
) -> Result<Vec<OperationEntry>, String> {
    if entry.title.trim().is_empty() {
        return Err("Operation history title cannot be empty".into());
    }
    entry.title = entry.title.chars().take(256).collect();
    if entry.details.len() > MAX_OPERATION_DETAILS {
        entry.details.truncate(MAX_OPERATION_DETAILS - 1);
        entry.details.push("Additional output omitted from saved activity record.".into());
    }
    for line in &mut entry.details {
        if line.chars().count() > MAX_OPERATION_LINE_CHARS {
            *line = line
                .chars()
                .take(MAX_OPERATION_LINE_CHARS - 1)
                .chain(std::iter::once('…'))
                .collect();
        }
    }
    let mut entries = load_operations_from(path)?;
    entries.push(entry);
    if entries.len() > MAX_OPERATIONS {
        entries.drain(..entries.len() - MAX_OPERATIONS);
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Operation history path has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create operation history directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(&entries)
        .map_err(|error| format!("Could not encode operation history: {error}"))?;
    if bytes.len() as u64 > MAX_OPERATION_FILE_BYTES {
        return Err("Operation history would exceed its 8 MiB safety limit".into());
    }
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)
        .map_err(|error| format!("Could not stage operation history: {error}"))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not write operation history: {error}"));
    }
    drop(file);
    if let Err(error) = replace_file(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not save operation history: {error}"));
    }
    Ok(entries)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
        | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
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

fn append_to(path: &Path, entry: Entry) -> Result<Vec<Entry>, String> {
    let mut entries = load_from(path)?;
    entries.push(entry);
    if entries.len() > MAX_ENTRIES {
        entries.drain(..entries.len() - MAX_ENTRIES);
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Tweak history path has no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create tweak history directory: {e}"))?;
    let tmp = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(&entries)
        .map_err(|e| format!("Could not encode tweak history: {e}"))?;
    let mut file =
        fs::File::create(&tmp).map_err(|e| format!("Could not stage tweak history: {e}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("Could not write tweak history: {e}"))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| format!("Could not save tweak history: {e}"))?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("opti-history-{name}-{}", std::process::id()))
    }

    fn entry(index: usize) -> Entry {
        Entry {
            time: format!("2026-10-29T00:00:{index:02}Z"),
            action: Action::Apply,
            changes: vec![Change {
                id: format!("tweak-{index}"),
                name: format!("Tweak {index}"),
                verified: true,
                undo_available: true,
                details: vec!["Registry: 0 -> 1".into()],
            }],
        }
    }

    fn operation(index: usize) -> OperationEntry {
        OperationEntry {
            time: format!("2026-10-29T00:00:{index:02}Z"),
            title: "Changing services".into(),
            details: vec![format!("Service {index}: Automatic -> Manual")],
        }
    }

    #[test]
    fn missing_history_starts_empty_and_entries_survive_round_trip() {
        let dir = temp("round-trip");
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("history.json");
        assert!(load_from(&file).unwrap().is_empty());
        append_to(&file, entry(1)).unwrap();
        assert_eq!(load_from(&file).unwrap(), vec![entry(1)]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn history_retains_only_the_newest_bounded_entries() {
        let dir = temp("bounded");
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("history.json");
        for index in 0..(MAX_ENTRIES + 3) {
            append_to(&file, entry(index)).unwrap();
        }
        let loaded = load_from(&file).unwrap();
        assert_eq!(loaded.len(), MAX_ENTRIES);
        assert_eq!(loaded.first().unwrap().time, entry(3).time);
        assert_eq!(loaded.last().unwrap().time, entry(MAX_ENTRIES + 2).time);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_history_is_preserved_and_never_overwritten() {
        let dir = temp("corrupt");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history.json");
        fs::write(&file, b"not json").unwrap();
        assert!(
            append_to(&file, entry(1))
                .unwrap_err()
                .contains("unreadable")
        );
        assert_eq!(fs::read(&file).unwrap(), b"not json");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn oversized_history_is_refused_without_reading_or_overwriting_it() {
        let dir = temp("oversized");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("history.json");
        fs::write(&file, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(load_from(&file).unwrap_err().contains("safety limit"));
        assert!(
            append_to(&file, entry(1))
                .unwrap_err()
                .contains("safety limit")
        );
        assert_eq!(fs::metadata(&file).unwrap().len(), MAX_FILE_BYTES + 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn operation_history_appends_and_replaces_existing_file() {
        let dir = temp("operations-round-trip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("operations.json");
        assert!(load_operations_from(&file).unwrap().is_empty());
        append_operation_to(&file, operation(1)).unwrap();
        append_operation_to(&file, operation(2)).unwrap();
        assert_eq!(load_operations_from(&file).unwrap(), vec![operation(1), operation(2)]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn operation_history_is_bounded_and_caps_saved_details() {
        let dir = temp("operations-bounded");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("operations.json");
        for index in 0..(MAX_OPERATIONS + 2) {
            append_operation_to(&file, operation(index)).unwrap();
        }
        let mut long = operation(1000);
        long.details = vec!["x".repeat(MAX_OPERATION_LINE_CHARS + 20)];
        let loaded = append_operation_to(&file, long).unwrap();
        assert_eq!(loaded.len(), MAX_OPERATIONS);
        assert_eq!(loaded.first().unwrap().time, operation(3).time);
        assert_eq!(
            loaded.last().unwrap().details[0].chars().count(),
            MAX_OPERATION_LINE_CHARS
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_operation_history_is_never_overwritten() {
        let dir = temp("operations-corrupt");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("operations.json");
        fs::write(&file, b"not json").unwrap();
        assert!(append_operation_to(&file, operation(1)).unwrap_err().contains("unreadable"));
        assert_eq!(fs::read(&file).unwrap(), b"not json");
        fs::remove_dir_all(dir).unwrap();
    }
}
