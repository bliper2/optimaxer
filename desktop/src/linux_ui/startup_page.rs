//! Startup: XDG autostart entries and systemd user services. Both are changed without administrator rights.

use super::*;
use crate::linux::startup::{
    Scope, prepare_autostart, prepare_user_unit, restore_autostart, restore_user_unit,
};

impl OptiUi {
    pub(super) fn startup_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Startup",
            "Programs that start with your desktop session, and systemd user services. Turning something off keeps it installed; it only stops starting by itself.",
        );
        if kit::ghost(ui, "Refresh", self.busy.is_none()).clicked() {
            self.refresh_startup(ctx);
        }
        let idle = self.busy.is_none();
        let mut toggle: Option<(AutostartItem, bool)> = None;
        let mut restore_file: Option<String> = None;
        kit::group_title(ui, "Autostart applications");
        if self.autostart.is_empty() {
            ui.label(
                RichText::new(if self.startup_loaded {
                    "No autostart entries found."
                } else {
                    "Loading…"
                })
                .size(T_SMALL)
                .color(t.subtle),
            );
        }
        for item in &self.autostart {
            let saved = self.state.autostart.contains_key(&item.file);
            kit::row(ui, ("linux-autostart", &item.file), false, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(&item.name)
                                    .font(FontId::new(T_BODY, semibold()))
                                    .color(t.text),
                            );
                            Self::badge(
                                ui,
                                if item.enabled { "On" } else { "Off" },
                                if item.enabled {
                                    Tone::Good
                                } else {
                                    Tone::Neutral
                                },
                            );
                            if item.scope == Scope::System {
                                Self::badge(ui, "System entry", Tone::Neutral);
                            }
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(&item.exec)
                                    .monospace()
                                    .size(T_CAPTION)
                                    .color(t.subtle),
                            )
                            .truncate(),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if saved && kit::ghost(ui, "Restore original", idle).clicked() {
                            restore_file = Some(item.file.clone());
                        }
                        if kit::secondary(
                            ui,
                            if item.enabled { "Turn off" } else { "Turn on" },
                            idle,
                        )
                        .clicked()
                        {
                            toggle = Some((item.clone(), !item.enabled));
                        }
                    });
                });
            });
        }

        let mut unit_toggle: Option<(String, bool)> = None;
        let mut unit_restore: Option<String> = None;
        kit::group_title(ui, "systemd user services");
        match &self.user_units {
            None => ui.label(RichText::new("Loading…").size(T_SMALL).color(t.subtle)),
            Some(Err(error)) => {
                ui.add(egui::Label::new(RichText::new(error).size(T_SMALL).color(t.subtle)).wrap())
            }
            Some(Ok(units)) if units.is_empty() => ui.label(
                RichText::new("No user services that can be switched.")
                    .size(T_SMALL)
                    .color(t.subtle),
            ),
            Some(Ok(units)) => {
                for u in units {
                    let saved = self.state.user_units.contains_key(&u.unit);
                    kit::row(ui, ("linux-user-unit", &u.unit), false, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&u.unit).color(t.text));
                            Self::badge(
                                ui,
                                if u.enabled { "Enabled" } else { "Disabled" },
                                if u.enabled {
                                    Tone::Brand
                                } else {
                                    Tone::Neutral
                                },
                            );
                            if u.active {
                                Self::badge(ui, "Running", Tone::Good);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if saved && kit::ghost(ui, "Restore original", idle).clicked() {
                                    unit_restore = Some(u.unit.clone());
                                }
                                if kit::secondary(
                                    ui,
                                    if u.enabled { "Disable" } else { "Enable" },
                                    idle,
                                )
                                .clicked()
                                {
                                    unit_toggle = Some((u.unit.clone(), !u.enabled));
                                }
                            });
                        });
                    });
                }
                ui.label(RichText::new("systemd user services are the units under ~/.config/systemd/user and /usr/lib/systemd/user.").size(T_CAPTION).color(t.subtle))
            }
        };

        if let Some((item, enable)) = toggle {
            match prepare_autostart(
                self.sys.as_ref(),
                &item,
                enable,
                self.state.autostart.get(&item.file),
            ) {
                Ok(change) => {
                    let mut state = self.state.clone();
                    state.autostart.insert(item.file.clone(), change.snapshot);
                    let verb = if enable { "Turn on" } else { "Turn off" };
                    let (path, content) = (change.path.clone(), change.new_content);
                    let lines = vec![
                        Line::new(format!("Write {path}")),
                        Line::new(if enable {
                            "The Hidden flag is removed so the entry starts again."
                        } else {
                            "Hidden=true is added; the entry stays installed but no longer starts."
                        }),
                    ];
                    self.ask(Ok(Pending {
                        title: format!("{verb} {} at startup", item.name),
                        intro: "Only a file in your own ~/.config/autostart changes. Any system entry is left as it is, and the original is saved.".into(),
                        lines,
                        ok_label: verb.into(),
                        run: Box::new(move |sys| sys.write_user(&path, &content).map(|()| vec![format!("Wrote {path}")])),
                        save_before: Some(state),
                        save_after: None,
                        followup: Followup::Startup,
                        script: None,
                    }));
                }
                Err(error) => self.toast(error),
            }
        }
        if let Some(file) = restore_file
            && let Some(snapshot) = self.state.autostart.get(&file).cloned()
        {
            let mut state = self.state.clone();
            state.autostart.remove(&file);
            self.ask(Ok(Pending {
                title: format!("Restore startup entry {file}"),
                intro: "The saved original is put back.".into(),
                lines: vec![Line::new(if snapshot.existed {
                    "The user file returns to its saved content."
                } else {
                    "The override file Optimaxer created is removed."
                })],
                ok_label: "Restore".into(),
                run: Box::new(move |sys| {
                    restore_autostart(sys, &snapshot).map(|()| vec!["Restored".to_string()])
                }),
                save_before: None,
                save_after: Some(state),
                followup: Followup::Startup,
                script: None,
            }));
        }
        if let Some((unit, enable)) = unit_toggle {
            let change = prepare_user_unit(
                self.sys.as_ref(),
                &unit,
                enable,
                self.state.user_units.get(&unit),
            );
            let mut state = self.state.clone();
            state.user_units.insert(unit.clone(), change.snapshot);
            let verb = if enable { "Enable" } else { "Disable" };
            let pending = self.plan_pending(format!("{verb} {unit}"), "A systemd user service is switched with systemctl --user. The original state is saved first.", Vec::new(), change.plan, verb, Followup::Startup, Some(state), None);
            self.ask(pending);
        }
        if let Some(unit) = unit_restore
            && let Some(snapshot) = self.state.user_units.get(&unit).cloned()
        {
            let mut state = self.state.clone();
            state.user_units.remove(&unit);
            let pending = self.plan_pending(
                format!("Restore {unit}"),
                "The saved original state is applied.",
                Vec::new(),
                restore_user_unit(&snapshot),
                "Restore",
                Followup::Startup,
                None,
                Some(state),
            );
            self.ask(pending);
        }
    }
}
