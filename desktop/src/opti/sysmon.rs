//! Live readings for the Overview page: CPU load from GetSystemTimes, memory, system drive, uptime and process
//! count, sampled once a second with a short history for the sparklines. Everything here is read-only and cheap
//! enough to run on the UI thread.

use super::backend::{RegData, System};
use super::hw;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2, MIB_IF_TYPE_LOOPBACK,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_IF_OPER_STATUS_UP;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
use windows_sys::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

pub const HISTORY: usize = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetworkSample {
    pub received_bytes_per_sec: f32,
    pub sent_bytes_per_sec: f32,
}

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
    pub network: Option<NetworkSample>,
    pub network_available: bool,
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
    pub network_history: VecDeque<Option<NetworkSample>>,
    last_network_totals: Option<(u64, u64)>,
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            last_times: read_times(),
            last_at: None,
            latest: Snapshot::default(),
            cpu_history: VecDeque::new(),
            ram_history: VecDeque::new(),
            network_history: VecDeque::new(),
            last_network_totals: read_network_totals(),
        }
    }

    /// Takes a new reading when a second has passed. Returns true when it did.
    pub fn tick(&mut self) -> bool {
        if self.last_at.is_some_and(|t| t.elapsed() < Duration::from_millis(950)) {
            return false;
        }
        let sample_elapsed = self.last_at.map(|at| at.elapsed().as_secs_f64()).unwrap_or(0.0);
        self.last_at = Some(Instant::now());
        let now = read_times();
        let cpu = match (self.last_times, now) {
            (Some(p), Some(n)) => cpu_percent(p, n),
            _ => self.latest.cpu,
        };
        self.last_times = now;
        let (ram_used, ram_total) = read_memory();
        let (disk_used, disk_total) = read_disk();
        let network_totals = read_network_totals();
        let network = network_totals
            .zip(self.last_network_totals)
            .and_then(|(now, previous)| network_rate(previous, now, sample_elapsed));
        if let Some(totals) = network_totals {
            self.last_network_totals = Some(totals);
        }
        self.latest = Snapshot {
            cpu, ram_used, ram_total, disk_used, disk_total, uptime: uptime_secs(), processes: process_count(),
            network, network_available: network_totals.is_some(),
        };
        push(&mut self.cpu_history, cpu);
        push(&mut self.ram_history, self.latest.ram_fraction() * 100.0);
        push_network(&mut self.network_history, network);
        true
    }

    /// Start a fresh chart window when the Overview becomes visible again.
    pub fn reset_window(&mut self) {
        self.last_times = read_times();
        self.last_at = None;
        self.latest.network = None;
        self.latest.network_available = false;
        self.cpu_history.clear();
        self.ram_history.clear();
        self.network_history.clear();
        self.last_network_totals = read_network_totals();
    }
}

fn push(h: &mut VecDeque<f32>, v: f32) {
    if h.len() >= HISTORY {
        h.pop_front();
    }
    h.push_back(v);
}

fn push_network(h: &mut VecDeque<Option<NetworkSample>>, value: Option<NetworkSample>) {
    if h.len() >= HISTORY {
        h.pop_front();
    }
    h.push_back(value);
}

/// Aggregate counters from active non-loopback adapters. These are adapter counters, not internet throughput.
fn read_network_totals() -> Option<(u64, u64)> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: `table` is a valid out-pointer. Windows owns the returned table until FreeMibTable.
    if unsafe { GetIfTable2(&mut table) } != 0 || table.is_null() {
        return None;
    }
    struct MibTable(*mut MIB_IF_TABLE2);
    impl Drop for MibTable {
        fn drop(&mut self) {
            // SAFETY: GetIfTable2 allocated this table and it is freed exactly once.
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }
    let table = MibTable(table);
    // SAFETY: GetIfTable2 guarantees `NumEntries` rows follow the fixed table header.
    let (rows, count) = unsafe {
        let count = (*table.0).NumEntries as usize;
        let first = std::ptr::addr_of!((*table.0).Table).cast::<MIB_IF_ROW2>();
        (std::slice::from_raw_parts(first, count), count)
    };
    if count == 0 {
        return None;
    }
    let totals = rows
        .iter()
        .filter(|row| row.OperStatus == NET_IF_OPER_STATUS_UP && row.Type != MIB_IF_TYPE_LOOPBACK)
        .fold((0u64, 0u64), |(received, sent), row| {
            (received.saturating_add(row.InOctets), sent.saturating_add(row.OutOctets))
        });
    Some(totals)
}

pub fn network_rate(
    previous: (u64, u64),
    current: (u64, u64),
    elapsed_seconds: f64,
) -> Option<NetworkSample> {
    if !elapsed_seconds.is_finite() || elapsed_seconds <= 0.0 {
        return None;
    }
    let received = current.0.checked_sub(previous.0)? as f64 / elapsed_seconds;
    let sent = current.1.checked_sub(previous.1)? as f64 / elapsed_seconds;
    (received.is_finite() && sent.is_finite()).then_some(NetworkSample {
        received_bytes_per_sec: received.min(f32::MAX as f64) as f32,
        sent_bytes_per_sec: sent.min(f32::MAX as f64) as f32,
    })
}

pub fn format_network_rate(bytes_per_sec: f32) -> String {
    if !bytes_per_sec.is_finite() || bytes_per_sec <= 0.0 {
        return "0 B/s".into();
    }
    let units = ["B/s", "KiB/s", "MiB/s", "GiB/s"];
    let mut value = bytes_per_sec as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{value:.0} {}", units[unit]) } else { format!("{value:.1} {}", units[unit]) }
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
    fn network_rate_uses_counter_deltas_and_rejects_resets() {
        assert_eq!(
            network_rate((1_000, 2_000), (3_000, 5_000), 2.0),
            Some(NetworkSample { received_bytes_per_sec: 1_000.0, sent_bytes_per_sec: 1_500.0 })
        );
        assert_eq!(network_rate((10, 20), (9, 30), 1.0), None);
        assert_eq!(network_rate((0, 0), (10, 10), 0.0), None);
        assert_eq!(network_rate((0, 0), (10, 10), f64::NAN), None);
    }

    #[test]
    fn network_rate_format_uses_byte_units() {
        assert_eq!(format_network_rate(0.0), "0 B/s");
        assert_eq!(format_network_rate(512.0), "512 B/s");
        assert_eq!(format_network_rate(1_536.0), "1.5 KiB/s");
        assert_eq!(format_network_rate(f32::INFINITY), "0 B/s");
    }

    #[test]
    fn network_history_is_bounded_and_keeps_unavailable_samples() {
        let mut history = VecDeque::new();
        for _ in 0..=HISTORY {
            push_network(&mut history, None);
        }
        assert_eq!(history.len(), HISTORY);
        assert!(history.iter().all(Option::is_none));
    }

    #[test]
    fn reset_window_discards_samples_from_hidden_time() {
        let mut sampler = Sampler::new();
        push(&mut sampler.cpu_history, 42.0);
        push(&mut sampler.ram_history, 37.0);
        push_network(&mut sampler.network_history, None);
        sampler.reset_window();
        assert!(sampler.cpu_history.is_empty());
        assert!(sampler.ram_history.is_empty());
        assert!(sampler.network_history.is_empty());
        assert!(sampler.latest.network.is_none());
        assert!(sampler.last_at.is_none());
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
        if n.network_available {
            std::thread::sleep(Duration::from_millis(1_000));
            assert!(s.tick());
            let network = s.latest.network.expect("active adapter counters produce a second sample");
            assert!(network.received_bytes_per_sec.is_finite() && network.received_bytes_per_sec >= 0.0);
            assert!(network.sent_bytes_per_sec.is_finite() && network.sent_bytes_per_sec >= 0.0);
        }
    }
}
