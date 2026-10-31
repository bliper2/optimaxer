//! Settings window and appearance import/export.

use super::*;

impl OrionApp {
    pub(super) fn save_settings(&mut self, label: &str) {
        if let Err(error) = self.settings.save_checked() {
            self.toast = Some((format!("{label} was not saved: {error}"), Instant::now()));
        }
    }

    pub(super) fn persist_appearance(
        &mut self,
        ctx: &egui::Context,
        previous: net::Settings,
        label: &str,
    ) {
        if let Err(error) = self.settings.save_checked() {
            self.settings = previous;
            self.reapply_theme(ctx);
            self.toast = Some((format!("{label} was not saved: {error}"), Instant::now()));
        }
    }

    pub(super) fn custom_surface_editor(&mut self, ui: &mut Ui, ctx: &egui::Context) {
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

    pub(super) fn settings_modal(&mut self, ctx: &egui::Context) {
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
                    if update::IN_APP {
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
                        } else {
                        ui.add(egui::Label::new(RichText::new(format!("Version v{}. In-app updating replaces the Windows program file, so it is off here. Download newer builds from the releases page.", update::CURRENT)).size(T_SMALL).color(t.muted)).wrap());
                    }
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

    pub(super) fn export_appearance(&mut self) {
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

    pub(super) fn import_appearance(&mut self, ctx: &egui::Context) {
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
