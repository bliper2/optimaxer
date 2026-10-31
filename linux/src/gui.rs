//! Graphical interface (egui): the same pages as the terminal menu, a dry-run switch, a live log and nine themes.

use crate::catalog::*;
use crate::env::{Env, VERSION, fmt_size};
use crate::ops::*;
use crate::tweaks::Tweak;
use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Design tokens of the Windows app (desktop/DESIGN.md): a tonal ladder instead of lines, indigo for selection, lime for the one main action.
#[derive(Clone, Copy)]
struct Tok {
    name: &'static str,
    dark: bool,
    pill: bool,
    canvas: Color32,
    rail: Color32,
    s1: Color32,
    s2: Color32,
    s3: Color32,
    hair: Color32,
    text: Color32,
    muted: Color32,
    subtle: Color32,
    brand: Color32,
    brand_text: Color32,
    good: Color32,
    warn: Color32,
    bad: Color32,
    action: Color32,
    on_action: Color32,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const BRAND: Color32 = c(0x74, 0x78, 0xF8);
const LIME: Color32 = c(0xA3, 0xE6, 0x35);
const ON_LIME: Color32 = c(0x14, 0x1A, 0x05);

const MODES: [Tok; 4] = [
    Tok { name: "Dark", dark: true, pill: false, canvas: c(0x0F, 0x10, 0x15), rail: c(0x0B, 0x0C, 0x10), s1: c(0x17, 0x18, 0x21), s2: c(0x1F, 0x20, 0x2B), s3: c(0x2A, 0x2B, 0x3A),
        hair: c(0x2B, 0x2C, 0x39), text: c(0xEC, 0xED, 0xF3), muted: c(0xA9, 0xAC, 0xBD), subtle: c(0x8A, 0x8D, 0xA2), brand: BRAND, brand_text: c(0xA3, 0xA6, 0xFF),
        good: LIME, warn: c(0xF5, 0xA6, 0x23), bad: c(0xFF, 0x7A, 0x7A), action: LIME, on_action: ON_LIME },
    Tok { name: "Light", dark: false, pill: false, canvas: c(0xF6, 0xF5, 0xFC), rail: c(0xEE, 0xED, 0xF8), s1: c(0xFF, 0xFF, 0xFF), s2: c(0xF0, 0xEF, 0xF9), s3: c(0xE4, 0xE3, 0xF2),
        hair: c(0xDD, 0xDC, 0xEC), text: c(0x1B, 0x1C, 0x26), muted: c(0x52, 0x55, 0x6C), subtle: c(0x66, 0x69, 0x80), brand: BRAND, brand_text: c(0x46, 0x4B, 0xCC),
        good: c(0x3F, 0x6B, 0x0B), warn: c(0x8F, 0x52, 0x00), bad: c(0xB4, 0x23, 0x23), action: LIME, on_action: ON_LIME },
    Tok { name: "Material dark", dark: true, pill: true, canvas: c(0x14, 0x12, 0x18), rail: c(0x0F, 0x0D, 0x13), s1: c(0x21, 0x1F, 0x26), s2: c(0x2B, 0x29, 0x30), s3: c(0x36, 0x34, 0x3B),
        hair: c(0x49, 0x45, 0x4F), text: c(0xE6, 0xE0, 0xE9), muted: c(0xCA, 0xC4, 0xD0), subtle: c(0x9A, 0x94, 0xA6), brand: c(0xD0, 0xBC, 0xFF), brand_text: c(0xD0, 0xBC, 0xFF),
        good: c(0xA8, 0xDA, 0xB5), warn: c(0xF9, 0xDE, 0x8D), bad: c(0xF2, 0xB8, 0xB5), action: c(0xD0, 0xBC, 0xFF), on_action: c(0x38, 0x1E, 0x72) },
    Tok { name: "Material light", dark: false, pill: true, canvas: c(0xFE, 0xF7, 0xFF), rail: c(0xF3, 0xED, 0xF7), s1: c(0xFF, 0xFF, 0xFF), s2: c(0xF3, 0xED, 0xF7), s3: c(0xE8, 0xDE, 0xF8),
        hair: c(0xCA, 0xC4, 0xD0), text: c(0x1D, 0x1B, 0x20), muted: c(0x49, 0x45, 0x4F), subtle: c(0x62, 0x5B, 0x71), brand: c(0x67, 0x50, 0xA4), brand_text: c(0x67, 0x50, 0xA4),
        good: c(0x14, 0x6C, 0x2E), warn: c(0x7D, 0x57, 0x00), bad: c(0xB3, 0x26, 0x1E), action: c(0x67, 0x50, 0xA4), on_action: c(0xFF, 0xFF, 0xFF) },
];

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn apply_theme(ctx: &egui::Context, t: &Tok) {
    let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = if t.pill { 18 } else { 8 };
    v.panel_fill = t.canvas;
    v.window_fill = t.s1;
    v.window_stroke = egui::Stroke::new(1.0, t.hair);
    v.extreme_bg_color = t.s2;
    v.faint_bg_color = t.s1;
    v.override_text_color = Some(t.text);
    v.hyperlink_color = t.brand_text;
    v.selection.bg_fill = mix(t.canvas, t.brand, 0.28);
    v.selection.stroke = egui::Stroke::new(1.0, t.brand_text);
    for (w, fill) in [(&mut v.widgets.inactive, t.s2), (&mut v.widgets.hovered, t.s3), (&mut v.widgets.active, t.s3)] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = egui::Stroke::new(1.0, t.hair);
        w.corner_radius = egui::CornerRadius::same(r);
        w.fg_stroke = egui::Stroke::new(1.0, t.text);
    }
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, t.brand);
    v.widgets.noninteractive.bg_fill = t.s1;
    v.widgets.noninteractive.corner_radius = egui::CornerRadius::same(r);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, t.muted);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 7.0);
        s.spacing.interact_size.y = 34.0;
    });
}

#[derive(Clone, Copy)]
enum Tone {
    Neutral,
    Brand,
    Good,
    Warn,
    #[allow(dead_code)]
    Bad,
}

/// Tinted pill: the only place colour carries meaning, always with a word.
fn chip(ui: &mut egui::Ui, t: &Tok, text: &str, tone: Tone) {
    let fg = match tone {
        Tone::Neutral => t.muted,
        Tone::Brand => t.brand_text,
        Tone::Good => t.good,
        Tone::Warn => t.warn,
        Tone::Bad => t.bad,
    };
    egui::Frame::new().fill(mix(t.s1, fg, 0.16)).corner_radius(egui::CornerRadius::same(10)).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(12.0).color(fg));
    });
}

fn card<R>(ui: &mut egui::Ui, t: &Tok, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let r = if t.pill { 14 } else { 10 };
    egui::Frame::new().fill(t.s1).stroke(egui::Stroke::new(1.0, t.hair)).corner_radius(egui::CornerRadius::same(r)).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, add).inner
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Home,
    Install,
    Tweaks,
    Cleaner,
    Services,
    Startup,
    Debloat,
    Network,
    Tools,
    Config,
    Appearance,
}

const PAGES: &[(Page, &str)] = &[
    (Page::Home, "Overview"), (Page::Install, "Install"), (Page::Tweaks, "Tweaks"), (Page::Cleaner, "Cleaner"), (Page::Services, "Services"),
    (Page::Startup, "Startup"), (Page::Debloat, "Debloat"), (Page::Network, "Network"), (Page::Tools, "Tools"), (Page::Config, "Config"),
    (Page::Appearance, "Appearance"),
];

const NAV: &[(&str, &[Page])] = &[
    ("Workspace", &[Page::Home, Page::Install, Page::Config]),
    ("Optimization", &[Page::Tweaks, Page::Cleaner, Page::Services, Page::Startup, Page::Debloat]),
    ("System", &[Page::Network, Page::Tools, Page::Appearance]),
];

fn page_info(page: Page) -> (&'static str, &'static str) {
    match page {
        Page::Home => ("Overview", "This PC and what Optimaxer has changed."),
        Page::Install => ("Install apps", "Official repositories, the AUR and Flathub. P = official repo, A = AUR, F = Flatpak."),
        Page::Tweaks => ("Tweaks", "Every tweak saves the original values first, so each one can be undone on its own."),
        Page::Cleaner => ("Cleaner", "Scan first, then clean. Cleaning cannot be undone."),
        Page::Services => ("Services", "Notable systemd units. The original state is saved before every change."),
        Page::Startup => ("Startup", "Desktop autostart entries. System entries are overridden per user, never edited."),
        Page::Debloat => ("Debloat", "Optional packages you may not need. Nothing is pre-selected."),
        Page::Network => ("Network", "DNS provider for the active NetworkManager connections. The current settings are saved first."),
        Page::Tools => ("Tools", "System update, mirrors, diagnostics and undo."),
        Page::Config => ("Config", "Export or import what you ticked, and add Optimaxer to the application menu."),
        Page::Appearance => ("Appearance", "Same palette and shape options as the Windows app."),
    }
}

#[derive(Default)]
struct Data {
    tweaks: Vec<(Tweak, bool)>,
    apps: Vec<(App, bool)>,
    clean: Vec<(CleanTarget, i64)>,
    services: Vec<ServiceRow>,
    startup: Vec<StartupItem>,
    debloat: Vec<DebloatRow>,
    info: Vec<(String, String)>,
    saved: usize,
    loaded: BTreeSet<&'static str>,
}

#[derive(Clone)]
enum Action {
    ApplyTweaks(Vec<String>),
    UndoTweaks(Vec<String>),
    InstallApps(Vec<String>),
    RemoveApps(Vec<String>),
    Clean(Vec<&'static str>),
    Service(String, bool),
    RestoreService(String),
    Startup(String, bool),
    RemovePkgs(Vec<String>),
    SetDns(&'static str),
    RestoreDns,
    UndoAll,
    UpdateSystem,
    RankMirrors,
    CheckUpdate,
    ImportConfig(String),
}

impl Action {
    fn question(&self) -> Option<String> {
        Some(match self {
            Action::ApplyTweaks(v) => format!("Apply {} tweak(s)?", v.len()),
            Action::UndoTweaks(v) => format!("Undo {} tweak(s)?", v.len()),
            Action::InstallApps(v) => format!("Install {} app(s)?", v.len()),
            Action::RemoveApps(v) => format!("Remove {} app(s)?", v.len()),
            Action::Clean(v) => format!("Clean {} item(s)? This cannot be undone.", v.len()),
            Action::RemovePkgs(v) => format!("Remove {} package(s)?", v.len()),
            Action::SetDns(k) => format!("Set DNS to {}? Optimaxer saves the current connection settings first.", dns_provider(k)?.label),
            Action::RestoreDns => "Restore the saved DNS settings on Optimaxer-managed connections?".into(),
            Action::UndoAll => "Undo everything Optimaxer changed (tweaks, services, DNS)?".into(),
            Action::UpdateSystem => "Update the whole system now?".into(),
            Action::ImportConfig(_) => "Apply the imported selection (tweaks first, then apps)?".into(),
            _ => return None,
        })
    }
}

struct Gui {
    env: Env,
    page: Page,
    mode: usize,
    filter: String,
    data: Arc<Mutex<Data>>,
    log: Arc<Mutex<Vec<(String, String)>>>,
    busy: Arc<AtomicBool>,
    refresh: Arc<AtomicBool>,
    pending: Option<Action>,
    sel_tweaks: BTreeSet<String>,
    sel_apps: BTreeSet<String>,
    sel_clean: BTreeSet<&'static str>,
    sel_services: BTreeSet<String>,
    sel_startup: BTreeSet<String>,
    sel_debloat: BTreeSet<String>,
    dns_choice: usize,
    config_path: String,
    notice: String,
}

impl Gui {
    fn new(cc: &eframe::CreationContext<'_>, env: Env) -> Gui {
        let settings = load_settings(&env);
        let theme = settings.get("theme").and_then(|v| v.as_str()).and_then(|n| MODES.iter().position(|t| t.name == n)).unwrap_or(0);
        apply_theme(&cc.egui_ctx, &MODES[theme]);
        let mut env = env;
        let log: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let (l2, ctx) = (log.clone(), cc.egui_ctx.clone());
        env.quiet = true;
        env.log_hook = Some(Arc::new(move |line, level| {
            l2.lock().unwrap().push((line.to_string(), level.to_string()));
            ctx.request_repaint();
        }));
        let home = env.real_home();
        let mut g = Gui {
            env, page: Page::Home, mode: theme, filter: String::new(), data: Arc::default(), log, busy: Arc::default(), refresh: Arc::default(), pending: None,
            sel_tweaks: BTreeSet::new(), sel_apps: BTreeSet::new(), sel_clean: BTreeSet::new(), sel_services: BTreeSet::new(), sel_startup: BTreeSet::new(),
            sel_debloat: BTreeSet::new(), dns_choice: 0, config_path: format!("{}/optimaxer-config.json", home.trim_end_matches('/')), notice: String::new(),
        };
        g.load(&cc.egui_ctx, Page::Home);
        g
    }

    fn key(page: Page) -> &'static str {
        match page {
            Page::Home | Page::Tools | Page::Config | Page::Appearance | Page::Network => "info",
            Page::Install => "apps",
            Page::Tweaks => "tweaks",
            Page::Cleaner => "clean",
            Page::Services => "services",
            Page::Startup => "startup",
            Page::Debloat => "debloat",
        }
    }

    /// Scan the system for a page on a worker thread; commands can take a while.
    fn load(&mut self, ctx: &egui::Context, page: Page) {
        let key = Self::key(page);
        let (env, data, ctx) = (self.env.clone(), self.data.clone(), ctx.clone());
        std::thread::spawn(move || {
            match key {
                "info" => {
                    let info = sysinfo(&env);
                    let saved = undoable_count(&env);
                    let mut d = data.lock().unwrap();
                    d.info = info;
                    d.saved = saved;
                }
                "apps" => {
                    let apps: Vec<_> = load_apps(env.pm).into_iter().map(|a| { let i = app_installed(&env, &a); (a, i) }).collect();
                    data.lock().unwrap().apps = apps;
                }
                "tweaks" => {
                    let t: Vec<_> = tweaks().into_iter().filter(|t| t.available(&env)).map(|t| { let a = t.is_applied(&env); (t, a) }).collect();
                    data.lock().unwrap().tweaks = t;
                }
                "clean" => {
                    let c: Vec<_> = clean_targets(env.pm).into_iter().map(|t| { let s = target_size(&env, &t); (t, s) }).collect();
                    data.lock().unwrap().clean = c;
                }
                "services" => {
                    let s = service_rows(&env);
                    data.lock().unwrap().services = s;
                }
                "startup" => {
                    let s = autostart_items(&env);
                    data.lock().unwrap().startup = s;
                }
                _ => {
                    let d = installed_debloat(&env);
                    data.lock().unwrap().debloat = d;
                }
            }
            data.lock().unwrap().loaded.insert(key);
            ctx.request_repaint();
        });
    }

    fn run(&mut self, ctx: &egui::Context, action: Action) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (env, busy, refresh, ctx) = (self.env.clone(), self.busy.clone(), self.refresh.clone(), ctx.clone());
        std::thread::spawn(move || {
            let ok = execute(&env, action);
            env.log(if ok { "Done." } else { "Finished with problems: see the messages above." }, if ok { "OK" } else { "WARN" });
            busy.store(false, Ordering::SeqCst);
            refresh.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    fn set_theme(&mut self, ctx: &egui::Context, i: usize) {
        self.mode = i;
        apply_theme(ctx, &MODES[i]);
        let mut s = load_settings(&self.env);
        s.insert("theme".into(), MODES[i].name.into());
        save_settings(&self.env, &s);
    }

}

fn matches_f(filter: &str, text: &str) -> bool {
    filter.is_empty() || text.to_lowercase().contains(&filter.to_lowercase())
}

fn execute(env: &Env, action: Action) -> bool {
    let apps = load_apps(env.pm);
    let find = |pkg: &String| apps.iter().find(|a| &a.pkg == pkg).cloned().unwrap_or(App { cat: String::new(), name: pkg.clone(), src: 'P', pkg: pkg.clone() });
    // every item runs even if an earlier one fails
    let all = |results: Vec<bool>| results.iter().all(|&b| b);
    match action {
        Action::ApplyTweaks(ids) => all(ids.iter().map(|i| tweak_by_id(i).is_some_and(|t| t.apply(env))).collect()),
        Action::UndoTweaks(ids) => all(ids.iter().map(|i| tweak_by_id(i).is_some_and(|t| t.undo(env))).collect()),
        Action::InstallApps(pk) => all(pk.iter().map(|p| install_app(env, &find(p))).collect()),
        Action::RemoveApps(pk) => all(pk.iter().map(|p| remove_app(env, &find(p))).collect()),
        Action::Clean(ids) => {
            let targets = clean_targets(env.pm);
            let freed: u64 = ids.iter().filter_map(|i| targets.iter().find(|t| t.id == *i)).map(|t| clean_target(env, t)).sum();
            env.ok(&format!("Total freed: {}", fmt_size(freed as f64)));
            true
        }
        Action::Service(unit, enable) => set_service(env, &unit, enable),
        Action::RestoreService(unit) => restore_service(env, &unit),
        Action::Startup(file, enable) => autostart_items(env).iter().find(|i| i.file == file).is_some_and(|i| set_autostart(env, i, enable)),
        Action::RemovePkgs(pk) => all(pk
            .iter()
            .map(|p| {
                let ok = crate::sys::pkg_remove(env, std::slice::from_ref(p)).0 == 0;
                env.log(&format!("{p}: {}", if ok { "removed" } else { "not removed (other packages depend on it?)" }), if ok { "OK" } else { "WARN" });
                ok
            })
            .collect()),
        Action::SetDns(key) => set_dns(env, key),
        Action::RestoreDns => restore_dns(env),
        Action::UndoAll => undo_all(env),
        Action::UpdateSystem => tool_update_system(env),
        Action::RankMirrors => crate::sys::rank_mirrors(env),
        Action::CheckUpdate => self_update(env),
        Action::ImportConfig(path) => match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<ConfigFile>(&t).map_err(|e| e.to_string())) {
            Ok(cfg) => apply_config(env, &cfg),
            Err(e) => {
                env.warn(&format!("Cannot read {path}: {e}"));
                false
            }
        },
    }
}

impl Gui {
    /// A selectable row card: checkbox and title on the left, chips on the right, detail below.
    fn row(ui: &mut egui::Ui, t: &Tok, set: &mut BTreeSet<String>, id: &str, title: &str, chips: &[(&str, Tone)], detail: &str) {
        let mut on = set.contains(id);
        let fill = if on { mix(t.s1, t.brand, 0.14) } else { t.s1 };
        let r = if t.pill { 14 } else { 10 };
        let resp = egui::Frame::new().fill(fill).stroke(egui::Stroke::new(1.0, if on { t.brand } else { t.hair })).corner_radius(egui::CornerRadius::same(r)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if ui.checkbox(&mut on, RichText::new(title).strong()).changed() {
                    if on { set.insert(id.to_string()); } else { set.remove(id); }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (text, tone) in chips.iter().rev() {
                        chip(ui, t, text, *tone);
                    }
                });
            });
            if !detail.is_empty() {
                ui.label(RichText::new(detail).size(13.0).color(t.muted));
            }
        });
        let _ = resp;
    }

    fn select_buttons(ui: &mut egui::Ui, set: &mut BTreeSet<String>, ids: Vec<String>) {
        ui.horizontal(|ui| {
            if ui.small_button("Select all shown").clicked() {
                set.extend(ids);
            }
            if ui.small_button("Clear selection").clicked() {
                set.clear();
            }
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, filter: bool) {
        let t = MODES[self.mode];
        let (title, intro) = page_info(self.page);
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(22.0).strong());
        ui.label(RichText::new(intro).color(t.muted));
        if filter {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text("Filter").desired_width(220.0));
            });
        }
        ui.add_space(4.0);
    }

    /// Primary is lime (the one main action of a view); everything else is a raised surface.
    fn button(&mut self, ui: &mut egui::Ui, label: &str, primary: bool, enabled: bool, action: Action, ctx: &egui::Context) {
        let t = MODES[self.mode];
        let enabled = enabled && !self.busy.load(Ordering::SeqCst);
        let btn = if primary && enabled {
            egui::Button::new(RichText::new(label).strong().color(t.on_action)).fill(t.action).stroke(egui::Stroke::NONE)
        } else {
            egui::Button::new(label)
        };
        if ui.add_enabled(enabled, btn).clicked() {
            if action.question().is_some() {
                self.pending = Some(action);
            } else {
                self.run(ctx, action);
            }
        }
    }

    /// Footer for list pages: result count and the action buttons stay visible while the list scrolls.
    fn list_height(ui: &egui::Ui) -> f32 {
        (ui.available_height() - 56.0).max(120.0)
    }

    fn page_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let t = MODES[self.mode];
        let flt = self.filter.clone();
        let loaded = self.data.lock().unwrap().loaded.contains(Self::key(self.page));
        if !loaded {
            self.header(ui, false);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Scanning this PC...").color(t.muted));
            });
            return;
        }
        match self.page {
            Page::Home => {
                self.header(ui, false);
                let (info, saved) = {
                    let d = self.data.lock().unwrap();
                    (d.info.clone(), d.saved)
                };
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    egui::Grid::new("info").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                        for (k, v) in info {
                            ui.label(RichText::new(k).color(t.subtle));
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                });
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        chip(ui, &t, &format!("{saved} saved"), if saved > 0 { Tone::Brand } else { Tone::Neutral });
                        ui.label("changes can be undone. Nothing changes until you confirm; tick Dry run in the top bar to preview every command and file change.");
                    });
                });
            }
            Page::Install => {
                self.header(ui, true);
                let rows = self.data.lock().unwrap().apps.clone();
                let shown: Vec<_> = rows.iter().filter(|(a, _)| matches_f(&flt, &format!("{} {} {}", a.name, a.pkg, a.cat))).collect();
                Self::select_buttons(ui, &mut self.sel_apps, shown.iter().map(|(a, _)| a.pkg.clone()).collect());
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(Self::list_height(ui)).show(ui, |ui| {
                    let mut cat = "";
                    for (a, installed) in &shown {
                        if a.cat != cat {
                            cat = &a.cat;
                            ui.add_space(6.0);
                            ui.label(RichText::new(cat).strong().color(t.brand_text));
                        }
                        let chips: Vec<(&str, Tone)> = if *installed { vec![("Installed", Tone::Good)] } else { vec![] };
                        let src = match a.src { 'A' => "AUR", 'F' => "Flatpak", _ => "Repo" };
                        let mut all = vec![(src, Tone::Neutral)];
                        all.extend(chips);
                        Self::row(ui, &t, &mut self.sel_apps, &a.pkg, &a.name, &all, "");
                    }
                });
                let sel: Vec<String> = self.sel_apps.iter().cloned().collect();
                ui.horizontal(|ui| {
                    self.button(ui, &format!("Install {} app(s)", sel.len()), true, !sel.is_empty(), Action::InstallApps(sel.clone()), ctx);
                    self.button(ui, "Remove selected", false, !sel.is_empty(), Action::RemoveApps(sel), ctx);
                    self.button(ui, &format!("Update system ({})", self.env.pm.update_label()), false, true, Action::UpdateSystem, ctx);
                });
            }
            Page::Tweaks => {
                self.header(ui, true);
                let rows = self.data.lock().unwrap().tweaks.clone();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Presets").color(t.subtle));
                    for p in PRESETS {
                        if ui.small_button(p).on_hover_text("Select this preset (nothing is applied yet)").clicked() {
                            self.sel_tweaks = preset_ids(&self.env, p).into_iter().collect();
                        }
                    }
                    if ui.small_button("Clear selection").clicked() {
                        self.sel_tweaks.clear();
                    }
                });
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(Self::list_height(ui)).show(ui, |ui| {
                    let mut cat = "";
                    for (tw, applied) in rows.iter().filter(|(tw, _)| matches_f(&flt, &format!("{} {} {}", tw.name, tw.desc, tw.cat))) {
                        if tw.cat != cat {
                            cat = &tw.cat;
                            ui.add_space(6.0);
                            ui.label(RichText::new(cat).strong().color(t.brand_text));
                        }
                        let risk = if tw.risk == "safe" { ("Safe", Tone::Neutral) } else { ("Moderate risk", Tone::Warn) };
                        let mut chips = vec![risk];
                        if *applied {
                            chips.push(("Applied", Tone::Good));
                        }
                        Self::row(ui, &t, &mut self.sel_tweaks, &tw.id, &tw.name, &chips, &tw.desc);
                    }
                });
                let sel: Vec<String> = self.sel_tweaks.iter().cloned().collect();
                ui.horizontal(|ui| {
                    self.button(ui, &format!("Apply {} tweak(s)", sel.len()), true, !sel.is_empty(), Action::ApplyTweaks(sel.clone()), ctx);
                    self.button(ui, "Undo selected", false, !sel.is_empty(), Action::UndoTweaks(sel), ctx);
                });
            }
            Page::Cleaner => {
                self.header(ui, false);
                let rows = self.data.lock().unwrap().clean.clone();
                let mut sel: BTreeSet<String> = self.sel_clean.iter().map(|s| s.to_string()).collect();
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(Self::list_height(ui)).show(ui, |ui| {
                    for (c, size) in &rows {
                        let risk = if c.risk == "safe" { ("Safe", Tone::Neutral) } else { ("Moderate risk", Tone::Warn) };
                        Self::row(ui, &t, &mut sel, c.id, c.name, &[risk, (&size_label(*size), Tone::Brand)], "");
                    }
                });
                self.sel_clean = rows.iter().filter(|(c, _)| sel.contains(c.id)).map(|(c, _)| c.id).collect();
                let ids: Vec<&'static str> = self.sel_clean.iter().copied().collect();
                ui.horizontal(|ui| {
                    self.button(ui, &format!("Clean {} item(s)", ids.len()), true, !ids.is_empty(), Action::Clean(ids), ctx);
                    if ui.button("Select safe items").clicked() {
                        self.sel_clean = rows.iter().filter(|(c, _)| c.risk == "safe").map(|(c, _)| c.id).collect();
                    }
                });
            }
            Page::Services => {
                self.header(ui, true);
                let rows = self.data.lock().unwrap().services.clone();
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(Self::list_height(ui)).show(ui, |ui| {
                    for r in rows.iter().filter(|r| matches_f(&flt, &format!("{} {}", r.unit, r.label))) {
                        let mut chips = vec![if r.enabled { ("Enabled", Tone::Good) } else { ("Disabled", Tone::Neutral) }];
                        if r.active {
                            chips.push(("Running", Tone::Brand));
                        }
                        Self::row(ui, &t, &mut self.sel_services, &r.unit, &r.unit, &chips, &format!("{}: {}", r.label, r.hint));
                    }
                });
                let sel: Vec<String> = self.sel_services.iter().cloned().collect();
                let free = !sel.is_empty() && !self.busy.load(Ordering::SeqCst);
                ui.horizontal(|ui| {
                    if ui.add_enabled(free, egui::Button::new(RichText::new(format!("Disable {} service(s)", sel.len())).strong().color(t.on_action)).fill(t.action)).clicked() {
                        self.run_each(ctx, sel.iter().map(|u| Action::Service(u.clone(), false)).collect());
                    }
                    if ui.add_enabled(free, egui::Button::new("Enable selected")).clicked() {
                        self.run_each(ctx, sel.iter().map(|u| Action::Service(u.clone(), true)).collect());
                    }
                    if ui.add_enabled(free, egui::Button::new("Restore saved state")).clicked() {
                        self.run_each(ctx, sel.iter().map(|u| Action::RestoreService(u.clone())).collect());
                    }
                });
            }
            Page::Startup => {
                self.header(ui, true);
                let rows = self.data.lock().unwrap().startup.clone();
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(Self::list_height(ui)).show(ui, |ui| {
                    for r in rows.iter().filter(|r| matches_f(&flt, &format!("{} {}", r.name, r.exec))) {
                        let chips = [if r.enabled { ("Enabled", Tone::Good) } else { ("Disabled", Tone::Neutral) }, (r.scope, Tone::Neutral)];
                        Self::row(ui, &t, &mut self.sel_startup, &r.file, &r.name, &chips, &r.exec);
                    }
                });
                let sel: Vec<String> = self.sel_startup.iter().cloned().collect();
                let free = !sel.is_empty() && !self.busy.load(Ordering::SeqCst);
                ui.horizontal(|ui| {
                    if ui.add_enabled(free, egui::Button::new(RichText::new(format!("Disable {} entr(ies)", sel.len())).strong().color(t.on_action)).fill(t.action)).clicked() {
                        self.run_each(ctx, sel.iter().map(|f| Action::Startup(f.clone(), false)).collect());
                    }
                    if ui.add_enabled(free, egui::Button::new("Enable selected")).clicked() {
                        self.run_each(ctx, sel.iter().map(|f| Action::Startup(f.clone(), true)).collect());
                    }
                });
            }
            Page::Debloat => {
                self.header(ui, false);
                let rows: Vec<(String, String, u64)> = self.data.lock().unwrap().debloat.iter().map(|r| (r.pkg.clone(), r.label.clone(), r.size)).collect();
                if rows.is_empty() {
                    ui.label(RichText::new("None of the known optional packages are installed.").color(t.muted));
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(Self::list_height(ui)).show(ui, |ui| {
                    for (pkg, label, size) in &rows {
                        Self::row(ui, &t, &mut self.sel_debloat, pkg, label, &[(&fmt_size(*size as f64), Tone::Neutral)], pkg);
                    }
                });
                let sel: Vec<String> = self.sel_debloat.iter().cloned().collect();
                self.button(ui, &format!("Remove {} package(s)", sel.len()), true, !sel.is_empty(), Action::RemovePkgs(sel), ctx);
            }
            Page::Network => {
                self.header(ui, false);
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, d) in DNS.iter().enumerate() {
                        let ips = if d.v4.is_empty() { "router default".to_string() } else { d.v4.join(", ") };
                        ui.radio_value(&mut self.dns_choice, i, format!("{}   {ips}", d.label));
                    }
                });
                ui.horizontal(|ui| {
                    self.button(ui, "Set DNS", true, true, Action::SetDns(DNS[self.dns_choice].key), ctx);
                    self.button(ui, "Restore saved DNS", false, true, Action::RestoreDns, ctx);
                });
            }
            Page::Tools => {
                self.header(ui, false);
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Maintenance").strong());
                });
                ui.horizontal_wrapped(|ui| {
                    self.button(ui, &format!("Update the system ({})", self.env.pm.update_label()), true, true, Action::UpdateSystem, ctx);
                    self.button(ui, self.env.pm.mirror_label(), false, true, Action::RankMirrors, ctx);
                    self.button(ui, "Check for an Optimaxer update", false, true, Action::CheckUpdate, ctx);
                });
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Diagnostics (read-only, output goes to the activity log)").strong());
                });
                ui.horizontal_wrapped(|ui| {
                    let free = !self.busy.load(Ordering::SeqCst);
                    if ui.add_enabled(free, egui::Button::new("Show failed systemd units")).clicked() {
                        let u = tool_failed_units(&self.env);
                        self.env.info(&if u.is_empty() { "No failed units".to_string() } else { format!("Failed units: {}", u.join(", ")) });
                    }
                    if ui.add_enabled(free, egui::Button::new(self.env.pm.conf_label())).clicked() {
                        let f = tool_pacnew(&self.env);
                        self.env.info(&if f.is_empty() { "No leftover config files".to_string() } else { format!("Leftover config files:\n{}", f.join("\n")) });
                    }
                    if ui.add_enabled(free, egui::Button::new("Disk health (SMART)")).clicked() {
                        for l in tool_smart(&self.env) {
                            self.env.info(&l);
                        }
                    }
                });
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Undo").strong());
                    ui.label(RichText::new("Restores every tweak, service and DNS change Optimaxer saved an original for.").color(t.muted));
                });
                self.button(ui, "Undo everything Optimaxer changed", false, true, Action::UndoAll, ctx);
            }
            Page::Config => {
                self.header(ui, false);
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label("Export saves the ticked tweaks and apps to a file; Import applies a file (tweaks first, then apps). The same file works with  optimaxer config apply <file>.");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("File").color(t.subtle));
                        ui.add(egui::TextEdit::singleline(&mut self.config_path).desired_width(380.0));
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button("Export ticked selection").clicked() {
                        let cfg = config_file(self.sel_tweaks.iter().cloned().collect(), self.sel_apps.iter().cloned().collect());
                        self.notice = match std::fs::write(&self.config_path, serde_json::to_string_pretty(&cfg).unwrap_or_default()) {
                            Ok(()) => format!("Saved {} tweak(s) and {} app(s) to {}", cfg.tweaks.len(), cfg.apps.len(), self.config_path),
                            Err(e) => format!("Could not write the file: {e}"),
                        };
                        self.env.info(&self.notice.clone());
                    }
                    let path = self.config_path.clone();
                    self.button(ui, "Import and apply", true, true, Action::ImportConfig(path), ctx);
                });
                ui.add_space(8.0);
                if ui.button("Add Optimaxer to the application menu").clicked() {
                    let r = install_command(&self.env).and_then(|mut a| create_launcher(&self.env).map(|b| { a.extend(b); a }));
                    match r {
                        Ok(made) => made.iter().for_each(|p| self.env.ok(&format!("Created {p}"))),
                        Err(e) => self.env.warn(&format!("Setup failed: {e}")),
                    }
                }
            }
            Page::Appearance => {
                self.header(ui, false);
                let mut choice = self.mode;
                card(ui, &t, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, m) in MODES.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut choice, i, m.name);
                            for c in [m.canvas, m.s1, m.brand, m.action, m.good, m.warn, m.bad] {
                                let (r, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                                ui.painter().rect_filled(r, 4.0, c);
                            }
                        });
                    }
                });
                if choice != self.mode {
                    self.set_theme(ctx, choice);
                }
            }
        }
    }

    /// Run several actions one after another as a single job.
    fn run_each(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (env, busy, refresh, ctx) = (self.env.clone(), self.busy.clone(), self.refresh.clone(), ctx.clone());
        std::thread::spawn(move || {
            // fold, not all(): every action runs even when an earlier one fails
            #[allow(clippy::unnecessary_fold)]
            let ok = actions.into_iter().map(|a| execute(&env, a)).fold(true, |a, b| a && b);
            env.log(if ok { "Finished" } else { "Finished with problems: see the messages above." }, if ok { "OK" } else { "WARN" });
            busy.store(false, Ordering::SeqCst);
            refresh.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }
}

impl eframe::App for Gui {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        if self.refresh.swap(false, Ordering::SeqCst) {
            self.data.lock().unwrap().loaded.clear();
            let page = self.page;
            self.load(&ctx, page);
        }
        let t = MODES[self.mode];
        let busy = self.busy.load(Ordering::SeqCst);
        let distro = self.env.distro().get("PRETTY_NAME").and_then(|v| v.as_str()).unwrap_or("Linux").to_string();

        egui::Panel::top("top").show_separator_line(false).frame(egui::Frame::new().fill(t.rail).inner_margin(egui::Margin::symmetric(16, 8))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Optimaxer").size(16.0).strong().color(t.brand_text));
                chip(ui, &t, &format!("v{VERSION}"), Tone::Neutral);
                chip(ui, &t, &distro, Tone::Neutral);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut dry = self.env.dry_run;
                    if ui.add_enabled(!busy, egui::Checkbox::new(&mut dry, "Dry run")).on_hover_text("Show every command and file change without making any").changed() {
                        self.env.dry_run = dry;
                    }
                    if self.env.dry_run {
                        chip(ui, &t, "Nothing will change", Tone::Brand);
                    }
                    if busy {
                        ui.spinner();
                    }
                });
            });
        });

        egui::Panel::left("nav").resizable(false).show_separator_line(false).exact_size(200.0).frame(egui::Frame::new().fill(t.rail).inner_margin(egui::Margin::symmetric(10, 8))).show(root, |ui| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for (group, pages) in NAV {
                    ui.add_space(8.0);
                    ui.label(RichText::new(*group).size(12.0).color(t.subtle));
                    for page in *pages {
                        let name = PAGES.iter().find(|p| p.0 == *page).unwrap().1;
                        if ui.selectable_label(self.page == *page, RichText::new(name).color(if self.page == *page { t.brand_text } else { t.text })).clicked() && self.page != *page {
                            self.page = *page;
                            self.filter.clear();
                            self.data.lock().unwrap().loaded.remove(Self::key(*page));
                            self.load(&ctx, *page);
                        }
                    }
                }
            });
        });

        egui::Panel::bottom("log").resizable(true).default_size(150.0).frame(egui::Frame::new().fill(t.s1).inner_margin(egui::Margin::symmetric(16, 8))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Activity").strong());
                if ui.small_button("Clear").clicked() {
                    self.log.lock().unwrap().clear();
                }
            });
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for (line, level) in self.log.lock().unwrap().iter() {
                    let c = match level.as_str() {
                        "OK" => t.good,
                        "WARN" => t.warn,
                        "ERROR" => t.bad,
                        "DRY" => t.brand_text,
                        _ => t.text,
                    };
                    ui.label(RichText::new(line).monospace().size(12.5).color(c));
                }
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas).inner_margin(egui::Margin::symmetric(24, 8))).show(root, |ui| {
            self.page_ui(ui, &ctx);
        });

        if let Some(action) = self.pending.clone() {
            let question = action.question().unwrap_or_default();
            let mut decided: Option<bool> = None;
            egui::Window::new("Confirm").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(&ctx, |ui| {
                ui.label(&question);
                if self.env.dry_run {
                    ui.label(RichText::new("Dry run is on: nothing will actually change.").color(t.brand_text));
                }
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Yes, continue").strong().color(t.on_action)).fill(t.action)).clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
            if let Some(yes) = decided {
                self.pending = None;
                if yes {
                    self.run(&ctx, action);
                }
            }
        }
    }
}

fn window_icon() -> egui::IconData {
    image::load_from_memory(include_bytes!("../../assets/logo.png")).map(|i| {
        let rgba = i.into_rgba8();
        let (width, height) = rgba.dimensions();
        egui::IconData { rgba: rgba.into_raw(), width, height }
    }).unwrap_or_default()
}

pub fn run(env: Env) -> i32 {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Optimaxer")
            .with_app_id("optimaxer")
            .with_icon(window_icon())
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    match eframe::run_native("Optimaxer", options, Box::new(move |cc| Ok(Box::new(Gui::new(cc, env))))) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("The window could not start ({e}). Use the text menu: optimaxer --text");
            2
        }
    }
}
