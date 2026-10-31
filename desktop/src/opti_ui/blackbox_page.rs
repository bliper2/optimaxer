//! Black Box page: the opt-in flight recorder, its timeline, saturation findings, bookmarks and exports.

use super::*;
use crate::opti::blackbox::{self, Incident, Sample, Store, WinEvent};
use crate::opti::sysmon::{NetworkSample, format_bytes_per_second};

const RANGES: [(&str, u64); 4] = [
    ("15 min", 900),
    ("1 hour", 3_600),
    ("6 hours", 21_600),
    ("24 hours", 86_400),
];
const CHART_COLUMNS: usize = 240;
const REFRESH: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct View {
    store: Store,
    refreshed: Option<Instant>,
    range: usize,
    /// A period picked from an incident or bookmark, shown instead of the recent range.
    selected: Option<(u64, u64)>,
    note: String,
    confirm_delete: bool,
    start_failed: bool,
    events: Option<Result<Vec<WinEvent>, String>>,
    events_loading: bool,
    status: Option<String>,
}

impl View {
    pub(super) fn with_default_range() -> Self {
        Self {
            range: 1,
            ..Self::default()
        }
    }

    pub(super) fn set_events(&mut self, result: Result<Vec<WinEvent>, String>) {
        self.events = Some(result);
        self.events_loading = false;
    }
}

impl OptiUi {
    /// Starts or stops the recorder to match the saved preference. Runs every frame; it only acts on a change.
    pub(super) fn tick_blackbox(&mut self, settings: &Settings) {
        let interval = Duration::from_secs(blackbox_interval(settings));
        match (settings.blackbox_enabled, self.blackbox.as_ref()) {
            (true, None) if !self.blackbox_view.start_failed => {
                match blackbox::Recorder::start(blackbox::default_path(), interval) {
                    Ok(recorder) => self.blackbox = Some(recorder),
                    Err(error) => {
                        self.blackbox_view.start_failed = true;
                        self.blackbox_view.status =
                            Some(format!("The recorder could not start: {error}"));
                    }
                }
            }
            (true, Some(recorder)) => recorder.set_interval(interval),
            (false, Some(_)) => {
                // Dropping the recorder stops the thread and saves what it holds.
                self.blackbox = None;
                self.blackbox_view.refreshed = None;
            }
            (false, None) => self.blackbox_view.start_failed = false,
            _ => {}
        }
    }

    fn refresh_blackbox_view(&mut self, ctx: &egui::Context) {
        let stale = self
            .blackbox_view
            .refreshed
            .is_none_or(|at| at.elapsed() >= REFRESH);
        if stale {
            self.blackbox_view.store = match &self.blackbox {
                Some(recorder) => recorder.snapshot(),
                None => blackbox::load(&blackbox::default_path()).unwrap_or_default(),
            };
            self.blackbox_view.refreshed = Some(Instant::now());
        }
        ctx.request_repaint_after(REFRESH);
    }

    pub(super) fn blackbox_page(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        settings: &mut Settings,
    ) {
        let t = tokens(ctx);
        self.refresh_blackbox_view(ctx);
        kit::page_header(
            ui,
            "Black Box",
            "A local flight recorder for finding out when this PC became slow.",
        );
        self.blackbox_controls(ui, settings);
        let interval = blackbox_interval(settings);
        let store = std::mem::take(&mut self.blackbox_view.store);
        if store.samples.is_empty() {
            ui.add_space(S2);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                let text = if settings.blackbox_enabled {
                    "Recording has started. The first readings appear after one sampling interval."
                } else {
                    "Recording is off, so there is no history yet. Turn it on, use the PC as usual, and come back when it feels slow."
                };
                ui.label(RichText::new(text).size(T_SMALL).color(t.muted));
            });
            self.blackbox_view.store = store;
            return;
        }
        ui.add_space(S2);
        let now = crate::opti::now_secs();
        let (from, to) = self.blackbox_view.selected.unwrap_or_else(|| {
            (
                now.saturating_sub(RANGES[self.blackbox_view.range.min(3)].1),
                now,
            )
        });
        self.blackbox_range_bar(ui, from, to);
        let samples = store.between(from, to);
        ui.add_space(S2);
        self.blackbox_charts(ui, &samples, from, to);
        ui.add_space(S2);
        self.blackbox_summary(ui, &samples);
        let incidents = blackbox::incidents(&samples, interval);
        ui.add_space(S2);
        self.blackbox_findings(ui, &incidents);
        ui.add_space(S2);
        self.blackbox_bookmarks(ui, &store, from, to);
        ui.add_space(S2);
        self.blackbox_events(ui, ctx, from, to);
        ui.add_space(S2);
        self.blackbox_export(ui, &store, from, to, interval);
        ui.add_space(S3);
        self.blackbox_view.store = store;
    }

    fn blackbox_controls(&mut self, ui: &mut Ui, settings: &mut Settings) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut enabled = settings.blackbox_enabled;
            if ui
                .checkbox(&mut enabled, "Record performance history")
                .changed()
            {
                settings.blackbox_enabled = enabled;
                if let Err(error) = settings.save_checked() {
                    settings.blackbox_enabled = !enabled;
                    self.blackbox_view.status = Some(format!("The setting was not saved: {error}"));
                }
                self.blackbox_view.start_failed = false;
            }
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Off by default. While on, Optimaxer notes processor, memory, disk, network and graphics readings every few seconds and keeps the newest {} readings in {}. It records no process names, window titles or file names, and nothing leaves this PC. Recording runs only while Optimaxer is open.",
                        blackbox::MAX_SAMPLES,
                        blackbox::default_path().display()
                    ))
                    .size(T_CAPTION)
                    .color(t.subtle),
                )
                .wrap(),
            );
            ui.add_space(S1);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Sampling").size(T_SMALL).color(t.muted));
                let mut index = blackbox::INTERVAL_SECONDS
                    .iter()
                    .position(|s| *s == blackbox_interval(settings))
                    .unwrap_or(1);
                if kit::segmented(ui, &mut index, &["5 s", "10 s", "30 s"]) {
                    let previous = settings.blackbox_interval_seconds;
                    settings.blackbox_interval_seconds = blackbox::INTERVAL_SECONDS[index];
                    if let Err(error) = settings.save_checked() {
                        settings.blackbox_interval_seconds = previous;
                        self.blackbox_view.status =
                            Some(format!("The interval was not saved: {error}"));
                    }
                }
                let held = self.blackbox_view.store.samples.len();
                ui.label(
                    RichText::new(format!("{held} readings stored"))
                        .size(T_SMALL)
                        .color(t.muted),
                );
                if self.blackbox_view.confirm_delete {
                    if kit::secondary(ui, "Delete everything recorded", true).clicked() {
                        self.blackbox_view.status = Some(match self.delete_blackbox_history() {
                            Ok(()) => "The recorded history was deleted.".to_owned(),
                            Err(error) => error,
                        });
                        self.blackbox_view.confirm_delete = false;
                        self.blackbox_view.selected = None;
                    }
                    if kit::ghost(ui, "Keep it", true).clicked() {
                        self.blackbox_view.confirm_delete = false;
                    }
                } else if kit::secondary(ui, "Delete history", held > 0).clicked() {
                    self.blackbox_view.confirm_delete = true;
                }
            });
            if let Some(error) = self.blackbox.as_ref().and_then(blackbox::Recorder::error) {
                ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
            }
            if let Some(status) = &self.blackbox_view.status {
                ui.label(RichText::new(status).size(T_SMALL).color(t.muted));
            }
        });
    }

    fn delete_blackbox_history(&mut self) -> Result<(), String> {
        let result = match &self.blackbox {
            Some(recorder) => recorder.delete_history(),
            None => match std::fs::remove_file(blackbox::default_path()) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("Could not delete the black box history: {e}")),
            },
        };
        self.blackbox_view.store = Store::default();
        self.blackbox_view.refreshed = None;
        result
    }

    fn blackbox_range_bar(&mut self, ui: &mut Ui, from: u64, to: u64) {
        let t = tokens(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            if let Some((start, end)) = self.blackbox_view.selected {
                ui.label(
                    RichText::new(format!(
                        "Investigating {} to {} UTC",
                        short_time(start),
                        short_time(end)
                    ))
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.text),
                );
                if kit::secondary(ui, "Back to recent", true).clicked() {
                    self.blackbox_view.selected = None;
                    self.blackbox_view.events = None;
                }
            } else {
                let mut index = self.blackbox_view.range.min(RANGES.len() - 1);
                let labels: Vec<&str> = RANGES.iter().map(|(label, _)| *label).collect();
                if kit::segmented(ui, &mut index, &labels) {
                    self.blackbox_view.range = index;
                    self.blackbox_view.events = None;
                }
                ui.label(
                    RichText::new(format!("{} to {} UTC", short_time(from), short_time(to)))
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
        });
    }

    fn blackbox_charts(&self, ui: &mut Ui, samples: &[Sample], from: u64, to: u64) {
        let t = tokens(ui.ctx());
        let percent = |ui: &mut Ui, title: &str, pick: fn(&Sample) -> Option<f32>| {
            let data = blackbox::buckets(samples, from, to, CHART_COLUMNS, pick);
            let latest = samples.iter().rev().find_map(pick);
            Self::performance_load_chart(
                ui,
                title,
                latest,
                &data,
                CHART_COLUMNS,
                latest.map_or(t.muted, |v| kit::load_color(&t, v)),
            );
        };
        percent(ui, "Processor (average over each interval)", |s| s.cpu);
        ui.add_space(S2);
        percent(ui, "Memory in use", |s| s.ram);
        ui.add_space(S2);
        percent(ui, "Busiest GPU engine", |s| s.gpu);
        ui.add_space(S2);
        let disk = blackbox::buckets(samples, from, to, CHART_COLUMNS, |s| s.disk);
        let down = blackbox::buckets(samples, from, to, CHART_COLUMNS, |s| s.down);
        let up = blackbox::buckets(samples, from, to, CHART_COLUMNS, |s| s.up);
        let network: std::collections::VecDeque<Option<NetworkSample>> = down
            .iter()
            .zip(&up)
            .map(|(down, up)| {
                (down.is_some() || up.is_some()).then(|| NetworkSample {
                    received_bytes_per_sec: down.unwrap_or(0.0),
                    sent_bytes_per_sec: up.unwrap_or(0.0),
                })
            })
            .collect();
        let width = ui.available_width();
        kit::card(ui, |ui| {
            ui.set_width(width - 2.0 * S4 - 2.0);
            ui.label(
                RichText::new("Disk throughput (peak per column)")
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.text),
            );
            kit::rate_chart(
                ui,
                vec2(ui.available_width(), 86.0),
                &disk,
                CHART_COLUMNS,
                t.brand_text,
            );
        });
        ui.add_space(S2);
        kit::card(ui, |ui| {
            ui.set_width(width - 2.0 * S4 - 2.0);
            ui.label(
                RichText::new("Adapter traffic (peak per column)")
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.text),
            );
            kit::network_chart(
                ui,
                vec2(ui.available_width(), 86.0),
                &network,
                CHART_COLUMNS,
                t.brand_text,
                t.good,
            );
        });
    }

    fn blackbox_summary(&self, ui: &mut Ui, samples: &[Sample]) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("This period")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            ui.add_space(S1);
            type Pick = fn(&Sample) -> Option<f32>;
            let rows: [(&str, bool, Pick); 6] = [
                ("Processor", false, |s| s.cpu),
                ("Memory", false, |s| s.ram),
                ("Disk", true, |s| s.disk),
                ("Network received", true, |s| s.down),
                ("Network sent", true, |s| s.up),
                ("GPU engine", false, |s| s.gpu),
            ];
            egui::Grid::new("blackbox-summary")
                .num_columns(4)
                .spacing(vec2(S5, S1 + 2.0))
                .show(ui, |ui| {
                    for (name, is_rate, pick) in rows {
                        let show = |v: f32| {
                            if is_rate {
                                format_bytes_per_second(v)
                            } else {
                                format!("{v:.0}%")
                            }
                        };
                        ui.label(RichText::new(name).size(T_SMALL).color(t.muted));
                        match blackbox::summarize(samples, pick) {
                            Some(s) => {
                                ui.label(
                                    RichText::new(format!("average {}", show(s.average)))
                                        .size(T_SMALL)
                                        .color(t.text),
                                );
                                ui.label(
                                    RichText::new(format!("peak {}", show(s.peak)))
                                        .size(T_SMALL)
                                        .color(t.text),
                                );
                                ui.label(
                                    RichText::new(format!("at {} UTC", short_time(s.peak_at)))
                                        .size(T_CAPTION)
                                        .color(t.subtle),
                                );
                            }
                            None => {
                                ui.label(
                                    RichText::new("No readings in this period")
                                        .size(T_SMALL)
                                        .color(t.subtle),
                                );
                                ui.label("");
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
        });
    }

    fn blackbox_findings(&mut self, ui: &mut Ui, incidents: &[Incident]) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Sustained saturation")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            if incidents.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "Processor and memory did not stay at or above {:.0}% for {} seconds in this period.",
                        blackbox::SATURATION_PERCENT,
                        blackbox::SUSTAINED_SECONDS
                    ))
                    .size(T_SMALL)
                    .color(t.muted),
                );
            }
            for incident in incidents {
                ui.horizontal_wrapped(|ui| {
                    kit::chip(ui, incident.metric, Tone::Warn);
                    ui.label(
                        RichText::new(format!(
                            "{} to {} UTC, peak {:.0}%",
                            short_time(incident.start),
                            short_time(incident.end),
                            incident.peak
                        ))
                        .size(T_SMALL)
                        .color(t.text),
                    );
                    if kit::secondary(ui, "Investigate", true).clicked() {
                        self.blackbox_view.selected =
                            Some((incident.start.saturating_sub(300), incident.end + 300));
                        self.blackbox_view.events = None;
                    }
                });
            }
            ui.label(
                RichText::new("A saturated resource shows that something was using it. It does not say which program, or whether it was a problem.")
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
        });
    }

    fn blackbox_bookmarks(&mut self, ui: &mut Ui, store: &Store, from: u64, to: u64) {
        let t = tokens(ui.ctx());
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Bookmarks")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.blackbox_view.note)
                        .hint_text("What just happened? (optional)")
                        .char_limit(blackbox::MAX_NOTE_CHARS)
                        .desired_width(260.0),
                );
                let recording = self.blackbox.is_some();
                if kit::secondary(ui, "Mark now", recording)
                    .on_disabled_hover_text("Turn recording on to add bookmarks")
                    .clicked()
                    && let Some(recorder) = &self.blackbox
                {
                    recorder.bookmark(&self.blackbox_view.note);
                    self.blackbox_view.note.clear();
                    self.blackbox_view.refreshed = None;
                }
            });
            let shown: Vec<_> = store
                .bookmarks
                .iter()
                .rev()
                .filter(|b| b.t >= from && b.t <= to)
                .take(10)
                .collect();
            if shown.is_empty() {
                ui.label(
                    RichText::new("No bookmarks in this period.")
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
            for bookmark in shown {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("{} UTC", short_time(bookmark.t)))
                            .size(T_CAPTION)
                            .color(t.subtle),
                    );
                    ui.label(RichText::new(&bookmark.note).size(T_SMALL).color(t.text));
                    if kit::ghost(ui, "Investigate", true).clicked() {
                        self.blackbox_view.selected =
                            Some((bookmark.t.saturating_sub(600), bookmark.t + 600));
                        self.blackbox_view.events = None;
                    }
                });
            }
        });
    }

    fn blackbox_events(&mut self, ui: &mut Ui, ctx: &egui::Context, from: u64, to: u64) {
        let t = tokens(ctx);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Windows events in this period")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            let loading = self.blackbox_view.events_loading;
            if kit::secondary(
                ui,
                if loading {
                    "Reading the System log…"
                } else {
                    "Check the System log"
                },
                !loading,
            )
            .clicked()
            {
                self.blackbox_view.events_loading = true;
                let worker = self.worker(ctx);
                std::thread::spawn(move || {
                    worker.send(Msg::BlackboxEvents(blackbox::windows_events(from, to)));
                });
            }
            match &self.blackbox_view.events {
                None => {
                    ui.label(
                        RichText::new(
                            "Reads critical, error and warning entries only. Nothing is changed.",
                        )
                        .size(T_CAPTION)
                        .color(t.subtle),
                    );
                }
                Some(Err(error)) => {
                    ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
                }
                Some(Ok(events)) if events.is_empty() => {
                    ui.label(
                        RichText::new(
                            "Windows logged no critical, error or warning entries in this period.",
                        )
                        .size(T_SMALL)
                        .color(t.muted),
                    );
                }
                Some(Ok(events)) => {
                    for event in events {
                        ui.horizontal_wrapped(|ui| {
                            let tone = if event.level == "Warning" {
                                Tone::Warn
                            } else {
                                Tone::Bad
                            };
                            kit::chip(ui, event.level, tone);
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
                        RichText::new("Timing alone does not show that an event caused a slowdown. Look the provider and event ID up before drawing a conclusion.")
                            .size(T_CAPTION)
                            .color(t.subtle),
                    );
                }
            }
        });
    }

    fn blackbox_export(&mut self, ui: &mut Ui, store: &Store, from: u64, to: u64, interval: u64) {
        ui.horizontal_wrapped(|ui| {
            if kit::secondary(ui, "Export report", true).clicked() {
                let text = blackbox::report(store, from, to, interval);
                self.save_blackbox_export("optimaxer-black-box-report.md", "Markdown", "md", text);
            }
            if kit::secondary(ui, "Export readings (CSV)", true).clicked() {
                let text = blackbox::csv(&store.between(from, to));
                self.save_blackbox_export("optimaxer-black-box-readings.csv", "CSV", "csv", text);
            }
        });
    }

    fn save_blackbox_export(&mut self, name: &str, kind: &str, extension: &str, text: String) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter(kind, &[extension])
            .save_file()
        else {
            return;
        };
        self.blackbox_view.status = Some(match std::fs::write(&path, text) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(error) => format!("Could not save the file: {error}"),
        });
    }
}

pub(super) fn blackbox_interval(settings: &Settings) -> u64 {
    if blackbox::INTERVAL_SECONDS.contains(&settings.blackbox_interval_seconds) {
        settings.blackbox_interval_seconds
    } else {
        blackbox::DEFAULT_INTERVAL_SECONDS
    }
}

/// `2026-10-30 12:00` from a unix time, in UTC.
fn short_time(secs: u64) -> String {
    crate::opti::iso_utc(secs)
        .replace('T', " ")
        .chars()
        .take(16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_saved_intervals_fall_back_to_the_default() {
        let mut settings = Settings {
            blackbox_interval_seconds: 7,
            ..Settings::default()
        };
        assert_eq!(
            blackbox_interval(&settings),
            blackbox::DEFAULT_INTERVAL_SECONDS
        );
        settings.blackbox_interval_seconds = 30;
        assert_eq!(blackbox_interval(&settings), 30);
    }

    #[test]
    fn recording_is_off_by_default() {
        assert!(!Settings::default().blackbox_enabled);
    }

    #[test]
    fn times_are_shown_to_the_minute_in_utc() {
        assert_eq!(short_time(1_792_715_831), "2026-10-23 00:37");
    }
}
