//! Network page: DNS provider switch (optionally with DNS-over-HTTPS), cache flush, stack reset and a latency
//! benchmark. Adapters come from GetAdaptersAddresses; changes go through netsh.

use super::backend::System;
use super::services::DnsProvider;
use std::net::{SocketAddr, TcpStream};
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

/// Average TCP connect time to port 53 over `samples` tries, in milliseconds.
pub fn latency_ms(ip: &str, samples: u32) -> Option<f64> {
    let addr: SocketAddr = format!("{ip}:53").parse().ok()?;
    let times: Vec<f64> = (0..samples)
        .filter_map(|_| {
            let t = Instant::now();
            TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).ok().map(|_| t.elapsed().as_secs_f64() * 1000.0)
        })
        .collect();
    (!times.is_empty()).then(|| (times.iter().sum::<f64>() / times.len() as f64 * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;
    use crate::opti::services::Data;

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
