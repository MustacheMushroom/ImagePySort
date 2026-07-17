use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

use eframe::egui;
use image_sorter::{
    ArchiveCompression, DuplicateGroups, FileActionSummary, ScanProgress, archive_files,
    computer_scan_roots, find_exact_duplicates_with_progress, format_scan_report,
    permanently_delete_files, recycle_files, sort_images, unkept_duplicate_paths,
};
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
    Scanned(Result<(DuplicateGroups, Vec<PathBuf>), String>),
    Action(Result<FileActionSummary, String>),
    Progress(ScanProgress),
}
#[derive(Clone, Copy)]
enum PendingAction {
    Recycle,
    Delete,
    Archive,
}
#[derive(Default)]
struct Review {
    groups: DuplicateGroups,
    kept: HashSet<PathBuf>,
    roots: Vec<PathBuf>,
}

#[derive(Default)]
struct ImageSorterApp {
    message: String,
    receiver: Option<Receiver<WorkResult>>,
    review: Option<Review>,
    include_removable: bool,
    include_network: bool,
    extra_roots: Vec<PathBuf>,
    compression: usize,
    pending_action: Option<PendingAction>,
    scan_progress: Option<ScanProgress>,
    scan_report: String,
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
        self.message = format!("Sorting {}...", directory.display());
        std::thread::spawn(move || {
            let _ = sender.send(WorkResult::Sorted(
                sort_images(&directory).map_err(|e| e.to_string()),
            ));
        });
    }
    fn add_folder(&mut self) {
        if let Some(folder) = FileDialog::new()
            .set_title("Add a media scan folder")
            .pick_folder()
            && !self.extra_roots.contains(&folder)
        {
            self.extra_roots.push(folder);
        }
    }
    fn start_media_scan(&mut self) {
        let mut roots = computer_scan_roots(self.include_removable, self.include_network);
        roots.extend(self.extra_roots.iter().cloned());
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            self.message = "No scan locations are selected.".to_owned();
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.message = format!(
            "Scanning {} location(s) for exact media duplicates...",
            roots.len()
        );
        self.review = None;
        self.scan_progress = Some(ScanProgress::default());
        self.scan_report.clear();
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let result = find_exact_duplicates_with_progress(&roots, move |progress| {
                let _ = progress_sender.send(WorkResult::Progress(progress));
            })
            .map(|groups| (groups, roots))
            .map_err(|e| e.to_string());
            let _ = sender.send(WorkResult::Scanned(result));
        });
    }
    fn selected_unkept(&self) -> Result<Vec<PathBuf>, String> {
        let review = self.review.as_ref().ok_or("Run a duplicate scan first.")?;
        unkept_duplicate_paths(&review.groups, &review.kept)
    }
    fn begin_action(&mut self, action: PendingAction) {
        match self.selected_unkept() {
            Ok(paths) if paths.is_empty() => {
                self.message =
                    "All duplicates are currently kept; there is nothing to action.".to_owned()
            }
            Ok(_) => self.pending_action = Some(action),
            Err(error) => self.message = error,
        }
    }
    fn run_action(&mut self, action: PendingAction) {
        let Ok(paths) = self.selected_unkept() else {
            return;
        };
        let archive = if matches!(action, PendingAction::Archive) {
            FileDialog::new()
                .set_title("Save media backup ZIP")
                .add_filter("ZIP archive", &["zip"])
                .save_file()
        } else {
            None
        };
        if matches!(action, PendingAction::Archive) && archive.is_none() {
            return;
        }
        let compression = match self.compression {
            0 => ArchiveCompression::Stored,
            1 => ArchiveCompression::Fast,
            3 => ArchiveCompression::Maximum,
            _ => ArchiveCompression::Default,
        };
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.pending_action = None;
        self.message = "Applying duplicate-file action...".to_owned();
        std::thread::spawn(move || {
            let result = match action {
                PendingAction::Recycle => Ok(recycle_files(&paths)),
                PendingAction::Delete => Ok(permanently_delete_files(&paths)),
                PendingAction::Archive => archive_files(
                    &paths,
                    &archive.expect("archive path selected"),
                    compression,
                ),
            };
            let _ = sender.send(WorkResult::Action(result));
        });
    }
    fn collect_result(&mut self) {
        let results: Vec<_> = self
            .receiver
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        let mut finished = false;
        for result in results {
            match result {
                WorkResult::Progress(progress) => {
                    self.scan_progress = Some(progress);
                }
                WorkResult::Sorted(Ok(moved)) => {
                    finished = true;
                    self.message = format!("Done — {moved} image(s) sorted.")
                }
                WorkResult::Scanned(Ok((groups, roots))) => {
                    finished = true;
                    let kept = groups.values().flatten().cloned().collect();
                    self.scan_report = format_scan_report(
                        &roots,
                        self.scan_progress
                            .as_ref()
                            .unwrap_or(&ScanProgress::default()),
                        &groups,
                    );
                    self.message = format!(
                        "Found {} exact duplicate group(s). All items start marked Keep.",
                        groups.len()
                    );
                    self.review = Some(Review {
                        groups,
                        kept,
                        roots,
                    });
                }
                WorkResult::Action(Ok(summary)) => {
                    finished = true;
                    self.message = format!(
                        "Processed {} file(s); {} failure(s). Run another scan to refresh the review.",
                        summary.processed,
                        summary.failures.len()
                    );
                }
                WorkResult::Sorted(Err(error))
                | WorkResult::Scanned(Err(error))
                | WorkResult::Action(Err(error)) => {
                    finished = true;
                    self.message = format!("Operation failed: {error}");
                }
            }
        }
        if finished {
            self.receiver = None;
        }
    }
    fn select_source(&mut self, root: &PathBuf, only: bool) {
        if let Some(review) = &mut self.review {
            if only {
                review.kept.clear();
            }
            for paths in review.groups.values() {
                for path in paths {
                    if path.starts_with(root) {
                        review.kept.insert(path.clone());
                    }
                }
            }
        }
    }
    fn save_scan_report(&mut self) {
        if self.scan_report.is_empty() {
            return;
        }
        if let Some(path) = FileDialog::new()
            .set_title("Save scan report")
            .add_filter("Text file", &["txt"])
            .set_file_name("media-scan-report.txt")
            .save_file()
        {
            match std::fs::write(&path, &self.scan_report) {
                Ok(()) => self.message = format!("Saved scan report to {}", path.display()),
                Err(error) => self.message = format!("Could not save report: {error}"),
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
            ui.heading("Media Duplicate Review"); ui.label("Exact, byte-for-byte matching for known image, video, and audio formats.");
            ui.separator(); ui.collapsing("Scan options", |ui| {
                ui.checkbox(&mut self.include_removable, "Include removable drives"); ui.checkbox(&mut self.include_network, "Include network drives");
                ui.horizontal(|ui| {
                    if ui.button("Add folder").clicked() {
                        self.add_folder();
                    }
                    if ui.button("Clear added folders").clicked() {
                        self.extra_roots.clear();
                    }
                });
                for root in &self.extra_roots { ui.label(format!("Added: {}", root.display())); }
                ui.label("Local fixed drives are included by default.");
            });
            ui.add_enabled_ui(!self.is_working(), |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Sort Images by EXIF Date").clicked() {
                        self.start_sort();
                    }
                    if ui.button("Scan Computer for Exact Media Duplicates").clicked() {
                        self.start_media_scan();
                    }
                });
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                if self.is_working() { ui.horizontal(|ui| { ui.spinner(); ui.strong("Scan in progress"); }); }
                if !self.message.is_empty() { ui.label(&self.message); }
                if let Some(progress) = &self.scan_progress { ui.label(format!("Folders: {}   Files: {}   Known media: {}   Hashed: {}   Groups: {}", progress.directories_visited, progress.files_visited, progress.media_files_found, progress.files_hashed, progress.duplicate_groups)); }
                if !self.scan_report.is_empty() {
                    ui.horizontal(|ui| {
                        if ui.button("Copy scan report").clicked() {
                            ui.ctx().copy_text(self.scan_report.clone());
                        }
                        if ui.button("Save scan report").clicked() {
                            self.save_scan_report();
                        }
                    });
                }
            });
            if let Some(action) = self.pending_action {
                egui::Window::new("Confirm duplicate-file action").collapsible(false).show(context, |ui| {
                    ui.label(match action { PendingAction::Recycle => "Move every unkept duplicate to the Recycle Bin?", PendingAction::Delete => "Permanently delete every unkept duplicate? This cannot be undone.", PendingAction::Archive => "Create a ZIP backup of every unkept duplicate? Originals will remain in place." });
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() { self.pending_action = None; }
                        if ui.button("Confirm").clicked() { self.run_action(action); }
                    });
                });
            }
            if self.review.is_some() { ui.separator(); self.review_ui(ui); }
        });
    }
}

impl ImageSorterApp {
    fn review_ui(&mut self, ui: &mut egui::Ui) {
        let roots = self
            .review
            .as_ref()
            .map(|review| review.roots.clone())
            .unwrap_or_default();
        ui.heading("Duplicate review");
        ui.label("Checked items are kept. Unchecked items are candidates for the chosen action.");
        ui.horizontal_wrapped(|ui| {
            for root in roots {
                if ui
                    .button(format!("Select all from {}", root.display()))
                    .clicked()
                {
                    self.select_source(&root, false);
                }
                if ui.button(format!("Only {}", root.display())).clicked() {
                    self.select_source(&root, true);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Keep all").clicked()
                && let Some(review) = &mut self.review
            {
                review.kept = review.groups.values().flatten().cloned().collect();
            }
            if ui.button("Keep first in each group").clicked()
                && let Some(review) = &mut self.review
            {
                review.kept = review
                    .groups
                    .values()
                    .filter_map(|paths| paths.first().cloned())
                    .collect();
            }
        });
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                let groups: Vec<_> = self
                    .review
                    .as_ref()
                    .unwrap()
                    .groups
                    .values()
                    .cloned()
                    .collect();
                for (index, paths) in groups.iter().enumerate() {
                    ui.group(|ui| {
                        ui.strong(format!(
                            "Group {} — {} exact copies",
                            index + 1,
                            paths.len()
                        ));
                        for path in paths {
                            let mut keep = self.review.as_ref().unwrap().kept.contains(path);
                            if ui
                                .checkbox(&mut keep, format!("Keep: {}", path.display()))
                                .changed()
                            {
                                let review = self.review.as_mut().unwrap();
                                if keep {
                                    review.kept.insert(path.clone());
                                } else {
                                    review.kept.remove(path);
                                }
                            }
                        }
                    });
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Recycle unkept").clicked() {
                self.begin_action(PendingAction::Recycle);
            }
            if ui.button("Permanently delete unkept").clicked() {
                self.begin_action(PendingAction::Delete);
            }
        });
        ui.horizontal(|ui| {
            ui.label("ZIP compression:");
            egui::ComboBox::from_id_salt("compression")
                .selected_text(["Stored (none)", "Fast", "Default", "Maximum"][self.compression])
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.compression, 0, "Stored (none)");
                    ui.selectable_value(&mut self.compression, 1, "Fast");
                    ui.selectable_value(&mut self.compression, 2, "Default");
                    ui.selectable_value(&mut self.compression, 3, "Maximum");
                });
            if ui.button("Create ZIP backup of unkept").clicked() {
                self.begin_action(PendingAction::Archive);
            }
        });
    }
}
