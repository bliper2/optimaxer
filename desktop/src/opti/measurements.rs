//! Bounded PresentMon captures used for observational before/after comparisons.

use crate::opti::profiles::write_atomic;
use crate::opti::state::state_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE,
    PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhGetFormattedCounterValue, PdhOpenQueryW,
};

const MAX_TOOL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CSV_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FRAMES: usize = 2_000_000;
const MAX_HISTORY: usize = 40;
const SECONDS: u32 = 30;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
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
    #[serde(default)]
    pub dpc_average_percent: Option<f64>,
    #[serde(default)]
    pub dpc_max_percent: Option<f64>,
    #[serde(default)]
    pub interrupt_average_percent: Option<f64>,
    #[serde(default)]
    pub dns_tcp_average_ms: Option<f64>,
    #[serde(default)]
    pub dns_tcp_jitter_ms: Option<f64>,
    #[serde(default)]
    pub dns_tcp_samples: usize,
    #[serde(default)]
    pub dns_tcp_failures: usize,
    #[serde(default)]
    pub timer_resolution_ms: Option<f64>,
    #[serde(default)]
    pub boot_duration_ms: Option<u64>,
    #[serde(default)]
    pub boot_event_time: Option<String>,
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

pub fn latest_pair<'a>(
    captures: &'a [Capture],
    process: &str,
) -> (Option<&'a Capture>, Option<&'a Capture>) {
    let matching_process = |capture: &&Capture| capture.process.eq_ignore_ascii_case(process);
    let before = captures
        .iter()
        .rev()
        .find(|capture| capture.label == "before" && matching_process(capture));
    let after = captures
        .iter()
        .rev()
        .find(|capture| capture.label == "after" && matching_process(capture));
    (before, after)
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
        ..Capture::default()
    }
}

struct SystemLatencyCounters {
    query: PDH_HQUERY,
    dpc: PDH_HCOUNTER,
    interrupt: PDH_HCOUNTER,
}

#[derive(Default)]
struct SystemLatencyReading {
    dpc_average_percent: Option<f64>,
    dpc_max_percent: Option<f64>,
    interrupt_average_percent: Option<f64>,
    dns_tcp_average_ms: Option<f64>,
    dns_tcp_jitter_ms: Option<f64>,
    dns_tcp_samples: usize,
    dns_tcp_failures: usize,
    timer_resolution_ms: Option<f64>,
    boot_duration_ms: Option<u64>,
    boot_event_time: Option<String>,
}

impl SystemLatencyCounters {
    fn open() -> Option<Self> {
        let mut query = std::ptr::null_mut();
        // SAFETY: query is a valid output pointer and null opens a live local PDH query.
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) } != 0 {
            return None;
        }
        let add = |path: &str| {
            let path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut counter = std::ptr::null_mut();
            // SAFETY: query is open, path is NUL-terminated, and counter is a valid output pointer.
            (unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) } == 0)
                .then_some(counter)
        };
        let dpc = add(r"\Processor(_Total)\% DPC Time");
        let interrupt = add(r"\Processor(_Total)\% Interrupt Time");
        match (dpc, interrupt) {
            (Some(dpc), Some(interrupt)) => Some(Self {
                query,
                dpc,
                interrupt,
            }),
            _ => {
                // SAFETY: query was opened above; any partially added counters are owned by it.
                unsafe { PdhCloseQuery(query) };
                None
            }
        }
    }

    fn sample(&self) -> (Option<f64>, Option<f64>) {
        // SAFETY: query remains owned until Drop.
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return (None, None);
        }
        (read_counter(self.dpc), read_counter(self.interrupt))
    }
}

impl Drop for SystemLatencyCounters {
    fn drop(&mut self) {
        // SAFETY: query was opened by open and closes exactly once.
        unsafe { PdhCloseQuery(self.query) };
    }
}

fn read_counter(counter: PDH_HCOUNTER) -> Option<f64> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    // SAFETY: counter belongs to a live query and value is a valid output structure.
    if unsafe {
        PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut value)
    } != 0
        || !matches!(value.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA)
    {
        return None;
    }
    // SAFETY: PDH_FMT_DOUBLE requests the doubleValue union member.
    let value = unsafe { value.Anonymous.doubleValue };
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn timer_resolution_ms() -> Option<f64> {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryTimerResolution(minimum: *mut u32, maximum: *mut u32, current: *mut u32) -> i32;
    }
    let (mut minimum, mut maximum, mut current) = (0, 0, 0);
    // SAFETY: all output pointers refer to initialized local storage.
    let status = unsafe { NtQueryTimerResolution(&mut minimum, &mut maximum, &mut current) };
    (status >= 0 && current > 0).then_some(f64::from(current) / 10_000.0)
}

fn xml_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(index) = rest.find(name) {
        if index > 0 && rest.as_bytes()[index - 1].is_ascii_alphanumeric() {
            rest = &rest[index + name.len()..];
            continue;
        }
        let after_name = rest[index + name.len()..].trim_start();
        let after_equals = after_name.strip_prefix('=')?.trim_start();
        let quote = after_equals.chars().next()?;
        if !matches!(quote, '\'' | '"') {
            return None;
        }
        let value = &after_equals[quote.len_utf8()..];
        let end = value.find(quote)?;
        return Some(&value[..end]);
    }
    None
}

fn parse_boot_event(xml: &str) -> Option<(u64, String)> {
    let mut rest = xml;
    let mut boot_duration_ms = None;
    while let Some(index) = rest.find("<Data") {
        let tag = &rest[index..];
        let header_end = tag.find('>')?;
        let header = &tag[..header_end];
        if xml_attribute(header, "Name") == Some("BootTime") {
            let value = tag[header_end + 1..].split_once("</Data>")?.0.trim();
            boot_duration_ms = value.parse::<u64>().ok();
            break;
        }
        rest = &tag[header_end + 1..];
    }
    let boot_duration_ms = boot_duration_ms?;
    let time_tag = xml.split("<TimeCreated").nth(1)?.split('>').next()?;
    let boot_event_time = xml_attribute(time_tag, "SystemTime")?.to_string();
    Some((boot_duration_ms, boot_event_time))
}

fn last_boot_event() -> Option<(u64, String)> {
    let output = Command::new("wevtutil.exe")
        .args([
            "qe",
            "Microsoft-Windows-Diagnostics-Performance/Operational",
            "/q:*[System[(EventID=100)]]",
            "/rd:true",
            "/c:1",
            "/f:xml",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 256 * 1024 {
        return None;
    }
    parse_boot_event(&String::from_utf8_lossy(&output.stdout))
}

fn dns_tcp_samples() -> (Option<f64>, Option<f64>, usize, usize) {
    let address: SocketAddr = match "1.1.1.1:53".parse() {
        Ok(address) => address,
        Err(_) => return (None, None, 0, 10),
    };
    let mut times = Vec::new();
    let mut failures = 0;
    for _ in 0..10 {
        let started = Instant::now();
        match TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
            Ok(_) => times.push(started.elapsed().as_secs_f64() * 1000.0),
            Err(_) => failures += 1,
        }
    }
    summarize_tcp_samples(&times, times.len() + failures)
}

fn summarize_tcp_samples(
    times: &[f64],
    attempts: usize,
) -> (Option<f64>, Option<f64>, usize, usize) {
    let valid = times
        .iter()
        .copied()
        .filter(|time| time.is_finite() && *time >= 0.0)
        .collect::<Vec<_>>();
    let average = (!valid.is_empty()).then(|| valid.iter().sum::<f64>() / valid.len() as f64);
    let jitter = (valid.len() > 1).then(|| {
        valid
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .sum::<f64>()
            / (valid.len() - 1) as f64
    });
    (
        average,
        jitter,
        valid.len(),
        attempts.saturating_sub(valid.len()),
    )
}

fn sample_system_latency() -> SystemLatencyReading {
    let timer_resolution_ms = timer_resolution_ms();
    let (dpc, interrupt) = if let Some(counters) = SystemLatencyCounters::open() {
        let (mut dpc, mut interrupt) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            std::thread::sleep(Duration::from_secs(1));
            let (dpc_value, interrupt_value) = counters.sample();
            if let Some(value) = dpc_value {
                dpc.push(value);
            }
            if let Some(value) = interrupt_value {
                interrupt.push(value);
            }
        }
        (dpc, interrupt)
    } else {
        (Vec::new(), Vec::new())
    };
    let (dns_tcp_average_ms, dns_tcp_jitter_ms, dns_tcp_samples, dns_tcp_failures) =
        dns_tcp_samples();
    let (boot_duration_ms, boot_event_time) = last_boot_event()
        .map_or((None, None), |(duration, time)| {
            (Some(duration), Some(time))
        });
    let average = |values: &[f64]| {
        (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
    };
    SystemLatencyReading {
        dpc_average_percent: average(&dpc),
        dpc_max_percent: dpc.iter().copied().reduce(f64::max),
        interrupt_average_percent: average(&interrupt),
        dns_tcp_average_ms,
        dns_tcp_jitter_ms,
        dns_tcp_samples,
        dns_tcp_failures,
        timer_resolution_ms,
        boot_duration_ms,
        boot_event_time,
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
        let mut capture = summarize(values, &process, label);
        let system = sample_system_latency();
        capture.dpc_average_percent = system.dpc_average_percent;
        capture.dpc_max_percent = system.dpc_max_percent;
        capture.interrupt_average_percent = system.interrupt_average_percent;
        capture.dns_tcp_average_ms = system.dns_tcp_average_ms;
        capture.dns_tcp_jitter_ms = system.dns_tcp_jitter_ms;
        capture.dns_tcp_samples = system.dns_tcp_samples;
        capture.dns_tcp_failures = system.dns_tcp_failures;
        capture.timer_resolution_ms = system.timer_resolution_ms;
        capture.boot_duration_ms = system.boot_duration_ms;
        capture.boot_event_time = system.boot_event_time;
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

    #[test]
    fn old_capture_records_default_new_system_readings_to_unavailable() {
        let mut value =
            serde_json::to_value(summarize(vec![10.0; 10], "game.exe", "before")).unwrap();
        let object = value.as_object_mut().unwrap();
        for name in [
            "dpc_average_percent",
            "dpc_max_percent",
            "interrupt_average_percent",
            "dns_tcp_average_ms",
            "dns_tcp_jitter_ms",
            "dns_tcp_samples",
            "dns_tcp_failures",
            "timer_resolution_ms",
        ] {
            object.remove(name);
        }
        let capture: Capture = serde_json::from_value(value).unwrap();
        assert!(capture.dpc_average_percent.is_none());
        assert!(capture.timer_resolution_ms.is_none());
        assert_eq!(capture.dns_tcp_samples, 0);
    }

    #[test]
    fn tcp_connect_metrics_report_average_jitter_and_failed_attempts() {
        let (average, jitter, successes, failures) = summarize_tcp_samples(&[10.0, 15.0, 20.0], 5);
        assert_eq!(average, Some(15.0));
        assert_eq!(jitter, Some(5.0));
        assert_eq!((successes, failures), (3, 2));
        assert_eq!(summarize_tcp_samples(&[], 10), (None, None, 0, 10));
    }

    #[test]
    fn boot_event_parser_requires_duration_and_event_timestamp() {
        let xml = "<Event><System><TimeCreated SystemTime='2026-10-30T04:00:00.000Z'/></System><EventData><Data Name='BootTime'>12345</Data></EventData></Event>";
        assert_eq!(
            parse_boot_event(xml),
            Some((12_345, "2026-10-30T04:00:00.000Z".into()))
        );
        assert!(parse_boot_event("<Event><Data Name=\"BootTime\">bad</Data></Event>").is_none());
    }

    #[test]
    fn latest_pair_matches_process_name_without_crossing_games() {
        let mut before = summarize(vec![10.0; 10], "game.exe", "before");
        before.time = "2026-10-30T10:00:00Z".into();
        let mut other_game = summarize(vec![10.0; 10], "other.exe", "after");
        other_game.time = "2026-10-30T10:01:00Z".into();
        let mut after = summarize(vec![10.0; 10], "GAME.EXE", "after");
        after.time = "2026-10-30T10:02:00Z".into();
        let captures = vec![before, other_game, after];
        let (before, after) = latest_pair(&captures, "game.exe");
        assert_eq!(before.unwrap().label, "before");
        assert_eq!(after.unwrap().process, "GAME.EXE");
        assert_eq!(latest_pair(&captures, "missing.exe"), (None, None));
    }
}
