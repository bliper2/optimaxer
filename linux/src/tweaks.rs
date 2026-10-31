//! Tweaks: files, config-line edits, packages, units and commands, each recorded for exact undo.

use crate::env::{BACKUP_DIR, Env, Pm, last_line, now_fmt, s};
use crate::sys::{pkg_install, pkg_installed, restore_unit, unit_active, unit_enabled};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ------------------------------------------------------------------------------------------------ line edits

/// Set `key = value` (or a bare flag when value is None) in a config file, remembering the original line.
#[derive(Clone, Debug)]
pub struct LineEdit {
    pub path: String,
    pub key: String,
    pub value: Option<String>,
    pub section: Option<String>,
    pub sep: String,
}

/// Collapse whitespace around `=` so `A = 1` and `A=1` compare equal.
fn norm(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.trim().chars().peekable();
    while let Some(c) = chars.next() {
        if c == '=' {
            while out.ends_with(char::is_whitespace) {
                out.pop();
            }
            out.push('=');
            while chars.peek().is_some_and(|n| n.is_whitespace()) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn key_then_eq_or_end(rest: &str, key: &str) -> bool {
    match rest.strip_prefix(key) {
        Some(after) => {
            let a = after.trim_start();
            a.is_empty() || a.starts_with('=')
        }
        None => false,
    }
}

impl LineEdit {
    pub fn new(path: &str, key: &str, value: Option<&str>, section: Option<&str>, sep: &str) -> LineEdit {
        LineEdit { path: path.into(), key: key.into(), value: value.map(Into::into), section: section.map(Into::into), sep: sep.into() }
    }

    pub fn desired(&self) -> String {
        match &self.value {
            None => self.key.clone(),
            Some(v) => format!("{}{}{}", self.key, self.sep, v),
        }
    }
    fn active(&self, line: &str) -> bool {
        key_then_eq_or_end(line.trim_start(), &self.key)
    }
    fn commented(&self, line: &str) -> bool {
        line.trim_start().strip_prefix('#').is_some_and(|r| key_then_eq_or_end(r.trim_start(), &self.key))
    }

    pub fn is_applied(&self, env: &Env) -> bool {
        let Some(text) = env.read_text(&self.path) else { return false };
        let want = norm(&self.desired());
        text.split('\n').any(|l| self.active(l) && norm(l) == want)
    }

    /// Returns (new text, original line). The original is None when a line was added.
    pub fn apply(&self, text: &str) -> (String, Option<String>) {
        let want = norm(&self.desired());
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        if let Some(i) = lines.iter().position(|l| self.active(l)) {
            let l = lines[i].clone();
            if norm(&l) == want {
                return (text.to_string(), Some(l));
            }
            lines[i] = self.desired();
            return (lines.join("\n"), Some(l));
        }
        if let Some(i) = lines.iter().position(|l| self.commented(l)) {
            let l = lines[i].clone();
            lines[i] = self.desired();
            return (lines.join("\n"), Some(l));
        }
        if let Some(sec) = &self.section {
            let header = format!("[{}]", sec.to_lowercase());
            if let Some(i) = lines.iter().position(|l| l.trim().to_lowercase() == header) {
                lines.insert(i + 1, self.desired());
                return (lines.join("\n"), None);
            }
        }
        if lines.last().is_some_and(|l| l.is_empty()) {
            let at = lines.len() - 1;
            lines.insert(at, self.desired());
        } else {
            lines.push(self.desired());
        }
        (lines.join("\n"), None)
    }

    pub fn revert(&self, text: &str, original: Option<&str>) -> String {
        let want = norm(&self.desired());
        let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
        if let Some(i) = lines.iter().position(|l| self.active(l) && norm(l) == want) {
            match original {
                None => {
                    lines.remove(i);
                }
                Some(o) => lines[i] = o.to_string(),
            }
        }
        lines.join("\n")
    }
}

// ------------------------------------------------------------------------------------------------ saved originals

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FileSnap {
    pub path: String,
    #[serde(default)]
    pub existed: bool,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EditSnap {
    pub path: String,
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default = "default_sep")]
    pub sep: String,
    #[serde(default)]
    pub orig: Option<String>,
    #[serde(default)]
    pub changed: bool,
}

fn default_sep() -> String {
    " = ".into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UnitSnap {
    pub unit: String,
    pub enabled: bool,
    pub active: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Snap {
    #[serde(default)]
    pub files: Vec<FileSnap>,
    #[serde(default)]
    pub edits: Vec<EditSnap>,
    #[serde(default)]
    pub units: Vec<UnitSnap>,
    #[serde(default)]
    pub pkgs_added: Vec<String>,
    #[serde(default)]
    pub time: String,
}

// ------------------------------------------------------------------------------------------------ tweak

#[derive(Clone, Debug)]
pub struct Tweak {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub cat: String,
    pub risk: String,
    pub tags: Vec<String>,
    pub files: Vec<(String, String)>,
    pub edits: Vec<LineEdit>,
    pub pkgs: Vec<String>,
    pub enable: Vec<String>,
    pub disable: Vec<String>,
    pub post: Vec<Vec<String>>,
    pub undo_post: Vec<Vec<String>>,
    pub cmds: Vec<Vec<String>>,
    pub undo_cmds: Vec<Vec<String>>,
    pub test_cmd: Option<Vec<String>>,
    pub needs: Vec<String>,
    pub pm: Option<Pm>,
}

impl Tweak {
    pub fn new(id: &str, name: &str, desc: &str, cat: &str) -> Tweak {
        Tweak {
            id: id.into(), name: name.into(), desc: desc.into(), cat: cat.into(), risk: "safe".into(), tags: vec![], files: vec![], edits: vec![],
            pkgs: vec![], enable: vec![], disable: vec![], post: vec![], undo_post: vec![], cmds: vec![], undo_cmds: vec![], test_cmd: None,
            needs: vec![], pm: None,
        }
    }
    pub fn risk(mut self, r: &str) -> Self {
        self.risk = r.into();
        self
    }
    pub fn tags(mut self, t: &[&str]) -> Self {
        self.tags = t.iter().map(|x| x.to_string()).collect();
        self
    }
    pub fn file(mut self, path: &str, content: impl Into<String>) -> Self {
        self.files.push((path.into(), content.into()));
        self
    }
    pub fn edit(mut self, e: LineEdit) -> Self {
        self.edits.push(e);
        self
    }
    pub fn pkgs(mut self, p: &[&str]) -> Self {
        self.pkgs = s(p);
        self
    }
    pub fn enable(mut self, u: &[&str]) -> Self {
        self.enable = s(u);
        self
    }
    pub fn disable(mut self, u: &[&str]) -> Self {
        self.disable = s(u);
        self
    }
    pub fn post(mut self, c: &[&[&str]]) -> Self {
        self.post = c.iter().map(|x| s(x)).collect();
        self
    }
    pub fn undo_post(mut self, c: &[&[&str]]) -> Self {
        self.undo_post = c.iter().map(|x| s(x)).collect();
        self
    }
    pub fn cmds(mut self, c: &[&[&str]]) -> Self {
        self.cmds = c.iter().map(|x| s(x)).collect();
        self
    }
    pub fn test_cmd(mut self, c: &[&str]) -> Self {
        self.test_cmd = Some(s(c));
        self
    }
    pub fn needs(mut self, n: &[&str]) -> Self {
        self.needs = s(n);
        self
    }
    pub fn pm(mut self, pm: Pm) -> Self {
        self.pm = Some(pm);
        self
    }

    pub fn available(&self, env: &Env) -> bool {
        if self.pm.is_some_and(|p| p != env.pm) {
            return false;
        }
        self.needs.iter().all(|n| if n.starts_with('/') { env.read_text(n).is_some() } else { env.have(n) })
    }

    pub fn is_applied(&self, env: &Env) -> bool {
        self.files.iter().all(|(p, c)| env.read_text(p).as_deref() == Some(c.as_str()))
            && self.edits.iter().all(|e| e.is_applied(env))
            && self.enable.iter().all(|u| unit_enabled(env, u))
            && self.disable.iter().all(|u| !unit_enabled(env, u))
            && self.pkgs.iter().all(|p| pkg_installed(env, p))
            && self.test_cmd.as_ref().is_none_or(|c| env.exec(c, false, false, None).0 == 0)
    }

    pub fn apply(&self, env: &Env) -> bool {
        let mut state = match env.load_state() {
            Ok(st) => st,
            Err(e) => {
                env.warn(&e);
                return false;
            }
        };
        // A failed apply or later setting drift must not replace the first saved originals on retry.
        let mut snap: Snap = state.get(&self.id).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_else(|| Snap {
            time: now_fmt("%Y-%m-%dT%H:%M:%S"),
            ..Default::default()
        });
        env.info(&format!("Apply: {}", self.name));

        for (path, _) in &self.files {
            if !snap.files.iter().any(|f| &f.path == path) {
                let prev = env.read_text(path);
                snap.files.push(FileSnap { path: path.clone(), existed: prev.is_some(), content: prev });
            }
        }
        let mut edited: Vec<(String, String)> = vec![];
        for e in &self.edits {
            let current = edited.iter().find(|(p, _)| p == &e.path).map(|(_, t)| t.clone()).or_else(|| env.read_text(&e.path));
            let Some(text) = current else {
                env.warn(&format!("  {} not found, skipped", e.path));
                continue;
            };
            let (new, orig) = e.apply(&text);
            if !snap.edits.iter().any(|d| d.path == e.path && d.key == e.key && d.section == e.section) {
                snap.edits.push(EditSnap {
                    path: e.path.clone(), key: e.key.clone(), value: e.value.clone(), section: e.section.clone(), sep: e.sep.clone(), orig,
                    changed: new != text,
                });
            }
            if new != text {
                match edited.iter_mut().find(|(p, _)| p == &e.path) {
                    Some(slot) => slot.1 = new,
                    None => edited.push((e.path.clone(), new)),
                }
            }
        }
        for u in self.enable.iter().chain(self.disable.iter()) {
            if !snap.units.iter().any(|x| &x.unit == u) {
                snap.units.push(UnitSnap { unit: u.clone(), enabled: unit_enabled(env, u), active: unit_active(env, u) });
            }
        }
        let missing: Vec<String> = self.pkgs.iter().filter(|p| !pkg_installed(env, p)).cloned().collect();
        for m in &missing {
            if !snap.pkgs_added.contains(m) {
                snap.pkgs_added.push(m.clone());
            }
        }
        state.insert(self.id.clone(), serde_json::to_value(&snap).unwrap());
        // Every original reaches disk before package, file, command or service changes begin.
        if let Err(e) = env.save_state(&state) {
            env.warn(&format!("  cannot save the undo state ({e}); nothing was changed"));
            return false;
        }

        if !missing.is_empty() {
            env.info(&format!("  installing packages: {}", missing.join(" ")));
            let (rc, out) = pkg_install(env, &missing);
            if rc != 0 {
                let l = last_line(&out);
                env.warn(&format!("  package install failed: {}", if l.is_empty() { rc.to_string() } else { l }));
                return false;
            }
        }
        let write_all = || -> std::io::Result<()> {
            for (path, content) in &self.files {
                env.write_text(path, content)?;
            }
            for (path, new) in &edited {
                if !env.dry_run {
                    std::fs::create_dir_all(env.p(BACKUP_DIR))?;
                    let base = std::path::Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
                    std::fs::copy(env.p(path), format!("{}/{}.{}.bak", env.p(BACKUP_DIR), base, now_fmt("%Y%m%d-%H%M%S")))?;
                }
                env.write_text(path, new)?;
            }
            Ok(())
        };
        if let Err(e) = write_all() {
            env.warn(&format!("  file change failed: {e}; originals kept for undo"));
            return false;
        }

        let mut commands: Vec<Vec<String>> = self.cmds.clone();
        commands.extend(self.enable.iter().map(|u| s(&["systemctl", "enable", "--now", u])));
        commands.extend(self.disable.iter().map(|u| s(&["systemctl", "disable", "--now", u])));
        commands.extend(self.post.iter().cloned());
        let mut commands_ok = true;
        for c in &commands {
            if !env.change(c) {
                commands_ok = false;
            }
        }
        let ok = commands_ok && (env.dry_run || self.is_applied(env));
        if env.dry_run {
            env.log("  dry run: nothing was changed", "DRY");
        } else {
            env.log(if ok { "  verified" } else { "  NOT verified: check the messages above" }, if ok { "OK" } else { "WARN" });
        }
        ok
    }

    pub fn undo(&self, env: &Env) -> bool {
        let mut state = match env.load_state() {
            Ok(st) => st,
            Err(e) => {
                env.warn(&e);
                return false;
            }
        };
        env.info(&format!("Undo: {}", self.name));
        let Some(snap) = state.get(&self.id).and_then(|v| serde_json::from_value::<Snap>(v.clone()).ok()) else {
            env.warn("  no saved original values for this tweak (was it applied by Optimaxer?)");
            return false;
        };
        let mut ok = true;
        for u in &snap.units {
            if !restore_unit(env, &u.unit, u.enabled, u.active) {
                ok = false;
            }
        }
        for f in &snap.files {
            let r = match (&f.existed, &f.content) {
                (true, Some(c)) => env.write_text(&f.path, c),
                _ => env.remove_file(&f.path),
            };
            if let Err(e) = r {
                env.warn(&format!("  could not restore {}: {e}", f.path));
                ok = false;
            }
        }
        for ed in snap.edits.iter().filter(|e| e.changed) {
            let Some(text) = env.read_text(&ed.path) else {
                env.warn(&format!("  cannot restore missing configuration {}", ed.path));
                ok = false;
                continue;
            };
            let le = LineEdit { path: ed.path.clone(), key: ed.key.clone(), value: ed.value.clone(), section: ed.section.clone(), sep: ed.sep.clone() };
            if let Err(e) = env.write_text(&ed.path, &le.revert(&text, ed.orig.as_deref())) {
                env.warn(&format!("  could not restore {}: {e}", ed.path));
                ok = false;
            }
        }
        let mut after = self.undo_cmds.clone();
        after.extend(self.undo_post.iter().cloned());
        after.extend(self.reload_commands());
        for c in &after {
            if !env.change(c) {
                ok = false;
            }
        }
        if !snap.pkgs_added.is_empty() {
            let kept: Vec<&String> = snap.pkgs_added.iter().filter(|p| pkg_installed(env, p)).collect();
            if !kept.is_empty() {
                env.info(&format!(
                    "  kept packages installed by this tweak: {} (remove with {} if unwanted)",
                    kept.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "),
                    env.pm.remove_hint()
                ));
            }
        }
        if ok {
            state.remove(&self.id);
            if let Err(e) = env.save_state(&state) {
                env.warn(&format!("  restored, but the undo record could not be cleared: {e}"));
            }
        }
        env.log(if ok { "  restored original values" } else { "  restore incomplete; saved values kept for retry" }, if ok { "OK" } else { "WARN" });
        ok
    }

    /// Configuration reloads that make removed files take effect.
    fn reload_commands(&self) -> Vec<Vec<String>> {
        let has = |prefix: &str| self.files.iter().any(|(p, _)| p.starts_with(prefix));
        let mut c = vec![];
        if has("/etc/sysctl.d/") {
            c.push(s(&["sysctl", "--system"]));
        }
        if has("/etc/udev/rules.d/") {
            c.push(s(&["udevadm", "control", "--reload"]));
        }
        if has("/etc/systemd/system.conf.d/") {
            c.push(s(&["systemctl", "daemon-reexec"]));
        }
        if has("/etc/systemd/journald.conf.d/") {
            c.push(s(&["systemctl", "restart", "systemd-journald"]));
        }
        c
    }
}

pub fn value_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}
