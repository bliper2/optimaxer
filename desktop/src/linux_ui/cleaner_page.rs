//! Cleaner: caches in the home folder and system caches. Deleted files cannot be restored, and the preview says so.

use super::*;
use crate::linux::cleaner::{Kind, Risk, TARGETS};
use crate::linux::fmt_size;

impl OptiUi {
    pub(super) fn cleaner_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Cleaner",
            "Caches and leftovers. Cleaning deletes files for good: they are not moved to the Trash and cannot be restored. Sizes are measured now, not guessed.",
        );
        ui.horizontal(|ui| {
            if kit::ghost(ui, "Rescan", self.busy.is_none()).clicked() {
                self.refresh_cleaner(ctx);
            }
        });
        ui.add_space(S2);
        let Some(scans) = &self.scans else {
            kit::skeleton(ui, 5);
            return;
        };
        let mut toggled: Option<&'static str> = None;
        let mut total_selected = 0u64;
        for target in TARGETS.iter() {
            let Some(scan) = scans.iter().find(|s| s.id == target.id) else {
                continue;
            };
            if !scan.present && matches!(target.kind, Kind::Home(_)) {
                continue;
            }
            let picked = self.clean_selected.contains(target.id);
            if picked {
                total_selected += scan.bytes.unwrap_or(0);
            }
            let row = kit::row(ui, ("linux-clean", target.id), picked, |ui| {
                ui.horizontal(|ui| {
                    kit::check(ui, picked);
                    ui.label(
                        RichText::new(target.name)
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    if target.risk == Risk::Moderate {
                        Self::badge(ui, "Check first", Tone::Warn);
                    }
                    if target.needs_root() {
                        Self::badge(ui, "Needs password", Tone::Neutral);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let size = scan.bytes.map_or("size unknown".to_string(), fmt_size);
                        ui.label(RichText::new(size).size(T_SMALL).color(t.muted));
                    });
                });
            });
            if row.response.interact(egui::Sense::click()).clicked() {
                toggled = Some(target.id);
            }
        }
        if let Some(id) = toggled
            && !self.clean_selected.remove(id)
        {
            self.clean_selected.insert(id);
        }
        ui.add_space(S3);
        let count = self.clean_selected.len();
        let mut go = false;
        ui.horizontal(|ui| {
            if kit::primary(
                ui,
                &format!("Preview and clean ({count})"),
                count > 0 && self.busy.is_none(),
            )
            .clicked()
            {
                go = true;
            }
            if total_selected > 0 {
                ui.label(
                    RichText::new(format!("about {} selected", fmt_size(total_selected)))
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
        });
        if go {
            self.clean_selected_targets();
        }
    }

    fn clean_selected_targets(&mut self) {
        let home = PathBuf::from(self.sys.home());
        let mut lines = Vec::new();
        let mut home_targets = Vec::new();
        let mut root_plan = Plan::default();
        let mut user_plan = Plan::default();
        for target in TARGETS
            .iter()
            .filter(|t| self.clean_selected.contains(t.id))
        {
            let size = self
                .scans
                .as_ref()
                .and_then(|s| s.iter().find(|s| s.id == target.id))
                .and_then(|s| s.bytes)
                .map_or(String::new(), |b| format!(" ({})", fmt_size(b)));
            lines.push(Line::new(format!("{}{size}", target.name)));
            match target.kind {
                Kind::Home(_) => {
                    for p in cleaner::home_paths(target, &home) {
                        lines.push(Line::new(format!("   empties {}", p.display())));
                    }
                    if target.id == "trash" {
                        lines.push(Line::warn("   Trash is emptied for good."));
                    }
                    home_targets.push(*target);
                }
                Kind::FlatpakUnused => user_plan.extend(cleaner::flatpak_plan()),
                _ => match cleaner::root_plan(self.sys.as_ref(), target) {
                    Some(plan) => root_plan.extend(plan),
                    None => lines.push(Line::warn("   Nothing to do on this system.")),
                },
            }
        }
        if home_targets.is_empty() && root_plan.is_empty() && user_plan.is_empty() {
            self.toast("Nothing to clean for the selection.");
            return;
        }
        if root_plan.needs_root() && !self.can_elevate {
            self.toast("System caches need administrator rights and pkexec (polkit) was not found. Untick them or install polkit.");
            return;
        }
        lines.extend(root_plan.describe().into_iter().map(Line::new));
        lines.extend(user_plan.describe().into_iter().map(Line::new));
        let script = root_plan.needs_root().then(|| root_plan.to_script());
        let mut intro = "These files are deleted permanently. Cache folders are emptied but kept, links are never followed, and nothing outside the listed folders is touched.".to_string();
        if script.is_some() && !self.is_root {
            intro.push_str(
                " Your system will ask for your password (polkit) for the system cleanups.",
            );
        }
        let home_for_job = home.clone();
        self.ask(Ok(Pending {
            title: "Clean selected caches".into(),
            intro,
            lines,
            ok_label: "Delete permanently".into(),
            run: Box::new(move |sys| {
                let mut out = Vec::new();
                for target in &home_targets {
                    let result = cleaner::clean_home_target(target, &home_for_job);
                    out.push(format!(
                        "{}: removed {} items",
                        target.name, result.removed_entries
                    ));
                    out.extend(result.skipped.into_iter().map(|s| format!("skipped {s}")));
                }
                for plan in [&user_plan, &root_plan] {
                    if !plan.is_empty() {
                        match sys.execute(plan) {
                            Ok(lines) => out.extend(lines),
                            Err(error) => {
                                out.extend(error.lines().map(String::from));
                                return Err(out.join("\n"));
                            }
                        }
                    }
                }
                Ok(out)
            }),
            save_before: None,
            save_after: None,
            followup: Followup::Cleaner,
            script,
        }));
        self.clean_selected.clear();
    }
}
