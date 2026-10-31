//! Execution context: command runner, file access under an optional test root, undo state and log.

use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO: &str = "bliper2/optimaxer";
pub const STATE_FILE: &str = "/var/lib/optimaxer/state.json";
pub const BACKUP_DIR: &str = "/var/lib/optimaxer/backups";
pub const LOG_FILE: &str = "/var/log/optimaxer.log";

/// Test hook: (command, run as user, is a write, stdin) -> (exit code, combined output).
pub type Runner = Arc<dyn Fn(&[String], Option<&str>, bool, Option<&str>) -> (i32, String) + Send + Sync>;
/// The GUI sets this to receive log lines: (line, level).
pub type LogHook = Arc<dyn Fn(&str, &str) + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pm {
    Pacman,
    Dnf,
}

impl Pm {
    pub fn remove_hint(self) -> &'static str {
        match self {
            Pm::Pacman => "pacman -Rns",
            Pm::Dnf => "dnf remove",
        }
    }
    pub fn update_label(self) -> &'static str {
        match self {
            Pm::Pacman => "pacman -Syu",
            Pm::Dnf => "dnf upgrade --refresh",
        }
    }
    pub fn mirror_label(self) -> &'static str {
        match self {
            Pm::Pacman => "Rank mirrors (Manjaro fasttrack)",
            Pm::Dnf => "Refresh package metadata (dnf makecache)",
        }
    }
    pub fn conf_label(self) -> &'static str {
        match self {
            Pm::Pacman => "Find .pacnew / .pacsave files",
            Pm::Dnf => "Find .rpmnew / .rpmsave files",
        }
    }
    pub fn conf_patterns(self) -> &'static [&'static str] {
        match self {
            Pm::Pacman => &["*.pacnew", "*.pacsave"],
            Pm::Dnf => &["*.rpmnew", "*.rpmsave"],
        }
    }
    pub fn install_cmd(self, pkg: &str) -> String {
        match self {
            Pm::Pacman => format!("sudo pacman -S {pkg}"),
            Pm::Dnf => format!("sudo dnf install {pkg}"),
        }
    }
}

#[derive(Clone)]
pub struct Env {
    /// Prefix for every path; tests use a temp dir. Empty on a real system.
    pub root: String,
    pub dry_run: bool,
    pub assume_yes: bool,
    pub quiet: bool,
    pub user: Option<String>,
    pub home: Option<String>,
    pub runner: Option<Runner>,
    pub log_hook: Option<LogHook>,
    pub pm: Pm,
}

pub fn now_fmt(fmt: &str) -> String {
    use std::ffi::CString;
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 64];
        let f = CString::new(fmt).unwrap();
        let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), f.as_ptr(), &tm);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

pub fn fmt_size(mut n: f64) -> String {
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if n < 1024.0 || unit == "TB" {
            return if unit == "B" { format!("{n:.0} {unit}") } else { format!("{n:.1} {unit}") };
        }
        n /= 1024.0;
    }
    format!("{n:.1} TB")
}

pub fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

pub fn parse_os_release(text: Option<&str>) -> Map<String, Value> {
    let mut out = Map::new();
    for line in text.unwrap_or("").lines() {
        if let Some((k, v)) = line.split_once('=')
            && !line.starts_with('#') {
                out.insert(k.trim().to_string(), Value::String(v.trim().trim_matches('"').to_string()));
            }
    }
    out
}

impl Env {
    pub fn new(pm: Pm) -> Env {
        Env { root: String::new(), dry_run: false, assume_yes: false, quiet: false, user: None, home: None, runner: None, log_hook: None, pm }
    }

    /// Map an absolute system path under the optional test root.
    pub fn p(&self, path: &str) -> String {
        if self.root.is_empty() { path.to_string() } else { format!("{}/{}", self.root.trim_end_matches('/'), path.trim_start_matches('/')) }
    }

    pub fn log(&self, msg: &str, level: &str) {
        let line = format!("[{}] {:<5} {}", now_fmt("%H:%M:%S"), level, msg);
        if let Some(h) = &self.log_hook {
            h(&line, level);
        } else if !self.quiet {
            println!("{line}");
        }
        if !self.dry_run && self.root.is_empty()
            && let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
                let _ = writeln!(f, "{line}");
            }
    }
    pub fn info(&self, msg: &str) {
        self.log(msg, "INFO");
    }
    pub fn warn(&self, msg: &str) {
        self.log(msg, "WARN");
    }
    pub fn ok(&self, msg: &str) {
        self.log(msg, "OK");
    }

    pub fn real_user(&self) -> String {
        if let Some(u) = &self.user {
            return u.clone();
        }
        std::env::var("SUDO_USER").ok().filter(|s| !s.is_empty()).or_else(|| std::env::var("USER").ok().filter(|s| !s.is_empty())).unwrap_or_else(|| "root".into())
    }

    /// (uid, gid, home) of a user from /etc/passwd.
    pub fn passwd(&self, user: &str) -> Option<(u32, u32, String)> {
        let text = fs::read_to_string("/etc/passwd").ok()?;
        for line in text.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() >= 6 && f[0] == user {
                return Some((f[2].parse().ok()?, f[3].parse().ok()?, f[5].to_string()));
            }
        }
        None
    }

    pub fn real_home(&self) -> String {
        if let Some(h) = &self.home {
            return h.clone();
        }
        if let Some((_, _, h)) = self.passwd(&self.real_user()) {
            return h;
        }
        std::env::var("HOME").unwrap_or_else(|_| "/root".into())
    }

    /// Give a file or directory back to the real user when running as root through sudo/pkexec.
    pub fn chown_user(&self, path: &str) {
        if self.dry_run || !self.root.is_empty() || !is_root() {
            return;
        }
        if let Some((uid, gid, _)) = self.passwd(&self.real_user()) {
            let _ = std::os::unix::fs::chown(path, Some(uid), Some(gid));
        }
    }

    /// Run a command, returning (exit code, combined output). Writes are skipped in dry-run mode.
    pub fn exec(&self, cmd: &[String], write: bool, as_user: bool, input: Option<&str>) -> (i32, String) {
        if cmd.is_empty() {
            return (127, "empty command".into());
        }
        if write && self.dry_run {
            self.log(&format!("[dry-run] {}", cmd.join(" ")), "DRY");
            return (0, String::new());
        }
        if let Some(r) = &self.runner {
            let u = self.real_user();
            return r(cmd, if as_user { Some(u.as_str()) } else { None }, write, input);
        }
        let mut argv: Vec<String> = cmd.to_vec();
        if as_user && is_root() && self.real_user() != "root" {
            let mut v = vec!["sudo".to_string(), "-u".into(), self.real_user(), "-H".into()];
            v.extend(argv);
            argv = v;
        }
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
        match c.spawn() {
            Ok(mut child) => {
                if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
                    let _ = stdin.write_all(text.as_bytes());
                }
                match child.wait_with_output() {
                    Ok(o) => {
                        let mut out = String::from_utf8_lossy(&o.stdout).into_owned();
                        out.push_str(&String::from_utf8_lossy(&o.stderr));
                        (o.status.code().unwrap_or(-1), out)
                    }
                    Err(e) => (1, e.to_string()),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (127, format!("{}: command not found", argv[0])),
            Err(e) => (126, format!("{}: {e}", argv[0])),
        }
    }

    /// Read-only command.
    pub fn run(&self, cmd: &[&str]) -> (i32, String) {
        self.exec(&s(cmd), false, false, None)
    }
    /// Command that changes the system.
    pub fn run_w(&self, cmd: &[&str]) -> (i32, String) {
        self.exec(&s(cmd), true, false, None)
    }
    pub fn run_user_w(&self, cmd: &[&str]) -> (i32, String) {
        self.exec(&s(cmd), true, true, None)
    }

    pub fn have(&self, cmd: &str) -> bool {
        if let Some(r) = &self.runner {
            return r(&s(&["which", cmd]), None, false, None).0 == 0;
        }
        if cmd.contains('/') {
            return Path::new(cmd).exists();
        }
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin".into());
        path.split(':').any(|d| {
            let f = Path::new(d).join(cmd);
            f.is_file() && fs::metadata(&f).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        })
    }

    /// Run a changing command; log and return false on failure.
    pub fn change(&self, cmd: &[String]) -> bool {
        let (rc, out) = self.exec(cmd, true, false, None);
        if rc != 0 {
            let tail: String = out.trim().chars().rev().take(160).collect::<Vec<_>>().into_iter().rev().collect();
            self.warn(&format!("Command failed ({}, exit {rc}): {tail}", cmd.join(" ")));
        }
        rc == 0
    }
    pub fn change_s(&self, cmd: &[&str]) -> bool {
        self.change(&s(cmd))
    }

    // ---------------------------------------------------------------- files & state

    pub fn read_text(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.p(path)).ok()
    }

    pub fn write_text(&self, path: &str, content: &str) -> std::io::Result<()> {
        if self.dry_run {
            self.log(&format!("[dry-run] write {path} ({} bytes)", content.len()), "DRY");
            return Ok(());
        }
        let full = self.p(path);
        if let Some(dir) = Path::new(&full).parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(full, content)
    }

    pub fn remove_file(&self, path: &str) -> std::io::Result<()> {
        if self.dry_run {
            self.log(&format!("[dry-run] remove {path}"), "DRY");
            return Ok(());
        }
        match fs::remove_file(self.p(path)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn load_state(&self) -> Result<Map<String, Value>, String> {
        let Some(raw) = self.read_text(STATE_FILE) else { return Ok(Map::new()) };
        let v: Value = serde_json::from_str(&raw).map_err(|_| format!("Undo state is unreadable at {STATE_FILE}; repair it before making changes"))?;
        match v {
            Value::Object(m) if m.values().all(|r| r.is_object()) => Ok(m),
            _ => Err(format!("Undo state has an invalid format at {STATE_FILE}; no changes made")),
        }
    }

    /// Atomic replace: the prior complete record survives a failed write.
    pub fn save_state(&self, state: &Map<String, Value>) -> std::io::Result<()> {
        if self.dry_run {
            return Ok(());
        }
        let full = self.p(STATE_FILE);
        let dir = Path::new(&full).parent().unwrap().to_path_buf();
        fs::create_dir_all(&dir)?;
        let tmp = dir.join(format!(".optimaxer-state-{}.tmp", std::process::id()));
        let result = (|| {
            let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode_600().open(&tmp)?;
            f.write_all(serde_json::to_string_pretty(state).map_err(std::io::Error::other)?.as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, &full)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    pub fn distro(&self) -> Map<String, Value> {
        parse_os_release(self.read_text("/etc/os-release").as_deref())
    }

    pub fn dir_size(&self, path: &str) -> u64 {
        fn walk(p: &Path) -> u64 {
            let Ok(md) = fs::symlink_metadata(p) else { return 0 };
            if !md.is_dir() {
                return md.len();
            }
            let Ok(rd) = fs::read_dir(p) else { return 0 };
            rd.flatten().map(|e| walk(&e.path())).sum()
        }
        let full = self.p(path);
        let p = Path::new(&full);
        match fs::symlink_metadata(p) {
            Ok(md) if md.is_file() => md.len(),
            Ok(md) if md.is_dir() => walk(p),
            _ => 0,
        }
    }
}

trait Mode600 {
    fn mode_600(&mut self) -> &mut Self;
}
impl Mode600 for fs::OpenOptions {
    fn mode_600(&mut self) -> &mut Self {
        std::os::unix::fs::OpenOptionsExt::mode(self, 0o600)
    }
}

fn id_set(d: &Map<String, Value>) -> Vec<String> {
    let mut ids = vec![d.get("ID").and_then(|v| v.as_str()).unwrap_or("").to_string()];
    ids.extend(d.get("ID_LIKE").and_then(|v| v.as_str()).unwrap_or("").split_whitespace().map(String::from));
    ids
}

/// Which package manager this machine uses, or None when the distribution is unsupported.
pub fn detect_pm(env: &Env) -> Option<Pm> {
    let ids = id_set(&env.distro());
    let any = |set: &[&str]| ids.iter().any(|i| set.contains(&i.as_str()));
    if any(&["manjaro", "arch", "endeavouros", "garuda", "cachyos", "artix"]) {
        Some(Pm::Pacman)
    } else if any(&["fedora", "rhel", "centos", "rocky", "almalinux", "nobara"]) {
        Some(Pm::Dnf)
    } else {
        None
    }
}

pub fn tail(out: &str, n: usize) -> String {
    let t = out.trim();
    let chars: Vec<char> = t.chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

pub fn last_line(out: &str) -> String {
    out.trim().lines().last().unwrap_or("").to_string()
}
