//! Processes page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn processes_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        self.refresh_processes(ctx);
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Processes",
            "Read-only process list. Select a process to inspect its executable path and embedded company metadata; Windows may restrict access.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [260.0, 32.0],
                egui::TextEdit::singleline(&mut self.process_search)
                    .hint_text("Search process, PID or path"),
            );
            ui.label(
                RichText::new(format!("{} processes", self.processes.len()))
                    .size(T_SMALL)
                    .color(t.muted),
            );
            ui.label(
                RichText::new("Refreshes every 3 seconds while open")
                    .size(T_CAPTION)
                    .color(t.subtle),
            );
            if kit::ghost(
                ui,
                if self.process_loading {
                    "Refreshing"
                } else {
                    "Refresh"
                },
                !self.process_loading,
            )
            .clicked()
            {
                self.process_last_scan = None;
            }
            ui.label(RichText::new("Sort").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("process-sort")
                .selected_text(["CPU", "Memory", "Name"][self.process_sort.min(2)])
                .show_ui(ui, |ui| {
                    for (index, label) in ["CPU", "Memory", "Name"].iter().enumerate() {
                        ui.selectable_value(&mut self.process_sort, index, *label);
                    }
                });
        });
        if let Some(error) = &self.process_error {
            ui.label(
                RichText::new(format!("Process scan failed: {error}"))
                    .size(T_SMALL)
                    .color(t.warn),
            );
        }
        if let Some(selected) = &self.process_inspection {
            ui.add_space(S2);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Process details")
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    ui.label(
                        RichText::new(format!("{} · PID {}", selected.name, selected.pid))
                            .size(T_SMALL)
                            .color(t.muted),
                    );
                });
                ui.add_space(S1);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Publisher metadata")
                            .size(T_SMALL)
                            .color(t.muted),
                    );
                    match &self.process_publisher {
                        Some(ProcessPublisherState::Loading) | None => {
                            ui.label(
                                RichText::new("Reading executable…")
                                    .size(T_SMALL)
                                    .color(t.subtle),
                            );
                        }
                        Some(ProcessPublisherState::Ready(Some(name))) => {
                            ui.label(RichText::new(name).size(T_SMALL).color(t.text));
                        }
                        Some(ProcessPublisherState::Ready(None)) => {
                            ui.label(
                                RichText::new("Not embedded in this file")
                                    .size(T_SMALL)
                                    .color(t.subtle),
                            );
                        }
                        Some(ProcessPublisherState::Error(error)) => {
                            ui.label(RichText::new(error).size(T_SMALL).color(t.warn));
                        }
                    }
                });
                ui.label(
                    RichText::new("CompanyName comes from executable version metadata; it is not a verified digital signature.")
                        .size(T_CAPTION)
                        .color(t.subtle),
                );
                ui.add_space(S1);
                let path = selected
                    .image_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Windows denied access to this executable path.".into());
                ui.add(
                    egui::Label::new(
                        RichText::new(path)
                            .size(T_CAPTION)
                            .monospace()
                            .color(t.muted),
                    )
                    .wrap(),
                );
            });
        }
        if self.processes.is_empty() {
            if self.process_loading {
                kit::skeleton(ui, 6);
            } else if self.process_error.is_some() {
                kit::empty_state(
                    ui,
                    "Could not read processes",
                    "Try Refresh after checking whether security software is blocking process queries.",
                );
            } else {
                kit::empty_state(
                    ui,
                    "No processes returned",
                    "Refresh to query the Windows process list.",
                );
            }
            return;
        }

        let query = self.process_search.trim().to_lowercase();
        let mut rows: Vec<&ProcessInfo> =
            self.processes
                .iter()
                .filter(|process| {
                    query.is_empty()
                        || process.name.to_lowercase().contains(&query)
                        || process.pid.to_string().contains(&query)
                        || process.image_path.as_ref().is_some_and(|path| {
                            path.to_string_lossy().to_lowercase().contains(&query)
                        })
                })
                .collect();
        match self.process_sort.min(2) {
            0 => rows.sort_by(|left, right| {
                right
                    .cpu_percent
                    .unwrap_or(-1.0)
                    .total_cmp(&left.cpu_percent.unwrap_or(-1.0))
                    .then(left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            }),
            1 => rows.sort_by(|left, right| {
                right
                    .working_set
                    .is_some()
                    .cmp(&left.working_set.is_some())
                    .then(
                        right
                            .working_set
                            .unwrap_or(0)
                            .cmp(&left.working_set.unwrap_or(0)),
                    )
                    .then(left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            }),
            _ => rows.sort_by(|left, right| {
                left.name
                    .to_lowercase()
                    .cmp(&right.name.to_lowercase())
                    .then(left.pid.cmp(&right.pid))
            }),
        }
        if rows.is_empty() {
            kit::empty_state(
                ui,
                "No matching processes",
                "Try a shorter search or clear the filter.",
            );
            return;
        }

        let selected_pid = self.process_inspection.as_ref().map(|process| process.pid);
        let mut inspection_clicked = None;
        egui_extras::TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(
                egui_extras::Column::initial(210.0)
                    .at_least(150.0)
                    .resizable(true),
            )
            .column(
                egui_extras::Column::initial(70.0)
                    .at_least(54.0)
                    .resizable(true),
            )
            .column(
                egui_extras::Column::initial(78.0)
                    .at_least(65.0)
                    .resizable(true),
            )
            .column(
                egui_extras::Column::initial(112.0)
                    .at_least(92.0)
                    .resizable(true),
            )
            .column(
                egui_extras::Column::remainder()
                    .at_least(200.0)
                    .resizable(true),
            )
            .header(30.0, |mut header| {
                for label in ["Process", "PID", "CPU", "Working set", "Image path"] {
                    header.col(|ui| {
                        ui.label(
                            RichText::new(label)
                                .font(FontId::new(T_SMALL, semibold()))
                                .color(t.muted),
                        );
                    });
                }
            })
            .body(|body| {
                body.rows(29.0, rows.len(), |mut row| {
                    let process = rows[row.index()];
                    row.col(|ui| {
                        if ui
                            .selectable_label(
                                selected_pid == Some(process.pid),
                                RichText::new(&process.name).size(T_SMALL).color(t.text),
                            )
                            .on_hover_text("Select to inspect executable details")
                            .clicked()
                        {
                            inspection_clicked = Some(process.pid);
                        }
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(process.pid.to_string())
                                .size(T_CAPTION)
                                .monospace()
                                .color(t.muted),
                        );
                    });
                    row.col(|ui| {
                        let value = process
                            .cpu_percent
                            .map(|value| format!("{value:.1}%"))
                            .unwrap_or_else(|| "—".into());
                        ui.label(RichText::new(value).size(T_SMALL).color(t.text));
                    });
                    row.col(|ui| {
                        let value = process
                            .working_set
                            .map(|bytes| format_size(Some(bytes)))
                            .unwrap_or_else(|| "Unavailable".into());
                        ui.label(RichText::new(value).size(T_SMALL).color(
                            if process.working_set.is_some() {
                                t.text
                            } else {
                                t.subtle
                            },
                        ));
                    });
                    row.col(|ui| {
                        let value = process
                            .image_path
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "Unavailable".into());
                        ui.add(
                            egui::Label::new(RichText::new(value).size(T_CAPTION).color(t.muted))
                                .truncate(),
                        )
                        .on_hover_text(process.image_path.as_ref().map_or(
                            "Windows denied access to this image path.".to_owned(),
                            |path| path.display().to_string(),
                        ));
                    });
                });
            });
        if let Some(pid) = inspection_clicked {
            if selected_pid == Some(pid) {
                self.process_inspection = None;
                self.process_publisher = None;
            } else if let Some(process) = self
                .processes
                .iter()
                .find(|process| process.pid == pid)
                .cloned()
            {
                self.inspect_process(ctx, process);
            }
        }
    }
}
