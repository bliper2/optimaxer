//! Command palette: command list, fuzzy matching and the palette window.

use super::*;

pub(super) const PALETTE_RECENT_ACTION_LIMIT: usize = 8;

#[derive(Clone, Copy)]
pub(super) enum PaletteCommand {
    Page(&'static str),
    RecentActivity,
    Store,
    SearchStore,
    SearchTweaks,
    WinGet,
    Downloads,
    Settings,
    Palette(theme::Palette),
    /// A palette together with the Glass control style, so one choice gives the whole look.
    GlassTheme(theme::Palette),
}

#[derive(Clone, Copy)]
pub(super) struct PaletteEntry {
    pub(super) name: &'static str,
    pub(super) category: &'static str,
    pub(super) hint: &'static str,
    pub(super) command: PaletteCommand,
}

#[derive(Clone)]
pub(super) struct PaletteMatch {
    pub(super) name: std::borrow::Cow<'static, str>,
    pub(super) category: &'static str,
    pub(super) hint: std::borrow::Cow<'static, str>,
    pub(super) command: PaletteCommand,
    pub(super) activity_search: Option<String>,
}

/// Pages the Linux edition has; the other entries describe Windows-only tools and stay out of the palette there.
#[cfg(not(windows))]
const LINUX_PAGES: [&str; 9] = [
    "Overview",
    "Performance",
    "Processes",
    "Activity",
    "Tweaks",
    "Services",
    "Startup",
    "Cleaner",
    "Network",
];

fn available(entry: &&PaletteEntry) -> bool {
    match entry.command {
        #[cfg(not(windows))]
        PaletteCommand::Page(name) => LINUX_PAGES.contains(&name),
        #[cfg(not(windows))]
        PaletteCommand::WinGet => false,
        _ => true,
    }
}

pub(super) const PALETTE_COMMANDS: [PaletteEntry; 22] = [
    PaletteEntry {
        name: "Overview",
        category: "Workspace",
        hint: "System overview",
        command: PaletteCommand::Page("Overview"),
    },
    PaletteEntry {
        name: "Performance",
        category: "Workspace",
        hint: "Live CPU, memory, disk and adapter traffic",
        command: PaletteCommand::Page("Performance"),
    },
    PaletteEntry {
        name: "Black Box",
        category: "Workspace",
        hint: "Flight recorder for investigating slowdowns",
        command: PaletteCommand::Page("Black Box"),
    },
    PaletteEntry {
        name: "Bottleneck Detective",
        category: "Workspace",
        hint: "Find out what is limiting this PC",
        command: PaletteCommand::Page("Bottleneck Detective"),
    },
    PaletteEntry {
        name: "Startup X-Ray",
        category: "Optimization",
        hint: "Boot history and what slowed each boot",
        command: PaletteCommand::Page("Startup X-Ray"),
    },
    PaletteEntry {
        name: "Activity",
        category: "Workspace",
        hint: "Search recorded changes and system-operation results",
        command: PaletteCommand::Page("Activity"),
    },
    PaletteEntry {
        name: "Processes",
        category: "System",
        hint: "Read-only process explorer",
        command: PaletteCommand::Page("Processes"),
    },
    PaletteEntry {
        name: "Diagnostics",
        category: "System",
        hint: "Read-only Windows integrity checks",
        command: PaletteCommand::Page("Diagnostics"),
    },
    PaletteEntry {
        name: "System Snapshot",
        category: "System",
        hint: "Capture and browse local hardware inventory",
        command: PaletteCommand::Page("System Snapshot"),
    },
    PaletteEntry {
        name: "Gaming",
        category: "Optimization",
        hint: "Review reversible Windows gaming settings",
        command: PaletteCommand::Page("Gaming"),
    },
    PaletteEntry {
        name: "Tweaks",
        category: "Optimization",
        hint: "Open the tweak catalog",
        command: PaletteCommand::Page("Tweaks"),
    },
    PaletteEntry {
        name: "Services",
        category: "Optimization",
        hint: "Review Windows services",
        command: PaletteCommand::Page("Services"),
    },
    PaletteEntry {
        name: "Startup",
        category: "Optimization",
        hint: "Manage startup entries",
        command: PaletteCommand::Page("Startup"),
    },
    PaletteEntry {
        name: "Cleaner",
        category: "Optimization",
        hint: "Review and remove selected files",
        command: PaletteCommand::Page("Cleaner"),
    },
    PaletteEntry {
        name: "Network",
        category: "Optimization",
        hint: "Network and DNS tools",
        command: PaletteCommand::Page("Network"),
    },
    PaletteEntry {
        name: "Tools",
        category: "System",
        hint: "Open maintenance tools",
        command: PaletteCommand::Page("Tools"),
    },
    PaletteEntry {
        name: "Store",
        category: "Software",
        hint: "Browse apps",
        command: PaletteCommand::Store,
    },
    PaletteEntry {
        name: "Search apps",
        category: "Software",
        hint: "Focus Store search",
        command: PaletteCommand::SearchStore,
    },
    PaletteEntry {
        name: "WinGet apps",
        category: "Software",
        hint: "Install, upgrade or remove Windows apps",
        command: PaletteCommand::WinGet,
    },
    PaletteEntry {
        name: "Downloads",
        category: "Software",
        hint: "Show download activity",
        command: PaletteCommand::Downloads,
    },
    PaletteEntry {
        name: "Search tweaks",
        category: "Optimization",
        hint: "Filter the tweak catalog by your query",
        command: PaletteCommand::SearchTweaks,
    },
    PaletteEntry {
        name: "Settings",
        category: "Application",
        hint: "Open application settings",
        command: PaletteCommand::Settings,
    },
];

pub(super) const PALETTE_THEME_COMMANDS: [PaletteEntry; 35] = [
    PaletteEntry {
        name: "System palette",
        category: "Appearance",
        hint: "Use the Windows system palette",
        command: PaletteCommand::Palette(theme::Palette::System),
    },
    PaletteEntry {
        name: "Midnight palette",
        category: "Appearance",
        hint: "Set the Midnight palette",
        command: PaletteCommand::Palette(theme::Palette::Midnight),
    },
    PaletteEntry {
        name: "OLED palette",
        category: "Appearance",
        hint: "Set the OLED palette",
        command: PaletteCommand::Palette(theme::Palette::Oled),
    },
    PaletteEntry {
        name: "Graphite palette",
        category: "Appearance",
        hint: "Set the Graphite palette",
        command: PaletteCommand::Palette(theme::Palette::Graphite),
    },
    PaletteEntry {
        name: "Nebula palette",
        category: "Appearance",
        hint: "Set the Nebula palette",
        command: PaletteCommand::Palette(theme::Palette::Nebula),
    },
    PaletteEntry {
        name: "Nord palette",
        category: "Appearance",
        hint: "Set the Nord palette",
        command: PaletteCommand::Palette(theme::Palette::Nord),
    },
    PaletteEntry {
        name: "Arctic palette",
        category: "Appearance",
        hint: "Set the Arctic palette",
        command: PaletteCommand::Palette(theme::Palette::Arctic),
    },
    PaletteEntry {
        name: "Fluent dark palette",
        category: "Appearance",
        hint: "Set the Fluent dark palette",
        command: PaletteCommand::Palette(theme::Palette::FluentDark),
    },
    PaletteEntry {
        name: "Fluent light palette",
        category: "Appearance",
        hint: "Set the Fluent light palette",
        command: PaletteCommand::Palette(theme::Palette::FluentLight),
    },
    PaletteEntry {
        name: "Material dark palette",
        category: "Appearance",
        hint: "Set the Material dark palette",
        command: PaletteCommand::Palette(theme::Palette::MaterialDark),
    },
    PaletteEntry {
        name: "Material light palette",
        category: "Appearance",
        hint: "Set the Material light palette",
        command: PaletteCommand::Palette(theme::Palette::MaterialLight),
    },
    PaletteEntry {
        name: "Emerald palette",
        category: "Appearance",
        hint: "Set the Emerald palette",
        command: PaletteCommand::Palette(theme::Palette::Emerald),
    },
    PaletteEntry {
        name: "Crimson palette",
        category: "Appearance",
        hint: "Set the Crimson palette",
        command: PaletteCommand::Palette(theme::Palette::Crimson),
    },
    PaletteEntry {
        name: "Solarized palette",
        category: "Appearance",
        hint: "Set the Solarized palette",
        command: PaletteCommand::Palette(theme::Palette::Solarized),
    },
    PaletteEntry {
        name: "Porcelain palette",
        category: "Appearance",
        hint: "Set the Porcelain palette",
        command: PaletteCommand::Palette(theme::Palette::Porcelain),
    },
    PaletteEntry {
        name: "Cyberpunk palette",
        category: "Appearance",
        hint: "Set the Cyberpunk palette",
        command: PaletteCommand::Palette(theme::Palette::Cyberpunk),
    },
    PaletteEntry {
        name: "Obsidian palette",
        category: "Appearance",
        hint: "Set the Obsidian palette",
        command: PaletteCommand::Palette(theme::Palette::Obsidian),
    },
    PaletteEntry {
        name: "Spectra palette",
        category: "Appearance",
        hint: "Set the Spectra palette",
        command: PaletteCommand::Palette(theme::Palette::Spectra),
    },
    PaletteEntry {
        name: "Ember palette",
        category: "Appearance",
        hint: "Set the Ember palette",
        command: PaletteCommand::Palette(theme::Palette::Ember),
    },
    PaletteEntry {
        name: "Tokyo Night palette",
        category: "Appearance",
        hint: "Set the Tokyo Night palette",
        command: PaletteCommand::Palette(theme::Palette::TokyoNight),
    },
    PaletteEntry {
        name: "Rose Quartz palette",
        category: "Appearance",
        hint: "Set the Rose Quartz palette",
        command: PaletteCommand::Palette(theme::Palette::RoseQuartz),
    },
    PaletteEntry {
        name: "Aurora palette",
        category: "Appearance",
        hint: "Set the Aurora palette",
        command: PaletteCommand::Palette(theme::Palette::Aurora),
    },
    PaletteEntry {
        name: "Retro Terminal palette",
        category: "Appearance",
        hint: "Set the Retro Terminal palette",
        command: PaletteCommand::Palette(theme::Palette::RetroTerminal),
    },
    PaletteEntry {
        name: "Noir Glass glass theme",
        category: "Appearance",
        hint: "Glass style with the Noir Glass palette",
        command: PaletteCommand::GlassTheme(theme::Palette::NoirGlass),
    },
    PaletteEntry {
        name: "Cobalt glass theme",
        category: "Appearance",
        hint: "Glass style with the Cobalt palette",
        command: PaletteCommand::GlassTheme(theme::Palette::Cobalt),
    },
    PaletteEntry {
        name: "Onyx glass theme",
        category: "Appearance",
        hint: "Glass style with the Onyx palette",
        command: PaletteCommand::GlassTheme(theme::Palette::Onyx),
    },
    PaletteEntry {
        name: "Slate glass theme",
        category: "Appearance",
        hint: "Glass style with the Slate palette",
        command: PaletteCommand::GlassTheme(theme::Palette::Slate),
    },
    PaletteEntry {
        name: "Mica glass theme",
        category: "Appearance",
        hint: "Glass style with the Mica palette",
        command: PaletteCommand::GlassTheme(theme::Palette::Mica),
    },
    PaletteEntry {
        name: "Ubuntu Yaru palette",
        category: "Appearance",
        hint: "Set the Ubuntu Yaru palette",
        command: PaletteCommand::Palette(theme::Palette::Yaru),
    },
    PaletteEntry {
        name: "GNOME Adwaita palette",
        category: "Appearance",
        hint: "Set the GNOME Adwaita palette",
        command: PaletteCommand::Palette(theme::Palette::Adwaita),
    },
    PaletteEntry {
        name: "KDE Breeze palette",
        category: "Appearance",
        hint: "Set the KDE Breeze palette",
        command: PaletteCommand::Palette(theme::Palette::Breeze),
    },
    PaletteEntry {
        name: "Linux Mint palette",
        category: "Appearance",
        hint: "Set the Linux Mint palette",
        command: PaletteCommand::Palette(theme::Palette::Mint),
    },
    PaletteEntry {
        name: "Pop!_OS palette",
        category: "Appearance",
        hint: "Set the Pop!_OS palette",
        command: PaletteCommand::Palette(theme::Palette::Pop),
    },
    PaletteEntry {
        name: "Manjaro palette",
        category: "Appearance",
        hint: "Set the Manjaro palette",
        command: PaletteCommand::Palette(theme::Palette::Manjaro),
    },
    PaletteEntry {
        name: "Custom palette",
        category: "Appearance",
        hint: "Use your custom palette",
        command: PaletteCommand::Palette(theme::Palette::Custom),
    },
];

#[cfg(test)]
pub(super) fn matching_palette_commands(query: &str) -> Vec<PaletteMatch> {
    matching_palette_commands_with_recent(query, &[])
}

pub(super) fn matching_palette_commands_with_recent(
    query: &str,
    recent_actions: &[RecentAction],
) -> Vec<PaletteMatch> {
    let normalized_query = search_chars(query);
    if normalized_query.is_empty() {
        let mut matches = recent_actions
            .iter()
            .map(recent_palette_match)
            .collect::<Vec<_>>();
        matches.extend(
            PALETTE_COMMANDS
                .iter()
                .chain(PALETTE_THEME_COMMANDS.iter())
                .filter(available)
                .map(palette_match)
                .collect::<Vec<_>>(),
        );
        return matches;
    }

    let mut matches = PALETTE_COMMANDS
        .iter()
        .chain(PALETTE_THEME_COMMANDS.iter())
        .filter(available)
        .filter_map(|entry| {
            let name_score = fuzzy_match_score(&normalized_query, entry.name);
            let hint_score = fuzzy_match_score(&normalized_query, entry.hint);
            name_score
                .map(|score| (entry, (true, score)))
                .or_else(|| hint_score.map(|score| (entry, (false, score))))
        })
        .map(|(entry, score)| (palette_match(entry), score))
        .collect::<Vec<_>>();

    matches.extend(recent_actions.iter().filter_map(|action| {
        let name_score = fuzzy_match_score(&normalized_query, &action.title);
        let hint_score = fuzzy_match_score(&normalized_query, &action.hint);
        let category_score = fuzzy_match_score(&normalized_query, "Recent action");
        name_score
            .map(|score| (true, score))
            .or_else(|| hint_score.map(|score| (false, score)))
            .or_else(|| category_score.map(|score| (false, score)))
            .map(|score| (recent_palette_match(action), score))
    }));
    matches.sort_by(|(_, left), (_, right)| right.cmp(left));
    if matches.is_empty() {
        let display_query = query
            .trim()
            .chars()
            .filter(|character| !character.is_control())
            .take(80)
            .collect::<String>();
        return vec![PaletteMatch {
            name: format!("Search tweaks for “{display_query}”").into(),
            category: "Optimization",
            hint: "Filter the tweak catalog by this text".into(),
            command: PaletteCommand::SearchTweaks,
            activity_search: None,
        }];
    }
    matches.into_iter().map(|(entry, _)| entry).collect()
}

pub(super) fn palette_match(entry: &PaletteEntry) -> PaletteMatch {
    PaletteMatch {
        name: entry.name.into(),
        category: entry.category,
        hint: entry.hint.into(),
        command: entry.command,
        activity_search: None,
    }
}

pub(super) fn recent_palette_match(action: &RecentAction) -> PaletteMatch {
    PaletteMatch {
        name: action.title.clone().into(),
        category: "Recent",
        hint: action.hint.clone().into(),
        command: PaletteCommand::RecentActivity,
        activity_search: Some(action.search.clone()),
    }
}

pub(super) fn search_chars(value: &str) -> Vec<char> {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_alphanumeric())
        .collect()
}

pub(super) fn palette_tweak_query(query: &str) -> &str {
    let query = query.trim();
    const PREFIX: &str = "search tweaks";
    if query
        .get(..PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
    {
        query[PREFIX.len()..].trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, ':' | '-')
        })
    } else {
        query
    }
}

pub(super) fn fuzzy_match_score(query: &[char], text: &str) -> Option<i32> {
    let mut searchable = Vec::new();
    let mut word_start = true;
    for character in text.chars() {
        if character.is_alphanumeric() {
            for normalized in character.to_lowercase() {
                searchable.push((normalized, word_start));
                word_start = false;
            }
        } else {
            word_start = true;
        }
    }

    let mut score = 0i32;
    let mut cursor = 0usize;
    let mut previous = None;
    for character in query {
        let (index, (_, starts_word)) = searchable
            .iter()
            .enumerate()
            .skip(cursor)
            .find(|(_, (candidate, _))| candidate == character)?;
        let skipped = index - cursor;
        let consecutive = previous.is_some_and(|last| index == last + 1);
        score += if consecutive { 8 } else { 2 };
        score += if *starts_word { 5 } else { 0 };
        score -= skipped as i32;
        cursor = index + 1;
        previous = Some(index);
    }

    score -= searchable.len().saturating_sub(query.len()) as i32 / 12;
    Some(score)
}

impl OrionApp {
    pub(super) fn open_command_palette(&mut self) {
        self.palette_query.clear();
        self.palette_selected = 0;
        self.show_palette = true;
    }

    pub(super) fn command_palette(&mut self, ctx: &egui::Context) {
        let t = tokens(ctx);
        let recent_actions = self.opti.recent_actions(PALETTE_RECENT_ACTION_LIMIT);
        let matches = matching_palette_commands_with_recent(&self.palette_query, &recent_actions);
        if matches.is_empty() {
            self.palette_selected = 0;
        } else {
            self.palette_selected = self.palette_selected.min(matches.len() - 1);
        }
        let mut chosen: Option<PaletteMatch> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::NULL.with("command-palette"))
            .backdrop_color(Color32::from_black_alpha(if t.dark { 155 } else { 80 }))
            .show(ctx, |ui| {
                ui.set_width(ui.available_width().clamp(240.0, 520.0));
                ui.heading("Command palette");
                ui.add_space(S2);
                let search_id = egui::Id::NULL.with("command-palette-search");
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .id(search_id)
                        .hint_text("Find a page or command…")
                        .desired_width(f32::INFINITY),
                );
                if !search.has_focus() {
                    ctx.memory_mut(|m| m.request_focus(search_id));
                }
                if search.changed() {
                    self.palette_selected = 0;
                }
                ui.add_space(S2);
                egui::ScrollArea::vertical()
                    .max_height(460.0)
                    .show(ui, |ui| {
                        for (index, command) in matches.iter().enumerate() {
                            let response = ui.selectable_label(
                                index == self.palette_selected,
                                format!(
                                    "{}    ·    {}    ·    {}",
                                    command.name, command.category, command.hint
                                ),
                            );
                            if response.clicked() {
                                chosen = Some(command.clone());
                            }
                        }
                    });
                ui.add_space(S2);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("↑ ↓ select   Enter open   Esc close")
                            .size(T_CAPTION)
                            .color(t.muted),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if kit::ghost(ui, "Close", true).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) && !matches.is_empty() {
            self.palette_selected = (self.palette_selected + 1) % matches.len();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) && !matches.is_empty() {
            self.palette_selected = self
                .palette_selected
                .checked_sub(1)
                .unwrap_or(matches.len() - 1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            chosen = matches.get(self.palette_selected).cloned();
        }
        if let Some(command) = chosen {
            let query = self.palette_query.clone();
            self.run_palette_command(ctx, command, &query);
        } else if close || modal.should_close() {
            self.show_palette = false;
        }
    }

    pub(super) fn run_palette_command(
        &mut self,
        ctx: &egui::Context,
        command: PaletteMatch,
        query: &str,
    ) {
        self.show_palette = false;
        match command.command {
            PaletteCommand::Page(name) => {
                self.set_section(ctx, Section::Optimaxer);
                self.opti.show_page(ctx, name);
            }
            PaletteCommand::RecentActivity => {
                self.set_section(ctx, Section::Optimaxer);
                if let Some(search) = command.activity_search.as_deref() {
                    self.opti.search_activity(ctx, search);
                } else {
                    self.opti.show_page(ctx, "Activity");
                }
            }
            PaletteCommand::Store => self.set_section(ctx, Section::Store),
            PaletteCommand::SearchStore => {
                self.set_section(ctx, Section::Store);
                self.detail = None;
                ctx.memory_mut(|m| m.request_focus(self.search_id));
            }
            PaletteCommand::SearchTweaks => {
                self.set_section(ctx, Section::Optimaxer);
                self.opti.search_tweaks(ctx, palette_tweak_query(query));
            }
            PaletteCommand::WinGet => {
                self.set_section(ctx, Section::Store);
                self.show_winget = true;
                self.detail = None;
            }
            PaletteCommand::Downloads => self.show_downloads = true,
            PaletteCommand::Settings => self.show_settings = true,
            PaletteCommand::Palette(palette) => {
                let previous = self.settings.clone();
                self.settings.palette = palette;
                self.reapply_theme(ctx);
                self.persist_appearance(ctx, previous, "Palette");
            }
            PaletteCommand::GlassTheme(palette) => {
                let previous = self.settings.clone();
                self.settings.palette = palette;
                self.settings.style = theme::Style::Glass;
                self.reapply_theme(ctx);
                self.persist_appearance(ctx, previous, "Glass theme");
            }
        }
    }
}
