//! Cleaner page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn cleaner_page(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        settings: &mut Settings,
    ) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Cleaner",
            "Scan first, then clean what you tick. Directory links are never followed, and deleted files do not go to the Recycle Bin.",
        );
        let idle = self.busy.is_none();
        ui.label(
            RichText::new("Path exclusions")
                .font(FontId::new(T_BODY, semibold()))
                .color(t.text),
        );
        ui.label(
            RichText::new("Exact files and folders, including their contents, stay untouched by path-based cleanup.")
                .size(T_SMALL)
                .color(t.muted),
        );
        ui.label(
            RichText::new("Event logs and Windows.old are skipped if exclusions overlap their storage. Recycle Bin and component-store cleanup cannot use path exclusions.")
                .size(T_SMALL)
                .color(t.muted),
        );
        ui.horizontal(|ui| {
            let input_width = (ui.available_width() - 150.0).max(120.0);
            ui.add_enabled_ui(idle, |ui| {
                ui.add_sized(
                    [input_width, 26.0],
                    egui::TextEdit::singleline(&mut self.clean_exclusion_input)
                        .hint_text(r"C:\path\to\protect"),
                );
            });
            if kit::secondary(
                ui,
                "Add exclusion",
                idle && !self.clean_exclusion_input.trim().is_empty(),
            )
            .clicked()
            {
                self.add_cleaner_exclusion(settings);
            }
        });
        if let Some(error) = &self.clean_exclusion_error {
            ui.label(RichText::new(error).size(T_SMALL).color(t.bad));
        }
        let mut remove_exclusion = None;
        if settings.cleaner_exclusions.is_empty() {
            ui.label(
                RichText::new("No paths excluded.")
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
        } else {
            egui::ScrollArea::vertical()
                .max_height(96.0)
                .show(ui, |ui| {
                    for (index, path) in settings.cleaner_exclusions.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(path)
                                        .monospace()
                                        .size(T_CAPTION)
                                        .color(t.text),
                                )
                                .truncate(),
                            )
                            .on_hover_text(path);
                            if kit::secondary(ui, "Remove", idle).clicked() {
                                remove_exclusion = Some(index);
                            }
                        });
                    }
                });
        }
        if let Some(index) = remove_exclusion {
            let mut exclusions = settings.cleaner_exclusions.clone();
            exclusions.remove(index);
            self.save_cleaner_exclusions(settings, exclusions);
        }
        ui.add_space(S2);
        let total: u64 = self.clean_sizes.values().flatten().sum();
        let picked: Vec<&'static str> = cleaner::TARGETS
            .iter()
            .map(|t| t.id)
            .filter(|id| self.clean_sel.contains(id))
            .collect();
        let preview_id = if picked.len() == 1 {
            picked.first().copied()
        } else {
            None
        };
        ui.horizontal_wrapped(|ui| {
            if kit::secondary(ui, "Scan", idle).clicked() {
                self.scan_cleaner(ctx, settings.cleaner_exclusions.clone());
            }
            let label = if picked.is_empty() {
                "Clean selected".to_string()
            } else {
                format!(
                    "Clean {} item{}",
                    picked.len(),
                    if picked.len() == 1 { "" } else { "s" }
                )
            };
            if kit::primary(ui, &label, idle && !picked.is_empty()).clicked() && self.need_admin() {
                let lines = picked
                    .iter()
                    .filter_map(|id| cleaner::TARGETS.iter().find(|target| target.id == *id))
                    .map(|target| {
                        let blocked = cleaner::blocked_by_exclusions(
                            target,
                            &settings.cleaner_exclusions,
                        );
                        Line {
                            text: format!(
                                "{}   {}{}",
                                target.name,
                                self.clean_sizes
                                    .get(target.id)
                                    .copied()
                                    .flatten()
                                    .map(|size| format_size(Some(size)))
                                    .unwrap_or_default(),
                                if blocked {
                                    " · skipped by path exclusions"
                                } else {
                                    ""
                                }
                            ),
                            warn: target.risk != "Safe" || blocked,
                            head: false,
                        }
                    })
                    .collect();
                self.pending = Some(Pending {
                    title: "Clean".into(),
                    intro: "Delete the selected items? This cannot be undone. Exclusions can skip Windows.old or event-log cleanup when they overlap those targets.".into(),
                    lines,
                    action: Action::Clean(picked.clone(), settings.cleaner_exclusions.clone()),
                    restore_option: false,
                    ok_label: format!(
                        "Delete {} item{}",
                        picked.len(),
                        if picked.len() == 1 { "" } else { "s" }
                    ),
                });
            }
            if kit::secondary(ui, "Preview paths", idle && preview_id.is_some()).clicked() {
                self.preview_cleaner(ctx, preview_id.unwrap(), settings.cleaner_exclusions.clone());
            }
            if total > 0 {
                ui.label(
                    RichText::new(format!("{} can be reclaimed", format_size(Some(total))))
                        .font(FontId::new(T_BODY, semibold()))
                        .color(t.good),
                );
            } else if self.clean_sizes.is_empty() {
                ui.label(
                    RichText::new(if self.clean_scan_cancelled {
                        "Scan cancelled. Partial results were discarded."
                    } else {
                        "Scan to see how much each item can free."
                    })
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
        });
        if let Some((id, preview)) = &self.clean_preview
            && let Some(target) = cleaner::TARGETS.iter().find(|target| target.id == *id)
        {
            ui.add_space(S2);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Path preview · {}", target.name))
                            .font(FontId::new(T_BODY, semibold())),
                    );
                    if preview.truncated {
                        kit::chip(ui, "List truncated", Tone::Warn);
                    }
                    if preview.blocked_by_exclusions {
                        kit::chip(ui, "Target skipped", Tone::Warn);
                    }
                });
                ui.add(
                    egui::Label::new(RichText::new(preview.summary).size(T_SMALL).color(t.muted))
                        .wrap(),
                );
                if preview.unreadable_directories > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} directories could not be read; preview may be incomplete.",
                            preview.unreadable_directories
                        ))
                        .size(T_SMALL)
                        .color(t.warn),
                    );
                }
                if preview.files.is_empty() {
                    ui.label(
                        RichText::new("No accessible file paths found for this target.")
                            .size(T_SMALL)
                            .color(t.subtle),
                    );
                } else {
                    ui.label(
                        RichText::new(format!(
                            "{} files listed · size can change before confirmation",
                            preview.files.len()
                        ))
                        .size(T_CAPTION)
                        .color(t.subtle),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for file in &preview.files {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&file.path)
                                                .monospace()
                                                .size(T_CAPTION)
                                                .color(t.text),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&file.path);
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(format_size(Some(file.bytes)))
                                                .size(T_CAPTION)
                                                .color(t.muted),
                                        );
                                    });
                                });
                            }
                        });
                }
            });
        }
        if let Some((id, error)) = &self.clean_preview_error
            && cleaner::TARGETS.iter().any(|target| target.id == *id)
        {
            ui.label(
                RichText::new(format!("Path preview failed: {error}"))
                    .size(T_SMALL)
                    .color(t.bad),
            );
        }
        ui.add_space(S2);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for category in cleaner::Category::ALL {
                    let targets: Vec<_> = cleaner::TARGETS
                        .iter()
                        .filter(|target| cleaner::category(target) == category)
                        .collect();
                    if targets.is_empty() {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(category.label())
                                .font(FontId::new(T_SMALL, semibold()))
                                .color(t.subtle),
                        );
                        ui.label(
                            RichText::new(targets.len().to_string())
                                .size(T_CAPTION)
                                .color(t.muted),
                        );
                    });
                    for target in targets {
                        self.cleaner_row(ui, ctx, target, &settings.cleaner_exclusions);
                    }
                }
            });
    }

    pub(super) fn cleaner_row(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        tg: &Target,
        exclusions: &[String],
    ) {
        let t = tokens(ctx);
        let on = self.clean_sel.contains(tg.id);
        let blocked = cleaner::blocked_by_exclusions(tg, exclusions);
        let r = kit::row(ui, ("cl", tg.id), on, |ui| {
            ui.horizontal(|ui| {
                kit::check(ui, on);
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(tg.name)
                                .font(FontId::new(T_BODY, semibold()))
                                .color(t.text),
                        );
                        if tg.risk != "Safe" {
                            kit::chip(ui, tg.risk, risk_tone(tg.risk));
                        }
                        if blocked {
                            kit::chip(ui, "Skipped by exclusion", Tone::Warn);
                        }
                    });
                    ui.add(
                        egui::Label::new(RichText::new(tg.desc).size(T_SMALL).color(t.muted))
                            .wrap(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let text = if blocked {
                        "Skipped".to_string()
                    } else {
                        match self.clean_sizes.get(tg.id) {
                            Some(Some(0)) => "Empty".to_string(),
                            Some(Some(s)) => format_size(Some(*s)),
                            Some(None) => "Sized when cleaned".to_string(),
                            None => String::new(),
                        }
                    };
                    ui.label(
                        RichText::new(text)
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.muted),
                    );
                });
            });
        });
        let click = ui.interact(
            r.response.rect,
            ui.scope_id().with(("cl-click", tg.id)),
            Sense::click(),
        );
        if click.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if click.clicked() {
            if on {
                self.clean_sel.remove(tg.id);
            } else {
                self.clean_sel.insert(tg.id);
            }
        }
    }
}
