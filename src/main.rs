use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use image_sorter::{find_exact_duplicates, format_duplicate_report, sort_images};
use rfd::FileDialog;

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Image Sorter",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::<ImageSorterApp>::default())),
    )
}

enum WorkResult {
    Sorted(Result<usize, String>),
    Scanned(Result<String, String>),
}

#[derive(Default)]
struct ImageSorterApp {
    message: String,
    report: String,
    receiver: Option<Receiver<WorkResult>>,
}

impl ImageSorterApp {
    fn is_working(&self) -> bool {
        self.receiver.is_some()
    }

    fn start_sort(&mut self) {
        let Some(directory) = FileDialog::new()
            .set_title("Select a folder to sort")
            .pick_folder()
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.message = format!("Sorting images in {}...", directory.display());
        self.report.clear();
        std::thread::spawn(move || {
            let result = sort_images(&directory).map_err(|error| error.to_string());
            let _ = sender.send(WorkResult::Sorted(result));
        });
    }

    fn start_duplicate_scan(&mut self) {
        let Some(directory) = FileDialog::new()
            .set_title("Select a folder or drive to scan")
            .pick_folder()
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.message = format!(
            "Scanning {} for exact duplicate images...",
            directory.display()
        );
        self.report.clear();
        std::thread::spawn(move || {
            let result = find_exact_duplicates(&directory)
                .map(|groups| format_duplicate_report(&groups))
                .map_err(|error| error.to_string());
            let _ = sender.send(WorkResult::Scanned(result));
        });
    }

    fn collect_result(&mut self) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };
        self.receiver = None;
        match result {
            WorkResult::Sorted(Ok(moved)) => {
                self.message = format!("Done — {moved} image(s) sorted successfully.");
            }
            WorkResult::Scanned(Ok(report)) => {
                self.message = "Duplicate scan complete.".to_owned();
                self.report = report;
            }
            WorkResult::Sorted(Err(error)) | WorkResult::Scanned(Err(error)) => {
                self.message = format!("Operation failed: {error}");
            }
        }
    }
}

impl eframe::App for ImageSorterApp {
    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        self.collect_result();
        if self.is_working() {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(context, |ui| {
            ui.heading("Image Sorter");
            ui.label("Organize images by date or find exact byte-for-byte duplicates.");
            ui.add_space(12.0);

            ui.add_enabled_ui(!self.is_working(), |ui| {
                if ui.button("Select Directory to Sort").clicked() {
                    self.start_sort();
                }
                if ui.button("Scan for Exact Duplicates").clicked() {
                    self.start_duplicate_scan();
                }
            });

            if !self.message.is_empty() {
                ui.add_space(12.0);
                ui.label(&self.message);
            }
            if !self.report.is_empty() {
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.monospace(&self.report);
                    });
            }
        });
    }
}
