//! Tools page: restore point, free RAM, TRIM, system-file checks, Windows Update reset, icon and Store cache
//! resets, performance counters and a registry backup. Each tool reports what it did in plain lines.

use super::backend::{RegData, System};
use std::path::Path;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::ProcessStatus::{EmptyWorkingSet, EnumProcesses};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    RestorePoint,
    FreeRam,
    Trim,
    Sfc,
    Dism,
    ResetUpdate,
    IconCache,
    StoreCache,
    PerfCounters,
    RegBackup,
}

pub struct ToolInfo {
    pub tool: Tool,
    pub title: &'static str,
    pub desc: &'static str,
    /// Changes the system (as opposed to only reading it), so the UI asks before running it.
    pub changes_system: bool,
}

pub const TOOLS: [ToolInfo; 10] = [
    ToolInfo { tool: Tool::RestorePoint, title: "Create restore point", desc: "Snapshot system settings so you can roll back from Windows System Restore.", changes_system: true },
    ToolInfo { tool: Tool::FreeRam, title: "Free RAM now", desc: "Trims the working set of running processes. Helps when memory is nearly full; Windows re-caches as you work.", changes_system: true },
    ToolInfo { tool: Tool::Trim, title: "Trim SSDs", desc: "Runs TRIM on every SSD. Takes seconds.", changes_system: true },
    ToolInfo { tool: Tool::Sfc, title: "Scan system files (SFC)", desc: "Repairs corrupted Windows system files. Takes several minutes.", changes_system: true },
    ToolInfo { tool: Tool::Dism, title: "Repair Windows image (DISM)", desc: "RestoreHealth: fixes the component store. Needs internet. Run before SFC if SFC fails.", changes_system: true },
    ToolInfo { tool: Tool::ResetUpdate, title: "Reset Windows Update", desc: "Clears the update cache and re-registers update services. Fixes stuck updates.", changes_system: true },
    ToolInfo { tool: Tool::IconCache, title: "Rebuild icon cache", desc: "Fixes blank or wrong icons and thumbnails. Restarts Explorer.", changes_system: true },
    ToolInfo { tool: Tool::StoreCache, title: "Reset Microsoft Store cache", desc: "Runs wsreset to fix Store download problems.", changes_system: true },
    ToolInfo { tool: Tool::PerfCounters, title: "Rebuild performance counters", desc: "Runs lodctr /r and winmgmt /resyncperf. Fixes empty Task Manager graphs and slow WMI.", changes_system: true },
    ToolInfo { tool: Tool::RegBackup, title: "Export registry backup", desc: "Saves .reg exports of the policy, Explorer, memory and network keys that Optimaxer edits.", changes_system: false },
];

const RESTORE_KEY: &str = r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore";

fn tail(out: &str, n: usize, skip: impl Fn(&str) -> bool) -> Vec<String> {
    let lines: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty() && !skip(l)).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

/// System Restore point via Checkpoint-Computer (one PowerShell call; there is no console tool for it). The 24-hour
/// limit is lifted for the call and put back afterwards.
pub fn create_restore_point(sys: &dyn System, description: &str, log: &mut dyn FnMut(&str)) -> bool {
    let old = sys.reg_get(RESTORE_KEY, "SystemRestorePointCreationFrequency");
    let _ = sys.reg_set(RESTORE_KEY, "SystemRestorePointCreationFrequency", &RegData::Dword(0));
    let script = format!("Enable-ComputerRestore -Drive \"$env:SystemDrive\\\" -ErrorAction SilentlyContinue; Checkpoint-Computer -Description '{}' -RestorePointType MODIFY_SETTINGS -ErrorAction Stop", description.replace('\'', ""));
    let out = sys.run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", &script]);
    match old {
        Some(v) => {
            let _ = sys.reg_set(RESTORE_KEY, "SystemRestorePointCreationFrequency", &v);
        }
        None => {
            let _ = sys.reg_delete_value(RESTORE_KEY, "SystemRestorePointCreationFrequency");
        }
    }
    if out.code == 0 {
        log(&format!("Restore point created: {description}"));
        true
    } else {
        log(&format!("WARN Restore point failed: {}", out.text.trim().lines().next().unwrap_or("unknown error")));
        false
    }
}

fn available_ram() -> u64 {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: dwLength is set and `m` is a valid out-pointer.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 { 0 } else { m.ullAvailPhys }
}

/// Trims the working set of every process this user may touch. Returns how many were trimmed.
fn trim_working_sets() -> u32 {
    let mut pids = vec![0u32; 4096];
    let mut bytes = 0u32;
    // SAFETY: buffer size in bytes is passed; `bytes` receives the amount written.
    if unsafe { EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes) } == 0 {
        return 0;
    }
    let me = std::process::id();
    let mut n = 0;
    for pid in pids.iter().take(bytes as usize / 4).copied().filter(|p| *p > 4 && *p != me) {
        // SAFETY: handle is closed on every path.
        unsafe {
            let h = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                continue;
            }
            if EmptyWorkingSet(h) != 0 {
                n += 1;
            }
            CloseHandle(h);
        }
    }
    n
}

const REG_BACKUP_KEYS: [&str; 7] = [
    r"HKLM\SOFTWARE\Policies",
    r"HKCU\SOFTWARE\Policies",
    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer",
    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
    r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management",
    r"HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl",
    r"HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters",
];

pub fn run(sys: &dyn System, tool: Tool, backups: &Path, log: &mut dyn FnMut(&str)) {
    match tool {
        Tool::RestorePoint => {
            create_restore_point(sys, "Optimaxer - manual", log);
        }
        Tool::FreeRam => {
            let before = available_ram();
            let n = trim_working_sets();
            std::thread::sleep(std::time::Duration::from_secs(1));
            let gained = available_ram().saturating_sub(before);
            log(&format!("Trimmed {n} processes. Free RAM +{}. (Windows re-caches as you work; this mainly helps when memory is full.)", crate::model::format_size(Some(gained))));
        }
        Tool::Trim => {
            let out = sys.run("defrag.exe", &["/C", "/L"]);
            for l in tail(&out.text, 6, |_| false) {
                log(&format!("  {l}"));
            }
            log("TRIM finished (hard disks are skipped by /L).");
        }
        Tool::Sfc => {
            let out = sys.run("sfc.exe", &["/scannow"]);
            for l in tail(&out.text, 6, |l| l.contains('%')) {
                log(&format!("  {l}"));
            }
        }
        Tool::Dism => {
            let out = sys.run("Dism.exe", &["/Online", "/Cleanup-Image", "/RestoreHealth"]);
            for l in tail(&out.text, 5, |l| l.starts_with('[')) {
                log(&format!("  {l}"));
            }
        }
        Tool::ResetUpdate => {
            let svcs = ["wuauserv", "bits", "cryptsvc", "msiserver"];
            for s in svcs {
                sys.run("net.exe", &["stop", s]);
            }
            let win = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
            for d in [format!(r"{win}\SoftwareDistribution"), format!(r"{win}\System32\catroot2")] {
                let bak = format!("{d}.bak");
                if Path::new(&bak).exists() {
                    let _ = std::fs::remove_dir_all(&bak);
                }
                if Path::new(&d).exists() {
                    let _ = std::fs::rename(&d, &bak);
                }
            }
            for s in svcs {
                sys.run("net.exe", &["start", s]);
            }
            log("Windows Update components reset. Check for updates again.");
        }
        Tool::IconCache => {
            sys.run("taskkill.exe", &["/F", "/IM", "explorer.exe"]);
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                let _ = std::fs::remove_file(format!(r"{local}\IconCache.db"));
                for pat in ["iconcache_*.db", "thumbcache_*.db"] {
                    for f in super::cleaner::expand(&format!(r"{local}\Microsoft\Windows\Explorer\{pat}")) {
                        let _ = std::fs::remove_file(f);
                    }
                }
            }
            sys.run("cmd.exe", &["/c", "start", "", "explorer.exe"]);
            log("Icon cache rebuilt.");
        }
        Tool::StoreCache => {
            sys.run("wsreset.exe", &[]);
            log("Store cache reset.");
        }
        Tool::PerfCounters => {
            sys.run("lodctr.exe", &["/r"]);
            sys.run("lodctr.exe", &["/r"]);
            sys.run("winmgmt.exe", &["/resyncperf"]);
            log("Performance counters rebuilt. Restart apps that read them (Task Manager, monitoring tools).");
        }
        Tool::RegBackup => {
            let dir = backups.join(format!("registry-{}", super::now_stamp()));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                log(&format!("WARN could not create {}: {e}", dir.display()));
                return;
            }
            for k in REG_BACKUP_KEYS {
                let file = dir.join(format!("{}.reg", k.replace(['\\', ' '], "_")));
                sys.run("reg.exe", &["export", k, &file.to_string_lossy(), "/y"]);
                log(&format!("  exported {k}"));
            }
            log(&format!("Registry backup saved to {}", dir.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    fn restore_point_lifts_and_restores_the_frequency_limit() {
        let sys = FakeSystem::default();
        sys.put(RESTORE_KEY, "SystemRestorePointCreationFrequency", RegData::Dword(1440));
        let mut lines = Vec::new();
        assert!(create_restore_point(&sys, "Optimaxer - test", &mut |l| lines.push(l.to_string())));
        assert!(sys.ran("Checkpoint-Computer"));
        assert_eq!(sys.reg_get(RESTORE_KEY, "SystemRestorePointCreationFrequency"), Some(RegData::Dword(1440)));
    }

    #[test]
    fn restore_point_removes_the_value_it_added() {
        let sys = FakeSystem::default();
        create_restore_point(&sys, "x", &mut |_| {});
        assert_eq!(sys.reg_get(RESTORE_KEY, "SystemRestorePointCreationFrequency"), None);
    }

    #[test]
    fn quotes_cannot_break_out_of_the_restore_point_command() {
        let sys = FakeSystem::default();
        create_restore_point(&sys, "a'; Remove-Item C:\\x; '", &mut |_| {});
        let cmd = sys.commands.borrow()[0].clone();
        assert!(!cmd.contains("a';"), "single quotes are stripped from the description: {cmd}");
    }

    #[test]
    fn registry_backup_exports_every_key() {
        let sys = FakeSystem::default();
        let dir = std::env::temp_dir().join(format!("opti-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        run(&sys, Tool::RegBackup, &dir, &mut |_| {});
        assert_eq!(sys.commands.borrow().iter().filter(|c| c.starts_with("reg.exe export")).count(), 7);
    }

    #[test]
    fn only_read_only_tools_skip_confirmation() {
        let ro: Vec<_> = TOOLS.iter().filter(|t| !t.changes_system).map(|t| t.tool).collect();
        assert_eq!(ro, vec![Tool::RegBackup]);
    }

    #[test]
    fn tail_keeps_the_last_lines_and_filters_progress() {
        let out = "a\nVerification 40%\nb\nc\n";
        assert_eq!(tail(out, 2, |l| l.contains('%')), vec!["b", "c"]);
    }
}
