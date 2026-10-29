//! Optimaxer section of the window: Tweaks, Services, Startup, Cleaner, Network and Tools pages.
//! Everything that reads or changes Windows runs on a worker thread and reports back through a channel, so the UI
//! never blocks. Every change goes through a confirmation window that lists exactly what will happen.

use crate::kit;
use crate::model::{format_size, AppItem};
use crate::net::Settings;
use crate::opti::backend::{System, WinSystem};
use crate::opti::catalog::{Catalog, Tweak};
use crate::opti::cleaner::{self, Target};
use crate::opti::engine::{Change, Engine};
use crate::opti::hw::{self, Traits};
use crate::opti::history::{self, Action as HistoryAction, Change as HistoryChange, Entry as HistoryEntry};
use crate::opti::network;
use crate::opti::processes::{self, CpuTimes, ProcessInfo};
use crate::opti::services::{self, Data, DnsProvider, SvcRow};
use crate::opti::startup;
use crate::opti::state::{backup_dir, State};
use crate::opti::tools::{self, Tool};
use crate::theme::{self, mix, semibold, tokens, Tone, R_LG, R_MD, S1, S2, S3, S4, S5, T_BODY, T_CAPTION, T_HEADING, T_SMALL, T_TITLE};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LOG_LINES: usize = 600;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Overview,
    Processes,
    Activity,
    Tweaks,
    Services,
    Startup,
    Cleaner,
    Network,
    Tools,
}

impl Page {
    const ALL: [Page; 9] = [
        Page::Overview,
        Page::Processes,
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
            Page::Processes => '\u{E9D9}',
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
            Page::Processes => "Processes",
            Page::Activity => "Activity",
            Page::Tweaks => "Tweaks",
            Page::Services => "Services",
            Page::Startup => "Startup",
            Page::Cleaner => "Cleaner",
            Page::Network => "Network",
            Page::Tools => "Tools",
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
    Idle,
    Traits(Traits),
    Status(HashMap<String, bool>),
    Plan(Pending),
    Services(Vec<SvcRow>),
    Startup(Vec<startup::Item>),
    Scan(Vec<(&'static str, Option<u64>)>),
    Adapters(Vec<String>),
    Bench(Vec<(String, Option<f64>)>),
    History(Result<Vec<HistoryEntry>, String>),
    Processes(Result<(Vec<ProcessInfo>, CpuTimes), String>),
    RestartRequired,
}

#[derive(Clone)]
struct Worker {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Worker {
    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
    fn log(&self, s: &str) {
        self.send(Msg::Log(s.to_string()));
    }
}

pub struct OptiUi {
    page: Page,
    cat: Arc<Catalog>,
    data: Arc<Data>,
    traits: Option<Traits>,
    status: HashMap<String, bool>,
    selected: HashSet<String>,
    tw_search: String,
    /// Risk filter on the Tweaks page: 0 all, 1 Safe, 2 Moderate, 3 Advanced.
    tw_risk: usize,
    /// Group filter on the Tweaks page, an index into `TWEAK_GROUPS`.
    tw_group: usize,
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
    history_search: String,
    restart_pending: bool,
    pending: Option<Pending>,
    restore_first: bool,
    elevated: bool,
    started: bool,
    services: Vec<SvcRow>,
    svc_sel: HashSet<String>,
    svc_search: String,
    startup: Vec<startup::Item>,
    st_sel: HashSet<String>,
    clean_sizes: HashMap<&'static str, Option<u64>>,
    clean_sel: HashSet<&'static str>,
    adapters: Vec<String>,
    bench: Vec<(String, Option<f64>)>,
    events: Vec<OptiEvent>,
    /// `ctx.input(|i| i.time)` when the page last changed, for the enter transition.
    page_started: f64,
    sampler: crate::opti::sysmon::Sampler,
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

fn risk_tone(risk: &str) -> Tone {
    match risk {
        "Moderate" => Tone::Warn,
        "Advanced" => Tone::Bad,
        _ => Tone::Good,
    }
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
        Self {
            page: Page::Overview,
            cat: Arc::new(Catalog::load()),
            data: Arc::new(Data::load()),
            traits: None,
            status: HashMap::new(),
            selected: HashSet::new(),
            tw_search: String::new(),
            tw_risk: 0,
            tw_group: 0,
            tw_open: HashSet::new(),
            tw_skip: Default::default(),
            tx,
            rx,
            busy: None,
            log: Vec::new(),
            log_expanded: false,
            history,
            history_error,
            history_search: String::new(),
            restart_pending: false,
            pending: None,
            restore_first: true,
            elevated: hw::is_elevated(),
            started: false,
            services: Vec::new(),
            svc_sel: HashSet::new(),
            svc_search: String::new(),
            startup: Vec::new(),
            st_sel: HashSet::new(),
            clean_sizes: HashMap::new(),
            clean_sel: HashSet::new(),
            adapters: Vec::new(),
            bench: Vec::new(),
            events: Vec::new(),
            page_started: 0.0,
            sampler: crate::opti::sysmon::Sampler::new(),
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
        Worker { tx: self.tx.clone(), ctx: ctx.clone() }
    }

    /// Starts a background job. One job at a time, like the PowerShell edition.
    fn spawn(&mut self, ctx: &egui::Context, title: &str, job: impl FnOnce(&Worker) + Send + 'static) -> bool {
        if self.busy.is_some() {
            self.events.push(OptiEvent::Toast("Another task is running, please wait.".into()));
            return false;
        }
        self.busy = Some(title.to_string());
        self.say(&format!("== {title}"));
        let w = self.worker(ctx);
        std::thread::spawn(move || {
            job(&w);
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
                Msg::Idle => self.busy = None,
                Msg::Traits(t) => self.traits = Some(t),
                Msg::Status(map) => self.status.extend(map),
                Msg::Plan(p) => self.pending = Some(p),
                Msg::Services(s) => {
                    self.svc_sel.retain(|n| s.iter().any(|r| &r.name == n));
                    self.services = s;
                    self.services_loaded = true;
                }
                Msg::Startup(s) => {
                    self.st_sel.clear();
                    self.startup = s;
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
                Msg::Adapters(a) => self.adapters = a,
                Msg::Bench(b) => self.bench = b,
                Msg::History(Ok(entries)) => {
                    self.history = entries;
                    self.history_error = None;
                }
                Msg::History(Err(error)) => self.history_error = Some(error),
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
            let items = startup::list(&WinSystem);
            w.log(&format!("Found {} startup items.", items.len()));
            w.send(Msg::Startup(items));
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

    /// Builds the confirmation for applying tweaks. Reads only; nothing changes until the user confirms.
    fn preview_apply(&mut self, ctx: &egui::Context) {
        let ids: Vec<String> = self.cat.tweaks.iter().filter(|t| self.selected.contains(&t.id)).map(|t| t.id.clone()).collect();
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
                self.spawn(ctx, "Undo all Optimaxer tweaks", move |w| {
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
                self.spawn(ctx, "Changing services", move |w| {
                    let n = services::set_start(&WinSystem, &data, &names, start, &backup_dir(), &mut |l| w.log(l));
                    w.log(&format!("{n} service(s) changed."));
                    w.send(Msg::Services(services::list(&WinSystem, &data)));
                });
            }
            Action::RestoreServices => {
                let data = self.data.clone();
                self.spawn(ctx, "Restoring services", move |w| {
                    services::restore_latest(&WinSystem, &backup_dir(), &mut |l| w.log(l));
                    w.send(Msg::Services(services::list(&WinSystem, &data)));
                });
            }
            Action::SetStartup { items, enable } => {
                self.spawn(ctx, if enable { "Enabling startup items" } else { "Disabling startup items" }, move |w| {
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
                self.spawn(ctx, "Cleaning", move |w| {
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
                self.spawn(ctx, &format!("DNS: {}", provider.name), move |w| {
                    let adapters = network::active_adapters();
                    network::apply_dns(&WinSystem, &adapters, &provider, &mut |l| w.log(l));
                });
            }
            Action::ResetNetwork => {
                self.spawn(ctx, "Reset network stack", move |w| network::reset_stack(&WinSystem, &mut |l| w.log(l)));
            }
            Action::RunTool(tool) => {
                self.spawn(ctx, "Running tool", move |w| tools::run(&WinSystem, tool, &backup_dir(), &mut |l| w.log(l)));
            }
            Action::RunInstaller(path) => {
                self.spawn(ctx, "Restore point, then installer", move |w| {
                    let ok = tools::create_restore_point(&WinSystem, "Optimaxer - before installer", &mut |l| w.log(l));
                    if ok {
                        match open::that(&path) {
                            Ok(()) => w.log(&format!("Opened {}", path.display())),
                            Err(e) => w.log(&format!("WARN could not open the file: {e}")),
                        }
                    } else {
                        w.log("The installer was not opened because the restore point failed. Open it from the Downloads list if you want to continue anyway.");
                    }
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
    // endregion

    // region drawing
    pub fn side_panel(&mut self, ui: &mut Ui, ctx: &egui::Context, collapsed: bool) {
        let t = tokens(ctx);
        ui.add_space(S3);
        if !collapsed {
            ui.label(RichText::new("Optimaxer").font(FontId::new(T_SMALL, semibold())).color(t.subtle));
            ui.add_space(S1);
        }
        for p in Page::ALL {
            if kit::nav_item_icon(ui, self.page == p, Some(p.icon()), p.label(), None).clicked() && self.page != p {
                self.set_page(ctx, p);
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
        egui::Panel::show_switched(root, &mut self.log_expanded, collapsed, expanded, |ui, is_expanded| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Activity").font(FontId::new(T_BODY, semibold())).color(t.text));
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
                    let color = if latest.contains("WARN") || latest.contains("failed") { t.warn } else { t.subtle };
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::ghost(ui, "Show", true).clicked() {
                            expansion_change = Some(true);
                        }
                        ui.add(egui::Label::new(RichText::new(latest).size(T_CAPTION).color(color)).truncate());
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
            settings.opti_restore_point = self.restore_first;
            settings.save();
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

    pub fn central(&mut self, ui: &mut Ui, ctx: &egui::Context, suggestions: &[AppItem]) {
        if !self.started {
            self.load_first_time(ctx);
        }
        let t = tokens(ctx);
        if !self.elevated {
            egui::Frame::new().fill(mix(t.canvas, t.warn, 0.14)).corner_radius(R_LG).inner_margin(Margin::symmetric(S4 as i8, S3 as i8)).show(ui, |ui| {
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
            egui::Frame::new().fill(mix(t.canvas, t.warn, 0.11)).corner_radius(R_LG).inner_margin(Margin::symmetric(S4 as i8, S3 as i8)).show(ui, |ui| {
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
                egui::ScrollArea::vertical().id_salt(("optimaxer-page", self.page.label())).auto_shrink([false, false]).show(ui, |ui| match self.page {
                    Page::Overview => self.overview_page(ui, ctx),
                    Page::Processes => self.processes_page(ui, ctx),
                    Page::Activity => self.activity_page(ui, ctx),
                    Page::Tweaks => self.tweaks_page(ui, ctx),
                    Page::Services => self.services_page(ui, ctx),
                    Page::Startup => self.startup_page(ui, ctx),
                    Page::Cleaner => self.cleaner_page(ui, ctx),
                    Page::Network => self.network_page(ui, ctx),
                    Page::Tools => self.tools_page(ui, ctx, suggestions),
                });
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

    fn overview_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        use crate::opti::sysmon::{format_uptime, read_info};
        let t = tokens(ctx);
        self.sampler.tick();
        ctx.request_repaint_after(std::time::Duration::from_millis(1000));
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
            ui.add_space(S1);
            let width = ui.available_width();
            let compact = width < 720.0;
            let w3 = ((width - 2.0 * gap) / 3.0 - 1.0).floor();
            ui.horizontal_top(|ui| {
                let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
                let cards = [
                    ("Processor", s.cpu / 100.0, format!("{:.0}%", s.cpu), format!("{} threads", info.threads), kit::load_color(&t, s.cpu)),
                    ("Memory", s.ram_fraction(), format!("{:.0}%", s.ram_fraction() * 100.0), format!("{} of {}", format_size(Some(s.ram_used)), format_size(Some(s.ram_total))), kit::load_color(&t, s.ram_fraction() * 100.0)),
                    ("Storage", s.disk_fraction(), format!("{:.0}%", s.disk_fraction() * 100.0), format!("{drive} {} of {}", format_size(Some(s.disk_used)), format_size(Some(s.disk_total))), kit::load_color(&t, s.disk_fraction() * 100.0)),
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
            let w2 = ((ui.available_width() - gap) / 2.0 - 1.0).floor();
            ui.horizontal_top(|ui| {
                for (title, data, color) in [("Processor, last 60 seconds", &self.sampler.cpu_history, kit::load_color(&t, s.cpu)), ("Memory, last 60 seconds", &self.sampler.ram_history, kit::load_color(&t, s.ram_fraction() * 100.0))] {
                    ui.allocate_ui_with_layout(vec2(w2, 150.0), Layout::top_down(Align::Min), |ui| {
                        kit::card(ui, |ui| {
                            ui.set_width(w2 - 2.0 * S4 - 2.0);
                            ui.label(RichText::new(title).font(FontId::new(T_BODY, semibold())).color(t.text));
                            ui.add_space(S1);
                            kit::sparkline(ui, vec2(w2 - 2.0 * S4 - 2.0, 86.0), data, color);
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

    fn activity_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "Activity", "Search confirmed tweak apply and undo records. Other system operations are not included in this history.");
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [280.0, 34.0],
                egui::TextEdit::singleline(&mut self.history_search)
                    .hint_text("Search action, tweak or date"),
            );
            let visible = self
                .history
                .iter()
                .filter(|entry| history_entry_matches(entry, &self.history_search))
                .count();
            ui.label(
                RichText::new(format!("{visible} of {} records", self.history.len()))
                    .size(T_SMALL)
                    .color(t.muted),
            );
        });
        ui.add_space(S2);
        if let Some(error) = &self.history_error {
            kit::empty_state(ui, "Activity history unavailable", error);
            return;
        }
        let matches: Vec<_> = self
            .history
            .iter()
            .rev()
            .filter(|entry| history_entry_matches(entry, &self.history_search))
            .collect();
        if matches.is_empty() {
            kit::empty_state(
                ui,
                if self.history.is_empty() {
                    "No recorded activity"
                } else {
                    "No matching activity"
                },
                if self.history.is_empty() {
                    "Confirmed tweak apply and undo operations will appear here."
                } else {
                    "Try another search or clear the filter."
                },
            );
            return;
        }
        for (index, entry) in matches.iter().enumerate() {
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
                .id_salt(("activity-record", index, &entry.time))
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

    fn tweaks_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(ui, "Tweaks", "Performance, gaming, network, privacy and interface settings. The original value is saved before each change, so every tweak can be undone.");
        let (on, total) = self.traits.as_ref().map(|tr| score(&self.cat, tr, &self.status)).unwrap_or((0, 0));
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
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
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = self.busy.is_none() && self.traits.is_some();
                    if kit::primary(ui, "Smart optimize", ready).on_hover_text("Reads your hardware (laptop or desktop, SSD or HDD, RAM, GPU) and selects only the tweaks that fit this PC").clicked() {
                        self.select_preset("auto");
                        if !self.selected.is_empty() {
                            self.preview_apply(ctx);
                        }
                    }
                });
            });
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
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.tw_search);
            self.toolbar_search(ui, &mut q, "Search tweaks");
            self.tw_search = q;
            let n = self.selected.len();
            if n > 0 {
                ui.label(RichText::new(format!("{n} selected")).size(T_SMALL).color(t.muted));
            }
            let idle = self.busy.is_none();
            let label = if n > 0 { format!("Apply {n} tweak{}", if n == 1 { "" } else { "s" }) } else { "Apply tweaks".to_string() };
            if kit::primary(ui, &label, idle && n > 0).clicked() {
                self.preview_apply(ctx);
            }
            if kit::secondary(ui, "Undo selected", idle && n > 0).clicked() {
                self.preview_undo();
            }
            if kit::ghost(ui, "Refresh", idle).clicked() {
                self.refresh_status(ctx);
            }
        });
        ui.add_space(S2);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Risk").size(T_SMALL).color(t.muted));
            kit::segmented(ui, &mut self.tw_risk, &["All", "Safe", "Moderate", "Advanced"]);
            ui.add_space(S3);
            ui.label(RichText::new("Show").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-group").selected_text(TWEAK_GROUPS[self.tw_group.min(TWEAK_GROUPS.len() - 1)]).show_ui(ui, |ui| {
                for (i, g) in TWEAK_GROUPS.iter().enumerate() {
                    ui.selectable_value(&mut self.tw_group, i, *g);
                }
            });
            if (self.tw_risk != 0 || self.tw_group != 0) && kit::ghost(ui, "Reset filters", true).clicked() {
                self.tw_risk = 0;
                self.tw_group = 0;
            }
        });
        ui.add_space(S2);

        let q = self.tw_search.trim().to_lowercase();
        let risk_filter = ["", "Safe", "Moderate", "Advanced"][self.tw_risk.min(3)];
        let group_filter = self.tw_group;
        let mut cats: Vec<&str> = Vec::new();
        for tw in &self.cat.tweaks {
            if !cats.contains(&tw.cat.as_str()) {
                cats.push(&tw.cat);
            }
        }
        let cat = self.cat.clone();
        let traits = self.traits.clone();
        let mut shown = 0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for c in cats {
                let rows: Vec<&Tweak> = cat.tweaks.iter().filter(|tw| {
                    // a retired tweak is listed only while it has a saved record, so it can still be undone
                    (tw.retired.is_empty() || self.status.get(&tw.id).copied().unwrap_or(false))
                        && tw.cat == c
                        && (q.is_empty() || tw.name.to_lowercase().contains(&q) || tw.desc.to_lowercase().contains(&q))
                        && (risk_filter.is_empty() || tw.risk == risk_filter)
                        && in_group(tw, group_filter, self.status.get(&tw.id).copied().unwrap_or(false))
                }).collect();
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
                        let r = kit::row(ui, ("tw", &tw.id), on, |ui| {
                            ui.horizontal(|ui| {
                                kit::check(ui, on);
                                ui.vertical(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(&tw.name).font(FontId::new(T_BODY, semibold())).color(t.text));
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
                            ui.painter().rect_stroke(r.response.rect, CornerRadius::same(R_MD), Stroke::new(2.0, t.ring()), StrokeKind::Inside);
                        }
                        if click.clicked() && ctx.pointer_interact_pos().is_some_and(|p| details_rect.contains(p)) {
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
        });
    }

    fn services_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        if !self.services_loaded && self.busy.is_none() {
            self.refresh_services(ctx);
        }
        kit::page_header(ui, "Services", "Optimaxer's recommended Manual and Disabled lists. Core Windows services are hidden. The current start types are saved to a backup before every change.");
        let idle = self.busy.is_none();
        ui.horizontal(|ui| {
            let mut q = std::mem::take(&mut self.svc_search);
            self.toolbar_search(ui, &mut q, "Search services");
            self.svc_search = q;
            if kit::ghost(ui, "Select recommended", true).clicked() {
                self.svc_sel = self.services.iter().filter(|s| s.rec.as_ref().is_some_and(|r| r.0 == 3)).map(|s| s.name.clone()).collect();
            }
            if kit::ghost(ui, "Clear", !self.svc_sel.is_empty()).clicked() {
                self.svc_sel.clear();
            }
            if !self.svc_sel.is_empty() {
                ui.label(RichText::new(format!("{} selected", self.svc_sel.len())).size(T_SMALL).color(t.muted));
            }
        });
        ui.horizontal(|ui| {
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
            if kit::ghost(ui, "Restore last backup", idle).clicked() && self.need_admin() {
                self.pending = Some(Pending { title: "Restore services".into(), intro: "Put every service back to the start type saved in the most recent backup?".into(), lines: vec![], action: Action::RestoreServices, restore_option: false, ok_label: "Restore services".into() });
            }
            if kit::ghost(ui, "Refresh", idle).clicked() {
                self.refresh_services(ctx);
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
        kit::page_header(ui, "Startup", "Apps that start with Windows. Turning one off uses the same switch as Task Manager, and nothing is uninstalled.");
        let idle = self.busy.is_none();
        let key = |i: &startup::Item| format!("{}|{}", i.scope, i.name);
        let picked: Vec<startup::Item> = self.startup.iter().filter(|i| self.st_sel.contains(&key(i))).cloned().collect();
        ui.horizontal(|ui| {
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
        if self.startup.is_empty() {
            if self.busy.is_some() {
                kit::skeleton(ui, 5);
            } else {
                kit::empty_state(ui, "No startup items found", "Nothing in the Run keys or Startup folders. Press Refresh to look again.");
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
        ui.horizontal(|ui| {
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
            if total > 0 {
                ui.label(RichText::new(format!("{} can be reclaimed", format_size(Some(total)))).font(FontId::new(T_BODY, semibold())).color(t.good));
            } else if self.clean_sizes.is_empty() {
                ui.label(RichText::new("Scan to see how much each item can free.").size(T_SMALL).color(t.muted));
            }
        });
        ui.add_space(S2);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for tg in cleaner::TARGETS {
                self.cleaner_row(ui, ctx, tg);
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
        kit::page_header(ui, "Network", "Switch DNS (with DNS-over-HTTPS where the provider offers it), flush the cache, reset the stack, or time the servers.");
        let idle = self.busy.is_none();
        ui.horizontal_wrapped(|ui| {
            if kit::secondary(ui, "Show current DNS", idle).clicked() {
                self.spawn(ctx, "Reading DNS settings", |w| {
                    w.send(Msg::Adapters(network::active_adapters()));
                    network::show_dns(&WinSystem, &mut |l| w.log(l));
                });
            }
            if kit::secondary(ui, "Flush DNS cache", idle).clicked() {
                self.spawn(ctx, "Flush DNS", |w| {
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
                        let ms = network::latency_ms(&p.v4[0], 4);
                        w.log(&format!("  {:<12} {}", p.name, ms.map(|m| format!("{m} ms")).unwrap_or_else(|| "no reply".into())));
                        rows.push((p.name.clone(), ms));
                    }
                    if let Some((n, m)) = rows.iter().filter_map(|(n, m)| m.map(|m| (n, m))).min_by(|a, b| a.1.total_cmp(&b.1)) {
                        w.log(&format!("Fastest from your connection: {n} ({m} ms)"));
                    }
                    w.send(Msg::Bench(rows));
                });
            }
            if kit::ghost(ui, "Reset network stack", idle).clicked() && self.need_admin() {
                self.pending = Some(Pending { title: "Reset network stack".into(), intro: "Reset Winsock and the IP stack? You may briefly lose connection and need to restart Windows.".into(), lines: vec![], action: Action::ResetNetwork, restore_option: false, ok_label: "Reset network stack".into() });
            }
        });
        ui.add_space(S1);
        ui.label(RichText::new("The benchmark measures TCP connect time to port 53 of each provider's first server.").size(T_CAPTION).color(t.subtle));
        if !self.adapters.is_empty() {
            ui.label(RichText::new(format!("Active adapters: {}", self.adapters.join(", "))).size(T_CAPTION).color(t.subtle));
        }
        kit::group_title(ui, "DNS provider");
        let providers = self.data.dns.clone();
        ui.horizontal_wrapped(|ui| {
            for p in providers {
                let ms = self.bench.iter().find(|b| b.0 == p.name).and_then(|b| b.1).map(|m| format!("{m} ms"));
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
            ui.horizontal_wrapped(|ui| {
                for info in tools::TOOLS.iter() {
                    if kit::tile(ui, size, info.title, info.desc, None, idle, false).clicked() {
                        if info.changes_system {
                            if self.need_admin() {
                                self.pending = Some(Pending { title: info.title.into(), intro: info.desc.into(), lines: vec![], action: Action::RunTool(info.tool), restore_option: false, ok_label: info.title.into() });
                            }
                        } else {
                            let tool = info.tool;
                            self.spawn(ctx, info.title, move |w| tools::run(&WinSystem, tool, &backup_dir(), &mut |l| w.log(l)));
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
