#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

use eframe::egui::{
    self, Align, Align2, Color32, FontFamily, FontId, Frame, Key, Layout, Margin, RichText, Stroke,
    TextStyle, Vec2,
};
use media_sift::{
    ArchiveCompression, DuplicateGroups, FileActionSummary, RenameSummary, ScanProgress,
    archive_files, computer_scan_roots, enhance_photo, find_exact_duplicates_with_progress,
    format_scan_report, permanently_delete_files, prefix_media_files_with_date, recycle_files,
    restoration_prompt, sort_images, unkept_duplicate_paths,
};
use rfd::FileDialog;

const ACCENT: Color32 = Color32::from_rgb(37, 99, 235);
const ACCENT_HOVER: Color32 = Color32::from_rgb(29, 78, 216);
const DANGER: Color32 = Color32::from_rgb(180, 35, 24);
const CONTENT_MAX_WIDTH: f32 = 1120.0;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([720.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "MediaSift",
        options,
        Box::new(|creation_context| Ok(Box::new(MediaSiftApp::new(creation_context)))),
    )
}

enum WorkResult {
    Sorted(Result<usize, String>),
    Prefixed(Result<RenameSummary, String>),
    Enhanced(Result<PathBuf, String>),
    Scanned(Result<(DuplicateGroups, Vec<PathBuf>), String>),
    Action(PendingAction, Result<FileActionSummary, String>),
    Progress(ScanProgress),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingAction {
    Recycle,
    Delete,
    Archive,
}

impl PendingAction {
    fn title(self) -> &'static str {
        match self {
            Self::Recycle => "Move files to the Recycle Bin?",
            Self::Delete => "Permanently delete files?",
            Self::Archive => "Create a ZIP backup?",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Overview,
    Duplicates,
    Organize,
    PhotoTools,
}

impl Page {
    const ALL: [(Self, &'static str, &'static str); 4] = [
        (Self::Overview, "Overview", "Alt+1"),
        (Self::Duplicates, "Duplicate finder", "Alt+2"),
        (Self::Organize, "Organize", "Alt+3"),
        (Self::PhotoTools, "Photo tools", "Alt+4"),
    ];

    fn heading(self) -> &'static str {
        match self {
            Self::Overview => "Your media workspace",
            Self::Duplicates => "Find exact duplicates",
            Self::Organize => "Organize files",
            Self::PhotoTools => "Photo tools",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Scan,
    Sort,
    Prefix,
    Enhance,
    FileAction,
}

impl Operation {
    fn label(self) -> &'static str {
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
enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

struct Notice {
    kind: NoticeKind,
    text: String,
}

impl Notice {
    fn new(kind: NoticeKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }

    fn label(&self) -> &'static str {
        match self.kind {
            NoticeKind::Info => "Ready",
            NoticeKind::Success => "Completed",
            NoticeKind::Warning => "Needs attention",
            NoticeKind::Error => "Could not complete",
        }
    }
}

#[derive(Default)]
struct Review {
    groups: DuplicateGroups,
    kept: HashSet<PathBuf>,
    roots: Vec<PathBuf>,
}

impl Review {
    fn total_files(&self) -> usize {
        self.groups.values().map(Vec::len).sum()
    }

    fn extra_copies(&self) -> usize {
        self.groups
            .values()
            .map(|paths| paths.len().saturating_sub(1))
            .sum()
    }

    fn action_count(&self) -> usize {
        self.groups
            .values()
            .flatten()
            .filter(|path| !self.kept.contains(*path))
            .count()
    }

    fn keep_all(&mut self) {
        self.kept = self.groups.values().flatten().cloned().collect();
    }

    fn keep_first_in_each_group(&mut self) {
        self.kept = self
            .groups
            .values()
            .filter_map(|paths| paths.first().cloned())
            .collect();
    }

    fn every_group_has_keeper(&self) -> bool {
        self.groups
            .values()
            .all(|paths| paths.iter().any(|path| self.kept.contains(path)))
    }
}

struct MediaSiftApp {
    page: Page,
    notice: Notice,
    receiver: Option<Receiver<WorkResult>>,
    operation: Option<Operation>,
    review: Option<Review>,
    include_removable: bool,
    include_network: bool,
    extra_roots: Vec<PathBuf>,
    compression: usize,
    pending_action: Option<PendingAction>,
    pending_sort: Option<PathBuf>,
    pending_date_prefix: Option<PathBuf>,
    use_oldest_date: bool,
    scan_progress: Option<ScanProgress>,
    scan_report: String,
}

impl MediaSiftApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        configure_style(&creation_context.egui_ctx);
        Self::initial()
    }

    fn initial() -> Self {
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
            extra_roots: Vec::new(),
            compression: 2,
            pending_action: None,
            pending_sort: None,
            pending_date_prefix: None,
            use_oldest_date: false,
            scan_progress: None,
            scan_report: String::new(),
        }
    }

    fn is_working(&self) -> bool {
        self.receiver.is_some()
    }

    fn set_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = Notice::new(kind, text);
    }

    fn choose_sort_folder(&mut self) {
        if let Some(directory) = FileDialog::new()
            .set_title("Choose the folder whose images you want to organize")
            .pick_folder()
        {
            self.pending_sort = Some(directory);
        }
    }

    fn run_sort(&mut self, directory: PathBuf) {
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::Sort);
        self.pending_sort = None;
        self.set_notice(
            NoticeKind::Info,
            format!("Organizing images below {}...", directory.display()),
        );
        std::thread::spawn(move || {
            let _ = sender.send(WorkResult::Sorted(
                sort_images(&directory).map_err(|error| error.to_string()),
            ));
        });
    }

    fn choose_date_prefix_folder(&mut self) {
        if let Some(directory) = FileDialog::new()
            .set_title("Choose the folder whose media files you want to rename")
            .pick_folder()
        {
            self.pending_date_prefix = Some(directory);
        }
    }

    fn run_date_prefix(&mut self, directory: PathBuf) {
        let use_oldest_date = self.use_oldest_date;
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.operation = Some(Operation::Prefix);
        self.pending_date_prefix = None;
        self.set_notice(
            NoticeKind::Info,
            format!("Adding date prefixes below {}...", directory.display()),
        );
        std::thread::spawn(move || {
            let _ = sender.send(WorkResult::Prefixed(prefix_media_files_with_date(
                &directory,
                use_oldest_date,
            )));
        });
    }

    fn start_photo_enhancement(&mut self) {
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

    fn copy_restoration_prompt(&mut self, context: &egui::Context) {
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

    fn add_folder(&mut self) {
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

    fn start_media_scan(&mut self) {
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

    fn selected_unkept(&self) -> Result<Vec<PathBuf>, String> {
        let review = self.review.as_ref().ok_or("Run a duplicate scan first.")?;
        unkept_duplicate_paths(&review.groups, &review.kept)
    }

    fn begin_action(&mut self, action: PendingAction) {
        match self.selected_unkept() {
            Ok(paths) if paths.is_empty() => self.set_notice(
                NoticeKind::Info,
                "No files are selected for action. Uncheck Keep on at least one extra copy.",
            ),
            Ok(_) => self.pending_action = Some(action),
            Err(error) => self.set_notice(NoticeKind::Error, error),
        }
    }

    fn run_action(&mut self, action: PendingAction) {
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

    fn collect_result(&mut self) {
        let results: Vec<_> = self
            .receiver
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        let mut finished = false;
        for result in results {
            match result {
                WorkResult::Progress(progress) => self.scan_progress = Some(progress),
                WorkResult::Sorted(Ok(moved)) => {
                    finished = true;
                    self.set_notice(
                        NoticeKind::Success,
                        format!(
                            "Organized {moved} image(s) into Year/Month folders. Images without a usable capture date were left in place."
                        ),
                    );
                }
                WorkResult::Prefixed(Ok(summary)) => {
                    finished = true;
                    let kind = if summary.failures.is_empty() {
                        NoticeKind::Success
                    } else {
                        NoticeKind::Warning
                    };
                    self.set_notice(
                        kind,
                        format!(
                            "Renamed {} media file(s), skipped {}, and encountered {} failure(s).",
                            summary.renamed,
                            summary.skipped,
                            summary.failures.len()
                        ),
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
                WorkResult::Sorted(Err(error))
                | WorkResult::Prefixed(Err(error))
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

    fn select_source(&mut self, root: &Path, only: bool) {
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

    fn save_scan_report(&mut self) {
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
            .frame(
                Frame::new()
                    .fill(context.style().visuals.panel_fill)
                    .inner_margin(Margin::symmetric(20, 12))
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

    fn status_bar(&self, context: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(
                Frame::new()
                    .fill(context.style().visuals.panel_fill)
                    .inner_margin(Margin::symmetric(20, 12))
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
                ui.horizontal(|ui| {
                    if let Some(operation) = self.operation {
                        ui.spinner();
                        ui.vertical(|ui| {
                            ui.strong(operation.label());
                            ui.label(&self.notice.text);
                        });
                    } else {
                        let color = notice_color(self.notice.kind, ui.visuals().dark_mode);
                        ui.label(RichText::new(self.notice.label()).strong().color(color));
                        ui.separator();
                        ui.label(&self.notice.text);
                    }
                });
            });
    }

    fn overview_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Your media workspace",
            "Choose what you want to accomplish. Every workflow runs locally, and MediaSift confirms file-changing actions first.",
        );

        let two_columns = ui.available_width() >= 760.0;
        if two_columns {
            ui.columns(2, |columns| {
                self.duplicate_workflow_card(&mut columns[0]);
                self.organize_workflow_card(&mut columns[1]);
            });
            ui.add_space(16.0);
            ui.columns(2, |columns| {
                self.rename_workflow_card(&mut columns[0]);
                self.photo_workflow_card(&mut columns[1]);
            });
        } else {
            self.duplicate_workflow_card(ui);
            ui.add_space(12.0);
            self.organize_workflow_card(ui);
            ui.add_space(12.0);
            self.rename_workflow_card(ui);
            ui.add_space(12.0);
            self.photo_workflow_card(ui);
        }

        ui.add_space(20.0);
        info_banner(
            ui,
            "What “exact duplicate” means",
            "Duplicate scans compare file contents byte for byte. Similar-looking photos, edited versions, and differently encoded copies are not treated as duplicates.",
        );
    }

    fn duplicate_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Find exact duplicates",
            "Scan image, video, and audio files; review every match before choosing what happens next.",
            "Read-only until you confirm an action",
            |ui| {
                if primary_button(ui, "Open duplicate finder").clicked() {
                    self.page = Page::Duplicates;
                }
            },
        );
    }

    fn organize_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Sort photos by capture date",
            "Move photos with EXIF capture dates into Year/Month folders while preserving their filenames.",
            "Moves files after confirmation",
            |ui| {
                if secondary_button(ui, "Open organizer").clicked() {
                    self.page = Page::Organize;
                }
            },
        );
    }

    fn rename_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Add dates to filenames",
            "Prefix known media files with YYYY-MM-DD so they sort chronologically in any file browser.",
            "Renames files after confirmation",
            |ui| {
                if secondary_button(ui, "Open rename tool").clicked() {
                    self.page = Page::Organize;
                }
            },
        );
    }

    fn photo_workflow_card(&mut self, ui: &mut egui::Ui) {
        workflow_card(
            ui,
            "Restore a photo",
            "Create a conservative enhanced copy, or prepare an identity-preserving prompt for an external AI tool.",
            "Source photo is preserved",
            |ui| {
                if secondary_button(ui, "Open photo tools").clicked() {
                    self.page = Page::PhotoTools;
                }
            },
        );
    }

    fn duplicates_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Find exact duplicates",
            "Scan locally, compare contents with SHA-256, then decide which copies to keep. Scanning never changes a file.",
        );

        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "1");
                ui.vertical(|ui| {
                    ui.heading("Choose where to scan");
                    ui.label("Local fixed drives are included automatically.");
                });
            });
            ui.add_space(14.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                ui.checkbox(&mut self.include_removable, "Include removable drives")
                    .on_hover_text("Include connected USB drives and memory cards.");
                ui.checkbox(&mut self.include_network, "Include network drives")
                    .on_hover_text(
                        "Include mapped network locations. This may make the scan much slower.",
                    );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if secondary_button(ui, "Add a specific folder...").clicked() {
                        self.add_folder();
                    }
                    let clear = ui.add_enabled(
                        !self.extra_roots.is_empty(),
                        egui::Button::new("Clear added folders"),
                    );
                    if clear.clicked() {
                        self.extra_roots.clear();
                    }
                });
            });
            if self.extra_roots.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("No extra folders added.")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            } else {
                ui.add_space(10.0);
                let mut remove = None;
                for (index, root) in self.extra_roots.iter().enumerate() {
                    Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .corner_radius(7)
                        .inner_margin(Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(root.display().to_string());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .small_button("Remove")
                                        .on_hover_text("Remove this folder from the scan scope")
                                        .clicked()
                                    {
                                        remove = Some(index);
                                    }
                                });
                            });
                        });
                    ui.add_space(4.0);
                }
                if let Some(index) = remove {
                    self.extra_roots.remove(index);
                }
            }
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Start read-only scan").clicked() {
                    self.start_media_scan();
                }
            });
        });

        if let Some(progress) = self.scan_progress.clone() {
            ui.add_space(16.0);
            self.scan_progress_ui(ui, &progress);
        }

        if self.review.is_some() {
            ui.add_space(16.0);
            self.review_ui(ui);
        } else if !self.is_working() {
            ui.add_space(16.0);
            empty_state(
                ui,
                "Results will appear here",
                "After the scan, exact matches are grouped together. Every copy starts marked Keep, so nothing is acted on by accident.",
            );
        }
    }

    fn scan_progress_ui(&mut self, ui: &mut egui::Ui, progress: &ScanProgress) {
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, if self.review.is_some() { "✓" } else { "2" });
                ui.vertical(|ui| {
                    ui.heading(if self.review.is_some() {
                        "Scan complete"
                    } else {
                        "Scanning your media"
                    });
                    ui.label(if self.review.is_some() {
                        "Review the matches below."
                    } else {
                        "Large drives can take a while. You can leave this window open in the background."
                    });
                });
            });
            ui.add_space(14.0);
            metric_row(
                ui,
                &[
                    ("Folders", progress.directories_visited),
                    ("Files seen", progress.files_visited),
                    ("Media", progress.media_files_found),
                    ("Hashed", progress.files_hashed),
                    ("Groups", progress.duplicate_groups),
                ],
            );
            if self.is_working() {
                ui.add_space(12.0);
                ui.add(
                    egui::ProgressBar::new(0.5)
                        .animate(true)
                        .text("Scanning..."),
                );
            }
            if !self.scan_report.is_empty() {
                ui.add_space(12.0);
                self.report_actions(ui);
            }
        });
    }

    fn review_ui(&mut self, ui: &mut egui::Ui) {
        let Some(review) = &self.review else {
            return;
        };
        if review.groups.is_empty() {
            success_state(
                ui,
                "No exact duplicates found",
                "The scanned media files do not contain any byte-for-byte duplicate copies.",
            );
            ui.add_space(10.0);
            self.report_actions(ui);
            return;
        }

        let groups = review.groups.len();
        let total_files = review.total_files();
        let extra_copies = review.extra_copies();
        let action_count = review.action_count();
        let valid = review.every_group_has_keeper();
        let roots = review.roots.clone();

        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "3");
                ui.vertical(|ui| {
                    ui.heading("Choose which copies to keep");
                    ui.label("Checked files stay where they are. Unchecked files become selected for the action below.");
                });
            });
            ui.add_space(14.0);
            metric_row(
                ui,
                &[
                    ("Groups", groups),
                    ("Matching files", total_files),
                    ("Extra copies", extra_copies),
                    ("Selected", action_count),
                ],
            );
            ui.add_space(14.0);
            ui.label(RichText::new("Quick selection").strong());
            ui.horizontal_wrapped(|ui| {
                if secondary_button(ui, "Keep everything").clicked()
                    && let Some(review) = &mut self.review
                {
                    review.keep_all();
                }
                if secondary_button(ui, "Keep the first copy in each group").clicked()
                    && let Some(review) = &mut self.review
                {
                    review.keep_first_in_each_group();
                }
            });
            ui.add_space(10.0);
            ui.collapsing("Prefer copies from a location", |ui| {
                ui.label("These controls mark copies in the chosen location as Keep.");
                for root in roots {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(root.display().to_string());
                        if ui.small_button("Keep from here").clicked() {
                            self.select_source(&root, false);
                        }
                        if ui.small_button("Keep only from here").clicked() {
                            self.select_source(&root, true);
                        }
                    });
                }
            });
        });

        ui.add_space(12.0);
        let group_paths: Vec<Vec<PathBuf>> = self
            .review
            .as_ref()
            .expect("review exists")
            .groups
            .values()
            .cloned()
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("duplicate_groups")
            .max_height(500.0)
            .show(ui, |ui| {
                for (index, paths) in group_paths.iter().enumerate() {
                    duplicate_group_card(ui, index, paths, &mut self.review);
                    ui.add_space(10.0);
                }
            });

        ui.add_space(6.0);
        section_card(ui, |ui| {
            ui.horizontal(|ui| {
                number_badge(ui, "4");
                ui.vertical(|ui| {
                    ui.heading("Choose what happens to selected copies");
                    ui.label(format!(
                        "{action_count} file(s) are selected. At least one copy in every group must remain marked Keep."
                    ));
                });
            });
            if !valid {
                ui.add_space(12.0);
                warning_banner(
                    ui,
                    "One or more groups have no keeper. Mark at least one file Keep in every group before continuing.",
                );
            }
            ui.add_space(14.0);
            ui.add_enabled_ui(!self.is_working() && valid && action_count > 0, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if primary_button(ui, &format!("Recycle {action_count} file(s)")).clicked() {
                        self.begin_action(PendingAction::Recycle);
                    }
                    if secondary_button(ui, "Create ZIP backup...").clicked() {
                        self.begin_action(PendingAction::Archive);
                    }
                    if danger_button(ui, "Permanently delete...").clicked() {
                        self.begin_action(PendingAction::Delete);
                    }
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("ZIP compression");
                egui::ComboBox::from_id_salt("compression")
                    .selected_text(["None", "Fast", "Default", "Maximum"][self.compression])
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.compression, 0, "None (fastest)");
                        ui.selectable_value(&mut self.compression, 1, "Fast");
                        ui.selectable_value(&mut self.compression, 2, "Default");
                        ui.selectable_value(&mut self.compression, 3, "Maximum (smallest)");
                    });
            });
            ui.add_space(10.0);
            self.report_actions(ui);
        });
    }

    fn report_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if secondary_button(ui, "Copy scan report").clicked() {
                ui.ctx().copy_text(self.scan_report.clone());
                self.set_notice(
                    NoticeKind::Success,
                    "Copied the scan report to the clipboard.",
                );
            }
            if secondary_button(ui, "Save scan report...").clicked() {
                self.save_scan_report();
            }
        });
    }

    fn organize_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Organize files",
            "Use capture dates to create a predictable library. Both tools show a confirmation before changing files.",
        );

        let two_columns = ui.available_width() >= 800.0;
        if two_columns {
            ui.columns(2, |columns| {
                self.sort_card(&mut columns[0]);
                self.prefix_card(&mut columns[1]);
            });
        } else {
            self.sort_card(ui);
            ui.add_space(14.0);
            self.prefix_card(ui);
        }
        ui.add_space(18.0);
        info_banner(
            ui,
            "Before you begin",
            "Consider making a backup before reorganizing a large library. MediaSift prevents destination-name collisions, but it does not provide an undo command for moves or renames.",
        );
    }

    fn sort_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Sort into Year/Month folders");
            ui.label("Example: Vacation/photo.jpg → Vacation/2025/Jul/photo.jpg");
            ui.add_space(10.0);
            check_line(ui, "Reads EXIF DateTimeOriginal capture dates");
            check_line(ui, "Leaves photos without a usable capture date in place");
            check_line(ui, "Adds a numeric suffix when a filename already exists");
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Choose folder and review...").clicked() {
                    self.choose_sort_folder();
                }
            });
        });
    }

    fn prefix_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Add dates to media filenames");
            ui.label("Example: photo.jpg → 2025-07-14 - photo.jpg");
            ui.add_space(10.0);
            check_line(ui, "Works with known image, video, and audio files");
            check_line(ui, "Skips files that already start with a date");
            check_line(ui, "Prevents filename collisions");
            ui.add_space(10.0);
            ui.checkbox(
                &mut self.use_oldest_date,
                "Use the older of created and modified dates",
            )
            .on_hover_text("By default, MediaSift uses the file's creation date when available.");
            ui.add_space(12.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Choose folder and review...").clicked() {
                    self.choose_date_prefix_folder();
                }
            });
        });
    }

    fn photo_tools_ui(&mut self, ui: &mut egui::Ui) {
        page_intro(
            ui,
            "Photo tools",
            "Make a conservative enhanced copy locally, or copy a careful restoration prompt for use in another tool.",
        );
        let two_columns = ui.available_width() >= 800.0;
        if two_columns {
            ui.columns(2, |columns| {
                self.enhance_card(&mut columns[0]);
                self.prompt_card(&mut columns[1]);
            });
        } else {
            self.enhance_card(ui);
            ui.add_space(14.0);
            self.prompt_card(ui);
        }
    }

    fn enhance_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Create an enhanced copy");
            ui.label("Apply mild local cleanup, contrast, and color adjustments.");
            ui.add_space(10.0);
            check_line(ui, "Never replaces the source photo");
            check_line(ui, "Saves a sibling file with “(Enhanced)” in its name");
            check_line(ui, "Supports PNG, BMP, and TIFF input");
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if primary_button(ui, "Choose a photo...").clicked() {
                    self.start_photo_enhancement();
                }
            });
        });
    }

    fn prompt_card(&mut self, ui: &mut egui::Ui) {
        section_card(ui, |ui| {
            ui.heading("Copy an AI restoration prompt");
            ui.label(
                "Prepare a conservative prompt that prioritizes identity and historical detail.",
            );
            ui.add_space(10.0);
            check_line(ui, "Names the selected file in the prompt");
            check_line(ui, "Requests no identity or facial-structure changes");
            check_line(ui, "Copies text only; MediaSift uploads nothing");
            ui.add_space(16.0);
            ui.add_enabled_ui(!self.is_working(), |ui| {
                if secondary_button(ui, "Choose photo and copy prompt").clicked() {
                    self.copy_restoration_prompt(ui.ctx());
                }
            });
        });
    }

    fn dialogs(&mut self, context: &egui::Context) {
        if let Some(action) = self.pending_action {
            let count = self.selected_unkept().map_or(0, |paths| paths.len());
            egui::Window::new(action.title())
                .id(egui::Id::new("confirm_file_action"))
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .default_width(470.0)
                .show(context, |ui| {
                    ui.label(match action {
                        PendingAction::Recycle => format!(
                            "{count} unchecked duplicate file(s) will move to the Recycle Bin. You can usually restore them later."
                        ),
                        PendingAction::Delete => format!(
                            "{count} unchecked duplicate file(s) will be permanently deleted. This cannot be undone."
                        ),
                        PendingAction::Archive => format!(
                            "{count} unchecked duplicate file(s) will be copied into a ZIP backup. The original files will remain in place."
                        ),
                    });
                    ui.add_space(8.0);
                    ui.label("Files marked Keep are never included in this action.");
                    ui.add_space(16.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let confirm_label = match action {
                            PendingAction::Recycle => format!("Recycle {count} file(s)"),
                            PendingAction::Delete => format!("Delete {count} file(s) permanently"),
                            PendingAction::Archive => "Choose ZIP location...".to_owned(),
                        };
                        let confirmed = if action == PendingAction::Delete {
                            danger_button(ui, &confirm_label).clicked()
                        } else {
                            primary_button(ui, &confirm_label).clicked()
                        };
                        if confirmed {
                            self.run_action(action);
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_action = None;
                        }
                    });
                });
        }

        if let Some(directory) = self.pending_sort.clone()
            && let Some(confirmed) = confirmation_dialog(
                context,
                "Organize this folder?",
                &format!(
                    "MediaSift will move images with capture dates below:\n{}",
                    directory.display()
                ),
                "Destination folders use Year/Month. Files without a usable capture date stay in place. MediaSift does not provide an undo command.",
                "Organize images",
            )
        {
            if confirmed {
                self.run_sort(directory);
            } else {
                self.pending_sort = None;
            }
        }

        if let Some(directory) = self.pending_date_prefix.clone()
            && let Some(confirmed) = confirmation_dialog(
                context,
                "Rename media files in this folder?",
                &format!(
                    "MediaSift will add YYYY-MM-DD prefixes to known media files below:\n{}",
                    directory.display()
                ),
                "Already-prefixed files are skipped and name collisions are prevented. MediaSift does not provide an undo command.",
                "Add date prefixes",
            )
        {
            if confirmed {
                self.run_date_prefix(directory);
            } else {
                self.pending_date_prefix = None;
            }
        }
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

        egui::CentralPanel::default()
            .frame(Frame::new().inner_margin(Margin::symmetric(24, 22)))
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

fn configure_style(context: &egui::Context) {
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

fn page_intro(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.heading(RichText::new(title).size(30.0).strong());
    ui.add_space(2.0);
    ui.label(
        RichText::new(description)
            .size(16.0)
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(22.0);
}

fn section_card<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::new()
        .fill(ui.visuals().panel_fill)
        .stroke(Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(12)
        .inner_margin(Margin::same(18))
        .show(ui, content)
        .inner
}

fn workflow_card(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    safety_note: &str,
    action: impl FnOnce(&mut egui::Ui),
) {
    section_card(ui, |ui| {
        ui.set_min_height(188.0);
        ui.heading(title);
        ui.label(description);
        ui.add_space(12.0);
        ui.label(
            RichText::new(safety_note)
                .small()
                .strong()
                .color(notice_color(NoticeKind::Info, ui.visuals().dark_mode)),
        );
        ui.with_layout(Layout::bottom_up(Align::LEFT), action);
    });
}

fn empty_state(ui: &mut egui::Ui, title: &str, description: &str) {
    Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(12)
        .inner_margin(Margin::same(24))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading(title);
                ui.label(description);
            });
        });
}

fn success_state(ui: &mut egui::Ui, title: &str, description: &str) {
    Frame::new()
        .fill(if ui.visuals().dark_mode {
            Color32::from_rgb(16, 52, 41)
        } else {
            Color32::from_rgb(236, 253, 243)
        })
        .stroke(Stroke::new(
            1.0_f32,
            notice_color(NoticeKind::Success, ui.visuals().dark_mode),
        ))
        .corner_radius(12)
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.heading(title);
            ui.label(description);
        });
}

fn info_banner(ui: &mut egui::Ui, title: &str, description: &str) {
    Frame::new()
        .fill(if ui.visuals().dark_mode {
            Color32::from_rgb(21, 39, 70)
        } else {
            Color32::from_rgb(239, 246, 255)
        })
        .stroke(Stroke::new(1.0_f32, ACCENT))
        .corner_radius(10)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.label(description);
        });
}

fn warning_banner(ui: &mut egui::Ui, message: &str) {
    Frame::new()
        .fill(if ui.visuals().dark_mode {
            Color32::from_rgb(65, 44, 18)
        } else {
            Color32::from_rgb(255, 250, 235)
        })
        .stroke(Stroke::new(
            1.0_f32,
            notice_color(NoticeKind::Warning, ui.visuals().dark_mode),
        ))
        .corner_radius(8)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.label(RichText::new("Selection needs attention").strong());
            ui.label(message);
        });
}

fn check_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("✓")
                .strong()
                .color(notice_color(NoticeKind::Success, ui.visuals().dark_mode)),
        );
        ui.label(text);
    });
}

fn number_badge(ui: &mut egui::Ui, number: &str) {
    Frame::new()
        .fill(ACCENT)
        .corner_radius(20)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.label(RichText::new(number).strong().color(Color32::WHITE));
        });
}

fn metric_row(ui: &mut egui::Ui, metrics: &[(&str, usize)]) {
    ui.horizontal_wrapped(|ui| {
        for (label, value) in metrics {
            Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(value.to_string()).size(20.0).strong());
                        ui.label(
                            RichText::new(*label)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                });
        }
    });
}

fn duplicate_group_card(
    ui: &mut egui::Ui,
    index: usize,
    paths: &[PathBuf],
    review: &mut Option<Review>,
) {
    section_card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Group {}", index + 1))
                    .heading()
                    .strong(),
            );
            ui.label(
                RichText::new(format!("{} exact copies", paths.len()))
                    .color(ui.visuals().weak_text_color()),
            );
        });
        ui.add_space(8.0);
        for path in paths {
            let mut keep = review
                .as_ref()
                .is_some_and(|review| review.kept.contains(path));
            let filename = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into(),
            );
            Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .checkbox(&mut keep, "Keep")
                            .on_hover_text("Checked files will not be included in any action")
                            .changed()
                            && let Some(review) = review
                        {
                            if keep {
                                review.kept.insert(path.clone());
                            } else {
                                review.kept.remove(path);
                            }
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(filename).strong());
                            ui.label(
                                RichText::new(path.display().to_string())
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        });
                    });
                });
            ui.add_space(5.0);
        }
    });
}

fn confirmation_dialog(
    context: &egui::Context,
    title: &str,
    primary_text: &str,
    detail: &str,
    confirm_label: &str,
) -> Option<bool> {
    let mut confirmed = false;
    let mut cancelled = false;
    egui::Window::new(title)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .default_width(480.0)
        .show(context, |ui| {
            ui.label(primary_text);
            ui.add_space(8.0);
            ui.label(detail);
            ui.add_space(16.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                confirmed = primary_button(ui, confirm_label).clicked();
                cancelled = ui.button("Cancel").clicked();
            });
        });
    if confirmed {
        Some(true)
    } else if cancelled {
        Some(false)
    } else {
        None
    }
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .corner_radius(8),
    )
}

fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(egui::Button::new(label).corner_radius(8))
}

fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
            .fill(DANGER)
            .stroke(Stroke::NONE)
            .corner_radius(8),
    )
}

fn notice_color(kind: NoticeKind, dark_mode: bool) -> Color32 {
    match (kind, dark_mode) {
        (NoticeKind::Info, false) => Color32::from_rgb(29, 78, 216),
        (NoticeKind::Info, true) => Color32::from_rgb(147, 197, 253),
        (NoticeKind::Success, false) => Color32::from_rgb(2, 122, 72),
        (NoticeKind::Success, true) => Color32::from_rgb(110, 231, 183),
        (NoticeKind::Warning, false) => Color32::from_rgb(181, 71, 8),
        (NoticeKind::Warning, true) => Color32::from_rgb(253, 186, 116),
        (NoticeKind::Error, false) => DANGER,
        (NoticeKind::Error, true) => Color32::from_rgb(253, 164, 175),
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

    fn render_at(size: Vec2, theme: egui::Theme, page: Page, review: Option<Review>) {
        let context = egui::Context::default();
        context.set_theme(theme);
        configure_style(&context);
        let mut app = MediaSiftApp::initial();
        app.page = page;
        app.review = review;
        app.scan_progress = Some(ScanProgress {
            directories_visited: 12,
            files_visited: 240,
            media_files_found: 180,
            files_hashed: 80,
            duplicate_groups: 2,
            ..Default::default()
        });
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| match app.page {
                    Page::Overview => app.overview_ui(ui),
                    Page::Duplicates => app.duplicates_ui(ui),
                    Page::Organize => app.organize_ui(ui),
                    Page::PhotoTools => app.photo_tools_ui(ui),
                });
            },
        );
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn overview_layout_renders_at_narrow_and_wide_sizes() {
        render_at(
            Vec2::new(720.0, 560.0),
            egui::Theme::Light,
            Page::Overview,
            None,
        );
        render_at(
            Vec2::new(1440.0, 900.0),
            egui::Theme::Dark,
            Page::Overview,
            None,
        );
    }

    #[test]
    fn duplicate_review_renders_in_both_themes() {
        let mut light_review = sample_review();
        light_review.keep_all();
        render_at(
            Vec2::new(820.0, 700.0),
            egui::Theme::Light,
            Page::Duplicates,
            Some(light_review),
        );

        let mut dark_review = sample_review();
        dark_review.keep_first_in_each_group();
        render_at(
            Vec2::new(1280.0, 800.0),
            egui::Theme::Dark,
            Page::Duplicates,
            Some(dark_review),
        );
    }
}
