//! Live telemetry read from `/proc` and `/sys`. A value the machine does not provide is `None`; nothing is guessed.
//! The same names as the Windows `sysmon` (config, `NetworkSample`, `uptime_secs`) are kept so the shared app code
//! and chart widgets work unchanged.

use super::procfs::{self, CpuTimes, MemInfo};
use super::system::System;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

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
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub cpu_percent: Option<f32>,
    pub mem: Option<MemInfo>,
    pub disk: Option<(u64, u64)>,
    pub disk_busy: Option<f32>,
    pub network: Option<NetworkSample>,
    pub gpu_percent: Option<f32>,
    pub cpu_temp_c: Option<f32>,
    pub load: Option<(f32, f32, f32)>,
    pub uptime: Option<u64>,
    pub battery: Option<Battery>,
}

impl Snapshot {
    pub fn ram_fraction(&self) -> Option<f32> {
        self.mem.map(|m| fraction(m.used(), m.total))
    }

    pub fn disk_fraction(&self) -> Option<f32> {
        self.disk.map(|(used, total)| fraction(used, total))
    }
}

pub fn fraction(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

pub fn format_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m {}s", secs % 60)
    }
}

pub fn format_bytes_per_second(bytes_per_sec: f32) -> String {
    let b = bytes_per_sec.max(0.0);
    if b >= 1_048_576.0 {
        format!("{:.1} MB/s", b / 1_048_576.0)
    } else if b >= 1024.0 {
        format!("{:.0} KB/s", b / 1024.0)
    } else {
        format!("{b:.0} B/s")
    }
}

/// Seconds since boot, or 0 when `/proc/uptime` cannot be read. The restart reminder compares two of these.
#[cfg(unix)]
pub fn uptime_secs() -> u64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| procfs::parse_uptime(&t))
        .unwrap_or(0)
}

#[cfg(not(unix))]
pub fn uptime_secs() -> u64 {
    0
}

/// Bytes per second from two counter readings; a smaller counter (interface reset) gives no rate.
pub fn rate(prev: u64, now: u64, seconds: f32) -> Option<f32> {
    (seconds > 0.0)
        .then(|| now.checked_sub(prev))
        .flatten()
        .map(|d| d as f32 / seconds)
}

fn push<T>(history: &mut VecDeque<T>, value: T, capacity: usize) {
    history.push_back(value);
    while history.len() > capacity {
        history.pop_front();
    }
}

pub struct Sampler {
    config: TelemetryConfig,
    prev_cpu: Option<CpuTimes>,
    prev_net: Option<(Instant, u64, u64)>,
    prev_disk: Option<(Instant, u64)>,
    pub snapshot: Snapshot,
    pub cpu: VecDeque<Option<f32>>,
    pub ram: VecDeque<Option<f32>>,
    pub disk: VecDeque<Option<f32>>,
    pub network: VecDeque<Option<NetworkSample>>,
    pub gpu: VecDeque<Option<f32>>,
    last: Option<Instant>,
}

impl Sampler {
    pub fn new(config: TelemetryConfig) -> Self {
        Self {
            config,
            prev_cpu: None,
            prev_net: None,
            prev_disk: None,
            snapshot: Snapshot::default(),
            cpu: VecDeque::new(),
            ram: VecDeque::new(),
            disk: VecDeque::new(),
            network: VecDeque::new(),
            gpu: VecDeque::new(),
            last: None,
        }
    }

    pub fn config(&self) -> TelemetryConfig {
        self.config
    }

    /// Returns true when the setting changed; the chart window is cleared then.
    pub fn configure(&mut self, config: TelemetryConfig) -> bool {
        if config == self.config {
            return false;
        }
        self.config = config;
        self.reset_window();
        true
    }

    /// Drops the charts and the baselines, so time spent on another page never shows as one long sample.
    pub fn reset_window(&mut self) {
        for h in [&mut self.cpu, &mut self.ram, &mut self.disk, &mut self.gpu] {
            h.clear();
        }
        self.network.clear();
        self.prev_cpu = None;
        self.prev_net = None;
        self.prev_disk = None;
        self.last = None;
    }

    pub fn due(&self, now: Instant) -> bool {
        self.last
            .is_none_or(|t| now.duration_since(t) >= self.config.interval())
    }

    pub fn sample(&mut self, sys: &dyn System, now: Instant) {
        self.last = Some(now);
        let cap = self.config.history_capacity();
        let read = |path: &str| super::system::read_text(sys, path);

        let cpu_now = read("/proc/stat").and_then(|t| procfs::parse_cpu_total(&t));
        let cpu = match (self.prev_cpu, cpu_now) {
            (Some(prev), Some(now)) => procfs::cpu_percent(prev, now),
            _ => None,
        };
        self.prev_cpu = cpu_now;

        let mem = read("/proc/meminfo").and_then(|t| procfs::parse_meminfo(&t));
        let disk = sys.disk_space("/");

        let net_now = read("/proc/net/dev").map(|t| {
            procfs::parse_net_dev(&t)
                .into_iter()
                .filter(|d| d.name != "lo")
                .fold((0u64, 0u64), |(rx, tx), d| {
                    (rx.saturating_add(d.rx_bytes), tx.saturating_add(d.tx_bytes))
                })
        });
        let network = match (self.prev_net, net_now) {
            (Some((at, rx0, tx0)), Some((rx, tx))) => {
                let secs = now.duration_since(at).as_secs_f32();
                rate(rx0, rx, secs)
                    .zip(rate(tx0, tx, secs))
                    .map(|(r, s)| NetworkSample {
                        received_bytes_per_sec: r,
                        sent_bytes_per_sec: s,
                    })
            }
            _ => None,
        };
        self.prev_net = net_now.map(|(rx, tx)| (now, rx, tx));

        let io_now = read("/proc/diskstats").map(|t| {
            procfs::parse_diskstats(&t)
                .into_iter()
                .filter(|d| !procfs::is_virtual_block(&d.name) && is_whole_disk(sys, &d.name))
                .map(|d| d.io_ms)
                .max()
                .unwrap_or(0)
        });
        let disk_busy = match (self.prev_disk, io_now) {
            (Some((at, before)), Some(io)) => {
                let ms = now.duration_since(at).as_secs_f32() * 1000.0;
                (ms > 0.0)
                    .then(|| io.checked_sub(before))
                    .flatten()
                    .map(|d| (d as f32 / ms * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        };
        self.prev_disk = io_now.map(|io| (now, io));

        self.snapshot = Snapshot {
            cpu_percent: cpu,
            mem,
            disk,
            disk_busy,
            network,
            gpu_percent: gpu_busy(sys),
            cpu_temp_c: cpu_temperature(sys),
            load: read("/proc/loadavg").and_then(|t| procfs::parse_loadavg(&t)),
            uptime: read("/proc/uptime").and_then(|t| procfs::parse_uptime(&t)),
            battery: battery(sys),
        };
        push(&mut self.cpu, cpu, cap);
        push(
            &mut self.ram,
            self.snapshot.ram_fraction().map(|f| f * 100.0),
            cap,
        );
        push(&mut self.disk, disk_busy, cap);
        push(&mut self.network, network, cap);
        push(&mut self.gpu, self.snapshot.gpu_percent, cap);
    }
}

/// Partitions are not listed in `/sys/block`; whole disks are.
fn is_whole_disk(sys: &dyn System, name: &str) -> bool {
    sys.list_dir("/sys/block").iter().any(|n| n == name)
}

/// amdgpu exposes its busy percentage; other drivers do not, and then there is no GPU reading.
pub fn gpu_busy(sys: &dyn System) -> Option<f32> {
    sys.list_dir("/sys/class/drm")
        .iter()
        .filter(|n| {
            n.strip_prefix("card")
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|card| {
            super::system::read_text(
                sys,
                &format!("/sys/class/drm/{card}/device/gpu_busy_percent"),
            )
            .and_then(|v| v.trim().parse::<f32>().ok())
        })
        .map(|v| v.clamp(0.0, 100.0))
        .reduce(f32::max)
}

/// CPU package temperature from the first hwmon chip known to report it.
pub fn cpu_temperature(sys: &dyn System) -> Option<f32> {
    const CHIPS: [&str; 5] = [
        "coretemp",
        "k10temp",
        "zenpower",
        "cpu_thermal",
        "cpu-thermal",
    ];
    for hwmon in sys.list_dir("/sys/class/hwmon") {
        let base = format!("/sys/class/hwmon/{hwmon}");
        let Some(name) = super::system::read_text(sys, &format!("{base}/name")) else {
            continue;
        };
        if !CHIPS.contains(&name.trim()) {
            continue;
        }
        let mut inputs: Vec<String> = sys
            .list_dir(&base)
            .into_iter()
            .filter(|f| f.starts_with("temp") && f.ends_with("_input"))
            .collect();
        inputs.sort();
        // Intel labels the package sensor; AMD's first input is Tctl.
        let package = inputs.iter().find(|input| {
            let label = input.replace("_input", "_label");
            super::system::read_text(sys, &format!("{base}/{label}"))
                .is_some_and(|l| l.trim().starts_with("Package") || l.trim() == "Tctl")
        });
        let chosen = package.or(inputs.first())?;
        let milli: f32 = super::system::read_text(sys, &format!("{base}/{chosen}"))?
            .trim()
            .parse()
            .ok()?;
        let celsius = milli / 1000.0;
        return (0.0..150.0).contains(&celsius).then_some(celsius);
    }
    None
}

pub fn battery(sys: &dyn System) -> Option<Battery> {
    sys.list_dir("/sys/class/power_supply")
        .into_iter()
        .filter(|n| n.starts_with("BAT"))
        .find_map(|bat| {
            let base = format!("/sys/class/power_supply/{bat}");
            let percent = super::system::read_text(sys, &format!("{base}/capacity"))?
                .trim()
                .parse::<u8>()
                .ok()?;
            let status =
                super::system::read_text(sys, &format!("{base}/status")).unwrap_or_default();
            Some(Battery {
                percent: percent.min(100),
                charging: status.trim() == "Charging",
            })
        })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SysInfo {
    pub distro: Option<String>,
    pub kernel: Option<String>,
    pub cpu: Option<String>,
    pub threads: usize,
    pub hostname: Option<String>,
    pub gpus: Vec<String>,
    pub total_memory: Option<u64>,
}

fn pci_vendor(id: &str) -> Option<&'static str> {
    match id.trim() {
        "0x10de" => Some("NVIDIA"),
        "0x1002" => Some("AMD"),
        "0x8086" => Some("Intel"),
        "0x1af4" => Some("virtio"),
        "0x15ad" => Some("VMware"),
        _ => None,
    }
}

/// One line per display adapter: vendor and kernel driver. Marketing names would need the PCI id database, which is
/// not guaranteed to be installed, so they are not invented.
pub fn gpus(sys: &dyn System) -> Vec<String> {
    sys.list_dir("/sys/class/drm")
        .iter()
        .filter(|n| {
            n.strip_prefix("card")
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|card| {
            let dir = format!("/sys/class/drm/{card}/device");
            let vendor = super::system::read_text(sys, &format!("{dir}/vendor"))?;
            let driver = super::system::read_text(sys, &format!("{dir}/uevent")).and_then(|u| {
                u.lines()
                    .find_map(|l| l.strip_prefix("DRIVER=").map(str::to_string))
            });
            let name = pci_vendor(&vendor)
                .map_or_else(|| format!("Vendor {}", vendor.trim()), str::to_string);
            Some(match driver {
                Some(d) => format!("{name} graphics ({d} driver)"),
                None => format!("{name} graphics"),
            })
        })
        .collect()
}

pub fn read_info(sys: &dyn System) -> SysInfo {
    let read = |p: &str| super::system::read_text(sys, p);
    let cpuinfo = read("/proc/cpuinfo").unwrap_or_default();
    SysInfo {
        distro: read("/etc/os-release").and_then(|t| procfs::os_release_value(&t, "PRETTY_NAME")),
        kernel: read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_string()),
        cpu: procfs::cpu_model(&cpuinfo),
        threads: procfs::cpu_threads(&cpuinfo),
        hostname: read("/proc/sys/kernel/hostname").map(|h| h.trim().to_string()),
        gpus: gpus(sys),
        total_memory: read("/proc/meminfo")
            .and_then(|t| procfs::parse_meminfo(&t))
            .map(|m| m.total),
    }
}

#[cfg(test)]
mod tests {
    use super::super::system::fake::FakeSystem;
    use super::*;

    #[test]
    fn telemetry_configuration_bounds_settings_and_history() {
        let c = TelemetryConfig::new(7, 999);
        assert_eq!((c.interval_seconds(), c.history_seconds()), (1, 60));
        assert_eq!(TelemetryConfig::new(5, 120).history_capacity(), 25);
        assert!(TelemetryConfig::new(1, 120).history_capacity() <= MAX_HISTORY);
    }

    #[test]
    fn formats() {
        assert_eq!(format_uptime(59), "0m 59s");
        assert_eq!(format_uptime(3 * 3600 + 120), "3h 2m");
        assert_eq!(format_uptime(2 * 86_400 + 3600), "2d 1h 0m");
        assert_eq!(format_bytes_per_second(500.0), "500 B/s");
        assert_eq!(format_bytes_per_second(2048.0), "2 KB/s");
        assert_eq!(format_bytes_per_second(3.0 * 1_048_576.0), "3.0 MB/s");
        assert_eq!(fraction(1, 0), 0.0);
        assert_eq!(fraction(5, 10), 0.5);
        assert_eq!(rate(100, 50, 1.0), None, "counter reset");
        assert_eq!(rate(0, 100, 2.0), Some(50.0));
        assert_eq!(rate(0, 100, 0.0), None);
    }

    fn machine(cpu: &str, rx: u64, tx: u64, io: u64) -> FakeSystem {
        let sys = FakeSystem::default()
            .with_file("/proc/stat", cpu)
            .with_file("/proc/meminfo", "MemTotal: 1000 kB\nMemAvailable: 250 kB\n")
            .with_file(
                "/proc/net/dev",
                &format!("h1\nh2\n lo: 9999 1 0 0 0 0 0 0 9999 1 0 0 0 0 0 0\n eth0: {rx} 1 0 0 0 0 0 0 {tx} 1 0 0 0 0 0 0\n"),
            )
            .with_file(
                "/proc/diskstats",
                &format!(" 8 0 sda 1 0 1 1 1 0 1 1 0 {io} 1\n 8 1 sda1 1 0 1 1 1 0 1 1 0 {} 1\n", io + 500),
            )
            .with_file("/sys/block/sda/size", "1")
            .with_file("/proc/loadavg", "0.10 0.20 0.30 1/2 3\n")
            .with_file("/proc/uptime", "100.5 1.0\n");
        sys.disk.set(Some((50, 200)));
        sys
    }

    #[test]
    fn the_first_sample_has_no_rates_and_the_second_does() {
        let mut sampler = Sampler::new(TelemetryConfig::new(1, 60));
        let t0 = Instant::now();
        sampler.sample(
            &machine("cpu  100 0 100 800 0 0 0 0 0 0\n", 1000, 2000, 100),
            t0,
        );
        assert_eq!(sampler.snapshot.cpu_percent, None);
        assert_eq!(sampler.snapshot.network, None);
        assert_eq!(sampler.snapshot.disk_busy, None);
        assert_eq!(sampler.snapshot.ram_fraction(), Some(0.75));
        assert_eq!(sampler.snapshot.disk_fraction(), Some(0.25));
        assert_eq!(sampler.snapshot.uptime, Some(100));
        sampler.sample(
            &machine("cpu  200 0 200 900 0 0 0 0 0 0\n", 3000, 2500, 600),
            t0 + Duration::from_secs(1),
        );
        assert!((sampler.snapshot.cpu_percent.unwrap() - 66.666_67).abs() < 0.01);
        let net = sampler.snapshot.network.unwrap();
        assert_eq!(
            (net.received_bytes_per_sec, net.sent_bytes_per_sec),
            (2000.0, 500.0)
        );
        assert_eq!(
            sampler.snapshot.disk_busy,
            Some(50.0),
            "only the whole disk counts, not its partition"
        );
        assert_eq!(
            sampler.snapshot.gpu_percent, None,
            "no amdgpu, no GPU reading"
        );
        assert_eq!(sampler.snapshot.cpu_temp_c, None);
        assert_eq!(sampler.cpu.len(), 2);
    }

    #[test]
    fn history_is_capped_and_changing_the_config_clears_it() {
        let mut sampler = Sampler::new(TelemetryConfig::new(5, 30));
        let t0 = Instant::now();
        let sys = machine("cpu  1 0 1 8 0 0 0 0 0 0\n", 0, 0, 0);
        for i in 0..40 {
            sampler.sample(&sys, t0 + Duration::from_secs(i * 5));
        }
        assert_eq!(
            sampler.cpu.len(),
            TelemetryConfig::new(5, 30).history_capacity()
        );
        assert!(!sampler.configure(TelemetryConfig::new(5, 30)));
        assert!(sampler.configure(TelemetryConfig::new(1, 30)));
        assert!(sampler.cpu.is_empty() && sampler.due(t0));
    }

    #[test]
    fn missing_proc_files_give_none_not_zero() {
        let mut sampler = Sampler::new(TelemetryConfig::new(1, 60));
        sampler.sample(&FakeSystem::default(), Instant::now());
        assert_eq!(sampler.snapshot, Snapshot::default());
        assert_eq!(sampler.ram.back(), Some(&None));
    }

    #[test]
    fn amd_gpu_load_cpu_temperature_and_battery_come_from_sysfs() {
        let sys = FakeSystem::default()
            .with_file("/sys/class/drm/card0/device/gpu_busy_percent", "37\n")
            .with_file("/sys/class/drm/card0-DP-1/status", "connected")
            .with_file("/sys/class/hwmon/hwmon2/name", "nvme\n")
            .with_file("/sys/class/hwmon/hwmon3/name", "coretemp\n")
            .with_file("/sys/class/hwmon/hwmon3/temp1_label", "Package id 0\n")
            .with_file("/sys/class/hwmon/hwmon3/temp1_input", "61000\n")
            .with_file("/sys/class/hwmon/hwmon3/temp2_input", "40000\n")
            .with_file("/sys/class/power_supply/BAT0/capacity", "82\n")
            .with_file("/sys/class/power_supply/BAT0/status", "Charging\n");
        assert_eq!(gpu_busy(&sys), Some(37.0));
        assert_eq!(cpu_temperature(&sys), Some(61.0));
        assert_eq!(
            battery(&sys),
            Some(Battery {
                percent: 82,
                charging: true
            })
        );
    }

    #[test]
    fn system_info_reads_distro_kernel_cpu_and_graphics_vendor() {
        let sys = FakeSystem::default()
            .with_file("/etc/os-release", "PRETTY_NAME=\"Fedora Linux 42\"\n")
            .with_file("/proc/sys/kernel/osrelease", "6.12.1\n")
            .with_file("/proc/cpuinfo", "processor : 0\nmodel name : Test CPU\n")
            .with_file("/proc/meminfo", "MemTotal: 2048 kB\nMemAvailable: 1 kB\n")
            .with_file("/sys/class/drm/card0/device/vendor", "0x1002\n")
            .with_file(
                "/sys/class/drm/card0/device/uevent",
                "DRIVER=amdgpu\nPCI_ID=1002:73BF\n",
            );
        let info = read_info(&sys);
        assert_eq!(info.distro.as_deref(), Some("Fedora Linux 42"));
        assert_eq!(info.kernel.as_deref(), Some("6.12.1"));
        assert_eq!(info.cpu.as_deref(), Some("Test CPU"));
        assert_eq!(info.threads, 1);
        assert_eq!(info.total_memory, Some(2048 * 1024));
        assert_eq!(info.gpus, vec!["AMD graphics (amdgpu driver)".to_string()]);
    }
}
