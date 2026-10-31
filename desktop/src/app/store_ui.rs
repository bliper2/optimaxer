//! Orion Store grid, detail view and downloads panel.

use super::*;

impl OrionApp {
    pub(super) fn store_view(&mut self, ctx: &egui::Context, ui: &mut Ui) {
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

    pub(super) fn detail_view(&mut self, ctx: &egui::Context, ui: &mut Ui) {
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

    pub(super) fn install_box(
        &mut self,
        ui: &mut Ui,
        app: &AppItem,
        download: &mut Option<(String, String)>,
    ) {
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

    pub(super) fn downloads_panel(&mut self, root: &mut Ui) {
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
}
