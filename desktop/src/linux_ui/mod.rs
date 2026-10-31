//! Optimaxer section of the window for Linux. Same shell as the Windows edition (sidebar pages, operation center,
//! confirmation window) but only the pages that mean something on Linux. Everything that reads or changes the
//! machine runs on a worker thread and reports back through a channel; every change is shown as a preview first and
//! its original values are saved before it runs.

use crate::kit;
use crate::linux::cleaner::{self, Scan};
use crate::linux::network::{self, Adapter};
use crate::linux::processes::{ProcessRow, ProcessSampler};
use crate::linux::services::{self, ServiceRow};
use crate::linux::startup::{self, AutostartItem, UserUnit};
use crate::linux::store::State;
use crate::linux::sysmon::{self, Sampler, SysInfo};
use crate::linux::system::{Plan, RealSystem, System};
use crate::linux::tweaks::{self, Tweak};
use crate::model::AppItem;
use crate::net::Settings;
use crate::opti::history::{self, OperationEntry, OperationStatus};
use crate::theme::{
    S1, S2, S3, S4, T_BODY, T_CAPTION, T_HEADING, T_SMALL, Tone, mix, semibold, tokens,
};
use egui::{Align, Color32, FontId, Layout, Margin, RichText, Ui};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod activity_page;
mod cleaner_page;
mod network_page;
mod overview_page;
mod performance_page;
mod processes_page;
mod services_page;
mod startup_page;
mod tweaks_page;

const LOG_LINES: usize = 600;
const PROCESS_REFRESH: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Overview,
    Performance,
    Processes,
    Activity,
    Tweaks,
    Services,
    Startup,
    Cleaner,
    Network,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageGroup {
    Workspace,
    Optimization,
}

impl Page {
    const ALL: [Page; 9] = [
        Page::Overview,
        Page::Performance,
        Page::Processes,
        Page::Activity,
        Page::Tweaks,
        Page::Services,
        Page::Startup,
        Page::Cleaner,
        Page::Network,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Performance => "Performance",
            Page::Processes => "Processes",
            Page::Activity => "Activity",
            Page::Tweaks => "Tweaks",
            Page::Services => "Services",
            Page::Startup => "Startup",
            Page::Cleaner => "Cleaner",
            Page::Network => "Network",
        }
    }

    /// Segoe Fluent Icons glyphs; the sidebar skips them where that font is not installed (which is most of Linux).
    fn icon(self) -> char {
        match self {
            Page::Overview => '\u{E80F}',
            Page::Performance => '\u{E9D2}',
            Page::Processes => '\u{E9D9}',
            Page::Activity => '\u{E81C}',
            Page::Tweaks => '\u{E9E9}',
            Page::Services => '\u{E9F5}',
            Page::Startup => '\u{E7E8}',
            Page::Cleaner => '\u{E74D}',
            Page::Network => '\u{E774}',
        }
    }

    fn group(self) -> PageGroup {
        match self {
            Page::Overview | Page::Performance | Page::Processes | Page::Activity => {
                PageGroup::Workspace
            }
            _ => PageGroup::Optimization,
        }
    }

    fn live(self) -> bool {
        matches!(self, Page::Overview | Page::Performance)
    }
}

/// "startup" finds "Startup", "services" finds "Services": case, spaces and punctuation do not matter.
fn page_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Things the Optimaxer section asks the rest of the app to do.
#[allow(dead_code)]
pub enum OptiEvent {
    CheckForUpdates,
    OpenStoreCategory(String),
    OpenApp(String),
    Toast(String),
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentAction {
    pub title: String,
    pub hint: String,
    pub search: String,
}

struct Line {
    text: String,
    warn: bool,
}

impl Line {
    fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            warn: false,
        }
    }

    fn warn(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            warn: true,
        }
    }
}

type Job = Box<dyn FnOnce(&RealSystem) -> Result<Vec<String>, String> + Send>;

/// What to reload once a change finished, and which tweaks to double check.
#[derive(Clone, Default)]
enum Followup {
    #[default]
    None,
    Tweaks(Vec<&'static str>),
    Services,
    Startup,
    Cleaner,
    Network,
}

/// A change waiting for the user's confirmation. Nothing has happened yet.
struct Pending {
    title: String,
    intro: String,
    lines: Vec<Line>,
    ok_label: String,
    run: Job,
    /// Saved originals written to disk before the change runs.
    save_before: Option<State>,
    /// State written only after the change succeeded (undo removes its records here).
    save_after: Option<State>,
    followup: Followup,
    script: Option<String>,
}

struct TweakData {
    status: HashMap<&'static str, bool>,
    available: HashSet<&'static str>,
    pm: Option<&'static str>,
}

struct NetworkData {
    adapters: Vec<Adapter>,
    dns: Vec<String>,
    default_interface: Option<String>,
    connections: Result<Vec<network::Connection>, String>,
}

enum Msg {
    Tweaks(TweakData),
    Services(Result<Vec<ServiceRow>, String>),
    Startup(Vec<AutostartItem>, Result<Vec<UserUnit>, String>),
    Cleaner(Vec<Scan>),
    Network(NetworkData),
    Health(Option<Vec<String>>),
    Processes(Vec<ProcessRow>),
    Lookup(Vec<(String, Result<Duration, String>)>),
    Finished {
        title: String,
        result: Result<Vec<String>, String>,
        followup: Followup,
        save_after: Option<State>,
    },
}

pub struct OptiUi {
    page: Page,
    page_started: f64,
    started: bool,
    sys: Arc<RealSystem>,
    /// Asked once: looking for pkexec walks PATH, which is too much work to repeat every frame.
    is_root: bool,
    can_elevate: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    events: Vec<OptiEvent>,
    log: Vec<String>,
    log_expanded: bool,
    busy: Option<String>,
    pending: Option<Pending>,
    show_script: bool,
    state: State,
    operations: Vec<OperationEntry>,
    activity_search: String,
    // telemetry
    sampler: Sampler,
    info: SysInfo,
    failed_units: Option<Vec<String>>,
    // tweaks
    tweaks: Vec<Tweak>,
    tweak_status: HashMap<&'static str, bool>,
    tweak_available: HashSet<&'static str>,
    tweak_pm: Option<&'static str>,
    tweaks_loaded: bool,
    selected: HashSet<&'static str>,
    tw_search: String,
    focus_tweak_search: bool,
    verify: Vec<&'static str>,
    // services
    services: Option<Result<Vec<ServiceRow>, String>>,
    service_search: String,
    service_all: bool,
    // startup
    autostart: Vec<AutostartItem>,
    user_units: Option<Result<Vec<UserUnit>, String>>,
    startup_loaded: bool,
    // cleaner
    scans: Option<Vec<Scan>>,
    clean_selected: HashSet<&'static str>,
    // network
    network: Option<NetworkData>,
    lookups: Option<Vec<(String, Result<Duration, String>)>>,
    lookup_running: bool,
    // processes
    process_sampler: Arc<Mutex<ProcessSampler>>,
    processes: Vec<ProcessRow>,
    process_search: String,
    process_selected: Option<u32>,
    process_loading: bool,
    process_refreshed: Option<Instant>,
}

impl OptiUi {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let sys = Arc::new(RealSystem);
        let (state, state_note) = match State::load() {
            Ok(state) => (state, None),
            Err(error) => (State::default(), Some(error)),
        };
        let mut ui = Self {
            page: Page::Overview,
            page_started: 0.0,
            started: false,
            info: sysmon::read_info(sys.as_ref()),
            is_root: sys.is_root(),
            can_elevate: sys.can_elevate(),
            sys,
            tx,
            rx,
            events: Vec::new(),
            log: Vec::new(),
            log_expanded: false,
            busy: None,
            pending: None,
            show_script: false,
            state,
            operations: history::load_operations().unwrap_or_default(),
            activity_search: String::new(),
            sampler: Sampler::new(sysmon::TelemetryConfig::new(1, 60)),
            failed_units: None,
            tweaks: tweaks::catalog(),
            tweak_status: HashMap::new(),
            tweak_available: HashSet::new(),
            tweak_pm: None,
            tweaks_loaded: false,
            selected: HashSet::new(),
            tw_search: String::new(),
            focus_tweak_search: false,
            verify: Vec::new(),
            services: None,
            service_search: String::new(),
            service_all: false,
            autostart: Vec::new(),
            user_units: None,
            startup_loaded: false,
            scans: None,
            clean_selected: HashSet::new(),
            network: None,
            lookups: None,
            lookup_running: false,
            process_sampler: Arc::new(Mutex::new(ProcessSampler::new())),
            processes: Vec::new(),
            process_search: String::new(),
            process_selected: None,
            process_loading: false,
            process_refreshed: None,
        };
        if let Some(note) = state_note {
            ui.say(format!("WARN {note}"));
        }
        ui
    }

    // region plumbing
    fn say(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > LOG_LINES {
            self.log.drain(..self.log.len() - LOG_LINES);
        }
    }

    fn toast(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.say(message.clone());
        self.events.push(OptiEvent::Toast(message));
    }

    /// Runs `work` on a thread and delivers its message to the UI.
    fn spawn(&self, ctx: &egui::Context, work: impl FnOnce(&RealSystem) -> Msg + Send + 'static) {
        let (sys, tx, ctx) = (self.sys.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work(sys.as_ref()));
            ctx.request_repaint();
        });
    }

    fn set_page(&mut self, ctx: &egui::Context, page: Page) {
        if page.live() != self.page.live() {
            self.sampler.reset_window();
        }
        self.page = page;
        self.start_transition(ctx);
        self.load_page(ctx);
    }

    /// Loads what the page shows the first time it opens.
    fn load_page(&mut self, ctx: &egui::Context) {
        match self.page {
            Page::Overview => {
                self.refresh_health(ctx);
                self.refresh_tweaks(ctx);
            }
            Page::Tweaks if !self.tweaks_loaded => self.refresh_tweaks(ctx),
            Page::Services if self.services.is_none() => self.refresh_services(ctx),
            Page::Startup if !self.startup_loaded => self.refresh_startup(ctx),
            Page::Cleaner if self.scans.is_none() => self.refresh_cleaner(ctx),
            Page::Network if self.network.is_none() => self.refresh_network(ctx),
            _ => {}
        }
    }

    fn refresh_health(&self, ctx: &egui::Context) {
        self.spawn(ctx, |sys| Msg::Health(services::failed_units(sys)));
    }

    fn refresh_tweaks(&mut self, ctx: &egui::Context) {
        self.tweaks_loaded = true;
        self.spawn(ctx, |sys| {
            let tweaks = tweaks::catalog();
            let pm = tweaks::detect_pm(sys);
            Msg::Tweaks(TweakData {
                status: tweaks
                    .iter()
                    .map(|t| (t.id, tweaks::is_applied(sys, t)))
                    .collect(),
                available: tweaks
                    .iter()
                    .filter(|t| tweaks::available(sys, t, pm))
                    .map(|t| t.id)
                    .collect(),
                pm,
            })
        });
    }

    fn refresh_services(&mut self, ctx: &egui::Context) {
        self.services = None;
        self.spawn(ctx, |sys| Msg::Services(services::list(sys)));
    }

    fn refresh_startup(&mut self, ctx: &egui::Context) {
        self.startup_loaded = true;
        self.spawn(ctx, |sys| {
            Msg::Startup(startup::list_autostart(sys), startup::list_user_units(sys))
        });
    }

    fn refresh_cleaner(&mut self, ctx: &egui::Context) {
        self.scans = None;
        self.spawn(ctx, |sys| {
            Msg::Cleaner(cleaner::scan(sys, std::path::Path::new(&sys.home())))
        });
    }

    fn refresh_network(&mut self, ctx: &egui::Context) {
        self.network = None;
        self.spawn(ctx, |sys| {
            Msg::Network(NetworkData {
                adapters: network::adapters(sys),
                dns: network::dns_servers(sys),
                default_interface: crate::linux::system::read_text(sys, "/proc/net/route")
                    .and_then(|t| network::default_interface(&t)),
                connections: network::active_connections(sys),
            })
        });
    }

    fn refresh_processes(&mut self, ctx: &egui::Context) {
        if self.process_loading {
            return;
        }
        self.process_loading = true;
        let sampler = self.process_sampler.clone();
        self.spawn(ctx, move |sys| {
            let rows = sampler
                .lock()
                .map(|mut s| s.refresh(sys, Instant::now()))
                .unwrap_or_default();
            Msg::Processes(rows)
        });
    }

    /// Builds the confirmation for a plan: what will be written and run, and who will be asked for a password.
    #[allow(clippy::too_many_arguments)]
    fn plan_pending(
        &self,
        title: impl Into<String>,
        intro: impl Into<String>,
        mut lines: Vec<Line>,
        plan: Plan,
        ok_label: impl Into<String>,
        followup: Followup,
        save_before: Option<State>,
        save_after: Option<State>,
    ) -> Result<Pending, String> {
        if plan.is_empty() {
            return Err("Nothing needs to change.".into());
        }
        let elevated = plan.needs_root();
        if elevated && !self.can_elevate {
            return Err("Changing system settings needs administrator rights, and pkexec (polkit) was not found. Install polkit or start Optimaxer as root.".into());
        }
        let mut intro = intro.into();
        if elevated && !self.is_root {
            intro.push_str(
                " Your system will ask for your password (polkit) before anything is changed.",
            );
        }
        lines.extend(plan.describe().into_iter().map(Line::new));
        let script = elevated.then(|| plan.to_script());
        // One argument of a program call is limited to 128 KB by the kernel.
        if script.as_ref().is_some_and(|s| s.len() > 100_000) {
            return Err("That is too much to change in one go. Select fewer items.".into());
        }
        Ok(Pending {
            title: title.into(),
            intro,
            lines,
            ok_label: ok_label.into(),
            run: Box::new(move |sys| sys.execute(&plan)),
            save_before,
            save_after,
            followup,
            script,
        })
    }

    fn ask(&mut self, pending: Result<Pending, String>) {
        match pending {
            Ok(p) => {
                self.show_script = false;
                self.pending = Some(p);
            }
            Err(error) => self.toast(error),
        }
    }

    fn confirm(&mut self, ctx: &egui::Context, p: Pending) {
        if let Some(state) = p.save_before {
            if let Err(error) = state.save() {
                self.toast(format!(
                    "Nothing was changed: the original values could not be saved ({error})."
                ));
                return;
            }
            self.state = state;
        }
        self.say(format!("== {}", p.title));
        self.busy = Some(p.title.clone());
        let (title, followup, save_after, run) = (p.title, p.followup, p.save_after, p.run);
        self.spawn(ctx, move |sys| Msg::Finished {
            result: run(sys),
            title,
            followup,
            save_after,
        });
    }

    fn finish(
        &mut self,
        ctx: &egui::Context,
        title: String,
        result: Result<Vec<String>, String>,
        followup: Followup,
        save_after: Option<State>,
    ) {
        self.busy = None;
        let mut details = Vec::new();
        let status = match &result {
            Ok(lines) => {
                for line in lines {
                    self.say(format!("  {line}"));
                    history::capture_operation_detail(&mut details, line);
                }
                self.say(format!("Done: {title}"));
                if let Some(state) = save_after {
                    self.state = state;
                    if let Err(error) = self.state.save() {
                        self.say(format!(
                            "WARN the saved values could not be updated: {error}"
                        ));
                    }
                }
                OperationStatus::Finished
            }
            Err(error) => {
                for line in error.lines() {
                    self.say(format!("  WARN {line}"));
                    history::capture_operation_detail(&mut details, line);
                }
                self.say(format!("WARN {title} failed; saved original values were kept so it can be undone or retried."));
                OperationStatus::Error
            }
        };
        let entry = OperationEntry {
            time: crate::opti::now_iso(),
            title: title.clone(),
            status,
            details,
        };
        match history::append_operation(entry) {
            Ok(entries) => self.operations = entries,
            Err(error) => self.say(format!("WARN {error}")),
        }
        if result.is_ok() {
            match followup {
                Followup::Tweaks(ids) => {
                    self.verify = ids;
                    self.refresh_tweaks(ctx);
                }
                Followup::Services => self.refresh_services(ctx),
                Followup::Startup => self.refresh_startup(ctx),
                Followup::Cleaner => self.refresh_cleaner(ctx),
                Followup::Network => self.refresh_network(ctx),
                Followup::None => {}
            }
        } else {
            // A failed run may still have changed part of the system; show what is true now.
            match followup {
                Followup::Tweaks(_) => self.refresh_tweaks(ctx),
                Followup::Services => self.refresh_services(ctx),
                Followup::Startup => self.refresh_startup(ctx),
                Followup::Network => self.refresh_network(ctx),
                _ => {}
            }
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Tweaks(data) => {
                    self.tweak_status = data.status;
                    self.tweak_available = data.available;
                    self.tweak_pm = data.pm;
                    for id in std::mem::take(&mut self.verify) {
                        let name = tweaks::by_id(&self.tweaks, id).map_or(id, |t| t.name);
                        if self.tweak_status.get(id).copied().unwrap_or(false) {
                            self.say(format!("  Verified: {name} is in effect"));
                        } else {
                            self.say(format!("  WARN {name}: the settings are written but not in effect yet (a restart may be needed)"));
                        }
                    }
                }
                Msg::Services(rows) => self.services = Some(rows),
                Msg::Startup(items, units) => {
                    self.autostart = items;
                    self.user_units = Some(units);
                }
                Msg::Cleaner(scans) => self.scans = Some(scans),
                Msg::Network(data) => self.network = Some(data),
                Msg::Health(failed) => self.failed_units = failed,
                Msg::Processes(rows) => {
                    self.processes = rows;
                    self.process_loading = false;
                    self.process_refreshed = Some(Instant::now());
                }
                Msg::Lookup(results) => {
                    self.lookups = Some(results);
                    self.lookup_running = false;
                }
                Msg::Finished {
                    title,
                    result,
                    followup,
                    save_after,
                } => self.finish(ctx, title, result, followup, save_after),
            }
        }
    }
    // endregion

    // region app interface
    pub fn show_page(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(page) = Page::ALL
            .iter()
            .copied()
            .find(|p| page_key(p.label()) == page_key(name))
        {
            self.set_page(ctx, page);
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

    pub fn take_events(&mut self) -> Vec<OptiEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn status_text(&self) -> String {
        let access = if self.is_root {
            "Running as root"
        } else if self.can_elevate {
            "Changes ask for your password"
        } else {
            "Read-only (pkexec not found)"
        };
        match &self.busy {
            Some(b) => format!("Working: {b}   ·   {access}"),
            None => {
                let applied = self.tweak_status.values().filter(|on| **on).count();
                format!(
                    "{} tweaks available  ·  {applied} active  ·  {access}",
                    self.tweak_available.len()
                )
            }
        }
    }

    pub fn operation_entries(&self) -> &[OperationEntry] {
        &self.operations
    }

    pub fn recent_actions(&self, limit: usize) -> Vec<RecentAction> {
        self.operations
            .iter()
            .rev()
            .take(limit)
            .map(|e| RecentAction {
                title: e.title.clone(),
                hint: e.time.clone(),
                search: e.title.clone(),
            })
            .collect()
    }

    pub fn search_activity(&mut self, ctx: &egui::Context, query: &str) {
        self.activity_search = query.to_owned();
        self.set_page(ctx, Page::Activity);
    }

    pub fn current_job(&self) -> Option<&str> {
        self.busy.as_deref()
    }

    /// Windows runs a downloaded installer after a restore point. Linux downloads can be AppImages, packages or
    /// archives, and running them blindly is not safe, so the folder is opened for the user to decide.
    pub fn request_run_installer(&mut self, path: PathBuf) {
        let folder = path.parent().map(PathBuf::from).unwrap_or(path);
        match open::that(&folder) {
            Ok(()) => self.toast("Opened the download folder. Optimaxer does not run downloaded files on Linux; open it yourself once you trust it."),
            Err(error) => self.toast(format!("Could not open the folder: {error}")),
        }
    }

    pub fn suspend_telemetry(&mut self) {
        self.sampler.reset_window();
    }

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
        for group in [PageGroup::Workspace, PageGroup::Optimization] {
            if !collapsed {
                ui.add_space(S2);
                let name = if group == PageGroup::Workspace {
                    "Workspace"
                } else {
                    "Optimization"
                };
                ui.label(
                    RichText::new(name)
                        .font(FontId::new(T_CAPTION, semibold()))
                        .color(t.subtle),
                );
                ui.add_space(S1);
            }
            for page in Page::ALL.into_iter().filter(|p| p.group() == group) {
                if kit::nav_item_icon(ui, self.page == page, Some(page.icon()), page.label(), None)
                    .clicked()
                    && self.page != page
                {
                    self.set_page(ctx, page);
                }
            }
        }
        if !collapsed {
            ui.add_space(S4);
            ui.label(
                RichText::new("This PC")
                    .font(FontId::new(T_SMALL, semibold()))
                    .color(t.subtle),
            );
            ui.add_space(S1);
            if let Some(distro) = &self.info.distro {
                ui.add(egui::Label::new(RichText::new(distro).size(T_SMALL).color(t.muted)).wrap());
            }
            if let Some(total) = self.info.total_memory {
                let threads = if self.info.threads > 0 {
                    format!(", {} threads", self.info.threads)
                } else {
                    String::new()
                };
                ui.label(
                    RichText::new(format!("{}{threads}", crate::linux::fmt_size(total)))
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
            for gpu in &self.info.gpus {
                ui.add(egui::Label::new(RichText::new(gpu).size(T_SMALL).color(t.muted)).wrap());
            }
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
        if open_activity {
            self.set_page(root.ctx(), Page::Activity);
        }
    }

    /// Runs every frame, whichever section is showing: collects worker results and draws the confirmation window.
    pub fn tick(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        self.poll(ctx);
        self.sampler.configure(settings.telemetry_config());
        self.confirm_window(ctx);
        if self.busy.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    pub fn central(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        _suggestions: &[AppItem],
        _settings: &mut Settings,
    ) {
        if !self.started {
            self.started = true;
            self.load_page(ctx);
        }
        let t = tokens(ctx);
        let now = Instant::now();
        if self.page.live() && self.sampler.due(now) {
            self.sampler.sample(self.sys.as_ref(), now);
        }
        if self.page.live() {
            ctx.request_repaint_after(self.sampler.config().interval());
        }
        if self.page == Page::Processes {
            if self
                .process_refreshed
                .is_none_or(|at| at.elapsed() >= PROCESS_REFRESH)
            {
                self.refresh_processes(ctx);
            }
            ctx.request_repaint_after(PROCESS_REFRESH);
        }
        if !self.can_elevate {
            egui::Frame::new().fill(mix(t.canvas, t.warn, 0.14)).corner_radius(t.card_radius()).inner_margin(Margin::symmetric(S4 as i8, S3 as i8)).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new("pkexec (polkit) was not found, so Optimaxer can read everything but cannot change system settings. Install polkit or start Optimaxer as root. Changes inside your home folder still work.").color(t.text)).wrap());
            });
            ui.add_space(S3);
        }
        let started = self.page_started;
        kit::enter(ui, started, |ui| {
            ui.add_enabled_ui(self.pending.is_none(), |ui| {
                if self.page == Page::Tweaks {
                    ui.vertical(|ui| {
                        let scroll_height = (ui.available_height() - 76.0).max(120.0);
                        egui::ScrollArea::vertical()
                            .id_salt(("optimaxer-linux-page", self.page.label()))
                            .auto_shrink([false, false])
                            .max_height(scroll_height)
                            .show(ui, |ui| self.tweaks_page(ui, ctx));
                        self.tweak_action_bar(ui, ctx);
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(("optimaxer-linux-page", self.page.label()))
                        .auto_shrink([false, false])
                        .show(ui, |ui| match self.page {
                            Page::Overview => self.overview_page(ui, ctx),
                            Page::Performance => self.performance_page(ui, ctx),
                            Page::Processes => self.processes_page(ui, ctx),
                            Page::Activity => self.activity_page(ui, ctx),
                            Page::Tweaks => {}
                            Page::Services => self.services_page(ui, ctx),
                            Page::Startup => self.startup_page(ui, ctx),
                            Page::Cleaner => self.cleaner_page(ui, ctx),
                            Page::Network => self.network_page(ui, ctx),
                        });
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
        let mut show_script = self.show_script;
        let resp = egui::Modal::new(egui::Id::NULL.with("opti-linux-confirm"))
            .backdrop_color(Color32::from_black_alpha(if t.dark { 150 } else { 70 }))
            .show(ctx, |ui| {
                ui.set_width(640.0);
                ui.label(
                    RichText::new(&p.title)
                        .font(FontId::new(crate::theme::T_TITLE, semibold()))
                        .color(t.text),
                );
                ui.add_space(S1);
                ui.add(egui::Label::new(RichText::new(&p.intro).color(t.muted)).wrap());
                if !p.lines.is_empty() {
                    ui.add_space(S3);
                    kit::card(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for l in &p.lines {
                                    let color = if l.warn { t.warn } else { t.muted };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&l.text).size(T_SMALL).color(color),
                                        )
                                        .wrap(),
                                    );
                                }
                            });
                    });
                }
                if let Some(script) = &p.script {
                    ui.add_space(S2);
                    ui.checkbox(
                        &mut show_script,
                        "Show the exact script that will run as administrator",
                    );
                    if show_script {
                        egui::ScrollArea::vertical()
                            .id_salt("opti-linux-script")
                            .max_height(160.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(script)
                                            .monospace()
                                            .size(T_CAPTION)
                                            .color(t.muted),
                                    )
                                    .wrap(),
                                );
                            });
                    }
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
                        if kit::ghost(ui, "Copy details", true).clicked() {
                            let mut text = format!("{}\n{}\n", p.title, p.intro);
                            for l in &p.lines {
                                text.push_str(&l.text);
                                text.push('\n');
                            }
                            ui.ctx().copy_text(text);
                        }
                    });
                });
            });
        self.show_script = show_script;
        if resp.should_close() && decision.is_none() {
            decision = Some(false);
        }
        if let Some(ok) = decision
            && let Some(p) = self.pending.take()
            && ok
        {
            self.confirm(ctx, p);
        }
    }

    fn badge(ui: &mut Ui, text: &str, tone: Tone) {
        kit::chip(ui, text, tone);
    }

    fn heading(ui: &mut Ui, text: &str) {
        let t = tokens(ui.ctx());
        ui.label(
            RichText::new(text)
                .font(FontId::new(T_HEADING, semibold()))
                .color(t.text),
        );
    }
    // endregion
}

/// Draws every page for a few frames against the machine running the tests, so a panic in layout or in a loader
/// shows up in CI instead of on a user's desktop. Unix only: it uses the real system.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn every_page_draws_with_real_data_without_panicking() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        crate::theme::apply(&ctx, settings.appearance());
        let mut oui = OptiUi::new();
        for page in Page::ALL {
            oui.set_page(&ctx, page);
            for _ in 0..8 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 720.0),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run_ui(input, |ui| {
                    oui.tick(ui.ctx(), &mut settings);
                    oui.side_panel(ui, &ctx, false);
                    oui.log_panel(ui);
                    oui.central(ui, &ctx, &[], &mut settings);
                });
                std::thread::sleep(Duration::from_millis(60));
            }
        }
        assert!(!oui.status_text().is_empty());
    }

    #[test]
    fn palette_names_find_pages_and_unknown_names_are_ignored() {
        let ctx = egui::Context::default();
        let mut oui = OptiUi::new();
        oui.show_page(&ctx, "startup");
        assert_eq!(oui.page, Page::Startup);
        oui.show_page(&ctx, "Startup X-Ray");
        assert_eq!(
            oui.page,
            Page::Startup,
            "pages that exist only on Windows are not reachable here"
        );
        oui.show_page(&ctx, "Network");
        assert_eq!(oui.page, Page::Network);
    }
}
