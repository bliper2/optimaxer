//! Things that start with the desktop session: XDG autostart entries (`~/.config/autostart` and
//! `/etc/xdg/autostart`) and systemd user services. Both are changed without administrator rights.

use super::store::{AutostartSnap, UnitSnap};
use super::system::{Plan, Step, System, read_text};

const MARK: &str = "# optimaxer-override";
const SYSTEM_DIR: &str = "/etc/xdg/autostart";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutostartItem {
    pub file: String,
    pub name: String,
    pub exec: String,
    pub scope: Scope,
    pub enabled: bool,
}

pub fn user_dir(sys: &dyn System) -> String {
    format!("{}/.config/autostart", sys.home().trim_end_matches('/'))
}

/// Key/value pairs of the `[Desktop Entry]` group; later duplicates and other groups are ignored.
pub fn parse_desktop(text: &str) -> Vec<(String, String)> {
    let mut in_entry = false;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && !line.starts_with('#')
            && let Some((k, v)) = line.split_once('=')
        {
            let key = k.trim().to_string();
            if !pairs.iter().any(|(existing, _)| *existing == key) {
                pairs.push((key, v.trim().to_string()));
            }
        }
    }
    pairs
}

fn value<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn list_autostart(sys: &dyn System) -> Vec<AutostartItem> {
    let user = user_dir(sys);
    let mut seen: Vec<String> = Vec::new();
    let mut items = Vec::new();
    for (scope, dir) in [(Scope::User, user.as_str()), (Scope::System, SYSTEM_DIR)] {
        for file in sys.list_dir(dir) {
            if !file.ends_with(".desktop") || seen.contains(&file) {
                continue;
            }
            seen.push(file.clone());
            let Some(mut text) = read_text(sys, &format!("{dir}/{file}")) else {
                continue;
            };
            if scope == Scope::System
                && let Some(over) = read_text(sys, &format!("{user}/{file}"))
            {
                text = over;
            }
            let pairs = parse_desktop(&text);
            let hidden = value(&pairs, "Hidden").is_some_and(|v| v.eq_ignore_ascii_case("true"))
                || value(&pairs, "X-GNOME-Autostart-enabled")
                    .is_some_and(|v| v.eq_ignore_ascii_case("false"));
            items.push(AutostartItem {
                name: value(&pairs, "Name").unwrap_or(&file).to_string(),
                exec: value(&pairs, "Exec").unwrap_or_default().to_string(),
                file,
                scope,
                enabled: !hidden,
            });
        }
    }
    items.sort_by_key(|i| i.name.to_lowercase());
    items
}

/// The user-owned file content that enables or disables `item`, and the original to save for undo.
#[derive(Debug)]
pub struct AutostartChange {
    pub path: String,
    pub new_content: String,
    pub snapshot: AutostartSnap,
}

pub fn prepare_autostart(
    sys: &dyn System,
    item: &AutostartItem,
    enable: bool,
    previous: Option<&AutostartSnap>,
) -> Result<AutostartChange, String> {
    let path = format!("{}/{}", user_dir(sys), item.file);
    let existing = sys.read(&path)?;
    let snapshot = previous.cloned().unwrap_or_else(|| AutostartSnap {
        file: item.file.clone(),
        existed: existing.is_some(),
        content: existing.clone(),
    });
    let base = match existing {
        Some(text) => text,
        None => {
            let system = sys
                .read(&format!("{SYSTEM_DIR}/{}", item.file))?
                .ok_or_else(|| format!("{} no longer exists in {SYSTEM_DIR}.", item.file))?;
            format!("{}\n{MARK}\n", system.trim_end_matches('\n'))
        }
    };
    let mut lines: Vec<String> = base
        .split('\n')
        .filter(|l| !l.trim_start().starts_with("Hidden") || !l.contains('='))
        .map(String::from)
        .collect();
    if enable {
        for line in &mut lines {
            if line.trim_start().starts_with("X-GNOME-Autostart-enabled") && line.contains('=') {
                *line = "X-GNOME-Autostart-enabled=true".into();
            }
        }
    } else {
        let at = lines
            .iter()
            .position(|l| l.trim() == "[Desktop Entry]")
            .map_or(0, |i| i + 1);
        lines.insert(at, "Hidden=true".into());
    }
    Ok(AutostartChange {
        path,
        new_content: lines.join("\n"),
        snapshot,
    })
}

pub fn restore_autostart(sys: &dyn System, snap: &AutostartSnap) -> Result<(), String> {
    let path = format!("{}/{}", user_dir(sys), snap.file);
    match (&snap.existed, &snap.content) {
        (true, Some(content)) => sys.write_user(&path, content),
        _ => sys.remove_user(&path),
    }
}

// region systemd user units
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserUnit {
    pub unit: String,
    pub enabled: bool,
    pub active: bool,
}

pub fn list_user_units(sys: &dyn System) -> Result<Vec<UserUnit>, String> {
    let files = sys.run(&[
        "systemctl",
        "--user",
        "list-unit-files",
        "--no-legend",
        "--no-pager",
        "--type=service,timer,socket",
    ]);
    if !files.ok() {
        return Err(format!("No systemd user session: {}", files.text.trim()));
    }
    let running = sys.run(&[
        "systemctl",
        "--user",
        "list-units",
        "--no-legend",
        "--no-pager",
        "--plain",
        "--type=service,timer,socket",
    ]);
    let active: Vec<String> = if running.ok() {
        super::services::parse_units(&running.text)
            .into_iter()
            .filter(|u| u.active == "active")
            .map(|u| u.unit)
            .collect()
    } else {
        Vec::new()
    };
    let mut units: Vec<UserUnit> = super::services::parse_unit_files(&files.text)
        .into_iter()
        .filter(|(unit, state)| {
            !unit.contains('@')
                && matches!(state.as_str(), "enabled" | "disabled" | "enabled-runtime")
        })
        .map(|(unit, state)| UserUnit {
            enabled: state != "disabled",
            active: active.contains(&unit),
            unit,
        })
        .collect();
    units.sort_by(|a, b| b.enabled.cmp(&a.enabled).then(a.unit.cmp(&b.unit)));
    Ok(units)
}

pub struct UserUnitChange {
    pub plan: Plan,
    pub snapshot: UnitSnap,
}

pub fn prepare_user_unit(
    sys: &dyn System,
    unit: &str,
    enable: bool,
    previous: Option<&UnitSnap>,
) -> UserUnitChange {
    // The user manager answers `is-enabled` like the system one, with --user.
    let state = |verb: &str| sys.run(&["systemctl", "--user", verb, unit]);
    let snapshot = previous.cloned().unwrap_or_else(|| UnitSnap {
        unit: unit.to_string(),
        enabled: matches!(
            state("is-enabled").text.trim(),
            "enabled" | "enabled-runtime"
        ),
        active: state("is-active").ok(),
    });
    let mut plan = Plan::default();
    plan.push(Step::run_user(&[
        "systemctl",
        "--user",
        if enable { "enable" } else { "disable" },
        "--now",
        unit,
    ]));
    UserUnitChange { plan, snapshot }
}

pub fn restore_user_unit(snapshot: &UnitSnap) -> Plan {
    let mut plan = Plan::default();
    plan.push(Step::run_user(&[
        "systemctl",
        "--user",
        if snapshot.enabled {
            "enable"
        } else {
            "disable"
        },
        &snapshot.unit,
    ]));
    plan.push(Step::run_user(&[
        "systemctl",
        "--user",
        if snapshot.active { "start" } else { "stop" },
        &snapshot.unit,
    ]));
    plan
}
// endregion

#[cfg(test)]
mod tests {
    use super::super::system::fake::FakeSystem;
    use super::*;

    const DESKTOP: &str = "[Desktop Entry]\nType=Application\nName=Cloud Sync\nExec=/usr/bin/cloudsync --tray\n# Name=Ignored\n[Desktop Action x]\nName=Other\n";

    #[test]
    fn desktop_entries_read_only_the_main_group_and_first_value() {
        let pairs = parse_desktop(&format!("{DESKTOP}Name=Late\n"));
        assert_eq!(value(&pairs, "Name"), Some("Cloud Sync"));
        assert_eq!(value(&pairs, "Exec"), Some("/usr/bin/cloudsync --tray"));
    }

    fn machine() -> FakeSystem {
        FakeSystem::default()
            .with_file("/etc/xdg/autostart/cloud.desktop", DESKTOP)
            .with_file(
                "/etc/xdg/autostart/chat.desktop",
                "[Desktop Entry]\nName=Chat\nExec=chat\nX-GNOME-Autostart-enabled=false\n",
            )
            .with_file(
                "/home/tester/.config/autostart/mine.desktop",
                "[Desktop Entry]\nName=Mine\nExec=mine\nHidden=true\n",
            )
            .with_file(
                "/home/tester/.config/autostart/chat.desktop",
                "[Desktop Entry]\nName=Chat\nExec=chat\nX-GNOME-Autostart-enabled=true\n",
            )
    }

    #[test]
    fn user_entries_override_system_ones_and_hidden_means_off() {
        let items = list_autostart(&machine());
        let by = |file: &str| items.iter().find(|i| i.file == file).unwrap();
        assert_eq!(items.len(), 3);
        assert!(by("cloud.desktop").enabled && by("cloud.desktop").scope == Scope::System);
        assert!(
            by("chat.desktop").enabled,
            "the user's file turns the system entry back on"
        );
        assert_eq!(by("chat.desktop").scope, Scope::User);
        assert!(!by("mine.desktop").enabled);
        assert_eq!(by("cloud.desktop").exec, "/usr/bin/cloudsync --tray");
    }

    #[test]
    fn disabling_a_system_entry_writes_a_marked_user_override_and_undo_removes_it() {
        let sys = machine();
        let item = list_autostart(&sys)
            .into_iter()
            .find(|i| i.file == "cloud.desktop")
            .unwrap();
        let change = prepare_autostart(&sys, &item, false, None).unwrap();
        assert_eq!(change.path, "/home/tester/.config/autostart/cloud.desktop");
        assert!(
            change
                .new_content
                .contains("[Desktop Entry]\nHidden=true\nType=Application")
        );
        assert!(change.new_content.contains(MARK));
        assert!(!change.snapshot.existed);
        sys.write_user(&change.path, &change.new_content).unwrap();
        assert!(
            !list_autostart(&sys)
                .into_iter()
                .find(|i| i.file == "cloud.desktop")
                .unwrap()
                .enabled
        );
        restore_autostart(&sys, &change.snapshot).unwrap();
        assert!(sys.file(&change.path).is_none());
        assert!(
            list_autostart(&sys)
                .into_iter()
                .find(|i| i.file == "cloud.desktop")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn enabling_removes_hidden_and_undo_restores_the_exact_original() {
        let sys = machine();
        let item = list_autostart(&sys)
            .into_iter()
            .find(|i| i.file == "mine.desktop")
            .unwrap();
        let before = sys
            .file("/home/tester/.config/autostart/mine.desktop")
            .unwrap();
        let change = prepare_autostart(&sys, &item, true, None).unwrap();
        assert!(!change.new_content.contains("Hidden"));
        sys.write_user(&change.path, &change.new_content).unwrap();
        assert!(
            list_autostart(&sys)
                .into_iter()
                .find(|i| i.file == "mine.desktop")
                .unwrap()
                .enabled
        );
        restore_autostart(&sys, &change.snapshot).unwrap();
        assert_eq!(sys.file(&change.path).unwrap(), before);
    }

    #[test]
    fn a_vanished_system_entry_is_reported() {
        let sys = FakeSystem::default();
        let item = AutostartItem {
            file: "gone.desktop".into(),
            name: "Gone".into(),
            exec: String::new(),
            scope: Scope::System,
            enabled: true,
        };
        assert!(
            prepare_autostart(&sys, &item, false, None)
                .unwrap_err()
                .contains("no longer exists")
        );
    }

    #[test]
    fn user_units_list_enabled_first_and_changes_need_no_root() {
        let sys = FakeSystem::default();
        sys.canned.borrow_mut().insert(
            "systemctl --user list-unit-files --no-legend --no-pager --type=service,timer,socket".into(),
            super::super::system::CmdOut { code: 0, text: "b.service disabled enabled\na.service enabled enabled\nc.service static -\nx@.service enabled enabled\n".into() },
        );
        sys.canned.borrow_mut().insert(
            "systemctl --user list-units --no-legend --no-pager --plain --type=service,timer,socket".into(),
            super::super::system::CmdOut { code: 0, text: "a.service loaded active running A\n".into() },
        );
        let units = list_user_units(&sys).unwrap();
        assert_eq!(
            units,
            vec![
                UserUnit {
                    unit: "a.service".into(),
                    enabled: true,
                    active: true
                },
                UserUnit {
                    unit: "b.service".into(),
                    enabled: false,
                    active: false
                },
            ]
        );
        let change = prepare_user_unit(&sys, "a.service", false, None);
        assert!(!change.plan.needs_root());
        assert_eq!(restore_user_unit(&change.snapshot).steps.len(), 2);
        assert!(list_user_units(&FakeSystem::default()).is_err());
    }
}
