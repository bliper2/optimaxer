use crate::kit;
use crate::model::{self, AppItem, Catalog, Sort, ANDROID, PC, TV};
use crate::net::{self, DlStatus, Download, Settings};
use crate::opti_ui::{OptiEvent, OptiUi};
use crate::theme::{self, ease_out, mix, motion, semibold, tokens, Tone, LIME, R_LG, R_MD, S1, S2, S3, S4, S5, T_BODY, T_CAPTION, T_DISPLAY, T_HEADING, T_SMALL};
use egui::{Align, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, StrokeKind, Ui, WidgetInfo, Role, vec2};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CARD_H: f32 = 108.0;
const GAP: f32 = 12.0;
const PLATFORMS: [&str; 4] = [PC, ANDROID, TV, "All"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Store,
    Optimaxer,
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
    toast: Option<(String, Instant)>,
    search_id: egui::Id,
    /// `ctx.input(|i| i.time)` of the last view switch, for the page-enter transition.
    view_started: f64,
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
            if let Ok(t) = std::env::var("ORION_SHOTS_THEME") {
                settings.dark = t != "light";
            }
        }
        theme::apply(&cc.egui_ctx, settings.dark, settings.reduce_motion);
        let catalog = net::load_cache().unwrap_or_else(net::embedded);
        let rx = spawn_refresh(&cc.egui_ctx);
        Self {
            shots: crate::shots::Shots::from_env(),
            section: if settings.section == "Optimaxer" { Section::Optimaxer } else { Section::Store },
            opti: OptiUi::new(),
            suggestions: suggest(&catalog.apps),
            settings,
            catalog,
            rx,
            loading: true,
            note: String::new(),
            query: String::new(),
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
            toast: None,
            search_id: egui::Id::unique("orion-search"),
            view_started: 0.0,
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

fn reveal(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
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
        let r = CornerRadius::same(R_LG);
        let pressed = resp.is_pointer_button_down_on();
        let fill = if pressed { t.surface3 } else { mix(t.surface1, t.surface2, h) };
        ui.painter().rect_filled(rect, r, fill);
        let ring = if resp.has_focus() { 2.0 } else { 1.0 };
        let edge = if resp.has_focus() { if t.dark { t.brand } else { t.brand_text } } else { mix(t.hairline, t.brand, h * 0.85) };
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
        self.opti.tick(ctx, &mut self.settings);
        self.handle_opti_events(ctx);
        self.drive_shots(ctx);
        if self.dirty {
            self.refilter();
        }
        self.tick_toast(ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.show_settings {
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

        self.top_bar(root);
        self.status_bar(root);
        if self.show_downloads {
            self.downloads_panel(root);
        }
        if self.section == Section::Optimaxer {
            self.opti.log_panel(root);
        }
        self.side_panel(root);
        let t = tokens(ctx);
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.canvas).inner_margin(Margin { left: S5 as i8, right: S5 as i8, top: S3 as i8, bottom: 0 })).show(root, |ui| {
            let started = self.view_started;
            kit::enter(ui, started, |ui| {
                if self.section == Section::Optimaxer {
                    self.opti.central(ui, ctx, &self.suggestions);
                } else if self.detail.is_some() {
                    self.detail_view(ctx, ui);
                } else {
                    self.store_view(ctx, ui);
                }
            });
        });
        if self.show_settings {
            self.settings_modal(ctx);
        }
        self.toast_view(ctx);
    }
}

impl OrionApp {
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

    fn top_bar(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let t = tokens(ctx);
        egui::Panel::top("top").show_separator_line(false).frame(egui::Frame::new().fill(t.rail).inner_margin(Margin::symmetric(S5 as i8, S3 as i8))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Orion").font(FontId::new(T_DISPLAY - 6.0, semibold())).color(t.brand_text));
                ui.add_space(S4);
                let mut idx = usize::from(self.section == Section::Optimaxer);
                if kit::segmented(ui, &mut idx, &["Store", "Optimaxer"]) {
                    self.set_section(ctx, if idx == 1 { Section::Optimaxer } else { Section::Store });
                }
                ui.add_space(S4);
                if self.section == Section::Store {
                    let w = (ui.available_width() - 330.0).clamp(180.0, 480.0);
                    let resp = ui.add_sized([w, 34.0], egui::TextEdit::singleline(&mut self.query).id(self.search_id).hint_text("Search apps, authors, tags  (Ctrl+F)").margin(Margin::symmetric(S3 as i8, 8)));
                    if resp.changed() {
                        self.dirty = true;
                        self.detail = None;
                        self.bump(ctx);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kit::ghost(ui, "Settings", true).clicked() {
                        self.show_settings = !self.show_settings;
                    }
                    let n = self.downloads.iter().filter(|d| matches!(d.status(), DlStatus::Running)).count();
                    let label = if n > 0 { format!("Downloads ({n})") } else { "Downloads".into() };
                    let dl = if self.show_downloads { kit::secondary(ui, &label, true) } else { kit::ghost(ui, &label, true) };
                    if dl.clicked() {
                        self.show_downloads = !self.show_downloads;
                    }
                    if self.section == Section::Store {
                        let label = if self.loading { "Refreshing" } else { "Refresh" };
                        if kit::ghost(ui, label, !self.loading).clicked() {
                            self.loading = true;
                            self.rx = spawn_refresh(ctx);
                        }
                    }
                });
            });
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
        egui::Panel::left("side").resizable(false).show_separator_line(false).exact_size(212.0).frame(egui::Frame::new().fill(t.rail).inner_margin(Margin::symmetric(S3 as i8, S2 as i8))).show(root, |ui| {
            if self.section == Section::Optimaxer {
                self.opti.side_panel(ui, ctx);
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
                        for s in &app.screenshots {
                            ui.add(egui::Image::new(s.as_str()).fit_to_exact_size(vec2(320.0, 190.0)).corner_radius(R_LG));
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
                    if kit::ghost(ui, "Open folder", true).clicked() {
                        let _ = std::fs::create_dir_all(&self.settings.download_dir);
                        let _ = open::that(&self.settings.download_dir);
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
                                    ui.add(egui::ProgressBar::new(f).fill(LIME).desired_height(8.0).corner_radius(4).animate(d.fraction().is_none()).desired_width(240.0));
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
                                    if kit::ghost(ui, "Show in folder", true).clicked() {
                                        reveal(&path);
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
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Appearance").font(FontId::new(T_BODY, semibold())).color(t.text));
                let mut dark = self.settings.dark;
                if ui.checkbox(&mut dark, "Dark theme").changed() {
                    self.settings.dark = dark;
                    theme::apply(ctx, dark, self.settings.reduce_motion);
                    self.settings.save();
                }
                let mut reduce = self.settings.reduce_motion;
                if ui.checkbox(&mut reduce, "Reduce motion").on_hover_text("Turns off transitions and the loading pulse. Follows the Windows setting by default.").changed() {
                    self.settings.reduce_motion = reduce;
                    theme::apply(ctx, self.settings.dark, reduce);
                    self.settings.save();
                }
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
        if resp.should_close() {
            self.show_settings = false;
        }
    }
}
