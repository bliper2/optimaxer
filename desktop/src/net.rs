use crate::model::{self, Catalog};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CONFIG_URLS: [&str; 4] = [
    "https://raw.githubusercontent.com/RookieEnough/Orion-Data/main/config.json",
    "https://gitlab.com/RookieEnough/Orion-Data/-/raw/main/config.json",
    "https://codeberg.org/RookieEnough/Orion-Data/raw/branch/main/config.json",
    "https://cdn.jsdelivr.net/gh/RookieEnough/Orion-Data@main/config.json",
];
const MIRROR_LABELS: [&str; 4] = ["GitHub", "GitLab", "Codeberg", "JSDelivr"];
const MIRROR_JSON: &str = "https://raw.githubusercontent.com/RookieEnough/Orion-Data/data/mirror.json";
const EMBEDDED_APPS: &str = include_str!("../assets/local_apps.json");

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .build()
        .into()
}

fn fetch_text(url: &str) -> Result<String, String> {
    let sep = if url.contains('?') { '&' } else { '?' };
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut resp = agent().get(&format!("{url}{sep}t={ts}")).header("Cache-Control", "no-store").call().map_err(|e| e.to_string())?;
    resp.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
}

fn fetch_json(url: &str) -> Result<Value, String> {
    serde_json::from_str(&fetch_text(url)?).map_err(|e| e.to_string())
}

fn derive_apps_url(config_url: &str) -> String {
    config_url.replace("config.json", "apps.json")
}

/// Config -> apps.json + mirror.json, trying GitHub, GitLab, Codeberg, JSDelivr in order, then the bundled catalog.
pub fn load_catalog() -> Catalog {
    for (i, cfg_url) in CONFIG_URLS.iter().enumerate() {
        let Ok(cfg) = fetch_json(cfg_url) else { continue };
        let pick = |key: &str| cfg.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(String::from);
        let apps_url = pick("appsJsonUrl").unwrap_or_else(|| derive_apps_url(cfg_url));
        let mirror_url = pick("mirrorJsonUrl").unwrap_or_else(|| MIRROR_JSON.into());
        let (apps, mirror) = std::thread::scope(|sc| {
            let m = sc.spawn(|| fetch_json(&mirror_url).unwrap_or(Value::Null));
            let a = fetch_json(&apps_url);
            (a, m.join().unwrap_or(Value::Null))
        });
        if let Ok(apps) = apps {
            let items = model::process(&apps, &mirror);
            if !items.is_empty() {
                return Catalog { apps: items, source: MIRROR_LABELS[i].into() };
            }
        }
    }
    embedded()
}

pub fn embedded() -> Catalog {
    let raw: Value = serde_json::from_str(EMBEDDED_APPS).unwrap_or(Value::Null);
    Catalog { apps: model::process(&raw, &Value::Null), source: "Offline (Local)".into() }
}

pub fn data_dir() -> PathBuf {
    let d = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("OrionStore");
    let _ = fs::create_dir_all(&d);
    d
}

/// Write via temp file + rename so a crash never leaves a half-written file behind.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("Could not stage {}: {e}", path.display()))?;
    if let Err(error) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not replace {}: {error}", path.display()));
    }
    Ok(())
}

pub fn load_cache() -> Option<Catalog> {
    serde_json::from_slice(&fs::read(data_dir().join("catalog.json")).ok()?).ok()
}

pub fn save_cache(c: &Catalog) {
    if let Ok(b) = serde_json::to_vec(c) {
        let _ = write_atomic(&data_dir().join("catalog.json"), &b);
    }
}

/// Set by the developer screenshot run so it never rewrites the user's saved settings.
pub static READ_ONLY: AtomicBool = AtomicBool::new(false);

const MAX_RECENT_DOWNLOADS: usize = 40;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentDownload {
    pub file: String,
    pub path: PathBuf,
    pub completed_at: u64,
}

pub fn load_recent_downloads() -> Result<Vec<RecentDownload>, String> {
    load_recent_downloads_from(&data_dir().join("recent-downloads.json"))
}

fn load_recent_downloads_from(path: &Path) -> Result<Vec<RecentDownload>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("Could not read download history: {e}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

pub fn record_recent_download(record: RecentDownload) -> Result<Vec<RecentDownload>, String> {
    if READ_ONLY.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }
    let records = record_recent_download_at(&data_dir().join("recent-downloads.json"), record)?;
    Ok(records)
}

fn record_recent_download_at(path: &Path, record: RecentDownload) -> Result<Vec<RecentDownload>, String> {
    let mut records = load_recent_downloads_from(path)?;
    records.retain(|item| item.path != record.path);
    records.insert(0, record);
    records.truncate(MAX_RECENT_DOWNLOADS);
    let bytes = serde_json::to_vec_pretty(&records).map_err(|e| format!("Could not encode download history: {e}"))?;
    write_atomic(path, &bytes)?;
    Ok(records)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "system" (follow Windows), "light" or "dark".
    pub theme: String,
    pub download_dir: String,
    pub sort: model::Sort,
    pub platform: String,
    /// Which part of the window was open last: "Store" or "Optimaxer".
    pub section: String,
    /// Keep the Optimaxer navigation as an icon rail when enabled.
    pub sidebar_collapsed: bool,
    /// Optimaxer: offer a System Restore point before applying tweaks.
    pub opti_restore_point: bool,
    /// A confirmed tweak changed system state that needs a Windows restart.
    pub opti_restart_required: bool,
    /// Tweak IDs pinned by the user on the Tweaks page.
    pub opti_tweak_favorites: Vec<String>,
    /// Orion Store catalog IDs saved by the user.
    pub store_favorites: Vec<String>,
    /// Stable IDs acknowledged in the desktop notification center.
    #[serde(default)]
    pub dismissed_notifications: Vec<String>,
    /// System uptime when the restart reminder was recorded. A lower uptime means Windows rebooted.
    pub opti_restart_uptime: u64,
    /// No transitions or pulses. Defaults to the Windows "animation effects" setting.
    pub reduce_motion: bool,
    /// Windows (Fluent), Classic or Material.
    pub style: crate::theme::Style,
    /// Spacing between controls.
    pub density: crate::theme::Density,
    /// Scale for text and interface dimensions.
    pub ui_scale: crate::theme::UiScale,
    /// Custom radius for controls; `None` follows the selected control style.
    pub corner_radius: Option<u8>,
    /// Named color palette; control shape remains controlled by `style`.
    #[serde(deserialize_with = "crate::theme::deserialize_palette")]
    pub palette: crate::theme::Palette,
    /// Saved custom accent as #RRGGBB.
    pub custom_accent: String,
    /// Accessible dark and light surface ladders for the Custom palette.
    #[serde(default)]
    pub custom_surfaces: crate::theme::CustomSurfaces,
    /// Look for a newer release when the app starts. Nothing is downloaded until you press the update button.
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let dir = dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")).join("Orion");
        Self { theme: "system".into(), download_dir: dir.to_string_lossy().into(), sort: model::Sort::Home, platform: model::PC.into(), section: "Store".into(), sidebar_collapsed: false, opti_restore_point: true, opti_restart_required: false, opti_tweak_favorites: Vec::new(), store_favorites: Vec::new(), dismissed_notifications: Vec::new(), opti_restart_uptime: 0, reduce_motion: crate::theme::system_reduces_motion(), style: crate::theme::Style::Windows, density: crate::theme::Density::Comfortable, ui_scale: crate::theme::UiScale::Default, corner_radius: None, palette: crate::theme::Palette::System, custom_accent: crate::theme::hex_color(crate::theme::BRAND), custom_surfaces: crate::theme::CustomSurfaces::default(), auto_update: true }
    }
}

impl Settings {
    pub fn toggle_store_favorite(&mut self, id: &str) -> bool {
        if self.store_favorites.iter().any(|favorite| favorite == id) {
            self.store_favorites.retain(|favorite| favorite != id);
            false
        } else {
            self.store_favorites.push(id.to_owned());
            self.store_favorites.sort();
            self.store_favorites.dedup();
            true
        }
    }

    pub fn appearance(&self) -> crate::theme::Appearance {
        crate::theme::Appearance {
            dark: self.effective_dark(),
            reduce_motion: self.reduce_motion,
            style: self.style,
            palette: self.palette,
            accent: self.accent(),
            density: self.density,
            scale: self.ui_scale,
            corner_radius: self.corner_radius,
            custom_surfaces: self.custom_surfaces.clone(),
        }
    }

    /// Clear a pending restart reminder after the system uptime resets.
    pub fn clear_restart_after_reboot(&mut self, uptime: u64) -> bool {
        if self.opti_restart_required && uptime < self.opti_restart_uptime {
            self.opti_restart_required = false;
            self.opti_restart_uptime = 0;
            true
        } else {
            false
        }
    }

    /// Whether the app draws dark: the saved choice, or Windows' own setting when it follows the system.
    pub fn effective_dark(&self) -> bool {
        if let Some(dark) = self.palette.forced_dark() {
            return dark;
        }
        match self.theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => crate::theme::system_dark(),
        }
    }

    pub fn accent(&self) -> egui::Color32 {
        if self.palette == crate::theme::Palette::Custom {
            crate::theme::parse_hex_color(&self.custom_accent).unwrap_or(crate::theme::BRAND)
        } else {
            crate::theme::system_accent()
        }
    }

    pub fn load() -> Self {
        fs::read(data_dir().join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save_checked(&self) -> Result<(), String> {
        if READ_ONLY.load(Ordering::Relaxed) {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| format!("Could not encode settings: {e}"))?;
        write_atomic(&data_dir().join("settings.json"), &bytes)
    }
}

// region downloads
#[derive(Clone)]
pub enum DlStatus {
    Running,
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

pub struct Download {
    pub file: String,
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    pub status: Mutex<DlStatus>,
    pub history_recorded: AtomicBool,
}

impl Download {
    pub fn status(&self) -> DlStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or(DlStatus::Running)
    }
    pub fn fraction(&self) -> Option<f32> {
        let t = self.total.load(Ordering::Relaxed);
        (t > 0).then(|| (self.done.load(Ordering::Relaxed) as f32 / t as f32).clamp(0.0, 1.0))
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Strip anything Windows rejects or that could escape the download folder.
pub fn safe_file_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c)).collect();
    let name = name.trim_matches(|c: char| c.is_whitespace() || c == '.').to_string();
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    const RESERVED: [&str; 22] =
        ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    if name.is_empty() || RESERVED.contains(&stem.as_str()) { "download.bin".into() } else { name }
}

pub fn file_name_for(url: &str, hint: &str) -> String {
    if !hint.is_empty() {
        return safe_file_name(hint);
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    safe_file_name(&percent_decode(path.rsplit('/').next().unwrap_or("")))
}

fn reserve_download(dir: &Path, name: &str) -> Result<(PathBuf, PathBuf, fs::File), String> {
    let name = safe_file_name(name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    for n in 0u64.. {
        let candidate = if n == 0 { name.clone() } else { format!("{stem} ({n}){ext}") };
        let target = dir.join(&candidate);
        if target.exists() {
            continue;
        }
        let part = dir.join(format!("{candidate}.part"));
        // Reserve the partial file atomically. Another app instance or an interrupted download may own it.
        match fs::OpenOptions::new().write(true).create_new(true).open(&part) {
            Ok(file) => return Ok((target, part, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot create download: {e}")),
        }
    }
    Err("No available download filename".into())
}

fn finish_download(part: &Path, target: &Path) -> Result<(), String> {
    // std::fs creates extended-length Windows paths. Keep that support when calling Win32 directly.
    let source = fs::canonicalize(part).map_err(|e| e.to_string())?;
    let parent = target.parent().ok_or("Download has no destination folder")?;
    let name = target.file_name().ok_or("Download has no filename")?;
    let destination = fs::canonicalize(parent).map_err(|e| e.to_string())?.join(name);
    let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are owned, NUL-terminated buffers. MoveFileW refuses to replace an existing target.
    if unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr()) } == 0 {
        return Err(format!("Cannot finish download: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

pub fn start_download(url: String, file: String, dir: PathBuf, on_change: impl Fn() + Send + 'static) -> Arc<Download> {
    let dl = Arc::new(Download {
        file,
        done: AtomicU64::new(0),
        total: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
        status: Mutex::new(DlStatus::Running),
        history_recorded: AtomicBool::new(false),
    });
    let job = dl.clone();
    std::thread::spawn(move || {
        let result = run_download(&job, &url, &dir, &on_change);
        if let Ok(mut st) = job.status.lock() {
            *st = match result {
                Ok(p) => DlStatus::Done(p),
                Err(e) if e == "cancelled" => DlStatus::Cancelled,
                Err(e) => DlStatus::Failed(e),
            };
        }
        on_change();
    });
    dl
}

fn run_download(job: &Download, url: &str, dir: &Path, on_change: &dyn Fn()) -> Result<PathBuf, String> {
    let l = url.to_lowercase();
    if !(l.starts_with("https://") || l.starts_with("http://")) {
        return Err("Unsupported link".into());
    }
    fs::create_dir_all(dir).map_err(|e| format!("Cannot create folder: {e}"))?;
    let mut resp = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .new_agent()
        .get(url)
        .call()
        .map_err(|e| e.to_string())?;
    if let Some(t) = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()) {
        job.total.store(t, Ordering::Relaxed);
    }
    let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
    let (target, part, mut out) = reserve_download(dir, &job.file)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut last_ping = std::time::Instant::now();
    let res = (|| loop {
        if job.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            out.sync_all().map_err(|e| e.to_string())?;
            return Ok(());
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        job.done.fetch_add(n as u64, Ordering::Relaxed);
        if last_ping.elapsed() > Duration::from_millis(100) {
            last_ping = std::time::Instant::now();
            on_change();
        }
    })();
    drop(out);
    match res.and_then(|_| finish_download(&part, &target)) {
        Ok(()) => Ok(target),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}
// endregion

#[cfg(test)]
mod tests {
    use super::*;

    fn response_url(response: &'static [u8]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = [0; 4096];
            let received = stream.read(&mut request).unwrap();
            assert!(received > 0);
            stream.write_all(response).unwrap();
        });
        format!("http://{address}/setup.exe")
    }

    fn download_job() -> Download {
        Download { file: "setup.exe".into(), done: AtomicU64::new(0), total: AtomicU64::new(0),
            cancel: AtomicBool::new(false), status: Mutex::new(DlStatus::Running), history_recorded: AtomicBool::new(false) }
    }

    #[test]
    fn http_download_saves_complete_bytes_and_cleans_failed_partials() {
        let root = std::env::temp_dir().join(format!("orion-download-http-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let good = response_url(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\ndownload");
        let job = download_job();
        let target = run_download(&job, &good, &root, &|| {}).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"download");
        assert_eq!(job.done.load(Ordering::Relaxed), 8);
        let short = response_url(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort");
        assert!(run_download(&download_job(), &short, &root, &|| {}).is_err());
        let cancelled = download_job();
        cancelled.cancel.store(true, Ordering::Relaxed);
        let url = response_url(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\ndownload");
        assert_eq!(run_download(&cancelled, &url, &root, &|| {}).unwrap_err(), "cancelled");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(fs::read(target).unwrap(), b"download");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn embedded_catalog_loads() {
        let c = embedded();
        assert!(!c.apps.is_empty());
        assert!(c.apps.iter().all(|a| !a.id.is_empty() && !a.name.is_empty()));
    }

    #[test]
    fn recent_download_history_persists_deduplicates_and_stays_bounded() {
        let root = std::env::temp_dir().join(format!("orion-recent-downloads-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let history = root.join("recent-downloads.json");
        for i in 0..MAX_RECENT_DOWNLOADS + 2 {
            record_recent_download_at(&history, RecentDownload {
                file: format!("tool-{i}.zip"),
                path: root.join(format!("tool-{i}.zip")),
                completed_at: i as u64,
            }).unwrap();
        }
        let duplicate_path = root.join("tool-41.zip");
        let records = record_recent_download_at(&history, RecentDownload {
            file: "renamed.zip".into(),
            path: duplicate_path.clone(),
            completed_at: 99,
        }).unwrap();
        assert_eq!(records.len(), MAX_RECENT_DOWNLOADS);
        assert_eq!(records[0].file, "renamed.zip");
        assert_eq!(records.iter().filter(|item| item.path == duplicate_path).count(), 1);
        assert_eq!(load_recent_downloads_from(&history).unwrap().len(), MAX_RECENT_DOWNLOADS);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_recent_history_is_reported_and_preserved() {
        let root = std::env::temp_dir().join(format!("orion-recent-downloads-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let history = root.join("recent-downloads.json");
        fs::write(&history, b"not json").unwrap();
        assert!(load_recent_downloads_from(&history).is_err());
        assert!(record_recent_download_at(&history, RecentDownload {
            file: "setup.exe".into(), path: root.join("setup.exe"), completed_at: 1,
        }).is_err());
        assert_eq!(fs::read(&history).unwrap(), b"not json");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_settings_gain_theme_defaults_without_resetting_existing_choices() {
        let settings: Settings = serde_json::from_str(r#"{"theme":"dark","download_dir":"D:/Apps"}"#).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.download_dir, "D:/Apps");
        assert_eq!(settings.palette, crate::theme::Palette::System);
        assert_eq!(settings.density, crate::theme::Density::Comfortable);
        assert_eq!(settings.ui_scale, crate::theme::UiScale::Default);
        assert_eq!(settings.corner_radius, None);
        assert_eq!(settings.custom_accent, "#7478F8");
        assert_eq!(settings.custom_surfaces, crate::theme::CustomSurfaces::default());
        assert!(settings.dismissed_notifications.is_empty());
        assert!(!settings.sidebar_collapsed);
        assert!(!settings.opti_restart_required);
        assert_eq!(settings.opti_restart_uptime, 0);
        assert!(settings.opti_tweak_favorites.is_empty());
        assert!(settings.store_favorites.is_empty());
        assert!(settings.effective_dark());
    }

    #[test]
    fn tweak_favorites_survive_settings_round_trip() {
        let settings = Settings {
            opti_tweak_favorites: vec!["game-mode".into(), "dark-mode".into()],
            ..Settings::default()
        };
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.opti_tweak_favorites, ["game-mode", "dark-mode"]);
    }

    #[test]
    fn store_favorites_toggle_in_stable_order_and_survive_settings_round_trip() {
        let mut settings = Settings {
            store_favorites: vec!["beta".into()],
            ..Settings::default()
        };
        assert!(settings.toggle_store_favorite("alpha"));
        assert_eq!(settings.store_favorites, ["alpha", "beta"]);
        assert!(!settings.toggle_store_favorite("beta"));
        assert_eq!(settings.store_favorites, ["alpha"]);

        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.store_favorites, ["alpha"]);
    }

    #[test]
    fn custom_corner_radius_survives_settings_round_trip() {
        let settings = Settings { corner_radius: Some(3), ..Settings::default() };
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.corner_radius, Some(3));
    }

    #[test]
    fn custom_surface_colors_survive_settings_round_trip() {
        let mut settings = Settings::default();
        settings.custom_surfaces.active_mut(true)[0] = "#080A10".into();
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.custom_surfaces, settings.custom_surfaces);
    }

    #[test]
    fn dismissed_notifications_survive_settings_round_trip() {
        let settings = Settings {
            dismissed_notifications: vec!["restart:3600".into()],
            ..Settings::default()
        };
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.dismissed_notifications, ["restart:3600"]);
    }

    #[test]
    fn restart_reminder_persists_until_uptime_resets() {
        let mut settings = Settings {
            opti_restart_required: true,
            opti_restart_uptime: 900,
            ..Settings::default()
        };
        let saved = serde_json::to_string(&settings).unwrap();
        settings = serde_json::from_str(&saved).unwrap();
        assert!(!settings.clear_restart_after_reboot(1_200));
        assert!(settings.opti_restart_required);
        assert!(settings.clear_restart_after_reboot(12));
        assert!(!settings.opti_restart_required);
        assert_eq!(settings.opti_restart_uptime, 0);
    }

    #[test]
    fn atomic_settings_write_reports_replace_failure_and_removes_temp_file() {
        let root = std::env::temp_dir().join(format!("orion-settings-write-{}", std::process::id()));
        let target = root.join("settings.json");
        fs::create_dir_all(&target).unwrap();

        let error = write_atomic(&target, b"settings").unwrap_err();

        assert!(error.starts_with("Could not replace "), "{error}");
        assert!(!root.join("settings.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn collapsed_sidebar_setting_survives_round_trip() {
        let settings = Settings { sidebar_collapsed: true, ..Settings::default() };
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert!(loaded.sidebar_collapsed);
    }

    #[test]
    fn unknown_palette_falls_back_without_resetting_saved_settings() {
        let settings: Settings = serde_json::from_str(r#"{"theme":"light","download_dir":"D:/Apps","palette":"future_palette"}"#).unwrap();
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.download_dir, "D:/Apps");
        assert_eq!(settings.palette, crate::theme::Palette::System);
        assert!(!settings.effective_dark());
    }

    #[test]
    fn explicit_palette_mode_and_custom_accent_survive_settings_round_trip() {
        let mut settings = Settings {
            palette: crate::theme::Palette::MaterialLight,
            theme: "dark".into(),
            custom_accent: "#12ABEF".into(),
            ..Settings::default()
        };
        assert!(!settings.effective_dark(), "fixed light palette wins over the generic mode control");
        settings.palette = crate::theme::Palette::Custom;
        assert_eq!(settings.accent(), egui::Color32::from_rgb(0x12, 0xAB, 0xEF));
        let saved = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.palette, crate::theme::Palette::Custom);
        assert_eq!(loaded.custom_accent, "#12ABEF");
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(file_name_for("https://x.io/a/My%20App%20Setup.exe?dl=1", ""), "My App Setup.exe");
        assert_eq!(safe_file_name(r"..\..\evil.exe"), "evil.exe");
        assert_eq!(safe_file_name("CON.txt"), "download.bin");
        assert_eq!(file_name_for("https://x.io/", ""), "download.bin");
        assert_eq!(safe_file_name(" . setup.exe. . "), "setup.exe");
    }

    #[test]
    fn downloads_reserve_distinct_files_and_keep_existing_partials() {
        let dir = std::env::temp_dir().join(format!("orion-download-reserve-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("setup.exe.part"), b"unfinished").unwrap();
        let (target_a, part_a, file_a) = reserve_download(&dir, "setup.exe").unwrap();
        let (target_b, part_b, file_b) = reserve_download(&dir, "setup.exe").unwrap();
        assert_ne!(target_a, target_b);
        assert_ne!(part_a, part_b);
        assert_eq!(fs::read(dir.join("setup.exe.part")).unwrap(), b"unfinished");
        drop((file_a, file_b));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn completing_download_never_replaces_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("orion-download-finish-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (target, part, mut file) = reserve_download(&dir, "setup.exe").unwrap();
        file.write_all(b"download").unwrap();
        drop(file);
        fs::write(&target, b"existing").unwrap();
        assert!(finish_download(&part, &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"existing");
        fs::remove_file(&target).unwrap();
        finish_download(&part, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"download");
        assert!(!part.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn downloads_finish_in_long_windows_paths() {
        let root = std::env::temp_dir().join(format!("orion-download-long-{}", std::process::id()));
        let dir = root.join("nested-folder-".repeat(12)).join("nested-folder-".repeat(12));
        fs::create_dir_all(&dir).unwrap();
        let (target, part, mut file) = reserve_download(&dir, "setup.exe").unwrap();
        file.write_all(b"download").unwrap();
        drop(file);
        finish_download(&part, &target).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"download");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "needs network"]
    fn live_catalog_has_pc_or_android_apps() {
        let c = load_catalog();
        println!("{} apps from {}", c.apps.len(), c.source);
        assert!(!c.apps.is_empty());
    }
}
