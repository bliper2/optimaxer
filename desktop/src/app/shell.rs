//! Window chrome: top bar, side panel, status bar, toast and lightbox.

use super::*;

impl OrionApp {
    pub(super) fn toast_view(&self, ctx: &egui::Context) {
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
    pub(super) fn lightbox_view(&mut self, ctx: &egui::Context) {
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

    pub(super) fn top_bar(&mut self, root: &mut Ui) {
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

    pub(super) fn status_bar(&mut self, root: &mut Ui) {
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

    pub(super) fn side_panel(&mut self, root: &mut Ui) {
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
                if cfg!(windows) {
                    side_title(ui, "Software");
                    if kit::nav_item(ui, false, "WinGet apps", None).clicked() {
                        self.show_winget = true;
                        self.detail = None;
                        self.bump(ctx);
                    }
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
}
