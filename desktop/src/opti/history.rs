//! Bounded history of verified tweak apply and undo batches.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 100;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

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

fn path() -> PathBuf {
    crate::opti::state::state_dir().join("history.json")
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
}
