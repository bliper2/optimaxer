//! Services page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn services_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        if !self.services_loaded && self.busy.is_none() {
            self.refresh_services(ctx);
        }
        kit::page_header(
            ui,
            "Services",
            "Optimaxer's recommended Manual and Disabled lists. Core Windows services are hidden. The current start types are saved to a backup before every change.",
        );
        let idle = self.busy.is_none();
        let compact = ui.available_width() < 520.0;
        let mut clear_selection = false;
        let mut restore_services = false;
        let mut refresh_services = false;
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.svc_search);
            let search_width = if compact { 110.0 } else { 260.0 };
            let search_hint = if compact { "Search" } else { "Search services" };
            ui.add_sized(
                [search_width, 34.0],
                egui::TextEdit::singleline(&mut q)
                    .hint_text(search_hint)
                    .margin(Margin::symmetric(S3 as i8, 8)),
            );
            self.svc_search = q;
            if kit::ghost(ui, "Select recommended", true).clicked() {
                self.svc_sel = self
                    .services
                    .iter()
                    .filter(|s| s.rec.as_ref().is_some_and(|r| r.0 == 3))
                    .map(|s| s.name.clone())
                    .collect();
            }
            if compact {
                ui.menu_button("More", |ui| {
                    if ui
                        .add_enabled(
                            !self.svc_sel.is_empty(),
                            egui::Button::new("Clear selection"),
                        )
                        .clicked()
                    {
                        clear_selection = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(idle, egui::Button::new("Restore last backup"))
                        .clicked()
                    {
                        restore_services = true;
                        ui.close();
                    }
                    if ui.add_enabled(idle, egui::Button::new("Refresh")).clicked() {
                        refresh_services = true;
                        ui.close();
                    }
                });
            } else {
                if kit::ghost(ui, "Clear", !self.svc_sel.is_empty()).clicked() {
                    self.svc_sel.clear();
                }
                if !self.svc_sel.is_empty() {
                    ui.label(
                        RichText::new(format!("{} selected", self.svc_sel.len()))
                            .size(T_SMALL)
                            .color(t.muted),
                    );
                }
            }
        });
        if clear_selection {
            self.svc_sel.clear();
        }
        if restore_services && idle && self.need_admin() {
            self.pending = Some(Pending {
                title: "Restore services".into(),
                intro: "Put every service back to the start type saved in the most recent backup?"
                    .into(),
                lines: vec![],
                action: Action::RestoreServices,
                restore_option: false,
                ok_label: "Restore services".into(),
            });
        }
        if refresh_services && idle {
            self.refresh_services(ctx);
        }
        ui.horizontal_wrapped(|ui| {
            let names: Vec<String> = self.svc_sel.iter().cloned().collect();
            let any = idle && !names.is_empty();
            for (label, start) in [("Set to Manual", 3u32), ("Disable", 4), ("Set to Automatic", 2)] {
                if kit::secondary(ui, label, any).clicked() && self.need_admin() {
                    let mut selected = names.clone();
                    selected.sort_by_key(|name| name.to_lowercase());
                    let targets: Vec<(String, String)> = selected
                        .iter()
                        .map(|name| {
                            let display = self
                                .services
                                .iter()
                                .find(|service| service.name.eq_ignore_ascii_case(name))
                                .map(|service| service.display.clone())
                                .unwrap_or_default();
                            (name.clone(), display)
                        })
                        .collect();
                    let count = selected.len();
                    let title = label.to_string();
                    let ok_label = format!("{label} ({count})");
                    self.spawn(ctx, "Reviewing service dependencies", move |worker| {
                        match services::dependency_info(&selected).and_then(|dependencies| {
                            service_dependency_lines(
                                    &targets,
                                    &dependencies,
                                    start == 4,
                                )
                        }) {
                            Ok(lines) => {
                                worker.send(Msg::Plan(Pending {
                                    title,
                                    intro: format!(
                                        "Review direct dependencies before changing {count} service{}. Disabling a service may prevent its dependents from starting; this changes future start behavior and does not stop running services. Current start types are backed up before the change.",
                                        if count == 1 { "" } else { "s" }
                                    ),
                                    lines,
                                    action: Action::SetServices {
                                        names: selected,
                                        start,
                                    },
                                    restore_option: false,
                                    ok_label,
                                }));
                            }
                            Err(error) => {
                                let message = format!(
                                    "Service dependency review failed; no services were changed: {error}"
                                );
                                worker.log(&format!("WARN {message}"));
                                worker.send(Msg::Toast(message));
                            }
                        }
                    });
                }
            }
            if !compact {
                if kit::ghost(ui, "Restore last backup", idle).clicked() && self.need_admin() {
                    self.pending = Some(Pending { title: "Restore services".into(), intro: "Put every service back to the start type saved in the most recent backup?".into(), lines: vec![], action: Action::RestoreServices, restore_option: false, ok_label: "Restore services".into() });
                }
                if kit::ghost(ui, "Refresh", idle).clicked() {
                    self.refresh_services(ctx);
                }
            }
        });
        ui.add_space(S2);
        if self.services.is_empty() {
            if self.busy.is_some() {
                kit::skeleton(ui, 6);
            } else {
                kit::empty_state(
                    ui,
                    "No services loaded",
                    "Press Refresh to read the service list.",
                );
            }
            return;
        }
        let q = self.svc_search.trim().to_lowercase();
        let mut last_group = "";
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for s in &self.services {
                    if !q.is_empty()
                        && !s.name.to_lowercase().contains(&q)
                        && !s.display.to_lowercase().contains(&q)
                        && !s.desc.to_lowercase().contains(&q)
                    {
                        continue;
                    }
                    let group = match s.rec.as_ref().map(|r| r.0) {
                        Some(3) => "Recommended: set to Manual",
                        Some(4) => "Recommended: disable",
                        _ => "Other services",
                    };
                    if group != last_group {
                        kit::group_title(ui, group);
                        last_group = group;
                    }
                    let on = self.svc_sel.contains(&s.name);
                    let r = kit::row(ui, ("svc", &s.name), on, |ui| {
                        ui.horizontal(|ui| {
                            kit::check(ui, on);
                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(if s.display.contains(s.name.as_str()) {
                                            s.display.clone()
                                        } else {
                                            format!("{}  ({})", s.display, s.name)
                                        })
                                        .font(FontId::new(T_BODY, semibold()))
                                        .color(t.text),
                                    );
                                    kit::chip(
                                        ui,
                                        &format!(
                                            "{}, {}",
                                            s.mode(),
                                            if s.running { "running" } else { "stopped" }
                                        ),
                                        if s.running {
                                            Tone::Brand
                                        } else {
                                            Tone::Neutral
                                        },
                                    );
                                });
                                let desc = s
                                    .rec
                                    .as_ref()
                                    .map(|r| r.1.as_str())
                                    .filter(|d| !d.is_empty())
                                    .unwrap_or(&s.desc);
                                if !desc.is_empty() {
                                    let shown: String = if desc.chars().count() > 170 {
                                        desc.chars().take(167).collect::<String>() + "..."
                                    } else {
                                        desc.to_string()
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(shown).size(T_SMALL).color(t.muted),
                                        )
                                        .wrap(),
                                    );
                                }
                            });
                        });
                    });
                    let click = ui.interact(
                        r.response.rect,
                        ui.scope_id().with(("svc-click", &s.name)),
                        Sense::click(),
                    );
                    if click.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if click.clicked() {
                        if on {
                            self.svc_sel.remove(&s.name);
                        } else {
                            self.svc_sel.insert(s.name.clone());
                        }
                    }
                }
            });
    }
}
