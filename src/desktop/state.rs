//! Desktop application state and pure selection rules.

use std::{
    collections::HashSet,
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
};

use crate::{
    DuplicateGroups, FileActionSummary, RenameSummary, ScanProgress,
    scan_cache::{CachedScan, ScanMode},
    selected_duplicate_paths,
};

pub(crate) enum WorkResult {
    Sorted {
        directory: PathBuf,
        open_when_finished: bool,
        result: Result<usize, String>,
        cache_error: Option<String>,
    },
    Prefixed {
        directory: PathBuf,
        open_when_finished: bool,
        result: Result<RenameSummary, String>,
        cache_error: Option<String>,
    },
    Enhanced {
        result: Result<PathBuf, String>,
        cache_error: Option<String>,
    },
    Scanned(Result<CachedScan, String>),
    ScanCancelled,
    CachedScanLoaded(Result<Option<CachedScan>, String>),
    CachedScanForgotten(Result<(), String>),
    Action {
        action: PendingAction,
        result: Result<FileActionSummary, String>,
        cache_error: Option<String>,
    },
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
    CacheLoad,
    CacheForget,
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
            Self::CacheLoad => "Loading saved scan",
            Self::CacheForget => "Forgetting saved scan",
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
    groups: DuplicateGroups,
    roots: Vec<PathBuf>,
    selected_for_action: HashSet<PathBuf>,
    total_files: usize,
    extra_copies: usize,
    groups_without_keeper: usize,
}

impl Review {
    pub(crate) fn new(groups: DuplicateGroups, roots: Vec<PathBuf>) -> Self {
        let total_files = groups.values().map(Vec::len).sum();
        let extra_copies = groups
            .values()
            .map(|paths| paths.len().saturating_sub(1))
            .sum();
        Self {
            groups,
            roots,
            selected_for_action: HashSet::new(),
            total_files,
            extra_copies,
            groups_without_keeper: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn sample_for_tests() -> Self {
        let first = PathBuf::from("A/one.jpg");
        let second = PathBuf::from("B/one.jpg");
        let third = PathBuf::from("C/two.mp4");
        let fourth = PathBuf::from("D/two.mp4");
        let fifth = PathBuf::from("E/two.mp4");
        Self::new(
            DuplicateGroups::from([
                ("first".to_owned(), vec![first, second]),
                ("second".to_owned(), vec![third, fourth, fifth]),
            ]),
            Vec::new(),
        )
    }

    pub(crate) fn total_files(&self) -> usize {
        self.total_files
    }

    pub(crate) fn groups(&self) -> &DuplicateGroups {
        &self.groups
    }

    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(crate) fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub(crate) fn extra_copies(&self) -> usize {
        self.extra_copies
    }

    pub(crate) fn action_count(&self) -> usize {
        self.selected_for_action.len()
    }

    pub(crate) fn keep_all(&mut self) {
        self.selected_for_action.clear();
        self.groups_without_keeper = 0;
    }

    pub(crate) fn keep_first_in_each_group(&mut self) {
        self.selected_for_action = self
            .groups
            .values()
            .flat_map(|paths| paths.iter().skip(1).cloned())
            .collect();
        self.groups_without_keeper = 0;
    }

    pub(crate) fn every_group_has_keeper(&self) -> bool {
        self.groups_without_keeper == 0
    }

    pub(crate) fn is_kept(&self, path: &PathBuf) -> bool {
        !self.selected_for_action.contains(path)
    }

    pub(crate) fn set_kept(&mut self, group: &str, path: &PathBuf, keep: bool) {
        let was_missing_keeper = self.group_has_no_keeper(group);
        if keep {
            self.selected_for_action.remove(path);
        } else {
            self.selected_for_action.insert(path.clone());
        }
        let is_missing_keeper = self.group_has_no_keeper(group);
        match (was_missing_keeper, is_missing_keeper) {
            (false, true) => self.groups_without_keeper += 1,
            (true, false) => self.groups_without_keeper -= 1,
            _ => {}
        }
    }

    pub(crate) fn keep_from_root(&mut self, root: &std::path::Path, only: bool) {
        if only {
            self.selected_for_action = self
                .groups
                .values()
                .flatten()
                .filter(|path| !path.starts_with(root))
                .cloned()
                .collect();
        } else {
            self.selected_for_action
                .retain(|path| !path.starts_with(root));
        }
        self.recount_groups_without_keeper();
    }

    pub(crate) fn selected_paths(&self) -> Result<Vec<PathBuf>, String> {
        selected_duplicate_paths(&self.groups, &self.selected_for_action)
    }

    pub(crate) fn page_count(&self, page_size: usize) -> usize {
        assert!(page_size > 0, "page size must be greater than zero");
        self.groups.len().div_ceil(page_size)
    }

    pub(crate) fn page_bounds(&self, page: usize, page_size: usize) -> Range<usize> {
        let page_count = self.page_count(page_size);
        if page_count == 0 {
            return 0..0;
        }
        let page = page.min(page_count - 1);
        let start = page * page_size;
        start..(start + page_size).min(self.groups.len())
    }

    fn group_has_no_keeper(&self, group: &str) -> bool {
        self.groups.get(group).is_some_and(|paths| {
            paths
                .iter()
                .all(|path| self.selected_for_action.contains(path))
        })
    }

    fn recount_groups_without_keeper(&mut self) {
        self.groups_without_keeper = self
            .groups
            .values()
            .filter(|paths| {
                paths
                    .iter()
                    .all(|path| self.selected_for_action.contains(path))
            })
            .count();
    }
}

pub(crate) struct MediaSiftApp {
    pub(crate) page: Page,
    pub(crate) notice: Notice,
    pub(crate) receiver: Option<Receiver<WorkResult>>,
    pub(crate) operation: Option<Operation>,
    pub(crate) review: Option<Review>,
    pub(crate) review_page: usize,
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
    pub(crate) scan_mode: ScanMode,
    pub(crate) saved_scan_at: Option<i64>,
    pub(crate) review_verified_this_session: bool,
    pub(crate) pending_forget_scan_cache: bool,
    pub(crate) saved_scan_error: bool,
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
            review_page: 0,
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
            scan_mode: ScanMode::Incremental,
            saved_scan_at: None,
            review_verified_this_session: false,
            pending_forget_scan_cache: false,
            saved_scan_error: false,
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

    #[test]
    fn review_metrics_distinguish_files_from_extra_copies() {
        let review = Review::sample_for_tests();
        assert_eq!(review.group_count(), 2);
        assert_eq!(review.total_files(), 5);
        assert_eq!(review.extra_copies(), 3);
    }

    #[test]
    fn keep_first_selects_every_extra_copy_for_action() {
        let mut review = Review::sample_for_tests();
        review.keep_first_in_each_group();
        assert!(review.every_group_has_keeper());
        assert_eq!(review.action_count(), 3);
    }

    #[test]
    fn keep_all_is_the_safe_default() {
        let mut review = Review::sample_for_tests();
        review.keep_all();
        assert!(review.every_group_has_keeper());
        assert_eq!(review.action_count(), 0);
    }

    #[test]
    fn toggling_the_last_keeper_blocks_actions_until_repaired() {
        let mut review = Review::sample_for_tests();
        let paths = review.groups()["first"].clone();

        review.set_kept("first", &paths[0], false);
        review.set_kept("first", &paths[1], false);
        assert!(!review.every_group_has_keeper());
        assert!(review.selected_paths().is_err());

        review.set_kept("first", &paths[0], true);
        assert!(review.every_group_has_keeper());
        assert_eq!(review.selected_paths().unwrap(), vec![paths[1].clone()]);
    }

    #[test]
    fn source_preference_keeps_matching_paths_without_dense_keep_state() {
        let mut review = Review::sample_for_tests();
        review.keep_first_in_each_group();

        review.keep_from_root(std::path::Path::new("B"), false);
        assert!(review.is_kept(&PathBuf::from("B/one.jpg")));
        assert_eq!(review.action_count(), 2);

        review.keep_from_root(std::path::Path::new("C"), true);
        assert!(review.is_kept(&PathBuf::from("C/two.mp4")));
        assert!(!review.is_kept(&PathBuf::from("A/one.jpg")));
        assert!(!review.every_group_has_keeper());
    }

    #[test]
    fn large_reviews_start_sparse_and_have_bounded_pages() {
        let groups = (0..55_298)
            .map(|index| {
                (
                    format!("group-{index:05}"),
                    vec![
                        PathBuf::from(format!("A/{index}.jpg")),
                        PathBuf::from(format!("B/{index}.jpg")),
                    ],
                )
            })
            .collect();
        let review = Review::new(groups, Vec::new());

        assert_eq!(review.total_files(), 110_596);
        assert_eq!(review.extra_copies(), 55_298);
        assert_eq!(review.action_count(), 0);
        assert!(review.selected_for_action.is_empty());
        assert_eq!(review.page_count(50), 1_106);
        assert_eq!(review.page_bounds(0, 50), 0..50);
        assert_eq!(review.page_bounds(1_105, 50), 55_250..55_298);
        assert_eq!(review.page_bounds(usize::MAX, 50), 55_250..55_298);
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
