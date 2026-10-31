//! Parsers for `/proc` and `/sys` text. Nothing here touches the disk, so the readings can be tested with fixtures.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

/// The aggregate `cpu` line of `/proc/stat`. Guest time is already counted inside user time, so it is not added again.
pub fn parse_cpu_total(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    if fields.len() < 4 {
        return None;
    }
    let get = |i: usize| fields.get(i).copied().unwrap_or(0);
    let idle = get(3) + get(4);
    let total = (0..8).map(get).sum();
    Some(CpuTimes { idle, total })
}

/// Busy share between two readings, 0 to 100. `None` when no time passed or the counters went backwards.
pub fn cpu_percent(prev: CpuTimes, now: CpuTimes) -> Option<f32> {
    let total = now.total.checked_sub(prev.total)?;
    let idle = now.idle.checked_sub(prev.idle)?;
    if total == 0 || idle > total {
        return None;
    }
    Some(((total - idle) as f32 / total as f32 * 100.0).clamp(0.0, 100.0))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

impl MemInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn swap_used(&self) -> u64 {
        self.swap_total.saturating_sub(self.swap_free)
    }
}

/// `/proc/meminfo` values are in kB; the result is in bytes. Needs `MemTotal` and `MemAvailable` (kernel 3.14+).
pub fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let mut info = MemInfo::default();
    let (mut have_total, mut have_available) = (false, false);
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let kb: Option<u64> = rest.split_whitespace().next().and_then(|v| v.parse().ok());
        let Some(bytes) = kb.and_then(|v| v.checked_mul(1024)) else {
            continue;
        };
        match key {
            "MemTotal" => {
                info.total = bytes;
                have_total = true;
            }
            "MemAvailable" => {
                info.available = bytes;
                have_available = true;
            }
            "SwapTotal" => info.swap_total = bytes,
            "SwapFree" => info.swap_free = bytes,
            _ => {}
        }
    }
    (have_total && have_available && info.total > 0).then_some(info)
}

pub fn parse_uptime(text: &str) -> Option<u64> {
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then_some(seconds as u64)
}

pub fn parse_loadavg(text: &str) -> Option<(f32, f32, f32)> {
    let mut parts = text.split_whitespace();
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetDev {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

pub fn parse_net_dev(text: &str) -> Vec<NetDev> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let fields: Vec<&str> = rest.split_whitespace().collect();
            Some(NetDev {
                name: name.trim().to_string(),
                rx_bytes: fields.first()?.parse().ok()?,
                tx_bytes: fields.get(8)?.parse().ok()?,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskStat {
    pub name: String,
    /// Milliseconds the device spent with I/O in flight (`io_ticks`); its delta over wall time is the busy share.
    pub io_ms: u64,
}

pub fn parse_diskstats(text: &str) -> Vec<DiskStat> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            Some(DiskStat {
                name: f.get(2)?.to_string(),
                io_ms: f.get(12)?.parse().ok()?,
            })
        })
        .collect()
}

/// Virtual devices that would double count or never represent hardware.
pub fn is_virtual_block(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "sr", "fd"]
        .iter()
        .any(|p| name.starts_with(p))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PidStat {
    pub pid: u32,
    pub name: String,
    pub state: char,
    pub ppid: u32,
    /// User plus kernel CPU time in clock ticks.
    pub cpu_ticks: u64,
    pub threads: u32,
    pub rss_pages: u64,
}

/// One `/proc/<pid>/stat` line. The name sits in parentheses and may itself contain spaces and parentheses, so the
/// fields are found from the last closing parenthesis.
pub fn parse_pid_stat(line: &str) -> Option<PidStat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close < open {
        return None;
    }
    let pid = line[..open].trim().parse().ok()?;
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    // rest[0] is field 3 (state); utime is field 14, stime 15, num_threads 20, rss 24.
    let field = |n: usize| rest.get(n - 3).copied();
    Some(PidStat {
        pid,
        name: line[open + 1..close].to_string(),
        state: field(3)?.chars().next()?,
        ppid: field(4)?.parse().ok()?,
        cpu_ticks: field(14)?.parse::<u64>().ok()? + field(15)?.parse::<u64>().ok()?,
        threads: field(20)?.parse().ok()?,
        rss_pages: field(24)?.parse().ok()?,
    })
}

/// `Key: value` pairs from `/proc/<pid>/status` style text.
pub fn status_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k == key).then(|| v.trim())
    })
}

/// First uid column of the `Uid:` line.
pub fn parse_uid(status: &str) -> Option<u32> {
    status_value(status, "Uid")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub fn parse_passwd(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            Some((f.get(2)?.parse().ok()?, f.first()?.to_string()))
        })
        .collect()
}

/// `PRETTY_NAME` and friends from `/etc/os-release`.
pub fn os_release_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

pub fn cpu_model(cpuinfo: &str) -> Option<String> {
    cpuinfo
        .lines()
        .find(|l| {
            l.starts_with("model name") || l.starts_with("Model") || l.starts_with("Hardware")
        })
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn cpu_threads(cpuinfo: &str) -> usize {
    cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor") && l.contains(':'))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str =
        "cpu  4705 150 1120 16250 520 30 45 0 0 0\ncpu0 2350 70 560 8100 260 15 20 0 0 0\nintr 1\n";

    #[test]
    fn cpu_line_sums_busy_and_idle_time() {
        let t = parse_cpu_total(STAT).unwrap();
        assert_eq!(t.idle, 16250 + 520);
        assert_eq!(t.total, 4705 + 150 + 1120 + 16250 + 520 + 30 + 45);
        assert!(parse_cpu_total("cpu0 1 2 3 4\n").is_none());
        assert!(parse_cpu_total("cpu  1 2\n").is_none());
    }

    #[test]
    fn cpu_percent_comes_from_deltas_and_rejects_resets() {
        let a = CpuTimes {
            idle: 100,
            total: 200,
        };
        let b = CpuTimes {
            idle: 150,
            total: 300,
        };
        assert_eq!(cpu_percent(a, b), Some(50.0));
        assert_eq!(cpu_percent(b, a), None);
        assert_eq!(cpu_percent(a, a), None);
        assert_eq!(
            cpu_percent(
                a,
                CpuTimes {
                    idle: 400,
                    total: 250
                }
            ),
            None
        );
    }

    #[test]
    fn meminfo_reads_kilobytes_as_bytes() {
        let m = parse_meminfo(
            "MemTotal:       16384 kB\nMemFree: 100 kB\nMemAvailable:    4096 kB\nSwapTotal: 2048 kB\nSwapFree: 512 kB\n",
        )
        .unwrap();
        assert_eq!(m.total, 16384 * 1024);
        assert_eq!(m.used(), 12288 * 1024);
        assert_eq!(m.swap_used(), 1536 * 1024);
        assert!(
            parse_meminfo("MemTotal: 10 kB\n").is_none(),
            "no MemAvailable means no honest used value"
        );
        assert!(parse_meminfo("").is_none());
    }

    #[test]
    fn uptime_and_load() {
        assert_eq!(parse_uptime("12345.67 99999.00\n"), Some(12345));
        assert_eq!(parse_uptime("x"), None);
        assert_eq!(
            parse_loadavg("0.52 0.40 0.30 1/500 1234\n"),
            Some((0.52, 0.40, 0.30))
        );
        assert_eq!(parse_loadavg("0.52"), None);
    }

    #[test]
    fn net_dev_skips_headers_and_reads_rx_tx() {
        let text = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0\n  eth0: 5000 50 0 0 0 0 0 0 7000 60 0 0 0 0 0 0\n";
        let devs = parse_net_dev(text);
        assert_eq!(devs.len(), 2);
        assert_eq!(
            devs[1],
            NetDev {
                name: "eth0".into(),
                rx_bytes: 5000,
                tx_bytes: 7000
            }
        );
    }

    #[test]
    fn diskstats_reads_io_ticks_and_flags_virtual_devices() {
        let text = "   8       0 sda 100 0 800 50 200 0 1600 90 0 400 140 0 0 0 0\n 259 0 nvme0n1 1 0 8 1 1 0 8 1 0 77 2 0 0 0 0\n   7 0 loop0 1 0 8 1 0 0 0 0 0 5 1\n";
        let disks = parse_diskstats(text);
        assert_eq!(
            disks[0],
            DiskStat {
                name: "sda".into(),
                io_ms: 400
            }
        );
        assert_eq!(disks[1].io_ms, 77);
        assert!(
            is_virtual_block("loop0") && is_virtual_block("dm-1") && !is_virtual_block("nvme0n1")
        );
    }

    #[test]
    fn pid_stat_handles_names_with_spaces_and_parentheses() {
        let line = "4242 (Web Content (x)) S 1 4242 4242 0 -1 4194560 100 0 0 0 150 50 0 0 20 0 12 0 5000 123456789 2500 18446744073709551615";
        let p = parse_pid_stat(line).unwrap();
        assert_eq!(p.pid, 4242);
        assert_eq!(p.name, "Web Content (x)");
        assert_eq!(p.state, 'S');
        assert_eq!(p.ppid, 1);
        assert_eq!(p.cpu_ticks, 200);
        assert_eq!(p.threads, 12);
        assert_eq!(p.rss_pages, 2500);
        assert!(parse_pid_stat("garbage").is_none());
        assert!(parse_pid_stat("1 (init) S 0").is_none());
    }

    #[test]
    fn status_passwd_and_os_release() {
        assert_eq!(
            parse_uid("Name:\tbash\nUid:\t1000\t1000\t1000\t1000\n"),
            Some(1000)
        );
        assert_eq!(
            parse_passwd("root:x:0:0:root:/root:/bin/bash\nme:x:1000:1000::/home/me:/bin/sh\n"),
            vec![(0, "root".to_string()), (1000, "me".to_string())]
        );
        assert_eq!(
            os_release_value("NAME=\"Arch\"\nPRETTY_NAME=\"Arch Linux\"\n", "PRETTY_NAME")
                .as_deref(),
            Some("Arch Linux")
        );
        assert_eq!(os_release_value("A=b\n", "PRETTY_NAME"), None);
    }

    #[test]
    fn cpuinfo_model_and_threads() {
        let text = "processor\t: 0\nmodel name\t: AMD Ryzen 7\nprocessor\t: 1\nmodel name\t: AMD Ryzen 7\n";
        assert_eq!(cpu_model(text).as_deref(), Some("AMD Ryzen 7"));
        assert_eq!(cpu_threads(text), 2);
        assert_eq!(cpu_model(""), None);
    }
}
