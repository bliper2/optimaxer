//! Performance page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn performance_page(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        settings: &Settings,
    ) {
        let t = tokens(ctx);
        self.tick_telemetry(ctx, settings);
        let snapshot = self.telemetry_frame.latest;
        kit::page_header(
            ui,
            "Performance",
            "Live CPU, memory, disk, adapter traffic and Windows GPU engine readings. GPU load is the busiest reported physical engine across adapters.",
        );
        if let Some(error) = &self.telemetry_error {
            ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
        }
        let processor = if snapshot.cpu_available {
            format!("{:.0}%", snapshot.cpu)
        } else {
            "Unavailable".into()
        };
        let processor_detail: String = if snapshot.cpu_available {
            "total logical CPU capacity".into()
        } else {
            "Windows CPU counters unavailable".into()
        };
        let memory = if snapshot.ram_available {
            format!("{:.1}%", snapshot.ram_fraction() * 100.0)
        } else {
            "Unavailable".into()
        };
        let memory_detail: String = if snapshot.ram_available {
            format!(
                "{} of {}",
                format_size(Some(snapshot.ram_used)),
                format_size(Some(snapshot.ram_total))
            )
        } else {
            "Windows memory counters unavailable".into()
        };
        let gpu = snapshot
            .gpu_busiest_engine
            .map_or_else(|| "Unavailable".into(), |value| format!("{value:.1}%"));
        let gpu_memory = snapshot.gpu_dedicated_memory_bytes.map_or_else(
            || "Dedicated GPU memory unavailable".into(),
            |bytes| format!("Dedicated GPU memory in use: {}", format_size(Some(bytes))),
        );
        let (network, network_detail): (String, String) = if let Some(sample) = snapshot.network {
            (
                format!(
                    "{} / {}",
                    crate::opti::sysmon::format_bytes_per_second(sample.received_bytes_per_sec),
                    crate::opti::sysmon::format_bytes_per_second(sample.sent_bytes_per_sec)
                ),
                "aggregate active adapters".into(),
            )
        } else {
            (
                if snapshot.network_available {
                    "Collecting first sample".into()
                } else {
                    "Unavailable".into()
                },
                "Windows adapter counters".into(),
            )
        };
        let metrics = [
            ("Processor", processor.as_str(), processor_detail.as_str()),
            ("Memory", memory.as_str(), memory_detail.as_str()),
            (
                "Adapter receive / send",
                network.as_str(),
                network_detail.as_str(),
            ),
            ("Busiest GPU engine", gpu.as_str(), gpu_memory.as_str()),
        ];
        if ui.available_width() < 960.0 {
            for (title, value, detail) in metrics {
                Self::performance_metric_card(ui, title, value, detail);
                ui.add_space(S2);
            }
        } else {
            ui.columns(4, |columns| {
                for (column, (title, value, detail)) in columns.iter_mut().zip(metrics) {
                    Self::performance_metric_card(column, title, value, detail);
                }
            });
        }
        ui.add_space(S2);
        let history_seconds = self.telemetry_config.history_seconds();
        let history_capacity = self.telemetry_config.history_capacity();
        let load_charts = [
            (
                format!("Processor, last {history_seconds} seconds"),
                &self.telemetry_frame.cpu_history,
                snapshot.cpu_available.then_some(snapshot.cpu),
                kit::load_color(&t, snapshot.cpu),
            ),
            (
                format!("Memory, last {history_seconds} seconds"),
                &self.telemetry_frame.ram_history,
                snapshot
                    .ram_available
                    .then_some(snapshot.ram_fraction() * 100.0),
                kit::load_color(&t, snapshot.ram_fraction() * 100.0),
            ),
            (
                format!("Busiest GPU engine, last {history_seconds} seconds"),
                &self.telemetry_frame.gpu_history,
                snapshot.gpu_busiest_engine,
                snapshot
                    .gpu_busiest_engine
                    .map_or(t.muted, |value| kit::load_color(&t, value)),
            ),
        ];
        if ui.available_width() < 900.0 {
            for (title, history, value, color) in load_charts {
                Self::performance_load_chart(ui, &title, value, history, history_capacity, color);
                ui.add_space(S2);
            }
        } else {
            ui.columns(3, |columns| {
                for (column, (title, history, value, color)) in columns.iter_mut().zip(load_charts)
                {
                    Self::performance_load_chart(
                        column,
                        &title,
                        value,
                        history,
                        history_capacity,
                        color,
                    );
                }
            });
        }
        ui.add_space(S2);
        let network_history = &self.telemetry_frame.network_history;
        let disk_history = &self.telemetry_frame.disk_history;
        let draw_network = |ui: &mut Ui| {
            let width = ui.available_width();
            kit::card(ui, |ui| {
                ui.set_width(width - 2.0 * S4 - 2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("Adapter traffic, last {history_seconds} seconds"))
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    if let Some(sample) = snapshot.network {
                        ui.label(
                            RichText::new(format!(
                                "↓ {}",
                                crate::opti::sysmon::format_bytes_per_second(
                                    sample.received_bytes_per_sec
                                )
                            ))
                            .size(T_SMALL)
                            .color(t.brand_text),
                        );
                        ui.label(
                            RichText::new(format!(
                                "↑ {}",
                                crate::opti::sysmon::format_bytes_per_second(
                                    sample.sent_bytes_per_sec
                                )
                            ))
                            .size(T_SMALL)
                            .color(t.good),
                        );
                    } else {
                        let status = if snapshot.network_available {
                            "Sampling adapter counters…"
                        } else {
                            "Adapter counters unavailable"
                        };
                        ui.label(RichText::new(status).size(T_SMALL).color(t.muted));
                    }
                });
                ui.label(RichText::new("Aggregate active adapter counters. Virtual adapters may be included; this is not internet throughput.").size(T_CAPTION).color(t.subtle));
                ui.add_space(S1);
                kit::network_chart(
                    ui,
                    vec2(ui.available_width(), 100.0),
                    network_history,
                    history_capacity,
                    t.brand_text,
                    t.good,
                );
            });
        };
        let draw_disk = |ui: &mut Ui| {
            let width = ui.available_width();
            kit::card(ui, |ui| {
                ui.set_width(width - 2.0 * S4 - 2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("Disk activity, last {history_seconds} seconds"))
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    let status = snapshot
                        .disk_activity
                        .map(crate::opti::sysmon::format_bytes_per_second)
                        .unwrap_or_else(|| {
                            if snapshot.disk_counter_available {
                                "Collecting first sample".into()
                            } else {
                                "Unavailable".into()
                            }
                        });
                    ui.label(RichText::new(status).size(T_SMALL).color(t.brand_text));
                });
                ui.label(RichText::new("Aggregate physical-disk read and write throughput from Windows performance counters.").size(T_CAPTION).color(t.subtle));
                ui.add_space(S1);
                kit::rate_chart(
                    ui,
                    vec2(ui.available_width(), 100.0),
                    disk_history,
                    history_capacity,
                    t.brand_text,
                );
            });
        };
        if ui.available_width() < 680.0 {
            draw_network(ui);
            ui.add_space(S2);
            draw_disk(ui);
        } else {
            ui.columns(2, |columns| {
                draw_network(&mut columns[0]);
                draw_disk(&mut columns[1]);
            });
        }
        ui.add_space(S2);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    "Process-level CPU and working-set readings are available in Processes.",
                )
                .size(T_SMALL)
                .color(t.muted),
            );
            if kit::secondary(ui, "Open Processes", true).clicked() {
                self.set_page(ctx, Page::Processes);
            }
        });
        ui.add_space(S2);
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Game frame-time comparison")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            ui.label(RichText::new("PresentMon records 30 seconds, then samples system counters and up to 10 TCP connections to 1.1.1.1:53. Keep the same game and scene running for both captures. First use downloads a SHA-256 verified tool.").size(T_SMALL).color(t.muted));
            ui.label(RichText::new("Results are observations, not proof a tweak caused a change. DNS endpoint TCP timings are not ICMP ping or game-server latency. Boot time comes from the latest Windows startup event; compare it only across different boot timestamps. Unsupported counters show Unavailable.").size(T_CAPTION).color(t.subtle));
            ui.add_space(S1);
            ui.horizontal_wrapped(|ui| {
                ui.add_sized(
                    [240.0, 32.0],
                    egui::TextEdit::singleline(&mut self.measurement_process)
                        .hint_text("Game process, for example game.exe")
                        .interactive(self.busy.is_none()),
                );
                let valid =
                    crate::opti::measurements::validate_process_name(&self.measurement_process)
                        .is_ok();
                if kit::secondary(ui, "Record before", valid && self.busy.is_none()).clicked() {
                    let process = self.measurement_process.trim().to_string();
                    self.start_measurement(ctx, process, "before");
                }
                if kit::secondary(ui, "Record after", valid && self.busy.is_none()).clicked() {
                    let process = self.measurement_process.trim().to_string();
                    self.start_measurement(ctx, process, "after");
                }
            });
            if let Some(error) = &self.measurements_error {
                ui.label(
                    RichText::new(format!("Measurement history unavailable: {error}"))
                        .size(T_SMALL)
                        .color(t.warn),
                );
            }
            let process = self.measurement_process.trim();
            let (before, after) =
                crate::opti::measurements::latest_pair(&self.measurements, process);
            if process.is_empty() || (before.is_none() && after.is_none()) {
                kit::empty_state(
                    ui,
                    "No matching captures",
                    "Record a before capture, change settings and restart if needed, then record after using the same process name.",
                );
            } else if let (Some(before), Some(after)) = (before, after)
                && after.time >= before.time
            {
                egui::Grid::new("frame-measurement-comparison")
                    .num_columns(3)
                    .spacing(vec2(S4, S1))
                    .show(ui, |ui| {
                        ui.label(RichText::new("Metric").size(T_SMALL).color(t.muted));
                        ui.label(RichText::new("Before").size(T_SMALL).color(t.muted));
                        ui.label(
                            RichText::new("After · observed")
                                .size(T_SMALL)
                                .color(t.muted),
                        );
                        ui.end_row();
                        for (name, first, second, unit) in [
                            ("Average", before.average_fps, after.average_fps, "FPS"),
                            ("1% low", before.low_1_fps, after.low_1_fps, "FPS"),
                            ("0.1% low", before.low_01_fps, after.low_01_fps, "FPS"),
                            (
                                "99th percentile frame time",
                                before.p99_frame_ms,
                                after.p99_frame_ms,
                                "ms",
                            ),
                        ] {
                            ui.label(RichText::new(name).size(T_SMALL).color(t.text));
                            ui.label(
                                RichText::new(format!("{first:.1} {unit}"))
                                    .size(T_SMALL)
                                    .color(t.text),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{second:.1} {unit} ({:+.1})",
                                    second - first
                                ))
                                .size(T_SMALL)
                                .color(t.text),
                            );
                            ui.end_row();
                        }
                        let metric = |value: Option<f64>, unit: &str| {
                            value.map_or_else(
                                || "Unavailable".into(),
                                |value| format!("{value:.2} {unit}"),
                            )
                        };
                        for (name, first, second, unit) in [
                            (
                                "DPC average",
                                before.dpc_average_percent,
                                after.dpc_average_percent,
                                "%",
                            ),
                            (
                                "DPC maximum",
                                before.dpc_max_percent,
                                after.dpc_max_percent,
                                "%",
                            ),
                            (
                                "Interrupt average",
                                before.interrupt_average_percent,
                                after.interrupt_average_percent,
                                "%",
                            ),
                            (
                                "DNS endpoint TCP average",
                                before.dns_tcp_average_ms,
                                after.dns_tcp_average_ms,
                                "ms",
                            ),
                            (
                                "DNS endpoint TCP jitter",
                                before.dns_tcp_jitter_ms,
                                after.dns_tcp_jitter_ms,
                                "ms",
                            ),
                            (
                                "Timer resolution",
                                before.timer_resolution_ms,
                                after.timer_resolution_ms,
                                "ms",
                            ),
                            (
                                "Last boot duration",
                                before.boot_duration_ms.map(|ms| ms as f64),
                                after.boot_duration_ms.map(|ms| ms as f64),
                                "ms",
                            ),
                        ] {
                            ui.label(RichText::new(name).size(T_SMALL).color(t.text));
                            ui.label(
                                RichText::new(metric(first, unit))
                                    .size(T_SMALL)
                                    .color(t.text),
                            );
                            ui.label(
                                RichText::new(metric(second, unit))
                                    .size(T_SMALL)
                                    .color(t.text),
                            );
                            ui.end_row();
                        }
                        ui.label(
                            RichText::new("Frames / stutters")
                                .size(T_SMALL)
                                .color(t.text),
                        );
                        ui.label(
                            RichText::new(format!("{} / {}", before.frames, before.stutters))
                                .size(T_SMALL)
                                .color(t.text),
                        );
                        ui.label(
                            RichText::new(format!("{} / {}", after.frames, after.stutters))
                                .size(T_SMALL)
                                .color(t.text),
                        );
                        ui.end_row();
                        ui.label(
                            RichText::new("DNS TCP successes / failures")
                                .size(T_SMALL)
                                .color(t.text),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} / {}",
                                before.dns_tcp_samples, before.dns_tcp_failures
                            ))
                            .size(T_SMALL)
                            .color(t.text),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} / {}",
                                after.dns_tcp_samples, after.dns_tcp_failures
                            ))
                            .size(T_SMALL)
                            .color(t.text),
                        );
                        ui.end_row();
                    });
                ui.label(RichText::new(format!("Before {} · After {} · {}-second frame captures; DPC/interrupt counters sample for 5 seconds when available", before.time, after.time, before.seconds)).size(T_CAPTION).color(t.subtle));
                ui.label(
                    RichText::new(format!(
                        "Boot events · {} · {}",
                        before.boot_event_time.as_deref().unwrap_or("Unavailable"),
                        after.boot_event_time.as_deref().unwrap_or("Unavailable")
                    ))
                    .size(T_CAPTION)
                    .color(t.subtle),
                );
            } else if before.is_some() && after.is_some() {
                kit::empty_state(
                    ui,
                    "Capture order is reversed",
                    "The latest after capture predates the before capture. Record after again to compare in order.",
                );
            } else {
                ui.label(
                    RichText::new(
                        "One capture saved. Record the other using the same process name.",
                    )
                    .size(T_SMALL)
                    .color(t.muted),
                );
            }
        });
    }

    pub(super) fn start_measurement(
        &mut self,
        ctx: &egui::Context,
        process: String,
        label: &'static str,
    ) {
        self.spawn(ctx, if label == "before" { "Recording before frame times" } else { "Recording after frame times" }, move |worker| {
            worker.log("PresentMon captures 30 seconds, followed by up to 10 seconds of system and network sampling. Keep the selected game running.");
            let result = crate::opti::measurements::record(&process, label);
            if let Err(error) = &result { worker.log(&format!("ERROR {error}")); }
            worker.send(Msg::Measurements(result));
        });
    }
}
