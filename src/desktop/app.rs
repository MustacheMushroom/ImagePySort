//! Native application bootstrap.

use eframe::egui;

use super::{state::MediaSiftApp, ui::configure_style};

pub(crate) fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([720.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MediaSift",
        options,
        Box::new(|creation_context| {
            configure_style(&creation_context.egui_ctx);
            Ok(Box::new(MediaSiftApp::initial()))
        }),
    )
}
