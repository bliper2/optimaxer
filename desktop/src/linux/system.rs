//! The seam between Optimaxer's Linux logic and the machine. Reads and read-only commands go through `System::read`
//! and `System::run`; every change is first built as a `Plan` that the user previews, then handed to
//! `System::execute`. `RealSystem` runs a plan with `pkexec` (or directly when the app already is root); the test
//! double in `fake` applies it to an in-memory file system, so apply and undo are checked without touching a system.

use super::sh_quote;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Write {
        path: String,
        content: String,
    },
    Remove {
        path: String,
    },
    Run {
        argv: Vec<String>,
        /// False for commands the logged-in user may run themselves (for example `nmcli` or `systemctl --user`).
        root: bool,
    },
}

impl Step {
    pub fn run(argv: &[&str]) -> Step {
        Step::Run {
            argv: argv.iter().map(|a| a.to_string()).collect(),
            root: true,
        }
    }

    pub fn run_user(argv: &[&str]) -> Step {
        Step::Run {
            argv: argv.iter().map(|a| a.to_string()).collect(),
            root: false,
        }
    }

    fn needs_root(&self) -> bool {
        match self {
            Step::Run { root, .. } => *root,
            _ => true,
        }
    }

    /// One line for the preview window.
    pub fn describe(&self) -> String {
        match self {
            Step::Write { path, content } => {
                format!("Write {path} ({} bytes)", content.len())
            }
            Step::Remove { path } => format!("Remove {path}"),
            Step::Run { argv, .. } => format!("Run {}", argv.join(" ")),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn push(&mut self, step: Step) {
        self.steps.push(step);
    }

    pub fn extend(&mut self, other: Plan) {
        self.steps.extend(other.steps);
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn needs_root(&self) -> bool {
        self.steps.iter().any(Step::needs_root)
    }

    pub fn describe(&self) -> Vec<String> {
        self.steps.iter().map(Step::describe).collect()
    }

    /// The exact `/bin/sh` script `pkexec` runs. Every step reports its own failure and the script exits non-zero if
    /// any step failed, so a half-applied plan is never reported as done.
    pub fn to_script(&self) -> String {
        let mut script = String::from("status=0\n");
        for step in &self.steps {
            match step {
                Step::Write { path, content } => {
                    let tmp = format!("{path}.optimaxer-new");
                    let (p, t) = (sh_quote(path), sh_quote(&tmp));
                    let dir = path
                        .rsplit_once('/')
                        .map_or("/", |(d, _)| if d.is_empty() { "/" } else { d });
                    script.push_str(&format!(
                        "mkdir -p {dir} && printf '%s' {content} > {t} && {{ [ -e {p} ] && chmod --reference={p} {t} 2>/dev/null || chmod 644 {t}; }} && mv -f {t} {p} || {{ echo {fail}; rm -f {t}; status=1; }}\n",
                        dir = sh_quote(dir),
                        content = sh_quote(content),
                        fail = sh_quote(&format!("FAILED: write {path}")),
                    ));
                }
                Step::Remove { path } => script.push_str(&format!(
                    "rm -f {} || {{ echo {}; status=1; }}\n",
                    sh_quote(path),
                    sh_quote(&format!("FAILED: remove {path}")),
                )),
                Step::Run { argv, .. } => {
                    let words: Vec<String> = argv.iter().map(|a| sh_quote(a)).collect();
                    script.push_str(&format!(
                        "{} || {{ echo {}; status=1; }}\n",
                        words.join(" "),
                        sh_quote(&format!("FAILED: {}", argv.join(" "))),
                    ));
                }
            }
        }
        script.push_str("exit $status\n");
        script
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmdOut {
    pub code: i32,
    /// Standard output, then standard error.
    pub text: String,
}

impl CmdOut {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

pub trait System {
    /// `Ok(None)` when the file does not exist, `Err` when it exists but cannot be read.
    fn read(&self, path: &str) -> Result<Option<String>, String>;
    fn have(&self, command: &str) -> bool;
    /// Runs a command that only reads state.
    fn run(&self, argv: &[&str]) -> CmdOut;
    /// File and folder names in a directory; empty when it is missing or unreadable.
    fn list_dir(&self, path: &str) -> Vec<String>;
    /// Used and total bytes of the file system holding `path`.
    fn disk_space(&self, path: &str) -> Option<(u64, u64)>;
    /// The user's home folder.
    fn home(&self) -> String;
    /// Writes a file the logged-in user owns (autostart entries); never elevates.
    fn write_user(&self, path: &str, content: &str) -> Result<(), String>;
    fn remove_user(&self, path: &str) -> Result<(), String>;
    /// True when changes can be made: the app runs as root or `pkexec` is installed.
    fn can_elevate(&self) -> bool;
    fn is_root(&self) -> bool;
    /// Runs the plan. Returns the output lines on success; an `Err` carries the reason and any output.
    fn execute(&self, plan: &Plan) -> Result<Vec<String>, String>;
}

pub fn read_text(sys: &dyn System, path: &str) -> Option<String> {
    sys.read(path).ok().flatten()
}

#[cfg(test)]
pub fn unit_exists(sys: &dyn System, unit: &str) -> bool {
    let out = sys.run(&[
        "systemctl",
        "list-unit-files",
        unit,
        "--no-legend",
        "--no-pager",
    ]);
    out.ok()
        && out
            .text
            .lines()
            .any(|l| l.split_whitespace().next() == Some(unit))
}

pub fn unit_enabled(sys: &dyn System, unit: &str) -> bool {
    matches!(
        sys.run(&["systemctl", "is-enabled", unit])
            .text
            .lines()
            .next()
            .map(str::trim),
        Some("enabled" | "enabled-runtime")
    )
}

pub fn unit_active(sys: &dyn System, unit: &str) -> bool {
    sys.run(&["systemctl", "is-active", unit]).ok()
}

#[cfg(unix)]
pub use real::RealSystem;

#[cfg(unix)]
mod real {
    use super::*;
    use std::process::{Command, Stdio};

    pub struct RealSystem;

    fn path_has(command: &str) -> bool {
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .chain(["/usr/sbin", "/sbin", "/usr/local/sbin"].map(Into::into))
            .any(|dir| {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(dir.join(command))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
    }

    fn output_text(out: std::process::Output) -> CmdOut {
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        CmdOut {
            code: out.status.code().unwrap_or(-1),
            text,
        }
    }

    impl System for RealSystem {
        fn read(&self, path: &str) -> Result<Option<String>, String> {
            match std::fs::read(path) {
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| format!("{path} is not valid UTF-8 text")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("Cannot read {path}: {e}")),
            }
        }

        fn have(&self, command: &str) -> bool {
            path_has(command)
        }

        fn run(&self, argv: &[&str]) -> CmdOut {
            let Some((program, args)) = argv.split_first() else {
                return CmdOut {
                    code: -1,
                    text: "empty command".into(),
                };
            };
            match Command::new(program)
                .args(args)
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .output()
            {
                Ok(out) => output_text(out),
                Err(e) => CmdOut {
                    code: -1,
                    text: format!("{program}: {e}"),
                },
            }
        }

        fn list_dir(&self, path: &str) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(path)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }

        fn disk_space(&self, path: &str) -> Option<(u64, u64)> {
            let c_path = std::ffi::CString::new(path).ok()?;
            // SAFETY: statvfs writes into the zeroed struct we own; the path is a valid NUL-terminated string.
            let stats = unsafe {
                let mut stats: libc::statvfs = std::mem::zeroed();
                if libc::statvfs(c_path.as_ptr(), &mut stats) != 0 {
                    return None;
                }
                stats
            };
            let block = stats.f_frsize as u64;
            let total = stats.f_blocks as u64 * block;
            let free = stats.f_bavail as u64 * block;
            (total > 0).then(|| (total.saturating_sub(free), total))
        }

        fn home(&self) -> String {
            std::env::var("HOME")
                .ok()
                .filter(|h| !h.is_empty())
                .unwrap_or_else(|| "/".into())
        }

        fn write_user(&self, path: &str, content: &str) -> Result<(), String> {
            let target = std::path::Path::new(path);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
            }
            crate::net::write_atomic(target, content.as_bytes())
        }

        fn remove_user(&self, path: &str) -> Result<(), String> {
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("Cannot remove {path}: {e}")),
            }
        }

        fn is_root(&self) -> bool {
            // SAFETY: geteuid has no preconditions and cannot fail.
            unsafe { libc::geteuid() == 0 }
        }

        fn can_elevate(&self) -> bool {
            self.is_root() || path_has("pkexec")
        }

        fn execute(&self, plan: &Plan) -> Result<Vec<String>, String> {
            if plan.is_empty() {
                return Ok(Vec::new());
            }
            let root_steps = plan.steps.iter().filter(|s| s.needs_root()).count();
            if root_steps != 0 && root_steps != plan.steps.len() {
                return Err("Internal error: a plan mixed administrator and user steps".into());
            }
            if root_steps == 0 {
                let mut lines = Vec::new();
                let mut failed = false;
                for step in &plan.steps {
                    if let Step::Run { argv, .. } = step {
                        let words: Vec<&str> = argv.iter().map(String::as_str).collect();
                        let out = self.run(&words);
                        lines.extend(out.text.lines().map(str::to_string));
                        if !out.ok() {
                            failed = true;
                            lines.push(format!("FAILED: {}", argv.join(" ")));
                        }
                    }
                }
                return if failed {
                    Err(lines.join("\n"))
                } else {
                    Ok(lines)
                };
            }
            let script = plan.to_script();
            let mut command = if self.is_root() {
                Command::new("/bin/sh")
            } else if path_has("pkexec") {
                let mut c = Command::new("pkexec");
                c.arg("/bin/sh");
                c
            } else {
                return Err("Administrator rights are needed and pkexec is not installed. Install polkit, or start Optimaxer as root.".into());
            };
            let out = command
                .args(["-c", &script])
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("Could not start the privileged helper: {e}"))?;
            let result = output_text(out);
            let lines: Vec<String> = result.text.lines().map(str::to_string).collect();
            match result.code {
                0 => Ok(lines),
                126 => Err(
                    "Authentication was cancelled or not authorized. Nothing was changed.".into(),
                ),
                127 if lines.is_empty() => {
                    Err("pkexec could not authenticate. Nothing was changed.".into())
                }
                _ => Err(lines.join("\n")),
            }
        }
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};

    /// In-memory machine: files, unit state and a log of every command, with failures that can be injected.
    #[derive(Default)]
    pub struct FakeSystem {
        pub files: RefCell<BTreeMap<String, String>>,
        pub unreadable: RefCell<BTreeSet<String>>,
        pub units: RefCell<BTreeSet<String>>,
        pub enabled: RefCell<BTreeSet<String>>,
        pub active: RefCell<BTreeSet<String>>,
        pub tools: RefCell<BTreeSet<String>>,
        pub canned: RefCell<BTreeMap<String, CmdOut>>,
        pub executed: RefCell<Vec<Step>>,
        pub fail_prefix: RefCell<Option<String>>,
        pub root: bool,
        pub refuse: RefCell<bool>,
        pub disk: std::cell::Cell<Option<(u64, u64)>>,
    }

    impl FakeSystem {
        pub fn with_file(self, path: &str, content: &str) -> Self {
            self.files.borrow_mut().insert(path.into(), content.into());
            self
        }

        pub fn with_unit(self, unit: &str, enabled: bool, active: bool) -> Self {
            self.units.borrow_mut().insert(unit.into());
            if enabled {
                self.enabled.borrow_mut().insert(unit.into());
            }
            if active {
                self.active.borrow_mut().insert(unit.into());
            }
            self
        }

        pub fn with_tool(self, tool: &str) -> Self {
            self.tools.borrow_mut().insert(tool.into());
            self
        }

        pub fn file(&self, path: &str) -> Option<String> {
            self.files.borrow().get(path).cloned()
        }

        fn systemctl(&self, argv: &[String]) -> bool {
            let rest: Vec<&str> = argv[1..]
                .iter()
                .map(String::as_str)
                .filter(|a| !a.starts_with("--"))
                .collect();
            let now = argv.iter().any(|a| a == "--now");
            let (Some(verb), Some(unit)) = (rest.first(), rest.get(1)) else {
                return true;
            };
            let unit = unit.to_string();
            match *verb {
                "enable" => {
                    self.enabled.borrow_mut().insert(unit.clone());
                    if now {
                        self.active.borrow_mut().insert(unit);
                    }
                }
                "disable" => {
                    self.enabled.borrow_mut().remove(&unit);
                    if now {
                        self.active.borrow_mut().remove(&unit);
                    }
                }
                "start" => {
                    self.active.borrow_mut().insert(unit);
                }
                "stop" => {
                    self.active.borrow_mut().remove(&unit);
                }
                _ => {}
            }
            true
        }
    }

    impl System for FakeSystem {
        fn read(&self, path: &str) -> Result<Option<String>, String> {
            if self.unreadable.borrow().contains(path) {
                return Err(format!("Cannot read {path}: permission denied"));
            }
            Ok(self.files.borrow().get(path).cloned())
        }

        fn have(&self, command: &str) -> bool {
            self.tools.borrow().contains(command)
        }

        fn run(&self, argv: &[&str]) -> CmdOut {
            let key = argv.join(" ");
            if let Some(out) = self.canned.borrow().get(&key) {
                return out.clone();
            }
            match argv {
                ["systemctl", "is-enabled", unit] => {
                    let on = self.enabled.borrow().contains(*unit);
                    CmdOut {
                        code: i32::from(!on),
                        text: (if on { "enabled\n" } else { "disabled\n" }).into(),
                    }
                }
                ["systemctl", "is-active", unit] => {
                    let on = self.active.borrow().contains(*unit);
                    CmdOut {
                        code: if on { 0 } else { 3 },
                        text: (if on { "active\n" } else { "inactive\n" }).into(),
                    }
                }
                ["systemctl", "list-unit-files", unit, ..] if !unit.starts_with("--") => {
                    if self.units.borrow().contains(*unit) {
                        CmdOut {
                            code: 0,
                            text: format!("{unit} enabled enabled\n"),
                        }
                    } else {
                        CmdOut {
                            code: 0,
                            text: String::new(),
                        }
                    }
                }
                _ => CmdOut {
                    code: 1,
                    text: "not available in the fake system".into(),
                },
            }
        }

        fn list_dir(&self, path: &str) -> Vec<String> {
            let prefix = format!("{}/", path.trim_end_matches('/'));
            let mut names: Vec<String> = self
                .files
                .borrow()
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
                .collect();
            names.sort();
            names.dedup();
            names
        }

        fn disk_space(&self, _path: &str) -> Option<(u64, u64)> {
            self.disk.get()
        }

        fn home(&self) -> String {
            "/home/tester".into()
        }

        fn write_user(&self, path: &str, content: &str) -> Result<(), String> {
            self.files.borrow_mut().insert(path.into(), content.into());
            Ok(())
        }

        fn remove_user(&self, path: &str) -> Result<(), String> {
            self.files.borrow_mut().remove(path);
            Ok(())
        }

        fn can_elevate(&self) -> bool {
            !*self.refuse.borrow()
        }

        fn is_root(&self) -> bool {
            self.root
        }

        fn execute(&self, plan: &Plan) -> Result<Vec<String>, String> {
            if *self.refuse.borrow() {
                return Err(
                    "Authentication was cancelled or not authorized. Nothing was changed.".into(),
                );
            }
            let mut failures = Vec::new();
            for step in &plan.steps {
                self.executed.borrow_mut().push(step.clone());
                match step {
                    Step::Write { path, content } => {
                        self.files
                            .borrow_mut()
                            .insert(path.clone(), content.clone());
                    }
                    Step::Remove { path } => {
                        self.files.borrow_mut().remove(path);
                    }
                    Step::Run { argv, .. } => {
                        let line = argv.join(" ");
                        if self
                            .fail_prefix
                            .borrow()
                            .as_ref()
                            .is_some_and(|p| line.starts_with(p.as_str()))
                        {
                            failures.push(format!("FAILED: {line}"));
                        } else if argv[0] == "systemctl" {
                            self.systemctl(argv);
                        }
                    }
                }
            }
            if failures.is_empty() {
                Ok(Vec::new())
            } else {
                Err(failures.join("\n"))
            }
        }
    }

    #[test]
    fn script_quotes_content_and_reports_failures() {
        let mut plan = Plan::default();
        plan.push(Step::Write {
            path: "/etc/x.d/a b.conf".into(),
            content: "it's = 1\n".into(),
        });
        plan.push(Step::Remove {
            path: "/etc/old.conf".into(),
        });
        plan.push(Step::run(&["sysctl", "--system"]));
        let script = plan.to_script();
        assert!(script.starts_with("status=0\n") && script.ends_with("exit $status\n"));
        assert!(script.contains("mkdir -p '/etc/x.d'"));
        assert!(script.contains(r"printf '%s' 'it'\''s = 1"));
        assert!(script.contains("mv -f '/etc/x.d/a b.conf.optimaxer-new' '/etc/x.d/a b.conf'"));
        assert!(script.contains("rm -f '/etc/old.conf' ||"));
        assert!(
            script.contains("'sysctl' '--system' || { echo 'FAILED: sysctl --system'; status=1; }")
        );
        assert_eq!(plan.describe()[0], "Write /etc/x.d/a b.conf (9 bytes)");
    }

    #[test]
    fn plans_know_whether_they_need_administrator_rights() {
        let mut user = Plan::default();
        user.push(Step::run_user(&["nmcli", "connection", "show"]));
        assert!(!user.needs_root());
        user.push(Step::Remove {
            path: "/etc/x".into(),
        });
        assert!(user.needs_root());
        assert!(!Plan::default().needs_root());
    }

    #[test]
    fn unit_helpers_read_the_fake_state() {
        let sys = FakeSystem::default().with_unit("fstrim.timer", true, false);
        assert!(unit_exists(&sys, "fstrim.timer"));
        assert!(!unit_exists(&sys, "nope.service"));
        assert!(unit_enabled(&sys, "fstrim.timer"));
        assert!(!unit_active(&sys, "fstrim.timer"));
    }
}

/// These talk to the machine running the tests, so they exist on Unix only. They only read.
#[cfg(all(test, unix))]
mod live {
    use super::*;

    #[test]
    fn the_real_system_reads_files_lists_folders_and_runs_commands() {
        let sys = RealSystem;
        assert_eq!(sys.read("/definitely/not/here").unwrap(), None);
        assert!(sys.read("/proc/uptime").unwrap().is_some());
        assert!(
            sys.list_dir("/proc")
                .iter()
                .any(|n| *n == std::process::id().to_string())
        );
        assert!(sys.list_dir("/definitely/not/here").is_empty());
        assert!(
            sys.disk_space("/")
                .is_some_and(|(used, total)| total > 0 && used <= total)
        );
        assert!(sys.have("sh"));
        assert!(!sys.have("definitely-not-a-command"));
        assert!(sys.run(&["sh", "-c", "echo hi"]).text.contains("hi"));
        assert!(!sys.run(&["definitely-not-a-command"]).ok());
    }

    #[test]
    fn a_user_only_plan_runs_directly_and_reports_failures() {
        let sys = RealSystem;
        let mut ok = Plan::default();
        ok.push(Step::run_user(&["sh", "-c", "echo done"]));
        assert_eq!(sys.execute(&ok).unwrap(), vec!["done".to_string()]);
        let mut bad = Plan::default();
        bad.push(Step::run_user(&["sh", "-c", "exit 3"]));
        assert!(sys.execute(&bad).unwrap_err().contains("FAILED"));
    }

    #[test]
    fn the_script_a_plan_generates_really_works_in_sh() {
        let dir = std::env::temp_dir().join(format!("optimaxer-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("sub dir/a b.conf");
        let mut plan = Plan::default();
        plan.push(Step::Write {
            path: file.display().to_string(),
            content: "it's = 1\nline two\n".into(),
        });
        plan.push(Step::Write {
            path: dir.join("gone.conf").display().to_string(),
            content: "x".into(),
        });
        plan.push(Step::Remove {
            path: dir.join("gone.conf").display().to_string(),
        });
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &plan.to_script()])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "it's = 1\nline two\n"
        );
        assert!(!dir.join("gone.conf").exists());
        let mut failing = Plan::default();
        failing.push(Step::run(&["sh", "-c", "exit 1"]));
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &failing.to_script()])
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("FAILED"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(all(test, unix))]
mod live_proc {
    use super::super::{
        processes::ProcessSampler,
        sysmon::{Sampler, TelemetryConfig},
    };
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn telemetry_and_processes_read_the_live_proc_tree() {
        let sys = RealSystem;
        let mut sampler = Sampler::new(TelemetryConfig::new(1, 60));
        let t0 = Instant::now();
        sampler.sample(&sys, t0);
        std::thread::sleep(Duration::from_millis(200));
        sampler.sample(&sys, Instant::now());
        let snap = &sampler.snapshot;
        assert!(
            snap.mem.is_some_and(|m| m.total > 0),
            "/proc/meminfo must be readable"
        );
        assert!(snap.cpu_percent.is_some_and(|c| (0.0..=100.0).contains(&c)));
        assert!(snap.uptime.is_some());
        let mut procs = ProcessSampler::new();
        let rows = procs.refresh(&sys, Instant::now());
        assert!(
            rows.iter().any(|r| r.pid == std::process::id()),
            "this test process must be listed"
        );
    }
}
