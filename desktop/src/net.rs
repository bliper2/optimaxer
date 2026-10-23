use crate::model::{self, Catalog};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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
fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

pub fn load_cache() -> Option<Catalog> {
    serde_json::from_slice(&fs::read(data_dir().join("catalog.json")).ok()?).ok()
}

pub fn save_cache(c: &Catalog) {
    if let Ok(b) = serde_json::to_vec(c) {
        write_atomic(&data_dir().join("catalog.json"), &b);
    }
}

/// Set by the developer screenshot run so it never rewrites the user's saved settings.
pub static READ_ONLY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub download_dir: String,
    pub sort: model::Sort,
    pub platform: String,
    /// Which part of the window was open last: "Store" or "Optimaxer".
    pub section: String,
    /// Optimaxer: offer a System Restore point before applying tweaks.
    pub opti_restore_point: bool,
    /// No transitions or pulses. Defaults to the Windows "animation effects" setting.
    pub reduce_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let dir = dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")).join("Orion");
        Self { dark: true, download_dir: dir.to_string_lossy().into(), sort: model::Sort::Home, platform: model::PC.into(), section: "Store".into(), opti_restore_point: true, reduce_motion: crate::theme::system_reduces_motion() }
    }
}

impl Settings {
    pub fn load() -> Self {
        fs::read(data_dir().join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        if READ_ONLY.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            write_atomic(&data_dir().join("settings.json"), &b);
        }
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
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Strip anything Windows rejects or that could escape the download folder.
pub fn safe_file_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c)).collect();
    let name = name.trim().trim_matches('.').trim().to_string();
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

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    (1..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap()
}

pub fn start_download(url: String, file: String, dir: PathBuf, on_change: impl Fn() + Send + 'static) -> Arc<Download> {
    let dl = Arc::new(Download {
        file,
        done: AtomicU64::new(0),
        total: AtomicU64::new(0),
        cancel: AtomicBool::new(false),
        status: Mutex::new(DlStatus::Running),
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
    let target = unique_path(dir, &job.file);
    let part = dir.join(format!("{}.part", target.file_name().unwrap_or_default().to_string_lossy()));
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
    let mut out = fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut last_ping = std::time::Instant::now();
    let res = (|| loop {
        if job.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
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
    match res {
        Ok(()) => {
            fs::rename(&part, &target).map_err(|e| e.to_string())?;
            Ok(target)
        }
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

    #[test]
    fn embedded_catalog_loads() {
        let c = embedded();
        assert!(!c.apps.is_empty());
        assert!(c.apps.iter().all(|a| !a.id.is_empty() && !a.name.is_empty()));
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(file_name_for("https://x.io/a/My%20App%20Setup.exe?dl=1", ""), "My App Setup.exe");
        assert_eq!(safe_file_name(r"..\..\evil.exe"), "evil.exe");
        assert_eq!(safe_file_name("CON.txt"), "download.bin");
        assert_eq!(file_name_for("https://x.io/", ""), "download.bin");
    }

    #[test]
    #[ignore = "needs network"]
    fn live_catalog_has_pc_or_android_apps() {
        let c = load_catalog();
        println!("{} apps from {}", c.apps.len(), c.source);
        assert!(!c.apps.is_empty());
    }
}
