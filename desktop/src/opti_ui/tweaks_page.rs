//! Tweaks page of the Optimaxer section.

use super::*;

impl OptiUi {
    pub(super) fn tweaks_page(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        settings: &mut Settings,
    ) {
        let t = tokens(ctx);
        kit::page_header(
            ui,
            "Tweaks",
            "Performance, gaming, network, privacy and interface settings. The original value is saved before each change, so every tweak can be undone.",
        );
        let (on, total) = self
            .traits
            .as_ref()
            .map(|tr| score(&self.cat, tr, &self.status))
            .unwrap_or((0, 0));
        kit::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let draw_stats = |ui: &mut Ui| {
                if total > 0 {
                    ui.label(
                        RichText::new(format!("{on}/{total}"))
                            .font(FontId::new(theme::T_DISPLAY + 8.0, semibold()))
                            .color(t.text),
                    );
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("recommended tweaks are active")
                                .font(FontId::new(T_BODY, semibold()))
                                .color(t.text),
                        );
                        ui.label(
                            RichText::new(format!("{total} of the recommended set fit this PC"))
                                .size(T_SMALL)
                                .color(t.muted),
                        );
                    });
                } else {
                    kit::skeleton(ui, 2);
                }
            };
            let ready = self.busy.is_none() && self.traits.is_some();
            let mut draw_action = |ui: &mut Ui| {
                if kit::primary(ui, "Smart optimize", ready).on_hover_text("Reads your hardware (laptop or desktop, SSD or HDD, RAM, GPU) and selects only the tweaks that fit this PC").clicked() {
                    self.select_preset("auto");
                    if !self.selected.is_empty() {
                        self.preview_apply(ctx);
                    }
                }
            };
            if ui.available_width() < 520.0 {
                ui.horizontal(|ui| draw_stats(ui));
                ui.add_space(S2);
                ui.horizontal(|ui| draw_action(ui));
            } else {
                ui.horizontal(|ui| {
                    draw_stats(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| draw_action(ui));
                });
            }
        });
        ui.add_space(S3);
        let level = usize::from(settings.tweak_experience.min(2));
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Detail").size(T_SMALL).color(t.muted));
            let mut chosen = level;
            if kit::segmented(ui, &mut chosen, &["Beginner", "Enthusiast", "Expert"]) {
                let previous = settings.tweak_experience;
                settings.tweak_experience = chosen as u8;
                if let Err(error) = settings.save_checked() {
                    settings.tweak_experience = previous;
                    let message = format!("The detail level was not saved: {error}");
                    self.say(&message);
                    self.events.push(OptiEvent::Toast(message));
                }
            }
            ui.label(
                RichText::new(
                    [
                        "Low-risk tweaks that suit this PC, in plain language.",
                        "The full catalog with search, filters, risk and compatibility.",
                        "Exact settings, saved originals and trade-offs for every tweak.",
                    ][level],
                )
                .size(T_CAPTION)
                .color(t.subtle),
            );
        });
        ui.add_space(S2);
        let beginner = level == 0;
        let expert = level == 2;
        if expert {
            let stale = self
                .tw_records
                .0
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(5));
            if stale {
                self.tw_records = (Some(Instant::now()), State::load().records);
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Presets").size(T_SMALL).color(t.muted));
            for (label, tag) in [
                ("Auto-detect", "auto"),
                ("Recommended", "safe"),
                ("Gaming", "gaming"),
                ("Privacy", "privacy"),
                ("Remove AI and debloat", "deai"),
                ("Maximum", "max"),
                ("Clear", "none"),
            ] {
                if beginner && !matches!(tag, "auto" | "safe" | "none") {
                    continue;
                }
                if kit::secondary(ui, label, true).clicked() {
                    self.select_preset(tag);
                }
            }
        });
        if !beginner {
            self.profiles_section(ui);
        }
        ui.horizontal_wrapped(|ui| {
            let mut q = std::mem::take(&mut self.tw_search);
            let search = self.toolbar_search(ui, &mut q, "Search tweaks");
            self.tw_search = q;
            if self.focus_tweak_search {
                search.request_focus();
                self.focus_tweak_search = false;
            }
            if kit::ghost(ui, "Refresh", self.busy.is_none()).clicked() {
                self.refresh_status(ctx);
            }
        });
        ui.add_space(S2);
        ui.horizontal_wrapped(|ui| {
            if beginner {
                return;
            }
            ui.label(RichText::new("Risk").size(T_SMALL).color(t.muted));
            kit::segmented(
                ui,
                &mut self.tw_risk,
                &["All", "Safe", "Moderate", "Advanced"],
            );
            ui.add_space(S3);
            ui.label(RichText::new("Show").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-group")
                .selected_text(TWEAK_GROUPS[self.tw_group.min(TWEAK_GROUPS.len() - 1)])
                .show_ui(ui, |ui| {
                    for (i, g) in TWEAK_GROUPS.iter().enumerate() {
                        ui.selectable_value(&mut self.tw_group, i, *g);
                    }
                });
            ui.label(RichText::new("State").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-state")
                .selected_text(["All", "Applied", "Not applied"][self.tw_state.min(2)])
                .show_ui(ui, |ui| {
                    for (i, label) in ["All", "Applied", "Not applied"].iter().enumerate() {
                        ui.selectable_value(&mut self.tw_state, i, *label);
                    }
                });
            ui.label(RichText::new("View").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-view")
                .selected_text(["All tweaks", "Recommended", "Favorites"][self.tw_view.min(2)])
                .show_ui(ui, |ui| {
                    for (i, label) in ["All tweaks", "Recommended", "Favorites"]
                        .iter()
                        .enumerate()
                    {
                        ui.selectable_value(&mut self.tw_view, i, *label);
                    }
                });
            ui.label(RichText::new("Compatibility").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-compatibility")
                .selected_text(
                    ["All PCs", "Fits this PC", "Does not fit"][self.tw_compatibility.min(2)],
                )
                .show_ui(ui, |ui| {
                    for (i, label) in ["All PCs", "Fits this PC", "Does not fit"]
                        .iter()
                        .enumerate()
                    {
                        ui.selectable_value(&mut self.tw_compatibility, i, *label);
                    }
                });
            ui.label(RichText::new("Sort").size(T_SMALL).color(t.muted));
            egui::ComboBox::from_id_salt("tw-sort")
                .selected_text(["Category", "Name", "Risk", "Status"][self.tw_sort.min(3)])
                .show_ui(ui, |ui| {
                    for (i, label) in ["Category", "Name", "Risk", "Status"].iter().enumerate() {
                        ui.selectable_value(&mut self.tw_sort, i, *label);
                    }
                });
            let filters_active = self.tw_risk != 0
                || self.tw_group != 0
                || self.tw_state != 0
                || self.tw_view != 0
                || self.tw_compatibility != 0
                || self.tw_sort != 0;
            if filters_active && kit::ghost(ui, "Reset filters", true).clicked() {
                self.tw_risk = 0;
                self.tw_group = 0;
                self.tw_state = 0;
                self.tw_view = 0;
                self.tw_compatibility = 0;
                self.tw_sort = 0;
            }
        });
        ui.add_space(S2);

        let q = self.tw_search.trim().to_lowercase();
        // Beginner ignores filters it does not show, so a filter set earlier cannot hide the guided tweaks.
        let (view_risk, group_filter, view_state, view_set, view_fit, view_sort) = if beginner {
            (0, 0, 0, 0, 0, 0)
        } else {
            (
                self.tw_risk,
                self.tw_group,
                self.tw_state,
                self.tw_view,
                self.tw_compatibility,
                self.tw_sort,
            )
        };
        let risk_filter = ["", "Safe", "Moderate", "Advanced"][view_risk.min(3)];
        let mut cats: Vec<String> = Vec::new();
        for tw in &self.cat.tweaks {
            if !cats.contains(&tw.cat) {
                cats.push(tw.cat.clone());
            }
        }
        if view_sort == 1 {
            cats.sort_by_key(|category| category.to_lowercase());
        }
        let cat = self.cat.clone();
        let traits = self.traits.clone();
        let favorites: HashSet<String> = settings.opti_tweak_favorites.iter().cloned().collect();
        let mut shown = 0;
        for c in cats {
            let mut rows: Vec<&Tweak> = cat
                .tweaks
                .iter()
                .filter(|tw| {
                    // a retired tweak is listed only while it has a saved record, so it can still be undone
                    let applied = self.status.get(&tw.id).copied().unwrap_or(false);
                    let fits = traits.as_ref().is_none_or(|tr| hw::fits(tw, tr));
                    (tw.retired.is_empty() || applied)
                        && tw.cat == c.as_str()
                        && (q.is_empty()
                            || tw.name.to_lowercase().contains(&q)
                            || tw.desc.to_lowercase().contains(&q))
                        && (risk_filter.is_empty() || tw.risk == risk_filter)
                        && in_group(tw, group_filter, applied)
                        && (!beginner || tweak_facts::guided(tw, fits))
                        && tweak_matches_view(
                            tw,
                            applied,
                            fits,
                            favorites.contains(&tw.id),
                            view_state,
                            view_set,
                            view_fit,
                        )
                })
                .collect();
            match view_sort {
                1 => rows.sort_by_cached_key(|tweak| tweak.name.to_lowercase()),
                2 => rows.sort_by_cached_key(|tweak| {
                    (risk_order(&tweak.risk), tweak.name.to_lowercase())
                }),
                3 => rows.sort_by_cached_key(|tweak| {
                    (
                        std::cmp::Reverse(self.status.get(&tweak.id).copied().unwrap_or(false)),
                        tweak.name.to_lowercase(),
                    )
                }),
                _ => {}
            }
            if rows.is_empty() {
                continue;
            }
            shown += rows.len();
            egui::CollapsingHeader::new(
                RichText::new(format!("{c}   {}", rows.len()))
                    .font(FontId::new(T_BODY, semibold()))
                    .color(t.brand_text),
            )
            .id_salt(c)
            .default_open(true)
            .show(ui, |ui| {
                for tw in rows {
                    let applied = self.status.get(&tw.id).copied().unwrap_or(false);
                    let fits = traits.as_ref().is_none_or(|tr| hw::fits(tw, tr));
                    let on = self.selected.contains(&tw.id);
                    let open = expert || (!beginner && self.tw_open.contains(&tw.id));
                    let mut details_rect = egui::Rect::NOTHING;
                    let mut favorite_rect = egui::Rect::NOTHING;
                    let favorite = settings.opti_tweak_favorites.iter().any(|id| id == &tw.id);
                    let r = kit::row(ui, ("tw", &tw.id), on, |ui| {
                        ui.horizontal(|ui| {
                            kit::check(ui, on);
                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(&tw.name)
                                            .font(FontId::new(T_BODY, semibold()))
                                            .color(t.text),
                                    );
                                    let favorite_label = ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new(if favorite {
                                                    "Favorited"
                                                } else {
                                                    "Favorite"
                                                })
                                                .size(T_CAPTION)
                                                .color(if favorite {
                                                    t.brand_text
                                                } else {
                                                    t.subtle
                                                }),
                                            )
                                            .sense(Sense::click()),
                                        )
                                        .on_hover_text(if favorite {
                                            "Remove from favorites"
                                        } else {
                                            "Add to favorites"
                                        });
                                    favorite_rect = favorite_label.rect;
                                    if !tw.risk.is_empty() && (!beginner && tw.risk != "Safe") {
                                        kit::chip(ui, &tw.risk, risk_tone(&tw.risk));
                                    } else if !tw.risk.is_empty() && !beginner {
                                        kit::chip(ui, "Safe", Tone::Good);
                                    }
                                    if !beginner {
                                        if tw.restart {
                                            kit::chip(ui, "Restart needed", Tone::Neutral);
                                        }
                                        let (kind, _) = tweak_facts::rollback(tw);
                                        kit::chip(
                                            ui,
                                            kind.label(),
                                            match kind {
                                                tweak_facts::Rollback::Reversible => Tone::Good,
                                                tweak_facts::Rollback::Conditional => Tone::Warn,
                                                tweak_facts::Rollback::Irreversible => Tone::Bad,
                                            },
                                        );
                                    }
                                    if applied {
                                        kit::chip(ui, "Applied", Tone::Good);
                                    }
                                    if !fits {
                                        kit::chip(ui, "Does not suit this PC", Tone::Neutral);
                                    }
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&tw.desc).size(T_SMALL).color(t.muted),
                                    )
                                    .wrap(),
                                );
                                if beginner {
                                    let (kind, _) = tweak_facts::rollback(tw);
                                    let undo = match kind {
                                        tweak_facts::Rollback::Reversible => "Easy to undo.",
                                        _ => "Can be undone while Optimaxer's saved original is intact.",
                                    };
                                    let restart = if tw.restart { " Needs a restart." } else { "" };
                                    ui.label(
                                        RichText::new(format!("{undo}{restart}"))
                                            .size(T_CAPTION)
                                            .color(t.subtle),
                                    );
                                } else {
                                    let notes = tweak_notes(tw);
                                    if !notes.is_empty() {
                                        ui.label(RichText::new(notes).size(T_CAPTION).color(t.subtle));
                                    }
                                }
                                if let Some(why) = self.tw_skip.get(&tw.id) {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "Smart optimize did not tick this: {why}"
                                            ))
                                            .size(T_CAPTION)
                                            .color(t.warn),
                                        )
                                        .wrap(),
                                    );
                                }
                                if !beginner && !expert {
                                    let toggle = ui.add(
                                        egui::Label::new(
                                            RichText::new(if open {
                                                "Hide technical details"
                                            } else {
                                                "Technical details"
                                            })
                                            .size(T_CAPTION)
                                            .color(t.brand_text),
                                        )
                                        .sense(Sense::click()),
                                    );
                                    details_rect = toggle.rect;
                                }
                                if open {
                                    let lines = tweak_technical(tw).into_iter().chain(
                                        tweak_fact_lines(tw, self.tw_records.1.get(&tw.id), applied),
                                    );
                                    for line in lines {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(line)
                                                    .monospace()
                                                    .size(T_CAPTION)
                                                    .color(t.muted),
                                            )
                                            .wrap(),
                                        );
                                    }
                                }
                            });
                        });
                    });
                    let click = ui.interact(
                        r.response.rect,
                        ui.scope_id().with(("tw-click", &tw.id)),
                        Sense::click(),
                    );
                    if click.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if click.has_focus() {
                        ui.painter().rect_stroke(
                            r.response.rect,
                            CornerRadius::same(t.ctl_radius()),
                            Stroke::new(2.0, t.ring()),
                            StrokeKind::Inside,
                        );
                    }
                    if click.clicked()
                        && ctx
                            .pointer_interact_pos()
                            .is_some_and(|p| favorite_rect.contains(p))
                    {
                        let previous_favorites = settings.opti_tweak_favorites.clone();
                        if favorite {
                            settings.opti_tweak_favorites.retain(|id| id != &tw.id);
                        } else {
                            settings.opti_tweak_favorites.push(tw.id.clone());
                            settings.opti_tweak_favorites.sort();
                            settings.opti_tweak_favorites.dedup();
                        }
                        if let Err(error) = settings.save_checked() {
                            settings.opti_tweak_favorites = previous_favorites;
                            let message = format!("Favorite could not be saved: {error}");
                            self.say(&message);
                            self.events.push(OptiEvent::Toast(message));
                        }
                    } else if click.clicked()
                        && ctx
                            .pointer_interact_pos()
                            .is_some_and(|p| details_rect.contains(p))
                    {
                        if !self.tw_open.remove(&tw.id) {
                            self.tw_open.insert(tw.id.clone());
                        }
                    } else if click.clicked() {
                        if on {
                            self.selected.remove(&tw.id);
                        } else {
                            if !tw.conflict.is_empty() {
                                // one choice per group: ticking this unticks the other tweaks that write the same setting
                                let peers: Vec<String> = cat
                                    .tweaks
                                    .iter()
                                    .filter(|p| p.conflict == tw.conflict && p.id != tw.id)
                                    .map(|p| p.id.clone())
                                    .collect();
                                for p in peers {
                                    self.selected.remove(&p);
                                }
                            }
                            self.selected.insert(tw.id.clone());
                        }
                    }
                }
            });
        }
        if shown == 0 {
            kit::empty_state(
                ui,
                "No tweaks match",
                "Try a shorter search or reset the filters.",
            );
        }
        if !cat.not_ported.is_empty() && q.is_empty() {
            ui.add_space(S4);
            ui.add(egui::Label::new(RichText::new(format!("{} tweaks exist only in the PowerShell edition of Optimaxer because they need cmdlets such as Appx or MMAgent: {}", cat.not_ported.len(), cat.not_ported.iter().map(|s| format!("{} ({})", s.name, s.id)).collect::<Vec<_>>().join(", "))).size(T_CAPTION).color(t.subtle)).wrap());
        }
    }
}
