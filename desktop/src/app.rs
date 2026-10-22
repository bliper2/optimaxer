use crate::model::{self, AppItem, Catalog, Sort, ANDROID, PC, TV};
use crate::net::{self, DlStatus, Download, Settings};
use crate::opti_ui::{OptiEvent, OptiUi};
use egui::{Align, Color32, CornerRadius, FontId, Layout, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const BRAND: Color32 = Color32::from_rgb(0x74, 0x78, 0xF8);
pub(crate) const LIME: Color32 = Color32::from_rgb(0xA3, 0xE6, 0x35);
const CARD_H: f32 = 104.0;
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
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, path) in [("segoe", r"C:\Windows\Fonts\segoeui.ttf"), ("segoe-bold", r"C:\Windows\Fonts\segoeuib.ttf")] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
        }
    }
    if fonts.font_data.contains_key("segoe") {
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    ctx.set_fonts(fonts);
}

fn apply_theme(ctx: &egui::Context, dark: bool) {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.selection.bg_fill = BRAND;
    v.hyperlink_color = BRAND;
    if dark {
        v.panel_fill = Color32::from_rgb(0x13, 0x14, 0x1A);
        v.window_fill = Color32::from_rgb(0x1B, 0x1C, 0x24);
        v.extreme_bg_color = Color32::from_rgb(0x0E, 0x0F, 0x14);
        v.faint_bg_color = Color32::from_rgb(0x1C, 0x1D, 0x26);
    } else {
        v.panel_fill = Color32::from_rgb(0xF6, 0xF5, 0xFC);
        v.faint_bg_color = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
    });
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
        install_fonts(&cc.egui_ctx);
        let settings = Settings::load();
        apply_theme(&cc.egui_ctx, settings.dark);
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

    fn poll_catalog(&mut self) {
        let Ok(fresh) = self.rx.try_recv() else { return };
        self.loading = false;
        let offline = fresh.source.starts_with("Offline");
        if offline && !self.catalog.source.starts_with("Offline") && !self.catalog.apps.is_empty() {
            self.note = format!("Offline, showing cached catalog ({})", self.catalog.source);
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

    fn open_detail(&mut self, app: AppItem) {
        self.sel_stream = app.versions.iter().position(|v| v.stream == "Stable").unwrap_or(0);
        self.sel_variant = 0;
        self.detail = Some(app);
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
                        self.set_section(Section::Store);
                        self.detail = None;
                    }
                    "store-detail" => {
                        if let Some(a) = self.view.first().cloned() {
                            self.open_detail(a);
                        }
                    }
                    other => {
                        self.set_section(Section::Optimaxer);
                        self.opti.show_page(other.trim_start_matches("optimaxer-"));
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

    fn set_section(&mut self, s: Section) {
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
                    self.set_section(Section::Store);
                    self.settings.platform = PC.into();
                    self.category = c;
                    self.query.clear();
                    self.detail = None;
                    self.dirty = true;
                    self.settings.save();
                }
                OptiEvent::OpenApp(id) => {
                    if let Some(a) = self.catalog.apps.iter().find(|a| a.id == id).cloned() {
                        self.set_section(Section::Store);
                        self.open_detail(a);
                    }
                }
            }
        }
    }

    fn tick_toast(&mut self, ctx: &egui::Context) {
        if let Some((_, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(3) {
                self.toast = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
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
    let r = CornerRadius::from(rect.width() * 0.22);
    if app.icon.is_empty() {
        ui.painter().rect_filled(rect, r, BRAND.gamma_multiply(0.35));
        let letter = app.name.chars().next().unwrap_or('?').to_uppercase().to_string();
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, FontId::proportional(rect.width() * 0.45), Color32::WHITE);
    } else {
        egui::Image::new(app.icon.as_str()).fit_to_exact_size(rect.size()).corner_radius(r).paint_at(ui, rect);
    }
}

pub(crate) fn chip(ui: &mut Ui, text: &str, fill: Color32, fg: Color32) {
    egui::Frame::new().fill(fill).corner_radius(8).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(12.0).color(fg));
    });
}

fn app_card(ui: &mut Ui, app: &AppItem, w: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, CARD_H), Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let fill = if resp.hovered() { v.widgets.hovered.weak_bg_fill } else { v.faint_bg_color };
        let stroke = if resp.hovered() { Stroke::new(1.5, BRAND) } else { Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color) };
        ui.painter().rect_filled(rect, 14, fill);
        ui.painter().rect_stroke(rect, 14, stroke, StrokeKind::Inside);
        let icon_rect = egui::Rect::from_min_size(rect.min + vec2(14.0, 14.0), vec2(64.0, 64.0));
        paint_icon(ui, app, icon_rect);
        let text_rect = egui::Rect::from_min_max(rect.min + vec2(90.0, 10.0), rect.max - vec2(10.0, 8.0));
        let p = ui.painter().with_clip_rect(text_rect);
        let strong = ui.visuals().strong_text_color();
        let weak = ui.visuals().weak_text_color();
        p.text(text_rect.min + vec2(0.0, 4.0), egui::Align2::LEFT_TOP, &app.name, FontId::proportional(16.0), strong);
        p.text(text_rect.min + vec2(0.0, 26.0), egui::Align2::LEFT_TOP, &app.author, FontId::proportional(12.5), weak);
        let meta = format!("{}  ·  {}", app.version, if app.size.is_empty() { "Varies" } else { &app.size });
        p.text(text_rect.min + vec2(0.0, 46.0), egui::Align2::LEFT_TOP, meta, FontId::proportional(12.5), weak);
        p.text(text_rect.min + vec2(0.0, 66.0), egui::Align2::LEFT_TOP, &app.category, FontId::proportional(12.0), BRAND);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
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
            } else {
                self.detail = None;
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
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
        egui::CentralPanel::default().show(root, |ui| {
            if self.section == Section::Optimaxer {
                self.opti.central(ui, ctx, &self.suggestions);
            } else if self.detail.is_some() {
                self.detail_view(ctx, ui);
            } else {
                self.store_view(ui);
            }
        });
        if self.show_settings {
            self.settings_window(ctx);
        }
        if let Some((msg, _)) = &self.toast {
            egui::Area::new(egui::Id::unique("orion-toast")).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -48.0)).order(egui::Order::Tooltip).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(msg.as_str());
                });
            });
        }
    }
}

impl OrionApp {
    fn top_bar(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        egui::Panel::top("top").show(root, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Orion").size(22.0).strong().color(BRAND));
                ui.label(RichText::new("Store").size(22.0).strong());
                ui.add_space(12.0);
                for (s, label) in [(Section::Store, "Store"), (Section::Optimaxer, "Optimaxer")] {
                    if ui.selectable_label(self.section == s, RichText::new(label).size(15.0).strong()).clicked() && self.section != s {
                        self.set_section(s);
                    }
                }
                ui.add_space(12.0);
                if self.section == Section::Store {
                    let resp = ui.add_sized(
                        [(ui.available_width() - 260.0).clamp(160.0, 520.0), 30.0],
                        egui::TextEdit::singleline(&mut self.query).id(self.search_id).hint_text("Search apps, authors, tags   (Ctrl+F)"),
                    );
                    if resp.changed() {
                        self.dirty = true;
                        self.detail = None;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Settings").clicked() {
                        self.show_settings = !self.show_settings;
                    }
                    let n = self.downloads.iter().filter(|d| matches!(d.status(), DlStatus::Running)).count();
                    let label = if n > 0 { format!("Downloads ({n})") } else { "Downloads".into() };
                    if ui.selectable_label(self.show_downloads, label).clicked() {
                        self.show_downloads = !self.show_downloads;
                    }
                    if self.section == Section::Store && ui.add_enabled(!self.loading, egui::Button::new(if self.loading { "Refreshing..." } else { "Refresh" })).clicked() {
                        self.loading = true;
                        self.rx = spawn_refresh(ctx);
                    }
                });
            });
            ui.add_space(6.0);
        });
    }

    fn status_bar(&mut self, root: &mut Ui) {
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                let weak = ui.visuals().weak_text_color();
                if self.section == Section::Optimaxer {
                    ui.label(RichText::new(self.opti.status_text()).size(12.0).color(weak));
                    return;
                }
                ui.label(RichText::new(format!("{} apps  ·  {} shown  ·  source: {}", self.catalog.apps.len(), self.view.len(), self.catalog.source)).size(12.0).color(weak));
                if self.loading {
                    ui.add(egui::Spinner::new().size(12.0));
                }
                if !self.note.is_empty() {
                    ui.label(RichText::new(&self.note).size(12.0).color(Color32::from_rgb(0xF5, 0xA6, 0x23)));
                }
            });
        });
    }

    fn side_panel(&mut self, root: &mut Ui) {
        egui::Panel::left("side").resizable(false).exact_size(190.0).show(root, |ui| {
            if self.section == Section::Optimaxer {
                self.opti.side_panel(ui);
                return;
            }
            ui.add_space(8.0);
            ui.label(RichText::new("PLATFORM").size(11.0).color(ui.visuals().weak_text_color()));
            let mut changed = false;
            for p in PLATFORMS {
                let n = self.catalog.apps.iter().filter(|a| p == "All" || a.platform == p).count();
                let selected = self.settings.platform == p;
                if ui.add_sized([ui.available_width(), 30.0], egui::Button::selectable(selected, format!("{p}   ({n})"))).clicked() && !selected {
                    self.settings.platform = p.into();
                    self.category = "All".into();
                    changed = true;
                }
            }
            ui.add_space(10.0);
            ui.label(RichText::new("CATEGORY").size(11.0).color(ui.visuals().weak_text_color()));
            let cats = self.categories();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let all = cats.iter().map(|c| c.1).sum::<usize>();
                for (name, n) in std::iter::once(("All".to_string(), all)).chain(cats) {
                    let selected = self.category == name;
                    if ui.add_sized([ui.available_width(), 26.0], egui::Button::selectable(selected, format!("{name}   ({n})"))).clicked() && !selected {
                        self.category = name;
                        changed = true;
                    }
                }
            });
            if changed {
                self.dirty = true;
                self.detail = None;
                self.settings.save();
            }
        });
    }

    fn store_view(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let title = if self.query.trim().is_empty() { format!("{} apps", self.settings.platform) } else { format!("Results for \"{}\"", self.query.trim()) };
            ui.heading(title);
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
                ui.label("Sort");
            });
        });
        ui.add_space(4.0);
        if self.view.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Nothing here").size(20.0).strong());
                ui.label("No apps match this platform, category or search.");
            });
            return;
        }
        let avail = ui.available_width();
        let cols = (((avail + GAP) / (300.0 + GAP)).floor() as usize).max(1);
        let w = (avail - GAP * (cols as f32 - 1.0) - 12.0) / cols as f32;
        let rows = self.view.len().div_ceil(cols);
        let mut picked: Option<AppItem> = None;
        if !self.featured.is_empty() {
            ui.label(RichText::new("Featured today").strong().color(LIME));
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            ui.horizontal(|ui| {
                for app in self.featured.iter().take(cols) {
                    if app_card(ui, app, w) {
                        picked = Some(app.clone());
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("All apps").strong());
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
            self.open_detail(a);
        }
    }

    fn detail_view(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        let Some(app) = self.detail.clone() else { return };
        let mut download: Option<(String, String)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if ui.button("< Back").clicked() {
                self.detail = None;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                icon(ui, &app, 96.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&app.name).size(26.0).strong());
                    ui.label(RichText::new(format!("by {}", app.author)).color(ui.visuals().weak_text_color()));
                    ui.horizontal(|ui| {
                        chip(ui, &app.platform, BRAND.gamma_multiply(0.3), BRAND);
                        chip(ui, &app.category, LIME.gamma_multiply(0.25), ui.visuals().text_color());
                        chip(ui, &if app.version.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit()) { format!("v{}", app.version.trim_start_matches('v')) } else { app.version.clone() }, Color32::from_gray(70).gamma_multiply(0.4), ui.visuals().text_color());
                        if !app.size.is_empty() {
                            chip(ui, &app.size, Color32::from_gray(70).gamma_multiply(0.4), ui.visuals().text_color());
                        }
                    });
                });
            });
            ui.add_space(10.0);
            self.install_box(ui, &app, &mut download);
            if app.platform != PC {
                ui.label(RichText::new(format!("{} package. It will not install on Windows; downloading saves the file for transfer to a {} device.", app.platform, app.platform)).size(12.0).color(ui.visuals().weak_text_color()));
            }
            ui.add_space(8.0);
            if !app.description.is_empty() {
                ui.label(&app.description);
                ui.add_space(8.0);
            }
            if !app.patches.is_empty() {
                ui.label(RichText::new("Patches").strong());
                ui.horizontal_wrapped(|ui| {
                    for p in &app.patches {
                        chip(ui, p, BRAND.gamma_multiply(0.2), ui.visuals().text_color());
                    }
                });
            }
            if !app.tags.is_empty() {
                ui.label(RichText::new("Tags").strong());
                ui.horizontal_wrapped(|ui| {
                    for t in &app.tags {
                        chip(ui, t, Color32::from_gray(70).gamma_multiply(0.4), ui.visuals().text_color());
                    }
                });
            }
            if !app.screenshots.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Screenshots").strong());
                egui::ScrollArea::horizontal().id_salt("shots").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for s in &app.screenshots {
                            ui.add(egui::Image::new(s.as_str()).fit_to_exact_size(vec2(320.0, 190.0)).corner_radius(10));
                        }
                    });
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if let Some(u) = &app.repo_url {
                    ui.hyperlink_to("Source repository", u);
                }
                if let Some(u) = &app.official_site {
                    ui.hyperlink_to("Official site", u);
                }
                if let Some(p) = &app.package_name {
                    ui.label(RichText::new(p).monospace().size(12.0).color(ui.visuals().weak_text_color()));
                }
            });
        });
        if let Some((url, hint)) = download {
            self.start(ctx, &url, &hint);
        }
    }

    fn install_box(&mut self, ui: &mut Ui, app: &AppItem, download: &mut Option<(String, String)>) {
        egui::Frame::group(ui.style()).corner_radius(12).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let variants = if app.versions.is_empty() { &app.variants } else { &app.versions[self.sel_stream.min(app.versions.len() - 1)].variants };
            if app.versions.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label("Release channel");
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
                    ui.label("No direct download published yet.");
                    if let Some(r) = &app.repo_url {
                        if ui.button("Open releases page").clicked() {
                            let _ = open::that(format!("{}/releases", r.trim_end_matches('/')));
                        }
                    }
                } else if ui.add(egui::Button::new(RichText::new("Download").strong().color(Color32::BLACK)).fill(LIME).min_size(vec2(160.0, 34.0))).clicked() {
                    *download = Some((app.download_url.clone(), String::new()));
                }
                return;
            }
            let idx = self.sel_variant.min(variants.len() - 1);
            let label = |v: &model::AppVariant| format!("{}   {}   {}", v.arch, v.name, model::format_size(v.size));
            if variants.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label("Build");
                    egui::ComboBox::from_id_salt("variant").width(420.0).selected_text(label(&variants[idx])).show_ui(ui, |ui| {
                        for (i, v) in variants.iter().enumerate() {
                            ui.selectable_value(&mut self.sel_variant, i, label(v));
                        }
                    });
                });
            }
            let v = &variants[idx];
            if ui.add(egui::Button::new(RichText::new(format!("Download  ({})", model::format_size(v.size))).strong().color(Color32::BLACK)).fill(LIME).min_size(vec2(200.0, 34.0))).clicked() {
                *download = Some((v.url.clone(), v.name.clone()));
            }
        });
    }

    fn downloads_panel(&mut self, root: &mut Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        let mut remove: Option<usize> = None;
        let mut note: Option<String> = None;
        let mut run_installer: Option<PathBuf> = None;
        egui::Panel::bottom("downloads").resizable(true).default_size(160.0).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Downloads").strong());
                if ui.small_button("Open folder").clicked() {
                    let _ = std::fs::create_dir_all(&self.settings.download_dir);
                    let _ = open::that(&self.settings.download_dir);
                }
                if ui.small_button("Clear finished").clicked() {
                    self.downloads.retain(|d| matches!(d.status(), DlStatus::Running));
                }
            });
            if self.downloads.is_empty() {
                ui.label(RichText::new("Nothing downloaded yet.").color(ui.visuals().weak_text_color()));
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, d) in self.downloads.iter().enumerate().rev() {
                    ui.horizontal(|ui| {
                        ui.add_sized([220.0, 18.0], egui::Label::new(RichText::new(&d.file).strong()).truncate());
                        match d.status() {
                            DlStatus::Running => {
                                let bar = match d.fraction() {
                                    Some(f) => egui::ProgressBar::new(f).text(format!("{:.0}%  {} / {}", f * 100.0, model::format_size(Some(d.done.load(std::sync::atomic::Ordering::Relaxed))), model::format_size(Some(d.total.load(std::sync::atomic::Ordering::Relaxed))))),
                                    None => egui::ProgressBar::new(0.0).animate(true).text(model::format_size(Some(d.done.load(std::sync::atomic::Ordering::Relaxed)))),
                                };
                                ui.add(bar.desired_width(260.0));
                                if ui.small_button("Cancel").clicked() {
                                    d.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            }
                            DlStatus::Done(path) => {
                                ui.label(RichText::new("Done").color(LIME));
                                if ui.small_button("Run / Open").clicked() {
                                    if open::that(&path).is_err() {
                                        note = Some("Could not open the file".into());
                                    }
                                }
                                if ui.small_button("Show in folder").clicked() {
                                    reveal(&path);
                                }
                                if is_installer(&path) && ui.small_button("Restore point + run").on_hover_text("Optimaxer creates a System Restore point, then opens the installer").clicked() {
                                    run_installer = Some(path.clone());
                                }
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(i);
                                }
                            }
                            DlStatus::Failed(e) => {
                                ui.label(RichText::new(format!("Failed: {e}")).color(Color32::from_rgb(0xFF, 0x6B, 0x6B)));
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(i);
                                }
                            }
                            DlStatus::Cancelled => {
                                ui.label("Cancelled");
                                if ui.small_button("Remove").clicked() {
                                    remove = Some(i);
                                }
                            }
                        }
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
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Settings").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            let mut dark = self.settings.dark;
            if ui.checkbox(&mut dark, "Dark theme").changed() {
                self.settings.dark = dark;
                apply_theme(ctx, dark);
                self.settings.save();
            }
            ui.add_space(6.0);
            ui.label("Download folder");
            let resp = ui.add(egui::TextEdit::singleline(&mut self.settings.download_dir).desired_width(380.0));
            if resp.lost_focus() {
                self.settings.save();
            }
            ui.horizontal(|ui| {
                if ui.button("Open folder").clicked() {
                    let _ = std::fs::create_dir_all(&self.settings.download_dir);
                    let _ = open::that(&self.settings.download_dir);
                }
                if ui.button("Reset").clicked() {
                    self.settings.download_dir = Settings::default().download_dir;
                    self.settings.save();
                }
            });
            ui.add_space(6.0);
            let mut rp = self.settings.opti_restore_point;
            if ui.checkbox(&mut rp, "Optimaxer: offer a System Restore point before applying tweaks").changed() {
                self.settings.opti_restore_point = rp;
                self.settings.save();
            }
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Cache: {}", net::data_dir().display())).size(12.0).color(ui.visuals().weak_text_color()));
        });
        if !open {
            self.show_settings = false;
        }
    }
}
