//! Process list from `/proc`. CPU share is the change in a process's clock ticks between two refreshes, so the first
//! refresh shows no CPU values rather than a made-up average since the process started.

use super::procfs;
use super::system::{System, read_text};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessRow {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub user: String,
    pub cpu_percent: Option<f32>,
    pub rss_bytes: u64,
    pub threads: u32,
    pub state: char,
}

pub struct ProcessSampler {
    prev: HashMap<u32, u64>,
    prev_at: Option<Instant>,
    users: Vec<(u32, String)>,
    clock_ticks: u64,
    page_size: u64,
}

#[cfg(unix)]
fn clock_ticks() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 { ticks as u64 } else { 100 }
}

#[cfg(unix)]
fn page_size() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 { size as u64 } else { 4096 }
}

#[cfg(not(unix))]
fn clock_ticks() -> u64 {
    100
}

#[cfg(not(unix))]
fn page_size() -> u64 {
    4096
}

impl ProcessSampler {
    pub fn new() -> Self {
        Self::with_units(clock_ticks(), page_size())
    }

    pub fn with_units(clock_ticks: u64, page_size: u64) -> Self {
        Self {
            prev: HashMap::new(),
            prev_at: None,
            users: Vec::new(),
            clock_ticks,
            page_size,
        }
    }

    pub fn refresh(&mut self, sys: &dyn System, now: Instant) -> Vec<ProcessRow> {
        if self.users.is_empty() {
            self.users = read_text(sys, "/etc/passwd")
                .map(|t| procfs::parse_passwd(&t))
                .unwrap_or_default();
        }
        let seconds = self.prev_at.map(|t| now.duration_since(t).as_secs_f32());
        let mut next = HashMap::new();
        let mut rows = Vec::new();
        for entry in sys.list_dir("/proc") {
            let Ok(pid) = entry.parse::<u32>() else {
                continue;
            };
            // A process can exit between the listing and the read; that is not an error.
            let Some(stat) = read_text(sys, &format!("/proc/{pid}/stat"))
                .and_then(|t| procfs::parse_pid_stat(&t))
            else {
                continue;
            };
            let cpu_percent = match (self.prev.get(&pid), seconds) {
                (Some(before), Some(secs)) if secs > 0.0 => stat
                    .cpu_ticks
                    .checked_sub(*before)
                    .map(|delta| (delta as f32 / self.clock_ticks as f32 / secs * 100.0).max(0.0)),
                _ => None,
            };
            next.insert(pid, stat.cpu_ticks);
            let uid =
                read_text(sys, &format!("/proc/{pid}/status")).and_then(|t| procfs::parse_uid(&t));
            let user = uid.map_or_else(
                || "?".to_string(),
                |uid| {
                    self.users
                        .iter()
                        .find(|(id, _)| *id == uid)
                        .map_or_else(|| uid.to_string(), |(_, name)| name.clone())
                },
            );
            rows.push(ProcessRow {
                pid,
                ppid: stat.ppid,
                name: stat.name,
                user,
                cpu_percent,
                rss_bytes: stat.rss_pages.saturating_mul(self.page_size),
                threads: stat.threads,
                state: stat.state,
            });
        }
        self.prev = next;
        self.prev_at = Some(now);
        rows
    }
}

pub fn state_label(state: char) -> &'static str {
    match state {
        'R' => "Running",
        'S' => "Sleeping",
        'D' => "Waiting for I/O",
        'Z' => "Zombie",
        'T' | 't' => "Stopped",
        'I' => "Idle",
        _ => "Other",
    }
}

/// Full command line and executable path of one process, when the kernel lets this user read them.
pub fn details(sys: &dyn System, pid: u32) -> (Option<String>, Option<String>) {
    let cmdline = read_text(sys, &format!("/proc/{pid}/cmdline"))
        .map(|c| {
            c.split('\0')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|c| !c.is_empty());
    #[cfg(unix)]
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|p| p.display().to_string());
    #[cfg(not(unix))]
    let exe = None;
    (cmdline, exe)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Terminate,
    Kill,
}

impl Signal {
    pub fn label(self) -> &'static str {
        match self {
            Signal::Terminate => "Ask to close (SIGTERM)",
            Signal::Kill => "Force quit (SIGKILL)",
        }
    }
}

/// Never the init process or Optimaxer itself.
pub fn can_signal(pid: u32) -> Result<(), String> {
    if pid <= 1 {
        Err("The init process cannot be stopped.".into())
    } else if pid == std::process::id() {
        Err("That is Optimaxer itself.".into())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub fn send_signal(pid: u32, signal: Signal) -> Result<(), String> {
    can_signal(pid)?;
    let number = match signal {
        Signal::Terminate => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: kill with a validated pid and a standard signal number.
    if unsafe { libc::kill(pid as libc::pid_t, number) } == 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        Err(match error.raw_os_error() {
            Some(libc::EPERM) => {
                "Only the owner of a process (or an administrator) can stop it.".to_string()
            }
            Some(libc::ESRCH) => "That process has already exited.".to_string(),
            _ => error.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::system::fake::FakeSystem;
    use super::*;
    use std::time::Duration;

    fn proc_fs(ticks_a: u64, ticks_b: u64) -> FakeSystem {
        FakeSystem::default()
            .with_file(
                "/etc/passwd",
                "root:x:0:0::/root:/bin/sh\nme:x:1000:1000::/home/me:/bin/sh\n",
            )
            .with_file(
                "/proc/1/stat",
                &format!("1 (systemd) S 0 1 1 0 -1 0 0 0 0 0 {ticks_a} 0 0 0 20 0 1 0 1 0 100"),
            )
            .with_file("/proc/1/status", "Name:\tsystemd\nUid:\t0\t0\t0\t0\n")
            .with_file(
                "/proc/77/stat",
                &format!(
                    "77 (firefox) R 1 77 77 0 -1 0 0 0 0 0 {ticks_b} 0 0 0 20 0 30 0 1 0 2560"
                ),
            )
            .with_file(
                "/proc/77/status",
                "Name:\tfirefox\nUid:\t1000\t1000\t1000\t1000\n",
            )
            .with_file("/proc/cpuinfo", "ignored")
            .with_file("/proc/9999/stat", "not a stat line")
    }

    #[test]
    fn first_refresh_has_users_and_memory_but_no_cpu_share() {
        let mut sampler = ProcessSampler::with_units(100, 4096);
        let rows = sampler.refresh(&proc_fs(10, 200), Instant::now());
        assert_eq!(
            rows.len(),
            2,
            "cpuinfo is not a pid and a broken stat is skipped"
        );
        let fox = rows.iter().find(|r| r.pid == 77).unwrap();
        assert_eq!(fox.user, "me");
        assert_eq!(fox.rss_bytes, 2560 * 4096);
        assert_eq!(fox.threads, 30);
        assert_eq!(fox.cpu_percent, None);
        assert_eq!(rows.iter().find(|r| r.pid == 1).unwrap().user, "root");
    }

    #[test]
    fn cpu_share_is_ticks_over_time() {
        let mut sampler = ProcessSampler::with_units(100, 4096);
        let t0 = Instant::now();
        sampler.refresh(&proc_fs(10, 200), t0);
        let rows = sampler.refresh(&proc_fs(10, 250), t0 + Duration::from_secs(2));
        let fox = rows.iter().find(|r| r.pid == 77).unwrap();
        assert_eq!(
            fox.cpu_percent,
            Some(25.0),
            "50 ticks at 100 Hz over 2 s is a quarter of one core"
        );
        assert_eq!(
            rows.iter().find(|r| r.pid == 1).unwrap().cpu_percent,
            Some(0.0)
        );
    }

    #[test]
    fn a_reused_pid_with_fewer_ticks_gives_no_value() {
        let mut sampler = ProcessSampler::with_units(100, 4096);
        let t0 = Instant::now();
        sampler.refresh(&proc_fs(10, 900), t0);
        let rows = sampler.refresh(&proc_fs(10, 5), t0 + Duration::from_secs(1));
        assert_eq!(rows.iter().find(|r| r.pid == 77).unwrap().cpu_percent, None);
    }

    #[test]
    fn labels_and_signal_guards() {
        assert_eq!(state_label('Z'), "Zombie");
        assert!(can_signal(1).is_err() && can_signal(0).is_err());
        assert!(can_signal(std::process::id()).is_err());
        assert!(can_signal(std::process::id() + 1).is_ok());
    }

    #[test]
    fn command_lines_are_nul_separated() {
        let sys = FakeSystem::default().with_file("/proc/5/cmdline", "/usr/bin/foo\0--bar\0baz\0");
        assert_eq!(
            details(&sys, 5).0.as_deref(),
            Some("/usr/bin/foo --bar baz")
        );
        assert_eq!(details(&sys, 6).0, None);
    }
}
