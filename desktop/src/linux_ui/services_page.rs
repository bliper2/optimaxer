//! Services: systemd system services. Switching one is previewed, saved and reversible.

use super::*;

impl OptiUi {
    pub(super) fn services_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Services",
            "systemd services on this machine. Disabling stops a service now and keeps it off at boot; Optimaxer remembers whether it was enabled and running, so Restore puts both back.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [260.0, 32.0],
                egui::TextEdit::singleline(&mut self.service_search).hint_text("Search services"),
            );
            ui.checkbox(&mut self.service_all, "Show all services");
            if kit::ghost(ui, "Refresh", self.busy.is_none()).clicked() {
                self.refresh_services(ctx);
            }
        });
        ui.add_space(S2);
        let rows = match &self.services {
            None => {
                kit::skeleton(ui, 5);
                return;
            }
            Some(Err(error)) => {
                kit::empty_state(ui, "Services are not available", error);
                return;
            }
            Some(Ok(rows)) => rows.clone(),
        };
        let query = self.service_search.trim().to_lowercase();
        let visible: Vec<&ServiceRow> = rows
            .iter()
            .filter(|r| {
                self.service_all || r.advice.is_some() || self.state.services.contains_key(&r.unit)
            })
            .filter(|r| {
                query.is_empty()
                    || r.unit.to_lowercase().contains(&query)
                    || r.description.to_lowercase().contains(&query)
            })
            .collect();
        let mut action: Option<(String, bool)> = None;
        let mut restore: Option<String> = None;
        for r in visible.iter().take(300) {
            let saved = self.state.services.contains_key(&r.unit);
            kit::row(ui, ("linux-svc", &r.unit), false, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(&r.unit)
                                    .font(FontId::new(T_BODY, semibold()))
                                    .color(t.text),
                            );
                            let (boot_tone, active_tone) = (
                                if r.is_enabled() {
                                    Tone::Brand
                                } else {
                                    Tone::Neutral
                                },
                                match r.active.as_str() {
                                    "active" => Tone::Good,
                                    "failed" => Tone::Bad,
                                    _ => Tone::Neutral,
                                },
                            );
                            Self::badge(ui, &r.boot, boot_tone);
                            if r.active != "-" {
                                Self::badge(ui, &r.active, active_tone);
                            }
                        });
                        if !r.description.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&r.description).size(T_SMALL).color(t.muted),
                                )
                                .truncate(),
                            );
                        }
                        if let Some(advice) = r.advice {
                            ui.label(
                                RichText::new(format!("Advice: {advice}"))
                                    .size(T_CAPTION)
                                    .color(t.subtle),
                            );
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let idle = self.busy.is_none();
                        if saved && kit::ghost(ui, "Restore original", idle).clicked() {
                            restore = Some(r.unit.clone());
                        }
                        if r.switchable() {
                            let enable = !r.is_enabled();
                            if kit::secondary(
                                ui,
                                if enable {
                                    "Enable and start"
                                } else {
                                    "Disable and stop"
                                },
                                idle,
                            )
                            .clicked()
                            {
                                action = Some((r.unit.clone(), enable));
                            }
                        }
                    });
                });
            });
        }
        if visible.len() > 300 {
            ui.label(
                RichText::new(format!(
                    "Showing 300 of {} matches. Narrow the search to see others.",
                    visible.len()
                ))
                .size(T_CAPTION)
                .color(t.subtle),
            );
        }
        if visible.is_empty() {
            kit::empty_state(
                ui,
                "No matching services",
                "Tick “Show all services” to list every service unit.",
            );
        }
        if let Some((unit, enable)) = action {
            let advised = rows
                .iter()
                .find(|r| r.unit == unit)
                .is_some_and(|r| r.advice.is_some());
            let prepared = services::prepare_set(
                self.sys.as_ref(),
                &unit,
                enable,
                self.state.services.get(&unit),
            );
            let mut state = self.state.clone();
            state
                .services
                .insert(unit.clone(), prepared.snapshot.clone());
            let mut lines = vec![Line::new(format!(
                "Before: {} at boot, {} now",
                if prepared.snapshot.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                if prepared.snapshot.active {
                    "running"
                } else {
                    "not running"
                }
            ))];
            if !advised {
                lines.push(Line::warn("Optimaxer has no advice for this service. Make sure you know what depends on it before changing it."));
            }
            let verb = if enable {
                "Enable and start"
            } else {
                "Disable and stop"
            };
            let pending = self.plan_pending(
                format!("{verb} {unit}"),
                "The unit is switched with systemctl. The original state is saved first.",
                lines,
                prepared.plan,
                verb,
                Followup::Services,
                Some(state),
                None,
            );
            self.ask(pending);
        }
        if let Some(unit) = restore
            && let Some(snapshot) = self.state.services.get(&unit).cloned()
        {
            let mut state = self.state.clone();
            state.services.remove(&unit);
            let lines = vec![Line::new(format!(
                "Back to: {} at boot, {} now",
                if snapshot.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                if snapshot.active {
                    "running"
                } else {
                    "not running"
                }
            ))];
            let pending = self.plan_pending(
                format!("Restore {unit}"),
                "The saved original state is applied.",
                lines,
                services::prepare_restore(&snapshot),
                "Restore",
                Followup::Services,
                None,
                Some(state),
            );
            self.ask(pending);
        }
    }
}
