//! Confirmation dialogs for file-changing actions.

use eframe::egui::{self, Align, Align2, Layout, Vec2};

use super::components::{danger_button, primary_button};
use crate::desktop::state::{MediaSiftApp, PendingAction};

impl MediaSiftApp {
    pub(crate) fn dialogs(&mut self, context: &egui::Context) {
        if let Some(action) = self.pending_action {
            let count = self.selected_unkept().map_or(0, |paths| paths.len());
            egui::Window::new(action.title())
                .id(egui::Id::new("confirm_file_action"))
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .default_width(470.0)
                .show(context, |ui| {
                    ui.label(match action {
                        PendingAction::Recycle => format!(
                            "{count} unchecked duplicate file(s) will move to the Recycle Bin. You can usually restore them later."
                        ),
                        PendingAction::Delete => format!(
                            "{count} unchecked duplicate file(s) will be permanently deleted. This cannot be undone."
                        ),
                        PendingAction::Archive => format!(
                            "{count} unchecked duplicate file(s) will be copied into a ZIP backup. The original files will remain in place."
                        ),
                    });
                    ui.add_space(8.0);
                    ui.label("Files marked Keep are never included in this action.");
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let confirm_label = match action {
                            PendingAction::Recycle => format!("Recycle {count} file(s)"),
                            PendingAction::Delete => format!("Delete {count} file(s) permanently"),
                            PendingAction::Archive => "Choose ZIP location...".to_owned(),
                        };
                        let confirmed = if action == PendingAction::Delete {
                            danger_button(ui, &confirm_label).clicked()
                        } else {
                            primary_button(ui, &confirm_label).clicked()
                        };
                        if confirmed {
                            self.run_action(action);
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_action = None;
                        }
                    });
                });
        }

        if let Some(directory) = self.pending_sort.clone()
            && let Some(confirmed) = confirmation_dialog(
                context,
                "Organize this folder?",
                &format!(
                    "MediaSift will move images with capture dates below:\n{}",
                    directory.display()
                ),
                "Destination folders use Year/Month. Files without a usable capture date stay in place. MediaSift does not provide an undo command.",
                "Organize images",
            )
        {
            if confirmed {
                self.run_sort(directory);
            } else {
                self.pending_sort = None;
            }
        }

        if let Some(directory) = self.pending_date_prefix.clone()
            && let Some(confirmed) = confirmation_dialog(
                context,
                "Rename media files in this folder?",
                &format!(
                    "MediaSift will add YYYY-MM-DD prefixes to known media files below:\n{}",
                    directory.display()
                ),
                "Already-prefixed files are skipped and name collisions are prevented. MediaSift does not provide an undo command.",
                "Add date prefixes",
            )
        {
            if confirmed {
                self.run_date_prefix(directory);
            } else {
                self.pending_date_prefix = None;
            }
        }
    }
}

pub(super) fn confirmation_dialog(
    context: &egui::Context,
    title: &str,
    primary_text: &str,
    detail: &str,
    confirm_label: &str,
) -> Option<bool> {
    let mut confirmed = false;
    let mut cancelled = false;
    egui::Window::new(title)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .show(context, |ui| {
            ui.label(primary_text);
            ui.add_space(8.0);
            ui.label(detail);
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                confirmed = primary_button(ui, confirm_label).clicked();
                cancelled = ui.button("Cancel").clicked();
            });
        });
    if confirmed {
        Some(true)
    } else if cancelled {
        Some(false)
    } else {
        None
    }
}
