//! Startup X-Ray page: Windows' own boot history, the items it measured as slowing boots, service dependencies,
//! related log entries, and boot times before and after startup changes made in Optimaxer.

use super::*;
use crate::opti::blackbox::{self, WinEvent};
use crate::opti::bootlog::{self, BootRecord, History, ReadError, SlowKind, Verdict};
use crate::opti::services::DependencyInfo;

#[derive(Default)]
pub(super) struct State {
    loading: bool,
    history: Option<Result<History, ReadError>>,
    selected: Option<usize>,
    compare: (usize, usize),
    deps_for: Option<String>,
    deps: Option<Result<Vec<DependencyInfo>, String>>,
    events: Option<Result<Vec<WinEvent>, String>>,
    events_loading: bool,
}

impl State {
    pub(super) fn set_history(&mut self, result: Result<History, ReadError>) {
        self.loading = false;
        if let Ok(history) = &result {
            let last = history.boots.len().saturating_sub(1);
            self.selected = (!history.boots.is_empty()).then_some(last);
            self.compare = (last.saturating_sub(1), last);
        }
        self.history = Some(result);
        self.events = None;
    }

    pub(super) fn set_deps(&mut self, name: String, result: Result<Vec<DependencyInfo>, String>) {
        self.deps_for = Some(name);
        self.deps = Some(result);
    }

    pub(super) fn set_events(&mut self, result: Result<Vec<WinEvent>, String>) {
        self.events_loading = false;
        self.events = Some(result);
    }
}

fn seconds(ms: u64) -> String {
    format!("{:.1} s", ms as f64 / 1000.0)
}

fn local_label(time: &str) -> String {
    time.replace('T', " ").chars().take(16).collect::<String>() + " UTC"
}

impl OptiUi {
    pub(super) fn xray_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        if self.xray.history.is_none() && !self.xray.loading {
            self.load_boot_history(ctx);
        }
        kit::page_header(
            ui,
            "Startup X-Ray",
            "Windows' own record of how long recent boots took and what slowed them. Optimaxer reads it; it does not time boots itself.",
        );
        ui.horizontal_wrapped(|ui| {
            if kit::secondary(ui, "Reload", !self.xray.loading).clicked() {
                self.load_boot_history(ctx);
            }
            if kit::ghost(ui, "Open Startup items", true).clicked() {
                self.set_page(ctx, Page::Startup);
            }
            if self.xray.loading {
                ui.label(
                    RichText::new("Reading the boot log…")
                        .size(T_SMALL)
                        .color(t.subtle),
                );
            }
        });
        ui.add_space(S2);
        let history = match &self.xray.history {
            None => return,
            Some(Err(ReadError::AccessDenied)) => {
                let mut relaunch = false;
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Windows only lets administrators read the boot log")
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    ui.label(
                        RichText::new("Optimaxer would restart with administrator rights, which Windows asks you to approve. Reading the log is all this page does; nothing is changed.")
                            .size(T_SMALL)
                            .color(t.muted),
                    );
                    ui.add_space(S1);
                    relaunch = kit::primary(ui, "Restart as administrator", true).clicked();
                });
                if relaunch {
                    if hw::relaunch_as_admin() {
                        self.events.push(OptiEvent::Close);
                    } else {
                        self.events.push(OptiEvent::Toast(
                            "Administrator access was not granted, so Startup X-Ray stays unavailable.".into(),
                        ));
                    }
                }
                return;
            }
            Some(Err(ReadError::Other(error))) => {
                ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
                return;
            }
            Some(Ok(history)) => history.clone(),
        };
        if history.boots.is_empty() {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Windows has no boot records to show. Boot logging may be off, or the log was cleared.")
                        .size(T_SMALL)
                        .color(t.muted),
                );
            });
            return;
        }
        self.xray_trend(ui, &history);
        ui.add_space(S2);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!(
                    "Last {} boots, oldest to newest",
                    history.boots.len()
                ))
                .font(FontId::new(T_HEADING, semibold()))
                .color(t.text),
            );
            if let Some(clicked) = boot_chart(ui, &history.boots, self.xray.selected) {
                self.xray.selected = Some(clicked);
                self.xray.events = None;
            }
            ui.label(
                RichText::new("Click a bar to inspect that boot. Bars Windows itself marked as slower than its baseline are highlighted.")
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
        });
        ui.add_space(S2);
        if let Some(index) = self.xray.selected.filter(|i| *i < history.boots.len()) {
            self.xray_boot_detail(ui, ctx, &history, index);
            ui.add_space(S2);
        }
        self.xray_recurring(ui, &history);
        ui.add_space(S2);
        self.xray_compare(ui, &history.boots);
        ui.add_space(S2);
        self.xray_changes(ui, &history.boots);
        ui.add_space(S3);
    }

    fn load_boot_history(&mut self, ctx: &egui::Context) {
        self.xray.loading = true;
        let worker = self.worker(ctx);
        std::thread::spawn(move || worker.send(Msg::XrayHistory(bootlog::read_history())));
    }

    fn xray_trend(&self, ui: &mut Ui, history: &History) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(latest) = history.boots.last() else {
                return;
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(seconds(latest.boot_ms))
                        .font(FontId::new(theme::T_DISPLAY, semibold()))
                        .color(t.text),
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("most recent boot")
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    ui.label(
                        RichText::new(local_label(&latest.time))
                            .size(T_SMALL)
                            .color(t.muted),
                    );
                });
            });
            match bootlog::latest_trend(&history.boots) {
                Some(trend) => {
                    let (tone, text) = match trend.verdict {
                        Verdict::Slower => (
                            Tone::Warn,
                            format!(
                                "Slower than before: {} against a median of {} over the {} boots before it.",
                                seconds(trend.latest_ms),
                                seconds(trend.median_before_ms),
                                trend.earlier_boots
                            ),
                        ),
                        Verdict::Faster => (
                            Tone::Good,
                            format!(
                                "Faster than before: {} against a median of {} over the {} boots before it.",
                                seconds(trend.latest_ms),
                                seconds(trend.median_before_ms),
                                trend.earlier_boots
                            ),
                        ),
                        Verdict::Similar => (
                            Tone::Neutral,
                            format!(
                                "Within normal variation: {} against a median of {} over the {} boots before it.",
                                seconds(trend.latest_ms),
                                seconds(trend.median_before_ms),
                                trend.earlier_boots
                            ),
                        ),
                    };
                    ui.horizontal_wrapped(|ui| {
                        kit::chip(ui, "Trend", tone);
                        ui.add(
                            egui::Label::new(RichText::new(text).size(T_SMALL).color(t.muted))
                                .wrap(),
                        );
                    });
                }
                None => {
                    ui.label(
                        RichText::new("Four or more boots are needed before a trend is stated.")
                            .size(T_SMALL)
                            .color(t.subtle),
                    );
                }
            }
            if latest.windows_flagged_slow == Some(true) {
                ui.label(
                    RichText::new("Windows itself marked this boot as slower than its baseline.")
                        .size(T_SMALL)
                        .color(t.warn),
                );
            }
        });
    }

    fn xray_boot_detail(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        history: &History,
        index: usize,
    ) {
        let t = tokens(ctx);
        let boot = &history.boots[index];
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("Boot of {}", local_label(&boot.time)))
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            let mut facts = vec![format!("total {}", seconds(boot.boot_ms))];
            if let Some(ms) = boot.main_path_ms {
                facts.push(format!("until the desktop {}", seconds(ms)));
            }
            if let Some(ms) = boot.post_boot_ms {
                facts.push(format!("settling afterwards {}", seconds(ms)));
            }
            if let Some(apps) = boot.startup_apps {
                facts.push(format!("{apps} startup apps"));
            }
            ui.label(
                RichText::new(facts.join(" · "))
                    .size(T_SMALL)
                    .color(t.muted),
            );
            let items = bootlog::slow_for_boot(history, index);
            ui.add_space(S1);
            if items.is_empty() {
                ui.label(
                    RichText::new("Windows did not flag any application, service or driver as slowing this boot.")
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
            let mut deps_request = None;
            for item in items.iter().take(12) {
                ui.horizontal_wrapped(|ui| {
                    kit::chip(ui, item.kind.label(), Tone::Neutral);
                    let label = if item.friendly.is_empty() || item.friendly == item.name {
                        item.name.clone()
                    } else {
                        format!("{} ({})", item.friendly, item.name)
                    };
                    ui.label(RichText::new(label).size(T_SMALL).color(t.text));
                    ui.label(
                        RichText::new(format!(
                            "delayed boot by {}, took {}",
                            seconds(item.degradation_ms),
                            seconds(item.total_ms)
                        ))
                        .size(T_CAPTION)
                        .color(t.subtle),
                    );
                    if item.kind == SlowKind::Service {
                        if crate::opti::services::is_protected(&self.data, &item.name) {
                            kit::chip(ui, "Protected by Optimaxer", Tone::Good);
                        }
                        if kit::ghost(ui, "Show dependencies", true).clicked() {
                            deps_request = Some(item.name.clone());
                        }
                    }
                });
            }
            if let Some(name) = deps_request {
                let worker = self.worker(ctx);
                std::thread::spawn(move || {
                    let result =
                        crate::opti::services::dependency_info(std::slice::from_ref(&name));
                    worker.send(Msg::XrayDeps(name, result));
                });
            }
            self.xray_deps(ui);
            ui.add_space(S1);
            self.xray_events(ui, ctx, boot);
        });
    }

    fn xray_deps(&self, ui: &mut Ui) {
        let t = tokens(ui.ctx());
        let (Some(name), Some(result)) = (&self.xray.deps_for, &self.xray.deps) else {
            return;
        };
        match result {
            Err(error) => {
                ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
            }
            Ok(list) => {
                for info in list {
                    let join = |items: &[String]| {
                        if items.is_empty() {
                            "nothing".to_owned()
                        } else {
                            items.join(", ")
                        }
                    };
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{name} needs: {}. Needed by: {}.",
                                join(&info.requires),
                                join(&info.required_by)
                            ))
                            .size(T_SMALL)
                            .color(t.muted),
                        )
                        .wrap(),
                    );
                }
                ui.label(
                    RichText::new("Turning off a service other services need can stop them starting. The Services page shows the same list before any change.")
                        .size(T_CAPTION)
                        .color(t.subtle),
                );
            }
        }
    }

    fn xray_events(&mut self, ui: &mut Ui, ctx: &egui::Context, boot: &BootRecord) {
        let t = tokens(ctx);
        let Some(end) = bootlog::epoch_seconds(&boot.time).and_then(|s| u64::try_from(s).ok())
        else {
            return;
        };
        let from = end.saturating_sub(boot.boot_ms / 1000 + 60);
        let to = end + 300;
        let loading = self.xray.events_loading;
        if kit::secondary(
            ui,
            if loading {
                "Reading the System log…"
            } else {
                "Check the System log around this boot"
            },
            !loading,
        )
        .clicked()
        {
            self.xray.events_loading = true;
            let worker = self.worker(ctx);
            std::thread::spawn(move || {
                worker.send(Msg::XrayEvents(blackbox::windows_events(from, to)));
            });
        }
        match &self.xray.events {
            None => {}
            Some(Err(error)) => {
                ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
            }
            Some(Ok(events)) if events.is_empty() => {
                ui.label(
                    RichText::new(
                        "No critical, error or warning entries in the System log around this boot.",
                    )
                    .size(T_SMALL)
                    .color(t.muted),
                );
            }
            Some(Ok(events)) => {
                for event in events {
                    ui.horizontal_wrapped(|ui| {
                        kit::chip(
                            ui,
                            event.level,
                            if event.level == "Warning" {
                                Tone::Warn
                            } else {
                                Tone::Bad
                            },
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} UTC · {} · event {}",
                                event.time, event.provider, event.id
                            ))
                            .size(T_SMALL)
                            .color(t.text),
                        );
                    });
                }
                ui.label(
                    RichText::new("Timing alone does not show an entry slowed the boot. Look the provider and event ID up before drawing a conclusion.")
                        .size(T_CAPTION)
                        .color(t.subtle),
                );
            }
        }
    }

    fn xray_recurring(&self, ui: &mut Ui, history: &History) {
        let t = tokens(ui.ctx());
        let rows = bootlog::recurring(history);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Flagged again and again")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            if rows.is_empty() {
                ui.label(
                    RichText::new("Nothing Windows flagged on more than one boot. One appearance is not a pattern.")
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
            for row in rows.iter().take(10) {
                ui.horizontal_wrapped(|ui| {
                    kit::chip(ui, row.kind.label(), Tone::Neutral);
                    ui.label(RichText::new(&row.name).size(T_SMALL).color(t.text));
                    ui.label(
                        RichText::new(format!(
                            "flagged {} times, delayed boot by {} on average, {} at worst",
                            row.boots,
                            seconds(row.average_degradation_ms),
                            seconds(row.worst_degradation_ms)
                        ))
                        .size(T_CAPTION)
                        .color(t.subtle),
                    );
                });
            }
        });
    }

    fn xray_compare(&mut self, ui: &mut Ui, boots: &[BootRecord]) {
        let t = tokens(ui.ctx());
        if boots.len() < 2 {
            return;
        }
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Compare two boots")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            let (mut a, mut b) = self.xray.compare;
            a = a.min(boots.len() - 1);
            b = b.min(boots.len() - 1);
            ui.horizontal_wrapped(|ui| {
                for (id, value) in [("xray-a", &mut a), ("xray-b", &mut b)] {
                    egui::ComboBox::from_id_salt(id)
                        .selected_text(local_label(&boots[*value].time))
                        .show_ui(ui, |ui| {
                            for (i, boot) in boots.iter().enumerate() {
                                ui.selectable_value(value, i, local_label(&boot.time));
                            }
                        });
                }
            });
            self.xray.compare = (a, b);
            let rows: [(&str, Option<u64>, Option<u64>); 3] = [
                ("Total", Some(boots[a].boot_ms), Some(boots[b].boot_ms)),
                (
                    "Until the desktop",
                    boots[a].main_path_ms,
                    boots[b].main_path_ms,
                ),
                (
                    "Settling afterwards",
                    boots[a].post_boot_ms,
                    boots[b].post_boot_ms,
                ),
            ];
            egui::Grid::new("xray-compare")
                .num_columns(4)
                .spacing(vec2(S5, S1 + 2.0))
                .show(ui, |ui| {
                    for (name, first, second) in rows {
                        ui.label(RichText::new(name).size(T_SMALL).color(t.muted));
                        match (first, second) {
                            (Some(first), Some(second)) => {
                                ui.label(RichText::new(seconds(first)).size(T_SMALL).color(t.text));
                                ui.label(
                                    RichText::new(seconds(second)).size(T_SMALL).color(t.text),
                                );
                                let delta = second as i64 - first as i64;
                                ui.label(
                                    RichText::new(format!(
                                        "{}{:.1} s",
                                        if delta > 0 {
                                            "+"
                                        } else if delta < 0 {
                                            "−"
                                        } else {
                                            ""
                                        },
                                        delta.unsigned_abs() as f64 / 1000.0
                                    ))
                                    .size(T_SMALL)
                                    .color(t.text),
                                );
                            }
                            _ => {
                                ui.label(
                                    RichText::new("Not recorded").size(T_SMALL).color(t.subtle),
                                );
                                ui.label("");
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            ui.label(
                RichText::new("The change column is the second boot minus the first. Single boots vary, so a small difference says little.")
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
        });
    }

    fn xray_changes(&self, ui: &mut Ui, boots: &[BootRecord]) {
        let t = tokens(ui.ctx());
        let changes: Vec<&OperationEntry> = self
            .operations
            .iter()
            .filter(|entry| is_startup_change(&entry.title))
            .collect();
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Startup changes made here")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            if changes.is_empty() {
                ui.label(
                    RichText::new("Turning startup items on or off from the Startup page will be listed here with the boots recorded before and after.")
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
            for entry in changes.iter().rev().take(10) {
                ui.label(
                    RichText::new(format!("{} · {}", local_label(&entry.time), entry.title))
                        .font(FontId::new(T_SMALL, semibold()))
                        .color(t.text),
                );
                for detail in entry.details.iter().take(6) {
                    ui.label(RichText::new(detail.trim()).size(T_CAPTION).color(t.muted));
                }
                let line = match bootlog::around_change(boots, &entry.time) {
                    Some(c) => {
                        let before = bootlog::median_ms(&c.before_ms).unwrap_or(0);
                        let after = bootlog::median_ms(&c.after_ms).unwrap_or(0);
                        let thin = c.before_ms.len().min(c.after_ms.len()) < 3;
                        format!(
                            "Median boot before {} ({} boots), after {} ({} boots).{}",
                            seconds(before),
                            c.before_ms.len(),
                            seconds(after),
                            c.after_ms.len(),
                            if thin {
                                " Fewer than three boots on one side: not conclusive."
                            } else {
                                ""
                            }
                        )
                    }
                    None => "No recorded boots on both sides of this change yet.".to_owned(),
                };
                ui.add(
                    egui::Label::new(RichText::new(line).size(T_CAPTION).color(t.subtle)).wrap(),
                );
                ui.add_space(S1);
            }
        });
    }
}

fn is_startup_change(title: &str) -> bool {
    matches!(title, "Enabling startup items" | "Disabling startup items")
}

/// Bar chart of boot durations; returns the bar clicked, if any.
fn boot_chart(ui: &mut Ui, boots: &[BootRecord], selected: Option<usize>) -> Option<usize> {
    let t = tokens(ui.ctx());
    let size = vec2(ui.available_width(), 120.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(t.ctl_radius()), t.surface2);
    let count = boots.len().max(1);
    let top = boots.iter().map(|b| b.boot_ms).max().unwrap_or(1).max(1) as f32;
    let slot = (rect.width() - 16.0) / count as f32;
    let bar = (slot * 0.7).clamp(3.0, 40.0);
    let height = rect.height() - 28.0;
    for (i, boot) in boots.iter().enumerate() {
        let h = (boot.boot_ms as f32 / top * height).max(2.0);
        let x = rect.left() + 8.0 + slot * i as f32 + (slot - bar) / 2.0;
        let bar_rect =
            egui::Rect::from_min_size(egui::pos2(x, rect.bottom() - 8.0 - h), vec2(bar, h));
        let color = if selected == Some(i) {
            t.brand_text
        } else if boot.windows_flagged_slow == Some(true) {
            t.warn
        } else {
            t.muted
        };
        painter.rect_filled(bar_rect, CornerRadius::same(2), color);
    }
    painter.text(
        rect.left_top() + vec2(8.0, 4.0),
        egui::Align2::LEFT_TOP,
        seconds(top as u64),
        FontId::new(T_CAPTION, egui::FontFamily::Proportional),
        t.subtle,
    );
    let index_at = |pos: egui::Pos2| {
        let i = ((pos.x - rect.left() - 8.0) / slot).floor();
        (i >= 0.0 && (i as usize) < boots.len()).then_some(i as usize)
    };
    if let Some(i) = response.hover_pos().and_then(index_at) {
        let boot = &boots[i];
        response.clone().on_hover_text_at_pointer(format!(
            "{}\n{}",
            local_label(&boot.time),
            seconds(boot.boot_ms)
        ));
    }
    if response.clicked() {
        return response.interact_pointer_pos().and_then(index_at);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_startup_operations_count_as_startup_changes() {
        assert!(is_startup_change("Enabling startup items"));
        assert!(is_startup_change("Disabling startup items"));
        assert!(!is_startup_change("Cleaning"));
    }

    #[test]
    fn durations_and_times_are_formatted_for_reading() {
        assert_eq!(seconds(71_234), "71.2 s");
        assert_eq!(local_label("2026-10-30T12:00:01"), "2026-10-30 12:00 UTC");
    }

    #[test]
    fn a_loaded_history_selects_the_newest_boot_and_compares_the_last_two() {
        let mut state = State::default();
        let boot = |time: &str| BootRecord {
            time: time.into(),
            boot_ms: 1,
            main_path_ms: None,
            post_boot_ms: None,
            startup_apps: None,
            windows_flagged_slow: None,
        };
        state.set_history(Ok(History {
            boots: vec![
                boot("2026-10-01T00:00:00"),
                boot("2026-10-02T00:00:00"),
                boot("2026-10-03T00:00:00"),
            ],
            slow: vec![],
        }));
        assert_eq!(state.selected, Some(2));
        assert_eq!(state.compare, (1, 2));
        state.set_history(Err(ReadError::AccessDenied));
        assert!(matches!(state.history, Some(Err(ReadError::AccessDenied))));
    }
}
