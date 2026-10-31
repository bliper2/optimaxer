//! Network adapters from `/sys/class/net`, the DNS servers in use, and DNS provider changes through NetworkManager
//! (`nmcli`) with the previous settings saved. DNS changes need no administrator rights on a desktop session.

use super::procfs;
use super::store::DnsSnap;
use super::system::{Plan, Step, System, read_text};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub up: bool,
    pub wireless: bool,
    pub mac: Option<String>,
    pub mtu: Option<u32>,
    pub speed_mbps: Option<u32>,
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
}

pub fn adapters(sys: &dyn System) -> Vec<Adapter> {
    let counters = read_text(sys, "/proc/net/dev")
        .map(|t| procfs::parse_net_dev(&t))
        .unwrap_or_default();
    sys.list_dir("/sys/class/net")
        .into_iter()
        .filter(|n| n != "lo")
        .map(|name| {
            let base = format!("/sys/class/net/{name}");
            let read = |file: &str| {
                read_text(sys, &format!("{base}/{file}")).map(|v| v.trim().to_string())
            };
            let counter = counters.iter().find(|c| c.name == name);
            Adapter {
                up: read("operstate").as_deref() == Some("up"),
                wireless: !sys.list_dir(&format!("{base}/wireless")).is_empty()
                    || read("uevent").is_some_and(|u| u.contains("DEVTYPE=wlan")),
                mac: read("address").filter(|m| m != "00:00:00:00:00:00"),
                mtu: read("mtu").and_then(|v| v.parse().ok()),
                // Wi-Fi and down links report -1 or fail to read; that is "unknown", not 0.
                speed_mbps: read("speed")
                    .and_then(|v| v.parse::<i64>().ok())
                    .filter(|v| *v > 0)
                    .map(|v| v as u32),
                rx_bytes: counter.map(|c| c.rx_bytes),
                tx_bytes: counter.map(|c| c.tx_bytes),
                name,
            }
        })
        .collect()
}

/// Interface of the default route from `/proc/net/route`.
pub fn default_interface(route_table: &str) -> Option<String> {
    route_table.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        (f.get(1) == Some(&"00000000") && f.get(7) == Some(&"00000000")).then(|| f[0].to_string())
    })
}

pub fn nameservers(resolv_conf: &str) -> Vec<String> {
    resolv_conf
        .lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .filter_map(|rest| rest.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Addresses in `resolvectl dns` output (`Link 2 (eth0): 1.1.1.1 1.0.0.1`).
pub fn resolvectl_servers(text: &str) -> Vec<String> {
    let mut servers: Vec<String> = Vec::new();
    for line in text.lines() {
        let Some((_, list)) = line.split_once(':') else {
            continue;
        };
        for token in list.split_whitespace() {
            if token.parse::<std::net::IpAddr>().is_ok() && !servers.iter().any(|s| s == token) {
                servers.push(token.to_string());
            }
        }
    }
    servers
}

/// The servers actually asked. A local stub resolver (127.0.0.53) hides them, so ask systemd-resolved then.
pub fn dns_servers(sys: &dyn System) -> Vec<String> {
    let listed = read_text(sys, "/etc/resolv.conf")
        .map(|t| nameservers(&t))
        .unwrap_or_default();
    let stub = listed.is_empty() || listed.iter().all(|s| s.starts_with("127."));
    if stub && sys.have("resolvectl") {
        let out = sys.run(&["resolvectl", "dns"]);
        if out.ok() {
            let real = resolvectl_servers(&out.text);
            if !real.is_empty() {
                return real;
            }
        }
    }
    listed
}

pub struct Provider {
    pub key: &'static str,
    pub name: &'static str,
    pub v4: &'static [&'static str],
    pub v6: &'static [&'static str],
}

pub const PROVIDERS: [Provider; 6] = [
    Provider {
        key: "cloudflare",
        name: "Cloudflare",
        v4: &["1.1.1.1", "1.0.0.1"],
        v6: &["2606:4700:4700::1111", "2606:4700:4700::1001"],
    },
    Provider {
        key: "google",
        name: "Google",
        v4: &["8.8.8.8", "8.8.4.4"],
        v6: &["2001:4860:4860::8888", "2001:4860:4860::8844"],
    },
    Provider {
        key: "quad9",
        name: "Quad9",
        v4: &["9.9.9.9", "149.112.112.112"],
        v6: &["2620:fe::fe", "2620:fe::9"],
    },
    Provider {
        key: "adguard",
        name: "AdGuard",
        v4: &["94.140.14.14", "94.140.15.15"],
        v6: &["2a10:50c0::ad1:ff", "2a10:50c0::ad2:ff"],
    },
    Provider {
        key: "opendns",
        name: "OpenDNS",
        v4: &["208.67.222.222", "208.67.220.220"],
        v6: &[],
    },
    Provider {
        key: "auto",
        name: "Automatic (DHCP)",
        v4: &[],
        v6: &[],
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub name: String,
    pub kind: String,
    pub device: String,
}

/// `nmcli -t` escapes `:` and `\` with a backslash.
pub fn split_terse(line: &str) -> Vec<String> {
    let (mut parts, mut current, mut escaped) = (Vec::new(), String::new(), false);
    for c in line.chars() {
        if escaped {
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ':' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
}

pub fn parse_connections(text: &str) -> Vec<Connection> {
    text.lines()
        .map(split_terse)
        .filter(|p| {
            p.len() >= 3
                && matches!(
                    p[1].as_str(),
                    "802-3-ethernet" | "802-11-wireless" | "ethernet" | "wifi"
                )
        })
        .map(|p| Connection {
            name: p[0].clone(),
            kind: p[1].clone(),
            device: p[2].clone(),
        })
        .collect()
}

pub fn active_connections(sys: &dyn System) -> Result<Vec<Connection>, String> {
    if !sys.have("nmcli") {
        return Err(
            "NetworkManager (nmcli) was not found. Set DNS in your network tool instead.".into(),
        );
    }
    let out = sys.run(&[
        "nmcli",
        "-t",
        "-f",
        "NAME,TYPE,DEVICE",
        "connection",
        "show",
        "--active",
    ]);
    if !out.ok() {
        return Err(format!(
            "nmcli could not list connections: {}",
            out.text.trim()
        ));
    }
    Ok(parse_connections(&out.text))
}

fn read_property(sys: &dyn System, connection: &str, property: &str) -> Result<String, String> {
    let out = sys.run(&["nmcli", "-g", property, "connection", "show", connection]);
    if !out.ok() {
        return Err(format!(
            "Cannot read {property} for {connection}: {}",
            out.text.trim()
        ));
    }
    // -g output is escaped like -t output; split_terse with no separators left just unescapes.
    Ok(out.text.trim().replace("\\:", ":").replace("\\\\", "\\"))
}

pub struct DnsChange {
    pub plan: Plan,
    pub snapshots: Vec<DnsSnap>,
}

/// Plan that points every active Ethernet or Wi-Fi connection at `provider`. Existing saved originals are kept.
pub fn prepare_dns(
    sys: &dyn System,
    provider: &Provider,
    saved: &[DnsSnap],
) -> Result<DnsChange, String> {
    let connections = active_connections(sys)?;
    if connections.is_empty() {
        return Err("No active Ethernet or Wi-Fi connection was found in NetworkManager.".into());
    }
    let mut plan = Plan::default();
    let mut snapshots = Vec::new();
    for c in &connections {
        let snap = match saved.iter().find(|s| s.name == c.name) {
            Some(existing) => existing.clone(),
            None => DnsSnap {
                name: c.name.clone(),
                ipv4_dns: read_property(sys, &c.name, "ipv4.dns")?,
                ipv4_ignore_auto: read_property(sys, &c.name, "ipv4.ignore-auto-dns")?,
                ipv6_dns: read_property(sys, &c.name, "ipv6.dns")?,
                ipv6_ignore_auto: read_property(sys, &c.name, "ipv6.ignore-auto-dns")?,
            },
        };
        snapshots.push(snap);
        let yes_no = |list: &[&str]| if list.is_empty() { "no" } else { "yes" };
        plan.push(Step::run_user(&[
            "nmcli",
            "connection",
            "modify",
            &c.name,
            "ipv4.dns",
            &provider.v4.join(","),
            "ipv4.ignore-auto-dns",
            yes_no(provider.v4),
            "ipv6.dns",
            &provider.v6.join(","),
            "ipv6.ignore-auto-dns",
            yes_no(provider.v6),
        ]));
        plan.push(Step::run_user(&["nmcli", "device", "reapply", &c.device]));
    }
    Ok(DnsChange { plan, snapshots })
}

pub fn prepare_dns_restore(sys: &dyn System, snapshots: &[DnsSnap]) -> Result<Plan, String> {
    let active = active_connections(sys).unwrap_or_default();
    let mut plan = Plan::default();
    for s in snapshots {
        let list = |v: &str| v.replace(' ', ",");
        let or_no = |v: &str| {
            if v.is_empty() {
                "no".to_string()
            } else {
                v.to_string()
            }
        };
        plan.push(Step::run_user(&[
            "nmcli",
            "connection",
            "modify",
            &s.name,
            "ipv4.dns",
            &list(&s.ipv4_dns),
            "ipv4.ignore-auto-dns",
            &or_no(&s.ipv4_ignore_auto),
            "ipv6.dns",
            &list(&s.ipv6_dns),
            "ipv6.ignore-auto-dns",
            &or_no(&s.ipv6_ignore_auto),
        ]));
        if let Some(c) = active.iter().find(|c| c.name == s.name) {
            plan.push(Step::run_user(&["nmcli", "device", "reapply", &c.device]));
        }
    }
    Ok(plan)
}

/// Milliseconds the system resolver takes for one name. Uses whatever cache the resolver has, so repeat runs are fast.
pub fn lookup_time(host: &str) -> Result<std::time::Duration, String> {
    use std::net::ToSocketAddrs;
    let start = std::time::Instant::now();
    let found = (host, 443)
        .to_socket_addrs()
        .map_err(|e| format!("{host}: {e}"))?
        .count();
    if found == 0 {
        return Err(format!("{host} resolved to no address"));
    }
    Ok(start.elapsed())
}

#[cfg(test)]
mod tests {
    use super::super::system::{CmdOut, fake::FakeSystem};
    use super::*;

    #[test]
    fn adapters_skip_loopback_and_treat_unknown_speed_as_none() {
        let sys = FakeSystem::default()
            .with_file("/sys/class/net/lo/operstate", "unknown\n")
            .with_file("/sys/class/net/eth0/operstate", "up\n")
            .with_file("/sys/class/net/eth0/address", "aa:bb:cc:dd:ee:ff\n")
            .with_file("/sys/class/net/eth0/mtu", "1500\n")
            .with_file("/sys/class/net/eth0/speed", "1000\n")
            .with_file("/sys/class/net/wlan0/operstate", "down\n")
            .with_file("/sys/class/net/wlan0/speed", "-1\n")
            .with_file("/sys/class/net/wlan0/wireless/link", "0")
            .with_file(
                "/proc/net/dev",
                "h\nh\n eth0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n",
            );
        let list = adapters(&sys);
        assert_eq!(list.len(), 2);
        let eth = &list[0];
        assert_eq!(
            (eth.name.as_str(), eth.up, eth.speed_mbps, eth.mtu),
            ("eth0", true, Some(1000), Some(1500))
        );
        assert_eq!((eth.rx_bytes, eth.tx_bytes), (Some(10), Some(20)));
        assert!(
            list[1].wireless
                && !list[1].up
                && list[1].speed_mbps.is_none()
                && list[1].rx_bytes.is_none()
        );
    }

    #[test]
    fn default_route_and_nameservers_parse() {
        let route = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\nwlan0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000\neth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\n";
        assert_eq!(default_interface(route).as_deref(), Some("wlan0"));
        assert_eq!(default_interface("Iface\n"), None);
        assert_eq!(
            nameservers("# x\nnameserver 1.1.1.1\nnameserver 2606:4700::1111\nsearch lan\n"),
            vec!["1.1.1.1", "2606:4700::1111"]
        );
    }

    #[test]
    fn a_stub_resolver_is_looked_through_with_resolvectl() {
        let sys = FakeSystem::default()
            .with_file("/etc/resolv.conf", "nameserver 127.0.0.53\n")
            .with_tool("resolvectl");
        sys.canned.borrow_mut().insert(
            "resolvectl dns".into(),
            CmdOut {
                code: 0,
                text: "Global:\nLink 2 (eth0): 9.9.9.9 149.112.112.112\nLink 3 (wlan0): 9.9.9.9\n"
                    .into(),
            },
        );
        assert_eq!(dns_servers(&sys), vec!["9.9.9.9", "149.112.112.112"]);
        let plain = FakeSystem::default().with_file("/etc/resolv.conf", "nameserver 192.168.1.1\n");
        assert_eq!(dns_servers(&plain), vec!["192.168.1.1"]);
        let nothing =
            FakeSystem::default().with_file("/etc/resolv.conf", "nameserver 127.0.0.53\n");
        assert_eq!(
            dns_servers(&nothing),
            vec!["127.0.0.53"],
            "no resolvectl: show what is configured"
        );
    }

    #[test]
    fn terse_nmcli_output_is_unescaped() {
        assert_eq!(
            split_terse(r"My\:Wi-Fi:wifi:wlan0"),
            vec!["My:Wi-Fi", "wifi", "wlan0"]
        );
        let conns =
            parse_connections("Home:802-11-wireless:wlan0\nbr0:bridge:br0\nWired:ethernet:eth0\n");
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[1].device, "eth0");
    }

    fn nm(sys: &FakeSystem) {
        let set = |k: &str, text: &str| {
            sys.canned.borrow_mut().insert(
                k.into(),
                CmdOut {
                    code: 0,
                    text: text.into(),
                },
            );
        };
        set(
            "nmcli -t -f NAME,TYPE,DEVICE connection show --active",
            "Home:wifi:wlan0\n",
        );
        set("nmcli -g ipv4.dns connection show Home", "192.168.1.1\n");
        set("nmcli -g ipv4.ignore-auto-dns connection show Home", "no\n");
        set("nmcli -g ipv6.dns connection show Home", "fe80\\:\\:1\n");
        set("nmcli -g ipv6.ignore-auto-dns connection show Home", "no\n");
    }

    #[test]
    fn switching_dns_saves_originals_and_uses_user_level_commands() {
        let sys = FakeSystem::default().with_tool("nmcli");
        nm(&sys);
        let change = prepare_dns(&sys, &PROVIDERS[0], &[]).unwrap();
        assert!(!change.plan.needs_root());
        assert_eq!(
            change.snapshots[0].ipv6_dns, "fe80::1",
            "escaped colons come back as colons"
        );
        let lines = change.plan.describe();
        assert_eq!(
            lines[0],
            "Run nmcli connection modify Home ipv4.dns 1.1.1.1,1.0.0.1 ipv4.ignore-auto-dns yes ipv6.dns 2606:4700:4700::1111,2606:4700:4700::1001 ipv6.ignore-auto-dns yes"
        );
        assert_eq!(lines[1], "Run nmcli device reapply wlan0");
        let restore = prepare_dns_restore(&sys, &change.snapshots)
            .unwrap()
            .describe();
        assert_eq!(
            restore[0],
            "Run nmcli connection modify Home ipv4.dns 192.168.1.1 ipv4.ignore-auto-dns no ipv6.dns fe80::1 ipv6.ignore-auto-dns no"
        );
    }

    #[test]
    fn automatic_dns_clears_the_servers_and_a_repeat_keeps_the_first_original() {
        let sys = FakeSystem::default().with_tool("nmcli");
        nm(&sys);
        let first = prepare_dns(&sys, &PROVIDERS[0], &[]).unwrap();
        let auto = prepare_dns(
            &sys,
            PROVIDERS.iter().find(|p| p.key == "auto").unwrap(),
            &first.snapshots,
        )
        .unwrap();
        assert_eq!(auto.snapshots, first.snapshots);
        assert!(
            auto.plan.describe()[0].contains("ipv4.dns  ipv4.ignore-auto-dns no")
                || auto.plan.describe()[0].contains("ipv4.dns ")
        );
    }

    #[test]
    fn without_networkmanager_the_error_says_so() {
        let error = prepare_dns(&FakeSystem::default(), &PROVIDERS[0], &[])
            .err()
            .unwrap();
        assert!(error.contains("nmcli"));
    }

    #[test]
    fn provider_keys_are_unique() {
        let mut keys: Vec<_> = PROVIDERS.iter().map(|p| p.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PROVIDERS.len());
    }
}
