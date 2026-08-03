//! Native application bootstrap.

use eframe::egui;

use super::{state::MediaSiftApp, ui::configure_style};

pub(crate) fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([720.0, 560.0])
            .with_icon(application_icon()),
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

fn application_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../assets/media-sift-icon.png"))
        .expect("embedded MediaSift application icon must be a valid PNG")
}

#[cfg(test)]
mod tests {
    use super::application_icon;

    #[test]
    fn embedded_application_icon_is_square_rgba() {
        let icon = application_icon();
        assert_eq!(icon.width, icon.height);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
