//! Background operations and file-dialog orchestration.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use eframe::egui;
use rfd::FileDialog;

use crate::{
    ArchiveCompression, ScanProgress, archive_files, computer_scan_roots, enhance_photo,
    format_scan_report, permanently_delete_files, prefix_media_files_with_date, recycle_files,
    restoration_prompt,
    scan_cache::{
        CachedScanOutcome, ScanMode, find_exact_duplicates_with_cache_and_cancel,
        forget_cached_scan, load_cached_scan,
    },
    sort_images, verify_duplicate_action_groups,
};

use super::state::{
    MediaSiftApp, Notice, NoticeKind, Operation, PendingAction, Review, ScanState, WorkResult,
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

fn resolved_scan_roots(
    selected_roots: &[PathBuf],
    discovered_drive_roots: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut roots = if selected_roots.is_empty() {
        discovered_drive_roots
    } else {
        selected_roots.to_vec()
    };
    roots.sort();
    roots.dedup();
    let mut non_overlapping_roots = Vec::new();
    for root in roots {
        if !non_overlapping_roots
            .iter()
            .any(|selected: &PathBuf| root.starts_with(selected))
        {
            non_overlapping_roots.push(root);
        }
    }
    non_overlapping_roots
}

impl MediaSiftApp {
    pub(crate) fn load_saved_scan(&mut self) {
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::CacheLoad);
        self.set_notice(
            NoticeKind::Info,
            "Loading the last saved duplicate scan from Local AppData...",
        );
        std::thread::spawn(move || {
            let result = load_cached_scan().map_err(|error| error.to_string());
            let _ = sender.send(WorkResult::CachedScanLoaded(result));
        });
    }

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
            let cache_error = forget_cached_scan().err().map(|error| error.to_string());
            let _ = sender.send(WorkResult::Sorted {
                directory,
                open_when_finished,
                result,
                cache_error,
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
            let cache_error = forget_cached_scan().err().map(|error| error.to_string());
            let _ = sender.send(WorkResult::Prefixed {
                directory,
                open_when_finished,
                result,
                cache_error,
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
            let result = enhance_photo(&photo);
            let cache_error = forget_cached_scan().err().map(|error| error.to_string());
            let _ = sender.send(WorkResult::Enhanced {
                result,
                cache_error,
            });
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

    pub(crate) fn add_scan_folders(&mut self) {
        if let Some(folders) = FileDialog::new()
            .set_title("Select one or more folders to scan")
            .pick_folders()
        {
            self.selected_scan_roots.extend(folders);
            self.selected_scan_roots.sort();
            self.selected_scan_roots.dedup();
            self.set_notice(
                NoticeKind::Info,
                format!(
                    "Selected {} folder(s). Only these folders will be scanned; automatic whole-drive scanning is off.",
                    self.selected_scan_roots.len()
                ),
            );
        }
    }

    pub(crate) fn start_media_scan(&mut self) {
        self.start_media_scan_with_mode(ScanMode::Incremental);
    }

    pub(crate) fn start_full_media_scan(&mut self) {
        self.start_media_scan_with_mode(ScanMode::Full);
    }

    pub(crate) fn refresh_saved_scan(&mut self, mode: ScanMode) {
        let Some(review) = &self.review else {
            self.set_notice(NoticeKind::Error, "There is no saved scan to refresh.");
            return;
        };
        self.start_media_scan_for_roots(review.roots().to_vec(), mode);
    }

    fn start_media_scan_with_mode(&mut self, mode: ScanMode) {
        let roots = resolved_scan_roots(
            &self.selected_scan_roots,
            computer_scan_roots(self.include_removable, self.include_network),
        );
        self.start_media_scan_for_roots(roots, mode);
    }

    fn start_media_scan_for_roots(&mut self, roots: Vec<PathBuf>, mode: ScanMode) {
        if self.saved_scan_error {
            self.set_notice(
                NoticeKind::Error,
                "Delete the unreadable saved-scan cache before starting a new scan.",
            );
            return;
        }
        if roots.is_empty() {
            self.set_notice(
                NoticeKind::Error,
                "No scan locations are available. Add a folder and try again.",
            );
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let cancel_token = Arc::new(AtomicBool::new(false));
        self.receiver = Some(receiver);
        self.operation = Some(Operation::Scan);
        self.scan_state = ScanState::Running;
        self.scan_mode = mode;
        self.review_verified_this_session = false;
        self.scan_cancel_token = Some(Arc::clone(&cancel_token));
        self.set_notice(
            NoticeKind::Info,
            format!(
                "{} {} location(s). The scan is read-only and safe to leave running.",
                if mode == ScanMode::Incremental {
                    "Refreshing"
                } else {
                    "Fully rescanning"
                },
                roots.len(),
            ),
        );
        self.scan_progress = Some(ScanProgress::default());
        if self.review.is_none() {
            self.scan_report.clear();
        }
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let result = find_exact_duplicates_with_cache_and_cancel(
                &roots,
                mode,
                move |progress| {
                    let _ = progress_sender.send(WorkResult::Progress(progress));
                },
                || cancel_token.load(Ordering::Relaxed),
            );
            match result {
                Ok(CachedScanOutcome::Completed(scan)) => {
                    let _ = sender.send(WorkResult::Scanned(Ok(scan)));
                }
                Ok(CachedScanOutcome::Cancelled) => {
                    let _ = sender.send(WorkResult::ScanCancelled);
                }
                Err(error) => {
                    let _ = sender.send(WorkResult::Scanned(Err(error.to_string())));
                }
            }
        });
    }

    pub(crate) fn forget_saved_scan(&mut self) {
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::CacheForget);
        self.pending_forget_scan_cache = false;
        self.set_notice(
            NoticeKind::Info,
            "Deleting saved scan metadata from Local AppData. Media files will not be touched...",
        );
        std::thread::spawn(move || {
            let result = forget_cached_scan().map_err(|error| error.to_string());
            let _ = sender.send(WorkResult::CachedScanForgotten(result));
        });
    }

    pub(crate) fn cancel_media_scan(&mut self) {
        if self.scan_state != ScanState::Running {
            return;
        }
        if let Some(token) = &self.scan_cancel_token {
            token.store(true, Ordering::Relaxed);
            self.scan_state = ScanState::Cancelling;
            self.set_notice(
                NoticeKind::Info,
                "Cancelling the duplicate scan. MediaSift will stop after the current filesystem step.",
            );
        }
    }

    pub(crate) fn selected_action_paths(&self) -> Result<Vec<PathBuf>, String> {
        let review = self.review.as_ref().ok_or("Run a duplicate scan first.")?;
        review.selected_paths()
    }

    pub(crate) fn begin_action(&mut self, action: PendingAction) {
        match self.selected_action_paths() {
            Ok(paths) if paths.is_empty() => self.set_notice(
                NoticeKind::Info,
                "No files are selected for action. Uncheck Keep on at least one extra copy.",
            ),
            Ok(_) => self.pending_action = Some(action),
            Err(error) => self.set_notice(NoticeKind::Error, error),
        }
    }

    pub(crate) fn run_action(&mut self, action: PendingAction) {
        let Ok(paths) = self.selected_action_paths() else {
            return;
        };
        let verifications = if matches!(action, PendingAction::Recycle | PendingAction::Delete) {
            let Some(review) = self.review.as_ref() else {
                return;
            };
            match review.action_verifications() {
                Ok(verifications) => verifications,
                Err(error) => {
                    self.set_notice(NoticeKind::Error, error);
                    return;
                }
            }
        } else {
            Vec::new()
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
            if verifications.is_empty() {
                format!("Processing {} selected file(s)...", paths.len())
            } else {
                format!(
                    "Re-verifying exact contents, then processing {} selected file(s)...",
                    paths.len()
                )
            },
        );
        std::thread::spawn(move || {
            let result = match action {
                PendingAction::Recycle => verify_duplicate_action_groups(&verifications)
                    .map(|()| recycle_files(&paths))
                    .map_err(|error| error.to_string()),
                PendingAction::Delete => verify_duplicate_action_groups(&verifications)
                    .map(|()| permanently_delete_files(&paths))
                    .map_err(|error| error.to_string()),
                PendingAction::Archive => archive_files(
                    &paths,
                    &archive.expect("archive path selected"),
                    compression,
                ),
            };
            let cache_error = matches!(action, PendingAction::Recycle | PendingAction::Delete)
                .then(|| forget_cached_scan().err().map(|error| error.to_string()))
                .flatten();
            let _ = sender.send(WorkResult::Action {
                action,
                result,
                cache_error,
            });
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
                    cache_error,
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
                    self.finish_cache_invalidation(cache_error);
                }
                WorkResult::Prefixed {
                    directory,
                    open_when_finished,
                    result: Ok(summary),
                    cache_error,
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
                    self.finish_cache_invalidation(cache_error);
                }
                WorkResult::Enhanced {
                    result: Ok(output),
                    cache_error,
                } => {
                    finished = true;
                    self.set_notice(
                        NoticeKind::Success,
                        format!(
                            "Saved an enhanced copy to {}. The source photo was not changed.",
                            output.display()
                        ),
                    );
                    self.finish_cache_invalidation(cache_error);
                }
                WorkResult::Scanned(Ok(scan)) => {
                    finished = true;
                    self.scan_state = ScanState::Complete;
                    let review = Review::new(scan.groups, scan.roots);
                    self.scan_progress = Some(scan.progress);
                    self.scan_report = format_scan_report(
                        review.roots(),
                        self.scan_progress
                            .as_ref()
                            .unwrap_or(&ScanProgress::default()),
                        review.groups(),
                    );
                    let group_count = review.group_count();
                    let extra_copies = review.extra_copies();
                    self.review = Some(review);
                    self.review_page = 0;
                    self.saved_scan_at = Some(scan.completed_at_unix_seconds);
                    self.review_verified_this_session = true;
                    self.saved_scan_error = false;
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
                WorkResult::ScanCancelled => {
                    finished = true;
                    self.scan_state = ScanState::Cancelled;
                    if self.review.is_none() {
                        self.scan_report.clear();
                    }
                    self.set_notice(
                        NoticeKind::Info,
                        if self.review.is_some() {
                            "Scan cancelled. The previous completed results and saved metadata are still available."
                        } else {
                            "Scan cancelled. Scanning never changes files; adjust the locations and start again when ready."
                        },
                    );
                }
                WorkResult::CachedScanLoaded(Ok(Some(scan))) => {
                    finished = true;
                    self.scan_state = ScanState::Complete;
                    let review = Review::new(scan.groups, scan.roots);
                    self.scan_report =
                        format_scan_report(review.roots(), &scan.progress, review.groups());
                    let group_count = review.group_count();
                    self.review = Some(review);
                    self.review_page = 0;
                    self.scan_progress = Some(scan.progress);
                    self.saved_scan_at = Some(scan.completed_at_unix_seconds);
                    self.review_verified_this_session = false;
                    self.saved_scan_error = false;
                    self.set_notice(
                        NoticeKind::Info,
                        format!(
                            "Opened the last saved scan with {group_count} duplicate group(s). Refresh changes before acting on files."
                        ),
                    );
                }
                WorkResult::CachedScanLoaded(Ok(None)) => {
                    finished = true;
                    self.scan_state = ScanState::Idle;
                    self.saved_scan_error = false;
                    self.set_notice(
                        NoticeKind::Info,
                        "Choose a workflow. MediaSift will explain any file changes before they happen.",
                    );
                }
                WorkResult::CachedScanLoaded(Err(error)) => {
                    finished = true;
                    self.scan_state = ScanState::Idle;
                    self.saved_scan_error = true;
                    self.set_notice(
                        NoticeKind::Warning,
                        format!(
                            "The saved scan could not be opened: {error} Delete the unreadable cache from Duplicate finder, then start a new scan."
                        ),
                    );
                }
                WorkResult::CachedScanForgotten(Ok(())) => {
                    finished = true;
                    self.saved_scan_at = None;
                    self.review_verified_this_session = false;
                    self.saved_scan_error = false;
                    self.set_notice(
                        NoticeKind::Success,
                        "Saved scan metadata was deleted from Local AppData. Media files were not touched.",
                    );
                }
                WorkResult::CachedScanForgotten(Err(error)) => {
                    finished = true;
                    self.set_notice(
                        NoticeKind::Error,
                        format!("Could not delete the saved scan metadata: {error}"),
                    );
                }
                WorkResult::Action {
                    action,
                    result: Ok(summary),
                    cache_error,
                } => {
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
                            self.review_verified_this_session = false;
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
                    if action != PendingAction::Archive {
                        self.finish_cache_invalidation(cache_error);
                    }
                }
                WorkResult::Sorted {
                    result: Err(error),
                    cache_error,
                    ..
                }
                | WorkResult::Prefixed {
                    result: Err(error),
                    cache_error,
                    ..
                }
                | WorkResult::Enhanced {
                    result: Err(error),
                    cache_error,
                } => {
                    finished = true;
                    self.set_notice(
                        NoticeKind::Error,
                        format!("{error} Try again or choose a different file or folder."),
                    );
                    self.finish_cache_invalidation(cache_error);
                }
                WorkResult::Action {
                    action,
                    result: Err(error),
                    cache_error,
                } => {
                    finished = true;
                    self.set_notice(
                        NoticeKind::Error,
                        format!("{error} Try again or choose a different file or folder."),
                    );
                    if action != PendingAction::Archive {
                        self.finish_cache_invalidation(cache_error);
                    }
                }
                WorkResult::Scanned(Err(error)) => {
                    finished = true;
                    self.scan_state = ScanState::Failed;
                    self.set_notice(
                        NoticeKind::Error,
                        format!("{error} Try again or choose different scan locations."),
                    );
                }
            }
        }
        if finished {
            self.receiver = None;
            self.operation = None;
            self.scan_cancel_token = None;
        }
    }

    pub(crate) fn select_source(&mut self, root: &Path, only: bool) {
        if let Some(review) = &mut self.review {
            review.keep_from_root(root, only);
        }
    }

    fn finish_cache_invalidation(&mut self, cache_error: Option<String>) {
        self.review = None;
        self.review_verified_this_session = false;
        self.saved_scan_at = None;
        if let Some(error) = cache_error {
            self.notice.kind = NoticeKind::Warning;
            self.notice.text.push_str(&format!(
                " Saved scan metadata could not be deleted: {error} Refresh before relying on it."
            ));
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

    #[test]
    fn selected_scan_folders_replace_automatic_drive_roots() {
        let selected = vec![
            PathBuf::from("D:/Photos"),
            PathBuf::from("D:/Photos"),
            PathBuf::from("D:/Photos/Trips"),
        ];
        let discovered = vec![PathBuf::from("C:/"), PathBuf::from("D:/")];

        assert_eq!(
            resolved_scan_roots(&selected, discovered),
            vec![PathBuf::from("D:/Photos")]
        );
    }

    #[test]
    fn automatic_drive_roots_are_used_only_without_selected_folders() {
        let discovered = vec![PathBuf::from("D:/"), PathBuf::from("C:/")];

        assert_eq!(
            resolved_scan_roots(&[], discovered),
            vec![PathBuf::from("C:/"), PathBuf::from("D:/")]
        );
    }

    #[test]
    fn cancelling_a_running_scan_sets_the_shared_token() {
        let mut app = MediaSiftApp::initial();
        let token = Arc::new(AtomicBool::new(false));
        app.scan_cancel_token = Some(Arc::clone(&token));
        app.scan_state = ScanState::Running;

        app.cancel_media_scan();

        assert!(token.load(Ordering::Relaxed));
        assert_eq!(app.scan_state, ScanState::Cancelling);
        assert!(app.notice.text.contains("Cancelling"));
    }

    #[test]
    fn cancelled_scan_result_clears_background_state_and_partial_report() {
        let mut app = MediaSiftApp::initial();
        let (sender, receiver) = mpsc::channel();
        app.receiver = Some(receiver);
        app.operation = Some(Operation::Scan);
        app.scan_state = ScanState::Cancelling;
        app.scan_cancel_token = Some(Arc::new(AtomicBool::new(true)));
        app.scan_report = "partial report".to_owned();
        sender
            .send(WorkResult::ScanCancelled)
            .expect("send cancellation result");

        app.collect_result();

        assert_eq!(app.scan_state, ScanState::Cancelled);
        assert!(app.receiver.is_none());
        assert!(app.operation.is_none());
        assert!(app.scan_cancel_token.is_none());
        assert!(app.scan_report.is_empty());
        assert!(app.notice.text.contains("Scan cancelled"));
    }
}
