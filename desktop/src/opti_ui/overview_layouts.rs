//! Alternative Overview layouts that go with the glass palettes: a gauge dashboard (large ring gauges and quick-action
//! tiles) and a compact list (grouped rows with thin bars, like a settings list). Both show only measured values;
//! a reading Windows does not give is marked unavailable, never drawn as zero.

use super::*;
use crate::opti::sysmon::{Snapshot, SysInfo};

struct Reading {
    title: &'static str,
    subtitle: String,
    /// 0..=1 when measured.
    fraction: Option<f32>,
    value: String,
    caption: String,
}

fn readings(s: &Snapshot, info: &SysInfo) -> Vec<Reading> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let or_unknown = |text: &str, fallback: &str| {
        if text.is_empty() {
            fallback.to_owned()
        } else {
            text.to_owned()
        }
    };
    vec![
        Reading {
            title: "Processor",
            subtitle: or_unknown(&info.cpu, "Reading system information…"),
            fraction: s.cpu_available.then_some(s.cpu / 100.0),
            value: if s.cpu_available {
                format!("{:.0}%", s.cpu)
            } else {
                "n/a".into()
            },
            caption: if s.cpu_available {
                format!("{} threads", info.threads)
            } else {
                "Unavailable".into()
            },
        },
        Reading {
            title: "Graphics",
            subtitle: or_unknown(
                &info.gpus.first().cloned().unwrap_or_default(),
                "No graphics adapter reported",
            ),
            fraction: s.gpu_busiest_engine.map(|v| v / 100.0),
            value: s
                .gpu_busiest_engine
                .map_or("n/a".into(), |v| format!("{v:.0}%")),
            caption: s
                .gpu_busiest_engine
                .map_or("Unavailable".into(), |_| "engine".into()),
        },
        Reading {
            title: "Memory",
            subtitle: if s.ram_available {
                format!(
                    "{} of {}",
                    format_size(Some(s.ram_used)),
                    format_size(Some(s.ram_total))
                )
            } else {
                "Windows memory counters unavailable".into()
            },
            fraction: s.ram_available.then_some(s.ram_fraction()),
            value: if s.ram_available {
                format!("{:.0}%", s.ram_fraction() * 100.0)
            } else {
                "n/a".into()
            },
            caption: if s.ram_available {
                "in use".into()
            } else {
                "Unavailable".into()
            },
        },
        Reading {
            title: "Storage",
            subtitle: if s.disk_available {
                format!(
                    "{drive} {} of {}",
                    format_size(Some(s.disk_used)),
                    format_size(Some(s.disk_total))
                )
            } else {
                "Windows volume counters unavailable".into()
            },
            fraction: s.disk_available.then_some(s.disk_fraction()),
            value: if s.disk_available {
                format!("{:.0}%", s.disk_fraction() * 100.0)
            } else {
                "n/a".into()
            },
            caption: if s.disk_available {
                "full".into()
            } else {
                "Unavailable".into()
            },
        },
    ]
}

const SHORTCUTS: [(&str, &str, Page); 4] = [
    (
        "Optimize my PC",
        "Review hardware-aware recommendations",
        Page::SmartOptimize,
    ),
    (
        "Bottleneck Detective",
        "Find what is limiting this PC",
        Page::Detective,
    ),
    (
        "Black Box",
        "Record readings to investigate slowdowns",
        Page::BlackBox,
    ),
    ("Startup", "See what starts with Windows", Page::Startup),
];

impl OptiUi {
    /// Ring-gauge dashboard: four large readings, then four quick-action tiles.
    pub(super) fn overview_dashboard(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        s: &Snapshot,
        info: &SysInfo,
    ) {
        let t = tokens(ctx);
        let items = readings(s, info);
        let two_columns = ui.available_width() >= 640.0;
        let gap = S3;
        let width = if two_columns {
            ((ui.available_width() - gap) / 2.0).floor()
        } else {
            ui.available_width()
        };
        for pair in items.chunks(if two_columns { 2 } else { 1 }) {
            ui.horizontal_top(|ui| {
                for reading in pair {
                    ui.allocate_ui_with_layout(
                        vec2(width, 150.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            kit::card(ui, |ui| {
                                ui.set_width(width - 2.0 * S4 - 2.0);
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.set_width((width - 2.0 * S4 - 2.0 - 124.0).max(80.0));
                                        ui.label(
                                            RichText::new(reading.title)
                                                .font(FontId::new(T_HEADING, semibold()))
                                                .color(t.text),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&reading.subtitle)
                                                    .size(T_SMALL)
                                                    .color(t.muted),
                                            )
                                            .truncate(),
                                        );
                                        ui.add_space(S2);
                                        let (tone, label) = match reading.fraction {
                                            Some(f) if f >= 0.9 => (Tone::Warn, "Heavily used"),
                                            Some(_) => (Tone::Good, "Measured"),
                                            None => (Tone::Neutral, "Unavailable"),
                                        };
                                        kit::chip(ui, label, tone);
                                    });
                                    let color = reading
                                        .fraction
                                        .map_or(t.muted, |f| kit::load_color(&t, f * 100.0));
                                    kit::gauge(
                                        ui,
                                        112.0,
                                        reading.fraction.unwrap_or(0.0),
                                        color,
                                        &reading.value,
                                        &reading.caption,
                                    );
                                });
                            });
                        },
                    );
                }
            });
        }
        ui.add_space(S2);
        self.shortcut_tiles(ui, ctx, two_columns);
    }

    /// Compact list: one grouped card of measured rows with thin bars, then a list of shortcuts.
    pub(super) fn overview_list(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        s: &Snapshot,
        info: &SysInfo,
    ) {
        let t = tokens(ctx);
        let items = readings(s, info);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, reading) in items.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(reading.title)
                                .font(FontId::new(T_BODY, semibold()))
                                .color(t.text),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(&reading.subtitle)
                                    .size(T_CAPTION)
                                    .color(t.muted),
                            )
                            .truncate(),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(&reading.value)
                                .font(FontId::new(T_HEADING, semibold()))
                                .color(t.text),
                        );
                    });
                });
                let color = reading
                    .fraction
                    .map_or(t.muted, |f| kit::load_color(&t, f * 100.0));
                ui.add(
                    egui::ProgressBar::new(reading.fraction.unwrap_or(0.0).clamp(0.0, 1.0))
                        .fill(color)
                        .desired_height(4.0)
                        .corner_radius(2),
                );
            }
        });
        ui.add_space(S2);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, (title, detail, page)) in SHORTCUTS.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                let row = kit::row(ui, ("overview-shortcut", i), false, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(*title)
                                    .font(FontId::new(T_BODY, semibold()))
                                    .color(t.text),
                            );
                            ui.label(RichText::new(*detail).size(T_CAPTION).color(t.muted));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new("›").size(T_HEADING).color(t.subtle));
                        });
                    });
                });
                if ui
                    .interact(
                        row.response.rect,
                        ui.scope_id().with(("shortcut", i)),
                        Sense::click(),
                    )
                    .clicked()
                {
                    self.set_page(ctx, *page);
                }
            }
        });
    }

    fn shortcut_tiles(&mut self, ui: &mut Ui, ctx: &egui::Context, two_columns: bool) {
        let t = tokens(ctx);
        let gap = S3;
        let width = if two_columns {
            ((ui.available_width() - gap) / 2.0).floor()
        } else {
            ui.available_width()
        };
        for pair in SHORTCUTS.chunks(if two_columns { 2 } else { 1 }) {
            ui.horizontal_top(|ui| {
                for (title, detail, page) in pair {
                    ui.allocate_ui_with_layout(
                        vec2(width, 64.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            let card = kit::card(ui, |ui| {
                                ui.set_width(width - 2.0 * S4 - 2.0);
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(*title)
                                                .font(FontId::new(T_BODY, semibold()))
                                                .color(t.text),
                                        );
                                        ui.label(
                                            RichText::new(*detail).size(T_CAPTION).color(t.muted),
                                        );
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new("›").size(T_HEADING).color(t.subtle),
                                        );
                                    });
                                });
                            });
                            let response = ui.interact(
                                card.response.rect,
                                ui.scope_id().with(("tile", *title)),
                                Sense::click(),
                            );
                            if response.hovered() {
                                ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if response.clicked() {
                                self.set_page(ctx, *page);
                            }
                        },
                    );
                }
            });
        }
    }
}
