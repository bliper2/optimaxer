//! Startup page: Run-key entries and Startup-folder shortcuts, enabled or disabled through the same
//! StartupApproved registry values Task Manager uses, so nothing is uninstalled or deleted.
//! (Scheduled tasks that run at logon are not listed; the PowerShell edition still has them.)

use super::backend::{RegData, System};
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
}

fn enabled_flag(sys: &dyn System, approved: &str, name: &str) -> bool {
    match sys.reg_get(approved, name) {
        Some(RegData::Bin(b)) => !b.first().is_some_and(|f| f & 1 == 1),
        _ => true,
    }
}

pub fn list(sys: &dyn System) -> Vec<Item> {
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
            let enabled = enabled_flag(sys, &approved, &name);
            out.push(Item { name, cmd, scope, approved: approved.clone(), enabled });
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
            out.push(Item { name, cmd: e.path().to_string_lossy().to_string(), scope, approved: approved.clone(), enabled });
        }
    }
    out
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
        let run = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run";
        sys.put(run, "Alpha", RegData::Str(r"C:\a.exe".into()));
        sys.put(run, "Beta", RegData::Str(r"C:\b.exe".into()));
        sys.put(&format!(r"HKCU:\{APR}\Run"), "Beta", RegData::Bin(approved_bytes(false, 1)));
        let items: Vec<_> = list(&sys).into_iter().filter(|i| i.scope == "Registry (user)").collect();
        let by = |n: &str| items.iter().find(|i| i.name.eq_ignore_ascii_case(n)).unwrap();
        assert!(by("alpha").enabled);
        assert!(!by("beta").enabled);
    }

    #[test]
    fn toggling_writes_the_approved_flag_and_nothing_else() {
        let sys = FakeSystem::default();
        let run = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Run";
        sys.put(run, "Alpha", RegData::Str(r"C:\a.exe".into()));
        let item = list(&sys).into_iter().find(|i| i.scope == "Registry (user)").unwrap();
        set_enabled(&sys, &item, false).unwrap();
        assert!(!list(&sys).into_iter().find(|i| i.scope == "Registry (user)").unwrap().enabled);
        assert!(sys.reg_get(run, "alpha").is_some(), "the Run entry itself must stay");
        set_enabled(&sys, &item, true).unwrap();
        assert!(list(&sys).into_iter().find(|i| i.scope == "Registry (user)").unwrap().enabled);
    }
}
