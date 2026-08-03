//! Background operations and file-dialog orchestration.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::mpsc,
};

use eframe::egui;
use rfd::FileDialog;

use crate::{
    ArchiveCompression, ScanProgress, archive_files, computer_scan_roots, enhance_photo,
    find_exact_duplicates_with_progress, format_scan_report, permanently_delete_files,
    prefix_media_files_with_date, recycle_files, restoration_prompt, sort_images,
    unkept_duplicate_paths,
};

use super::state::{
    MediaSiftApp, Notice, NoticeKind, Operation, PendingAction, Review, WorkResult,
};

enum FolderOpenOutcome {
    NotRequested,
    Opened,
    Failed(String),
}

fn organize_completion_notice(
    kind: NoticeKind,
    message: String,
    directory: &Path,
    outcome: FolderOpenOutcome,
) -> Notice {
    match outcome {
        FolderOpenOutcome::NotRequested => Notice::new(kind, message),
        FolderOpenOutcome::Opened => Notice::new(
            kind,
            format!("{message} Opened {} in File Explorer.", directory.display()),
        ),
        FolderOpenOutcome::Failed(error) => Notice::new(
            NoticeKind::Warning,
            format!(
                "{message} The file operation completed, but MediaSift could not open {} in File Explorer: {error}. Open the folder manually to review the results.",
                directory.display()
            ),
        ),
    }
}

fn open_completed_folder(directory: &Path, requested: bool) -> FolderOpenOutcome {
    if !requested {
        return FolderOpenOutcome::NotRequested;
    }
    match open::that_detached(directory) {
        Ok(()) => FolderOpenOutcome::Opened,
        Err(error) => FolderOpenOutcome::Failed(error.to_string()),
    }
}

impl MediaSiftApp {
    pub(crate) fn choose_sort_folder(&mut self) {
        if let Some(directory) = FileDialog::new()
            .set_title("Choose the folder whose images you want to organize")
            .pick_folder()
        {
            self.pending_sort = Some(directory);
        }
    }

    pub(crate) fn run_sort(&mut self, directory: PathBuf) {
        let open_when_finished = self.open_sort_folder_when_finished;
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::Sort);
        self.pending_sort = None;
        self.set_notice(
            NoticeKind::Info,
            format!("Organizing images below {}...", directory.display()),
        );
        std::thread::spawn(move || {
            let result = sort_images(&directory).map_err(|error| error.to_string());
            let _ = sender.send(WorkResult::Sorted {
                directory,
                open_when_finished,
                result,
            });
        });
    }

    pub(crate) fn choose_date_prefix_folder(&mut self) {
        if let Some(directory) = FileDialog::new()
            .set_title("Choose the folder whose media files you want to rename")
            .pick_folder()
        {
            self.pending_date_prefix = Some(directory);
        }
    }

    pub(crate) fn run_date_prefix(&mut self, directory: PathBuf) {
        let use_oldest_date = self.use_oldest_date;
        let open_when_finished = self.open_prefix_folder_when_finished;
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::Prefix);
        self.pending_date_prefix = None;
        self.set_notice(
            NoticeKind::Info,
            format!("Adding date prefixes below {}...", directory.display()),
        );
        std::thread::spawn(move || {
            let result = prefix_media_files_with_date(&directory, use_oldest_date);
            let _ = sender.send(WorkResult::Prefixed {
                directory,
                open_when_finished,
                result,
            });
        });
    }

    pub(crate) fn start_photo_enhancement(&mut self) {
        let Some(photo) = FileDialog::new()
            .set_title("Choose a photo to enhance")
            .add_filter("Supported images", &["png", "bmp", "tif", "tiff"])
            .pick_file()
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::Enhance);
        self.set_notice(
            NoticeKind::Info,
            format!("Creating an enhanced copy of {}...", photo.display()),
        );
        std::thread::spawn(move || {
            let _ = sender.send(WorkResult::Enhanced(enhance_photo(&photo)));
        });
    }

    pub(crate) fn copy_restoration_prompt(&mut self, context: &egui::Context) {
        if let Some(photo) = FileDialog::new()
            .set_title("Choose a photo for the restoration prompt")
            .add_filter(
                "Images",
                &["jpg", "jpeg", "png", "bmp", "webp", "tif", "tiff"],
            )
            .pick_file()
        {
            context.copy_text(restoration_prompt(&photo));
            self.set_notice(
                NoticeKind::Success,
                format!(
                    "Copied an identity-preserving restoration prompt for {}.",
                    photo.display()
                ),
            );
        }
    }

    pub(crate) fn add_folder(&mut self) {
        if let Some(folder) = FileDialog::new()
            .set_title("Add a folder to this duplicate scan")
            .pick_folder()
        {
            if self.extra_roots.contains(&folder) {
                self.set_notice(
                    NoticeKind::Info,
                    format!("{} is already in the scan scope.", folder.display()),
                );
            } else {
                self.extra_roots.push(folder);
            }
        }
    }

    pub(crate) fn start_media_scan(&mut self) {
        let mut roots = computer_scan_roots(self.include_removable, self.include_network);
        roots.extend(self.extra_roots.iter().cloned());
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            self.set_notice(
                NoticeKind::Error,
                "No scan locations are available. Add a folder and try again.",
            );
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::Scan);
        self.set_notice(
            NoticeKind::Info,
            format!(
                "Scanning {} location(s). The scan is read-only and safe to leave running.",
                roots.len()
            ),
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
            .map_err(|error| error.to_string());
            let _ = sender.send(WorkResult::Scanned(result));
        });
    }

    pub(crate) fn selected_unkept(&self) -> Result<Vec<PathBuf>, String> {
        let review = self.review.as_ref().ok_or("Run a duplicate scan first.")?;
        unkept_duplicate_paths(&review.groups, &review.kept)
    }

    pub(crate) fn begin_action(&mut self, action: PendingAction) {
        match self.selected_unkept() {
            Ok(paths) if paths.is_empty() => self.set_notice(
                NoticeKind::Info,
                "No files are selected for action. Uncheck Keep on at least one extra copy.",
            ),
            Ok(_) => self.pending_action = Some(action),
            Err(error) => self.set_notice(NoticeKind::Error, error),
        }
    }

    pub(crate) fn run_action(&mut self, action: PendingAction) {
        let Ok(paths) = self.selected_unkept() else {
            return;
        };
        let archive = if action == PendingAction::Archive {
            FileDialog::new()
                .set_title("Choose where to save the duplicate backup")
                .add_filter("ZIP archive", &["zip"])
                .set_file_name("media-sift-duplicate-backup.zip")
                .save_file()
        } else {
            None
        };
        if action == PendingAction::Archive && archive.is_none() {
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
        self.operation = Some(Operation::FileAction);
        self.pending_action = None;
        self.set_notice(
            NoticeKind::Info,
            format!("Processing {} selected file(s)...", paths.len()),
        );
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
            let _ = sender.send(WorkResult::Action(action, result));
        });
    }

    pub(crate) fn collect_result(&mut self) {
        let results: Vec<_> = self
            .receiver
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        let mut finished = false;
        for result in results {
            match result {
                WorkResult::Progress(progress) => self.scan_progress = Some(progress),
                WorkResult::Sorted {
                    directory,
                    open_when_finished,
                    result: Ok(moved),
                } => {
                    finished = true;
                    self.notice = organize_completion_notice(
                        NoticeKind::Success,
                        format!(
                            "Organized {moved} image(s) into Year/Month folders. Images without a usable capture date were left in place."
                        ),
                        &directory,
                        open_completed_folder(&directory, open_when_finished),
                    );
                }
                WorkResult::Prefixed {
                    directory,
                    open_when_finished,
                    result: Ok(summary),
                } => {
                    finished = true;
                    let kind = if summary.failures.is_empty() {
                        NoticeKind::Success
                    } else {
                        NoticeKind::Warning
                    };
                    self.notice = organize_completion_notice(
                        kind,
                        format!(
                            "Renamed {} media file(s), skipped {}, and encountered {} failure(s).",
                            summary.renamed,
                            summary.skipped,
                            summary.failures.len()
                        ),
                        &directory,
                        open_completed_folder(&directory, open_when_finished),
                    );
                }
                WorkResult::Enhanced(Ok(output)) => {
                    finished = true;
                    self.set_notice(
                        NoticeKind::Success,
                        format!(
                            "Saved an enhanced copy to {}. The source photo was not changed.",
                            output.display()
                        ),
                    );
                }
                WorkResult::Scanned(Ok((groups, roots))) => {
                    finished = true;
                    let mut review = Review {
                        groups,
                        kept: HashSet::new(),
                        roots,
                    };
                    review.keep_all();
                    self.scan_report = format_scan_report(
                        &review.roots,
                        self.scan_progress
                            .as_ref()
                            .unwrap_or(&ScanProgress::default()),
                        &review.groups,
                    );
                    let group_count = review.groups.len();
                    let extra_copies = review.extra_copies();
                    self.review = Some(review);
                    self.set_notice(
                        NoticeKind::Success,
                        if group_count == 0 {
                            "Scan complete. No exact duplicate media files were found.".to_owned()
                        } else {
                            format!(
                                "Scan complete: {group_count} duplicate group(s) with {extra_copies} extra copy/copies. Nothing is selected for removal yet."
                            )
                        },
                    );
                }
                WorkResult::Action(action, Ok(summary)) => {
                    finished = true;
                    let kind = if summary.failures.is_empty() {
                        NoticeKind::Success
                    } else {
                        NoticeKind::Warning
                    };
                    let follow_up = match action {
                        PendingAction::Archive => "The originals remain in place.",
                        PendingAction::Recycle | PendingAction::Delete => {
                            self.review = None;
                            "The previous review was cleared because the files changed. Run a new scan to refresh it."
                        }
                    };
                    self.set_notice(
                        kind,
                        format!(
                            "Processed {} file(s) with {} failure(s). {follow_up}",
                            summary.processed,
                            summary.failures.len()
                        ),
                    );
                }
                WorkResult::Sorted {
                    result: Err(error), ..
                }
                | WorkResult::Prefixed {
                    result: Err(error), ..
                }
                | WorkResult::Enhanced(Err(error))
                | WorkResult::Scanned(Err(error))
                | WorkResult::Action(_, Err(error)) => {
                    finished = true;
                    self.set_notice(
                        NoticeKind::Error,
                        format!("{error} Try again or choose a different file or folder."),
                    );
                }
            }
        }
        if finished {
            self.receiver = None;
            self.operation = None;
        }
    }

    pub(crate) fn select_source(&mut self, root: &Path, only: bool) {
        if let Some(review) = &mut self.review {
            if only {
                review.kept.clear();
            }
            for path in review.groups.values().flatten() {
                if path.starts_with(root) {
                    review.kept.insert(path.clone());
                }
            }
        }
    }

    pub(crate) fn save_scan_report(&mut self) {
        if self.scan_report.is_empty() {
            return;
        }
        if let Some(path) = FileDialog::new()
            .set_title("Save the duplicate scan report")
            .add_filter("Text file", &["txt"])
            .set_file_name("media-scan-report.txt")
            .save_file()
        {
            match std::fs::write(&path, &self.scan_report) {
                Ok(()) => self.set_notice(
                    NoticeKind::Success,
                    format!("Saved the scan report to {}.", path.display()),
                ),
                Err(error) => self.set_notice(
                    NoticeKind::Error,
                    format!("Could not save the report: {error}"),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_completion_keeps_success_when_folder_opening_was_not_requested() {
        let notice = organize_completion_notice(
            NoticeKind::Success,
            "Organized files.".to_owned(),
            Path::new("C:/Media"),
            FolderOpenOutcome::NotRequested,
        );
        assert_eq!(notice.kind, NoticeKind::Success);
        assert_eq!(notice.text, "Organized files.");
    }

    #[test]
    fn folder_open_failure_is_a_warning_without_hiding_operation_success() {
        let notice = organize_completion_notice(
            NoticeKind::Success,
            "Organized files.".to_owned(),
            Path::new("C:/Media"),
            FolderOpenOutcome::Failed("launcher unavailable".to_owned()),
        );
        assert_eq!(notice.kind, NoticeKind::Warning);
        assert!(notice.text.contains("The file operation completed"));
        assert!(notice.text.contains("C:/Media"));
        assert!(notice.text.contains("launcher unavailable"));
    }
}
