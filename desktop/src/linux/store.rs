//! Saved original values, kept in the user's data folder (never in a root-owned place) so that undo works even if
//! the privileged helper is later unavailable. Every change writes its originals here before the system is touched.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnap {
    pub path: String,
    pub existed: bool,
    pub content: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditSnap {
    pub path: String,
    pub key: String,
    pub value: Option<String>,
    pub section: Option<String>,
    pub sep: String,
    /// The line that was replaced; `None` when the setting was appended.
    pub orig: Option<String>,
    /// False when the file already had the wanted line, so undo must leave it alone.
    pub changed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitSnap {
    pub unit: String,
    pub enabled: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TweakRecord {
    pub time: String,
    pub files: Vec<FileSnap>,
    pub edits: Vec<EditSnap>,
    pub units: Vec<UnitSnap>,
    /// Running kernel values (`key`, `value`) read before a sysctl file was written, so undo can put them back now
    /// instead of waiting for a reboot.
    #[serde(default)]
    pub sysctl: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsSnap {
    pub name: String,
    pub ipv4_dns: String,
    pub ipv4_ignore_auto: String,
    pub ipv6_dns: String,
    pub ipv6_ignore_auto: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutostartSnap {
    pub file: String,
    pub existed: bool,
    pub content: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub tweaks: BTreeMap<String, TweakRecord>,
    #[serde(default)]
    pub services: BTreeMap<String, UnitSnap>,
    #[serde(default)]
    pub user_units: BTreeMap<String, UnitSnap>,
    #[serde(default)]
    pub dns: BTreeMap<String, DnsSnap>,
    #[serde(default)]
    pub autostart: BTreeMap<String, AutostartSnap>,
}

pub fn state_path() -> PathBuf {
    let dir = crate::net::data_dir().join("optimaxer-linux");
    let _ = fs::create_dir_all(&dir);
    dir.join("state.json")
}

impl State {
    /// A missing file is an empty state. An unreadable one is an error and is copied aside first, because saving over
    /// it would destroy every saved original value.
    pub fn load_from(path: &Path) -> Result<State, String> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
            Err(e) => return Err(format!("Could not read {}: {e}", path.display())),
        };
        serde_json::from_slice(&bytes).map_err(|e| {
            let keep = path.with_file_name(format!(
                "state-unreadable-{}.json",
                crate::opti::now_stamp()
            ));
            let _ = fs::copy(path, &keep);
            format!(
                "Saved original values could not be read ({e}). The file was kept as {}.",
                keep.display()
            )
        })
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::net::write_atomic(path, &bytes)
    }

    pub fn load() -> Result<State, String> {
        Self::load_from(&state_path())
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&state_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "optimaxer-linux-store-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn state_round_trips_and_missing_file_is_empty() {
        let dir = temp("rt");
        let path = dir.join("state.json");
        assert_eq!(State::load_from(&path).unwrap(), State::default());
        let mut state = State::default();
        state.tweaks.insert(
            "perf-vm".into(),
            TweakRecord {
                time: "2026-10-31T00:00:00".into(),
                files: vec![FileSnap {
                    path: "/etc/x".into(),
                    existed: false,
                    content: None,
                }],
                edits: vec![],
                units: vec![UnitSnap {
                    unit: "a.timer".into(),
                    enabled: true,
                    active: false,
                }],
                sysctl: vec![("vm.swappiness".into(), "60".into())],
            },
        );
        state.save_to(&path).unwrap();
        assert_eq!(State::load_from(&path).unwrap(), state);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_corrupt_file_is_kept_and_reported() {
        let dir = temp("bad");
        let path = dir.join("state.json");
        fs::write(&path, "{ not json").unwrap();
        let error = State::load_from(&path).unwrap_err();
        assert!(error.contains("state-unreadable-"));
        assert!(path.exists(), "the original stays in place");
        let kept = fs::read_dir(&dir).unwrap().filter_map(Result::ok).any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("state-unreadable-")
        });
        assert!(kept);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn older_files_without_every_section_still_load() {
        let dir = temp("old");
        let path = dir.join("state.json");
        fs::write(&path, r#"{"tweaks":{}}"#).unwrap();
        assert_eq!(State::load_from(&path).unwrap(), State::default());
        fs::remove_dir_all(dir).unwrap();
    }
}
