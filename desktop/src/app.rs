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

const CARD_H: f32 = 108.0;
const GAP: f32 = 12.0;
// Below this width, move Store search to its own row and replace text actions with icons.
const TOP_BAR_COMPACT_WIDTH: f32 = 1280.0;
// The full text actions and native window controls share the row with Store search.
const TOP_BAR_ACTIONS_WIDTH: f32 = 780.0;
const PLATFORMS: [&str; 4] = [PC, ANDROID, TV, "All"];
const PALETTE_RECENT_ACTION_LIMIT: usize = 8;

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

#[derive(Clone, Copy)]
enum PaletteCommand {
    Page(&'static str),
    RecentActivity,
    Store,
    SearchStore,
    SearchTweaks,
    WinGet,
    Downloads,
    Settings,
    Palette(theme::Palette),
}

#[derive(Clone, Copy)]
struct PaletteEntry {
    name: &'static str,
    category: &'static str,
    hint: &'static str,
    command: PaletteCommand,
}

#[derive(Clone)]
struct PaletteMatch {
    name: std::borrow::Cow<'static, str>,
    category: &'static str,
    hint: std::borrow::Cow<'static, str>,
    command: PaletteCommand,
    activity_search: Option<String>,
}

const PALETTE_COMMANDS: [PaletteEntry; 19] = [
    PaletteEntry {
        name: "Overview",
        category: "Workspace",
        hint: "System overview",
        command: PaletteCommand::Page("Overview"),
    },
    PaletteEntry {
        name: "Performance",
        category: "Workspace",
        hint: "Live CPU, memory, disk and adapter traffic",
        command: PaletteCommand::Page("Performance"),
    },
    PaletteEntry {
        name: "Activity",
        category: "Workspace",
        hint: "Search recorded changes and system-operation results",
        command: PaletteCommand::Page("Activity"),
    },
    PaletteEntry {
        name: "Processes",
        category: "System",
        hint: "Read-only process explorer",
        command: PaletteCommand::Page("Processes"),
    },
    PaletteEntry {
        name: "Diagnostics",
        category: "System",
        hint: "Read-only Windows integrity checks",
        command: PaletteCommand::Page("Diagnostics"),
    },
    PaletteEntry {
        name: "System Snapshot",
        category: "System",
        hint: "Capture and browse local hardware inventory",
        command: PaletteCommand::Page("System Snapshot"),
    },
    PaletteEntry {
        name: "Gaming",
        category: "Optimization",
        hint: "Review reversible Windows gaming settings",
        command: PaletteCommand::Page("Gaming"),
    },
    PaletteEntry {
        name: "Tweaks",
        category: "Optimization",
        hint: "Open the tweak catalog",
        command: PaletteCommand::Page("Tweaks"),
    },
    PaletteEntry {
        name: "Services",
        category: "Optimization",
        hint: "Review Windows services",
        command: PaletteCommand::Page("Services"),
    },
    PaletteEntry {
        name: "Startup",
        category: "Optimization",
        hint: "Manage startup entries",
        command: PaletteCommand::Page("Startup"),
    },
    PaletteEntry {
        name: "Cleaner",
        category: "Optimization",
        hint: "Review and remove selected files",
        command: PaletteCommand::Page("Cleaner"),
    },
    PaletteEntry {
        name: "Network",
        category: "Optimization",
        hint: "Network and DNS tools",
        command: PaletteCommand::Page("Network"),
    },
    PaletteEntry {
        name: "Tools",
        category: "System",
        hint: "Open maintenance tools",
        command: PaletteCommand::Page("Tools"),
    },
    PaletteEntry {
        name: "Store",
        category: "Software",
        hint: "Browse apps",
        command: PaletteCommand::Store,
    },
    PaletteEntry {
        name: "Search apps",
        category: "Software",
        hint: "Focus Store search",
        command: PaletteCommand::SearchStore,
    },
    PaletteEntry {
        name: "WinGet apps",
        category: "Software",
        hint: "Install, upgrade or remove Windows apps",
        command: PaletteCommand::WinGet,
    },
    PaletteEntry {
        name: "Downloads",
        category: "Software",
        hint: "Show download activity",
        command: PaletteCommand::Downloads,
    },
    PaletteEntry {
        name: "Search tweaks",
        category: "Optimization",
        hint: "Filter the tweak catalog by your query",
        command: PaletteCommand::SearchTweaks,
    },
    PaletteEntry {
        name: "Settings",
        category: "Application",
        hint: "Open application settings",
        command: PaletteCommand::Settings,
    },
];

const PALETTE_THEME_COMMANDS: [PaletteEntry; 17] = [
    PaletteEntry {
        name: "System palette",
        category: "Appearance",
        hint: "Use the Windows system palette",
        command: PaletteCommand::Palette(theme::Palette::System),
    },
    PaletteEntry {
        name: "Midnight palette",
        category: "Appearance",
        hint: "Set the Midnight palette",
        command: PaletteCommand::Palette(theme::Palette::Midnight),
    },
    PaletteEntry {
        name: "OLED palette",
        category: "Appearance",
        hint: "Set the OLED palette",
        command: PaletteCommand::Palette(theme::Palette::Oled),
    },
    PaletteEntry {
        name: "Graphite palette",
        category: "Appearance",
        hint: "Set the Graphite palette",
        command: PaletteCommand::Palette(theme::Palette::Graphite),
    },
    PaletteEntry {
        name: "Nebula palette",
        category: "Appearance",
        hint: "Set the Nebula palette",
        command: PaletteCommand::Palette(theme::Palette::Nebula),
    },
    PaletteEntry {
        name: "Nord palette",
        category: "Appearance",
        hint: "Set the Nord palette",
        command: PaletteCommand::Palette(theme::Palette::Nord),
    },
    PaletteEntry {
        name: "Arctic palette",
        category: "Appearance",
        hint: "Set the Arctic palette",
        command: PaletteCommand::Palette(theme::Palette::Arctic),
    },
    PaletteEntry {
        name: "Fluent dark palette",
        category: "Appearance",
        hint: "Set the Fluent dark palette",
        command: PaletteCommand::Palette(theme::Palette::FluentDark),
    },
    PaletteEntry {
        name: "Fluent light palette",
        category: "Appearance",
        hint: "Set the Fluent light palette",
        command: PaletteCommand::Palette(theme::Palette::FluentLight),
    },
    PaletteEntry {
        name: "Material dark palette",
        category: "Appearance",
        hint: "Set the Material dark palette",
        command: PaletteCommand::Palette(theme::Palette::MaterialDark),
    },
    PaletteEntry {
        name: "Material light palette",
        category: "Appearance",
        hint: "Set the Material light palette",
        command: PaletteCommand::Palette(theme::Palette::MaterialLight),
    },
    PaletteEntry {
        name: "Emerald palette",
        category: "Appearance",
        hint: "Set the Emerald palette",
        command: PaletteCommand::Palette(theme::Palette::Emerald),
    },
    PaletteEntry {
        name: "Crimson palette",
        category: "Appearance",
        hint: "Set the Crimson palette",
        command: PaletteCommand::Palette(theme::Palette::Crimson),
    },
    PaletteEntry {
        name: "Solarized palette",
        category: "Appearance",
        hint: "Set the Solarized palette",
        command: PaletteCommand::Palette(theme::Palette::Solarized),
    },
    PaletteEntry {
        name: "Porcelain palette",
        category: "Appearance",
        hint: "Set the Porcelain palette",
        command: PaletteCommand::Palette(theme::Palette::Porcelain),
    },
    PaletteEntry {
        name: "Cyberpunk palette",
        category: "Appearance",
        hint: "Set the Cyberpunk palette",
        command: PaletteCommand::Palette(theme::Palette::Cyberpunk),
    },
    PaletteEntry {
        name: "Custom palette",
        category: "Appearance",
        hint: "Use your custom palette",
        command: PaletteCommand::Palette(theme::Palette::Custom),
    },
];

#[cfg(test)]
fn matching_palette_commands(query: &str) -> Vec<PaletteMatch> {
    matching_palette_commands_with_recent(query, &[])
}

fn matching_palette_commands_with_recent(
    query: &str,
    recent_actions: &[RecentAction],
) -> Vec<PaletteMatch> {
    let normalized_query = search_chars(query);
    if normalized_query.is_empty() {
        let mut matches = recent_actions
            .iter()
            .map(recent_palette_match)
            .collect::<Vec<_>>();
        matches.extend(
            PALETTE_COMMANDS
                .iter()
                .chain(PALETTE_THEME_COMMANDS.iter())
                .map(palette_match)
                .collect::<Vec<_>>(),
        );
        return matches;
    }

    let mut matches = PALETTE_COMMANDS
        .iter()
        .chain(PALETTE_THEME_COMMANDS.iter())
        .filter_map(|entry| {
            let name_score = fuzzy_match_score(&normalized_query, entry.name);
            let hint_score = fuzzy_match_score(&normalized_query, entry.hint);
            name_score
                .map(|score| (entry, (true, score)))
                .or_else(|| hint_score.map(|score| (entry, (false, score))))
        })
        .map(|(entry, score)| (palette_match(entry), score))
        .collect::<Vec<_>>();

    matches.extend(recent_actions.iter().filter_map(|action| {
        let name_score = fuzzy_match_score(&normalized_query, &action.title);
        let hint_score = fuzzy_match_score(&normalized_query, &action.hint);
        let category_score = fuzzy_match_score(&normalized_query, "Recent action");
        name_score
            .map(|score| (true, score))
            .or_else(|| hint_score.map(|score| (false, score)))
            .or_else(|| category_score.map(|score| (false, score)))
            .map(|score| (recent_palette_match(action), score))
    }));
    matches.sort_by(|(_, left), (_, right)| right.cmp(left));
    if matches.is_empty() {
        let display_query = query
            .trim()
            .chars()
            .filter(|character| !character.is_control())
            .take(80)
            .collect::<String>();
        return vec![PaletteMatch {
            name: format!("Search tweaks for “{display_query}”").into(),
            category: "Optimization",
            hint: "Filter the tweak catalog by this text".into(),
            command: PaletteCommand::SearchTweaks,
            activity_search: None,
        }];
    }
    matches.into_iter().map(|(entry, _)| entry).collect()
}

fn palette_match(entry: &PaletteEntry) -> PaletteMatch {
    PaletteMatch {
        name: entry.name.into(),
        category: entry.category,
        hint: entry.hint.into(),
        command: entry.command,
        activity_search: None,
    }
}

fn recent_palette_match(action: &RecentAction) -> PaletteMatch {
    PaletteMatch {
        name: action.title.clone().into(),
        category: "Recent",
        hint: action.hint.clone().into(),
        command: PaletteCommand::RecentActivity,
        activity_search: Some(action.search.clone()),
    }
}

fn search_chars(value: &str) -> Vec<char> {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn palette_tweak_query(query: &str) -> &str {
    let query = query.trim();
    const PREFIX: &str = "search tweaks";
    if query
        .get(..PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
    {
        query[PREFIX.len()..].trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, ':' | '-')
        })
    } else {
        query
    }
}

fn fuzzy_match_score(query: &[char], text: &str) -> Option<i32> {
    let mut searchable = Vec::new();
    let mut word_start = true;
    for character in text.chars() {
        if character.is_alphanumeric() {
            for normalized in character.to_lowercase() {
                searchable.push((normalized, word_start));
                word_start = false;
            }
        } else {
            word_start = true;
        }
    }

    let mut score = 0i32;
    let mut cursor = 0usize;
    let mut previous = None;
    for character in query {
        let (index, (_, starts_word)) = searchable
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, (candidate, _))| candidate == character)?;
        let skipped = index - cursor;
        let consecutive = previous.is_some_and(|last| index == last + 1);
        score += if consecutive { 8 } else { 2 };
        score += if *starts_word { 5 } else { 0 };
        score -= skipped as i32;
        cursor = index + 1;
        previous = Some(index);
    }

    score -= searchable.len().saturating_sub(query.len()) as i32 / 12;
    Some(score)
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
            match std::env::var("ORION_SHOTS_STYLE").as_deref() {
                Ok("classic") => settings.style = theme::Style::Classic,
                Ok("material") => settings.style = theme::Style::Material,
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

impl OrionApp {
    fn save_settings(&mut self, label: &str) {
        if let Err(error) = self.settings.save_checked() {
            self.toast = Some((format!("{label} was not saved: {error}"), Instant::now()));
        }
    }

    fn persist_appearance(&mut self, ctx: &egui::Context, previous: net::Settings, label: &str) {
        if let Err(error) = self.settings.save_checked() {
            self.settings = previous;
            self.reapply_theme(ctx);
            self.toast = Some((format!("{label} was not saved: {error}"), Instant::now()));
        }
    }

    fn custom_surface_editor(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let dark = self.settings.effective_dark();
        let mut candidate = self.settings.custom_surfaces.clone();
        let mut changed = false;
        egui::CollapsingHeader::new(if dark {
            "Dark surface colors"
        } else {
            "Light surface colors"
        })
        .id_salt("custom-surface-colors")
        .show(ui, |ui| {
            egui::Grid::new("custom-surface-grid")
                .num_columns(2)
                .show(ui, |ui| {
                    for (label, value) in theme::CustomSurfaces::LABELS
                        .into_iter()
                        .zip(candidate.active_mut(dark))
                    {
                        ui.label(RichText::new(label).color(tokens(ctx).muted));
                        let mut color =
                            theme::parse_hex_color(value).unwrap_or(tokens(ctx).surface1);
                        if ui.color_edit_button_srgba(&mut color).changed() {
                            *value = theme::hex_color(color);
                            changed = true;
                        }
                        ui.end_row();
                    }
                });
            if kit::ghost(ui, "Reset surface colors", true).clicked() {
                candidate.reset_mode(dark);
                changed = true;
            }
        });
        if !changed {
            return;
        }
        if let Err(error) = candidate.validate() {
            self.toast = Some((error, Instant::now()));
            return;
        }
        let previous = self.settings.clone();
        self.settings.custom_surfaces = candidate;
        self.reapply_theme(ctx);
        self.persist_appearance(ctx, previous, "Custom surfaces");
    }

    fn notifications(&self) -> Vec<Notification> {
        let mut notifications = Vec::new();
        if self.settings.opti_restart_required {
            notifications.push(Notification {
                id: format!("restart-required:{}", self.settings.opti_restart_uptime),
                kind: NotificationKind::Warning,
                title: "Restart required".into(),
                detail: "A Windows restart is required to finish applying changes.".into(),
                lines: Vec::new(),
                progress: None,
                action: Some(NotificationAction::ReviewActivity),
                dismissible: true,
                persistent: true,
            });
        }
        if let Some(notification) = update_notification(&self.update) {
            notifications.push(notification);
        }
        if let Some(job) = self.opti.current_job() {
            notifications.push(Notification {
                id: format!("operation-running:{job}"),
                kind: NotificationKind::Progress,
                title: job.to_owned(),
                detail: "System operation is running. Progress is not available for this task."
                    .into(),
                lines: Vec::new(),
                progress: None,
                action: Some(NotificationAction::ReviewActivity),
                dismissible: false,
                persistent: false,
            });
        }
        notifications.extend(
            self.opti
                .operation_entries()
                .iter()
                .rev()
                .filter_map(notifications::operation)
                .take(40),
        );
        for download in self.downloads.iter().rev().take(40) {
            if let Some(notification) = notifications::download(download) {
                notifications.push(notification);
            }
        }
        notifications.retain(|notification| {
            !self
                .settings
                .dismissed_notifications
                .contains(&notification.id)
                && !self
                    .dismissed_session_notifications
                    .contains(&notification.id)
        });
        notifications.truncate(notifications::MAX_VISIBLE);
        notifications
    }

    fn dismiss_notification(&mut self, notification: &Notification) {
        if !notification.dismissible {
            return;
        }
        if notification.persistent {
            let previous = self.settings.dismissed_notifications.clone();
            if !self
                .settings
                .dismissed_notifications
                .contains(&notification.id)
            {
                notifications::remember_dismissal(
                    &mut self.settings.dismissed_notifications,
                    &notification.id,
                );
                if let Err(error) = self.settings.save_checked() {
                    self.settings.dismissed_notifications = previous;
                    self.say(format!("Could not save notification dismissal: {error}"));
                }
            }
        } else if !self
            .dismissed_session_notifications
            .contains(&notification.id)
        {
            self.dismissed_session_notifications
                .push(notification.id.clone());
            if self.dismissed_session_notifications.len() > MAX_SAVED_DISMISSALS {
                self.dismissed_session_notifications.remove(0);
            }
        }
    }

    fn notification_center(&mut self, ctx: &egui::Context) {
        let notifications = self.notifications();
        let mut open = self.show_notifications;
        let mut dismissed = Vec::new();
        let mut action = None;
        egui::Window::new("Notifications")
            .id(egui::Id::NULL.with("notification-center"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .max_width(520.0)
            .max_height((ctx.content_rect().height() - 80.0).max(220.0))
            .anchor(egui::Align2::RIGHT_TOP, vec2(-16.0, 52.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if notifications.is_empty() {
                            ui.label(
                                RichText::new("No current notifications.").color(tokens(ctx).muted),
                            );
                        }
                        for notification in &notifications {
                            kit::card(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    kit::chip(
                                        ui,
                                        notification.kind.label(),
                                        notification.kind.tone(),
                                    );
                                    ui.label(
                                        RichText::new(&notification.title)
                                            .font(FontId::new(T_BODY, semibold())),
                                    );
                                    if notification.dismissible
                                        && kit::ghost(ui, "Dismiss", true).clicked()
                                    {
                                        dismissed.push(notification.id.clone());
                                    }
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&notification.detail)
                                            .size(T_SMALL)
                                            .color(tokens(ctx).muted),
                                    )
                                    .wrap(),
                                );
                                if let Some(progress) = notification.progress {
                                    ui.add(egui::ProgressBar::new(progress).show_percentage());
                                }
                                if !notification.lines.is_empty() {
                                    egui::CollapsingHeader::new("Details")
                                        .id_salt(("notification-details", &notification.id))
                                        .show(ui, |ui| {
                                            for line in &notification.lines {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(line)
                                                            .size(T_CAPTION)
                                                            .color(tokens(ctx).subtle),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                        });
                                }
                                if let Some(item_action) = &notification.action {
                                    let label = match item_action {
                                        NotificationAction::ReviewActivity => "Review activity",
                                        NotificationAction::OpenDownloads => "Open downloads",
                                        NotificationAction::StartUpdate(_) => "Update and restart",
                                        NotificationAction::CheckUpdates => "Check again",
                                        NotificationAction::RevealFile(_) => "Show in folder",
                                    };
                                    if kit::secondary(ui, label, true).clicked() {
                                        action = Some(item_action.clone());
                                    }
                                }
                            });
                            ui.add_space(S2);
                        }
                    });
            });
        self.show_notifications = open;
        for id in dismissed {
            if let Some(notification) = notifications.iter().find(|item| item.id == id) {
                self.dismiss_notification(notification);
            }
        }
        if let Some(action) = action {
            self.run_notification_action(ctx, action);
        }
    }

    fn run_notification_action(&mut self, ctx: &egui::Context, action: NotificationAction) {
        self.show_notifications = false;
        match action {
            NotificationAction::ReviewActivity => {
                self.set_section(ctx, Section::Optimaxer);
                self.opti.show_page(ctx, "Activity");
            }
            NotificationAction::OpenDownloads => self.show_downloads = true,
            NotificationAction::StartUpdate(release) => self.start_install(ctx, release),
            NotificationAction::CheckUpdates => self.start_check(ctx, true),
            NotificationAction::RevealFile(path) => {
                if let Err(error) = reveal(&path) {
                    self.say(format!("Could not show the downloaded file: {error}"));
                }
            }
        }
    }

    fn open_command_palette(&mut self) {
        self.palette_query.clear();
        self.palette_selected = 0;
        self.show_palette = true;
    }

    fn command_palette(&mut self, ctx: &egui::Context) {
        let t = tokens(ctx);
        let recent_actions = self.opti.recent_actions(PALETTE_RECENT_ACTION_LIMIT);
        let matches = matching_palette_commands_with_recent(&self.palette_query, &recent_actions);
        if matches.is_empty() {
            self.palette_selected = 0;
        } else {
            self.palette_selected = self.palette_selected.min(matches.len() - 1);
        }
        let mut chosen: Option<PaletteMatch> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::NULL.with("command-palette"))
            .backdrop_color(Color32::from_black_alpha(if t.dark { 155 } else { 80 }))
            .show(ctx, |ui| {
                ui.set_width(ui.available_width().clamp(240.0, 520.0));
                ui.heading("Command palette");
                ui.add_space(S2);
                let search_id = egui::Id::NULL.with("command-palette-search");
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .id(search_id)
                        .hint_text("Find a page or command…")
                        .desired_width(f32::INFINITY),
                );
                if !search.has_focus() {
                    ctx.memory_mut(|m| m.request_focus(search_id));
                }
                if search.changed() {
                    self.palette_selected = 0;
                }
                ui.add_space(S2);
                egui::ScrollArea::vertical()
                    .max_height(460.0)
                    .show(ui, |ui| {
                        for (index, command) in matches.iter().enumerate() {
                            let response = ui.selectable_label(
                                index == self.palette_selected,
                                format!(
                                    "{}    ·    {}    ·    {}",
                                    command.name, command.category, command.hint
                                ),
                            );
                            if response.clicked() {
                                chosen = Some(command.clone());
                            }
                        }
                    });
                ui.add_space(S2);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("↑ ↓ select   Enter open   Esc close")
                            .size(T_CAPTION)
                            .color(t.muted),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::ghost(ui, "Close", true).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) && !matches.is_empty() {
            self.palette_selected = (self.palette_selected + 1) % matches.len();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) && !matches.is_empty() {
            self.palette_selected = self
                .palette_selected
                .checked_sub(1)
                .unwrap_or(matches.len() - 1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            chosen = matches.get(self.palette_selected).cloned();
        }
        if let Some(command) = chosen {
            let query = self.palette_query.clone();
            self.run_palette_command(ctx, command, &query);
        } else if close || modal.should_close() {
            self.show_palette = false;
        }
    }

    fn run_palette_command(&mut self, ctx: &egui::Context, command: PaletteMatch, query: &str) {
        self.show_palette = false;
        match command.command {
            PaletteCommand::Page(name) => {
                self.set_section(ctx, Section::Optimaxer);
                self.opti.show_page(ctx, name);
            }
            PaletteCommand::RecentActivity => {
                self.set_section(ctx, Section::Optimaxer);
                if let Some(search) = command.activity_search.as_deref() {
                    self.opti.search_activity(ctx, search);
                } else {
                    self.opti.show_page(ctx, "Activity");
                }
            }
            PaletteCommand::Store => self.set_section(ctx, Section::Store),
            PaletteCommand::SearchStore => {
                self.set_section(ctx, Section::Store);
                self.detail = None;
                ctx.memory_mut(|m| m.request_focus(self.search_id));
            }
            PaletteCommand::SearchTweaks => {
                self.set_section(ctx, Section::Optimaxer);
                self.opti.search_tweaks(ctx, palette_tweak_query(query));
            }
            PaletteCommand::WinGet => {
                self.set_section(ctx, Section::Store);
                self.show_winget = true;
                self.detail = None;
            }
            PaletteCommand::Downloads => self.show_downloads = true,
            PaletteCommand::Settings => self.show_settings = true,
            PaletteCommand::Palette(palette) => {
                let previous = self.settings.clone();
                self.settings.palette = palette;
                self.reapply_theme(ctx);
                self.persist_appearance(ctx, previous, "Palette");
            }
        }
    }

    fn toast_view(&self, ctx: &egui::Context) {
        let Some((msg, born)) = &self.toast else {
            return;
        };
        let (t, m) = (tokens(ctx), motion(ctx));
        let age = born.elapsed().as_secs_f32();
        let fade_in = if m.reduce { 1.0 } else { ease_out(age / 0.18) };
        let fade_out = if m.reduce {
            1.0
        } else {
            ((3.6 - age) / 0.3).clamp(0.0, 1.0)
        };
        let a = fade_in.min(fade_out);
        egui::Area::new(egui::Id::unique("orion-toast"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                vec2(0.0, -48.0 + (1.0 - fade_in) * 10.0),
            )
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(a);
                egui::Frame::new()
                    .fill(t.surface3)
                    .stroke(Stroke::new(1.0, t.hairline))
                    .corner_radius(t.card_radius())
                    .inner_margin(Margin::symmetric(S4 as i8, S3 as i8))
                    .shadow(ctx.global_style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.label(RichText::new(msg).color(t.text));
                    });
            });
    }

    /// Full-window view of one screenshot: click outside or press Esc to close, arrow keys or the side buttons to step.
    fn lightbox_view(&mut self, ctx: &egui::Context) {
        let Some((urls, idx)) = self.lightbox.as_mut() else {
            return;
        };
        let n = urls.len();
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            *idx = (*idx + 1) % n;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            *idx = (*idx + n - 1) % n;
        }
        let t = tokens(ctx);
        let screen = ctx.content_rect();
        let mut close = false;
        let mut step: i32 = 0;
        egui::Area::new(egui::Id::NULL.with("lightbox"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, backdrop) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(225));
                let img = ui.put(
                    rect.shrink2(vec2(80.0, 56.0)),
                    egui::Image::new(urls[*idx].as_str())
                        .corner_radius(t.card_radius())
                        .sense(Sense::click()),
                );
                if backdrop.clicked() && !img.clicked() {
                    close = true;
                }
                if n > 1 {
                    let side = |ui: &mut Ui, x: f32, label: &str| {
                        let r = egui::Rect::from_center_size(
                            egui::pos2(x, rect.center().y),
                            vec2(44.0, 64.0),
                        );
                        ui.put(
                            r,
                            egui::Button::new(
                                RichText::new(label).size(T_HEADING).color(Color32::WHITE),
                            )
                            .fill(Color32::from_white_alpha(28))
                            .corner_radius(t.ctl_radius()),
                        )
                        .clicked()
                    };
                    if side(ui, rect.left() + 40.0, "<") {
                        step = -1;
                    }
                    if side(ui, rect.right() - 40.0, ">") {
                        step = 1;
                    }
                }
                let caption = format!("{} / {}", *idx + 1, n);
                ui.painter().text(
                    egui::pos2(rect.center().x, rect.bottom() - 24.0),
                    egui::Align2::CENTER_CENTER,
                    caption,
                    FontId::proportional(T_SMALL),
                    Color32::from_white_alpha(200),
                );
                let x = egui::Rect::from_center_size(
                    egui::pos2(rect.right() - 32.0, rect.top() + 28.0),
                    vec2(40.0, 40.0),
                );
                if ui
                    .put(
                        x,
                        egui::Button::new(RichText::new("x").size(T_HEADING).color(Color32::WHITE))
                            .fill(Color32::from_white_alpha(28))
                            .corner_radius(t.ctl_radius()),
                    )
                    .on_hover_text("Close")
                    .clicked()
                {
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
        let notification_count = self.notifications().len();
        egui::Panel::top("top")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(t.rail)
                    .inner_margin(Margin::symmetric(S5 as i8, S3 as i8)),
            )
            .show(root, |ui| {
                let compact = ui.available_width() < TOP_BAR_COMPACT_WIDTH;
                ui.horizontal(|ui| {
                    // the bar is the window's drag handle now that the system title bar is gone; widgets added after it sit on top
                    let drag = ui.interact(
                        ui.max_rect(),
                        ui.scope_id().with("titlebar"),
                        Sense::click_and_drag(),
                    );
                    if drag.drag_started_by(egui::PointerButton::Primary) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag.double_clicked() {
                        let max = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                    ui.label(
                        RichText::new("Orion")
                            .font(FontId::new(T_DISPLAY - 6.0, semibold()))
                            .color(t.brand_text),
                    );
                    ui.add_space(S4);
                    let mut idx = usize::from(self.section == Section::Optimaxer);
                    if kit::segmented(ui, &mut idx, &["Store", "Optimaxer"]) {
                        self.set_section(
                            ctx,
                            if idx == 1 {
                                Section::Optimaxer
                            } else {
                                Section::Store
                            },
                        );
                    }
                    ui.add_space(S4);
                    if self.section == Section::Store && !compact {
                        let w = (ui.available_width() - TOP_BAR_ACTIONS_WIDTH).clamp(180.0, 480.0);
                        let (query, hint) = if self.show_winget {
                            (
                                &mut self.software.query,
                                "Search WinGet apps or package IDs  (Ctrl+F)",
                            )
                        } else {
                            (&mut self.query, "Search apps, authors, tags  (Ctrl+F)")
                        };
                        let resp = ui.add_sized(
                            [w, 34.0],
                            egui::TextEdit::singleline(query)
                                .id(self.search_id)
                                .hint_text(hint)
                                .margin(Margin::symmetric(S3 as i8, 8)),
                        );
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
                        let (glyph, fallback, label) = if maximized {
                            ('\u{E923}', "o", "Restore")
                        } else {
                            ('\u{E922}', "[]", "Maximize")
                        };
                        if kit::caption_button(ui, glyph, fallback, false, label).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if kit::caption_button(ui, '\u{E921}', "-", false, "Minimize").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.add_space(S3);
                        if compact {
                            if kit::toolbar_icon_button(
                                ui,
                                '\u{E721}',
                                "K",
                                "Commands (Ctrl+K)",
                                false,
                            )
                            .clicked()
                            {
                                self.open_command_palette();
                            }
                            let label = if notification_count == 0 {
                                "Notifications".into()
                            } else {
                                format!("Notifications ({notification_count})")
                            };
                            if kit::toolbar_icon_button(
                                ui,
                                '\u{E7F4}',
                                "N",
                                &label,
                                self.show_notifications,
                            )
                            .clicked()
                            {
                                self.show_notifications = !self.show_notifications;
                            }
                            if kit::toolbar_icon_button(
                                ui,
                                '\u{E713}',
                                "S",
                                "Settings",
                                self.show_settings,
                            )
                            .clicked()
                            {
                                self.show_settings = !self.show_settings;
                            }
                        } else {
                            if kit::ghost(ui, "Commands  Ctrl+K", true).clicked() {
                                self.open_command_palette();
                            }
                            let label = if notification_count == 0 {
                                "Notifications".into()
                            } else {
                                format!("Notifications ({notification_count})")
                            };
                            if kit::ghost(ui, &label, true).clicked() {
                                self.show_notifications = !self.show_notifications;
                            }
                            if kit::ghost(ui, "Settings", true).clicked() {
                                self.show_settings = !self.show_settings;
                            }
                        }
                        let n = self
                            .downloads
                            .iter()
                            .filter(|d| matches!(d.status(), DlStatus::Running))
                            .count();
                        let label = if n > 0 {
                            format!("Downloads ({n})")
                        } else {
                            "Downloads".into()
                        };
                        let dl = if compact {
                            kit::toolbar_icon_button(
                                ui,
                                '\u{E896}',
                                "D",
                                &label,
                                self.show_downloads,
                            )
                        } else if self.show_downloads {
                            kit::secondary(ui, &label, true)
                        } else {
                            kit::ghost(ui, &label, true)
                        };
                        if dl.clicked() {
                            self.show_downloads = !self.show_downloads;
                        }
                        if self.section == Section::Store && !compact && !self.show_winget {
                            let label = if self.loading {
                                "Refreshing"
                            } else {
                                "Refresh"
                            };
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
                        let (query, hint) = if self.show_winget {
                            (
                                &mut self.software.query,
                                "Search WinGet apps or package IDs  (Ctrl+F)",
                            )
                        } else {
                            (&mut self.query, "Search apps, authors, tags  (Ctrl+F)")
                        };
                        let search = ui.add_sized(
                            [width, 34.0],
                            egui::TextEdit::singleline(query)
                                .id(self.search_id)
                                .hint_text(hint)
                                .margin(Margin::symmetric(S3 as i8, 8)),
                        );
                        if search.changed() {
                            self.dirty = true;
                            self.detail = None;
                            self.bump(ctx);
                        }
                        if !self.show_winget
                            && kit::ghost(
                                ui,
                                if self.loading {
                                    "Refreshing"
                                } else {
                                    "Refresh"
                                },
                                !self.loading,
                            )
                            .clicked()
                        {
                            self.loading = true;
                            self.rx = spawn_refresh(ctx);
                        }
                    });
                }
            });
    }

    fn status_bar(&mut self, root: &mut Ui) {
        let t = tokens(root.ctx());
        egui::Panel::bottom("status")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(t.rail)
                    .inner_margin(Margin::symmetric(S5 as i8, S2 as i8)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    if self.section == Section::Optimaxer {
                        ui.label(
                            RichText::new(self.opti.status_text())
                                .size(T_CAPTION)
                                .color(t.subtle),
                        );
                        return;
                    }
                    if self.show_winget {
                        ui.label(
                            RichText::new(self.software.status_line())
                                .size(T_CAPTION)
                                .color(t.subtle),
                        );
                        return;
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} apps  ·  {} shown  ·  source: {}",
                            self.catalog.apps.len(),
                            self.view.len(),
                            self.catalog.source
                        ))
                        .size(T_CAPTION)
                        .color(t.subtle),
                    );
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
        egui::Panel::left("side")
            .resizable(false)
            .show_separator_line(false)
            .exact_size(if compact { 64.0 } else { 212.0 })
            .frame(
                egui::Frame::new()
                    .fill(t.rail)
                    .inner_margin(Margin::symmetric(S3 as i8, S2 as i8)),
            )
            .show(root, |ui| {
                if self.section == Section::Optimaxer {
                    egui::ScrollArea::vertical()
                        .id_salt("optimaxer-navigation")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(S2);
                            let label = if self.settings.sidebar_collapsed {
                                "Expand navigation"
                            } else {
                                "Collapse navigation"
                            };
                            let glyph = if self.settings.sidebar_collapsed {
                                "›"
                            } else {
                                "‹"
                            };
                            let mut toggle_clicked = false;
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let toggle = ui
                                        .add_sized(
                                            [30.0, 30.0],
                                            egui::Button::new(glyph)
                                                .fill(t.surface2)
                                                .stroke(egui::Stroke::new(1.0, t.hairline)),
                                        )
                                        .on_hover_text(label);
                                    toggle.widget_info(|| {
                                        egui::WidgetInfo::labeled(egui::Role::Button, true, label)
                                    });
                                    toggle_clicked = toggle.clicked();
                                });
                            });
                            if toggle_clicked {
                                self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed;
                                self.save_settings("Navigation preference");
                            }
                            self.opti
                                .side_panel(ui, ctx, self.settings.sidebar_collapsed);
                        });
                    return;
                }
                if self.show_winget {
                    side_title(ui, "Software");
                    if kit::nav_item(ui, false, "Orion Store", None).clicked() {
                        self.show_winget = false;
                        self.category = "All".into();
                        self.favorites_only = false;
                        self.detail = None;
                        self.bump(ctx);
                    }
                    self.software.navigation(ui);
                    return;
                }
                side_title(ui, "Platform");
                let mut changed = false;
                for p in PLATFORMS {
                    let n = self
                        .catalog
                        .apps
                        .iter()
                        .filter(|a| p == "All" || a.platform == p)
                        .count();
                    let selected = self.settings.platform == p;
                    if kit::nav_item(ui, selected, p, Some(&n.to_string())).clicked() && !selected {
                        self.settings.platform = p.into();
                        self.category = "All".into();
                        changed = true;
                    }
                }
                side_title(ui, "Software");
                if kit::nav_item(ui, false, "WinGet apps", None).clicked() {
                    self.show_winget = true;
                    self.detail = None;
                    self.bump(ctx);
                }
                side_title(ui, "Library");
                let favorite_count = self
                    .catalog
                    .apps
                    .iter()
                    .filter(|app| {
                        self.platform_filter()
                            .is_none_or(|platform| app.platform == platform)
                            && self.settings.store_favorites.contains(&app.id)
                    })
                    .count();
                if kit::nav_item(
                    ui,
                    self.favorites_only,
                    "Favorites",
                    Some(&favorite_count.to_string()),
                )
                .clicked()
                    && !self.favorites_only
                {
                    self.category = "All".into();
                    self.favorites_only = true;
                    changed = true;
                }
                side_title(ui, "Category");
                let cats = self.categories();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let all = cats.iter().map(|c| c.1).sum::<usize>();
                        for (name, n) in std::iter::once(("All".to_string(), all)).chain(cats) {
                            let selected = !self.favorites_only && self.category == name;
                            if kit::nav_item(ui, selected, &name, Some(&n.to_string())).clicked()
                                && !selected
                            {
                                self.category = name;
                                self.favorites_only = false;
                                changed = true;
                            }
                        }
                    });
                if changed {
                    self.dirty = true;
                    self.detail = None;
                    self.bump(ctx);
                    self.save_settings("Store filters");
                }
            });
    }

    fn store_view(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let t = tokens(ctx);
        ui.horizontal(|ui| {
            let title = if !self.query.trim().is_empty() {
                format!("Results for \"{}\"", self.query.trim())
            } else if self.favorites_only {
                "Favorites".into()
            } else {
                format!("{} apps", self.settings.platform)
            };
            ui.label(
                RichText::new(title)
                    .font(FontId::new(theme::T_TITLE, semibold()))
                    .color(t.text),
            );
            ui.label(
                RichText::new(format!("{}", self.view.len()))
                    .size(T_SMALL)
                    .color(t.subtle),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let before = self.settings.sort;
                egui::ComboBox::from_id_salt("sort")
                    .selected_text(self.settings.sort.label())
                    .show_ui(ui, |ui| {
                        for s in Sort::ALL {
                            ui.selectable_value(&mut self.settings.sort, s, s.label());
                        }
                    });
                if before != self.settings.sort {
                    self.dirty = true;
                    self.save_settings("Store sort order");
                }
                ui.label(RichText::new("Sort").size(T_SMALL).color(t.muted));
            });
        });
        ui.add_space(S2);
        if self.view.is_empty() {
            if self.loading && self.catalog.apps.is_empty() {
                kit::skeleton(ui, 6);
            } else if self.favorites_only && self.settings.store_favorites.is_empty() {
                kit::empty_state(
                    ui,
                    "No favorite apps",
                    "Open an app and add it to Favorites.",
                );
            } else {
                kit::empty_state(
                    ui,
                    "No apps match",
                    "Try another platform or category, or clear the search.",
                );
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
            ui.label(
                RichText::new("All apps")
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.muted),
            );
            ui.add_space(S1);
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, CARD_H + GAP, rows, |ui, range| {
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
        let Some(app) = self.detail.clone() else {
            return;
        };
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
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&app.name).font(FontId::new(T_DISPLAY, semibold())).color(t.text));
                        let saved = self.settings.store_favorites.contains(&app.id);
                        if kit::ghost(ui, if saved { "Remove favorite" } else { "Add to favorites" }, true).clicked() {
                            self.toggle_store_favorite(&app.id);
                        }
                    });
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
                            let resp = ui.add(egui::Image::new(s.as_str()).fit_to_exact_size(vec2(320.0, 190.0)).corner_radius(t.card_radius()).sense(Sense::click()));
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
            let variants = if app.versions.is_empty() {
                &app.variants
            } else {
                &app.versions[self.sel_stream.min(app.versions.len() - 1)].variants
            };
            if app.versions.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Release channel").color(t.muted));
                    let cur = &app.versions[self.sel_stream.min(app.versions.len() - 1)];
                    egui::ComboBox::from_id_salt("stream")
                        .selected_text(format!("{} {}", cur.stream, cur.version))
                        .show_ui(ui, |ui| {
                            for (i, v) in app.versions.iter().enumerate() {
                                if ui
                                    .selectable_value(
                                        &mut self.sel_stream,
                                        i,
                                        format!("{}  {}  {}", v.stream, v.version, v.date),
                                    )
                                    .clicked()
                                {
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
            let label = |v: &model::AppVariant| {
                format!("{}   {}   {}", v.arch, v.name, model::format_size(v.size))
            };
            if variants.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Build").color(t.muted));
                    egui::ComboBox::from_id_salt("variant")
                        .width(420.0)
                        .selected_text(label(&variants[idx]))
                        .show_ui(ui, |ui| {
                            for (i, v) in variants.iter().enumerate() {
                                ui.selectable_value(&mut self.sel_variant, i, label(v));
                            }
                        });
                });
                ui.add_space(S1);
            }
            let v = &variants[idx];
            if kit::primary(
                ui,
                &format!("Download  ({})", model::format_size(v.size)),
                true,
            )
            .clicked()
            {
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
            let previous_downloads = self.recent_downloads.iter().filter(|recent| {
                !self.downloads.iter().any(|download| {
                    matches!(download.status(), DlStatus::Done(path) if path == recent.path)
                })
            }).collect::<Vec<_>>();
            if self.downloads.is_empty() && previous_downloads.is_empty() {
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
                                    ui.add(egui::ProgressBar::new(f).fill(t.action).desired_height(8.0).corner_radius(t.ctl_radius().min(4)).animate(d.fraction().is_none()).desired_width(240.0));
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
            if !previous_downloads.is_empty() {
                ui.add_space(S2);
                egui::CollapsingHeader::new(RichText::new(format!("Recent downloads ({})", previous_downloads.len())).font(FontId::new(T_SMALL, semibold())).color(t.muted))
                    .id_salt("recent-downloads")
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(112.0).auto_shrink([false, false]).show(ui, |ui| {
                            for recent in previous_downloads {
                                kit::row(ui, ("recent-dl", &recent.path), false, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.add_sized([210.0, 20.0], egui::Label::new(RichText::new(&recent.file).font(FontId::new(T_BODY, semibold())).color(t.text)).truncate());
                                        kit::chip(ui, "Downloaded", Tone::Neutral);
                                        if kit::secondary(ui, "Open", true).clicked() && open::that(&recent.path).is_err() {
                                            note = Some("Could not open the downloaded file. It may have been moved or removed.".into());
                                        }
                                        if kit::ghost(ui, "Show in folder", true).clicked()
                                            && let Err(error) = reveal(&recent.path) {
                                            note = Some(format!("Could not show the downloaded file: {error}"));
                                        }
                                    });
                                });
                            }
                        });
                    });
            }
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
        if self
            .downloads
            .iter()
            .any(|d| matches!(d.status(), DlStatus::Running))
        {
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
                            let previous = self.settings.clone();
                            self.settings.theme = ["system", "light", "dark"][ti].into();
                            self.reapply_theme(ctx);
                            self.persist_appearance(ctx, previous, "Theme");
                        }
                    });
                    let mut si = theme::Style::ALL.iter().position(|x| *x == self.settings.style).unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Style").color(t.muted));
                        if kit::segmented(ui, &mut si, &["Windows", "Classic", "Material"]) {
                            let previous = self.settings.clone();
                            self.settings.style = theme::Style::ALL[si];
                            self.reapply_theme(ctx);
                            self.persist_appearance(ctx, previous, "Control style");
                        }
                    });
                    let mut density_index = theme::Density::ALL
                        .iter()
                        .position(|choice| *choice == self.settings.density)
                        .unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Density").color(t.muted));
                        if kit::segmented(ui, &mut density_index, &["Comfortable", "Compact"]) {
                            let previous = self.settings.clone();
                            self.settings.density = theme::Density::ALL[density_index];
                            self.reapply_theme(ctx);
                            self.persist_appearance(ctx, previous, "Density");
                        }
                    });
                    let mut scale_index = theme::UiScale::ALL
                        .iter()
                        .position(|choice| *choice == self.settings.ui_scale)
                        .unwrap_or(1);
                    let scale_labels = theme::UiScale::ALL.map(theme::UiScale::label);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Scale").color(t.muted));
                        if kit::segmented(ui, &mut scale_index, &scale_labels)
                        {
                            let previous = self.settings.clone();
                            self.settings.ui_scale = theme::UiScale::ALL[scale_index];
                            self.reapply_theme(ctx);
                            self.persist_appearance(ctx, previous, "Interface scale");
                        }
                    })
                    .response
                    .on_hover_text("Changes text and interface size across Store and Optimaxer.");
                    let mut custom_radius = self.settings.corner_radius.is_some();
                    if ui.checkbox(&mut custom_radius, "Customize corner radius").changed() {
                        let previous = self.settings.clone();
                        self.settings.corner_radius = custom_radius.then(|| {
                            theme::Tokens::of(
                                self.settings.effective_dark(),
                                self.settings.style,
                                self.settings.accent(),
                            )
                            .with_palette(self.settings.palette, self.settings.accent())
                            .ctl_radius()
                            .min(16)
                        });
                        self.reapply_theme(ctx);
                        self.persist_appearance(ctx, previous, "Corner radius");
                    }
                    if let Some(mut radius) = self.settings.corner_radius {
                        let previous_radius = radius;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Radius").color(t.muted));
                            ui.add(egui::Slider::new(&mut radius, 0..=16).suffix(" px"));
                        });
                        if radius != previous_radius {
                            let previous = self.settings.clone();
                            self.settings.corner_radius = Some(radius);
                            self.reapply_theme(ctx);
                            self.persist_appearance(ctx, previous, "Corner radius");
                        }
                    }
                    let mut palette = self.settings.palette;
                    ui.label(RichText::new("Palette").font(FontId::new(T_SMALL, semibold())).color(t.muted));
                    egui::Grid::new("appearance-palette-gallery").num_columns(3).spacing(vec2(S3, S1)).show(ui, |ui| {
                        for (index, choice) in theme::Palette::ALL.into_iter().enumerate() {
                            let dark = choice.forced_dark().unwrap_or(self.settings.effective_dark());
                            let sample = theme::Tokens::of(dark, self.settings.style, self.settings.accent())
                                .with_palette(choice, self.settings.accent());
                            ui.horizontal(|ui| {
                                let (swatch, _) = ui.allocate_exact_size(vec2(27.0, 10.0), egui::Sense::hover());
                                for (offset, color) in [sample.brand, sample.good, sample.warn].into_iter().enumerate() {
                                    let rect = egui::Rect::from_min_size(swatch.min + vec2(offset as f32 * 9.0, 0.0), vec2(8.0, 10.0));
                                    ui.painter().rect_filled(rect, CornerRadius::same(2), color);
                                }
                                if ui.selectable_label(palette == choice, choice.label())
                                    .on_hover_text(format!("Preview {} accent and status colors", choice.label())).clicked() {
                                    palette = choice;
                                }
                            });
                            if index % 3 == 2 { ui.end_row(); }
                        }
                    });
                    if palette != self.settings.palette {
                        let previous = self.settings.clone();
                        self.settings.palette = palette;
                        self.reapply_theme(ctx);
                        self.persist_appearance(ctx, previous, "Palette");
                    }
                    if self.settings.palette == theme::Palette::Custom {
                        let mut accent = theme::parse_hex_color(&self.settings.custom_accent).unwrap_or(theme::BRAND);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Custom accent").color(t.muted));
                            if ui.color_edit_button_srgba(&mut accent).changed() {
                                let previous = self.settings.clone();
                                self.settings.custom_accent = theme::hex_color(accent);
                                self.reapply_theme(ctx);
                                self.persist_appearance(ctx, previous, "Custom accent");
                            }
                            if kit::ghost(ui, "Reset", true).clicked() {
                                let previous = self.settings.clone();
                                self.settings.custom_accent = theme::hex_color(theme::BRAND);
                                self.reapply_theme(ctx);
                                self.persist_appearance(ctx, previous, "Custom accent");
                            }
                            ui.label(RichText::new(&self.settings.custom_accent).size(T_CAPTION).color(t.subtle));
                        });
                        self.custom_surface_editor(ui, ctx);
                    }
                    ui.add(egui::Label::new(RichText::new("Color palettes are separate from control shape. System follows Windows; Custom saves an accent and editable light/dark surfaces with contrast-adjusted text.").size(T_CAPTION).color(t.subtle)).wrap());
                    let mut reduce = self.settings.reduce_motion;
                    if ui.checkbox(&mut reduce, "Reduce motion").on_hover_text("Turns off transitions and the loading pulse. Follows the Windows setting by default.").changed() {
                        let previous = self.settings.clone();
                        self.settings.reduce_motion = reduce;
                        self.reapply_theme(ctx);
                        self.persist_appearance(ctx, previous, "Motion preference");
                    }
                    ui.horizontal_wrapped(|ui| {
                        if kit::secondary(ui, "Export appearance…", !net::READ_ONLY.load(Ordering::Relaxed)).clicked() {
                            self.export_appearance();
                        }
                        if kit::secondary(ui, "Import appearance…", !net::READ_ONLY.load(Ordering::Relaxed)).clicked() {
                            self.import_appearance(ctx);
                        }
                    });
                    if let Some((message, at)) = &self.toast
                        && at.elapsed() < Duration::from_secs(4)
                    {
                        ui.add(egui::Label::new(RichText::new(message).size(T_CAPTION).color(t.muted)).wrap());
                    }
                    ui.add(egui::Label::new(RichText::new("Appearance files include theme, control style, palette, accent, surfaces, density, scale, corner radius and motion preference. They do not change other app settings or Windows settings.").size(T_CAPTION).color(t.subtle)).wrap());
                });
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Updates").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let mut auto = self.settings.auto_update;
                    if ui.checkbox(&mut auto, "Check for updates when the app starts").on_hover_text("Only checks. Nothing is downloaded until you press Update and restart.").changed() {
                        self.settings.auto_update = auto;
                        self.save_settings("Update preference");
                    }
                    ui.horizontal(|ui| {
                        let busy = self.update.is_busy();
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
                    ui.label(RichText::new("Performance").font(FontId::new(T_BODY, semibold())).color(t.text));
                    ui.label(RichText::new("Sampling runs only while Overview or Performance is open. Changing either option clears the current chart window.").size(T_CAPTION).color(t.subtle));
                    let mut interval = crate::opti::sysmon::SAMPLE_INTERVAL_SECONDS
                        .iter()
                        .position(|value| *value == self.settings.telemetry_config().interval_seconds())
                        .unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Sample interval").color(t.muted));
                        if kit::segmented(ui, &mut interval, &["1s", "2s", "5s"]) {
                            let previous = self.settings.opti_sample_interval_seconds;
                            self.settings.opti_sample_interval_seconds =
                                crate::opti::sysmon::SAMPLE_INTERVAL_SECONDS[interval];
                            if let Err(error) = self.settings.save_checked() {
                                self.settings.opti_sample_interval_seconds = previous;
                                self.toast = Some((format!("Sample interval was not saved: {error}"), Instant::now()));
                            }
                        }
                    });
                    let mut history = crate::opti::sysmon::HISTORY_WINDOW_SECONDS
                        .iter()
                        .position(|value| *value == self.settings.telemetry_config().history_seconds())
                        .unwrap_or(1);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Chart history").color(t.muted));
                        if kit::segmented(ui, &mut history, &["30s", "60s", "120s"]) {
                            let previous = self.settings.opti_history_seconds;
                            self.settings.opti_history_seconds =
                                crate::opti::sysmon::HISTORY_WINDOW_SECONDS[history];
                            if let Err(error) = self.settings.save_checked() {
                                self.settings.opti_history_seconds = previous;
                                self.toast = Some((format!("Chart history was not saved: {error}"), Instant::now()));
                            }
                        }
                    });
                });
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Downloads").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let resp = ui.add(egui::TextEdit::singleline(&mut self.settings.download_dir).desired_width(f32::INFINITY).margin(Margin::symmetric(S3 as i8, 8)));
                    if resp.lost_focus() {
                        self.save_settings("Download folder");
                    }
                    ui.horizontal(|ui| {
                        if kit::secondary(ui, "Open folder", true).clicked() {
                            let _ = std::fs::create_dir_all(&self.settings.download_dir);
                            let _ = open::that(&self.settings.download_dir);
                        }
                        if kit::ghost(ui, "Reset", true).clicked() {
                            self.settings.download_dir = Settings::default().download_dir;
                            self.save_settings("Download folder");
                        }
                    });
                });
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Optimaxer").font(FontId::new(T_BODY, semibold())).color(t.text));
                    let mut rp = self.settings.opti_restore_point;
                    if ui.checkbox(&mut rp, "Offer a System Restore point before applying tweaks").changed() {
                        let previous = self.settings.opti_restore_point;
                        self.settings.opti_restore_point = rp;
                        if let Err(error) = self.settings.save_checked() {
                            self.settings.opti_restore_point = previous;
                            self.toast = Some((format!("Restore point preference was not saved: {error}"), Instant::now()));
                        }
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

    fn export_appearance(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Optimaxer appearance", &["json"])
            .set_file_name("optimaxer-appearance.json")
            .save_file()
        else {
            return;
        };
        let preset = crate::appearance::Preset::capture(&self.settings);
        let message = match crate::appearance::write(&path, &preset) {
            Ok(()) => format!("Appearance exported to {}.", path.display()),
            Err(error) => format!("Could not export appearance: {error}"),
        };
        self.toast = Some((message, Instant::now()));
    }

    fn import_appearance(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Optimaxer appearance", &["json"])
            .pick_file()
        else {
            return;
        };
        let preset = match crate::appearance::read(&path) {
            Ok(preset) => preset,
            Err(error) => {
                self.toast = Some((
                    format!("Could not import appearance: {error}"),
                    Instant::now(),
                ));
                return;
            }
        };
        let previous = self.settings.clone();
        preset.apply(&mut self.settings);
        self.reapply_theme(ctx);
        match self.settings.save_checked() {
            Ok(()) => {
                self.toast = Some((
                    format!("Imported appearance from {}.", path.display()),
                    Instant::now(),
                ));
            }
            Err(error) => {
                self.settings = previous;
                self.reapply_theme(ctx);
                self.toast = Some((format!("Appearance was not saved: {error}"), Instant::now()));
            }
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
        assert_eq!(matching_palette_commands("  ").len(), 36);
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
