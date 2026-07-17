use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

use eframe::egui;
use image_sorter::{
    ArchiveCompression, DuplicateGroups, FileActionSummary, archive_files, computer_scan_roots,
    find_exact_duplicates_in_roots, permanently_delete_files, recycle_files, sort_images,
    unkept_duplicate_paths,
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
        std::thread::spawn(move || {
            let result = find_exact_duplicates_in_roots(&roots)
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
        let Some(receiver) = &self.receiver else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };
        self.receiver = None;
        match result {
            WorkResult::Sorted(Ok(moved)) => {
                self.message = format!("Done — {moved} image(s) sorted.")
            }
            WorkResult::Scanned(Ok((groups, roots))) => {
                let kept = groups.values().flatten().cloned().collect();
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
                self.message = format!(
                    "Processed {} file(s); {} failure(s). Run another scan to refresh the review.",
                    summary.processed,
                    summary.failures.len()
                );
            }
            WorkResult::Sorted(Err(error))
            | WorkResult::Scanned(Err(error))
            | WorkResult::Action(Err(error)) => self.message = format!("Operation failed: {error}"),
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
}

impl eframe::App for ImageSorterApp {
    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        self.collect_result();
        if self.is_working() {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }
        egui::CentralPanel::default().show(context, |ui| {
            ui.heading("Media Sorter & Exact Duplicate Review"); ui.label("Only known image, video, and audio formats are scanned. Duplicate matching is byte-for-byte exact.");
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
            if !self.message.is_empty() { ui.add_space(8.0); ui.label(&self.message); }
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
