//! ROM Manager window: DAT audit, 1G1R selection and safe rebuilding.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

use egui::{Color32, RichText};
use unko_rom_manager::{
    clear_cache_file, parse_dat_path, rebuild_with_progress, scan_path, summary_line, write_csv,
    write_fixdat, Catalog, MatchReport, MatchStatus, OneGameOneRom, OutputFormat, RebuildOptions,
    RebuildPhase, RebuildProgress, RebuildReport, ScanCache, ScanOptions, ScanProgress,
    VerifyReport,
};

use crate::rom_manager_view::{verify_folder_with_progress, VerifyView};

type ScanJob = std::result::Result<Vec<MatchReport>, String>;

struct RebuildOutcome {
    result: RebuildReport,
    verify: Option<std::result::Result<VerifyView, String>>,
}

enum WorkerMessage {
    Scan(ScanJob),
    Rebuild(std::result::Result<Box<RebuildOutcome>, String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusyKind {
    Scan,
    Rebuild,
}

/// ROM Manager state.
pub struct RomManager {
    /// Window visibility.
    pub open: bool,
    focus_pending: bool,
    focus_frames: u8,
    dat_paths: Vec<PathBuf>,
    dat_input: String,
    source: PathBuf,
    output: PathBuf,
    cache: PathBuf,
    use_cache: bool,
    cache_entries: usize,
    region_order: String,
    language_order: String,
    one_g1r: bool,
    include_bios: bool,
    zip_sets: bool,
    torrentzip: bool,
    move_files: bool,
    prefer_parent: bool,
    prefer_verified: bool,
    prefer_retail: bool,
    prefer_newer: bool,
    filter: String,
    busy: bool,
    status: String,
    progress: Arc<Mutex<ScanProgress>>,
    rebuild_progress: Arc<Mutex<RebuildProgress>>,
    busy_kind: BusyKind,
    rx: Option<Receiver<WorkerMessage>>,
    reports: Vec<MatchReport>,
    selected: usize,
    verify: Option<VerifyView>,
    verify_after_scan: bool,
}

impl RomManager {
    /// Creates an idle manager.
    #[must_use]
    pub fn new(save_dir: PathBuf, source: PathBuf) -> Self {
        let cache = save_dir.join("rom-manager").join("scan-cache.tsv");
        let cache_entries = ScanCache::load(&cache).len();
        Self {
            open: false,
            focus_pending: false,
            focus_frames: 0,
            dat_paths: Vec::new(),
            dat_input: String::new(),
            source,
            output: save_dir.join("rom-manager").join("rebuilt"),
            cache,
            use_cache: true,
            cache_entries,
            region_order: "USA,Europe,World,Japan".into(),
            language_order: "En".into(),
            one_g1r: true,
            include_bios: false,
            zip_sets: false,
            torrentzip: false,
            move_files: false,
            prefer_parent: true,
            prefer_verified: true,
            prefer_retail: true,
            prefer_newer: true,
            filter: String::new(),
            busy: false,
            status: "Choose one or more DAT files and a source folder, then scan.".into(),
            progress: Arc::new(Mutex::new(ScanProgress::default())),
            rebuild_progress: Arc::new(Mutex::new(RebuildProgress::default())),
            busy_kind: BusyKind::Scan,
            rx: None,
            reports: Vec::new(),
            selected: 0,
            verify: None,
            verify_after_scan: false,
        }
    }

    /// Adds DAT files and opens the window.
    pub fn preload(&mut self, dats: Vec<PathBuf>, open: bool) {
        if !dats.is_empty() {
            self.dat_paths = dats;
        }
        self.open |= open;
        if open {
            self.focus_pending = true;
            self.focus_frames = 8;
        }
    }

    /// Opens the verification report automatically after the next scan.
    pub fn set_verify_after_scan(&mut self, value: bool) {
        self.verify_after_scan = value;
    }

    /// Opens the manager and asks the OS window to take focus.
    pub fn request_focus(&mut self) {
        self.open = true;
        self.focus_pending = true;
        self.focus_frames = 8;
    }

    /// Polls the background scan.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        match rx.try_recv() {
            Ok(WorkerMessage::Scan(Ok(reports))) => {
                self.reports = reports;
                self.selected = 0;
                self.verify = None;
                self.busy = false;
                self.rx = None;
                self.cache_entries = ScanCache::load(&self.cache).len();
                if let Some(report) = self.reports.first() {
                    self.status = summary_line(report);
                    if self.verify_after_scan {
                        self.verify = Some(VerifyView::from_report(&VerifyReport::build(report)));
                    }
                } else {
                    self.status = "No DAT produced a report.".into();
                }
            }
            Ok(WorkerMessage::Scan(Err(error))) => {
                self.busy = false;
                self.rx = None;
                self.status = error;
            }
            Ok(WorkerMessage::Rebuild(Ok(outcome))) => {
                self.busy = false;
                self.rx = None;
                self.status = format!(
                    "Rebuilt {} files in {} archives, {} moved ({} skipped, {} errors) into {}",
                    outcome.result.written,
                    outcome.result.archives,
                    outcome.result.moved,
                    outcome.result.skipped,
                    outcome.result.errors.len(),
                    self.output.display()
                );
                if let Some(error) = outcome.result.errors.first() {
                    self.status.push_str(&format!(" — {error}"));
                }
                if let Some(verify) = outcome.verify {
                    match verify {
                        Ok(view) => {
                            self.status =
                                format!("{} — verification: {}", self.status, view.status_label());
                            self.verify = Some(view);
                        }
                        Err(err) => {
                            self.status = format!("{} — verification failed: {err}", self.status);
                        }
                    }
                }
            }
            Ok(WorkerMessage::Rebuild(Err(error))) => {
                self.busy = false;
                self.rx = None;
                self.status = error;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.busy = false;
                self.rx = None;
                self.status = "ROM Manager worker stopped".into();
            }
        }
    }

    /// Draws the detached OS window contents.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn show_contents(&mut self, ctx: &egui::Context) {
        if self.focus_pending || self.focus_frames > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.focus_pending = false;
            self.focus_frames = self.focus_frames.saturating_sub(1);
        }
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        egui::CentralPanel::default().show(ctx, |ui| self.body(ui));
    }

    /// Draws an embedded window for mobile and non-desktop targets.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    pub fn show_embedded(&mut self, ctx: &egui::Context, open: &mut bool) {
        if !*open {
            return;
        }
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        egui::Window::new("ROM Manager")
            .open(open)
            .resizable(true)
            .default_size([1000.0, 700.0])
            .show(ctx, |ui| self.body(ui));
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        ui.heading("ROM Manager");
        ui.label(
            RichText::new(
                "DAT audit, 1G1R selection and safe copying. The manager is a standalone core; it does not change the NES emulator.",
            )
            .weak()
            .small(),
        );
        ui.add_space(6.0);

        ui.label("DAT files");
        let mut remove = None;
        for (index, path) in self.dat_paths.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.monospace(path.display().to_string());
                if ui.small_button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.dat_paths.remove(index);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.dat_input)
                    .hint_text("path to .dat or .xml")
                    .desired_width(500.0),
            );
            if ui.button("Add path").clicked() && !self.dat_input.trim().is_empty() {
                self.dat_paths.push(PathBuf::from(self.dat_input.trim()));
                self.dat_input.clear();
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse DATs…").clicked() {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("DAT", &["dat", "xml"])
                    .pick_files()
                {
                    self.dat_paths.extend(paths);
                }
            }
        });

        ui.add_space(4.0);
        ui.label("Source folder");
        ui.horizontal(|ui| {
            let mut source = self.source.display().to_string();
            if ui.text_edit_singleline(&mut source).changed() {
                self.source = PathBuf::from(source);
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.source)
                    .pick_folder()
                {
                    self.source = path;
                }
            }
        });
        ui.label("Output folder");
        ui.horizontal(|ui| {
            let mut output = self.output.display().to_string();
            if ui.text_edit_singleline(&mut output).changed() {
                self.output = PathBuf::from(output);
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.output)
                    .pick_folder()
                {
                    self.output = path;
                }
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.one_g1r, "1G1R selection");
            ui.checkbox(&mut self.include_bios, "Include [BIOS] entries");
            ui.checkbox(&mut self.zip_sets, "ZIP sets");
            ui.add_enabled(
                self.zip_sets,
                egui::Checkbox::new(&mut self.torrentzip, "TorrentZip-style"),
            );
            ui.add_enabled(
                !self.zip_sets,
                egui::Checkbox::new(&mut self.move_files, "Move (delete sources)"),
            );
        });
        ui.horizontal(|ui| {
            ui.add_enabled_ui(self.one_g1r, |ui| {
                ui.checkbox(&mut self.prefer_parent, "Prefer parent");
                ui.checkbox(&mut self.prefer_verified, "Prefer verified");
                ui.checkbox(&mut self.prefer_retail, "Prefer retail");
                ui.checkbox(&mut self.prefer_newer, "Prefer newer revisions");
            });
        });
        ui.horizontal(|ui| {
            ui.label("Regions");
            ui.add(
                egui::TextEdit::singleline(&mut self.region_order)
                    .desired_width(260.0)
                    .hint_text("USA,Europe,World,Japan"),
            );
            ui.label("Languages");
            ui.add(
                egui::TextEdit::singleline(&mut self.language_order)
                    .desired_width(120.0)
                    .hint_text("En"),
            );
        });

        ui.add_space(7.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.use_cache, "Use scan cache");
            let bytes = fs::metadata(&self.cache).map_or(0, |meta| meta.len());
            ui.label(
                RichText::new(format!(
                    "{} entries · {}",
                    self.cache_entries,
                    cache_bytes_label(bytes)
                ))
                .small()
                .weak(),
            );
            if ui.button("Clear cache").clicked() {
                self.clear_scan_cache();
            }
        });
        ui.label(
            RichText::new(self.cache.display().to_string())
                .small()
                .weak(),
        );

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let can_scan =
                !self.busy && !self.dat_paths.is_empty() && !self.source.as_os_str().is_empty();
            if ui
                .add_enabled(can_scan, egui::Button::new("Scan / audit"))
                .clicked()
            {
                self.start_scan();
            }
            if self.busy {
                ui.spinner();
                match self.busy_kind {
                    BusyKind::Scan => {
                        let progress = self
                            .progress
                            .lock()
                            .map(|progress| progress.clone())
                            .unwrap_or_default();
                        ui.label(format!(
                            "{} files, {} members, {:.1} MiB",
                            progress.files,
                            progress.members,
                            progress.bytes as f64 / (1024.0 * 1024.0)
                        ));
                    }
                    BusyKind::Rebuild => {
                        let progress = self
                            .rebuild_progress
                            .lock()
                            .map(|progress| progress.clone())
                            .unwrap_or_default();
                        let detail = match progress.phase {
                            RebuildPhase::Writing => format!(
                                "Writing {}/{} games · {} written, {} archives, {} skipped, {} errors{}",
                                progress.games_done,
                                progress.games_total,
                                progress.written,
                                progress.archives,
                                progress.skipped,
                                progress.errors,
                                if progress.current.is_empty() {
                                    String::new()
                                } else {
                                    format!(" — {}", progress.current)
                                }
                            ),
                            RebuildPhase::Verifying => format!(
                                "Verifying output · {} files, {:.1} MiB",
                                progress.verified_files,
                                progress.verified_bytes as f64 / (1024.0 * 1024.0)
                            ),
                            RebuildPhase::Done => "Finishing…".into(),
                        };
                        ui.label(detail);
                    }
                }
            }
        });
        ui.label(&self.status);
        if self.busy {
            return;
        }
        let report_index = self.selected;
        let action = {
            let Some(report) = self.reports.get(report_index) else {
                return;
            };
            ui.add_space(4.0);
            if self.reports.len() > 1 {
                egui::ComboBox::from_id_salt("rom-manager-dat")
                    .selected_text(&report.dat.name)
                    .show_ui(ui, |ui| {
                        for (index, candidate) in self.reports.iter().enumerate() {
                            ui.selectable_value(&mut self.selected, index, &candidate.dat.name);
                        }
                    });
            }
            ui.label(
                RichText::new(format!("{} ({})", report.dat.name, report.dat.version)).strong(),
            );
            ui.label(summary_line(report));
            let selected_games = self.selected_games(report_index);
            ui.label(format!(
                "1G1R: {} games selected ({} complete in the source)",
                selected_games.len(),
                selected_games
                    .iter()
                    .filter(|index| report
                        .games
                        .get(**index)
                        .is_some_and(|game| game.status == MatchStatus::Complete))
                    .count()
            ));

            let mut action = None;
            ui.horizontal(|ui| {
                if ui.button("Export fixdat…").clicked() {
                    action = Some(Action::Fixdat);
                }
                if ui.button("Export CSV…").clicked() {
                    action = Some(Action::Csv);
                }
                if ui.button("Rebuild selected…").clicked() {
                    action = Some(Action::Rebuild);
                }
                if ui.button("Verify").clicked() {
                    action = Some(Action::Verify);
                }
            });

            if let Some(view) = &self.verify {
                verify_panel(ui, view, &mut action);
            } else {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Filter");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("game name")
                            .desired_width(300.0),
                    );
                });
                ui.separator();
                let filter = self.filter.to_lowercase();
                let mut order: Vec<usize> = (0..report.dat.games.len()).collect();
                order.sort_by_key(|index| match report.games[*index].status {
                    MatchStatus::Complete => 0,
                    MatchStatus::Partial => 1,
                    MatchStatus::Missing => 2,
                });
                let filtered: Vec<usize> = order
                    .into_iter()
                    .filter(|index| {
                        filter.is_empty()
                            || report.dat.games[*index]
                                .name
                                .to_lowercase()
                                .contains(&filter)
                    })
                    .collect();
                ui.label(format!("{} matching games", filtered.len()));
                let row_height = ui.spacing().interact_size.y.max(18.0);
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [380.0, row_height],
                        egui::Label::new(RichText::new("Game").strong()),
                    );
                    ui.add_sized(
                        [90.0, row_height],
                        egui::Label::new(RichText::new("Status").strong()),
                    );
                    ui.add_sized(
                        [60.0, row_height],
                        egui::Label::new(RichText::new("ROMs").strong()),
                    );
                    ui.add_sized(
                        [ui.available_width(), row_height],
                        egui::Label::new(RichText::new("Source").strong()),
                    );
                });
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height().max(140.0))
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height, filtered.len(), |ui, range| {
                        for index in filtered[range].iter().copied() {
                            let game = &report.dat.games[index];
                            let status = report.games[index];
                            let color = match status.status {
                                MatchStatus::Complete => Color32::LIGHT_GREEN,
                                MatchStatus::Partial => Color32::YELLOW,
                                MatchStatus::Missing => Color32::GRAY,
                            };
                            let source = game
                                .roms
                                .iter()
                                .enumerate()
                                .find_map(|(rom, _)| {
                                    report
                                        .flat_index(index, rom)
                                        .and_then(|flat| report.source(flat))
                                        .map(|rom| rom.origin.display())
                                })
                                .unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [380.0, row_height],
                                    egui::Label::new(&game.name).truncate(),
                                );
                                ui.add_sized(
                                    [90.0, row_height],
                                    egui::Label::new(
                                        RichText::new(status.status.label()).color(color),
                                    ),
                                );
                                ui.add_sized(
                                    [60.0, row_height],
                                    egui::Label::new(format!(
                                        "{}/{}",
                                        status.matched, status.total
                                    )),
                                );
                                ui.add_sized(
                                    [ui.available_width(), row_height],
                                    egui::Label::new(RichText::new(source).small().weak())
                                        .truncate(),
                                );
                            });
                        }
                    });
            }
            action
        };
        match action {
            Some(Action::Fixdat) => self.export_fixdat(report_index),
            Some(Action::Csv) => self.export_csv(report_index),
            Some(Action::Rebuild) => {
                let selected = self.selected_games(report_index);
                self.start_rebuild(report_index, &selected);
            }
            Some(Action::Verify) => {
                if let Some(report) = self.reports.get(report_index) {
                    self.verify = Some(VerifyView::from_report(&VerifyReport::build(report)));
                    self.status = "Verification report ready".into();
                }
            }
            Some(Action::CloseVerify) => self.verify = None,
            None => {}
        }
    }

    /// Scan options honouring the cache toggle.
    #[must_use]
    pub fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            cache: self.use_cache.then(|| self.cache.clone()),
            ..ScanOptions::default()
        }
    }

    /// Deletes the persistent scan cache and resets the displayed count.
    pub fn clear_scan_cache(&mut self) {
        match clear_cache_file(&self.cache) {
            Ok(()) => {
                self.cache_entries = 0;
                self.status = "Scan cache cleared".into();
            }
            Err(err) => self.status = format!("cache clear: {err}"),
        }
    }

    /// Starts a scan when DAT files and a source folder are present.
    pub fn start_scan(&mut self) {
        if self.busy {
            return;
        }
        let dats = self.dat_paths.clone();
        let source = self.source.clone();
        let options = self.scan_options();
        let progress = Arc::clone(&self.progress);
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.verify = None;
        self.rebuild_progress = Arc::new(Mutex::new(RebuildProgress::default()));
        self.busy_kind = BusyKind::Scan;
        self.busy = true;
        self.status = "Scanning…".into();
        thread::spawn(move || {
            let result = (|| -> ScanJob {
                let mut report_progress = |scan: &ScanProgress| {
                    if let Ok(mut current) = progress.lock() {
                        *current = scan.clone();
                    }
                };
                let (scanned, _) = scan_path(&source, &options, &mut report_progress)
                    .map_err(|err| err.to_string())?;
                let mut reports = Vec::new();
                for path in dats {
                    let dat = parse_dat_path(&path)
                        .map_err(|err| format!("{}: {err}", path.display()))?;
                    reports.push(Catalog::new(dat).match_all(scanned.clone()));
                }
                Ok(reports)
            })();
            let _ = tx.send(WorkerMessage::Scan(result));
        });
    }

    fn selected_games(&self, report_index: usize) -> Vec<usize> {
        let Some(report) = self.reports.get(report_index) else {
            return Vec::new();
        };
        if !self.one_g1r {
            return (0..report.dat.games.len()).collect();
        }
        OneGameOneRom {
            regions: split_list(&self.region_order),
            languages: split_list(&self.language_order),
            prefer_parent: self.prefer_parent,
            prefer_verified: self.prefer_verified,
            prefer_retail: self.prefer_retail,
            prefer_newer: self.prefer_newer,
            exclude_bios: !self.include_bios,
        }
        .select(&report.dat)
    }

    fn export_fixdat(&mut self, report_index: usize) {
        let Some(report) = self.reports.get(report_index) else {
            return;
        };
        let path = self.suggested_path(report_index, "fixdat.xml");
        let result = write_fixdat(report, &path);
        self.status = match result {
            Ok(()) => format!("Fixdat written to {}", path.display()),
            Err(err) => err.to_string(),
        };
    }

    fn export_csv(&mut self, report_index: usize) {
        let Some(report) = self.reports.get(report_index) else {
            return;
        };
        let path = self.suggested_path(report_index, "report.csv");
        let result = write_csv(report, &path);
        self.status = match result {
            Ok(()) => format!("CSV written to {}", path.display()),
            Err(err) => err.to_string(),
        };
    }

    /// Starts a rebuild and its post-verify on the worker thread.
    pub fn start_rebuild(&mut self, report_index: usize, selected: &[usize]) {
        if self.busy {
            return;
        }
        let Some(report) = self.reports.get(report_index).cloned() else {
            return;
        };
        let selected = selected.to_vec();
        let options = RebuildOptions {
            output: self.output.clone(),
            move_files: self.move_files && !self.zip_sets,
            format: if self.zip_sets && self.torrentzip {
                OutputFormat::TorrentZip
            } else if self.zip_sets {
                OutputFormat::Zip
            } else {
                OutputFormat::Loose
            },
            ..RebuildOptions::default()
        };
        let output = self.output.clone();
        let progress = Arc::new(Mutex::new(RebuildProgress {
            games_total: selected.len(),
            ..RebuildProgress::default()
        }));
        let worker_progress = Arc::clone(&progress);
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.rebuild_progress = progress;
        self.busy_kind = BusyKind::Rebuild;
        self.verify = None;
        self.busy = true;
        self.status = "Rebuilding…".into();
        thread::spawn(move || {
            let mut report_progress = |state: &RebuildProgress| {
                if let Ok(mut current) = worker_progress.lock() {
                    *current = state.clone();
                }
            };
            let result = rebuild_with_progress(&report, &selected, &options, &mut report_progress);
            let verify = if result.errors.is_empty() {
                if let Ok(mut current) = worker_progress.lock() {
                    current.phase = RebuildPhase::Verifying;
                    current.current = "Verifying output".into();
                }
                Some(verify_folder_with_progress(&report, &output, &mut |scan| {
                    if let Ok(mut current) = worker_progress.lock() {
                        current.verified_files = scan.files + scan.members;
                        current.verified_bytes = scan.bytes;
                    }
                }))
            } else {
                None
            };
            if let Ok(mut current) = worker_progress.lock() {
                current.phase = RebuildPhase::Done;
            }
            let _ = tx.send(WorkerMessage::Rebuild(Ok(Box::new(RebuildOutcome {
                result,
                verify,
            }))));
        });
    }

    fn suggested_path(&self, report_index: usize, suffix: &str) -> PathBuf {
        let stem = self
            .reports
            .get(report_index)
            .map_or("dat".into(), |report| sanitize(&report.dat.name));
        self.output
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            .join(format!("{stem}.{suffix}"))
    }
}

enum Action {
    Fixdat,
    Csv,
    Rebuild,
    Verify,
    CloseVerify,
}

fn verify_panel(ui: &mut egui::Ui, view: &VerifyView, action: &mut Option<Action>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let color = if view.clean {
            Color32::LIGHT_GREEN
        } else {
            Color32::LIGHT_RED
        };
        ui.label(RichText::new(view.status_label()).strong().color(color));
        ui.label(RichText::new(&view.dat).strong());
        if ui.button("Back to games").clicked() {
            *action = Some(Action::CloseVerify);
        }
    });
    ui.label(view.summary.as_str());
    if !view.unknown.is_empty() {
        verify_section(ui, "Unknown files", &view.unknown);
    }
    if !view.duplicates.is_empty() {
        verify_section(ui, "Duplicate files", &view.duplicates);
    }
    if !view.missing.is_empty() {
        egui::CollapsingHeader::new(format!("Missing ROMs ({})", view.missing.len()))
            .default_open(true)
            .show(ui, |ui| {
                for (game, rom) in view.missing.iter().take(300) {
                    ui.label(format!("{game}: {rom}"));
                }
                if view.missing.len() > 300 {
                    ui.label(format!("… {} more", view.missing.len() - 300));
                }
            });
    }
    ui.separator();
    ui.label(
        RichText::new(format!(
            "ROMs: {} found / {} expected",
            view.found, view.total
        ))
        .strong(),
    );
    let row_height = ui.spacing().interact_size.y.max(18.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            [250.0, row_height],
            egui::Label::new(RichText::new("Game").strong()),
        );
        ui.add_sized(
            [220.0, row_height],
            egui::Label::new(RichText::new("ROM").strong()),
        );
        ui.add_sized(
            [70.0, row_height],
            egui::Label::new(RichText::new("Status").strong()),
        );
        ui.add_sized(
            [150.0, row_height],
            egui::Label::new(RichText::new("Hash").strong()),
        );
        ui.add_sized(
            [ui.available_width(), row_height],
            egui::Label::new(RichText::new("Source").strong()),
        );
    });
    egui::ScrollArea::vertical()
        .max_height(ui.available_height().max(140.0))
        .auto_shrink([false, false])
        .show_rows(ui, row_height, view.rows.len(), |ui, range| {
            for row in &view.rows[range] {
                let color = match view.row_status(row) {
                    MatchStatus::Complete => Color32::LIGHT_GREEN,
                    MatchStatus::Partial => Color32::YELLOW,
                    MatchStatus::Missing => Color32::GRAY,
                };
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [250.0, row_height],
                        egui::Label::new(row.game.as_str()).truncate(),
                    );
                    ui.add_sized(
                        [220.0, row_height],
                        egui::Label::new(row.rom.as_str()).truncate(),
                    );
                    ui.add_sized(
                        [70.0, row_height],
                        egui::Label::new(RichText::new(row.status_text()).color(color)),
                    );
                    ui.add_sized([150.0, row_height], egui::Label::new(row.hash_text()));
                    ui.add_sized(
                        [ui.available_width(), row_height],
                        egui::Label::new(RichText::new(row.source_text()).small().weak())
                            .truncate(),
                    );
                });
            }
        });
}

fn verify_section(ui: &mut egui::Ui, title: &str, entries: &[String]) {
    egui::CollapsingHeader::new(format!("{title} ({})", entries.len())).show(ui, |ui| {
        for entry in entries.iter().take(300) {
            ui.monospace(entry);
        }
        if entries.len() > 300 {
            ui.label(format!("… {} more", entries.len() - 300));
        }
    });
}

fn cache_bytes_label(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (KIB * KIB))
    } else if bytes >= 1024 {
        format!("{:.0} KiB", bytes as f64 / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn sanitize(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' ' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    while out.ends_with([' ', '.']) {
        out.pop();
    }
    if out.is_empty() {
        "dat".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use unko_rom_manager::{hash_bytes, parse_dat_str};

    #[test]
    fn rebuild_runs_on_the_worker_and_verifies() {
        let dir = std::env::temp_dir().join(format!("unko-rm-postverify-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let source = dir.join("source");
        let output = dir.join("out");
        fs::create_dir_all(&source).expect("source");
        let payload = b"rom-manager-post-verify";
        let hashes = hash_bytes(payload);
        let mut file = vec![0u8; 16];
        file[..4].copy_from_slice(b"NES\x1a");
        file[4] = 1;
        file[5] = 1;
        file.extend_from_slice(payload);
        fs::write(source.join("a.nes"), file).expect("rom");
        let dat = parse_dat_str(&format!(
            "<datafile><header><name>Post</name><version>1</version></header>\
             <game name=\"Alpha (USA)\"><rom name=\"a.nes\" size=\"{}\" crc=\"{:08x}\" sha1=\"{}\"/></game>\
             </datafile>",
            payload.len(),
            hashes.crc,
            hashes.sha1_hex()
        ))
        .expect("dat");
        let (scanned, _) = scan_path(&source, &ScanOptions::default(), &mut |_| {}).expect("scan");
        let report = Catalog::new(dat).match_all(scanned);
        let mut manager = RomManager::new(dir.join("saves"), source);
        manager.output = output.clone();
        manager.reports = vec![report];
        manager.start_rebuild(0, &[0]);
        assert!(manager.busy);
        assert_eq!(manager.busy_kind, BusyKind::Rebuild);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while manager.busy && std::time::Instant::now() < deadline {
            manager.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!manager.busy, "worker did not finish: {}", manager.status);
        let view = manager.verify.as_ref().expect("auto verification panel");
        assert!(view.clean, "{}", view.summary);
        assert_eq!(view.found, 1);
        assert!(
            manager.status.contains("verification: CLEAN"),
            "status was {}",
            manager.status
        );
        let progress = manager.rebuild_progress.lock().expect("progress");
        assert_eq!(progress.phase, RebuildPhase::Done);
        assert_eq!(progress.games_done, 1);
        assert!(progress.written > 0);
        assert!(progress.verified_files > 0);
        drop(progress);
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use unko_rom_manager::hash_bytes;

    #[test]
    fn cache_toggle_count_and_clear_are_wired() {
        let dir = std::env::temp_dir().join(format!("unko-rm-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let save_dir = dir.join("saves");
        let cache = save_dir.join("rom-manager").join("scan-cache.tsv");
        let hashes = hash_bytes(b"payload");
        let mut seeded = ScanCache::load(&cache);
        seeded.store("a".into(), hashes.size, None, hashes, None);
        seeded.save().expect("seed cache");

        let mut manager = RomManager::new(save_dir, dir.join("roms"));
        assert_eq!(manager.cache_entries, 1);
        assert!(manager.scan_options().cache.is_some());
        manager.use_cache = false;
        assert!(
            manager.scan_options().cache.is_none(),
            "disabling the cache must leave scan options without a cache"
        );

        manager.clear_scan_cache();
        assert_eq!(manager.cache_entries, 0);
        assert!(!cache.exists(), "clear must remove the cache file");
        assert!(manager.status.contains("cleared"), "{}", manager.status);
        let _ = fs::remove_dir_all(dir);
    }
}
