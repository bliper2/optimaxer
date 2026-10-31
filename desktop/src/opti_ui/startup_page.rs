//! Startup page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn startup_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        if !self.startup_loaded && self.busy.is_none() {
            self.refresh_startup(ctx);
        }
        kit::page_header(
            ui,
            "Startup",
            "Apps and third-party tasks that start with Windows. Turning one off changes its startup state; nothing is uninstalled.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Last boot")
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.text),
            );
            if self.startup_boot_loading {
                ui.label(
                    RichText::new("Reading Windows startup record…")
                        .size(T_SMALL)
                        .color(t.subtle),
                );
            } else if let Some((duration_ms, event_time)) = &self.startup_boot_record {
                ui.label(
                    RichText::new(format!("{:.1} s", *duration_ms as f64 / 1000.0))
                        .font(FontId::new(T_BODY, semibold()))
                        .color(t.text),
                );
                ui.label(
                    RichText::new(format!("Windows Event 100 · {event_time}"))
                        .size(T_CAPTION)
                        .color(t.muted),
                );
            } else {
                ui.label(
                    RichText::new("No recent Windows boot record")
                        .size(T_SMALL)
                        .color(t.subtle),
                );
            }
        });
        ui.label(
            RichText::new("Per-app impact is not inferred from startup commands; Windows must measure launch CPU and disk use.")
                .size(T_CAPTION)
                .color(t.subtle),
        );
        ui.add_space(S2);
        let idle = self.busy.is_none();
        let picked: Vec<startup::Item> = self
            .startup
            .iter()
            .filter(|i| self.st_sel.contains(&startup_item_key(i)))
            .cloned()
            .collect();
        ui.horizontal_wrapped(|ui| {
            for (label, enable) in [("Turn off selected", false), ("Turn on selected", true)] {
                if kit::secondary(ui, label, idle && !picked.is_empty()).clicked() && self.need_admin() {
                    let n = picked.len();
                    self.pending = Some(Pending {
                        title: label.into(),
                        intro: format!("{} {n} startup item{}? Machine-wide entries need administrator rights.", if enable { "Turn on" } else { "Turn off" }, if n == 1 { "" } else { "s" }),
                        lines: picked.iter().map(|i| Line { text: format!("{}  ({})", i.name, i.scope), warn: false, head: false }).collect(),
                        action: Action::SetStartup { items: picked.clone(), enable },
                        restore_option: false,
                        ok_label: format!("{label} ({n})"),
                    });
                }
            }
            if kit::ghost(ui, "Refresh", idle).clicked() {
                self.refresh_startup(ctx);
            }
            if !picked.is_empty() {
                ui.label(RichText::new(format!("{} selected", picked.len())).size(T_SMALL).color(t.muted));
            }
        });
        ui.add_space(S2);
        if let Some(warning) = &self.startup_warning {
            ui.label(RichText::new(warning).size(T_SMALL).color(t.warn));
            ui.add_space(S2);
        }
        if self.startup.is_empty() {
            if self.busy.is_some() {
                kit::skeleton(ui, 5);
            } else {
                kit::empty_state(
                    ui,
                    "No startup items found",
                    "Nothing in Run keys, Startup folders, or third-party logon and boot tasks. Press Refresh to look again.",
                );
            }
            return;
        }
        let mut clear_inspection = false;
        if let Some(item) = &self.startup_inspection {
            kit::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&item.name)
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    kit::chip(ui, item.scope, Tone::Neutral);
                    if kit::ghost(ui, "Close details", true).clicked() {
                        clear_inspection = true;
                    }
                });
                ui.label(
                    RichText::new("Launch command")
                        .size(T_CAPTION)
                        .color(t.muted),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(&item.cmd)
                            .monospace()
                            .size(T_CAPTION)
                            .color(t.subtle),
                    )
                    .wrap(),
                );
                match self.startup_inspection_state.as_ref() {
                    Some(StartupInspectionState::Loading(path)) => {
                        ui.label(
                            RichText::new("Reading executable metadata…")
                                .size(T_SMALL)
                                .color(t.subtle),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(path.to_string_lossy())
                                    .monospace()
                                    .size(T_CAPTION)
                                    .color(t.text),
                            )
                            .wrap(),
                        );
                    }
                    Some(StartupInspectionState::Unavailable) | None => {
                        ui.label(
                            RichText::new("Executable location not resolved. Shortcut targets, scripts, relative commands, and ambiguous launchers are not guessed.")
                                .size(T_SMALL)
                                .color(t.subtle),
                        );
                    }
                    Some(StartupInspectionState::Ready { path, company_name }) => {
                        ui.label(
                            RichText::new("Executable location")
                                .size(T_CAPTION)
                                .color(t.muted),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(path.to_string_lossy())
                                    .monospace()
                                    .size(T_CAPTION)
                                    .color(t.text),
                            )
                            .wrap(),
                        );
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("Embedded CompanyName")
                                    .size(T_CAPTION)
                                    .color(t.muted),
                            );
                            match company_name {
                                Some(name) => {
                                    ui.label(RichText::new(name).size(T_SMALL).color(t.text))
                                }
                                None => ui.label(
                                    RichText::new("Not present in this executable")
                                        .size(T_SMALL)
                                        .color(t.subtle),
                                ),
                            };
                        });
                        ui.label(
                            RichText::new("This file metadata is not signature verification and may identify a launcher rather than the application.")
                                .size(T_CAPTION)
                                .color(t.subtle),
                        );
                    }
                    Some(StartupInspectionState::Error { path, message }) => {
                        if let Some(path) = path {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(path.to_string_lossy())
                                        .monospace()
                                        .size(T_CAPTION)
                                        .color(t.text),
                                )
                                .wrap(),
                            );
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!(
                                    "Executable metadata unavailable: {message}"
                                ))
                                .size(T_SMALL)
                                .color(t.warn),
                            )
                            .wrap(),
                        );
                    }
                }
            });
            ui.add_space(S2);
        }
        if clear_inspection {
            self.startup_inspection = None;
            self.startup_inspection_state = None;
        }
        let mut inspect = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in &self.startup {
                    let k = startup_item_key(i);
                    let on = self.st_sel.contains(&k);
                    let mut inspect_clicked = false;
                    let mut details_rect = None;
                    let r = kit::row(ui, ("st", &k), on, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            kit::check(ui, on);
                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(&i.name)
                                            .font(FontId::new(T_BODY, semibold()))
                                            .color(t.text),
                                    );
                                    kit::chip(
                                        ui,
                                        if i.enabled { "On" } else { "Off" },
                                        if i.enabled { Tone::Good } else { Tone::Neutral },
                                    );
                                    kit::chip(ui, i.scope, Tone::Neutral);
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&i.cmd)
                                            .monospace()
                                            .size(T_CAPTION)
                                            .color(t.subtle),
                                    )
                                    .truncate(),
                                );
                            });
                            let details = kit::ghost(ui, "Details", true);
                            details_rect = Some(details.rect);
                            if details.clicked() {
                                inspect = Some(i.clone());
                                inspect_clicked = true;
                            }
                        });
                    });
                    let click = ui.interact(
                        r.response.rect,
                        ui.scope_id().with(("st-click", &k)),
                        Sense::click(),
                    );
                    if click.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if click.clicked() {
                        let clicked_details = click
                            .interact_pointer_pos()
                            .is_some_and(|pos| details_rect.is_some_and(|rect| rect.contains(pos)));
                        if clicked_details {
                            inspect = Some(i.clone());
                        } else if !inspect_clicked {
                            if on {
                                self.st_sel.remove(&k);
                            } else {
                                self.st_sel.insert(k);
                            }
                        }
                    }
                }
            });
        if let Some(item) = inspect {
            self.inspect_startup(ctx, item);
        }
    }
}
