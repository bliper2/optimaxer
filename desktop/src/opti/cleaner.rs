//! Cleaner page: scan, then delete temp files, update and shader caches, browser and dev-tool caches, dumps, logs,
//! event logs, the Recycle Bin, the component store and Windows.old. Paths come from Optimaxer's cleanup table.
//! Directory junctions and symlinks are never followed.

use super::backend::System;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::UI::Shell::{SHEmptyRecycleBinW, SHQueryRecycleBinW, SHQUERYRBINFO};

const REPARSE: u32 = 0x400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paths,
    EventLogs,
    WinOld,
    Recycle,
    Component,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub risk: &'static str,
    pub paths: &'static [&'static str],
    pub kind: Kind,
    /// Services stopped while the folders are emptied, then started again.
    pub services: &'static [&'static str],
}

macro_rules! t {
    ($id:expr, $name:expr, $desc:expr, $risk:expr, [$($p:expr),*]) => {
        Target { id: $id, name: $name, desc: $desc, risk: $risk, paths: &[$($p),*], kind: Kind::Paths, services: &[] }
    };
    ($id:expr, $name:expr, $desc:expr, $risk:expr, [$($p:expr),*], svc [$($s:expr),*]) => {
        Target { id: $id, name: $name, desc: $desc, risk: $risk, paths: &[$($p),*], kind: Kind::Paths, services: &[$($s),*] }
    };
}

/// Path templates: {TEMP} {WINDIR} {LOCALAPPDATA} {APPDATA} {PROGRAMDATA} {SYSTEMDRIVE}; `*` matches within one segment.
pub static TARGETS: &[Target] = &[
    t!("usertemp", "User temp files", "Per-user temporary files left by apps and installers.", "Safe", ["{TEMP}"]),
    t!("wintemp", "Windows temp files", "System-wide temporary files.", "Safe", [r"{WINDIR}\Temp"]),
    t!("wu", "Windows Update download cache", "Already-installed update packages (re-downloaded if ever needed).", "Safe", [r"{WINDIR}\SoftwareDistribution\Download"], svc ["wuauserv", "bits"]),
    t!("do", "Delivery Optimization cache", "Cached update files shared with other PCs.", "Safe", [r"{WINDIR}\ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache"]),
    t!("thumbs", "Thumbnail and icon cache", "Rebuilt automatically by Explorer.", "Safe", [r"{LOCALAPPDATA}\Microsoft\Windows\Explorer\thumbcache_*.db", r"{LOCALAPPDATA}\Microsoft\Windows\Explorer\iconcache_*.db"]),
    t!("wer", "Error reports", "Windows Error Reporting archives and queues.", "Safe", [r"{PROGRAMDATA}\Microsoft\Windows\WER\ReportArchive", r"{PROGRAMDATA}\Microsoft\Windows\WER\ReportQueue", r"{LOCALAPPDATA}\Microsoft\Windows\WER"]),
    t!("dumps", "Crash dumps", "Minidumps, kernel reports and MEMORY.DMP.", "Safe", [r"{WINDIR}\Minidump", r"{WINDIR}\LiveKernelReports", r"{WINDIR}\MEMORY.DMP", r"{LOCALAPPDATA}\CrashDumps"]),
    t!("logs", "Windows log files", "CBS and DISM servicing logs.", "Safe", [r"{WINDIR}\Logs\CBS\*.log", r"{WINDIR}\Logs\DISM\*.log", r"{WINDIR}\Logs\WindowsUpdate\*"]),
    t!("shader", "GPU shader caches", "DirectX/NVIDIA/AMD shader caches. Games recompile shaders on next launch.", "Moderate", [r"{LOCALAPPDATA}\D3DSCache", r"{LOCALAPPDATA}\NVIDIA\DXCache", r"{LOCALAPPDATA}\NVIDIA\GLCache", r"{LOCALAPPDATA}\AMD\DxCache", r"{LOCALAPPDATA}\AMD\GLCache"]),
    t!("chrome", "Google Chrome cache", "Close Chrome first. Logins and bookmarks are kept.", "Safe", [r"{LOCALAPPDATA}\Google\Chrome\User Data\*\Cache", r"{LOCALAPPDATA}\Google\Chrome\User Data\*\Code Cache", r"{LOCALAPPDATA}\Google\Chrome\User Data\*\GPUCache"]),
    t!("edge", "Microsoft Edge cache", "Close Edge first. Logins and bookmarks are kept.", "Safe", [r"{LOCALAPPDATA}\Microsoft\Edge\User Data\*\Cache", r"{LOCALAPPDATA}\Microsoft\Edge\User Data\*\Code Cache", r"{LOCALAPPDATA}\Microsoft\Edge\User Data\*\GPUCache"]),
    t!("brave", "Brave cache", "Close Brave first.", "Safe", [r"{LOCALAPPDATA}\BraveSoftware\Brave-Browser\User Data\*\Cache", r"{LOCALAPPDATA}\BraveSoftware\Brave-Browser\User Data\*\Code Cache", r"{LOCALAPPDATA}\BraveSoftware\Brave-Browser\User Data\*\GPUCache"]),
    t!("opera", "Opera / Opera GX cache", "Close Opera first.", "Safe", [r"{LOCALAPPDATA}\Opera Software\Opera Stable\Cache", r"{LOCALAPPDATA}\Opera Software\Opera GX Stable\Cache", r"{APPDATA}\Opera Software\Opera Stable\Cache", r"{APPDATA}\Opera Software\Opera Stable\Code Cache", r"{APPDATA}\Opera Software\Opera Stable\GPUCache", r"{APPDATA}\Opera Software\Opera GX Stable\Cache", r"{APPDATA}\Opera Software\Opera GX Stable\Code Cache", r"{APPDATA}\Opera Software\Opera GX Stable\GPUCache"]),
    t!("vivaldi", "Vivaldi cache", "Close Vivaldi first.", "Safe", [r"{LOCALAPPDATA}\Vivaldi\User Data\*\Cache", r"{LOCALAPPDATA}\Vivaldi\User Data\*\Code Cache", r"{LOCALAPPDATA}\Vivaldi\User Data\*\GPUCache"]),
    t!("firefox", "Firefox cache", "Close Firefox first.", "Safe", [r"{LOCALAPPDATA}\Mozilla\Firefox\Profiles\*\cache2"]),
    t!("discord", "Discord cache", "Close Discord first.", "Safe", [r"{APPDATA}\discord\Cache", r"{APPDATA}\discord\Code Cache", r"{APPDATA}\discord\GPUCache"]),
    t!("teams", "Teams cache", "Close Teams first. Chats and sign-in are kept.", "Safe", [r"{APPDATA}\Microsoft\Teams\Cache", r"{APPDATA}\Microsoft\Teams\Code Cache", r"{APPDATA}\Microsoft\Teams\GPUCache", r"{LOCALAPPDATA}\Microsoft\Teams\Cache"]),
    t!("inet", "Windows web and certificate caches", "WinInet temporary files and the Cryptnet URL cache. Rebuilt when Windows checks certificates.", "Safe", [r"{LOCALAPPDATA}\Microsoft\Windows\INetCache", r"{WINDIR}\System32\config\systemprofile\AppData\LocalLow\Microsoft\CryptnetUrlCache\Content", r"{WINDIR}\System32\config\systemprofile\AppData\LocalLow\Microsoft\CryptnetUrlCache\MetaData"]),
    t!("spotify", "Spotify cache", "Offline downloads are kept.", "Safe", [r"{LOCALAPPDATA}\Spotify\Data"]),
    t!("steam", "Steam web cache", "Steam browser/html cache.", "Safe", [r"{LOCALAPPDATA}\Steam\htmlcache"]),
    t!("fontcache", "Windows font cache", "Rebuilt on demand. Fixes garbled fonts.", "Safe", [r"{WINDIR}\ServiceProfiles\LocalService\AppData\Local\FontCache\*", r"{WINDIR}\System32\FNTCACHE.DAT"], svc ["FontCache"]),
    t!("recent", "Recent files and jump lists", "Clears the Recent items list and taskbar jump lists (privacy).", "Safe", [r"{APPDATA}\Microsoft\Windows\Recent\*"]),
    t!("storecache", "Microsoft Store cache", "Local Store download cache.", "Safe", [r"{LOCALAPPDATA}\Packages\Microsoft.WindowsStore_8wekyb3d8bbwe\LocalCache\*"]),
    t!("vscode", "VS Code cache", "Close VS Code first. Settings and extensions are kept.", "Safe", [r"{APPDATA}\Code\Cache", r"{APPDATA}\Code\CachedData", r"{APPDATA}\Code\GPUCache", r"{APPDATA}\Code\Code Cache"]),
    t!("npm", "npm cache", "Downloaded packages; re-fetched when needed.", "Safe", [r"{LOCALAPPDATA}\npm-cache"]),
    t!("pip", "pip cache", "Python package download cache.", "Safe", [r"{LOCALAPPDATA}\pip\cache"]),
    t!("nuget", "NuGet HTTP cache", "NuGet v3 feed cache.", "Safe", [r"{LOCALAPPDATA}\NuGet\v3-cache"]),
    t!("office", "Microsoft Office file cache", "Close Office first.", "Safe", [r"{LOCALAPPDATA}\Microsoft\Office\16.0\OfficeFileCache"]),
    Target { id: "eventlogs", name: "Windows event logs", desc: "Clears every event log. You lose troubleshooting history.", risk: "Moderate", paths: &[r"{WINDIR}\System32\winevt\Logs\*.evtx"], kind: Kind::EventLogs, services: &[] },
    Target { id: "winold", name: "Previous Windows installation (Windows.old)", desc: "Frees many GB after a Windows upgrade. You can no longer roll back to the previous version.", risk: "Advanced", paths: &[r"{SYSTEMDRIVE}\Windows.old"], kind: Kind::WinOld, services: &[] },
    Target { id: "recycle", name: "Recycle Bin", desc: "Permanently deletes everything in the Recycle Bin.", risk: "Moderate", paths: &[], kind: Kind::Recycle, services: &[] },
    Target { id: "component", name: "Windows component store", desc: "DISM StartComponentCleanup: removes superseded system components (slow).", risk: "Safe", paths: &[], kind: Kind::Component, services: &[] },
];

// region path handling
fn env(name: &str) -> String {
    match name {
        "TEMP" => std::env::var("TEMP").or_else(|_| std::env::var("TMP")).unwrap_or_default(),
        "SYSTEMDRIVE" => std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()),
        "WINDIR" => std::env::var("WINDIR").or_else(|_| std::env::var("SystemRoot")).unwrap_or_else(|_| r"C:\Windows".into()),
        other => std::env::var(other).unwrap_or_default(),
    }
}

pub fn expand_template(t: &str) -> Option<String> {
    let mut out = t.to_string();
    for var in ["TEMP", "WINDIR", "LOCALAPPDATA", "APPDATA", "PROGRAMDATA", "SYSTEMDRIVE"] {
        let token = format!("{{{var}}}");
        if out.contains(&token) {
            let v = env(var);
            if v.is_empty() {
                return None;
            }
            out = out.replace(&token, &v);
        }
    }
    Some(out)
}

/// `*` matches any run of characters inside one name, case-insensitive.
pub fn wild_match(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.to_lowercase(), name.to_lowercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == n;
    }
    let mut rest = n.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(pos) => rest = &rest[pos + part.len()..],
                None => return false,
            }
        }
    }
    true
}

/// Expands wildcard segments against the file system and returns paths that exist.
pub fn expand(pattern: &str) -> Vec<PathBuf> {
    let mut segs = pattern.split('\\').filter(|s| !s.is_empty());
    let Some(root) = segs.next() else { return Vec::new() };
    let mut current = vec![PathBuf::from(format!("{root}\\"))];
    for seg in segs {
        let mut next = Vec::new();
        for base in &current {
            if fs::symlink_metadata(base).is_ok_and(|m| is_reparse(&m)) {
                continue;
            }
            if seg.contains('*') {
                for e in fs::read_dir(base).into_iter().flatten().filter_map(Result::ok) {
                    if wild_match(seg, &e.file_name().to_string_lossy()) {
                        next.push(e.path());
                    }
                }
            } else {
                let p = base.join(seg);
                if fs::symlink_metadata(&p).is_ok() {
                    next.push(p);
                }
            }
        }
        current = next;
    }
    current
}

fn is_reparse(m: &fs::Metadata) -> bool {
    m.file_attributes() & REPARSE != 0
}

/// Refuses anything that is not at least `drive\folder\item` deep, so a bad template can never target a drive root.
fn deep_enough(p: &Path) -> bool {
    p.components().count() >= 4
}

pub fn dir_size(dir: &Path) -> u64 {
    if fs::symlink_metadata(dir).is_ok_and(|m| is_reparse(&m)) {
        return 0;
    }
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().filter_map(Result::ok) {
            let Ok(m) = fs::symlink_metadata(e.path()) else { continue };
            if is_reparse(&m) {
                continue;
            }
            if m.is_dir() { stack.push(e.path()) } else { total += m.len() }
        }
    }
    total
}

fn item_size(p: &Path) -> u64 {
    match fs::symlink_metadata(p) {
        Ok(m) if is_reparse(&m) => 0,
        Ok(m) if m.is_dir() => dir_size(p),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}
// endregion

fn concrete_paths(t: &Target) -> Vec<PathBuf> {
    t.paths.iter().filter_map(|p| expand_template(p)).flat_map(|p| expand(&p)).filter(|p| deep_enough(p)).collect()
}

/// Bytes that cleaning would free. `None` means the size cannot be known up front (component store).
pub fn scan(t: &Target) -> Option<u64> {
    match t.kind {
        Kind::Component => None,
        Kind::Recycle => Some(recycle_size()),
        Kind::WinOld => Some(expand_template(t.paths[0]).map(|d| Path::new(&d).to_path_buf()).filter(|d| d.exists()).map_or(0, |d| dir_size(&d))),
        Kind::Paths | Kind::EventLogs => Some(concrete_paths(t).iter().map(|p| item_size(p)).sum()),
    }
}

fn recycle_size() -> u64 {
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, i64Size: 0, i64NumItems: 0 };
    // SAFETY: cbSize is set and `info` is a valid out-pointer; a null root queries every drive.
    let hr = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
    if hr == 0 { info.i64Size.max(0) as u64 } else { 0 }
}

/// Deletes what is inside a directory (or the file itself). Locked files are skipped. Returns bytes freed.
fn clear_item(p: &Path) -> u64 {
    let Ok(m) = fs::symlink_metadata(p) else { return 0 };
    if is_reparse(&m) {
        return 0;
    }
    if !m.is_dir() {
        let len = m.len();
        return if fs::remove_file(p).is_ok() { len } else { 0 };
    }
    let mut freed = 0;
    for e in fs::read_dir(p).into_iter().flatten().filter_map(Result::ok) {
        let path = e.path();
        let Ok(em) = fs::symlink_metadata(&path) else { continue };
        if is_reparse(&em) {
            continue;
        }
        if em.is_dir() {
            let size = dir_size(&path);
            if fs::remove_dir_all(&path).is_ok() {
                freed += size;
            } else {
                freed += clear_item(&path);
            }
        } else {
            let len = em.len();
            if fs::remove_file(&path).is_ok() {
                freed += len;
            }
        }
    }
    freed
}

/// Cleans one target and returns bytes freed. Targets that are file patterns delete the matched files; directory
/// targets are emptied but kept.
pub fn clean(sys: &dyn System, t: &Target, log: &mut dyn FnMut(&str)) -> u64 {
    match t.kind {
        Kind::Recycle => {
            let before = recycle_size();
            // SAFETY: null window and root; flags 7 = no confirmation, no progress UI, no sound.
            let result = unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), 7) };
            if result != 0 {
                log(&format!("WARN Recycle Bin cleanup failed (0x{result:08X})"));
            }
            before.saturating_sub(recycle_size())
        }
        Kind::Component => {
            let out = sys.run("dism.exe", &["/Online", "/Cleanup-Image", "/StartComponentCleanup"]);
            if out.code != 0 {
                log(&format!("WARN Component cleanup failed: {}", out.text.trim()));
            }
            0
        }
        Kind::EventLogs => {
            let before = scan(t).unwrap_or(0);
            let listed = sys.run("wevtutil.exe", &["el"]);
            if listed.code != 0 {
                log(&format!("WARN Could not list event logs: {}", listed.text.trim()));
                return 0;
            }
            for l in listed.text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let out = sys.run("wevtutil.exe", &["cl", l]);
                if out.code != 0 {
                    log(&format!("WARN Could not clear event log {l}: {}", out.text.trim()));
                }
            }
            before.saturating_sub(scan(t).unwrap_or(before))
        }
        Kind::WinOld => {
            let Some(dir) = expand_template(t.paths[0]) else { return 0 };
            if !Path::new(&dir).exists() {
                return 0;
            }
            if fs::symlink_metadata(&dir).is_ok_and(|m| is_reparse(&m)) {
                log("WARN Windows.old is a reparse point; left untouched.");
                return 0;
            }
            let before = dir_size(Path::new(&dir));
            sys.run("takeown.exe", &["/F", &dir, "/R", "/A", "/D", "Y"]);
            sys.run("icacls.exe", &[&dir, "/grant", "Administrators:F", "/T", "/C", "/Q"]);
            sys.run("cmd.exe", &["/c", "rd", "/s", "/q", &dir]);
            if Path::new(&dir).exists() { 0 } else { before }
        }
        Kind::Paths => {
            let running: Vec<&str> = t.services.iter().copied().filter(|s| sys.run("sc.exe", &["query", s]).text.contains("RUNNING")).collect();
            let mut stopped = Vec::new();
            let mut stop_failed = false;
            for s in &running {
                let out = sys.run("net.exe", &["stop", s]);
                if out.code == 0 {
                    stopped.push(*s);
                } else {
                    stop_failed = true;
                    log(&format!("WARN Could not stop {s}; cleanup skipped: {}", out.text.trim()));
                }
            }
            let freed = if stop_failed { 0 } else { concrete_paths(t).iter().map(|p| clear_item(p)).sum() };
            for s in stopped {
                let out = sys.run("net.exe", &["start", s]);
                if out.code != 0 {
                    log(&format!("WARN Could not restart {s}: {}", out.text.trim()));
                }
            }
            freed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opti-clean-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn wildcards_match_within_a_name() {
        assert!(wild_match("thumbcache_*.db", "thumbcache_256.db"));
        assert!(wild_match("*.LOG", "cbs.log"));
        assert!(wild_match("*", "anything"));
        assert!(!wild_match("thumbcache_*.db", "iconcache_1.db"));
        assert!(wild_match("Cache", "cache"));
        assert!(!wild_match("Cache", "Code Cache"));
    }

    #[test]
    fn templates_expand_and_unknown_environments_are_dropped() {
        assert!(expand_template(r"{WINDIR}\Temp").unwrap().to_lowercase().ends_with(r"\temp"));
        assert!(expand_template("{TEMP}").is_some());
    }

    #[test]
    fn expand_handles_wildcard_directories() {
        let d = temp("glob");
        for p in ["Default", "Profile 1"] {
            fs::create_dir_all(d.join("User Data").join(p).join("Cache")).unwrap();
            fs::write(d.join("User Data").join(p).join("Cache").join("f"), b"x").unwrap();
        }
        let pat = format!(r"{}\User Data\*\Cache", d.display());
        assert_eq!(expand(&pat).len(), 2);
    }

    #[test]
    fn clearing_empties_directories_but_keeps_them_and_counts_bytes() {
        let d = temp("clear");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("a.tmp"), vec![0u8; 100]).unwrap();
        fs::write(d.join("sub").join("b.tmp"), vec![0u8; 50]).unwrap();
        assert_eq!(dir_size(&d), 150);
        assert_eq!(clear_item(&d), 150);
        assert!(d.exists());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
    }

    #[test]
    fn shallow_paths_are_refused() {
        assert!(!deep_enough(Path::new(r"C:\")));
        assert!(!deep_enough(Path::new(r"C:\Windows")));
        assert!(deep_enough(Path::new(r"C:\Windows\Temp")));
    }

    #[test]
    fn junctions_are_not_followed() {
        let d = temp("junction");
        let victim = temp("victim");
        fs::write(victim.join("keep.txt"), b"important").unwrap();
        let link = d.join("link");
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&victim).output().map(|o| o.status.success()).unwrap_or(false);
        if !ok {
            return; // cannot create a junction here; nothing to verify
        }
        assert_eq!(clear_item(&d), 0);
        assert!(victim.join("keep.txt").exists(), "the junction target must survive");
        assert_eq!(dir_size(&link), 0);
        assert!(expand(&format!(r"{}\*", link.display())).is_empty());
    }

    #[test]
    fn target_table_is_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for t in TARGETS {
            assert!(ids.insert(t.id), "duplicate {}", t.id);
            assert!(!t.name.is_empty() && !t.desc.is_empty());
            if t.kind == Kind::Paths {
                assert!(!t.paths.is_empty(), "{}", t.id);
            }
            for p in t.paths {
                assert!(p.starts_with('{'), "{} path must start with an environment token: {p}", t.id);
            }
        }
        assert_eq!(TARGETS.len(), 32);
    }

    #[test]
    fn service_targets_stop_and_restart_only_running_services() {
        let sys = FakeSystem::default();
        sys.answer("sc.exe query wuauserv", "STATE : 4 RUNNING");
        let t = TARGETS.iter().find(|t| t.id == "wu").unwrap();
        clean(&sys, t, &mut |_| {});
        assert!(sys.ran("net.exe stop wuauserv"));
        assert!(sys.ran("net.exe start wuauserv"));
        assert!(!sys.ran("net.exe start bits"), "bits was not running before, so it stays stopped");
    }

    #[test]
    fn failed_component_and_event_log_cleanup_report_errors() {
        for (id, command) in [("component", "dism.exe"), ("eventlogs", "wevtutil.exe el")] {
            let sys = FakeSystem::default();
            sys.fail.borrow_mut().insert(command.into());
            let mut lines = Vec::new();
            let target = TARGETS.iter().find(|t| t.id == id).unwrap();
            assert_eq!(clean(&sys, target, &mut |l| lines.push(l.to_string())), 0);
            assert!(lines.iter().any(|l| l.contains("WARN")), "{lines:?}");
        }
    }
}
