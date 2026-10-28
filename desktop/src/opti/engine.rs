//! Apply, undo, test and dry-run for tweaks. A direct port of Optimaxer's Invoke-TweakApply / Invoke-TweakUndo /
//! Test-Tweak: original values are snapshotted before the first change, a value another tweak already saved is
//! reused (so undo works in any order), and every apply is verified by testing the tweak again.

use super::backend::{RegData, System};
use super::catalog::Tweak;
use super::special::Ctx;
use super::state::{Record, RegRec, State, SvcRec, TaskRec};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Reg { path: String, name: String, from: Option<RegData>, to: RegData },
    Svc { name: String, from: u32, to: u32 },
    Task { full: String },
    Note(String),
}

impl Change {
    pub fn describe(&self) -> String {
        match self {
            Change::Reg { path, name, from, to } => {
                let old = from.as_ref().map(RegData::show).unwrap_or_else(|| "(not set)".into());
                format!("{path}\\{name}: {old} -> {}", to.show())
            }
            Change::Svc { name, from, to } => format!("Service {name}: start type {} -> {}", start_label(*from), start_label(*to)),
            Change::Task { full } => format!("Scheduled task {full}: disable"),
            Change::Note(n) => n.clone(),
        }
    }
}

pub fn start_label(s: u32) -> &'static str {
    match s {
        0 => "Boot",
        1 => "System",
        2 => "Automatic",
        3 => "Manual",
        4 => "Disabled",
        _ => "?",
    }
}

pub struct Engine<'a> {
    pub sys: &'a dyn System,
    pub backups: PathBuf,
    tasks: RefCell<HashMap<String, Option<bool>>>,
}

impl<'a> Engine<'a> {
    pub fn new(sys: &'a dyn System, backups: PathBuf) -> Self {
        Engine { sys, backups, tasks: RefCell::new(HashMap::new()) }
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx { sys: self.sys, backups: &self.backups }
    }

    fn task_enabled(&self, full: &str) -> Option<bool> {
        if let Some(v) = self.tasks.borrow().get(full) {
            return *v;
        }
        let v = self.sys.task_enabled(full);
        self.tasks.borrow_mut().insert(full.to_string(), v);
        v
    }

    pub fn clear_cache(&self) {
        self.tasks.borrow_mut().clear();
    }

    /// True when the tweak is currently in effect.
    pub fn test(&self, t: &Tweak, state: &State) -> bool {
        if let Some(v) = t.special.as_ref().and_then(|s| s.test(&self.ctx())) {
            return v;
        }
        let mut checked = false;
        for r in &t.reg {
            checked = true;
            match self.sys.reg_get(&r.p, &r.n) {
                Some(actual) if r.wanted().matches(&actual) => {}
                _ => return false,
            }
        }
        if !t.svc.is_empty() {
            let (mut total, mut ok) = (0u32, 0u32);
            for s in &t.svc {
                if let Some(snap) = self.sys.svc_get(&s.name) {
                    total += 1;
                    if snap.start == s.start {
                        ok += 1;
                    }
                }
            }
            if total > 0 {
                checked = true;
                if (ok as f64 / total as f64) < t.svc_tol {
                    return false;
                }
            }
        }
        for tk in &t.task {
            if let Some(enabled) = self.task_enabled(tk) {
                checked = true;
                if enabled {
                    return false;
                }
            }
        }
        if !checked {
            return state.contains(&t.id);
        }
        true
    }

    pub fn status_all(&self, tweaks: &[Tweak], state: &State) -> HashMap<String, bool> {
        self.clear_cache();
        let out = tweaks.iter().map(|t| (t.id.clone(), self.test(t, state))).collect();
        self.clear_cache();
        out
    }

    /// Dry run: what applying this tweak would change right now. Reads only.
    pub fn plan(&self, t: &Tweak) -> Vec<Change> {
        let mut out = Vec::new();
        for r in &t.reg {
            let want = r.wanted();
            let cur = self.sys.reg_get(&r.p, &r.n);
            if !cur.as_ref().is_some_and(|c| want.matches(c)) {
                out.push(Change::Reg { path: r.p.clone(), name: r.n.clone(), from: cur, to: want });
            }
        }
        for s in &t.svc {
            if let Some(snap) = self.sys.svc_get(&s.name) {
                if snap.start != s.start {
                    out.push(Change::Svc { name: s.name.clone(), from: snap.start, to: s.start });
                }
            }
        }
        for tk in &t.task {
            if self.task_enabled(tk) == Some(true) {
                out.push(Change::Task { full: tk.clone() });
            }
        }
        if let Some(sp) = &t.special {
            if sp.test(&self.ctx()) != Some(true) {
                out.push(Change::Note(sp.describe()));
            }
        }
        if t.partial {
            out.push(Change::Note("A PowerShell-only extra step of this tweak is not available here and is skipped.".into()));
        }
        out
    }

    /// Same order as the PowerShell Invoke-TweakApply: every original is read first and the record is saved BEFORE
    /// the first change, so a step that fails later cannot leave a change behind without its original value.
    pub fn apply(&self, t: &Tweak, state: &mut State, log: &mut dyn FnMut(&str)) -> bool {
        log(&format!("Apply: {}", t.name));
        self.clear_cache();
        // A failed apply or later setting drift must not replace the first saved originals on retry.
        let fresh = !state.contains(&t.id);
        let mut rec = if fresh { Record { time: super::now_iso(), ..Default::default() } } else { state.records.get(&t.id).cloned().unwrap_or_default() };

        // 1. read the originals
        let mut svc_now = Vec::new();
        for r in &t.reg {
            if fresh {
                let saved = find_shared_reg(state, &t.id, &r.p, &r.n).unwrap_or_else(|| snapshot_reg(self.sys, &r.p, &r.n));
                rec.reg.push(saved);
            }
        }
        for s in &t.svc {
            let Some(snap) = self.sys.svc_get(&s.name) else { continue };
            if fresh {
                let saved = find_shared_svc(state, &t.id, &s.name).unwrap_or(SvcRec { name: s.name.clone(), start: snap.start as i64, delayed: snap.delayed as i64 });
                rec.svc.push(saved);
            }
            svc_now.push((s, snap));
        }
        let mut tasks_now = Vec::new();
        for tk in &t.task {
            let Some(enabled) = self.task_enabled(tk) else { continue };
            if fresh {
                let saved = find_shared_task(state, &t.id, tk).unwrap_or(TaskRec { full: tk.clone(), enabled: enabled as i64 });
                rec.task.push(saved);
            }
            tasks_now.push(tk);
        }
        let snap = match &t.special {
            Some(sp) => match sp.snapshot(&self.ctx()) {
                Ok(s) => s,
                Err(e) => {
                    log(&format!("  WARN could not read the current values, nothing changed: {e}"));
                    return self.test(t, state);
                }
            },
            None => None,
        };
        if fresh {
            if let Some(s) = &snap {
                rec.data = s.clone();
            }
        }

        // 2. save the record before anything changes
        state.records.insert(t.id.clone(), rec.clone());
        if let Err(e) = state.save() {
            log(&format!("  WARN could not save the undo record, nothing changed: {e}"));
            if fresh {
                state.records.remove(&t.id);
            }
            return self.test(t, state);
        }

        // 3. change things
        for r in &t.reg {
            if let Err(e) = self.sys.reg_set(&r.p, &r.n, &r.wanted()) {
                let hint = if e.to_lowercase().contains("denied") { " (write-protected here, or run as administrator)" } else { "" };
                log(&format!("  WARN registry {}\\{}: {e}{hint}", r.p, r.n));
            }
        }
        for (s, now) in svc_now {
            if now.start != s.start && !self.sys.svc_set(&s.name, s.start, false) {
                log(&format!("  WARN service {}: could not change (protected?)", s.name));
            }
        }
        for tk in tasks_now {
            if let Err(e) = self.sys.task_set(tk, false) {
                log(&format!("  WARN task {tk}: {e}"));
            }
            self.tasks.borrow_mut().remove(tk.as_str());
        }
        if let Some(sp) = &t.special {
            let res = match &snap {
                Some(s) => sp.apply_snapshot(&self.ctx(), s, log).map(|_| None),
                None => sp.apply(&self.ctx(), log).map(Some),
            };
            match res {
                // ports without a snapshot return their undo data from apply: save it as soon as it exists
                Ok(Some(data)) if fresh => {
                    rec.data = data;
                    state.records.insert(t.id.clone(), rec);
                    if let Err(e) = state.save() {
                        log(&format!("  WARN could not save the undo record: {e}"));
                    }
                }
                Ok(_) => {}
                Err(e) => log(&format!("  WARN {e}")),
            }
        }
        self.clear_cache();
        self.test(t, state)
    }

    #[cfg(test)]
    pub fn undo(&self, t: &Tweak, state: &mut State, log: &mut dyn FnMut(&str)) -> bool {
        self.undo_checked(t, state, log).0
    }

    /// Restores every saved original. Returns (still active, steps that failed). The record is removed only when
    /// nothing failed; otherwise it is kept so the undo can be tried again.
    pub fn undo_checked(&self, t: &Tweak, state: &mut State, log: &mut dyn FnMut(&str)) -> (bool, Vec<String>) {
        log(&format!("Undo: {}", t.name));
        self.clear_cache();
        let rec = state.records.get(&t.id).cloned();
        let mut failed = Vec::new();
        match &rec {
            Some(rec) => {
                // a value another applied tweak also changed stays until that tweak is undone too (same as PowerShell)
                for r in &rec.reg {
                    if find_shared_reg(state, &t.id, &r.p, &r.n).is_some() {
                        log(&format!("  kept {}\\{}: another applied tweak still uses it", r.p, r.n));
                        continue;
                    }
                    if !restore_reg(self.sys, r, log) {
                        failed.push(format!("registry {}\\{}", r.p, r.n));
                    }
                }
                for s in &rec.svc {
                    if find_shared_svc(state, &t.id, &s.name).is_some() {
                        continue;
                    }
                    if !self.sys.svc_set(&s.name, s.start as u32, s.delayed != 0) {
                        log(&format!("  WARN service {}: could not restore", s.name));
                        failed.push(format!("service {}", s.name));
                    }
                }
                for tk in &rec.task {
                    if tk.enabled != 0 && find_shared_task(state, &t.id, &tk.full).is_none() {
                        if let Err(e) = self.sys.task_set(&tk.full, true) {
                            log(&format!("  WARN task {}: {e}", tk.full));
                            failed.push(format!("task {}", tk.full));
                        }
                    }
                }
            }
            None if !t.reg.is_empty() || !t.svc.is_empty() || !t.task.is_empty() => {
                log("  WARN no saved original values for this tweak (was it applied by Optimaxer?)");
            }
            None => {}
        }
        if let Some(sp) = &t.special {
            if let Err(e) = sp.undo(&self.ctx(), rec.as_ref().map(|r| &r.data).unwrap_or(&serde_json::Value::Null), log) {
                failed.push(e);
            }
        }
        if failed.is_empty() {
            if state.records.remove(&t.id).is_some() {
                if let Err(e) = state.save() {
                    log(&format!("  WARN could not save the undo record: {e}"));
                }
            }
        } else {
            for f in &failed {
                log(&format!("  WARN not restored: {f}"));
            }
            log(&format!("  WARN {} was not fully undone; its saved values are kept so you can try again.", t.name));
        }
        self.clear_cache();
        (self.test(t, state), failed)
    }
}

fn snapshot_reg(sys: &dyn System, path: &str, name: &str) -> RegRec {
    match sys.reg_get(path, name) {
        Some(d) => RegRec { p: path.into(), n: name.into(), exists: true, value: d.to_json(), ty: Some(d.type_name().into()) },
        None => RegRec { p: path.into(), n: name.into(), exists: false, ..Default::default() },
    }
}

fn restore_reg(sys: &dyn System, r: &RegRec, log: &mut dyn FnMut(&str)) -> bool {
    let res = if r.exists {
        match RegData::from_json(&r.value, r.ty.as_deref().unwrap_or("String")) {
            Some(d) => sys.reg_set(&r.p, &r.n, &d),
            None => Err("saved value is unreadable".into()),
        }
    } else {
        sys.reg_delete_value(&r.p, &r.n)
    };
    match res {
        Ok(()) => true,
        Err(e) => {
            log(&format!("  WARN restore failed {}\\{}: {e}", r.p, r.n));
            false
        }
    }
}

fn find_shared_reg(state: &State, except: &str, p: &str, n: &str) -> Option<RegRec> {
    state.records.iter().filter(|(id, _)| id.as_str() != except).flat_map(|(_, r)| r.reg.iter()).find(|r| r.p.eq_ignore_ascii_case(p) && r.n.eq_ignore_ascii_case(n)).cloned()
}

fn find_shared_svc(state: &State, except: &str, name: &str) -> Option<SvcRec> {
    state.records.iter().filter(|(id, _)| id.as_str() != except).flat_map(|(_, r)| r.svc.iter()).find(|s| s.name.eq_ignore_ascii_case(name)).cloned()
}

fn find_shared_task(state: &State, except: &str, full: &str) -> Option<TaskRec> {
    state.records.iter().filter(|(id, _)| id.as_str() != except).flat_map(|(_, r)| r.task.iter()).find(|t| t.full.eq_ignore_ascii_case(full)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;
    use crate::opti::backend::SvcState;
    use crate::opti::catalog::{RegItem, RegVal, SvcItem};

    fn state(name: &str) -> State {
        let d = std::env::temp_dir().join(format!("opti-engine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        State::load_from(d.join("state.json"), None)
    }

    fn reg_tweak(id: &str, path: &str, name: &str, v: i64) -> Tweak {
        Tweak { id: id.into(), name: id.into(), svc_tol: 1.0, reg: vec![RegItem { p: path.into(), n: name.into(), v: RegVal::Num(v), t: "DWord".into() }], ..Default::default() }
    }

    #[test]
    #[ignore = "reads the live registry; writes a comparison file to %TEMP%"]
    fn dump_live_status() {
        use crate::opti::backend::WinSystem;
        let cat = crate::opti::catalog::Catalog::load();
        let st = State::load_from(std::env::temp_dir().join("opti-nonexistent-state.json"), None);
        let sys = WinSystem;
        let eng = Engine::new(&sys, PathBuf::new());
        let map = eng.status_all(&cat.tweaks, &st);
        let mut lines: Vec<String> = map.iter().map(|(k, v)| format!("{k}={}", if *v { "True" } else { "False" })).collect();
        lines.sort();
        std::fs::write(std::env::temp_dir().join("rust-status.txt"), lines.join("
")).unwrap();
        println!("{} tweaks, {} active", lines.len(), map.values().filter(|v| **v).count());
    }

    const K: &str = r"HKCU:\Software\Test";

    #[test]
    fn apply_changes_value_verifies_and_undo_restores() {
        let sys = FakeSystem::default();
        sys.put(K, "A", RegData::Dword(5));
        let mut st = state("basic");
        let e = Engine::new(&sys, PathBuf::new());
        let t = reg_tweak("t1", K, "A", 0);
        assert!(!e.test(&t, &st));
        assert!(e.apply(&t, &mut st, &mut |_| {}));
        assert_eq!(sys.reg_get(K, "A"), Some(RegData::Dword(0)));
        assert!(st.contains("t1"));
        assert!(!e.undo(&t, &mut st, &mut |_| {}));
        assert_eq!(sys.reg_get(K, "A"), Some(RegData::Dword(5)));
        assert!(!st.contains("t1"));
    }

    #[test]
    fn value_that_did_not_exist_is_removed_on_undo() {
        let sys = FakeSystem::default();
        let mut st = state("absent");
        let e = Engine::new(&sys, PathBuf::new());
        let t = reg_tweak("t1", K, "New", 1);
        e.apply(&t, &mut st, &mut |_| {});
        assert!(sys.reg_get(K, "New").is_some());
        e.undo(&t, &mut st, &mut |_| {});
        assert_eq!(sys.reg_get(K, "New"), None);
    }

    #[test]
    fn two_tweaks_sharing_a_value_undo_in_any_order() {
        for first_a in [true, false] {
            let sys = FakeSystem::default();
            sys.put(K, "Shared", RegData::Dword(9));
            let mut st = state(&format!("shared{first_a}"));
            let e = Engine::new(&sys, PathBuf::new());
            let a = reg_tweak("a", K, "Shared", 0);
            let b = reg_tweak("b", K, "Shared", 0);
            e.apply(&a, &mut st, &mut |_| {});
            e.apply(&b, &mut st, &mut |_| {});
            // b must have reused a's saved original (9), not the 0 that a wrote
            assert_eq!(st.records["b"].reg[0].value, serde_json::json!(9));
            let (x, y) = if first_a { (&a, &b) } else { (&b, &a) };
            e.undo(x, &mut st, &mut |_| {});
            assert_eq!(sys.reg_get(K, "Shared"), Some(RegData::Dword(0)), "the first undo must keep the value the other tweak still needs");
            assert!(e.test(y, &st));
            e.undo(y, &mut st, &mut |_| {});
            assert_eq!(sys.reg_get(K, "Shared"), Some(RegData::Dword(9)), "undo order a-first={first_a}");
        }
    }

    #[test]
    fn retry_after_partial_apply_preserves_originals() {
        let sys = FakeSystem::default();
        let blocked = format!(r"{K}\Blocked");
        sys.put(K, "A", RegData::Dword(5));
        sys.put(&blocked, "B", RegData::Dword(7));
        let mut t = reg_tweak("t", K, "A", 0);
        t.reg.push(RegItem { p: blocked.clone(), n: "B".into(), v: RegVal::Num(0), t: "DWord".into() });
        let mut st = state("retry");
        let e = Engine::new(&sys, PathBuf::new());
        sys.deny_writes.borrow_mut().insert(blocked.to_lowercase());
        assert!(!e.apply(&t, &mut st, &mut |_| {}));
        assert_eq!(sys.reg_get(K, "A"), Some(RegData::Dword(0)));
        let original = st.records["t"].clone();
        let mut st = State::load_from(st.path.clone(), None);
        sys.deny_writes.borrow_mut().clear();
        assert!(e.apply(&t, &mut st, &mut |_| {}));
        assert_eq!(st.records["t"], original);
        let mut st = State::load_from(st.path.clone(), None);
        let (_, failed) = e.undo_checked(&t, &mut st, &mut |_| {});
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(sys.reg_get(K, "A"), Some(RegData::Dword(5)));
        assert_eq!(sys.reg_get(&blocked, "B"), Some(RegData::Dword(7)));
    }

    #[test]
    fn denied_write_is_logged_and_verification_fails() {
        let sys = FakeSystem::default();
        sys.put(K, "A", RegData::Dword(5));
        sys.deny_writes.borrow_mut().insert(K.to_lowercase());
        let mut st = state("denied");
        let e = Engine::new(&sys, PathBuf::new());
        let mut lines = Vec::new();
        let ok = e.apply(&reg_tweak("t", K, "A", 0), &mut st, &mut |l| lines.push(l.to_string()));
        assert!(!ok);
        assert!(lines.iter().any(|l| l.contains("denied")));
    }

    #[test]
    fn services_and_tasks_are_saved_changed_and_restored() {
        let sys = FakeSystem::default();
        sys.services.borrow_mut().insert("diagtrack".into(), SvcState { start: 2, delayed: true });
        sys.tasks.borrow_mut().insert("microsoft\\windows\\x\\task".into(), true);
        let mut st = state("svc");
        let e = Engine::new(&sys, PathBuf::new());
        let t = Tweak {
            id: "s".into(),
            name: "s".into(),
            svc_tol: 1.0,
            svc: vec![SvcItem { name: "DiagTrack".into(), start: 4 }],
            task: vec![r"Microsoft\Windows\X\Task".into()],
            ..Default::default()
        };
        assert!(e.apply(&t, &mut st, &mut |_| {}));
        assert_eq!(sys.svc_get("DiagTrack").unwrap().start, 4);
        assert_eq!(sys.task_enabled(r"Microsoft\Windows\X\Task"), Some(false));
        e.undo(&t, &mut st, &mut |_| {});
        assert_eq!(sys.svc_get("DiagTrack"), Some(SvcState { start: 2, delayed: true }));
        assert_eq!(sys.task_enabled(r"Microsoft\Windows\X\Task"), Some(true));
    }

    #[test]
    fn service_tolerance_counts_only_installed_services() {
        let sys = FakeSystem::default();
        sys.services.borrow_mut().insert("a".into(), SvcState { start: 3, delayed: false });
        let st = state("tol");
        let e = Engine::new(&sys, PathBuf::new());
        let t = Tweak { id: "s".into(), svc_tol: 1.0, svc: vec![SvcItem { name: "a".into(), start: 3 }, SvcItem { name: "missing".into(), start: 3 }], ..Default::default() };
        assert!(e.test(&t, &st));
    }

    #[test]
    fn dry_run_lists_only_real_changes_and_writes_nothing() {
        let sys = FakeSystem::default();
        sys.put(K, "Done", RegData::Dword(0));
        sys.put(K, "Todo", RegData::Dword(1));
        let e = Engine::new(&sys, PathBuf::new());
        let mut t = reg_tweak("t", K, "Done", 0);
        t.reg.push(RegItem { p: K.into(), n: "Todo".into(), v: RegVal::Num(0), t: "DWord".into() });
        let plan = e.plan(&t);
        assert_eq!(plan.len(), 1);
        assert!(plan[0].describe().contains("Todo"));
        assert_eq!(sys.reg_get(K, "Todo"), Some(RegData::Dword(1)));
    }

    #[test]
    fn reapplying_an_active_tweak_keeps_the_original_record() {
        let sys = FakeSystem::default();
        sys.put(K, "A", RegData::Dword(5));
        let mut st = state("reapply");
        let e = Engine::new(&sys, PathBuf::new());
        let t = reg_tweak("t", K, "A", 0);
        e.apply(&t, &mut st, &mut |_| {});
        e.apply(&t, &mut st, &mut |_| {});
        assert_eq!(st.records["t"].reg[0].value, serde_json::json!(5));
    }

    #[test]
    fn record_is_saved_before_the_first_change_and_kept_when_undo_fails() {
        let sys = FakeSystem::default();
        sys.put(K, "A", RegData::Dword(5));
        let mut st = state("order");
        let e = Engine::new(&sys, PathBuf::new());
        // the write is refused: the original must already be on disk
        sys.deny_writes.borrow_mut().insert(K.to_lowercase());
        e.apply(&reg_tweak("t", K, "A", 0), &mut st, &mut |_| {});
        let disk = State::load_from(st.path.clone(), None);
        assert_eq!(disk.records["t"].reg[0].value, serde_json::json!(5));
        // the restore is refused too: the record stays and the failure is reported
        let (_, failed) = e.undo_checked(&reg_tweak("t", K, "A", 0), &mut st, &mut |_| {});
        assert!(!failed.is_empty());
        assert!(st.contains("t") && State::load_from(st.path.clone(), None).contains("t"));
        sys.deny_writes.borrow_mut().clear();
        let (_, failed) = e.undo_checked(&reg_tweak("t", K, "A", 0), &mut st, &mut |_| {});
        assert!(failed.is_empty() && !st.contains("t"));
    }

    #[test]
    fn nagle_through_the_engine_restores_the_old_value() {
        let sys = FakeSystem::default();
        let a = r"HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{A}";
        sys.put(a, "IPAddress", RegData::Multi(vec!["10.0.0.2".into()]));
        sys.put(a, "TcpAckFrequency", RegData::Dword(2));
        let cat = crate::opti::catalog::Catalog::load();
        let t = cat.find("net-nagle").unwrap();
        let mut st = state("nagle");
        let e = Engine::new(&sys, PathBuf::new());
        e.apply(t, &mut st, &mut |_| {});
        assert_eq!(sys.reg_get(a, "TCPNoDelay"), Some(RegData::Dword(1)));
        let mut st = State::load_from(st.path.clone(), None);
        let (_, failed) = e.undo_checked(t, &mut st, &mut |_| {});
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(sys.reg_get(a, "TcpAckFrequency"), Some(RegData::Dword(2)));
        assert_eq!(sys.reg_get(a, "TCPNoDelay"), None);
    }

    #[test]
    fn undo_without_a_record_is_reported_not_guessed() {
        let sys = FakeSystem::default();
        let mut st = state("norec");
        let e = Engine::new(&sys, PathBuf::new());
        let mut lines = Vec::new();
        e.undo(&reg_tweak("t", K, "A", 0), &mut st, &mut |l| lines.push(l.to_string()));
        assert!(lines.iter().any(|l| l.contains("no saved original")));
    }
}
