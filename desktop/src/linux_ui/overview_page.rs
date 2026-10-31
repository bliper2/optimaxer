//! Overview: live gauges and a plain list of what the machine reports. A value the kernel does not give is left out.

use super::*;
use crate::linux::fmt_size;

impl OptiUi {
    pub(super) fn overview_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Overview",
            "Live readings from /proc and /sys. Nothing here is estimated: a reading the machine does not provide is not shown.",
        );
        let snap = self.sampler.snapshot.clone();
        ui.horizontal_wrapped(|ui| {
            let cpu = snap.cpu_percent;
            kit::gauge(
                ui,
                132.0,
                cpu.unwrap_or(0.0) / 100.0,
                kit::load_color(&t, cpu.unwrap_or(0.0)),
                &cpu.map_or("–".to_string(), |v| format!("{v:.0}%")),
                "CPU",
            );
            let ram = snap.ram_fraction();
            kit::gauge(
                ui,
                132.0,
                ram.unwrap_or(0.0),
                kit::load_color(&t, ram.unwrap_or(0.0) * 100.0),
                &ram.map_or("–".to_string(), |v| format!("{:.0}%", v * 100.0)),
                "Memory",
            );
            let disk = snap.disk_fraction();
            kit::gauge(
                ui,
                132.0,
                disk.unwrap_or(0.0),
                kit::load_color(&t, disk.unwrap_or(0.0) * 100.0),
                &disk.map_or("–".to_string(), |v| format!("{:.0}%", v * 100.0)),
                "Disk (/)",
            );
        });
        ui.add_space(S3);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            Self::heading(ui, "This machine");
            let mut rows: Vec<(&str, String)> = Vec::new();
            if let Some(v) = &self.info.distro {
                rows.push(("Distribution", v.clone()));
            }
            if let Some(v) = &self.info.kernel {
                rows.push(("Kernel", v.clone()));
            }
            if let Some(v) = &self.info.hostname {
                rows.push(("Host name", v.clone()));
            }
            if let Some(v) = &self.info.cpu {
                let threads = if self.info.threads > 0 {
                    format!(" ({} threads)", self.info.threads)
                } else {
                    String::new()
                };
                rows.push(("Processor", format!("{v}{threads}")));
            }
            if let Some(m) = snap.mem {
                rows.push((
                    "Memory",
                    format!("{} used of {}", fmt_size(m.used()), fmt_size(m.total)),
                ));
                if m.swap_total > 0 {
                    rows.push((
                        "Swap",
                        format!(
                            "{} used of {}",
                            fmt_size(m.swap_used()),
                            fmt_size(m.swap_total)
                        ),
                    ));
                }
            }
            if let Some((used, total)) = snap.disk {
                rows.push((
                    "Disk (/)",
                    format!("{} used of {}", fmt_size(used), fmt_size(total)),
                ));
            }
            for gpu in &self.info.gpus {
                rows.push(("Graphics", gpu.clone()));
            }
            if let Some(temp) = snap.cpu_temp_c {
                rows.push(("CPU temperature", format!("{temp:.0} °C")));
            }
            if let Some((a, b, c)) = snap.load {
                rows.push(("Load average", format!("{a:.2}  {b:.2}  {c:.2}")));
            }
            if let Some(up) = snap.uptime {
                rows.push(("Uptime", sysmon::format_uptime(up)));
            }
            if let Some(b) = snap.battery {
                rows.push((
                    "Battery",
                    format!(
                        "{}%{}",
                        b.percent,
                        if b.charging { ", charging" } else { "" }
                    ),
                ));
            }
            egui::Grid::new("linux-overview-info")
                .num_columns(2)
                .spacing([S4, S1])
                .show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(RichText::new(k).size(T_SMALL).color(t.subtle));
                        ui.add(
                            egui::Label::new(RichText::new(v).size(T_SMALL).color(t.text)).wrap(),
                        );
                        ui.end_row();
                    }
                });
        });
        ui.add_space(S2);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            Self::heading(ui, "Health");
            match &self.failed_units {
                Some(failed) if failed.is_empty() => {
                    ui.horizontal(|ui| {
                        Self::badge(ui, "OK", Tone::Good);
                        ui.label(
                            RichText::new("No failed systemd units.")
                                .size(T_SMALL)
                                .color(t.muted),
                        );
                    });
                }
                Some(failed) => {
                    ui.horizontal_wrapped(|ui| {
                        Self::badge(ui, &format!("{} failed", failed.len()), Tone::Warn);
                        ui.label(
                            RichText::new(failed.join(", "))
                                .size(T_SMALL)
                                .color(t.muted),
                        );
                    });
                    ui.label(RichText::new("Open Services to see them, or run `systemctl status <unit>` in a terminal.").size(T_CAPTION).color(t.subtle));
                }
                None => {
                    ui.label(
                        RichText::new("systemd is not reporting here, so unit health is unknown.")
                            .size(T_SMALL)
                            .color(t.subtle),
                    );
                }
            }
            let applied = self.tweak_status.values().filter(|on| **on).count();
            if self.tweaks_loaded {
                ui.label(
                    RichText::new(format!(
                        "{applied} of {} available tweaks are in effect.",
                        self.tweak_available.len()
                    ))
                    .size(T_SMALL)
                    .color(t.muted),
                );
            }
            ui.horizontal(|ui| {
                if kit::ghost(ui, "Re-check", true).clicked() {
                    self.refresh_health(ctx);
                    self.refresh_tweaks(ctx);
                }
            });
        });
    }
}
