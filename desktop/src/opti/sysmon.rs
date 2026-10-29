//! Live readings for the Overview page: CPU load from GetSystemTimes, memory, system drive, uptime and process
//! count, sampled once a second with a short history for the sparklines. Everything here is read-only and cheap
//! enough to run on the UI thread.

use super::backend::{RegData, System};
use super::hw;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
use windows_sys::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

pub const HISTORY: usize = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// 0 to 100
    pub cpu: f32,
    pub ram_used: u64,
    pub ram_total: u64,
    pub disk_used: u64,
    pub disk_total: u64,
    pub uptime: u64,
    pub processes: u32,
}

impl Snapshot {
    pub fn ram_fraction(&self) -> f32 {
        fraction(self.ram_used, self.ram_total)
    }
    pub fn disk_fraction(&self) -> f32 {
        fraction(self.disk_used, self.disk_total)
    }
}

pub fn fraction(used: u64, total: u64) -> f32 {
    if total == 0 { 0.0 } else { (used as f64 / total as f64).clamp(0.0, 1.0) as f32 }
}

/// CPU load in percent between two (idle, kernel, user) readings. Kernel time includes idle time.
pub fn cpu_percent(prev: (u64, u64, u64), now: (u64, u64, u64)) -> f32 {
    let idle = now.0.wrapping_sub(prev.0);
    let total = now.1.wrapping_sub(prev.1) + now.2.wrapping_sub(prev.2);
    if total == 0 { 0.0 } else { (100.0 * total.saturating_sub(idle) as f64 / total as f64).clamp(0.0, 100.0) as f32 }
}

/// 3d 4h 12m, 4h 12m or 12m.
pub fn format_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h {m}m"),
    }
}

fn ft(f: &FILETIME) -> u64 {
    (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
}

fn read_times() -> Option<(u64, u64, u64)> {
    let (mut i, mut k, mut u) = (FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 });
    // SAFETY: three valid out-pointers.
    (unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } != 0).then(|| (ft(&i), ft(&k), ft(&u)))
}

fn read_memory() -> (u64, u64) {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: dwLength is set and `m` is a valid out-pointer.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return (0, 0);
    }
    (m.ullTotalPhys.saturating_sub(m.ullAvailPhys), m.ullTotalPhys)
}

fn read_disk() -> (u64, u64) {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let path: Vec<u16> = format!("{drive}\\").encode_utf16().chain(std::iter::once(0)).collect();
    let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
    // SAFETY: NUL-terminated path and three valid out-pointers.
    if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut free, &mut total, &mut total_free) } == 0 {
        return (0, 0);
    }
    (total.saturating_sub(total_free), total)
}

fn process_count() -> u32 {
    let mut pids = vec![0u32; 4096];
    let mut bytes = 0u32;
    // SAFETY: the buffer size in bytes is passed and `bytes` receives the amount written.
    if unsafe { EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes) } == 0 { 0 } else { bytes / 4 }
}

pub fn uptime_secs() -> u64 {
    // SAFETY: GetTickCount64 has no pointer parameters.
    unsafe { GetTickCount64() / 1000 }
}

pub struct Sampler {
    last_times: Option<(u64, u64, u64)>,
    last_at: Option<Instant>,
    pub latest: Snapshot,
    pub cpu_history: VecDeque<f32>,
    pub ram_history: VecDeque<f32>,
}

impl Sampler {
    pub fn new() -> Self {
        Sampler { last_times: read_times(), last_at: None, latest: Snapshot::default(), cpu_history: VecDeque::new(), ram_history: VecDeque::new() }
    }

    /// Takes a new reading when a second has passed. Returns true when it did.
    pub fn tick(&mut self) -> bool {
        if self.last_at.is_some_and(|t| t.elapsed() < Duration::from_millis(950)) {
            return false;
        }
        self.last_at = Some(Instant::now());
        let now = read_times();
        let cpu = match (self.last_times, now) {
            (Some(p), Some(n)) => cpu_percent(p, n),
            _ => self.latest.cpu,
        };
        self.last_times = now;
        let (ram_used, ram_total) = read_memory();
        let (disk_used, disk_total) = read_disk();
        self.latest = Snapshot { cpu, ram_used, ram_total, disk_used, disk_total, uptime: uptime_secs(), processes: process_count() };
        push(&mut self.cpu_history, cpu);
        push(&mut self.ram_history, self.latest.ram_fraction() * 100.0);
        true
    }
}

fn push(h: &mut VecDeque<f32>, v: f32) {
    if h.len() >= HISTORY {
        h.pop_front();
    }
    h.push_back(v);
}

/// What does not change while the app runs: the processor name, Windows edition and graphics adapters.
#[derive(Clone, Debug, Default)]
pub struct SysInfo {
    pub cpu: String,
    pub os: String,
    pub gpus: Vec<String>,
    pub threads: u32,
}

pub fn read_info(sys: &dyn System) -> SysInfo {
    let text = |path: &str, name: &str| match sys.reg_get(path, name) {
        Some(RegData::Str(s) | RegData::Expand(s)) => s.trim().to_string(),
        _ => String::new(),
    };
    let cur = r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build: u32 = text(cur, "CurrentBuildNumber").parse().unwrap_or(0);
    let mut name = text(cur, "ProductName");
    // the registry still says "Windows 10" on Windows 11
    if build >= 22_000 {
        name = name.replace("Windows 10", "Windows 11");
    }
    let version = text(cur, "DisplayVersion");
    SysInfo {
        cpu: text(r"HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").split_whitespace().collect::<Vec<_>>().join(" "),
        os: if version.is_empty() { format!("{name} (build {build})") } else { format!("{name} {version} (build {build})") },
        gpus: hw::gpu_names(sys),
        threads: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    fn cpu_percent_comes_from_idle_versus_total() {
        assert_eq!(cpu_percent((0, 0, 0), (50, 100, 0)), 50.0);
        assert_eq!(cpu_percent((0, 0, 0), (0, 100, 0)), 100.0);
        assert_eq!(cpu_percent((0, 0, 0), (100, 100, 0)), 0.0);
        // kernel time already contains idle time, user time is added on top
        assert_eq!(cpu_percent((0, 0, 0), (60, 80, 20)), 40.0);
        assert_eq!(cpu_percent((10, 10, 10), (10, 10, 10)), 0.0, "no time passed");
    }

    #[test]
    fn fractions_and_uptime_format() {
        assert_eq!(fraction(0, 0), 0.0);
        assert_eq!(fraction(50, 200), 0.25);
        assert_eq!(fraction(300, 200), 1.0);
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(3_660), "1h 1m");
        assert_eq!(format_uptime(90_000), "1d 1h 0m");
    }

    #[test]
    fn history_is_capped() {
        let mut h = VecDeque::new();
        for i in 0..(HISTORY + 20) {
            push(&mut h, i as f32);
        }
        assert_eq!(h.len(), HISTORY);
        assert_eq!(*h.front().unwrap(), 20.0);
        assert_eq!(*h.back().unwrap(), (HISTORY + 19) as f32);
    }

    #[test]
    fn windows_11_is_named_correctly() {
        let sys = FakeSystem::default();
        let cur = r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        sys.put(cur, "ProductName", RegData::Str("Windows 10 Pro".into()));
        sys.put(cur, "CurrentBuildNumber", RegData::Str("26100".into()));
        sys.put(cur, "DisplayVersion", RegData::Str("24H2".into()));
        sys.put(r"HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString", RegData::Str("Intel(R) Core(TM)  i7-14700KF".into()));
        let i = read_info(&sys);
        assert_eq!(i.os, "Windows 11 Pro 24H2 (build 26100)");
        assert_eq!(i.cpu, "Intel(R) Core(TM) i7-14700KF");
    }

    #[test]
    fn live_readings_are_sane() {
        let mut s = Sampler::new();
        std::thread::sleep(Duration::from_millis(200));
        assert!(s.tick());
        let n = s.latest;
        assert!(n.ram_total > 1 << 30 && n.ram_used <= n.ram_total);
        assert!(n.disk_total > 0 && n.disk_used <= n.disk_total);
        assert!(n.processes > 10 && n.uptime > 0);
        assert!((0.0..=100.0).contains(&n.cpu));
        assert!(!s.tick(), "a second reading inside a second is skipped");
    }
}
