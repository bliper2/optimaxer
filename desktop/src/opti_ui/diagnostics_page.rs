//! Diagnostics page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn run_diagnostic(&mut self, ctx: &egui::Context, check: DiagnosticCheck) {
        let title = format!("Scan: {}", check.title());
        self.spawn_recorded(ctx, &title, move |worker| {
            worker.log(&format!(
                "Running {} with read-only options.",
                check.executable()
            ));
            let result = diagnostics::run(&WinSystem, check);
            for line in result.output.lines() {
                worker.log(line);
            }
            worker.log(&format!("Exit code: {}", result.exit_code));
            worker.send(Msg::Diagnostic(check, result));
        });
    }

    pub(super) fn check_saved_backups(&mut self, ctx: &egui::Context) {
        let optimaxer_dir = net::data_dir().join("optimaxer");
        let state_file = optimaxer_dir.join("state.json");
        let backup_dir = optimaxer_dir.join("backups");
        self.backup_integrity = None;
        self.spawn(ctx, "Checking saved backups", move |worker| {
            worker.send(Msg::BackupIntegrity(backup_integrity::inspect(
                &state_file,
                &backup_dir,
            )));
        });
    }

    pub(super) fn capture_system_snapshot(&mut self, ctx: &egui::Context) {
        self.spawn_recorded(ctx, "Capture system snapshot", |worker| {
            let snapshot = snapshots::capture(&WinSystem);
            let result = snapshots::append(snapshot);
            match &result {
                Ok(entries) => worker.log(&format!(
                    "Saved local inventory snapshot. {} snapshots retained.",
                    entries.len()
                )),
                Err(error) => worker.log(&format!("ERROR Could not save system snapshot: {error}")),
            }
            worker.send(Msg::Snapshots(result));
        });
    }

    pub(super) fn snapshots_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "System Snapshot",
            "Capture and browse a local inventory of this PC. Snapshots are not backups and cannot restore Windows.",
        );
        ui.horizontal_wrapped(|ui| {
            if kit::primary(ui, if self.busy.as_deref() == Some("Capture system snapshot") { "Capturing…" } else { "Capture snapshot" }, self.busy.is_none()).clicked() {
                self.capture_system_snapshot(ctx);
            }
            ui.label(RichText::new("Stores OS, processor, graphics, memory and system-volume readings on this PC. Keeps the 30 newest captures.").size(T_SMALL).color(t.muted));
        });
        ui.add_space(S2);

        if let Some(error) = &self.snapshots_error {
            ui.label(RichText::new(error).size(T_SMALL).color(t.bad));
            ui.add_space(S2);
        }
        if self.snapshots.is_empty() {
            if self.snapshots_error.is_none() {
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("No snapshots yet")
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    ui.label(RichText::new("Capture an inventory to record the current Windows build and detected hardware. No system settings are changed.").size(T_SMALL).color(t.muted));
                });
            }
            return;
        }

        if self.snapshots.len() >= 2 {
            let changes = snapshots::compare_latest(&self.snapshots);
            ui.label(
                RichText::new("Changes since previous capture")
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.text),
            );
            ui.add_space(S1);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                if changes.is_empty() {
                    ui.label(
                        RichText::new(
                            "No inventory changes detected between the two newest captures.",
                        )
                        .size(T_SMALL)
                        .color(t.muted),
                    );
                } else {
                    for change in changes {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(change.label).size(T_SMALL).color(t.muted));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(change.earlier).size(T_SMALL).color(t.text),
                                )
                                .wrap(),
                            );
                            ui.label(RichText::new("→").size(T_SMALL).color(t.subtle));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(change.latest).size(T_SMALL).color(t.text),
                                )
                                .wrap(),
                            );
                        });
                    }
                }
            });
            ui.add_space(S2);
        }

        for (index, snapshot) in self.snapshots.iter().enumerate() {
            let os = if snapshot.operating_system.is_empty() {
                "Operating system unavailable"
            } else {
                &snapshot.operating_system
            };
            egui::CollapsingHeader::new(format!("{} · {os}", snapshot.captured_at))
                .id_salt(("system-snapshot", index))
                .default_open(index == 0)
                .show(ui, |ui| {
                    let processor = if snapshot.processor.is_empty() {
                        "Unavailable"
                    } else {
                        &snapshot.processor
                    };
                    let graphics = if snapshot.graphics.is_empty() {
                        "Unavailable".into()
                    } else {
                        snapshot.graphics.join(", ")
                    };
                    let memory = match (snapshot.memory_used, snapshot.memory_total) {
                        (Some(used), Some(total)) => format!(
                            "{} used of {}",
                            format_size(Some(used)),
                            format_size(Some(total))
                        ),
                        _ => "Unavailable".into(),
                    };
                    let drive = match (snapshot.drive_used, snapshot.drive_total) {
                        (Some(used), Some(total)) => format!(
                            "{} used of {}",
                            format_size(Some(used)),
                            format_size(Some(total))
                        ),
                        _ => "Unavailable".into(),
                    };
                    for (label, value) in [
                        ("Processor", processor.to_owned()),
                        ("Logical threads", snapshot.logical_threads.to_string()),
                        ("Graphics", graphics),
                        ("Memory", memory),
                        (
                            "System volume",
                            if snapshot.system_drive.is_empty() {
                                drive
                            } else {
                                format!("{} · {drive}", snapshot.system_drive)
                            },
                        ),
                        (
                            "Uptime at capture",
                            crate::opti::sysmon::format_uptime(snapshot.uptime_seconds),
                        ),
                    ] {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(label).size(T_SMALL).color(t.muted));
                            ui.add(
                                egui::Label::new(RichText::new(value).size(T_SMALL).color(t.text))
                                    .wrap(),
                            );
                        });
                    }
                });
        }
    }

    pub(super) fn diagnostics_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Diagnostics",
            "Read-only Windows integrity scans. Review the command output; Optimaxer does not infer a system health score.",
        );
        ui.label(RichText::new("Both checks require administrator access. They may keep CPU and storage busy for several minutes. They do not repair files or change registry settings.").size(T_SMALL).color(t.muted));
        ui.add_space(S3);

        for check in [
            DiagnosticCheck::SystemFiles,
            DiagnosticCheck::ComponentStore,
        ] {
            let result = self.diagnostic_results.get(&check).cloned();
            let running = self.busy.as_deref()
                == Some(match check {
                    DiagnosticCheck::SystemFiles => "Scan: Windows system files",
                    DiagnosticCheck::ComponentStore => "Scan: Windows component store",
                });
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(check.title()).font(FontId::new(T_BODY, semibold())).color(t.text));
                        let description = match check {
                            DiagnosticCheck::SystemFiles => "SFC /verifyonly checks protected Windows system files without repairing them.",
                            DiagnosticCheck::ComponentStore => "DISM /ScanHealth checks the component store without running RestoreHealth.",
                        };
                        ui.label(RichText::new(description).size(T_SMALL).color(t.muted));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.elevated {
                            kit::chip(ui, "Administrator required", Tone::Warn);
                        }
                        if kit::primary(ui, if running { "Scanning…" } else { "Run scan" }, self.elevated && self.busy.is_none()).clicked() {
                            self.run_diagnostic(ctx, check);
                        }
                    });
                });
                if let Some(result) = result {
                    ui.add_space(S2);
                    let status = if result.exit_code == 0 {
                        "Completed; review the command output"
                    } else {
                        "Command failed; review the error output"
                    };
                    ui.label(
                        RichText::new(format!("{status} · exit code {}", result.exit_code))
                            .size(T_SMALL)
                            .color(if result.exit_code == 0 {
                                t.muted
                            } else {
                                t.bad
                            }),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(result.output)
                                        .monospace()
                                        .size(T_SMALL)
                                        .color(t.text),
                                )
                                .wrap(),
                            );
                        });
                }
            });
            ui.add_space(S2);
        }
    }
}
