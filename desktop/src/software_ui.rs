use crate::kit;
use crate::theme::{self, Tone, semibold, tokens};
use crate::winget::{self, Action, InstalledPackage, Package};
use egui::{Align, FontId, Layout, RichText, Ui};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct PendingAction {
    action: Action,
    package: Option<Package>,
}

pub struct SoftwareUi {
    packages: Vec<Package>,
    catalog_error: Option<String>,
    pub query: String,
    category: String,
    installed_only: bool,
    installed: HashMap<String, InstalledPackage>,
    has_checked: bool,
    refresh_rx: Option<Receiver<Result<HashMap<String, InstalledPackage>, String>>>,
    busy: Option<String>,
    operation_rx: Option<Receiver<Result<i32, String>>>,
    message: Option<(bool, String)>,
    pending: Option<PendingAction>,
}

impl SoftwareUi {
    pub fn new() -> Self {
        let (packages, catalog_error) = match winget::catalog() {
            Ok(packages) => (packages, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Self {
            packages,
            catalog_error,
            query: String::new(),
            category: "All".into(),
            installed_only: false,
            installed: HashMap::new(),
            has_checked: false,
            refresh_rx: None,
            busy: None,
            operation_rx: None,
            message: None,
            pending: None,
        }
    }

    pub fn categories(&self) -> Vec<(String, usize)> {
        let mut counts = BTreeMap::new();
        for package in &self.packages {
            *counts.entry(package.category.clone()).or_insert(0usize) += 1;
        }
        counts.into_iter().collect()
    }

    pub fn status_line(&self) -> String {
        if let Some(operation) = &self.busy {
            return format!("WinGet: {operation}");
        }
        if self.refresh_rx.is_some() {
            return "WinGet: checking installed package IDs".into();
        }
        if self.has_checked {
            format!("WinGet: {} package IDs recognized", self.installed.len())
        } else {
            "WinGet: installed status not checked".into()
        }
    }

    pub fn set_installed_only(&mut self, installed_only: bool) {
        self.installed_only = installed_only;
        self.category = "All".into();
    }

    pub fn select_category(&mut self, category: &str) {
        self.category = category.into();
        self.installed_only = false;
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.refresh_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.refresh_rx = None;
                    match result {
                        Ok(installed) => {
                            let count = installed.len();
                            self.installed = installed;
                            self.has_checked = true;
                            self.message = Some((
                                true,
                                format!("WinGet reported {count} installed package entries."),
                            ));
                        }
                        Err(error) => self.message = Some((false, error)),
                    }
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.refresh_rx = None;
                    self.message = Some((
                        false,
                        "WinGet status worker stopped without returning a result.".into(),
                    ));
                }
            }
        }
        if let Some(rx) = &self.operation_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.operation_rx = None;
                    let label = self.busy.take().unwrap_or_else(|| "WinGet".into());
                    self.message = Some(match result {
                        Ok(0) => (
                            true,
                            format!(
                                "{label} finished. Refresh installed status to verify the result."
                            ),
                        ),
                        Ok(code) => (
                            false,
                            format!(
                                "{label} exited with code {code}. Review the WinGet console for details."
                            ),
                        ),
                        Err(error) => (false, error),
                    });
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.operation_rx = None;
                    self.busy = None;
                    self.message = Some((
                        false,
                        "WinGet operation worker stopped without returning a result.".into(),
                    ));
                }
            }
        }
    }

    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.refresh_rx.is_some() || self.operation_rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(winget::installed_packages());
            repaint.request_repaint();
        });
        self.refresh_rx = Some(rx);
        self.message = None;
    }

    fn request(&mut self, action: Action, package: Option<&Package>) {
        self.pending = Some(PendingAction {
            action,
            package: package.cloned(),
        });
    }

    fn can_run_action(&self) -> bool {
        self.refresh_rx.is_none() && self.operation_rx.is_none()
    }

    fn start_operation(&mut self, ctx: &egui::Context, pending: PendingAction) {
        let id = pending.package.as_ref().map(|package| package.id.clone());
        let label = match (&pending.package, pending.action) {
            (Some(package), action) => format!("{} {}", action.label(), package.name),
            (None, Action::UpgradeAll) => "Upgrade all WinGet apps".into(),
            _ => "Run WinGet operation".into(),
        };
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        let action = pending.action;
        std::thread::spawn(move || {
            let _ = tx.send(winget::run_visible(action, id.as_deref()));
            repaint.request_repaint();
        });
        self.busy = Some(label);
        self.operation_rx = Some(rx);
        self.message = Some((true, "WinGet opened in a separate console. Review its prompts there; Optimaxer does not accept package agreements or suppress installer UI.".into()));
    }

    pub fn navigation(&mut self, ui: &mut Ui) {
        let count = self.packages.len();
        if kit::nav_item(
            ui,
            !self.installed_only && self.category == "All",
            "WinGet catalog",
            Some(&count.to_string()),
        )
        .clicked()
        {
            self.select_category("All");
        }
        if kit::nav_item(
            ui,
            self.installed_only,
            "Installed",
            Some(&self.installed.len().to_string()),
        )
        .clicked()
        {
            self.set_installed_only(true);
        }
        let categories = self.categories();
        for (name, count) in categories {
            if kit::nav_item(
                ui,
                !self.installed_only && self.category == name,
                &name,
                Some(&count.to_string()),
            )
            .clicked()
            {
                self.select_category(&name);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                kit::page_header(ui, "WinGet", "Install, upgrade and remove curated Windows apps with the system package manager.");
                ui.label(RichText::new("Package installers run in a visible WinGet console. Source agreements and installer prompts stay yours to review. Installed status includes only apps WinGet can match in its export.").size(theme::T_SMALL).color(t.muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::secondary(ui, if self.refresh_rx.is_some() { "Checking…" } else { "Refresh installed" }, self.refresh_rx.is_none() && self.operation_rx.is_none()).clicked() {
                    self.refresh(ctx);
                }
                if kit::ghost(ui, "Upgrade all", self.can_run_action() && !self.packages.is_empty()).clicked() {
                    self.request(Action::UpgradeAll, None);
                }
            });
        });
        ui.add_space(theme::S2);
        if let Some(error) = &self.catalog_error {
            kit::empty_state(ui, "WinGet catalog unavailable", error);
            return;
        }
        if let Some((success, message)) = &self.message {
            let color = if *success { t.good } else { t.bad };
            ui.add(
                egui::Label::new(RichText::new(message).size(theme::T_SMALL).color(color)).wrap(),
            );
            ui.add_space(theme::S1);
        }
        if let Some(operation) = &self.busy {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(RichText::new(operation).color(t.text));
                ui.label(RichText::new("Use the separate WinGet console for prompts. Close it only if you intend to stop the package operation.").size(theme::T_CAPTION).color(t.muted));
            });
            ui.add_space(theme::S1);
        }
        if self.installed_only && !self.has_checked {
            kit::empty_state(
                ui,
                "Installed status not checked",
                "Refresh installed to ask WinGet which curated packages it can match on this PC.",
            );
            return;
        }
        let query = self.query.trim().to_lowercase();
        let visible = self
            .packages
            .iter()
            .filter(|package| {
                let in_category = self.category == "All" || package.category == self.category;
                let installed = self
                    .installed
                    .contains_key(&package.id.to_ascii_lowercase());
                let in_state = !self.installed_only || installed;
                let in_query = query.is_empty()
                    || package.name.to_lowercase().contains(&query)
                    || package.id.to_lowercase().contains(&query)
                    || package.category.to_lowercase().contains(&query)
                    || package.domain.to_lowercase().contains(&query);
                in_category && in_state && in_query
            })
            .collect::<Vec<_>>();
        ui.horizontal(|ui| {
            let heading = if self.installed_only {
                "Installed curated packages"
            } else if self.category == "All" {
                "Catalog"
            } else {
                &self.category
            };
            ui.label(
                RichText::new(heading)
                    .font(FontId::new(theme::T_HEADING, semibold()))
                    .color(t.text),
            );
            ui.label(
                RichText::new(format!("{} shown", visible.len()))
                    .size(theme::T_SMALL)
                    .color(t.subtle),
            );
            if self.has_checked {
                ui.label(
                    RichText::new(format!(
                        "{} package IDs recognized by WinGet",
                        self.installed.len()
                    ))
                    .size(theme::T_SMALL)
                    .color(t.muted),
                );
            } else {
                ui.label(
                    RichText::new("Installed status not checked")
                        .size(theme::T_SMALL)
                        .color(t.muted),
                );
            }
        });
        if visible.is_empty() {
            let (title, detail) = if self.installed_only {
                (
                    "No recognized apps found",
                    "WinGet export may omit apps it cannot match. This is not a full inventory of installed Windows software.",
                )
            } else {
                ("No apps match", "Try another name, package ID or category.")
            };
            kit::empty_state(ui, title, detail);
            return;
        }
        let mut requested = None;
        let mut open_error = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for package in visible {
                    let installed = self.installed.get(&package.id.to_ascii_lowercase());
                    kit::row(ui, ("winget-package", &package.id), false, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&package.name)
                                            .font(FontId::new(theme::T_BODY, semibold()))
                                            .color(t.text),
                                    )
                                    .truncate(),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{}  ·  {}",
                                        package.id, package.category
                                    ))
                                    .size(theme::T_CAPTION)
                                    .color(t.muted),
                                );
                            });
                            if let Some(installed) = installed {
                                let version = installed
                                    .version
                                    .as_deref()
                                    .unwrap_or("Version not reported");
                                kit::chip(ui, &format!("Installed · {version}"), Tone::Good);
                                if kit::secondary(ui, "Upgrade", self.can_run_action()).clicked() {
                                    requested = Some((Action::Upgrade, package.clone()));
                                }
                                if kit::ghost(ui, "Uninstall", self.can_run_action()).clicked() {
                                    requested = Some((Action::Uninstall, package.clone()));
                                }
                            } else {
                                kit::chip(
                                    ui,
                                    if self.has_checked {
                                        "Not installed"
                                    } else {
                                        "Status unknown"
                                    },
                                    Tone::Neutral,
                                );
                                if kit::primary(ui, "Install", self.can_run_action()).clicked() {
                                    requested = Some((Action::Install, package.clone()));
                                }
                            }
                            if !package.website.is_empty()
                                && kit::ghost(ui, "Project website", true).clicked()
                                && let Err(error) = open::that(&package.website)
                            {
                                open_error =
                                    Some(format!("Could not open the publisher website: {error}"));
                            }
                        });
                    });
                }
            });
        if let Some((action, package)) = requested {
            self.request(action, Some(&package));
        }
        if let Some(error) = open_error {
            self.message = Some((false, error));
        }
        self.confirm_action(ctx);
    }

    fn confirm_action(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending else { return };
        let package_name = pending.package.as_ref().map_or_else(
            || "all WinGet-managed apps".into(),
            |package| package.name.clone(),
        );
        let package_id = pending.package.as_ref().map(|package| package.id.as_str());
        let command = winget::command_args(pending.action, package_id)
            .map(|args| format!("winget.exe {}", args.join(" ")));
        let command_valid = command.is_ok();
        let action = pending.action;
        let mut run = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::NULL.with("winget-confirm")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.label(RichText::new(format!("{} {package_name}?", action.label())).font(FontId::new(theme::T_TITLE, semibold())).color(tokens(ctx).text));
            ui.add_space(theme::S2);
            let t = tokens(ctx);
            match action {
                Action::Install => ui.label(RichText::new("WinGet will install the exact catalog package. Its interactive installer may request administrator approval or a restart.").color(t.muted)),
                Action::Upgrade => ui.label(RichText::new("WinGet will attempt to upgrade this exact package. If no update applies, WinGet will report that in its console.").color(t.muted)),
                Action::Uninstall => ui.label(RichText::new("This removes the package through WinGet. The uninstaller may remove app data or require a restart.").color(t.warn)),
                Action::UpgradeAll => ui.label(RichText::new("WinGet will attempt to upgrade every package it can update. Individual installers may require administrator approval or a restart.").color(t.warn)),
            };
            ui.add_space(theme::S2);
            match command {
                Ok(command) => ui.label(RichText::new(command).monospace().size(theme::T_CAPTION).color(t.text)),
                Err(ref error) => ui.label(RichText::new(error).color(t.bad)),
            };
            ui.add_space(theme::S3);
            ui.horizontal(|ui| {
                if kit::primary(ui, "Open WinGet console", command_valid && self.operation_rx.is_none()).clicked() {
                    run = true;
                }
                if kit::secondary(ui, "Cancel", true).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.pending = None;
        }
        if run && let Some(pending) = self.pending.take() {
            self.start_operation(ctx, pending);
        }
    }
}
