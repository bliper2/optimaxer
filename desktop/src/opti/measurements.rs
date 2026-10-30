//! Bounded PresentMon captures used for observational before/after comparisons.

use crate::opti::profiles::write_atomic;
use crate::opti::state::state_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::fs;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const MAX_TOOL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CSV_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FRAMES: usize = 2_000_000;
const MAX_HISTORY: usize = 40;
const SECONDS: u32 = 30;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Capture {
    pub label: String,
    pub time: String,
    pub process: String,
    pub seconds: u32,
    pub frames: usize,
    pub average_fps: f64,
    pub low_1_fps: f64,
    pub low_01_fps: f64,
    pub p99_frame_ms: f64,
    pub median_frame_ms: f64,
    pub stutters: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
struct Store {
    captures: Vec<Capture>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ToolManifest {
    digest: String,
    version: String,
}

fn store_path() -> PathBuf {
    state_dir().join("measurements.json")
}

pub fn load() -> Result<Vec<Capture>, String> {
    load_path(&store_path())
}

fn load_path(path: &Path) -> Result<Vec<Capture>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path).map_err(|e| format!("Could not read measurements: {e}"))?;
    if bytes.len() > 256 * 1024 {
        return Err("Measurement history exceeds the 256 KiB safety limit.".into());
    }
    let store: Store = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Could not parse measurement history: {e}"))?;
    if store.captures.len() > MAX_HISTORY {
        return Err("Measurement history exceeds the capture limit.".into());
    }
    Ok(store.captures)
}

fn save_path(path: &Path, captures: &[Capture]) -> Result<(), String> {
    let bounded = captures
        .iter()
        .rev()
        .take(MAX_HISTORY)
        .cloned()
        .collect::<Vec<_>>();
    let store = Store {
        captures: bounded.into_iter().rev().collect(),
    };
    let bytes = serde_json::to_vec_pretty(&store).map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("Measurement history exceeds the 256 KiB safety limit.".into());
    }
    write_atomic(path, &bytes)
}

pub fn validate_process_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.len() < 5
        || name.len() > 128
        || !name.to_ascii_lowercase().ends_with(".exe")
        || name
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        || name == ".exe"
        || name.starts_with('.')
    {
        return Err("Enter a process name such as game.exe. Do not include a path.".into());
    }
    Ok(name)
}

fn csv_fields(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut field)),
            _ => field.push(ch),
        }
    }
    if quoted {
        return Err("PresentMon CSV contains an unterminated quoted field.".into());
    }
    out.push(field);
    Ok(out)
}

pub fn parse_frame_times(csv: &str) -> Result<Vec<f64>, String> {
    let mut lines = csv.lines();
    let headers = csv_fields(lines.next().ok_or("PresentMon returned an empty CSV.")?)?;
    let index = headers
        .iter()
        .position(|h| h.trim().eq_ignore_ascii_case("FrameTime"))
        .or_else(|| {
            headers
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case("MsBetweenPresents"))
        })
        .ok_or("PresentMon CSV has no FrameTime or MsBetweenPresents column.")?;
    let mut values = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        if values.len() >= MAX_FRAMES {
            return Err("PresentMon capture exceeds the 2,000,000-frame safety limit.".into());
        }
        let fields = csv_fields(line)?;
        let Some(value) = fields.get(index).and_then(|s| s.trim().parse::<f64>().ok()) else {
            continue;
        };
        if value.is_finite() && value > 0.0 {
            values.push(value);
        }
    }
    if values.len() < 10 {
        return Err(format!(
            "PresentMon returned only {} valid frame times; at least 10 are required.",
            values.len()
        ));
    }
    Ok(values)
}

fn summarize(mut values: Vec<f64>, process: &str, label: &str) -> Capture {
    let frames = values.len();
    let average = values.iter().sum::<f64>() / frames as f64;
    values.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
    let low = |fraction: f64| {
        let count = ((frames as f64 * fraction).floor() as usize)
            .max(1)
            .min(frames);
        1000.0 / (values[..count].iter().sum::<f64>() / count as f64)
    };
    let p99 = values[((frames as f64 * 0.01).floor() as usize).min(frames - 1)];
    let mut ascending = values.clone();
    ascending.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let median = ascending[frames / 2];
    let stutters = values.iter().filter(|v| **v > median * 2.0).count();
    Capture {
        label: label.into(),
        time: format!("{}Z", crate::opti::now_iso()),
        process: process.into(),
        seconds: SECONDS,
        frames,
        average_fps: 1000.0 / average,
        low_1_fps: low(0.01),
        low_01_fps: low(0.001),
        p99_frame_ms: p99,
        median_frame_ms: median,
        stutters,
    }
}

fn tool_paths() -> Result<(PathBuf, PathBuf), String> {
    let dir = state_dir().join("tools");
    fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create PresentMon tool directory: {e}"))?;
    Ok((dir.join("PresentMon.exe"), dir.join("PresentMon.json")))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .build()
        .into()
}

fn presentmon() -> Result<PathBuf, String> {
    let (exe, manifest_path) = tool_paths()?;
    let cached = || -> Result<PathBuf, String> {
        let bytes = fs::read(&manifest_path)
            .map_err(|_| "No verified PresentMon copy is cached.".to_string())?;
        let manifest: ToolManifest = serde_json::from_slice(&bytes)
            .map_err(|e| format!("PresentMon manifest is invalid: {e}"))?;
        crate::update::verify(&exe, &format!("sha256:{}", manifest.digest))?;
        Ok(exe.clone())
    };
    let release = (|| -> Result<(String, String, String), String> {
        let mut response = agent()
            .get("https://api.github.com/repos/GameTechDev/PresentMon/releases/latest")
            .header("User-Agent", "Optimaxer-PresentMon")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| format!("Could not reach PresentMon releases: {e}"))?;
        let mut body = String::new();
        response
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .reader()
            .read_to_string(&mut body)
            .map_err(|e| e.to_string())?;
        let json: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let version = json["tag_name"]
            .as_str()
            .ok_or("PresentMon release has no version.")?
            .to_string();
        let assets = json["assets"]
            .as_array()
            .ok_or("PresentMon release has no assets.")?;
        let asset = assets
            .iter()
            .find(|asset| {
                asset["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("PresentMon-") && n.ends_with("-x64.exe"))
            })
            .ok_or("PresentMon release has no x64 executable.")?;
        let url = asset["browser_download_url"]
            .as_str()
            .ok_or("PresentMon release has no download URL.")?;
        if !url.starts_with("https://") {
            return Err("PresentMon download URL must use HTTPS.".into());
        }
        let digest = asset["digest"]
            .as_str()
            .ok_or("PresentMon release publishes no SHA-256 digest.")?;
        let digest = crate::update::digest_hex(digest)
            .ok_or("PresentMon release SHA-256 digest is malformed.")?;
        let size = asset["size"]
            .as_u64()
            .ok_or("PresentMon release has no asset size.")?;
        if size == 0 || size > MAX_TOOL_BYTES {
            return Err("PresentMon asset exceeds the 64 MiB safety limit or has no size.".into());
        }
        Ok((version, url.to_string(), digest))
    })();
    let (version, url, digest) = match release {
        Ok(release) => release,
        Err(error) => return cached().map_err(|cache_error| format!("{error}. {cache_error}")),
    };
    if exe.exists() && cached().is_ok() {
        let manifest: ToolManifest =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if manifest.digest == digest {
            return Ok(exe);
        }
    }
    let mut response = agent()
        .get(&url)
        .header("User-Agent", "Optimaxer-PresentMon")
        .call()
        .map_err(|e| format!("PresentMon download failed: {e}"))?;
    let part = exe.with_extension("exe.part");
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_TOOL_BYTES)
        .reader();
    let mut output = fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
    }
    drop(output);
    if let Err(error) = crate::update::verify(&part, &format!("sha256:{digest}")) {
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    let metadata = fs::metadata(&part).map_err(|e| e.to_string())?;
    if metadata.len() < 1_000_000 {
        let _ = fs::remove_file(&part);
        return Err("PresentMon download is too small to be a valid executable.".into());
    }
    crate::opti::profiles::replace_file(&part, &exe)
        .map_err(|e| format!("Could not install verified PresentMon: {e}"))?;
    let manifest =
        serde_json::to_vec(&ToolManifest { digest, version }).map_err(|e| e.to_string())?;
    write_atomic(&manifest_path, &manifest)?;
    Ok(exe)
}

pub fn record(process: &str, label: &str) -> Result<Vec<Capture>, String> {
    let process = validate_process_name(process)?.to_string();
    if !matches!(label, "before" | "after") {
        return Err("Capture label must be before or after.".into());
    }
    let exe = presentmon()?;
    let csv = state_dir().join(format!(
        "presentmon-{}-{}.csv",
        std::process::id(),
        crate::opti::now_stamp()
    ));
    let output = Command::new(exe)
        .args(["--process_name", &process, "--output_file"])
        .arg(&csv)
        .args([
            "--timed",
            "30",
            "--terminate_after_timed",
            "--stop_existing_session",
            "--no_console_stats",
            "--session_name",
            "Optimaxer",
            "--v2_metrics",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not start PresentMon: {e}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr)
            .trim()
            .chars()
            .take(600)
            .collect::<String>();
        let _ = fs::remove_file(&csv);
        return Err(if detail.is_empty() {
            format!("PresentMon exited with {}.", output.status)
        } else {
            format!("PresentMon failed: {detail}")
        });
    }
    let result = (|| {
        let metadata = fs::metadata(&csv)
            .map_err(|e| format!("PresentMon did not create a result file: {e}"))?;
        if metadata.len() > MAX_CSV_BYTES {
            return Err("PresentMon output exceeds the 32 MiB safety limit.".into());
        }
        let contents = fs::read_to_string(&csv)
            .map_err(|e| format!("Could not read PresentMon output: {e}"))?;
        let values = parse_frame_times(&contents)?;
        let capture = summarize(values, &process, label);
        let mut captures = load()?;
        captures.push(capture);
        save_path(&store_path(), &captures)?;
        load()
    })();
    let _ = fs::remove_file(csv);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "optimaxer-measurements-{}-{}-{name}.json",
            std::process::id(),
            NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        ))
    }

    #[test]
    fn process_name_is_filename_only_and_exe() {
        assert_eq!(
            validate_process_name("  Game-1.exe ").unwrap(),
            "Game-1.exe"
        );
        for invalid in [
            "game",
            "../game.exe",
            "C:\\game.exe",
            ".exe",
            "game.exe --help",
        ] {
            assert!(validate_process_name(invalid).is_err());
        }
    }

    #[test]
    fn parses_quoted_csv_and_prefers_frame_time() {
        let mut rows = (0..10).map(|_| "\"game, one\",16,99").collect::<Vec<_>>();
        rows.insert(0, "Name,FrameTime,MsBetweenPresents");
        let values = parse_frame_times(&rows.join("\n")).unwrap();
        assert_eq!(values, vec![16.0; 10]);
        assert!(csv_fields("\"broken").is_err());
    }

    #[test]
    fn rejects_bad_or_too_short_csv() {
        assert!(parse_frame_times("Other\n1").is_err());
        assert!(parse_frame_times("FrameTime\n1\n2\n3").is_err());
    }

    #[test]
    fn computes_fps_tails_percentile_median_and_stutters() {
        let mut values = vec![10.0; 100];
        values[0] = 40.0;
        let capture = summarize(values, "game.exe", "before");
        assert_eq!(capture.frames, 100);
        assert!((capture.low_1_fps - 25.0).abs() < 0.001);
        assert!((capture.low_01_fps - 25.0).abs() < 0.001);
        assert!((capture.p99_frame_ms - 10.0).abs() < 0.001);
        assert_eq!(capture.median_frame_ms, 10.0);
        assert_eq!(capture.stutters, 1);
    }

    #[test]
    fn history_is_bounded_and_corruption_is_reported() {
        let path = temp("bounded");
        let sample = summarize(vec![10.0; 10], "game.exe", "before");
        let captures = (0..45).map(|_| sample.clone()).collect::<Vec<_>>();
        save_path(&path, &captures).unwrap();
        assert_eq!(load_path(&path).unwrap().len(), 40);
        fs::write(&path, b"{").unwrap();
        assert!(load_path(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}
