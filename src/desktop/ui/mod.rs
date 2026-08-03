//! Responsive application shell and global visual configuration.

mod components;
mod dialogs;
mod pages;
#[cfg(test)]
mod tests;

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Frame, Key, Layout, Margin, RichText, Stroke,
    TextStyle, Vec2,
};

use super::state::{MediaSiftApp, Operation, Page, ScanState};
use components::{ACCENT, ACCENT_HOVER, CONTENT_MAX_WIDTH, chrome_frame, notice_color};

impl MediaSiftApp {
    fn handle_shortcuts(&mut self, context: &egui::Context) {
        let shortcuts = [
            (Key::Num1, Page::Overview),
            (Key::Num2, Page::Duplicates),
            (Key::Num3, Page::Organize),
            (Key::Num4, Page::PhotoTools),
        ];
        for (key, page) in shortcuts {
            if context.input_mut(|input| input.consume_key(egui::Modifiers::ALT, key)) {
                self.page = page;
            }
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.pending_action = None;
            self.pending_sort = None;
            self.pending_date_prefix = None;
        }
    }

    fn navigation(&mut self, ui: &mut egui::Ui, vertical: bool) {
        let show_item = |ui: &mut egui::Ui,
                         page: Page,
                         label: &'static str,
                         shortcut: &'static str,
                         current: &mut Page| {
            let selected = *current == page;
            let text = if selected {
                RichText::new(label).strong().color(Color32::WHITE)
            } else {
                RichText::new(label)
            };
            let response = ui
                .add_sized(
                    if vertical {
                        [ui.available_width(), 40.0]
                    } else {
                        [0.0, 36.0]
                    },
                    egui::Button::new(text)
                        .selected(selected)
                        .corner_radius(8)
                        .frame(selected),
                )
                .on_hover_text(format!("Open {label} ({shortcut})"));
            if response.clicked() {
                *current = page;
            }
        };

        if vertical {
            for (page, label, shortcut) in Page::ALL {
                show_item(ui, page, label, shortcut, &mut self.page);
                ui.add_space(4.0);
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                for (page, label, shortcut) in Page::ALL {
                    show_item(ui, page, label, shortcut, &mut self.page);
                }
            });
        }
    }

    fn top_bar(&mut self, context: &egui::Context, wide: bool) {
        egui::TopBottomPanel::top("app_header")
            .frame(chrome_frame(context))
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("MS")
                            .strong()
                            .color(Color32::WHITE)
                            .background_color(ACCENT),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new("MediaSift").heading().strong());
                    if wide {
                        ui.separator();
                        ui.label(
                            RichText::new(self.page.heading())
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        theme_menu(ui);
                    });
                });
                if !wide {
                    ui.add_space(8.0);
                    self.navigation(ui, false);
                }
            });
    }

    fn side_bar(&mut self, context: &egui::Context) {
        egui::SidePanel::left("navigation")
            .exact_width(224.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(context.style().visuals.faint_bg_color)
                    .inner_margin(Margin::symmetric(16, 20))
                    .stroke(Stroke::new(
                        1.0_f32,
                        context
                            .style()
                            .visuals
                            .widgets
                            .noninteractive
                            .bg_stroke
                            .color,
                    )),
            )
            .show(context, |ui| {
                ui.label(
                    RichText::new("WORKFLOWS")
                        .small()
                        .strong()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(10.0);
                self.navigation(ui, true);
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new("Exact matching · Local processing")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
            });
    }

    fn status_bar(&mut self, context: &egui::Context) {
        let compact = context.available_rect().width() < 760.0;
        egui::TopBottomPanel::bottom("status")
            .frame(chrome_frame(context))
            .show(context, |ui| {
                let content = |ui: &mut egui::Ui, app: &mut Self| {
                    if let Some(operation) = app.operation {
                        ui.spinner();
                        ui.vertical(|ui| {
                            ui.strong(operation.label());
                            ui.label(&app.notice.text);
                        });
                        if operation == Operation::Scan {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if app.scan_state == ScanState::Running {
                                    if ui.button("Cancel scan").clicked() {
                                        app.cancel_media_scan();
                                    }
                                } else if app.scan_state == ScanState::Cancelling {
                                    ui.add_enabled(false, egui::Button::new("Cancelling..."));
                                }
                            });
                        }
                    } else {
                        let color = notice_color(app.notice.kind, ui.visuals().dark_mode);
                        ui.label(RichText::new(app.notice.label()).strong().color(color));
                        ui.separator();
                        ui.label(&app.notice.text);
                    }
                };
                if compact {
                    ui.vertical(|ui| content(ui, self));
                } else {
                    ui.horizontal(|ui| content(ui, self));
                }
            });
    }
}

impl eframe::App for MediaSiftApp {
    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        self.collect_result();
        self.handle_shortcuts(context);
        if self.is_working() {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let wide = context.available_rect().width() >= 900.0;
        self.top_bar(context, wide);
        self.status_bar(context);
        if wide {
            self.side_bar(context);
        }

        let compact = context.available_rect().width() < 760.0;
        let horizontal_margin = if compact { 14 } else { 24 };
        let vertical_margin = if compact { 14 } else { 22 };
        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::symmetric(horizontal_margin, vertical_margin)))
            .show(context, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("main_content")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = ui.available_width().min(CONTENT_MAX_WIDTH);
                        ui.allocate_ui_with_layout(
                            Vec2::new(width, 0.0),
                            Layout::top_down(Align::Min),
                            |ui| match self.page {
                                Page::Overview => self.overview_ui(ui),
                                Page::Duplicates => self.duplicates_ui(ui),
                                Page::Organize => self.organize_ui(ui),
                                Page::PhotoTools => self.photo_tools_ui(ui),
                            },
                        );
                    });
            });
        self.dialogs(context);
    }
}

pub(crate) fn configure_style(context: &egui::Context) {
    context.all_styles_mut(|style| {
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.spacing.interact_size.y = 34.0;
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(24.0, FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.widgets.active.bg_fill = ACCENT;
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.5_f32, ACCENT_HOVER);
        style.visuals.widgets.inactive.corner_radius = 8.into();
        style.visuals.widgets.hovered.corner_radius = 8.into();
        style.visuals.widgets.active.corner_radius = 8.into();
        style.visuals.window_corner_radius = 12.into();
    });
}

fn theme_menu(ui: &mut egui::Ui) {
    let mut preference = ui.ctx().options(|options| options.theme_preference);
    ui.menu_button("Theme", |ui| {
        ui.set_min_width(150.0);
        if ui
            .selectable_value(
                &mut preference,
                egui::ThemePreference::System,
                "Follow system",
            )
            .clicked()
        {
            ui.ctx().set_theme(preference);
            ui.close();
        }
        if ui
            .selectable_value(&mut preference, egui::ThemePreference::Light, "Light")
            .clicked()
        {
            ui.ctx().set_theme(preference);
            ui.close();
        }
        if ui
            .selectable_value(&mut preference, egui::ThemePreference::Dark, "Dark")
            .clicked()
        {
            ui.ctx().set_theme(preference);
            ui.close();
        }
    })
    .response
    .on_hover_text("Choose light, dark, or the Windows theme");
}
