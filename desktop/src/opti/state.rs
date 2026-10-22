//! Undo store. Same JSON shape as Optimaxer's `state.json` (original registry values, service start types, task
//! states and any data a special tweak returned), kept in the shared data directory. The first load imports an
//! existing `%ProgramData%\Optimaxer\state.json` once, so tweaks applied by the PowerShell edition stay undoable.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RegRec {
    #[serde(rename = "P")]
    pub p: String,
    #[serde(rename = "N")]
    pub n: String,
    #[serde(rename = "Exists")]
    pub exists: bool,
    #[serde(rename = "Value")]
    pub value: Value,
    #[serde(rename = "Type")]
    pub ty: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SvcRec {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Start")]
    pub start: i64,
    #[serde(rename = "Delayed")]
    pub delayed: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TaskRec {
    #[serde(rename = "Full")]
    pub full: String,
    #[serde(rename = "Enabled")]
    pub enabled: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Record {
    #[serde(rename = "Reg")]
    pub reg: Vec<RegRec>,
    #[serde(rename = "Svc")]
    pub svc: Vec<SvcRec>,
    #[serde(rename = "Task")]
    pub task: Vec<TaskRec>,
    #[serde(rename = "Data")]
    pub data: Value,
    #[serde(rename = "Time")]
    pub time: String,
}

#[derive(Default)]
pub struct State {
    pub records: BTreeMap<String, Record>,
    path: PathBuf,
}

pub fn state_dir() -> PathBuf {
    let d = crate::net::data_dir().join("optimaxer");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn backup_dir() -> PathBuf {
    let d = state_dir().join("backups");
    let _ = fs::create_dir_all(&d);
    d
}

fn legacy_path() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join("Optimaxer").join("state.json"))
}

fn read_records(path: &Path) -> Result<BTreeMap<String, Record>, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    // PowerShell writes a UTF-8 BOM
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes)).into_owned();
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

impl State {
    pub fn load() -> Self {
        Self::load_from(state_dir().join("state.json"), legacy_path())
    }

    pub fn load_from(path: PathBuf, legacy: Option<PathBuf>) -> Self {
        let mut records = BTreeMap::new();
        if path.exists() {
            match read_records(&path) {
                Ok(r) => records = r,
                Err(why) => {
                    // keep the unreadable file: the next save would otherwise overwrite every saved original value
                    let keep = path.with_file_name(format!("state-unreadable-{}.json", super::now_stamp()));
                    let _ = fs::copy(&path, keep);
                    eprintln!("optimaxer state unreadable ({why}); copy kept");
                }
            }
        } else if let Some(l) = legacy.filter(|l| l.exists()) {
            if let Ok(r) = read_records(&l) {
                records = r;
                let s = State { records: records.clone(), path: path.clone() };
                let _ = s.save();
            }
        }
        State { records, path }
    }

    pub fn save(&self) -> Result<(), String> {
        let tmp = self.path.with_extension("tmp");
        let json = serde_json::to_vec_pretty(&self.records).map_err(|e| e.to_string())?;
        fs::write(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    pub fn contains(&self, id: &str) -> bool {
        self.records.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opti-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_the_powershell_format_including_bom() {
        let d = temp("ps");
        let ps = d.join("legacy.json");
        let json = r#"{"perf-animations":{"Reg":[{"P":"HKCU:\\Control Panel\\Desktop","N":"MenuShowDelay","Exists":true,"Value":"400","Type":"String"}],"Svc":[],"Task":[],"Data":null,"Time":"2026-10-12T10:00:00"}}"#;
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(json.as_bytes());
        fs::write(&ps, bytes).unwrap();
        let s = State::load_from(d.join("state.json"), Some(ps.clone()));
        let rec = &s.records["perf-animations"];
        assert_eq!(rec.reg[0].n, "MenuShowDelay");
        assert_eq!(rec.reg[0].value, Value::String("400".into()));
        assert!(d.join("state.json").exists(), "imported once into the shared data directory");
        // legacy file is never touched
        assert_eq!(fs::read(&ps).unwrap().len(), json.len() + 3);
    }

    #[test]
    fn import_happens_only_once() {
        let d = temp("once");
        let ps = d.join("legacy.json");
        fs::write(&ps, r#"{"a":{"Reg":[],"Svc":[],"Task":[],"Data":null,"Time":"t"}}"#).unwrap();
        let mut s = State::load_from(d.join("state.json"), Some(ps.clone()));
        s.records.remove("a");
        s.save().unwrap();
        let again = State::load_from(d.join("state.json"), Some(ps));
        assert!(!again.contains("a"), "an undone tweak must not come back from the legacy file");
    }

    #[test]
    fn unreadable_state_is_kept_not_overwritten() {
        let d = temp("bad");
        fs::write(d.join("state.json"), "{ not json").unwrap();
        let s = State::load_from(d.join("state.json"), None);
        assert!(s.records.is_empty());
        let kept = fs::read_dir(&d).unwrap().filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().starts_with("state-unreadable-"));
        assert!(kept);
    }

    #[test]
    fn save_round_trips() {
        let d = temp("rt");
        let mut s = State::load_from(d.join("state.json"), None);
        s.records.insert("x".into(), Record { data: serde_json::json!({"Prev": 2}), ..Default::default() });
        s.save().unwrap();
        let again = State::load_from(d.join("state.json"), None);
        assert_eq!(again.records["x"].data["Prev"], 2);
    }
}
