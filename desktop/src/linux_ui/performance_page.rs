//! Performance: rolling charts of the same readings as Overview.

use super::*;
use crate::linux::sysmon::{format_bytes_per_second, format_uptime};
use egui::vec2;

impl OptiUi {
    pub(super) fn performance_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        let cap = self.sampler.config().history_capacity();
        let secs = self.sampler.config().history_seconds();
        kit::page_header(
            ui,
            "Performance",
            &format!(
                "The last {secs} seconds. Sampling runs only while Overview or Performance is open; the interval and window are in Settings."
            ),
        );
        let snap = self.sampler.snapshot.clone();
        let width = (ui.available_width() - S4 * 2.0).max(240.0);
        let size = vec2(width, 96.0);

        let chart = |ui: &mut Ui,
                     title: &str,
                     value: String,
                     data: &std::collections::VecDeque<Option<f32>>,
                     color: Color32| {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(title)
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    ui.label(RichText::new(value).size(T_SMALL).color(t.muted));
                });
                kit::sparkline(ui, size, data, cap, color);
            });
            ui.add_space(S2);
        };

        chart(
            ui,
            "CPU",
            snap.cpu_percent
                .map_or("collecting…".into(), |v| format!("{v:.0}%")),
            &self.sampler.cpu,
            t.brand,
        );
        chart(
            ui,
            "Memory",
            snap.mem.map_or("unavailable".into(), |m| {
                format!(
                    "{} of {}",
                    crate::linux::fmt_size(m.used()),
                    crate::linux::fmt_size(m.total)
                )
            }),
            &self.sampler.ram,
            t.good,
        );
        chart(
            ui,
            "Disk activity",
            snap.disk_busy
                .map_or("collecting…".into(), |v| format!("{v:.0}% busy")),
            &self.sampler.disk,
            t.warn,
        );
        if snap.gpu_percent.is_some() {
            chart(
                ui,
                "GPU",
                snap.gpu_percent
                    .map_or(String::new(), |v| format!("{v:.0}%")),
                &self.sampler.gpu,
                t.brand_text,
            );
        }
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Network")
                        .font(FontId::new(T_BODY, semibold()))
                        .color(t.text),
                );
                let text = snap.network.map_or("collecting…".to_string(), |n| {
                    format!(
                        "down {}   up {}",
                        format_bytes_per_second(n.received_bytes_per_sec),
                        format_bytes_per_second(n.sent_bytes_per_sec)
                    )
                });
                ui.label(RichText::new(text).size(T_SMALL).color(t.muted));
            });
            kit::network_chart(ui, size, &self.sampler.network, cap, t.brand, t.good);
        });
        ui.add_space(S2);
        ui.horizontal_wrapped(|ui| {
            if let Some(temp) = snap.cpu_temp_c {
                ui.label(
                    RichText::new(format!("CPU {temp:.0} °C"))
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
            if let Some((a, b, c)) = snap.load {
                ui.label(
                    RichText::new(format!("Load {a:.2} / {b:.2} / {c:.2}"))
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
            if let Some(up) = snap.uptime {
                ui.label(
                    RichText::new(format!("Up {}", format_uptime(up)))
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
        });
        if snap.gpu_percent.is_none() {
            ui.label(RichText::new("GPU load is shown for AMD graphics (amdgpu). Other drivers do not expose a load value without vendor tools, so none is shown.").size(T_CAPTION).color(t.subtle));
        }
    }
}
