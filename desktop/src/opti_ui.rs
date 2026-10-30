//! Optimaxer section of the window: Tweaks, Services, Startup, Cleaner, Network and Tools pages.
//! Everything that reads or changes Windows runs on a worker thread and reports back through a channel, so the UI
//! never blocks. Every change goes through a confirmation window that lists exactly what will happen.

use crate::kit;
use crate::model::{format_size, AppItem};
use crate::net::{self, Settings};
use crate::opti::backend::{RegData, System, WinSystem};
use crate::opti::backup_integrity::{self, Check as BackupCheck, Status as BackupStatus};
use crate::opti::catalog::{Catalog, Tweak};
use crate::opti::cleaner::{self, Target};
use crate::opti::diagnostics::{self, Check as DiagnosticCheck, Result as DiagnosticResult};
use crate::opti::engine::{Change, Engine};
use crate::opti::hw::{self, Traits};
use crate::opti::history::{
    self, Action as HistoryAction, Change as HistoryChange, Entry as HistoryEntry,
    OperationEntry, OperationStatus,
};
use crate::opti::gaming::{self, GameProfile, GraphicsPreference, ProfileState};
use crate::opti::network;
use crate::opti::processes::{self, CpuTimes, ProcessInfo};
use crate::opti::profiles::{self, Profile};
use crate::opti::services::{self, Data, DnsProvider, SvcRow};
use crate::opti::snapshots::{self, SystemSnapshot};
use crate::opti::startup;
use crate::opti::state::{backup_dir, State};
use crate::opti::tools::{self, Tool};
use crate::theme::{self, mix, semibold, tokens, Tone, S1, S2, S3, S4, S5, T_BODY, T_CAPTION, T_HEADING, T_SMALL, T_TITLE};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOG_LINES: usize = 600;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Overview,
    Performance,
    Diagnostics,
    Snapshots,
    Processes,
    Gaming,
    Activity,
    Tweaks,
    Services,
    Startup,
    Cleaner,
    Network,
    Tools,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageGroup {
    Workspace,
    Optimization,
    System,
}

impl PageGroup {
    const ALL: [Self; 3] = [Self::Workspace, Self::Optimization, Self::System];

    fn label(self) -> &'static str {
        match self {
            Self::Workspace => "Workspace",
            Self::Optimization => "Optimization",
            Self::System => "System",
        }
    }
}

impl Page {
    const ALL: [Page; 13] = [
        Page::Overview,
        Page::Performance,
        Page::Diagnostics,
        Page::Snapshots,
        Page::Processes,
        Page::Gaming,
        Page::Activity,
        Page::Tweaks,
        Page::Services,
        Page::Startup,
        Page::Cleaner,
        Page::Network,
        Page::Tools,
    ];
    /// Segoe Fluent Icons glyph shown in the sidebar.
    fn icon(self) -> char {
        match self {
            Page::Overview => '\u{E80F}',
            Page::Performance => '\u{E9D2}',
            Page::Diagnostics => '\u{E72E}',
            Page::Snapshots => '\u{E8D4}',
            Page::Processes => '\u{E9D9}',
            Page::Gaming => '\u{E7FC}',
            Page::Activity => '\u{E81C}',
            Page::Tweaks => '\u{E9E9}',
            Page::Services => '\u{E9F5}',
            Page::Startup => '\u{E7E8}',
            Page::Cleaner => '\u{E74D}',
            Page::Network => '\u{E774}',
            Page::Tools => '\u{E90F}',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Performance => "Performance",
            Page::Diagnostics => "Diagnostics",
            Page::Snapshots => "System Snapshot",
            Page::Processes => "Processes",
            Page::Gaming => "Gaming",
            Page::Activity => "Activity",
            Page::Tweaks => "Tweaks",
            Page::Services => "Services",
            Page::Startup => "Startup",
            Page::Cleaner => "Cleaner",
            Page::Network => "Network",
            Page::Tools => "Tools",
        }
    }

    fn group(self) -> PageGroup {
        match self {
            Self::Overview | Self::Performance | Self::Processes | Self::Activity => PageGroup::Workspace,
            Self::Gaming | Self::Tweaks | Self::Services | Self::Startup | Self::Cleaner | Self::Network => PageGroup::Optimization,
            Self::Diagnostics | Self::Snapshots | Self::Tools => PageGroup::System,
        }
    }
}

/// Things the Optimaxer section asks the rest of the app to do.
pub enum OptiEvent {
    OpenStoreCategory(String),
    OpenApp(String),
    Toast(String),
    Close,
}

#[derive(Clone)]
enum Action {
    ApplyTweaks(Vec<String>),
    UndoTweaks(Vec<String>),
    UndoAll,
    /// Second step after a failed restore point: the user confirmed to go on without one.
    ApplyWithoutRestore(Vec<String>),
    SetServices { names: Vec<String>, start: u32 },
    RestoreServices,
    SetStartup { items: Vec<startup::Item>, enable: bool },
    Clean(Vec<&'static str>),
    SetDns(DnsProvider),
    ResetNetwork,
    RunTool(Tool),
    RunInstaller(PathBuf),
    ApplyGameGraphics { executable: String, expected_current: Option<RegData> },
    UndoGameGraphics { executable: String },
}

struct Line {
    text: String,
    warn: bool,
    head: bool,
}

struct Pending {
    title: String,
    intro: String,
    lines: Vec<Line>,
    action: Action,
    /// Offer "create a restore point first" for this action.
    restore_option: bool,
    ok_label: String,
}

enum Msg {
    Log(String),
    Toast(String),
    Idle,
    Traits(Traits),
    Status(HashMap<String, bool>),
    Plan(Pending),
    Services(Vec<SvcRow>),
    Startup(startup::Listing),
    Scan(Vec<(&'static str, Option<u64>)>),
    CleanPreview(&'static str, Result<cleaner::Preview, String>),
    Adapters(Vec<String>),
    Bench(Vec<(String, network::DnsBenchmark)>),
    History(Result<Vec<HistoryEntry>, String>),
    OperationHistory(Result<Vec<OperationEntry>, String>),
    GameProfiles(Result<Vec<GameProfile>, String>),
    GameStates(HashMap<String, Result<ProfileState, String>>),
    GameLibraries(gaming::LibraryScan),
    Measurements(Result<Vec<crate::opti::measurements::Capture>, String>),
    Diagnostic(DiagnosticCheck, DiagnosticResult),
    Snapshots(Result<Vec<SystemSnapshot>, String>),
    Processes(Result<(Vec<ProcessInfo>, CpuTimes), String>),
    RestartRequired,
    BackupIntegrity(Vec<BackupCheck>),
}

#[derive(Clone)]
struct Worker {
    tx: Sender<Msg>,
    ctx: egui::Context,
    operation_details: Option<Arc<Mutex<Vec<String>>>>,
    operation_status: Option<Arc<AtomicU8>>,
}

fn operation_log_severity(line: &str) -> u8 {
    let line = line.trim_start();
    if line.starts_with("ERROR") { 2 } else if line.starts_with("WARN") { 1 } else { 0 }
}

fn catch_worker_panic(job: impl FnOnce()) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).map_err(|payload| {
        payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic payload")
            .to_string()
    })
}

impl Worker {
    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
    fn log(&self, s: &str) {
        if let Some(status) = &self.operation_status {
            status.fetch_max(operation_log_severity(s), Ordering::Relaxed);
        }
        if let Some(details) = &self.operation_details {
            match details.lock() {
                Ok(mut details) => history::capture_operation_detail(&mut details, s),
                Err(_) => self.send(Msg::Log(
                    "WARN Operation output could not be captured because its log buffer was unavailable.".into(),
                )),
            }
        }
        self.send(Msg::Log(s.to_string()));
    }
}

fn refresh_game_profile_messages(worker: &Worker, store_path: &std::path::Path) {
    match gaming::load_store(store_path) {
        Ok(profiles) => {
            let states = profiles.iter().map(|profile| {
                (profile.executable.clone(), gaming::state(&WinSystem, profile))
            }).collect();
            worker.send(Msg::GameProfiles(Ok(profiles)));
            worker.send(Msg::GameStates(states));
        }
        Err(error) => worker.send(Msg::GameProfiles(Err(error))),
    }
}

pub struct OptiUi {
    page: Page,
    cat: Arc<Catalog>,
    data: Arc<Data>,
    traits: Option<Traits>,
    status: HashMap<String, bool>,
    status_ready: bool,
    selected: HashSet<String>,
    tw_search: String,
    /// Risk filter on the Tweaks page: 0 all, 1 Safe, 2 Moderate, 3 Advanced.
    tw_risk: usize,
    /// Group filter on the Tweaks page, an index into `TWEAK_GROUPS`.
    tw_group: usize,
    /// Tweaks page filters: state (all/applied/not applied), view (all/recommended/favorites), compatibility (all/fits/does not fit).
    tw_state: usize,
    tw_view: usize,
    tw_compatibility: usize,
    /// Tweaks page order: category, name, risk, status.
    tw_sort: usize,
    /// Tweaks whose technical details are expanded.
    tw_open: HashSet<String>,
    /// Why Smart optimize left a tweak unticked (risk or the hardware fact). Cleared by the other presets.
    tw_skip: std::collections::BTreeMap<String, String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: Option<String>,
    log: Vec<String>,
    log_expanded: bool,
    history: Vec<HistoryEntry>,
    history_error: Option<String>,
    operations: Vec<OperationEntry>,
    operations_error: Option<String>,
    history_search: String,
    profiles: Vec<Profile>,
    profiles_error: Option<String>,
    game_profiles: Vec<GameProfile>,
    game_profiles_error: Option<String>,
    game_states: HashMap<String, Result<ProfileState, String>>,
    game_states_loaded: bool,
    game_status_loading: bool,
    game_library_scan: Option<gaming::LibraryScan>,
    game_library_scanning: bool,
    measurement_process: String,
    measurements: Vec<crate::opti::measurements::Capture>,
    measurements_error: Option<String>,
    diagnostic_results: HashMap<DiagnosticCheck, DiagnosticResult>,
    snapshots: Vec<SystemSnapshot>,
    snapshots_error: Option<String>,
    backup_integrity: Option<Vec<BackupCheck>>,
    profile_name: String,
    compare_profile_left: String,
    compare_profile_right: String,
    restart_pending: bool,
    pending: Option<Pending>,
    restore_first: bool,
    elevated: bool,
    started: bool,
    services: Vec<SvcRow>,
    svc_sel: HashSet<String>,
    svc_search: String,
    startup: Vec<startup::Item>,
    startup_warning: Option<String>,
    st_sel: HashSet<String>,
    clean_sizes: HashMap<&'static str, Option<u64>>,
    clean_sel: HashSet<&'static str>,
    clean_preview: Option<(&'static str, cleaner::Preview)>,
    clean_preview_error: Option<(&'static str, String)>,
    adapters: Vec<String>,
    bench: Vec<(String, network::DnsBenchmark)>,
    events: Vec<OptiEvent>,
    /// `ctx.input(|i| i.time)` when the page last changed, for the enter transition.
    page_started: f64,
    sampler: crate::opti::sysmon::Sampler,
    telemetry_needs_reset: bool,
    info: Option<crate::opti::sysmon::SysInfo>,
    processes: Vec<ProcessInfo>,
    process_cpu: CpuTimes,
    process_error: Option<String>,
    process_loading: bool,
    process_last_scan: Option<Instant>,
    process_search: String,
    process_sort: usize,
    services_loaded: bool,
    startup_loaded: bool,
}

// region pure logic (unit tested)
/// (active, applicable) among the recommended tweaks that fit this PC.
pub fn score(cat: &Catalog, tr: &Traits, status: &HashMap<String, bool>) -> (usize, usize) {
    // the same tweaks Smart optimize would tick on this PC
    let applicable: Vec<&Tweak> = cat.tweaks.iter().filter(|t| t.auto_pick() && hw::fits(t, tr)).collect();
    let on = applicable.iter().filter(|t| status.get(&t.id).copied().unwrap_or(false)).count();
    (on, applicable.len())
}

fn storage_needs_review(snapshot: &crate::opti::sysmon::Snapshot) -> bool {
    snapshot.disk_available && snapshot.disk_total > 0 && snapshot.disk_fraction() >= 0.9
}

fn risk_tone(risk: &str) -> Tone {
    match risk {
        "Moderate" => Tone::Warn,
        "Advanced" => Tone::Bad,
        _ => Tone::Good,
    }
}

fn risk_order(risk: &str) -> u8 {
    match risk {
        "Safe" => 0,
        "Moderate" => 1,
        "Advanced" => 2,
        _ => 3,
    }
}

fn setting_match_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Configured",
        Some(false) => "Values differ",
        None => "Unavailable",
    }
}

fn tweak_matches_view(tweak: &Tweak, applied: bool, fits: bool, favorite: bool, state: usize, view: usize, compatibility: usize) -> bool {
    let state_matches = match state {
        1 => applied,
        2 => !applied,
        _ => true,
    };
    let view_matches = match view {
        1 => tweak.auto_pick() && fits,
        2 => favorite,
        _ => true,
    };
    let compatibility_matches = match compatibility {
        1 => fits,
        2 => !fits,
        _ => true,
    };
    state_matches && view_matches && compatibility_matches
}

fn profile_summary(tr: &Traits) -> String {
    let form = if tr.has("laptop") {
        "Laptop"
    } else if tr.has("desktop") {
        "Desktop"
    } else {
        "Form factor unknown"
    };
    let storage = if tr.has("nvme") {
        "NVMe SSD"
    } else if tr.has("ssd") {
        "SSD"
    } else if tr.has("hdd") {
        "Hard disk"
    } else {
        "Storage unknown"
    };
    let windows = if tr.has("win11") {
        "Windows 11"
    } else if tr.has("win10") {
        "Windows 10"
    } else {
        "Windows version unknown"
    };
    [form, storage, windows].join(" · ")
}

fn history_entry_matches(entry: &HistoryEntry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || entry.time.to_lowercase().contains(&query)
        || match entry.action {
            HistoryAction::Apply => {
                "apply".contains(&query) || "applied".contains(&query)
            }
            HistoryAction::Undo => "undo".contains(&query) || "undid".contains(&query),
        }
        || entry.changes.iter().any(|change| {
            change.id.to_lowercase().contains(&query)
                || change.name.to_lowercase().contains(&query)
                || change
                    .details
                    .iter()
                    .any(|detail| detail.to_lowercase().contains(&query))
        })
}

fn operation_entry_matches(entry: &OperationEntry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || entry.time.to_lowercase().contains(&query)
        || entry.title.to_lowercase().contains(&query)
        || entry.details.iter().any(|detail| detail.to_lowercase().contains(&query))
}

fn game_preference_history(entries: &[OperationEntry], executable: &str) -> Vec<OperationEntry> {
    let mut matches: Vec<_> = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.title.as_str(),
                "Setting per-game graphics preference" | "Restoring per-game graphics preference"
            ) && entry.details.iter().any(|detail| {
                detail
                    .strip_prefix("Executable: ")
                    .is_some_and(|path| path.eq_ignore_ascii_case(executable))
            })
        })
        .cloned()
        .collect();
    matches.sort_by(|left, right| right.time.cmp(&left.time));
    matches
}

enum ActivityEntry<'a> {
    Tweak(&'a HistoryEntry),
    Operation(&'a OperationEntry),
}

impl ActivityEntry<'_> {
    fn time(&self) -> &str {
        match self {
            Self::Tweak(entry) => &entry.time,
            Self::Operation(entry) => &entry.time,
        }
    }
}

fn activity_entries<'a>(
    history: &'a [HistoryEntry],
    operations: &'a [OperationEntry],
    query: &str,
) -> Vec<ActivityEntry<'a>> {
    let mut entries: Vec<_> = history
        .iter()
        .filter(|entry| history_entry_matches(entry, query))
        .map(ActivityEntry::Tweak)
        .chain(
            operations
                .iter()
                .filter(|entry| operation_entry_matches(entry, query))
                .map(ActivityEntry::Operation),
        )
        .collect();
    entries.sort_by(|left, right| right.time().cmp(left.time()));
    entries
}

#[cfg(test)]
mod operation_history_tests {
    use super::*;

    fn entry() -> OperationEntry {
        OperationEntry {
            time: "2026-10-29T12:30:00Z".into(),
            title: "Reset network stack".into(),
            status: OperationStatus::Finished,
            details: vec!["DNS cache flushed".into()],
        }
    }

    #[test]
    fn operation_search_matches_title_timestamp_and_output_case_insensitively() {
        let entry = entry();
        assert!(operation_entry_matches(&entry, "NETWORK"));
        assert!(operation_entry_matches(&entry, "12:30"));
        assert!(operation_entry_matches(&entry, "flushed"));
        assert!(!operation_entry_matches(&entry, "installer"));
        assert!(operation_entry_matches(&entry, "  "));
    }

    #[test]
    fn per_game_history_matches_only_graphics_operations_for_the_exact_executable() {
        let executable = r"C:\Games\Example\game.exe";
        let mut older = entry();
        older.title = "Setting per-game graphics preference".into();
        older.time = "2026-10-28T12:30:00Z".into();
        older.details = vec![
            format!("Executable: {executable}"),
            "Requested preference: High performance".into(),
        ];
        let mut newer = older.clone();
        newer.title = "Restoring per-game graphics preference".into();
        newer.time = "2026-10-29T12:30:00Z".into();
        newer.status = OperationStatus::Warnings;
        let mut other_game = older.clone();
        other_game.details[0] = r"Executable: C:\Games\Other\game.exe".into();
        let unrelated = entry();
        let operations = [older, newer.clone(), other_game, unrelated];

        let history = game_preference_history(&operations, &executable.to_uppercase());
        assert_eq!(history, vec![newer, operations[0].clone()]);
        assert_eq!(history[0].status, OperationStatus::Warnings);
    }

    #[test]
    fn worker_log_severity_uses_explicit_warning_and_error_prefixes() {
        assert_eq!(operation_log_severity("finished"), 0);
        assert_eq!(operation_log_severity("  WARN partial result"), 1);
        assert_eq!(operation_log_severity("ERROR command failed"), 2);
    }

    #[test]
    fn background_worker_panic_becomes_a_visible_error() {
        let result = catch_worker_panic(|| panic!("test worker failure"));
        assert_eq!(result.unwrap_err(), "test worker failure");
        assert!(catch_worker_panic(|| {}).is_ok());
    }

    #[test]
    fn activity_entries_merge_tweak_and_system_records_newest_first() {
        let older_tweak = HistoryEntry {
            time: "2026-10-29T12:00:00Z".into(),
            action: HistoryAction::Apply,
            changes: vec![],
        };
        let newer_operation = OperationEntry {
            time: "2026-10-29T12:05:00Z".into(),
            title: "Flush DNS".into(),
            status: OperationStatus::Finished,
            details: vec![],
        };
        let history = [older_tweak];
        let operations = [newer_operation];
        let entries = activity_entries(&history, &operations, "");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].time(), "2026-10-29T12:05:00Z");
        assert_eq!(entries[1].time(), "2026-10-29T12:00:00Z");
    }
}

fn draw_profile_difference(
    ui: &mut Ui,
    catalog: &Catalog,
    profile_name: &str,
    ids: Vec<String>,
    tokens: &theme::Tokens,
) {
    ui.vertical(|ui| {
        ui.label(RichText::new(format!("Only in {profile_name} · {}", ids.len()))
            .font(FontId::new(T_SMALL, semibold())).color(tokens.muted));
        if ids.is_empty() {
            ui.label(RichText::new("No unique tweaks").size(T_CAPTION).color(tokens.subtle));
        } else {
            egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                for id in ids {
                    if let Some(tweak) = catalog.find(&id) {
                        ui.label(RichText::new(&tweak.name).size(T_CAPTION).color(tokens.text))
                            .on_hover_text(&tweak.desc);
                    } else {
                        ui.label(RichText::new(format!("Unknown tweak · {id}"))
                            .size(T_CAPTION).color(tokens.warn));
                    }
                }
            });
        }
    });
}

fn tweak_notes(t: &Tweak) -> String {
    let mut n: Vec<String> = Vec::new();
    if t.desktop_only {
        n.push("desktop PCs".into());
    }
    if t.ssd_only {
        n.push("SSD only".into());
    }
    if !t.vendor.is_empty() {
        n.push(format!("{} systems only", t.vendor));
    }
    if t.min_ram > 0.0 {
        n.push(format!("{:.0} GB+ RAM", t.min_ram.round()));
    }
    if t.restart {
        n.push("restart needed".into());
    }
    if t.has_tag("optin") {
        n.push("opt-in, never in presets".into());
    }
    if t.partial {
        n.push("one PowerShell-only step skipped".into());
    }
    if !t.conflict.is_empty() {
        n.push(format!("one choice per group: {}", t.conflict));
    }
    if !t.retired.is_empty() {
        n.push(format!("retired: {} Undo it to put your old value back", t.retired));
    }
    n.join(", ")
}

/// Groups the Tweaks page can be filtered by. The first entry shows everything.
const TWEAK_GROUPS: [&str; 8] = ["All", "Recommended", "Gaming", "Privacy", "AI and debloat", "Opt-in", "Applied", "Not applied"];

/// Whether a tweak belongs to the chosen group (an index into `TWEAK_GROUPS`).
fn in_group(tw: &Tweak, group: usize, applied: bool) -> bool {
    match group {
        1 => tw.has_tag("safe"),
        2 => tw.has_tag("gaming"),
        3 => tw.has_tag("privacy"),
        4 => tw.has_tag("deai"),
        5 => tw.has_tag("optin"),
        6 => applied,
        7 => !applied,
        _ => true,
    }
}

/// The exact changes a tweak makes, for the expandable technical details: registry values, services, tasks and flags.
fn tweak_technical(t: &Tweak) -> Vec<String> {
    use crate::opti::catalog::RegVal;
    let mut out = vec![format!("id {}   risk {}   tags {}", t.id, t.risk, if t.tags.is_empty() { "none".to_string() } else { t.tags.join(", ") })];
    for r in &t.reg {
        let (value, kind) = match &r.v {
            RegVal::Num(n) => (format!("{n} (0x{:X})", n & 0xFFFF_FFFF), if r.t.is_empty() { "REG_DWORD".to_string() } else { format!("REG_{}", r.t.to_uppercase()) }),
            RegVal::Text(v) => (format!("\"{v}\""), if r.t == "ExpandString" { "REG_EXPAND_SZ".to_string() } else { "REG_SZ".to_string() }),
            RegVal::Bytes(b) => (format!("{} bytes", b.len()), "REG_BINARY".to_string()),
            RegVal::Multi(m) => (m.join(" | "), "REG_MULTI_SZ".to_string()),
        };
        out.push(format!("registry  {}\\{} = {}  [{}]", r.p, r.n, value, kind));
    }
    for sv in &t.svc {
        let start = match sv.start {
            0 => "Boot",
            1 => "System",
            2 => "Automatic",
            3 => "Manual",
            4 => "Disabled",
            _ => "other",
        };
        out.push(format!("service  {} start type -> {} ({})", sv.name, start, sv.start));
    }
    for tk in &t.task {
        out.push(format!("scheduled task  {tk} -> disabled"));
    }
    if t.special.is_some() || t.script.apply {
        out.push("also runs a built-in routine (not only registry, service and task changes)".into());
    }
    let mut flags: Vec<&str> = Vec::new();
    if t.restart {
        flags.push("restart needed");
    }
    if t.explorer {
        flags.push("restarts Explorer");
    }
    if t.script.undo {
        flags.push("custom undo step");
    }
    if !flags.is_empty() {
        out.push(flags.join(", "));
    }
    if out.len() == 1 {
        out.push("no registry, service or task entries".into());
    }
    out
}

struct Batch {
    restore_failed: bool,
    ok: usize,
    restart: bool,
    explorer: bool,
    now: HashMap<String, bool>,
    changes: Vec<HistoryChange>,
}

/// Applies or undoes a list of tweaks. With a requested restore point that fails, nothing is changed and
/// `restore_failed` is set, so the window can ask before going on without one.
fn run_batch(eng: &Engine, cat: &Catalog, ids: &[String], apply: bool, restore_point: bool, st: &mut State, log: &mut dyn FnMut(&str)) -> Batch {
    let mut b = Batch { restore_failed: false, ok: 0, restart: false, explorer: false, now: HashMap::new(), changes: Vec::new() };
    if apply && restore_point && !tools::create_restore_point(eng.sys, "Optimaxer - before tweaks", log) {
        log("WARN No restore point, so nothing was changed.");
        b.restore_failed = true;
        return b;
    }
    for id in ids {
        let Some(t) = cat.find(id) else { continue };
        let planned_before = eng.plan(t);
        let details = if apply {
            planned_before.iter().map(Change::describe).collect()
        } else {
            st.records.get(id).map(|record| vec![format!("Saved originals: {} registry values, {} service settings, {} scheduled tasks", record.reg.len(), record.svc.len(), record.task.len())]).unwrap_or_default()
        };
        let (on, failed) = if apply { (eng.apply(t, st, log), Vec::new()) } else { eng.undo_checked(t, st, log) };
        let verified = on == apply && failed.is_empty();
        let changed_state = planned_before != eng.plan(t);
        b.restart |= t.restart && changed_state;
        b.explorer |= t.explorer && verified && changed_state;
        if verified {
            b.ok += 1;
        }
        b.changes.push(HistoryChange { id: t.id.clone(), name: t.name.clone(), verified, undo_available: st.contains(id), details });
    }
    // A changed value can also change the status of another tweak that uses it.
    b.now = eng.status_all(&cat.tweaks, st);
    b
}

fn plan_lines(eng: &Engine, tweaks: &[&Tweak]) -> Vec<Line> {
    let mut out = Vec::new();
    for t in tweaks {
        out.push(Line { text: format!("{}  ({})", t.name, t.risk), warn: t.risk == "Advanced", head: true });
        let changes = eng.plan(t);
        if changes.is_empty() {
            out.push(Line { text: "already in effect, nothing to change".into(), warn: false, head: false });
        }
        for c in changes {
            out.push(Line { warn: matches!(c, Change::Note(_)), text: c.describe(), head: false });
        }
    }
    out
}
// endregion

impl OptiUi {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let (history, history_error) = match history::load() {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (operations, operations_error) = match history::load_operations() {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (profiles, profiles_error) = match profiles::load_store(&net::data_dir().join("profiles.json")) {
            Ok(profiles) => (profiles, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (game_profiles, game_profiles_error) = match gaming::load_store(&gaming::store_path()) {
            Ok(profiles) => (profiles, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (snapshots, snapshots_error) = match snapshots::load() {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let (measurements, measurements_error) = match crate::opti::measurements::load() {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Self {
            page: Page::Overview,
            cat: Arc::new(Catalog::load()),
            data: Arc::new(Data::load()),
            traits: None,
            status: HashMap::new(),
            status_ready: false,
            selected: HashSet::new(),
            tw_search: String::new(),
            tw_risk: 0,
            tw_group: 0,
            tw_state: 0,
            tw_view: 0,
            tw_compatibility: 0,
            tw_sort: 0,
            tw_open: HashSet::new(),
            tw_skip: Default::default(),
            tx,
            rx,
            busy: None,
            log: Vec::new(),
            log_expanded: false,
            history,
            history_error,
            operations,
            operations_error,
            history_search: String::new(),
            profiles,
            profiles_error,
            game_profiles,
            game_profiles_error,
            game_states: HashMap::new(),
            game_states_loaded: false,
            game_status_loading: false,
            game_library_scan: None,
            game_library_scanning: false,
            measurement_process: String::new(),
            measurements,
            measurements_error,
            diagnostic_results: HashMap::new(),
            snapshots,
            snapshots_error,
            backup_integrity: None,
            profile_name: "My profile".into(),
            compare_profile_left: String::new(),
            compare_profile_right: String::new(),
            restart_pending: false,
            pending: None,
            restore_first: true,
            elevated: hw::is_elevated(),
            started: false,
            services: Vec::new(),
            svc_sel: HashSet::new(),
            svc_search: String::new(),
            startup: Vec::new(),
            startup_warning: None,
            st_sel: HashSet::new(),
            clean_sizes: HashMap::new(),
            clean_sel: HashSet::new(),
            clean_preview: None,
            clean_preview_error: None,
            adapters: Vec::new(),
            bench: Vec::new(),
            events: Vec::new(),
            page_started: 0.0,
            sampler: crate::opti::sysmon::Sampler::new(),
            telemetry_needs_reset: true,
            info: None,
            processes: Vec::new(),
            process_cpu: CpuTimes::new(),
            process_error: None,
            process_loading: false,
            process_last_scan: None,
            process_search: String::new(),
            process_sort: 0,
            services_loaded: false,
            startup_loaded: false,
        }
    }

    /// Switches page by name. Used by the screenshot runner.
    pub fn show_page(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(p) = Page::ALL.iter().copied().find(|p| p.label().eq_ignore_ascii_case(name)) {
            self.set_page(ctx, p);
        }
    }

    pub fn start_transition(&mut self, ctx: &egui::Context) {
        self.page_started = ctx.input(|i| i.time);
    }

    fn set_page(&mut self, ctx: &egui::Context, p: Page) {
        let live = |page| matches!(page, Page::Overview | Page::Performance);
        if live(p) != live(self.page) {
            self.telemetry_needs_reset = true;
        }
        if p == Page::Gaming && self.page != Page::Gaming {
            self.game_states_loaded = false;
        }
        self.page = p;
        self.start_transition(ctx);
    }

    pub fn take_events(&mut self) -> Vec<OptiEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn status_text(&self) -> String {
        let admin = if self.elevated { "Administrator" } else { "Not elevated" };
        match &self.busy {
            Some(b) => format!("Working: {b}   ·   {admin}"),
            None => {
                let (on, total) = self.traits.as_ref().map(|t| score(&self.cat, t, &self.status)).unwrap_or((0, 0));
                format!("{} tweaks in the catalog  ·  {on} of {total} recommended active  ·  {admin}", self.cat.tweaks.len())
            }
        }
    }

    pub fn operation_entries(&self) -> &[OperationEntry] {
        &self.operations
    }

    pub fn current_job(&self) -> Option<&str> {
        self.busy.as_deref()
    }

    /// Open a downloaded installer from the store through Optimaxer: a restore point first, then the file.
    pub fn request_run_installer(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.pending = Some(Pending {
            title: "Create a restore point, then open the installer".into(),
            intro: format!("Optimaxer will create a System Restore point and then open {name}. You decide inside the installer whether to continue. Only run files you trust."),
            lines: vec![Line { text: path.to_string_lossy().to_string(), warn: false, head: false }],
            action: Action::RunInstaller(path),
            restore_option: false,
            ok_label: "Create restore point and open".into(),
        });
    }

    fn worker(&self, ctx: &egui::Context) -> Worker {
        Worker { tx: self.tx.clone(), ctx: ctx.clone(), operation_details: None, operation_status: None }
    }

    /// Starts a background job. One job at a time, like the PowerShell edition.
    fn spawn(&mut self, ctx: &egui::Context, title: &str, job: impl FnOnce(&Worker) + Send + 'static) -> bool {
        self.spawn_inner(ctx, title, false, job)
    }

    fn spawn_recorded(&mut self, ctx: &egui::Context, title: &str, job: impl FnOnce(&Worker) + Send + 'static) -> bool {
        self.spawn_inner(ctx, title, true, job)
    }

    fn spawn_inner(&mut self, ctx: &egui::Context, title: &str, record: bool, job: impl FnOnce(&Worker) + Send + 'static) -> bool {
        if self.busy.is_some() {
            self.events.push(OptiEvent::Toast("Another task is running, please wait.".into()));
            return false;
        }
        self.busy = Some(title.to_string());
        self.say(&format!("== {title}"));
        let operation_details = record.then(|| Arc::new(Mutex::new(Vec::new())));
        let operation_status = record.then(|| Arc::new(AtomicU8::new(0)));
        let w = Worker {
            tx: self.tx.clone(),
            ctx: ctx.clone(),
            operation_details: operation_details.clone(),
            operation_status: operation_status.clone(),
        };
        let title = title.to_owned();
        std::thread::spawn(move || {
            if let Err(message) = catch_worker_panic(|| job(&w)) {
                w.log(&format!("ERROR Background operation stopped unexpectedly: {message}"));
            }
            if let Some(details) = operation_details {
                let details = match details.lock() {
                    Ok(lines) => lines.clone(),
                    Err(_) => {
                        w.log("WARN Operation output could not be captured because its log buffer was unavailable.");
                        Vec::new()
                    }
                };
                let status = match operation_status.as_ref().map_or(0, |status| status.load(Ordering::Relaxed)) {
                    2 => OperationStatus::Error,
                    1 => OperationStatus::Warnings,
                    _ => OperationStatus::Finished,
                };
                let entry = OperationEntry {
                    time: format!("{}Z", crate::opti::now_iso()),
                    title,
                    status,
                    details,
                };
                match history::append_operation(entry) {
                    Ok(entries) => w.send(Msg::OperationHistory(Ok(entries))),
                    Err(error) => w.log(&format!("WARN Operation ran, but its activity record could not be saved: {error}")),
                }
            }
            w.send(Msg::Idle);
        });
        true
    }

    fn say(&mut self, line: &str) {
        self.log.push(line.to_string());
        if self.log.len() > LOG_LINES {
            self.log.drain(..self.log.len() - LOG_LINES);
        }
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(s) => self.say(&s),
                Msg::Toast(message) => self.events.push(OptiEvent::Toast(message)),
                Msg::Idle => self.busy = None,
                Msg::Traits(t) => self.traits = Some(t),
                Msg::Status(map) => {
                    self.status.extend(map);
                    self.status_ready = true;
                }
                Msg::Plan(p) => self.pending = Some(p),
                Msg::Services(s) => {
                    self.svc_sel.retain(|n| s.iter().any(|r| &r.name == n));
                    self.services = s;
                    self.services_loaded = true;
                }
                Msg::Startup(s) => {
                    self.st_sel.clear();
                    self.startup = s.items;
                    self.startup_warning = s.warning;
                    self.startup_loaded = true;
                }
                Msg::Scan(v) => {
                    self.clean_sel.clear();
                    for (id, size) in v {
                        let risk_ok = cleaner::TARGETS.iter().find(|t| t.id == id).is_some_and(|t| t.risk == "Safe");
                        if size.is_some_and(|s| s > 0) && risk_ok {
                            self.clean_sel.insert(id);
                        }
                        self.clean_sizes.insert(id, size);
                    }
                }
                Msg::CleanPreview(id, Ok(preview)) => {
                    self.clean_preview = Some((id, preview));
                    self.clean_preview_error = None;
                }
                Msg::CleanPreview(id, Err(error)) => {
                    self.clean_preview_error = Some((id, error));
                    self.clean_preview = None;
                }
                Msg::Adapters(a) => self.adapters = a,
                Msg::Bench(b) => self.bench = b,
                Msg::History(Ok(entries)) => {
                    self.history = entries;
                    self.history_error = None;
                }
                Msg::History(Err(error)) => self.history_error = Some(error),
                Msg::OperationHistory(Ok(entries)) => {
                    self.operations = entries;
                    self.operations_error = None;
                }
                Msg::OperationHistory(Err(error)) => self.operations_error = Some(error),
                Msg::GameProfiles(Ok(profiles)) => {
                    self.game_profiles = profiles;
                    self.game_profiles_error = None;
                    self.game_states_loaded = false;
                }
                Msg::GameProfiles(Err(error)) => self.game_profiles_error = Some(error),
                Msg::GameStates(states) => {
                    self.game_states = states;
                    self.game_states_loaded = true;
                    self.game_status_loading = false;
                }
                Msg::GameLibraries(scan) => {
                    self.game_library_scan = Some(scan);
                    self.game_library_scanning = false;
                }
                Msg::Measurements(Ok(captures)) => {
                    self.measurements = captures;
                    self.measurements_error = None;
                }
                Msg::Measurements(Err(error)) => self.measurements_error = Some(error),
                Msg::Diagnostic(check, result) => {
                    self.diagnostic_results.insert(check, result);
                }
                Msg::Snapshots(Ok(entries)) => {
                    self.snapshots = entries;
                    self.snapshots_error = None;
                }
                Msg::Snapshots(Err(error)) => self.snapshots_error = Some(error),
                Msg::BackupIntegrity(checks) => self.backup_integrity = Some(checks),
                Msg::Processes(Ok((rows, cpu_times))) => {
                    self.processes = rows;
                    self.process_cpu = cpu_times;
                    self.process_error = None;
                    self.process_loading = false;
                }
                Msg::Processes(Err(error)) => {
                    self.process_error = Some(error);
                    self.process_loading = false;
                }
                Msg::RestartRequired => self.restart_pending = true,
            }
        }
    }

    // region jobs
    fn load_first_time(&mut self, ctx: &egui::Context) {
        self.started = true;
        let cat = self.cat.clone();
        self.spawn(ctx, "Reading system information", move |w| {
            let sys = WinSystem;
            w.send(Msg::Traits(hw::detect(&sys)));
            let st = State::load();
            let eng = Engine::new(&sys, backup_dir());
            w.send(Msg::Status(eng.status_all(&cat.tweaks, &st)));
            if let Some(error) = st.load_error() {
                w.log(&format!("WARN Undo state could not be loaded: {error}. Tweak changes are blocked until the saved state is recovered."));
            } else {
                w.log("Optimaxer ready.");
            }
        });
    }

    fn refresh_processes(&mut self, ctx: &egui::Context) {
        if self.process_loading
            || self.process_last_scan.is_some_and(|last| last.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        self.process_loading = true;
        self.process_last_scan = Some(Instant::now());
        let previous = self.process_cpu.clone();
        let cores = std::thread::available_parallelism().map(|count| count.get() as u32).unwrap_or(1);
        let worker = self.worker(ctx);
        std::thread::spawn(move || worker.send(Msg::Processes(processes::scan(&previous, cores))));
    }

    fn refresh_status(&mut self, ctx: &egui::Context) {
        let cat = self.cat.clone();
        self.spawn(ctx, "Checking active tweaks", move |w| {
            let sys = WinSystem;
            let st = State::load();
            let eng = Engine::new(&sys, backup_dir());
            w.send(Msg::Status(eng.status_all(&cat.tweaks, &st)));
        });
    }

    fn refresh_services(&mut self, ctx: &egui::Context) {
        let data = self.data.clone();
        self.spawn(ctx, "Reading services", move |w| {
            let rows = services::list(&WinSystem, &data);
            w.log(&format!("Loaded {} services.", rows.len()));
            w.send(Msg::Services(rows));
        });
    }

    fn refresh_startup(&mut self, ctx: &egui::Context) {
        self.spawn(ctx, "Reading startup items", move |w| {
            let listing = startup::list(&WinSystem);
            w.log(&format!("Found {} startup items.", listing.items.len()));
            if let Some(warning) = &listing.warning { w.log(&format!("WARN {warning}")); }
            w.send(Msg::Startup(listing));
        });
    }

    fn scan_cleaner(&mut self, ctx: &egui::Context) {
        self.spawn(ctx, "Scanning for junk", move |w| {
            let sizes: Vec<_> = cleaner::TARGETS.iter().map(|t| (t.id, cleaner::scan(t))).collect();
            let total: u64 = sizes.iter().filter_map(|s| s.1).sum();
            w.log(&format!("Scan finished: {} can be reclaimed.", format_size(Some(total))));
            w.send(Msg::Scan(sizes));
        });
    }

    fn preview_cleaner(&mut self, ctx: &egui::Context, id: &'static str) {
        let Some(target) = cleaner::TARGETS.iter().find(|target| target.id == id).cloned() else { return };
        self.clean_preview = None;
        self.clean_preview_error = None;
        self.spawn(ctx, "Listing affected paths", move |worker| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cleaner::preview(&target)))
                .map_err(|_| "Path preview could not be completed.".to_string());
            worker.send(Msg::CleanPreview(id, result));
        });
    }

    /// Builds the confirmation for applying tweaks. Reads only; nothing changes until the user confirms.
    fn preview_apply(&mut self, ctx: &egui::Context) {
        let ids: Vec<String> = self.cat.tweaks.iter().filter(|t| self.selected.contains(&t.id)).map(|t| t.id.clone()).collect();
        self.preview_apply_ids(ctx, ids);
    }

    fn preview_apply_ids(&mut self, ctx: &egui::Context, ids: Vec<String>) {
        if ids.is_empty() {
            self.events.push(OptiEvent::Toast("No tweaks selected.".into()));
            return;
        }
        if !self.need_admin() {
            return;
        }
        let cat = self.cat.clone();
        self.spawn(ctx, "Preparing the list of changes", move |w| {
            let sys = WinSystem;
            let eng = Engine::new(&sys, backup_dir());
            let tweaks: Vec<&Tweak> = ids.iter().filter_map(|i| cat.find(i)).collect();
            let adv = tweaks.iter().filter(|t| t.risk == "Advanced").count();
            let mut intro = format!("Apply {} tweak(s)? Each one saves your current values first, so it can be undone.", tweaks.len());
            if adv > 0 {
                intro.push_str(&format!(" {adv} Advanced tweak(s) are marked in red: read them."));
            }
            w.send(Msg::Plan(Pending { title: "Apply tweaks".into(), intro, lines: plan_lines(&eng, &tweaks), action: Action::ApplyTweaks(ids), restore_option: true, ok_label: format!("Apply {} tweak{}", tweaks.len(), if tweaks.len() == 1 { "" } else { "s" }) }));
        });
    }

    fn preview_undo(&mut self) {
        let ids: Vec<String> = self.cat.tweaks.iter().filter(|t| self.selected.contains(&t.id)).map(|t| t.id.clone()).collect();
        self.preview_undo_ids(ids);
    }

    fn preview_undo_ids(&mut self, ids: Vec<String>) {
        if ids.is_empty() {
            self.events.push(OptiEvent::Toast("No tweaks selected.".into()));
            return;
        }
        if !self.need_admin() {
            return;
        }
        let lines = ids.iter().filter_map(|i| self.cat.find(i)).map(|t| Line { text: t.name.clone(), warn: false, head: false }).collect();
        let n = ids.len();
        self.pending = Some(Pending {
            title: "Undo tweaks".into(),
            intro: format!("Restore the original values saved for {} tweak(s)? Tweaks that were not applied by Optimaxer have nothing to restore.", ids.len()),
            lines,
            action: Action::UndoTweaks(ids),
            restore_option: false,
            ok_label: format!("Undo {n} tweak{}", if n == 1 { "" } else { "s" }),
        });
    }

    fn need_admin(&mut self) -> bool {
        if self.elevated {
            return true;
        }
        self.events.push(OptiEvent::Toast("This action changes system settings and needs administrator rights. Use the banner above to restart as administrator.".into()));
        false
    }

    fn confirm(&mut self, ctx: &egui::Context, p: Pending, restore_point: bool) {
        match p.action {
            Action::ApplyTweaks(ids) => self.run_tweaks(ctx, ids, true, restore_point),
            Action::UndoTweaks(ids) => self.run_tweaks(ctx, ids, false, false),
            Action::ApplyWithoutRestore(ids) => self.run_tweaks(ctx, ids, true, false),
            Action::UndoAll => {
                let cat = self.cat.clone();
                self.spawn_recorded(ctx, "Undo all Optimaxer tweaks", move |w| {
                    let sys = WinSystem;
                    let mut st = State::load();
                    let eng = Engine::new(&sys, backup_dir());
                    let ids: Vec<String> = st.records.keys().cloned().collect();
                    let (mut n, mut bad) = (0, Vec::new());
                    for id in &ids {
                        if let Some(t) = cat.find(id) {
                            let (_, failed) = eng.undo_checked(t, &mut st, &mut |l| w.log(l));
                            if failed.is_empty() {
                                n += 1;
                            } else {
                                bad.push(format!("{id} ({})", failed.join("; ")));
                            }
                        } else {
                            bad.push(format!("{id} (unsupported here; restore with the PowerShell edition)"));
                        }
                    }
                    if bad.is_empty() {
                        w.log(&format!("Reverted {n} tweak(s)."));
                    } else {
                        w.log(&format!("WARN Reverted {n} tweak(s); {} NOT undone, their saved values are kept: {}", bad.len(), bad.join(", ")));
                    }
                    w.send(Msg::Status(eng.status_all(&cat.tweaks, &st)));
                });
            }
            Action::SetServices { names, start } => {
                let data = self.data.clone();
                self.spawn_recorded(ctx, "Changing services", move |w| {
                    let n = services::set_start(&WinSystem, &data, &names, start, &backup_dir(), &mut |l| w.log(l));
                    w.log(&format!("{n} service(s) changed."));
                    w.send(Msg::Services(services::list(&WinSystem, &data)));
                });
            }
            Action::RestoreServices => {
                let data = self.data.clone();
                self.spawn_recorded(ctx, "Restoring services", move |w| {
                    services::restore_latest(&WinSystem, &backup_dir(), &mut |l| w.log(l));
                    w.send(Msg::Services(services::list(&WinSystem, &data)));
                });
            }
            Action::SetStartup { items, enable } => {
                self.spawn_recorded(ctx, if enable { "Enabling startup items" } else { "Disabling startup items" }, move |w| {
                    let sys = WinSystem;
                    for i in &items {
                        match startup::set_enabled(&sys, i, enable) {
                            Ok(()) => w.log(&format!("  {}: {}", i.name, if enable { "enabled" } else { "disabled" })),
                            Err(e) => w.log(&format!("  WARN {}: {e}", i.name)),
                        }
                    }
                    w.send(Msg::Startup(startup::list(&sys)));
                });
            }
            Action::Clean(ids) => {
                self.spawn_recorded(ctx, "Cleaning", move |w| {
                    let sys = WinSystem;
                    let mut sum = 0u64;
                    for id in &ids {
                        let Some(t) = cleaner::TARGETS.iter().find(|t| t.id == *id) else { continue };
                        let freed = cleaner::clean(&sys, t, &mut |l| w.log(l));
                        sum += freed;
                        w.log(&format!("  {}: freed {}", t.name, format_size(Some(freed))));
                    }
                    w.log(&format!("Total freed: {}", format_size(Some(sum))));
                    w.send(Msg::Scan(cleaner::TARGETS.iter().map(|t| (t.id, cleaner::scan(t))).collect()));
                });
            }
            Action::SetDns(provider) => {
                self.spawn_recorded(ctx, &format!("DNS: {}", provider.name), move |w| {
                    let adapters = network::active_adapters();
                    network::apply_dns(&WinSystem, &adapters, &provider, &mut |l| w.log(l));
                });
            }
            Action::ResetNetwork => {
                self.spawn_recorded(ctx, "Reset network stack", move |w| network::reset_stack(&WinSystem, &mut |l| w.log(l)));
            }
            Action::RunTool(tool) => {
                let title = tools::TOOLS.iter().find(|info| info.tool == tool).map_or("Running tool", |info| info.title);
                self.spawn_recorded(ctx, title, move |w| tools::run(&WinSystem, tool, &backup_dir(), &mut |l| w.log(l)));
            }
            Action::RunInstaller(path) => {
                self.spawn_recorded(ctx, "Restore point, then installer", move |w| {
                    let ok = tools::create_restore_point(&WinSystem, "Optimaxer - before installer", &mut |l| w.log(l));
                    if ok {
                        match open::that(&path) {
                            Ok(()) => w.log(&format!("Opened {}", path.display())),
                            Err(e) => w.log(&format!("WARN could not open the file: {e}")),
                        }
                    } else {
                        w.log("WARN The installer was not opened because the restore point failed. Open it from the Downloads list if you want to continue anyway.");
                    }
                });
            }
            Action::ApplyGameGraphics { executable, expected_current } => {
                self.spawn_recorded(ctx, "Setting per-game graphics preference", move |w| {
                    let store_path = gaming::store_path();
                    match gaming::load_store(&store_path) {
                        Ok(mut profiles) => {
                            let requested = profiles.iter().find(|profile| profile.executable.eq_ignore_ascii_case(&executable))
                                .map(|profile| profile.preference.label().to_owned());
                            w.log(&format!("Executable: {executable}"));
                            w.log(&format!("Previous value: {}", expected_current.as_ref().map_or("not set".into(), RegData::show)));
                            if let Some(requested) = requested { w.log(&format!("Requested preference: {requested}")); }
                            match gaming::apply(&WinSystem, &store_path, &mut profiles, &executable, &expected_current) {
                                Ok(()) => w.log("Windows graphics preference applied and verified."),
                                Err(error) => w.log(&format!("WARN Could not apply the graphics preference: {error}")),
                            }
                        }
                        Err(error) => w.log(&format!("WARN Could not read game profiles: {error}")),
                    }
                    refresh_game_profile_messages(w, &store_path);
                });
            }
            Action::UndoGameGraphics { executable } => {
                self.spawn_recorded(ctx, "Restoring per-game graphics preference", move |w| {
                    let store_path = gaming::store_path();
                    match gaming::load_store(&store_path) {
                        Ok(mut profiles) => {
                            let exact = profiles.iter().find(|profile| profile.executable.eq_ignore_ascii_case(&executable))
                                .ok_or_else(|| "Game profile no longer exists.".to_string())
                                .and_then(|profile| gaming::undo_preview(&WinSystem, profile));
                            match exact {
                                Ok((current, original)) => {
                                    w.log(&format!("Executable: {executable}"));
                                    w.log(&format!("Current value: {}", current.as_ref().map_or("not set".into(), RegData::show)));
                                    w.log(&format!("Restoring original: {}", original.as_ref().map_or("not set".into(), RegData::show)));
                                    match gaming::undo(&WinSystem, &store_path, &mut profiles, &executable) {
                                        Ok(()) => w.log("Original Windows graphics preference restored and verified."),
                                        Err(error) => w.log(&format!("WARN Could not restore the graphics preference: {error}")),
                                    }
                                }
                                Err(error) => w.log(&format!("WARN Could not prepare graphics restore: {error}")),
                            }
                        }
                        Err(error) => w.log(&format!("WARN Could not read game profiles: {error}")),
                    }
                    refresh_game_profile_messages(w, &store_path);
                });
            }
        }
    }

    fn run_tweaks(&mut self, ctx: &egui::Context, ids: Vec<String>, apply: bool, restore_point: bool) {
        let cat = self.cat.clone();
        let title = format!("{} {} tweak(s)", if apply { "Apply" } else { "Undo" }, ids.len());
        self.spawn(ctx, &title, move |w| {
            let sys = WinSystem;
            let mut st = State::load();
            let eng = Engine::new(&sys, backup_dir());
            let b = run_batch(&eng, &cat, &ids, apply, restore_point, &mut st, &mut |l| w.log(l));
            if b.restore_failed {
                w.send(Msg::Plan(Pending {
                    title: "No restore point".into(),
                    intro: format!("The restore point could not be created, so nothing was changed. Apply the {} tweak(s) anyway, without a restore point? Every tweak still saves its original values for undo.", ids.len()),
                    lines: vec![],
                    action: Action::ApplyWithoutRestore(ids),
                    restore_option: false,
                    ok_label: "Apply without a restore point".into(),
                }));
                return;
            }
            let (ok, restart, explorer, now, changes) = (b.ok, b.restart, b.explorer, b.now, b.changes);
            if !changes.is_empty() {
                let entry = HistoryEntry {
                    time: format!("{}Z", crate::opti::now_iso()),
                    action: if apply { HistoryAction::Apply } else { HistoryAction::Undo },
                    changes,
                };
                match history::append(entry) {
                    Ok(entries) => w.send(Msg::History(Ok(entries))),
                    Err(error) => {
                        w.log(&format!("WARN Changes ran, but tweak history could not be saved: {error}"));
                        w.send(Msg::History(Err(error)));
                    }
                }
            }
            if explorer {
                w.log("Restarting Explorer to apply interface changes");
                sys.run("taskkill.exe", &["/F", "/IM", "explorer.exe"]);
            }
            w.log(&format!("Finished: {ok} of {} tweak(s) verified.", ids.len()));
            if restart {
                w.log("Restart Windows for all changes to take full effect.");
                w.send(Msg::RestartRequired);
            }
            w.send(Msg::Status(now));
        });
    }

    fn select_preset(&mut self, preset: &str) {
        let Some(tr) = self.traits.clone() else {
            self.events.push(OptiEvent::Toast("Still analyzing, try again in a moment.".into()));
            return;
        };
        self.selected.clear();
        self.tw_skip.clear();
        match preset {
            "none" => {}
            "auto" => {
                let plan = hw::smart_plan(&self.cat, &tr, &self.status);
                for n in &plan.notes {
                    self.say(&format!("  {n}"));
                }
                for (id, why) in &plan.skipped {
                    let name = self.cat.find(id).map(|t| t.name.clone()).unwrap_or_else(|| id.clone());
                    self.say(&format!("  not ticked: {name} ({why})"));
                }
                self.tw_skip = plan.skipped.clone();
                self.say(&format!("Auto-detect: {} tweak(s) selected for this PC ({}).", plan.ids.len(), profile_summary(&tr)));
                if plan.ids.is_empty() {
                    self.events.push(OptiEvent::Toast("This PC is already optimized: every fitting tweak is applied.".into()));
                }
                self.selected.extend(plan.ids);
            }
            tag => self.selected.extend(hw::preset_ids(&self.cat, &tr, tag)),
        }
    }

    fn save_selected_profile(&mut self) {
        if self.profiles_error.is_some() {
            self.events.push(OptiEvent::Toast("Saved profiles could not be read. Export the current selection or recover the profiles file before saving.".into()));
            return;
        }
        if self.selected.is_empty() {
            self.events.push(OptiEvent::Toast("Select at least one tweak before saving a profile.".into()));
            return;
        }
        let ids = self.selected.iter().cloned().collect::<Vec<_>>();
        let profile = match Profile::new(&self.profile_name, ids) {
            Ok(profile) => profile,
            Err(error) => {
                self.profile_error(error);
                return;
            }
        };
        if self.profiles.iter().any(|saved| saved.name.eq_ignore_ascii_case(&profile.name)) {
            self.profile_error(format!("A profile named '{}' already exists. Remove it before saving a replacement.", profile.name));
            return;
        }
        let mut updated = self.profiles.clone();
        updated.push(profile);
        self.store_profiles(updated);
    }

    fn store_profiles(&mut self, updated: Vec<Profile>) {
        match profiles::save_store(&net::data_dir().join("profiles.json"), &updated) {
            Ok(()) => {
                self.profiles = updated;
                self.profiles_error = None;
                self.say("Saved tweak profile.");
            }
            Err(error) => self.profile_error(error),
        }
    }

    fn profile_error(&mut self, error: String) {
        self.say(&format!("Profile error: {error}"));
        self.events.push(OptiEvent::Toast(error));
    }

    fn select_profile(&mut self, profile: &Profile) {
        let Some(traits) = self.traits.as_ref() else {
            self.events.push(OptiEvent::Toast("Still analyzing this PC. Try again in a moment.".into()));
            return;
        };
        match profiles::compatible_ids(profile, &self.cat, traits) {
            Ok((ids, skipped)) if !ids.is_empty() => {
                self.selected = ids.into_iter().collect();
                self.tw_skip.clear();
                self.say(&format!("Selected profile '{}': {} tweak(s) fit this PC; {} skipped.", profile.name, self.selected.len(), skipped));
            }
            Ok((_, skipped)) => self.events.push(OptiEvent::Toast(format!("No tweaks in '{}' fit this PC ({skipped} skipped).", profile.name))),
            Err(error) => self.profile_error(error),
        }
    }

    fn import_profile(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Optimaxer profile", &["json"]).pick_file() else { return };
        let profile = match profiles::read_export(&path) {
            Ok(profile) => profile,
            Err(error) => {
                self.profile_error(error);
                return;
            }
        };
        let Some(traits) = self.traits.as_ref() else {
            self.events.push(OptiEvent::Toast("Still analyzing this PC. Try again in a moment.".into()));
            return;
        };
        match profiles::compatible_ids(&profile, &self.cat, traits) {
            Ok((ids, skipped)) if !ids.is_empty() => {
                self.selected = ids.into_iter().collect();
                self.tw_skip.clear();
                self.profile_name = profile.name.clone();
                let message = format!("Imported '{}': {} tweak(s) selected; {} unavailable or incompatible tweak(s) skipped. Review the list before applying.", profile.name, self.selected.len(), skipped);
                self.say(&message);
                self.events.push(OptiEvent::Toast(message));
            }
            Ok((_, skipped)) => self.events.push(OptiEvent::Toast(format!("No imported tweaks fit this PC ({skipped} skipped). Existing selection kept."))),
            Err(error) => self.profile_error(error),
        }
    }

    fn export_profile(&mut self, profile: &Profile) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Optimaxer profile", &["json"])
            .set_file_name("optimaxer-profile.json")
            .save_file()
        else {
            return;
        };
        match profiles::write_export(&path, profile) {
            Ok(()) => self.say(&format!("Exported profile '{}' to {}.", profile.name, path.display())),
            Err(error) => self.profile_error(error),
        }
    }

    fn export_selected_profile(&mut self) {
        if self.selected.is_empty() {
            self.events.push(OptiEvent::Toast("Select at least one tweak before exporting a profile.".into()));
            return;
        }
        let ids = self.selected.iter().cloned().collect::<Vec<_>>();
        match Profile::new(&self.profile_name, ids) {
            Ok(profile) => self.export_profile(&profile),
            Err(error) => self.profile_error(error),
        }
    }

    fn profiles_section(&mut self, ui: &mut Ui) {
        let t = tokens(ui.ctx());
        let screenshot_open = std::env::var_os("ORION_SHOTS").is_some() && std::env::var_os("ORION_SHOTS_PROFILE_OPEN").is_some();
        if self.profiles.len() >= 2 {
            if !self.profiles.iter().any(|profile| profile.name == self.compare_profile_left) {
                self.compare_profile_left = self.profiles[0].name.clone();
            }
            if !self.profiles.iter().any(|profile| profile.name == self.compare_profile_right)
                || self.compare_profile_right == self.compare_profile_left
            {
                self.compare_profile_right = self.profiles.iter()
                    .find(|profile| profile.name != self.compare_profile_left)
                    .map(|profile| profile.name.clone())
                    .unwrap_or_default();
            }
        } else {
            self.compare_profile_left.clear();
            self.compare_profile_right.clear();
        }
        egui::CollapsingHeader::new("Profiles").default_open(screenshot_open).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new("Save and move tweak selections between PCs. Imported profiles only select tweaks that fit this machine; applying still requires review and confirmation.").size(T_CAPTION).color(t.muted)).wrap());
            ui.add_space(S2);
            ui.horizontal_wrapped(|ui| {
                ui.add_sized([220.0, 32.0], egui::TextEdit::singleline(&mut self.profile_name).hint_text("Profile name"));
                if kit::secondary(ui, "Save selection", !self.selected.is_empty()).clicked() {
                    self.save_selected_profile();
                }
                if kit::ghost(ui, "Export selection…", !self.selected.is_empty()).clicked() {
                    self.export_selected_profile();
                }
                if kit::secondary(ui, "Import…", true).clicked() {
                    self.import_profile();
                }
            });
            if let Some(error) = &self.profiles_error {
                ui.label(RichText::new(format!("Saved profiles unavailable: {error}")).size(T_SMALL).color(t.warn));
            }
            if self.profiles.is_empty() && self.profiles_error.is_none() {
                ui.label(RichText::new("No saved profiles yet.").size(T_SMALL).color(t.subtle));
            }
            let mut select = None;
            let mut export = None;
            let mut remove = None;
            for (index, profile) in self.profiles.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&profile.name).font(FontId::new(T_BODY, semibold())).color(t.text));
                    ui.label(RichText::new(format!("{} tweaks", profile.tweak_ids.len())).size(T_CAPTION).color(t.muted));
                    if kit::ghost(ui, "Select", true).clicked() {
                        select = Some(index);
                    }
                    if kit::ghost(ui, "Export…", true).clicked() {
                        export = Some(index);
                    }
                    if kit::ghost(ui, "Remove", true).clicked() {
                        remove = Some(index);
                    }
                });
            }
            if self.profiles.len() >= 2 {
                ui.add_space(S2);
                ui.label(RichText::new("Compare saved selections").font(FontId::new(T_SMALL, semibold())).color(t.text));
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("profile-compare-left")
                        .selected_text(&self.compare_profile_left)
                        .show_ui(ui, |ui| {
                            for profile in &self.profiles {
                                ui.selectable_value(&mut self.compare_profile_left, profile.name.clone(), &profile.name);
                            }
                        });
                    ui.label(RichText::new("against").size(T_CAPTION).color(t.subtle));
                    egui::ComboBox::from_id_salt("profile-compare-right")
                        .selected_text(&self.compare_profile_right)
                        .show_ui(ui, |ui| {
                            for profile in &self.profiles {
                                if profile.name != self.compare_profile_left {
                                    ui.selectable_value(&mut self.compare_profile_right, profile.name.clone(), &profile.name);
                                }
                            }
                        });
                });
                let left = self.profiles.iter().find(|profile| profile.name == self.compare_profile_left);
                let right = self.profiles.iter().find(|profile| profile.name == self.compare_profile_right);
                if let (Some(left), Some(right)) = (left, right) {
                    let (only_left, only_right) = profiles::difference(left, right);
                    ui.columns(2, |columns| {
                        draw_profile_difference(&mut columns[0], &self.cat, &left.name, only_left, &t);
                        draw_profile_difference(&mut columns[1], &self.cat, &right.name, only_right, &t);
                    });
                }
            }
            if let Some(index) = select {
                let profile = self.profiles[index].clone();
                self.select_profile(&profile);
            }
            if let Some(index) = export {
                let profile = self.profiles[index].clone();
                self.export_profile(&profile);
            }
            if let Some(index) = remove {
                let mut updated = self.profiles.clone();
                updated.remove(index);
                self.store_profiles(updated);
            }
        });
    }
    // endregion

    // region drawing
    pub fn side_panel(&mut self, ui: &mut Ui, ctx: &egui::Context, collapsed: bool) {
        let t = tokens(ctx);
        ui.add_space(S3);
        if !collapsed {
            ui.label(RichText::new("Optimaxer").font(FontId::new(T_SMALL, semibold())).color(t.subtle));
            ui.add_space(S1);
        }
        for group in PageGroup::ALL {
            if !collapsed {
                ui.add_space(S2);
                ui.label(RichText::new(group.label()).font(FontId::new(T_CAPTION, semibold())).color(t.subtle));
                ui.add_space(S1);
            }
            for page in Page::ALL.into_iter().filter(|page| page.group() == group) {
                if kit::nav_item_icon(ui, self.page == page, Some(page.icon()), page.label(), None).clicked() && self.page != page {
                    self.set_page(ctx, page);
                }
            }
        }
        if !collapsed && let Some(tr) = &self.traits {
            ui.add_space(S4);
            ui.label(RichText::new("This PC").font(FontId::new(T_SMALL, semibold())).color(t.subtle));
            ui.add_space(S1);
            ui.label(RichText::new(format!("{:.0} GB RAM, {} threads", tr.ram_gb, tr.threads)).size(T_SMALL).color(t.muted));
            for g in &tr.gpus {
                ui.add(egui::Label::new(RichText::new(g).size(T_SMALL).color(t.muted)).wrap());
            }
            ui.add_space(S1);
            ui.add(egui::Label::new(RichText::new(profile_summary(tr)).size(T_CAPTION).color(t.subtle)).wrap());
        }
    }

    pub fn log_panel(&mut self, root: &mut Ui) {
        let t = tokens(root.ctx());
        let max_height = (root.available_height() * 0.42).clamp(180.0, 320.0);
        let collapsed = egui::Panel::bottom("opti-log-collapsed")
            .resizable(true)
            .exact_size(44.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.surface1).inner_margin(Margin::symmetric(S4 as i8, S1 as i8)));
        let expanded = egui::Panel::bottom("opti-log-expanded")
            .resizable(true)
            .default_size(216.0)
            .min_size(120.0)
            .max_size(max_height)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(t.surface1).inner_margin(Margin::symmetric(S4 as i8, S1 as i8)));

        let mut expansion_change = None;
        let mut open_activity = false;
        egui::Panel::show_switched(root, &mut self.log_expanded, collapsed, expanded, |ui, is_expanded| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Operation center").font(FontId::new(T_BODY, semibold())).color(t.text));
                if let Some(job) = &self.busy {
                    kit::chip(ui, "Running", Tone::Brand);
                    ui.add(egui::Label::new(RichText::new(job).size(T_CAPTION).color(t.muted)).truncate());
                }
                if !self.log.is_empty() {
                    kit::chip(ui, &format!("{} entries", self.log.len()), Tone::Neutral);
                }
                if self.busy.is_some() {
                    ui.add(egui::Spinner::new().size(12.0).color(t.brand_text));
                }
                if is_expanded {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::ghost(ui, "Hide", true).clicked() {
                            expansion_change = Some(false);
                        }
                        if kit::ghost(ui, "View history", true).clicked() {
                            open_activity = true;
                        }
                        let has = !self.log.is_empty();
                        if kit::ghost(ui, "Copy", has).clicked() {
                            ui.ctx().copy_text(self.log.join("\n"));
                        }
                        if kit::ghost(ui, "Clear", has).clicked() {
                            self.log.clear();
                        }
                    });
                } else {
                    let latest = self.log.last().map(String::as_str).unwrap_or("No recent activity");
                    let summary = self.busy.as_deref().unwrap_or(latest);
                    let color = if summary.contains("WARN") || summary.contains("failed") { t.warn } else { t.subtle };
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::ghost(ui, "Show", true).clicked() {
                            expansion_change = Some(true);
                        }
                        ui.add(egui::Label::new(RichText::new(summary).size(T_CAPTION).color(color)).truncate());
                    });
                }
            });
            if is_expanded {
                if self.log.is_empty() {
                    ui.label(RichText::new("Output from the actions you run appears here.").size(T_SMALL).color(t.subtle));
                }
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                    for l in &self.log {
                        let color = if l.contains("WARN") || l.contains("failed") { t.warn } else if l.starts_with("==") { t.brand_text } else { t.muted };
                        ui.label(RichText::new(l).monospace().size(T_CAPTION).color(color));
                    }
                });
            }
        });
        if let Some(expanded) = expansion_change {
            self.log_expanded = expanded;
        }
        if open_activity {
            self.set_page(root.ctx(), Page::Activity);
        }
    }

    /// Runs every frame, whichever section is showing: collects worker results, draws the confirmation window and
    /// keeps the restore-point choice in `Settings`.
    pub fn tick(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        self.poll();
        self.restore_first = settings.opti_restore_point;
        if self.busy.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.confirm_window(ctx);
        if self.restore_first != settings.opti_restore_point {
            let previous = settings.opti_restore_point;
            settings.opti_restore_point = self.restore_first;
            if let Err(error) = settings.save_checked() {
                settings.opti_restore_point = previous;
                self.restore_first = previous;
                let message = format!("Restore point preference was not saved: {error}");
                self.say(&message);
                self.events.push(OptiEvent::Toast(message));
            }
        }
        if self.restart_pending && !settings.opti_restart_required {
            settings.opti_restart_required = true;
            settings.opti_restart_uptime = crate::opti::sysmon::uptime_secs();
            if let Err(error) = settings.save_checked() {
                let message = format!("Restart reminder could not be saved: {error}");
                self.say(&message);
                self.events.push(OptiEvent::Toast(message));
            }
        }
        self.restart_pending = self.restart_pending || settings.opti_restart_required;
    }

    pub fn central(&mut self, ui: &mut Ui, ctx: &egui::Context, suggestions: &[AppItem], settings: &mut Settings) {
        if !self.started {
            self.load_first_time(ctx);
        }
        let t = tokens(ctx);
        if !self.elevated {
            egui::Frame::new().fill(mix(t.canvas, t.warn, 0.14)).corner_radius(t.card_radius()).inner_margin(Margin::symmetric(S4 as i8, S3 as i8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Changes need administrator rights. Everything can be read without them.").color(t.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::secondary(ui, "Restart as administrator", true).clicked() {
                            if hw::relaunch_as_admin() {
                                self.events.push(OptiEvent::Close);
                            } else {
                                self.events.push(OptiEvent::Toast("Administrator access was not granted, so Optimaxer stays in read-only mode.".into()));
                            }
                        }
                    });
                });
            });
            ui.add_space(S3);
        }
        if self.restart_pending {
            egui::Frame::new().fill(mix(t.canvas, t.warn, 0.11)).corner_radius(t.card_radius()).inner_margin(Margin::symmetric(S4 as i8, S3 as i8)).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    kit::chip(ui, "Restart required", Tone::Warn);
                    ui.label(RichText::new("Restart Windows when convenient to finish applying system changes.").color(t.text));
                });
            });
            ui.add_space(S3);
        }
        let started = self.page_started;
        kit::enter(ui, started, |ui| {
            ui.add_enabled_ui(self.pending.is_none(), |ui| {
                if self.page == Page::Tweaks {
                    ui.vertical(|ui| {
                        let action_height = if ui.available_width() < 680.0 { 104.0 } else { 68.0 };
                        let scroll_height = (ui.available_height() - action_height).max(120.0);
                        egui::ScrollArea::vertical().id_salt(("optimaxer-page", self.page.label())).auto_shrink([false, false])
                            .max_height(scroll_height).show(ui, |ui| self.tweaks_page(ui, ctx, settings));
                        self.tweak_action_bar(ui, ctx);
                    });
                } else {
                    egui::ScrollArea::vertical().id_salt(("optimaxer-page", self.page.label())).auto_shrink([false, false]).show(ui, |ui| match self.page {
                        Page::Overview => self.overview_page(ui, ctx),
                        Page::Performance => self.performance_page(ui, ctx),
                        Page::Diagnostics => self.diagnostics_page(ui, ctx),
                        Page::Snapshots => self.snapshots_page(ui, ctx),
                        Page::Processes => self.processes_page(ui, ctx),
                        Page::Gaming => self.gaming_page(ui, ctx),
                        Page::Activity => self.activity_page(ui, ctx),
                        Page::Tweaks => unreachable!("Tweaks uses its pinned action bar layout"),
                        Page::Services => self.services_page(ui, ctx),
                        Page::Startup => self.startup_page(ui, ctx),
                        Page::Cleaner => self.cleaner_page(ui, ctx),
                        Page::Network => self.network_page(ui, ctx),
                        Page::Tools => self.tools_page(ui, ctx, suggestions),
                    });
                }
            });
        });
    }

    fn tweak_action_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        let selected = self.selected.len();
        let recommendation = self.traits.as_ref().map(|traits| {
            self.selected.iter().filter_map(|id| self.cat.find(id))
                .filter(|tweak| tweak.auto_pick() && hw::fits(tweak, traits)).count()
        });
        let undoable = self.selected.iter().filter(|id| self.status.get(*id).copied().unwrap_or(false)).count();
        ui.add_space(S1);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if selected == 0 {
                    ui.label(RichText::new("Select tweaks to build a change plan.").size(T_SMALL).color(t.muted));
                } else if let Some(recommended) = recommendation {
                    let review = selected.saturating_sub(recommended);
                    ui.label(RichText::new(format!("{selected} selected · {recommended} recommended · {review} require review"))
                        .font(FontId::new(T_SMALL, semibold())).color(t.text));
                } else {
                    ui.label(RichText::new(format!("{selected} selected · analyzing this PC's compatibility"))
                        .font(FontId::new(T_SMALL, semibold())).color(t.muted));
                }
                if kit::primary(ui, "Preview changes", self.busy.is_none() && selected > 0).clicked() {
                    self.preview_apply(ctx);
                }
                if kit::secondary(ui, "Undo selected", self.busy.is_none() && undoable > 0).clicked() {
                    self.preview_undo();
                }
                if kit::secondary(ui, "Save selection", self.busy.is_none() && selected > 0).clicked() {
                    self.save_selected_profile();
                }
                if kit::ghost(ui, "Clear selection", selected > 0).clicked() {
                    self.selected.clear();
                }
            });
        });
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.as_ref() else { return };
        let t = tokens(ctx);
        let mut decision: Option<bool> = None;
        let mut restore = self.restore_first;
        let resp = egui::Modal::new(egui::Id::unique("opti-confirm")).backdrop_color(Color32::from_black_alpha(if t.dark { 150 } else { 70 })).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.label(RichText::new(&p.title).font(FontId::new(theme::T_TITLE, semibold())).color(t.text));
            ui.add_space(S1);
            ui.add(egui::Label::new(RichText::new(&p.intro).color(t.muted)).wrap());
            if !p.lines.is_empty() {
                ui.add_space(S3);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().max_height(330.0).auto_shrink([false, true]).show(ui, |ui| {
                        for l in &p.lines {
                            if l.head {
                                ui.add_space(S1);
                                ui.label(RichText::new(&l.text).font(FontId::new(T_BODY, semibold())).color(if l.warn { t.bad } else { t.text }));
                            } else {
                                ui.add(egui::Label::new(RichText::new(&l.text).size(T_SMALL).color(if l.warn { t.warn } else { t.muted })).wrap());
                            }
                        }
                    });
                });
            }
            if p.restore_option {
                ui.add_space(S3);
                ui.checkbox(&mut restore, "Create a System Restore point first");
            }
            ui.add_space(S4);
            ui.horizontal(|ui| {
                if kit::primary(ui, &p.ok_label, true).clicked() {
                    decision = Some(true);
                }
                if kit::ghost(ui, "Cancel", true).clicked() {
                    decision = Some(false);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let copied_id = egui::Id::NULL.with("opti-confirm-copied");
                    let now = ui.input(|i| i.time);
                    let copied = ui.data(|d| d.get_temp::<f64>(copied_id)).is_some_and(|at| now - at < 1.6);
                    if copied {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(1700));
                    }
                    if kit::ghost(ui, if copied { "Copied" } else { "Copy details" }, true).clicked() {
                        ui.ctx().copy_text(confirm_text(p));
                        ui.data_mut(|d| d.insert_temp(copied_id, now));
                    }
                });
            });
        });
        if resp.should_close() && decision.is_none() {
            decision = Some(false);
        }
        self.restore_first = restore;
        if let Some(ok) = decision
            && let Some(p) = self.pending.take()
            && ok {
            self.confirm(ctx, p, restore);
        }
    }

    fn toolbar_search(&self, ui: &mut Ui, text: &mut String, hint: &str) {
        ui.add_sized([260.0, 34.0], egui::TextEdit::singleline(text).hint_text(hint).margin(Margin::symmetric(S3 as i8, 8)));
    }

    fn tick_telemetry(&mut self, ctx: &egui::Context) {
        if self.telemetry_needs_reset {
            self.sampler.reset_window();
            self.telemetry_needs_reset = false;
        }
        self.sampler.tick();
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    pub fn suspend_telemetry(&mut self) {
        self.telemetry_needs_reset = true;
    }

    fn overview_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        use crate::opti::sysmon::{format_uptime, read_info};
        let t = tokens(ctx);
        self.tick_telemetry(ctx);
        let info = self.info.get_or_insert_with(|| read_info(&WinSystem)).clone();
        let s = self.sampler.latest;
        kit::page_header(ui, "Overview", "Live readings from this PC, and the quickest ways to tune it.");
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let gap = S3;
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            ui.horizontal(|ui| {
                let ready = self.busy.is_none() && self.traits.is_some();
                if kit::primary(ui, "Smart optimize", ready).on_hover_text("Reads your hardware and selects only the tweaks that fit this PC").clicked() {
                    self.select_preset("auto");
                    if !self.selected.is_empty() {
                        self.set_page(ctx, Page::Tweaks);
                        self.preview_apply(ctx);
                    }
                }
                if kit::secondary(ui, "Free RAM", self.busy.is_none()).clicked() && self.need_admin() {
                    self.pending = Some(Pending { title: "Free RAM now".into(), intro: "Clears every RAM cache Windows allows: process and system working sets, the file cache, the modified and standby lists, identical pages and the registry cache. Windows refills the file cache as you open files again.".into(), lines: vec![], action: Action::RunTool(Tool::FreeRam), restore_option: false, ok_label: "Free RAM".into() });
                }
                if kit::secondary(ui, "Scan for junk", self.busy.is_none()).clicked() {
                    self.set_page(ctx, Page::Cleaner);
                    self.scan_cleaner(ctx);
                }
            });
            let inactive_recommended = if self.status_ready {
                self.traits.as_ref().map(|tr| {
                    let (active, total) = score(&self.cat, tr, &self.status);
                    total.saturating_sub(active)
                })
            } else {
                None
            };
            let storage_review = storage_needs_review(&s);
            if inactive_recommended.is_some_and(|count| count > 0) || storage_review {
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Recommended actions").font(FontId::new(T_BODY, semibold())).color(t.text));
                    ui.add_space(S1);
                    if let Some(count) = inactive_recommended.filter(|count| *count > 0) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{count} Smart Optimize setting{} not active on this PC.", if count == 1 { " is" } else { "s are" })).size(T_SMALL).color(t.muted));
                            if kit::secondary(ui, "Review tweaks", self.busy.is_none()).clicked() {
                                self.tw_state = 2;
                                self.tw_view = 1;
                                self.tw_compatibility = 1;
                                self.set_page(ctx, Page::Tweaks);
                            }
                        });
                    }
                    if storage_review {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("System drive is {:.0}% full. Scan cleanup targets before choosing what to remove.", s.disk_fraction() * 100.0)).size(T_SMALL).color(t.muted));
                            if kit::secondary(ui, "Scan storage", self.busy.is_none()).clicked() && self.busy.is_none() {
                                self.set_page(ctx, Page::Cleaner);
                                self.scan_cleaner(ctx);
                            }
                        });
                    }
                });
            }
            ui.add_space(S1);
            let width = ui.available_width();
            let compact = width < 720.0;
            let w3 = ((width - 2.0 * gap) / 3.0 - 1.0).floor();
            ui.horizontal_top(|ui| {
                let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
                let cards = [
                    ("Processor", s.cpu / 100.0, if s.cpu_available { format!("{:.0}%", s.cpu) } else { "Unavailable".into() }, if s.cpu_available { format!("{} threads", info.threads) } else { "Windows CPU counters unavailable".into() }, if s.cpu_available { kit::load_color(&t, s.cpu) } else { t.muted }),
                    ("Memory", s.ram_fraction(), if s.ram_available { format!("{:.0}%", s.ram_fraction() * 100.0) } else { "Unavailable".into() }, if s.ram_available { format!("{} of {}", format_size(Some(s.ram_used)), format_size(Some(s.ram_total))) } else { "Windows memory counters unavailable".into() }, if s.ram_available { kit::load_color(&t, s.ram_fraction() * 100.0) } else { t.muted }),
                    ("Storage", s.disk_fraction(), if s.disk_available { format!("{:.0}%", s.disk_fraction() * 100.0) } else { "Unavailable".into() }, if s.disk_available { format!("{drive} {} of {}", format_size(Some(s.disk_used)), format_size(Some(s.disk_total))) } else { "Windows volume counters unavailable".into() }, if s.disk_available { kit::load_color(&t, s.disk_fraction() * 100.0) } else { t.muted }),
                ];
                for (name, frac, value, sub, color) in cards {
                    let card_height = if compact { 126.0 } else { 132.0 };
                    ui.allocate_ui_with_layout(vec2(w3, card_height), Layout::top_down(Align::Min), |ui| {
                        kit::card(ui, |ui| {
                            let inner_width = (w3 - 2.0 * S4 - 2.0).max(40.0);
                            ui.set_width(inner_width);
                            if compact {
                                ui.label(RichText::new(name).font(FontId::new(T_SMALL, semibold())).color(t.text));
                                ui.label(RichText::new(&value).font(FontId::new(T_TITLE, semibold())).color(t.text));
                                ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).fill(color).desired_height(6.0).corner_radius(3).desired_width(inner_width));
                                ui.add(egui::Label::new(RichText::new(sub).size(T_CAPTION).color(t.muted)).truncate());
                            } else {
                                ui.horizontal(|ui| {
                                    ui.allocate_ui_with_layout(vec2((inner_width - 104.0).max(80.0), 104.0), Layout::top_down(Align::Min), |ui| {
                                        ui.add_space(S2);
                                        ui.label(RichText::new(name).font(FontId::new(T_BODY, semibold())).color(t.text));
                                        ui.add_space(S2);
                                        ui.add(egui::Label::new(RichText::new(sub).size(T_SMALL).color(t.muted)).truncate());
                                    });
                                    kit::gauge(ui, 92.0, frac, color, &value, "in use");
                                });
                            }
                        });
                    });
                }
            });
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("This PC").font(FontId::new(T_HEADING, semibold())).color(t.text));
                ui.add_space(S1);
                egui::Grid::new("pc-info").num_columns(2).spacing(vec2(S5, S1 + 2.0)).show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(RichText::new(k).size(T_SMALL).color(t.muted));
                        ui.add(egui::Label::new(RichText::new(v).size(T_BODY).color(t.text)).wrap());
                        ui.end_row();
                    };
                    row("Processor", if info.cpu.is_empty() { "Unknown".into() } else { info.cpu.clone() });
                    row("Graphics", if info.gpus.is_empty() { "Unknown".into() } else { info.gpus.join(", ") });
                    row("Windows", info.os.clone());
                    row("Uptime", format_uptime(s.uptime));
                    row("Processes", s.processes.to_string());
                    if let Some(tr) = &self.traits {
                        row("Profile", profile_summary(tr));
                    }
                });
            });
            ui.add_space(S2);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Recent tweak changes").font(FontId::new(T_HEADING, semibold())).color(t.text));
                ui.add_space(S1);
                if let Some(error) = &self.history_error {
                    ui.label(RichText::new(format!("History unavailable: {error}")).size(T_SMALL).color(t.warn));
                } else if self.history.is_empty() {
                    ui.label(RichText::new("Confirmed tweak apply and undo operations will appear here.").size(T_SMALL).color(t.muted));
                } else {
                    for (index, entry) in self.history.iter().rev().take(8).enumerate() {
                        let verified = entry.changes.iter().filter(|change| change.verified).count();
                        let undoable = entry.changes.iter().filter(|change| change.undo_available).count();
                        let action = match entry.action {
                            HistoryAction::Apply => "Applied",
                            HistoryAction::Undo => "Undid",
                        };
                        egui::CollapsingHeader::new(format!("{action} tweaks · {verified}/{} verified · {undoable} undoable", entry.changes.len()))
                            .id_salt(("tweak-history", index, &entry.time))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&entry.time).size(T_CAPTION).color(t.subtle));
                                for change in &entry.changes {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(&change.name).size(T_SMALL).color(t.text));
                                        kit::chip(ui, if change.verified { "Verified" } else { "Incomplete" }, if change.verified { Tone::Good } else { Tone::Warn });
                                        if change.undo_available {
                                            kit::chip(ui, "Undo available", Tone::Neutral);
                                        }
                                    });
                                    for detail in &change.details {
                                        ui.add(egui::Label::new(RichText::new(detail).size(T_CAPTION).color(t.muted)).wrap());
                                    }
                                }
                            });
                    }
                    ui.label(RichText::new("Records confirmed tweak changes and verification. Performance gains are not measured.").size(T_CAPTION).color(t.subtle));
                }
            });
            ui.add_space(S3);
        });
    }

    fn performance_metric_card(ui: &mut Ui, title: &str, value: &str, detail: &str) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(T_SMALL).color(t.muted));
            ui.label(RichText::new(value).font(FontId::new(T_TITLE, semibold())).color(t.text));
            ui.label(RichText::new(detail).size(T_CAPTION).color(t.subtle));
        });
    }

    fn performance_load_chart(
        ui: &mut Ui,
        title: &str,
        value: Option<f32>,
        data: &VecDeque<Option<f32>>,
        color: Color32,
    ) {
        let t = tokens(ui.ctx());
        let width = ui.available_width();
        kit::card(ui, |ui| {
            ui.set_width(width - 2.0 * S4 - 2.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(FontId::new(T_BODY, semibold())).color(t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let value = value.map_or_else(|| "Unavailable".into(), |value| format!("{value:.0}%"));
                    ui.label(RichText::new(value).size(T_SMALL).color(t.muted));
                });
            });
            ui.add_space(S1);
            kit::sparkline(ui, vec2(ui.available_width(), 86.0), data, color);
        });
    }

    fn performance_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        self.tick_telemetry(ctx);
        let snapshot = self.sampler.latest;
        kit::page_header(
            ui,
            "Performance",
            "Live CPU, memory, disk, adapter traffic and Windows GPU engine readings. GPU load is the busiest reported physical engine across adapters.",
        );
        let processor = if snapshot.cpu_available { format!("{:.0}%", snapshot.cpu) } else { "Unavailable".into() };
        let processor_detail: String = if snapshot.cpu_available { "total logical CPU capacity".into() } else { "Windows CPU counters unavailable".into() };
        let memory = if snapshot.ram_available { format!("{:.1}%", snapshot.ram_fraction() * 100.0) } else { "Unavailable".into() };
        let memory_detail: String = if snapshot.ram_available {
            format!("{} of {}", format_size(Some(snapshot.ram_used)), format_size(Some(snapshot.ram_total)))
        } else {
            "Windows memory counters unavailable".into()
        };
        let gpu = snapshot.gpu_busiest_engine.map_or_else(|| "Unavailable".into(), |value| format!("{value:.1}%"));
        let gpu_memory = snapshot.gpu_dedicated_memory_bytes.map_or_else(
            || "Dedicated GPU memory unavailable".into(),
            |bytes| format!("Dedicated GPU memory in use: {}", format_size(Some(bytes))),
        );
        let (network, network_detail): (String, String) = if let Some(sample) = snapshot.network {
            (
                format!("{} / {}", crate::opti::sysmon::format_bytes_per_second(sample.received_bytes_per_sec), crate::opti::sysmon::format_bytes_per_second(sample.sent_bytes_per_sec)),
                "aggregate active adapters".into(),
            )
        } else {
            (
                if snapshot.network_available { "Collecting first sample".into() } else { "Unavailable".into() },
                "Windows adapter counters".into(),
            )
        };
        let metrics = [
            ("Processor", processor.as_str(), processor_detail.as_str()),
            ("Memory", memory.as_str(), memory_detail.as_str()),
            ("Adapter receive / send", network.as_str(), network_detail.as_str()),
            ("Busiest GPU engine", gpu.as_str(), gpu_memory.as_str()),
        ];
        if ui.available_width() < 960.0 {
            for (title, value, detail) in metrics {
                Self::performance_metric_card(ui, title, value, detail);
                ui.add_space(S2);
            }
        } else {
            ui.columns(4, |columns| {
                for (column, (title, value, detail)) in columns.iter_mut().zip(metrics) {
                    Self::performance_metric_card(column, title, value, detail);
                }
            });
        }
        ui.add_space(S2);
        let load_charts = [
            ("Processor, last 60 seconds", &self.sampler.cpu_history, snapshot.cpu_available.then_some(snapshot.cpu), kit::load_color(&t, snapshot.cpu)),
            ("Memory, last 60 seconds", &self.sampler.ram_history, snapshot.ram_available.then_some(snapshot.ram_fraction() * 100.0), kit::load_color(&t, snapshot.ram_fraction() * 100.0)),
            ("Busiest GPU engine, last 60 seconds", &self.sampler.gpu_history, snapshot.gpu_busiest_engine, snapshot.gpu_busiest_engine.map_or(t.muted, |value| kit::load_color(&t, value))),
        ];
        if ui.available_width() < 900.0 {
            for (title, history, value, color) in load_charts {
                Self::performance_load_chart(ui, title, value, history, color);
                ui.add_space(S2);
            }
        } else {
            ui.columns(3, |columns| {
                for (column, (title, history, value, color)) in columns.iter_mut().zip(load_charts) {
                    Self::performance_load_chart(column, title, value, history, color);
                }
            });
        }
        ui.add_space(S2);
        let network_history = &self.sampler.network_history;
        let disk_history = &self.sampler.disk_history;
        let draw_network = |ui: &mut Ui| {
            let width = ui.available_width();
            kit::card(ui, |ui| {
                ui.set_width(width - 2.0 * S4 - 2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Adapter traffic, last 60 seconds").font(FontId::new(T_BODY, semibold())).color(t.text));
                    if let Some(sample) = snapshot.network {
                        ui.label(RichText::new(format!("↓ {}", crate::opti::sysmon::format_bytes_per_second(sample.received_bytes_per_sec))).size(T_SMALL).color(t.brand_text));
                        ui.label(RichText::new(format!("↑ {}", crate::opti::sysmon::format_bytes_per_second(sample.sent_bytes_per_sec))).size(T_SMALL).color(t.good));
                    } else {
                        let status = if snapshot.network_available { "Sampling adapter counters…" } else { "Adapter counters unavailable" };
                        ui.label(RichText::new(status).size(T_SMALL).color(t.muted));
                    }
                });
                ui.label(RichText::new("Aggregate active adapter counters. Virtual adapters may be included; this is not internet throughput.").size(T_CAPTION).color(t.subtle));
                ui.add_space(S1);
                kit::network_chart(ui, vec2(ui.available_width(), 100.0), network_history, t.brand_text, t.good);
            });
        };
        let draw_disk = |ui: &mut Ui| {
            let width = ui.available_width();
            kit::card(ui, |ui| {
                ui.set_width(width - 2.0 * S4 - 2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Disk activity, last 60 seconds").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let status = snapshot.disk_activity
                        .map(crate::opti::sysmon::format_bytes_per_second)
                        .unwrap_or_else(|| if snapshot.disk_counter_available { "Collecting first sample".into() } else { "Unavailable".into() });
                    ui.label(RichText::new(status).size(T_SMALL).color(t.brand_text));
                });
                ui.label(RichText::new("Aggregate physical-disk read and write throughput from Windows performance counters.").size(T_CAPTION).color(t.subtle));
                ui.add_space(S1);
                kit::rate_chart(ui, vec2(ui.available_width(), 100.0), disk_history, t.brand_text);
            });
        };
        if ui.available_width() < 680.0 {
            draw_network(ui);
            ui.add_space(S2);
            draw_disk(ui);
        } else {
            ui.columns(2, |columns| {
                draw_network(&mut columns[0]);
                draw_disk(&mut columns[1]);
            });
        }
        ui.add_space(S2);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Process-level CPU and working-set readings are available in Processes.").size(T_SMALL).color(t.muted));
            if kit::secondary(ui, "Open Processes", true).clicked() {
                self.set_page(ctx, Page::Processes);
            }
        });
        ui.add_space(S2);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Game frame-time comparison").font(FontId::new(T_HEADING, semibold())).color(t.text));
            ui.label(RichText::new("PresentMon records 30 seconds, then samples system counters and up to 10 TCP connections to 1.1.1.1:53. Keep the same game and scene running for both captures. First use downloads a SHA-256 verified tool.").size(T_SMALL).color(t.muted));
            ui.label(RichText::new("Results are observations, not proof a tweak caused a change. DNS endpoint TCP timings are not ICMP ping or game-server latency. Boot time comes from the latest Windows startup event; compare it only across different boot timestamps. Unsupported counters show Unavailable.").size(T_CAPTION).color(t.subtle));
            ui.add_space(S1);
            ui.horizontal_wrapped(|ui| {
                ui.add_sized([240.0, 32.0], egui::TextEdit::singleline(&mut self.measurement_process).hint_text("Game process, for example game.exe").interactive(self.busy.is_none()));
                let valid = crate::opti::measurements::validate_process_name(&self.measurement_process).is_ok();
                if kit::secondary(ui, "Record before", valid && self.busy.is_none()).clicked() {
                    let process = self.measurement_process.trim().to_string();
                    self.start_measurement(ctx, process, "before");
                }
                if kit::secondary(ui, "Record after", valid && self.busy.is_none()).clicked() {
                    let process = self.measurement_process.trim().to_string();
                    self.start_measurement(ctx, process, "after");
                }
            });
            if let Some(error) = &self.measurements_error {
                ui.label(RichText::new(format!("Measurement history unavailable: {error}")).size(T_SMALL).color(t.warn));
            }
            let process = self.measurement_process.trim();
            let (before, after) = crate::opti::measurements::latest_pair(&self.measurements, process);
            if process.is_empty() || (before.is_none() && after.is_none()) {
                kit::empty_state(ui, "No matching captures", "Record a before capture, change settings and restart if needed, then record after using the same process name.");
            } else if let (Some(before), Some(after)) = (before, after) && after.time >= before.time {
                egui::Grid::new("frame-measurement-comparison").num_columns(3).spacing(vec2(S4, S1)).show(ui, |ui| {
                    ui.label(RichText::new("Metric").size(T_SMALL).color(t.muted));
                    ui.label(RichText::new("Before").size(T_SMALL).color(t.muted));
                    ui.label(RichText::new("After · observed").size(T_SMALL).color(t.muted));
                    ui.end_row();
                    for (name, first, second, unit) in [
                        ("Average", before.average_fps, after.average_fps, "FPS"),
                        ("1% low", before.low_1_fps, after.low_1_fps, "FPS"),
                        ("0.1% low", before.low_01_fps, after.low_01_fps, "FPS"),
                        ("99th percentile frame time", before.p99_frame_ms, after.p99_frame_ms, "ms"),
                    ] {
                        ui.label(RichText::new(name).size(T_SMALL).color(t.text));
                        ui.label(RichText::new(format!("{first:.1} {unit}")).size(T_SMALL).color(t.text));
                        ui.label(RichText::new(format!("{second:.1} {unit} ({:+.1})", second - first)).size(T_SMALL).color(t.text));
                        ui.end_row();
                    }
                    let metric = |value: Option<f64>, unit: &str| value.map_or_else(|| "Unavailable".into(), |value| format!("{value:.2} {unit}"));
                    for (name, first, second, unit) in [
                        ("DPC average", before.dpc_average_percent, after.dpc_average_percent, "%"),
                        ("DPC maximum", before.dpc_max_percent, after.dpc_max_percent, "%"),
                        ("Interrupt average", before.interrupt_average_percent, after.interrupt_average_percent, "%"),
                        ("DNS endpoint TCP average", before.dns_tcp_average_ms, after.dns_tcp_average_ms, "ms"),
                        ("DNS endpoint TCP jitter", before.dns_tcp_jitter_ms, after.dns_tcp_jitter_ms, "ms"),
                        ("Timer resolution", before.timer_resolution_ms, after.timer_resolution_ms, "ms"),
                        ("Last boot duration", before.boot_duration_ms.map(|ms| ms as f64), after.boot_duration_ms.map(|ms| ms as f64), "ms"),
                    ] {
                        ui.label(RichText::new(name).size(T_SMALL).color(t.text));
                        ui.label(RichText::new(metric(first, unit)).size(T_SMALL).color(t.text));
                        ui.label(RichText::new(metric(second, unit)).size(T_SMALL).color(t.text));
                        ui.end_row();
                    }
                    ui.label(RichText::new("Frames / stutters").size(T_SMALL).color(t.text));
                    ui.label(RichText::new(format!("{} / {}", before.frames, before.stutters)).size(T_SMALL).color(t.text));
                    ui.label(RichText::new(format!("{} / {}", after.frames, after.stutters)).size(T_SMALL).color(t.text));
                    ui.end_row();
                    ui.label(RichText::new("DNS TCP successes / failures").size(T_SMALL).color(t.text));
                    ui.label(RichText::new(format!("{} / {}", before.dns_tcp_samples, before.dns_tcp_failures)).size(T_SMALL).color(t.text));
                    ui.label(RichText::new(format!("{} / {}", after.dns_tcp_samples, after.dns_tcp_failures)).size(T_SMALL).color(t.text));
                    ui.end_row();
                });
                ui.label(RichText::new(format!("Before {} · After {} · {}-second frame captures; DPC/interrupt counters sample for 5 seconds when available", before.time, after.time, before.seconds)).size(T_CAPTION).color(t.subtle));
                ui.label(RichText::new(format!("Boot events · {} · {}", before.boot_event_time.as_deref().unwrap_or("Unavailable"), after.boot_event_time.as_deref().unwrap_or("Unavailable"))).size(T_CAPTION).color(t.subtle));
            } else if before.is_some() && after.is_some() {
                kit::empty_state(ui, "Capture order is reversed", "The latest after capture predates the before capture. Record after again to compare in order.");
            } else {
                ui.label(RichText::new("One capture saved. Record the other using the same process name.").size(T_SMALL).color(t.muted));
            }
        });
    }

    fn start_measurement(&mut self, ctx: &egui::Context, process: String, label: &'static str) {
        self.spawn(ctx, if label == "before" { "Recording before frame times" } else { "Recording after frame times" }, move |worker| {
            worker.log("PresentMon captures 30 seconds, followed by up to 10 seconds of system and network sampling. Keep the selected game running.");
            let result = crate::opti::measurements::record(&process, label);
            if let Err(error) = &result { worker.log(&format!("ERROR {error}")); }
            worker.send(Msg::Measurements(result));
        });
    }

    fn activity_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "Activity", "Search confirmed tweak changes and system-action results. Recorded warnings and errors describe operation output, not final Windows state.");
        if let Some(job) = self.busy.clone() {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(t.brand_text));
                    kit::chip(ui, "Running", Tone::Brand);
                    ui.label(RichText::new(job).font(FontId::new(T_BODY, semibold())).color(t.text));
                    if kit::secondary(ui, "Show live output", true).clicked() {
                        self.log_expanded = true;
                    }
                });
                ui.label(RichText::new("Live output is available in the Operation center.").size(T_CAPTION).color(t.subtle));
            });
            ui.add_space(S2);
        }
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [280.0, 34.0],
                    egui::TextEdit::singleline(&mut self.history_search)
                    .hint_text("Search action, tweak, output or date"),
            );
            let visible = self
                .history
                .iter()
                .filter(|entry| history_entry_matches(entry, &self.history_search))
                .count()
                + self
                    .operations
                    .iter()
                    .filter(|entry| operation_entry_matches(entry, &self.history_search))
                    .count();
            ui.label(
                RichText::new(format!("{visible} matching records"))
                    .size(T_SMALL)
                    .color(t.muted),
            );
        });
        ui.add_space(S2);
        if let Some(error) = &self.history_error {
            ui.label(RichText::new(format!("Tweak history unavailable: {error}"))
                .size(T_SMALL).color(t.warn));
        }
        if let Some(error) = &self.operations_error {
            ui.label(RichText::new(format!("System action history unavailable: {error}"))
                .size(T_SMALL).color(t.warn));
        }
        if self.history.is_empty() && self.operations.is_empty() {
            if self.history_error.is_none() && self.operations_error.is_none() {
                kit::empty_state(ui, "No recorded activity", "Confirmed tweak changes and system-action output will appear here.");
            }
            return;
        }
        let entries = activity_entries(&self.history, &self.operations, &self.history_search);
        if entries.is_empty() {
            kit::empty_state(ui, "No matching activity", "Try another search or clear the filter.");
            return;
        }
        for (index, item) in entries.iter().enumerate() {
            match item {
                ActivityEntry::Tweak(entry) => {
                    let verified = entry.changes.iter().filter(|change| change.verified).count();
                    let undoable = entry.changes.iter().filter(|change| change.undo_available).count();
                    let action = match entry.action {
                        HistoryAction::Apply => "Applied",
                        HistoryAction::Undo => "Undid",
                    };
                    egui::CollapsingHeader::new(format!(
                        "{action} {} tweaks · {verified}/{} verified · {undoable} undoable",
                        entry.changes.len(),
                        entry.changes.len()
                    ))
                    .id_salt(("activity-tweak", index, &entry.time))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&entry.time).size(T_CAPTION).color(t.subtle));
                        for change in &entry.changes {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&change.name).size(T_SMALL).color(t.text));
                                kit::chip(ui, if change.verified { "Verified" } else { "Incomplete" }, if change.verified { Tone::Good } else { Tone::Warn });
                                if change.undo_available {
                                    kit::chip(ui, "Undo available", Tone::Neutral);
                                }
                            });
                            for detail in &change.details {
                                ui.add(egui::Label::new(RichText::new(detail).size(T_CAPTION).color(t.muted)).wrap());
                            }
                        }
                    });
                }
                ActivityEntry::Operation(entry) => {
                    egui::CollapsingHeader::new(format!("{} · {} · {} output line(s)", entry.title, entry.status.label(), entry.details.len()))
                        .id_salt(("activity-operation", index, &entry.time))
                        .show(ui, |ui| {
                            ui.label(RichText::new(&entry.time).size(T_CAPTION).color(t.subtle));
                            let (summary, tone) = match entry.status {
                                OperationStatus::Finished => ("The worker finished. Final system state is not verified by this record.", Tone::Neutral),
                                OperationStatus::Warnings => ("The worker finished with warnings. Review the captured output.", Tone::Warn),
                                OperationStatus::Error => ("The worker reported an error. Review the captured output.", Tone::Bad),
                                OperationStatus::Unknown => ("This record has no status supported by this version.", Tone::Neutral),
                            };
                            kit::chip(ui, entry.status.label(), tone);
                            ui.label(RichText::new(summary).size(T_CAPTION).color(t.muted));
                            if entry.details.is_empty() {
                                ui.label(RichText::new("No output was captured.")
                                    .size(T_CAPTION).color(t.warn));
                            } else {
                                for detail in &entry.details {
                                    let line = detail.trim_start();
                                    let color = if line.starts_with("ERROR") {
                                        t.bad
                                    } else if line.starts_with("WARN") || line.contains("failed") {
                                        t.warn
                                    } else {
                                        t.muted
                                    };
                                    ui.add(egui::Label::new(RichText::new(detail).monospace().size(T_CAPTION).color(color)).wrap());
                                }
                            }
                        });
                }
            }
        }
    }

    fn processes_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.refresh_processes(ctx);
        let t = tokens(ctx);
        kit::page_header(ui, "Processes", "Read-only process list. CPU is sampled over time; Windows may restrict paths and memory details for protected processes.");
        ui.horizontal_wrapped(|ui| {
            ui.add_sized([260.0, 32.0], egui::TextEdit::singleline(&mut self.process_search).hint_text("Search process, PID or path"));
            ui.label(RichText::new(format!("{} processes", self.processes.len())).size(T_SMALL).color(t.muted));
            ui.label(RichText::new("Refreshes every 3 seconds while open").size(T_CAPTION).color(t.subtle));
            if kit::ghost(ui, if self.process_loading { "Refreshing" } else { "Refresh" }, !self.process_loading).clicked() {
                self.process_last_scan = None;
            }
            ui.label(RichText::new("Sort").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("process-sort")
                .selected_text(["CPU", "Memory", "Name"][self.process_sort.min(2)])
                .show_ui(ui, |ui| {
                    for (index, label) in ["CPU", "Memory", "Name"].iter().enumerate() {
                        ui.selectable_value(&mut self.process_sort, index, *label);
                    }
                });
        });
        if let Some(error) = &self.process_error {
            ui.label(RichText::new(format!("Process scan failed: {error}")).size(T_SMALL).color(t.warn));
        }
        if self.processes.is_empty() {
            if self.process_loading {
                kit::skeleton(ui, 6);
            } else if self.process_error.is_some() {
                kit::empty_state(ui, "Could not read processes", "Try Refresh after checking whether security software is blocking process queries.");
            } else {
                kit::empty_state(ui, "No processes returned", "Refresh to query the Windows process list.");
            }
            return;
        }

        let query = self.process_search.trim().to_lowercase();
        let mut rows: Vec<&ProcessInfo> = self.processes.iter().filter(|process| {
            query.is_empty()
                || process.name.to_lowercase().contains(&query)
                || process.pid.to_string().contains(&query)
                || process.image_path.as_ref().is_some_and(|path| path.to_string_lossy().to_lowercase().contains(&query))
        }).collect();
        match self.process_sort.min(2) {
            0 => rows.sort_by(|left, right| {
                right.cpu_percent.unwrap_or(-1.0).total_cmp(&left.cpu_percent.unwrap_or(-1.0))
                    .then(left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            }),
            1 => rows.sort_by(|left, right| {
                right.working_set.is_some().cmp(&left.working_set.is_some())
                    .then(right.working_set.unwrap_or(0).cmp(&left.working_set.unwrap_or(0)))
                    .then(left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            }),
            _ => rows.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()).then(left.pid.cmp(&right.pid))),
        }
        if rows.is_empty() {
            kit::empty_state(ui, "No matching processes", "Try a shorter search or clear the filter.");
            return;
        }

        egui_extras::TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(egui_extras::Column::initial(210.0).at_least(150.0).resizable(true))
            .column(egui_extras::Column::initial(70.0).at_least(54.0).resizable(true))
            .column(egui_extras::Column::initial(78.0).at_least(65.0).resizable(true))
            .column(egui_extras::Column::initial(112.0).at_least(92.0).resizable(true))
            .column(egui_extras::Column::remainder().at_least(200.0).resizable(true))
            .header(30.0, |mut header| {
                for label in ["Process", "PID", "CPU", "Working set", "Image path"] {
                    header.col(|ui| {
                        ui.label(RichText::new(label).font(FontId::new(T_SMALL, semibold())).color(t.muted));
                    });
                }
            })
            .body(|body| {
                body.rows(29.0, rows.len(), |mut row| {
                    let process = rows[row.index()];
                    row.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(&process.name).size(T_SMALL).color(t.text)).truncate())
                            .on_hover_text(&process.name);
                    });
                    row.col(|ui| { ui.label(RichText::new(process.pid.to_string()).size(T_CAPTION).monospace().color(t.muted)); });
                    row.col(|ui| {
                        let value = process.cpu_percent.map(|value| format!("{value:.1}%")).unwrap_or_else(|| "—".into());
                        ui.label(RichText::new(value).size(T_SMALL).color(t.text));
                    });
                    row.col(|ui| {
                        let value = process.working_set.map(|bytes| format_size(Some(bytes))).unwrap_or_else(|| "Unavailable".into());
                        ui.label(RichText::new(value).size(T_SMALL).color(if process.working_set.is_some() { t.text } else { t.subtle }));
                    });
                    row.col(|ui| {
                        let value = process.image_path.as_ref().map(|path| path.display().to_string()).unwrap_or_else(|| "Unavailable".into());
                        ui.add(egui::Label::new(RichText::new(value).size(T_CAPTION).color(t.muted)).truncate())
                            .on_hover_text(process.image_path.as_ref().map_or("Windows denied access to this image path.".to_owned(), |path| path.display().to_string()));
                    });
                });
            });
    }

    fn run_diagnostic(&mut self, ctx: &egui::Context, check: DiagnosticCheck) {
        let title = format!("Scan: {}", check.title());
        self.spawn_recorded(ctx, &title, move |worker| {
            worker.log(&format!("Running {} with read-only options.", check.executable()));
            let result = diagnostics::run(&WinSystem, check);
            for line in result.output.lines() {
                worker.log(line);
            }
            worker.log(&format!("Exit code: {}", result.exit_code));
            worker.send(Msg::Diagnostic(check, result));
        });
    }

    fn check_saved_backups(&mut self, ctx: &egui::Context) {
        let optimaxer_dir = net::data_dir().join("optimaxer");
        let state_file = optimaxer_dir.join("state.json");
        let backup_dir = optimaxer_dir.join("backups");
        self.backup_integrity = None;
        self.spawn(ctx, "Checking saved backups", move |worker| {
            worker.send(Msg::BackupIntegrity(backup_integrity::inspect(&state_file, &backup_dir)));
        });
    }

    fn capture_system_snapshot(&mut self, ctx: &egui::Context) {
        self.spawn_recorded(ctx, "Capture system snapshot", |worker| {
            let snapshot = snapshots::capture(&WinSystem);
            let result = snapshots::append(snapshot);
            match &result {
                Ok(entries) => worker.log(&format!("Saved local inventory snapshot. {} snapshots retained.", entries.len())),
                Err(error) => worker.log(&format!("ERROR Could not save system snapshot: {error}")),
            }
            worker.send(Msg::Snapshots(result));
        });
    }

    fn snapshots_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "System Snapshot", "Capture and browse a local inventory of this PC. Snapshots are not backups and cannot restore Windows.");
        ui.horizontal_wrapped(|ui| {
            if kit::primary(ui, if self.busy.as_deref() == Some("Capture system snapshot") { "Capturing…" } else { "Capture snapshot" }, self.busy.is_none()).clicked() {
                self.capture_system_snapshot(ctx);
            }
            ui.label(RichText::new("Stores OS, processor, graphics, memory and system-volume readings on this PC. Keeps the 30 newest captures.").size(T_SMALL).color(t.muted));
        });
        ui.add_space(S2);

        if let Some(error) = &self.snapshots_error {
            ui.label(RichText::new(error).size(T_SMALL).color(t.bad));
            ui.add_space(S2);
        }
        if self.snapshots.is_empty() {
            if self.snapshots_error.is_none() {
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("No snapshots yet").font(FontId::new(T_BODY, semibold())).color(t.text));
                    ui.label(RichText::new("Capture an inventory to record the current Windows build and detected hardware. No system settings are changed.").size(T_SMALL).color(t.muted));
                });
            }
            return;
        }

        if self.snapshots.len() >= 2 {
            let changes = snapshots::compare_latest(&self.snapshots);
            ui.label(RichText::new("Changes since previous capture").font(FontId::new(T_BODY, semibold())).color(t.text));
            ui.add_space(S1);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                if changes.is_empty() {
                    ui.label(RichText::new("No inventory changes detected between the two newest captures.").size(T_SMALL).color(t.muted));
                } else {
                    for change in changes {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(change.label).size(T_SMALL).color(t.muted));
                            ui.add(egui::Label::new(RichText::new(change.earlier).size(T_SMALL).color(t.text)).wrap());
                            ui.label(RichText::new("→").size(T_SMALL).color(t.subtle));
                            ui.add(egui::Label::new(RichText::new(change.latest).size(T_SMALL).color(t.text)).wrap());
                        });
                    }
                }
            });
            ui.add_space(S2);
        }

        for (index, snapshot) in self.snapshots.iter().enumerate() {
            let os = if snapshot.operating_system.is_empty() { "Operating system unavailable" } else { &snapshot.operating_system };
            egui::CollapsingHeader::new(format!("{} · {os}", snapshot.captured_at))
                .id_salt(("system-snapshot", index))
                .default_open(index == 0)
                .show(ui, |ui| {
                    let processor = if snapshot.processor.is_empty() { "Unavailable" } else { &snapshot.processor };
                    let graphics = if snapshot.graphics.is_empty() { "Unavailable".into() } else { snapshot.graphics.join(", ") };
                    let memory = match (snapshot.memory_used, snapshot.memory_total) {
                        (Some(used), Some(total)) => format!("{} used of {}", format_size(Some(used)), format_size(Some(total))),
                        _ => "Unavailable".into(),
                    };
                    let drive = match (snapshot.drive_used, snapshot.drive_total) {
                        (Some(used), Some(total)) => format!("{} used of {}", format_size(Some(used)), format_size(Some(total))),
                        _ => "Unavailable".into(),
                    };
                    for (label, value) in [
                        ("Processor", processor.to_owned()),
                        ("Logical threads", snapshot.logical_threads.to_string()),
                        ("Graphics", graphics),
                        ("Memory", memory),
                        ("System volume", if snapshot.system_drive.is_empty() { drive } else { format!("{} · {drive}", snapshot.system_drive) }),
                        ("Uptime at capture", crate::opti::sysmon::format_uptime(snapshot.uptime_seconds)),
                    ] {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(label).size(T_SMALL).color(t.muted));
                            ui.add(egui::Label::new(RichText::new(value).size(T_SMALL).color(t.text)).wrap());
                        });
                    }
                });
        }
    }

    fn diagnostics_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "Diagnostics", "Read-only Windows integrity scans. Review the command output; Optimaxer does not infer a system health score.");
        ui.label(RichText::new("Both checks require administrator access. They may keep CPU and storage busy for several minutes. They do not repair files or change registry settings.").size(T_SMALL).color(t.muted));
        ui.add_space(S3);

        for check in [DiagnosticCheck::SystemFiles, DiagnosticCheck::ComponentStore] {
            let result = self.diagnostic_results.get(&check).cloned();
            let running = self.busy.as_deref() == Some(match check {
                DiagnosticCheck::SystemFiles => "Scan: Windows system files",
                DiagnosticCheck::ComponentStore => "Scan: Windows component store",
            });
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(check.title()).font(FontId::new(T_BODY, semibold())).color(t.text));
                        let description = match check {
                            DiagnosticCheck::SystemFiles => "SFC /verifyonly checks protected Windows system files without repairing them.",
                            DiagnosticCheck::ComponentStore => "DISM /ScanHealth checks the component store without running RestoreHealth.",
                        };
                        ui.label(RichText::new(description).size(T_SMALL).color(t.muted));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.elevated {
                            kit::chip(ui, "Administrator required", Tone::Warn);
                        }
                        if kit::primary(ui, if running { "Scanning…" } else { "Run scan" }, self.elevated && self.busy.is_none()).clicked() {
                            self.run_diagnostic(ctx, check);
                        }
                    });
                });
                if let Some(result) = result {
                    ui.add_space(S2);
                    let status = if result.exit_code == 0 {
                        "Completed; review the command output"
                    } else {
                        "Command failed; review the error output"
                    };
                    ui.label(RichText::new(format!("{status} · exit code {}", result.exit_code)).size(T_SMALL).color(if result.exit_code == 0 { t.muted } else { t.bad }));
                    egui::ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(result.output).monospace().size(T_SMALL).color(t.text)).wrap());
                    });
                }
            });
            ui.add_space(S2);
        }
    }

    fn refresh_game_states(&mut self, ctx: &egui::Context) {
        if self.game_status_loading || self.game_profiles_error.is_some() || self.game_states_loaded {
            return;
        }
        if self.game_profiles.is_empty() {
            self.game_states_loaded = true;
            return;
        }
        self.game_status_loading = true;
        let profiles = self.game_profiles.clone();
        let worker = self.worker(ctx);
        std::thread::spawn(move || {
            let states = profiles.iter().map(|profile| {
                (profile.executable.clone(), gaming::state(&WinSystem, profile))
            }).collect();
            worker.send(Msg::GameStates(states));
        });
    }

    fn add_game_executable(&mut self, ctx: &egui::Context) {
        self.add_game_executable_from(ctx, None);
    }

    fn add_game_executable_from(&mut self, ctx: &egui::Context, directory: Option<&std::path::Path>) {
        if self.game_profiles_error.is_some() {
            self.events.push(OptiEvent::Toast("Saved game profiles are unreadable. Recover the file before adding profiles.".into()));
            return;
        }
        let mut dialog = rfd::FileDialog::new().add_filter("Windows game executable", &["exe"]);
        if let Some(directory) = directory.filter(|directory| directory.is_dir()) {
            dialog = dialog.set_directory(directory);
        }
        let Some(path) = dialog.pick_file() else { return };
        let canonical = match path.canonicalize() {
            Ok(path) if path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) => {
                let text = path.to_string_lossy();
                text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
            }
            Ok(_) => {
                self.events.push(OptiEvent::Toast("Choose an existing Windows .exe file.".into()));
                return;
            }
            Err(error) => {
                self.events.push(OptiEvent::Toast(format!("Could not resolve that executable path: {error}")));
                return;
            }
        };
        let profile = match GameProfile::new(&canonical) {
            Ok(profile) => profile,
            Err(error) => {
                self.events.push(OptiEvent::Toast(error));
                return;
            }
        };
        let mut profiles = self.game_profiles.clone();
        profiles.push(profile);
        match gaming::save_store(&gaming::store_path(), &profiles) {
            Ok(()) => {
                self.game_profiles = profiles;
                self.game_profiles_error = None;
                self.game_states.clear();
                self.game_states_loaded = false;
                self.refresh_game_states(ctx);
            }
            Err(error) => self.events.push(OptiEvent::Toast(error)),
        }
    }

    fn scan_game_libraries(&mut self, ctx: &egui::Context) {
        if self.game_library_scanning { return; }
        self.game_library_scanning = true;
        self.game_library_scan = None;
        let mut steam_roots = Vec::new();
        for variable in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(variable) {
                let base = PathBuf::from(base);
                steam_roots.push(base.join("Steam"));
                steam_roots.push(base.join("steam"));
            }
        }
        if let Some(home) = dirs::home_dir() {
            steam_roots.push(home.join(".steam").join("steam"));
            steam_roots.push(home.join(".local").join("share").join("Steam"));
        }
        let epic_manifests = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join(r"Epic\EpicGamesLauncher\Data\Manifests");
        let worker = self.worker(ctx);
        std::thread::spawn(move || {
            for (key, value) in [
                (r"HKCU:\Software\Valve\Steam", "SteamPath"),
                (r"HKLM:\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
                (r"HKLM:\SOFTWARE\Valve\Steam", "InstallPath"),
            ] {
                if let Some(RegData::Str(path) | RegData::Expand(path)) = WinSystem.reg_get(key, value) {
                    steam_roots.push(PathBuf::from(path));
                }
            }
            worker.send(Msg::GameLibraries(gaming::scan_libraries(&steam_roots, &epic_manifests)));
        });
    }

    fn export_game_preference(&mut self, profile: &GameProfile) {
        let name = std::path::Path::new(&profile.executable).file_stem()
            .map_or_else(|| "game".into(), |stem| stem.to_string_lossy().to_string());
        let Some(path) = rfd::FileDialog::new().add_filter("Optimaxer graphics preset", &["json"])
            .set_file_name(format!("{name}-graphics.json")).save_file() else { return };
        if let Err(error) = gaming::write_export(&path, profile) {
            self.events.push(OptiEvent::Toast(error));
        }
    }

    fn import_game_preference(&mut self, profile: &GameProfile) {
        if !profile.can_remove() {
            self.events.push(OptiEvent::Toast("Restore the active setting before importing another graphics preference.".into()));
            return;
        }
        let Some(path) = rfd::FileDialog::new().add_filter("Optimaxer graphics preset", &["json"]).pick_file() else { return };
        let preference = match gaming::read_export(&path) {
            Ok(preference) => preference,
            Err(error) => {
                self.events.push(OptiEvent::Toast(error));
                return;
            }
        };
        let mut updated = self.game_profiles.clone();
        match gaming::set_preference(&gaming::store_path(), &mut updated, &profile.executable, preference) {
            Ok(()) => {
                self.game_profiles = updated;
                self.game_states_loaded = false;
            }
            Err(error) => self.events.push(OptiEvent::Toast(error)),
        }
    }

    fn preview_game_apply(&mut self, ctx: &egui::Context, executable: String) {
        let store_path = gaming::store_path();
        self.spawn(ctx, "Reading per-game graphics preference", move |worker| {
            let result = gaming::load_store(&store_path).and_then(|profiles| {
                let profile = profiles.iter().find(|profile| profile.executable.eq_ignore_ascii_case(&executable))
                    .ok_or_else(|| "Game profile no longer exists. Refresh the page.".to_string())?;
                let (current, desired) = gaming::preview(&WinSystem, profile)?;
                worker.log(&format!("Preview for {executable}: {} -> {}", current.as_ref().map_or("not set".into(), RegData::show), desired.as_ref().map_or("Windows default".into(), RegData::show)));
                Ok(Pending {
                    title: "Set Windows per-game graphics preference".into(),
                    intro: "This changes the current Windows user's graphics preference for this executable. Start the game again for Windows and the driver to use the new preference. It does not guarantee higher performance.".into(),
                    lines: vec![
                        Line { text: format!("Executable: {executable}"), warn: false, head: false },
                        Line { text: format!("Current: {}", current.as_ref().map_or("not set".into(), RegData::show)), warn: false, head: false },
                        Line { text: format!("Requested: {}", desired.as_ref().map_or("Windows default (remove override)".into(), RegData::show)), warn: false, head: false },
                    ],
                    action: Action::ApplyGameGraphics { executable: executable.clone(), expected_current: current },
                    restore_option: false,
                    ok_label: "Apply preference".into(),
                })
            });
            match result {
                Ok(plan) => worker.send(Msg::Plan(plan)),
                Err(error) => worker.send(Msg::Toast(format!("Could not prepare graphics preference: {error}"))),
            }
        });
    }

    fn preview_game_undo(&mut self, ctx: &egui::Context, executable: String) {
        let store_path = gaming::store_path();
        self.spawn(ctx, "Preparing graphics preference restore", move |worker| {
            let result = gaming::load_store(&store_path).and_then(|profiles| {
                let profile = profiles.iter().find(|profile| profile.executable.eq_ignore_ascii_case(&executable))
                    .ok_or_else(|| "Game profile no longer exists. Refresh the page.".to_string())?;
                let (current, original) = gaming::undo_preview(&WinSystem, profile)?;
                Ok(Pending {
                    title: "Restore original Windows graphics preference".into(),
                    intro: "Optimaxer will restore the exact registry value captured before the profile was applied. If it was already restored, this only clears the completed recovery record.".into(),
                    lines: vec![
                        Line { text: format!("Executable: {executable}"), warn: false, head: false },
                        Line { text: format!("Current: {}", current.as_ref().map_or("not set".into(), RegData::show)), warn: false, head: false },
                        Line { text: format!("Original: {}", original.as_ref().map_or("not set".into(), RegData::show)), warn: false, head: false },
                    ],
                    action: Action::UndoGameGraphics { executable: executable.clone() },
                    restore_option: false,
                    ok_label: "Restore original".into(),
                })
            });
            match result {
                Ok(plan) => worker.send(Msg::Plan(plan)),
                Err(error) => worker.send(Msg::Toast(format!("Could not prepare graphics restore: {error}"))),
            }
        });
    }

    fn gaming_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "Gaming", "Review the existing reversible Windows gaming settings. These switches do not promise higher frame rates.");
        self.refresh_game_states(ctx);

        let supported: Vec<_> = self.cat.tweaks.iter().filter(|tw| tw.has_tag("gaming") && tw.retired.is_empty() && !tw.has_tag("optin")).collect();
        let Some(traits) = self.traits.as_ref() else {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Reading this PC’s hardware and current settings…").color(t.muted));
            });
            return;
        };
        if !self.status_ready {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Checking the current Windows settings…").color(t.muted));
            });
            return;
        }

        let to_apply: Vec<String> = hw::preset_ids(&self.cat, traits, "gaming").into_iter()
            .filter(|id| !self.status.get(id).copied().unwrap_or(false)).collect();
        let to_undo: Vec<String> = supported.iter().filter(|tw| self.status.get(&tw.id).copied().unwrap_or(false))
            .map(|tw| tw.id.clone()).collect();
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Windows gaming setting status").font(FontId::new(T_BODY, semibold())).color(t.text));
                ui.label(RichText::new("Registry configuration only; this does not measure live Game Bar or capture activity.").size(T_SMALL).color(t.muted));
            });
            ui.add_space(S1);
            ui.horizontal_wrapped(|ui| {
                for (label, id) in [
                    ("Game Mode", "game-mode"),
                    ("Game DVR capture", "game-dvr"),
                    ("DVR machine policy", "game-dvr-policy"),
                    ("Global fullscreen override", "game-fso"),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).size(T_SMALL).color(t.text));
                        let current = self.status.get(id).copied();
                        let tone = match current {
                            Some(true) => Tone::Good,
                            Some(false) => Tone::Warn,
                            None => Tone::Bad,
                        };
                        kit::chip(ui, setting_match_label(current), tone);
                    });
                    ui.add_space(S2);
                }
            });
        });
        ui.add_space(S2);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                kit::chip(ui, &format!("{} settings", supported.len()), Tone::Brand);
                ui.label(RichText::new("Current state is read from Windows. Incompatible settings stay visible with the reason they were skipped.").size(T_SMALL).color(t.muted));
            });
            ui.add_space(S2);
            for tw in &supported {
                let applied = self.status.get(&tw.id).copied().unwrap_or(false);
                let compatible = hw::fits(tw, traits);
                egui::Frame::new().inner_margin(Margin::symmetric(S2 as i8, S1 as i8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let details_width = (ui.available_width() - 96.0).max(120.0);
                    ui.horizontal(|ui| {
                        ui.scope(|ui| {
                            ui.set_max_width(details_width);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&tw.name).font(FontId::new(T_BODY, semibold())).color(t.text));
                                ui.add(egui::Label::new(RichText::new(&tw.desc).size(T_CAPTION).color(t.muted)).wrap());
                            });
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if applied {
                                kit::chip(ui, "Applied", Tone::Good);
                            } else if !compatible {
                                kit::chip(ui, "Not applicable", Tone::Neutral);
                            } else {
                                kit::chip(ui, "Available", Tone::Neutral);
                            }
                        });
                    });
                    if !compatible && let Some(reason) = hw::fit_reason(tw, traits) {
                        ui.label(RichText::new(reason).size(T_SMALL).color(t.subtle));
                    }
                });
                ui.separator();
            }
        });
        ui.add_space(S2);
        ui.horizontal_wrapped(|ui| {
            let ready = self.busy.is_none() && self.elevated;
            if kit::primary(ui, "Review compatible settings", ready && !to_apply.is_empty()).clicked() {
                self.preview_apply_ids(ctx, to_apply);
            }
            if kit::secondary(ui, "Review undo", ready && !to_undo.is_empty()).clicked() {
                self.preview_undo_ids(to_undo);
            }
            if !self.elevated {
                ui.label(RichText::new("Administrator access is required to change Windows settings.").size(T_SMALL).color(t.muted));
            }
        });

        ui.add_space(S4);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Per-game graphics preferences").font(FontId::new(T_HEADING, semibold())).color(t.text));
            if kit::ghost(ui, "Refresh", !self.game_status_loading && self.game_profiles_error.is_none()).clicked() {
                self.game_states.clear();
                self.game_states_loaded = false;
                self.refresh_game_states(ctx);
            }
            if kit::secondary(ui, "Add game…", self.game_profiles_error.is_none()).clicked() {
                self.add_game_executable(ctx);
            }
        });
        ui.label(RichText::new("Choose a Windows GPU preference for a game executable. Optimaxer stores the exact prior registry value and refuses to overwrite a later change made elsewhere.").size(T_SMALL).color(t.muted));
        ui.horizontal(|ui| {
            if kit::secondary(ui, if self.game_library_scanning { "Scanning libraries…" } else { "Find installed games" }, !self.game_library_scanning).clicked() {
                self.scan_game_libraries(ctx);
            }
            ui.label(RichText::new("Reads Steam and Epic manifests; it does not inspect game files or change launcher data.").size(T_SMALL).color(t.subtle));
        });
        if let Some(scan) = self.game_library_scan.clone() {
            if scan.games.is_empty() {
                ui.label(RichText::new("No installed games found in the detected Steam and Epic libraries.").size(T_SMALL).color(t.muted));
            } else {
                ui.label(RichText::new(format!("{} installed games found", scan.games.len())).size(T_SMALL).color(t.muted));
                let mut selected_library: Option<PathBuf> = None;
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                    for game in &scan.games {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&game.name).size(T_BODY).color(t.text));
                                    kit::chip(ui, game.source.label(), Tone::Neutral);
                                });
                                ui.add(egui::Label::new(RichText::new(game.install_dir.display().to_string()).size(T_CAPTION).color(t.subtle)).truncate());
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if kit::ghost(ui, "Choose executable…", self.game_profiles_error.is_none()).clicked() {
                                    selected_library = Some(game.install_dir.clone());
                                }
                            });
                        });
                        ui.separator();
                    }
                });
                if let Some(directory) = selected_library {
                    self.add_game_executable_from(ctx, Some(&directory));
                }
            }
            for warning in &scan.warnings {
                ui.label(RichText::new(warning).size(T_CAPTION).color(t.warn));
            }
        }
        if let Some(error) = &self.game_profiles_error {
            ui.add_space(S2);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                kit::chip(ui, "Profiles unavailable", Tone::Bad);
                ui.add(egui::Label::new(RichText::new(error).size(T_SMALL).color(t.bad)).wrap());
            });
        } else if self.game_profiles.is_empty() {
            ui.add_space(S2);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("No game executables added.").font(FontId::new(T_BODY, semibold())).color(t.text));
                ui.label(RichText::new("Add a game's .exe file to create a per-user Windows graphics preference. No setting changes until you review and confirm it.").size(T_SMALL).color(t.muted));
            });
        } else if self.game_status_loading || !self.game_states_loaded {
            ui.add_space(S2);
            ui.label(RichText::new("Reading current Windows graphics preferences…").size(T_SMALL).color(t.muted));
        } else {
            let profile_rows = self.game_profiles.clone();
            for profile in profile_rows {
                let state_result = self.game_states.get(&profile.executable);
                let state = state_result.and_then(|result| result.as_ref().ok()).copied();
                let state_error = state_result.and_then(|result| result.as_ref().err()).cloned();
                let preference_history = game_preference_history(&self.operations, &profile.executable);
                let mut requested_action: Option<bool> = None;
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            let name = std::path::Path::new(&profile.executable).file_stem().map_or_else(|| "Game".into(), |name| name.to_string_lossy().to_string());
                            ui.label(RichText::new(name).font(FontId::new(T_BODY, semibold())).color(t.text));
                            ui.add(egui::Label::new(RichText::new(&profile.executable).size(T_SMALL).color(t.subtle)).wrap());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if state_error.is_some() {
                                kit::chip(ui, "Unavailable", Tone::Bad);
                            } else if let Some(state) = state {
                                let (label, tone) = match state {
                                    ProfileState::WindowsDefault => ("Windows default", Tone::Neutral),
                                    ProfileState::Selected if profile.has_saved_original() => ("Applied by Optimaxer", Tone::Good),
                                    ProfileState::Selected => ("Already selected", Tone::Good),
                                    ProfileState::Available => ("Not applied", Tone::Neutral),
                                    ProfileState::RestorePending => ("Restore pending", Tone::Warn),
                                    ProfileState::ChangedElsewhere => ("Changed elsewhere", Tone::Warn),
                                };
                                kit::chip(ui, label, tone);
                            } else {
                                kit::chip(ui, "Checking", Tone::Neutral);
                            }
                        });
                    });
                    ui.add_space(S1);
                    ui.horizontal_wrapped(|ui| {
                        let mut preference = profile.preference;
                        egui::ComboBox::from_id_salt(("game-gpu", &profile.executable))
                            .selected_text(preference.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut preference, GraphicsPreference::Windows, GraphicsPreference::Windows.label());
                                ui.selectable_value(&mut preference, GraphicsPreference::PowerSaving, GraphicsPreference::PowerSaving.label());
                                ui.selectable_value(&mut preference, GraphicsPreference::HighPerformance, GraphicsPreference::HighPerformance.label());
                            });
                        if preference != profile.preference {
                            let mut updated = self.game_profiles.clone();
                            match gaming::set_preference(&gaming::store_path(), &mut updated, &profile.executable, preference) {
                                Ok(()) => {
                                    self.game_profiles = updated;
                                    self.game_states_loaded = false;
                                }
                                Err(error) => self.events.push(OptiEvent::Toast(error)),
                            }
                        }
                        if state == Some(ProfileState::Available) && kit::primary(ui, "Review", self.busy.is_none()).clicked() {
                            requested_action = Some(true);
                        }
                        if profile.has_saved_original()
                            && matches!(state, Some(ProfileState::Selected | ProfileState::RestorePending))
                            && kit::secondary(ui, "Restore original", self.busy.is_none()).clicked()
                        {
                            requested_action = Some(false);
                        }
                        if kit::ghost(ui, "Import preset", profile.can_remove()).clicked() {
                            self.import_game_preference(&profile);
                        }
                        if kit::ghost(ui, "Export preset", true).clicked() {
                            self.export_game_preference(&profile);
                        }
                        if profile.can_remove() && kit::ghost(ui, "Remove", true).clicked() {
                            let mut updated = self.game_profiles.clone();
                            match gaming::remove(&gaming::store_path(), &mut updated, &profile.executable) {
                                Ok(()) => {
                                    self.game_profiles = updated;
                                    self.game_states.remove(&profile.executable);
                                    self.game_states_loaded = false;
                                }
                                Err(error) => self.events.push(OptiEvent::Toast(error)),
                            }
                        }
                    });
                    if let Some(error) = state_error {
                        ui.add(egui::Label::new(RichText::new(error).size(T_SMALL).color(t.bad)).wrap());
                    }
                    if state == Some(ProfileState::ChangedElsewhere) {
                        ui.add(egui::Label::new(RichText::new("Saved originals are protected. Restore either Optimaxer's selected value or the original Windows value, then refresh to retry.").size(T_SMALL).color(t.warn)).wrap());
                    }
                });
                if let Some(apply) = requested_action {
                    if apply {
                        self.preview_game_apply(ctx, profile.executable.clone());
                    } else {
                        self.preview_game_undo(ctx, profile.executable.clone());
                    }
                }
                egui::CollapsingHeader::new(format!(
                    "Per-game history · {}",
                    preference_history.len()
                ))
                    .id_salt(("game-history", &profile.executable))
                    .show(ui, |ui| {
                        if preference_history.is_empty() {
                            ui.label(
                                RichText::new("No graphics preference changes recorded for this executable.")
                                    .size(T_SMALL)
                                    .color(t.muted),
                            );
                        } else {
                            for entry in preference_history.iter().take(10) {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(&entry.time)
                                            .size(T_CAPTION)
                                            .monospace()
                                            .color(t.subtle),
                                    );
                                    ui.label(
                                        RichText::new(&entry.title).size(T_SMALL).color(t.text),
                                    );
                                    let tone = match entry.status {
                                        OperationStatus::Warnings => Tone::Warn,
                                        OperationStatus::Error => Tone::Bad,
                                        OperationStatus::Finished | OperationStatus::Unknown => {
                                            Tone::Neutral
                                        }
                                    };
                                    kit::chip(ui, entry.status.label(), tone);
                                });
                                for detail in entry
                                    .details
                                    .iter()
                                    .filter(|detail| !detail.starts_with("Executable: "))
                                {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(detail).size(T_CAPTION).color(t.muted),
                                        )
                                        .wrap(),
                                    );
                                }
                                ui.separator();
                            }
                        }
                    });
                ui.add_space(S1);
            }
        }
    }

    fn tweaks_page(&mut self, ui: &mut Ui, ctx: &egui::Context, settings: &mut Settings) {
        let t = tokens(ctx);
        kit::page_header(ui, "Tweaks", "Performance, gaming, network, privacy and interface settings. The original value is saved before each change, so every tweak can be undone.");
        let (on, total) = self.traits.as_ref().map(|tr| score(&self.cat, tr, &self.status)).unwrap_or((0, 0));
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let draw_stats = |ui: &mut Ui| {
                if total > 0 {
                    let pct = (100.0 * on as f32 / total as f32).round();
                    let col = if pct >= 80.0 { t.good } else if pct >= 45.0 { t.text } else { t.bad };
                    ui.label(RichText::new(format!("{pct:.0}%")).font(FontId::new(theme::T_DISPLAY + 8.0, semibold())).color(col));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("of the recommended tweaks are active").font(FontId::new(T_BODY, semibold())).color(t.text));
                        ui.label(RichText::new(format!("{on} of {total} fit this PC")).size(T_SMALL).color(t.muted));
                    });
                } else {
                    kit::skeleton(ui, 2);
                }
            };
            let ready = self.busy.is_none() && self.traits.is_some();
            let mut draw_action = |ui: &mut Ui| {
                if kit::primary(ui, "Smart optimize", ready).on_hover_text("Reads your hardware (laptop or desktop, SSD or HDD, RAM, GPU) and selects only the tweaks that fit this PC").clicked() {
                    self.select_preset("auto");
                    if !self.selected.is_empty() {
                        self.preview_apply(ctx);
                    }
                }
            };
            if ui.available_width() < 520.0 {
                ui.horizontal(|ui| draw_stats(ui));
                ui.add_space(S2);
                ui.horizontal(|ui| draw_action(ui));
            } else {
                ui.horizontal(|ui| {
                    draw_stats(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| draw_action(ui));
                });
            }
        });
        ui.add_space(S3);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Presets").size(T_SMALL).color(t.muted));
            for (label, tag) in [("Auto-detect", "auto"), ("Recommended", "safe"), ("Gaming", "gaming"), ("Privacy", "privacy"), ("Remove AI and debloat", "deai"), ("Maximum", "max"), ("Clear", "none")] {
                if kit::secondary(ui, label, true).clicked() {
                    self.select_preset(tag);
                }
            }
        });
        self.profiles_section(ui);
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.tw_search);
            self.toolbar_search(ui, &mut q, "Search tweaks");
            self.tw_search = q;
            if kit::ghost(ui, "Refresh", self.busy.is_none()).clicked() {
                self.refresh_status(ctx);
            }
        });
        ui.add_space(S2);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Risk").size(T_SMALL).color(t.muted));
            kit::segmented(ui, &mut self.tw_risk, &["All", "Safe", "Moderate", "Advanced"]);
            ui.add_space(S3);
            ui.label(RichText::new("Show").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-group").selected_text(TWEAK_GROUPS[self.tw_group.min(TWEAK_GROUPS.len() - 1)]).show_ui(ui, |ui| {
                for (i, g) in TWEAK_GROUPS.iter().enumerate() {
                    ui.selectable_value(&mut self.tw_group, i, *g);
                }
            });
            ui.label(RichText::new("State").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-state").selected_text(["All", "Applied", "Not applied"][self.tw_state.min(2)]).show_ui(ui, |ui| {
                for (i, label) in ["All", "Applied", "Not applied"].iter().enumerate() {
                    ui.selectable_value(&mut self.tw_state, i, *label);
                }
            });
            ui.label(RichText::new("View").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-view").selected_text(["All tweaks", "Recommended", "Favorites"][self.tw_view.min(2)]).show_ui(ui, |ui| {
                for (i, label) in ["All tweaks", "Recommended", "Favorites"].iter().enumerate() {
                    ui.selectable_value(&mut self.tw_view, i, *label);
                }
            });
            ui.label(RichText::new("Compatibility").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-compatibility").selected_text(["All PCs", "Fits this PC", "Does not fit"][self.tw_compatibility.min(2)]).show_ui(ui, |ui| {
                for (i, label) in ["All PCs", "Fits this PC", "Does not fit"].iter().enumerate() {
                    ui.selectable_value(&mut self.tw_compatibility, i, *label);
                }
            });
            ui.label(RichText::new("Sort").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-sort").selected_text(["Category", "Name", "Risk", "Status"][self.tw_sort.min(3)]).show_ui(ui, |ui| {
                for (i, label) in ["Category", "Name", "Risk", "Status"].iter().enumerate() {
                    ui.selectable_value(&mut self.tw_sort, i, *label);
                }
            });
            let filters_active = self.tw_risk != 0 || self.tw_group != 0 || self.tw_state != 0
                || self.tw_view != 0 || self.tw_compatibility != 0 || self.tw_sort != 0;
            if filters_active && kit::ghost(ui, "Reset filters", true).clicked() {
                self.tw_risk = 0;
                self.tw_group = 0;
                self.tw_state = 0;
                self.tw_view = 0;
                self.tw_compatibility = 0;
                self.tw_sort = 0;
            }
        });
        ui.add_space(S2);

        let q = self.tw_search.trim().to_lowercase();
        let risk_filter = ["", "Safe", "Moderate", "Advanced"][self.tw_risk.min(3)];
        let group_filter = self.tw_group;
        let mut cats: Vec<String> = Vec::new();
        for tw in &self.cat.tweaks {
            if !cats.contains(&tw.cat) {
                cats.push(tw.cat.clone());
            }
        }
        if self.tw_sort == 1 {
            cats.sort_by_key(|category| category.to_lowercase());
        }
        let cat = self.cat.clone();
        let traits = self.traits.clone();
        let favorites: HashSet<String> = settings.opti_tweak_favorites.iter().cloned().collect();
        let mut shown = 0;
        for c in cats {
                let mut rows: Vec<&Tweak> = cat.tweaks.iter().filter(|tw| {
                    // a retired tweak is listed only while it has a saved record, so it can still be undone
                    let applied = self.status.get(&tw.id).copied().unwrap_or(false);
                    let fits = traits.as_ref().is_none_or(|tr| hw::fits(tw, tr));
                    (tw.retired.is_empty() || applied)
                        && tw.cat == c.as_str()
                        && (q.is_empty() || tw.name.to_lowercase().contains(&q) || tw.desc.to_lowercase().contains(&q))
                        && (risk_filter.is_empty() || tw.risk == risk_filter)
                        && in_group(tw, group_filter, applied)
                        && tweak_matches_view(tw, applied, fits, favorites.contains(&tw.id), self.tw_state, self.tw_view, self.tw_compatibility)
                }).collect();
                match self.tw_sort {
                    1 => rows.sort_by_cached_key(|tweak| tweak.name.to_lowercase()),
                    2 => rows.sort_by_cached_key(|tweak| (risk_order(&tweak.risk), tweak.name.to_lowercase())),
                    3 => rows.sort_by_cached_key(|tweak| {
                        (std::cmp::Reverse(self.status.get(&tweak.id).copied().unwrap_or(false)), tweak.name.to_lowercase())
                    }),
                    _ => {}
                }
                if rows.is_empty() {
                    continue;
                }
                shown += rows.len();
                egui::CollapsingHeader::new(RichText::new(format!("{c}   {}", rows.len())).font(FontId::new(T_BODY, semibold())).color(t.brand_text)).id_salt(c).default_open(true).show(ui, |ui| {
                    for tw in rows {
                        let applied = self.status.get(&tw.id).copied().unwrap_or(false);
                        let fits = traits.as_ref().is_none_or(|tr| hw::fits(tw, tr));
                        let on = self.selected.contains(&tw.id);
                        let open = self.tw_open.contains(&tw.id);
                        let mut details_rect = egui::Rect::NOTHING;
                        let mut favorite_rect = egui::Rect::NOTHING;
                        let favorite = settings.opti_tweak_favorites.iter().any(|id| id == &tw.id);
                        let r = kit::row(ui, ("tw", &tw.id), on, |ui| {
                            ui.horizontal(|ui| {
                                kit::check(ui, on);
                                ui.vertical(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(&tw.name).font(FontId::new(T_BODY, semibold())).color(t.text));
                                        let favorite_label = ui.add(egui::Label::new(RichText::new(if favorite { "Favorited" } else { "Favorite" }).size(T_CAPTION).color(if favorite { t.brand_text } else { t.subtle })).sense(Sense::click()))
                                            .on_hover_text(if favorite { "Remove from favorites" } else { "Add to favorites" });
                                        favorite_rect = favorite_label.rect;
                                        if !tw.risk.is_empty() && tw.risk != "Safe" {
                                            kit::chip(ui, &tw.risk, risk_tone(&tw.risk));
                                        }
                                        if applied {
                                            kit::chip(ui, "Applied", Tone::Good);
                                        }
                                        if !fits {
                                            kit::chip(ui, "Does not suit this PC", Tone::Neutral);
                                        }
                                    });
                                    ui.add(egui::Label::new(RichText::new(&tw.desc).size(T_SMALL).color(t.muted)).wrap());
                                    let notes = tweak_notes(tw);
                                    if !notes.is_empty() {
                                        ui.label(RichText::new(notes).size(T_CAPTION).color(t.subtle));
                                    }
                                    if let Some(why) = self.tw_skip.get(&tw.id) {
                                        ui.add(egui::Label::new(RichText::new(format!("Smart optimize did not tick this: {why}")).size(T_CAPTION).color(t.warn)).wrap());
                                    }
                                    let toggle = ui.add(egui::Label::new(RichText::new(if open { "Hide technical details" } else { "Technical details" }).size(T_CAPTION).color(t.brand_text)).sense(Sense::click()));
                                    details_rect = toggle.rect;
                                    if open {
                                        for line in tweak_technical(tw) {
                                            ui.add(egui::Label::new(RichText::new(line).monospace().size(T_CAPTION).color(t.muted)).wrap());
                                        }
                                    }
                                });
                            });
                        });
                        let click = ui.interact(r.response.rect, ui.scope_id().with(("tw-click", &tw.id)), Sense::click());
                        if click.hovered() {
                            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if click.has_focus() {
                            ui.painter().rect_stroke(r.response.rect, CornerRadius::same(t.ctl_radius()), Stroke::new(2.0, t.ring()), StrokeKind::Inside);
                        }
                        if click.clicked() && ctx.pointer_interact_pos().is_some_and(|p| favorite_rect.contains(p)) {
                            let previous_favorites = settings.opti_tweak_favorites.clone();
                            if favorite {
                                settings.opti_tweak_favorites.retain(|id| id != &tw.id);
                            } else {
                                settings.opti_tweak_favorites.push(tw.id.clone());
                                settings.opti_tweak_favorites.sort();
                                settings.opti_tweak_favorites.dedup();
                            }
                            if let Err(error) = settings.save_checked() {
                                settings.opti_tweak_favorites = previous_favorites;
                                let message = format!("Favorite could not be saved: {error}");
                                self.say(&message);
                                self.events.push(OptiEvent::Toast(message));
                            }
                        } else if click.clicked() && ctx.pointer_interact_pos().is_some_and(|p| details_rect.contains(p)) {
                            if !self.tw_open.remove(&tw.id) {
                                self.tw_open.insert(tw.id.clone());
                            }
                        } else if click.clicked() {
                            if on {
                                self.selected.remove(&tw.id);
                            } else {
                                if !tw.conflict.is_empty() {
                                    // one choice per group: ticking this unticks the other tweaks that write the same setting
                                    let peers: Vec<String> = cat.tweaks.iter().filter(|p| p.conflict == tw.conflict && p.id != tw.id).map(|p| p.id.clone()).collect();
                                    for p in peers {
                                        self.selected.remove(&p);
                                    }
                                }
                                self.selected.insert(tw.id.clone());
                            }
                        }
                    }
                });
        }
        if shown == 0 {
            kit::empty_state(ui, "No tweaks match", "Try a shorter search or reset the filters.");
        }
        if !cat.not_ported.is_empty() && q.is_empty() {
            ui.add_space(S4);
            ui.add(egui::Label::new(RichText::new(format!("{} tweaks exist only in the PowerShell edition of Optimaxer because they need cmdlets such as Appx or MMAgent: {}", cat.not_ported.len(), cat.not_ported.iter().map(|s| format!("{} ({})", s.name, s.id)).collect::<Vec<_>>().join(", "))).size(T_CAPTION).color(t.subtle)).wrap());
        }
    }

    fn services_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        if !self.services_loaded && self.busy.is_none() {
            self.refresh_services(ctx);
        }
        kit::page_header(ui, "Services", "Optimaxer's recommended Manual and Disabled lists. Core Windows services are hidden. The current start types are saved to a backup before every change.");
        let idle = self.busy.is_none();
        let compact = ui.available_width() < 520.0;
        let mut clear_selection = false;
        let mut restore_services = false;
        let mut refresh_services = false;
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.svc_search);
            let search_width = if compact { 110.0 } else { 260.0 };
            let search_hint = if compact { "Search" } else { "Search services" };
            ui.add_sized([search_width, 34.0], egui::TextEdit::singleline(&mut q).hint_text(search_hint).margin(Margin::symmetric(S3 as i8, 8)));
            self.svc_search = q;
            if kit::ghost(ui, "Select recommended", true).clicked() {
                self.svc_sel = self.services.iter().filter(|s| s.rec.as_ref().is_some_and(|r| r.0 == 3)).map(|s| s.name.clone()).collect();
            }
            if compact {
                ui.menu_button("More", |ui| {
                    if ui.add_enabled(!self.svc_sel.is_empty(), egui::Button::new("Clear selection")).clicked() {
                        clear_selection = true;
                        ui.close();
                    }
                    if ui.add_enabled(idle, egui::Button::new("Restore last backup")).clicked() {
                        restore_services = true;
                        ui.close();
                    }
                    if ui.add_enabled(idle, egui::Button::new("Refresh")).clicked() {
                        refresh_services = true;
                        ui.close();
                    }
                });
            } else {
                if kit::ghost(ui, "Clear", !self.svc_sel.is_empty()).clicked() {
                    self.svc_sel.clear();
                }
                if !self.svc_sel.is_empty() {
                    ui.label(RichText::new(format!("{} selected", self.svc_sel.len())).size(T_SMALL).color(t.muted));
                }
            }
        });
        if clear_selection {
            self.svc_sel.clear();
        }
        if restore_services && idle && self.need_admin() {
            self.pending = Some(Pending { title: "Restore services".into(), intro: "Put every service back to the start type saved in the most recent backup?".into(), lines: vec![], action: Action::RestoreServices, restore_option: false, ok_label: "Restore services".into() });
        }
        if refresh_services && idle {
            self.refresh_services(ctx);
        }
        ui.horizontal_wrapped(|ui| {
            let names: Vec<String> = self.svc_sel.iter().cloned().collect();
            let any = idle && !names.is_empty();
            for (label, start) in [("Set to Manual", 3u32), ("Disable", 4), ("Set to Automatic", 2)] {
                if kit::secondary(ui, label, any).clicked() && self.need_admin() {
                    let n = names.len();
                    self.pending = Some(Pending {
                        title: label.to_string(),
                        intro: format!("Change the start type of {n} service{}? Their current start types are saved first, so you can restore them.", if n == 1 { "" } else { "s" }),
                        lines: names.iter().map(|nm| Line { text: nm.clone(), warn: start == 4, head: false }).collect(),
                        action: Action::SetServices { names: names.clone(), start },
                        restore_option: false,
                        ok_label: format!("{label} ({n})"),
                    });
                }
            }
            if !compact {
                if kit::ghost(ui, "Restore last backup", idle).clicked() && self.need_admin() {
                    self.pending = Some(Pending { title: "Restore services".into(), intro: "Put every service back to the start type saved in the most recent backup?".into(), lines: vec![], action: Action::RestoreServices, restore_option: false, ok_label: "Restore services".into() });
                }
                if kit::ghost(ui, "Refresh", idle).clicked() {
                    self.refresh_services(ctx);
                }
            }
        });
        ui.add_space(S2);
        if self.services.is_empty() {
            if self.busy.is_some() {
                kit::skeleton(ui, 6);
            } else {
                kit::empty_state(ui, "No services loaded", "Press Refresh to read the service list.");
            }
            return;
        }
        let q = self.svc_search.trim().to_lowercase();
        let mut last_group = "";
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for s in &self.services {
                if !q.is_empty() && !s.name.to_lowercase().contains(&q) && !s.display.to_lowercase().contains(&q) && !s.desc.to_lowercase().contains(&q) {
                    continue;
                }
                let group = match s.rec.as_ref().map(|r| r.0) {
                    Some(3) => "Recommended: set to Manual",
                    Some(4) => "Recommended: disable",
                    _ => "Other services",
                };
                if group != last_group {
                    kit::group_title(ui, group);
                    last_group = group;
                }
                let on = self.svc_sel.contains(&s.name);
                let r = kit::row(ui, ("svc", &s.name), on, |ui| {
                    ui.horizontal(|ui| {
                        kit::check(ui, on);
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(if s.display.contains(s.name.as_str()) { s.display.clone() } else { format!("{}  ({})", s.display, s.name) }).font(FontId::new(T_BODY, semibold())).color(t.text));
                                kit::chip(ui, &format!("{}, {}", s.mode(), if s.running { "running" } else { "stopped" }), if s.running { Tone::Brand } else { Tone::Neutral });
                            });
                            let desc = s.rec.as_ref().map(|r| r.1.as_str()).filter(|d| !d.is_empty()).unwrap_or(&s.desc);
                            if !desc.is_empty() {
                                let shown: String = if desc.chars().count() > 170 { desc.chars().take(167).collect::<String>() + "..." } else { desc.to_string() };
                                ui.add(egui::Label::new(RichText::new(shown).size(T_SMALL).color(t.muted)).wrap());
                            }
                        });
                    });
                });
                let click = ui.interact(r.response.rect, ui.scope_id().with(("svc-click", &s.name)), Sense::click());
                if click.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if click.clicked() {
                    if on {
                        self.svc_sel.remove(&s.name);
                    } else {
                        self.svc_sel.insert(s.name.clone());
                    }
                }
            }
        });
    }

    fn startup_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        if !self.startup_loaded && self.busy.is_none() {
            self.refresh_startup(ctx);
        }
        kit::page_header(ui, "Startup", "Apps and third-party tasks that start with Windows. Turning one off changes its startup state; nothing is uninstalled.");
        let idle = self.busy.is_none();
        let key = |i: &startup::Item| format!("{}|{}|{}", i.scope, i.task_path.as_deref().unwrap_or(""), i.name);
        let picked: Vec<startup::Item> = self.startup.iter().filter(|i| self.st_sel.contains(&key(i))).cloned().collect();
        ui.horizontal_wrapped(|ui| {
            for (label, enable) in [("Turn off selected", false), ("Turn on selected", true)] {
                if kit::secondary(ui, label, idle && !picked.is_empty()).clicked() && self.need_admin() {
                    let n = picked.len();
                    self.pending = Some(Pending {
                        title: label.into(),
                        intro: format!("{} {n} startup item{}? Machine-wide entries need administrator rights.", if enable { "Turn on" } else { "Turn off" }, if n == 1 { "" } else { "s" }),
                        lines: picked.iter().map(|i| Line { text: format!("{}  ({})", i.name, i.scope), warn: false, head: false }).collect(),
                        action: Action::SetStartup { items: picked.clone(), enable },
                        restore_option: false,
                        ok_label: format!("{label} ({n})"),
                    });
                }
            }
            if kit::ghost(ui, "Refresh", idle).clicked() {
                self.refresh_startup(ctx);
            }
            if !picked.is_empty() {
                ui.label(RichText::new(format!("{} selected", picked.len())).size(T_SMALL).color(t.muted));
            }
        });
        ui.add_space(S2);
        if let Some(warning) = &self.startup_warning {
            ui.label(RichText::new(warning).size(T_SMALL).color(t.warn));
            ui.add_space(S2);
        }
        if self.startup.is_empty() {
            if self.busy.is_some() {
                kit::skeleton(ui, 5);
            } else {
                kit::empty_state(ui, "No startup items found", "Nothing in Run keys, Startup folders, or third-party logon and boot tasks. Press Refresh to look again.");
            }
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for i in &self.startup {
                let k = key(i);
                let on = self.st_sel.contains(&k);
                let r = kit::row(ui, ("st", &k), on, |ui| {
                    ui.horizontal(|ui| {
                        kit::check(ui, on);
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&i.name).font(FontId::new(T_BODY, semibold())).color(t.text));
                                kit::chip(ui, if i.enabled { "On" } else { "Off" }, if i.enabled { Tone::Good } else { Tone::Neutral });
                                kit::chip(ui, i.scope, Tone::Neutral);
                            });
                            ui.add(egui::Label::new(RichText::new(&i.cmd).monospace().size(T_CAPTION).color(t.subtle)).truncate());
                        });
                    });
                });
                let click = ui.interact(r.response.rect, ui.scope_id().with(("st-click", &k)), Sense::click());
                if click.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if click.clicked() {
                    if on {
                        self.st_sel.remove(&k);
                    } else {
                        self.st_sel.insert(k);
                    }
                }
            }
        });
    }

    fn cleaner_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "Cleaner", "Scan first, then clean what you tick. Directory links are never followed, and deleted files do not go to the Recycle Bin.");
        let idle = self.busy.is_none();
        let total: u64 = self.clean_sizes.values().flatten().sum();
        let picked: Vec<&'static str> = cleaner::TARGETS.iter().map(|t| t.id).filter(|id| self.clean_sel.contains(id)).collect();
        let preview_id = if picked.len() == 1 { picked.first().copied() } else { None };
        ui.horizontal_wrapped(|ui| {
            if kit::secondary(ui, "Scan", idle).clicked() {
                self.scan_cleaner(ctx);
            }
            let label = if picked.is_empty() { "Clean selected".to_string() } else { format!("Clean {} item{}", picked.len(), if picked.len() == 1 { "" } else { "s" }) };
            if kit::primary(ui, &label, idle && !picked.is_empty()).clicked() && self.need_admin() {
                let lines = picked
                    .iter()
                    .filter_map(|id| cleaner::TARGETS.iter().find(|t| t.id == *id))
                    .map(|t| Line { text: format!("{}   {}", t.name, self.clean_sizes.get(t.id).copied().flatten().map(|s| format_size(Some(s))).unwrap_or_default()), warn: t.risk != "Safe", head: false })
                    .collect();
                self.pending = Some(Pending { title: "Clean".into(), intro: "Delete the selected items? This cannot be undone.".into(), lines, action: Action::Clean(picked.clone()), restore_option: false, ok_label: format!("Delete {} item{}", picked.len(), if picked.len() == 1 { "" } else { "s" }) });
            }
            if kit::secondary(ui, "Preview paths", idle && preview_id.is_some()).clicked() {
                self.preview_cleaner(ctx, preview_id.unwrap());
            }
            if total > 0 {
                ui.label(RichText::new(format!("{} can be reclaimed", format_size(Some(total)))).font(FontId::new(T_BODY, semibold())).color(t.good));
            } else if self.clean_sizes.is_empty() {
                ui.label(RichText::new("Scan to see how much each item can free.").size(T_SMALL).color(t.muted));
            }
        });
        if let Some((id, preview)) = &self.clean_preview
            && let Some(target) = cleaner::TARGETS.iter().find(|target| target.id == *id)
        {
            ui.add_space(S2);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Path preview · {}", target.name)).font(FontId::new(T_BODY, semibold())));
                    if preview.truncated { kit::chip(ui, "List truncated", Tone::Warn); }
                });
                ui.add(egui::Label::new(RichText::new(preview.summary).size(T_SMALL).color(t.muted)).wrap());
                if preview.unreadable_directories > 0 {
                    ui.label(RichText::new(format!("{} directories could not be read; preview may be incomplete.", preview.unreadable_directories)).size(T_SMALL).color(t.warn));
                }
                if preview.files.is_empty() {
                    ui.label(RichText::new("No accessible file paths found for this target.").size(T_SMALL).color(t.subtle));
                } else {
                    ui.label(RichText::new(format!("{} files listed · size can change before confirmation", preview.files.len())).size(T_CAPTION).color(t.subtle));
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for file in &preview.files {
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new(RichText::new(&file.path).monospace().size(T_CAPTION).color(t.text)).truncate()).on_hover_text(&file.path);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(format_size(Some(file.bytes))).size(T_CAPTION).color(t.muted));
                                });
                            });
                        }
                    });
                }
            });
        }
        if let Some((id, error)) = &self.clean_preview_error
            && cleaner::TARGETS.iter().any(|target| target.id == *id)
        {
            ui.label(RichText::new(format!("Path preview failed: {error}")).size(T_SMALL).color(t.bad));
        }
        ui.add_space(S2);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for category in cleaner::Category::ALL {
                let targets: Vec<_> = cleaner::TARGETS.iter().filter(|target| cleaner::category(target) == category).collect();
                if targets.is_empty() { continue; }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(category.label()).font(FontId::new(T_SMALL, semibold())).color(t.subtle));
                    ui.label(RichText::new(targets.len().to_string()).size(T_CAPTION).color(t.muted));
                });
                for target in targets {
                    self.cleaner_row(ui, ctx, target);
                }
            }
        });
    }

    fn cleaner_row(&mut self, ui: &mut Ui, ctx: &egui::Context, tg: &Target) {
        let t = tokens(ctx);
        let on = self.clean_sel.contains(tg.id);
        let r = kit::row(ui, ("cl", tg.id), on, |ui| {
            ui.horizontal(|ui| {
                kit::check(ui, on);
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(tg.name).font(FontId::new(T_BODY, semibold())).color(t.text));
                        if tg.risk != "Safe" {
                            kit::chip(ui, tg.risk, risk_tone(tg.risk));
                        }
                    });
                    ui.add(egui::Label::new(RichText::new(tg.desc).size(T_SMALL).color(t.muted)).wrap());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let text = match self.clean_sizes.get(tg.id) {
                        Some(Some(0)) => "Empty".to_string(),
                        Some(Some(s)) => format_size(Some(*s)),
                        Some(None) => "Sized when cleaned".to_string(),
                        None => String::new(),
                    };
                    ui.label(RichText::new(text).font(FontId::new(T_BODY, semibold())).color(t.muted));
                });
            });
        });
        let click = ui.interact(r.response.rect, ui.scope_id().with(("cl-click", tg.id)), Sense::click());
        if click.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if click.clicked() {
            if on {
                self.clean_sel.remove(tg.id);
            } else {
                self.clean_sel.insert(tg.id);
            }
        }
    }

    fn network_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "Network", "Switch DNS, compare resolver responses, flush the cache, or reset the network stack.");
        let idle = self.busy.is_none();
        ui.horizontal_wrapped(|ui| {
            if kit::secondary(ui, "Show current DNS", idle).clicked() {
                self.spawn(ctx, "Reading DNS settings", |w| {
                    w.send(Msg::Adapters(network::active_adapters()));
                    network::show_dns(&WinSystem, &mut |l| w.log(l));
                });
            }
            if kit::secondary(ui, "Flush DNS cache", idle).clicked() {
                self.spawn_recorded(ctx, "Flush DNS", |w| {
                    let result = WinSystem.run("ipconfig.exe", &["/flushdns"]);
                    if result.code == 0 {
                        w.log("DNS cache flushed.");
                    } else {
                        w.log(&format!("WARN DNS cache flush failed: {}", result.text.trim()));
                    }
                });
            }
            if kit::secondary(ui, "Benchmark servers", idle).clicked() {
                let data = self.data.clone();
                self.spawn(ctx, "Benchmarking DNS servers", move |w| {
                    let mut rows = Vec::new();
                    for p in data.dns.iter().filter(|p| !p.v4.is_empty()) {
                        let result = network::benchmark_dns(&p.v4[0], 5);
                        let timing = result.average_ms.map(|ms| format!("{ms:.1} ms average")).unwrap_or_else(|| "no valid DNS answers".into());
                        let jitter = result.jitter_ms.map(|ms| format!("{ms:.1} ms jitter")).unwrap_or_else(|| "jitter unavailable".into());
                        w.log(&format!("  {}: {timing}, {jitter}, {}/{} answers", p.name, result.answers, result.attempts));
                        rows.push((p.name.clone(), result));
                    }
                    if let Some((name, ms)) = rows.iter().filter_map(|(name, result)| result.average_ms.map(|ms| (name, ms))).min_by(|a, b| a.1.total_cmp(&b.1)) {
                        w.log(&format!("Lowest measured DNS query average: {name} ({ms:.1} ms). This does not predict ping to game servers."));
                    }
                    w.send(Msg::Bench(rows));
                });
            }
            if kit::ghost(ui, "Reset network stack", idle).clicked() && self.need_admin() {
                self.pending = Some(Pending { title: "Reset network stack".into(), intro: "Reset Winsock and the IP stack? You may briefly lose connection and need to restart Windows.".into(), lines: vec![], action: Action::ResetNetwork, restore_option: false, ok_label: "Reset network stack".into() });
            }
        });
        ui.add_space(S1);
        ui.label(RichText::new("Sends five UDP A-record lookups for example.com directly to each provider's first IPv4 server; DNS settings are not changed. Jitter is mean absolute change between consecutive successful response times. Failed probes are not IP packet-loss measurements.").size(T_CAPTION).color(t.subtle));
        if !self.bench.is_empty() {
            ui.label(RichText::new("Resolver response time does not predict latency to a game server.").size(T_CAPTION).color(t.subtle));
        }
        if !self.adapters.is_empty() {
            ui.label(RichText::new(format!("Active adapters: {}", self.adapters.join(", "))).size(T_CAPTION).color(t.subtle));
        }
        kit::group_title(ui, "DNS provider");
        let providers = self.data.dns.clone();
        ui.horizontal_wrapped(|ui| {
            for p in providers {
                let ms = self.bench.iter().find(|b| b.0 == p.name).map(|(_, result)| {
                    let average = result.average_ms.map(|m| format!("{m:.1} ms avg")).unwrap_or_else(|| "No DNS answer".into());
                    let jitter = result.jitter_ms.map(|m| format!(" · {m:.1} ms jitter")).unwrap_or_default();
                    format!("{average}{jitter} · {}/{}", result.answers, result.attempts)
                });
                if kit::tile(ui, vec2(252.0, 84.0), &p.name, &p.desc, ms.as_deref(), idle, false).clicked() && self.need_admin() {
                    self.pending = Some(Pending {
                        title: format!("Use {} for DNS", p.name),
                        intro: format!("Set DNS to {} on all active network adapters?", p.name),
                        lines: p.v4.iter().chain(p.v6.iter()).map(|s| Line { text: s.clone(), warn: false, head: false }).collect(),
                        action: Action::SetDns(p.clone()),
                        restore_option: false,
                        ok_label: format!("Use {}", p.name),
                    });
                }
            }
        });
    }

    fn tools_page(&mut self, ui: &mut Ui, ctx: &egui::Context, suggestions: &[AppItem]) {
        let t = tokens(ctx);
        kit::page_header(ui, "Tools", "Maintenance helpers. Anything that changes the system asks first.");
        let idle = self.busy.is_none();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let size = vec2(268.0, 100.0);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Saved backup integrity").font(FontId::new(T_BODY, semibold())).color(t.text));
                        ui.add(egui::Label::new(RichText::new("Checks saved tweak originals and service backup files without changing them.").size(T_SMALL).color(t.muted)).wrap());
                    });
                    if kit::secondary(ui, if self.busy.as_deref() == Some("Checking saved backups") { "Checking…" } else { "Check backups" }, idle).clicked() {
                        self.check_saved_backups(ctx);
                    }
                });
                if let Some(checks) = &self.backup_integrity {
                    ui.add_space(S1);
                    for check in checks {
                        let (label, tone) = match check.status {
                            BackupStatus::Good => ("Readable", Tone::Good),
                            BackupStatus::Missing => ("Not found", Tone::Neutral),
                            BackupStatus::Warning => ("Check needed", Tone::Warn),
                        };
                        ui.horizontal_wrapped(|ui| {
                            kit::chip(ui, label, tone);
                            ui.label(RichText::new(check.title).font(FontId::new(T_SMALL, semibold())).color(t.text));
                            ui.add(egui::Label::new(RichText::new(&check.detail).size(T_CAPTION).color(if check.status == BackupStatus::Warning { t.warn } else { t.muted })).wrap());
                        });
                    }
                } else {
                    ui.label(RichText::new("Readability checks cannot guarantee Windows will accept every restore operation.").size(T_CAPTION).color(t.subtle));
                }
            });
            ui.add_space(S3);
            ui.horizontal_wrapped(|ui| {
                for info in tools::TOOLS.iter() {
                    if kit::tile(ui, size, info.title, info.desc, None, idle, false).clicked() {
                        if info.changes_system {
                            if self.need_admin() {
                                self.pending = Some(Pending { title: info.title.into(), intro: info.desc.into(), lines: vec![], action: Action::RunTool(info.tool), restore_option: false, ok_label: info.title.into() });
                            }
                        } else {
                            let tool = info.tool;
                            self.spawn_recorded(ctx, info.title, move |w| tools::run(&WinSystem, tool, &backup_dir(), &mut |l| w.log(l)));
                        }
                    }
                }
                if kit::tile(ui, size, "Open System Restore", "Roll your PC back to a restore point.", None, true, false).clicked()
                    && let Err(error) = std::process::Command::new("rstrui.exe").spawn() {
                    self.events.push(OptiEvent::Toast(format!("Could not open System Restore: {error}")));
                }
                let vendor = self.traits.as_ref().map(|tr| tr.vendor.clone()).unwrap_or_default();
                let url = hw::vendor_url(&vendor);
                let (title, desc) = match url {
                    Some(_) => ("Vendor driver page".to_string(), format!("Opens the official {vendor} support page in your browser. Optimaxer downloads and installs nothing.")),
                    None => ("Vendor driver page".to_string(), "No MSI, Acer or Gigabyte board detected. Open your PC maker's support site instead.".to_string()),
                };
                if kit::tile(ui, size, &title, &desc, None, url.is_some(), false).clicked()
                    && let Some(u) = url
                    && let Err(error) = open::that(u) {
                    self.events.push(OptiEvent::Toast(format!("Could not open the driver support page: {error}")));
                }
                if kit::tile(ui, size, "Undo all Optimaxer tweaks", "Restores every setting Optimaxer changed to its original value.", None, idle, true).clicked() && self.need_admin() {
                    self.pending = Some(Pending { title: "Undo all tweaks".into(), intro: "Restore every setting changed by Optimaxer to its original value?".into(), lines: vec![], action: Action::UndoAll, restore_option: false, ok_label: "Undo all tweaks".into() });
                }
            });
            kit::group_title(ui, "PC tools from the Orion catalog");
            ui.add(egui::Label::new(RichText::new("Utility and Customization apps for Windows. Downloading stays your decision: open an app, pick a build and press Download.").size(T_SMALL).color(t.muted)).wrap());
            ui.add_space(S2);
            ui.horizontal(|ui| {
                if kit::secondary(ui, "Browse Utility apps", true).clicked() {
                    self.events.push(OptiEvent::OpenStoreCategory("Utility".into()));
                }
                if kit::secondary(ui, "Browse Customization apps", true).clicked() {
                    self.events.push(OptiEvent::OpenStoreCategory("Customization".into()));
                }
            });
            ui.add_space(S2);
            ui.horizontal_wrapped(|ui| {
                for a in suggestions {
                    if kit::ghost(ui, &a.name, true).on_hover_text(&a.description).clicked() {
                        self.events.push(OptiEvent::OpenApp(a.id.clone()));
                    }
                }
            });
            ui.add_space(S5);
        });
    }
    // endregion
}

/// Everything the confirmation window shows, as plain text for the clipboard.
fn confirm_text(p: &Pending) -> String {
    let mut out = format!("{}
{}
", p.title, p.intro);
    for l in &p.lines {
        if l.head {
            out.push_str(&format!("
{}
", l.text));
        } else {
            out.push_str(&format!("  {}
", l.text));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::fake::FakeSystem;
    use crate::opti::backend::RegData;

    fn tr(laptop: bool) -> Traits {
        let gpus = vec!["NVIDIA GeForce RTX 3090".to_string()];
        Traits { laptop, ssd: true, ram_gb: 32.0, threads: 16, tags: hw::tags_for(laptop, true, true, 32.0, 16, &gpus, 26100), gpus, ..Default::default() }
    }

    #[test]
    fn every_navigation_page_belongs_to_one_visible_sidebar_group() {
        assert_eq!(Page::ALL.len(), 13);
        for (group, expected_count) in PageGroup::ALL.into_iter().zip([4, 6, 3]) {
            assert_eq!(Page::ALL.into_iter().filter(|page| page.group() == group).count(), expected_count, "{}", group.label());
        }
    }

    #[test]
    fn score_counts_only_recommended_tweaks_that_fit() {
        let cat = Catalog::load();
        let t = tr(false);
        let (_, total) = score(&cat, &t, &HashMap::new());
        assert!(total >= 10);
        let all: HashMap<String, bool> = cat.tweaks.iter().map(|t| (t.id.clone(), true)).collect();
        assert_eq!(score(&cat, &t, &all), (total, total));
        assert_eq!(score(&cat, &t, &HashMap::new()).0, 0);
        let (_, laptop_total) = score(&cat, &tr(true), &HashMap::new());
        assert!(laptop_total <= total, "desktop-only tweaks must not count on a laptop");
    }

    #[test]
    fn storage_review_uses_available_capacity_and_ninety_percent_threshold() {
        use crate::opti::sysmon::Snapshot;

        let mut snapshot = Snapshot { disk_used: 89, disk_total: 100, disk_available: true, ..Default::default() };
        assert!(!storage_needs_review(&snapshot));
        snapshot.disk_used = 90;
        assert!(storage_needs_review(&snapshot));
        snapshot.disk_available = false;
        assert!(!storage_needs_review(&snapshot));
        snapshot.disk_available = true;
        snapshot.disk_total = 0;
        assert!(!storage_needs_review(&snapshot));
    }

    #[test]
    fn activity_search_matches_dates_actions_names_ids_and_change_details() {
        let entry = HistoryEntry {
            time: "2026-10-29T12:30:00Z".into(),
            action: HistoryAction::Apply,
            changes: vec![HistoryChange {
                id: "disable-copilot".into(),
                name: "Disable Copilot".into(),
                verified: true,
                undo_available: true,
                details: vec!["registry value changed".into()],
            }],
        };
        for query in [
            "copilot",
            "disable-copilot",
            "2026-10-29",
            "apply",
            "registry value",
        ] {
            assert!(history_entry_matches(&entry, query), "should match {query}");
        }
        assert!(!history_entry_matches(&entry, "service"));
        assert!(history_entry_matches(&entry, "  "));
    }

    #[test]
    fn plan_lines_show_only_what_would_change() {
        let cat = Catalog::load();
        let sys = FakeSystem::default();
        let t = cat.find("perf-shutdown").unwrap();
        for r in &t.reg {
            sys.put(&r.p, &r.n, r.wanted());
        }
        let eng = Engine::new(&sys, PathBuf::new());
        let done = plan_lines(&eng, &[t]);
        assert!(done.iter().any(|l| l.text.contains("already in effect")));
        sys.put(&t.reg[0].p, &t.reg[0].n, RegData::Str("20000".into()));
        let todo = plan_lines(&eng, &[t]);
        assert!(todo.iter().any(|l| l.text.contains("20000") && l.text.contains("5000")));
    }

    #[test]
    fn batch_refreshes_other_tweaks_that_share_a_setting() {
        use crate::opti::catalog::{RegItem, RegVal};
        let path = r"HKCU:\Software\OptimaxerBatchTest";
        let first = Tweak { id: "first".into(), name: "First".into(), reg: vec![RegItem {
            p: path.into(), n: "Choice".into(), v: RegVal::Num(1), t: "DWord".into(),
        }], ..Default::default() };
        let mut second = first.clone();
        second.id = "second".into();
        second.reg[0].v = RegVal::Num(2);
        let cat = Catalog { tweaks: vec![first, second], ..Default::default() };
        let sys = FakeSystem::default();
        sys.put(path, "Choice", RegData::Dword(2));
        let dir = std::env::temp_dir().join(format!("opti-batch-status-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::load_from(dir.join("state.json"), None);
        let eng = Engine::new(&sys, PathBuf::new());
        let batch = run_batch(&eng, &cat, &["first".into()], true, false, &mut state, &mut |_| {});
        assert_eq!(batch.now.get("first"), Some(&true));
        assert_eq!(batch.now.get("second"), Some(&false));
        assert!(batch.changes[0].verified && batch.changes[0].undo_available);
        assert!(batch.changes[0].details.iter().any(|line| line.contains("2 -> 1")));
        let undone = run_batch(&eng, &cat, &["first".into()], false, false, &mut state, &mut |_| {});
        assert!(undone.changes[0].verified && !undone.changes[0].undo_available);
        assert!(undone.changes[0].details[0].contains("Saved originals: 1 registry values"));
        assert_eq!(sys.reg_get(path, "Choice"), Some(RegData::Dword(2)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn batch_requests_restart_only_after_a_state_transition() {
        use crate::opti::catalog::{RegItem, RegVal};
        let path = r"HKCU:\Software\OptimaxerRestartTest";
        let tweak = Tweak {
            id: "restart-test".into(),
            name: "Restart test".into(),
            reg: vec![RegItem { p: path.into(), n: "Enabled".into(), v: RegVal::Num(1), t: "DWord".into() }],
            restart: true,
            explorer: true,
            ..Default::default()
        };
        let cat = Catalog { tweaks: vec![tweak], ..Default::default() };
        let sys = FakeSystem::default();
        sys.put(path, "Enabled", RegData::Dword(0));
        let dir = std::env::temp_dir().join(format!("opti-restart-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::load_from(dir.join("state.json"), None);
        let engine = Engine::new(&sys, PathBuf::new());

        let applied = run_batch(&engine, &cat, &["restart-test".into()], true, false, &mut state, &mut |_| {});
        assert!(applied.changes[0].verified && applied.restart && applied.explorer);

        let unchanged = run_batch(&engine, &cat, &["restart-test".into()], true, false, &mut state, &mut |_| {});
        assert!(unchanged.changes[0].verified);
        assert!(!unchanged.restart && !unchanged.explorer, "reapplying an active tweak must not request restarts");

        let undone = run_batch(&engine, &cat, &["restart-test".into()], false, false, &mut state, &mut |_| {});
        assert!(undone.changes[0].verified && undone.restart && undone.explorer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn partial_tweak_change_requests_restart_without_restarting_explorer() {
        use crate::opti::catalog::{RegItem, RegVal};
        let denied_path = r"HKCU:\Software\OptimaxerRestartFailureTestDenied";
        let writable_path = r"HKCU:\Software\OptimaxerRestartFailureTestWritable";
        let tweak = Tweak {
            id: "restart-failure-test".into(),
            name: "Restart failure test".into(),
            reg: vec![
                RegItem { p: denied_path.into(), n: "Enabled".into(), v: RegVal::Num(1), t: "DWord".into() },
                RegItem { p: writable_path.into(), n: "Enabled".into(), v: RegVal::Num(1), t: "DWord".into() },
            ],
            restart: true,
            explorer: true,
            ..Default::default()
        };
        let cat = Catalog { tweaks: vec![tweak], ..Default::default() };
        let sys = FakeSystem::default();
        sys.put(denied_path, "Enabled", RegData::Dword(0));
        sys.put(writable_path, "Enabled", RegData::Dword(0));
        sys.deny_writes.borrow_mut().insert(denied_path.to_lowercase());
        let dir = std::env::temp_dir().join(format!("opti-restart-failure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::load_from(dir.join("state.json"), None);
        let engine = Engine::new(&sys, PathBuf::new());

        let failed = run_batch(&engine, &cat, &["restart-failure-test".into()], true, false, &mut state, &mut |_| {});
        assert!(!failed.changes[0].verified && failed.restart && !failed.explorer);
        assert_eq!(sys.reg_get(writable_path, "Enabled"), Some(RegData::Dword(1)));
        assert!(!sys.ran("taskkill.exe"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_restore_point_applies_nothing() {
        use crate::opti::backend::fake::FakeSystem;
        let sys = FakeSystem::default();
        sys.fail.borrow_mut().insert("powershell.exe".into());
        let cat = Catalog::load();
        let d = std::env::temp_dir().join(format!("opti-ui-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut st = State::load_from(d.join("state.json"), None);
        let eng = Engine::new(&sys, PathBuf::new());
        let ids = vec!["ui-ext".to_string(), "game-mode".to_string()];
        let b = run_batch(&eng, &cat, &ids, true, true, &mut st, &mut |_| {});
        assert!(b.restore_failed && b.now.is_empty());
        assert!(st.records.is_empty(), "a tweak was applied after the restore point failed");
        assert!(sys.values.borrow().is_empty(), "the registry was written");
        let b = run_batch(&eng, &cat, &ids, true, false, &mut st, &mut |_| {});
        assert!(!b.restore_failed && ids.iter().all(|id| b.now.contains_key(id)), "without a restore point the batch runs");
    }

    #[test]
    fn technical_details_list_the_exact_changes() {
        let cat = Catalog::load();
        let t = cat.find("perf-shutdown").unwrap();
        let lines = tweak_technical(t);
        assert!(lines[0].contains("perf-shutdown") && lines[0].contains(&t.risk));
        assert!(lines.iter().any(|l| l.starts_with("registry") && l.contains("WaitToKillAppTimeout") && l.contains("REG_SZ")));
        // every tweak produces at least its header and one more line
        assert!(cat.tweaks.iter().all(|t| tweak_technical(t).len() >= 2));
    }

    #[test]
    fn group_filter_matches_tags_and_applied_state() {
        let cat = Catalog::load();
        let all = cat.tweaks.len();
        for (g, name) in TWEAK_GROUPS.iter().enumerate().take(6) {
            let n = cat.tweaks.iter().filter(|t| in_group(t, g, false)).count();
            assert!(n > 0 && n <= all, "group {name} is empty");
        }
        let t = &cat.tweaks[0];
        assert!(in_group(t, 6, true) && !in_group(t, 6, false) && in_group(t, 7, false));
        assert!(cat.tweaks.iter().filter(|t| in_group(t, 1, false)).all(|t| t.has_tag("safe")));
    }

    #[test]
    fn tweak_filters_combine_state_recommendation_favorites_and_compatibility() {
        let recommended = Tweak { id: "recommended".into(), risk: "Safe".into(), tags: vec!["safe".into()], ..Default::default() };
        let opt_in = Tweak { id: "opt-in".into(), risk: "Safe".into(), tags: vec!["optin".into()], ..Default::default() };

        assert!(tweak_matches_view(&recommended, true, true, false, 1, 1, 1));
        assert!(!tweak_matches_view(&recommended, false, true, false, 1, 1, 1));
        assert!(!tweak_matches_view(&recommended, false, false, false, 0, 1, 0));
        assert!(!tweak_matches_view(&opt_in, false, true, false, 0, 1, 0));
        assert!(tweak_matches_view(&opt_in, false, true, true, 2, 2, 1));
        assert!(!tweak_matches_view(&opt_in, false, false, true, 2, 2, 1));
        assert!(tweak_matches_view(&opt_in, false, false, true, 2, 2, 2));
    }

    #[test]
    fn tweak_risk_sort_has_explicit_unknown_order() {
        assert!(risk_order("Safe") < risk_order("Moderate"));
        assert!(risk_order("Moderate") < risk_order("Advanced"));
        assert!(risk_order("Advanced") < risk_order("Other"));
    }

    #[test]
    fn notes_describe_hardware_gates() {
        let cat = Catalog::load();
        let n = tweak_notes(cat.find("perf-paging").unwrap());
        assert!(n.contains("16 GB+ RAM") && n.contains("restart needed"));
        assert!(tweak_notes(cat.find("perf-ultimate").unwrap()).contains("desktop"));
    }

    #[test]
    fn risk_tones_distinguish_levels() {
        assert_ne!(risk_tone("Safe"), risk_tone("Advanced"));
        assert_eq!(risk_tone(""), risk_tone("Safe"));
        assert_eq!(risk_tone("Moderate"), Tone::Warn);
    }

    #[test]
    fn gaming_status_labels_distinguish_unavailable_from_changed_values() {
        assert_eq!(setting_match_label(Some(true)), "Configured");
        assert_eq!(setting_match_label(Some(false)), "Values differ");
        assert_eq!(setting_match_label(None), "Unavailable");
    }

    #[test]
    fn profile_summary_uses_readable_hardware_names() {
        let summary = profile_summary(&tr(false));
        assert_eq!(summary, "Desktop · NVMe SSD · Windows 11");
        assert!(!summary.contains("ram32") && !summary.contains("dgpu"));
    }

    #[test]
    fn profile_summary_reports_unknown_facts_explicitly() {
        assert_eq!(profile_summary(&Traits::default()), "Form factor unknown · Storage unknown · Windows version unknown");
    }
}
