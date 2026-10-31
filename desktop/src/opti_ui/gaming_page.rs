//! Gaming page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn refresh_game_states(&mut self, ctx: &egui::Context) {
        if self.game_status_loading || self.game_profiles_error.is_some() || self.game_states_loaded
        {
            return;
        }
        if self.game_profiles.is_empty() {
            self.game_states_loaded = true;
            return;
        }
        self.game_status_loading = true;
        let profiles = self.game_profiles.clone();
        let worker = self.worker(ctx);
        std::thread::spawn(move || {
            let states = profiles
                .iter()
                .map(|profile| {
                    (
                        profile.executable.clone(),
                        gaming::state(&WinSystem, profile),
                    )
                })
                .collect();
            worker.send(Msg::GameStates(states));
        });
    }

    pub(super) fn add_game_executable(&mut self, ctx: &egui::Context) {
        self.add_game_executable_from(ctx, None);
    }

    pub(super) fn add_game_executable_from(
        &mut self,
        ctx: &egui::Context,
        directory: Option<&std::path::Path>,
    ) {
        if self.game_profiles_error.is_some() {
            self.events.push(OptiEvent::Toast(
                "Saved game profiles are unreadable. Recover the file before adding profiles."
                    .into(),
            ));
            return;
        }
        let mut dialog = rfd::FileDialog::new().add_filter("Windows game executable", &["exe"]);
        if let Some(directory) = directory.filter(|directory| directory.is_dir()) {
            dialog = dialog.set_directory(directory);
        }
        let Some(path) = dialog.pick_file() else {
            return;
        };
        let canonical = match path.canonicalize() {
            Ok(path)
                if path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) =>
            {
                let text = path.to_string_lossy();
                text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
            }
            Ok(_) => {
                self.events.push(OptiEvent::Toast(
                    "Choose an existing Windows .exe file.".into(),
                ));
                return;
            }
            Err(error) => {
                self.events.push(OptiEvent::Toast(format!(
                    "Could not resolve that executable path: {error}"
                )));
                return;
            }
        };
        let profile = match GameProfile::new(&canonical) {
            Ok(profile) => profile,
            Err(error) => {
                self.events.push(OptiEvent::Toast(error));
                return;
            }
        };
        let mut profiles = self.game_profiles.clone();
        profiles.push(profile);
        match gaming::save_store(&gaming::store_path(), &profiles) {
            Ok(()) => {
                self.game_profiles = profiles;
                self.game_profiles_error = None;
                self.game_states.clear();
                self.game_states_loaded = false;
                self.refresh_game_states(ctx);
            }
            Err(error) => self.events.push(OptiEvent::Toast(error)),
        }
    }

    pub(super) fn scan_game_libraries(&mut self, ctx: &egui::Context) {
        if self.game_library_scanning {
            return;
        }
        self.game_library_scanning = true;
        self.game_library_scan = None;
        let mut steam_roots = Vec::new();
        for variable in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(variable) {
                let base = PathBuf::from(base);
                steam_roots.push(base.join("Steam"));
                steam_roots.push(base.join("steam"));
            }
        }
        if let Some(home) = dirs::home_dir() {
            steam_roots.push(home.join(".steam").join("steam"));
            steam_roots.push(home.join(".local").join("share").join("Steam"));
        }
        let epic_manifests = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
            .join(r"Epic\EpicGamesLauncher\Data\Manifests");
        let worker = self.worker(ctx);
        std::thread::spawn(move || {
            for (key, value) in [
                (r"HKCU:\Software\Valve\Steam", "SteamPath"),
                (r"HKLM:\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
                (r"HKLM:\SOFTWARE\Valve\Steam", "InstallPath"),
            ] {
                if let Some(RegData::Str(path) | RegData::Expand(path)) =
                    WinSystem.reg_get(key, value)
                {
                    steam_roots.push(PathBuf::from(path));
                }
            }
            worker.send(Msg::GameLibraries(gaming::scan_libraries(
                &steam_roots,
                &epic_manifests,
            )));
        });
    }

    pub(super) fn export_game_preference(&mut self, profile: &GameProfile) {
        let name = std::path::Path::new(&profile.executable)
            .file_stem()
            .map_or_else(|| "game".into(), |stem| stem.to_string_lossy().to_string());
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Optimaxer graphics preset", &["json"])
            .set_file_name(format!("{name}-graphics.json"))
            .save_file()
        else {
            return;
        };
        if let Err(error) = gaming::write_export(&path, profile) {
            self.events.push(OptiEvent::Toast(error));
        }
    }

    pub(super) fn import_game_preference(&mut self, profile: &GameProfile) {
        if !profile.can_remove() {
            self.events.push(OptiEvent::Toast(
                "Restore the active setting before importing another graphics preference.".into(),
            ));
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Optimaxer graphics preset", &["json"])
            .pick_file()
        else {
            return;
        };
        let preference = match gaming::read_export(&path) {
            Ok(preference) => preference,
            Err(error) => {
                self.events.push(OptiEvent::Toast(error));
                return;
            }
        };
        let mut updated = self.game_profiles.clone();
        match gaming::set_preference(
            &gaming::store_path(),
            &mut updated,
            &profile.executable,
            preference,
        ) {
            Ok(()) => {
                self.game_profiles = updated;
                self.game_states_loaded = false;
            }
            Err(error) => self.events.push(OptiEvent::Toast(error)),
        }
    }

    pub(super) fn preview_game_apply(&mut self, ctx: &egui::Context, executable: String) {
        let store_path = gaming::store_path();
        self.spawn(ctx, "Reading per-game graphics preference", move |worker| {
            let result = gaming::load_store(&store_path).and_then(|profiles| {
                let profile = profiles.iter().find(|profile| profile.executable.eq_ignore_ascii_case(&executable))
                    .ok_or_else(|| "Game profile no longer exists. Refresh the page.".to_string())?;
                let (current, desired) = gaming::preview(&WinSystem, profile)?;
                worker.log(&format!("Preview for {executable}: {} -> {}", current.as_ref().map_or("not set".into(), RegData::show), desired.as_ref().map_or("Windows default".into(), RegData::show)));
                Ok(Pending {
                    title: "Set Windows per-game graphics preference".into(),
                    intro: "This changes the current Windows user's graphics preference for this executable. Start the game again for Windows and the driver to use the new preference. It does not guarantee higher performance.".into(),
                    lines: vec![
                        Line { text: format!("Executable: {executable}"), warn: false, head: false },
                        Line { text: format!("Current: {}", current.as_ref().map_or("not set".into(), RegData::show)), warn: false, head: false },
                        Line { text: format!("Requested: {}", desired.as_ref().map_or("Windows default (remove override)".into(), RegData::show)), warn: false, head: false },
                    ],
                    action: Action::ApplyGameGraphics { executable: executable.clone(), expected_current: current },
                    restore_option: false,
                    ok_label: "Apply preference".into(),
                })
            });
            match result {
                Ok(plan) => worker.send(Msg::Plan(plan)),
                Err(error) => worker.send(Msg::Toast(format!("Could not prepare graphics preference: {error}"))),
            }
        });
    }

    pub(super) fn preview_game_undo(&mut self, ctx: &egui::Context, executable: String) {
        let store_path = gaming::store_path();
        self.spawn(ctx, "Preparing graphics preference restore", move |worker| {
            let result = gaming::load_store(&store_path).and_then(|profiles| {
                let profile = profiles.iter().find(|profile| profile.executable.eq_ignore_ascii_case(&executable))
                    .ok_or_else(|| "Game profile no longer exists. Refresh the page.".to_string())?;
                let (current, original) = gaming::undo_preview(&WinSystem, profile)?;
                Ok(Pending {
                    title: "Restore original Windows graphics preference".into(),
                    intro: "Optimaxer will restore the exact registry value captured before the profile was applied. If it was already restored, this only clears the completed recovery record.".into(),
                    lines: vec![
                        Line { text: format!("Executable: {executable}"), warn: false, head: false },
                        Line { text: format!("Current: {}", current.as_ref().map_or("not set".into(), RegData::show)), warn: false, head: false },
                        Line { text: format!("Original: {}", original.as_ref().map_or("not set".into(), RegData::show)), warn: false, head: false },
                    ],
                    action: Action::UndoGameGraphics { executable: executable.clone() },
                    restore_option: false,
                    ok_label: "Restore original".into(),
                })
            });
            match result {
                Ok(plan) => worker.send(Msg::Plan(plan)),
                Err(error) => worker.send(Msg::Toast(format!("Could not prepare graphics restore: {error}"))),
            }
        });
    }

    pub(super) fn gaming_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Gaming",
            "Review the existing reversible Windows gaming settings. These settings do not promise higher frame rates.",
        );
        self.refresh_game_states(ctx);

        let supported: Vec<_> = self
            .cat
            .tweaks
            .iter()
            .filter(|tw| tw.has_tag("gaming") && tw.retired.is_empty() && !tw.has_tag("optin"))
            .collect();
        let Some(traits) = self.traits.as_ref() else {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Reading this PC’s hardware and current settings…")
                        .color(t.muted),
                );
            });
            return;
        };
        if !self.status_ready {
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Checking the current Windows settings…").color(t.muted));
            });
            return;
        }

        let to_apply: Vec<String> = hw::preset_ids(&self.cat, traits, "gaming")
            .into_iter()
            .filter(|id| !self.status.get(id).copied().unwrap_or(false))
            .collect();
        let to_undo: Vec<String> = supported
            .iter()
            .filter(|tw| self.status.get(&tw.id).copied().unwrap_or(false))
            .map(|tw| tw.id.clone())
            .collect();
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let status_items = [
                ("Game Mode", "game-mode"),
                ("Game DVR capture", "game-dvr"),
                ("DVR machine policy", "game-dvr-policy"),
                ("Global fullscreen override", "game-fso"),
            ];
            let draw_status = |ui: &mut Ui, label: &str, id: &str| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(label).size(T_SMALL).color(t.text));
                    let current = self.status.get(id).copied();
                    let tone = match current {
                        Some(true) => Tone::Good,
                        Some(false) => Tone::Warn,
                        None => Tone::Bad,
                    };
                    kit::chip(ui, setting_match_label(current), tone);
                });
            };
            if ui.available_width() < 640.0 {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Windows gaming setting status")
                            .font(FontId::new(T_BODY, semibold()))
                            .color(t.text),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new("Registry configuration only; this does not measure live Game Bar or capture activity.")
                                .size(T_SMALL)
                                .color(t.muted),
                        )
                        .wrap(),
                    );
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Windows gaming setting status").font(FontId::new(T_BODY, semibold())).color(t.text));
                    ui.add(egui::Label::new(RichText::new("Registry configuration only; this does not measure live Game Bar or capture activity.").size(T_SMALL).color(t.muted)).wrap());
                });
            }
            ui.add_space(S1);
            if ui.available_width() < 640.0 {
                ui.columns(2, |columns| {
                    let column_count = columns.len();
                    for (index, (label, id)) in status_items.iter().enumerate() {
                        let column = &mut columns[index % column_count];
                        draw_status(column, label, id);
                        column.add_space(S2);
                    }
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    for (label, id) in status_items {
                        draw_status(ui, label, id);
                        ui.add_space(S2);
                    }
                });
            }
        });
        ui.add_space(S2);
        let mut requested_tweak_review: Option<(String, bool)> = None;
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            if ui.available_width() < 640.0 {
                ui.vertical(|ui| {
                    kit::chip(ui, &format!("{} settings", supported.len()), Tone::Brand);
                    ui.add(
                        egui::Label::new(
                            RichText::new("Current state is read from Windows. Incompatible rows show why; review one setting at a time or use the batch actions below.")
                                .size(T_SMALL)
                                .color(t.muted),
                        )
                        .wrap(),
                    );
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    kit::chip(ui, &format!("{} settings", supported.len()), Tone::Brand);
                    ui.add(egui::Label::new(RichText::new("Current state is read from Windows. Incompatible rows show why; review one setting at a time or use the batch actions below.").size(T_SMALL).color(t.muted)).wrap());
                });
            }
            ui.add_space(S2);
            for tw in &supported {
                let applied = self.status.get(&tw.id).copied().unwrap_or(false);
                let compatible = hw::fits(tw, traits);
                let can_review = self.busy.is_none() && self.elevated;
                egui::Frame::new()
                    .inner_margin(Margin::symmetric(S2 as i8, S1 as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let draw_details = |ui: &mut Ui| {
                            ui.vertical(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&tw.name)
                                            .font(FontId::new(T_BODY, semibold()))
                                            .color(t.text),
                                    )
                                    .wrap(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&tw.desc).size(T_CAPTION).color(t.muted),
                                    )
                                    .wrap(),
                                );
                            });
                        };
                        let mut draw_action = |ui: &mut Ui| {
                            if applied {
                                kit::chip(ui, "Applied", Tone::Good);
                                if kit::secondary(ui, "Review undo", can_review).clicked() {
                                    requested_tweak_review = Some((tw.id.clone(), true));
                                }
                            } else if !compatible {
                                kit::chip(ui, "Not applicable", Tone::Neutral);
                            } else {
                                kit::chip(ui, "Available", Tone::Neutral);
                                if kit::secondary(ui, "Review apply", can_review).clicked() {
                                    requested_tweak_review = Some((tw.id.clone(), false));
                                }
                            }
                        };
                        let row_width = ui.available_width();
                        if row_width < 640.0 {
                            ui.vertical(|ui| {
                                ui.set_width(row_width);
                                draw_details(ui);
                                ui.add_space(S1);
                                ui.horizontal(|ui| draw_action(ui));
                            });
                        } else {
                            let details_width = (row_width - 184.0).max(120.0);
                            ui.horizontal(|ui| {
                                ui.scope(|ui| {
                                    ui.set_max_width(details_width);
                                    draw_details(ui);
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    draw_action(ui);
                                });
                            });
                        }
                        if !compatible && let Some(reason) = hw::fit_reason(tw, traits) {
                            ui.label(RichText::new(reason).size(T_SMALL).color(t.subtle));
                        }
                    });
                ui.separator();
            }
        });
        if let Some((id, undo)) = requested_tweak_review {
            if undo {
                self.preview_undo_ids(vec![id]);
            } else {
                self.preview_apply_ids(ctx, vec![id]);
            }
        }
        ui.add_space(S2);
        ui.horizontal_wrapped(|ui| {
            let ready = self.busy.is_none() && self.elevated;
            if kit::primary(
                ui,
                "Review compatible settings",
                ready && !to_apply.is_empty(),
            )
            .clicked()
            {
                self.preview_apply_ids(ctx, to_apply);
            }
            if kit::secondary(ui, "Review undo", ready && !to_undo.is_empty()).clicked() {
                self.preview_undo_ids(to_undo);
            }
            if !self.elevated {
                ui.label(
                    RichText::new("Administrator access is required to change Windows settings.")
                        .size(T_SMALL)
                        .color(t.muted),
                );
            }
        });

        ui.add_space(S4);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Per-game graphics preferences")
                    .font(FontId::new(T_HEADING, semibold()))
                    .color(t.text),
            );
            if kit::ghost(
                ui,
                "Refresh",
                !self.game_status_loading && self.game_profiles_error.is_none(),
            )
            .clicked()
            {
                self.game_states.clear();
                self.game_states_loaded = false;
                self.refresh_game_states(ctx);
            }
            if kit::secondary(ui, "Add game…", self.game_profiles_error.is_none()).clicked() {
                self.add_game_executable(ctx);
            }
        });
        ui.label(RichText::new("Choose a Windows GPU preference for a game executable. Optimaxer stores the exact prior registry value and refuses to overwrite a later change made elsewhere.").size(T_SMALL).color(t.muted));
        ui.horizontal(|ui| {
            if kit::secondary(ui, if self.game_library_scanning { "Scanning libraries…" } else { "Find installed games" }, !self.game_library_scanning).clicked() {
                self.scan_game_libraries(ctx);
            }
            ui.label(RichText::new("Reads Steam and Epic manifests; it does not inspect game files or change launcher data.").size(T_SMALL).color(t.subtle));
        });
        if let Some(scan) = self.game_library_scan.clone() {
            if scan.games.is_empty() {
                ui.label(
                    RichText::new(
                        "No installed games found in the detected Steam and Epic libraries.",
                    )
                    .size(T_SMALL)
                    .color(t.muted),
                );
            } else {
                ui.label(
                    RichText::new(format!("{} installed games found", scan.games.len()))
                        .size(T_SMALL)
                        .color(t.muted),
                );
                let mut selected_library: Option<PathBuf> = None;
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for game in &scan.games {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(&game.name).size(T_BODY).color(t.text),
                                        );
                                        kit::chip(ui, game.source.label(), Tone::Neutral);
                                    });
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(game.install_dir.display().to_string())
                                                .size(T_CAPTION)
                                                .color(t.subtle),
                                        )
                                        .truncate(),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if kit::ghost(
                                        ui,
                                        "Choose executable…",
                                        self.game_profiles_error.is_none(),
                                    )
                                    .clicked()
                                    {
                                        selected_library = Some(game.install_dir.clone());
                                    }
                                });
                            });
                            ui.separator();
                        }
                    });
                if let Some(directory) = selected_library {
                    self.add_game_executable_from(ctx, Some(&directory));
                }
            }
            for warning in &scan.warnings {
                ui.label(RichText::new(warning).size(T_CAPTION).color(t.warn));
            }
        }
        if let Some(error) = &self.game_profiles_error {
            ui.add_space(S2);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                kit::chip(ui, "Profiles unavailable", Tone::Bad);
                ui.add(egui::Label::new(RichText::new(error).size(T_SMALL).color(t.bad)).wrap());
            });
        } else if self.game_profiles.is_empty() {
            ui.add_space(S2);
            kit::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("No game executables added.")
                        .font(FontId::new(T_BODY, semibold()))
                        .color(t.text),
                );
                ui.label(RichText::new("Add a game's .exe file to create a per-user Windows graphics preference. No setting changes until you review and confirm it.").size(T_SMALL).color(t.muted));
            });
        } else if self.game_status_loading || !self.game_states_loaded {
            ui.add_space(S2);
            ui.label(
                RichText::new("Reading current Windows graphics preferences…")
                    .size(T_SMALL)
                    .color(t.muted),
            );
        } else {
            let profile_rows = self.game_profiles.clone();
            for profile in profile_rows {
                let state_result = self.game_states.get(&profile.executable);
                let state = state_result
                    .and_then(|result| result.as_ref().ok())
                    .copied();
                let state_error = state_result
                    .and_then(|result| result.as_ref().err())
                    .cloned();
                let preference_history =
                    game_preference_history(&self.operations, &profile.executable);
                let mut requested_action: Option<bool> = None;
                kit::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            let name = std::path::Path::new(&profile.executable)
                                .file_stem()
                                .map_or_else(
                                    || "Game".into(),
                                    |name| name.to_string_lossy().to_string(),
                                );
                            ui.label(
                                RichText::new(name)
                                    .font(FontId::new(T_BODY, semibold()))
                                    .color(t.text),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&profile.executable)
                                        .size(T_SMALL)
                                        .color(t.subtle),
                                )
                                .wrap(),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if state_error.is_some() {
                                kit::chip(ui, "Unavailable", Tone::Bad);
                            } else if let Some(state) = state {
                                let (label, tone) = match state {
                                    ProfileState::WindowsDefault => {
                                        ("Windows default", Tone::Neutral)
                                    }
                                    ProfileState::Selected if profile.has_saved_original() => {
                                        ("Applied by Optimaxer", Tone::Good)
                                    }
                                    ProfileState::Selected => ("Already selected", Tone::Good),
                                    ProfileState::Available => ("Not applied", Tone::Neutral),
                                    ProfileState::RestorePending => ("Restore pending", Tone::Warn),
                                    ProfileState::ChangedElsewhere => {
                                        ("Changed elsewhere", Tone::Warn)
                                    }
                                };
                                kit::chip(ui, label, tone);
                            } else {
                                kit::chip(ui, "Checking", Tone::Neutral);
                            }
                        });
                    });
                    ui.add_space(S1);
                    ui.horizontal_wrapped(|ui| {
                        let mut preference = profile.preference;
                        egui::ComboBox::from_id_salt(("game-gpu", &profile.executable))
                            .selected_text(preference.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut preference,
                                    GraphicsPreference::Windows,
                                    GraphicsPreference::Windows.label(),
                                );
                                ui.selectable_value(
                                    &mut preference,
                                    GraphicsPreference::PowerSaving,
                                    GraphicsPreference::PowerSaving.label(),
                                );
                                ui.selectable_value(
                                    &mut preference,
                                    GraphicsPreference::HighPerformance,
                                    GraphicsPreference::HighPerformance.label(),
                                );
                            });
                        if preference != profile.preference {
                            let mut updated = self.game_profiles.clone();
                            match gaming::set_preference(
                                &gaming::store_path(),
                                &mut updated,
                                &profile.executable,
                                preference,
                            ) {
                                Ok(()) => {
                                    self.game_profiles = updated;
                                    self.game_states_loaded = false;
                                }
                                Err(error) => self.events.push(OptiEvent::Toast(error)),
                            }
                        }
                        if state == Some(ProfileState::Available)
                            && kit::primary(ui, "Review", self.busy.is_none()).clicked()
                        {
                            requested_action = Some(true);
                        }
                        if profile.has_saved_original()
                            && matches!(
                                state,
                                Some(ProfileState::Selected | ProfileState::RestorePending)
                            )
                            && kit::secondary(ui, "Restore original", self.busy.is_none()).clicked()
                        {
                            requested_action = Some(false);
                        }
                        if kit::ghost(ui, "Import preset", profile.can_remove()).clicked() {
                            self.import_game_preference(&profile);
                        }
                        if kit::ghost(ui, "Export preset", true).clicked() {
                            self.export_game_preference(&profile);
                        }
                        if profile.can_remove() && kit::ghost(ui, "Remove", true).clicked() {
                            let mut updated = self.game_profiles.clone();
                            match gaming::remove(
                                &gaming::store_path(),
                                &mut updated,
                                &profile.executable,
                            ) {
                                Ok(()) => {
                                    self.game_profiles = updated;
                                    self.game_states.remove(&profile.executable);
                                    self.game_states_loaded = false;
                                }
                                Err(error) => self.events.push(OptiEvent::Toast(error)),
                            }
                        }
                    });
                    if let Some(error) = state_error {
                        ui.add(
                            egui::Label::new(RichText::new(error).size(T_SMALL).color(t.bad))
                                .wrap(),
                        );
                    }
                    if state == Some(ProfileState::ChangedElsewhere) {
                        ui.add(egui::Label::new(RichText::new("Saved originals are protected. Restore either Optimaxer's selected value or the original Windows value, then refresh to retry.").size(T_SMALL).color(t.warn)).wrap());
                    }
                });
                if let Some(apply) = requested_action {
                    if apply {
                        self.preview_game_apply(ctx, profile.executable.clone());
                    } else {
                        self.preview_game_undo(ctx, profile.executable.clone());
                    }
                }
                egui::CollapsingHeader::new(format!(
                    "Per-game history · {}",
                    preference_history.len()
                ))
                .id_salt(("game-history", &profile.executable))
                .show(ui, |ui| {
                    if preference_history.is_empty() {
                        ui.label(
                            RichText::new(
                                "No graphics preference changes recorded for this executable.",
                            )
                            .size(T_SMALL)
                            .color(t.muted),
                        );
                    } else {
                        for entry in preference_history.iter().take(10) {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(&entry.time)
                                        .size(T_CAPTION)
                                        .monospace()
                                        .color(t.subtle),
                                );
                                ui.label(RichText::new(&entry.title).size(T_SMALL).color(t.text));
                                let tone = match entry.status {
                                    OperationStatus::Warnings => Tone::Warn,
                                    OperationStatus::Error => Tone::Bad,
                                    OperationStatus::Finished | OperationStatus::Unknown => {
                                        Tone::Neutral
                                    }
                                };
                                kit::chip(ui, entry.status.label(), tone);
                            });
                            for detail in entry
                                .details
                                .iter()
                                .filter(|detail| !detail.starts_with("Executable: "))
                            {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(detail).size(T_CAPTION).color(t.muted),
                                    )
                                    .wrap(),
                                );
                            }
                            ui.separator();
                        }
                    }
                });
                ui.add_space(S1);
            }
        }
    }
}
