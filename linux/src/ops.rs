//! Services, startup entries, DNS, tools, launcher, settings and self-update.

use crate::catalog::App;
use crate::data::SERVICES;
use crate::env::{Env, REPO, VERSION, fmt_size, s, tail};
use crate::sys::{restore_unit, unit_active, unit_enabled, unit_exists};
use crate::tweaks::value_str;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

// ------------------------------------------------------------------------------------------------ services

#[derive(Clone, Debug)]
pub struct ServiceRow {
    pub unit: String,
    pub label: String,
    pub hint: String,
    pub enabled: bool,
    pub active: bool,
}

pub fn service_rows(env: &Env) -> Vec<ServiceRow> {
    SERVICES
        .iter()
        .filter(|(unit, _, _)| unit_exists(env, unit))
        .map(|(unit, label, hint)| ServiceRow {
            unit: unit.to_string(), label: label.to_string(), hint: hint.to_string(), enabled: unit_enabled(env, unit), active: unit_active(env, unit),
        })
        .collect()
}

pub fn set_service(env: &Env, unit: &str, enable: bool) -> bool {
    let mut state = match env.load_state() {
        Ok(st) => st,
        Err(e) => {
            env.warn(&e);
            return false;
        }
    };
    let key = format!("service:{unit}");
    if !state.contains_key(&key) {
        state.insert(key, json!({"unit": unit, "enabled": unit_enabled(env, unit), "active": unit_active(env, unit)}));
        if let Err(e) = env.save_state(&state) {
            env.warn(&format!("cannot save the undo state ({e}); nothing was changed"));
            return false;
        }
    }
    let changed = env.change_s(&["systemctl", if enable { "enable" } else { "disable" }, "--now", unit]);
    let ok = changed && (env.dry_run || (unit_enabled(env, unit) == enable && unit_active(env, unit) == enable));
    env.log(&format!("{} {unit}{}", if enable { "Enabled" } else { "Disabled" }, if ok { "" } else { " (state did not change)" }), if ok { "OK" } else { "WARN" });
    ok
}

pub fn restore_service(env: &Env, unit: &str) -> bool {
    let Ok(mut state) = env.load_state() else {
        env.warn("Undo state is unreadable; no changes made");
        return false;
    };
    let Some(snap) = state.get(&format!("service:{unit}")).cloned() else {
        env.warn(&format!("No saved state for {unit}"));
        return false;
    };
    let ok = restore_unit(env, unit, snap["enabled"].as_bool().unwrap_or(false), snap["active"].as_bool().unwrap_or(false));
    if ok {
        state.remove(&format!("service:{unit}"));
        let _ = env.save_state(&state);
    }
    env.log(&if ok { format!("Restored {unit}") } else { format!("Restore failed for {unit}; saved state kept for retry") }, if ok { "OK" } else { "WARN" });
    ok
}

// ------------------------------------------------------------------------------------------------ startup (XDG autostart)

const MARK: &str = "# optimaxer-override";

#[derive(Clone, Debug)]
pub struct StartupItem {
    pub file: String,
    pub name: String,
    pub exec: String,
    pub scope: &'static str,
    pub enabled: bool,
}

pub fn parse_desktop(text: &str) -> Map<String, Value> {
    let mut d = Map::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=')
            && !line.trim().starts_with('#') {
                d.entry(k.trim().to_string()).or_insert(Value::String(v.trim().to_string()));
            }
    }
    d
}

fn user_autostart(env: &Env) -> String {
    format!("{}/.config/autostart", env.real_home().trim_end_matches('/'))
}

pub fn autostart_items(env: &Env) -> Vec<StartupItem> {
    let mut items = vec![];
    let mut seen: Vec<String> = vec![];
    let user_dir = user_autostart(env);
    for (scope, dir) in [("user", user_dir.clone()), ("system", "/etc/xdg/autostart".to_string())] {
        let Ok(rd) = fs::read_dir(env.p(&dir)) else { continue };
        let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for f in names {
            if !f.ends_with(".desktop") || seen.contains(&f) {
                continue;
            }
            seen.push(f.clone());
            let mut text = env.read_text(&format!("{dir}/{f}")).unwrap_or_default();
            if scope == "system"
                && let Some(o) = env.read_text(&format!("{user_dir}/{f}")) {
                    text = o;
                }
            let kv = parse_desktop(&text);
            let get = |k: &str, d: &str| kv.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_lowercase();
            let hidden = get("Hidden", "false") == "true" || get("X-GNOME-Autostart-enabled", "true") == "false";
            items.push(StartupItem {
                name: kv.get("Name").and_then(|v| v.as_str()).unwrap_or(&f).to_string(),
                exec: kv.get("Exec").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                file: f, scope, enabled: !hidden,
            });
        }
    }
    items
}

fn key_is(line: &str, key: &str) -> bool {
    line.trim_start().strip_prefix(key).is_some_and(|r| r.trim_start().starts_with('='))
}

pub fn set_autostart(env: &Env, item: &StartupItem, enable: bool) -> bool {
    let user_dir = user_autostart(env);
    let path = format!("{user_dir}/{}", item.file);
    let sys_path = format!("/etc/xdg/autostart/{}", item.file);
    let current = match env.read_text(&path) {
        Some(c) => c,
        None => match env.read_text(&sys_path) {
            Some(base) => format!("{}\n{MARK}\n", base.trim_end_matches('\n')),
            None => return false,
        },
    };
    let mut lines: Vec<String> = current.split('\n').filter(|l| !key_is(l, "Hidden")).map(String::from).collect();
    if enable {
        for l in lines.iter_mut() {
            if key_is(l, "X-GNOME-Autostart-enabled") {
                *l = "X-GNOME-Autostart-enabled=true".into();
            }
        }
    } else {
        let idx = lines.iter().position(|l| l.trim() == "[Desktop Entry]").map(|i| i + 1).unwrap_or(0);
        lines.insert(idx, "Hidden=true".into());
    }
    if let Err(e) = env.write_text(&path, &lines.join("\n")) {
        env.warn(&format!("Could not write {path}: {e}"));
        return false;
    }
    if !env.dry_run {
        env.chown_user(&env.p(&path));
    }
    env.ok(&format!("{} autostart: {}", if enable { "Enabled" } else { "Disabled" }, item.name));
    true
}

// ------------------------------------------------------------------------------------------------ network / DNS

pub struct DnsProvider {
    pub key: &'static str,
    pub label: &'static str,
    pub v4: &'static [&'static str],
    pub v6: &'static [&'static str],
}

pub const DNS: &[DnsProvider] = &[
    DnsProvider { key: "cloudflare", label: "Cloudflare", v4: &["1.1.1.1", "1.0.0.1"], v6: &["2606:4700:4700::1111", "2606:4700:4700::1001"] },
    DnsProvider { key: "google", label: "Google", v4: &["8.8.8.8", "8.8.4.4"], v6: &["2001:4860:4860::8888", "2001:4860:4860::8844"] },
    DnsProvider { key: "quad9", label: "Quad9", v4: &["9.9.9.9", "149.112.112.112"], v6: &["2620:fe::fe", "2620:fe::9"] },
    DnsProvider { key: "adguard", label: "AdGuard", v4: &["94.140.14.14", "94.140.15.15"], v6: &["2a10:50c0::ad1:ff", "2a10:50c0::ad2:ff"] },
    DnsProvider { key: "opendns", label: "OpenDNS", v4: &["208.67.222.222", "208.67.220.220"], v6: &[] },
    DnsProvider { key: "auto", label: "Automatic (DHCP)", v4: &[], v6: &[] },
];

pub fn dns_provider(key: &str) -> Option<&'static DnsProvider> {
    DNS.iter().find(|d| d.key == key)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub name: String,
    pub kind: String,
    pub device: String,
}

/// Split an `nmcli -t` line; nmcli escapes colons and backslashes in connection names.
pub fn split_nmcli(line: &str) -> Vec<String> {
    let (mut parts, mut cur, mut esc) = (vec![], String::new(), false);
    for c in line.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ':' {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    parts.push(cur);
    parts
}

pub fn active_connections(env: &Env) -> Vec<Connection> {
    let (rc, out) = env.run(&["nmcli", "-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"]);
    if rc != 0 {
        return vec![];
    }
    out.lines()
        .map(split_nmcli)
        .filter(|p| p.len() >= 3 && ["802-3-ethernet", "802-11-wireless", "ethernet", "wifi"].contains(&p[1].as_str()))
        .map(|p| Connection { name: p[0].clone(), kind: p[1].clone(), device: p[2].clone() })
        .collect()
}

pub fn set_dns(env: &Env, key: &str) -> bool {
    let Some(prov) = dns_provider(key) else {
        env.log(&format!("Unknown DNS provider {key}"), "ERROR");
        return false;
    };
    if !env.have("nmcli") {
        env.warn("NetworkManager (nmcli) not found: set DNS in your network manager instead");
        return false;
    }
    let mut state = match env.load_state() {
        Ok(st) => st,
        Err(e) => {
            env.warn(&e);
            return false;
        }
    };
    let connections = active_connections(env);
    if connections.is_empty() {
        env.warn("No active physical NetworkManager connections found");
        return false;
    }
    let mut ok_all = true;
    for c in &connections {
        let sk = format!("dns:{}", c.name);
        if !state.contains_key(&sk) {
            let mut rec = Map::new();
            rec.insert("name".into(), Value::String(c.name.clone()));
            for prop in ["ipv4.dns", "ipv4.ignore-auto-dns", "ipv6.dns", "ipv6.ignore-auto-dns"] {
                let (rc, out) = env.run(&["nmcli", "-g", prop, "connection", "show", &c.name]);
                if rc != 0 {
                    env.warn(&format!("Cannot read {prop} for {}: {}; nothing was changed", c.name, out.trim()));
                    return false;
                }
                rec.insert(prop.into(), Value::String(out.trim().to_string()));
            }
            state.insert(sk, Value::Object(rec));
            if let Err(e) = env.save_state(&state) {
                env.warn(&format!("cannot save the undo state ({e}); nothing was changed"));
                return false;
            }
        }
        let yn = |b: bool| if b { "yes" } else { "no" };
        let (v4, v6) = (prov.v4.join(","), prov.v6.join(","));
        let (rc, out) = env.run_w(&["nmcli", "connection", "modify", &c.name, "ipv4.dns", &v4, "ipv4.ignore-auto-dns", yn(!prov.v4.is_empty()), "ipv6.dns", &v6, "ipv6.ignore-auto-dns", yn(!prov.v6.is_empty())]);
        if rc == 0 {
            if env.change_s(&["nmcli", "device", "reapply", &c.device]) {
                env.ok(&format!("DNS on {}: {}", c.name, prov.label));
            } else {
                ok_all = false;
            }
        } else {
            ok_all = false;
            env.warn(&format!("DNS on {}: {}", c.name, tail(&out, 100)));
        }
    }
    if env.have("resolvectl") {
        env.run_w(&["resolvectl", "flush-caches"]);
    }
    ok_all
}

pub fn restore_dns(env: &Env) -> bool {
    let mut state = match env.load_state() {
        Ok(st) => st,
        Err(e) => {
            env.warn(&e);
            return false;
        }
    };
    let names: Vec<String> = state.keys().filter(|k| k.starts_with("dns:")).cloned().collect();
    if names.is_empty() {
        env.warn("No saved DNS settings");
        return false;
    }
    let connections = active_connections(env);
    let mut ok = true;
    for k in names {
        let rec = state[&k].clone();
        let name = value_str(&rec, "name");
        let comma = |p: &str| value_str(&rec, p).replace(' ', ",");
        let auto = |p: &str| {
            let v = value_str(&rec, p);
            if v.is_empty() { "no".to_string() } else { v }
        };
        let mut restored = env.change(&s(&["nmcli", "connection", "modify", &name, "ipv4.dns", &comma("ipv4.dns"), "ipv4.ignore-auto-dns", &auto("ipv4.ignore-auto-dns"), "ipv6.dns", &comma("ipv6.dns"), "ipv6.ignore-auto-dns", &auto("ipv6.ignore-auto-dns")]));
        if restored
            && let Some(c) = connections.iter().find(|c| c.name == name) {
                restored = env.change_s(&["nmcli", "device", "reapply", &c.device]);
            }
        if restored {
            state.remove(&k);
            env.ok(&format!("Restored DNS on {name}"));
        } else {
            ok = false;
            env.warn(&format!("DNS restore failed on {name}; saved settings kept"));
        }
    }
    if let Err(e) = env.save_state(&state) {
        env.warn(&format!("could not update the undo state: {e}"));
        ok = false;
    }
    ok
}

// ------------------------------------------------------------------------------------------------ tools

pub fn sysinfo(env: &Env) -> Vec<(String, String)> {
    let d = env.distro();
    let mem = env.read_text("/proc/meminfo").unwrap_or_default();
    let mem_kb = mem.lines().find_map(|l| l.strip_prefix("MemTotal:")).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<f64>().ok());
    let cpu = env.read_text("/proc/cpuinfo").unwrap_or_default().lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim().to_string());
    let kernel = env.run(&["uname", "-r"]).1.trim().to_string();
    let mut rows = vec![
        ("Distro".to_string(), d.get("PRETTY_NAME").and_then(|v| v.as_str()).unwrap_or("unknown").to_string()),
        ("Kernel".into(), kernel),
        ("CPU".into(), cpu.unwrap_or_else(|| "unknown".into())),
        ("Memory".into(), mem_kb.map(|k| fmt_size(k * 1024.0)).unwrap_or_else(|| "unknown".into())),
    ];
    let (rc, out) = env.run(&["df", "-h", "--output=target,size,used,avail", "/"]);
    if rc == 0 && !out.trim().is_empty() {
        rows.push(("Root disk".into(), out.trim().lines().last().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ")));
    }
    rows.push(("Optimaxer".into(), VERSION.into()));
    rows
}

pub fn tool_update_system(env: &Env) -> bool {
    env.info(&format!("Updating the system ({})...", env.pm.update_label()));
    let (rc, out) = crate::sys::pkg_update(env);
    if rc == 0 { env.ok("System updated") } else { env.warn(&format!("Update failed: {}", tail(&out, 160))) }
    rc == 0
}

pub fn tool_failed_units(env: &Env) -> Vec<String> {
    let (_, out) = env.run(&["systemctl", "--failed", "--no-legend", "--plain"]);
    out.lines().filter_map(|l| l.split_whitespace().next().map(String::from)).collect()
}

pub fn tool_pacnew(env: &Env) -> Vec<String> {
    let mut cmd = s(&["find", "/etc"]);
    for (i, pat) in env.pm.conf_patterns().iter().enumerate() {
        if i > 0 {
            cmd.push("-o".into());
        }
        cmd.extend(s(&["-name", pat]));
    }
    let (_, out) = env.exec(&cmd, false, false, None);
    out.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

pub fn tool_smart(env: &Env) -> Vec<String> {
    if !env.have("smartctl") {
        return vec![format!("smartctl not installed ({})", env.pm.install_cmd("smartmontools"))];
    }
    let mut rows = vec![];
    let (_, out) = env.run(&["lsblk", "-dno", "NAME,TYPE"]);
    for line in out.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() == 2 && p[1] == "disk" && !p[0].starts_with("loop") && !p[0].starts_with("zram") {
            let (_, h) = env.run(&["smartctl", "-H", &format!("/dev/{}", p[0])]);
            let health = h.lines().find(|l| l.contains("overall-health")).and_then(|l| l.split_once(':')).and_then(|(_, v)| v.split_whitespace().next().map(String::from));
            rows.push(format!("/dev/{}: {}", p[0], health.unwrap_or_else(|| "unknown".into())));
        }
    }
    rows
}

// ------------------------------------------------------------------------------------------------ config export / import

#[derive(Serialize, Deserialize, Default)]
pub struct ConfigFile {
    #[serde(default)]
    pub optimaxer: String,
    #[serde(default)]
    pub tweaks: Vec<String>,
    #[serde(default)]
    pub apps: Vec<String>,
}

pub fn config_file(mut tweak_ids: Vec<String>, mut app_pkgs: Vec<String>) -> ConfigFile {
    tweak_ids.sort();
    app_pkgs.sort();
    ConfigFile { optimaxer: VERSION.into(), tweaks: tweak_ids, apps: app_pkgs }
}

/// Apply an exported selection: the tweaks first, then the apps.
pub fn apply_config(env: &Env, cfg: &ConfigFile) -> bool {
    let mut ok = true;
    for tid in &cfg.tweaks {
        match crate::catalog::tweak_by_id(tid) {
            None => {
                env.warn(&format!("Unknown tweak {tid}"));
                ok = false;
            }
            Some(t) => {
                if !t.apply(env) {
                    ok = false;
                }
            }
        }
    }
    let apps = crate::catalog::load_apps(env.pm);
    for pkg in &cfg.apps {
        let app = apps.iter().find(|a| &a.pkg == pkg).cloned().unwrap_or(App { cat: String::new(), name: pkg.clone(), src: 'P', pkg: pkg.clone() });
        if !crate::catalog::install_app(env, &app) {
            ok = false;
        }
    }
    ok
}

// ------------------------------------------------------------------------------------------------ settings & launcher

pub fn settings_path(env: &Env) -> String {
    env.p(&format!("{}/.config/optimaxer/settings.json", env.real_home().trim_end_matches('/')))
}

pub fn load_settings(env: &Env) -> Map<String, Value> {
    fs::read_to_string(settings_path(env)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

pub fn save_settings(env: &Env, data: &Map<String, Value>) {
    let path = settings_path(env);
    let dir = std::path::Path::new(&path).parent().unwrap().to_path_buf();
    if fs::create_dir_all(&dir).is_ok() && fs::write(&path, serde_json::to_string_pretty(data).unwrap_or_default()).is_ok() {
        env.chown_user(&dir.to_string_lossy());
        env.chown_user(&path);
    }
}

const LOGO_PNG: &[u8] = include_bytes!("../../assets/logo.png");

pub fn installed_binary(env: &Env) -> String {
    env.p(&format!("{}/.local/bin/optimaxer", env.real_home().trim_end_matches('/')))
}

fn own_exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "optimaxer".into())
}

pub fn desktop_entry(exe: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName=Optimaxer\nComment=Tweaks, apps and cleanup for Manjaro, Arch and Fedora\nExec={exe} gui\nIcon=optimaxer\nTerminal=false\nCategories=System;Settings;\n")
}

/// Put an `optimaxer` command in ~/.local/bin. Returns the paths written.
pub fn install_command(env: &Env) -> std::io::Result<Vec<String>> {
    if env.dry_run {
        env.log("[dry-run] install Optimaxer command", "DRY");
        return Ok(vec![]);
    }
    let target = installed_binary(env);
    let own = own_exe();
    let dir = std::path::Path::new(&target).parent().unwrap().to_path_buf();
    fs::create_dir_all(&dir)?;
    if fs::canonicalize(&target).ok() != fs::canonicalize(&own).ok() {
        let tmp = format!("{target}.new");
        fs::copy(&own, &tmp)?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        fs::rename(&tmp, &target)?;
    }
    env.chown_user(&dir.to_string_lossy());
    env.chown_user(&target);
    Ok(vec![target])
}

/// Write an application-menu entry (and a copy on the Desktop if there is one). Returns the paths written.
pub fn create_launcher(env: &Env) -> std::io::Result<Vec<String>> {
    if env.dry_run {
        env.log("[dry-run] create Optimaxer application-menu launcher", "DRY");
        return Ok(vec![]);
    }
    let home = env.p(env.real_home().trim_end_matches('/'));
    let icon_dir = format!("{home}/.local/share/icons/hicolor/256x256/apps");
    fs::create_dir_all(&icon_dir)?;
    let icon = format!("{icon_dir}/optimaxer.png");
    fs::write(&icon, LOGO_PNG)?;
    let installed = installed_binary(env);
    let exe = if fs::metadata(&installed).is_ok() { installed } else { own_exe() };
    let mut made = vec![];
    for folder in [format!("{home}/.local/share/applications"), format!("{home}/Desktop")] {
        if folder.ends_with("Desktop") && !std::path::Path::new(&folder).is_dir() {
            continue;
        }
        fs::create_dir_all(&folder)?;
        let path = format!("{folder}/optimaxer.desktop");
        fs::write(&path, desktop_entry(&exe))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        made.push(path);
    }
    for p in made.iter().chain(std::iter::once(&icon)) {
        env.chown_user(p);
    }
    Ok(made)
}

// ------------------------------------------------------------------------------------------------ self update

pub fn vtuple(v: &str) -> (u32, u32, u32) {
    let mut nums = v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse::<u32>().unwrap_or(0));
    (nums.next().unwrap_or(0), nums.next().unwrap_or(0), nums.next().unwrap_or(0))
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(secs))).build().into()
}

pub fn asset_name() -> String {
    format!("optimaxer-linux-{}", std::env::consts::ARCH)
}

pub struct Release {
    pub tag: String,
    pub newer: bool,
    pub url: String,
    pub digest: String,
}

pub fn latest_release() -> Result<Release, String> {
    let mut resp = agent(15)
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("User-Agent", "optimaxer-linux")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let rel: Value = resp.body_mut().read_json().map_err(|e| e.to_string())?;
    let name = asset_name();
    let asset = rel["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"] == name.as_str())).ok_or_else(|| format!("latest release has no {name} asset"))?;
    let tag = rel["tag_name"].as_str().unwrap_or("").to_string();
    Ok(Release {
        newer: vtuple(&tag) > vtuple(VERSION),
        url: asset["browser_download_url"].as_str().unwrap_or("").to_string(),
        digest: asset["digest"].as_str().unwrap_or("").to_string(),
        tag,
    })
}

pub fn self_update(env: &Env) -> bool {
    let rel = match latest_release() {
        Ok(r) => r,
        Err(e) => {
            env.warn(&format!("Update check failed: {e}"));
            return false;
        }
    };
    if !rel.newer {
        env.ok(&format!("Optimaxer is up to date (v{VERSION})."));
        return true;
    }
    env.info(&format!("Updating v{VERSION} -> {}", rel.tag));
    let Some(want) = rel.digest.strip_prefix("sha256:").filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())) else {
        env.log("Update rejected: release has no valid SHA-256 checksum", "ERROR");
        return false;
    };
    if env.dry_run {
        env.log(&format!("[dry-run] update Optimaxer to {}", rel.tag), "DRY");
        return true;
    }
    let data = match agent(120).get(&rel.url).header("User-Agent", "optimaxer-linux").call().and_then(|mut r| r.body_mut().with_config().limit(256 * 1024 * 1024).read_to_vec()) {
        Ok(d) => d,
        Err(e) => {
            env.warn(&format!("Download failed: {e}"));
            return false;
        }
    };
    let got = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if !got.eq_ignore_ascii_case(want) {
        env.log("Update rejected: checksum mismatch", "ERROR");
        return false;
    }
    if !data.starts_with(b"\x7fELF") {
        env.log("Update rejected: not a Linux executable", "ERROR");
        return false;
    }
    let dest = own_exe();
    let tmp = format!("{dest}.new");
    let result = fs::write(&tmp, &data).and_then(|_| fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))).and_then(|_| fs::rename(&tmp, &dest));
    match result {
        Ok(()) => {
            env.ok(&format!("Updated {dest} to {}. Run it again.", rel.tag));
            true
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            env.warn(&format!("Could not replace {dest}: {e}"));
            false
        }
    }
}

// ------------------------------------------------------------------------------------------------ undo everything

pub fn undo_all(env: &Env) -> bool {
    let state = match env.load_state() {
        Ok(st) => st,
        Err(e) => {
            env.warn(&e);
            return false;
        }
    };
    let mut ok = true;
    for k in state.keys().cloned().collect::<Vec<_>>() {
        if let Some(t) = crate::catalog::tweak_by_id(&k) {
            ok &= t.undo(env);
        }
    }
    let services: Vec<String> = env.load_state().map(|s| s.keys().filter_map(|k| k.strip_prefix("service:").map(String::from)).collect()).unwrap_or_default();
    for unit in services {
        ok &= restore_service(env, &unit);
    }
    if env.load_state().map(|s| s.keys().any(|k| k.starts_with("dns:"))).unwrap_or(false) {
        ok &= restore_dns(env);
    }
    ok
}

/// Number of saved originals that undo_all would try to restore.
pub fn undoable_count(env: &Env) -> usize {
    env.load_state().map(|s| s.keys().filter(|k| crate::catalog::tweak_by_id(k).is_some() || k.starts_with("service:") || k.starts_with("dns:")).count()).unwrap_or(0)
}
