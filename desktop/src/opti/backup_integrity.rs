//! Read-only structural checks for saved Optimaxer undo and service backup files.

use crate::opti::state::{Record, SvcRec};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Good,
    Missing,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub title: &'static str,
    pub status: Status,
    pub detail: String,
}

pub fn inspect(state_file: &Path, backup_dir: &Path) -> Vec<Check> {
    vec![
        inspect_undo_state(state_file),
        inspect_service_backups(backup_dir),
    ]
}

fn inspect_undo_state(path: &Path) -> Check {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Check {
                title: "Tweak undo state",
                status: Status::Missing,
                detail: "No saved tweak originals yet.".into(),
            };
        }
        Err(error) => {
            return Check {
                title: "Tweak undo state",
                status: Status::Warning,
                detail: format!("Could not read state file: {error}"),
            };
        }
    };
    let bytes = without_utf8_bom(&bytes);
    let records: BTreeMap<String, Record> = match serde_json::from_slice(bytes) {
        Ok(records) => records,
        Err(error) => {
            return Check {
                title: "Tweak undo state",
                status: Status::Warning,
                detail: format!("Saved state is not readable JSON: {error}"),
            };
        }
    };

    let mut registry = 0;
    let mut services = 0;
    let mut tasks = 0;
    let mut issues = Vec::new();
    for (id, record) in &records {
        if id.trim().is_empty()
            || (record.reg.is_empty()
                && record.svc.is_empty()
                && record.task.is_empty()
                && record.data.is_null())
        {
            issues.push(format!("{id}: no usable original values"));
        }
        for original in &record.reg {
            registry += 1;
            if original.p.trim().is_empty() {
                issues.push(format!("{id}: registry original has no key path"));
            }
        }
        for original in &record.svc {
            services += 1;
            if original.name.trim().is_empty()
                || !(0..=4).contains(&original.start)
                || !(0..=1).contains(&original.delayed)
            {
                issues.push(format!("{id}: invalid service original"));
            }
        }
        for original in &record.task {
            tasks += 1;
            if original.full.trim().is_empty() || !(0..=1).contains(&original.enabled) {
                issues.push(format!("{id}: invalid scheduled-task original"));
            }
        }
    }

    if issues.is_empty() {
        let detail = if records.is_empty() {
            "Readable; no saved tweak records.".to_owned()
        } else {
            format!(
                "Readable; {} tweak record(s), {registry} registry, {services} service and {tasks} task original(s).",
                records.len()
            )
        };
        Check {
            title: "Tweak undo state",
            status: Status::Good,
            detail,
        }
    } else {
        Check {
            title: "Tweak undo state",
            status: Status::Warning,
            detail: format!(
                "{} record issue(s): {}",
                issues.len(),
                issues.into_iter().take(5).collect::<Vec<_>>().join("; ")
            ),
        }
    }
}

fn inspect_service_backups(dir: &Path) -> Check {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Check {
                title: "Service backups",
                status: Status::Missing,
                detail: "No service backups saved yet.".into(),
            };
        }
        Err(error) => {
            return Check {
                title: "Service backups",
                status: Status::Warning,
                detail: format!("Could not read backup directory: {error}"),
            };
        }
    };
    let mut files = 0;
    let mut readable = 0;
    let mut invalid = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            invalid.push("directory entry could not be read".to_owned());
            continue;
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !entry.file_type().is_ok_and(|kind| kind.is_file())
            || !name.starts_with("services-")
            || path
                .extension()
                .is_none_or(|extension| !extension.to_string_lossy().eq_ignore_ascii_case("json"))
        {
            continue;
        }
        files += 1;
        let result = fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<Vec<SvcRec>>(without_utf8_bom(&bytes))
                    .map_err(|error| error.to_string())
            });
        let valid = result.is_ok_and(|list| {
            let mut names = HashSet::new();
            !list.is_empty()
                && list.iter().all(|service| {
                    !service.name.trim().is_empty()
                        && (2..=4).contains(&service.start)
                        && (0..=1).contains(&service.delayed)
                        && names.insert(service.name.to_lowercase())
                })
        });
        if valid {
            readable += 1;
        } else {
            invalid.push(name);
        }
    }

    if files == 0 && invalid.is_empty() {
        Check {
            title: "Service backups",
            status: Status::Missing,
            detail: "No service backups saved yet.".into(),
        }
    } else if invalid.is_empty() {
        Check {
            title: "Service backups",
            status: Status::Good,
            detail: format!("{files} readable backup file(s)."),
        }
    } else {
        Check {
            title: "Service backups",
            status: Status::Warning,
            detail: format!(
                "{readable} readable file(s), {} invalid: {}",
                invalid.len(),
                invalid.into_iter().take(5).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

fn without_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::state::{Record, RegRec};
    use serde_json::Value;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opti-backup-integrity-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn inspect_reports_readable_state_and_service_backups() {
        let root = temp_dir("valid");
        let state = BTreeMap::from([(
            "test".to_owned(),
            Record {
                reg: vec![RegRec {
                    p: r"HKCU:\Software\Test".into(),
                    n: "Value".into(),
                    exists: true,
                    value: Value::from(1),
                    ty: Some("DWord".into()),
                }],
                ..Default::default()
            },
        )]);
        let json = serde_json::to_vec(&state).unwrap();
        let mut state_with_bom = vec![0xEF, 0xBB, 0xBF];
        state_with_bom.extend(json);
        fs::write(root.join("state.json"), state_with_bom).unwrap();
        fs::write(
            root.join("services-20261030.json"),
            r#"[{"Name":"TestService","Start":3,"Delayed":0}]"#,
        )
        .unwrap();

        let checks = inspect(&root.join("state.json"), &root);

        assert_eq!(checks[0].status, Status::Good);
        assert!(checks[0].detail.contains("1 tweak record"));
        assert_eq!(checks[1].status, Status::Good);
        assert!(checks[1].detail.contains("1 readable backup"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_and_incomplete_backups_warn_without_changing_files() {
        let root = temp_dir("corrupt");
        let state_path = root.join("state.json");
        fs::write(
            &state_path,
            r#"{"empty":{"Reg":[],"Svc":[],"Task":[],"Data":null}}"#,
        )
        .unwrap();
        let service_path = root.join("services-broken.json");
        fs::write(&service_path, "not json").unwrap();
        let state_before = fs::read(&state_path).unwrap();
        let service_before = fs::read(&service_path).unwrap();

        let checks = inspect(&state_path, &root);

        assert_eq!(checks[0].status, Status::Warning);
        assert_eq!(checks[1].status, Status::Warning);
        assert_eq!(fs::read(&state_path).unwrap(), state_before);
        assert_eq!(fs::read(&service_path).unwrap(), service_before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_backups_are_reported_as_missing() {
        let root = temp_dir("missing");
        let checks = inspect(&root.join("state.json"), &root.join("backups"));
        assert!(checks.iter().all(|check| check.status == Status::Missing));
        fs::remove_dir_all(root).unwrap();
    }
}
