//! Reusable, stateless UI components and semantic design tokens.

use std::path::PathBuf;

use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, RichText, Stroke};

use crate::desktop::state::{NoticeKind, Review};

pub(super) const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
pub(super) const ACCENT_HOVER: Color32 = Color32::from_rgb(29, 78, 216);
const DANGER: Color32 = Color32::from_rgb(180, 35, 24);
pub(super) const CONTENT_MAX_WIDTH: f32 = 1120.0;

pub(super) fn chrome_frame(context: &egui::Context) -> Frame {
    Frame::new()
        .fill(context.style().visuals.panel_fill)
        .inner_margin(Margin::symmetric(20, 12))
        .stroke(Stroke::new(
            1.0_f32,
            context
                .style()
                .visuals
                .widgets
                .noninteractive
                .bg_stroke
                .color,
        ))
}

pub(super) fn page_intro(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.heading(RichText::new(title).size(30.0).strong());
    ui.add_space(2.0);
    ui.label(
        RichText::new(description)
            .size(16.0)
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(22.0);
}

pub(super) fn section_card<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(ui.visuals().panel_fill)
        .stroke(Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(12)
        .inner_margin(Margin::same(18))
        .show(ui, content)
        .inner
}

pub(super) fn workflow_card(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    safety_note: &str,
    action: impl FnOnce(&mut egui::Ui),
) {
    section_card(ui, |ui| {
        ui.set_min_height(188.0);
        ui.heading(title);
        ui.label(description);
        ui.add_space(12.0);
        ui.label(
            RichText::new(safety_note)
                .small()
                .strong()
                .color(notice_color(NoticeKind::Info, ui.visuals().dark_mode)),
        );
        ui.with_layout(Layout::bottom_up(Align::LEFT), action);
    });
}

pub(super) fn empty_state(ui: &mut egui::Ui, title: &str, description: &str) {
    Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(12)
        .inner_margin(Margin::same(24))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(title);
                ui.label(description);
            });
        });
}

pub(super) fn success_state(ui: &mut egui::Ui, title: &str, description: &str) {
    Frame::new()
        .fill(if ui.visuals().dark_mode {
            Color32::from_rgb(16, 52, 41)
        } else {
            Color32::from_rgb(236, 253, 243)
        })
        .stroke(Stroke::new(
            1.0_f32,
            notice_color(NoticeKind::Success, ui.visuals().dark_mode),
        ))
        .corner_radius(12)
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.heading(title);
            ui.label(description);
        });
}

pub(super) fn info_banner(ui: &mut egui::Ui, title: &str, description: &str) {
    Frame::new()
        .fill(if ui.visuals().dark_mode {
            Color32::from_rgb(21, 39, 70)
        } else {
            Color32::from_rgb(239, 246, 255)
        })
        .stroke(Stroke::new(1.0_f32, ACCENT))
        .corner_radius(10)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.label(description);
        });
}

pub(super) fn warning_banner(ui: &mut egui::Ui, message: &str) {
    Frame::new()
        .fill(if ui.visuals().dark_mode {
            Color32::from_rgb(65, 44, 18)
        } else {
            Color32::from_rgb(255, 250, 235)
        })
        .stroke(Stroke::new(
            1.0_f32,
            notice_color(NoticeKind::Warning, ui.visuals().dark_mode),
        ))
        .corner_radius(8)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.label(RichText::new("Selection needs attention").strong());
            ui.label(message);
        });
}

pub(super) fn check_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("✓")
                .strong()
                .color(notice_color(NoticeKind::Success, ui.visuals().dark_mode)),
        );
        ui.label(text);
    });
}

pub(super) fn number_badge(ui: &mut egui::Ui, number: &str) {
    Frame::new()
        .fill(ACCENT)
        .corner_radius(20)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.label(RichText::new(number).strong().color(Color32::WHITE));
        });
}

pub(super) fn metric_row(ui: &mut egui::Ui, metrics: &[(&str, usize)]) {
    ui.horizontal_wrapped(|ui| {
        for (label, value) in metrics {
            Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(value.to_string()).size(20.0).strong());
                        ui.label(
                            RichText::new(*label)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                });
        }
    });
}

pub(super) struct KeepChange {
    pub(super) group: String,
    pub(super) path: PathBuf,
    pub(super) keep: bool,
}

pub(super) fn duplicate_group_card(
    ui: &mut egui::Ui,
    index: usize,
    group: &str,
    paths: &[PathBuf],
    review: &Review,
) -> Vec<KeepChange> {
    let mut changes = Vec::new();
    section_card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Group {}", index + 1))
                    .heading()
                    .strong(),
            );
            ui.label(
                RichText::new(format!("{} exact copies", paths.len()))
                    .color(ui.visuals().weak_text_color()),
            );
        });
        ui.add_space(8.0);
        for path in paths {
            let mut keep = review.is_kept(path);
            let filename = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into(),
            );
            Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .checkbox(&mut keep, "Keep")
                            .on_hover_text("Checked files will not be included in any action")
                            .changed()
                        {
                            changes.push(KeepChange {
                                group: group.to_owned(),
                                path: path.clone(),
                                keep,
                            });
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(filename).strong());
                            ui.label(
                                RichText::new(path.display().to_string())
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        });
                    });
                });
            ui.add_space(5.0);
        }
    });
    changes
}

pub(super) fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .corner_radius(8),
    )
}

pub(super) fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(label).corner_radius(8))
}

pub(super) fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
            .fill(DANGER)
            .stroke(Stroke::NONE)
            .corner_radius(8),
    )
}

pub(super) fn notice_color(kind: NoticeKind, dark_mode: bool) -> Color32 {
    match (kind, dark_mode) {
        (NoticeKind::Info, false) => Color32::from_rgb(29, 78, 216),
        (NoticeKind::Info, true) => Color32::from_rgb(147, 197, 253),
        (NoticeKind::Success, false) => Color32::from_rgb(2, 122, 72),
        (NoticeKind::Success, true) => Color32::from_rgb(110, 231, 183),
        (NoticeKind::Warning, false) => Color32::from_rgb(181, 71, 8),
        (NoticeKind::Warning, true) => Color32::from_rgb(253, 186, 116),
        (NoticeKind::Error, false) => DANGER,
        (NoticeKind::Error, true) => Color32::from_rgb(253, 164, 175),
    }
}
