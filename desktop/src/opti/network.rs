//! Network page: DNS provider switch (optionally with DNS-over-HTTPS), cache flush, stack reset and DNS query timing.
//! Adapters come from GetAdaptersAddresses; changes go through netsh.

use super::backend::System;
use super::services::DnsProvider;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
};
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
        let mut r = GetAdaptersAddresses(
            AF_UNSPEC as u32,
            flags,
            std::ptr::null(),
            buf.as_mut_ptr().cast(),
            &mut size,
        );
        if r == ERROR_BUFFER_OVERFLOW {
            buf = vec![0; size as usize / 8 + 1];
            r = GetAdaptersAddresses(
                AF_UNSPEC as u32,
                flags,
                std::ptr::null(),
                buf.as_mut_ptr().cast(),
                &mut size,
            );
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
    ![
        "virtual",
        "vmware",
        "vethernet",
        "tap-",
        "tap ",
        "wintun",
        "wireguard",
        "loopback",
        "bluetooth",
        "vpn",
        "tailscale",
        "zerotier",
    ]
    .iter()
    .any(|w| text.contains(w))
}

/// netsh argument lists that point one adapter at a provider, or back to DHCP when the provider has no servers.
pub fn dns_commands(alias: &str, p: &DnsProvider) -> Vec<Vec<String>> {
    let name = format!("name={alias}");
    let mut out = Vec::new();
    for (proto, servers) in [("ipv4", &p.v4), ("ipv6", &p.v6)] {
        let base = |verb: &str| {
            vec![
                "interface".to_string(),
                proto.to_string(),
                verb.to_string(),
                "dnsservers".to_string(),
                name.clone(),
            ]
        };
        if servers.is_empty() {
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
    }
    out
}

fn doh_commands(p: &DnsProvider, capture: &DnsCapture) -> Result<Vec<Vec<String>>, String> {
    if p.doh.is_empty() {
        return Ok(Vec::new());
    }
    p.v4.iter()
        .chain(p.v6.iter())
        .map(|ip| {
            let action = if doh_for(capture, ip)?.is_some() {
                "set"
            } else {
                "add"
            };
            Ok(vec![
                "dns".into(),
                action.into(),
                "encryption".into(),
                format!("server={ip}"),
                format!("dohtemplate={}", p.doh),
                "autoupgrade=yes".into(),
                "udpfallback=yes".into(),
            ])
        })
        .collect()
}

fn netsh(sys: &dyn System, args: &[String]) -> super::backend::CmdOut {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    sys.run("netsh.exe", &refs)
}

pub fn show_dns(sys: &dyn System, log: &mut dyn FnMut(&str)) {
    let out = sys.run("netsh.exe", &["interface", "ipv4", "show", "dnsservers"]);
    for l in out.text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        log(&format!("  {l}"));
    }
}

const DNS_BACKUP_VERSION: u32 = 1;
const DNS_CAPTURE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$WarningPreference = 'SilentlyContinue'
$dohRequired = $REQUIRE_DOH
$excluded = 'virtual|vmware|vethernet|tap-|tap |wintun|wireguard|loopback|bluetooth|vpn|tailscale|zerotier'
$interfaces = @(Get-NetAdapter -IncludeHidden -ErrorAction Stop | Where-Object {
    $_.HardwareInterface -and "$($_.Name) $($_.InterfaceDescription)" -notmatch $excluded
} | ForEach-Object {
    $guid = ([guid]$_.InterfaceGuid).ToString('D')
    $path4 = "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{$guid}"
    $path6 = "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters\Interfaces\{$guid}"
    $raw4 = [string](Get-ItemProperty -LiteralPath $path4 -ErrorAction Stop).NameServer
    $raw6 = [string](Get-ItemProperty -LiteralPath $path6 -ErrorAction Stop).NameServer
    [pscustomobject]@{
        interface_guid = $guid
        name = $_.Name
        ipv4 = [pscustomobject]@{ dhcp = [string]::IsNullOrWhiteSpace($raw4); servers = @($raw4 -split '[,; ]+' | Where-Object { $_ }) }
        ipv6 = [pscustomobject]@{ dhcp = [string]::IsNullOrWhiteSpace($raw6); servers = @($raw6 -split '[,; ]+' | Where-Object { $_ }) }
    }
})
$dohCommand = Get-Command Get-DnsClientDohServerAddress -ErrorAction SilentlyContinue
$dohSupported = $null -ne $dohCommand
if ($dohRequired -and -not $dohSupported) { throw 'This Windows version cannot read DNS-over-HTTPS settings; no DNS changes were made.' }
$doh = @()
if ($dohSupported) {
    $doh = @(Get-DnsClientDohServerAddress -ErrorAction Stop | ForEach-Object {
        [pscustomobject]@{
            server_address = [string]$_.ServerAddress
            doh_template = [string]$_.DohTemplate
            auto_upgrade = [bool]$_.AutoUpgrade
            allow_fallback_to_udp = [bool]$_.AllowFallbackToUdp
        }
    })
}
[pscustomobject]@{ interfaces = $interfaces; doh_supported = $dohSupported; doh_servers = $doh } | ConvertTo-Json -Depth 7 -Compress
"#;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
struct DnsFamilySnapshot {
    dhcp: bool,
    #[serde(default)]
    servers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
struct DnsInterfaceSnapshot {
    interface_guid: String,
    name: String,
    ipv4: DnsFamilySnapshot,
    ipv6: DnsFamilySnapshot,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
struct DohServer {
    server_address: String,
    doh_template: String,
    auto_upgrade: bool,
    allow_fallback_to_udp: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct DnsCapture {
    interfaces: Vec<DnsInterfaceSnapshot>,
    doh_supported: bool,
    #[serde(default)]
    doh_servers: Vec<DohServer>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct DnsBackup {
    version: u32,
    interfaces: Vec<DnsInterfaceBackup>,
    #[serde(default)]
    doh_changes: Vec<DohChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct DnsInterfaceBackup {
    interface_guid: String,
    name: String,
    original_ipv4: DnsFamilySnapshot,
    original_ipv6: DnsFamilySnapshot,
    applied: Vec<AppliedDns>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
struct AppliedDns {
    ipv4: DnsFamilySnapshot,
    ipv6: DnsFamilySnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct DohChange {
    server_address: String,
    original: Option<DohServer>,
    applied: Vec<DohServer>,
}

pub fn dns_backup_available() -> bool {
    dns_backup_path().is_file()
}

fn dns_backup_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OrionStore")
        .join("optimaxer")
        .join("dns-backup.json")
}

fn capture_dns(sys: &dyn System, require_doh: bool) -> Result<DnsCapture, String> {
    let script =
        DNS_CAPTURE_SCRIPT.replace("$REQUIRE_DOH", if require_doh { "$true" } else { "$false" });
    let output = sys.run(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    );
    if output.code != 0 {
        return Err(format!(
            "Could not capture DNS settings: {}",
            output.text.trim()
        ));
    }
    let capture: DnsCapture = serde_json::from_str(output.text.trim())
        .map_err(|error| format!("Windows returned an unreadable DNS snapshot: {error}"))?;
    if require_doh && !capture.doh_supported {
        return Err("Could not read DNS-over-HTTPS settings; no DNS changes were made.".into());
    }
    Ok(capture)
}

fn validate_family(family: &DnsFamilySnapshot) -> Result<(), String> {
    if family.dhcp {
        return Ok(());
    }
    if family.servers.is_empty()
        || family
            .servers
            .iter()
            .any(|server| !valid_dns_server(server))
    {
        return Err(
            "DNS backup contains a static server address that cannot be safely restored.".into(),
        );
    }
    Ok(())
}

fn valid_dns_server(value: &str) -> bool {
    let (address, scope) = value.split_once('%').unwrap_or((value, ""));
    let Ok(address) = address.parse::<IpAddr>() else {
        return false;
    };
    scope.is_empty() || (address.is_ipv6() && scope.bytes().all(|byte| byte.is_ascii_digit()))
}

fn same_family(left: &DnsFamilySnapshot, right: &DnsFamilySnapshot) -> bool {
    if left.dhcp || right.dhcp {
        return left.dhcp == right.dhcp;
    }
    left.servers.len() == right.servers.len()
        && left
            .servers
            .iter()
            .zip(&right.servers)
            .all(|(a, b)| normalize_dns_server(a) == normalize_dns_server(b))
}

fn normalize_dns_server(value: &str) -> String {
    let (address, scope) = value.split_once('%').unwrap_or((value, ""));
    match address.parse::<IpAddr>() {
        Ok(ip) if scope.is_empty() => ip.to_string(),
        Ok(ip) => format!("{ip}%{scope}"),
        Err(_) => value.to_ascii_lowercase(),
    }
}

fn dns_targets(p: &DnsProvider) -> Result<AppliedDns, String> {
    let ipv4 = DnsFamilySnapshot {
        dhcp: p.v4.is_empty(),
        servers: p.v4.clone(),
    };
    let ipv6 = DnsFamilySnapshot {
        dhcp: p.v6.is_empty(),
        servers: p.v6.clone(),
    };
    validate_family(&ipv4)?;
    validate_family(&ipv6)?;
    Ok(AppliedDns { ipv4, ipv6 })
}

fn same_doh(left: &DohServer, right: &DohServer) -> bool {
    normalize_dns_server(&left.server_address) == normalize_dns_server(&right.server_address)
        && left.doh_template == right.doh_template
        && left.auto_upgrade == right.auto_upgrade
        && left.allow_fallback_to_udp == right.allow_fallback_to_udp
}

fn doh_for(capture: &DnsCapture, address: &str) -> Result<Option<DohServer>, String> {
    let matches = capture
        .doh_servers
        .iter()
        .filter(|record| {
            normalize_dns_server(&record.server_address) == normalize_dns_server(address)
        })
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(format!(
            "Windows returned duplicate DNS-over-HTTPS entries for {address}."
        ));
    }
    Ok(matches.first().map(|record| (*record).clone()))
}

fn prepare_backup(
    existing: Option<DnsBackup>,
    capture: DnsCapture,
    adapters: &[String],
    p: &DnsProvider,
) -> Result<DnsBackup, String> {
    let target = dns_targets(p)?;
    if !p.doh.is_empty()
        && (p.doh.chars().any(char::is_whitespace) || !p.doh.starts_with("https://"))
    {
        return Err("DNS-over-HTTPS template is invalid; no DNS changes were made.".into());
    }
    let mut backup = existing.unwrap_or(DnsBackup {
        version: DNS_BACKUP_VERSION,
        interfaces: Vec::new(),
        doh_changes: Vec::new(),
    });
    if backup.version != DNS_BACKUP_VERSION {
        return Err(
            "Saved DNS backup uses an unsupported format; no DNS changes were made.".into(),
        );
    }
    for name in adapters {
        let current = capture
            .interfaces
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                format!(
                    "Could not read original DNS settings for {name}; no DNS changes were made."
                )
            })?;
        validate_family(&current.ipv4)?;
        validate_family(&current.ipv6)?;
        let applied = AppliedDns {
            ipv4: target.ipv4.clone(),
            ipv6: target.ipv6.clone(),
        };
        if let Some(saved) = backup.interfaces.iter_mut().find(|entry| {
            entry
                .interface_guid
                .eq_ignore_ascii_case(&current.interface_guid)
        }) {
            let known_v4 = same_family(&current.ipv4, &saved.original_ipv4)
                || saved
                    .applied
                    .iter()
                    .any(|state| same_family(&current.ipv4, &state.ipv4));
            let known_v6 = same_family(&current.ipv6, &saved.original_ipv6)
                || saved
                    .applied
                    .iter()
                    .any(|state| same_family(&current.ipv6, &state.ipv6));
            if !known_v4 || !known_v6 {
                return Err(format!(
                    "DNS on {name} changed outside Optimaxer since its saved backup. Restore or review it before applying another provider."
                ));
            }
            if !saved.applied.contains(&applied) {
                saved.applied.push(applied);
            }
        } else {
            backup.interfaces.push(DnsInterfaceBackup {
                interface_guid: current.interface_guid.clone(),
                name: current.name.clone(),
                original_ipv4: current.ipv4.clone(),
                original_ipv6: current.ipv6.clone(),
                applied: vec![applied],
            });
        }
    }
    if !p.doh.is_empty() {
        if !capture.doh_supported {
            return Err(
                "Could not back up DNS-over-HTTPS settings; no DNS changes were made.".into(),
            );
        }
        for address in p.v4.iter().chain(&p.v6) {
            let expected = DohServer {
                server_address: address.clone(),
                doh_template: p.doh.clone(),
                auto_upgrade: true,
                allow_fallback_to_udp: true,
            };
            validate_doh(&expected)?;
            if let Some(change) = backup.doh_changes.iter_mut().find(|change| {
                normalize_dns_server(&change.server_address) == normalize_dns_server(address)
            }) {
                let current = doh_for(&capture, address)?;
                let known = match current.as_ref() {
                    Some(current) => {
                        change
                            .original
                            .as_ref()
                            .is_some_and(|original| same_doh(current, original))
                            || change
                                .applied
                                .iter()
                                .any(|applied| same_doh(current, applied))
                    }
                    None => change.original.is_none(),
                };
                if !known {
                    return Err(format!(
                        "DNS-over-HTTPS for {address} changed outside Optimaxer since its saved backup."
                    ));
                }
                if !change
                    .applied
                    .iter()
                    .any(|applied| same_doh(applied, &expected))
                {
                    change.applied.push(expected);
                }
            } else {
                let original = doh_for(&capture, address)?;
                backup.doh_changes.push(DohChange {
                    server_address: address.clone(),
                    original,
                    applied: vec![expected],
                });
            }
        }
    }
    Ok(backup)
}

fn load_backup(path: &Path) -> Result<Option<DnsBackup>, String> {
    match fs::read(path) {
        Ok(bytes) => {
            let backup: DnsBackup = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Saved DNS backup is invalid and was kept: {error}"))?;
            if backup.version != DNS_BACKUP_VERSION || backup.interfaces.is_empty() {
                return Err(
                    "Saved DNS backup is incomplete or uses an unsupported format; it was kept."
                        .into(),
                );
            }
            for interface in &backup.interfaces {
                validate_family(&interface.original_ipv4)?;
                validate_family(&interface.original_ipv6)?;
                if interface.interface_guid.is_empty()
                    || interface.name.is_empty()
                    || interface.applied.is_empty()
                {
                    return Err("Saved DNS backup is incomplete; it was kept.".into());
                }
                for applied in &interface.applied {
                    validate_family(&applied.ipv4)?;
                    validate_family(&applied.ipv6)?;
                }
            }
            for change in &backup.doh_changes {
                if change.applied.is_empty() || !valid_dns_server(&change.server_address) {
                    return Err("Saved DNS-over-HTTPS backup is incomplete; it was kept.".into());
                }
                if let Some(original) = &change.original {
                    validate_doh(original)?;
                }
                for applied in &change.applied {
                    validate_doh(applied)?;
                }
            }
            Ok(Some(backup))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not read saved DNS backup: {error}")),
    }
}

fn validate_doh(record: &DohServer) -> Result<(), String> {
    if !valid_dns_server(&record.server_address)
        || !record.doh_template.starts_with("https://")
        || record.doh_template.chars().any(char::is_whitespace)
    {
        return Err(
            "DNS-over-HTTPS settings contain values that cannot be safely restored.".into(),
        );
    }
    Ok(())
}

fn save_backup(path: &Path, backup: &DnsBackup) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(backup)
        .map_err(|error| format!("Could not encode DNS backup: {error}"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create DNS backup folder: {error}"))?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes).map_err(|error| format!("Could not write DNS backup: {error}"))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Could not save DNS backup before changing settings: {error}"
        ));
    }
    Ok(())
}

fn family_commands(alias: &str, protocol: &str, family: &DnsFamilySnapshot) -> Vec<Vec<String>> {
    let base = |verb: &str| {
        vec![
            "interface".into(),
            protocol.into(),
            verb.into(),
            "dnsservers".into(),
            format!("name={alias}"),
        ]
    };
    if family.dhcp {
        let mut command = base("set");
        command.push("source=dhcp".into());
        return vec![command];
    }
    family
        .servers
        .iter()
        .enumerate()
        .map(|(index, server)| {
            let mut command = base(if index == 0 { "set" } else { "add" });
            if index == 0 {
                command.push("source=static".into());
            }
            command.push(format!("address={server}"));
            if index > 0 {
                command.push(format!("index={}", index + 1));
            }
            command.push("validate=no".into());
            command
        })
        .collect()
}

fn family_is_known(
    current: &DnsFamilySnapshot,
    original: &DnsFamilySnapshot,
    applied: &[DnsFamilySnapshot],
) -> bool {
    same_family(current, original) || applied.iter().any(|target| same_family(current, target))
}

fn restore_doh_commands(
    change: &DohChange,
    current: Option<&DohServer>,
) -> Result<Vec<Vec<String>>, String> {
    if change
        .original
        .as_ref()
        .is_some_and(|original| current.is_some_and(|now| same_doh(now, original)))
    {
        return Ok(Vec::new());
    }
    if change.original.is_none() && current.is_none() {
        return Ok(Vec::new());
    }
    let current_is_applied =
        current.is_some_and(|now| change.applied.iter().any(|applied| same_doh(now, applied)));
    if !current_is_applied {
        return Err(format!(
            "DNS-over-HTTPS for {} no longer matches Optimaxer's saved change; it was left untouched.",
            change.server_address
        ));
    }
    if let Some(original) = &change.original {
        if validate_doh(original).is_err() {
            return Err(format!(
                "Saved DNS-over-HTTPS details for {} cannot be restored safely.",
                change.server_address
            ));
        }
        return Ok(vec![vec![
            "dns".into(),
            "set".into(),
            "encryption".into(),
            format!("server={}", original.server_address),
            format!("dohtemplate={}", original.doh_template),
            format!(
                "autoupgrade={}",
                if original.auto_upgrade { "yes" } else { "no" }
            ),
            format!(
                "udpfallback={}",
                if original.allow_fallback_to_udp {
                    "yes"
                } else {
                    "no"
                }
            ),
        ]]);
    }
    Ok(vec![vec![
        "dns".into(),
        "delete".into(),
        "encryption".into(),
        format!("server={}", change.server_address),
    ]])
}

pub fn apply_dns(
    sys: &dyn System,
    adapters: &[String],
    p: &DnsProvider,
    log: &mut dyn FnMut(&str),
) {
    let ok = apply_dns_with_path(sys, adapters, p, &dns_backup_path(), log);
    if !ok {
        log(
            "DNS provider change did not complete. Saved originals were kept for review or restore.",
        );
    }
}

fn apply_dns_with_path(
    sys: &dyn System,
    adapters: &[String],
    p: &DnsProvider,
    path: &Path,
    log: &mut dyn FnMut(&str),
) -> bool {
    if adapters.is_empty() {
        log("No active network adapter found; DNS settings were not changed.");
        return false;
    }
    let target = match dns_targets(p) {
        Ok(target) => target,
        Err(error) => {
            log(&format!("ERROR {error}"));
            return false;
        }
    };
    let existing = match load_backup(path) {
        Ok(backup) => backup,
        Err(error) => {
            log(&format!("ERROR {error}"));
            return false;
        }
    };
    let needs_doh = !p.doh.is_empty()
        || existing
            .as_ref()
            .is_some_and(|backup| !backup.doh_changes.is_empty());
    let capture = match capture_dns(sys, needs_doh) {
        Ok(capture) => capture,
        Err(error) => {
            log(&format!("ERROR {error}"));
            return false;
        }
    };
    let doh_commands = match doh_commands(p, &capture) {
        Ok(commands) => commands,
        Err(error) => {
            log(&format!("ERROR {error}"));
            return false;
        }
    };
    let backup = match prepare_backup(existing, capture, adapters, p) {
        Ok(backup) => backup,
        Err(error) => {
            log(&format!("ERROR {error}"));
            return false;
        }
    };
    if let Err(error) = save_backup(path, &backup) {
        log(&format!("ERROR {error}"));
        return false;
    }
    log(&format!(
        "Original DNS configuration saved for {} adapter(s).",
        adapters.len()
    ));
    for adapter in adapters {
        for command in dns_commands(adapter, p) {
            let output = netsh(sys, &command);
            if output.code != 0 {
                log(&format!("ERROR DNS on {adapter}: {}", output.text.trim()));
                return false;
            }
        }
        log(&format!(
            "  DNS on {adapter}: {}",
            if p.v4.is_empty() {
                "automatic"
            } else {
                &p.name
            }
        ));
    }
    for command in doh_commands {
        let output = netsh(sys, &command);
        if output.code != 0 {
            log(&format!(
                "ERROR DNS-over-HTTPS registration failed: {}",
                output.text.trim()
            ));
            return false;
        }
    }
    let flush = sys.run("ipconfig.exe", &["/flushdns"]);
    if flush.code != 0 {
        log(&format!(
            "WARN DNS cache flush failed: {}",
            flush.text.trim()
        ));
    }
    match capture_dns(sys, needs_doh) {
        Ok(after) => {
            let verified = adapters.iter().all(|name| {
                after
                    .interfaces
                    .iter()
                    .find(|item| item.name.eq_ignore_ascii_case(name))
                    .is_some_and(|current| {
                        same_family(&current.ipv4, &target.ipv4)
                            && same_family(&current.ipv6, &target.ipv6)
                    })
            });
            if !verified {
                log(
                    "ERROR Windows did not report the requested DNS servers after the change. Backup retained.",
                );
                return false;
            }
            if !p.doh.is_empty() {
                for address in p.v4.iter().chain(&p.v6) {
                    let expected = DohServer {
                        server_address: address.clone(),
                        doh_template: p.doh.clone(),
                        auto_upgrade: true,
                        allow_fallback_to_udp: true,
                    };
                    if !doh_for(&after, address).is_ok_and(|current| {
                        current.is_some_and(|current| same_doh(&current, &expected))
                    }) {
                        log(&format!(
                            "ERROR Windows did not report the requested DNS-over-HTTPS settings for {address}. Backup retained."
                        ));
                        return false;
                    }
                }
            }
        }
        Err(error) => {
            log(&format!(
                "ERROR Could not verify DNS settings after change: {error}"
            ));
            return false;
        }
    }
    log(
        "DNS provider applied and verified. Use Restore original DNS to return to the saved settings.",
    );
    true
}

pub fn restore_dns(sys: &dyn System, log: &mut dyn FnMut(&str)) {
    let restored = restore_dns_with_path(sys, &dns_backup_path(), log);
    if !restored {
        log("DNS restore incomplete. Saved originals were kept for retry.");
    }
}

fn restore_dns_with_path(sys: &dyn System, path: &Path, log: &mut dyn FnMut(&str)) -> bool {
    let backup = match load_backup(path) {
        Ok(Some(backup)) => backup,
        Ok(None) => {
            log("No saved DNS configuration found.");
            return false;
        }
        Err(error) => {
            log(&format!("ERROR {error}"));
            return false;
        }
    };
    let capture = match capture_dns(sys, !backup.doh_changes.is_empty()) {
        Ok(capture) => capture,
        Err(error) => {
            log(&format!("ERROR {error}"));
            return false;
        }
    };
    let mut plan = Vec::<Vec<String>>::new();
    for saved in &backup.interfaces {
        let Some(current) = capture.interfaces.iter().find(|item| {
            item.interface_guid
                .eq_ignore_ascii_case(&saved.interface_guid)
        }) else {
            log(&format!(
                "WARN Adapter {} is unavailable; its DNS backup was kept.",
                saved.name
            ));
            return false;
        };
        let v4_targets = saved
            .applied
            .iter()
            .map(|state| state.ipv4.clone())
            .collect::<Vec<_>>();
        let v6_targets = saved
            .applied
            .iter()
            .map(|state| state.ipv6.clone())
            .collect::<Vec<_>>();
        if !family_is_known(&current.ipv4, &saved.original_ipv4, &v4_targets)
            || !family_is_known(&current.ipv6, &saved.original_ipv6, &v6_targets)
        {
            log(&format!(
                "WARN DNS on {} changed outside Optimaxer; left untouched.",
                current.name
            ));
            return false;
        }
        for (protocol, current_family, original) in [
            ("ipv4", &current.ipv4, &saved.original_ipv4),
            ("ipv6", &current.ipv6, &saved.original_ipv6),
        ] {
            if same_family(current_family, original) {
                continue;
            }
            plan.extend(family_commands(&current.name, protocol, original));
        }
    }
    for change in &backup.doh_changes {
        let current = match doh_for(&capture, &change.server_address) {
            Ok(current) => current,
            Err(error) => {
                log(&format!("WARN {error}"));
                return false;
            }
        };
        match restore_doh_commands(change, current.as_ref()) {
            Ok(commands) => plan.extend(commands),
            Err(error) => {
                log(&format!("WARN {error}"));
                return false;
            }
        }
    }
    for command in plan {
        let output = netsh(sys, &command);
        if output.code != 0 {
            log(&format!(
                "ERROR DNS restore command failed: {} {}",
                command.join(" "),
                output.text.trim()
            ));
            return false;
        }
    }
    let flush = sys.run("ipconfig.exe", &["/flushdns"]);
    if flush.code != 0 {
        log(&format!(
            "WARN DNS cache flush failed after restore: {}",
            flush.text.trim()
        ));
    }
    let verified = match capture_dns(sys, !backup.doh_changes.is_empty()) {
        Ok(after) => {
            backup.interfaces.iter().all(|saved| {
                after
                    .interfaces
                    .iter()
                    .find(|item| {
                        item.interface_guid
                            .eq_ignore_ascii_case(&saved.interface_guid)
                    })
                    .is_some_and(|current| {
                        same_family(&current.ipv4, &saved.original_ipv4)
                            && same_family(&current.ipv6, &saved.original_ipv6)
                    })
            }) && backup.doh_changes.iter().all(|change| {
                doh_for(&after, &change.server_address).is_ok_and(|current| {
                    match (&change.original, current) {
                        (None, None) => true,
                        (Some(original), Some(current)) => same_doh(original, &current),
                        _ => false,
                    }
                })
            })
        }
        Err(error) => {
            log(&format!(
                "ERROR Could not verify restored DNS settings: {error}"
            ));
            false
        }
    };
    if !verified {
        log("ERROR Windows DNS settings do not match the saved originals. Backup retained.");
        return false;
    }
    match fs::remove_file(path) {
        Ok(()) => {
            log("Original DNS settings restored and verified; saved backup removed.");
            true
        }
        Err(error) => {
            log(&format!(
                "WARN DNS was restored, but its backup could not be removed: {error}"
            ));
            false
        }
    }
}

pub fn reset_stack(sys: &dyn System, log: &mut dyn FnMut(&str)) {
    let mut succeeded = true;
    for (exe, args) in [
        ("netsh.exe", &["winsock", "reset"][..]),
        ("netsh.exe", &["int", "ip", "reset"][..]),
        ("ipconfig.exe", &["/flushdns"][..]),
    ] {
        let out = sys.run(exe, args);
        if out.code != 0 {
            succeeded = false;
            log(&format!(
                "  WARN {exe} {} failed: {}",
                args.join(" "),
                out.text.trim()
            ));
        }
    }
    log(if succeeded {
        "Network stack reset. Restart Windows to finish."
    } else {
        "Network reset incomplete. Review the failed steps and restart Windows after resolving them."
    });
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
        let changes: Vec<f64> = times
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .collect();
        average(&changes)
    } else {
        None
    };
    DnsBenchmark {
        attempts,
        answers: times.len() as u32,
        average_ms,
        jitter_ms,
    }
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty())
        .then(|| (values.iter().sum::<f64>() / values.len() as f64 * 10.0).round() / 10.0)
}

fn dns_query_ms(server: SocketAddr, timeout: Duration) -> Option<f64> {
    let bind_addr = if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
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
        let data_length =
            u16::from_be_bytes([response[end_name + 8], response[end_name + 9]]) as usize;
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

    static NEXT_BACKUP_TEST_ID: AtomicU16 = AtomicU16::new(1);

    fn provider(name: &str) -> DnsProvider {
        Data::load()
            .dns
            .into_iter()
            .find(|p| p.name == name)
            .unwrap()
    }

    fn family(dhcp: bool, servers: &[&str]) -> DnsFamilySnapshot {
        DnsFamilySnapshot {
            dhcp,
            servers: servers.iter().map(|server| (*server).into()).collect(),
        }
    }

    fn doh_for_provider(provider: &DnsProvider) -> Vec<DohServer> {
        provider
            .v4
            .iter()
            .chain(&provider.v6)
            .map(|address| DohServer {
                server_address: address.clone(),
                doh_template: provider.doh.clone(),
                auto_upgrade: true,
                allow_fallback_to_udp: true,
            })
            .collect()
    }

    fn capture_json(
        ipv4: DnsFamilySnapshot,
        ipv6: DnsFamilySnapshot,
        doh_supported: bool,
        doh: &[DohServer],
    ) -> String {
        serde_json::json!({
            "interfaces": [{ "interface_guid": "{11111111-1111-1111-1111-111111111111}", "name": "Ethernet", "ipv4": ipv4, "ipv6": ipv6 }],
            "doh_supported": doh_supported,
            "doh_servers": doh,
        })
        .to_string()
    }

    fn backup_path(label: &str) -> PathBuf {
        let id = NEXT_BACKUP_TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "optimaxer-dns-{label}-{}-{id}.json",
            std::process::id()
        ))
    }

    #[test]
    fn static_dns_commands_set_then_add() {
        let cmds = dns_commands("Ethernet 2", &provider("Cloudflare"));
        let v4: Vec<String> = cmds
            .iter()
            .filter(|c| c[1] == "ipv4")
            .map(|c| c.join(" "))
            .collect();
        assert_eq!(
            v4[0],
            "interface ipv4 set dnsservers name=Ethernet 2 source=static address=1.1.1.1 validate=no"
        );
        assert_eq!(
            v4[1],
            "interface ipv4 add dnsservers name=Ethernet 2 address=1.0.0.1 index=2 validate=no"
        );
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
        assert!(
            cmds.iter()
                .any(|c| c[1] == "ipv6" && c.contains(&"source=dhcp".to_string()))
        );
    }

    #[test]
    fn dns_commands_handle_each_address_family_independently() {
        let p = DnsProvider {
            name: "v6 only".into(),
            desc: String::new(),
            v4: vec![],
            v6: vec!["2001:db8::1".into()],
            doh: String::new(),
        };
        let commands = dns_commands("Wi-Fi", &p);
        assert!(
            commands
                .iter()
                .any(|command| command[1] == "ipv4" && command.contains(&"source=dhcp".into()))
        );
        assert!(commands.iter().any(|command| {
            command[1] == "ipv6" && command.contains(&"address=2001:db8::1".into())
        }));
    }

    #[test]
    fn dns_backup_validates_literal_server_addresses() {
        assert!(valid_dns_server("1.1.1.1"));
        assert!(valid_dns_server("fe80::1%12"));
        assert!(!valid_dns_server("1.1.1.1 address=evil"));
        assert!(!valid_dns_server("fe80::1%Ethernet"));
        assert!(validate_family(&family(false, &["1.1.1.1"])).is_ok());
        assert!(validate_family(&family(false, &[])).is_err());
    }

    #[test]
    fn backup_keeps_originals_across_provider_changes_and_rejects_external_edits() {
        let first = provider("Cloudflare");
        let capture = DnsCapture {
            interfaces: vec![DnsInterfaceSnapshot {
                interface_guid: "{11111111-1111-1111-1111-111111111111}".into(),
                name: "Ethernet".into(),
                ipv4: family(true, &[]),
                ipv6: family(true, &[]),
            }],
            doh_supported: true,
            doh_servers: vec![],
        };
        let backup = prepare_backup(None, capture, &["Ethernet".into()], &first).unwrap();
        assert!(backup.interfaces[0].original_ipv4.dhcp);
        assert_eq!(backup.interfaces[0].applied.len(), 1);
        assert_eq!(backup.doh_changes.len(), first.v4.len() + first.v6.len());

        let first_target = dns_targets(&first).unwrap();
        let next = DnsProvider {
            name: "second provider".into(),
            desc: String::new(),
            v4: vec!["9.9.9.9".into()],
            v6: vec![],
            doh: String::new(),
        };
        let mut current = DnsCapture {
            interfaces: vec![DnsInterfaceSnapshot {
                interface_guid: backup.interfaces[0].interface_guid.clone(),
                name: "Ethernet".into(),
                ipv4: first_target.ipv4.clone(),
                ipv6: first_target.ipv6.clone(),
            }],
            doh_supported: true,
            doh_servers: doh_for_provider(&first),
        };
        let updated =
            prepare_backup(Some(backup), current.clone(), &["Ethernet".into()], &next).unwrap();
        assert!(updated.interfaces[0].original_ipv4.dhcp);
        assert_eq!(updated.interfaces[0].applied.len(), 2);

        current.interfaces[0].ipv4 = family(false, &["4.4.4.4"]);
        assert!(prepare_backup(Some(updated), current, &["Ethernet".into()], &next).is_err());
    }

    #[test]
    fn doh_restore_replaces_only_the_expected_record_and_is_retryable() {
        let original = DohServer {
            server_address: "1.1.1.1".into(),
            doh_template: "https://old.example/dns-query".into(),
            auto_upgrade: false,
            allow_fallback_to_udp: false,
        };
        let applied = DohServer {
            server_address: "1.1.1.1".into(),
            doh_template: "https://new.example/dns-query".into(),
            auto_upgrade: true,
            allow_fallback_to_udp: true,
        };
        let change = DohChange {
            server_address: "1.1.1.1".into(),
            original: Some(original),
            applied: vec![applied.clone()],
        };
        let commands = restore_doh_commands(&change, Some(&applied)).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0][..4],
            ["dns", "set", "encryption", "server=1.1.1.1"]
        );
        assert!(
            commands[0]
                .iter()
                .any(|argument| argument == "dohtemplate=https://old.example/dns-query")
        );
        assert!(restore_doh_commands(&change, None).is_err());
        assert!(
            restore_doh_commands(
                &change,
                Some(&DohServer {
                    doh_template: "https://manual.example/dns-query".into(),
                    ..applied
                })
            )
            .is_err()
        );
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
        let capture = DnsCapture {
            interfaces: vec![],
            doh_supported: true,
            doh_servers: vec![],
        };
        assert!(
            doh_commands(&provider("OpenDNS"), &capture)
                .unwrap()
                .is_empty()
        );
        let c = doh_commands(&provider("Cloudflare"), &capture).unwrap();
        assert!(
            c.len() >= 2
                && c[0][1] == "add"
                && c[0]
                    .join(" ")
                    .contains("dohtemplate=https://cloudflare-dns.com/dns-query")
        );
    }

    #[test]
    fn doh_update_uses_set_for_existing_resolver_and_add_for_new_one() {
        let provider = provider("Cloudflare");
        let capture = DnsCapture {
            interfaces: vec![],
            doh_supported: true,
            doh_servers: vec![DohServer {
                server_address: provider.v4[0].clone(),
                doh_template: "https://old.example/dns-query".into(),
                auto_upgrade: false,
                allow_fallback_to_udp: false,
            }],
        };
        let commands = doh_commands(&provider, &capture).unwrap();
        assert_eq!(commands[0][1], "set");
        assert_eq!(commands[1][1], "add");
    }

    #[test]
    fn removed_existing_doh_record_blocks_a_repeat_provider_change() {
        let provider = provider("Cloudflare");
        let original = DohServer {
            server_address: provider.v4[0].clone(),
            doh_template: "https://old.example/dns-query".into(),
            auto_upgrade: false,
            allow_fallback_to_udp: false,
        };
        let initial = DnsCapture {
            interfaces: vec![DnsInterfaceSnapshot {
                interface_guid: "11111111-1111-1111-1111-111111111111".into(),
                name: "Ethernet".into(),
                ipv4: family(true, &[]),
                ipv6: family(true, &[]),
            }],
            doh_supported: true,
            doh_servers: vec![original],
        };
        let backup = prepare_backup(None, initial, &["Ethernet".into()], &provider).unwrap();
        let target = dns_targets(&provider).unwrap();
        let current = DnsCapture {
            interfaces: vec![DnsInterfaceSnapshot {
                interface_guid: backup.interfaces[0].interface_guid.clone(),
                name: "Ethernet".into(),
                ipv4: target.ipv4,
                ipv6: target.ipv6,
            }],
            doh_supported: true,
            doh_servers: doh_for_provider(&provider)
                .into_iter()
                .filter(|entry| entry.server_address != provider.v4[0])
                .collect(),
        };
        assert!(prepare_backup(Some(backup), current, &["Ethernet".into()], &provider).is_err());
    }

    #[test]
    fn virtual_adapters_are_skipped() {
        assert!(is_physical_candidate(1, 6, "Ethernet", "Intel(R) I225-V"));
        assert!(!is_physical_candidate(
            1,
            6,
            "vEthernet (WSL)",
            "Hyper-V Virtual Ethernet Adapter"
        ));
        assert!(!is_physical_candidate(2, 6, "Ethernet", "Intel"));
        assert!(!is_physical_candidate(
            1,
            24,
            "Loopback",
            "Software Loopback"
        ));
        assert!(!is_physical_candidate(
            1,
            6,
            "Tailscale",
            "Tailscale Tunnel"
        ));
    }

    #[test]
    fn applying_dns_runs_netsh_and_flushes() {
        let sys = FakeSystem::default();
        let p = provider("Cloudflare");
        let target = dns_targets(&p).unwrap();
        let before = capture_json(family(true, &[]), family(true, &[]), true, &[]);
        let after = capture_json(target.ipv4, target.ipv6, true, &doh_for_provider(&p));
        sys.answer_sequence("powershell.exe", &[before, after]);
        let path = backup_path("apply");
        let mut lines = Vec::new();
        assert!(apply_dns_with_path(
            &sys,
            &["Ethernet".to_string()],
            &p,
            &path,
            &mut |l| lines.push(l.to_string()),
        ));
        assert!(sys.ran(
            "netsh.exe interface ipv4 set dnsservers name=Ethernet source=static address=1.1.1.1"
        ));
        assert!(sys.ran("ipconfig.exe /flushdns"));
        assert!(lines.iter().any(|l| l.contains("Cloudflare")));
        assert!(
            path.is_file(),
            "original DNS backup must remain available after apply"
        );
        let saved = load_backup(&path).unwrap().unwrap();
        assert!(saved.interfaces[0].original_ipv4.dhcp);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn applying_doh_updates_existing_windows_resolver_entry() {
        let sys = FakeSystem::default();
        let p = provider("Cloudflare");
        let target = dns_targets(&p).unwrap();
        let original = DohServer {
            server_address: p.v4[0].clone(),
            doh_template: "https://old.example/dns-query".into(),
            auto_upgrade: false,
            allow_fallback_to_udp: false,
        };
        let before = capture_json(
            family(true, &[]),
            family(true, &[]),
            true,
            std::slice::from_ref(&original),
        );
        let after = capture_json(target.ipv4, target.ipv6, true, &doh_for_provider(&p));
        sys.answer_sequence("powershell.exe", &[before, after]);
        let path = backup_path("doh-update-existing");
        assert!(apply_dns_with_path(
            &sys,
            &["Ethernet".to_string()],
            &p,
            &path,
            &mut |_| {},
        ));
        assert!(sys.ran("netsh.exe dns set encryption server=1.1.1.1"));
        assert!(path.is_file());
        let _ = fs::remove_file(path);
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
        sys.fail.borrow_mut().clear();
        sys.fail
            .borrow_mut()
            .insert("netsh.exe dns add encryption".into());
        let p = provider("Cloudflare");
        let before = capture_json(family(true, &[]), family(true, &[]), true, &[]);
        sys.answer("powershell.exe", &before);
        let path = backup_path("doh-failure");
        assert!(!apply_dns_with_path(
            &sys,
            &["Ethernet".into()],
            &p,
            &path,
            &mut |l| lines.push(l.to_string()),
        ));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("DNS-over-HTTPS registration failed"))
        );
        assert!(path.is_file(), "failed apply must retain originals");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn restore_returns_dns_to_original_snapshot_and_removes_backup_only_after_verification() {
        let sys = FakeSystem::default();
        let path = backup_path("restore");
        let p = provider("Cloudflare");
        let target = dns_targets(&p).unwrap();
        let before = DnsCapture {
            interfaces: vec![DnsInterfaceSnapshot {
                interface_guid: "{11111111-1111-1111-1111-111111111111}".into(),
                name: "Ethernet".into(),
                ipv4: family(true, &[]),
                ipv6: family(true, &[]),
            }],
            doh_supported: true,
            doh_servers: vec![],
        };
        let backup = prepare_backup(None, before, &["Ethernet".into()], &p).unwrap();
        save_backup(&path, &backup).unwrap();
        let current = capture_json(target.ipv4, target.ipv6, true, &doh_for_provider(&p));
        let restored = capture_json(family(true, &[]), family(true, &[]), true, &[]);
        sys.answer_sequence("powershell.exe", &[current, restored]);
        let mut lines = Vec::new();
        assert!(restore_dns_with_path(&sys, &path, &mut |line| lines.push(line.to_string())));
        assert!(sys.ran("netsh.exe interface ipv4 set dnsservers name=Ethernet source=dhcp"));
        assert!(sys.ran("netsh.exe dns delete encryption server=1.1.1.1"));
        assert!(
            !path.exists(),
            "verified restoration removes the consumed backup"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("restored and verified"))
        );
    }

    #[test]
    fn invalid_snapshot_stops_before_network_mutation() {
        let sys = FakeSystem::default();
        sys.answer("powershell.exe", "not-json");
        let path = backup_path("invalid-capture");
        assert!(!apply_dns_with_path(
            &sys,
            &["Ethernet".into()],
            &provider("OpenDNS"),
            &path,
            &mut |_| {},
        ));
        assert!(!sys.ran("netsh.exe"));
        assert!(!path.exists());
    }

    #[test]
    #[ignore = "Read-only integration test for a real Windows DNS configuration"]
    fn windows_dns_snapshot_reads_live_settings() {
        let capture = capture_dns(&crate::opti::backend::WinSystem, false).unwrap();
        assert!(!capture.interfaces.is_empty());
        assert!(capture.interfaces.iter().all(|adapter| {
            !adapter.interface_guid.is_empty()
                && !adapter.name.is_empty()
                && validate_family(&adapter.ipv4).is_ok()
                && validate_family(&adapter.ipv6).is_ok()
        }));
    }
}
