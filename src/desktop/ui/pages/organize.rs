//! Capture-date sorting and filename-prefix workflows.

use eframe::egui;

use super::super::components::{check_line, info_banner, page_intro, primary_button, section_card};
use crate::desktop::state::MediaSiftApp;

impl MediaSiftApp {
    pub(crate) fn organize_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Organize files",
            "Use capture dates to create a predictable library. Both tools show a confirmation before changing files.",
        );

        let two_columns = ui.available_width() >= 800.0;
        if two_columns {
            ui.columns(2, |columns| {
                self.sort_card(&mut columns[0]);
                self.prefix_card(&mut columns[1]);
            });
        } else {
            self.sort_card(ui);
            ui.add_space(14.0);
            self.prefix_card(ui);
        }
        ui.add_space(18.0);
        info_banner(
            ui,
            "Before you begin",
            "Consider making a backup before reorganizing a large library. MediaSift prevents destination-name collisions, but it does not provide an undo command for moves or renames.",
        );
    }

    pub(crate) fn sort_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Sort into Year/Month folders");
            ui.label("Example: Vacation/photo.jpg → Vacation/2025/Jul/photo.jpg");
            ui.add_space(10.0);
            check_line(ui, "Reads EXIF DateTimeOriginal capture dates");
            check_line(ui, "Leaves photos without a usable capture date in place");
            check_line(ui, "Adds a numeric suffix when a filename already exists");
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Choose folder and review...").clicked() {
                    self.choose_sort_folder();
                }
            });
        });
    }

    pub(crate) fn prefix_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Add dates to media filenames");
            ui.label("Example: photo.jpg → 2025-07-14 - photo.jpg");
            ui.add_space(10.0);
            check_line(ui, "Works with known image, video, and audio files");
            check_line(ui, "Skips files that already start with a date");
            check_line(ui, "Prevents filename collisions");
            ui.add_space(10.0);
            ui.checkbox(
                &mut self.use_oldest_date,
                "Use the older of created and modified dates",
            )
            .on_hover_text("By default, MediaSift uses the file's creation date when available.");
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Choose folder and review...").clicked() {
                    self.choose_date_prefix_folder();
                }
            });
        });
    }
}
