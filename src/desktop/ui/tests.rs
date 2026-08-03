use std::path::PathBuf;

use eframe::egui::{self, Vec2};

use crate::{DuplicateGroups, ScanProgress};

use super::configure_style;
use crate::desktop::state::{MediaSiftApp, Page, Review, ScanState};

fn test_context(theme: egui::Theme) -> egui::Context {
    let context = egui::Context::default();
    context.set_theme(theme);
    configure_style(&context);
    context
}

fn test_progress() -> ScanProgress {
    ScanProgress {
        directories_visited: 12,
        files_visited: 240,
        media_files_found: 180,
        files_hashed: 80,
        duplicate_groups: 2,
        ..Default::default()
    }
}

fn render_frame(context: &egui::Context, size: Vec2, mut content: impl FnMut(&mut egui::Ui)) {
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| content(ui));
        },
    );
    assert!(!output.shapes.is_empty());
}

fn render_at(size: Vec2, theme: egui::Theme, page: Page, review: Option<Review>) {
    let context = test_context(theme);
    let mut app = MediaSiftApp::initial();
    app.page = page;
    app.review = review;
    if app.review.is_some() {
        app.scan_state = ScanState::Complete;
    }
    app.scan_progress = Some(test_progress());
    render_frame(&context, size, |ui| match app.page {
        Page::Overview => app.overview_ui(ui),
        Page::Duplicates => app.duplicates_ui(ui),
        Page::Organize => app.organize_ui(ui),
        Page::PhotoTools => app.photo_tools_ui(ui),
    });
}

fn render_active_scan(size: Vec2, theme: egui::Theme, scan_state: ScanState) {
    let context = test_context(theme);
    let mut app = MediaSiftApp::initial();
    app.page = Page::Duplicates;
    app.scan_state = scan_state;
    app.selected_scan_roots = vec![
        PathBuf::from("D:/Photos"),
        PathBuf::from("E:/Family videos"),
    ];
    app.scan_progress = Some(test_progress());
    render_frame(&context, size, |ui| app.duplicates_ui(ui));
}

#[test]
fn overview_layout_renders_at_narrow_and_wide_sizes() {
    render_at(
        Vec2::new(720.0, 560.0),
        egui::Theme::Light,
        Page::Overview,
        None,
    );
    render_at(
        Vec2::new(1440.0, 900.0),
        egui::Theme::Dark,
        Page::Overview,
        None,
    );
}

#[test]
fn duplicate_review_renders_in_both_themes() {
    let mut light_review = Review::sample_for_tests();
    light_review.keep_all();
    render_at(
        Vec2::new(820.0, 700.0),
        egui::Theme::Light,
        Page::Duplicates,
        Some(light_review),
    );

    let mut dark_review = Review::sample_for_tests();
    dark_review.keep_first_in_each_group();
    render_at(
        Vec2::new(1280.0, 800.0),
        egui::Theme::Dark,
        Page::Duplicates,
        Some(dark_review),
    );
}

#[test]
fn duplicate_review_clamps_and_renders_the_last_bounded_page() {
    let groups: DuplicateGroups = (0..55_298)
        .map(|index| {
            (
                format!("hash-{index:03}"),
                vec![
                    PathBuf::from(format!("A/{index}.jpg")),
                    PathBuf::from(format!("B/{index}.jpg")),
                ],
            )
        })
        .collect();
    let context = test_context(egui::Theme::Dark);
    let mut app = MediaSiftApp::initial();
    app.page = Page::Duplicates;
    app.review = Some(Review::new(groups, Vec::new()));
    app.review_page = usize::MAX;
    app.scan_state = ScanState::Complete;
    app.scan_progress = Some(test_progress());

    render_frame(&context, Vec2::new(1280.0, 800.0), |ui| {
        app.duplicates_ui(ui);
    });

    assert_eq!(app.review_page, 1_105);
}

#[test]
fn organize_options_render_at_narrow_and_wide_sizes() {
    render_at(
        Vec2::new(720.0, 640.0),
        egui::Theme::Light,
        Page::Organize,
        None,
    );
    render_at(
        Vec2::new(1440.0, 900.0),
        egui::Theme::Dark,
        Page::Organize,
        None,
    );
}

#[test]
fn multi_folder_scan_and_cancellation_states_render_responsively() {
    render_active_scan(
        Vec2::new(720.0, 700.0),
        egui::Theme::Light,
        ScanState::Running,
    );
    render_active_scan(
        Vec2::new(1440.0, 900.0),
        egui::Theme::Dark,
        ScanState::Cancelling,
    );
}
