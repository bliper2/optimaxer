//! Activity page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn activity_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Activity",
            "Search confirmed tweak changes and system-action results. Recorded warnings and errors describe operation output, not final Windows state.",
        );
        if let Some(job) = self.busy.clone() {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(t.brand_text));
                    kit::chip(ui, "Running", Tone::Brand);
                    ui.label(
                        RichText::new(job)
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    if kit::secondary(ui, "Show live output", true).clicked() {
                        self.log_expanded = true;
                    }
                });
                ui.label(
                    RichText::new("Live output is available in the Operation center.")
                        .size(T_CAPTION)
                        .color(t.subtle),
                );
            });
            ui.add_space(S2);
        }
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [280.0, 34.0],
                egui::TextEdit::singleline(&mut self.history_search)
                    .hint_text("Search action, tweak, output or date"),
            );
            let visible = self
                .history
                .iter()
                .filter(|entry| history_entry_matches(entry, &self.history_search))
                .count()
                + self
                    .operations
                    .iter()
                    .filter(|entry| operation_entry_matches(entry, &self.history_search))
                    .count();
            ui.label(
                RichText::new(format!("{visible} matching records"))
                    .size(T_SMALL)
                    .color(t.muted),
            );
        });
        ui.add_space(S2);
        if let Some(error) = &self.history_error {
            ui.label(
                RichText::new(format!("Tweak history unavailable: {error}"))
                    .size(T_SMALL)
                    .color(t.warn),
            );
        }
        if let Some(error) = &self.operations_error {
            ui.label(
                RichText::new(format!("System action history unavailable: {error}"))
                    .size(T_SMALL)
                    .color(t.warn),
            );
        }
        if self.history.is_empty() && self.operations.is_empty() {
            if self.history_error.is_none() && self.operations_error.is_none() {
                kit::empty_state(
                    ui,
                    "No recorded activity",
                    "Confirmed tweak changes and system-action output will appear here.",
                );
            }
            return;
        }
        let entries = activity_entries(&self.history, &self.operations, &self.history_search);
        if entries.is_empty() {
            kit::empty_state(
                ui,
                "No matching activity",
                "Try another search or clear the filter.",
            );
            return;
        }
        for (index, item) in entries.iter().enumerate() {
            match item {
                ActivityEntry::Tweak(entry) => {
                    let verified = entry
                        .changes
                        .iter()
                        .filter(|change| change.verified)
                        .count();
                    let undoable = entry
                        .changes
                        .iter()
                        .filter(|change| change.undo_available)
                        .count();
                    let action = match entry.action {
                        HistoryAction::Apply => "Applied",
                        HistoryAction::Undo => "Undid",
                    };
                    egui::CollapsingHeader::new(format!(
                        "{action} {} tweaks · {verified}/{} verified · {undoable} undoable",
                        entry.changes.len(),
                        entry.changes.len()
                    ))
                    .id_salt(("activity-tweak", index, &entry.time))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&entry.time).size(T_CAPTION).color(t.subtle));
                        for change in &entry.changes {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&change.name).size(T_SMALL).color(t.text));
                                kit::chip(
                                    ui,
                                    if change.verified {
                                        "Verified"
                                    } else {
                                        "Incomplete"
                                    },
                                    if change.verified {
                                        Tone::Good
                                    } else {
                                        Tone::Warn
                                    },
                                );
                                if change.undo_available {
                                    kit::chip(ui, "Undo available", Tone::Neutral);
                                }
                            });
                            for detail in &change.details {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(detail).size(T_CAPTION).color(t.muted),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                    });
                }
                ActivityEntry::Operation(entry) => {
                    egui::CollapsingHeader::new(format!("{} · {} · {} output line(s)", entry.title, entry.status.label(), entry.details.len()))
                        .id_salt(("activity-operation", index, &entry.time))
                        .show(ui, |ui| {
                            ui.label(RichText::new(&entry.time).size(T_CAPTION).color(t.subtle));
                            let (summary, tone) = match entry.status {
                                OperationStatus::Finished => ("The worker finished. Final system state is not verified by this record.", Tone::Neutral),
                                OperationStatus::Warnings => ("The worker finished with warnings. Review the captured output.", Tone::Warn),
                                OperationStatus::Error => ("The worker reported an error. Review the captured output.", Tone::Bad),
                                OperationStatus::Unknown => ("This record has no status supported by this version.", Tone::Neutral),
                            };
                            kit::chip(ui, entry.status.label(), tone);
                            ui.label(RichText::new(summary).size(T_CAPTION).color(t.muted));
                            if entry.details.is_empty() {
                                ui.label(RichText::new("No output was captured.")
                                    .size(T_CAPTION).color(t.warn));
                            } else {
                                for detail in &entry.details {
                                    let line = detail.trim_start();
                                    let color = if line.starts_with("ERROR") {
                                        t.bad
                                    } else if line.starts_with("WARN") || line.contains("failed") {
                                        t.warn
                                    } else {
                                        t.muted
                                    };
                                    ui.add(egui::Label::new(RichText::new(detail).monospace().size(T_CAPTION).color(color)).wrap());
                                }
                            }
                        });
                }
            }
        }
    }
}
