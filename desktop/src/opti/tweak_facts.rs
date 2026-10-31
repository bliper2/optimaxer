//! Plain facts about a tweak, derived only from what the catalog actually contains: which settings it writes, whether
//! it needs a restart, and how undo works. Nothing here is a claim about speed. The catalog has no per-tweak
//! "trade-off" text, so trade-offs are limited to consequences that follow from the kind of setting changed.

use super::catalog::Tweak;
use super::special::Special;
use super::state::Record;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rollback {
    /// Every change is a registry value, service start type or scheduled task whose original is saved before the change.
    Reversible,
    /// Undo uses a saved record or backup for a built-in routine, so it depends on that record being intact.
    Conditional,
    /// The change cannot be taken back by Optimaxer.
    Irreversible,
}

impl Rollback {
    pub fn label(self) -> &'static str {
        match self {
            Rollback::Reversible => "Reversible",
            Rollback::Conditional => "Conditionally reversible",
            Rollback::Irreversible => "Not reversible",
        }
    }
}

pub fn rollback(tweak: &Tweak) -> (Rollback, &'static str) {
    match &tweak.special {
        Some(Special::KillProcs(_)) => (
            Rollback::Irreversible,
            "Ends programs that are running now. Undo cannot bring back what they were doing; they can be started again by hand.",
        ),
        Some(_) => (
            Rollback::Conditional,
            "Undo relies on the original state Optimaxer saved before the change. If that record or backup is damaged or the setting was changed elsewhere, undo is refused instead of guessing.",
        ),
        None if tweak.script.apply => (
            Rollback::Conditional,
            "Part of this tweak is a routine rather than plain settings, so undo depends on what Optimaxer saved before running it.",
        ),
        None => (
            Rollback::Reversible,
            "The original registry values, service start types and task states are saved before the change and put back on undo.",
        ),
    }
}

/// Who the change applies to, from the registry hives the tweak writes.
pub fn scope(tweak: &Tweak) -> &'static str {
    let machine = tweak
        .reg
        .iter()
        .any(|r| !r.p.to_ascii_uppercase().starts_with("HKCU"))
        || !tweak.svc.is_empty()
        || !tweak.task.is_empty()
        || tweak.special.is_some();
    let user = tweak
        .reg
        .iter()
        .any(|r| r.p.to_ascii_uppercase().starts_with("HKCU"));
    match (machine, user) {
        (true, true) => "This PC and the current user",
        (true, false) => "This PC, all users",
        (false, true) => "Current user only",
        (false, false) => "No settings written",
    }
}

/// Whether applying needs administrator rights: anything outside the current user's registry hive.
pub fn needs_admin(tweak: &Tweak) -> bool {
    !matches!(scope(tweak), "Current user only" | "No settings written")
}

/// Consequences that follow from the kind of setting changed. Not a promise that anything noticeable happens.
pub fn tradeoffs(tweak: &Tweak) -> Vec<String> {
    let mut out = Vec::new();
    if !tweak.svc.is_empty() {
        out.push(format!(
            "Changes the start type of {} Windows service(s). Features that depend on them can stop working until you undo this.",
            tweak.svc.len()
        ));
    }
    if !tweak.task.is_empty() {
        out.push(format!(
            "Disables {} scheduled task(s), so the work they do no longer runs on its own.",
            tweak.task.len()
        ));
    }
    if tweak.explorer {
        out.push("Restarts Windows Explorer; open File Explorer windows close.".into());
    }
    if tweak.restart {
        out.push("Takes full effect only after a restart.".into());
    }
    if matches!(tweak.risk.as_str(), "Moderate" | "Advanced") {
        out.push(format!(
            "Rated {}: read the preview first and keep a restore point.",
            tweak.risk
        ));
    }
    if !tweak.conflict.is_empty() {
        out.push(format!(
            "Only one tweak in the \"{}\" group can be active; choosing this replaces the others.",
            tweak.conflict
        ));
    }
    if tweak.has_tag("optin") {
        out.push("Opt-in: never included in a preset.".into());
    }
    out
}

/// The set Beginner shows: low risk, suited to this PC, and eligible for an automatic plan. Never an opt-in tweak.
pub fn guided(tweak: &Tweak, fits: bool) -> bool {
    fits && tweak.risk == "Safe" && tweak.auto_pick() && tweak.retired.is_empty()
}

fn show(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

/// What Optimaxer saved before it changed the tweak, as readable lines for the Expert view.
pub fn original_lines(record: &Record) -> Vec<String> {
    let mut out = vec![format!("saved {}", record.time)];
    for r in &record.reg {
        out.push(if r.exists {
            format!("original  {}\\{} = {}", r.p, r.n, show(&r.value))
        } else {
            format!("original  {}\\{} did not exist", r.p, r.n)
        });
    }
    for s in &record.svc {
        out.push(format!(
            "original  service {} start type {}",
            s.name, s.start
        ));
    }
    for t in &record.task {
        out.push(format!(
            "original  task {} {}",
            t.full,
            if t.enabled != 0 {
                "enabled"
            } else {
                "disabled"
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::catalog::{RegItem, RegVal, SvcItem};
    use crate::opti::state::{RegRec, SvcRec};

    fn registry(path: &str) -> Tweak {
        Tweak {
            risk: "Safe".into(),
            tags: vec!["safe".into()],
            reg: vec![RegItem {
                p: path.into(),
                n: "X".into(),
                v: RegVal::Num(1),
                t: String::new(),
            }],
            ..Tweak::default()
        }
    }

    #[test]
    fn plain_settings_are_reversible_and_routines_are_not_overstated() {
        let mut tweak = registry(r"HKCU:\Software\X");
        assert_eq!(rollback(&tweak).0, Rollback::Reversible);
        tweak.special = Some(Special::Teredo);
        assert_eq!(rollback(&tweak).0, Rollback::Conditional);
        tweak.special = Some(Special::KillProcs(&["x.exe"]));
        assert_eq!(rollback(&tweak).0, Rollback::Irreversible);
    }

    #[test]
    fn scope_and_admin_follow_the_hives_written() {
        let user = registry(r"HKCU:\Software\X");
        assert_eq!(scope(&user), "Current user only");
        assert!(!needs_admin(&user));
        let mut both = registry(r"HKLM:\SOFTWARE\X");
        assert_eq!(scope(&both), "This PC, all users");
        assert!(needs_admin(&both));
        both.reg.push(user.reg[0].clone());
        assert_eq!(scope(&both), "This PC and the current user");
    }

    #[test]
    fn trade_offs_come_only_from_the_data() {
        let mut tweak = registry(r"HKCU:\Software\X");
        assert!(tradeoffs(&tweak).is_empty());
        tweak.svc.push(SvcItem {
            name: "Fax".into(),
            start: 4,
        });
        tweak.restart = true;
        tweak.risk = "Moderate".into();
        let lines = tradeoffs(&tweak);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("1 Windows service"));
    }

    #[test]
    fn beginner_set_excludes_risky_unfit_and_opt_in_tweaks() {
        let tweak = registry(r"HKCU:\Software\X");
        assert!(guided(&tweak, true));
        assert!(!guided(&tweak, false));
        let mut moderate = tweak.clone();
        moderate.risk = "Moderate".into();
        assert!(!guided(&moderate, true));
        let mut optin = tweak;
        optin.tags.push("optin".into());
        assert!(!guided(&optin, true));
    }

    #[test]
    fn saved_originals_are_listed_including_values_that_did_not_exist() {
        let record = Record {
            reg: vec![
                RegRec {
                    p: r"HKCU:\A".into(),
                    n: "One".into(),
                    exists: true,
                    value: Value::from(7),
                    ty: None,
                },
                RegRec {
                    p: r"HKCU:\A".into(),
                    n: "Two".into(),
                    exists: false,
                    ..RegRec::default()
                },
            ],
            svc: vec![SvcRec {
                name: "Fax".into(),
                start: 3,
                delayed: 0,
            }],
            time: "2026-10-30T10:00:00".into(),
            ..Record::default()
        };
        let lines = original_lines(&record);
        assert_eq!(lines[1], r"original  HKCU:\A\One = 7");
        assert_eq!(lines[2], r"original  HKCU:\A\Two did not exist");
        assert_eq!(lines[3], "original  service Fax start type 3");
    }
}
