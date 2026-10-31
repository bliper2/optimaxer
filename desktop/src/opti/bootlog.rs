//! Boot history from Windows' own Diagnostics-Performance log: how long each recent boot took, and which applications,
//! services and drivers Windows itself measured as slowing it. Optimaxer does not time boots or infer per-app impact;
//! it reads what Windows recorded. That log is readable only by administrators.

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const LOG: &str = "Microsoft-Windows-Diagnostics-Performance/Operational";
const MAX_BOOTS: usize = 30;
const MAX_SLOW_EVENTS: usize = 300;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// A slow-item event belongs to the nearest boot event within this many seconds.
const ATTRIBUTION_WINDOW_SECS: i64 = 15 * 60;
/// Boots needed before a trend is stated: the latest plus at least three earlier ones.
const MIN_BOOTS_FOR_TREND: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootRecord {
    /// UTC time the boot event was logged, `2026-10-30T12:00:00`.
    pub time: String,
    pub boot_ms: u64,
    pub main_path_ms: Option<u64>,
    pub post_boot_ms: Option<u64>,
    pub startup_apps: Option<u32>,
    /// Windows' own flag that this boot was slower than its baseline.
    pub windows_flagged_slow: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlowKind {
    Application,
    Driver,
    Service,
}

impl SlowKind {
    pub fn label(self) -> &'static str {
        match self {
            SlowKind::Application => "Application",
            SlowKind::Driver => "Driver",
            SlowKind::Service => "Service",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlowItem {
    pub time: String,
    pub kind: SlowKind,
    /// Service or driver ID, or the program's file name.
    pub name: String,
    pub friendly: String,
    pub total_ms: u64,
    /// How much of the total Windows attributes to this item slowing the boot.
    pub degradation_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    /// Oldest first.
    pub boots: Vec<BootRecord>,
    pub slow: Vec<SlowItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The log needs administrator rights.
    AccessDenied,
    Other(String),
}

fn query(ids: &str, count: usize) -> Result<String, ReadError> {
    let output = Command::new("wevtutil.exe")
        .args(["qe", LOG])
        .arg(format!("/q:*[System[({ids})]]"))
        .args([&format!("/c:{count}"), "/rd:true", "/f:xml"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| ReadError::Other(format!("Could not start the Windows event reader: {e}")))?;
    if !output.status.success() {
        return Err(if output.status.code() == Some(5) {
            ReadError::AccessDenied
        } else {
            ReadError::Other("Windows would not return the boot performance log.".into())
        });
    }
    if output.stdout.len() > MAX_OUTPUT {
        return Err(ReadError::Other(
            "The boot log output was larger than the safety limit.".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn read_history() -> Result<History, ReadError> {
    let boots = parse_boots(&query("EventID=100", MAX_BOOTS)?);
    let slow = parse_slow(&query(
        "EventID=101 or EventID=102 or EventID=103",
        MAX_SLOW_EVENTS,
    )?);
    let mut boots = boots;
    boots.reverse();
    Ok(History { boots, slow })
}

fn data<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    for quote in ['\'', '"'] {
        let open = format!("<Data Name={quote}{name}{quote}>");
        if let Some(start) = block.find(&open) {
            let rest = &block[start + open.len()..];
            return Some(rest[..rest.find("</Data>")?].trim());
        }
    }
    None
}

fn number(block: &str, name: &str) -> Option<u64> {
    data(block, name)?.parse().ok()
}

fn event_id(block: &str) -> Option<u32> {
    let start = block.find("<EventID")?;
    let rest = &block[block[start..].find('>')? + start + 1..];
    rest[..rest.find('<')?].trim().parse().ok()
}

fn event_time(block: &str) -> Option<String> {
    let start = block.find("SystemTime=")? + "SystemTime=".len();
    let rest = &block[start..];
    let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let value = &rest[1..];
    let value = &value[..value.find(quote)?];
    (value.len() >= 19).then(|| value[..19].to_owned())
}

fn events(xml: &str) -> impl Iterator<Item = &str> {
    xml.split("<Event ")
        .skip(1)
        .map(|block| match block.find("</Event>") {
            Some(end) => &block[..end],
            None => block,
        })
}

pub fn parse_boots(xml: &str) -> Vec<BootRecord> {
    events(xml)
        .filter(|block| event_id(block) == Some(100))
        .filter_map(|block| {
            Some(BootRecord {
                time: event_time(block)?,
                boot_ms: number(block, "BootTime")?,
                main_path_ms: number(block, "MainPathBootTime"),
                post_boot_ms: number(block, "BootPostBootTime"),
                startup_apps: number(block, "BootNumStartupApps").map(|n| n as u32),
                windows_flagged_slow: data(block, "BootIsDegradation")
                    .map(|v| v == "true" || v == "1"),
            })
        })
        .collect()
}

pub fn parse_slow(xml: &str) -> Vec<SlowItem> {
    events(xml)
        .filter_map(|block| {
            let kind = match event_id(block)? {
                101 => SlowKind::Application,
                102 => SlowKind::Driver,
                103 => SlowKind::Service,
                _ => return None,
            };
            let friendly = data(block, "FriendlyName").unwrap_or_default().to_owned();
            let name = data(block, "Name")
                .filter(|n| !n.is_empty())
                .map_or_else(|| friendly.clone(), str::to_owned);
            if name.is_empty() {
                return None;
            }
            Some(SlowItem {
                time: event_time(block)?,
                kind,
                name,
                friendly,
                total_ms: number(block, "TotalTime")?,
                degradation_ms: number(block, "DegradationTime").unwrap_or(0),
            })
        })
        .collect()
}

/// Seconds since 1970 for a `YYYY-MM-DDTHH:MM:SS` UTC time, or `None` when the text is not in that form.
pub fn epoch_seconds(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let part = |range: std::ops::Range<usize>| iso.get(range)?.parse::<i64>().ok();
    let (y, m, d) = (part(0..4)?, part(5..7)?, part(8..10)?);
    let (h, mi, s) = (part(11..13)?, part(14..16)?, part(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + h * 3_600 + mi * 60 + s)
}

/// Slow-item events Windows logged for one boot: those closest to it, within a quarter of an hour.
pub fn slow_for_boot(history: &History, boot_index: usize) -> Vec<&SlowItem> {
    let Some(boot) = history.boots.get(boot_index) else {
        return Vec::new();
    };
    let Some(boot_at) = epoch_seconds(&boot.time) else {
        return Vec::new();
    };
    let boot_times: Vec<Option<i64>> = history
        .boots
        .iter()
        .map(|b| epoch_seconds(&b.time))
        .collect();
    let mut items: Vec<&SlowItem> = history
        .slow
        .iter()
        .filter(|item| {
            let Some(at) = epoch_seconds(&item.time) else {
                return false;
            };
            let mine = (at - boot_at).abs();
            mine <= ATTRIBUTION_WINDOW_SECS
                && boot_times
                    .iter()
                    .flatten()
                    .all(|other| (at - other).abs() >= mine)
        })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.degradation_ms));
    items
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Slower,
    Faster,
    /// Inside the spread the earlier boots already had.
    Similar,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trend {
    pub latest_ms: u64,
    pub median_before_ms: u64,
    pub verdict: Verdict,
    pub earlier_boots: usize,
}

fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2
    } else {
        values[mid]
    }
}

/// Compares the newest boot with the median of the earlier ones. A change counts only if it exceeds both a quarter of
/// that median and half the spread the earlier boots already showed, so ordinary boot-to-boot variation is not
/// reported as a trend.
pub fn latest_trend(boots: &[BootRecord]) -> Option<Trend> {
    if boots.len() < MIN_BOOTS_FOR_TREND {
        return None;
    }
    let (latest, earlier) = boots.split_last()?;
    let mut times: Vec<u64> = earlier.iter().map(|b| b.boot_ms).collect();
    let spread = times.iter().max()? - times.iter().min()?;
    let baseline = median(&mut times);
    let delta = latest.boot_ms.abs_diff(baseline);
    let margin = (baseline / 4).max(spread / 2);
    let verdict = if delta <= margin {
        Verdict::Similar
    } else if latest.boot_ms > baseline {
        Verdict::Slower
    } else {
        Verdict::Faster
    };
    Some(Trend {
        latest_ms: latest.boot_ms,
        median_before_ms: baseline,
        verdict,
        earlier_boots: earlier.len(),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recurring {
    pub name: String,
    pub friendly: String,
    pub kind: SlowKind,
    pub boots: usize,
    pub average_degradation_ms: u64,
    pub worst_degradation_ms: u64,
}

/// Items Windows flagged on at least two different boots, worst average first. One appearance is not a pattern.
pub fn recurring(history: &History) -> Vec<Recurring> {
    let mut groups: HashMap<(SlowKind, String), Vec<&SlowItem>> = HashMap::new();
    for item in &history.slow {
        groups
            .entry((item.kind, item.name.to_lowercase()))
            .or_default()
            .push(item);
    }
    let mut out: Vec<Recurring> = groups
        .into_values()
        .filter_map(|items| {
            let mut days: Vec<&str> = items.iter().map(|i| i.time.as_str()).collect();
            days.sort_unstable();
            days.dedup();
            if days.len() < 2 {
                return None;
            }
            let total: u64 = items.iter().map(|i| i.degradation_ms).sum();
            Some(Recurring {
                name: items[0].name.clone(),
                friendly: items[0].friendly.clone(),
                kind: items[0].kind,
                boots: items.len(),
                average_degradation_ms: total / items.len() as u64,
                worst_degradation_ms: items.iter().map(|i| i.degradation_ms).max().unwrap_or(0),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.average_degradation_ms
            .cmp(&a.average_degradation_ms)
            .then(a.name.cmp(&b.name))
    });
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeComparison {
    pub before_ms: Vec<u64>,
    pub after_ms: Vec<u64>,
}

/// Boot durations recorded before and after a change at `change_iso`. Boots logged within a minute of the change
/// are left out of both sides, since they may straddle it.
pub fn around_change(boots: &[BootRecord], change_iso: &str) -> Option<ChangeComparison> {
    let change = epoch_seconds(change_iso)?;
    let (mut before, mut after) = (Vec::new(), Vec::new());
    for boot in boots {
        let at = epoch_seconds(&boot.time)?;
        if at < change - 60 {
            before.push(boot.boot_ms);
        } else if at > change + 60 {
            after.push(boot.boot_ms);
        }
    }
    (!before.is_empty() && !after.is_empty()).then_some(ChangeComparison {
        before_ms: before,
        after_ms: after,
    })
}

pub fn median_ms(values: &[u64]) -> Option<u64> {
    (!values.is_empty()).then(|| median(&mut values.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot(time: &str, ms: u64) -> BootRecord {
        BootRecord {
            time: time.into(),
            boot_ms: ms,
            main_path_ms: None,
            post_boot_ms: None,
            startup_apps: None,
            windows_flagged_slow: None,
        }
    }

    fn slow(time: &str, kind: SlowKind, name: &str, degradation_ms: u64) -> SlowItem {
        SlowItem {
            time: time.into(),
            kind,
            name: name.into(),
            friendly: name.into(),
            total_ms: degradation_ms + 1_000,
            degradation_ms,
        }
    }

    #[test]
    fn boot_events_are_parsed_with_optional_fields() {
        let xml = "<Event xmlns='x'><System><EventID>100</EventID><TimeCreated SystemTime='2026-10-30T12:00:01.123Z'/></System>\
            <EventData><Data Name='BootTime'>71234</Data><Data Name='MainPathBootTime'>40000</Data>\
            <Data Name='BootPostBootTime'>31234</Data><Data Name='BootNumStartupApps'>9</Data>\
            <Data Name='BootIsDegradation'>true</Data></EventData></Event>\
            <Event xmlns='x'><System><EventID>100</EventID><TimeCreated SystemTime='2026-10-29T08:00:00.000Z'/></System>\
            <EventData><Data Name='BootTime'>50000</Data></EventData></Event>\
            <Event xmlns='x'><System><EventID>100</EventID><TimeCreated SystemTime='2026-10-28T08:00:00.000Z'/></System>\
            <EventData><Data Name='Other'>1</Data></EventData></Event>";
        let boots = parse_boots(xml);
        assert_eq!(boots.len(), 2, "an event without BootTime is skipped");
        assert_eq!(boots[0].time, "2026-10-30T12:00:01");
        assert_eq!(boots[0].boot_ms, 71_234);
        assert_eq!(boots[0].startup_apps, Some(9));
        assert_eq!(boots[0].windows_flagged_slow, Some(true));
        assert_eq!(boots[1].main_path_ms, None);
    }

    #[test]
    fn slow_items_are_typed_by_event_id() {
        let xml = "<Event xmlns='x'><System><EventID>101</EventID><TimeCreated SystemTime='2026-10-30T12:00:05Z'/></System>\
            <EventData><Data Name='Name'>chat.exe</Data><Data Name='FriendlyName'>Chat</Data>\
            <Data Name='TotalTime'>9000</Data><Data Name='DegradationTime'>4000</Data></EventData></Event>\
            <Event xmlns='x'><System><EventID>103</EventID><TimeCreated SystemTime='2026-10-30T12:00:06Z'/></System>\
            <EventData><Data Name='Name'>Spooler</Data><Data Name='TotalTime'>3000</Data></EventData></Event>\
            <Event xmlns='x'><System><EventID>200</EventID><TimeCreated SystemTime='2026-10-30T12:00:07Z'/></System>\
            <EventData><Data Name='Name'>ignored</Data><Data Name='TotalTime'>1</Data></EventData></Event>";
        let items = parse_slow(xml);
        assert_eq!(items.len(), 2);
        assert_eq!(
            (items[0].kind, items[0].degradation_ms),
            (SlowKind::Application, 4_000)
        );
        assert_eq!(
            (items[1].kind, items[1].degradation_ms),
            (SlowKind::Service, 0)
        );
    }

    #[test]
    fn garbage_and_empty_input_parse_to_nothing() {
        assert!(parse_boots("").is_empty());
        assert!(parse_boots("not xml <Event ").is_empty());
        assert!(parse_slow("<Event><EventID>101</EventID>").is_empty());
    }

    #[test]
    fn epoch_seconds_matches_known_dates_and_rejects_bad_text() {
        assert_eq!(epoch_seconds("1970-01-01T00:00:00"), Some(0));
        assert_eq!(
            epoch_seconds("2000-02-29T01:01:01"),
            Some(951_782_400 + 3_661)
        );
        assert_eq!(epoch_seconds("2026-10-23T00:37:11"), Some(1_792_715_831));
        assert_eq!(epoch_seconds("tomorrow"), None);
        assert_eq!(epoch_seconds("2026-13-01T00:00:00"), None);
    }

    #[test]
    fn trend_needs_enough_boots_and_ignores_normal_variation() {
        let few = [
            boot("2026-10-01T00:00:00", 30_000),
            boot("2026-10-02T00:00:00", 31_000),
        ];
        assert!(latest_trend(&few).is_none());
        let steady: Vec<BootRecord> = [30_000, 34_000, 29_000, 33_000, 38_000]
            .iter()
            .enumerate()
            .map(|(i, ms)| boot(&format!("2026-10-0{}T00:00:00", i + 1), *ms))
            .collect();
        assert_eq!(latest_trend(&steady).unwrap().verdict, Verdict::Similar);
        let mut slower = steady.clone();
        slower.last_mut().unwrap().boot_ms = 80_000;
        let trend = latest_trend(&slower).unwrap();
        assert_eq!(trend.verdict, Verdict::Slower);
        assert_eq!((trend.median_before_ms, trend.earlier_boots), (31_500, 4));
        let mut faster = steady;
        faster.last_mut().unwrap().boot_ms = 10_000;
        assert_eq!(latest_trend(&faster).unwrap().verdict, Verdict::Faster);
    }

    #[test]
    fn a_noisy_history_does_not_call_a_modest_change_a_trend() {
        let boots: Vec<BootRecord> = [20_000, 60_000, 25_000, 55_000, 58_000]
            .iter()
            .enumerate()
            .map(|(i, ms)| boot(&format!("2026-10-0{}T00:00:00", i + 1), *ms))
            .collect();
        assert_eq!(latest_trend(&boots).unwrap().verdict, Verdict::Similar);
    }

    #[test]
    fn slow_items_attach_to_the_nearest_boot_only() {
        let history = History {
            boots: vec![
                boot("2026-10-29T08:00:00", 50_000),
                boot("2026-10-30T12:00:00", 70_000),
            ],
            slow: vec![
                slow("2026-10-30T12:00:30", SlowKind::Application, "a.exe", 5_000),
                slow("2026-10-29T08:00:10", SlowKind::Application, "b.exe", 9_000),
                slow("2026-10-30T12:01:00", SlowKind::Service, "svc", 7_000),
                slow("2026-10-20T00:00:00", SlowKind::Driver, "old", 1_000),
            ],
        };
        let latest = slow_for_boot(&history, 1);
        assert_eq!(
            latest.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["svc", "a.exe"]
        );
        let first = slow_for_boot(&history, 0);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "b.exe");
        assert!(slow_for_boot(&history, 9).is_empty());
    }

    #[test]
    fn only_items_flagged_on_more_than_one_boot_are_recurring() {
        let history = History {
            boots: vec![],
            slow: vec![
                slow(
                    "2026-10-01T08:00:00",
                    SlowKind::Application,
                    "Chat.exe",
                    4_000,
                ),
                slow(
                    "2026-10-02T08:00:00",
                    SlowKind::Application,
                    "chat.exe",
                    6_000,
                ),
                slow("2026-10-02T08:00:01", SlowKind::Service, "once", 99_000),
            ],
        };
        let found = recurring(&history);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].boots, 2);
        assert_eq!(
            (
                found[0].average_degradation_ms,
                found[0].worst_degradation_ms
            ),
            (5_000, 6_000)
        );
    }

    #[test]
    fn change_comparison_splits_boots_and_needs_both_sides() {
        let boots = vec![
            boot("2026-10-01T08:00:00", 60_000),
            boot("2026-10-02T08:00:00", 62_000),
            boot("2026-10-03T08:00:20", 61_000),
            boot("2026-10-04T08:00:00", 40_000),
        ];
        let comparison = around_change(&boots, "2026-10-03T08:00:00").unwrap();
        assert_eq!(comparison.before_ms, [60_000, 62_000]);
        assert_eq!(comparison.after_ms, [40_000]);
        assert!(around_change(&boots, "2026-09-01T00:00:00").is_none());
        assert_eq!(median_ms(&comparison.before_ms), Some(61_000));
        assert_eq!(median_ms(&[]), None);
    }
}
