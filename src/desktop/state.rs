//! Desktop application state and pure selection rules.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
};

use crate::{DuplicateGroups, FileActionSummary, RenameSummary, ScanProgress};

pub(crate) enum WorkResult {
    Sorted {
        directory: PathBuf,
        open_when_finished: bool,
        result: Result<usize, String>,
    },
    Prefixed {
        directory: PathBuf,
        open_when_finished: bool,
        result: Result<RenameSummary, String>,
    },
    Enhanced(Result<PathBuf, String>),
    Scanned(Result<(DuplicateGroups, Vec<PathBuf>), String>),
    ScanCancelled,
    Action(PendingAction, Result<FileActionSummary, String>),
    Progress(ScanProgress),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingAction {
    Recycle,
    Delete,
    Archive,
}

impl PendingAction {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Recycle => "Move files to the Recycle Bin?",
            Self::Delete => "Permanently delete files?",
            Self::Archive => "Create a ZIP backup?",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Overview,
    Duplicates,
    Organize,
    PhotoTools,
}

impl Page {
    pub(crate) const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Overview, "Overview", "Alt+1"),
        (Self::Duplicates, "Duplicate finder", "Alt+2"),
        (Self::Organize, "Organize", "Alt+3"),
        (Self::PhotoTools, "Photo tools", "Alt+4"),
    ];

    pub(crate) fn heading(self) -> &'static str {
        match self {
            Self::Overview => "Your media workspace",
            Self::Duplicates => "Find exact duplicates",
            Self::Organize => "Organize files",
            Self::PhotoTools => "Photo tools",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Scan,
    Sort,
    Prefix,
    Enhance,
    FileAction,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScanState {
    #[default]
    Idle,
    Running,
    Cancelling,
    Cancelled,
    Complete,
    Failed,
}

impl ScanState {
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}

impl Operation {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Scan => "Scanning for exact duplicates",
            Self::Sort => "Organizing images",
            Self::Prefix => "Renaming media files",
            Self::Enhance => "Creating an enhanced copy",
            Self::FileAction => "Processing duplicate files",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

pub(crate) struct Notice {
    pub(crate) kind: NoticeKind,
    pub(crate) text: String,
}

impl Notice {
    pub(crate) fn new(kind: NoticeKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self.kind {
            NoticeKind::Info => "Ready",
            NoticeKind::Success => "Completed",
            NoticeKind::Warning => "Needs attention",
            NoticeKind::Error => "Could not complete",
        }
    }
}

#[derive(Default)]
pub(crate) struct Review {
    pub(crate) groups: DuplicateGroups,
    pub(crate) kept: HashSet<PathBuf>,
    pub(crate) roots: Vec<PathBuf>,
}

impl Review {
    pub(crate) fn total_files(&self) -> usize {
        self.groups.values().map(Vec::len).sum()
    }

    pub(crate) fn extra_copies(&self) -> usize {
        self.groups
            .values()
            .map(|paths| paths.len().saturating_sub(1))
            .sum()
    }

    pub(crate) fn action_count(&self) -> usize {
        self.groups
            .values()
            .flatten()
            .filter(|path| !self.kept.contains(*path))
            .count()
    }

    pub(crate) fn keep_all(&mut self) {
        self.kept = self.groups.values().flatten().cloned().collect();
    }

    pub(crate) fn keep_first_in_each_group(&mut self) {
        self.kept = self
            .groups
            .values()
            .filter_map(|paths| paths.first().cloned())
            .collect();
    }

    pub(crate) fn every_group_has_keeper(&self) -> bool {
        self.groups
            .values()
            .all(|paths| paths.iter().any(|path| self.kept.contains(path)))
    }
}

pub(crate) struct MediaSiftApp {
    pub(crate) page: Page,
    pub(crate) notice: Notice,
    pub(crate) receiver: Option<Receiver<WorkResult>>,
    pub(crate) operation: Option<Operation>,
    pub(crate) review: Option<Review>,
    pub(crate) include_removable: bool,
    pub(crate) include_network: bool,
    pub(crate) selected_scan_roots: Vec<PathBuf>,
    pub(crate) scan_state: ScanState,
    pub(crate) scan_cancel_token: Option<Arc<AtomicBool>>,
    pub(crate) compression: usize,
    pub(crate) pending_action: Option<PendingAction>,
    pub(crate) pending_sort: Option<PathBuf>,
    pub(crate) pending_date_prefix: Option<PathBuf>,
    pub(crate) use_oldest_date: bool,
    pub(crate) open_sort_folder_when_finished: bool,
    pub(crate) open_prefix_folder_when_finished: bool,
    pub(crate) scan_progress: Option<ScanProgress>,
    pub(crate) scan_report: String,
}

impl MediaSiftApp {
    pub(crate) fn initial() -> Self {
        Self {
            page: Page::Overview,
            notice: Notice::new(
                NoticeKind::Info,
                "Choose a workflow. MediaSift will explain any file changes before they happen.",
            ),
            receiver: None,
            operation: None,
            review: None,
            include_removable: false,
            include_network: false,
            selected_scan_roots: Vec::new(),
            scan_state: ScanState::Idle,
            scan_cancel_token: None,
            compression: 2,
            pending_action: None,
            pending_sort: None,
            pending_date_prefix: None,
            use_oldest_date: false,
            open_sort_folder_when_finished: false,
            open_prefix_folder_when_finished: false,
            scan_progress: None,
            scan_report: String::new(),
        }
    }

    pub(crate) fn is_working(&self) -> bool {
        self.receiver.is_some()
    }

    pub(crate) fn set_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = Notice::new(kind, text);
    }
}

impl Drop for MediaSiftApp {
    fn drop(&mut self) {
        if let Some(token) = &self.scan_cancel_token {
            token.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_review() -> Review {
        let first = PathBuf::from("A/one.jpg");
        let second = PathBuf::from("B/one.jpg");
        let third = PathBuf::from("C/two.mp4");
        let fourth = PathBuf::from("D/two.mp4");
        let fifth = PathBuf::from("E/two.mp4");
        Review {
            groups: DuplicateGroups::from([
                ("first".to_owned(), vec![first, second]),
                ("second".to_owned(), vec![third, fourth, fifth]),
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn review_metrics_distinguish_files_from_extra_copies() {
        let review = sample_review();
        assert_eq!(review.groups.len(), 2);
        assert_eq!(review.total_files(), 5);
        assert_eq!(review.extra_copies(), 3);
    }

    #[test]
    fn keep_first_selects_every_extra_copy_for_action() {
        let mut review = sample_review();
        review.keep_first_in_each_group();
        assert!(review.every_group_has_keeper());
        assert_eq!(review.kept.len(), 2);
        assert_eq!(review.action_count(), 3);
    }

    #[test]
    fn keep_all_is_the_safe_default() {
        let mut review = sample_review();
        review.keep_all();
        assert!(review.every_group_has_keeper());
        assert_eq!(review.action_count(), 0);
    }

    #[test]
    fn organizer_folder_opening_is_opt_in() {
        let app = MediaSiftApp::initial();
        assert!(!app.open_sort_folder_when_finished);
        assert!(!app.open_prefix_folder_when_finished);
    }

    #[test]
    fn duplicate_scan_starts_idle_without_selected_folders() {
        let app = MediaSiftApp::initial();
        assert_eq!(app.scan_state, ScanState::Idle);
        assert!(app.selected_scan_roots.is_empty());
        assert!(app.scan_cancel_token.is_none());
    }
}
