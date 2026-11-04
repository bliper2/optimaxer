//! Restarting the Windows shell (explorer.exe). Windows relaunches the shell by itself when it is killed, so the
//! normal path only closes it and waits for it to come back. Starting explorer.exe from this process is a last resort,
//! because an Optimaxer that runs as administrator would start an administrator Explorer.

use super::backend::System;
use std::time::{Duration, Instant};

/// How long to wait for Windows to bring the shell back before starting it by hand.
pub const RELAUNCH_WAIT: Duration = Duration::from_secs(10);
/// How long to wait after starting it by hand.
pub const FALLBACK_WAIT: Duration = Duration::from_secs(6);
const POLL: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    /// Explorer was not running (a different shell, or a session without one), so nothing was touched.
    NotRunning,
    Restarted,
    /// Explorer could not be closed, or did not come back.
    Failed,
}

pub fn explorer_running(sys: &dyn System) -> bool {
    let out = sys.run(
        "tasklist.exe",
        &["/FI", "IMAGENAME eq explorer.exe", "/FO", "CSV", "/NH"],
    );
    out.code == 0 && out.text.to_ascii_lowercase().contains("explorer.exe")
}

fn wait_until_running(sys: &dyn System, wait: Duration, poll: Duration) -> bool {
    let started = Instant::now();
    loop {
        if explorer_running(sys) {
            return true;
        }
        if started.elapsed() >= wait {
            return false;
        }
        std::thread::sleep(poll);
    }
}

/// Starts Explorer by hand if it is not running. Returns whether it is running afterwards.
pub fn ensure_explorer_running(
    sys: &dyn System,
    log: &mut dyn FnMut(&str),
    wait: Duration,
    poll: Duration,
) -> bool {
    if wait_until_running(sys, Duration::ZERO, poll) {
        return true;
    }
    log("Explorer did not restart by itself; starting it.");
    let out = sys.run("cmd.exe", &["/c", "start", "", "explorer.exe"]);
    if out.code != 0 {
        log(&format!(
            "WARN Could not start Explorer: {}",
            out.text.trim()
        ));
        return false;
    }
    wait_until_running(sys, wait, poll)
}

pub fn restart_explorer(
    sys: &dyn System,
    log: &mut dyn FnMut(&str),
    relaunch_wait: Duration,
    fallback_wait: Duration,
    poll: Duration,
) -> Restart {
    if !explorer_running(sys) {
        log("Explorer is not running, so it was left alone.");
        return Restart::NotRunning;
    }
    log("Restarting Explorer so interface changes show.");
    let killed = sys.run("taskkill.exe", &["/F", "/IM", "explorer.exe"]);
    if killed.code != 0 {
        log(&format!(
            "WARN Explorer could not be closed ({}): {}",
            killed.code,
            killed.text.trim()
        ));
        return Restart::Failed;
    }
    if wait_until_running(sys, relaunch_wait, poll) {
        log("Explorer is back.");
        return Restart::Restarted;
    }
    if ensure_explorer_running(sys, log, fallback_wait, poll) {
        log("Explorer is back.");
        return Restart::Restarted;
    }
    log(
        "WARN Explorer did not come back. Press Ctrl+Shift+Esc, choose Run new task and enter explorer.exe, or sign out and in.",
    );
    Restart::Failed
}

/// The restart used after tweaks, with the production timings.
pub fn restart_explorer_now(sys: &dyn System, log: &mut dyn FnMut(&str)) -> Restart {
    restart_explorer(sys, log, RELAUNCH_WAIT, FALLBACK_WAIT, POLL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    const TICK: Duration = Duration::from_millis(1);
    const RUNNING: &str = "\"explorer.exe\",\"4242\",\"Console\",\"1\",\"80,000 K\"";
    const GONE: &str = "INFO: No tasks are running which match the specified criteria.";

    fn restart(sys: &FakeSystem) -> (Restart, Vec<String>) {
        let mut lines = Vec::new();
        let result = restart_explorer(
            sys,
            &mut |l| lines.push(l.to_string()),
            Duration::from_millis(30),
            Duration::from_millis(30),
            TICK,
        );
        (result, lines)
    }

    #[test]
    fn a_shell_that_is_not_running_is_left_alone() {
        let sys = FakeSystem::default();
        sys.answer("tasklist.exe", GONE);
        let (result, _) = restart(&sys);
        assert_eq!(result, Restart::NotRunning);
        assert!(!sys.ran("taskkill.exe"));
    }

    #[test]
    fn windows_relaunching_the_shell_is_enough() {
        let sys = FakeSystem::default();
        sys.answer_sequence(
            "tasklist.exe",
            &[RUNNING.into(), GONE.into(), RUNNING.into()],
        );
        let (result, _) = restart(&sys);
        assert_eq!(result, Restart::Restarted);
        assert!(sys.ran("taskkill.exe /F /IM explorer.exe"));
        assert!(
            !sys.ran("cmd.exe"),
            "Explorer is only started by hand when Windows did not bring it back"
        );
    }

    #[test]
    fn a_shell_that_stays_down_is_started_by_hand_once() {
        let sys = FakeSystem::default();
        let mut answers = vec![RUNNING.to_string()];
        answers.extend((0..200).map(|_| GONE.to_string()));
        sys.answer_sequence("tasklist.exe", &answers);
        let (result, lines) = restart(&sys);
        assert_eq!(result, Restart::Failed);
        assert_eq!(
            sys.commands
                .borrow()
                .iter()
                .filter(|c| c.starts_with("cmd.exe"))
                .count(),
            1
        );
        assert!(lines.iter().any(|l| l.contains("did not come back")));
    }

    #[test]
    fn a_shell_started_by_hand_counts_as_restarted() {
        let sys = FakeSystem::default();
        let mut answers = vec![RUNNING.to_string()];
        answers.extend((0..40).map(|_| GONE.to_string()));
        answers.push(RUNNING.to_string());
        sys.answer_sequence("tasklist.exe", &answers);
        let mut lines = Vec::new();
        let result = restart_explorer(
            &sys,
            &mut |l| lines.push(l.to_string()),
            Duration::from_millis(5),
            Duration::from_millis(400),
            TICK,
        );
        assert_eq!(result, Restart::Restarted, "{lines:?}");
        assert!(sys.ran("cmd.exe"));
    }

    #[test]
    fn a_failed_kill_is_reported_and_nothing_else_runs() {
        let sys = FakeSystem::default();
        sys.answer("tasklist.exe", RUNNING);
        sys.fail.borrow_mut().insert("taskkill.exe".into());
        let (result, lines) = restart(&sys);
        assert_eq!(result, Restart::Failed);
        assert!(lines.iter().any(|l| l.starts_with("WARN")));
        assert!(!sys.ran("cmd.exe"));
    }

    #[test]
    fn ensure_does_nothing_when_the_shell_is_already_up() {
        let sys = FakeSystem::default();
        sys.answer("tasklist.exe", RUNNING);
        assert!(ensure_explorer_running(
            &sys,
            &mut |_| {},
            Duration::from_millis(10),
            TICK
        ));
        assert!(!sys.ran("cmd.exe"));
    }
}
