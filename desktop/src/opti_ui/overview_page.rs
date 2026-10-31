//! Overview page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn smart_optimize_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Smart Optimize",
            "Hardware-aware recommendations from this PC's current configuration. Review every change before applying it.",
        );

        let (Some(traits), true) = (self.traits.as_ref(), self.status_ready) else {
            kit::empty_state(
                ui,
                "Reading this PC's configuration",
                "Hardware details and current tweak states are collected in the background. This page does not change Windows settings.",
            );
            kit::skeleton(ui, 3);
            return;
        };

        let plan = hw::smart_plan(&self.cat, traits, &self.status);
        let (active, total) = score(&self.cat, traits, &self.status);
        let planned = plan.ids.len();
        let refreshing = self.busy.as_deref() == Some("Checking active tweaks");
        draw_smart_plan_summary(ui, traits, &plan, active, total, refreshing, &t);
        ui.add_space(S3);
        draw_smart_plan_changes(ui, &self.cat, &plan, &t);
        draw_smart_plan_reasons(ui, &self.cat, &plan, &t);
        let review_label = if planned == 0 {
            "No changes to review".to_string()
        } else {
            format!(
                "Review {planned} change{}",
                if planned == 1 { "" } else { "s" }
            )
        };
        ui.add_space(S3);
        ui.horizontal_wrapped(|ui| {
            if kit::primary(ui, &review_label, planned > 0 && self.busy.is_none()).clicked() {
                self.review_smart_plan(ctx);
            }
            if kit::secondary(ui, "Refresh current state", self.busy.is_none()).clicked() {
                self.refresh_status(ctx);
            }
            if kit::ghost(ui, "Open full tweak catalog", true).clicked() {
                self.set_page(ctx, Page::Tweaks);
            }
        });
        ui.add_space(S1);
        ui.add(
            egui::Label::new(
                RichText::new("Review opens the selected tweaks in the catalog. Preview changes to inspect exact updates before confirmation. Applying saves originals and verifies supported settings; no FPS gain is promised.")
                    .size(T_SMALL)
                    .color(t.muted),
            )
            .wrap(),
        );
    }

    pub(super) fn overview_page(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        settings: &mut Settings,
    ) {
        use crate::opti::sysmon::format_uptime;
        let t = tokens(ctx);
        self.tick_telemetry(ctx, settings);
        let info = self.info.clone().unwrap_or_default();
        let s = self.telemetry_frame.latest;
        kit::page_header(
            ui,
            "Overview",
            "Live readings from this PC, and the quickest ways to tune it.",
        );
        if let Some(error) = &self.telemetry_error {
            ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let gap = S3;
            ui.spacing_mut().item_spacing = vec2(gap, gap);
            let compact_actions = ui.available_width() < 680.0;
            ui.horizontal_wrapped(|ui| {
                let ready = self.busy.is_none() && self.traits.is_some();
                if kit::primary(ui, "Smart Optimize", ready).on_hover_text("Review hardware-aware recommendations before selecting changes").clicked() {
                    self.set_page(ctx, Page::SmartOptimize);
                }
                if kit::secondary(ui, "Scan for junk", self.busy.is_none()).clicked() {
                    self.set_page(ctx, Page::Cleaner);
                    self.scan_cleaner(ctx, settings.cleaner_exclusions.clone());
                }
                if kit::secondary(ui, "Review startup", true).clicked() {
                    self.set_page(ctx, Page::Startup);
                }
                if !compact_actions && kit::secondary(ui, "Open diagnostics", true).clicked() {
                    self.set_page(ctx, Page::Diagnostics);
                }
            });
            ui.horizontal_wrapped(|ui| {
                if kit::secondary(ui, "Check for updates", true).clicked() {
                    self.events.push(OptiEvent::CheckForUpdates);
                }
                if let Some(restore_point) = tools::TOOLS
                    .iter()
                    .find(|info| info.tool == Tool::RestorePoint)
                    && kit::secondary(ui, "Create restore point", self.busy.is_none()).clicked()
                {
                    self.request_system_tool(restore_point);
                }
                if kit::secondary(ui, "Free RAM", self.busy.is_none()).clicked()
                    && self.need_admin()
                {
                    self.pending = Some(Pending { title: "Free RAM now".into(), intro: "Clears every RAM cache Windows allows: process and system working sets, the file cache, the modified and standby lists, identical pages and the registry cache. Windows refills the file cache as you open files again.".into(), lines: vec![], action: Action::RunTool(Tool::FreeRam), restore_option: false, ok_label: "Free RAM".into() });
                }
                if compact_actions && kit::secondary(ui, "Open diagnostics", true).clicked() {
                    self.set_page(ctx, Page::Diagnostics);
                }
            });
            self.overview_layout_controls(ui, settings);
            self.refresh_pending_restart();
            let inactive_recommended = if self.status_ready {
                self.traits.as_ref().map(|tr| {
                    let (active, total) = score(&self.cat, tr, &self.status);
                    total.saturating_sub(active)
                })
            } else {
                None
            };
            let storage_review = storage_needs_review(&s);
            let restart_reasons = self.restart_reasons.1.clone();
            let restart_flagged = settings.opti_restart_required;
            if inactive_recommended.is_some_and(|count| count > 0)
                || storage_review
                || restart_flagged
                || !restart_reasons.is_empty()
            {
                ui.add_space(S2);
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Recommended actions").font(FontId::new(T_BODY, semibold())).color(t.text));
                    ui.add_space(S1);
                    if let Some(count) = inactive_recommended.filter(|count| *count > 0) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{count} Smart Optimize setting{} not active on this PC.", if count == 1 { " is" } else { "s are" })).size(T_SMALL).color(t.muted));
                            if kit::secondary(ui, "Review plan", self.busy.is_none()).clicked() {
                                self.set_page(ctx, Page::SmartOptimize);
                            }
                        });
                    }
                    if restart_flagged || !restart_reasons.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            kit::chip(ui, "Restart pending", Tone::Warn);
                            let text = if restart_reasons.is_empty() {
                                "A change you confirmed needs a Windows restart to finish.".to_owned()
                            } else {
                                format!("Windows reports a pending restart from: {}.", restart_reasons.join(", "))
                            };
                            ui.label(RichText::new(text).size(T_SMALL).color(t.muted));
                        });
                    }
                    if storage_review {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("System drive is {:.0}% full. Scan cleanup targets before choosing what to remove.", s.disk_fraction() * 100.0)).size(T_SMALL).color(t.muted));
                            if kit::secondary(ui, "Scan storage", self.busy.is_none()).clicked() && self.busy.is_none() {
                                self.set_page(ctx, Page::Cleaner);
                                self.scan_cleaner(ctx, settings.cleaner_exclusions.clone());
                            }
                        });
                    }
                });
            }
            ui.add_space(S1);
            let width = ui.available_width();
            let compact = width < 720.0;
            let w3 = ((width - 2.0 * gap) / 3.0 - 1.0).floor();
            match settings.palette.overview_layout() {
                theme::OverviewLayout::Dashboard => self.overview_dashboard(ui, ctx, &s, &info),
                theme::OverviewLayout::List => self.overview_list(ui, ctx, &s, &info),
                theme::OverviewLayout::Cards => {
            ui.horizontal_top(|ui| {
                let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
                let cards = [
                    ("Processor", s.cpu / 100.0, if s.cpu_available { format!("{:.0}%", s.cpu) } else { "Unavailable".into() }, if s.cpu_available { format!("{} threads", info.threads) } else { "Windows CPU counters unavailable".into() }, if s.cpu_available { kit::load_color(&t, s.cpu) } else { t.muted }),
                    ("Memory", s.ram_fraction(), if s.ram_available { format!("{:.0}%", s.ram_fraction() * 100.0) } else { "Unavailable".into() }, if s.ram_available { format!("{} of {}", format_size(Some(s.ram_used)), format_size(Some(s.ram_total))) } else { "Windows memory counters unavailable".into() }, if s.ram_available { kit::load_color(&t, s.ram_fraction() * 100.0) } else { t.muted }),
                    ("Storage", s.disk_fraction(), if s.disk_available { format!("{:.0}%", s.disk_fraction() * 100.0) } else { "Unavailable".into() }, if s.disk_available { format!("{drive} {} of {}", format_size(Some(s.disk_used)), format_size(Some(s.disk_total))) } else { "Windows volume counters unavailable".into() }, if s.disk_available { kit::load_color(&t, s.disk_fraction() * 100.0) } else { t.muted }),
                ];
                for (name, frac, value, sub, color) in cards {
                    let card_height = if compact { 126.0 } else { 132.0 };
                    ui.allocate_ui_with_layout(vec2(w3, card_height), Layout::top_down(Align::Min), |ui| {
                        kit::card(ui, |ui| {
                            let inner_width = (w3 - 2.0 * S4 - 2.0).max(40.0);
                            ui.set_width(inner_width);
                            if compact {
                                ui.label(RichText::new(name).font(FontId::new(T_SMALL, semibold())).color(t.text));
                                ui.label(RichText::new(&value).font(FontId::new(T_TITLE, semibold())).color(t.text));
                                ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).fill(color).desired_height(6.0).corner_radius(3).desired_width(inner_width));
                                ui.add(egui::Label::new(RichText::new(sub).size(T_CAPTION).color(t.muted)).truncate());
                            } else {
                                ui.horizontal(|ui| {
                                    ui.allocate_ui_with_layout(vec2((inner_width - 104.0).max(80.0), 104.0), Layout::top_down(Align::Min), |ui| {
                                        ui.add_space(S2);
                                        ui.label(RichText::new(name).font(FontId::new(T_BODY, semibold())).color(t.text));
                                        ui.add_space(S2);
                                        ui.add(egui::Label::new(RichText::new(sub).size(T_SMALL).color(t.muted)).truncate());
                                    });
                                    kit::gauge(ui, 92.0, frac, color, &value, "in use");
                                });
                            }
                        });
                    });
                }
            });
                }
            }
            if overview_shows(settings, "activity") {
                self.overview_activity(ui, settings, &s);
            }
            if overview_shows(settings, "system") {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("This PC").font(FontId::new(T_HEADING, semibold())).color(t.text));
                ui.add_space(S1);
                egui::Grid::new("pc-info").num_columns(2).spacing(vec2(S5, S1 + 2.0)).show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(RichText::new(k).size(T_SMALL).color(t.muted));
                        ui.add(egui::Label::new(RichText::new(v).size(T_BODY).color(t.text)).wrap());
                        ui.end_row();
                    };
                    let system_info = |value: String| {
                        if self.info.is_none() {
                            "Reading system information…".into()
                        } else if value.is_empty() {
                            "Unknown".into()
                        } else {
                            value
                        }
                    };
                    row("Processor", system_info(info.cpu.clone()));
                    row("Graphics", system_info(info.gpus.join(", ")));
                    row("Windows", system_info(info.os.clone()));
                    row("Uptime", format_uptime(s.uptime));
                    row("Processes", s.processes.to_string());
                    if let Some(tr) = &self.traits {
                        row("Profile", profile_summary(tr));
                    }
                });
            });
            }
            ui.add_space(S2);
            if overview_shows(settings, "history") {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Recent tweak changes").font(FontId::new(T_HEADING, semibold())).color(t.text));
                ui.add_space(S1);
                if let Some(error) = &self.history_error {
                    ui.label(RichText::new(format!("History unavailable: {error}")).size(T_SMALL).color(t.warn));
                } else if self.history.is_empty() {
                    ui.label(RichText::new("Confirmed tweak apply and undo operations will appear here.").size(T_SMALL).color(t.muted));
                } else {
                    for (index, entry) in self.history.iter().rev().take(8).enumerate() {
                        let verified = entry.changes.iter().filter(|change| change.verified).count();
                        let undoable = entry.changes.iter().filter(|change| change.undo_available).count();
                        let action = match entry.action {
                            HistoryAction::Apply => "Applied",
                            HistoryAction::Undo => "Undid",
                        };
                        egui::CollapsingHeader::new(format!("{action} tweaks · {verified}/{} verified · {undoable} undoable", entry.changes.len()))
                            .id_salt(("tweak-history", index, &entry.time))
                            .show(ui, |ui| {
                                ui.label(RichText::new(&entry.time).size(T_CAPTION).color(t.subtle));
                                for change in &entry.changes {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(&change.name).size(T_SMALL).color(t.text));
                                        kit::chip(ui, if change.verified { "Verified" } else { "Incomplete" }, if change.verified { Tone::Good } else { Tone::Warn });
                                        if change.undo_available {
                                            kit::chip(ui, "Undo available", Tone::Neutral);
                                        }
                                    });
                                    for detail in &change.details {
                                        ui.add(egui::Label::new(RichText::new(detail).size(T_CAPTION).color(t.muted)).wrap());
                                    }
                                }
                            });
                    }
                    ui.label(RichText::new("Records confirmed tweak changes and verification. Performance gains are not measured.").size(T_CAPTION).color(t.subtle));
                }
            });
            }
            ui.add_space(S3);
        });
    }

    /// Re-reads the Windows restart markers at most every 30 seconds; the reads are cheap but the page redraws often.
    fn refresh_pending_restart(&mut self) {
        let stale = self
            .restart_reasons
            .0
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(30));
        if stale {
            self.restart_reasons = (
                Some(Instant::now()),
                crate::opti::pending_restart::detect(&WinSystem),
            );
        }
    }

    /// Which Overview sections are shown. The choice is saved with the other settings.
    fn overview_layout_controls(&mut self, ui: &mut Ui, settings: &mut Settings) {
        let t = tokens(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Show").size(T_SMALL).color(t.muted));
            for (id, label) in OVERVIEW_WIDGETS {
                let mut shown = overview_shows(settings, id);
                if ui.checkbox(&mut shown, label).changed() {
                    let previous = settings.overview_hidden.clone();
                    settings.overview_hidden.retain(|hidden| hidden != id);
                    if !shown {
                        settings.overview_hidden.push(id.to_owned());
                    }
                    if let Err(error) = settings.save_checked() {
                        settings.overview_hidden = previous;
                        self.say(&format!("Overview layout was not saved: {error}"));
                    }
                }
            }
        });
    }

    /// Readings for the sensors that are not on the gauges, each marked unavailable when Windows gives no value.
    fn overview_activity(
        &mut self,
        ui: &mut Ui,
        settings: &mut Settings,
        s: &crate::opti::sysmon::Snapshot,
    ) {
        use crate::opti::sysmon::{HISTORY_WINDOW_SECONDS, format_bytes_per_second};
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Live activity")
                        .font(FontId::new(T_HEADING, semibold()))
                        .color(t.text),
                );
                let mut range = HISTORY_WINDOW_SECONDS
                    .iter()
                    .position(|seconds| *seconds == settings.telemetry_config().history_seconds())
                    .unwrap_or(1);
                if kit::segmented(ui, &mut range, &["30s", "60s", "120s"]) {
                    let previous = settings.opti_history_seconds;
                    settings.opti_history_seconds = HISTORY_WINDOW_SECONDS[range];
                    if let Err(error) = settings.save_checked() {
                        settings.opti_history_seconds = previous;
                        self.say(&format!("Chart history was not saved: {error}"));
                    }
                }
            });
            ui.add_space(S1);
            let none = || "Unavailable".to_owned();
            let gpu = s
                .gpu_busiest_engine
                .map_or_else(none, |v| format!("{v:.0}%"));
            let gpu_memory = s
                .gpu_dedicated_memory_bytes
                .map_or_else(none, |bytes| format_size(Some(bytes)));
            let disk = s.disk_activity.map_or_else(
                || {
                    if s.disk_counter_available {
                        "Collecting".to_owned()
                    } else {
                        none()
                    }
                },
                format_bytes_per_second,
            );
            let network = s.network.map_or_else(
                || {
                    if s.network_available {
                        "Collecting".to_owned()
                    } else {
                        none()
                    }
                },
                |n| {
                    format!(
                        "↓ {}  ↑ {}",
                        format_bytes_per_second(n.received_bytes_per_sec),
                        format_bytes_per_second(n.sent_bytes_per_sec)
                    )
                },
            );
            let readings = [
                ("GPU engine", gpu, "Busiest engine"),
                ("GPU memory", gpu_memory, "Dedicated, in use"),
                ("Disk activity", disk, "Read + write"),
                ("Network", network, "Adapter counters"),
            ];
            let columns = if ui.available_width() < 720.0 { 2 } else { 4 };
            egui::Grid::new("overview-activity")
                .num_columns(columns)
                .spacing(vec2(S5, S2))
                .show(ui, |ui| {
                    for (index, (name, value, detail)) in readings.into_iter().enumerate() {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(name).size(T_SMALL).color(t.muted));
                            ui.label(
                                RichText::new(value)
                                    .font(FontId::new(T_HEADING, semibold()))
                                    .color(t.text),
                            );
                            ui.label(RichText::new(detail).size(T_CAPTION).color(t.subtle));
                        });
                        if (index + 1) % columns == 0 {
                            ui.end_row();
                        }
                    }
                });
            ui.label(
                RichText::new(format!(
                    "Charts for the last {} seconds are on the Performance page.",
                    settings.telemetry_config().history_seconds()
                ))
                .size(T_CAPTION)
                .color(t.subtle),
            );
        });
        ui.add_space(S2);
    }

    pub(super) fn performance_metric_card(ui: &mut Ui, title: &str, value: &str, detail: &str) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(T_SMALL).color(t.muted));
            ui.label(
                RichText::new(value)
                    .font(FontId::new(T_TITLE, semibold()))
                    .color(t.text),
            );
            ui.label(RichText::new(detail).size(T_CAPTION).color(t.subtle));
        });
    }

    pub(super) fn performance_load_chart(
        ui: &mut Ui,
        title: &str,
        value: Option<f32>,
        data: &VecDeque<Option<f32>>,
        history_capacity: usize,
        color: Color32,
    ) {
        let t = tokens(ui.ctx());
        let width = ui.available_width();
        kit::card(ui, |ui| {
            ui.set_width(width - 2.0 * S4 - 2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(title)
                        .font(FontId::new(T_BODY, semibold()))
                        .color(t.text),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let value =
                        value.map_or_else(|| "Unavailable".into(), |value| format!("{value:.0}%"));
                    ui.label(RichText::new(value).size(T_SMALL).color(t.muted));
                });
            });
            ui.add_space(S1);
            kit::sparkline(
                ui,
                vec2(ui.available_width(), 86.0),
                data,
                history_capacity,
                color,
            );
        });
    }
}

/// Overview sections the user can hide, by the id saved in settings.
const OVERVIEW_WIDGETS: [(&str, &str); 3] = [
    ("activity", "Live activity"),
    ("system", "This PC"),
    ("history", "Recent changes"),
];

fn overview_shows(settings: &Settings, id: &str) -> bool {
    !settings.overview_hidden.iter().any(|hidden| hidden == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_sections_are_off_and_everything_else_is_shown() {
        let mut settings = Settings::default();
        assert!(
            OVERVIEW_WIDGETS
                .iter()
                .all(|(id, _)| overview_shows(&settings, id))
        );
        settings.overview_hidden.push("activity".into());
        assert!(!overview_shows(&settings, "activity"));
        assert!(overview_shows(&settings, "system"));
    }

    #[test]
    fn settings_without_the_layout_field_load_with_every_section_visible() {
        let settings: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert!(settings.overview_hidden.is_empty());
        let saved = serde_json::to_string(&Settings {
            overview_hidden: vec!["history".into()],
            ..Settings::default()
        })
        .unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.overview_hidden, ["history"]);
    }
}
