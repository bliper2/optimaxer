//! Optimaxer section of the window: Tweaks, Services, Startup, Cleaner, Network and Tools pages.
//! Everything that reads or changes Windows runs on a worker thread and reports back through a channel, so the UI
//! never blocks. Every change goes through a confirmation window that lists exactly what will happen.

use crate::kit;
use crate::model::{AppItem, format_size};
use crate::net::{self, Settings};
use crate::opti::backend::{RegData, System, WinSystem};
use crate::opti::backup_integrity::{self, Check as BackupCheck, Status as BackupStatus};
use crate::opti::catalog::{Catalog, Tweak};
use crate::opti::cleaner::{self, Target};
use crate::opti::diagnostics::{self, Check as DiagnosticCheck, Result as DiagnosticResult};
use crate::opti::engine::{Change, Engine};
use crate::opti::gaming::{self, GameProfile, GraphicsPreference, ProfileState};
use crate::opti::history::{
    self, Action as HistoryAction, Change as HistoryChange, Entry as HistoryEntry, OperationEntry,
    OperationStatus,
};
use crate::opti::hw::{self, Traits};
use crate::opti::measurements;
use crate::opti::network;
use crate::opti::processes::{self, CpuTimes, ProcessInfo};
use crate::opti::profiles::{self, Profile};
use crate::opti::services::{self, Data, DnsProvider, SvcRow};
use crate::opti::snapshots::{self, SystemSnapshot};
use crate::opti::startup;
use crate::opti::state::{State, backup_dir};
use crate::opti::tools::{self, Tool};
use crate::opti::tweak_facts;
use crate::theme::{
    self, S1, S2, S3, S4, S5, T_BODY, T_CAPTION, T_HEADING, T_SMALL, T_TITLE, Tone, mix, semibold,
    tokens,
};
use egui::{
    Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, StrokeKind, Ui,
    vec2,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod activity_page;
mod blackbox_page;
mod cleaner_page;
mod detective_page;
mod diagnostics_page;
mod gaming_page;
mod network_page;
mod overview_page;
mod performance_page;
mod processes_page;
mod services_page;
mod startup_page;
mod tools_page;
mod tweaks_page;
mod xray_page;

const LOG_LINES: usize = 600;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Overview,
    SmartOptimize,
    Performance,
    BlackBox,
    Detective,
    Diagnostics,
    Snapshots,
    Processes,
    Gaming,
    Activity,
    Tweaks,
    Services,
    Startup,
    StartupXray,
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
    const ALL: [Page; 17] = [
        Page::Overview,
        Page::SmartOptimize,
        Page::Performance,
        Page::BlackBox,
        Page::Detective,
        Page::Diagnostics,
        Page::Snapshots,
        Page::Processes,
        Page::Gaming,
        Page::Activity,
        Page::Tweaks,
        Page::Services,
        Page::Startup,
        Page::StartupXray,
        Page::Cleaner,
        Page::Network,
        Page::Tools,
    ];
    /// Segoe Fluent Icons glyph shown in the sidebar.
    fn icon(self) -> char {
        match self {
            Page::Overview => '\u{E80F}',
            Page::SmartOptimize => '\u{E945}',
            Page::Performance => '\u{E9D2}',
            Page::BlackBox => '\u{E823}',
            Page::Detective => '\u{E721}',
            Page::Diagnostics => '\u{E72E}',
            Page::Snapshots => '\u{E8D4}',
            Page::Processes => '\u{E9D9}',
            Page::Gaming => '\u{E7FC}',
            Page::Activity => '\u{E81C}',
            Page::Tweaks => '\u{E9E9}',
            Page::Services => '\u{E9F5}',
            Page::Startup => '\u{E7E8}',
            Page::StartupXray => '\u{E946}',
            Page::Cleaner => '\u{E74D}',
            Page::Network => '\u{E774}',
            Page::Tools => '\u{E90F}',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::SmartOptimize => "Smart Optimize",
            Page::Performance => "Performance",
            Page::BlackBox => "Black Box",
            Page::Detective => "Bottleneck Detective",
            Page::Diagnostics => "Diagnostics",
            Page::Snapshots => "System Snapshot",
            Page::Processes => "Processes",
            Page::Gaming => "Gaming",
            Page::Activity => "Activity",
            Page::Tweaks => "Tweaks",
            Page::Services => "Services",
            Page::Startup => "Startup",
            Page::StartupXray => "Startup X-Ray",
            Page::Cleaner => "Cleaner",
            Page::Network => "Network",
            Page::Tools => "Tools",
        }
    }

    fn group(self) -> PageGroup {
        match self {
            Self::Overview
            | Self::SmartOptimize
            | Self::Performance
            | Self::BlackBox
            | Self::Detective
            | Self::Processes
            | Self::Activity => PageGroup::Workspace,
            Self::Gaming
            | Self::Tweaks
            | Self::Services
            | Self::Startup
            | Self::StartupXray
            | Self::Cleaner
            | Self::Network => PageGroup::Optimization,
            Self::Diagnostics | Self::Snapshots | Self::Tools => PageGroup::System,
        }
    }
}

/// Page names compare without case, spaces or punctuation, so "startup x ray" finds "Startup X-Ray".
fn page_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Things the Optimaxer section asks the rest of the app to do.
pub enum OptiEvent {
    CheckForUpdates,
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
    SetServices {
        names: Vec<String>,
        start: u32,
    },
    RestoreServices,
    SetStartup {
        items: Vec<startup::Item>,
        enable: bool,
    },
    Clean(Vec<&'static str>, Vec<String>),
    SetDns(DnsProvider),
    RestoreDns,
    ResetNetwork,
    RunTool(Tool),
    RunInstaller(PathBuf),
    ApplyGameGraphics {
        executable: String,
        expected_current: Option<RegData>,
    },
    UndoGameGraphics {
        executable: String,
    },
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

fn service_dependency_lines(
    targets: &[(String, String)],
    dependencies: &[services::DependencyInfo],
    disabling: bool,
) -> Result<Vec<Line>, String> {
    if targets.len() != dependencies.len()
        || targets
            .iter()
            .zip(dependencies)
            .any(|((name, _), dependency)| !name.eq_ignore_ascii_case(&dependency.service))
    {
        return Err("Service dependency results did not match the selected services.".into());
    }
    let mut lines = Vec::new();
    for ((name, display), dependency) in targets.iter().zip(dependencies) {
        lines.push(Line {
            text: if display.is_empty() || display.eq_ignore_ascii_case(name) {
                name.clone()
            } else {
                format!("{display} ({name})")
            },
            warn: false,
            head: true,
        });
        let requires = if dependency.requires.is_empty() {
            "none".to_string()
        } else {
            dependency
                .requires
                .iter()
                .map(|required| {
                    required
                        .strip_prefix('+')
                        .map(|group| format!("load-order group {group}"))
                        .unwrap_or_else(|| required.clone())
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(Line {
            text: format!("Requires: {requires}"),
            warn: false,
            head: false,
        });
        let required_by = if dependency.required_by.is_empty() {
            "none".to_string()
        } else {
            dependency.required_by.join(", ")
        };
        lines.push(Line {
            text: format!("Required by: {required_by}"),
            warn: disabling && !dependency.required_by.is_empty(),
            head: false,
        });
    }
    Ok(lines)
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
    StartupBoot(Option<(u64, String)>),
    StartupPublisher(String, PathBuf, Result<Option<String>, String>),
    Scan(Vec<(&'static str, Option<u64>)>),
    ScanCancelled,
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
    ProcessPublisher(u32, PathBuf, Result<Option<String>, String>),
    RestartRequired,
    BlackboxEvents(Result<Vec<crate::opti::blackbox::WinEvent>, String>),
    DetectiveProgress(u64),
    XrayHistory(Result<crate::opti::bootlog::History, crate::opti::bootlog::ReadError>),
    XrayDeps(String, Result<Vec<services::DependencyInfo>, String>),
    XrayEvents(Result<Vec<crate::opti::blackbox::WinEvent>, String>),
    DetectiveDone(Result<crate::opti::detective::Evidence, String>),
    BackupIntegrity(Vec<BackupCheck>),
}

#[derive(Clone, Debug, PartialEq)]
struct ProcessInspection {
    pid: u32,
    name: String,
    image_path: Option<PathBuf>,
}

enum ProcessPublisherState {
    Loading,
    Ready(Option<String>),
    Error(String),
}

enum StartupInspectionState {
    Loading(PathBuf),
    Unavailable,
    Ready {
        path: PathBuf,
        company_name: Option<String>,
    },
    Error {
        path: Option<PathBuf>,
        message: String,
    },
}

fn startup_item_key(item: &startup::Item) -> String {
    format!(
        "{}|{}|{}|{}",
        item.scope,
        item.task_path.as_deref().unwrap_or(""),
        item.name,
        item.cmd
    )
}

#[derive(Clone)]
struct Worker {
    tx: Sender<Msg>,
    ctx: egui::Context,
    cancel: Option<Arc<AtomicBool>>,
    operation_details: Option<Arc<Mutex<Vec<String>>>>,
    operation_status: Option<Arc<AtomicU8>>,
}

fn operation_log_severity(line: &str) -> u8 {
    let line = line.trim_start();
    if line.starts_with("ERROR") {
        2
    } else if line.starts_with("WARN") {
        1
    } else {
        0
    }
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

    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
    }
}

fn refresh_game_profile_messages(worker: &Worker, store_path: &std::path::Path) {
    match gaming::load_store(store_path) {
        Ok(profiles) => {
            let states = profiles
                .iter()
                .map(|profile| {
                    (
                        profile.executable.clone(),
                        gaming::state(&WinSystem, profile),
                    )
                })
                .collect();
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
    focus_tweak_search: bool,
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
    /// Saved originals for the Expert view, re-read at most every few seconds.
    tw_records: (
        Option<Instant>,
        std::collections::BTreeMap<String, crate::opti::state::Record>,
    ),
    /// Why Smart optimize left a tweak unticked (risk or the hardware fact). Cleared by the other presets.
    tw_skip: std::collections::BTreeMap<String, String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: Option<String>,
    cancel_current: Option<Arc<AtomicBool>>,
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
    startup_inspection: Option<startup::Item>,
    startup_inspection_state: Option<StartupInspectionState>,
    clean_sizes: HashMap<&'static str, Option<u64>>,
    clean_scan_cancelled: bool,
    clean_sel: HashSet<&'static str>,
    clean_preview: Option<(&'static str, cleaner::Preview)>,
    clean_preview_error: Option<(&'static str, String)>,
    clean_exclusion_input: String,
    clean_exclusion_error: Option<String>,
    adapters: Vec<String>,
    bench: Vec<(String, network::DnsBenchmark)>,
    events: Vec<OptiEvent>,
    /// `ctx.input(|i| i.time)` when the page last changed, for the enter transition.
    page_started: f64,
    telemetry_worker: Option<crate::opti::sysmon::TelemetryWorker>,
    telemetry_frame: crate::opti::sysmon::TelemetryFrame,
    telemetry_config: crate::opti::sysmon::TelemetryConfig,
    telemetry_error: Option<String>,
    telemetry_needs_reset: bool,
    /// Present only while the user has Black Box recording switched on.
    blackbox: Option<crate::opti::blackbox::Recorder>,
    blackbox_view: blackbox_page::View,
    detective: detective_page::State,
    xray: xray_page::State,
    /// Time of the last restart-marker read and the reasons Windows reported.
    restart_reasons: (Option<Instant>, Vec<&'static str>),
    info: Option<crate::opti::sysmon::SysInfo>,
    processes: Vec<ProcessInfo>,
    process_cpu: CpuTimes,
    process_error: Option<String>,
    process_loading: bool,
    process_last_scan: Option<Instant>,
    process_search: String,
    process_sort: usize,
    process_inspection: Option<ProcessInspection>,
    process_publisher: Option<ProcessPublisherState>,
    services_loaded: bool,
    startup_loaded: bool,
    startup_boot_loading: bool,
    startup_boot_record: Option<(u64, String)>,
}

// region pure logic (unit tested)
/// (active, applicable) among the recommended tweaks that fit this PC.
pub fn score(cat: &Catalog, tr: &Traits, status: &HashMap<String, bool>) -> (usize, usize) {
    // the same tweaks Smart optimize would tick on this PC
    let applicable: Vec<&Tweak> = cat
        .tweaks
        .iter()
        .filter(|t| t.auto_pick() && hw::fits(t, tr))
        .collect();
    let on = applicable
        .iter()
        .filter(|t| status.get(&t.id).copied().unwrap_or(false))
        .count();
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

fn tweak_matches_view(
    tweak: &Tweak,
    applied: bool,
    fits: bool,
    favorite: bool,
    state: usize,
    view: usize,
    compatibility: usize,
) -> bool {
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
            HistoryAction::Apply => "apply".contains(&query) || "applied".contains(&query),
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
        || entry
            .details
            .iter()
            .any(|detail| detail.to_lowercase().contains(&query))
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentAction {
    pub title: String,
    pub hint: String,
    pub search: String,
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

fn recent_actions(
    history: &[HistoryEntry],
    operations: &[OperationEntry],
    limit: usize,
) -> Vec<RecentAction> {
    activity_entries(history, operations, "")
        .into_iter()
        .take(limit)
        .map(|entry| match entry {
            ActivityEntry::Tweak(entry) => {
                let action = match entry.action {
                    HistoryAction::Apply => "Applied",
                    HistoryAction::Undo => "Undid",
                };
                let count = entry.changes.len();
                RecentAction {
                    title: format!(
                        "{action} {count} {}",
                        if count == 1 { "tweak" } else { "tweaks" }
                    ),
                    hint: format!("Activity · {}", entry.time),
                    search: entry.time.clone(),
                }
            }
            ActivityEntry::Operation(entry) => RecentAction {
                title: entry.title.clone(),
                hint: format!("{} · {}", entry.status.label(), entry.time),
                search: entry.time.clone(),
            },
        })
        .collect()
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

    #[test]
    fn recent_actions_use_newest_records_and_activity_search_timestamps() {
        let older_tweak = HistoryEntry {
            time: "2026-10-29T12:00:00Z".into(),
            action: HistoryAction::Apply,
            changes: vec![HistoryChange {
                id: "privacy.telemetry".into(),
                name: "Disable telemetry".into(),
                verified: true,
                undo_available: true,
                details: Vec::new(),
            }],
        };
        let newer_operation = OperationEntry {
            time: "2026-10-30T12:00:00Z".into(),
            title: "Reset network stack".into(),
            status: OperationStatus::Warnings,
            details: Vec::new(),
        };

        let recent = recent_actions(&[older_tweak], &[newer_operation], 1);

        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].title, "Reset network stack");
        assert_eq!(recent[0].hint, "Warnings · 2026-10-30T12:00:00Z");
        assert_eq!(recent[0].search, "2026-10-30T12:00:00Z");
    }

    #[test]
    fn recent_tweak_action_label_handles_singular_and_plural() {
        let one = HistoryEntry {
            time: "2026-10-30T12:00:00Z".into(),
            action: HistoryAction::Undo,
            changes: vec![HistoryChange {
                id: "privacy.telemetry".into(),
                name: "Disable telemetry".into(),
                verified: true,
                undo_available: false,
                details: Vec::new(),
            }],
        };
        let two = HistoryEntry {
            time: "2026-10-30T12:01:00Z".into(),
            changes: vec![one.changes[0].clone(), one.changes[0].clone()],
            ..one.clone()
        };

        let recent = recent_actions(&[one, two], &[], 2);

        assert_eq!(recent[0].title, "Undid 2 tweaks");
        assert_eq!(recent[1].title, "Undid 1 tweak");
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
        ui.label(
            RichText::new(format!("Only in {profile_name} · {}", ids.len()))
                .font(FontId::new(T_SMALL, semibold()))
                .color(tokens.muted),
        );
        if ids.is_empty() {
            ui.label(
                RichText::new("No unique tweaks")
                    .size(T_CAPTION)
                    .color(tokens.subtle),
            );
        } else {
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    for id in ids {
                        if let Some(tweak) = catalog.find(&id) {
                            ui.label(
                                RichText::new(&tweak.name)
                                    .size(T_CAPTION)
                                    .color(tokens.text),
                            )
                            .on_hover_text(&tweak.desc);
                        } else {
                            ui.label(
                                RichText::new(format!("Unknown tweak · {id}"))
                                    .size(T_CAPTION)
                                    .color(tokens.warn),
                            );
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
        n.push(format!(
            "retired: {} Undo it to put your old value back",
            t.retired
        ));
    }
    n.join(", ")
}

/// Groups the Tweaks page can be filtered by. The first entry shows everything.
const TWEAK_GROUPS: [&str; 8] = [
    "All",
    "Recommended",
    "Gaming",
    "Privacy",
    "AI and debloat",
    "Opt-in",
    "Applied",
    "Not applied",
];

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
    let mut out = vec![format!(
        "id {}   risk {}   tags {}",
        t.id,
        t.risk,
        if t.tags.is_empty() {
            "none".to_string()
        } else {
            t.tags.join(", ")
        }
    )];
    for r in &t.reg {
        let (value, kind) = match &r.v {
            RegVal::Num(n) => (
                format!("{n} (0x{:X})", n & 0xFFFF_FFFF),
                if r.t.is_empty() {
                    "REG_DWORD".to_string()
                } else {
                    format!("REG_{}", r.t.to_uppercase())
                },
            ),
            RegVal::Text(v) => (
                format!("\"{v}\""),
                if r.t == "ExpandString" {
                    "REG_EXPAND_SZ".to_string()
                } else {
                    "REG_SZ".to_string()
                },
            ),
            RegVal::Bytes(b) => (format!("{} bytes", b.len()), "REG_BINARY".to_string()),
            RegVal::Multi(m) => (m.join(" | "), "REG_MULTI_SZ".to_string()),
        };
        out.push(format!(
            "registry  {}\\{} = {}  [{}]",
            r.p, r.n, value, kind
        ));
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
        out.push(format!(
            "service  {} start type -> {} ({})",
            sv.name, start, sv.start
        ));
    }
    for tk in &t.task {
        out.push(format!("scheduled task  {tk} -> disabled"));
    }
    if t.special.is_some() || t.script.apply {
        out.push(
            "also runs a built-in routine (not only registry, service and task changes)".into(),
        );
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

/// Facts shown under a tweak in the Enthusiast and Expert views: scope, trade-offs, undo and the saved original.
fn tweak_fact_lines(
    t: &Tweak,
    saved: Option<&crate::opti::state::Record>,
    applied: bool,
) -> Vec<String> {
    let (_, how) = tweak_facts::rollback(t);
    let mut out = vec![
        format!(
            "scope  {}   administrator rights {}",
            tweak_facts::scope(t),
            if tweak_facts::needs_admin(t) {
                "required"
            } else {
                "not required"
            }
        ),
        format!("undo  {how}"),
        format!(
            "state at the last check  {}",
            if applied { "applied" } else { "not applied" }
        ),
    ];
    if let Some(special) = &t.special {
        out.push(format!("routine  {}", special.describe()));
    }
    out.extend(
        tweak_facts::tradeoffs(t)
            .into_iter()
            .map(|line| format!("trade-off  {line}")),
    );
    if let Some(record) = saved {
        out.extend(tweak_facts::original_lines(record));
    }
    out.push("Optimaxer does not measure the speed effect of this tweak. Compare with the Black Box or a benchmark.".into());
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
fn run_batch(
    eng: &Engine,
    cat: &Catalog,
    ids: &[String],
    apply: bool,
    restore_point: bool,
    st: &mut State,
    log: &mut dyn FnMut(&str),
) -> Batch {
    let mut b = Batch {
        restore_failed: false,
        ok: 0,
        restart: false,
        explorer: false,
        now: HashMap::new(),
        changes: Vec::new(),
    };
    if apply
        && restore_point
        && !tools::create_restore_point(eng.sys, "Optimaxer - before tweaks", log)
    {
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
        let (on, failed) = if apply {
            (eng.apply(t, st, log), Vec::new())
        } else {
            eng.undo_checked(t, st, log)
        };
        let verified = on == apply && failed.is_empty();
        let changed_state = planned_before != eng.plan(t);
        b.restart |= t.restart && changed_state;
        b.explorer |= t.explorer && verified && changed_state;
        if verified {
            b.ok += 1;
        }
        b.changes.push(HistoryChange {
            id: t.id.clone(),
            name: t.name.clone(),
            verified,
            undo_available: st.contains(id),
            details,
        });
    }
    // A changed value can also change the status of another tweak that uses it.
    b.now = eng.status_all(&cat.tweaks, st);
    b
}

fn plan_lines(eng: &Engine, tweaks: &[&Tweak]) -> Vec<Line> {
    let mut out = Vec::new();
    for t in tweaks {
        out.push(Line {
            text: format!("{}  ({})", t.name, t.risk),
            warn: t.risk == "Advanced",
            head: true,
        });
        let changes = eng.plan(t);
        if changes.is_empty() {
            out.push(Line {
                text: "already in effect, nothing to change".into(),
                warn: false,
                head: false,
            });
        }
        for c in changes {
            out.push(Line {
                warn: matches!(c, Change::Note(_)),
                text: c.describe(),
                head: false,
            });
        }
    }
    out
}
// endregion

fn draw_smart_plan_summary(
    ui: &mut Ui,
    traits: &Traits,
    plan: &hw::Plan,
    active: usize,
    total: usize,
    refreshing: bool,
    t: &theme::Tokens,
) {
    let planned = plan.ids.len();
    let ram = if traits.ram_gb > 0.0 {
        format!("{:.0} GB RAM", traits.ram_gb)
    } else {
        "RAM unavailable".into()
    };
    let threads = if traits.threads > 0 {
        format!("{} threads", traits.threads)
    } else {
        "Processor count unavailable".into()
    };
    let graphics = if traits.gpus.is_empty() {
        "Graphics adapter unavailable".into()
    } else {
        traits.gpus.join(", ")
    };

    kit::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            kit::chip(ui, &profile_summary(traits), Tone::Brand);
            ui.label(
                RichText::new(format!(
                    "{active} of {total} applicable recommendations already active"
                ))
                .size(T_SMALL)
                .color(t.muted),
            );
            if refreshing {
                kit::chip(ui, "Refreshing current state", Tone::Neutral);
            }
        });
        ui.add(
            egui::Label::new(
                RichText::new(format!("{ram} · {threads} · {graphics}"))
                    .size(T_SMALL)
                    .color(t.muted),
            )
            .wrap(),
        );
        ui.add_space(S2);
        if planned == 0 {
            let summary = if total == 0 {
                "No automatic recommendations fit this PC."
            } else if active >= total {
                "All applicable automatic recommendations are active."
            } else {
                "No changes are ready to review. Check the exclusions below."
            };
            ui.label(
                RichText::new(summary)
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.text),
            );
        } else {
            ui.label(
                RichText::new(format!(
                    "{planned} change{} ready for review",
                    if planned == 1 { "" } else { "s" }
                ))
                .font(FontId::new(T_HEADING, semibold()))
                .color(t.text),
            );
            ui.add(
                egui::Label::new(
                    RichText::new("Opt-in and retired tweaks stay excluded. Advanced changes are never selected automatically.")
                        .size(T_SMALL)
                        .color(t.muted),
                )
                .wrap(),
            );
        }
    });
}

fn draw_smart_plan_changes(ui: &mut Ui, cat: &Catalog, plan: &hw::Plan, t: &theme::Tokens) {
    kit::group_title(ui, "Recommended changes");
    if plan.ids.is_empty() {
        kit::empty_state(
            ui,
            "No changes ready to review",
            "This plan selects only inactive recommendations that fit the detected hardware. Open the full catalog to inspect other settings.",
        );
        return;
    }

    for (index, id) in plan.ids.iter().enumerate() {
        if let Some(tweak) = cat.find(id) {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&tweak.name)
                        .font(FontId::new(T_BODY, semibold()))
                        .color(t.text),
                );
                kit::chip(ui, &tweak.risk, risk_tone(&tweak.risk));
                if tweak.restart {
                    kit::chip(ui, "Restart required", Tone::Warn);
                }
                if tweak.partial {
                    kit::chip(ui, "Partial support", Tone::Warn);
                }
            });
            ui.add(
                egui::Label::new(RichText::new(&tweak.desc).size(T_SMALL).color(t.muted)).wrap(),
            );
            ui.label(RichText::new(&tweak.cat).size(T_CAPTION).color(t.subtle));
            if index + 1 < plan.ids.len() {
                ui.add_space(S2);
                ui.separator();
                ui.add_space(S2);
            }
        } else {
            ui.label(
                RichText::new(format!("Catalog entry {id} is unavailable."))
                    .size(T_SMALL)
                    .color(t.bad),
            );
        }
    }
}

fn draw_smart_plan_reasons(ui: &mut Ui, cat: &Catalog, plan: &hw::Plan, t: &theme::Tokens) {
    if !plan.notes.is_empty() {
        ui.add_space(S2);
        egui::CollapsingHeader::new("How this hardware shaped the plan")
            .default_open(false)
            .show(ui, |ui| {
                for note in &plan.notes {
                    ui.add(
                        egui::Label::new(RichText::new(note).size(T_SMALL).color(t.muted)).wrap(),
                    );
                    ui.add_space(S1);
                }
            });
    }
    if !plan.skipped.is_empty() {
        ui.add_space(S1);
        egui::CollapsingHeader::new(format!(
            "Not selected automatically ({})",
            plan.skipped.len()
        ))
        .default_open(false)
        .show(ui, |ui| {
            for (id, reason) in &plan.skipped {
                let name = cat
                    .find(id)
                    .map_or(id.as_str(), |tweak| tweak.name.as_str());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(name)
                            .size(T_SMALL)
                            .font(FontId::new(T_SMALL, semibold()))
                            .color(t.text),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(reason).size(T_SMALL).color(t.muted)).wrap(),
                    );
                });
            }
        });
    }
}

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
        let (profiles, profiles_error) =
            match profiles::load_store(&net::data_dir().join("profiles.json")) {
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
        let (telemetry_worker, telemetry_error) = match crate::opti::sysmon::TelemetryWorker::new()
        {
            Ok(worker) => (Some(worker), None),
            Err(error) => (
                None,
                Some(format!("Live system readings could not start: {error}")),
            ),
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
            focus_tweak_search: false,
            tw_risk: 0,
            tw_group: 0,
            tw_state: 0,
            tw_view: 0,
            tw_compatibility: 0,
            tw_sort: 0,
            tw_open: HashSet::new(),
            tw_records: (None, std::collections::BTreeMap::new()),
            tw_skip: Default::default(),
            tx,
            rx,
            busy: None,
            cancel_current: None,
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
            startup_inspection: None,
            startup_inspection_state: None,
            clean_sizes: HashMap::new(),
            clean_scan_cancelled: false,
            clean_sel: HashSet::new(),
            clean_preview: None,
            clean_preview_error: None,
            clean_exclusion_input: String::new(),
            clean_exclusion_error: None,
            adapters: Vec::new(),
            bench: Vec::new(),
            events: Vec::new(),
            page_started: 0.0,
            telemetry_worker,
            telemetry_frame: crate::opti::sysmon::TelemetryFrame::default(),
            telemetry_config: crate::opti::sysmon::TelemetryConfig::new(1, 60),
            telemetry_error,
            telemetry_needs_reset: true,
            blackbox: None,
            blackbox_view: blackbox_page::View::with_default_range(),
            detective: detective_page::State::default(),
            xray: xray_page::State::default(),
            restart_reasons: (None, Vec::new()),
            info: None,
            processes: Vec::new(),
            process_cpu: CpuTimes::new(),
            process_error: None,
            process_loading: false,
            process_last_scan: None,
            process_search: String::new(),
            process_sort: 0,
            process_inspection: None,
            process_publisher: None,
            services_loaded: false,
            startup_loaded: false,
            startup_boot_loading: false,
            startup_boot_record: None,
        }
    }

    /// Switches page by name. Used by the screenshot runner.
    pub fn show_page(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(p) = Page::ALL
            .iter()
            .copied()
            .find(|p| page_key(p.label()) == page_key(name))
        {
            self.set_page(ctx, p);
        }
    }

    pub fn search_tweaks(&mut self, ctx: &egui::Context, query: &str) {
        self.tw_search = query.to_owned();
        self.focus_tweak_search = true;
        self.set_page(ctx, Page::Tweaks);
    }

    pub fn start_transition(&mut self, ctx: &egui::Context) {
        self.page_started = ctx.input(|i| i.time);
    }

    fn set_page(&mut self, ctx: &egui::Context, p: Page) {
        let live = |page| matches!(page, Page::Overview | Page::Performance);
        if live(p) != live(self.page) {
            if live(p) {
                self.telemetry_needs_reset = true;
            } else {
                self.suspend_telemetry();
            }
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
        let admin = if self.elevated {
            "Administrator"
        } else {
            "Not elevated"
        };
        match &self.busy {
            Some(b) => format!("Working: {b}   ·   {admin}"),
            None => {
                let (on, total) = self
                    .traits
                    .as_ref()
                    .map(|t| score(&self.cat, t, &self.status))
                    .unwrap_or((0, 0));
                format!(
                    "{} tweaks in the catalog  ·  {on} of {total} recommended active  ·  {admin}",
                    self.cat.tweaks.len()
                )
            }
        }
    }

    pub fn operation_entries(&self) -> &[OperationEntry] {
        &self.operations
    }

    pub fn recent_actions(&self, limit: usize) -> Vec<RecentAction> {
        recent_actions(&self.history, &self.operations, limit)
    }

    pub fn search_activity(&mut self, ctx: &egui::Context, query: &str) {
        self.history_search = query.to_owned();
        self.set_page(ctx, Page::Activity);
    }

    pub fn current_job(&self) -> Option<&str> {
        self.busy.as_deref()
    }

    /// Open a downloaded installer from the store through Optimaxer: a restore point first, then the file.
    pub fn request_run_installer(&mut self, path: PathBuf) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.pending = Some(Pending {
            title: "Create a restore point, then open the installer".into(),
            intro: format!(
                "Optimaxer will create a System Restore point and then open {name}. You decide inside the installer whether to continue. Only run files you trust."
            ),
            lines: vec![Line {
                text: path.to_string_lossy().to_string(),
                warn: false,
                head: false,
            }],
            action: Action::RunInstaller(path),
            restore_option: false,
            ok_label: "Create restore point and open".into(),
        });
    }

    fn worker(&self, ctx: &egui::Context) -> Worker {
        Worker {
            tx: self.tx.clone(),
            ctx: ctx.clone(),
            cancel: None,
            operation_details: None,
            operation_status: None,
        }
    }

    /// Starts a background job. One job at a time, like the PowerShell edition.
    fn spawn(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        job: impl FnOnce(&Worker) + Send + 'static,
    ) -> bool {
        self.spawn_inner(ctx, title, false, false, job)
    }

    fn spawn_cancellable(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        job: impl FnOnce(&Worker) + Send + 'static,
    ) -> bool {
        self.spawn_inner(ctx, title, false, true, job)
    }

    fn spawn_recorded(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        job: impl FnOnce(&Worker) + Send + 'static,
    ) -> bool {
        self.spawn_inner(ctx, title, true, false, job)
    }

    fn spawn_inner(
        &mut self,
        ctx: &egui::Context,
        title: &str,
        record: bool,
        cancellable: bool,
        job: impl FnOnce(&Worker) + Send + 'static,
    ) -> bool {
        if self.busy.is_some() {
            self.events.push(OptiEvent::Toast(
                "Another task is running, please wait.".into(),
            ));
            return false;
        }
        self.busy = Some(title.to_string());
        self.say(&format!("== {title}"));
        let cancel = cancellable.then(|| Arc::new(AtomicBool::new(false)));
        self.cancel_current = cancel.clone();
        let operation_details = record.then(|| Arc::new(Mutex::new(Vec::new())));
        let operation_status = record.then(|| Arc::new(AtomicU8::new(0)));
        let w = Worker {
            tx: self.tx.clone(),
            ctx: ctx.clone(),
            cancel: cancel.clone(),
            operation_details: operation_details.clone(),
            operation_status: operation_status.clone(),
        };
        let title = title.to_owned();
        std::thread::spawn(move || {
            if let Err(message) = catch_worker_panic(|| job(&w)) {
                w.log(&format!(
                    "ERROR Background operation stopped unexpectedly: {message}"
                ));
            }
            if let Some(details) = operation_details {
                let details = match details.lock() {
                    Ok(lines) => lines.clone(),
                    Err(_) => {
                        w.log("WARN Operation output could not be captured because its log buffer was unavailable.");
                        Vec::new()
                    }
                };
                let status = match operation_status
                    .as_ref()
                    .map_or(0, |status| status.load(Ordering::Relaxed))
                {
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
                    Err(error) => w.log(&format!(
                        "WARN Operation ran, but its activity record could not be saved: {error}"
                    )),
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
                Msg::Idle => {
                    self.busy = None;
                    self.cancel_current = None;
                }
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
                    self.startup_inspection = None;
                    self.startup_inspection_state = None;
                    self.startup = s.items;
                    self.startup_warning = s.warning;
                    self.startup_loaded = true;
                }
                Msg::StartupBoot(record) => {
                    self.startup_boot_record = record;
                    self.startup_boot_loading = false;
                }
                Msg::StartupPublisher(key, path, result) => {
                    if self
                        .startup_inspection
                        .as_ref()
                        .is_some_and(|item| startup_item_key(item) == key)
                    {
                        self.startup_inspection_state = Some(match result {
                            Ok(company_name) => {
                                StartupInspectionState::Ready { path, company_name }
                            }
                            Err(message) => StartupInspectionState::Error {
                                path: Some(path),
                                message,
                            },
                        });
                    }
                }
                Msg::Scan(v) => {
                    self.clean_scan_cancelled = false;
                    self.clean_sel.clear();
                    for (id, size) in v {
                        let risk_ok = cleaner::TARGETS
                            .iter()
                            .find(|t| t.id == id)
                            .is_some_and(|t| t.risk == "Safe");
                        if size.is_some_and(|s| s > 0) && risk_ok {
                            self.clean_sel.insert(id);
                        }
                        self.clean_sizes.insert(id, size);
                    }
                }
                Msg::ScanCancelled => {
                    self.clean_sizes.clear();
                    self.clean_sel.clear();
                    self.clean_preview = None;
                    self.clean_scan_cancelled = true;
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
                Msg::BlackboxEvents(result) => self.blackbox_view.set_events(result),
                Msg::DetectiveProgress(seconds) => self.detective.progress(seconds),
                Msg::XrayHistory(result) => self.xray.set_history(result),
                Msg::XrayDeps(name, result) => self.xray.set_deps(name, result),
                Msg::XrayEvents(result) => self.xray.set_events(result),
                Msg::DetectiveDone(outcome) => self.detective.finish(outcome),
                Msg::Processes(Ok((rows, cpu_times))) => {
                    if let Some(selected) = self.process_inspection.as_ref() {
                        let latest = rows.iter().find(|process| process.pid == selected.pid);
                        if latest.is_none_or(|process| process.image_path != selected.image_path) {
                            self.process_inspection = None;
                            self.process_publisher = None;
                        } else if let (Some(selected), Some(latest)) =
                            (self.process_inspection.as_mut(), latest)
                        {
                            selected.name.clone_from(&latest.name);
                        }
                    }
                    self.processes = rows;
                    self.process_cpu = cpu_times;
                    self.process_error = None;
                    self.process_loading = false;
                }
                Msg::Processes(Err(error)) => {
                    self.process_error = Some(error);
                    self.process_loading = false;
                }
                Msg::ProcessPublisher(pid, path, result) => {
                    if self.process_inspection.as_ref().is_some_and(|selected| {
                        selected.pid == pid && selected.image_path.as_ref() == Some(&path)
                    }) {
                        self.process_publisher = Some(match result {
                            Ok(name) => ProcessPublisherState::Ready(name),
                            Err(error) => ProcessPublisherState::Error(error),
                        });
                    }
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
            || self
                .process_last_scan
                .is_some_and(|last| last.elapsed() < Duration::from_secs(3))
        {
            return;
        }
        self.process_loading = true;
        self.process_last_scan = Some(Instant::now());
        let previous = self.process_cpu.clone();
        let cores = std::thread::available_parallelism()
            .map(|count| count.get() as u32)
            .unwrap_or(1);
        let worker = self.worker(ctx);
        std::thread::spawn(move || worker.send(Msg::Processes(processes::scan(&previous, cores))));
    }

    fn inspect_process(&mut self, ctx: &egui::Context, process: ProcessInfo) {
        let selected = ProcessInspection {
            pid: process.pid,
            name: process.name,
            image_path: process.image_path,
        };
        let Some(path) = selected.image_path.clone() else {
            self.process_inspection = Some(selected);
            self.process_publisher = Some(ProcessPublisherState::Error(
                "Windows denied access to this executable path.".into(),
            ));
            return;
        };

        let pid = selected.pid;
        self.process_inspection = Some(selected);
        self.process_publisher = Some(ProcessPublisherState::Loading);
        let worker = self.worker(ctx);
        std::thread::spawn(move || {
            worker.send(Msg::ProcessPublisher(
                pid,
                path.clone(),
                processes::company_name(&path),
            ));
        });
    }

    fn inspect_startup(&mut self, ctx: &egui::Context, item: startup::Item) {
        let key = startup_item_key(&item);
        self.startup_inspection = Some(item.clone());
        match startup::resolve_executable(item.executable.as_deref()) {
            Ok(Some(path)) => {
                self.startup_inspection_state = Some(StartupInspectionState::Loading(path.clone()));
                let worker = self.worker(ctx);
                std::thread::spawn(move || {
                    let result = processes::company_name(&path);
                    worker.send(Msg::StartupPublisher(key, path, result));
                });
            }
            Ok(None) => {
                self.startup_inspection_state = Some(StartupInspectionState::Unavailable);
            }
            Err(message) => {
                self.startup_inspection_state = Some(StartupInspectionState::Error {
                    path: None,
                    message,
                });
            }
        }
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
        self.startup_boot_loading = true;
        self.spawn(ctx, "Reading startup items", move |w| {
            let listing = startup::list(&WinSystem);
            w.log(&format!("Found {} startup items.", listing.items.len()));
            if let Some(warning) = &listing.warning {
                w.log(&format!("WARN {warning}"));
            }
            w.send(Msg::Startup(listing));
            w.send(Msg::StartupBoot(measurements::latest_boot_event()));
        });
    }

    fn save_cleaner_exclusions(
        &mut self,
        settings: &mut Settings,
        exclusions: Vec<String>,
    ) -> bool {
        let previous = std::mem::replace(&mut settings.cleaner_exclusions, exclusions);
        if let Err(error) = settings.save_checked() {
            settings.cleaner_exclusions = previous;
            let message = format!("Cleaner exclusions were not saved: {error}");
            self.clean_exclusion_error = Some(message.clone());
            self.say(&message);
            self.events.push(OptiEvent::Toast(message));
            return false;
        }
        self.clean_sizes.clear();
        self.clean_sel.clear();
        self.clean_preview = None;
        self.clean_preview_error = None;
        self.clean_exclusion_error = None;
        true
    }

    fn add_cleaner_exclusion(&mut self, settings: &mut Settings) {
        let path = match cleaner::normalize_exclusion(&self.clean_exclusion_input) {
            Ok(path) => path,
            Err(error) => {
                self.clean_exclusion_error = Some(error);
                return;
            }
        };
        if settings
            .cleaner_exclusions
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&path))
        {
            self.clean_exclusion_error = Some("That path is already excluded.".into());
            return;
        }
        let mut exclusions = settings.cleaner_exclusions.clone();
        exclusions.push(path);
        exclusions.sort_by_key(|path| path.to_lowercase());
        if self.save_cleaner_exclusions(settings, exclusions) {
            self.clean_exclusion_input.clear();
        }
    }

    fn scan_cleaner(&mut self, ctx: &egui::Context, exclusions: Vec<String>) {
        self.clean_sizes.clear();
        self.clean_sel.clear();
        self.clean_preview = None;
        self.clean_preview_error = None;
        self.clean_scan_cancelled = false;
        self.spawn_cancellable(ctx, "Scanning for junk", move |w| {
            let mut sizes = Vec::with_capacity(cleaner::TARGETS.len());
            for target in cleaner::TARGETS {
                match cleaner::scan_cancellable(target, &exclusions, &|| w.is_cancelled()) {
                    Ok(size) => sizes.push((target.id, size)),
                    Err(cleaner::ScanCancelled) => {
                        w.log("Scan cancelled. Partial results were discarded.");
                        w.send(Msg::ScanCancelled);
                        return;
                    }
                }
            }
            if w.is_cancelled() {
                w.log("Scan cancelled. Partial results were discarded.");
                w.send(Msg::ScanCancelled);
                return;
            }
            let total: u64 = sizes.iter().filter_map(|s| s.1).sum();
            w.log(&format!(
                "Scan finished: {} can be reclaimed.",
                format_size(Some(total))
            ));
            w.send(Msg::Scan(sizes));
        });
    }

    fn preview_cleaner(&mut self, ctx: &egui::Context, id: &'static str, exclusions: Vec<String>) {
        let Some(target) = cleaner::TARGETS
            .iter()
            .find(|target| target.id == id)
            .cloned()
        else {
            return;
        };
        self.clean_preview = None;
        self.clean_preview_error = None;
        self.spawn(ctx, "Listing affected paths", move |worker| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cleaner::preview(&target, &exclusions)
            }))
            .map_err(|_| "Path preview could not be completed.".to_string());
            worker.send(Msg::CleanPreview(id, result));
        });
    }

    /// Builds the confirmation for applying tweaks. Reads only; nothing changes until the user confirms.
    fn preview_apply(&mut self, ctx: &egui::Context) {
        let ids: Vec<String> = self
            .cat
            .tweaks
            .iter()
            .filter(|t| self.selected.contains(&t.id))
            .map(|t| t.id.clone())
            .collect();
        self.preview_apply_ids(ctx, ids);
    }

    fn preview_apply_ids(&mut self, ctx: &egui::Context, ids: Vec<String>) {
        if ids.is_empty() {
            self.events
                .push(OptiEvent::Toast("No tweaks selected.".into()));
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
            let mut intro = format!(
                "Apply {} tweak(s)? Each one saves your current values first, so it can be undone.",
                tweaks.len()
            );
            if adv > 0 {
                intro.push_str(&format!(
                    " {adv} Advanced tweak(s) are marked in red: read them."
                ));
            }
            w.send(Msg::Plan(Pending {
                title: "Apply tweaks".into(),
                intro,
                lines: plan_lines(&eng, &tweaks),
                action: Action::ApplyTweaks(ids),
                restore_option: true,
                ok_label: format!(
                    "Apply {} tweak{}",
                    tweaks.len(),
                    if tweaks.len() == 1 { "" } else { "s" }
                ),
            }));
        });
    }

    fn preview_undo(&mut self) {
        let ids: Vec<String> = self
            .cat
            .tweaks
            .iter()
            .filter(|t| self.selected.contains(&t.id))
            .map(|t| t.id.clone())
            .collect();
        self.preview_undo_ids(ids);
    }

    fn preview_undo_ids(&mut self, ids: Vec<String>) {
        if ids.is_empty() {
            self.events
                .push(OptiEvent::Toast("No tweaks selected.".into()));
            return;
        }
        if !self.need_admin() {
            return;
        }
        let lines = ids
            .iter()
            .filter_map(|i| self.cat.find(i))
            .map(|t| Line {
                text: t.name.clone(),
                warn: false,
                head: false,
            })
            .collect();
        let n = ids.len();
        self.pending = Some(Pending {
            title: "Undo tweaks".into(),
            intro: format!(
                "Restore the original values saved for {} tweak(s)? Tweaks that were not applied by Optimaxer have nothing to restore.",
                ids.len()
            ),
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

    fn request_system_tool(&mut self, info: &tools::ToolInfo) {
        if !info.changes_system {
            return;
        }
        if self.need_admin() {
            self.pending = Some(Pending {
                title: info.title.into(),
                intro: info.desc.into(),
                lines: vec![],
                action: Action::RunTool(info.tool),
                restore_option: false,
                ok_label: info.title.into(),
            });
        }
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
                    let n = services::set_start(
                        &WinSystem,
                        &data,
                        &names,
                        start,
                        &backup_dir(),
                        &mut |l| w.log(l),
                    );
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
                self.spawn_recorded(
                    ctx,
                    if enable {
                        "Enabling startup items"
                    } else {
                        "Disabling startup items"
                    },
                    move |w| {
                        let sys = WinSystem;
                        for i in &items {
                            match startup::set_enabled(&sys, i, enable) {
                                Ok(()) => w.log(&format!(
                                    "  {}: {}",
                                    i.name,
                                    if enable { "enabled" } else { "disabled" }
                                )),
                                Err(e) => w.log(&format!("  WARN {}: {e}", i.name)),
                            }
                        }
                        w.send(Msg::Startup(startup::list(&sys)));
                    },
                );
            }
            Action::Clean(ids, exclusions) => {
                self.spawn_recorded(ctx, "Cleaning", move |w| {
                    let sys = WinSystem;
                    let mut sum = 0u64;
                    for id in &ids {
                        let Some(t) = cleaner::TARGETS.iter().find(|t| t.id == *id) else {
                            continue;
                        };
                        let freed = cleaner::clean(&sys, t, &exclusions, &mut |l| w.log(l));
                        sum += freed;
                        w.log(&format!("  {}: freed {}", t.name, format_size(Some(freed))));
                    }
                    w.log(&format!("Total freed: {}", format_size(Some(sum))));
                    w.send(Msg::Scan(
                        cleaner::TARGETS
                            .iter()
                            .map(|t| (t.id, cleaner::scan(t, &exclusions)))
                            .collect(),
                    ));
                });
            }
            Action::SetDns(provider) => {
                self.spawn_recorded(ctx, &format!("DNS: {}", provider.name), move |w| {
                    let adapters = network::active_adapters();
                    network::apply_dns(&WinSystem, &adapters, &provider, &mut |l| w.log(l));
                });
            }
            Action::RestoreDns => {
                self.spawn_recorded(ctx, "Restore original DNS", move |w| {
                    network::restore_dns(&WinSystem, &mut |l| w.log(l))
                });
            }
            Action::ResetNetwork => {
                self.spawn_recorded(ctx, "Reset network stack", move |w| {
                    network::reset_stack(&WinSystem, &mut |l| w.log(l))
                });
            }
            Action::RunTool(tool) => {
                let title = tools::TOOLS
                    .iter()
                    .find(|info| info.tool == tool)
                    .map_or("Running tool", |info| info.title);
                self.spawn_recorded(ctx, title, move |w| {
                    tools::run(&WinSystem, tool, &backup_dir(), &mut |l| w.log(l))
                });
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
            Action::ApplyGameGraphics {
                executable,
                expected_current,
            } => {
                self.spawn_recorded(ctx, "Setting per-game graphics preference", move |w| {
                    let store_path = gaming::store_path();
                    match gaming::load_store(&store_path) {
                        Ok(mut profiles) => {
                            let requested = profiles
                                .iter()
                                .find(|profile| {
                                    profile.executable.eq_ignore_ascii_case(&executable)
                                })
                                .map(|profile| profile.preference.label().to_owned());
                            w.log(&format!("Executable: {executable}"));
                            w.log(&format!(
                                "Previous value: {}",
                                expected_current
                                    .as_ref()
                                    .map_or("not set".into(), RegData::show)
                            ));
                            if let Some(requested) = requested {
                                w.log(&format!("Requested preference: {requested}"));
                            }
                            match gaming::apply(
                                &WinSystem,
                                &store_path,
                                &mut profiles,
                                &executable,
                                &expected_current,
                            ) {
                                Ok(()) => {
                                    w.log("Windows graphics preference applied and verified.")
                                }
                                Err(error) => w.log(&format!(
                                    "WARN Could not apply the graphics preference: {error}"
                                )),
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

    fn run_tweaks(
        &mut self,
        ctx: &egui::Context,
        ids: Vec<String>,
        apply: bool,
        restore_point: bool,
    ) {
        let cat = self.cat.clone();
        let title = format!(
            "{} {} tweak(s)",
            if apply { "Apply" } else { "Undo" },
            ids.len()
        );
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
            self.events.push(OptiEvent::Toast(
                "Still analyzing, try again in a moment.".into(),
            ));
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
                    let name = self
                        .cat
                        .find(id)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| id.clone());
                    self.say(&format!("  not ticked: {name} ({why})"));
                }
                self.tw_skip = plan.skipped.clone();
                self.say(&format!(
                    "Auto-detect: {} tweak(s) selected for this PC ({}).",
                    plan.ids.len(),
                    profile_summary(&tr)
                ));
                if plan.ids.is_empty() {
                    self.events.push(OptiEvent::Toast(
                        "This PC is already optimized: every fitting tweak is applied.".into(),
                    ));
                }
                self.selected.extend(plan.ids);
            }
            tag => self.selected.extend(hw::preset_ids(&self.cat, &tr, tag)),
        }
    }

    fn review_smart_plan(&mut self, ctx: &egui::Context) {
        if self.busy.is_some() || !self.status_ready || self.traits.is_none() {
            return;
        }
        self.select_preset("auto");
        if self.selected.is_empty() {
            return;
        }
        self.tw_search.clear();
        self.focus_tweak_search = false;
        self.tw_risk = 0;
        self.tw_group = 0;
        self.tw_state = 2;
        self.tw_view = 1;
        self.tw_compatibility = 1;
        self.tw_sort = 0;
        self.set_page(ctx, Page::Tweaks);
    }

    fn save_selected_profile(&mut self) {
        if self.profiles_error.is_some() {
            self.events.push(OptiEvent::Toast("Saved profiles could not be read. Export the current selection or recover the profiles file before saving.".into()));
            return;
        }
        if self.selected.is_empty() {
            self.events.push(OptiEvent::Toast(
                "Select at least one tweak before saving a profile.".into(),
            ));
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
        if self
            .profiles
            .iter()
            .any(|saved| saved.name.eq_ignore_ascii_case(&profile.name))
        {
            self.profile_error(format!(
                "A profile named '{}' already exists. Remove it before saving a replacement.",
                profile.name
            ));
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
            self.events.push(OptiEvent::Toast(
                "Still analyzing this PC. Try again in a moment.".into(),
            ));
            return;
        };
        match profiles::compatible_ids(profile, &self.cat, traits) {
            Ok((ids, skipped)) if !ids.is_empty() => {
                self.selected = ids.into_iter().collect();
                self.tw_skip.clear();
                self.say(&format!(
                    "Selected profile '{}': {} tweak(s) fit this PC; {} skipped.",
                    profile.name,
                    self.selected.len(),
                    skipped
                ));
            }
            Ok((_, skipped)) => self.events.push(OptiEvent::Toast(format!(
                "No tweaks in '{}' fit this PC ({skipped} skipped).",
                profile.name
            ))),
            Err(error) => self.profile_error(error),
        }
    }

    fn import_profile(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Optimaxer profile", &["json"])
            .pick_file()
        else {
            return;
        };
        let profile = match profiles::read_export(&path) {
            Ok(profile) => profile,
            Err(error) => {
                self.profile_error(error);
                return;
            }
        };
        let Some(traits) = self.traits.as_ref() else {
            self.events.push(OptiEvent::Toast(
                "Still analyzing this PC. Try again in a moment.".into(),
            ));
            return;
        };
        match profiles::compatible_ids(&profile, &self.cat, traits) {
            Ok((ids, skipped)) if !ids.is_empty() => {
                self.selected = ids.into_iter().collect();
                self.tw_skip.clear();
                self.profile_name = profile.name.clone();
                let message = format!(
                    "Imported '{}': {} tweak(s) selected; {} unavailable or incompatible tweak(s) skipped. Review the list before applying.",
                    profile.name,
                    self.selected.len(),
                    skipped
                );
                self.say(&message);
                self.events.push(OptiEvent::Toast(message));
            }
            Ok((_, skipped)) => self.events.push(OptiEvent::Toast(format!(
                "No imported tweaks fit this PC ({skipped} skipped). Existing selection kept."
            ))),
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
            Ok(()) => self.say(&format!(
                "Exported profile '{}' to {}.",
                profile.name,
                path.display()
            )),
            Err(error) => self.profile_error(error),
        }
    }

    fn export_selected_profile(&mut self) {
        if self.selected.is_empty() {
            self.events.push(OptiEvent::Toast(
                "Select at least one tweak before exporting a profile.".into(),
            ));
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
        let screenshot_open = std::env::var_os("ORION_SHOTS").is_some()
            && std::env::var_os("ORION_SHOTS_PROFILE_OPEN").is_some();
        if self.profiles.len() >= 2 {
            if !self
                .profiles
                .iter()
                .any(|profile| profile.name == self.compare_profile_left)
            {
                self.compare_profile_left = self.profiles[0].name.clone();
            }
            if !self
                .profiles
                .iter()
                .any(|profile| profile.name == self.compare_profile_right)
                || self.compare_profile_right == self.compare_profile_left
            {
                self.compare_profile_right = self
                    .profiles
                    .iter()
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
            ui.label(
                RichText::new("Optimaxer")
                    .font(FontId::new(T_SMALL, semibold()))
                    .color(t.subtle),
            );
            ui.add_space(S1);
        }
        for group in PageGroup::ALL {
            if !collapsed {
                ui.add_space(S2);
                ui.label(
                    RichText::new(group.label())
                        .font(FontId::new(T_CAPTION, semibold()))
                        .color(t.subtle),
                );
                ui.add_space(S1);
            }
            for page in Page::ALL.into_iter().filter(|page| page.group() == group) {
                if kit::nav_item_icon(ui, self.page == page, Some(page.icon()), page.label(), None)
                    .clicked()
                    && self.page != page
                {
                    self.set_page(ctx, page);
                }
            }
        }
        if !collapsed && let Some(tr) = &self.traits {
            ui.add_space(S4);
            ui.label(
                RichText::new("This PC")
                    .font(FontId::new(T_SMALL, semibold()))
                    .color(t.subtle),
            );
            ui.add_space(S1);
            ui.label(
                RichText::new(format!("{:.0} GB RAM, {} threads", tr.ram_gb, tr.threads))
                    .size(T_SMALL)
                    .color(t.muted),
            );
            for g in &tr.gpus {
                ui.add(egui::Label::new(RichText::new(g).size(T_SMALL).color(t.muted)).wrap());
            }
            ui.add_space(S1);
            ui.add(
                egui::Label::new(
                    RichText::new(profile_summary(tr))
                        .size(T_CAPTION)
                        .color(t.subtle),
                )
                .wrap(),
            );
        }
    }

    pub fn log_panel(&mut self, root: &mut Ui) {
        let t = tokens(root.ctx());
        let max_height = (root.available_height() * 0.42).clamp(180.0, 320.0);
        let collapsed = egui::Panel::bottom("opti-log-collapsed")
            .resizable(true)
            .exact_size(44.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(t.surface1)
                    .inner_margin(Margin::symmetric(S4 as i8, S1 as i8)),
            );
        let expanded = egui::Panel::bottom("opti-log-expanded")
            .resizable(true)
            .default_size(216.0)
            .min_size(120.0)
            .max_size(max_height)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(t.surface1)
                    .inner_margin(Margin::symmetric(S4 as i8, S1 as i8)),
            );

        let mut expansion_change = None;
        let mut open_activity = false;
        let mut request_cancel = false;
        egui::Panel::show_switched(
            root,
            &mut self.log_expanded,
            collapsed,
            expanded,
            |ui, is_expanded| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Operation center")
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    if let Some(job) = &self.busy {
                        kit::chip(ui, "Running", Tone::Brand);
                        ui.add(
                            egui::Label::new(RichText::new(job).size(T_CAPTION).color(t.muted))
                                .truncate(),
                        );
                    }
                    if !self.log.is_empty() {
                        kit::chip(ui, &format!("{} entries", self.log.len()), Tone::Neutral);
                    }
                    if self.busy.is_some() {
                        ui.add(egui::Spinner::new().size(12.0).color(t.brand_text));
                    }
                    if let Some(cancel) = &self.cancel_current {
                        let requested = cancel.load(Ordering::Relaxed);
                        if kit::ghost(
                            ui,
                            if requested {
                                "Cancelling…"
                            } else {
                                "Cancel scan"
                            },
                            !requested,
                        )
                        .clicked()
                        {
                            request_cancel = true;
                        }
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
                        let latest = self
                            .log
                            .last()
                            .map(String::as_str)
                            .unwrap_or("No recent activity");
                        let summary = self.busy.as_deref().unwrap_or(latest);
                        let color = if summary.contains("WARN") || summary.contains("failed") {
                            t.warn
                        } else {
                            t.subtle
                        };
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if kit::ghost(ui, "Show", true).clicked() {
                                expansion_change = Some(true);
                            }
                            ui.add(
                                egui::Label::new(
                                    RichText::new(summary).size(T_CAPTION).color(color),
                                )
                                .truncate(),
                            );
                        });
                    }
                });
                if is_expanded {
                    if self.log.is_empty() {
                        ui.label(
                            RichText::new("Output from the actions you run appears here.")
                                .size(T_SMALL)
                                .color(t.subtle),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for l in &self.log {
                                let color = if l.contains("WARN") || l.contains("failed") {
                                    t.warn
                                } else if l.starts_with("==") {
                                    t.brand_text
                                } else {
                                    t.muted
                                };
                                ui.label(RichText::new(l).monospace().size(T_CAPTION).color(color));
                            }
                        });
                }
            },
        );
        if let Some(expanded) = expansion_change {
            self.log_expanded = expanded;
        }
        if request_cancel && let Some(cancel) = &self.cancel_current {
            cancel.store(true, Ordering::Relaxed);
            self.say("Cancellation requested for the active read-only scan.");
        }
        if open_activity {
            self.set_page(root.ctx(), Page::Activity);
        }
    }

    /// Runs every frame, whichever section is showing: collects worker results, draws the confirmation window and
    /// keeps the restore-point choice in `Settings`.
    pub fn tick(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        self.poll();
        self.tick_blackbox(settings);
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

    pub fn central(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        suggestions: &[AppItem],
        settings: &mut Settings,
    ) {
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
                        let action_height = if ui.available_width() < 680.0 {
                            104.0
                        } else {
                            68.0
                        };
                        let scroll_height = (ui.available_height() - action_height).max(120.0);
                        egui::ScrollArea::vertical()
                            .id_salt(("optimaxer-page", self.page.label()))
                            .auto_shrink([false, false])
                            .max_height(scroll_height)
                            .show(ui, |ui| self.tweaks_page(ui, ctx, settings));
                        self.tweak_action_bar(ui, ctx);
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(("optimaxer-page", self.page.label()))
                        .auto_shrink([false, false])
                        .show(ui, |ui| match self.page {
                            Page::Overview => self.overview_page(ui, ctx, settings),
                            Page::SmartOptimize => self.smart_optimize_page(ui, ctx),
                            Page::Performance => self.performance_page(ui, ctx, settings),
                            Page::BlackBox => self.blackbox_page(ui, ctx, settings),
                            Page::Detective => self.detective_page(ui, ctx, settings),
                            Page::Diagnostics => self.diagnostics_page(ui, ctx),
                            Page::Snapshots => self.snapshots_page(ui, ctx),
                            Page::Processes => self.processes_page(ui, ctx),
                            Page::Gaming => self.gaming_page(ui, ctx),
                            Page::Activity => self.activity_page(ui, ctx),
                            Page::Tweaks => {
                                unreachable!("Tweaks uses its pinned action bar layout")
                            }
                            Page::Services => self.services_page(ui, ctx),
                            Page::Startup => self.startup_page(ui, ctx),
                            Page::StartupXray => self.xray_page(ui, ctx),
                            Page::Cleaner => self.cleaner_page(ui, ctx, settings),
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
            self.selected
                .iter()
                .filter_map(|id| self.cat.find(id))
                .filter(|tweak| tweak.auto_pick() && hw::fits(tweak, traits))
                .count()
        });
        let undoable = self
            .selected
            .iter()
            .filter(|id| self.status.get(*id).copied().unwrap_or(false))
            .count();
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
        let Some(p) = self.pending.as_ref() else {
            return;
        };
        let t = tokens(ctx);
        let mut decision: Option<bool> = None;
        let mut restore = self.restore_first;
        let resp = egui::Modal::new(egui::Id::unique("opti-confirm"))
            .backdrop_color(Color32::from_black_alpha(if t.dark { 150 } else { 70 }))
            .show(ctx, |ui| {
                ui.set_width(640.0);
                ui.label(
                    RichText::new(&p.title)
                        .font(FontId::new(theme::T_TITLE, semibold()))
                        .color(t.text),
                );
                ui.add_space(S1);
                ui.add(egui::Label::new(RichText::new(&p.intro).color(t.muted)).wrap());
                if !p.lines.is_empty() {
                    ui.add_space(S3);
                    kit::card(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .max_height(330.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for l in &p.lines {
                                    if l.head {
                                        ui.add_space(S1);
                                        ui.label(
                                            RichText::new(&l.text)
                                                .font(FontId::new(T_BODY, semibold()))
                                                .color(if l.warn { t.bad } else { t.text }),
                                        );
                                    } else {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&l.text)
                                                    .size(T_SMALL)
                                                    .color(if l.warn { t.warn } else { t.muted }),
                                            )
                                            .wrap(),
                                        );
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
                        let copied = ui
                            .data(|d| d.get_temp::<f64>(copied_id))
                            .is_some_and(|at| now - at < 1.6);
                        if copied {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(1700));
                        }
                        if kit::ghost(ui, if copied { "Copied" } else { "Copy details" }, true)
                            .clicked()
                        {
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
            && ok
        {
            self.confirm(ctx, p, restore);
        }
    }

    fn toolbar_search(&self, ui: &mut Ui, text: &mut String, hint: &str) -> egui::Response {
        ui.add_sized(
            [260.0, 34.0],
            egui::TextEdit::singleline(text)
                .id_salt("optimaxer-tweak-search")
                .hint_text(hint)
                .margin(Margin::symmetric(S3 as i8, 8)),
        )
    }

    fn tick_telemetry(&mut self, ctx: &egui::Context, settings: &Settings) {
        let config = settings.telemetry_config();
        let reconfigured = self.telemetry_needs_reset || config != self.telemetry_config;
        if let Some(worker) = &self.telemetry_worker {
            let result = if self.telemetry_needs_reset {
                worker.resume(config)
            } else if config != self.telemetry_config {
                worker.configure(config)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                self.telemetry_error = Some(error);
                self.telemetry_worker = None;
            } else if self.telemetry_needs_reset || config != self.telemetry_config {
                self.telemetry_config = config;
                self.telemetry_needs_reset = false;
            }
        }
        if let Some(worker) = &self.telemetry_worker
            && let Some(update) = worker.try_latest()
        {
            if let Some(info) = update.info {
                self.info = Some(info);
            }
            self.telemetry_frame = update.frame;
        }
        let repaint_after = if reconfigured {
            Duration::from_millis(100)
        } else if self.info.is_none() {
            Duration::from_millis(250)
        } else {
            config.interval()
        };
        if self.telemetry_worker.is_some() {
            ctx.request_repaint_after(repaint_after);
        }
    }

    pub fn suspend_telemetry(&mut self) {
        if let Some(worker) = &self.telemetry_worker
            && let Err(error) = worker.pause()
        {
            self.telemetry_error = Some(error);
            self.telemetry_worker = None;
        }
        self.telemetry_needs_reset = true;
    }

    // endregion
}

/// Everything the confirmation window shows, as plain text for the clipboard.
fn confirm_text(p: &Pending) -> String {
    let mut out = format!(
        "{}
{}
",
        p.title, p.intro
    );
    for l in &p.lines {
        if l.head {
            out.push_str(&format!(
                "
{}
",
                l.text
            ));
        } else {
            out.push_str(&format!(
                "  {}
",
                l.text
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opti::backend::RegData;
    use crate::opti::backend::fake::FakeSystem;

    #[test]
    fn service_change_review_lists_direct_dependencies_and_warns_on_dependents() {
        let targets = vec![("spooler".into(), "Print Spooler".into())];
        let dependencies = vec![services::DependencyInfo {
            service: "Spooler".into(),
            requires: vec!["RPCSS".into(), "+PrintProvider".into()],
            required_by: vec!["Example service (ExampleSvc)".into()],
        }];

        let disabling = service_dependency_lines(&targets, &dependencies, true).unwrap();
        assert_eq!(disabling.len(), 3);
        assert_eq!(disabling[0].text, "Print Spooler (spooler)");
        assert_eq!(
            disabling[1].text,
            "Requires: RPCSS, load-order group PrintProvider"
        );
        assert_eq!(
            disabling[2].text,
            "Required by: Example service (ExampleSvc)"
        );
        assert!(disabling[2].warn);

        let manual = service_dependency_lines(&targets, &dependencies, false).unwrap();
        assert!(!manual[2].warn);
    }

    #[test]
    fn service_change_review_rejects_mismatched_dependency_results() {
        let targets = vec![("spooler".into(), "Print Spooler".into())];
        let dependencies = vec![services::DependencyInfo {
            service: "other".into(),
            requires: Vec::new(),
            required_by: Vec::new(),
        }];

        assert!(service_dependency_lines(&targets, &dependencies, true).is_err());
        assert!(service_dependency_lines(&targets, &[], true).is_err());
    }

    fn tr(laptop: bool) -> Traits {
        let gpus = vec!["NVIDIA GeForce RTX 3090".to_string()];
        Traits {
            laptop,
            ssd: true,
            ram_gb: 32.0,
            threads: 16,
            tags: hw::tags_for(laptop, true, true, 32.0, 16, &gpus, 26100),
            gpus,
            ..Default::default()
        }
    }

    #[test]
    fn page_names_match_ignoring_case_spacing_and_punctuation() {
        assert_eq!(page_key("startup x ray"), page_key("Startup X-Ray"));
        assert_ne!(page_key("startup"), page_key("Startup X-Ray"));
    }

    #[test]
    fn every_navigation_page_belongs_to_one_visible_sidebar_group() {
        assert_eq!(Page::ALL.len(), 17);
        for (group, expected_count) in PageGroup::ALL.into_iter().zip([7, 7, 3]) {
            assert_eq!(
                Page::ALL
                    .into_iter()
                    .filter(|page| page.group() == group)
                    .count(),
                expected_count,
                "{}",
                group.label()
            );
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
        assert!(
            laptop_total <= total,
            "desktop-only tweaks must not count on a laptop"
        );
    }

    #[test]
    fn storage_review_uses_available_capacity_and_ninety_percent_threshold() {
        use crate::opti::sysmon::Snapshot;

        let mut snapshot = Snapshot {
            disk_used: 89,
            disk_total: 100,
            disk_available: true,
            ..Default::default()
        };
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
        assert!(
            todo.iter()
                .any(|l| l.text.contains("20000") && l.text.contains("5000"))
        );
    }

    #[test]
    fn batch_refreshes_other_tweaks_that_share_a_setting() {
        use crate::opti::catalog::{RegItem, RegVal};
        let path = r"HKCU:\Software\OptimaxerBatchTest";
        let first = Tweak {
            id: "first".into(),
            name: "First".into(),
            reg: vec![RegItem {
                p: path.into(),
                n: "Choice".into(),
                v: RegVal::Num(1),
                t: "DWord".into(),
            }],
            ..Default::default()
        };
        let mut second = first.clone();
        second.id = "second".into();
        second.reg[0].v = RegVal::Num(2);
        let cat = Catalog {
            tweaks: vec![first, second],
            ..Default::default()
        };
        let sys = FakeSystem::default();
        sys.put(path, "Choice", RegData::Dword(2));
        let dir = std::env::temp_dir().join(format!("opti-batch-status-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::load_from(dir.join("state.json"), None);
        let eng = Engine::new(&sys, PathBuf::new());
        let batch = run_batch(
            &eng,
            &cat,
            &["first".into()],
            true,
            false,
            &mut state,
            &mut |_| {},
        );
        assert_eq!(batch.now.get("first"), Some(&true));
        assert_eq!(batch.now.get("second"), Some(&false));
        assert!(batch.changes[0].verified && batch.changes[0].undo_available);
        assert!(
            batch.changes[0]
                .details
                .iter()
                .any(|line| line.contains("2 -> 1"))
        );
        let undone = run_batch(
            &eng,
            &cat,
            &["first".into()],
            false,
            false,
            &mut state,
            &mut |_| {},
        );
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
            reg: vec![RegItem {
                p: path.into(),
                n: "Enabled".into(),
                v: RegVal::Num(1),
                t: "DWord".into(),
            }],
            restart: true,
            explorer: true,
            ..Default::default()
        };
        let cat = Catalog {
            tweaks: vec![tweak],
            ..Default::default()
        };
        let sys = FakeSystem::default();
        sys.put(path, "Enabled", RegData::Dword(0));
        let dir = std::env::temp_dir().join(format!("opti-restart-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::load_from(dir.join("state.json"), None);
        let engine = Engine::new(&sys, PathBuf::new());

        let applied = run_batch(
            &engine,
            &cat,
            &["restart-test".into()],
            true,
            false,
            &mut state,
            &mut |_| {},
        );
        assert!(applied.changes[0].verified && applied.restart && applied.explorer);

        let unchanged = run_batch(
            &engine,
            &cat,
            &["restart-test".into()],
            true,
            false,
            &mut state,
            &mut |_| {},
        );
        assert!(unchanged.changes[0].verified);
        assert!(
            !unchanged.restart && !unchanged.explorer,
            "reapplying an active tweak must not request restarts"
        );

        let undone = run_batch(
            &engine,
            &cat,
            &["restart-test".into()],
            false,
            false,
            &mut state,
            &mut |_| {},
        );
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
                RegItem {
                    p: denied_path.into(),
                    n: "Enabled".into(),
                    v: RegVal::Num(1),
                    t: "DWord".into(),
                },
                RegItem {
                    p: writable_path.into(),
                    n: "Enabled".into(),
                    v: RegVal::Num(1),
                    t: "DWord".into(),
                },
            ],
            restart: true,
            explorer: true,
            ..Default::default()
        };
        let cat = Catalog {
            tweaks: vec![tweak],
            ..Default::default()
        };
        let sys = FakeSystem::default();
        sys.put(denied_path, "Enabled", RegData::Dword(0));
        sys.put(writable_path, "Enabled", RegData::Dword(0));
        sys.deny_writes
            .borrow_mut()
            .insert(denied_path.to_lowercase());
        let dir = std::env::temp_dir().join(format!("opti-restart-failure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = State::load_from(dir.join("state.json"), None);
        let engine = Engine::new(&sys, PathBuf::new());

        let failed = run_batch(
            &engine,
            &cat,
            &["restart-failure-test".into()],
            true,
            false,
            &mut state,
            &mut |_| {},
        );
        assert!(!failed.changes[0].verified && failed.restart && !failed.explorer);
        assert_eq!(
            sys.reg_get(writable_path, "Enabled"),
            Some(RegData::Dword(1))
        );
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
        assert!(
            st.records.is_empty(),
            "a tweak was applied after the restore point failed"
        );
        assert!(sys.values.borrow().is_empty(), "the registry was written");
        let b = run_batch(&eng, &cat, &ids, true, false, &mut st, &mut |_| {});
        assert!(
            !b.restore_failed && ids.iter().all(|id| b.now.contains_key(id)),
            "without a restore point the batch runs"
        );
    }

    #[test]
    fn technical_details_list_the_exact_changes() {
        let cat = Catalog::load();
        let t = cat.find("perf-shutdown").unwrap();
        let lines = tweak_technical(t);
        assert!(lines[0].contains("perf-shutdown") && lines[0].contains(&t.risk));
        assert!(lines.iter().any(|l| l.starts_with("registry")
            && l.contains("WaitToKillAppTimeout")
            && l.contains("REG_SZ")));
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
        assert!(
            cat.tweaks
                .iter()
                .filter(|t| in_group(t, 1, false))
                .all(|t| t.has_tag("safe"))
        );
    }

    #[test]
    fn tweak_filters_combine_state_recommendation_favorites_and_compatibility() {
        let recommended = Tweak {
            id: "recommended".into(),
            risk: "Safe".into(),
            tags: vec!["safe".into()],
            ..Default::default()
        };
        let opt_in = Tweak {
            id: "opt-in".into(),
            risk: "Safe".into(),
            tags: vec!["optin".into()],
            ..Default::default()
        };

        assert!(tweak_matches_view(&recommended, true, true, false, 1, 1, 1));
        assert!(!tweak_matches_view(
            &recommended,
            false,
            true,
            false,
            1,
            1,
            1
        ));
        assert!(!tweak_matches_view(
            &recommended,
            false,
            false,
            false,
            0,
            1,
            0
        ));
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
        assert_eq!(
            profile_summary(&Traits::default()),
            "Form factor unknown · Storage unknown · Windows version unknown"
        );
    }
}
