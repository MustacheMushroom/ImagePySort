//! Local enhancement and restoration-prompt workflows.

use eframe::egui;

use super::super::components::{
    check_line, page_intro, primary_button, secondary_button, section_card,
};
use crate::desktop::state::MediaSiftApp;

impl MediaSiftApp {
    pub(crate) fn photo_tools_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Photo tools",
            "Make a conservative enhanced copy locally, or copy a careful restoration prompt for use in another tool.",
        );
        let two_columns = ui.available_width() >= 800.0;
        if two_columns {
            ui.columns(2, |columns| {
                self.enhance_card(&mut columns[0]);
                self.prompt_card(&mut columns[1]);
            });
        } else {
            self.enhance_card(ui);
            ui.add_space(14.0);
            self.prompt_card(ui);
        }
    }

    pub(crate) fn enhance_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Create an enhanced copy");
            ui.label("Apply mild local cleanup, contrast, and color adjustments.");
            ui.add_space(10.0);
            check_line(ui, "Never replaces the source photo");
            check_line(ui, "Saves a sibling file with “(Enhanced)” in its name");
            check_line(ui, "Supports PNG, BMP, and TIFF input");
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Choose a photo...").clicked() {
                    self.start_photo_enhancement();
                }
            });
        });
    }

    pub(crate) fn prompt_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Copy an AI restoration prompt");
            ui.label(
                "Prepare a conservative prompt that prioritizes identity and historical detail.",
            );
            ui.add_space(10.0);
            check_line(ui, "Names the selected file in the prompt");
            check_line(ui, "Requests no identity or facial-structure changes");
            check_line(ui, "Copies text only; MediaSift uploads nothing");
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if secondary_button(ui, "Choose photo and copy prompt").clicked() {
                    self.copy_restoration_prompt(ui.ctx());
                }
            });
        });
    }
}
