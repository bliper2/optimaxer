//! systemd system services: list, and change with a saved original (enabled at boot and running now are separate).

use super::store::UnitSnap;
use super::system::{Plan, Step, System, unit_active, unit_enabled};

/// Services that commonly matter for speed or privacy, with advice from the Python edition.
pub const CURATED: [(&str, &str, &str); 15] = [
    (
        "NetworkManager-wait-online.service",
        "Boot waits until the network is up",
        "disable if you have no network mounts",
    ),
    (
        "avahi-daemon.service",
        "mDNS/Zeroconf discovery",
        "disable unless you use network printers or Chromecast",
    ),
    (
        "avahi-daemon.socket",
        "Avahi socket activation",
        "disable together with avahi-daemon",
    ),
    (
        "ModemManager.service",
        "Mobile broadband modem support",
        "disable on desktops without a modem",
    ),
    (
        "bluetooth.service",
        "Bluetooth",
        "disable if you never use Bluetooth",
    ),
    (
        "cups.service",
        "Printing (CUPS)",
        "disable if you never print",
    ),
    (
        "cups.socket",
        "CUPS socket activation",
        "disable together with cups",
    ),
    (
        "cups-browsed.service",
        "Network printer discovery",
        "disable unless you print to network printers",
    ),
    (
        "smb.service",
        "Samba file sharing server",
        "disable unless you share files with Windows",
    ),
    (
        "sshd.service",
        "SSH server",
        "disable unless you log in remotely",
    ),
    (
        "docker.service",
        "Docker daemon",
        "disable and start on demand if you rarely use containers",
    ),
    (
        "tlp.service",
        "TLP laptop power management",
        "leave enabled on laptops",
    ),
    ("fstrim.timer", "Weekly SSD TRIM", "leave enabled on SSDs"),
    (
        "paccache.timer",
        "Weekly package cache cleaning",
        "leave enabled",
    ),
    (
        "lvm2-monitor.service",
        "LVM monitoring",
        "disable if you do not use LVM",
    ),
];

pub fn hint(unit: &str) -> Option<(&'static str, &'static str)> {
    CURATED
        .iter()
        .find(|(u, _, _)| *u == unit)
        .map(|(_, label, hint)| (*label, *hint))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitStatus {
    pub unit: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

/// `systemctl list-units --type=service --all --no-legend --plain` rows.
pub fn parse_units(text: &str) -> Vec<UnitStatus> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let (unit, load, active, sub) =
                (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            unit.contains('.').then(|| UnitStatus {
                unit: unit.to_string(),
                load: load.to_string(),
                active: active.to_string(),
                sub: sub.to_string(),
                description: parts.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

/// `systemctl list-unit-files --type=service --no-legend` rows: unit and its boot-time state.
pub fn parse_unit_files(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let (unit, state) = (parts.next()?, parts.next()?);
            unit.contains('.')
                .then(|| (unit.to_string(), state.to_string()))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceRow {
    pub unit: String,
    pub description: String,
    /// `enabled`, `disabled`, `static`, `masked`, `indirect`, ...
    pub boot: String,
    /// `active`, `inactive`, `failed`, or `-` for a unit file that is not loaded.
    pub active: String,
    pub sub: String,
    pub advice: Option<&'static str>,
}

impl ServiceRow {
    /// Only units with an `[Install]` section can be switched on or off.
    pub fn switchable(&self) -> bool {
        matches!(
            self.boot.as_str(),
            "enabled" | "disabled" | "enabled-runtime"
        )
    }

    pub fn is_enabled(&self) -> bool {
        matches!(self.boot.as_str(), "enabled" | "enabled-runtime")
    }
}

pub fn list(sys: &dyn System) -> Result<Vec<ServiceRow>, String> {
    let files = sys.run(&[
        "systemctl",
        "list-unit-files",
        "--type=service",
        "--no-legend",
        "--no-pager",
    ]);
    if !files.ok() {
        return Err(format!(
            "systemctl is not available here: {}",
            files.text.trim()
        ));
    }
    let loaded = sys.run(&[
        "systemctl",
        "list-units",
        "--type=service",
        "--all",
        "--no-legend",
        "--no-pager",
        "--plain",
    ]);
    let statuses = if loaded.ok() {
        parse_units(&loaded.text)
    } else {
        Vec::new()
    };
    let mut rows: Vec<ServiceRow> = parse_unit_files(&files.text)
        .into_iter()
        .filter(|(unit, _)| !unit.contains('@'))
        .map(|(unit, boot)| {
            let status = statuses.iter().find(|s| s.unit == unit);
            ServiceRow {
                description: status
                    .map(|s| s.description.clone())
                    .or_else(|| hint(&unit).map(|(l, _)| l.to_string()))
                    .unwrap_or_default(),
                active: status.map_or_else(|| "-".to_string(), |s| s.active.clone()),
                sub: status.map(|s| s.sub.clone()).unwrap_or_default(),
                advice: hint(&unit).map(|(_, h)| h),
                unit,
                boot,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        (a.advice.is_none(), a.unit.to_lowercase())
            .cmp(&(b.advice.is_none(), b.unit.to_lowercase()))
    });
    Ok(rows)
}

pub struct Prepared {
    pub plan: Plan,
    pub snapshot: UnitSnap,
}

/// Enable and start, or disable and stop. `previous` keeps the first original across repeated changes.
pub fn prepare_set(
    sys: &dyn System,
    unit: &str,
    enable: bool,
    previous: Option<&UnitSnap>,
) -> Prepared {
    let snapshot = previous.cloned().unwrap_or_else(|| UnitSnap {
        unit: unit.to_string(),
        enabled: unit_enabled(sys, unit),
        active: unit_active(sys, unit),
    });
    let mut plan = Plan::default();
    plan.push(Step::run(&[
        "systemctl",
        if enable { "enable" } else { "disable" },
        "--now",
        unit,
    ]));
    Prepared { plan, snapshot }
}

pub fn prepare_restore(snapshot: &UnitSnap) -> Plan {
    let mut plan = Plan::default();
    plan.push(Step::run(&[
        "systemctl",
        if snapshot.enabled {
            "enable"
        } else {
            "disable"
        },
        &snapshot.unit,
    ]));
    plan.push(Step::run(&[
        "systemctl",
        if snapshot.active { "start" } else { "stop" },
        &snapshot.unit,
    ]));
    plan
}

#[cfg(test)]
pub fn matches_snapshot(sys: &dyn System, snapshot: &UnitSnap) -> bool {
    unit_enabled(sys, &snapshot.unit) == snapshot.enabled
        && unit_active(sys, &snapshot.unit) == snapshot.active
}

/// Names of units in the failed state, for the Overview health line.
pub fn failed_units(sys: &dyn System) -> Option<Vec<String>> {
    let out = sys.run(&[
        "systemctl",
        "--failed",
        "--no-legend",
        "--no-pager",
        "--plain",
    ]);
    out.ok()
        .then(|| parse_units(&out.text).into_iter().map(|u| u.unit).collect())
}

#[cfg(test)]
mod tests {
    use super::super::system::fake::FakeSystem;
    use super::*;

    const LOADED: &str = "avahi-daemon.service loaded active running Avahi mDNS/DNS-SD Stack\nbluetooth.service loaded inactive dead Bluetooth service\nfoo.service not-found inactive dead foo.service\nbroken.service loaded failed failed Broken thing\n";
    const FILES: &str = "avahi-daemon.service enabled disabled\nbluetooth.service disabled disabled\nsystemd-journald.service static -\nuser@.service static -\ngetty@.service enabled enabled\nzzz.service masked enabled\n";

    #[test]
    fn units_and_unit_files_parse() {
        let units = parse_units(LOADED);
        assert_eq!(units.len(), 4);
        assert_eq!(units[0].description, "Avahi mDNS/DNS-SD Stack");
        assert_eq!(units[3].active, "failed");
        assert_eq!(
            parse_unit_files(FILES)[1],
            ("bluetooth.service".to_string(), "disabled".to_string())
        );
        assert!(parse_units("garbage\n\n").is_empty());
    }

    #[test]
    fn list_merges_files_with_runtime_state_and_puts_advice_first() {
        let sys = FakeSystem::default();
        sys.canned.borrow_mut().insert(
            "systemctl list-unit-files --type=service --no-legend --no-pager".into(),
            super::super::system::CmdOut {
                code: 0,
                text: FILES.into(),
            },
        );
        sys.canned.borrow_mut().insert(
            "systemctl list-units --type=service --all --no-legend --no-pager --plain".into(),
            super::super::system::CmdOut {
                code: 0,
                text: LOADED.into(),
            },
        );
        let rows = list(&sys).unwrap();
        let names: Vec<_> = rows.iter().map(|r| r.unit.as_str()).collect();
        assert_eq!(
            &names[..2],
            ["avahi-daemon.service", "bluetooth.service"],
            "units with advice come first"
        );
        assert!(
            !names.iter().any(|n| n.contains('@')),
            "template units are not real services: {names:?}"
        );
        let journald = rows
            .iter()
            .find(|r| r.unit == "systemd-journald.service")
            .unwrap();
        assert_eq!(journald.active, "-");
        assert!(!journald.switchable() && rows[0].switchable() && rows[0].is_enabled());
        assert_eq!(
            rows[0].advice,
            Some("disable unless you use network printers or Chromecast")
        );
    }

    #[test]
    fn systemctl_missing_is_an_error_not_an_empty_list() {
        assert!(list(&FakeSystem::default()).is_err());
    }

    #[test]
    fn disable_keeps_the_first_original_and_restore_puts_both_states_back() {
        let sys = FakeSystem::default().with_unit("bluetooth.service", true, false);
        let first = prepare_set(&sys, "bluetooth.service", false, None);
        assert_eq!(
            first.snapshot,
            UnitSnap {
                unit: "bluetooth.service".into(),
                enabled: true,
                active: false
            }
        );
        sys.execute(&first.plan).unwrap();
        assert!(!sys.enabled.borrow().contains("bluetooth.service"));
        let again = prepare_set(&sys, "bluetooth.service", true, Some(&first.snapshot));
        assert_eq!(
            again.snapshot, first.snapshot,
            "a later change must not replace the real original"
        );
        sys.execute(&prepare_restore(&first.snapshot)).unwrap();
        assert!(
            matches_snapshot(&sys, &first.snapshot),
            "enabled but not running, exactly as before"
        );
    }

    #[test]
    fn failed_units_come_from_systemctl_failed() {
        let sys = FakeSystem::default();
        assert_eq!(failed_units(&sys), None);
        sys.canned.borrow_mut().insert(
            "systemctl --failed --no-legend --no-pager --plain".into(),
            super::super::system::CmdOut {
                code: 0,
                text: "broken.service loaded failed failed Broken thing\n".into(),
            },
        );
        assert_eq!(failed_units(&sys), Some(vec!["broken.service".to_string()]));
    }
}
