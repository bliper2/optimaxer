//! Notification centre.

use super::*;

impl OrionApp {
    pub(super) fn notifications(&self) -> Vec<Notification> {
        let mut notifications = Vec::new();
        if self.settings.opti_restart_required {
            notifications.push(Notification {
                id: format!("restart-required:{}", self.settings.opti_restart_uptime),
                kind: NotificationKind::Warning,
                title: "Restart required".into(),
                detail: "A Windows restart is required to finish applying changes.".into(),
                lines: Vec::new(),
                progress: None,
                action: Some(NotificationAction::ReviewActivity),
                dismissible: true,
                persistent: true,
            });
        }
        if let Some(notification) = update_notification(&self.update) {
            notifications.push(notification);
        }
        if let Some(job) = self.opti.current_job() {
            notifications.push(Notification {
                id: format!("operation-running:{job}"),
                kind: NotificationKind::Progress,
                title: job.to_owned(),
                detail: "System operation is running. Progress is not available for this task."
                    .into(),
                lines: Vec::new(),
                progress: None,
                action: Some(NotificationAction::ReviewActivity),
                dismissible: false,
                persistent: false,
            });
        }
        notifications.extend(
            self.opti
                .operation_entries()
                .iter()
                .rev()
                .filter_map(notifications::operation)
                .take(40),
        );
        for download in self.downloads.iter().rev().take(40) {
            if let Some(notification) = notifications::download(download) {
                notifications.push(notification);
            }
        }
        notifications.retain(|notification| {
            !self
                .settings
                .dismissed_notifications
                .contains(&notification.id)
                && !self
                    .dismissed_session_notifications
                    .contains(&notification.id)
        });
        notifications.truncate(notifications::MAX_VISIBLE);
        notifications
    }

    pub(super) fn dismiss_notification(&mut self, notification: &Notification) {
        if !notification.dismissible {
            return;
        }
        if notification.persistent {
            let previous = self.settings.dismissed_notifications.clone();
            if !self
                .settings
                .dismissed_notifications
                .contains(&notification.id)
            {
                notifications::remember_dismissal(
                    &mut self.settings.dismissed_notifications,
                    &notification.id,
                );
                if let Err(error) = self.settings.save_checked() {
                    self.settings.dismissed_notifications = previous;
                    self.say(format!("Could not save notification dismissal: {error}"));
                }
            }
        } else if !self
            .dismissed_session_notifications
            .contains(&notification.id)
        {
            self.dismissed_session_notifications
                .push(notification.id.clone());
            if self.dismissed_session_notifications.len() > MAX_SAVED_DISMISSALS {
                self.dismissed_session_notifications.remove(0);
            }
        }
    }

    pub(super) fn notification_center(&mut self, ctx: &egui::Context) {
        let notifications = self.notifications();
        let mut open = self.show_notifications;
        let mut dismissed = Vec::new();
        let mut action = None;
        egui::Window::new("Notifications")
            .id(egui::Id::NULL.with("notification-center"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .max_width(520.0)
            .max_height((ctx.content_rect().height() - 80.0).max(220.0))
            .anchor(egui::Align2::RIGHT_TOP, vec2(-16.0, 52.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if notifications.is_empty() {
                            ui.label(
                                RichText::new("No current notifications.").color(tokens(ctx).muted),
                            );
                        }
                        for notification in &notifications {
                            kit::card(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal_wrapped(|ui| {
                                    kit::chip(
                                        ui,
                                        notification.kind.label(),
                                        notification.kind.tone(),
                                    );
                                    ui.label(
                                        RichText::new(&notification.title)
                                            .font(FontId::new(T_BODY, semibold())),
                                    );
                                    if notification.dismissible
                                        && kit::ghost(ui, "Dismiss", true).clicked()
                                    {
                                        dismissed.push(notification.id.clone());
                                    }
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&notification.detail)
                                            .size(T_SMALL)
                                            .color(tokens(ctx).muted),
                                    )
                                    .wrap(),
                                );
                                if let Some(progress) = notification.progress {
                                    ui.add(egui::ProgressBar::new(progress).show_percentage());
                                }
                                if !notification.lines.is_empty() {
                                    egui::CollapsingHeader::new("Details")
                                        .id_salt(("notification-details", &notification.id))
                                        .show(ui, |ui| {
                                            for line in &notification.lines {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(line)
                                                            .size(T_CAPTION)
                                                            .color(tokens(ctx).subtle),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                        });
                                }
                                if let Some(item_action) = &notification.action {
                                    let label = match item_action {
                                        NotificationAction::ReviewActivity => "Review activity",
                                        NotificationAction::OpenDownloads => "Open downloads",
                                        NotificationAction::StartUpdate(_) => "Update and restart",
                                        NotificationAction::CheckUpdates => "Check again",
                                        NotificationAction::RevealFile(_) => "Show in folder",
                                    };
                                    if kit::secondary(ui, label, true).clicked() {
                                        action = Some(item_action.clone());
                                    }
                                }
                            });
                            ui.add_space(S2);
                        }
                    });
            });
        self.show_notifications = open;
        for id in dismissed {
            if let Some(notification) = notifications.iter().find(|item| item.id == id) {
                self.dismiss_notification(notification);
            }
        }
        if let Some(action) = action {
            self.run_notification_action(ctx, action);
        }
    }

    pub(super) fn run_notification_action(
        &mut self,
        ctx: &egui::Context,
        action: NotificationAction,
    ) {
        self.show_notifications = false;
        match action {
            NotificationAction::ReviewActivity => {
                self.set_section(ctx, Section::Optimaxer);
                self.opti.show_page(ctx, "Activity");
            }
            NotificationAction::OpenDownloads => self.show_downloads = true,
            NotificationAction::StartUpdate(release) => self.start_install(ctx, release),
            NotificationAction::CheckUpdates => self.start_check(ctx, true),
            NotificationAction::RevealFile(path) => {
                if let Err(error) = reveal(&path) {
                    self.say(format!("Could not show the downloaded file: {error}"));
                }
            }
        }
    }
}
