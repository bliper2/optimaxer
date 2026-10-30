//! Cleaner page: scan, then delete temp files, update and shader caches, browser and dev-tool caches, dumps, logs,
//! event logs, the Recycle Bin, the component store and Windows.old. Paths come from Optimaxer's cleanup table.
//! Directory junctions and symlinks are never followed.

use super::backend::System;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::UI::Shell::{SHEmptyRecycleBinW, SHQUERYRBINFO, SHQueryRecycleBinW};

const REPARSE: u32 = 0x400;
const MAX_PREVIEW_FILES: usize = 200;
const MAX_PREVIEW_NODES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewFile {
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub files: Vec<PreviewFile>,
    pub truncated: bool,
    pub unreadable_directories: usize,
    pub blocked_by_exclusions: bool,
    pub summary: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paths,
    EventLogs,
    WinOld,
    Recycle,
    Component,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Temporary,
    WindowsUpdate,
    Browser,
    Shader,
    Development,
    Application,
    Logs,
    Recycle,
    Other,
}

impl Category {
    pub const ALL: [Self; 9] = [
        Self::Temporary,
        Self::WindowsUpdate,
        Self::Browser,
        Self::Shader,
        Self::Development,
        Self::Application,
        Self::Logs,
        Self::Recycle,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Temporary => "Temporary files",
            Self::WindowsUpdate => "Windows update caches",
            Self::Browser => "Browser caches",
            Self::Shader => "Shader caches",
            Self::Development => "Development caches",
            Self::Application => "Application caches",
            Self::Logs => "Logs and crash dumps",
            Self::Recycle => "Recycle Bin",
            Self::Other => "Other supported targets",
        }
    }
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

pub fn category(target: &Target) -> Category {
    match target.id {
        "usertemp" | "wintemp" => Category::Temporary,
        "wu" | "do" => Category::WindowsUpdate,
        "chrome" | "edge" | "brave" | "opera" | "vivaldi" | "firefox" => Category::Browser,
        "shader" => Category::Shader,
        "vscode" | "npm" | "pip" | "nuget" => Category::Development,
        "discord" | "teams" | "spotify" | "steam" | "office" | "storecache" => {
            Category::Application
        }
        "wer" | "dumps" | "logs" | "eventlogs" => Category::Logs,
        "recycle" => Category::Recycle,
        "thumbs" | "fontcache" | "inet" | "recent" | "winold" | "component" => Category::Other,
        _ => Category::Other,
    }
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
    t!(
        "usertemp",
        "User temp files",
        "Per-user temporary files left by apps and installers.",
        "Safe",
        ["{TEMP}"]
    ),
    t!(
        "wintemp",
        "Windows temp files",
        "System-wide temporary files.",
        "Safe",
        [r"{WINDIR}\Temp"]
    ),
    t!("wu", "Windows Update download cache", "Already-installed update packages (re-downloaded if ever needed).", "Safe", [r"{WINDIR}\SoftwareDistribution\Download"], svc ["wuauserv", "bits"]),
    t!(
        "do",
        "Delivery Optimization cache",
        "Cached update files shared with other PCs.",
        "Safe",
        [
            r"{WINDIR}\ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache"
        ]
    ),
    t!(
        "thumbs",
        "Thumbnail and icon cache",
        "Rebuilt automatically by Explorer.",
        "Safe",
        [
            r"{LOCALAPPDATA}\Microsoft\Windows\Explorer\thumbcache_*.db",
            r"{LOCALAPPDATA}\Microsoft\Windows\Explorer\iconcache_*.db"
        ]
    ),
    t!(
        "wer",
        "Error reports",
        "Windows Error Reporting archives and queues.",
        "Safe",
        [
            r"{PROGRAMDATA}\Microsoft\Windows\WER\ReportArchive",
            r"{PROGRAMDATA}\Microsoft\Windows\WER\ReportQueue",
            r"{LOCALAPPDATA}\Microsoft\Windows\WER"
        ]
    ),
    t!(
        "dumps",
        "Crash dumps",
        "Minidumps, kernel reports and MEMORY.DMP.",
        "Safe",
        [
            r"{WINDIR}\Minidump",
            r"{WINDIR}\LiveKernelReports",
            r"{WINDIR}\MEMORY.DMP",
            r"{LOCALAPPDATA}\CrashDumps"
        ]
    ),
    t!(
        "logs",
        "Windows log files",
        "CBS and DISM servicing logs.",
        "Safe",
        [
            r"{WINDIR}\Logs\CBS\*.log",
            r"{WINDIR}\Logs\DISM\*.log",
            r"{WINDIR}\Logs\WindowsUpdate\*"
        ]
    ),
    t!(
        "shader",
        "GPU shader caches",
        "DirectX/NVIDIA/AMD shader caches. Games recompile shaders on next launch.",
        "Moderate",
        [
            r"{LOCALAPPDATA}\D3DSCache",
            r"{LOCALAPPDATA}\NVIDIA\DXCache",
            r"{LOCALAPPDATA}\NVIDIA\GLCache",
            r"{LOCALAPPDATA}\AMD\DxCache",
            r"{LOCALAPPDATA}\AMD\GLCache"
        ]
    ),
    t!(
        "chrome",
        "Google Chrome cache",
        "Close Chrome first. Logins and bookmarks are kept.",
        "Safe",
        [
            r"{LOCALAPPDATA}\Google\Chrome\User Data\*\Cache",
            r"{LOCALAPPDATA}\Google\Chrome\User Data\*\Code Cache",
            r"{LOCALAPPDATA}\Google\Chrome\User Data\*\GPUCache"
        ]
    ),
    t!(
        "edge",
        "Microsoft Edge cache",
        "Close Edge first. Logins and bookmarks are kept.",
        "Safe",
        [
            r"{LOCALAPPDATA}\Microsoft\Edge\User Data\*\Cache",
            r"{LOCALAPPDATA}\Microsoft\Edge\User Data\*\Code Cache",
            r"{LOCALAPPDATA}\Microsoft\Edge\User Data\*\GPUCache"
        ]
    ),
    t!(
        "brave",
        "Brave cache",
        "Close Brave first.",
        "Safe",
        [
            r"{LOCALAPPDATA}\BraveSoftware\Brave-Browser\User Data\*\Cache",
            r"{LOCALAPPDATA}\BraveSoftware\Brave-Browser\User Data\*\Code Cache",
            r"{LOCALAPPDATA}\BraveSoftware\Brave-Browser\User Data\*\GPUCache"
        ]
    ),
    t!(
        "opera",
        "Opera / Opera GX cache",
        "Close Opera first.",
        "Safe",
        [
            r"{LOCALAPPDATA}\Opera Software\Opera Stable\Cache",
            r"{LOCALAPPDATA}\Opera Software\Opera GX Stable\Cache",
            r"{APPDATA}\Opera Software\Opera Stable\Cache",
            r"{APPDATA}\Opera Software\Opera Stable\Code Cache",
            r"{APPDATA}\Opera Software\Opera Stable\GPUCache",
            r"{APPDATA}\Opera Software\Opera GX Stable\Cache",
            r"{APPDATA}\Opera Software\Opera GX Stable\Code Cache",
            r"{APPDATA}\Opera Software\Opera GX Stable\GPUCache"
        ]
    ),
    t!(
        "vivaldi",
        "Vivaldi cache",
        "Close Vivaldi first.",
        "Safe",
        [
            r"{LOCALAPPDATA}\Vivaldi\User Data\*\Cache",
            r"{LOCALAPPDATA}\Vivaldi\User Data\*\Code Cache",
            r"{LOCALAPPDATA}\Vivaldi\User Data\*\GPUCache"
        ]
    ),
    t!(
        "firefox",
        "Firefox cache",
        "Close Firefox first.",
        "Safe",
        [r"{LOCALAPPDATA}\Mozilla\Firefox\Profiles\*\cache2"]
    ),
    t!(
        "discord",
        "Discord cache",
        "Close Discord first.",
        "Safe",
        [
            r"{APPDATA}\discord\Cache",
            r"{APPDATA}\discord\Code Cache",
            r"{APPDATA}\discord\GPUCache"
        ]
    ),
    t!(
        "teams",
        "Teams cache",
        "Close Teams first. Chats and sign-in are kept.",
        "Safe",
        [
            r"{APPDATA}\Microsoft\Teams\Cache",
            r"{APPDATA}\Microsoft\Teams\Code Cache",
            r"{APPDATA}\Microsoft\Teams\GPUCache",
            r"{LOCALAPPDATA}\Microsoft\Teams\Cache"
        ]
    ),
    t!(
        "inet",
        "Windows web and certificate caches",
        "WinInet temporary files and the Cryptnet URL cache. Rebuilt when Windows checks certificates.",
        "Safe",
        [
            r"{LOCALAPPDATA}\Microsoft\Windows\INetCache",
            r"{WINDIR}\System32\config\systemprofile\AppData\LocalLow\Microsoft\CryptnetUrlCache\Content",
            r"{WINDIR}\System32\config\systemprofile\AppData\LocalLow\Microsoft\CryptnetUrlCache\MetaData"
        ]
    ),
    t!(
        "spotify",
        "Spotify cache",
        "Offline downloads are kept.",
        "Safe",
        [r"{LOCALAPPDATA}\Spotify\Data"]
    ),
    t!(
        "steam",
        "Steam web cache",
        "Steam browser/html cache.",
        "Safe",
        [r"{LOCALAPPDATA}\Steam\htmlcache"]
    ),
    t!(
        "fontcache",
        "Windows font cache",
        "Rebuilt on demand. Fixes garbled fonts.",
        "Safe",
        [
            r"{WINDIR}\ServiceProfiles\LocalService\AppData\Local\FontCache\*",
            r"{WINDIR}\System32\FNTCACHE.DAT"
        ],
        svc["FontCache"]
    ),
    t!(
        "recent",
        "Recent files and jump lists",
        "Clears the Recent items list and taskbar jump lists (privacy).",
        "Safe",
        [r"{APPDATA}\Microsoft\Windows\Recent\*"]
    ),
    t!(
        "storecache",
        "Microsoft Store cache",
        "Local Store download cache.",
        "Safe",
        [r"{LOCALAPPDATA}\Packages\Microsoft.WindowsStore_8wekyb3d8bbwe\LocalCache\*"]
    ),
    t!(
        "vscode",
        "VS Code cache",
        "Close VS Code first. Settings and extensions are kept.",
        "Safe",
        [
            r"{APPDATA}\Code\Cache",
            r"{APPDATA}\Code\CachedData",
            r"{APPDATA}\Code\GPUCache",
            r"{APPDATA}\Code\Code Cache"
        ]
    ),
    t!(
        "npm",
        "npm cache",
        "Downloaded packages; re-fetched when needed.",
        "Safe",
        [r"{LOCALAPPDATA}\npm-cache"]
    ),
    t!(
        "pip",
        "pip cache",
        "Python package download cache.",
        "Safe",
        [r"{LOCALAPPDATA}\pip\cache"]
    ),
    t!(
        "nuget",
        "NuGet HTTP cache",
        "NuGet v3 feed cache.",
        "Safe",
        [r"{LOCALAPPDATA}\NuGet\v3-cache"]
    ),
    t!(
        "office",
        "Microsoft Office file cache",
        "Close Office first.",
        "Safe",
        [r"{LOCALAPPDATA}\Microsoft\Office\16.0\OfficeFileCache"]
    ),
    Target {
        id: "eventlogs",
        name: "Windows event logs",
        desc: "Clears every event log. You lose troubleshooting history.",
        risk: "Moderate",
        paths: &[r"{WINDIR}\System32\winevt\Logs\*.evtx"],
        kind: Kind::EventLogs,
        services: &[],
    },
    Target {
        id: "winold",
        name: "Previous Windows installation (Windows.old)",
        desc: "Frees many GB after a Windows upgrade. You can no longer roll back to the previous version.",
        risk: "Advanced",
        paths: &[r"{SYSTEMDRIVE}\Windows.old"],
        kind: Kind::WinOld,
        services: &[],
    },
    Target {
        id: "recycle",
        name: "Recycle Bin",
        desc: "Permanently deletes everything in the Recycle Bin.",
        risk: "Moderate",
        paths: &[],
        kind: Kind::Recycle,
        services: &[],
    },
    Target {
        id: "component",
        name: "Windows component store",
        desc: "DISM StartComponentCleanup: removes superseded system components (slow).",
        risk: "Safe",
        paths: &[],
        kind: Kind::Component,
        services: &[],
    },
];

// region path handling
fn env(name: &str) -> String {
    match name {
        "TEMP" => std::env::var("TEMP")
            .or_else(|_| std::env::var("TMP"))
            .unwrap_or_default(),
        "SYSTEMDRIVE" => std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()),
        "WINDIR" => std::env::var("WINDIR")
            .or_else(|_| std::env::var("SystemRoot"))
            .unwrap_or_else(|_| r"C:\Windows".into()),
        other => std::env::var(other).unwrap_or_default(),
    }
}

pub fn expand_template(t: &str) -> Option<String> {
    let mut out = t.to_string();
    for var in [
        "TEMP",
        "WINDIR",
        "LOCALAPPDATA",
        "APPDATA",
        "PROGRAMDATA",
        "SYSTEMDRIVE",
    ] {
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

/// Validate and normalize an exact Windows path exclusion. Wildcards and relative paths are rejected.
pub fn normalize_exclusion(value: &str) -> Result<String, String> {
    let value = value.trim().replace('/', "\\");
    if value.is_empty() {
        return Err("Enter a file or folder path.".into());
    }
    if value.contains('*') || value.contains('?') {
        return Err("Exclusions must be exact paths; wildcards are not supported.".into());
    }
    if !Path::new(&value).is_absolute() {
        return Err("Enter an absolute Windows path.".into());
    }
    if value.split('\\').any(|part| part == "." || part == "..") {
        return Err("Paths containing . or .. segments are not accepted.".into());
    }
    let normalized = if value.len() == 3 && value.as_bytes()[1] == b':' {
        value
    } else {
        value.trim_end_matches('\\').to_string()
    };
    Ok(normalized)
}

fn normalized_path(value: &str) -> String {
    let value = value.replace('/', "\\");
    let value = if value.len() == 3 && value.as_bytes()[1] == b':' {
        value
    } else {
        value.trim_end_matches('\\').to_string()
    };
    value.to_lowercase()
}

fn path_is_same_or_child(path: &str, parent: &str) -> bool {
    let path = normalized_path(path);
    let parent = normalized_path(parent);
    if path == parent {
        return true;
    }
    if parent.ends_with('\\') {
        return path.starts_with(&parent);
    }
    path.strip_prefix(&parent)
        .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn is_excluded(path: &Path, exclusions: &[String]) -> bool {
    let path = path.to_string_lossy();
    exclusions
        .iter()
        .any(|excluded| path_is_same_or_child(&path, excluded))
}

fn has_excluded_descendant(path: &Path, exclusions: &[String]) -> bool {
    let path = path.to_string_lossy();
    exclusions
        .iter()
        .any(|excluded| path_is_same_or_child(excluded, &path))
}

/// Returns true when a system-managed target cannot honor an exclusion below its storage root.
pub fn blocked_by_exclusions(target: &Target, exclusions: &[String]) -> bool {
    if !matches!(target.kind, Kind::WinOld | Kind::EventLogs) {
        return false;
    }
    target.paths.iter().any(|pattern| {
        let Some(path) = expand_template(pattern) else {
            return false;
        };
        let root = path
            .split('\\')
            .take_while(|part| !part.contains('*') && !part.contains('?'))
            .collect::<Vec<_>>()
            .join("\\");
        !root.is_empty()
            && exclusions.iter().any(|excluded| {
                path_is_same_or_child(excluded, &root) || path_is_same_or_child(&root, excluded)
            })
    })
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
    expand_inner(pattern, usize::MAX).0
}

fn expand_limited(pattern: &str, limit: usize) -> (Vec<PathBuf>, bool) {
    expand_inner(pattern, limit)
}

fn expand_inner(pattern: &str, limit: usize) -> (Vec<PathBuf>, bool) {
    let mut segs = pattern.split('\\').filter(|s| !s.is_empty());
    let Some(root) = segs.next() else {
        return (Vec::new(), false);
    };
    let mut current = vec![PathBuf::from(format!("{root}\\"))];
    let mut truncated = false;
    for seg in segs {
        let mut next = Vec::new();
        for base in &current {
            if fs::symlink_metadata(base).is_ok_and(|m| is_reparse(&m)) {
                continue;
            }
            if seg.contains('*') {
                for e in fs::read_dir(base)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                {
                    if wild_match(seg, &e.file_name().to_string_lossy()) {
                        if next.len() == limit {
                            truncated = true;
                            break;
                        }
                        next.push(e.path());
                    }
                }
            } else {
                let p = base.join(seg);
                if fs::symlink_metadata(&p).is_ok() {
                    if next.len() == limit {
                        truncated = true;
                        break;
                    }
                    next.push(p);
                }
            }
            if truncated {
                break;
            }
        }
        current = next;
    }
    (current, truncated)
}

fn is_reparse(m: &fs::Metadata) -> bool {
    m.file_attributes() & REPARSE != 0
}

/// Refuses anything that is not at least `drive\folder\item` deep, so a bad template can never target a drive root.
fn deep_enough(p: &Path) -> bool {
    p.components().count() >= 4
}

pub fn dir_size(dir: &Path) -> u64 {
    dir_size_excluding(dir, &[])
}

fn dir_size_excluding(dir: &Path, exclusions: &[String]) -> u64 {
    if is_excluded(dir, exclusions) {
        return 0;
    }
    if fs::symlink_metadata(dir).is_ok_and(|m| is_reparse(&m)) {
        return 0;
    }
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            if is_excluded(&e.path(), exclusions) {
                continue;
            }
            let Ok(m) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if is_reparse(&m) {
                continue;
            }
            if m.is_dir() {
                stack.push(e.path())
            } else {
                total += m.len()
            }
        }
    }
    total
}

fn item_size_excluding(p: &Path, exclusions: &[String]) -> u64 {
    if is_excluded(p, exclusions) {
        return 0;
    }
    match fs::symlink_metadata(p) {
        Ok(m) if is_reparse(&m) => 0,
        Ok(m) if m.is_dir() => dir_size_excluding(p, exclusions),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}
// endregion

fn concrete_paths(t: &Target) -> Vec<PathBuf> {
    t.paths
        .iter()
        .filter_map(|p| expand_template(p))
        .flat_map(|p| expand(&p))
        .filter(|p| deep_enough(p))
        .collect()
}

fn preview_concrete_paths(target: &Target) -> (Vec<PathBuf>, bool) {
    let mut paths = Vec::new();
    let mut truncated = false;
    for pattern in target.paths {
        let Some(pattern) = expand_template(pattern) else {
            continue;
        };
        let remaining = MAX_PREVIEW_NODES.saturating_sub(paths.len());
        let (matches, more) = expand_limited(&pattern, remaining);
        paths.extend(matches.into_iter().filter(|path| deep_enough(path)));
        if more {
            truncated = true;
            break;
        }
    }
    (paths, truncated)
}

fn preview_paths(
    paths: &[PathBuf],
    initially_truncated: bool,
    exclusions: &[String],
) -> (Vec<PreviewFile>, bool, usize) {
    let mut files = Vec::new();
    let mut pending = paths.to_vec();
    let mut truncated = initially_truncated;
    let mut unreadable_directories = 0;
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        if visited == MAX_PREVIEW_NODES {
            truncated = true;
            break;
        }
        visited += 1;
        if is_excluded(&path, exclusions) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if is_reparse(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(_) => {
                    unreadable_directories += 1;
                    continue;
                }
            };
            let mut children = Vec::new();
            for entry in entries.filter_map(Result::ok) {
                if pending.len() + visited + children.len() == MAX_PREVIEW_NODES {
                    truncated = true;
                    break;
                }
                children.push(entry.path());
            }
            children.sort();
            for child in children.into_iter().rev() {
                pending.push(child);
            }
            continue;
        }
        if files.len() == MAX_PREVIEW_FILES {
            truncated = true;
            break;
        }
        files.push(PreviewFile {
            path: path.to_string_lossy().into_owned(),
            bytes: metadata.len(),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    (files, truncated, unreadable_directories)
}

/// Preview file paths without mutation. System-managed operations return a scope summary.
pub fn preview(target: &Target, exclusions: &[String]) -> Preview {
    let (paths, summary, path_truncated) = match target.kind {
        Kind::Paths => {
            let (paths, truncated) = preview_concrete_paths(target);
            (
                paths,
                "Matched files are listed below. Cleaner empties matched directories but keeps their roots; junctions and symlinks are skipped.",
                truncated,
            )
        }
        Kind::EventLogs => {
            let (paths, truncated) = preview_concrete_paths(target);
            (
                paths,
                "Accessible .evtx files are listed. Cleaner clears event logs through wevtutil; Windows may deny access to some log files.",
                truncated,
            )
        }
        Kind::WinOld => {
            let (paths, truncated) = preview_concrete_paths(target);
            (
                paths,
                "Files found under Windows.old are listed. Cleaning this target removes the previous installation and its rollback option.",
                truncated,
            )
        }
        Kind::Recycle => (
            Vec::new(),
            "All current Recycle Bin items are affected. The Windows shell API reports total size, not per-item paths.",
            false,
        ),
        Kind::Component => (
            Vec::new(),
            "DISM removes superseded Windows components. Windows does not provide affected component paths through this operation.",
            false,
        ),
    };
    let blocked_by_exclusions = blocked_by_exclusions(target, exclusions);
    let (files, truncated, unreadable_directories) =
        preview_paths(&paths, path_truncated, exclusions);
    Preview {
        files,
        truncated,
        unreadable_directories,
        blocked_by_exclusions,
        summary,
    }
}

/// Bytes that cleaning would free. `None` means the size cannot be known up front (component store).
pub fn scan(t: &Target, exclusions: &[String]) -> Option<u64> {
    if blocked_by_exclusions(t, exclusions) {
        return Some(0);
    }
    match t.kind {
        Kind::Component => None,
        Kind::Recycle => Some(recycle_size()),
        Kind::WinOld => Some(
            expand_template(t.paths[0])
                .map(|d| Path::new(&d).to_path_buf())
                .filter(|d| d.exists())
                .map_or(0, |d| dir_size_excluding(&d, exclusions)),
        ),
        Kind::Paths | Kind::EventLogs => Some(
            concrete_paths(t)
                .iter()
                .map(|p| item_size_excluding(p, exclusions))
                .sum(),
        ),
    }
}

fn recycle_size() -> u64 {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        i64Size: 0,
        i64NumItems: 0,
    };
    // SAFETY: cbSize is set and `info` is a valid out-pointer; a null root queries every drive.
    let hr = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
    if hr == 0 {
        info.i64Size.max(0) as u64
    } else {
        0
    }
}

/// Deletes what is inside a directory (or the file itself). Locked files are skipped. Returns bytes freed.
fn clear_item(p: &Path, exclusions: &[String]) -> u64 {
    if is_excluded(p, exclusions) {
        return 0;
    }
    let Ok(m) = fs::symlink_metadata(p) else {
        return 0;
    };
    if is_reparse(&m) {
        return 0;
    }
    if !m.is_dir() {
        let len = m.len();
        return if fs::remove_file(p).is_ok() { len } else { 0 };
    }
    let mut freed = 0;
    let mut pending = vec![p.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if is_reparse(&metadata) || is_excluded(&path, exclusions) {
                continue;
            }
            if metadata.is_dir() {
                if has_excluded_descendant(&path, exclusions) {
                    pending.push(path);
                    continue;
                }
                let size = dir_size(&path);
                if fs::remove_dir_all(&path).is_ok() {
                    freed += size;
                } else {
                    pending.push(path);
                }
            } else {
                let len = metadata.len();
                if fs::remove_file(&path).is_ok() {
                    freed += len;
                }
            }
        }
    }
    freed
}

/// Cleans one target and returns bytes freed. Targets that are file patterns delete the matched files; directory
/// targets are emptied but kept.
pub fn clean(
    sys: &dyn System,
    t: &Target,
    exclusions: &[String],
    log: &mut dyn FnMut(&str),
) -> u64 {
    if blocked_by_exclusions(t, exclusions) {
        log("WARN Cleanup skipped because an exclusion overlaps this system-managed target.");
        return 0;
    }
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
            let out = sys.run(
                "dism.exe",
                &["/Online", "/Cleanup-Image", "/StartComponentCleanup"],
            );
            if out.code != 0 {
                log(&format!(
                    "WARN Component cleanup failed: {}",
                    out.text.trim()
                ));
            }
            0
        }
        Kind::EventLogs => {
            let before = scan(t, exclusions).unwrap_or(0);
            let listed = sys.run("wevtutil.exe", &["el"]);
            if listed.code != 0 {
                log(&format!(
                    "WARN Could not list event logs: {}",
                    listed.text.trim()
                ));
                return 0;
            }
            for l in listed.text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let out = sys.run("wevtutil.exe", &["cl", l]);
                if out.code != 0 {
                    log(&format!(
                        "WARN Could not clear event log {l}: {}",
                        out.text.trim()
                    ));
                }
            }
            before.saturating_sub(scan(t, exclusions).unwrap_or(before))
        }
        Kind::WinOld => {
            let Some(dir) = expand_template(t.paths[0]) else {
                return 0;
            };
            if !Path::new(&dir).exists() {
                return 0;
            }
            if fs::symlink_metadata(&dir).is_ok_and(|m| is_reparse(&m)) {
                log("WARN Windows.old is a reparse point; left untouched.");
                return 0;
            }
            let before = dir_size(Path::new(&dir));
            sys.run("takeown.exe", &["/F", &dir, "/R", "/A", "/D", "Y"]);
            sys.run(
                "icacls.exe",
                &[&dir, "/grant", "Administrators:F", "/T", "/C", "/Q"],
            );
            sys.run("cmd.exe", &["/c", "rd", "/s", "/q", &dir]);
            if Path::new(&dir).exists() { 0 } else { before }
        }
        Kind::Paths => {
            let running: Vec<&str> = t
                .services
                .iter()
                .copied()
                .filter(|s| sys.run("sc.exe", &["query", s]).text.contains("RUNNING"))
                .collect();
            let mut stopped = Vec::new();
            let mut stop_failed = false;
            for s in &running {
                let out = sys.run("net.exe", &["stop", s]);
                if out.code == 0 {
                    stopped.push(*s);
                } else {
                    stop_failed = true;
                    log(&format!(
                        "WARN Could not stop {s}; cleanup skipped: {}",
                        out.text.trim()
                    ));
                }
            }
            let freed = if stop_failed {
                0
            } else {
                concrete_paths(t)
                    .iter()
                    .map(|p| clear_item(p, exclusions))
                    .sum()
            };
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
        assert!(
            expand_template(r"{WINDIR}\Temp")
                .unwrap()
                .to_lowercase()
                .ends_with(r"\temp")
        );
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
        assert_eq!(clear_item(&d, &[]), 150);
        assert!(d.exists());
        assert_eq!(fs::read_dir(&d).unwrap().count(), 0);
    }

    #[test]
    fn preview_lists_real_files_and_sizes_without_changing_them() {
        let d = temp("preview");
        fs::create_dir_all(d.join("nested")).unwrap();
        fs::write(d.join("a.tmp"), vec![0u8; 23]).unwrap();
        fs::write(d.join("nested").join("b.tmp"), vec![0u8; 41]).unwrap();
        let (files, truncated, unreadable) = preview_paths(std::slice::from_ref(&d), false, &[]);
        assert_eq!(files.len(), 2);
        assert_eq!(files.iter().map(|file| file.bytes).sum::<u64>(), 64);
        assert!(!truncated);
        assert_eq!(unreadable, 0);
        assert!(d.join("a.tmp").exists());
        assert!(d.join("nested").join("b.tmp").exists());
    }

    #[test]
    fn preview_bounds_file_count_and_marks_truncation() {
        let d = temp("preview-bound");
        for index in 0..=MAX_PREVIEW_FILES {
            fs::write(d.join(format!("{index:03}.tmp")), b"x").unwrap();
        }
        let (files, truncated, _) = preview_paths(std::slice::from_ref(&d), false, &[]);
        assert_eq!(files.len(), MAX_PREVIEW_FILES);
        assert!(truncated);
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
        let ok = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&victim)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            return; // cannot create a junction here; nothing to verify
        }
        let (preview, _, _) = preview_paths(std::slice::from_ref(&d), false, &[]);
        assert!(preview.is_empty(), "the preview must not follow a junction");
        assert_eq!(clear_item(&d, &[]), 0);
        assert!(
            victim.join("keep.txt").exists(),
            "the junction target must survive"
        );
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
                assert!(
                    p.starts_with('{'),
                    "{} path must start with an environment token: {p}",
                    t.id
                );
            }
        }
        assert_eq!(TARGETS.len(), 32);
        let expected = [2, 2, 6, 1, 4, 6, 4, 1, 6];
        for (category, expected_count) in Category::ALL.into_iter().zip(expected) {
            assert_eq!(
                TARGETS
                    .iter()
                    .filter(|target| self::category(target) == category)
                    .count(),
                expected_count,
                "{}",
                category.label()
            );
        }
    }

    #[test]
    fn service_targets_stop_and_restart_only_running_services() {
        let sys = FakeSystem::default();
        sys.answer("sc.exe query wuauserv", "STATE : 4 RUNNING");
        let t = TARGETS.iter().find(|t| t.id == "wu").unwrap();
        clean(&sys, t, &[], &mut |_| {});
        assert!(sys.ran("net.exe stop wuauserv"));
        assert!(sys.ran("net.exe start wuauserv"));
        assert!(
            !sys.ran("net.exe start bits"),
            "bits was not running before, so it stays stopped"
        );
    }

    #[test]
    fn failed_component_and_event_log_cleanup_report_errors() {
        for (id, command) in [("component", "dism.exe"), ("eventlogs", "wevtutil.exe el")] {
            let sys = FakeSystem::default();
            sys.fail.borrow_mut().insert(command.into());
            let mut lines = Vec::new();
            let target = TARGETS.iter().find(|t| t.id == id).unwrap();
            assert_eq!(
                clean(&sys, target, &[], &mut |l| lines.push(l.to_string())),
                0
            );
            assert!(lines.iter().any(|l| l.contains("WARN")), "{lines:?}");
        }
    }

    #[test]
    fn exclusion_paths_are_exact_case_insensitive_and_component_bounded() {
        assert!(path_is_same_or_child(
            r"C:\Cache\nested\item.tmp",
            r"c:\cache"
        ));
        assert!(path_is_same_or_child(r"C:\Cache", r"c:\cache"));
        assert!(path_is_same_or_child(r"C:\Cache\item.tmp", r"C:\"));
        assert!(path_is_same_or_child(r"C:\", r"c:\"));
        assert!(!path_is_same_or_child(r"C:\Cache2\item.tmp", r"C:\Cache"));
        assert!(!path_is_same_or_child(r"D:\Cache\item.tmp", r"C:\Cache"));
    }

    #[test]
    fn exclusion_input_requires_an_absolute_exact_path() {
        assert!(normalize_exclusion(r"C:\Users\Public\Cache").is_ok());
        assert!(normalize_exclusion("relative\\Cache").is_err());
        assert!(normalize_exclusion(r"C:\Cache\..\Documents").is_err());
        assert!(normalize_exclusion(r"C:\Cache\*").is_err());
    }

    #[test]
    fn scan_preview_and_cleanup_preserve_excluded_subtree() {
        let root = temp("excluded");
        let protected = root.join("Profiles").join("Keep");
        fs::create_dir_all(&protected).unwrap();
        fs::write(protected.join("saved.dat"), vec![0; 31]).unwrap();
        fs::write(root.join("remove.tmp"), vec![0; 19]).unwrap();
        let exclusion = normalize_exclusion(&protected.to_string_lossy()).unwrap();
        let exclusions = [exclusion];

        assert_eq!(dir_size_excluding(&root, &exclusions), 19);
        let (files, truncated, unreadable) =
            preview_paths(std::slice::from_ref(&root), false, &exclusions);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].bytes, 19);
        assert!(!truncated);
        assert_eq!(unreadable, 0);

        assert_eq!(clear_item(&root, &exclusions), 19);
        assert!(!root.join("remove.tmp").exists());
        assert_eq!(fs::read(protected.join("saved.dat")).unwrap().len(), 31);
    }

    #[test]
    fn overlapping_event_log_exclusion_skips_global_clear() {
        let target = TARGETS
            .iter()
            .find(|target| target.id == "eventlogs")
            .unwrap();
        let pattern = expand_template(target.paths[0]).unwrap();
        let root = pattern.rsplit_once('\\').unwrap().0;
        let exclusions = [format!(r"{root}\Application.evtx")];
        assert!(blocked_by_exclusions(target, &exclusions));
        let sys = FakeSystem::default();
        let mut lines = Vec::new();
        assert_eq!(
            clean(&sys, target, &exclusions, &mut |line| lines
                .push(line.to_owned())),
            0
        );
        assert!(sys.commands.borrow().is_empty());
        assert!(lines.iter().any(|line| line.contains("skipped")));
        assert!(!blocked_by_exclusions(
            target,
            &[r"C:\Users\Public\Cache".into()]
        ));
    }
}
