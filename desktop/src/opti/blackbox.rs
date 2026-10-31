//! Performance black box: a bounded, opt-in flight recorder. While enabled, it samples the same Windows counters the
//! live charts use at a slow interval and keeps a rolling window on disk. Readings Windows does not provide are stored
//! as missing, never as zero. Nothing leaves the machine; the history can be exported or deleted at any time.

use super::sysmon::{Sampler, Snapshot};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Rolling window: 24 hours at the default 10 second interval, 72 hours at 30 seconds.
pub const MAX_SAMPLES: usize = 8_640;
pub const MAX_BOOKMARKS: usize = 100;
pub const MAX_NOTE_CHARS: usize = 120;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const SAVE_EVERY: Duration = Duration::from_secs(60);

pub const INTERVAL_SECONDS: [u64; 3] = [5, 10, 30];
pub const DEFAULT_INTERVAL_SECONDS: u64 = 10;

/// Share of CPU or memory that counts as saturated, and how long it must last to be reported.
pub const SATURATION_PERCENT: f32 = 90.0;
pub const SUSTAINED_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Unix seconds.
    pub t: u64,
    pub cpu: Option<f32>,
    /// Percent of physical memory in use.
    pub ram: Option<f32>,
    /// Physical-disk read plus write, bytes per second.
    pub disk: Option<f32>,
    pub down: Option<f32>,
    pub up: Option<f32>,
    pub gpu: Option<f32>,
}

impl Sample {
    pub fn from_snapshot(t: u64, s: &Snapshot) -> Self {
        Self {
            t,
            cpu: s.cpu_available.then_some(s.cpu),
            ram: s.ram_available.then_some(s.ram_fraction() * 100.0),
            disk: s.disk_activity,
            down: s.network.map(|n| n.received_bytes_per_sec),
            up: s.network.map(|n| n.sent_bytes_per_sec),
            gpu: s.gpu_busiest_engine,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub t: u64,
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Store {
    pub samples: VecDeque<Sample>,
    pub bookmarks: Vec<Bookmark>,
}

impl Store {
    pub fn push(&mut self, sample: Sample) {
        self.samples.push_back(sample);
        while self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
    }

    pub fn add_bookmark(&mut self, t: u64, note: &str) {
        let note: String = note.trim().chars().take(MAX_NOTE_CHARS).collect();
        self.bookmarks.push(Bookmark {
            t,
            note: if note.is_empty() {
                "Marked".into()
            } else {
                note
            },
        });
        let excess = self.bookmarks.len().saturating_sub(MAX_BOOKMARKS);
        self.bookmarks.drain(..excess);
    }

    pub fn between(&self, from: u64, to: u64) -> Vec<Sample> {
        self.samples
            .iter()
            .filter(|s| s.t >= from && s.t <= to)
            .copied()
            .collect()
    }
}

pub fn load(path: &Path) -> Result<Store, String> {
    if !path.exists() {
        return Ok(Store::default());
    }
    let size = fs::metadata(path)
        .map_err(|e| format!("Could not inspect the black box history: {e}"))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(format!(
            "The black box history exceeds the {MAX_FILE_BYTES}-byte safety limit"
        ));
    }
    let bytes = fs::read(path).map_err(|e| format!("Could not read the black box history: {e}"))?;
    let mut store: Store = serde_json::from_slice(&bytes)
        .map_err(|e| format!("The black box history is unreadable: {e}"))?;
    while store.samples.len() > MAX_SAMPLES {
        store.samples.pop_front();
    }
    let excess = store.bookmarks.len().saturating_sub(MAX_BOOKMARKS);
    store.bookmarks.drain(..excess);
    Ok(store)
}

pub fn save(path: &Path, store: &Store) -> Result<(), String> {
    let bytes = serde_json::to_vec(store).map_err(|e| e.to_string())?;
    crate::net::write_atomic(path, &bytes)
}

pub fn default_path() -> PathBuf {
    super::state::state_dir().join("blackbox.json")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub average: f32,
    pub peak: f32,
    pub peak_at: u64,
    pub samples: usize,
}

/// Average and peak of one metric. `None` when the window holds no reading of it.
pub fn summarize(samples: &[Sample], pick: impl Fn(&Sample) -> Option<f32>) -> Option<Summary> {
    let mut total = 0.0f64;
    let mut count = 0usize;
    let mut peak: Option<(f32, u64)> = None;
    for sample in samples {
        let Some(value) = pick(sample) else { continue };
        total += f64::from(value);
        count += 1;
        if peak.is_none_or(|(best, _)| value > best) {
            peak = Some((value, sample.t));
        }
    }
    let (peak, peak_at) = peak?;
    Some(Summary {
        average: (total / count as f64) as f32,
        peak,
        peak_at,
        samples: count,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Incident {
    pub metric: &'static str,
    pub start: u64,
    pub end: u64,
    pub peak: f32,
}

/// Periods where CPU or memory stayed at or above [`SATURATION_PERCENT`] for at least [`SUSTAINED_SECONDS`].
/// A gap longer than three sampling intervals ends a period, because nothing was recorded across it.
pub fn incidents(samples: &[Sample], interval_seconds: u64) -> Vec<Incident> {
    let max_gap = interval_seconds.max(1) * 3;
    let mut found = Vec::new();
    type Pick = fn(&Sample) -> Option<f32>;
    let metrics: [(&'static str, Pick); 2] = [("Processor", |s| s.cpu), ("Memory", |s| s.ram)];
    for (metric, pick) in metrics {
        let mut run: Option<Incident> = None;
        let close = |run: &mut Option<Incident>, found: &mut Vec<Incident>| {
            if let Some(done) = run.take()
                && done.end.saturating_sub(done.start) + interval_seconds >= SUSTAINED_SECONDS
            {
                found.push(done);
            }
        };
        for sample in samples {
            match (pick(sample), run.as_mut()) {
                (Some(v), Some(open))
                    if v >= SATURATION_PERCENT && sample.t.saturating_sub(open.end) <= max_gap =>
                {
                    open.end = sample.t;
                    open.peak = open.peak.max(v);
                }
                (value, _) => {
                    close(&mut run, &mut found);
                    if let Some(v) = value.filter(|v| *v >= SATURATION_PERCENT) {
                        run = Some(Incident {
                            metric,
                            start: sample.t,
                            end: sample.t,
                            peak: v,
                        });
                    }
                }
            }
        }
        close(&mut run, &mut found);
    }
    found.sort_by_key(|incident| incident.start);
    found
}

/// Reduce a window to `columns` buckets for a chart, keeping each bucket's peak so short spikes stay visible.
/// A bucket with no reading is `None`, which the charts draw as a gap.
pub fn buckets(
    samples: &[Sample],
    from: u64,
    to: u64,
    columns: usize,
    pick: impl Fn(&Sample) -> Option<f32>,
) -> VecDeque<Option<f32>> {
    let columns = columns.max(2);
    let span = to.saturating_sub(from).max(1) as f64;
    let mut out: VecDeque<Option<f32>> = vec![None; columns].into();
    for sample in samples.iter().filter(|s| s.t >= from && s.t <= to) {
        let Some(value) = pick(sample) else { continue };
        let index = (((sample.t - from) as f64 / span) * (columns - 1) as f64).round() as usize;
        let slot = &mut out[index.min(columns - 1)];
        *slot = Some(slot.map_or(value, |old| old.max(value)));
    }
    out
}

fn csv_cell(value: Option<f32>) -> String {
    value.map_or_else(String::new, |v| format!("{v:.1}"))
}

/// Raw samples for a spreadsheet. Empty cells are readings Windows did not provide.
pub fn csv(samples: &[Sample]) -> String {
    let mut out = String::from(
        "time_utc,cpu_percent,memory_percent,disk_bytes_per_sec,net_down_bytes_per_sec,net_up_bytes_per_sec,gpu_engine_percent\n",
    );
    for s in samples {
        out.push_str(&format!(
            "{}Z,{},{},{},{},{},{}\n",
            super::iso_utc(s.t),
            csv_cell(s.cpu),
            csv_cell(s.ram),
            csv_cell(s.disk),
            csv_cell(s.down),
            csv_cell(s.up),
            csv_cell(s.gpu),
        ));
    }
    out
}

/// A readable report of one period: what was recorded, what stood out, and what is unknown.
pub fn report(store: &Store, from: u64, to: u64, interval_seconds: u64) -> String {
    let samples = store.between(from, to);
    let mut out = format!(
        "# Optimaxer black box report\n\nPeriod: {}Z to {}Z (UTC)\nSamples: {} at roughly {} s intervals; each reading is an average or instantaneous value from Windows counters.\n\n",
        super::iso_utc(from),
        super::iso_utc(to),
        samples.len(),
        interval_seconds
    );
    out.push_str("## Summary\n\n");
    type Pick = fn(&Sample) -> Option<f32>;
    let rows: [(&str, &str, Pick); 6] = [
        ("Processor", "%", |s| s.cpu),
        ("Memory in use", "%", |s| s.ram),
        ("Disk throughput", " B/s", |s| s.disk),
        ("Network received", " B/s", |s| s.down),
        ("Network sent", " B/s", |s| s.up),
        ("GPU busiest engine", "%", |s| s.gpu),
    ];
    for (name, unit, pick) in rows {
        match summarize(&samples, pick) {
            Some(s) => out.push_str(&format!(
                "- {name}: average {:.1}{unit}, peak {:.1}{unit} at {}Z ({} readings)\n",
                s.average,
                s.peak,
                super::iso_utc(s.peak_at),
                s.samples
            )),
            None => out.push_str(&format!("- {name}: no readings in this period\n")),
        }
    }
    out.push_str("\n## Sustained saturation\n\n");
    let found = incidents(&samples, interval_seconds);
    if found.is_empty() {
        out.push_str(&format!(
            "None. Neither processor nor memory stayed at or above {SATURATION_PERCENT:.0}% for {SUSTAINED_SECONDS} seconds.\n"
        ));
    }
    for i in &found {
        out.push_str(&format!(
            "- {} at or above {SATURATION_PERCENT:.0}% from {}Z to {}Z, peak {:.0}%\n",
            i.metric,
            super::iso_utc(i.start),
            super::iso_utc(i.end),
            i.peak
        ));
    }
    out.push_str("\n## Bookmarks\n\n");
    let marks: Vec<_> = store
        .bookmarks
        .iter()
        .filter(|b| b.t >= from && b.t <= to)
        .collect();
    if marks.is_empty() {
        out.push_str("None in this period.\n");
    }
    for b in marks {
        out.push_str(&format!("- {}Z: {}\n", super::iso_utc(b.t), b.note));
    }
    out.push_str(
        "\nThese are observations, not diagnoses. A busy counter shows that something used the resource, not which program or why.\n",
    );
    out
}

/// A Windows System-log entry that falls inside a recorded period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinEvent {
    pub time: String,
    pub level: &'static str,
    pub provider: String,
    pub id: u32,
}

const MAX_EVENT_OUTPUT: usize = 512 * 1024;
const MAX_EVENTS: usize = 40;

/// Critical, error and warning entries from the System log between two unix times, oldest first. The event text is
/// not fetched; provider and ID are enough to look an entry up and do not depend on the Windows display language.
pub fn windows_events(from: u64, to: u64) -> Result<Vec<WinEvent>, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let query = format!(
        "*[System[(Level=1 or Level=2 or Level=3) and TimeCreated[@SystemTime>='{}.000Z' and @SystemTime<='{}.000Z']]]",
        super::iso_utc(from),
        super::iso_utc(to)
    );
    let output = std::process::Command::new("wevtutil.exe")
        .args(["qe", "System"])
        .arg(format!("/q:{query}"))
        .args([&format!("/c:{MAX_EVENTS}"), "/rd:false", "/f:xml"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Could not start the Windows event reader: {e}"))?;
    if !output.status.success() {
        return Err("Windows would not return the System log for this period.".into());
    }
    if output.stdout.len() > MAX_EVENT_OUTPUT {
        return Err("The event output was larger than the safety limit.".into());
    }
    Ok(parse_events(&String::from_utf8_lossy(&output.stdout)))
}

fn attribute<'a>(block: &'a str, tag: &str, name: &str) -> Option<&'a str> {
    let element = &block[block.find(&format!("<{tag} "))?..];
    let element = &element[..element.find('>')?];
    let after = &element[element.find(&format!("{name}="))? + name.len() + 1..];
    let quote = after.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let rest = &after[1..];
    Some(&rest[..rest.find(quote)?])
}

pub fn parse_events(xml: &str) -> Vec<WinEvent> {
    xml.split("<Event ")
        .skip(1)
        .filter_map(|block| {
            let block = &block[..block.find("</Event>").unwrap_or(block.len())];
            let id_start = block.find("<EventID")?;
            let id_text = &block[block[id_start..].find('>')? + id_start + 1..];
            let id = id_text[..id_text.find('<')?].trim().parse().ok()?;
            let level = match attribute(block, "Level", "x").or_else(|| {
                let start = block.find("<Level>")? + 7;
                Some(&block[start..start + block[start..].find('<')?])
            })? {
                "1" => "Critical",
                "2" => "Error",
                "3" => "Warning",
                _ => return None,
            };
            Some(WinEvent {
                time: attribute(block, "TimeCreated", "SystemTime")?
                    .trim_end_matches('Z')
                    .replace('T', " ")
                    .chars()
                    .take(19)
                    .collect(),
                level,
                provider: attribute(block, "Provider", "Name")?.to_owned(),
                id,
            })
        })
        .collect()
}

struct Shared {
    store: Mutex<Store>,
    running: AtomicBool,
    interval_millis: AtomicU64,
    last_error: Mutex<Option<String>>,
}

/// Owns the sampling thread. Dropping it stops the thread and saves the history.
pub struct Recorder {
    shared: Arc<Shared>,
    path: PathBuf,
    handle: Option<JoinHandle<()>>,
}

impl Recorder {
    /// Loads the saved history and starts sampling. A history that cannot be read is moved aside, not overwritten.
    pub fn start(path: PathBuf, interval: Duration) -> std::io::Result<Self> {
        let mut last_error = None;
        let store = match load(&path) {
            Ok(store) => store,
            Err(error) => {
                let aside = path.with_extension("unreadable");
                let _ = fs::rename(&path, &aside);
                last_error = Some(format!("{error}. It was kept as {}.", aside.display()));
                Store::default()
            }
        };
        let shared = Arc::new(Shared {
            store: Mutex::new(store),
            running: AtomicBool::new(true),
            interval_millis: AtomicU64::new(interval.as_millis() as u64),
            last_error: Mutex::new(last_error),
        });
        let worker = shared.clone();
        let worker_path = path.clone();
        let handle = std::thread::Builder::new()
            .name("optimaxer-blackbox".into())
            .spawn(move || record_loop(&worker, &worker_path))?;
        Ok(Self {
            shared,
            path,
            handle: Some(handle),
        })
    }

    pub fn set_interval(&self, interval: Duration) {
        self.shared
            .interval_millis
            .store(interval.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Store {
        lock(&self.shared.store).clone()
    }

    pub fn bookmark(&self, note: &str) {
        lock(&self.shared.store).add_bookmark(super::now_secs(), note);
    }

    pub fn error(&self) -> Option<String> {
        lock(&self.shared.last_error).clone()
    }

    /// Stops sampling, saves, and waits for the thread.
    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    /// Empties the history in memory and removes the saved file.
    pub fn delete_history(&self) -> Result<(), String> {
        *lock(&self.shared.store) = Store::default();
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Could not delete the black box history: {e}")),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn record_loop(shared: &Shared, path: &Path) {
    let mut sampler = Sampler::new();
    let mut last_save = Instant::now();
    let mut dirty = false;
    let mut next = Instant::now();
    while shared.running.load(Ordering::Relaxed) {
        if Instant::now() >= next {
            sampler.sample();
            let sample = Sample::from_snapshot(super::now_secs(), &sampler.latest);
            lock(&shared.store).push(sample);
            dirty = true;
            next = Instant::now()
                + Duration::from_millis(shared.interval_millis.load(Ordering::Relaxed));
        }
        if dirty && last_save.elapsed() >= SAVE_EVERY {
            persist(shared, path);
            dirty = false;
            last_save = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if dirty {
        persist(shared, path);
    }
}

fn persist(shared: &Shared, path: &Path) {
    let copy = lock(&shared.store).clone();
    *lock(&shared.last_error) = save(path, &copy).err();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t: u64, cpu: Option<f32>, ram: Option<f32>) -> Sample {
        Sample {
            t,
            cpu,
            ram,
            ..Sample::default()
        }
    }

    #[test]
    fn history_is_bounded_and_keeps_the_newest_samples() {
        let mut store = Store::default();
        for t in 0..(MAX_SAMPLES as u64 + 25) {
            store.push(at(t, Some(1.0), None));
        }
        assert_eq!(store.samples.len(), MAX_SAMPLES);
        assert_eq!(store.samples.front().unwrap().t, 25);
    }

    #[test]
    fn bookmarks_are_trimmed_bounded_and_never_empty() {
        let mut store = Store::default();
        store.add_bookmark(1, "   ");
        assert_eq!(store.bookmarks[0].note, "Marked");
        store.add_bookmark(2, &"x".repeat(500));
        assert_eq!(store.bookmarks[1].note.chars().count(), MAX_NOTE_CHARS);
        for t in 0..(MAX_BOOKMARKS as u64 + 5) {
            store.add_bookmark(t, "m");
        }
        assert_eq!(store.bookmarks.len(), MAX_BOOKMARKS);
    }

    #[test]
    fn summaries_skip_missing_readings_and_report_none_when_empty() {
        let samples = [
            at(1, Some(10.0), None),
            at(2, None, None),
            at(3, Some(30.0), None),
        ];
        let cpu = summarize(&samples, |s| s.cpu).unwrap();
        assert_eq!(
            (cpu.average, cpu.peak, cpu.peak_at, cpu.samples),
            (20.0, 30.0, 3, 2)
        );
        assert!(summarize(&samples, |s| s.ram).is_none());
    }

    #[test]
    fn only_sustained_saturation_is_an_incident() {
        let mut samples: Vec<Sample> = (0..4).map(|i| at(i * 10, Some(99.0), Some(40.0))).collect();
        samples.extend((4..20).map(|i| at(i * 10, Some(10.0), Some(40.0))));
        assert!(
            incidents(&samples, 10).is_empty(),
            "40 s is below the minimum"
        );
        let long: Vec<Sample> = (0..8).map(|i| at(i * 10, Some(95.0), None)).collect();
        let found = incidents(&long, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].metric, found[0].start, found[0].end),
            ("Processor", 0, 70)
        );
    }

    #[test]
    fn a_recording_gap_splits_an_incident() {
        let mut samples: Vec<Sample> = (0..7).map(|i| at(i * 10, Some(95.0), None)).collect();
        samples.extend((0..7).map(|i| at(1_000 + i * 10, Some(95.0), None)));
        assert_eq!(incidents(&samples, 10).len(), 2);
    }

    #[test]
    fn missing_readings_do_not_count_as_saturation_or_zero() {
        let samples: Vec<Sample> = (0..10).map(|i| at(i * 10, None, None)).collect();
        assert!(incidents(&samples, 10).is_empty());
        let columns = buckets(&samples, 0, 90, 5, |s| s.cpu);
        assert!(columns.iter().all(Option::is_none));
    }

    #[test]
    fn buckets_keep_the_peak_and_leave_gaps_empty() {
        let samples = [
            at(0, Some(10.0), None),
            at(1, Some(80.0), None),
            at(100, Some(5.0), None),
        ];
        let columns = buckets(&samples, 0, 100, 3, |s| s.cpu);
        assert_eq!(columns, VecDeque::from([Some(80.0), None, Some(5.0)]));
    }

    #[test]
    fn history_round_trips_and_a_corrupt_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("optimaxer-bb-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blackbox.json");
        let mut store = Store::default();
        store.push(at(5, Some(1.5), Some(2.5)));
        store.add_bookmark(6, "slow");
        save(&path, &store).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.samples.len(), 1);
        assert_eq!(loaded.bookmarks[0].note, "slow");
        fs::write(&path, b"not json").unwrap();
        assert!(load(&path).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exports_state_unknowns_instead_of_inventing_values() {
        let mut store = Store::default();
        store.push(at(60, Some(50.0), None));
        let text = report(&store, 0, 120, 10);
        assert!(text.contains("Memory in use: no readings in this period"));
        assert!(text.contains("observations, not diagnoses"));
        let table = csv(&store.between(0, 120));
        assert!(table.lines().nth(1).unwrap().contains(",50.0,,"));
    }

    #[test]
    fn event_xml_is_parsed_without_depending_on_display_language() {
        let xml = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>            <Provider Name='Microsoft-Windows-Kernel-Power' Guid='{x}'/><EventID>41</EventID><Level>1</Level>            <TimeCreated SystemTime='2026-10-30T12:00:01.123456700Z'/></System></Event>            <Event xmlns='y'><System><Provider Name='disk'/><EventID Qualifiers='49152'>7</EventID><Level>2</Level>            <TimeCreated SystemTime='2026-10-30T12:05:00.000Z'/></System></Event>";
        let events = parse_events(xml);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            WinEvent {
                time: "2026-10-30 12:00:01".into(),
                level: "Critical",
                provider: "Microsoft-Windows-Kernel-Power".into(),
                id: 41
            }
        );
        assert_eq!((events[1].level, events[1].id), ("Error", 7));
        assert!(parse_events("not xml").is_empty());
    }

    #[test]
    fn recorder_samples_saves_and_deletes_history() {
        let dir = std::env::temp_dir().join(format!("optimaxer-rec-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blackbox.json");
        let mut recorder = Recorder::start(path.clone(), Duration::from_millis(50)).unwrap();
        std::thread::sleep(Duration::from_millis(600));
        recorder.bookmark("test");
        assert!(!recorder.snapshot().samples.is_empty());
        recorder.stop();
        assert!(load(&path).unwrap().samples.len() > 1);
        recorder.delete_history().unwrap();
        assert!(!path.exists());
        assert!(recorder.snapshot().samples.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
