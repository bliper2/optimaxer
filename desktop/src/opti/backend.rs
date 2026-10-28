//! Everything that touches Windows lives behind the `System` trait: registry, services, scheduled tasks and
//! external tools. `WinSystem` is the real thing. Tests use `fake::FakeSystem`, so apply/undo logic is verified
//! without writing a single registry value.

use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::process::Command;
use winreg::enums::*;
use winreg::{RegKey, RegValue, HKEY};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, PartialEq)]
pub enum RegData {
    Dword(u32),
    Qword(u64),
    Str(String),
    Expand(String),
    Multi(Vec<String>),
    Bin(Vec<u8>),
}

impl RegData {
    pub fn type_name(&self) -> &'static str {
        match self {
            RegData::Dword(_) => "DWord",
            RegData::Qword(_) => "QWord",
            RegData::Str(_) => "String",
            RegData::Expand(_) => "ExpandString",
            RegData::Multi(_) => "MultiString",
            RegData::Bin(_) => "Binary",
        }
    }

    /// Same rules as Optimaxer's Compare-RegValue: numbers compare as unsigned 32/64 bit, strings by text.
    pub fn matches(&self, actual: &RegData) -> bool {
        match (self, actual) {
            (RegData::Dword(a), RegData::Dword(b)) => a == b,
            (RegData::Qword(a), RegData::Qword(b)) => a == b,
            (RegData::Dword(a), RegData::Qword(b)) | (RegData::Qword(b), RegData::Dword(a)) => *a as u64 == *b,
            (RegData::Bin(a), RegData::Bin(b)) => a == b,
            (RegData::Multi(a), RegData::Multi(b)) => a == b,
            (RegData::Str(_) | RegData::Expand(_), _) => self.text().eq_ignore_ascii_case(&actual.text()),
            _ => false,
        }
    }

    pub fn text(&self) -> String {
        match self {
            RegData::Dword(v) => v.to_string(),
            RegData::Qword(v) => v.to_string(),
            RegData::Str(s) | RegData::Expand(s) => s.clone(),
            RegData::Multi(v) => v.join(","),
            RegData::Bin(b) => b.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
        }
    }

    /// Short human form for the confirmation window.
    pub fn show(&self) -> String {
        match self {
            RegData::Dword(v) => v.to_string(),
            RegData::Qword(v) => v.to_string(),
            RegData::Str(s) | RegData::Expand(s) => format!("\"{s}\""),
            RegData::Multi(v) => format!("[{}]", v.join("; ")),
            RegData::Bin(b) => format!("{} bytes", b.len()),
        }
    }

    /// Value part of Optimaxer's state.json record (DWord is stored as a signed 32 bit number).
    pub fn to_json(&self) -> Value {
        match self {
            RegData::Dword(v) => json!(*v as i32),
            RegData::Qword(v) => json!(*v as i64),
            RegData::Str(s) | RegData::Expand(s) => json!(s),
            RegData::Multi(v) => json!(v),
            RegData::Bin(b) => json!(b),
        }
    }

    pub fn from_json(value: &Value, ty: &str) -> Option<RegData> {
        Some(match ty {
            "DWord" => RegData::Dword((value.as_i64()? & 0xFFFF_FFFF) as u32),
            "QWord" => RegData::Qword(value.as_i64()? as u64),
            "ExpandString" => RegData::Expand(value.as_str()?.to_string()),
            "MultiString" => RegData::Multi(match value {
                Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                Value::String(s) => vec![s.clone()],
                _ => return None,
            }),
            "Binary" => RegData::Bin(value.as_array()?.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect()),
            _ => RegData::Str(value.as_str()?.to_string()),
        })
    }

    fn to_raw(&self) -> RegValue {
        fn utf16(s: &str) -> Vec<u8> {
            s.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
        }
        let (bytes, vtype) = match self {
            RegData::Dword(v) => (v.to_le_bytes().to_vec(), REG_DWORD),
            RegData::Qword(v) => (v.to_le_bytes().to_vec(), REG_QWORD),
            RegData::Str(s) => (utf16(s), REG_SZ),
            RegData::Expand(s) => (utf16(s), REG_EXPAND_SZ),
            RegData::Multi(v) => {
                let mut b: Vec<u8> = v.iter().flat_map(|s| s.encode_utf16().chain(std::iter::once(0))).flat_map(u16::to_le_bytes).collect();
                b.extend_from_slice(&[0, 0]);
                if v.is_empty() {
                    b.extend_from_slice(&[0, 0]);
                }
                (b, REG_MULTI_SZ)
            }
            RegData::Bin(b) => (b.clone(), REG_BINARY),
        };
        RegValue { bytes, vtype }
    }

    fn from_raw(v: &RegValue) -> RegData {
        fn units(b: &[u8]) -> Vec<u16> {
            b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect()
        }
        let b: &[u8] = &v.bytes;
        match v.vtype {
            REG_DWORD if b.len() >= 4 => RegData::Dword(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            REG_QWORD if b.len() >= 8 => RegData::Qword(u64::from_le_bytes(b[..8].try_into().unwrap_or_default())),
            REG_SZ | REG_EXPAND_SZ => {
                let u = units(b);
                let end = u.iter().position(|c| *c == 0).unwrap_or(u.len());
                let s = String::from_utf16_lossy(&u[..end]);
                if v.vtype == REG_SZ { RegData::Str(s) } else { RegData::Expand(s) }
            }
            REG_MULTI_SZ => {
                let u = units(b);
                RegData::Multi(u.split(|c| *c == 0).filter(|p| !p.is_empty()).map(String::from_utf16_lossy).collect())
            }
            _ => RegData::Bin(b.to_vec()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvcState {
    pub start: u32,
    pub delayed: bool,
}

pub struct CmdOut {
    pub code: i32,
    pub text: String,
}

pub trait System {
    fn reg_get(&self, path: &str, name: &str) -> Option<RegData>;
    fn reg_set(&self, path: &str, name: &str, data: &RegData) -> Result<(), String>;
    fn reg_delete_value(&self, path: &str, name: &str) -> Result<(), String>;
    fn reg_key_exists(&self, path: &str) -> bool;
    fn reg_create_key(&self, path: &str) -> Result<(), String>;
    fn reg_delete_key(&self, path: &str) -> Result<(), String>;
    fn reg_subkeys(&self, path: &str) -> Vec<String>;
    fn reg_value_names(&self, path: &str) -> Vec<String>;
    fn svc_get(&self, name: &str) -> Option<SvcState>;
    fn svc_set(&self, name: &str, start: u32, delayed: bool) -> bool;
    fn task_enabled(&self, full: &str) -> Option<bool>;
    fn task_set(&self, full: &str, enabled: bool) -> Result<(), String>;
    fn run(&self, exe: &str, args: &[&str]) -> CmdOut;
    fn file_exists(&self, path: &str) -> bool;
}

// region real implementation
fn split_path(path: &str) -> Option<(HKEY, String)> {
    const HIVES: [(&str, HKEY); 8] = [
        ("HKLM:", HKEY_LOCAL_MACHINE),
        ("HKCU:", HKEY_CURRENT_USER),
        ("HKU:", HKEY_USERS),
        ("HKCR:", HKEY_CLASSES_ROOT),
        ("HKEY_LOCAL_MACHINE", HKEY_LOCAL_MACHINE),
        ("HKEY_CURRENT_USER", HKEY_CURRENT_USER),
        ("HKEY_CLASSES_ROOT", HKEY_CLASSES_ROOT),
        ("HKEY_USERS", HKEY_USERS),
    ];
    for (prefix, hive) in HIVES {
        if path.len() >= prefix.len() && path[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return Some((hive, path[prefix.len()..].trim_matches('\\').to_string()));
        }
    }
    None
}

fn open_read(path: &str) -> Option<RegKey> {
    let (hive, sub) = split_path(path)?;
    let root = RegKey::predef(hive);
    if sub.is_empty() { Some(root) } else { root.open_subkey_with_flags(&sub, KEY_READ).ok() }
}

pub struct WinSystem;

impl System for WinSystem {
    fn reg_get(&self, path: &str, name: &str) -> Option<RegData> {
        let key = open_read(path)?;
        key.get_raw_value(name).ok().map(|v| RegData::from_raw(&v))
    }

    fn reg_set(&self, path: &str, name: &str, data: &RegData) -> Result<(), String> {
        let (hive, sub) = split_path(path).ok_or_else(|| format!("unsupported registry path {path}"))?;
        let root = RegKey::predef(hive);
        let (key, _) = root.create_subkey_with_flags(&sub, KEY_WRITE).map_err(|e| e.to_string())?;
        key.set_raw_value(name, &data.to_raw()).map_err(|e| e.to_string())
    }

    fn reg_delete_value(&self, path: &str, name: &str) -> Result<(), String> {
        let (hive, sub) = split_path(path).ok_or_else(|| format!("unsupported registry path {path}"))?;
        let key = match RegKey::predef(hive).open_subkey_with_flags(&sub, KEY_WRITE) {
            Ok(key) => key,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        match key.delete_value(name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn reg_key_exists(&self, path: &str) -> bool {
        open_read(path).is_some()
    }

    fn reg_create_key(&self, path: &str) -> Result<(), String> {
        let (hive, sub) = split_path(path).ok_or_else(|| format!("unsupported registry path {path}"))?;
        RegKey::predef(hive).create_subkey_with_flags(&sub, KEY_WRITE).map(|_| ()).map_err(|e| e.to_string())
    }

    fn reg_delete_key(&self, path: &str) -> Result<(), String> {
        let (hive, sub) = split_path(path).ok_or_else(|| format!("unsupported registry path {path}"))?;
        if sub.is_empty() {
            return Err("refusing to delete a registry hive".into());
        }
        match RegKey::predef(hive).delete_subkey_all(&sub) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn reg_subkeys(&self, path: &str) -> Vec<String> {
        open_read(path).map(|k| k.enum_keys().filter_map(Result::ok).collect()).unwrap_or_default()
    }

    fn reg_value_names(&self, path: &str) -> Vec<String> {
        open_read(path).map(|k| k.enum_values().filter_map(Result::ok).map(|(n, _)| n).collect()).unwrap_or_default()
    }

    fn svc_get(&self, name: &str) -> Option<SvcState> {
        let key = format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}");
        let Some(RegData::Dword(start)) = self.reg_get(&key, "Start") else { return None };
        let delayed = matches!(self.reg_get(&key, "DelayedAutostart"), Some(RegData::Dword(1)));
        Some(SvcState { start, delayed })
    }

    fn svc_set(&self, name: &str, start: u32, delayed: bool) -> bool {
        let mode = match start {
            2 if delayed => "delayed-auto",
            2 => "auto",
            3 => "demand",
            4 => "disabled",
            _ => return false,
        };
        let out = self.run("sc.exe", &["config", name, "start=", mode]);
        if out.code != 0 {
            let key = format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}");
            if self.reg_set(&key, "Start", &RegData::Dword(start)).is_err() {
                return false;
            }
            let delayed_result = if start == 2 && delayed { self.reg_set(&key, "DelayedAutostart", &RegData::Dword(1)) } else { self.reg_delete_value(&key, "DelayedAutostart") };
            if delayed_result.is_err() {
                return false;
            }
        }
        if start == 4 {
            let _ = self.run("sc.exe", &["stop", name]);
        }
        true
    }

    fn task_enabled(&self, full: &str) -> Option<bool> {
        let out = self.run("schtasks.exe", &["/Query", "/TN", &format!(r"\{}", full.trim_start_matches('\\')), "/XML"]);
        if out.code != 0 {
            return None;
        }
        Some(!out.text.contains("<Enabled>false</Enabled>"))
    }

    fn task_set(&self, full: &str, enabled: bool) -> Result<(), String> {
        let tn = format!(r"\{}", full.trim_start_matches('\\'));
        let out = self.run("schtasks.exe", &["/Change", "/TN", &tn, if enabled { "/ENABLE" } else { "/DISABLE" }]);
        if out.code == 0 { Ok(()) } else { Err(out.text.trim().lines().last().unwrap_or("schtasks failed").to_string()) }
    }

    fn file_exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn run(&self, exe: &str, args: &[&str]) -> CmdOut {
        match Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).output() {
            Ok(o) => {
                let mut bytes = o.stdout;
                bytes.extend_from_slice(&o.stderr);
                CmdOut { code: o.status.code().unwrap_or(-1), text: decode_output(&bytes) }
            }
            Err(e) => CmdOut { code: -1, text: format!("could not start {exe}: {e}") },
        }
    }
}

/// Console tools answer in the OEM code page or, for sfc, in UTF-16. Keep what is readable, drop the rest.
pub fn decode_output(bytes: &[u8]) -> String {
    let zeros = bytes.iter().filter(|b| **b == 0).count();
    if bytes.len() >= 4 && zeros * 3 >= bytes.len() {
        let u: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        return String::from_utf16_lossy(&u).replace('\0', "");
    }
    String::from_utf8_lossy(bytes).replace('\0', "")
}
// endregion

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};

    /// In-memory registry, services, tasks and command log. Paths compare case-insensitively like Windows.
    #[derive(Default)]
    pub struct FakeSystem {
        pub values: RefCell<BTreeMap<(String, String), RegData>>,
        pub keys: RefCell<BTreeSet<String>>,
        pub services: RefCell<BTreeMap<String, SvcState>>,
        pub tasks: RefCell<BTreeMap<String, bool>>,
        pub commands: RefCell<Vec<String>>,
        pub answers: RefCell<BTreeMap<String, String>>,
        pub deny_writes: RefCell<BTreeSet<String>>,
        /// Command-line prefixes that exit with code 1.
        pub fail: RefCell<BTreeSet<String>>,
    }

    fn k(s: &str) -> String {
        s.to_lowercase().replace('/', "\\").trim_end_matches('\\').to_string()
    }

    impl FakeSystem {
        pub fn put(&self, path: &str, name: &str, data: RegData) {
            self.keys.borrow_mut().insert(k(path));
            self.values.borrow_mut().insert((k(path), k(name)), data);
        }
        pub fn answer(&self, cmd_prefix: &str, text: &str) {
            self.answers.borrow_mut().insert(cmd_prefix.to_string(), text.to_string());
        }
        pub fn ran(&self, needle: &str) -> bool {
            self.commands.borrow().iter().any(|c| c.contains(needle))
        }
    }

    impl System for FakeSystem {
        fn reg_get(&self, path: &str, name: &str) -> Option<RegData> {
            self.values.borrow().get(&(k(path), k(name))).cloned()
        }
        fn reg_set(&self, path: &str, name: &str, data: &RegData) -> Result<(), String> {
            if self.deny_writes.borrow().contains(&k(path)) {
                return Err("access denied".into());
            }
            self.put(path, name, data.clone());
            Ok(())
        }
        fn reg_delete_value(&self, path: &str, name: &str) -> Result<(), String> {
            self.values.borrow_mut().remove(&(k(path), k(name)));
            Ok(())
        }
        fn reg_key_exists(&self, path: &str) -> bool {
            let p = k(path);
            self.keys.borrow().iter().any(|x| *x == p || x.starts_with(&format!("{p}\\")))
        }
        fn reg_create_key(&self, path: &str) -> Result<(), String> {
            self.keys.borrow_mut().insert(k(path));
            Ok(())
        }
        fn reg_delete_key(&self, path: &str) -> Result<(), String> {
            let p = k(path);
            self.keys.borrow_mut().retain(|x| *x != p && !x.starts_with(&format!("{p}\\")));
            self.values.borrow_mut().retain(|(kp, _), _| *kp != p && !kp.starts_with(&format!("{p}\\")));
            Ok(())
        }
        fn reg_subkeys(&self, path: &str) -> Vec<String> {
            let p = format!("{}\\", k(path));
            let mut out: Vec<String> = self.keys.borrow().iter().filter_map(|x| x.strip_prefix(&p)).map(|r| r.split('\\').next().unwrap_or("").to_string()).filter(|s| !s.is_empty()).collect();
            out.dedup();
            out
        }
        fn reg_value_names(&self, path: &str) -> Vec<String> {
            let p = k(path);
            self.values.borrow().keys().filter(|(kp, _)| *kp == p).map(|(_, n)| n.clone()).collect()
        }
        fn svc_get(&self, name: &str) -> Option<SvcState> {
            self.services.borrow().get(&k(name)).copied()
        }
        fn svc_set(&self, name: &str, start: u32, delayed: bool) -> bool {
            match self.services.borrow_mut().get_mut(&k(name)) {
                Some(s) => {
                    *s = SvcState { start, delayed };
                    true
                }
                None => false,
            }
        }
        fn task_enabled(&self, full: &str) -> Option<bool> {
            self.tasks.borrow().get(&k(full)).copied()
        }
        fn task_set(&self, full: &str, enabled: bool) -> Result<(), String> {
            match self.tasks.borrow_mut().get_mut(&k(full)) {
                Some(t) => {
                    *t = enabled;
                    Ok(())
                }
                None => Err("no such task".into()),
            }
        }
        fn file_exists(&self, _path: &str) -> bool {
            true
        }
        fn run(&self, exe: &str, args: &[&str]) -> CmdOut {
            let line = format!("{exe} {}", args.join(" "));
            self.commands.borrow_mut().push(line.clone());
            let text = self.answers.borrow().iter().find(|(p, _)| line.starts_with(p.as_str())).map(|(_, t)| t.clone()).unwrap_or_default();
            let code = if self.fail.borrow().iter().any(|p| line.starts_with(p.as_str())) { 1 } else { 0 };
            CmdOut { code, text }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dword_round_trips_through_json_as_signed() {
        let d = RegData::Dword(0xFFFF_FFFF);
        assert_eq!(d.to_json(), json!(-1));
        assert_eq!(RegData::from_json(&json!(-1), "DWord"), Some(d));
    }

    #[test]
    fn raw_encoding_round_trips() {
        for d in [
            RegData::Dword(7),
            RegData::Qword(1 << 40),
            RegData::Str("hello".into()),
            RegData::Expand("%TEMP%\\x".into()),
            RegData::Multi(vec!["a".into(), "b".into()]),
            RegData::Bin(vec![1, 2, 3]),
        ] {
            assert_eq!(RegData::from_raw(&d.to_raw()), d);
        }
    }

    #[test]
    fn matches_follows_optimaxer_rules() {
        assert!(RegData::Dword(0).matches(&RegData::Dword(0)));
        assert!(RegData::Str("5000".into()).matches(&RegData::Str("5000".into())));
        assert!(RegData::Str("1".into()).matches(&RegData::Dword(1)));
        assert!(!RegData::Dword(1).matches(&RegData::Dword(2)));
        assert!(!RegData::Bin(vec![1]).matches(&RegData::Bin(vec![2])));
    }

    #[test]
    fn hive_prefixes_parse() {
        assert_eq!(split_path(r"HKLM:\SOFTWARE\X").unwrap().1, r"SOFTWARE\X");
        assert_eq!(split_path(r"HKEY_CLASSES_ROOT\*\shellex").unwrap().1, r"*\shellex");
        assert!(split_path(r"C:\x").is_none());
    }

    #[test]
    fn utf16_output_is_decoded() {
        let bytes: Vec<u8> = "Windows Resource Protection".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode_output(&bytes), "Windows Resource Protection");
    }
}
