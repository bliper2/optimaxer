//! Activity: tweaks Optimaxer applied (with undo) and the history of confirmed operations.

use super::*;

impl OptiUi {
    pub(super) fn activity_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Activity",
            "What Optimaxer changed on this machine. Originals are kept in your data folder, so every change below can be undone.",
        );
        let mut undo: Option<&'static str> = None;
        let applied: Vec<(&'static str, &'static str, String)> = self
            .tweaks
            .iter()
            .filter_map(|tw| {
                self.state
                    .tweaks
                    .get(tw.id)
                    .map(|r| (tw.id, tw.name, r.time.clone()))
            })
            .collect();
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            Self::heading(ui, "Applied tweaks");
            if applied.is_empty() {
                ui.label(
                    RichText::new("No tweaks applied through Optimaxer.")
                        .size(T_SMALL)
                        .color(t.subtle),
                );
            }
            for (id, name, time) in &applied {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(*name).color(t.text));
                    ui.label(RichText::new(time).size(T_CAPTION).color(t.subtle));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::secondary(ui, "Undo", self.busy.is_none()).clicked() {
                            undo = Some(*id);
                        }
                    });
                });
            }
            let others = self.state.services.len()
                + self.state.user_units.len()
                + self.state.dns.len()
                + self.state.autostart.len();
            if others > 0 {
                ui.label(RichText::new(format!("{others} other saved original value(s): services, startup items or DNS. Restore them on their own pages.")).size(T_CAPTION).color(t.subtle));
            }
        });
        ui.add_space(S2);
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [260.0, 32.0],
                egui::TextEdit::singleline(&mut self.activity_search).hint_text("Search history"),
            );
            ui.label(
                RichText::new(format!("{} operations", self.operations.len()))
                    .size(T_SMALL)
                    .color(t.muted),
            );
        });
        ui.add_space(S2);
        let query = self.activity_search.trim().to_lowercase();
        let mut shown = 0;
        for entry in self.operations.iter().rev() {
            if !query.is_empty()
                && !entry.title.to_lowercase().contains(&query)
                && !entry
                    .details
                    .iter()
                    .any(|d| d.to_lowercase().contains(&query))
            {
                continue;
            }
            shown += 1;
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let tone = match entry.status {
                        OperationStatus::Finished => Tone::Good,
                        OperationStatus::Warnings => Tone::Warn,
                        OperationStatus::Error => Tone::Bad,
                        OperationStatus::Unknown => Tone::Neutral,
                    };
                    Self::badge(ui, entry.status.label(), tone);
                    ui.label(RichText::new(&entry.title).color(t.text));
                    ui.label(RichText::new(&entry.time).size(T_CAPTION).color(t.subtle));
                });
                if !entry.details.is_empty() {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("{} lines of output", entry.details.len()))
                            .size(T_CAPTION)
                            .color(t.subtle),
                    )
                    .id_salt(("linux-op", &entry.time, &entry.title))
                    .show(ui, |ui| {
                        for d in &entry.details {
                            ui.label(RichText::new(d).monospace().size(T_CAPTION).color(t.muted));
                        }
                    });
                }
            });
            ui.add_space(S1);
            if shown >= 100 {
                break;
            }
        }
        if shown == 0 {
            kit::empty_state(
                ui,
                "No matching activity",
                "Changes you confirm are listed here with their output.",
            );
        }
        if let Some(id) = undo {
            self.undo_tweaks(vec![id]);
        }
    }
}
