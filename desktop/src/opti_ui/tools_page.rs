//! Tools page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn tools_page(&mut self, ui: &mut Ui, ctx: &egui::Context, suggestions: &[AppItem]) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Tools",
            "Maintenance helpers. Anything that changes the system asks first.",
        );
        let idle = self.busy.is_none();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let size = vec2(268.0, 100.0);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Saved backup integrity").font(FontId::new(T_BODY, semibold())).color(t.text));
                        ui.add(egui::Label::new(RichText::new("Checks saved tweak originals and service backup files without changing them.").size(T_SMALL).color(t.muted)).wrap());
                    });
                    if kit::secondary(ui, if self.busy.as_deref() == Some("Checking saved backups") { "Checking…" } else { "Check backups" }, idle).clicked() {
                        self.check_saved_backups(ctx);
                    }
                });
                if let Some(checks) = &self.backup_integrity {
                    ui.add_space(S1);
                    for check in checks {
                        let (label, tone) = match check.status {
                            BackupStatus::Good => ("Readable", Tone::Good),
                            BackupStatus::Missing => ("Not found", Tone::Neutral),
                            BackupStatus::Warning => ("Check needed", Tone::Warn),
                        };
                        ui.horizontal_wrapped(|ui| {
                            kit::chip(ui, label, tone);
                            ui.label(RichText::new(check.title).font(FontId::new(T_SMALL, semibold())).color(t.text));
                            ui.add(egui::Label::new(RichText::new(&check.detail).size(T_CAPTION).color(if check.status == BackupStatus::Warning { t.warn } else { t.muted })).wrap());
                        });
                    }
                } else {
                    ui.label(RichText::new("Readability checks cannot guarantee Windows will accept every restore operation.").size(T_CAPTION).color(t.subtle));
                }
            });
            ui.add_space(S3);
            ui.horizontal_wrapped(|ui| {
                for info in tools::TOOLS.iter() {
                    if kit::tile(ui, size, info.title, info.desc, None, idle, false).clicked() {
                        if info.changes_system {
                            self.request_system_tool(info);
                        } else {
                            let tool = info.tool;
                            self.spawn_recorded(ctx, info.title, move |w| tools::run(&WinSystem, tool, &backup_dir(), &mut |l| w.log(l)));
                        }
                    }
                }
                if kit::tile(ui, size, "Open System Restore", "Roll your PC back to a restore point.", None, true, false).clicked()
                    && let Err(error) = std::process::Command::new("rstrui.exe").spawn() {
                    self.events.push(OptiEvent::Toast(format!("Could not open System Restore: {error}")));
                }
                let vendor = self.traits.as_ref().map(|tr| tr.vendor.clone()).unwrap_or_default();
                let url = hw::vendor_url(&vendor);
                let (title, desc) = match url {
                    Some(_) => ("Vendor driver page".to_string(), format!("Opens the official {vendor} support page in your browser. Optimaxer downloads and installs nothing.")),
                    None => ("Vendor driver page".to_string(), "No MSI, Acer or Gigabyte board detected. Open your PC maker's support site instead.".to_string()),
                };
                if kit::tile(ui, size, &title, &desc, None, url.is_some(), false).clicked()
                    && let Some(u) = url
                    && let Err(error) = open::that(u) {
                    self.events.push(OptiEvent::Toast(format!("Could not open the driver support page: {error}")));
                }
                if kit::tile(ui, size, "Undo all Optimaxer tweaks", "Restores every setting Optimaxer changed to its original value.", None, idle, true).clicked() && self.need_admin() {
                    self.pending = Some(Pending { title: "Undo all tweaks".into(), intro: "Restore every setting changed by Optimaxer to its original value?".into(), lines: vec![], action: Action::UndoAll, restore_option: false, ok_label: "Undo all tweaks".into() });
                }
            });
            kit::group_title(ui, "PC tools from the Orion catalog");
            ui.add(egui::Label::new(RichText::new("Utility and Customization apps for Windows. Downloading stays your decision: open an app, pick a build and press Download.").size(T_SMALL).color(t.muted)).wrap());
            ui.add_space(S2);
            ui.horizontal(|ui| {
                if kit::secondary(ui, "Browse Utility apps", true).clicked() {
                    self.events.push(OptiEvent::OpenStoreCategory("Utility".into()));
                }
                if kit::secondary(ui, "Browse Customization apps", true).clicked() {
                    self.events.push(OptiEvent::OpenStoreCategory("Customization".into()));
                }
            });
            ui.add_space(S2);
            ui.horizontal_wrapped(|ui| {
                for a in suggestions {
                    if kit::ghost(ui, &a.name, true).on_hover_text(&a.description).clicked() {
                        self.events.push(OptiEvent::OpenApp(a.id.clone()));
                    }
                }
            });
            ui.add_space(S5);
        });
    }
}
