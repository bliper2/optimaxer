use crate::kit;
use crate::model::{self, ANDROID, AppItem, Catalog, PC, Sort, TV};
use crate::net::{self, DlStatus, Download, Settings};
use crate::notifications::{
    self, Action as NotificationAction, Item as Notification, Kind as NotificationKind,
    MAX_SAVED_DISMISSALS,
};
use crate::opti_ui::{OptiEvent, OptiUi, RecentAction};
use crate::software_ui::SoftwareUi;
use crate::theme::{
    self, S1, S2, S3, S4, S5, T_BODY, T_CAPTION, T_DISPLAY, T_HEADING, T_SMALL, Tone, ease_out,
    mix, motion, semibold, tokens,
};
use crate::update::{self, Release};
use egui::{
    Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Role, Sense, Stroke,
    StrokeKind, Ui, WidgetInfo, vec2,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

mod notifications_ui;
mod palette;
mod settings;
mod shell;
mod store_ui;

#[cfg(test)]
use palette::*;

const CARD_H: f32 = 108.0;
const GAP: f32 = 12.0;
// Below this width, move Store search to its own row and replace text actions with icons.
const TOP_BAR_COMPACT_WIDTH: f32 = 1280.0;
// The full text actions and native window controls share the row with Store search.
const TOP_BAR_ACTIONS_WIDTH: f32 = 780.0;
const PLATFORMS: [&str; 4] = [PC, ANDROID, TV, "All"];

enum UpdateState {
    Idle,
    Checking,
    Available(Release),
    Downloading { rel: Release, done: u64, total: u64 },
    Failed(String),
}

impl UpdateState {
    fn is_busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. })
    }
}

enum UpdateMsg {
    Checked {
        manual: bool,
        result: Result<Option<Release>, String>,
    },
    Progress(u64, u64),
    Installed(Result<(), String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Store,
    Optimaxer,
}

/// PC tools from the catalog that Optimaxer offers on its Tools page.
fn suggest(apps: &[AppItem]) -> Vec<AppItem> {
    let mut v: Vec<AppItem> = apps
        .iter()
        .filter(|a| a.platform == PC && matches!(a.category.as_str(), "Utility" | "Customization"))
        .cloned()
        .collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v.truncate(12);
    v
}

fn is_installer(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "exe" | "msi" | "msix" | "appx" | "msixbundle" | "appxbundle"
        )
    })
}
fn update_notification(state: &UpdateState) -> Option<Notification> {
    match state {
        UpdateState::Idle => None,
        UpdateState::Checking => Some(Notification {
            id: "update-check-progress".into(),
            kind: NotificationKind::Progress,
            title: "Checking for updates".into(),
            detail: "Contacting GitHub.".into(),
            lines: Vec::new(),
            progress: None,
            action: None,
            dismissible: false,
            persistent: false,
        }),
        UpdateState::Available(release) => Some(Notification {
            id: format!("update-available:{}", release.tag),
            kind: NotificationKind::Information,
            title: format!("Optimaxer {} is available", release.label()),
            detail: format!("You have v{}.", update::CURRENT),
            lines: Vec::new(),
            progress: None,
            action: Some(NotificationAction::StartUpdate(release.clone())),
            dismissible: true,
            persistent: true,
        }),
        UpdateState::Downloading { rel, done, total } => Some(Notification {
            id: format!("update-progress:{}", rel.tag),
            kind: NotificationKind::Progress,
            title: format!("Updating to {}", rel.label()),
            detail: notifications::byte_progress_detail(*done, *total),
            lines: Vec::new(),
            progress: notifications::byte_progress(*done, *total),
            action: None,
            dismissible: false,
            persistent: false,
        }),
        UpdateState::Failed(error) => Some(Notification {
            id: notifications::stable_id("update-failed", error),
            kind: NotificationKind::Error,
            title: "Update failed".into(),
            detail: "The current app is still installed.".into(),
            lines: vec![error.clone()],
            progress: None,
            action: Some(NotificationAction::CheckUpdates),
            dismissible: true,
            persistent: true,
        }),
    }
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
    show_winget: bool,
    software: SoftwareUi,
    category: String,
    favorites_only: bool,
    view: Vec<AppItem>,
    featured: Vec<AppItem>,
    dirty: bool,
    detail: Option<AppItem>,
    sel_stream: usize,
    sel_variant: usize,
    downloads: Vec<Arc<Download>>,
    recent_downloads: Vec<net::RecentDownload>,
    show_downloads: bool,
    show_notifications: bool,
    dismissed_session_notifications: Vec<String>,
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
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
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

fn retain_store_favorites(apps: &mut Vec<AppItem>, favorite_ids: &[String]) {
    apps.retain(|app| favorite_ids.contains(&app.id));
}

impl OrionApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::install_fonts(&cc.egui_ctx);
        let mut settings = Settings::load();
        let mut startup_toast = None;
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
            match std::env::var("ORION_SHOTS_DENSITY").as_deref() {
                Ok("compact") => settings.density = theme::Density::Compact,
                Ok("comfortable") => settings.density = theme::Density::Comfortable,
                _ => {}
            }
            match std::env::var("ORION_SHOTS_SCALE").as_deref() {
                Ok("90") => settings.ui_scale = theme::UiScale::Small,
                Ok("100") => settings.ui_scale = theme::UiScale::Default,
                Ok("110") => settings.ui_scale = theme::UiScale::Large,
                Ok("125") => settings.ui_scale = theme::UiScale::Larger,
                _ => {}
            }
            if let Some(palette) = std::env::var("ORION_SHOTS_PALETTE")
                .ok()
                .and_then(|key| theme::Palette::from_key(&key))
            {
                settings.palette = palette;
            }
            match std::env::var("ORION_SHOTS_STYLE").as_deref() {
                Ok("classic") => settings.style = theme::Style::Classic,
                Ok("material") => settings.style = theme::Style::Material,
                Ok("glass") => settings.style = theme::Style::Glass,
                Ok("windows") => settings.style = theme::Style::Windows,
                _ => {}
            }
        }
        if settings.clear_restart_after_reboot(crate::opti::sysmon::uptime_secs())
            && let Err(error) = settings.save_checked()
        {
            startup_toast = Some((
                format!("Restart reminder could not be cleared on disk: {error}"),
                Instant::now(),
            ));
        }
        theme::apply(&cc.egui_ctx, settings.appearance());
        let catalog = net::load_cache().unwrap_or_else(net::embedded);
        let rx = spawn_refresh(&cc.egui_ctx);
        let (update_tx, update_rx) = channel();
        if std::env::var_os("ORION_SHOTS").is_none() {
            update::cleanup_old();
        }
        let auto_check = settings.auto_update && std::env::var_os("ORION_SHOTS").is_none();
        let mut app = Self {
            shots: crate::shots::Shots::from_env(),
            section: if settings.section == "Optimaxer" {
                Section::Optimaxer
            } else {
                Section::Store
            },
            opti: OptiUi::new(),
            suggestions: suggest(&catalog.apps),
            settings,
            catalog,
            rx,
            loading: true,
            note: String::new(),
            query: if std::env::var_os("ORION_SHOTS").is_some() {
                std::env::var("ORION_SHOTS_QUERY").unwrap_or_default()
            } else {
                String::new()
            },
            show_winget: false,
            software: SoftwareUi::new(),
            category: "All".into(),
            favorites_only: false,
            view: Vec::new(),
            featured: Vec::new(),
            dirty: true,
            detail: None,
            sel_stream: 0,
            sel_variant: 0,
            downloads: Vec::new(),
            recent_downloads: if std::env::var_os("ORION_SHOTS").is_none() {
                match net::load_recent_downloads() {
                    Ok(records) => records,
                    Err(error) => {
                        startup_toast = Some((
                            format!("Recent downloads could not be loaded: {error}"),
                            Instant::now(),
                        ));
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            },
            show_downloads: false,
            show_notifications: false,
            dismissed_session_notifications: Vec::new(),
            show_settings: false,
            show_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            lightbox: None,
            toast: startup_toast,
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
            app.update = UpdateState::Available(Release {
                version: (9, 9, 9),
                tag: "v9.9.9".into(),
                notes: String::new(),
                page: String::new(),
                url: String::new(),
                digest: String::new(),
                size: 0,
            });
        }
        app
    }

    fn reapply_theme(&mut self, ctx: &egui::Context) {
        theme::apply(ctx, self.settings.appearance());
        self.sys_look = (theme::system_dark(), theme::system_accent());
    }

    /// Follows Windows' dark mode and accent colour while the app runs (checked every couple of seconds).
    fn follow_windows(&mut self, ctx: &egui::Context) {
        if self.sys_checked.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.sys_checked = Instant::now();
        let now = (theme::system_dark(), theme::system_accent());
        if now != self.sys_look
            && (self.settings.theme == "system" || self.settings.style == theme::Style::Windows)
        {
            self.reapply_theme(ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(2));
    }

    fn start_check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.update.is_busy() {
            return;
        }
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
        self.update = UpdateState::Downloading {
            rel: rel.clone(),
            done: 0,
            total: rel.size,
        };
        let (tx, ctx, cancel) = (
            self.update_tx.clone(),
            ctx.clone(),
            self.update_cancel.clone(),
        );
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
                        self.update_note =
                            format!("You have the latest version (v{}).", update::CURRENT);
                        self.update = UpdateState::Idle;
                    }
                    Err(e) => {
                        self.update_note = format!("Could not check for updates: {e}");
                        self.update = if manual {
                            UpdateState::Failed(e)
                        } else {
                            UpdateState::Idle
                        };
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
                    self.update = if e == "cancelled" {
                        UpdateState::Idle
                    } else {
                        UpdateState::Failed(e)
                    };
                }
            }
        }
    }

    fn update_banner(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let t = tokens(ctx);
        let show = matches!(
            self.update,
            UpdateState::Available(_) | UpdateState::Downloading { .. } | UpdateState::Failed(_)
        );
        if !show {
            return;
        }
        let fill = if matches!(self.update, UpdateState::Failed(_)) {
            mix(t.rail, t.warn, 0.16)
        } else {
            mix(t.rail, t.brand, 0.16)
        };
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
        self.view = model::search(
            &self.query,
            &self.category,
            self.platform_filter(),
            self.settings.sort,
            &self.catalog.apps,
        );
        if self.favorites_only {
            retain_store_favorites(&mut self.view, &self.settings.store_favorites);
        }
        self.featured = if !self.favorites_only
            && self.settings.sort == Sort::Home
            && self.query.trim().is_empty()
            && self.category == "All"
            && self.view.len() > 8
        {
            shuffled(&self.view, daily_seed())
                .into_iter()
                .take(4)
                .collect()
        } else {
            Vec::new()
        };
        self.dirty = false;
    }

    fn categories(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        let plat = self.platform_filter();
        for a in self
            .catalog
            .apps
            .iter()
            .filter(|a| plat.is_none_or(|p| a.platform == p))
        {
            *counts.entry(a.category.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }

    fn toggle_store_favorite(&mut self, id: &str) {
        let previous = self.settings.store_favorites.clone();
        let saved = self.settings.toggle_store_favorite(id);
        if let Err(error) = self.settings.save_checked() {
            self.settings.store_favorites = previous;
            self.say(format!("Favorite was not saved: {error}"));
            return;
        }
        self.dirty = true;
        self.say(if saved {
            "Added to Favorites"
        } else {
            "Removed from Favorites"
        });
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Marks a view switch so the content eases in.
    fn bump(&mut self, ctx: &egui::Context) {
        self.view_started = ctx.input(|i| i.time);
    }

    fn poll_catalog(&mut self) {
        let Ok(fresh) = self.rx.try_recv() else {
            return;
        };
        self.loading = false;
        let offline = fresh.source.starts_with("Offline");
        if offline && !self.catalog.source.starts_with("Offline") && !self.catalog.apps.is_empty() {
            self.note = format!(
                "Offline, showing the cached catalog ({})",
                self.catalog.source
            );
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
        self.sel_stream = app
            .versions
            .iter()
            .position(|v| v.stream == "Stable")
            .unwrap_or(0);
        self.sel_variant = 0;
        self.detail = Some(app);
        self.bump(ctx);
    }

    fn start(&mut self, ctx: &egui::Context, url: &str, hint: &str) {
        let file = net::file_name_for(url, hint);
        if self
            .downloads
            .iter()
            .any(|d| d.file == file && matches!(d.status(), DlStatus::Running))
        {
            self.say("Already downloading that file");
            return;
        }
        let ctx = ctx.clone();
        let dl = net::start_download(
            url.into(),
            file,
            PathBuf::from(&self.settings.download_dir),
            move || ctx.request_repaint(),
        );
        self.downloads.push(dl);
        self.show_downloads = true;
    }

    fn record_completed_downloads(&mut self) {
        let mut history_update = None;
        let mut history_error = None;
        for download in &self.downloads {
            if download.history_recorded.load(Ordering::Relaxed) {
                continue;
            }
            let DlStatus::Done(path) = download.status() else {
                continue;
            };
            download.history_recorded.store(true, Ordering::Relaxed);
            let completed_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            match net::record_recent_download(net::RecentDownload {
                file: download.file.clone(),
                path,
                completed_at,
            }) {
                Ok(records) if !records.is_empty() => history_update = Some(records),
                Ok(_) => {}
                Err(error) => history_error = Some(error),
            }
        }
        if let Some(records) = history_update {
            self.recent_downloads = records;
        }
        if let Some(error) = history_error {
            self.say(format!(
                "Download finished, but history could not be saved: {error}"
            ));
        }
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
        let Some(mut sh) = self.shots.take() else {
            return;
        };
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
                    "store-favorites" => {
                        self.set_section(ctx, Section::Store);
                        self.detail = None;
                        self.category = "All".into();
                        self.favorites_only = true;
                        self.dirty = true;
                    }
                    "winget" => {
                        self.set_section(ctx, Section::Store);
                        self.show_winget = true;
                        self.detail = None;
                    }
                    "settings" => {
                        self.set_section(ctx, Section::Optimaxer);
                        self.show_settings = true;
                    }
                    other => {
                        self.set_section(ctx, Section::Optimaxer);
                        self.opti.show_page(ctx, &crate::shots::page_name(other));
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
            } else {
                self.opti.suspend_telemetry();
            }
        }
        self.section = s;
        if s == Section::Store {
            self.show_winget = false;
        }
        self.settings.section = if s == Section::Optimaxer {
            "Optimaxer"
        } else {
            "Store"
        }
        .into();
        self.save_settings("Window section");
    }

    fn handle_opti_events(&mut self, ctx: &egui::Context) {
        for ev in self.opti.take_events() {
            match ev {
                OptiEvent::Toast(m) => self.say(m),
                OptiEvent::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                OptiEvent::CheckForUpdates => {
                    if self.update.is_busy() {
                        self.say("An update check or download is already running.");
                    } else {
                        self.start_check(ctx, true);
                    }
                }
                OptiEvent::OpenStoreCategory(c) => {
                    self.set_section(ctx, Section::Store);
                    self.settings.platform = PC.into();
                    self.category = c;
                    self.query.clear();
                    self.detail = None;
                    self.dirty = true;
                    self.save_settings("Store platform");
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
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(|_| ())
}

fn icon(ui: &mut Ui, app: &AppItem, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_icon(ui, app, rect);
}

fn paint_icon(ui: &mut Ui, app: &AppItem, rect: egui::Rect) {
    let t = tokens(ui.ctx());
    let r = CornerRadius::from(rect.width() * 0.22);
    if app.icon.is_empty() {
        ui.painter()
            .rect_filled(rect, r, mix(t.surface2, t.brand, 0.35));
        let letter = app
            .name
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string();
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            FontId::new(rect.width() * 0.45, semibold()),
            t.text,
        );
    } else {
        egui::Image::new(app.icon.as_str())
            .fit_to_exact_size(rect.size())
            .corner_radius(r)
            .paint_at(ui, rect);
    }
}

fn app_card(ui: &mut Ui, app: &AppItem, w: f32) -> bool {
    let ctx = ui.ctx().clone();
    let (t, m) = (tokens(&ctx), motion(&ctx));
    let (rect, resp) = ui.allocate_exact_size(vec2(w, CARD_H), Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &app.name));
    let h = ctx.animate_bool_with_time_and_easing(
        resp.id.with("h"),
        resp.hovered() || resp.has_focus(),
        m.fast(),
        ease_out,
    );
    if ui.is_rect_visible(rect) {
        let r = CornerRadius::same(t.card_radius());
        let pressed = resp.is_pointer_button_down_on();
        let fill = if pressed {
            t.surface3
        } else {
            mix(t.surface1, t.surface2, h)
        };
        ui.painter().rect_filled(rect, r, fill);
        let ring = if resp.has_focus() { 2.0 } else { 1.0 };
        let edge = if resp.has_focus() {
            t.ring()
        } else {
            mix(t.hairline, t.brand, h * 0.85)
        };
        ui.painter()
            .rect_stroke(rect, r, Stroke::new(ring, edge), StrokeKind::Inside);
        paint_icon(
            ui,
            app,
            egui::Rect::from_min_size(rect.min + vec2(S4, S4 + 2.0), vec2(64.0, 64.0)),
        );
        let text_rect =
            egui::Rect::from_min_max(rect.min + vec2(88.0, S3), rect.max - vec2(S3, S2));
        let p = ui.painter().with_clip_rect(text_rect);
        let at = |y: f32| text_rect.min + vec2(0.0, y);
        p.text(
            at(2.0),
            egui::Align2::LEFT_TOP,
            &app.name,
            FontId::new(T_HEADING - 1.0, semibold()),
            t.text,
        );
        p.text(
            at(26.0),
            egui::Align2::LEFT_TOP,
            &app.author,
            FontId::proportional(T_SMALL),
            t.muted,
        );
        let meta = format!(
            "{}   {}",
            app.version,
            if app.size.is_empty() {
                "Varies"
            } else {
                &app.size
            }
        );
        p.text(
            at(46.0),
            egui::Align2::LEFT_TOP,
            meta,
            FontId::proportional(T_CAPTION),
            t.subtle,
        );
        p.text(
            at(68.0),
            egui::Align2::LEFT_TOP,
            &app.category,
            FontId::new(T_CAPTION, semibold()),
            t.brand_text,
        );
    }
    if resp.hovered() {
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

fn side_title(ui: &mut Ui, text: &str) {
    ui.add_space(S3);
    ui.label(
        RichText::new(text)
            .font(FontId::new(T_SMALL, semibold()))
            .color(tokens(ui.ctx()).subtle),
    );
    ui.add_space(S1);
}

impl eframe::App for OrionApp {
    fn ui(&mut self, root: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        self.poll_catalog();
        self.poll_update(ctx);
        self.software.tick(ctx);
        self.record_completed_downloads();
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
            } else if self.show_notifications {
                self.show_notifications = false;
            } else if self.detail.is_some() {
                self.detail = None;
                self.bump(ctx);
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            if self.section != Section::Store {
                self.set_section(ctx, Section::Store);
            }
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
        if self.show_notifications {
            self.notification_center(ctx);
        }
        if self.section == Section::Optimaxer {
            self.opti.log_panel(root);
        }
        self.side_panel(root);
        let t = tokens(ctx);
        // Windows 11 layering: the content area is one step lighter than the rail around it
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.canvas).inner_margin(Margin {
                left: S5 as i8,
                right: S5 as i8,
                top: S3 as i8,
                bottom: 0,
            }))
            .show(root, |ui| {
                if self.section == Section::Optimaxer {
                    self.opti
                        .central(ui, ctx, &self.suggestions, &mut self.settings);
                } else if self.show_winget {
                    kit::enter(ui, self.view_started, |ui| self.software.ui(ui, ctx));
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

/// The window has no system frame, so its edges are resize handles: a 6 px band that shows the resize cursor and
/// starts a native resize when pressed. Nothing happens while maximized.
fn resize_edges(ctx: &egui::Context) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    if ctx.input(|i| i.viewport().maximized).unwrap_or(false) {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let r = ctx.content_rect();
    let b = 6.0;
    let (l, rt, tp, bt) = (
        p.x - r.left() < b,
        r.right() - p.x < b,
        p.y - r.top() < b,
        r.bottom() - p.y < b,
    );
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
mod app_tests {
    use super::{
        PaletteCommand, UpdateState, matching_palette_commands,
        matching_palette_commands_with_recent, palette_tweak_query, retain_store_favorites,
    };
    use crate::model::AppItem;
    use crate::opti_ui::RecentAction;
    use crate::update::Release;

    #[test]
    fn update_busy_state_only_matches_running_operations() {
        let release = || Release {
            version: (2, 15, 39),
            tag: "v2.15.39".into(),
            notes: String::new(),
            page: String::new(),
            url: String::new(),
            digest: String::new(),
            size: 0,
        };

        assert!(!UpdateState::Idle.is_busy());
        assert!(UpdateState::Checking.is_busy());
        assert!(
            UpdateState::Downloading {
                rel: release(),
                done: 0,
                total: 0,
            }
            .is_busy()
        );
        assert!(!UpdateState::Available(release()).is_busy());
        assert!(!UpdateState::Failed("offline".into()).is_busy());
    }

    #[test]
    fn palette_lists_all_commands_for_blank_query() {
        assert_eq!(matching_palette_commands("  ").len(), 51);
    }

    #[test]
    fn palette_shows_recent_actions_before_static_commands() {
        let recent = [RecentAction {
            title: "Reset network stack".into(),
            hint: "Finished · 2026-10-30T12:00:00Z".into(),
            search: "2026-10-30T12:00:00Z".into(),
        }];

        let results = matching_palette_commands_with_recent("", &recent);

        assert_eq!(results[0].name, "Reset network stack");
        assert_eq!(results[0].category, "Recent");
        assert_eq!(
            results[0].activity_search.as_deref(),
            Some("2026-10-30T12:00:00Z")
        );
        assert_eq!(results[1].name, "Overview");
    }

    #[test]
    fn palette_fuzzy_search_finds_recent_actions_and_timestamps() {
        let recent = [RecentAction {
            title: "Reset network stack".into(),
            hint: "Finished · 2026-10-30T12:00:00Z".into(),
            search: "2026-10-30T12:00:00Z".into(),
        }];

        let title_match = matching_palette_commands_with_recent("reset net", &recent);
        let title_action = title_match
            .iter()
            .find(|entry| entry.name == "Reset network stack")
            .expect("recent title should match");
        assert_eq!(
            title_action.activity_search.as_deref(),
            Some("2026-10-30T12:00:00Z")
        );

        let date_match = matching_palette_commands_with_recent("2026-10-30", &recent);
        let date_action = date_match
            .iter()
            .find(|entry| entry.name == "Reset network stack")
            .expect("recent timestamp should match");
        assert_eq!(
            date_action.activity_search.as_deref(),
            Some("2026-10-30T12:00:00Z")
        );
    }

    #[test]
    fn performance_command_navigates_to_its_page() {
        let results = matching_palette_commands("performance");
        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0].command,
            PaletteCommand::Page("Performance")
        ));
    }

    #[test]
    fn gaming_command_navigates_to_its_page() {
        let results = matching_palette_commands("gaming");
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].command, PaletteCommand::Page("Gaming")));
    }

    #[test]
    fn diagnostics_command_navigates_to_its_page() {
        let results = matching_palette_commands("diagnostics");
        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0].command,
            PaletteCommand::Page("Diagnostics")
        ));
    }

    #[test]
    fn system_snapshot_command_navigates_to_its_page() {
        let results = matching_palette_commands("system snapshot");
        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0].command,
            PaletteCommand::Page("System Snapshot")
        ));
    }

    #[test]
    fn palette_search_is_case_insensitive_and_matches_hints() {
        let page = matching_palette_commands("  tWeAkS  ");
        assert_eq!(page[0].name, "Tweaks");
        let hint = matching_palette_commands("focus store search");
        assert_eq!(hint.len(), 1);
        assert_eq!(hint[0].name, "Search apps");
    }

    #[test]
    fn palette_search_matches_fuzzy_subsequences_and_word_boundaries() {
        let typo = matching_palette_commands("prfmnce");
        assert_eq!(typo.len(), 1);
        assert_eq!(typo[0].name, "Performance");

        let words = matching_palette_commands("sys snap");
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].name, "System Snapshot");
    }

    #[test]
    fn palette_ranks_name_matches_above_hint_matches() {
        let results = matching_palette_commands("store");
        assert_eq!(results[0].name, "Store");
        assert!(results.iter().any(|command| command.name == "Search apps"));
    }

    #[test]
    fn unknown_palette_query_routes_to_tweak_search() {
        let results = matching_palette_commands("disable telemetry");
        assert_eq!(results.len(), 1);
        assert!(results[0].name.contains("disable telemetry"));
        assert!(matches!(results[0].command, PaletteCommand::SearchTweaks));
        assert_eq!(
            palette_tweak_query("disable telemetry"),
            "disable telemetry"
        );
        assert_eq!(palette_tweak_query("Search tweaks: bluetooth"), "bluetooth");
    }

    #[test]
    fn palette_commands_can_change_the_color_palette() {
        let results = matching_palette_commands("oled");
        let oled = results
            .iter()
            .find(|result| {
                matches!(
                    result.command,
                    PaletteCommand::Palette(crate::theme::Palette::Oled)
                )
            })
            .expect("the OLED palette command is searchable");
        assert_eq!(oled.name, "OLED palette");
    }

    #[test]
    fn store_favorites_filter_preserves_catalog_order() {
        let mut apps = vec![
            AppItem {
                id: "first".into(),
                ..AppItem::default()
            },
            AppItem {
                id: "saved".into(),
                ..AppItem::default()
            },
            AppItem {
                id: "last".into(),
                ..AppItem::default()
            },
        ];
        retain_store_favorites(&mut apps, &["saved".into(), "last".into()]);
        assert_eq!(
            apps.iter().map(|app| app.id.as_str()).collect::<Vec<_>>(),
            ["saved", "last"]
        );
    }
}
