//! Exact-duplicate scan, progress, review, and action workflow.

use std::path::PathBuf;

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText};

use super::super::components::{
    danger_button, duplicate_group_card, empty_state, metric_row, number_badge, page_intro,
    primary_button, secondary_button, section_card, success_state, warning_banner,
};
use crate::ScanProgress;
use crate::desktop::state::{MediaSiftApp, NoticeKind, PendingAction};

impl MediaSiftApp {
    pub(crate) fn duplicates_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Find exact duplicates",
            "Scan locally, compare contents with SHA-256, then decide which copies to keep. Scanning never changes a file.",
        );

        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "1");
                ui.vertical(|ui| {
                    ui.heading("Choose where to scan");
                    ui.label("Local fixed drives are included automatically.");
                });
            });
            ui.add_space(14.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                ui.checkbox(&mut self.include_removable, "Include removable drives")
                    .on_hover_text("Include connected USB drives and memory cards.");
                ui.checkbox(&mut self.include_network, "Include network drives")
                    .on_hover_text(
                        "Include mapped network locations. This may make the scan much slower.",
                    );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if secondary_button(ui, "Add a specific folder...").clicked() {
                        self.add_folder();
                    }
                    let clear = ui.add_enabled(
                        !self.extra_roots.is_empty(),
                        egui::Button::new("Clear added folders"),
                    );
                    if clear.clicked() {
                        self.extra_roots.clear();
                    }
                });
            });
            if self.extra_roots.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("No extra folders added.")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            } else {
                ui.add_space(10.0);
                let mut remove = None;
                for (index, root) in self.extra_roots.iter().enumerate() {
                    Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .corner_radius(7)
                        .inner_margin(Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(root.display().to_string());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .small_button("Remove")
                                        .on_hover_text("Remove this folder from the scan scope")
                                        .clicked()
                                    {
                                        remove = Some(index);
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
                if let Some(index) = remove {
                    self.extra_roots.remove(index);
                }
            }
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Start read-only scan").clicked() {
                    self.start_media_scan();
                }
            });
        });

        if let Some(progress) = self.scan_progress.clone() {
            ui.add_space(16.0);
            self.scan_progress_ui(ui, &progress);
        }

        if self.review.is_some() {
            ui.add_space(16.0);
            self.review_ui(ui);
        } else if !self.is_working() {
            ui.add_space(16.0);
            empty_state(
                ui,
                "Results will appear here",
                "After the scan, exact matches are grouped together. Every copy starts marked Keep, so nothing is acted on by accident.",
            );
        }
    }

    pub(crate) fn scan_progress_ui(&mut self, ui: &mut egui::Ui, progress: &ScanProgress) {
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, if self.review.is_some() { "✓" } else { "2" });
                ui.vertical(|ui| {
                    ui.heading(if self.review.is_some() {
                        "Scan complete"
                    } else {
                        "Scanning your media"
                    });
                    ui.label(if self.review.is_some() {
                        "Review the matches below."
                    } else {
                        "Large drives can take a while. You can leave this window open in the background."
                    });
                });
            });
            ui.add_space(14.0);
            metric_row(
                ui,
                &[
                    ("Folders", progress.directories_visited),
                    ("Files seen", progress.files_visited),
                    ("Media", progress.media_files_found),
                    ("Hashed", progress.files_hashed),
                    ("Groups", progress.duplicate_groups),
                ],
            );
            if self.is_working() {
                ui.add_space(12.0);
                ui.add(
                    egui::ProgressBar::new(0.5)
                        .animate(true)
                        .text("Scanning..."),
                );
            }
            if !self.scan_report.is_empty() {
                ui.add_space(12.0);
                self.report_actions(ui);
            }
        });
    }

    pub(crate) fn review_ui(&mut self, ui: &mut egui::Ui) {
        let Some(review) = &self.review else {
            return;
        };
        if review.groups.is_empty() {
            success_state(
                ui,
                "No exact duplicates found",
                "The scanned media files do not contain any byte-for-byte duplicate copies.",
            );
            ui.add_space(10.0);
            self.report_actions(ui);
            return;
        }

        let groups = review.groups.len();
        let total_files = review.total_files();
        let extra_copies = review.extra_copies();
        let action_count = review.action_count();
        let valid = review.every_group_has_keeper();
        let roots = review.roots.clone();

        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "3");
                ui.vertical(|ui| {
                    ui.heading("Choose which copies to keep");
                    ui.label("Checked files stay where they are. Unchecked files become selected for the action below.");
                });
            });
            ui.add_space(14.0);
            metric_row(
                ui,
                &[
                    ("Groups", groups),
                    ("Matching files", total_files),
                    ("Extra copies", extra_copies),
                    ("Selected", action_count),
                ],
            );
            ui.add_space(14.0);
            ui.label(RichText::new("Quick selection").strong());
            ui.horizontal_wrapped(|ui| {
                if secondary_button(ui, "Keep everything").clicked()
                    && let Some(review) = &mut self.review
                {
                    review.keep_all();
                }
                if secondary_button(ui, "Keep the first copy in each group").clicked()
                    && let Some(review) = &mut self.review
                {
                    review.keep_first_in_each_group();
                }
            });
            ui.add_space(10.0);
            ui.collapsing("Prefer copies from a location", |ui| {
                ui.label("These controls mark copies in the chosen location as Keep.");
                for root in roots {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(root.display().to_string());
                        if ui.small_button("Keep from here").clicked() {
                            self.select_source(&root, false);
                        }
                        if ui.small_button("Keep only from here").clicked() {
                            self.select_source(&root, true);
                        }
                    });
                }
            });
        });

        ui.add_space(12.0);
        let group_paths: Vec<Vec<PathBuf>> = self
            .review
            .as_ref()
            .expect("review exists")
            .groups
            .values()
            .cloned()
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("duplicate_groups")
            .max_height(500.0)
            .show(ui, |ui| {
                for (index, paths) in group_paths.iter().enumerate() {
                    duplicate_group_card(ui, index, paths, &mut self.review);
                    ui.add_space(10.0);
                }
            });

        ui.add_space(6.0);
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "4");
                ui.vertical(|ui| {
                    ui.heading("Choose what happens to selected copies");
                    ui.label(format!(
                        "{action_count} file(s) are selected. At least one copy in every group must remain marked Keep."
                    ));
                });
            });
            if !valid {
                ui.add_space(12.0);
                warning_banner(
                    ui,
                    "One or more groups have no keeper. Mark at least one file Keep in every group before continuing.",
                );
            }
            ui.add_space(14.0);
            ui.add_enabled_ui(!self.is_working() && valid && action_count > 0, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if primary_button(ui, &format!("Recycle {action_count} file(s)")).clicked() {
                        self.begin_action(PendingAction::Recycle);
                    }
                    if secondary_button(ui, "Create ZIP backup...").clicked() {
                        self.begin_action(PendingAction::Archive);
                    }
                    if danger_button(ui, "Permanently delete...").clicked() {
                        self.begin_action(PendingAction::Delete);
                    }
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("ZIP compression");
                egui::ComboBox::from_id_salt("compression")
                    .selected_text(["None", "Fast", "Default", "Maximum"][self.compression])
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.compression, 0, "None (fastest)");
                        ui.selectable_value(&mut self.compression, 1, "Fast");
                        ui.selectable_value(&mut self.compression, 2, "Default");
                        ui.selectable_value(&mut self.compression, 3, "Maximum (smallest)");
                    });
            });
            ui.add_space(10.0);
            self.report_actions(ui);
        });
    }

    pub(crate) fn report_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if secondary_button(ui, "Copy scan report").clicked() {
                ui.ctx().copy_text(self.scan_report.clone());
                self.set_notice(
                    NoticeKind::Success,
                    "Copied the scan report to the clipboard.",
                );
            }
            if secondary_button(ui, "Save scan report...").clicked() {
                self.save_scan_report();
            }
        });
    }
}
