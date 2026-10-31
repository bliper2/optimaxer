//! Processes: live list from /proc; stop a process you own after confirming.

use super::*;
use crate::linux::fmt_size;
use crate::linux::processes::{self, Signal};

impl OptiUi {
    pub(super) fn processes_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Processes",
            "Everything running now, read from /proc. Refreshes every 3 seconds while this page is open. You can ask your own processes to close.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.add_sized(
                [260.0, 32.0],
                egui::TextEdit::singleline(&mut self.process_search)
                    .hint_text("Search name, PID or user"),
            );
            ui.label(
                RichText::new(format!("{} processes", self.processes.len()))
                    .size(T_SMALL)
                    .color(t.muted),
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
                self.refresh_processes(ctx);
            }
        });
        ui.add_space(S2);
        let query = self.process_search.trim().to_lowercase();
        let mut rows: Vec<&ProcessRow> = self
            .processes
            .iter()
            .filter(|p| {
                query.is_empty()
                    || p.name.to_lowercase().contains(&query)
                    || p.user.to_lowercase().contains(&query)
                    || p.pid.to_string() == query
            })
            .collect();
        // CPU first; before the second refresh there is no CPU value, so memory orders the list.
        rows.sort_by(|a, b| {
            b.cpu_percent
                .unwrap_or(-1.0)
                .total_cmp(&a.cpu_percent.unwrap_or(-1.0))
                .then(b.rss_bytes.cmp(&a.rss_bytes))
        });
        if self.processes.is_empty() {
            kit::skeleton(ui, 4);
            return;
        }
        let selected = self.process_selected;
        let mut clicked = None;
        for p in rows.iter().take(250) {
            kit::row(ui, ("linux-proc", p.pid), selected == Some(p.pid), |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [190.0, 18.0],
                        egui::Label::new(RichText::new(&p.name).color(t.text)).truncate(),
                    );
                    ui.add_sized(
                        [60.0, 18.0],
                        egui::Label::new(
                            RichText::new(p.pid.to_string())
                                .size(T_SMALL)
                                .color(t.subtle),
                        ),
                    );
                    ui.add_sized(
                        [90.0, 18.0],
                        egui::Label::new(RichText::new(&p.user).size(T_SMALL).color(t.muted))
                            .truncate(),
                    );
                    ui.add_sized(
                        [60.0, 18.0],
                        egui::Label::new(
                            RichText::new(p.cpu_percent.map_or("–".into(), |v| format!("{v:.1}%")))
                                .size(T_SMALL)
                                .color(t.muted),
                        ),
                    );
                    ui.add_sized(
                        [80.0, 18.0],
                        egui::Label::new(
                            RichText::new(fmt_size(p.rss_bytes))
                                .size(T_SMALL)
                                .color(t.muted),
                        ),
                    );
                });
            })
            .response
            .interact(egui::Sense::click())
            .clicked()
            .then(|| clicked = Some(p.pid));
        }
        if rows.len() > 250 {
            ui.label(
                RichText::new(format!(
                    "Showing the first 250 of {} matches. Narrow the search to see others.",
                    rows.len()
                ))
                .size(T_CAPTION)
                .color(t.subtle),
            );
        }
        if let Some(pid) = clicked {
            self.process_selected = Some(pid);
        }
        let Some(pid) = self.process_selected else {
            return;
        };
        let Some(p) = self.processes.iter().find(|p| p.pid == pid).cloned() else {
            return;
        };
        ui.add_space(S3);
        let (cmdline, exe) = processes::details(self.sys.as_ref(), pid);
        let mut action = None;
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            Self::heading(ui, &format!("{} (PID {})", p.name, p.pid));
            let line = |ui: &mut Ui, k: &str, v: String| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(k).size(T_SMALL).color(t.subtle));
                    ui.label(RichText::new(v).size(T_SMALL).color(t.muted));
                });
            };
            line(ui, "State", processes::state_label(p.state).into());
            line(ui, "Owner", p.user.clone());
            line(ui, "Parent PID", p.ppid.to_string());
            line(ui, "Threads", p.threads.to_string());
            line(ui, "Memory", fmt_size(p.rss_bytes));
            if let Some(e) = exe {
                line(ui, "Program", e);
            }
            if let Some(c) = cmdline {
                line(ui, "Command", c);
            }
            let allowed = processes::can_signal(pid).is_ok();
            ui.horizontal(|ui| {
                if kit::secondary(ui, Signal::Terminate.label(), allowed).clicked() {
                    action = Some(Signal::Terminate);
                }
                if kit::ghost(ui, Signal::Kill.label(), allowed).clicked() {
                    action = Some(Signal::Kill);
                }
            });
        });
        if let Some(signal) = action {
            let title = format!(
                "{} {} (PID {pid})",
                if signal == Signal::Kill {
                    "Force quit"
                } else {
                    "Close"
                },
                p.name
            );
            let mut lines = vec![Line::new(format!(
                "Send {} to process {pid} ({})",
                if signal == Signal::Kill {
                    "SIGKILL"
                } else {
                    "SIGTERM"
                },
                p.name
            ))];
            if signal == Signal::Kill {
                lines.push(Line::warn(
                    "The process gets no chance to save its work. Unsaved data in it is lost.",
                ));
            }
            lines.push(Line::new(
                "Only processes you own can be stopped; others report a permission error.",
            ));
            self.ask(Ok(Pending {
                title,
                intro: "This stops the running program. It does not change any file on disk."
                    .into(),
                lines,
                ok_label: if signal == Signal::Kill {
                    "Force quit".into()
                } else {
                    "Ask to close".into()
                },
                run: Box::new(move |_| {
                    processes::send_signal(pid, signal)
                        .map(|()| vec![format!("Signal sent to {pid}")])
                }),
                save_before: None,
                save_after: None,
                followup: Followup::None,
                script: None,
            }));
        }
    }
}
