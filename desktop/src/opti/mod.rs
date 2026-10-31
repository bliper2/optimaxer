//! Optimaxer, ported to Rust. Engine and system access live here; the egui pages are in `opti_ui.rs`.

#[cfg(windows)]
pub mod backend;
#[cfg(windows)]
pub mod backup_integrity;
#[cfg(windows)]
pub mod blackbox;
#[cfg(windows)]
pub mod bootlog;
#[cfg(windows)]
pub mod catalog;
#[cfg(windows)]
pub mod cleaner;
#[cfg(windows)]
pub mod detective;
#[cfg(windows)]
pub mod diagnostics;
#[cfg(windows)]
pub mod engine;
#[cfg(windows)]
pub mod gaming;
#[cfg_attr(unix, allow(dead_code))]
pub mod history;
#[cfg(windows)]
pub mod hw;
#[cfg(windows)]
pub mod measurements;
#[cfg(windows)]
pub mod network;
#[cfg(windows)]
pub mod pending_restart;
#[cfg(windows)]
pub mod processes;
#[cfg(windows)]
pub mod profiles;
#[cfg(windows)]
pub mod services;
#[cfg(windows)]
pub mod snapshots;
#[cfg(windows)]
pub mod special;
#[cfg(windows)]
pub mod startup;
#[cfg_attr(unix, allow(dead_code))]
pub mod state;
#[cfg(windows)]
pub mod sysmon;
#[cfg(unix)]
pub use crate::linux::sysmon;
#[cfg(windows)]
pub mod tools;
#[cfg(windows)]
pub mod tweak_facts;

use std::time::{SystemTime, UNIX_EPOCH};

/// UTC civil date and time from a unix timestamp (days-from-civil, no date crate needed).
fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `2026-10-23T00:37:11` for a unix timestamp, in UTC and without a zone suffix.
#[cfg_attr(unix, allow(dead_code))]
pub fn iso_utc(secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

/// `20261023-003711`, for backup file names.
pub fn now_stamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (y, mo, d, h, mi, s) = civil(now.as_secs());
    format!(
        "{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}-{:09}-{}",
        now.subsec_nanos(),
        std::process::id()
    )
}

/// `2026-10-23T00:37:11`, for undo records.
pub fn now_iso() -> String {
    let (y, mo, d, h, mi, s) = civil(now_secs());
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::civil;

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(951_782_400 + 3_661), (2000, 2, 29, 1, 1, 1));
        assert_eq!(civil(1_792_715_831), (2026, 10, 23, 0, 37, 11));
    }
}
