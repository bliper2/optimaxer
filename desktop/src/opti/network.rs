//! Network page: DNS provider switch (optionally with DNS-over-HTTPS), cache flush, stack reset and DNS query timing.
//! Adapters come from GetAdaptersAddresses; changes go through netsh.

use super::backend::System;
use super::services::DnsProvider;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
use windows_sys::Win32::NetworkManagement::IpHelper::{GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH};
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

fn pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: GetAdaptersAddresses returns NUL-terminated strings inside the buffer we own.
    let len = (0..).take_while(|i| unsafe { *p.add(*i) } != 0).count();
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

/// Friendly names of adapters that are up, wired or Wi-Fi, and not virtual.
pub fn active_adapters() -> Vec<String> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size = 32 * 1024u32;
    let mut buf: Vec<u64> = vec![0; size as usize / 8 + 1];
    // SAFETY: standard size-then-fill pattern; the u64 buffer is aligned for IP_ADAPTER_ADDRESSES_LH.
    unsafe {
        let mut r = GetAdaptersAddresses(AF_UNSPEC as u32, flags, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size);
        if r == ERROR_BUFFER_OVERFLOW {
            buf = vec![0; size as usize / 8 + 1];
            r = GetAdaptersAddresses(AF_UNSPEC as u32, flags, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size);
        }
        if r != 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cur = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !cur.is_null() {
            let a = &*cur;
            let name = pwstr(a.FriendlyName);
            let desc = pwstr(a.Description);
            if is_physical_candidate(a.OperStatus, a.IfType, &name, &desc) {
                out.push(name);
            }
            cur = a.Next;
        }
        out
    }
}

/// IfOperStatusUp (1), Ethernet (6) or Wi-Fi (71), and not a virtual, VPN or tunnel adapter.
pub fn is_physical_candidate(oper_status: i32, if_type: u32, name: &str, desc: &str) -> bool {
    if oper_status != 1 || !(if_type == 6 || if_type == 71) {
        return false;
    }
    let text = format!("{name} {desc}").to_lowercase();
    !["virtual", "vmware", "vethernet", "tap-", "tap ", "wintun", "wireguard", "loopback", "bluetooth", "vpn", "tailscale", "zerotier"].iter().any(|w| text.contains(w))
}

/// netsh argument lists that point one adapter at a provider, or back to DHCP when the provider has no servers.
pub fn dns_commands(alias: &str, p: &DnsProvider) -> Vec<Vec<String>> {
    let name = format!("name={alias}");
    let mut out = Vec::new();
    for (proto, servers) in [("ipv4", &p.v4), ("ipv6", &p.v6)] {
        let base = |verb: &str| vec!["interface".to_string(), proto.to_string(), verb.to_string(), "dnsservers".to_string(), name.clone()];
        if p.v4.is_empty() {
            let mut c = base("set");
            c.push("source=dhcp".into());
            out.push(c);
            continue;
        }
        for (i, ip) in servers.iter().enumerate() {
            let mut c = base(if i == 0 { "set" } else { "add" });
            if i == 0 {
                c.push("source=static".into());
            }
            c.push(format!("address={ip}"));
            if i > 0 {
                c.push(format!("index={}", i + 1));
            }
            c.push("validate=no".into());
            out.push(c);
        }
        if servers.is_empty() {
            // provider has no IPv6 servers: clear any old ones
            let mut c = base("set");
            c.push("source=dhcp".into());
            out.push(c);
        }
    }
    out
}

pub fn doh_commands(p: &DnsProvider) -> Vec<Vec<String>> {
    if p.doh.is_empty() {
        return Vec::new();
    }
    p.v4.iter().chain(p.v6.iter()).map(|ip| vec!["dns".into(), "add".into(), "encryption".into(), format!("server={ip}"), format!("dohtemplate={}", p.doh), "autoupgrade=yes".into(), "udpfallback=yes".into()]).collect()
}

fn netsh(sys: &dyn System, args: &[String]) -> super::backend::CmdOut {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    sys.run("netsh.exe", &refs)
}

pub fn apply_dns(sys: &dyn System, adapters: &[String], p: &DnsProvider, log: &mut dyn FnMut(&str)) {
    if adapters.is_empty() {
        log("No active network adapter found");
        return;
    }
    for a in adapters {
        let mut failed = false;
        for c in dns_commands(a, p) {
            let out = netsh(sys, &c);
            if out.code != 0 {
                failed = true;
                log(&format!("  WARN DNS on {a}: {}", out.text.trim().lines().next().unwrap_or("netsh failed")));
                break;
            }
        }
        if !failed {
            log(&format!("  DNS on {a}: {}", if p.v4.is_empty() { "automatic" } else { &p.name }));
        }
    }
    for c in doh_commands(p) {
        let out = netsh(sys, &c);
        if out.code != 0 {
            log(&format!("  WARN DNS-over-HTTPS registration failed: {}", out.text.trim()));
        }
    }
    let out = sys.run("ipconfig.exe", &["/flushdns"]);
    if out.code != 0 {
        log(&format!("  WARN DNS cache flush failed: {}", out.text.trim()));
    }
}

pub fn show_dns(sys: &dyn System, log: &mut dyn FnMut(&str)) {
    let out = sys.run("netsh.exe", &["interface", "ipv4", "show", "dnsservers"]);
    for l in out.text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        log(&format!("  {l}"));
    }
}

pub fn reset_stack(sys: &dyn System, log: &mut dyn FnMut(&str)) {
    let mut succeeded = true;
    for (exe, args) in [("netsh.exe", &["winsock", "reset"][..]), ("netsh.exe", &["int", "ip", "reset"][..]), ("ipconfig.exe", &["/flushdns"][..])] {
        let out = sys.run(exe, args);
        if out.code != 0 {
            succeeded = false;
            log(&format!("  WARN {exe} {} failed: {}", args.join(" "), out.text.trim()));
        }
    }
    log(if succeeded { "Network stack reset. Restart Windows to finish." } else { "Network reset incomplete. Review the failed steps and restart Windows after resolving them." });
}

const DNS_TIMEOUT: Duration = Duration::from_secs(1);
const DNS_MAX_PROBES: u32 = 5;
static NEXT_QUERY_ID: AtomicU16 = AtomicU16::new(1);

#[derive(Clone, Debug, Default)]
pub struct DnsBenchmark {
    pub attempts: u32,
    pub answers: u32,
    pub average_ms: Option<f64>,
    /// Mean absolute change between consecutive successful DNS response times.
    pub jitter_ms: Option<f64>,
}

/// Times UDP A-record queries against a DNS server. Failed probes are not IP packet-loss data.
pub fn benchmark_dns(ip: &str, samples: u32) -> DnsBenchmark {
    let Ok(ip) = ip.parse::<IpAddr>() else {
        return DnsBenchmark::default();
    };
    benchmark_dns_at(SocketAddr::new(ip, 53), samples, DNS_TIMEOUT)
}

fn benchmark_dns_at(server: SocketAddr, samples: u32, timeout: Duration) -> DnsBenchmark {
    let attempts = samples.clamp(1, DNS_MAX_PROBES);
    let mut times = Vec::with_capacity(attempts as usize);
    for _ in 0..attempts {
        if let Some(ms) = dns_query_ms(server, timeout) {
            times.push(ms);
        }
    }
    let average_ms = average(&times);
    let jitter_ms = if times.len() > 1 {
        let changes: Vec<f64> = times.windows(2).map(|pair| (pair[1] - pair[0]).abs()).collect();
        average(&changes)
    } else {
        None
    };
    DnsBenchmark { attempts, answers: times.len() as u32, average_ms, jitter_ms }
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| (values.iter().sum::<f64>() / values.len() as f64 * 10.0).round() / 10.0)
}

fn dns_query_ms(server: SocketAddr, timeout: Duration) -> Option<f64> {
    let bind_addr = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let socket = UdpSocket::bind(bind_addr).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    let id = NEXT_QUERY_ID.fetch_add(1, Ordering::Relaxed);
    let query = dns_query(id);
    let started = Instant::now();
    socket.send_to(&query, server).ok()?;
    let mut response = [0u8; 2048];
    let (length, source) = socket.recv_from(&mut response).ok()?;
    if source != server || !valid_dns_answer(&response[..length], id) {
        return None;
    }
    Some(started.elapsed().as_secs_f64() * 1000.0)
}

fn dns_query(id: u16) -> Vec<u8> {
    let mut query = Vec::with_capacity(29);
    query.extend_from_slice(&id.to_be_bytes());
    query.extend_from_slice(&0x0100u16.to_be_bytes()); // standard query, recursion desired
    query.extend_from_slice(&1u16.to_be_bytes());
    query.extend_from_slice(&[0; 6]); // answer, authority, additional counts
    for label in ["example", "com"] {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&1u16.to_be_bytes()); // A
    query.extend_from_slice(&1u16.to_be_bytes()); // IN
    query
}

fn valid_dns_answer(response: &[u8], id: u16) -> bool {
    if response.len() < 12
        || u16::from_be_bytes([response[0], response[1]]) != id
        || response[2] & 0x80 == 0 // response, not another query
        || response[2] & 0x02 != 0 // not truncated
        || response[3] & 0x0f != 0 // NOERROR
        || u16::from_be_bytes([response[4], response[5]]) != 1
    {
        return false;
    }

    let answers = u16::from_be_bytes([response[6], response[7]]);
    if answers == 0 {
        return false;
    }

    let Some(mut offset) = skip_dns_name(response, 12).and_then(|end| end.checked_add(4)) else {
        return false;
    };
    if offset > response.len() {
        return false;
    }

    let mut found_ipv4 = false;
    for _ in 0..answers {
        let Some(end_name) = skip_dns_name(response, offset) else {
            return false;
        };
        let Some(record_end) = end_name.checked_add(10) else {
            return false;
        };
        if record_end > response.len() {
            return false;
        }
        let record_type = u16::from_be_bytes([response[end_name], response[end_name + 1]]);
        let record_class = u16::from_be_bytes([response[end_name + 2], response[end_name + 3]]);
        let data_length = u16::from_be_bytes([response[end_name + 8], response[end_name + 9]]) as usize;
        let Some(next_record) = record_end.checked_add(data_length) else {
            return false;
        };
        if next_record > response.len() {
            return false;
        }
        found_ipv4 |= record_type == 1 && record_class == 1 && data_length == 4;
        offset = next_record;
    }
    found_ipv4
}

fn skip_dns_name(message: &[u8], start: usize) -> Option<usize> {
    let mut offset = start;
    loop {
        let length = *message.get(offset)?;
        match length & 0xc0 {
            0xc0 => {
                let pointer = message.get(offset + 1)?;
                let target = usize::from(length & 0x3f) << 8 | usize::from(*pointer);
                return (target < offset).then_some(offset + 2);
            }
            0 => {
                offset += 1;
                if length == 0 {
                    return Some(offset);
                }
                if length > 63 {
                    return None;
                }
                offset = offset.checked_add(usize::from(length))?;
                if offset > message.len() {
                    return None;
                }
            }
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;
    use crate::opti::services::Data;
    use std::thread;

    fn provider(name: &str) -> DnsProvider {
        Data::load().dns.into_iter().find(|p| p.name == name).unwrap()
    }

    #[test]
    fn static_dns_commands_set_then_add() {
        let cmds = dns_commands("Ethernet 2", &provider("Cloudflare"));
        let v4: Vec<String> = cmds.iter().filter(|c| c[1] == "ipv4").map(|c| c.join(" ")).collect();
        assert_eq!(v4[0], "interface ipv4 set dnsservers name=Ethernet 2 source=static address=1.1.1.1 validate=no");
        assert_eq!(v4[1], "interface ipv4 add dnsservers name=Ethernet 2 address=1.0.0.1 index=2 validate=no");
        assert!(cmds.iter().any(|c| c[1] == "ipv6"));
    }

    #[test]
    fn automatic_resets_both_protocols_to_dhcp() {
        let cmds = dns_commands("Wi-Fi", &provider("Automatic (DHCP)"));
        assert_eq!(cmds.len(), 2);
        assert!(cmds.iter().all(|c| c.contains(&"source=dhcp".to_string())));
    }

    #[test]
    fn provider_without_ipv6_clears_old_ipv6_servers() {
        let cmds = dns_commands("Wi-Fi", &provider("OpenDNS"));
        assert!(cmds.iter().any(|c| c[1] == "ipv6" && c.contains(&"source=dhcp".to_string())));
    }

    #[test]
    fn dns_query_builds_recursive_example_com_a_lookup() {
        let query = dns_query(0x1234);
        assert_eq!(&query[..2], &[0x12, 0x34]);
        assert_eq!(&query[2..4], &[0x01, 0x00]);
        assert_eq!(&query[12..], b"\x07example\x03com\x00\x00\x01\x00\x01");
    }

    #[test]
    fn dns_response_validation_rejects_wrong_or_incomplete_answers() {
        let mut response = dns_query(0x1234);
        response[2] = 0x81;
        response[3] = 0x80;
        response[6..8].copy_from_slice(&1u16.to_be_bytes());
        response.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
        response.extend_from_slice(&[0, 0, 0, 60, 0, 4, 93, 184, 216, 34]);
        assert!(valid_dns_answer(&response, 0x1234));
        assert!(!valid_dns_answer(&response, 0x4321));
        response[2] = 0x01; // query, not response
        assert!(!valid_dns_answer(&response, 0x1234));
        response[2] = 0x83; // truncated response
        assert!(!valid_dns_answer(&response, 0x1234));
        response[2] = 0x81;
        response[3] = 0x83; // NXDOMAIN
        assert!(!valid_dns_answer(&response, 0x1234));
        response[3] = 0x80;
        response[6..8].copy_from_slice(&0u16.to_be_bytes());
        assert!(!valid_dns_answer(&response, 0x1234));
        response[6..8].copy_from_slice(&1u16.to_be_bytes());
        response.truncate(12);
        assert!(!valid_dns_answer(&response, 0x1234));
    }

    #[test]
    fn dns_benchmark_measures_udp_answers_and_response_jitter() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let responder = thread::spawn(move || {
            let mut query = [0u8; 512];
            for _ in 0..3 {
                let (length, client) = server.recv_from(&mut query).unwrap();
                let request = &query[..length];
                let mut answer = request.to_vec();
                answer[2] = 0x81; // response, recursion available
                answer[3] = 0x80;
                answer[6..8].copy_from_slice(&1u16.to_be_bytes());
                answer.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
                answer.extend_from_slice(&[0, 0, 0, 60, 0, 4, 93, 184, 216, 34]);
                server.send_to(&answer, client).unwrap();
            }
        });
        let result = benchmark_dns_at(address, 3, Duration::from_millis(500));
        responder.join().unwrap();
        assert_eq!(result.attempts, 3);
        assert_eq!(result.answers, 3);
        assert!(result.average_ms.is_some());
        assert!(result.jitter_ms.is_some());
    }

    #[test]
    fn dns_benchmark_limits_probes_and_reports_no_answer() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        drop(server);
        let result = benchmark_dns_at(address, u32::MAX, Duration::from_millis(30));
        assert_eq!(result.attempts, DNS_MAX_PROBES);
        assert_eq!(result.answers, 0);
        assert_eq!(result.average_ms, None);
        assert_eq!(result.jitter_ms, None);
    }

    #[test]
    fn doh_is_registered_only_for_providers_that_publish_a_template() {
        assert_eq!(doh_commands(&provider("OpenDNS")).len(), 0);
        let c = doh_commands(&provider("Cloudflare"));
        assert!(c.len() >= 2 && c[0].join(" ").contains("dohtemplate=https://cloudflare-dns.com/dns-query"));
    }

    #[test]
    fn virtual_adapters_are_skipped() {
        assert!(is_physical_candidate(1, 6, "Ethernet", "Intel(R) I225-V"));
        assert!(!is_physical_candidate(1, 6, "vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter"));
        assert!(!is_physical_candidate(2, 6, "Ethernet", "Intel"));
        assert!(!is_physical_candidate(1, 24, "Loopback", "Software Loopback"));
        assert!(!is_physical_candidate(1, 6, "Tailscale", "Tailscale Tunnel"));
    }

    #[test]
    fn applying_dns_runs_netsh_and_flushes() {
        let sys = FakeSystem::default();
        let mut lines = Vec::new();
        apply_dns(&sys, &["Ethernet".to_string()], &provider("Quad9"), &mut |l| lines.push(l.to_string()));
        assert!(sys.ran("netsh.exe interface ipv4 set dnsservers name=Ethernet source=static address=9.9.9.9"));
        assert!(sys.ran("ipconfig.exe /flushdns"));
        assert!(lines.iter().any(|l| l.contains("Quad9")));
    }

    #[test]
    fn no_adapter_changes_nothing() {
        let sys = FakeSystem::default();
        apply_dns(&sys, &[], &provider("Google"), &mut |_| {});
        assert!(sys.commands.borrow().is_empty());
    }

    #[test]
    fn failed_network_commands_are_reported_without_success() {
        let sys = FakeSystem::default();
        sys.fail.borrow_mut().insert("netsh.exe".into());
        let mut lines = Vec::new();
        reset_stack(&sys, &mut |l| lines.push(l.to_string()));
        assert!(lines.iter().any(|l| l.contains("incomplete")));
        assert!(!lines.iter().any(|l| l.starts_with("Network stack reset.")));
        lines.clear();
        apply_dns(&sys, &["Ethernet".into()], &provider("Cloudflare"), &mut |l| lines.push(l.to_string()));
        assert!(lines.iter().any(|l| l.contains("DNS-over-HTTPS registration failed")));
        assert!(!lines.iter().any(|l| l.contains("DNS on Ethernet: Cloudflare")));
    }
}
