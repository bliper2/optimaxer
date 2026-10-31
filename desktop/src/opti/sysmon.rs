//! Live readings for Overview and Performance: CPU load, memory, system-drive capacity, uptime, process count,
//! aggregate physical-disk throughput, adapter traffic, busiest GPU-engine utilization and dedicated GPU memory.
//! Samples are read-only, collected on a pausable worker, and kept in bounded chart histories.

use super::backend::{RegData, System, WinSystem};
use super::hw;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2, MIB_IF_TYPE_LOOPBACK,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_IF_OPER_STATUS_UP;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW,
};
use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
use windows_sys::Win32::System::SystemInformation::{
    GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::GetSystemTimes;

pub const MAX_HISTORY: usize = 121;

pub const SAMPLE_INTERVAL_SECONDS: [u64; 3] = [1, 2, 5];
pub const HISTORY_WINDOW_SECONDS: [u64; 3] = [30, 60, 120];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TelemetryConfig {
    interval_seconds: u64,
    history_seconds: u64,
}

impl TelemetryConfig {
    pub fn new(interval_seconds: u64, history_seconds: u64) -> Self {
        let interval_seconds = if SAMPLE_INTERVAL_SECONDS.contains(&interval_seconds) {
            interval_seconds
        } else {
            1
        };
        let history_seconds = if HISTORY_WINDOW_SECONDS.contains(&history_seconds) {
            history_seconds
        } else {
            60
        };
        Self {
            interval_seconds,
            history_seconds,
        }
    }

    pub fn interval(self) -> Duration {
        Duration::from_secs(self.interval_seconds)
    }

    pub fn interval_seconds(self) -> u64 {
        self.interval_seconds
    }

    pub fn history_seconds(self) -> u64 {
        self.history_seconds
    }

    pub fn history_capacity(self) -> usize {
        (self.history_seconds.div_ceil(self.interval_seconds) + 1).clamp(2, MAX_HISTORY as u64)
            as usize
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetworkSample {
    pub received_bytes_per_sec: f32,
    pub sent_bytes_per_sec: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// 0 to 100
    pub cpu: f32,
    pub cpu_available: bool,
    pub ram_used: u64,
    pub ram_total: u64,
    pub ram_available: bool,
    pub disk_used: u64,
    pub disk_total: u64,
    pub disk_available: bool,
    pub uptime: u64,
    pub processes: u32,
    pub network: Option<NetworkSample>,
    pub network_available: bool,
    pub disk_activity: Option<f32>,
    pub disk_counter_available: bool,
    pub gpu_busiest_engine: Option<f32>,
    pub gpu_dedicated_memory_bytes: Option<u64>,
}

impl Snapshot {
    pub fn ram_fraction(&self) -> f32 {
        fraction(self.ram_used, self.ram_total)
    }
    pub fn disk_fraction(&self) -> f32 {
        fraction(self.disk_used, self.disk_total)
    }
}

#[derive(Clone, Debug, Default)]
pub struct TelemetryFrame {
    pub latest: Snapshot,
    pub cpu_history: VecDeque<Option<f32>>,
    pub ram_history: VecDeque<Option<f32>>,
    pub network_history: VecDeque<Option<NetworkSample>>,
    pub disk_history: VecDeque<Option<f32>>,
    pub gpu_history: VecDeque<Option<f32>>,
}

#[derive(Clone, Debug, Default)]
pub struct TelemetryUpdate {
    pub info: Option<SysInfo>,
    pub frame: TelemetryFrame,
}

enum TelemetryCommand {
    Resume(TelemetryConfig),
    Pause,
    Configure(TelemetryConfig),
}

/// Owns the sampling thread. The UI only sends lifecycle/configuration changes and drains snapshots.
pub struct TelemetryWorker {
    commands: mpsc::Sender<TelemetryCommand>,
    updates: Receiver<TelemetryUpdate>,
}

impl TelemetryWorker {
    pub fn new() -> std::io::Result<Self> {
        let (command_tx, command_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("optimaxer-telemetry".into())
            .spawn(move || telemetry_loop(command_rx, update_tx))?;
        Ok(Self {
            commands: command_tx,
            updates: update_rx,
        })
    }

    pub fn resume(&self, config: TelemetryConfig) -> Result<(), String> {
        let _ = self.try_latest();
        self.send(TelemetryCommand::Resume(config))
    }

    pub fn pause(&self) -> Result<(), String> {
        self.send(TelemetryCommand::Pause)
    }

    pub fn configure(&self, config: TelemetryConfig) -> Result<(), String> {
        self.send(TelemetryCommand::Configure(config))
    }

    fn send(&self, command: TelemetryCommand) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "The telemetry worker stopped unexpectedly.".into())
    }

    pub fn try_latest(&self) -> Option<TelemetryUpdate> {
        let mut latest = None;
        loop {
            match self.updates.try_recv() {
                Ok(update) => latest = Some(update),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return latest,
            }
        }
    }
}

fn telemetry_loop(commands: Receiver<TelemetryCommand>, updates: SyncSender<TelemetryUpdate>) {
    let mut active = false;
    let mut config = TelemetryConfig::new(1, 60);
    let mut sampler: Option<Sampler> = None;
    let mut info: Option<SysInfo> = None;

    loop {
        let wait = if active {
            config.interval()
        } else {
            Duration::from_secs(86_400)
        };
        match commands.recv_timeout(wait) {
            Ok(TelemetryCommand::Resume(next)) => {
                config = next;
                let sampler = sampler.get_or_insert_with(Sampler::new);
                sampler.configure(config);
                sampler.reset_window();
                active = true;
                if info.is_none() {
                    info = Some(read_info(&WinSystem));
                }
                sample_and_send(sampler, &mut info, &updates);
            }
            Ok(TelemetryCommand::Pause) => active = false,
            Ok(TelemetryCommand::Configure(next)) => {
                if next != config {
                    config = next;
                    if let Some(sampler) = sampler.as_mut() {
                        sampler.configure(config);
                        if active {
                            sampler.reset_window();
                            sample_and_send(sampler, &mut info, &updates);
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) if active => {
                if let Some(sampler) = sampler.as_mut() {
                    sample_and_send(sampler, &mut info, &updates);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn sample_and_send(
    sampler: &mut Sampler,
    info: &mut Option<SysInfo>,
    updates: &SyncSender<TelemetryUpdate>,
) {
    sampler.tick();
    let update = TelemetryUpdate {
        info: info.clone(),
        frame: sampler.frame(),
    };
    match updates.try_send(update) {
        Ok(()) => *info = None,
        Err(TrySendError::Full(_)) => {}
        Err(TrySendError::Disconnected(_)) => {}
    }
}

pub fn fraction(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

/// CPU load in percent between two (idle, kernel, user) readings. Kernel time includes idle time.
pub fn cpu_percent(prev: (u64, u64, u64), now: (u64, u64, u64)) -> f32 {
    let idle = now.0.wrapping_sub(prev.0);
    let total = now.1.wrapping_sub(prev.1) + now.2.wrapping_sub(prev.2);
    if total == 0 {
        0.0
    } else {
        (100.0 * total.saturating_sub(idle) as f64 / total as f64).clamp(0.0, 100.0) as f32
    }
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
    let (mut i, mut k, mut u) = (
        FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        },
        FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        },
        FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        },
    );
    // SAFETY: three valid out-pointers.
    (unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } != 0).then(|| (ft(&i), ft(&k), ft(&u)))
}

pub(crate) fn read_memory() -> Option<(u64, u64)> {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: dwLength is set and `m` is a valid out-pointer.
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return None;
    }
    Some((
        m.ullTotalPhys.saturating_sub(m.ullAvailPhys),
        m.ullTotalPhys,
    ))
}

pub(crate) fn read_disk() -> Option<(u64, u64)> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let path: Vec<u16> = format!("{drive}\\")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
    // SAFETY: NUL-terminated path and three valid out-pointers.
    if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut free, &mut total, &mut total_free) } == 0 {
        return None;
    }
    Some((total.saturating_sub(total_free), total))
}

fn process_count() -> u32 {
    let mut pids = vec![0u32; 4096];
    let mut bytes = 0u32;
    // SAFETY: the buffer size in bytes is passed and `bytes` receives the amount written.
    if unsafe { EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes) } == 0 {
        0
    } else {
        bytes / 4
    }
}

pub fn uptime_secs() -> u64 {
    // SAFETY: GetTickCount64 has no pointer parameters.
    unsafe { GetTickCount64() / 1000 }
}

pub struct Sampler {
    config: TelemetryConfig,
    last_times: Option<(u64, u64, u64)>,
    last_at: Option<Instant>,
    pub latest: Snapshot,
    pub cpu_history: VecDeque<Option<f32>>,
    pub ram_history: VecDeque<Option<f32>>,
    pub network_history: VecDeque<Option<NetworkSample>>,
    last_network_totals: Option<(u64, u64)>,
    disk_counter: Option<DiskCounter>,
    pub disk_history: VecDeque<Option<f32>>,
    pub gpu_history: VecDeque<Option<f32>>,
    gpu_counter: Option<GpuCounter>,
}

struct DiskCounter {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

impl DiskCounter {
    fn new() -> Option<Self> {
        let mut query = std::ptr::null_mut();
        // SAFETY: query is a valid out-pointer; a null data source opens a live query.
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) } != 0 {
            return None;
        }
        let path: Vec<u16> = r"\PhysicalDisk(_Total)\Disk Bytes/sec"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut counter = std::ptr::null_mut();
        // SAFETY: query is open and path/counter are valid; the English API resolves the counter on localized Windows.
        if unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) } != 0 {
            // SAFETY: query was returned successfully by PdhOpenQueryW.
            unsafe { PdhCloseQuery(query) };
            return None;
        }
        let this = Self { query, counter };
        this.collect();
        Some(this)
    }

    fn collect(&self) -> bool {
        // SAFETY: self.query is owned until Drop and contains the registered counter.
        unsafe { PdhCollectQueryData(self.query) == 0 }
    }

    fn sample(&self) -> Option<f32> {
        if !self.collect() {
            return None;
        }
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: counter belongs to the live query and value is a valid output structure.
        if unsafe {
            PdhGetFormattedCounterValue(
                self.counter,
                PDH_FMT_DOUBLE,
                std::ptr::null_mut(),
                &mut value,
            )
        } != 0
        {
            return None;
        }
        if !matches!(value.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA) {
            return None;
        }
        // SAFETY: PDH_FMT_DOUBLE requests that this union member be written.
        let rate = unsafe { value.Anonymous.doubleValue };
        (rate.is_finite() && rate >= 0.0).then_some(rate.min(f32::MAX as f64) as f32)
    }
}

impl Drop for DiskCounter {
    fn drop(&mut self) {
        // SAFETY: this query was opened by new and is closed exactly once.
        unsafe { PdhCloseQuery(self.query) };
    }
}

struct GpuCounter {
    query: PDH_HQUERY,
    engine: Option<PDH_HCOUNTER>,
    dedicated_memory: Option<PDH_HCOUNTER>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GpuReading {
    busiest_engine_percent: Option<f32>,
    dedicated_memory_bytes: Option<u64>,
}

impl GpuCounter {
    fn new() -> Option<Self> {
        let mut query = std::ptr::null_mut();
        // SAFETY: query is a valid out-pointer; a null data source opens a live local query.
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) } != 0 {
            return None;
        }
        let add = |path: &str| {
            let path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut counter = std::ptr::null_mut();
            // SAFETY: the query is open and both path and counter are valid for the call.
            (unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) } == 0)
                .then_some(counter)
        };
        let engine = add(r"\GPU Engine(*)\Utilization Percentage");
        let dedicated_memory = add(r"\GPU Adapter Memory(*)\Dedicated Usage");
        if engine.is_none() && dedicated_memory.is_none() {
            // SAFETY: query was opened above and no counters retain it.
            unsafe { PdhCloseQuery(query) };
            return None;
        }
        let this = Self {
            query,
            engine,
            dedicated_memory,
        };
        this.collect();
        Some(this)
    }

    fn collect(&self) -> bool {
        // SAFETY: the query remains owned until Drop and contains the registered counters.
        unsafe { PdhCollectQueryData(self.query) == 0 }
    }

    fn sample(&self) -> GpuReading {
        if !self.collect() {
            return GpuReading::default();
        }
        let engine = self.engine.and_then(formatted_counter_array);
        let memory = self.dedicated_memory.and_then(formatted_counter_array);
        let busiest_engine_percent = engine.as_deref().and_then(aggregate_gpu_engines);
        let dedicated_memory_bytes = memory.as_deref().and_then(sum_dedicated_memory);
        GpuReading {
            busiest_engine_percent,
            dedicated_memory_bytes,
        }
    }
}

impl Drop for GpuCounter {
    fn drop(&mut self) {
        // SAFETY: this query was opened by new and is closed exactly once.
        unsafe { PdhCloseQuery(self.query) };
    }
}

fn formatted_counter_array(counter: PDH_HCOUNTER) -> Option<Vec<(String, f64)>> {
    let mut bytes = 0u32;
    let mut count = 0u32;
    // SAFETY: null output requests the required buffer size; both size outputs are valid.
    let status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut bytes,
            &mut count,
            std::ptr::null_mut(),
        )
    };
    if status != PDH_MORE_DATA || bytes == 0 || count == 0 {
        return None;
    }
    let item_size = std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>();
    let item_capacity = (bytes as usize).div_ceil(item_size);
    let mut items = vec![PDH_FMT_COUNTERVALUE_ITEM_W::default(); item_capacity];
    let mut actual_bytes = (items.len() * item_size).min(u32::MAX as usize) as u32;
    let mut actual_count = 0u32;
    // SAFETY: items is aligned and provides at least the byte count requested by PDH.
    let status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut actual_bytes,
            &mut actual_count,
            items.as_mut_ptr(),
        )
    };
    if status != 0 {
        return None;
    }
    if actual_count as usize > items.len() {
        return None;
    }
    let mut samples = Vec::with_capacity(actual_count as usize);
    for item in items.iter().take(actual_count as usize) {
        if !matches!(
            item.FmtValue.CStatus,
            PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
        ) {
            continue;
        }
        let name = unsafe { wide_counter_name(item.szName)? };
        // SAFETY: PDH_FMT_DOUBLE requests this union member.
        let value = unsafe { item.FmtValue.Anonymous.doubleValue };
        if value.is_finite() && value >= 0.0 {
            samples.push((name, value));
        }
    }
    Some(samples)
}

unsafe fn wide_counter_name(pointer: *const u16) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let len = (0..4096usize).find(|index| {
        // SAFETY: the PDH item name is a NUL-terminated string owned by the returned array buffer.
        unsafe { *pointer.add(*index) == 0 }
    })?;
    // SAFETY: the preceding scan found a terminator within the bounded counter-name length.
    Some(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(pointer, len)
    }))
}

fn aggregate_gpu_engines(samples: &[(String, f64)]) -> Option<f32> {
    let mut engines = HashMap::<String, f64>::new();
    for (name, value) in samples {
        let normalized = name.to_ascii_lowercase();
        let Some(start) = normalized.find("luid_") else {
            continue;
        };
        let Some((adapter, rest)) = normalized[start + 5..].split_once("_phys_") else {
            continue;
        };
        let Some((physical, rest)) = rest.split_once("_eng_") else {
            continue;
        };
        let Some((engine, engine_type)) = rest.split_once("_engtype_") else {
            continue;
        };
        if adapter.is_empty() || physical.is_empty() || engine.is_empty() || engine_type.is_empty()
        {
            continue;
        }
        let key = format!("{adapter}:{physical}:{engine}:{engine_type}");
        *engines.entry(key).or_default() += value;
    }
    engines
        .into_values()
        .map(|busy| busy.clamp(0.0, 100.0) as f32)
        .reduce(f32::max)
        .filter(|busy| busy.is_finite())
}

fn sum_dedicated_memory(samples: &[(String, f64)]) -> Option<u64> {
    let mut found = false;
    let total = samples.iter().try_fold(0u64, |total, (_, value)| {
        if !value.is_finite() || *value < 0.0 || *value >= u64::MAX as f64 {
            return None;
        }
        found = true;
        total.checked_add(value.round() as u64)
    })?;
    found.then_some(total)
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            config: TelemetryConfig::new(1, 60),
            last_times: read_times(),
            last_at: None,
            latest: Snapshot::default(),
            cpu_history: VecDeque::new(),
            ram_history: VecDeque::new(),
            network_history: VecDeque::new(),
            last_network_totals: read_network_totals(),
            disk_counter: DiskCounter::new(),
            disk_history: VecDeque::new(),
            gpu_counter: GpuCounter::new(),
            gpu_history: VecDeque::new(),
        }
    }

    pub fn configure(&mut self, config: TelemetryConfig) -> bool {
        if self.config == config {
            return false;
        }
        self.config = config;
        true
    }

    pub fn frame(&self) -> TelemetryFrame {
        TelemetryFrame {
            latest: self.latest,
            cpu_history: self.cpu_history.clone(),
            ram_history: self.ram_history.clone(),
            network_history: self.network_history.clone(),
            disk_history: self.disk_history.clone(),
            gpu_history: self.gpu_history.clone(),
        }
    }

    /// Takes a new reading when the configured interval has passed. Returns true when it did.
    pub fn tick(&mut self) -> bool {
        if self
            .last_at
            .is_some_and(|t| t.elapsed() < self.config.interval())
        {
            return false;
        }
        self.sample();
        true
    }

    /// Takes a reading now, whatever the configured interval. Rates use the time since the previous reading.
    pub fn sample(&mut self) {
        let sample_elapsed = self
            .last_at
            .map(|at| at.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        self.last_at = Some(Instant::now());
        let now = read_times();
        let (cpu, cpu_available) = match (self.last_times, now) {
            (Some(p), Some(n)) => (cpu_percent(p, n), true),
            _ => (self.latest.cpu, false),
        };
        self.last_times = now;
        let (ram_used, ram_total, ram_available) = read_memory()
            .map(|(used, total)| (used, total, true))
            .unwrap_or((self.latest.ram_used, self.latest.ram_total, false));
        let (disk_used, disk_total, disk_available) = read_disk()
            .map(|(used, total)| (used, total, true))
            .unwrap_or((self.latest.disk_used, self.latest.disk_total, false));
        let network_totals = read_network_totals();
        let network = network_totals
            .zip(self.last_network_totals)
            .and_then(|(now, previous)| network_rate(previous, now, sample_elapsed));
        if let Some(totals) = network_totals {
            self.last_network_totals = Some(totals);
        }
        let disk_activity = self.disk_counter.as_ref().and_then(DiskCounter::sample);
        let gpu = self
            .gpu_counter
            .as_ref()
            .map(GpuCounter::sample)
            .unwrap_or_default();
        self.latest = Snapshot {
            cpu,
            cpu_available,
            ram_used,
            ram_total,
            ram_available,
            disk_used,
            disk_total,
            disk_available,
            uptime: uptime_secs(),
            processes: process_count(),
            network,
            network_available: network_totals.is_some(),
            disk_activity,
            disk_counter_available: self.disk_counter.is_some(),
            gpu_busiest_engine: gpu.busiest_engine_percent,
            gpu_dedicated_memory_bytes: gpu.dedicated_memory_bytes,
        };
        let history_capacity = self.config.history_capacity();
        push(
            &mut self.cpu_history,
            cpu_available.then_some(cpu),
            history_capacity,
        );
        push(
            &mut self.ram_history,
            ram_available.then_some(self.latest.ram_fraction() * 100.0),
            history_capacity,
        );
        push_network(&mut self.network_history, network, history_capacity);
        push(&mut self.disk_history, disk_activity, history_capacity);
        push(
            &mut self.gpu_history,
            gpu.busiest_engine_percent,
            history_capacity,
        );
    }

    /// Start a fresh chart window when the Overview becomes visible again.
    pub fn reset_window(&mut self) {
        self.last_times = read_times();
        self.last_at = None;
        self.latest.cpu_available = false;
        self.latest.ram_available = false;
        self.latest.disk_available = false;
        self.latest.network = None;
        self.latest.network_available = false;
        self.latest.disk_activity = None;
        self.latest.gpu_busiest_engine = None;
        self.latest.gpu_dedicated_memory_bytes = None;
        self.cpu_history.clear();
        self.ram_history.clear();
        self.network_history.clear();
        self.disk_history.clear();
        self.gpu_history.clear();
        if let Some(counter) = self.gpu_counter.as_ref() {
            counter.collect();
        }
        if let Some(counter) = self.disk_counter.as_ref() {
            counter.collect();
        }
        self.last_network_totals = read_network_totals();
    }
}

fn push(h: &mut VecDeque<Option<f32>>, v: Option<f32>, capacity: usize) {
    let capacity = capacity.clamp(1, MAX_HISTORY);
    if h.len() >= capacity {
        h.pop_front();
    }
    h.push_back(v);
}

fn push_network(
    h: &mut VecDeque<Option<NetworkSample>>,
    value: Option<NetworkSample>,
    capacity: usize,
) {
    let capacity = capacity.clamp(1, MAX_HISTORY);
    if h.len() >= capacity {
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
            (
                received.saturating_add(row.InOctets),
                sent.saturating_add(row.OutOctets),
            )
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

pub fn format_bytes_per_second(bytes_per_sec: f32) -> String {
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
    if unit == 0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
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
        cpu: text(
            r"HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "ProcessorNameString",
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" "),
        os: if version.is_empty() {
            format!("{name} (build {build})")
        } else {
            format!("{name} {version} (build {build})")
        },
        gpus: hw::gpu_names(sys),
        threads: std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;

    const HISTORY: usize = 60;

    #[test]
    fn cpu_percent_comes_from_idle_versus_total() {
        assert_eq!(cpu_percent((0, 0, 0), (50, 100, 0)), 50.0);
        assert_eq!(cpu_percent((0, 0, 0), (0, 100, 0)), 100.0);
        assert_eq!(cpu_percent((0, 0, 0), (100, 100, 0)), 0.0);
        // kernel time already contains idle time, user time is added on top
        assert_eq!(cpu_percent((0, 0, 0), (60, 80, 20)), 40.0);
        assert_eq!(
            cpu_percent((10, 10, 10), (10, 10, 10)),
            0.0,
            "no time passed"
        );
    }

    #[test]
    fn telemetry_configuration_bounds_settings_and_history() {
        let valid = TelemetryConfig::new(5, 120);
        assert_eq!(valid.interval_seconds(), 5);
        assert_eq!(valid.history_seconds(), 120);
        assert_eq!(valid.history_capacity(), 25);

        let fallback = TelemetryConfig::new(3, 90);
        assert_eq!(fallback.interval_seconds(), 1);
        assert_eq!(fallback.history_seconds(), 60);
        assert_eq!(fallback.history_capacity(), 61);
        assert_eq!(TelemetryConfig::new(1, 120).history_capacity(), MAX_HISTORY);
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
            push(&mut h, Some(i as f32), HISTORY);
        }
        assert_eq!(h.len(), HISTORY);
        assert_eq!(*h.front().unwrap(), Some(20.0));
        assert_eq!(*h.back().unwrap(), Some((HISTORY + 19) as f32));
    }

    #[test]
    fn network_rate_uses_counter_deltas_and_rejects_resets() {
        assert_eq!(
            network_rate((1_000, 2_000), (3_000, 5_000), 2.0),
            Some(NetworkSample {
                received_bytes_per_sec: 1_000.0,
                sent_bytes_per_sec: 1_500.0
            })
        );
        assert_eq!(network_rate((10, 20), (9, 30), 1.0), None);
        assert_eq!(network_rate((0, 0), (10, 10), 0.0), None);
        assert_eq!(network_rate((0, 0), (10, 10), f64::NAN), None);
    }

    #[test]
    fn bytes_per_second_format_uses_byte_units() {
        assert_eq!(format_bytes_per_second(0.0), "0 B/s");
        assert_eq!(format_bytes_per_second(512.0), "512 B/s");
        assert_eq!(format_bytes_per_second(1_536.0), "1.5 KiB/s");
        assert_eq!(format_bytes_per_second(f32::INFINITY), "0 B/s");
    }

    #[test]
    fn network_history_is_bounded_and_keeps_unavailable_samples() {
        let mut history = VecDeque::new();
        for _ in 0..=HISTORY {
            push_network(&mut history, None, 3);
        }
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(Option::is_none));
    }

    #[test]
    fn disk_history_is_bounded_and_keeps_unavailable_samples() {
        let mut history = VecDeque::new();
        for _ in 0..=HISTORY {
            push(&mut history, None, 3);
        }
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(Option::is_none));
    }

    #[test]
    fn gpu_engine_usage_sums_processes_per_engine_and_caps_at_one_hundred() {
        let samples = vec![
            (
                "pid_1_luid_0x0000_0x0010_phys_0_eng_0_engtype_3d".into(),
                70.0,
            ),
            (
                "pid_2_luid_0x0000_0x0010_phys_0_eng_0_engtype_3d".into(),
                45.0,
            ),
            (
                "pid_1_luid_0x0000_0x0010_phys_0_eng_1_engtype_copy".into(),
                60.0,
            ),
            (
                "pid_3_luid_0x0000_0x0020_phys_0_eng_0_engtype_3d".into(),
                35.0,
            ),
            ("invalid counter instance".into(), 100.0),
        ];
        assert_eq!(aggregate_gpu_engines(&samples), Some(100.0));
        assert_eq!(aggregate_gpu_engines(&[]), None);
    }

    #[test]
    fn gpu_dedicated_memory_sum_rejects_invalid_values_and_overflow() {
        assert_eq!(
            sum_dedicated_memory(&[("adapter 0".into(), 1_024.0), ("adapter 1".into(), 2_048.0)]),
            Some(3_072)
        );
        assert_eq!(sum_dedicated_memory(&[("adapter".into(), f64::NAN)]), None);
        assert_eq!(
            sum_dedicated_memory(&[
                ("adapter 0".into(), u64::MAX as f64),
                ("adapter 1".into(), 1.0)
            ]),
            None
        );
    }

    #[test]
    fn gpu_history_is_bounded_and_keeps_unavailable_samples() {
        let mut history = VecDeque::new();
        for _ in 0..=HISTORY {
            push(&mut history, None, HISTORY);
        }
        assert_eq!(history.len(), HISTORY);
        assert!(history.iter().all(Option::is_none));
    }

    #[test]
    fn gpu_counter_readings_are_valid_or_unavailable() {
        let Some(counter) = GpuCounter::new() else {
            return;
        };
        let reading = counter.sample();
        assert!(
            reading
                .busiest_engine_percent
                .is_none_or(|value| (0.0..=100.0).contains(&value))
        );
    }

    #[test]
    fn reset_window_discards_samples_from_hidden_time() {
        let mut sampler = Sampler::new();
        push(&mut sampler.cpu_history, Some(42.0), HISTORY);
        push(&mut sampler.ram_history, Some(37.0), HISTORY);
        push_network(&mut sampler.network_history, None, HISTORY);
        push(&mut sampler.disk_history, Some(12.0), HISTORY);
        push(&mut sampler.gpu_history, Some(24.0), HISTORY);
        sampler.reset_window();
        assert!(sampler.cpu_history.is_empty());
        assert!(sampler.ram_history.is_empty());
        assert!(sampler.network_history.is_empty());
        assert!(sampler.disk_history.is_empty());
        assert!(sampler.gpu_history.is_empty());
        assert!(sampler.latest.network.is_none());
        assert!(sampler.latest.disk_activity.is_none());
        assert!(sampler.latest.gpu_busiest_engine.is_none());
        assert!(sampler.latest.gpu_dedicated_memory_bytes.is_none());
        assert!(sampler.last_at.is_none());
    }

    #[test]
    fn windows_11_is_named_correctly() {
        let sys = FakeSystem::default();
        let cur = r"HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        sys.put(cur, "ProductName", RegData::Str("Windows 10 Pro".into()));
        sys.put(cur, "CurrentBuildNumber", RegData::Str("26100".into()));
        sys.put(cur, "DisplayVersion", RegData::Str("24H2".into()));
        sys.put(
            r"HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "ProcessorNameString",
            RegData::Str("Intel(R) Core(TM)  i7-14700KF".into()),
        );
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
        assert!(n.cpu_available && n.ram_available && n.disk_available);
        assert!(n.ram_total > 1 << 30 && n.ram_used <= n.ram_total);
        assert!(n.disk_total > 0 && n.disk_used <= n.disk_total);
        assert!(n.processes > 10 && n.uptime > 0);
        assert!((0.0..=100.0).contains(&n.cpu));
        assert!(!s.tick(), "a second reading inside a second is skipped");
        if n.network_available {
            std::thread::sleep(Duration::from_millis(1_000));
            assert!(s.tick());
            let network = s
                .latest
                .network
                .expect("active adapter counters produce a second sample");
            assert!(
                network.received_bytes_per_sec.is_finite() && network.received_bytes_per_sec >= 0.0
            );
            assert!(network.sent_bytes_per_sec.is_finite() && network.sent_bytes_per_sec >= 0.0);
        }
        assert!(
            n.disk_activity
                .is_none_or(|rate| rate.is_finite() && rate >= 0.0)
        );
    }
}
