//! Tweaks: sysctl, config-file and systemd changes. Select, preview the exact changes, confirm; originals are saved
//! first and every applied tweak can be undone.

use super::*;
use crate::linux::tweaks::Risk;

impl OptiUi {
    pub(super) fn tweaks_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Tweaks",
            "Kernel settings, config drop-ins and services. Nothing runs until you have read the preview and confirmed. Tweaks this system cannot use are hidden.",
        );
        ui.horizontal_wrapped(|ui| {
            let search = ui.add_sized(
                [260.0, 34.0],
                egui::TextEdit::singleline(&mut self.tw_search)
                    .id_salt("optimaxer-tweak-search")
                    .hint_text("Search tweaks")
                    .margin(Margin::symmetric(S3 as i8, 8)),
            );
            if std::mem::take(&mut self.focus_tweak_search) {
                search.request_focus();
            }
            ui.label(RichText::new("Select:").size(T_SMALL).color(t.subtle));
            for (label, preset) in [
                ("Safe", "safe"),
                ("Performance", "perf"),
                ("Privacy", "privacy"),
                ("Everything", "max"),
            ] {
                if kit::ghost(ui, label, self.tweaks_loaded).clicked() {
                    let ids = tweaks::preset_ids(&self.tweaks, preset, |id| {
                        self.tweak_available.contains(id)
                    });
                    self.selected = ids.into_iter().collect();
                }
            }
        });
        ui.add_space(S2);
        let query = self.tw_search.trim().to_lowercase();
        let mut toggled: Option<&'static str> = None;
        let mut categories: Vec<&'static str> = Vec::new();
        for tw in &self.tweaks {
            if !categories.contains(&tw.cat) {
                categories.push(tw.cat);
            }
        }
        let mut shown = 0;
        for cat in categories {
            let items: Vec<&Tweak> = self
                .tweaks
                .iter()
                .filter(|tw| tw.cat == cat && self.tweak_available.contains(tw.id))
                .filter(|tw| {
                    query.is_empty()
                        || tw.name.to_lowercase().contains(&query)
                        || tw.desc.to_lowercase().contains(&query)
                        || tw.id.contains(&query)
                })
                .collect();
            if items.is_empty() {
                continue;
            }
            kit::group_title(ui, cat);
            for tw in items {
                shown += 1;
                let picked = self.selected.contains(tw.id);
                let active = self.tweak_status.get(tw.id).copied().unwrap_or(false);
                let saved = self.state.tweaks.contains_key(tw.id);
                let row = kit::row(ui, ("linux-tweak", tw.id), picked, |ui| {
                    ui.horizontal(|ui| {
                        kit::check(ui, picked);
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(tw.name)
                                        .font(FontId::new(T_BODY, semibold()))
                                        .color(t.text),
                                );
                                if tw.risk == Risk::Moderate {
                                    Self::badge(ui, "Moderate risk", Tone::Warn);
                                }
                                if active {
                                    Self::badge(ui, "Active", Tone::Good);
                                }
                                if saved {
                                    Self::badge(ui, "Can be undone", Tone::Neutral);
                                }
                            });
                            ui.add(
                                egui::Label::new(
                                    RichText::new(tw.desc).size(T_SMALL).color(t.muted),
                                )
                                .wrap(),
                            );
                        });
                    });
                });
                if row.response.interact(egui::Sense::click()).clicked() {
                    toggled = Some(tw.id);
                }
            }
        }
        if let Some(id) = toggled
            && !self.selected.remove(id)
        {
            self.selected.insert(id);
        }
        if !self.tweaks_loaded || (shown == 0 && self.tweak_available.is_empty()) {
            kit::skeleton(ui, 4);
        } else if shown == 0 {
            kit::empty_state(ui, "No matching tweaks", "Try a different search.");
        }
        let hidden = self.tweaks.len().saturating_sub(self.tweak_available.len());
        if self.tweaks_loaded && !self.tweak_available.is_empty() && hidden > 0 {
            ui.add_space(S3);
            ui.label(RichText::new(format!("{hidden} more tweaks are hidden because this system lacks something they need (a package manager, NetworkManager or a systemd unit).")).size(T_CAPTION).color(t.subtle));
        }
    }

    pub(super) fn tweak_action_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        let to_apply: Vec<&'static str> = self
            .selected_in_order()
            .into_iter()
            .filter(|id| !self.tweak_status.get(id).copied().unwrap_or(false))
            .collect();
        let to_undo: Vec<&'static str> = self
            .selected_in_order()
            .into_iter()
            .filter(|id| self.state.tweaks.contains_key(*id))
            .collect();
        ui.add_space(S1);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!("{} selected", self.selected.len()))
                    .size(T_SMALL)
                    .color(t.muted),
            );
            let idle = self.busy.is_none();
            if kit::primary(
                ui,
                &format!("Preview and apply ({})", to_apply.len()),
                idle && !to_apply.is_empty(),
            )
            .clicked()
            {
                self.apply_tweaks(to_apply.clone());
            }
            if kit::secondary(
                ui,
                &format!("Undo ({})", to_undo.len()),
                idle && !to_undo.is_empty(),
            )
            .clicked()
            {
                self.undo_tweaks(to_undo.clone());
            }
            if kit::ghost(ui, "Clear", !self.selected.is_empty()).clicked() {
                self.selected.clear();
            }
            if kit::ghost(ui, "Re-check", self.busy.is_none()).clicked() {
                self.refresh_tweaks(ctx);
            }
        });
    }

    fn selected_in_order(&self) -> Vec<&'static str> {
        self.tweaks
            .iter()
            .filter(|tw| self.selected.contains(tw.id))
            .map(|tw| tw.id)
            .collect()
    }

    pub(super) fn apply_tweaks(&mut self, ids: Vec<&'static str>) {
        let now = crate::opti::now_iso();
        let mut plan = Plan::default();
        let mut lines = Vec::new();
        let mut state = self.state.clone();
        let mut backups: Vec<(String, String)> = Vec::new();
        let mut applying = Vec::new();
        for id in ids {
            let Some(tw) = tweaks::by_id(&self.tweaks, id) else {
                continue;
            };
            match tweaks::prepare_apply(self.sys.as_ref(), tw, state.tweaks.get(id), &now) {
                Ok(prepared) => {
                    lines.push(Line::new(format!("{}  [{}]", tw.name, tw.risk.label())));
                    lines.extend(prepared.notes.into_iter().map(Line::warn));
                    if prepared.plan.is_empty() {
                        lines.push(Line::new(
                            "  Files already match; only the running state is refreshed.",
                        ));
                    }
                    plan.extend(prepared.plan);
                    backups.extend(prepared.backups);
                    state.tweaks.insert(id.to_string(), prepared.record);
                    applying.push(id);
                }
                Err(error) => {
                    lines.push(Line::warn(format!("{}: {error}", tw.name)));
                    self.say(format!("WARN {}: {error}", tw.name));
                }
            }
        }
        if applying.is_empty() {
            self.toast("Nothing could be prepared. See the operation center for the reasons.");
            return;
        }
        let mut intro = String::from(
            "Each file below is written exactly as shown. The original of every file, line and service is saved first, so Undo can put it back.",
        );
        if !backups.is_empty() {
            intro.push_str(" Edited files are also copied into Optimaxer's backup folder.");
        }
        let title = if applying.len() == 1 {
            format!(
                "Apply: {}",
                tweaks::by_id(&self.tweaks, applying[0]).map_or("tweak", |t| t.name)
            )
        } else {
            format!("Apply {} tweaks", applying.len())
        };
        let mut pending = self.plan_pending(
            title,
            intro,
            lines,
            plan,
            "Apply",
            Followup::Tweaks(applying),
            Some(state),
            None,
        );
        if let Ok(p) = &mut pending
            && !backups.is_empty()
        {
            let inner = std::mem::replace(&mut p.run, Box::new(|_| Ok(Vec::new())));
            p.run = Box::new(move |sys| {
                save_backups(&backups)?;
                inner(sys)
            });
        }
        self.ask(pending);
    }

    pub(super) fn undo_tweaks(&mut self, ids: Vec<&'static str>) {
        let mut plan = Plan::default();
        let mut lines = Vec::new();
        let mut state = self.state.clone();
        let mut undoing = Vec::new();
        for id in ids {
            let Some(tw) = tweaks::by_id(&self.tweaks, id) else {
                continue;
            };
            let Some(record) = self.state.tweaks.get(id) else {
                lines.push(Line::warn(format!("{}: no saved original values (it was not applied by Optimaxer), so it is left alone.", tw.name)));
                continue;
            };
            match tweaks::prepare_undo(self.sys.as_ref(), tw, record) {
                Ok((undo, notes)) => {
                    lines.push(Line::new(format!("{}  (applied {})", tw.name, record.time)));
                    lines.extend(notes.into_iter().map(Line::warn));
                    plan.extend(undo);
                    state.tweaks.remove(id);
                    undoing.push(id);
                }
                Err(error) => lines.push(Line::warn(format!("{}: {error}", tw.name))),
            }
        }
        if undoing.is_empty() {
            self.toast("Nothing to undo for the selection.");
            return;
        }
        let title = if undoing.len() == 1 {
            format!(
                "Undo: {}",
                tweaks::by_id(&self.tweaks, undoing[0]).map_or("tweak", |t| t.name)
            )
        } else {
            format!("Undo {} tweaks", undoing.len())
        };
        let pending = self.plan_pending(
            title,
            "The saved original values are written back. Running kernel settings are set back at once where Optimaxer recorded them.",
            lines,
            plan,
            "Undo",
            Followup::Tweaks(Vec::new()),
            None,
            Some(state),
        );
        self.ask(pending);
    }
}

/// Whole-file copies of the files about to be edited, kept in Optimaxer's backup folder. A failed copy stops the apply.
fn save_backups(backups: &[(String, String)]) -> Result<(), String> {
    let dir = crate::opti::state::backup_dir();
    for (path, content) in backups {
        let name = path.rsplit('/').next().unwrap_or("file");
        let target = dir.join(format!("{name}.{}.bak", crate::opti::now_stamp()));
        std::fs::write(&target, content).map_err(|e| {
            format!("Nothing was changed: could not save a backup of {path} ({e}).")
        })?;
    }
    Ok(())
}
