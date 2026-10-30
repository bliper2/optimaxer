//! Startup page: Run-key entries and Startup-folder shortcuts, enabled or disabled through the same
//! StartupApproved registry values Task Manager uses, so nothing is uninstalled or deleted.
//! Third-party logon and boot tasks use Task Scheduler's enable state.

use super::backend::{RegData, System};
use serde::Deserialize;
use std::path::PathBuf;

const APR: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub name: String,
    pub cmd: String,
    pub scope: &'static str,
    /// StartupApproved key that holds this item's on/off flag.
    pub approved: String,
    pub enabled: bool,
    /// Scheduled-task folder when this item is a third-party logon/boot task.
    pub task_path: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub items: Vec<Item>,
    pub warning: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScheduledTask {
    name: String,
    task_path: String,
    command: String,
    enabled: bool,
}

fn enabled_flag(sys: &dyn System, approved: &str, name: &str) -> bool {
    match sys.reg_get(approved, name) {
        Some(RegData::Bin(b)) => !b.first().is_some_and(|f| f & 1 == 1),
        _ => true,
    }
}

const TASK_QUERY: &str = r#"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$ErrorActionPreference = 'Stop'
try {
    if (-not (Get-Command Get-ScheduledTask -ErrorAction SilentlyContinue)) { throw 'ScheduledTasks module is unavailable' }
    $tasks = @(Get-ScheduledTask | Where-Object {
        $_.TaskPath -notlike '\Microsoft\*' -and
        @($_.Triggers | Where-Object { $_.CimClass.CimClassName -in @('MSFT_TaskLogonTrigger', 'MSFT_TaskBootTrigger') }).Count -gt 0
    } | ForEach-Object {
        $command = @($_.Actions | ForEach-Object { ("$($_.Execute) $($_.Arguments)").Trim() }) -join '; '
        [pscustomobject]@{ name = $_.TaskName; taskPath = $_.TaskPath; command = $command; enabled = ($_.State -ne 'Disabled') }
    })
    [Console]::Out.Write((ConvertTo-Json -InputObject $tasks -Depth 3 -Compress))
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
"#;

pub fn list(sys: &dyn System) -> Listing {
    let defs = [
        ("Registry (user)", r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run".to_string(), format!(r"HKCU:\{APR}\Run")),
        ("Registry (machine)", r"HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Run".to_string(), format!(r"HKLM:\{APR}\Run")),
        ("Registry (32-bit)", r"HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run".to_string(), format!(r"HKLM:\{APR}\Run32")),
    ];
    let mut out = Vec::new();
    for (scope, path, approved) in defs {
        for name in sys.reg_value_names(&path) {
            let cmd = match sys.reg_get(&path, &name) {
                Some(RegData::Str(s) | RegData::Expand(s)) => s,
                _ => String::new(),
            };
            // The unnamed default value and empty commands do not launch startup programs.
            if name.trim().is_empty() || cmd.trim().is_empty() { continue; }
            let enabled = enabled_flag(sys, &approved, &name);
            out.push(Item { name, cmd, scope, approved: approved.clone(), enabled, task_path: None });
        }
    }
    let folders: [(&str, Option<PathBuf>, String); 2] = [
        ("Startup folder (user)", std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup")), format!(r"HKCU:\{APR}\StartupFolder")),
        ("Startup folder (machine)", std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\StartUp")), format!(r"HKLM:\{APR}\StartupFolder")),
    ];
    for (scope, dir, approved) in folders {
        let Some(dir) = dir else { continue };
        for e in std::fs::read_dir(&dir).into_iter().flatten().filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case("desktop.ini") || !e.path().is_file() {
                continue;
            }
            let enabled = enabled_flag(sys, &approved, &name);
            out.push(Item { name, cmd: e.path().to_string_lossy().to_string(), scope, approved: approved.clone(), enabled, task_path: None });
        }
    }
    let result = sys.run("powershell.exe", &["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", TASK_QUERY]);
    let warning = if result.code != 0 {
        Some(format!("Scheduled tasks could not be read: {}", result.text.trim().lines().last().unwrap_or("PowerShell query failed")))
    } else {
        match serde_json::from_str::<Vec<ScheduledTask>>(&result.text) {
            Ok(tasks) => {
                let mut seen = std::collections::HashSet::new();
                for task in tasks {
                    let full = format!("{}\\{}", task.task_path.trim_end_matches('\\'), task.name);
                    if task.name.trim().is_empty() || !task.task_path.starts_with('\\') || task.task_path.to_lowercase().starts_with("\\microsoft\\") || !seen.insert(full.to_lowercase()) { continue; }
                    out.push(Item { name: task.name, cmd: task.command, scope: "Scheduled task (logon/boot)", approved: String::new(), enabled: task.enabled, task_path: Some(task.task_path) });
                }
                None
            }
            Err(error) => Some(format!("Scheduled task results could not be parsed: {error}")),
        }
    };
    Listing { items: out, warning }
}

/// The 12-byte StartupApproved value: first byte 2 = enabled, 3 = disabled, then the disable time as a FILETIME.
pub fn approved_bytes(enable: bool, unix_secs: u64) -> Vec<u8> {
    let mut b = vec![0u8; 12];
    if enable {
        b[0] = 2;
    } else {
        b[0] = 3;
        let filetime = (unix_secs + 11_644_473_600) * 10_000_000;
        b[4..12].copy_from_slice(&filetime.to_le_bytes());
    }
    b
}

pub fn set_enabled(sys: &dyn System, item: &Item, enable: bool) -> Result<(), String> {
    if let Some(task_path) = &item.task_path {
        let full = format!("{}\\{}", task_path.trim_end_matches('\\'), item.name);
        return sys.task_set(&full, enable);
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    sys.reg_set(&item.approved, &item.name, &RegData::Bin(approved_bytes(enable, now)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    fn approved_bytes_match_task_manager_format() {
        assert_eq!(approved_bytes(true, 0), vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let d = approved_bytes(false, 0);
        assert_eq!(d[0], 3);
        assert_eq!(u64::from_le_bytes(d[4..12].try_into().unwrap()), 116_444_736_000_000_000);
    }

    #[test]
    fn run_key_entries_report_their_enabled_state() {
        let sys = FakeSystem::default();
        sys.answer("powershell.exe", "[]");
        let run = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run";
        sys.put(run, "Alpha", RegData::Str(r"C:\a.exe".into()));
        sys.put(run, "Beta", RegData::Str(r"C:\b.exe".into()));
        sys.put(&format!(r"HKCU:\{APR}\Run"), "Beta", RegData::Bin(approved_bytes(false, 1)));
        let items: Vec<_> = list(&sys).items.into_iter().filter(|i| i.scope == "Registry (user)").collect();
        let by = |n: &str| items.iter().find(|i| i.name.eq_ignore_ascii_case(n)).unwrap();
        assert!(by("alpha").enabled);
        assert!(!by("beta").enabled);
    }

    #[test]
    fn toggling_writes_the_approved_flag_and_nothing_else() {
        let sys = FakeSystem::default();
        sys.answer("powershell.exe", "[]");
        let run = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run";
        sys.put(run, "Alpha", RegData::Str(r"C:\a.exe".into()));
        let item = list(&sys).items.into_iter().find(|i| i.scope == "Registry (user)").unwrap();
        set_enabled(&sys, &item, false).unwrap();
        assert!(!list(&sys).items.into_iter().find(|i| i.scope == "Registry (user)").unwrap().enabled);
        assert!(sys.reg_get(run, "alpha").is_some(), "the Run entry itself must stay");
        set_enabled(&sys, &item, true).unwrap();
        assert!(list(&sys).items.into_iter().find(|i| i.scope == "Registry (user)").unwrap().enabled);
    }

    #[test]
    fn unnamed_defaults_empty_commands_and_non_strings_are_not_startup_items() {
        let sys = FakeSystem::default();
        sys.answer("powershell.exe", "[]");
        let run = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run";
        sys.put(run, "", RegData::Str("default metadata".into()));
        sys.put(run, "Empty", RegData::Str("  ".into()));
        sys.put(run, "Number", RegData::Dword(1));
        sys.put(run, "Valid", RegData::Str(r"C:\app.exe".into()));
        let names: Vec<_> = list(&sys).items.into_iter().filter(|item| item.scope == "Registry (user)").map(|item| item.name).collect();
        assert_eq!(names, vec!["valid"]);
    }

    #[test]
    fn lists_only_third_party_logon_and_boot_tasks_and_toggles_by_full_path() {
        let sys = FakeSystem::default();
        sys.answer("powershell.exe", r#"[
            {"name":"Agent","taskPath":"\\Vendor\\","command":"agent.exe","enabled":true},
            {"name":"Update","taskPath":"\\Microsoft\\Windows\\","command":"update.exe","enabled":true}
        ]"#);
        sys.tasks.borrow_mut().insert(r"\vendor\agent".into(), true);

        let listing = list(&sys);
        assert_eq!(listing.warning, None);
        let tasks: Vec<_> = listing.items.iter().filter(|item| item.scope == "Scheduled task (logon/boot)").collect();
        assert_eq!(tasks.len(), 1);
        let task = tasks[0];
        assert_eq!(task.name, "Agent");
        assert_eq!(task.task_path.as_deref(), Some("\\Vendor\\"));
        set_enabled(&sys, task, false).unwrap();
        assert_eq!(sys.task_enabled(r"\Vendor\Agent"), Some(false));
        set_enabled(&sys, task, true).unwrap();
        assert_eq!(sys.task_enabled(r"\Vendor\Agent"), Some(true));
    }

    #[test]
    fn task_query_failure_keeps_registry_entries_and_reports_warning() {
        let sys = FakeSystem::default();
        let run = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run";
        sys.put(run, "Alpha", RegData::Str(r"C:\a.exe".into()));
        sys.fail.borrow_mut().insert("powershell.exe".into());

        let listing = list(&sys);
        assert!(listing.items.iter().any(|item| item.name == "alpha"));
        assert!(listing.warning.is_some());
    }
}
