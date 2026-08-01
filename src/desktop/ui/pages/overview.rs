//! Overview workflow and entry cards.

use eframe::egui;

use super::super::components::{
    info_banner, page_intro, primary_button, secondary_button, workflow_card,
};
use crate::desktop::state::{MediaSiftApp, Page};

impl MediaSiftApp {
    pub(crate) fn overview_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Your media workspace",
            "Choose what you want to accomplish. Every workflow runs locally, and MediaSift confirms file-changing actions first.",
        );

        let two_columns = ui.available_width() >= 760.0;
        if two_columns {
            ui.columns(2, |columns| {
                self.duplicate_workflow_card(&mut columns[0]);
                self.organize_workflow_card(&mut columns[1]);
            });
            ui.add_space(16.0);
            ui.columns(2, |columns| {
                self.rename_workflow_card(&mut columns[0]);
                self.photo_workflow_card(&mut columns[1]);
            });
        } else {
            self.duplicate_workflow_card(ui);
            ui.add_space(12.0);
            self.organize_workflow_card(ui);
            ui.add_space(12.0);
            self.rename_workflow_card(ui);
            ui.add_space(12.0);
            self.photo_workflow_card(ui);
        }

        ui.add_space(20.0);
        info_banner(
            ui,
            "What “exact duplicate” means",
            "Duplicate scans compare file contents byte for byte. Similar-looking photos, edited versions, and differently encoded copies are not treated as duplicates.",
        );
    }

    pub(crate) fn duplicate_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Find exact duplicates",
            "Scan image, video, and audio files; review every match before choosing what happens next.",
            "Read-only until you confirm an action",
            |ui| {
                if primary_button(ui, "Open duplicate finder").clicked() {
                    self.page = Page::Duplicates;
                }
            },
        );
    }

    pub(crate) fn organize_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Sort photos by capture date",
            "Move photos with EXIF capture dates into Year/Month folders while preserving their filenames.",
            "Moves files after confirmation",
            |ui| {
                if secondary_button(ui, "Open organizer").clicked() {
                    self.page = Page::Organize;
                }
            },
        );
    }

    pub(crate) fn rename_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Add dates to filenames",
            "Prefix known media files with YYYY-MM-DD so they sort chronologically in any file browser.",
            "Renames files after confirmation",
            |ui| {
                if secondary_button(ui, "Open rename tool").clicked() {
                    self.page = Page::Organize;
                }
            },
        );
    }

    pub(crate) fn photo_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Restore a photo",
            "Create a conservative enhanced copy, or prepare an identity-preserving prompt for an external AI tool.",
            "Source photo is preserved",
            |ui| {
                if secondary_button(ui, "Open photo tools").clicked() {
                    self.page = Page::PhotoTools;
                }
            },
        );
    }
}
