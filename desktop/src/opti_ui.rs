//! Optimaxer section of the window: Tweaks, Services, Startup, Cleaner, Network and Tools pages.
//! Everything that reads or changes Windows runs on a worker thread and reports back through a channel, so the UI
//! never blocks. Every change goes through a confirmation window that lists exactly what will happen.

use crate::app::{chip, BRAND, LIME};
use crate::model::{format_size, AppItem};
use crate::net::Settings;
use crate::opti::backend::{System, WinSystem};
use crate::opti::catalog::{Catalog, Tweak};
use crate::opti::cleaner::{self, Target};
use crate::opti::engine::{Change, Engine};
use crate::opti::hw::{self, Traits};
use crate::opti::network;
use crate::opti::services::{self, Data, DnsProvider, SvcRow};
use crate::opti::startup;
use crate::opti::state::{backup_dir, State};
use crate::opti::tools::{self, Tool};
use egui::{Align, Color32, Layout, RichText, Ui, vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

const WARN: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);
const BAD: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
const LOG_LINES: usize = 600;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Tweaks,
    Services,
    Startup,
    Cleaner,
    Network,
    Tools,
}

impl Page {
    const ALL: [Page; 6] = [Page::Tweaks, Page::Services, Page::Startup, Page::Cleaner, Page::Network, Page::Tools];
    fn label(self) -> &'static str {
        match self {
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
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: Option<String>,
    log: Vec<String>,
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
}

/// Fixed-size button card used on the Tools page.
fn card(ui: &mut Ui, enabled: bool, text: RichText) -> bool {
    // not add_enabled_ui: a nested ui would stop the surrounding row from wrapping
    let text = if enabled { text } else { text.color(ui.visuals().weak_text_color()) };
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    ui.add_sized([262.0, 92.0], egui::Button::new(text).wrap().sense(sense)).clicked()
}

// region pure logic (unit tested)
/// (active, applicable) among the recommended tweaks that fit this PC.
pub fn score(cat: &Catalog, tr: &Traits, status: &HashMap<String, bool>) -> (usize, usize) {
    let applicable: Vec<&Tweak> = cat.tweaks.iter().filter(|t| (t.has_tag("safe") || t.has_tag("smart")) && !t.has_tag("optin") && hw::fits(t, tr)).collect();
    let on = applicable.iter().filter(|t| status.get(&t.id).copied().unwrap_or(false)).count();
    (on, applicable.len())
}

fn risk_colors(risk: &str) -> (Color32, Color32) {
    match risk {
        "Moderate" => (WARN.gamma_multiply(0.25), WARN),
        "Advanced" => (BAD.gamma_multiply(0.25), BAD),
        _ => (LIME.gamma_multiply(0.2), LIME),
    }
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
    n.join(", ")
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
        Self {
            page: Page::Tweaks,
            cat: Arc::new(Catalog::load()),
            data: Arc::new(Data::load()),
            traits: None,
            status: HashMap::new(),
            selected: HashSet::new(),
            tw_search: String::new(),
            tx,
            rx,
            busy: None,
            log: Vec::new(),
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
        }
    }

    /// Switches page by name. Used by the screenshot runner.
    pub fn show_page(&mut self, name: &str) {
        if let Some(p) = Page::ALL.iter().copied().find(|p| p.label().eq_ignore_ascii_case(name)) {
            self.page = p;
        }
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
                }
                Msg::Startup(s) => {
                    self.st_sel.clear();
                    self.startup = s;
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
            w.log("Optimaxer ready.");
        });
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
            w.send(Msg::Plan(Pending { title: "Apply tweaks".into(), intro, lines: plan_lines(&eng, &tweaks), action: Action::ApplyTweaks(ids), restore_option: true, ok_label: "Apply".into() }));
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
        self.pending = Some(Pending {
            title: "Undo tweaks".into(),
            intro: format!("Restore the original values saved for {} tweak(s)? Tweaks that were not applied by Optimaxer have nothing to restore.", ids.len()),
            lines,
            action: Action::UndoTweaks(ids),
            restore_option: false,
            ok_label: "Undo".into(),
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
            Action::UndoAll => {
                let cat = self.cat.clone();
                self.spawn(ctx, "Undo all Optimaxer tweaks", move |w| {
                    let sys = WinSystem;
                    let mut st = State::load();
                    let eng = Engine::new(&sys, backup_dir());
                    let ids: Vec<String> = st.records.keys().cloned().collect();
                    let mut n = 0;
                    for id in &ids {
                        if let Some(t) = cat.find(id) {
                            eng.undo(t, &mut st, &mut |l| w.log(l));
                            n += 1;
                        }
                    }
                    w.log(&format!("Reverted {n} tweak(s)."));
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
            if apply && restore_point {
                tools::create_restore_point(&sys, "Optimaxer - before tweaks", &mut |l| w.log(l));
            }
            let mut st = State::load();
            let eng = Engine::new(&sys, backup_dir());
            let (mut ok, mut restart, mut explorer) = (0, false, false);
            let mut now = HashMap::new();
            for id in &ids {
                let Some(t) = cat.find(id) else { continue };
                let on = if apply { eng.apply(t, &mut st, &mut |l| w.log(l)) } else { eng.undo(t, &mut st, &mut |l| w.log(l)) };
                restart |= t.restart;
                explorer |= t.explorer;
                if on == apply {
                    ok += 1;
                }
                now.insert(id.clone(), on);
            }
            if explorer {
                w.log("Restarting Explorer to apply interface changes");
                sys.run("taskkill.exe", &["/F", "/IM", "explorer.exe"]);
            }
            w.log(&format!("Finished: {ok} of {} tweak(s) verified.", ids.len()));
            if restart {
                w.log("Restart Windows for all changes to take full effect.");
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
        match preset {
            "none" => {}
            "auto" => {
                let plan = hw::smart_plan(&self.cat, &tr, &self.status);
                for n in &plan.notes {
                    self.say(&format!("  {n}"));
                }
                self.say(&format!("Auto-detect: {} tweak(s) selected for this PC ({}).", plan.ids.len(), tr.tags.join(", ")));
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
    pub fn side_panel(&mut self, ui: &mut Ui) {
        ui.add_space(8.0);
        ui.label(RichText::new("OPTIMAXER").size(11.0).color(ui.visuals().weak_text_color()));
        for p in Page::ALL {
            let selected = self.page == p;
            if ui.add_sized([ui.available_width(), 30.0], egui::Button::selectable(selected, p.label())).clicked() {
                self.page = p;
            }
        }
        ui.add_space(12.0);
        if let Some(t) = &self.traits {
            ui.label(RichText::new("THIS PC").size(11.0).color(ui.visuals().weak_text_color()));
            ui.label(RichText::new(format!("{:.0} GB RAM, {} threads", t.ram_gb, t.threads)).size(12.0));
            for g in &t.gpus {
                ui.label(RichText::new(g).size(12.0));
            }
            ui.label(RichText::new(t.tags.join(", ")).size(12.0).color(ui.visuals().weak_text_color()));
        }
    }

    pub fn log_panel(&mut self, root: &mut Ui) {
        egui::Panel::bottom("opti-log").resizable(true).default_size(120.0).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Activity").strong());
                if ui.small_button("Clear").clicked() {
                    self.log.clear();
                }
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(self.log.join("\n"));
                }
                if self.busy.is_some() {
                    ui.add(egui::Spinner::new().size(12.0));
                }
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for l in &self.log {
                    let color = if l.contains("WARN") || l.contains("failed") { WARN } else { ui.visuals().text_color() };
                    ui.label(RichText::new(l).monospace().size(12.0).color(color));
                }
            });
        });
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
    }

    pub fn central(&mut self, ui: &mut Ui, ctx: &egui::Context, suggestions: &[AppItem]) {
        if !self.started {
            self.load_first_time(ctx);
        }
        if !self.elevated {
            egui::Frame::new().fill(WARN.gamma_multiply(0.15)).corner_radius(10).inner_margin(10).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Not running as administrator. You can look around, but changes need elevation.").color(WARN));
                    if ui.button("Restart as administrator").clicked() {
                        if hw::relaunch_as_admin() {
                            self.events.push(OptiEvent::Close);
                        } else {
                            self.events.push(OptiEvent::Toast("Elevation was cancelled.".into()));
                        }
                    }
                });
            });
            ui.add_space(6.0);
        }
        ui.add_enabled_ui(self.pending.is_none(), |ui| match self.page {
            Page::Tweaks => self.tweaks_page(ui, ctx),
            Page::Services => self.services_page(ui, ctx),
            Page::Startup => self.startup_page(ui, ctx),
            Page::Cleaner => self.cleaner_page(ui, ctx),
            Page::Network => self.network_page(ui, ctx),
            Page::Tools => self.tools_page(ui, ctx, suggestions),
        });
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending.as_ref() else { return };
        let mut decision: Option<bool> = None;
        let mut restore = self.restore_first;
        egui::Window::new(p.title.clone()).collapsible(false).resizable(true).default_width(640.0).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label(&p.intro);
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                for l in &p.lines {
                    let text = if l.head { RichText::new(&l.text).strong() } else { RichText::new(format!("   {}", l.text)).size(12.5) };
                    ui.label(if l.warn { text.color(if l.head { BAD } else { WARN }) } else { text });
                }
            });
            ui.add_space(8.0);
            if p.restore_option {
                ui.checkbox(&mut restore, "Create a System Restore point first");
            }
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new(&p.ok_label).strong().color(Color32::BLACK)).fill(LIME).min_size(vec2(120.0, 30.0))).clicked() {
                    decision = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    decision = Some(false);
                }
            });
        });
        self.restore_first = restore;
        if let Some(ok) = decision {
            if let Some(p) = self.pending.take() {
                if ok {
                    self.confirm(ctx, p, restore);
                }
            }
        }
    }

    fn page_title(&self, ui: &mut Ui, title: &str, sub: &str) {
        ui.heading(title);
        ui.label(RichText::new(sub).size(12.5).color(ui.visuals().weak_text_color()));
        ui.add_space(4.0);
    }

    fn tweaks_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.page_title(ui, "Tweaks", "Performance, gaming, network, privacy and interface tweaks. Original values are saved, so each one can be undone.");
        // score and smart optimize
        let (on, total) = self.traits.as_ref().map(|t| score(&self.cat, t, &self.status)).unwrap_or((0, 0));
        ui.horizontal(|ui| {
            if total > 0 {
                let pct = (100.0 * on as f32 / total as f32).round();
                let col = if pct >= 80.0 { LIME } else if pct >= 45.0 { ui.visuals().text_color() } else { BAD };
                ui.label(RichText::new(format!("{pct:.0}%")).size(26.0).strong().color(col));
                ui.label(format!("{on} of {total} recommended tweaks are active on this PC"));
            } else {
                ui.label("Analyzing this PC...");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let smart = ui.add_enabled(self.busy.is_none() && self.traits.is_some(), egui::Button::new(RichText::new("Smart optimize").strong().color(Color32::BLACK)).fill(LIME).min_size(vec2(150.0, 32.0)));
                if smart.on_hover_text("Detects your hardware (laptop or desktop, SSD or HDD, RAM, GPU) and selects only the tweaks that fit this PC").clicked() {
                    self.select_preset("auto");
                    if !self.selected.is_empty() {
                        self.preview_apply(ctx);
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Presets").color(ui.visuals().weak_text_color()));
            for (label, tag) in [("Auto-detect", "auto"), ("Recommended", "safe"), ("Gaming", "gaming"), ("Privacy", "privacy"), ("Remove AI and debloat", "deai"), ("Maximum", "max"), ("Clear", "none")] {
                if ui.button(label).clicked() {
                    self.select_preset(tag);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.tw_search).hint_text("Search tweaks").desired_width(240.0));
            let n = self.selected.len();
            ui.label(if n > 0 { format!("{n} selected") } else { String::new() });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let idle = self.busy.is_none();
                if ui.add_enabled(idle, egui::Button::new("Refresh")).clicked() {
                    self.refresh_status(ctx);
                }
                if ui.add_enabled(idle, egui::Button::new("Undo selected")).clicked() {
                    self.preview_undo();
                }
                if ui.add_enabled(idle, egui::Button::new(RichText::new("Apply selected").strong())).clicked() {
                    self.preview_apply(ctx);
                }
            });
        });
        ui.add_space(4.0);

        let q = self.tw_search.trim().to_lowercase();
        let mut cats: Vec<&str> = Vec::new();
        for t in &self.cat.tweaks {
            if !cats.contains(&t.cat.as_str()) {
                cats.push(&t.cat);
            }
        }
        let cat = self.cat.clone();
        let traits = self.traits.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for c in cats {
                let rows: Vec<&Tweak> = cat.tweaks.iter().filter(|t| t.cat == c && (q.is_empty() || t.name.to_lowercase().contains(&q) || t.desc.to_lowercase().contains(&q))).collect();
                if rows.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(RichText::new(format!("{c}  ({})", rows.len())).strong()).id_salt(c).default_open(true).show(ui, |ui| {
                    for t in rows {
                        let applied = self.status.get(&t.id).copied().unwrap_or(false);
                        let fits = traits.as_ref().is_none_or(|tr| hw::fits(t, tr));
                        let mut on = self.selected.contains(&t.id);
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut on, "").changed() {
                                if on { self.selected.insert(t.id.clone()) } else { self.selected.remove(&t.id) };
                            }
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&t.name).strong());
                                    let (bg, fg) = risk_colors(&t.risk);
                                    chip(ui, if t.risk.is_empty() { "Safe" } else { &t.risk }, bg, fg);
                                    if applied {
                                        chip(ui, "Applied", LIME.gamma_multiply(0.25), LIME);
                                    }
                                    if !fits {
                                        chip(ui, "does not suit this PC", Color32::from_gray(70).gamma_multiply(0.4), ui.visuals().weak_text_color());
                                    }
                                });
                                ui.label(RichText::new(&t.desc).size(12.5).color(ui.visuals().weak_text_color()));
                                let notes = tweak_notes(t);
                                if !notes.is_empty() {
                                    ui.label(RichText::new(format!("[{notes}]")).size(11.5).color(ui.visuals().weak_text_color()));
                                }
                            });
                        });
                        ui.separator();
                    }
                });
            }
            if !cat.not_ported.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} tweaks exist only in the PowerShell edition of Optimaxer (they need cmdlets such as Appx or MMAgent):", cat.not_ported.len())).size(12.0).color(ui.visuals().weak_text_color()));
                ui.label(RichText::new(cat.not_ported.iter().map(|s| format!("{} ({})", s.name, s.id)).collect::<Vec<_>>().join(", ")).size(11.5).color(ui.visuals().weak_text_color()));
            }
        });
    }

    fn services_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        if self.services.is_empty() && self.busy.is_none() && !self.log.iter().any(|l| l.starts_with("Loaded ") && l.ends_with("services.")) {
            self.refresh_services(ctx);
        }
        self.page_title(ui, "Services", "Recommended Manual and Disabled lists. Core Windows services are hidden. A JSON backup is saved before every change.");
        let idle = self.busy.is_none();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.svc_search).hint_text("Search services").desired_width(220.0));
            if ui.button("Select recommended").clicked() {
                self.svc_sel = self.services.iter().filter(|s| s.rec.as_ref().is_some_and(|r| r.0 == 3)).map(|s| s.name.clone()).collect();
            }
            if ui.button("Clear").clicked() {
                self.svc_sel.clear();
            }
            ui.label(format!("{} selected", self.svc_sel.len()));
        });
        ui.horizontal(|ui| {
            let names: Vec<String> = self.svc_sel.iter().cloned().collect();
            for (label, start) in [("Set to Manual", 3u32), ("Disable", 4), ("Set to Automatic", 2)] {
                if ui.add_enabled(idle && !names.is_empty(), egui::Button::new(label)).clicked() && self.need_admin() {
                    self.pending = Some(Pending {
                        title: format!("{label}"),
                        intro: format!("Change {} service(s)? A backup of their current start types is saved first, so you can restore it.", names.len()),
                        lines: names.iter().map(|n| Line { text: n.clone(), warn: start == 4, head: false }).collect(),
                        action: Action::SetServices { names: names.clone(), start },
                        restore_option: false,
                        ok_label: "Apply".into(),
                    });
                }
            }
            if ui.add_enabled(idle, egui::Button::new("Restore last backup")).clicked() && self.need_admin() {
                self.pending = Some(Pending { title: "Restore services".into(), intro: "Put every service back to the start type saved in the most recent backup?".into(), lines: vec![], action: Action::RestoreServices, restore_option: false, ok_label: "Restore".into() });
            }
            if ui.add_enabled(idle, egui::Button::new("Refresh")).clicked() {
                self.refresh_services(ctx);
            }
        });
        ui.add_space(4.0);
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
                    ui.add_space(6.0);
                    ui.label(RichText::new(group).strong().color(BRAND));
                    last_group = group;
                }
                let mut on = self.svc_sel.contains(&s.name);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, "").changed() {
                        if on { self.svc_sel.insert(s.name.clone()) } else { self.svc_sel.remove(&s.name) };
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(if s.display == s.name { s.name.clone() } else { format!("{}  ({})", s.display, s.name) }).strong());
                            chip(ui, &format!("{} / {}", s.mode(), if s.running { "Running" } else { "Stopped" }), Color32::from_gray(70).gamma_multiply(0.4), ui.visuals().text_color());
                        });
                        let desc = s.rec.as_ref().map(|r| r.1.as_str()).filter(|d| !d.is_empty()).unwrap_or(&s.desc);
                        if !desc.is_empty() {
                            let shown: String = if desc.chars().count() > 170 { desc.chars().take(167).collect::<String>() + "..." } else { desc.to_string() };
                            ui.label(RichText::new(shown).size(12.5).color(ui.visuals().weak_text_color()));
                        }
                    });
                });
            }
        });
    }

    fn startup_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        if self.startup.is_empty() && self.busy.is_none() && !self.log.iter().any(|l| l.starts_with("Found ") && l.ends_with("startup items.")) {
            self.refresh_startup(ctx);
        }
        self.page_title(ui, "Startup", "Run-key and Startup-folder entries. Disabling uses the same switch as Task Manager; nothing is uninstalled.");
        let idle = self.busy.is_none();
        ui.horizontal(|ui| {
            let key = |i: &startup::Item| format!("{}|{}", i.scope, i.name);
            let picked: Vec<startup::Item> = self.startup.iter().filter(|i| self.st_sel.contains(&key(i))).cloned().collect();
            for (label, enable) in [("Disable selected", false), ("Enable selected", true)] {
                if ui.add_enabled(idle && !picked.is_empty(), egui::Button::new(label)).clicked() && self.need_admin() {
                    self.pending = Some(Pending {
                        title: label.into(),
                        intro: format!("{} {} startup item(s)? Machine-wide entries need administrator rights.", if enable { "Enable" } else { "Disable" }, picked.len()),
                        lines: picked.iter().map(|i| Line { text: format!("{}  ({})", i.name, i.scope), warn: false, head: false }).collect(),
                        action: Action::SetStartup { items: picked.clone(), enable },
                        restore_option: false,
                        ok_label: "Apply".into(),
                    });
                }
            }
            if ui.add_enabled(idle, egui::Button::new("Refresh")).clicked() {
                self.refresh_startup(ctx);
            }
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for i in &self.startup {
                let key = format!("{}|{}", i.scope, i.name);
                let mut on = self.st_sel.contains(&key);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, "").changed() {
                        if on { self.st_sel.insert(key.clone()) } else { self.st_sel.remove(&key) };
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&i.name).strong());
                            chip(ui, if i.enabled { "Enabled" } else { "Disabled" }, if i.enabled { LIME.gamma_multiply(0.2) } else { Color32::from_gray(70).gamma_multiply(0.4) }, if i.enabled { LIME } else { ui.visuals().weak_text_color() });
                            chip(ui, i.scope, BRAND.gamma_multiply(0.25), BRAND);
                        });
                        ui.label(RichText::new(&i.cmd).size(12.0).monospace().color(ui.visuals().weak_text_color()));
                    });
                });
                ui.separator();
            }
        });
    }

    fn cleaner_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.page_title(ui, "Cleaner", "Scan first, then clean what you tick. Directory links are never followed. Deleted files do not go to the Recycle Bin.");
        let idle = self.busy.is_none();
        let total: u64 = self.clean_sizes.values().flatten().sum();
        ui.horizontal(|ui| {
            if ui.add_enabled(idle, egui::Button::new("Scan")).clicked() {
                self.scan_cleaner(ctx);
            }
            let picked: Vec<&'static str> = cleaner::TARGETS.iter().map(|t| t.id).filter(|id| self.clean_sel.contains(id)).collect();
            if ui.add_enabled(idle && !picked.is_empty(), egui::Button::new(RichText::new("Clean selected").strong())).clicked() && self.need_admin() {
                let lines = picked
                    .iter()
                    .filter_map(|id| cleaner::TARGETS.iter().find(|t| t.id == *id))
                    .map(|t| Line { text: format!("{}  {}", t.name, self.clean_sizes.get(t.id).copied().flatten().map(|s| format_size(Some(s))).unwrap_or_default()), warn: t.risk != "Safe", head: false })
                    .collect();
                self.pending = Some(Pending { title: "Clean".into(), intro: "Delete the selected items? This cannot be undone.".into(), lines, action: Action::Clean(picked), restore_option: false, ok_label: "Delete".into() });
            }
            ui.label(if total > 0 { format!("Reclaimable: {}", format_size(Some(total))) } else { String::new() });
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for t in cleaner::TARGETS {
                self.cleaner_row(ui, t);
                ui.separator();
            }
        });
    }

    fn cleaner_row(&mut self, ui: &mut Ui, t: &Target) {
        let mut on = self.clean_sel.contains(t.id);
        ui.horizontal(|ui| {
            if ui.checkbox(&mut on, "").changed() {
                if on { self.clean_sel.insert(t.id) } else { self.clean_sel.remove(t.id) };
            }
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t.name).strong());
                    let (bg, fg) = risk_colors(t.risk);
                    chip(ui, t.risk, bg, fg);
                });
                ui.label(RichText::new(t.desc).size(12.5).color(ui.visuals().weak_text_color()));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let text = match self.clean_sizes.get(t.id) {
                    Some(Some(0)) => "empty".to_string(),
                    Some(Some(s)) => format_size(Some(*s)),
                    Some(None) => "on demand".to_string(),
                    None => String::new(),
                };
                ui.label(RichText::new(text).color(ui.visuals().weak_text_color()));
            });
        });
    }

    fn network_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.page_title(ui, "Network", "Switch DNS (with DNS-over-HTTPS where the provider offers it), flush the cache, reset the stack, or time the servers.");
        let idle = self.busy.is_none();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(idle, egui::Button::new("Show current DNS")).clicked() {
                self.spawn(ctx, "Reading DNS settings", |w| {
                    w.send(Msg::Adapters(network::active_adapters()));
                    network::show_dns(&WinSystem, &mut |l| w.log(l));
                });
            }
            if ui.add_enabled(idle, egui::Button::new("Flush DNS cache")).clicked() {
                self.spawn(ctx, "Flush DNS", |w| {
                    WinSystem.run("ipconfig.exe", &["/flushdns"]);
                    w.log("DNS cache flushed.");
                });
            }
            if ui.add_enabled(idle, egui::Button::new("Reset network stack")).clicked() && self.need_admin() {
                self.pending = Some(Pending { title: "Reset network stack".into(), intro: "Reset Winsock and the IP stack? You may briefly lose connection and need to restart.".into(), lines: vec![], action: Action::ResetNetwork, restore_option: false, ok_label: "Reset".into() });
            }
            if ui.add_enabled(idle, egui::Button::new("Benchmark DNS servers")).clicked() {
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
        });
        ui.label(RichText::new("The benchmark measures TCP connect time to port 53 of each provider's first server.").size(12.0).color(ui.visuals().weak_text_color()));
        if !self.adapters.is_empty() {
            ui.label(RichText::new(format!("Active adapters: {}", self.adapters.join(", "))).size(12.0).color(ui.visuals().weak_text_color()));
        }
        ui.add_space(8.0);
        let providers = self.data.dns.clone();
        ui.horizontal_wrapped(|ui| {
            for p in providers {
                let ms = self.bench.iter().find(|b| b.0 == p.name).and_then(|b| b.1);
                let resp = ui.add_sized([250.0, 74.0], egui::Button::new(RichText::new(format!("{}\n{}{}", p.name, p.desc, ms.map(|m| format!("\n{m} ms")).unwrap_or_default())).size(12.5)).wrap());
                if resp.clicked() && self.need_admin() {
                    self.pending = Some(Pending {
                        title: format!("DNS: {}", p.name),
                        intro: format!("Set DNS to {} on all active network adapters?", p.name),
                        lines: p.v4.iter().chain(p.v6.iter()).map(|s| Line { text: s.clone(), warn: false, head: false }).collect(),
                        action: Action::SetDns(p.clone()),
                        restore_option: false,
                        ok_label: "Apply".into(),
                    });
                }
            }
        });
    }

    fn tools_page(&mut self, ui: &mut Ui, ctx: &egui::Context, suggestions: &[AppItem]) {
        self.page_title(ui, "Tools", "Maintenance helpers. Anything that changes the system asks first.");
        let idle = self.busy.is_none();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for info in tools::TOOLS.iter() {
                    let text = RichText::new(format!("{}\n{}", info.title, info.desc)).size(12.5);
                    if card(ui, idle, text) {
                        if info.changes_system {
                            if self.need_admin() {
                                self.pending = Some(Pending { title: info.title.into(), intro: info.desc.into(), lines: vec![], action: Action::RunTool(info.tool), restore_option: false, ok_label: "Run".into() });
                            }
                        } else {
                            let tool = info.tool;
                            self.spawn(ctx, info.title, move |w| tools::run(&WinSystem, tool, &backup_dir(), &mut |l| w.log(l)));
                        }
                    }
                }
                if card(ui, idle, RichText::new("Undo ALL Optimaxer tweaks\nRestores every setting changed by Optimaxer to its original value.").size(12.5).color(BAD)) && self.need_admin() {
                    self.pending = Some(Pending { title: "Undo ALL tweaks".into(), intro: "Restore ALL settings changed by Optimaxer to their original values?".into(), lines: vec![], action: Action::UndoAll, restore_option: false, ok_label: "Undo all".into() });
                }
                if card(ui, true, RichText::new("Open System Restore\nRoll your PC back to a restore point.").size(12.5)) {
                    let _ = std::process::Command::new("rstrui.exe").spawn();
                }
                let vendor = self.traits.as_ref().map(|t| t.vendor.clone()).unwrap_or_default();
                let url = hw::vendor_url(&vendor);
                let label = match url {
                    Some(_) => format!("Vendor driver center\nOpens the official {vendor} download page in your browser. Nothing is downloaded or installed by Optimaxer."),
                    None => "Vendor driver center\nNo MSI, Acer or Gigabyte board detected; open your PC maker's support site manually.".to_string(),
                };
                if card(ui, url.is_some(), RichText::new(label).size(12.5)) {
                    if let Some(u) = url {
                        let _ = open::that(u);
                    }
                }
            });
            ui.add_space(14.0);
            ui.label(RichText::new("PC tools from the Orion catalog").strong().color(LIME));
            ui.label(RichText::new("Utility and Customization apps for Windows. Downloading stays your decision: open an app, pick a build and press Download.").size(12.5).color(ui.visuals().weak_text_color()));
            ui.horizontal(|ui| {
                if ui.button("Browse Utility apps").clicked() {
                    self.events.push(OptiEvent::OpenStoreCategory("Utility".into()));
                }
                if ui.button("Browse Customization apps").clicked() {
                    self.events.push(OptiEvent::OpenStoreCategory("Customization".into()));
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for a in suggestions {
                    if ui.button(format!("{}  ·  {}", a.name, a.category)).on_hover_text(&a.description).clicked() {
                        self.events.push(OptiEvent::OpenApp(a.id.clone()));
                    }
                }
            });
        });
    }
    // endregion
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
        assert!(total > 40);
        let all: HashMap<String, bool> = cat.tweaks.iter().map(|t| (t.id.clone(), true)).collect();
        assert_eq!(score(&cat, &t, &all), (total, total));
        assert_eq!(score(&cat, &t, &HashMap::new()).0, 0);
        let (_, laptop_total) = score(&cat, &tr(true), &HashMap::new());
        assert!(laptop_total <= total, "desktop-only tweaks must not count on a laptop");
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
    fn notes_describe_hardware_gates() {
        let cat = Catalog::load();
        let n = tweak_notes(cat.find("perf-paging").unwrap());
        assert!(n.contains("16 GB+ RAM") && n.contains("restart needed"));
        assert!(tweak_notes(cat.find("perf-ultimate").unwrap()).contains("desktop"));
    }

    #[test]
    fn risk_colors_distinguish_levels() {
        assert_ne!(risk_colors("Safe").1, risk_colors("Advanced").1);
        assert_eq!(risk_colors("").1, risk_colors("Safe").1);
    }
}
