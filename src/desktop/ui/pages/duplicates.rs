//! Exact-duplicate scan, progress, review, and action workflow.

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText};

use super::super::components::{
    danger_button, duplicate_group_card, empty_state, metric_row, number_badge, page_intro,
    primary_button, secondary_button, section_card, success_state, warning_banner,
};
use crate::ScanProgress;
use crate::desktop::state::{MediaSiftApp, NoticeKind, PendingAction, ScanState};

const DUPLICATE_GROUPS_PER_PAGE: usize = 50;
const TWO_COLUMN_WORKFLOW_MIN_WIDTH: f32 = 1_040.0;
const TWO_COLUMN_RESULTS_MIN_WIDTH: f32 = 980.0;

impl MediaSiftApp {
    pub(crate) fn duplicates_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Find exact duplicates",
            "Scan locally, compare contents with SHA-256, then decide which copies to keep. Scanning never changes a file.",
        );

        let progress = self.scan_progress.clone();
        if ui.available_width() >= TWO_COLUMN_WORKFLOW_MIN_WIDTH
            && let Some(progress) = progress.as_ref()
        {
            ui.columns(2, |columns| {
                self.scan_scope_card(&mut columns[0]);
                self.scan_progress_ui(&mut columns[1], progress);
            });
        } else {
            self.scan_scope_card(ui);
            if let Some(progress) = progress.as_ref() {
                ui.add_space(16.0);
                self.scan_progress_ui(ui, progress);
            }
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

    fn scan_scope_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "1");
                ui.vertical(|ui| {
                    ui.heading("Choose scan locations");
                    ui.label(if self.selected_scan_roots.is_empty() {
                        "No folders selected: all local fixed drives will be scanned."
                    } else {
                        "Only the selected folders below will be scanned. Whole-drive scanning is off."
                    });
                });
            });
            ui.add_space(14.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                ui.add_enabled_ui(self.selected_scan_roots.is_empty(), |ui| {
                    ui.checkbox(&mut self.include_removable, "Include removable drives")
                        .on_hover_text(
                            "Used with the automatic whole-drive scope when no folders are selected.",
                        );
                    ui.checkbox(&mut self.include_network, "Include network drives")
                        .on_hover_text(
                            "Used with the automatic whole-drive scope when no folders are selected. Network scans may be much slower.",
                        );
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if secondary_button(ui, "Select one or more folders...").clicked() {
                        self.add_scan_folders();
                    }
                    let clear = ui.add_enabled(
                        !self.selected_scan_roots.is_empty(),
                        egui::Button::new("Use whole-drive default"),
                    );
                    if clear.clicked() {
                        self.selected_scan_roots.clear();
                    }
                });
            });
            if self.selected_scan_roots.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "Automatic scope: all local fixed drives, plus any enabled drive types above.",
                    )
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            } else {
                ui.add_space(10.0);
                let mut remove = None;
                for (index, root) in self.selected_scan_roots.iter().enumerate() {
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
                    self.selected_scan_roots.remove(index);
                }
            }
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                let label = if self.selected_scan_roots.is_empty() {
                    "Scan configured drives"
                } else {
                    "Scan selected folders"
                };
                if primary_button(ui, label).clicked() {
                    self.start_media_scan();
                }
            });
        });
    }

    pub(crate) fn scan_progress_ui(&mut self, ui: &mut egui::Ui, progress: &ScanProgress) {
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, if self.review.is_some() { "✓" } else { "2" });
                ui.vertical(|ui| {
                    let (heading, detail) = match self.scan_state {
                        ScanState::Idle | ScanState::Running => (
                            "Scanning your media",
                            "Large scans can take a while. You can cancel without changing any files.",
                        ),
                        ScanState::Cancelling => (
                            "Stopping the scan",
                            "Finishing the current filesystem step, then the scan will stop.",
                        ),
                        ScanState::Cancelled => (
                            "Scan cancelled",
                            "No files were changed. Adjust the locations above and start again when ready.",
                        ),
                        ScanState::Complete => ("Scan complete", "Review the matches below."),
                        ScanState::Failed => (
                            "Scan stopped",
                            "Review the error below, adjust the locations, and try again.",
                        ),
                    };
                    ui.heading(heading);
                    ui.label(detail);
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
            if self.scan_state.is_active() {
                ui.add_space(12.0);
                ui.add(egui::ProgressBar::new(0.5).animate(true).text(
                    if self.scan_state == ScanState::Cancelling {
                        "Cancelling..."
                    } else {
                        "Scanning..."
                    },
                ));
                ui.add_space(10.0);
                if self.scan_state == ScanState::Running {
                    if secondary_button(ui, "Cancel scan").clicked() {
                        self.cancel_media_scan();
                    }
                } else {
                    ui.add_enabled(false, egui::Button::new("Cancellation requested"));
                }
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
        if review.is_empty() {
            success_state(
                ui,
                "No exact duplicates found",
                "The scanned media files do not contain any byte-for-byte duplicate copies.",
            );
            ui.add_space(10.0);
            self.report_actions(ui);
            return;
        }

        let groups = review.group_count();
        let total_files = review.total_files();
        let extra_copies = review.extra_copies();
        let action_count = review.action_count();
        let roots = review.roots().to_vec();
        let page_count = review.page_count(DUPLICATE_GROUPS_PER_PAGE);
        self.review_page = self.review_page.min(page_count.saturating_sub(1));
        let two_column_workflow = ui.available_width() >= TWO_COLUMN_WORKFLOW_MIN_WIDTH;

        if two_column_workflow {
            ui.columns(2, |columns| {
                self.review_selection_card(
                    &mut columns[0],
                    groups,
                    total_files,
                    extra_copies,
                    action_count,
                    &roots,
                );
                let (action_count, valid) = self.review_status();
                self.review_action_card(&mut columns[1], action_count, valid);
            });
        } else {
            self.review_selection_card(ui, groups, total_files, extra_copies, action_count, &roots);
        }

        ui.add_space(12.0);
        self.review_results(ui, groups, page_count);

        if !two_column_workflow {
            let (action_count, valid) = self.review_status();
            ui.add_space(12.0);
            self.review_action_card(ui, action_count, valid);
        }
    }

    fn review_selection_card(
        &mut self,
        ui: &mut egui::Ui,
        groups: usize,
        total_files: usize,
        extra_copies: usize,
        action_count: usize,
        roots: &[std::path::PathBuf],
    ) {
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "3");
                ui.vertical(|ui| {
                    ui.heading("Choose which copies to keep");
                    ui.label(
                        "Checked files stay in place. Clear Keep to select a copy for action.",
                    );
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
                if secondary_button(ui, "Keep one copy per group").clicked()
                    && let Some(review) = &mut self.review
                {
                    review.keep_first_in_each_group();
                }
            });
            if !roots.is_empty() {
                ui.add_space(10.0);
                ui.collapsing("Prefer copies from a location", |ui| {
                    ui.label("Mark copies in a scanned location as Keep.");
                    for root in roots {
                        Frame::new()
                            .fill(ui.visuals().faint_bg_color)
                            .corner_radius(8)
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.label(root.display().to_string());
                                ui.horizontal_wrapped(|ui| {
                                    if ui.small_button("Keep from here").clicked() {
                                        self.select_source(root, false);
                                    }
                                    if ui.small_button("Keep only from here").clicked() {
                                        self.select_source(root, true);
                                    }
                                });
                            });
                    }
                });
            }
        });
    }

    fn review_results(&mut self, ui: &mut egui::Ui, groups: usize, page_count: usize) {
        let bounds = self
            .review
            .as_ref()
            .expect("review exists")
            .page_bounds(self.review_page, DUPLICATE_GROUPS_PER_PAGE);
        section_card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Duplicate groups {}–{} of {groups}",
                        bounds.start + 1,
                        bounds.end,
                    ))
                    .heading()
                    .strong(),
                );
                if page_count > 1 {
                    ui.separator();
                    ui.add_enabled_ui(self.review_page > 0, |ui| {
                        if ui.small_button("First").clicked() {
                            self.review_page = 0;
                        }
                        if ui.small_button("Previous").clicked() {
                            self.review_page = self.review_page.saturating_sub(1);
                        }
                    });
                    let mut page_number = self.review_page + 1;
                    if ui
                        .add(
                            egui::DragValue::new(&mut page_number)
                                .range(1..=page_count)
                                .prefix("Page "),
                        )
                        .on_hover_text("Type or drag to jump directly to a results page")
                        .changed()
                    {
                        self.review_page = page_number - 1;
                    }
                    ui.label(format!("of {page_count}"));
                    ui.add_enabled_ui(self.review_page + 1 < page_count, |ui| {
                        if ui.small_button("Next").clicked() {
                            self.review_page += 1;
                        }
                        if ui.small_button("Last").clicked() {
                            self.review_page = page_count - 1;
                        }
                    });
                }
            });
            ui.label(
                RichText::new(if page_count > 1 {
                    "Up to 50 groups are shown per page. Checked copies are protected."
                } else {
                    "Checked copies are protected. Clear Keep only on copies you want to process."
                })
                .small()
                .color(ui.visuals().weak_text_color()),
            );
        });
        ui.add_space(10.0);

        let bounds = self
            .review
            .as_ref()
            .expect("review exists")
            .page_bounds(self.review_page, DUPLICATE_GROUPS_PER_PAGE);
        let two_columns = ui.available_width() >= TWO_COLUMN_RESULTS_MIN_WIDTH;
        let changes = {
            let review = self.review.as_ref().expect("review exists");
            let page_groups: Vec<_> = review
                .groups()
                .iter()
                .skip(bounds.start)
                .take(bounds.len())
                .enumerate()
                .collect();
            let mut changes = Vec::new();
            if two_columns {
                ui.columns(2, |columns| {
                    for (offset, (group, paths)) in &page_groups {
                        let column = *offset % 2;
                        changes.extend(duplicate_group_card(
                            &mut columns[column],
                            bounds.start + *offset,
                            group,
                            paths,
                            review,
                        ));
                        columns[column].add_space(10.0);
                    }
                });
            } else {
                for (offset, (group, paths)) in page_groups {
                    changes.extend(duplicate_group_card(
                        ui,
                        bounds.start + offset,
                        group,
                        paths,
                        review,
                    ));
                    ui.add_space(10.0);
                }
            }
            changes
        };
        if let Some(review) = &mut self.review {
            for change in changes {
                review.set_kept(&change.group, &change.path, change.keep);
            }
        }
    }

    fn review_status(&self) -> (usize, bool) {
        self.review.as_ref().map_or((0, true), |review| {
            (review.action_count(), review.every_group_has_keeper())
        })
    }

    fn review_action_card(&mut self, ui: &mut egui::Ui, action_count: usize, valid: bool) {
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "4");
                ui.vertical(|ui| {
                    ui.heading("Process selected copies");
                    ui.label(format!(
                        "{action_count} selected. Every duplicate group must keep at least one copy."
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
