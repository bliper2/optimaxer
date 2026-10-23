//! Hand-ported tweaks whose PowerShell versions ran scripts: powercfg, fsutil, bcdedit, netsh, wevtutil, dism and
//! registry key removal. Each one reports what it will do (`describe`), whether it is active (`test`), and how to
//! apply and undo itself. Whatever `apply` returns is stored in the undo record and handed back to `undo`.
//! Ports that change many values (Nagle, the boot timer, event logs) first take a `snapshot` of every original; the
//! engine saves it before anything changes and `undo` puts exactly that back, failing loudly when it cannot.

use super::backend::{RegData, System};
use serde_json::{json, Value};
use std::path::Path;

const SVC_ROOT: &str = r"HKLM:\SYSTEM\CurrentControlSet\Services";
const CPU_SUB: &str = "54533251-82be-4824-96c1-47b60b740d00";
const POWER_KEY: &str = r"HKLM:\SYSTEM\CurrentControlSet\Control\Power";
const FS_KEY: &str = r"HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fs {
    LastAccess,
    Trim,
    Short8dot3,
}

#[derive(Debug)]
pub struct AcItem {
    pub sub: &'static str,
    pub setting: &'static str,
    pub val: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum Special {
    Ultimate,
    Hibernate,
    Fsutil(Fs),
    PowerAc(&'static [AcItem]),
    BcdDynamicTick,
    BcdLegacyBootMenu,
    Teredo,
    Nagle,
    SvcHostSplit,
    DropRegValue { path: &'static str, name: &'static str },
    CreateKey { create: &'static str, remove: &'static str },
    EventLogs(&'static [&'static str]),
    ReservedStorage,
    RemoveKeys { keys: &'static [&'static str], test: &'static [&'static str], tag: &'static str },
    KillProcs(&'static [&'static str]),
}

pub struct Ctx<'a> {
    pub sys: &'a dyn System,
    pub backups: &'a Path,
}

pub static CPU_AC: [AcItem; 2] = [
    AcItem { sub: CPU_SUB, setting: "0cc5b647-c1df-4637-891a-dec35c318583", val: 100 },
    AcItem { sub: CPU_SUB, setting: "bc5038f7-23e0-4960-96da-33abaf5935ec", val: 100 },
];
pub static USB_PCIE_AC: [AcItem; 2] = [
    AcItem { sub: "2a737441-1930-4402-8d77-b2bebba308a3", setting: "48e6b7a6-50f5-4782-a5d4-53bb8f07e226", val: 0 },
    AcItem { sub: "501a4d13-42af-4429-9fd1-a8218c268e20", setting: "ee12f906-d277-404b-b6da-e5fa1a576df5", val: 0 },
];

// region text helpers
/// First 8-4-4-4-12 hex GUID in the text. powercfg output is parsed this way so it works in any language.
pub fn find_guid(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let shape = [8usize, 4, 4, 4, 12];
    let total = 36;
    (0..b.len().saturating_sub(total - 1)).find_map(|i| {
        let mut pos = i;
        for (n, len) in shape.iter().enumerate() {
            if !b[pos..pos + len].iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            pos += len;
            if n < 4 {
                if b.get(pos) != Some(&b'-') {
                    return None;
                }
                pos += 1;
            }
        }
        Some(text[i..i + total].to_string())
    })
}

/// Name in parentheses after a GUID in `powercfg /list` output, e.g. "Ultimate Performance".
fn scheme_name(list: &str, guid: &str) -> String {
    let l = list.to_lowercase();
    let Some(i) = l.find(&guid.to_lowercase()) else { return String::new() };
    let rest = &list[i + guid.len()..];
    match (rest.find('('), rest.find(')')) {
        (Some(a), Some(b)) if b > a => rest[a + 1..b].to_string(),
        _ => String::new(),
    }
}

fn first_digit_after(text: &str, marker: &str) -> Option<u32> {
    let l = text.to_lowercase();
    let i = l.find(marker)? + marker.len();
    l[i..].chars().find(char::is_ascii_digit).and_then(|c| c.to_digit(10))
}

/// True when a line starts with `key` and its next word is `val` (case-insensitive): bcdedit's "key   value" rows.
fn has_pair(text: &str, key: &str, val: &str) -> bool {
    text.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next().is_some_and(|k| k.eq_ignore_ascii_case(key)) && w.next().is_some_and(|v| v.eq_ignore_ascii_case(val))
    })
}

/// Second word of the bcdedit row whose first word is `key`, e.g. "Yes" for "disabledynamictick  Yes".
fn bcd_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let mut w = l.split_whitespace();
        if w.next().is_some_and(|k| k.eq_ignore_ascii_case(key)) { w.next().map(str::to_string) } else { None }
    })
}

const BCD_TIMER: [&str; 3] = ["disabledynamictick", "useplatformclock", "useplatformtick"];

fn checked(sys: &dyn System, exe: &str, args: &[&str]) -> Result<String, String> {
    let out = sys.run(exe, args);
    if out.code == 0 { Ok(out.text) } else { Err(format!("{exe} {} failed: {}", args.join(" "), out.text.trim())) }
}

/// The log's enabled flag from `wevtutil gl`, or None when this Windows has no such log.
fn log_enabled(sys: &dyn System, log: &str) -> Option<bool> {
    let out = sys.run("wevtutil.exe", &["gl", log]);
    if out.code != 0 {
        return None;
    }
    out.text.lines().find_map(|l| {
        let l = l.trim().to_lowercase();
        l.strip_prefix("enabled:").map(|v| v.trim() == "true")
    })
}

fn reg_snapshot(sys: &dyn System, path: &str, name: &str) -> Value {
    match sys.reg_get(path, name) {
        Some(d) => json!({ "Exists": true, "Value": d.to_json(), "Type": d.type_name() }),
        None => json!({ "Exists": false, "Value": null, "Type": null }),
    }
}

fn reg_restore(sys: &dyn System, path: &str, name: &str, snap: &Value) -> Result<(), String> {
    if snap.get("Exists").and_then(Value::as_bool).unwrap_or(false) {
        let ty = snap.get("Type").and_then(Value::as_str).unwrap_or("DWord");
        let d = snap.get("Value").and_then(|v| RegData::from_json(v, ty)).ok_or_else(|| format!("{path}\\{name}: saved value is unreadable"))?;
        sys.reg_set(path, name, &d)
    } else if sys.reg_get(path, name).is_some() {
        sys.reg_delete_value(path, name)
    } else {
        Ok(())
    }
}

fn active_scheme(sys: &dyn System) -> Option<String> {
    find_guid(&sys.run("powercfg", &["/getactivescheme"]).text)
}

fn get_ac(sys: &dyn System, sub: &str, setting: &str) -> Option<u32> {
    let out = sys.run("powercfg", &["/q", "SCHEME_CURRENT", sub, setting]).text;
    let hex: Vec<u32> = out
        .split_whitespace()
        .filter_map(|w| w.strip_prefix("0x").filter(|h| h.len() == 8).and_then(|h| u32::from_str_radix(h, 16).ok()))
        .collect();
    (hex.len() >= 2).then(|| hex[hex.len() - 2])
}

fn set_ac(sys: &dyn System, items: &[(String, String, u32)]) -> Vec<Value> {
    let mut old = Vec::new();
    for (sub, setting, val) in items {
        if let Some(cur) = get_ac(sys, sub, setting) {
            old.push(json!({ "Sub": sub, "Set": setting, "Val": cur }));
        }
        sys.run("powercfg", &["/setacvalueindex", "SCHEME_CURRENT", sub, setting, &val.to_string()]);
    }
    sys.run("powercfg", &["/setactive", "SCHEME_CURRENT"]);
    old
}

fn fs_query(sys: &dyn System, kind: Fs) -> String {
    match kind {
        Fs::LastAccess => sys.run("fsutil", &["behavior", "query", "disablelastaccess"]).text,
        Fs::Trim => sys.run("fsutil", &["behavior", "query", "DisableDeleteNotify"]).text,
        Fs::Short8dot3 => sys.run("fsutil", &["8dot3name", "query"]).text,
    }
}

fn fs_set(sys: &dyn System, kind: Fs, v: u32) {
    let v = v.to_string();
    match kind {
        Fs::LastAccess => sys.run("fsutil", &["behavior", "set", "disablelastaccess", &v]),
        Fs::Trim => sys.run("fsutil", &["behavior", "set", "DisableDeleteNotify", &v]),
        Fs::Short8dot3 => sys.run("fsutil", &["8dot3name", "set", &v]),
    };
}

fn trim_enabled(query: &str) -> bool {
    query.lines().any(|l| l.trim_start().starts_with("NTFS") && l.contains("= 0"))
}
// endregion

impl Special {
    pub fn describe(&self) -> String {
        match self {
            Special::Ultimate => "Switch the active power plan to Ultimate Performance (created if missing)".into(),
            Special::Hibernate => "powercfg /hibernate off (deletes hiberfil.sys)".into(),
            Special::Fsutil(Fs::LastAccess) => "fsutil behavior set disablelastaccess 1".into(),
            Special::Fsutil(Fs::Trim) => "fsutil behavior set DisableDeleteNotify 0 (TRIM on)".into(),
            Special::Fsutil(Fs::Short8dot3) => "fsutil 8dot3name set 1".into(),
            Special::PowerAc(items) => format!("Set {} power-plan value(s) on AC power with powercfg", items.len()),
            Special::BcdDynamicTick => "bcdedit /set disabledynamictick yes (restart needed; the current boot values are saved first)".into(),
            Special::BcdLegacyBootMenu => "bcdedit /set {default} bootmenupolicy legacy".into(),
            Special::Teredo => "netsh interface teredo set state disabled".into(),
            Special::Nagle => "Set TcpAckFrequency=1 and TCPNoDelay=1 on every active network interface (old values saved first)".into(),
            Special::SvcHostSplit => "Set SvcHostSplitDisable=1 on every service key (Xbox services excluded)".into(),
            Special::DropRegValue { path, name } => format!("Remove {path}\\{name} (the old value is saved for undo)"),
            Special::CreateKey { create, .. } => format!("Create registry key {create}"),
            Special::EventLogs(logs) => format!("Turn off {} diagnostic event log(s) with wevtutil", logs.len()),
            Special::ReservedStorage => "DISM: disable Reserved Storage".into(),
            Special::RemoveKeys { keys, .. } => format!("Export, then delete {} registry key(s); undo imports the backup", keys.len()),
            Special::KillProcs(p) => format!("End running process(es): {}", p.join(", ")),
        }
    }

    /// `None` means the script version had no check of its own, so the declarative part (or the undo record) decides.
    pub fn test(&self, ctx: &Ctx) -> Option<bool> {
        let sys = ctx.sys;
        match self {
            Special::Ultimate => {
                let guid = active_scheme(sys)?;
                Some(scheme_name(&sys.run("powercfg", &["/list"]).text, &guid).to_lowercase().contains("ultimate"))
            }
            Special::Hibernate => Some(matches!(sys.reg_get(POWER_KEY, "HibernateEnabled"), Some(RegData::Dword(0)))),
            Special::Fsutil(Fs::LastAccess) => {
                let q = fs_query(sys, Fs::LastAccess);
                Some(matches!(first_digit_after(&q, "="), Some(1 | 3)))
            }
            Special::Fsutil(Fs::Trim) => Some(trim_enabled(&fs_query(sys, Fs::Trim))),
            Special::Fsutil(Fs::Short8dot3) => Some(matches!(sys.reg_get(FS_KEY, "NtfsDisable8dot3NameCreation"), Some(RegData::Dword(1)))),
            Special::PowerAc(items) => Some(items.iter().all(|i| get_ac(sys, i.sub, i.setting) == Some(i.val))),
            Special::BcdDynamicTick => Some(has_pair(&sys.run("bcdedit", &["/enum", "{current}"]).text, "disabledynamictick", "yes")),
            Special::BcdLegacyBootMenu => Some(has_pair(&sys.run("bcdedit", &["/enum", "{current}"]).text, "bootmenupolicy", "legacy")),
            Special::SvcHostSplit => {
                let on = |n: &str| matches!(sys.reg_get(&format!(r"{SVC_ROOT}\{n}"), "SvcHostSplitDisable"), Some(RegData::Dword(1)));
                Some(on("Dnscache") && on("Schedule"))
            }
            Special::CreateKey { create, .. } => Some(sys.reg_key_exists(create)),
            Special::RemoveKeys { test, .. } => Some(test.iter().all(|k| !sys.reg_key_exists(k))),
            _ => None,
        }
    }

    /// Reads the originals of a port that changes many values, without changing anything. `None` for ports whose
    /// `apply` returns its own undo data.
    pub fn snapshot(&self, ctx: &Ctx) -> Result<Option<Value>, String> {
        let sys = ctx.sys;
        Ok(match self {
            Special::Nagle => {
                let base = format!(r"{SVC_ROOT}\Tcpip\Parameters\Interfaces");
                let mut out = Vec::new();
                for k in sys.reg_subkeys(&base) {
                    let p = format!(r"{base}\{k}");
                    let has_ip = matches!(sys.reg_get(&p, "DhcpIPAddress"), Some(RegData::Str(s)) if !s.is_empty())
                        || matches!(sys.reg_get(&p, "IPAddress"), Some(RegData::Multi(m)) if m.iter().any(|s| !s.is_empty()))
                        || matches!(sys.reg_get(&p, "IPAddress"), Some(RegData::Str(s)) if !s.is_empty());
                    if has_ip {
                        out.push(json!({ "If": k, "TcpAckFrequency": reg_snapshot(sys, &p, "TcpAckFrequency"), "TCPNoDelay": reg_snapshot(sys, &p, "TCPNoDelay") }));
                    }
                }
                Some(Value::Array(out))
            }
            Special::BcdDynamicTick => {
                let text = checked(sys, "bcdedit", &["/enum", "{current}"])?;
                let mut m = serde_json::Map::new();
                for n in BCD_TIMER {
                    m.insert(n.into(), bcd_value(&text, n).map(Value::String).unwrap_or(Value::Null));
                }
                Some(Value::Object(m))
            }
            Special::EventLogs(logs) => {
                let mut m = serde_json::Map::new();
                for l in logs.iter() {
                    if let Some(on) = log_enabled(sys, l) {
                        m.insert(l.to_string(), Value::Bool(on));
                    }
                }
                Some(Value::Object(m))
            }
            _ => None,
        })
    }

    /// Applies a snapshot port with the originals the engine already saved.
    pub fn apply_snapshot(&self, ctx: &Ctx, snap: &Value, log: &mut dyn FnMut(&str)) -> Result<(), String> {
        let sys = ctx.sys;
        match self {
            Special::Nagle => {
                let base = format!(r"{SVC_ROOT}\Tcpip\Parameters\Interfaces");
                let mut n = 0;
                for i in snap.as_array().into_iter().flatten() {
                    let Some(k) = i.get("If").and_then(Value::as_str) else { continue };
                    let p = format!(r"{base}\{k}");
                    sys.reg_set(&p, "TcpAckFrequency", &RegData::Dword(1))?;
                    sys.reg_set(&p, "TCPNoDelay", &RegData::Dword(1))?;
                    n += 1;
                }
                log(&format!("  {n} network interface(s) updated"));
                Ok(())
            }
            Special::BcdDynamicTick => {
                checked(sys, "bcdedit", &["/set", "disabledynamictick", "yes"])?;
                if snap.get("useplatformclock").is_some_and(|v| !v.is_null()) {
                    checked(sys, "bcdedit", &["/deletevalue", "useplatformclock"])?;
                }
                Ok(())
            }
            Special::EventLogs(_) => {
                for (l, _) in snap.as_object().into_iter().flatten() {
                    checked(sys, "wevtutil.exe", &["set-log", l, "/e:false"])?;
                }
                Ok(())
            }
            other => other.apply(ctx, log).map(|_| ()),
        }
    }

    pub fn apply(&self, ctx: &Ctx, log: &mut dyn FnMut(&str)) -> Result<Value, String> {
        let sys = ctx.sys;
        match self {
            Special::Ultimate => {
                let prev = active_scheme(sys);
                let list = sys.run("powercfg", &["/list"]).text;
                let existing = list.lines().find(|l| l.to_lowercase().contains("ultimate")).and_then(find_guid);
                let mut created = None;
                let guid = match existing {
                    Some(g) => Some(g),
                    None => {
                        let g = find_guid(&sys.run("powercfg", &["-duplicatescheme", "e9a42b02-d5df-448d-aa00-03f14749eb61"]).text);
                        created = g.clone();
                        g
                    }
                };
                let guid = guid.ok_or("could not create the Ultimate Performance plan on this Windows edition")?;
                sys.run("powercfg", &["-setactive", &guid]);
                Ok(json!({ "Prev": prev, "Created": created }))
            }
            Special::Hibernate => {
                sys.run("powercfg", &["/hibernate", "off"]);
                Ok(Value::Null)
            }
            Special::Fsutil(kind) => {
                let q = fs_query(sys, *kind);
                let (prev, target) = match kind {
                    Fs::LastAccess => (first_digit_after(&q, "=").unwrap_or(2), 1),
                    Fs::Trim => (if trim_enabled(&q) { 0 } else { 1 }, 0),
                    Fs::Short8dot3 => (first_digit_after(&q, "state").or_else(|| first_digit_after(&q, "value")).unwrap_or(2), 1),
                };
                fs_set(sys, *kind, target);
                Ok(json!({ "Prev": prev }))
            }
            Special::PowerAc(items) => {
                let want: Vec<_> = items.iter().map(|i| (i.sub.to_string(), i.setting.to_string(), i.val)).collect();
                Ok(Value::Array(set_ac(sys, &want)))
            }
            Special::BcdDynamicTick | Special::Nagle | Special::EventLogs(_) => {
                let snap = self.snapshot(ctx)?.unwrap_or(Value::Null);
                self.apply_snapshot(ctx, &snap, log)?;
                Ok(snap)
            }
            Special::BcdLegacyBootMenu => {
                sys.run("bcdedit", &["/set", "{default}", "bootmenupolicy", "legacy"]);
                Ok(Value::Null)
            }
            Special::Teredo => {
                sys.run("netsh", &["interface", "teredo", "set", "state", "disabled"]);
                Ok(Value::Null)
            }
            Special::SvcHostSplit => {
                let mut done = Vec::new();
                for k in sys.reg_subkeys(SVC_ROOT) {
                    if k.contains("Xbl") || k.contains("Xbox") {
                        continue;
                    }
                    let p = format!(r"{SVC_ROOT}\{k}");
                    if sys.reg_get(&p, "Start").is_some() && sys.reg_get(&p, "SvcHostSplitDisable").is_none() && sys.reg_set(&p, "SvcHostSplitDisable", &RegData::Dword(1)).is_ok() {
                        done.push(k);
                    }
                }
                log(&format!("  {} service key(s) updated", done.len()));
                Ok(json!(done))
            }
            Special::DropRegValue { path, name } => {
                let cur = sys.reg_get(path, name);
                if cur.is_some() {
                    sys.reg_delete_value(path, name)?;
                }
                Ok(match cur {
                    Some(d) => json!({ "Existed": true, "Value": d.to_json(), "Type": d.type_name() }),
                    None => json!({ "Existed": false }),
                })
            }
            Special::CreateKey { create, .. } => {
                sys.reg_create_key(create)?;
                // the classic-menu key needs an empty default value to take effect
                sys.reg_set(create, "", &RegData::Str(String::new()))?;
                Ok(Value::Null)
            }
            Special::ReservedStorage => {
                sys.run("dism.exe", &["/Online", "/Set-ReservedStorageState", "/State:Disabled"]);
                Ok(Value::Null)
            }
            Special::RemoveKeys { keys, tag, .. } => {
                let stamp = super::now_stamp();
                let mut files = Vec::new();
                for (i, k) in keys.iter().enumerate() {
                    if !sys.reg_key_exists(k) {
                        continue;
                    }
                    let file = ctx.backups.join(format!("{tag}-{stamp}-{}.reg", i + 1));
                    let f = file.to_string_lossy().to_string();
                    sys.run("reg.exe", &["export", k, &f, "/y"]);
                    if sys.file_exists(&f) {
                        sys.reg_delete_key(k)?;
                        files.push(f);
                    } else {
                        log(&format!("  could not back up {k}, left untouched"));
                    }
                }
                Ok(json!(files))
            }
            Special::KillProcs(names) => {
                for n in names.iter() {
                    sys.run("taskkill", &["/F", "/IM", &format!("{n}.exe")]);
                }
                Ok(Value::Null)
            }
        }
    }

    /// Puts the saved originals back. An error means something was not restored; the engine then keeps the record.
    pub fn undo(&self, ctx: &Ctx, data: &Value, log: &mut dyn FnMut(&str)) -> Result<(), String> {
        let sys = ctx.sys;
        match self {
            Special::Ultimate => {
                if let Some(p) = data.get("Prev").and_then(Value::as_str) {
                    sys.run("powercfg", &["-setactive", p]);
                }
                if let Some(c) = data.get("Created").and_then(Value::as_str) {
                    sys.run("powercfg", &["-delete", c]);
                }
            }
            Special::Hibernate => {
                sys.run("powercfg", &["/hibernate", "on"]);
            }
            Special::Fsutil(kind) => {
                let prev = data.get("Prev").and_then(Value::as_u64).unwrap_or(2) as u32;
                match kind {
                    Fs::Trim => {
                        if prev == 1 {
                            fs_set(sys, Fs::Trim, 1);
                        }
                    }
                    k => fs_set(sys, *k, prev),
                }
            }
            Special::PowerAc(_) => {
                let old: Vec<(String, String, u32)> = data
                    .as_array()
                    .map(|a| a.iter().filter_map(|o| Some((o.get("Sub")?.as_str()?.to_string(), o.get("Set")?.as_str()?.to_string(), o.get("Val")?.as_u64()? as u32))).collect())
                    .unwrap_or_default();
                if !old.is_empty() {
                    set_ac(sys, &old);
                }
            }
            Special::BcdDynamicTick => {
                if let Some(had) = data.get("HadPlatformClock") {
                    // record from v2.13.5 or older, which kept only this flag
                    checked(sys, "bcdedit", &["/deletevalue", "disabledynamictick"])?;
                    if had.as_bool().unwrap_or(false) {
                        checked(sys, "bcdedit", &["/set", "useplatformclock", "yes"])?;
                    }
                    return Ok(());
                }
                if !data.is_object() {
                    return Err("no saved boot values".into());
                }
                let now = checked(sys, "bcdedit", &["/enum", "{current}"])?;
                for n in BCD_TIMER {
                    let cur = bcd_value(&now, n);
                    match data.get(n).and_then(Value::as_str) {
                        Some(old) if cur.as_deref() != Some(old) => {
                            checked(sys, "bcdedit", &["/set", n, old])?;
                        }
                        None if cur.is_some() => {
                            checked(sys, "bcdedit", &["/deletevalue", n])?;
                        }
                        _ => {}
                    }
                }
            }
            Special::BcdLegacyBootMenu => {
                sys.run("bcdedit", &["/set", "{default}", "bootmenupolicy", "standard"]);
            }
            Special::Teredo => {
                sys.run("netsh", &["interface", "teredo", "set", "state", "default"]);
            }
            Special::Nagle => {
                let base = format!(r"{SVC_ROOT}\Tcpip\Parameters\Interfaces");
                let mut bad = Vec::new();
                for i in data.as_array().into_iter().flatten() {
                    if let Some(k) = i.as_str() {
                        // record from v2.13.5 or older: only the interface name was kept, and both values were deleted
                        let p = format!(r"{base}\{k}");
                        let _ = sys.reg_delete_value(&p, "TcpAckFrequency");
                        let _ = sys.reg_delete_value(&p, "TCPNoDelay");
                        continue;
                    }
                    let Some(k) = i.get("If").and_then(Value::as_str) else { continue };
                    let p = format!(r"{base}\{k}");
                    for n in ["TcpAckFrequency", "TCPNoDelay"] {
                        if let Some(snap) = i.get(n) {
                            if let Err(e) = reg_restore(sys, &p, n, snap) {
                                bad.push(format!("{p}\\{n}: {e}"));
                            }
                        }
                    }
                }
                if !bad.is_empty() {
                    return Err(format!("could not restore {}", bad.join("; ")));
                }
            }
            Special::SvcHostSplit => {
                for n in data.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    let _ = sys.reg_delete_value(&format!(r"{SVC_ROOT}\{n}"), "SvcHostSplitDisable");
                }
            }
            Special::DropRegValue { path, name } => {
                if data.get("Existed").and_then(Value::as_bool).unwrap_or(false) {
                    let ty = data.get("Type").and_then(Value::as_str).unwrap_or("String");
                    let d = data.get("Value").and_then(|v| RegData::from_json(v, ty)).ok_or_else(|| format!("{path}\\{name}: saved value is unreadable"))?;
                    if let Err(e) = sys.reg_set(path, name, &d) {
                        log(&format!("  restore failed {path}\\{name}: {e}"));
                        return Err(e);
                    }
                }
            }
            Special::CreateKey { remove, .. } => {
                sys.reg_delete_key(remove)?;
            }
            Special::EventLogs(logs) => {
                let mut bad = Vec::new();
                let pairs: Vec<(String, bool)> = match data.as_object() {
                    Some(m) => m.iter().map(|(k, v)| (k.clone(), v.as_bool().unwrap_or(true))).collect(),
                    // records from v2.13.5 or older saved nothing; those versions always turned the logs back on
                    None => logs.iter().filter(|l| log_enabled(sys, l).is_some()).map(|l| (l.to_string(), true)).collect(),
                };
                for (l, on) in pairs {
                    if let Err(e) = checked(sys, "wevtutil.exe", &["set-log", &l, if on { "/e:true" } else { "/e:false" }]) {
                        bad.push(e);
                    }
                }
                if !bad.is_empty() {
                    return Err(bad.join("; "));
                }
            }
            Special::ReservedStorage => {
                sys.run("dism.exe", &["/Online", "/Set-ReservedStorageState", "/State:Enabled"]);
            }
            Special::RemoveKeys { .. } => {
                for f in data.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    if sys.file_exists(f) {
                        sys.run("reg.exe", &["import", f]);
                    }
                }
            }
            Special::KillProcs(_) => {}
        }
        Ok(())
    }
}

/// Maps a PowerShell tweak id to its Rust port. Ids missing here and still holding a script are not ported.
pub fn special_for(id: &str) -> Option<Special> {
    const SHARE: &str = r"HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\ModernSharing";
    const ACCESS: [&str; 6] = [
        r"HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\Sharing",
        r"HKEY_CLASSES_ROOT\Directory\Background\shellex\ContextMenuHandlers\Sharing",
        r"HKEY_CLASSES_ROOT\Directory\shellex\ContextMenuHandlers\Sharing",
        r"HKEY_CLASSES_ROOT\Drive\shellex\ContextMenuHandlers\Sharing",
        r"HKEY_CLASSES_ROOT\LibraryFolder\background\shellex\ContextMenuHandlers\Sharing",
        r"HKEY_CLASSES_ROOT\UserLibraryFolder\shellex\ContextMenuHandlers\Sharing",
    ];
    static SHARE_KEYS: [&str; 1] = [SHARE];
    static ACCESS_KEYS: [&str; 6] = ACCESS;
    static ACCESS_TEST: [&str; 3] = [ACCESS[0], ACCESS[2], ACCESS[3]];
    static LIB_KEYS: [&str; 2] = [r"HKEY_CLASSES_ROOT\Folder\ShellEx\ContextMenuHandlers\Library Location", r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\Folder\ShellEx\ContextMenuHandlers\Library Location"];
    static LIB_TEST: [&str; 1] = [LIB_KEYS[1]];
    static THISPC_KEYS: [&str; 5] = [
        concat!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace", r"\{0DB7E03F-FC29-4DC6-9020-FF41B59E513A}"),
        concat!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace", r"\{0DB7E03F-FC29-4DC6-9020-FF41B59E513A}"),
        concat!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace", r"\{3dfdf296-dbec-4fb4-81d1-6a3438bcf4de}"),
        concat!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Explorer\MyComputer\NameSpace", r"\{3dfdf296-dbec-4fb4-81d1-6a3438bcf4de}"),
        r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace\DelegateFolders\{F5FB2C77-0E2F-4A16-A381-3E560C68BC83}",
    ];
    static THISPC_TEST: [&str; 2] = [THISPC_KEYS[2], THISPC_KEYS[4]];
    static OOBE_KEYS: [&str; 2] = [
        r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\WindowsUpdate\Orchestrator\UScheduler_Oobe\OutlookUpdate",
        r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\WindowsUpdate\Orchestrator\UScheduler_Oobe\DevHomeUpdate",
    ];
    static SLEEP_LOGS: [&str; 3] = ["Microsoft-Windows-SleepStudy/Diagnostic", "Microsoft-Windows-Kernel-Processor-Power/Diagnostic", "Microsoft-Windows-UserModePowerService/Diagnostic"];
    static WIDGET_PROCS: [&str; 3] = ["Widgets", "WidgetService", "Win32Bridge.Server"];
    Some(match id {
        "perf-ultimate" => Special::Ultimate,
        "perf-hibernate" => Special::Hibernate,
        "perf-ntfs" => Special::Fsutil(Fs::LastAccess),
        "perf-trimon" => Special::Fsutil(Fs::Trim),
        "perf-8dot3" => Special::Fsutil(Fs::Short8dot3),
        "perf-cpupower" => Special::PowerAc(&CPU_AC),
        "perf-usbpcie" => Special::PowerAc(&USB_PCIE_AC),
        "perf-timer" => Special::BcdDynamicTick,
        "ui-legacyboot" => Special::BcdLegacyBootMenu,
        "net-teredo" => Special::Teredo,
        "net-nagle" => Special::Nagle,
        "perf-svchostsplit" => Special::SvcHostSplit,
        "priv-feedback" => Special::DropRegValue { path: r"HKCU:\Software\Microsoft\Siuf\Rules", name: "PeriodInNanoSeconds" },
        "ui-classicmenu" => Special::CreateKey {
            create: r"HKCU:\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32",
            remove: r"HKCU:\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}",
        },
        "ui-netwizard" => Special::CreateKey {
            create: r"HKLM:\SYSTEM\CurrentControlSet\Control\Network\NewNetworkWindowOff",
            remove: r"HKLM:\SYSTEM\CurrentControlSet\Control\Network\NewNetworkWindowOff",
        },
        "perf-sleepstudy" => Special::EventLogs(&SLEEP_LOGS),
        "sto-reserved" => Special::ReservedStorage,
        "ui-ctx-share" => Special::RemoveKeys { keys: &SHARE_KEYS, test: &SHARE_KEYS, tag: "ctx-share" },
        "ui-ctx-giveaccess" => Special::RemoveKeys { keys: &ACCESS_KEYS, test: &ACCESS_TEST, tag: "ctx-giveaccess" },
        "ui-ctx-library" => Special::RemoveKeys { keys: &LIB_KEYS, test: &LIB_TEST, tag: "ctx-library" },
        "ui-thispc-clean" => Special::RemoveKeys { keys: &THISPC_KEYS, test: &THISPC_TEST, tag: "thispc" },
        "debloat-oobeapps" => Special::RemoveKeys { keys: &OOBE_KEYS, test: &OOBE_KEYS, tag: "oobe" },
        "debloat-widgets" => Special::KillProcs(&WIDGET_PROCS),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;
    use std::path::PathBuf;

    fn ctx<'a>(sys: &'a FakeSystem, dir: &'a Path) -> Ctx<'a> {
        Ctx { sys, backups: dir }
    }

    #[test]
    fn guid_and_scheme_name_parse_in_any_language() {
        let out = "Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)";
        assert_eq!(find_guid(out).as_deref(), Some("381b4222-f694-41f0-9685-ff5bb260df2e"));
        assert_eq!(scheme_name(out, "381b4222-f694-41f0-9685-ff5bb260df2e"), "Balanced");
        assert_eq!(find_guid("no guid here"), None);
    }

    #[test]
    fn ultimate_plan_is_created_activated_and_undone() {
        let sys = FakeSystem::default();
        sys.answer("powercfg /getactivescheme", "GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)");
        sys.answer("powercfg /list", "GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced) *");
        sys.answer("powercfg -duplicatescheme", "GUID: 11111111-2222-3333-4444-555555555555  (Ultimate Performance)");
        let dir = PathBuf::new();
        let c = ctx(&sys, &dir);
        let data = Special::Ultimate.apply(&c, &mut |_| {}).unwrap();
        assert!(sys.ran("powercfg -setactive 11111111-2222-3333-4444-555555555555"));
        assert_eq!(data["Prev"], "381b4222-f694-41f0-9685-ff5bb260df2e");
        Special::Ultimate.undo(&c, &data, &mut |_| {}).unwrap();
        assert!(sys.ran("powercfg -setactive 381b4222-f694-41f0-9685-ff5bb260df2e"));
        assert!(sys.ran("powercfg -delete 11111111-2222-3333-4444-555555555555"));
    }

    #[test]
    fn ac_values_use_the_second_to_last_hex_value() {
        let sys = FakeSystem::default();
        sys.answer("powercfg /q", "Current AC Power Setting Index: 0x00000005\nCurrent DC Power Setting Index: 0x00000000");
        assert_eq!(get_ac(&sys, "a", "b"), Some(5));
    }

    #[test]
    fn fsutil_remembers_and_restores_the_previous_value() {
        let sys = FakeSystem::default();
        sys.answer("fsutil behavior query disablelastaccess", "DisableLastAccess = 2  (System Managed, Enabled)");
        let dir = PathBuf::new();
        let c = ctx(&sys, &dir);
        let s = Special::Fsutil(Fs::LastAccess);
        let data = s.apply(&c, &mut |_| {}).unwrap();
        assert_eq!(data["Prev"], 2);
        assert!(sys.ran("fsutil behavior set disablelastaccess 1"));
        s.undo(&c, &data, &mut |_| {}).unwrap();
        assert!(sys.ran("fsutil behavior set disablelastaccess 2"));
    }

    #[test]
    fn trim_check_reads_the_ntfs_line() {
        assert!(trim_enabled("NTFS DisableDeleteNotify = 0  (Disabled)\nReFS DisableDeleteNotify = 1"));
        assert!(!trim_enabled("NTFS DisableDeleteNotify = 1  (Enabled)"));
    }

    #[test]
    fn remove_keys_backs_up_before_deleting_and_undo_imports() {
        let sys = FakeSystem::default();
        let key = r"HKEY_CLASSES_ROOT\*\shellex\ContextMenuHandlers\ModernSharing";
        sys.reg_create_key(key).unwrap();
        let dir = PathBuf::from(r"C:\backups");
        let c = ctx(&sys, &dir);
        let s = special_for("ui-ctx-share").unwrap();
        assert_eq!(s.test(&c), Some(false));
        let data = s.apply(&c, &mut |_| {}).unwrap();
        assert!(sys.ran("reg.exe export"));
        assert_eq!(s.test(&c), Some(true));
        s.undo(&c, &data, &mut |_| {}).unwrap();
        assert!(sys.ran("reg.exe import"));
    }

    #[test]
    fn nagle_touches_only_interfaces_with_an_address() {
        let sys = FakeSystem::default();
        let base = r"HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces";
        sys.put(&format!(r"{base}\{{A}}"), "DhcpIPAddress", RegData::Str("192.168.1.5".into()));
        sys.put(&format!(r"{base}\{{B}}"), "DhcpIPAddress", RegData::Str(String::new()));
        let dir = PathBuf::new();
        let c = ctx(&sys, &dir);
        let data = Special::Nagle.apply(&c, &mut |_| {}).unwrap();
        assert_eq!(data.as_array().unwrap().len(), 1);
        assert_eq!(data[0]["If"], "{a}");
        assert_eq!(sys.reg_get(&format!(r"{base}\{{A}}"), "TCPNoDelay"), Some(RegData::Dword(1)));
        assert_eq!(sys.reg_get(&format!(r"{base}\{{B}}"), "TCPNoDelay"), None);
    }

    #[test]
    fn nagle_undo_restores_a_pre_existing_value_instead_of_deleting_it() {
        let sys = FakeSystem::default();
        let base = r"HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces";
        let a = format!(r"{base}\{{A}}");
        sys.put(&a, "DhcpIPAddress", RegData::Str("192.168.1.5".into()));
        sys.put(&a, "TcpAckFrequency", RegData::Dword(2));
        let dir = PathBuf::new();
        let c = ctx(&sys, &dir);
        let snap = Special::Nagle.snapshot(&c).unwrap().unwrap();
        Special::Nagle.apply_snapshot(&c, &snap, &mut |_| {}).unwrap();
        assert_eq!(sys.reg_get(&a, "TcpAckFrequency"), Some(RegData::Dword(1)));
        // through JSON, like a record read back from state.json
        let saved: Value = serde_json::from_str(&snap.to_string()).unwrap();
        Special::Nagle.undo(&c, &saved, &mut |_| {}).unwrap();
        assert_eq!(sys.reg_get(&a, "TcpAckFrequency"), Some(RegData::Dword(2)));
        assert_eq!(sys.reg_get(&a, "TCPNoDelay"), None);
    }

    #[test]
    fn boot_timer_undo_puts_back_the_exact_entries() {
        let sys = FakeSystem::default();
        sys.answer("bcdedit /enum", "identifier  {current}\nuseplatformclock        Yes\nuseplatformtick         No");
        let dir = PathBuf::new();
        let c = ctx(&sys, &dir);
        let snap = Special::BcdDynamicTick.snapshot(&c).unwrap().unwrap();
        assert_eq!(snap["useplatformclock"], "Yes");
        assert!(snap["disabledynamictick"].is_null());
        Special::BcdDynamicTick.apply_snapshot(&c, &snap, &mut |_| {}).unwrap();
        assert!(sys.ran("bcdedit /set disabledynamictick yes") && sys.ran("bcdedit /deletevalue useplatformclock"));
        sys.answer("bcdedit /enum", "identifier  {current}\ndisabledynamictick      Yes\nuseplatformtick         No");
        sys.commands.borrow_mut().clear();
        Special::BcdDynamicTick.undo(&c, &snap, &mut |_| {}).unwrap();
        assert!(sys.ran("bcdedit /deletevalue disabledynamictick"));
        assert!(sys.ran("bcdedit /set useplatformclock Yes"));
        assert!(!sys.ran("useplatformtick"), "an unchanged entry was touched");
    }

    #[test]
    fn event_log_undo_restores_each_state_and_reports_failure() {
        let sys = FakeSystem::default();
        sys.answer("wevtutil.exe gl Microsoft-Windows-SleepStudy", "name: x\nenabled: false\n");
        sys.answer("wevtutil.exe gl Microsoft-Windows-Kernel-Processor-Power", "name: y\nenabled: true\n");
        let dir = PathBuf::new();
        let c = ctx(&sys, &dir);
        let s = special_for("perf-sleepstudy").unwrap();
        let snap = s.snapshot(&c).unwrap().unwrap();
        s.undo(&c, &snap, &mut |_| {}).unwrap();
        assert!(sys.ran("wevtutil.exe set-log Microsoft-Windows-SleepStudy/Diagnostic /e:false"));
        assert!(sys.ran("wevtutil.exe set-log Microsoft-Windows-Kernel-Processor-Power/Diagnostic /e:true"));
        sys.fail.borrow_mut().insert("wevtutil.exe set-log".into());
        assert!(s.undo(&c, &snap, &mut |_| {}).is_err());
    }

    #[test]
    fn every_special_has_a_description() {
        assert!(special_for("perf-fth").is_none(), "the FTH list reset cannot be undone and is gone");
        for id in ["perf-ultimate", "perf-timer", "ui-classicmenu", "net-nagle", "debloat-widgets"] {
            assert!(!special_for(id).unwrap().describe().is_empty());
        }
    }
}
