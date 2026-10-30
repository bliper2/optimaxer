//! Services page: lists Windows services with Optimaxer's recommended start types, and changes them with a JSON
//! backup written first. Core services (RPC, Defender, networking, update...) are protected and never offered.

use super::backend::System;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, GetLastError};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW, OpenSCManagerW,
    SC_ENUM_PROCESS_INFO, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_RUNNING, SERVICE_STATE_ALL,
    SERVICE_WIN32,
};
use windows_sys::Win32::UI::Shell::SHLoadIndirectString;

const DATA: &str = include_str!("../../assets/optimaxer/data.json");

#[derive(Clone, Debug, Deserialize)]
pub struct Rec {
    pub name: String,
    pub start: u32,
    pub desc: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DnsProvider {
    pub name: String,
    pub desc: String,
    pub v4: Vec<String>,
    pub v6: Vec<String>,
    pub doh: String,
}

#[derive(Debug, Deserialize)]
pub struct Data {
    pub service_recs: Vec<Rec>,
    pub protected: Vec<String>,
    pub dns: Vec<DnsProvider>,
}

impl Data {
    pub fn load() -> Data {
        serde_json::from_str(DATA.trim_start_matches('\u{feff}')).unwrap_or(Data {
            service_recs: vec![],
            protected: vec![],
            dns: vec![],
        })
    }
}

#[derive(Clone, Debug)]
pub struct SvcRow {
    pub name: String,
    pub display: String,
    pub desc: String,
    pub start: u32,
    pub delayed: bool,
    pub running: bool,
    /// Recommended start type (3 = Manual, 4 = Disabled) and why, when Optimaxer has an opinion.
    pub rec: Option<(u32, String)>,
}

impl SvcRow {
    pub fn mode(&self) -> String {
        match self.start {
            2 if self.delayed => "Auto (delayed)".into(),
            s => super::engine::start_label(s).into(),
        }
    }
}

fn pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the service manager returns NUL-terminated strings inside the buffer we own.
    let len = (0..).take_while(|i| unsafe { *p.add(*i) } != 0).count();
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

/// Per-user service instances are named like `CDPUserSvc_3f2a1`.
pub fn is_per_user_instance(name: &str) -> bool {
    name.rsplit_once('_').is_some_and(|(_, suffix)| {
        suffix.len() >= 4
            && suffix
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

fn resolve(text: &str) -> String {
    if !text.starts_with('@') {
        return text.to_string();
    }
    let src: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let mut out = [0u16; 512];
    // SAFETY: both buffers are valid for the lengths passed.
    let hr = unsafe {
        SHLoadIndirectString(
            src.as_ptr(),
            out.as_mut_ptr(),
            out.len() as u32,
            std::ptr::null_mut(),
        )
    };
    if hr < 0 {
        String::new()
    } else {
        pwstr(out.as_ptr())
    }
}

/// (name, display name, running) for every Win32 service, in one service-manager call.
fn enumerate() -> Vec<(String, String, bool)> {
    // SAFETY: standard two-call buffer pattern; the buffer is u64-aligned for ENUM_SERVICE_STATUS_PROCESSW.
    unsafe {
        let scm = OpenSCManagerW(
            std::ptr::null(),
            std::ptr::null(),
            SC_MANAGER_ENUMERATE_SERVICE,
        );
        if scm.is_null() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut buf: Vec<u64> = vec![0; 1 << 14]; // 128 KB: the service manager rejects buffers over 256 KB
        let mut resume = 0u32;
        loop {
            let (mut needed, mut count) = (0u32, 0u32);
            let ok = EnumServicesStatusExW(
                scm,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut needed,
                &mut count,
                &mut resume,
                std::ptr::null(),
            );
            let more = ok == 0 && GetLastError() == ERROR_MORE_DATA;
            if ok == 0 && !more {
                break;
            }
            let items = std::slice::from_raw_parts(
                buf.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>(),
                count as usize,
            );
            for s in items {
                out.push((
                    pwstr(s.lpServiceName),
                    pwstr(s.lpDisplayName),
                    s.ServiceStatusProcess.dwCurrentState == SERVICE_RUNNING,
                ));
            }
            if !more {
                break;
            }
        }
        CloseServiceHandle(scm);
        out
    }
}

pub fn list(sys: &dyn System, data: &Data) -> Vec<SvcRow> {
    let recs: HashMap<String, &Rec> = data
        .service_recs
        .iter()
        .map(|r| (r.name.to_lowercase(), r))
        .collect();
    let mut rows: Vec<SvcRow> = enumerate()
        .into_iter()
        .filter(|(n, _, _)| !is_per_user_instance(n) && !is_protected(data, n))
        .filter_map(|(name, display, running)| {
            let snap = sys.svc_get(&name)?;
            let desc = match sys.reg_get(
                &format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}"),
                "Description",
            ) {
                Some(super::backend::RegData::Str(s) | super::backend::RegData::Expand(s)) => {
                    resolve(&s)
                }
                _ => String::new(),
            };
            let rec = recs
                .get(&name.to_lowercase())
                .map(|r| (r.start, r.desc.clone()));
            Some(SvcRow {
                name,
                display,
                desc,
                start: snap.start,
                delayed: snap.delayed,
                running,
                rec,
            })
        })
        .collect();
    rows.sort_by_key(|r| {
        (
            match r.rec.as_ref().map(|x| x.0) {
                Some(3) => 0,
                Some(4) => 1,
                _ => 2,
            },
            r.display.to_lowercase(),
        )
    });
    rows
}

pub fn is_protected(data: &Data, name: &str) -> bool {
    data.protected.iter().any(|p| p.eq_ignore_ascii_case(name))
}

/// Writes the current start types to `services-<stamp>.json` (same shape Optimaxer used), then applies the change.
/// Returns how many services were changed.
pub fn set_start(
    sys: &dyn System,
    data: &Data,
    names: &[String],
    start: u32,
    backups: &Path,
    log: &mut dyn FnMut(&str),
) -> usize {
    let snaps: Vec<_> = names
        .iter()
        .filter(|n| !is_protected(data, n))
        .filter_map(|n| sys.svc_get(n).map(|s| (n.clone(), s)))
        .collect();
    if snaps.is_empty() {
        return 0;
    }
    let backup: Vec<_> = snaps.iter().map(|(n, s)| serde_json::json!({ "Name": n, "Start": s.start, "Delayed": s.delayed as u32 })).collect();
    let file = backups.join(format!("services-{}.json", super::now_stamp()));
    match std::fs::write(
        &file,
        serde_json::to_vec_pretty(&backup).unwrap_or_default(),
    ) {
        Ok(()) => log(&format!("Backup saved: {}", file.display())),
        Err(e) => {
            log(&format!("Backup failed ({e}); nothing was changed"));
            return 0;
        }
    }
    let mut done = 0;
    for (n, _) in &snaps {
        if sys.svc_set(n, start, false) {
            done += 1;
            log(&format!("  {n} updated"));
        } else {
            log(&format!("  WARN {n}: refused (protected by Windows)"));
        }
    }
    done
}

pub fn restore_latest(sys: &dyn System, backups: &Path, log: &mut dyn FnMut(&str)) -> bool {
    let mut files: Vec<_> = std::fs::read_dir(backups)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("services-"))
        })
        .collect();
    files.sort();
    let Some(file) = files.pop() else {
        log("No service backup found.");
        return false;
    };
    let bytes = std::fs::read(&file).unwrap_or_default();
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes))
        .into_owned();
    let Ok(list) = serde_json::from_str::<Vec<super::state::SvcRec>>(&text) else {
        log("The service backup is unreadable.");
        return false;
    };
    let data = Data::load();
    if list.is_empty()
        || list.iter().any(|b| {
            b.name.is_empty() || !(2..=4).contains(&b.start) || is_protected(&data, &b.name)
        })
    {
        log(
            "The service backup contains no restorable services or invalid/protected entries; nothing was changed.",
        );
        return false;
    }
    let mut succeeded = true;
    for b in list {
        if sys.svc_set(&b.name, b.start as u32, b.delayed != 0) {
            log(&format!("  restored {}", b.name));
        } else {
            succeeded = false;
            log(&format!("  WARN could not restore {}", b.name));
        }
    }
    log(&format!(
        "{} from {}",
        if succeeded {
            "Restored"
        } else {
            "Restore incomplete"
        },
        file.display()
    ));
    succeeded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::SvcState;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    #[ignore = "reads the live service manager"]
    fn live_enumeration_finds_services() {
        let all = enumerate();
        println!("enumerate: {}", all.len());
        let rows = list(&crate::opti::backend::WinSystem, &Data::load());
        println!("rows: {}", rows.len());
        assert!(all.len() > 100 && rows.len() > 50);
    }

    #[test]
    fn data_tables_load() {
        let d = Data::load();
        assert!(d.service_recs.len() > 80 && d.protected.len() > 40 && d.dns.len() == 6);
        assert!(d.service_recs.iter().all(|r| r.start == 3 || r.start == 4));
        assert!(
            d.dns
                .iter()
                .any(|p| p.name == "Cloudflare" && p.v4[0] == "1.1.1.1")
        );
    }

    #[test]
    fn per_user_instances_are_recognized() {
        assert!(is_per_user_instance("CDPUserSvc_3f2a1"));
        assert!(!is_per_user_instance("wuauserv"));
        assert!(!is_per_user_instance("Some_Name"));
    }

    #[test]
    fn protected_services_are_never_changed() {
        let sys = FakeSystem::default();
        sys.services.borrow_mut().insert(
            "rpcss".into(),
            SvcState {
                start: 2,
                delayed: false,
            },
        );
        sys.services.borrow_mut().insert(
            "fax".into(),
            SvcState {
                start: 2,
                delayed: false,
            },
        );
        let data = Data::load();
        let dir = std::env::temp_dir().join(format!("opti-svc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let n = set_start(
            &sys,
            &data,
            &["RpcSs".into(), "Fax".into()],
            4,
            &dir,
            &mut |_| {},
        );
        assert_eq!(n, 1);
        assert_eq!(sys.svc_get("RpcSs").unwrap().start, 2);
        assert_eq!(sys.svc_get("Fax").unwrap().start, 4);
    }

    #[test]
    fn backup_is_written_before_the_change_and_restores() {
        let sys = FakeSystem::default();
        sys.services.borrow_mut().insert(
            "fax".into(),
            SvcState {
                start: 3,
                delayed: false,
            },
        );
        let data = Data::load();
        let dir = std::env::temp_dir().join(format!("opti-svc2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_start(&sys, &data, &["Fax".into()], 4, &dir, &mut |_| {});
        assert!(std::fs::read_dir(&dir).unwrap().count() == 1);
        assert!(restore_latest(&sys, &dir, &mut |_| {}));
        assert_eq!(sys.svc_get("Fax").unwrap().start, 3);
    }

    #[test]
    fn unwritable_backup_folder_blocks_the_change() {
        let sys = FakeSystem::default();
        sys.services.borrow_mut().insert(
            "fax".into(),
            SvcState {
                start: 3,
                delayed: false,
            },
        );
        let data = Data::load();
        let n = set_start(
            &sys,
            &data,
            &["Fax".into()],
            4,
            Path::new(r"Z:\definitely\missing"),
            &mut |_| {},
        );
        assert_eq!(n, 0);
        assert_eq!(sys.svc_get("Fax").unwrap().start, 3);
    }

    #[test]
    fn restore_reports_refused_and_invalid_entries() {
        let sys = FakeSystem::default();
        let dir = std::env::temp_dir().join(format!("opti-svc-invalid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("services-test.json");
        std::fs::write(&file, r#"[{"Name":"Missing","Start":3,"Delayed":0}]"#).unwrap();
        let mut lines = Vec::new();
        assert!(!restore_latest(&sys, &dir, &mut |l| lines.push(l.to_string())));
        assert!(!lines.iter().any(|l| l.contains("restored Missing")));
        sys.services.borrow_mut().insert(
            "rpcss".into(),
            SvcState {
                start: 2,
                delayed: false,
            },
        );
        std::fs::write(&file, r#"[{"Name":"RpcSs","Start":4,"Delayed":0}]"#).unwrap();
        assert!(!restore_latest(&sys, &dir, &mut |_| {}));
        assert_eq!(sys.svc_get("RpcSs").unwrap().start, 2);
        std::fs::write(&file, r#"[{"Start":3}]"#).unwrap();
        assert!(!restore_latest(&sys, &dir, &mut |_| {}));
    }
}
