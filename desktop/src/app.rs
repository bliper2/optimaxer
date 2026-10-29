use crate::kit;
use crate::model::{self, AppItem, Catalog, Sort, ANDROID, PC, TV};
use crate::net::{self, DlStatus, Download, Settings};
use crate::opti_ui::{OptiEvent, OptiUi};
use crate::update::{self, Release};
use crate::theme::{self, ease_out, mix, motion, semibold, tokens, Tone, R_LG, R_MD, S1, S2, S3, S4, S5, T_BODY, T_CAPTION, T_DISPLAY, T_HEADING, T_SMALL};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, StrokeKind, Ui, WidgetInfo, Role, vec2};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CARD_H: f32 = 108.0;
const GAP: f32 = 12.0;
const PLATFORMS: [&str; 4] = [PC, ANDROID, TV, "All"];

enum UpdateState {
    Idle,
    Checking,
    Available(Release),
    Downloading { rel: Release, done: u64, total: u64 },
    Failed(String),
}

enum UpdateMsg {
    Checked { manual: bool, result: Result<Option<Release>, String> },
    Progress(u64, u64),
    Installed(Result<(), String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Store,
    Optimaxer,
}

#[derive(Clone, Copy)]
enum PaletteCommand {
    Page(&'static str),
    Store,
    SearchStore,
    Downloads,
    Settings,
}

const PALETTE_COMMANDS: [(&str, &str, PaletteCommand); 13] = [
    ("Overview", "Optimaxer page", PaletteCommand::Page("Overview")),
    ("Processes", "Read-only process explorer", PaletteCommand::Page("Processes")),
    ("Activity", "Search confirmed tweak changes", PaletteCommand::Page("Activity")),
    ("Tweaks", "Optimaxer page", PaletteCommand::Page("Tweaks")),
    ("Services", "Optimaxer page", PaletteCommand::Page("Services")),
    ("Startup", "Optimaxer page", PaletteCommand::Page("Startup")),
    ("Cleaner", "Optimaxer page", PaletteCommand::Page("Cleaner")),
    ("Network", "Optimaxer page", PaletteCommand::Page("Network")),
    ("Tools", "Optimaxer page", PaletteCommand::Page("Tools")),
    ("Store", "Browse apps", PaletteCommand::Store),
    ("Search apps", "Focus Store search", PaletteCommand::SearchStore),
    ("Downloads", "Show download activity", PaletteCommand::Downloads),
    ("Settings", "Open application settings", PaletteCommand::Settings),
];

fn matching_palette_commands(query: &str) -> Vec<&'static (&'static str, &'static str, PaletteCommand)> {
    let query = query.trim().to_lowercase();
    PALETTE_COMMANDS.iter().filter(|(name, hint, _)| query.is_empty() || name.to_lowercase().contains(&query) || hint.to_lowercase().contains(&query)).collect()
}

/// PC tools from the catalog that Optimaxer offers on its Tools page.
fn suggest(apps: &[AppItem]) -> Vec<AppItem> {
    let mut v: Vec<AppItem> = apps.iter().filter(|a| a.platform == PC && matches!(a.category.as_str(), "Utility" | "Customization")).cloned().collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v.truncate(12);
    v
}

fn is_installer(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "exe" | "msi" | "msix" | "appx" | "msixbundle" | "appxbundle"))
}

pub struct OrionApp {
    shots: Option<crate::shots::Shots>,
    section: Section,
    opti: OptiUi,
    suggestions: Vec<AppItem>,
    settings: Settings,
    catalog: Catalog,
    rx: Receiver<Catalog>,
    loading: bool,
    note: String,
    query: String,
    category: String,
    view: Vec<AppItem>,
    featured: Vec<AppItem>,
    dirty: bool,
    detail: Option<AppItem>,
    sel_stream: usize,
    sel_variant: usize,
    downloads: Vec<Arc<Download>>,
    show_downloads: bool,
    show_settings: bool,
    show_palette: bool,
    palette_query: String,
    palette_selected: usize,
    /// Screenshots of the open app and the one shown full size, if any.
    lightbox: Option<(Vec<String>, usize)>,
    toast: Option<(String, Instant)>,
    search_id: egui::Id,
    /// `ctx.input(|i| i.time)` of the last view switch, for the page-enter transition.
    view_started: f64,
    update: UpdateState,
    update_tx: Sender<UpdateMsg>,
    update_rx: Receiver<UpdateMsg>,
    update_cancel: Arc<AtomicBool>,
    /// Result of the last update check, shown in Settings.
    update_note: String,
    /// Windows' dark mode and accent colour as last read, so the app follows them while it runs.
    sys_look: (bool, Color32),
    sys_checked: Instant,
}

fn spawn_refresh(ctx: &egui::Context) -> Receiver<Catalog> {
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(net::load_catalog());
        ctx.request_repaint();
    });
    rx
}

fn daily_seed() -> u32 {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0);
    days as u32
}

/// Deterministic per-day shuffle so "Featured Today" is stable within a day.
fn shuffled(apps: &[AppItem], seed: u32) -> Vec<AppItem> {
    let mut v = apps.to_vec();
    let mut a = seed.wrapping_mul(2654435761).wrapping_add(0x6D2B79F5);
    for i in (1..v.len()).rev() {
        a = a.wrapping_add(0x6D2B79F5);
        let mut t = a;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        let r = (t ^ (t >> 14)) as f64 / 4294967296.0;
        v.swap(i, (r * (i + 1) as f64) as usize);
    }
    v
}

impl OrionApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::install_fonts(&cc.egui_ctx);
        let mut settings = Settings::load();
        if std::env::var_os("ORION_SHOTS").is_some() {
            // developer screenshots: nothing below may be written to the user's settings file
            net::READ_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Ok(collapsed) = std::env::var("ORION_SHOTS_SIDEBAR_COLLAPSED") {
                settings.sidebar_collapsed = collapsed.eq_ignore_ascii_case("true");
            }
            if let Ok(restart_required) = std::env::var("ORION_SHOTS_RESTART_REQUIRED") {
                settings.opti_restart_required = restart_required.eq_ignore_ascii_case("true");
                settings.opti_restart_uptime = crate::opti::sysmon::uptime_secs();
            }
            if let Ok(t) = std::env::var("ORION_SHOTS_THEME") {
                settings.theme = t;
            }
            if let Ok(p) = std::env::var("ORION_SHOTS_PLATFORM") {
                settings.platform = p;
            }
            match std::env::var("ORION_SHOTS_STYLE").as_deref() {
                Ok("classic") => settings.style = theme::Style::Classic,
                Ok("material") => settings.style = theme::Style::Material,
                Ok("windows") => settings.style = theme::Style::Windows,
                _ => {}
            }
        }
        if settings.clear_restart_after_reboot(crate::opti::sysmon::uptime_secs()) {
            settings.save();
        }
        theme::apply(&cc.egui_ctx, settings.effective_dark(), settings.reduce_motion, settings.style, settings.palette, settings.accent());
        let catalog = net::load_cache().unwrap_or_else(net::embedded);
        let rx = spawn_refresh(&cc.egui_ctx);
        let (update_tx, update_rx) = channel();
        update::cleanup_old();
        let auto_check = settings.auto_update && std::env::var_os("ORION_SHOTS").is_none();
        let mut app = Self {
            shots: crate::shots::Shots::from_env(),
            section: if settings.section == "Optimaxer" { Section::Optimaxer } else { Section::Store },
            opti: OptiUi::new(),
            suggestions: suggest(&catalog.apps),
            settings,
            catalog,
            rx,
            loading: true,
            note: String::new(),
            query: if std::env::var_os("ORION_SHOTS").is_some() { std::env::var("ORION_SHOTS_QUERY").unwrap_or_default() } else { String::new() },
            category: "All".into(),
            view: Vec::new(),
            featured: Vec::new(),
            dirty: true,
            detail: None,
            sel_stream: 0,
            sel_variant: 0,
            downloads: Vec::new(),
            show_downloads: false,
            show_settings: false,
            show_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            lightbox: None,
            toast: None,
            search_id: egui::Id::unique("orion-search"),
            view_started: 0.0,
            update: UpdateState::Idle,
            update_tx,
            update_rx,
            update_cancel: Arc::new(AtomicBool::new(false)),
            update_note: String::new(),
            sys_look: (theme::system_dark(), theme::system_accent()),
            sys_checked: Instant::now(),
        };
        if auto_check {
            app.start_check(&cc.egui_ctx, false);
        }
        if std::env::var_os("ORION_SHOTS_FAKE_UPDATE").is_some() {
            // developer screenshots: show the update banner without a real release
            app.update = UpdateState::Available(Release { version: (9, 9, 9), tag: "v9.9.9".into(), notes: String::new(), page: String::new(), url: String::new(), digest: String::new(), size: 0 });
        }
        app
    }

    fn reapply_theme(&mut self, ctx: &egui::Context) {
        theme::apply(ctx, self.settings.effective_dark(), self.settings.reduce_motion, self.settings.style, self.settings.palette, self.settings.accent());
        self.sys_look = (theme::system_dark(), theme::system_accent());
    }

    /// Follows Windows' dark mode and accent colour while the app runs (checked every couple of seconds).
    fn follow_windows(&mut self, ctx: &egui::Context) {
        if self.sys_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.sys_checked = Instant::now();
        let now = (theme::system_dark(), theme::system_accent());
        if now != self.sys_look && (self.settings.theme == "system" || self.settings.style == theme::Style::Windows) {
            self.reapply_theme(ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(2));
    }

    fn start_check(&mut self, ctx: &egui::Context, manual: bool) {
        self.update = UpdateState::Checking;
        let (tx, ctx) = (self.update_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = update::check();
            let _ = tx.send(UpdateMsg::Checked { manual, result });
            ctx.request_repaint();
        });
    }

    /// Downloads, verifies and installs a release, then restarts. Runs only after the user presses the button.
    fn start_install(&mut self, ctx: &egui::Context, rel: Release) {
        self.update_cancel.store(false, Ordering::Relaxed);
        self.update = UpdateState::Downloading { rel: rel.clone(), done: 0, total: rel.size };
        let (tx, ctx, cancel) = (self.update_tx.clone(), ctx.clone(), self.update_cancel.clone());
        std::thread::spawn(move || {
            let (ptx, pctx) = (tx.clone(), ctx.clone());
            let result = update::install(
                &rel,
                &move |d, t| {
                    let _ = ptx.send(UpdateMsg::Progress(d, t));
                    pctx.request_repaint();
                },
                &|| cancel.load(Ordering::Relaxed),
            );
            let _ = tx.send(UpdateMsg::Installed(result));
            ctx.request_repaint();
        });
    }

    fn poll_update(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.update_rx.try_recv() {
            match m {
                UpdateMsg::Checked { manual, result } => match result {
                    Ok(Some(rel)) => {
                        self.update_note = format!("Version {} is available.", rel.label());
                        self.update = UpdateState::Available(rel);
                    }
                    Ok(None) => {
                        self.update_note = format!("You have the latest version (v{}).", update::CURRENT);
                        self.update = UpdateState::Idle;
                    }
                    Err(e) => {
                        self.update_note = format!("Could not check for updates: {e}");
                        self.update = if manual { UpdateState::Failed(e) } else { UpdateState::Idle };
                    }
                },
                UpdateMsg::Progress(d, t) => {
                    if let UpdateState::Downloading { done, total, .. } = &mut self.update {
                        *done = d;
                        *total = t;
                    }
                }
                UpdateMsg::Installed(Ok(())) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                UpdateMsg::Installed(Err(e)) => {
                    self.update = if e == "cancelled" { UpdateState::Idle } else { UpdateState::Failed(e) };
                }
            }
        }
    }

    fn update_banner(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let t = tokens(ctx);
        let show = matches!(self.update, UpdateState::Available(_) | UpdateState::Downloading { .. } | UpdateState::Failed(_));
        if !show {
            return;
        }
        let fill = if matches!(self.update, UpdateState::Failed(_)) { mix(t.rail, t.warn, 0.16) } else { mix(t.rail, t.brand, 0.16) };
        let mut start: Option<Release> = None;
        let mut action: Option<&str> = None;
        egui::Panel::top("update").show_separator_line(false).frame(egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(S5 as i8, S2 as i8))).show(root, |ui| {
            ui.horizontal(|ui| match &self.update {
                UpdateState::Available(rel) => {
                    ui.label(RichText::new(format!("Version {} is available", rel.label())).font(FontId::new(T_BODY, semibold())).color(t.text));
                    ui.label(RichText::new(format!("You have v{}.", update::CURRENT)).size(T_SMALL).color(t.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::primary(ui, "Update and restart", true).on_hover_text("Downloads the new version, checks its checksum, then restarts the app").clicked() {
                            start = Some(rel.clone());
                        }
                        if kit::ghost(ui, "Later", true).clicked() {
                            action = Some("later");
                        }
                        if !rel.page.is_empty() && kit::ghost(ui, "What's new", true).clicked() {
                            let _ = open::that(&rel.page);
                        }
                    });
                }
                UpdateState::Downloading { rel, done, total } => {
                    ui.label(RichText::new(format!("Updating to version {}", rel.label())).font(FontId::new(T_BODY, semibold())).color(t.text));
                    let f = if *total > 0 { (*done as f32 / *total as f32).clamp(0.0, 1.0) } else { 0.0 };
                    ui.add(egui::ProgressBar::new(f).fill(t.action).desired_height(8.0).corner_radius(4).desired_width(260.0).animate(*total == 0));
                    ui.label(RichText::new(format!("{} of {}", model::format_size(Some(*done)), model::format_size(Some((*total).max(*done))))).size(T_SMALL).color(t.muted));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::ghost(ui, "Cancel", true).clicked() {
                            action = Some("cancel");
                        }
                    });
                }
                UpdateState::Failed(e) => {
                    ui.add(egui::Label::new(RichText::new(format!("The update did not install: {e}")).color(t.text)).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::ghost(ui, "Dismiss", true).clicked() {
                            action = Some("later");
                        }
                        if kit::secondary(ui, "Try again", true).clicked() {
                            action = Some("retry");
                        }
                    });
                }
                _ => {}
            });
        });
        if let Some(rel) = start {
            self.start_install(ctx, rel);
        }
        match action {
            Some("later") => self.update = UpdateState::Idle,
            Some("cancel") => self.update_cancel.store(true, Ordering::Relaxed),
            Some("retry") => self.start_check(ctx, true),
            _ => {}
        }
    }

    fn platform_filter(&self) -> Option<&str> {
        (self.settings.platform != "All").then_some(self.settings.platform.as_str())
    }

    fn refilter(&mut self) {
        self.view = model::search(&self.query, &self.category, self.platform_filter(), self.settings.sort, &self.catalog.apps);
        self.featured = if self.settings.sort == Sort::Home && self.query.trim().is_empty() && self.category == "All" && self.view.len() > 8 {
            shuffled(&self.view, daily_seed()).into_iter().take(4).collect()
        } else {
            Vec::new()
        };
        self.dirty = false;
    }

    fn categories(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        let plat = self.platform_filter();
        for a in self.catalog.apps.iter().filter(|a| plat.is_none_or(|p| a.platform == p)) {
            *counts.entry(a.category.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Marks a view switch so the content eases in.
    fn bump(&mut self, ctx: &egui::Context) {
        self.view_started = ctx.input(|i| i.time);
    }

    fn poll_catalog(&mut self) {
        let Ok(fresh) = self.rx.try_recv() else { return };
        self.loading = false;
        let offline = fresh.source.starts_with("Offline");
        if offline && !self.catalog.source.starts_with("Offline") && !self.catalog.apps.is_empty() {
            self.note = format!("Offline, showing the cached catalog ({})", self.catalog.source);
        } else {
            if !offline {
                net::save_cache(&fresh);
            }
            self.note.clear();
            self.catalog = fresh;
            self.suggestions = suggest(&self.catalog.apps);
            if let Some(d) = &self.detail {
                self.detail = self.catalog.apps.iter().find(|a| a.id == d.id).cloned();
            }
            self.dirty = true;
        }
    }

    fn open_detail(&mut self, ctx: &egui::Context, app: AppItem) {
        self.sel_stream = app.versions.iter().position(|v| v.stream == "Stable").unwrap_or(0);
        self.sel_variant = 0;
        self.detail = Some(app);
        self.bump(ctx);
    }

    fn start(&mut self, ctx: &egui::Context, url: &str, hint: &str) {
        let file = net::file_name_for(url, hint);
        if self.downloads.iter().any(|d| d.file == file && matches!(d.status(), DlStatus::Running)) {
            self.say("Already downloading that file");
            return;
        }
        let ctx = ctx.clone();
        let dl = net::start_download(url.into(), file, PathBuf::from(&self.settings.download_dir), move || ctx.request_repaint());
        self.downloads.push(dl);
        self.show_downloads = true;
    }

    fn tick_toast(&mut self, ctx: &egui::Context) {
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_millis(3600) {
                self.toast = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
    }

    /// Developer screenshot walk (ORION_SHOTS): each page is rendered to a PNG of this window only.
    fn drive_shots(&mut self, ctx: &egui::Context) {
        let Some(mut sh) = self.shots.take() else { return };
        let now = ctx.input(|i| i.time);
        if let Some(img) = sh.try_recv() {
            sh.save(img.width(), img.height(), img.as_raw());
        }
        if sh.finished() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if let Some((name, _)) = sh.step() {
            if sh.should_navigate(now) {
                match name {
                    "store" => {
                        self.set_section(ctx, Section::Store);
                        self.detail = None;
                    }
                    "store-detail" => {
                        if let Some(a) = self.view.first().cloned() {
                            self.open_detail(ctx, a);
                        }
                    }
                    other => {
                        self.set_section(ctx, Section::Optimaxer);
                        self.opti.show_page(ctx, other.trim_start_matches("optimaxer-"));
                    }
                }
            }
            if sh.should_capture(now) {
                let tx = sh.sender();
                let repaint = ctx.clone();
                ctx.request_screenshot(move |img| {
                    let _ = tx.send(img);
                    repaint.request_repaint();
                });
            }
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        self.shots = Some(sh);
    }

    fn set_section(&mut self, ctx: &egui::Context, s: Section) {
        if self.section != s {
            self.bump(ctx);
            if s == Section::Optimaxer {
                self.opti.start_transition(ctx);
            }
        }
        self.section = s;
        self.settings.section = if s == Section::Optimaxer { "Optimaxer" } else { "Store" }.into();
        self.settings.save();
    }

    fn handle_opti_events(&mut self, ctx: &egui::Context) {
        for ev in self.opti.take_events() {
            match ev {
                OptiEvent::Toast(m) => self.say(m),
                OptiEvent::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                OptiEvent::OpenStoreCategory(c) => {
                    self.set_section(ctx, Section::Store);
                    self.settings.platform = PC.into();
                    self.category = c;
                    self.query.clear();
                    self.detail = None;
                    self.dirty = true;
                    self.settings.save();
                }
                OptiEvent::OpenApp(id) => {
                    if let Some(a) = self.catalog.apps.iter().find(|a| a.id == id).cloned() {
                        self.set_section(ctx, Section::Store);
                        self.open_detail(ctx, a);
                    }
                }
            }
        }
    }
}

fn reveal(path: &std::path::Path) -> Result<(), std::io::Error> {
    std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn().map(|_| ())
}

fn icon(ui: &mut Ui, app: &AppItem, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_icon(ui, app, rect);
}

fn paint_icon(ui: &mut Ui, app: &AppItem, rect: egui::Rect) {
    let t = tokens(ui.ctx());
    let r = CornerRadius::from(rect.width() * 0.22);
    if app.icon.is_empty() {
        ui.painter().rect_filled(rect, r, mix(t.surface2, t.brand, 0.35));
        let letter = app.name.chars().next().unwrap_or('?').to_uppercase().to_string();
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, FontId::new(rect.width() * 0.45, semibold()), t.text);
    } else {
        egui::Image::new(app.icon.as_str()).fit_to_exact_size(rect.size()).corner_radius(r).paint_at(ui, rect);
    }
}

fn app_card(ui: &mut Ui, app: &AppItem, w: f32) -> bool {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let (rect, resp) = ui.allocate_exact_size(vec2(w, CARD_H), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &app.name));
    let h = ctx.animate_bool_with_time_and_easing(resp.id.with("h"), resp.hovered() || resp.has_focus(), m.fast(), ease_out);
    if ui.is_rect_visible(rect) {
        let r = CornerRadius::same(t.card_radius());
        let pressed = resp.is_pointer_button_down_on();
        let fill = if pressed { t.surface3 } else { mix(t.surface1, t.surface2, h) };
        ui.painter().rect_filled(rect, r, fill);
        let ring = if resp.has_focus() { 2.0 } else { 1.0 };
        let edge = if resp.has_focus() { t.ring() } else { mix(t.hairline, t.brand, h * 0.85) };
        ui.painter().rect_stroke(rect, r, Stroke::new(ring, edge), StrokeKind::Inside);
        paint_icon(ui, app, egui::Rect::from_min_size(rect.min + vec2(S4, S4 + 2.0), vec2(64.0, 64.0)));
        let text_rect = egui::Rect::from_min_max(rect.min + vec2(88.0, S3), rect.max - vec2(S3, S2));
        let p = ui.painter().with_clip_rect(text_rect);
        let at = |y: f32| text_rect.min + vec2(0.0, y);
        p.text(at(2.0), egui::Align2::LEFT_TOP, &app.name, FontId::new(T_HEADING - 1.0, semibold()), t.text);
        p.text(at(26.0), egui::Align2::LEFT_TOP, &app.author, FontId::proportional(T_SMALL), t.muted);
        let meta = format!("{}   {}", app.version, if app.size.is_empty() { "Varies" } else { &app.size });
        p.text(at(46.0), egui::Align2::LEFT_TOP, meta, FontId::proportional(T_CAPTION), t.subtle);
        p.text(at(68.0), egui::Align2::LEFT_TOP, &app.category, FontId::new(T_CAPTION, semibold()), t.brand_text);
    }
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn side_title(ui: &mut Ui, text: &str) {
    ui.add_space(S3);
    ui.label(RichText::new(text).font(FontId::new(T_SMALL, semibold())).color(tokens(ui.ctx()).subtle));
    ui.add_space(S1);
}

impl eframe::App for OrionApp {
    fn ui(&mut self, root: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        self.poll_catalog();
        self.poll_update(ctx);
        if self.shots.is_none() {
            self.follow_windows(ctx);
        }
        self.opti.tick(ctx, &mut self.settings);
        self.handle_opti_events(ctx);
        self.drive_shots(ctx);
        if self.dirty {
            self.refilter();
        }
        self.tick_toast(ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_palette {
                self.show_palette = false;
            } else if self.lightbox.is_some() {
                self.lightbox = None;
            } else if self.show_settings {
                self.show_settings = false;
            } else if self.detail.is_some() {
                self.detail = None;
                self.bump(ctx);
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            self.set_section(ctx, Section::Store);
            self.detail = None;
            ctx.memory_mut(|m| m.request_focus(self.search_id));
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::K)) {
            self.open_command_palette();
        }

        if self.shots.is_none() {
            resize_edges(ctx);
        }
        self.top_bar(root);
        self.update_banner(root);
        self.status_bar(root);
        if self.show_downloads {
            self.downloads_panel(root);
        }
        if self.section == Section::Optimaxer {
            self.opti.log_panel(root);
        }
        self.side_panel(root);
        let t = tokens(ctx);
        // Windows 11 layering: the content area is one step lighter than the rail around it
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas).inner_margin(Margin { left: S5 as i8, right: S5 as i8, top: S3 as i8, bottom: 0 })).show(root, |ui| {
            if self.section == Section::Optimaxer {
                self.opti.central(ui, ctx, &self.suggestions);
            } else {
                kit::enter(ui, self.view_started, |ui| {
                    if self.detail.is_some() {
                        self.detail_view(ctx, ui);
                    } else {
                        self.store_view(ctx, ui);
                    }
                });
            }
        });
        if self.show_settings {
            self.settings_modal(ctx);
        }
        if self.show_palette {
            self.command_palette(ctx);
        }
        self.lightbox_view(ctx);
        self.toast_view(ctx);
    }
}

impl OrionApp {
    fn open_command_palette(&mut self) {
        self.palette_query.clear();
        self.palette_selected = 0;
        self.show_palette = true;
    }

    fn command_palette(&mut self, ctx: &egui::Context) {
        let t = tokens(ctx);
        let matches = matching_palette_commands(&self.palette_query);
        if matches.is_empty() {
            self.palette_selected = 0;
        } else {
            self.palette_selected = self.palette_selected.min(matches.len() - 1);
        }
        let mut chosen = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::NULL.with("command-palette")).backdrop_color(Color32::from_black_alpha(if t.dark { 155 } else { 80 })).show(ctx, |ui| {
            ui.set_width(ui.available_width().clamp(240.0, 520.0));
            ui.heading("Command palette");
            ui.add_space(S2);
            let search_id = egui::Id::NULL.with("command-palette-search");
            let search = ui.add(egui::TextEdit::singleline(&mut self.palette_query).id(search_id).hint_text("Find a page or command…").desired_width(f32::INFINITY));
            if !search.has_focus() {
                ctx.memory_mut(|m| m.request_focus(search_id));
            }
            if search.changed() {
                self.palette_selected = 0;
            }
            ui.add_space(S2);
            egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                for (index, (name, hint, command)) in matches.iter().enumerate() {
                    let response = ui.selectable_label(index == self.palette_selected, format!("{name}    ·    {hint}"));
                    if response.clicked() {
                        chosen = Some(*command);
                    }
                }
            });
            ui.add_space(S2);
            ui.horizontal(|ui| {
                ui.label(RichText::new("↑ ↓ select   Enter open   Esc close").size(T_CAPTION).color(t.muted));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::ghost(ui, "Close", true).clicked() { close = true; }
                });
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) && !matches.is_empty() {
            self.palette_selected = (self.palette_selected + 1) % matches.len();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) && !matches.is_empty() {
            self.palette_selected = self.palette_selected.checked_sub(1).unwrap_or(matches.len() - 1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            chosen = matches.get(self.palette_selected).map(|(_, _, command)| *command);
        }
        if let Some(command) = chosen {
            self.run_palette_command(ctx, command);
        } else if close || modal.should_close() {
            self.show_palette = false;
        }
    }

    fn run_palette_command(&mut self, ctx: &egui::Context, command: PaletteCommand) {
        self.show_palette = false;
        match command {
            PaletteCommand::Page(name) => {
                self.set_section(ctx, Section::Optimaxer);
                self.opti.show_page(ctx, name);
            }
            PaletteCommand::Store => self.set_section(ctx, Section::Store),
            PaletteCommand::SearchStore => {
                self.set_section(ctx, Section::Store);
                self.detail = None;
                ctx.memory_mut(|m| m.request_focus(self.search_id));
            }
            PaletteCommand::Downloads => self.show_downloads = true,
            PaletteCommand::Settings => self.show_settings = true,
        }
    }

    fn toast_view(&self, ctx: &egui::Context) {
        let Some((msg, born)) = &self.toast else { return };
        let (t, m) = (tokens(ctx), motion(ctx));
        let age = born.elapsed().as_secs_f32();
        let fade_in = if m.reduce { 1.0 } else { ease_out(age / 0.18) };
        let fade_out = if m.reduce { 1.0 } else { ((3.6 - age) / 0.3).clamp(0.0, 1.0) };
        let a = fade_in.min(fade_out);
        egui::Area::new(egui::Id::unique("orion-toast")).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -48.0 + (1.0 - fade_in) * 10.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
            ui.set_opacity(a);
            egui::Frame::new().fill(t.surface3).stroke(Stroke::new(1.0, t.hairline)).corner_radius(R_MD + 2).inner_margin(Margin::symmetric(S4 as i8, S3 as i8)).shadow(ctx.global_style().visuals.popup_shadow).show(ui, |ui| {
                ui.set_max_width(520.0);
                ui.label(RichText::new(msg).color(t.text));
            });
        });
    }

    /// Full-window view of one screenshot: click outside or press Esc to close, arrow keys or the side buttons to step.
    fn lightbox_view(&mut self, ctx: &egui::Context) {
        let Some((urls, idx)) = self.lightbox.as_mut() else { return };
        let n = urls.len();
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            *idx = (*idx + 1) % n;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            *idx = (*idx + n - 1) % n;
        }
        let screen = ctx.content_rect();
        let mut close = false;
        let mut step: i32 = 0;
        egui::Area::new(egui::Id::NULL.with("lightbox")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (rect, backdrop) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(225));
            let img = ui.put(rect.shrink2(vec2(80.0, 56.0)), egui::Image::new(urls[*idx].as_str()).corner_radius(R_MD).sense(Sense::click()));
            if backdrop.clicked() && !img.clicked() {
                close = true;
            }
            if n > 1 {
                let side = |ui: &mut Ui, x: f32, label: &str| {
                    let r = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), vec2(44.0, 64.0));
                    ui.put(r, egui::Button::new(RichText::new(label).size(T_HEADING).color(Color32::WHITE)).fill(Color32::from_white_alpha(28)).corner_radius(R_MD)).clicked()
                };
                if side(ui, rect.left() + 40.0, "<") {
                    step = -1;
                }
                if side(ui, rect.right() - 40.0, ">") {
                    step = 1;
                }
            }
            let caption = format!("{} / {}", *idx + 1, n);
            ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - 24.0), egui::Align2::CENTER_CENTER, caption, FontId::proportional(T_SMALL), Color32::from_white_alpha(200));
            let x = egui::Rect::from_center_size(egui::pos2(rect.right() - 32.0, rect.top() + 28.0), vec2(40.0, 40.0));
            if ui.put(x, egui::Button::new(RichText::new("x").size(T_HEADING).color(Color32::WHITE)).fill(Color32::from_white_alpha(28)).corner_radius(R_MD)).on_hover_text("Close").clicked() {
                close = true;
            }
        });
        if step != 0 {
            *idx = (*idx as i32 + step).rem_euclid(n as i32) as usize;
        }
        if close {
            self.lightbox = None;
        }
    }

    fn top_bar(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let t = tokens(ctx);
        egui::Panel::top("top").show_separator_line(false).frame(egui::Frame::new().fill(t.rail).inner_margin(Margin::symmetric(S5 as i8, S3 as i8))).show(root, |ui| {
            let compact = ui.available_width() < 1000.0;
            ui.horizontal(|ui| {
                // the bar is the window's drag handle now that the system title bar is gone; widgets added after it sit on top
                let drag = ui.interact(ui.max_rect(), ui.scope_id().with("titlebar"), Sense::click_and_drag());
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
                ui.label(RichText::new("Orion").font(FontId::new(T_DISPLAY - 6.0, semibold())).color(t.brand_text));
                ui.add_space(S4);
                let mut idx = usize::from(self.section == Section::Optimaxer);
                if kit::segmented(ui, &mut idx, &["Store", "Optimaxer"]) {
                    self.set_section(ctx, if idx == 1 { Section::Optimaxer } else { Section::Store });
                }
                ui.add_space(S4);
                if self.section == Section::Store && !compact {
                    let w = (ui.available_width() - 330.0).clamp(180.0, 480.0);
                    let resp = ui.add_sized([w, 34.0], egui::TextEdit::singleline(&mut self.query).id(self.search_id).hint_text("Search apps, authors, tags  (Ctrl+F)").margin(Margin::symmetric(S3 as i8, 8)));
                    if resp.changed() {
                        self.dirty = true;
                        self.detail = None;
                        self.bump(ctx);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
                    if kit::caption_button(ui, '\u{E8BB}', "x", true, "Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    let (glyph, fallback, label) = if maximized { ('\u{E923}', "o", "Restore") } else { ('\u{E922}', "[]", "Maximize") };
                    if kit::caption_button(ui, glyph, fallback, false, label).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if kit::caption_button(ui, '\u{E921}', "-", false, "Minimize").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    ui.add_space(S3);
                    let command_label = if compact { "Commands" } else { "Commands  Ctrl+K" };
                    if kit::ghost(ui, command_label, true).clicked() {
                        self.open_command_palette();
                    }
                    if kit::ghost(ui, "Settings", true).clicked() {
                        self.show_settings = !self.show_settings;
                    }
                    let n = self.downloads.iter().filter(|d| matches!(d.status(), DlStatus::Running)).count();
                    let label = if n > 0 { format!("Downloads ({n})") } else { "Downloads".into() };
                    let dl = if self.show_downloads { kit::secondary(ui, &label, true) } else { kit::ghost(ui, &label, true) };
                    if dl.clicked() {
                        self.show_downloads = !self.show_downloads;
                    }
                    if self.section == Section::Store && !compact {
                        let label = if self.loading { "Refreshing" } else { "Refresh" };
                        if kit::ghost(ui, label, !self.loading).clicked() {
                            self.loading = true;
                            self.rx = spawn_refresh(ctx);
                        }
                    }
                });
            });
            if self.section == Section::Store && compact {
                ui.add_space(S2);
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 105.0).max(120.0);
                    let search = ui.add_sized([width, 34.0], egui::TextEdit::singleline(&mut self.query).id(self.search_id).hint_text("Search apps, authors, tags  (Ctrl+F)").margin(Margin::symmetric(S3 as i8, 8)));
                    if search.changed() {
                        self.dirty = true;
                        self.detail = None;
                        self.bump(ctx);
                    }
                    if kit::ghost(ui, if self.loading { "Refreshing" } else { "Refresh" }, !self.loading).clicked() {
                        self.loading = true;
                        self.rx = spawn_refresh(ctx);
                    }
                });
            }
        });
    }

    fn status_bar(&mut self, root: &mut Ui) {
        let t = tokens(root.ctx());
        egui::Panel::bottom("status").show_separator_line(false).frame(egui::Frame::new().fill(t.rail).inner_margin(Margin::symmetric(S5 as i8, S2 as i8))).show(root, |ui| {
            ui.horizontal(|ui| {
                if self.section == Section::Optimaxer {
                    ui.label(RichText::new(self.opti.status_text()).size(T_CAPTION).color(t.subtle));
                    return;
                }
                ui.label(RichText::new(format!("{} apps  ·  {} shown  ·  source: {}", self.catalog.apps.len(), self.view.len(), self.catalog.source)).size(T_CAPTION).color(t.subtle));
                if self.loading {
                    ui.add(egui::Spinner::new().size(12.0).color(t.brand_text));
                }
                if !self.note.is_empty() {
                    ui.label(RichText::new(&self.note).size(T_CAPTION).color(t.warn));
                }
            });
        });
    }

    fn side_panel(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let t = tokens(ctx);
        let compact = self.section == Section::Optimaxer && self.settings.sidebar_collapsed;
        egui::Panel::left("side").resizable(false).show_separator_line(false).exact_size(if compact { 64.0 } else { 212.0 }).frame(egui::Frame::new().fill(t.rail).inner_margin(Margin::symmetric(S3 as i8, S2 as i8))).show(root, |ui| {
            if self.section == Section::Optimaxer {
                egui::ScrollArea::vertical().id_salt("optimaxer-navigation").auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(S2);
                    let label = if self.settings.sidebar_collapsed { "Expand navigation" } else { "Collapse navigation" };
                    let glyph = if self.settings.sidebar_collapsed { "›" } else { "‹" };
                    let mut toggle_clicked = false;
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let toggle = ui.add_sized([30.0, 30.0], egui::Button::new(glyph).fill(t.surface2).stroke(egui::Stroke::new(1.0, t.hairline))).on_hover_text(label);
                            toggle.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
                            toggle_clicked = toggle.clicked();
                        });
                    });
                    if toggle_clicked {
                        self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed;
                        self.settings.save();
                    }
                    self.opti.side_panel(ui, ctx, self.settings.sidebar_collapsed);
                });
                return;
            }
            side_title(ui, "Platform");
            let mut changed = false;
            for p in PLATFORMS {
                let n = self.catalog.apps.iter().filter(|a| p == "All" || a.platform == p).count();
                let selected = self.settings.platform == p;
                if kit::nav_item(ui, selected, p, Some(&n.to_string())).clicked() && !selected {
                    self.settings.platform = p.into();
                    self.category = "All".into();
                    changed = true;
                }
            }
            side_title(ui, "Category");
            let cats = self.categories();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let all = cats.iter().map(|c| c.1).sum::<usize>();
                for (name, n) in std::iter::once(("All".to_string(), all)).chain(cats) {
                    let selected = self.category == name;
                    if kit::nav_item(ui, selected, &name, Some(&n.to_string())).clicked() && !selected {
                        self.category = name;
                        changed = true;
                    }
                }
            });
            if changed {
                self.dirty = true;
                self.detail = None;
                self.bump(ctx);
                self.settings.save();
            }
        });
    }

    fn store_view(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let t = tokens(ctx);
        ui.horizontal(|ui| {
            let title = if self.query.trim().is_empty() { format!("{} apps", self.settings.platform) } else { format!("Results for \"{}\"", self.query.trim()) };
            ui.label(RichText::new(title).font(FontId::new(theme::T_TITLE, semibold())).color(t.text));
            ui.label(RichText::new(format!("{}", self.view.len())).size(T_SMALL).color(t.subtle));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let before = self.settings.sort;
                egui::ComboBox::from_id_salt("sort").selected_text(self.settings.sort.label()).show_ui(ui, |ui| {
                    for s in Sort::ALL {
                        ui.selectable_value(&mut self.settings.sort, s, s.label());
                    }
                });
                if before != self.settings.sort {
                    self.dirty = true;
                    self.settings.save();
                }
                ui.label(RichText::new("Sort").size(T_SMALL).color(t.muted));
            });
        });
        ui.add_space(S2);
        if self.view.is_empty() {
            if self.loading && self.catalog.apps.is_empty() {
                kit::skeleton(ui, 6);
            } else {
                kit::empty_state(ui, "No apps match", "Try another platform or category, or clear the search.");
            }
            return;
        }
        let avail = ui.available_width();
        let cols = (((avail + GAP) / (300.0 + GAP)).floor() as usize).max(1);
        let w = (avail - GAP * (cols as f32 - 1.0) - 14.0) / cols as f32;
        let rows = self.view.len().div_ceil(cols);
        let mut picked: Option<AppItem> = None;
        if !self.featured.is_empty() {
            kit::group_title(ui, "Featured today");
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            ui.horizontal(|ui| {
                for app in self.featured.iter().take(cols) {
                    if app_card(ui, app, w) {
                        picked = Some(app.clone());
                    }
                }
            });
            ui.add_space(S1);
            ui.label(RichText::new("All apps").font(FontId::new(T_BODY, semibold())).color(t.muted));
            ui.add_space(S1);
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, CARD_H + GAP, rows, |ui, range| {
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            for r in range {
                ui.horizontal(|ui| {
                    for app in self.view.iter().skip(r * cols).take(cols) {
                        if app_card(ui, app, w) {
                            picked = Some(app.clone());
                        }
                    }
                });
            }
        });
        if let Some(a) = picked {
            self.open_detail(ctx, a);
        }
    }

    fn detail_view(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let Some(app) = self.detail.clone() else { return };
        let t = tokens(ctx);
        let mut download: Option<(String, String)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(980.0));
            if kit::ghost(ui, "Back to apps", true).clicked() {
                self.detail = None;
                self.bump(ctx);
            }
            ui.add_space(S3);
            ui.horizontal(|ui| {
                icon(ui, &app, 96.0);
                ui.add_space(S2);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&app.name).font(FontId::new(T_DISPLAY, semibold())).color(t.text));
                    ui.label(RichText::new(format!("by {}", app.author)).size(T_BODY).color(t.muted));
                    ui.add_space(S1);
                    ui.horizontal(|ui| {
                        kit::chip(ui, &app.platform, Tone::Brand);
                        kit::chip(ui, &app.category, Tone::Good);
                        let v = if app.version.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit()) { format!("v{}", app.version.trim_start_matches('v')) } else { app.version.clone() };
                        kit::chip(ui, &v, Tone::Neutral);
                        if !app.size.is_empty() {
                            kit::chip(ui, &app.size, Tone::Neutral);
                        }
                    });
                });
            });
            ui.add_space(S4);
            self.install_box(ui, &app, &mut download);
            if app.platform != PC {
                ui.add_space(S1);
                ui.add(egui::Label::new(RichText::new(format!("This is a {} package and will not install on Windows. Download it to transfer to a {} device.", app.platform, app.platform)).size(T_SMALL).color(t.muted)).wrap());
            }
            ui.add_space(S4);
            if !app.description.is_empty() {
                ui.add(egui::Label::new(RichText::new(&app.description).size(T_BODY + 1.0).color(t.text)).wrap());
                ui.add_space(S4);
            }
            if !app.patches.is_empty() {
                ui.label(RichText::new("Patches").font(FontId::new(T_BODY, semibold())).color(t.text));
                ui.horizontal_wrapped(|ui| {
                    for p in &app.patches {
                        kit::chip(ui, p, Tone::Brand);
                    }
                });
                ui.add_space(S3);
            }
            if !app.tags.is_empty() {
                ui.label(RichText::new("Tags").font(FontId::new(T_BODY, semibold())).color(t.text));
                ui.horizontal_wrapped(|ui| {
                    for tag in &app.tags {
                        kit::chip(ui, tag, Tone::Neutral);
                    }
                });
                ui.add_space(S3);
            }
            if !app.screenshots.is_empty() {
                ui.label(RichText::new("Screenshots").font(FontId::new(T_BODY, semibold())).color(t.text));
                ui.add_space(S1);
                egui::ScrollArea::horizontal().id_salt("shots").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, s) in app.screenshots.iter().enumerate() {
                            let resp = ui.add(egui::Image::new(s.as_str()).fit_to_exact_size(vec2(320.0, 190.0)).corner_radius(R_LG).sense(Sense::click()));
                            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Click to enlarge");
                            if resp.clicked() {
                                self.lightbox = Some((app.screenshots.clone(), i));
                            }
                        }
                    });
                });
                ui.add_space(S3);
            }
            ui.horizontal(|ui| {
                if let Some(u) = &app.repo_url {
                    ui.hyperlink_to("Source repository", u);
                }
                if let Some(u) = &app.official_site {
                    ui.hyperlink_to("Official site", u);
                }
                if let Some(p) = &app.package_name {
                    ui.label(RichText::new(p).monospace().size(T_CAPTION).color(t.subtle));
                }
            });
            ui.add_space(S5);
        });
        if let Some((url, hint)) = download {
            self.start(ctx, &url, &hint);
        }
    }

    fn install_box(&mut self, ui: &mut Ui, app: &AppItem, download: &mut Option<(String, String)>) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let variants = if app.versions.is_empty() { &app.variants } else { &app.versions[self.sel_stream.min(app.versions.len() - 1)].variants };
            if app.versions.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Release channel").color(t.muted));
                    let cur = &app.versions[self.sel_stream.min(app.versions.len() - 1)];
                    egui::ComboBox::from_id_salt("stream").selected_text(format!("{} {}", cur.stream, cur.version)).show_ui(ui, |ui| {
                        for (i, v) in app.versions.iter().enumerate() {
                            if ui.selectable_value(&mut self.sel_stream, i, format!("{}  {}  {}", v.stream, v.version, v.date)).clicked() {
                                self.sel_variant = 0;
                            }
                        }
                    });
                });
            }
            if variants.is_empty() {
                if app.download_url.is_empty() {
                    ui.label(RichText::new("No direct download is published yet.").color(t.text));
                    if let Some(r) = &app.repo_url {
                        ui.add_space(S1);
                        if kit::secondary(ui, "Open releases page", true).clicked() {
                            let _ = open::that(format!("{}/releases", r.trim_end_matches('/')));
                        }
                    }
                } else if kit::primary(ui, "Download", true).clicked() {
                    *download = Some((app.download_url.clone(), String::new()));
                }
                return;
            }
            let idx = self.sel_variant.min(variants.len() - 1);
            let label = |v: &model::AppVariant| format!("{}   {}   {}", v.arch, v.name, model::format_size(v.size));
            if variants.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Build").color(t.muted));
                    egui::ComboBox::from_id_salt("variant").width(420.0).selected_text(label(&variants[idx])).show_ui(ui, |ui| {
                        for (i, v) in variants.iter().enumerate() {
                            ui.selectable_value(&mut self.sel_variant, i, label(v));
                        }
                    });
                });
                ui.add_space(S1);
            }
            let v = &variants[idx];
            if kit::primary(ui, &format!("Download  ({})", model::format_size(v.size)), true).clicked() {
                *download = Some((v.url.clone(), v.name.clone()));
            }
        });
    }

    fn downloads_panel(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let t = tokens(ctx);
        let mut remove: Option<usize> = None;
        let mut note: Option<String> = None;
        let mut run_installer: Option<PathBuf> = None;
        egui::Panel::bottom("downloads").resizable(true).default_size(168.0).show_separator_line(false).frame(egui::Frame::new().fill(t.surface1).inner_margin(Margin::symmetric(S5 as i8, S3 as i8))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Downloads").font(FontId::new(T_HEADING, semibold())).color(t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::ghost(ui, "Clear finished", true).clicked() {
                        self.downloads.retain(|d| matches!(d.status(), DlStatus::Running));
                    }
                    if kit::ghost(ui, "Open folder", true).clicked()
                        && let Err(error) = std::fs::create_dir_all(&self.settings.download_dir).and_then(|_| open::that(&self.settings.download_dir)) {
                        note = Some(format!("Could not open the download folder: {error}"));
                    }
                });
            });
            if self.downloads.is_empty() {
                kit::empty_state(ui, "Nothing downloaded yet", "Pick an app and press Download. Files save to the folder in Settings.");
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (i, d) in self.downloads.iter().enumerate().rev() {
                    kit::row(ui, ("dl", i), false, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_sized([210.0, 20.0], egui::Label::new(RichText::new(&d.file).font(FontId::new(T_BODY, semibold())).color(t.text)).truncate());
                            match d.status() {
                                DlStatus::Running => {
                                    let done = d.done.load(std::sync::atomic::Ordering::Relaxed);
                                    let total = d.total.load(std::sync::atomic::Ordering::Relaxed);
                                    let (f, text) = match d.fraction() {
                                        Some(f) => (f, format!("{:.0}%   {} of {}", f * 100.0, model::format_size(Some(done)), model::format_size(Some(total)))),
                                        None => (0.0, model::format_size(Some(done))),
                                    };
                                    ui.add(egui::ProgressBar::new(f).fill(t.action).desired_height(8.0).corner_radius(4).animate(d.fraction().is_none()).desired_width(240.0));
                                    ui.label(RichText::new(text).size(T_SMALL).color(t.muted));
                                    if kit::ghost(ui, "Cancel", true).clicked() {
                                        d.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                    }
                                }
                                DlStatus::Done(path) => {
                                    kit::chip(ui, "Done", Tone::Good);
                                    if kit::secondary(ui, "Open", true).clicked() && open::that(&path).is_err() {
                                        note = Some("Could not open the file.".into());
                                    }
                                    if kit::ghost(ui, "Show in folder", true).clicked()
                                        && let Err(error) = reveal(&path) {
                                        note = Some(format!("Could not show the downloaded file: {error}"));
                                    }
                                    if is_installer(&path) && kit::ghost(ui, "Restore point, then open", true).on_hover_text("Optimaxer creates a System Restore point, then opens the installer").clicked() {
                                        run_installer = Some(path.clone());
                                    }
                                    if kit::ghost(ui, "Remove", true).clicked() {
                                        remove = Some(i);
                                    }
                                }
                                DlStatus::Failed(e) => {
                                    kit::chip(ui, "Failed", Tone::Bad);
                                    ui.label(RichText::new(e).size(T_SMALL).color(t.bad));
                                    if kit::ghost(ui, "Remove", true).clicked() {
                                        remove = Some(i);
                                    }
                                }
                                DlStatus::Cancelled => {
                                    kit::chip(ui, "Cancelled", Tone::Neutral);
                                    if kit::ghost(ui, "Remove", true).clicked() {
                                        remove = Some(i);
                                    }
                                }
                            }
                        });
                    });
                }
            });
        });
        if let Some(i) = remove {
            self.downloads.remove(i);
        }
        if let Some(p) = run_installer {
            self.opti.request_run_installer(p);
        }
        if let Some(n) = note {
            self.say(n);
        }
        if self.downloads.iter().any(|d| matches!(d.status(), DlStatus::Running)) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) {
        let t = tokens(ctx);
        let resp = egui::Modal::new(egui::Id::unique("settings-modal")).backdrop_color(Color32::from_black_alpha(if t.dark { 150 } else { 70 })).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Settings").font(FontId::new(theme::T_TITLE, semibold())).color(t.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::ghost(ui, "Close", true).clicked() {
                        self.show_settings = false;
                    }
                });
            });
            ui.add_space(S3);
            egui::ScrollArea::vertical()
                .id_salt("settings-content")
                .max_height((ctx.content_rect().height() - 110.0).max(200.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Appearance").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let mut ti = match self.settings.theme.as_str() {
                        "light" => 1,
                        "dark" => 2,
                        _ => 0,
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Theme").color(t.muted));
                        if let Some(dark) = self.settings.palette.forced_dark() {
                            ui.label(if dark { "Dark (set by palette)" } else { "Light (set by palette)" });
                        } else if kit::segmented(ui, &mut ti, &["System", "Light", "Dark"]) {
                            self.settings.theme = ["system", "light", "dark"][ti].into();
                            self.reapply_theme(ctx);
                            self.settings.save();
                        }
                    });
                    let mut si = theme::Style::ALL.iter().position(|x| *x == self.settings.style).unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Style").color(t.muted));
                        if kit::segmented(ui, &mut si, &["Windows", "Classic", "Material"]) {
                            self.settings.style = theme::Style::ALL[si];
                            self.reapply_theme(ctx);
                            self.settings.save();
                        }
                    });
                    let mut palette = self.settings.palette;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Palette").color(t.muted));
                        egui::ComboBox::from_id_salt("appearance-palette").selected_text(palette.label()).show_ui(ui, |ui| {
                            for choice in theme::Palette::ALL {
                                ui.selectable_value(&mut palette, choice, choice.label());
                            }
                        });
                    });
                    if palette != self.settings.palette {
                        self.settings.palette = palette;
                        self.reapply_theme(ctx);
                        self.settings.save();
                    }
                    if self.settings.palette == theme::Palette::Custom {
                        let mut accent = theme::parse_hex_color(&self.settings.custom_accent).unwrap_or(theme::BRAND);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Custom accent").color(t.muted));
                            if ui.color_edit_button_srgba(&mut accent).changed() {
                                self.settings.custom_accent = theme::hex_color(accent);
                                self.reapply_theme(ctx);
                                self.settings.save();
                            }
                            if kit::ghost(ui, "Reset", true).clicked() {
                                self.settings.custom_accent = theme::hex_color(theme::BRAND);
                                self.reapply_theme(ctx);
                                self.settings.save();
                            }
                            ui.label(RichText::new(&self.settings.custom_accent).size(T_CAPTION).color(t.subtle));
                        });
                    }
                    ui.add(egui::Label::new(RichText::new("Color palettes are separate from control shape. System follows Windows; Custom saves an accessible accent and keeps contrast-adjusted text.").size(T_CAPTION).color(t.subtle)).wrap());
                    let mut reduce = self.settings.reduce_motion;
                    if ui.checkbox(&mut reduce, "Reduce motion").on_hover_text("Turns off transitions and the loading pulse. Follows the Windows setting by default.").changed() {
                        self.settings.reduce_motion = reduce;
                        self.reapply_theme(ctx);
                        self.settings.save();
                    }
                });
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Updates").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let mut auto = self.settings.auto_update;
                    if ui.checkbox(&mut auto, "Check for updates when the app starts").on_hover_text("Only checks. Nothing is downloaded until you press Update and restart.").changed() {
                        self.settings.auto_update = auto;
                        self.settings.save();
                    }
                    ui.horizontal(|ui| {
                        let busy = matches!(self.update, UpdateState::Checking | UpdateState::Downloading { .. });
                        if kit::secondary(ui, if matches!(self.update, UpdateState::Checking) { "Checking" } else { "Check now" }, !busy).clicked() {
                            self.start_check(ctx, true);
                        }
                        let text = if self.update_note.is_empty() { format!("Version v{}", update::CURRENT) } else { self.update_note.clone() };
                        ui.add(egui::Label::new(RichText::new(text).size(T_SMALL).color(t.muted)).wrap());
                    });
                });
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Downloads").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let resp = ui.add(egui::TextEdit::singleline(&mut self.settings.download_dir).desired_width(f32::INFINITY).margin(Margin::symmetric(S3 as i8, 8)));
                    if resp.lost_focus() {
                        self.settings.save();
                    }
                    ui.horizontal(|ui| {
                        if kit::secondary(ui, "Open folder", true).clicked() {
                            let _ = std::fs::create_dir_all(&self.settings.download_dir);
                            let _ = open::that(&self.settings.download_dir);
                        }
                        if kit::ghost(ui, "Reset", true).clicked() {
                            self.settings.download_dir = Settings::default().download_dir;
                            self.settings.save();
                        }
                    });
                });
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Optimaxer").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let mut rp = self.settings.opti_restore_point;
                    if ui.checkbox(&mut rp, "Offer a System Restore point before applying tweaks").changed() {
                        self.settings.opti_restore_point = rp;
                        self.settings.save();
                    }
                });
                ui.add_space(S3);
                ui.label(RichText::new(format!("Data folder: {}", net::data_dir().display())).size(T_CAPTION).color(t.subtle));
                });
        });
        if resp.should_close() {
            self.show_settings = false;
        }
    }
}

/// The window has no system frame, so its edges are resize handles: a 6 px band that shows the resize cursor and
/// starts a native resize when pressed. Nothing happens while maximized.
fn resize_edges(ctx: &egui::Context) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if ctx.input(|i| i.viewport().maximized).unwrap_or(false) {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.content_rect();
    let b = 6.0;
    let (l, rt, tp, bt) = (p.x - r.left() < b, r.right() - p.x < b, p.y - r.top() < b, r.bottom() - p.y < b);
    let hit = match (l, rt, tp, bt) {
        (true, _, true, _) => Some((D::NorthWest, C::ResizeNwSe)),
        (_, true, true, _) => Some((D::NorthEast, C::ResizeNeSw)),
        (true, _, _, true) => Some((D::SouthWest, C::ResizeNeSw)),
        (_, true, _, true) => Some((D::SouthEast, C::ResizeNwSe)),
        (true, ..) => Some((D::West, C::ResizeHorizontal)),
        (_, true, ..) => Some((D::East, C::ResizeHorizontal)),
        (_, _, true, _) => Some((D::North, C::ResizeVertical)),
        (_, _, _, true) => Some((D::South, C::ResizeVertical)),
        _ => None,
    };
    if let Some((dir, cursor)) = hit {
        ctx.set_cursor_icon(cursor);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }
}

#[cfg(test)]
mod palette_tests {
    use super::matching_palette_commands;

    #[test]
    fn palette_lists_all_commands_for_blank_query() {
        assert_eq!(matching_palette_commands("  ").len(), 13);
    }

    #[test]
    fn palette_search_is_case_insensitive_and_matches_hints() {
        let page = matching_palette_commands("  tWeAkS  ");
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].0, "Tweaks");
        let hint = matching_palette_commands("focus store search");
        assert_eq!(hint.len(), 1);
        assert_eq!(hint[0].0, "Search apps");
    }

    #[test]
    fn palette_returns_no_commands_for_unknown_query() {
        assert!(matching_palette_commands("not a command").is_empty());
    }
}
