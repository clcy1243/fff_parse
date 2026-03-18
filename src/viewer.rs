use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use fff_viewer::color::{self, IccProfileInfo, SettingsPreset, TargetColorSpace};
use fff_viewer::flexcolor::{self, EditHistory, ImageCorrection};
use fff_viewer::i18n::{self, Language, Strings};
use fff_viewer::tiff::TiffFile;

/// Maximum pixel dimension for display preview.
/// Larger images are subsampled during decode for speed.
/// 4096 gives good quality for typical screen sizes while being ~25× faster
/// than full-resolution decode of large scanner images.
const DISPLAY_MAX_DIM: u32 = 4096;

// ─── Enums ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum ViewMode {
    Grid,
    Loupe,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum InfoPanel {
    Metadata,
    EditHistory,
    AllTags,
    ColorProfile,
}

// ─── Thumbnail cache entry ──────────────────────────────────────────────────

struct ThumbEntry {
    texture: egui::TextureHandle,
    width: u32,
    height: u32,
}

// ─── Loaded detail for the selected file ────────────────────────────────────

struct LoadedDetail {
    path: PathBuf,
    #[allow(dead_code)]
    tiff: TiffFile,
    metadata: Vec<(String, String)>,
    all_tags: Vec<(String, String, String, String)>,
    edit_history: Option<EditHistory>,
    texture: Option<egui::TextureHandle>,
    embedded_icc: Option<Vec<u8>>,
}

// ─── Export state ───────────────────────────────────────────────────────────

struct ExportState {
    status: ExportStatus,
}

#[derive(Debug, Clone)]
enum ExportStatus {
    Idle,
    Exporting { current: usize, total: usize, current_name: String },
    Done { count: usize, dir: PathBuf },
    Error(String),
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            status: ExportStatus::Idle,
        }
    }
}

// ─── Loading status ─────────────────────────────────────────────────────────

enum LoadingStatus {
    Idle,
    LoadingThumbnails,
    LoadingFile(String),       // file name being loaded
    ApplyingColorProfile,
}

impl Default for LoadingStatus {
    fn default() -> Self { Self::Idle }
}

// ─── Background thread messages ─────────────────────────────────────────────

/// Result of loading a thumbnail on a background thread.
struct ThumbResult {
    path: PathBuf,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// Result of loading a full detail file on a background thread.
/// Textures cannot be created off the main thread, so we send raw image data.
struct DetailResult {
    path: PathBuf,
    tiff: TiffFile,
    metadata: Vec<(String, String)>,
    all_tags: Vec<(String, String, String, String)>,
    edit_history: Option<EditHistory>,
    preview_rgba: Option<(Vec<u8>, u32, u32)>, // (pixels, width, height)
    embedded_icc: Option<Vec<u8>>,
    auto_corrected: bool, // true if embedded correction was auto-applied
}

enum DetailMsg {
    Loaded(DetailResult),
    Error(PathBuf, String),
}

// ─── App state ──────────────────────────────────────────────────────────────

pub struct FffViewerApp {
    // Directory tree
    current_dir: Option<PathBuf>,
    expanded_dirs: HashSet<PathBuf>,

    // File list in current directory
    fff_files: Vec<PathBuf>,

    // Thumbnails cache
    thumbnails: HashMap<PathBuf, ThumbEntry>,
    thumb_rx: mpsc::Receiver<ThumbResult>,
    thumb_tx: mpsc::Sender<ThumbResult>,
    thumb_pending: usize,

    // View state
    view_mode: ViewMode,
    selected_index: Option<usize>,

    // Detail of selected file
    detail: Option<LoadedDetail>,
    detail_rx: mpsc::Receiver<DetailMsg>,
    detail_tx: mpsc::Sender<DetailMsg>,

    // Right panel
    info_panel: InfoPanel,
    tag_filter: String,
    expanded_setting: Option<usize>,

    // Editing state
    export_state: ExportState,

    // Color management
    available_profiles: Vec<IccProfileInfo>,
    available_presets: Vec<SettingsPreset>,
    selected_input_profile: Option<usize>,
    selected_preset: Option<usize>,
    use_embedded_icc: bool,
    use_embedded_correction: bool,
    preset_category_filter: String,
    color_status: Option<String>,
    target_color_space: TargetColorSpace,

    // Loading progress
    loading_status: LoadingStatus,

    // Error
    error_msg: Option<String>,

    // UI toggles
    show_info_panel: bool,

    // Language
    language: Language,
}

// ─── Font loading ───────────────────────────────────────────────────────────

fn setup_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Try loading CJK font from system — prefer fonts with good Latin + CJK coverage
    let cjk_font_paths = [
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        // Windows
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    for font_path in &cjk_font_paths {
        if let Ok(font_data) = std::fs::read(font_path) {
            let mut fd = egui::FontData::from_owned(font_data);
            fd.index = 0;
            fonts.font_data.insert("cjk".to_owned(), fd.into());

            // Put CJK font FIRST so Latin and CJK characters share the same
            // baseline and line metrics — eliminates misalignment between scripts
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "cjk".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.insert(0, "cjk".to_owned());
            }

            ctx.set_fonts(fonts);
            log::info!("Loaded CJK font from: {}", font_path);
            return;
        }
    }

    log::warn!("No CJK font found on system");
}

// ─── App impl ───────────────────────────────────────────────────────────────

impl FffViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        log::info!("Initializing FffViewerApp");
        setup_cjk_fonts(&cc.egui_ctx);

        // Scan for bundled ICC profiles and settings presets
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let profiles_dir = find_resource_dir("profiles", exe_dir.as_deref());
        let settings_dir = find_resource_dir("settings", exe_dir.as_deref());

        let available_profiles = profiles_dir
            .map(|d| color::scan_icc_profiles(&d))
            .unwrap_or_default();
        let available_presets = settings_dir
            .map(|d| color::scan_settings_presets(&d))
            .unwrap_or_default();
        log::info!(
            "Found {} ICC profiles, {} settings presets",
            available_profiles.len(),
            available_presets.len()
        );

        let (thumb_tx, thumb_rx) = mpsc::channel();
        let (detail_tx, detail_rx) = mpsc::channel();

        let mut app = Self {
            current_dir: None,
            expanded_dirs: HashSet::new(),
            fff_files: Vec::new(),
            thumbnails: HashMap::new(),
            thumb_rx,
            thumb_tx,
            thumb_pending: 0,
            view_mode: ViewMode::Grid,
            selected_index: None,
            detail: None,
            detail_rx,
            detail_tx,
            info_panel: InfoPanel::Metadata,
            tag_filter: String::new(),
            expanded_setting: None,
            export_state: ExportState::default(),
            available_profiles,
            available_presets,
            selected_input_profile: None,
            selected_preset: None,
            use_embedded_icc: false,
            use_embedded_correction: false,
            preset_category_filter: String::new(),
            color_status: None,
            target_color_space: TargetColorSpace::default(),
            loading_status: LoadingStatus::Idle,
            error_msg: None,
            show_info_panel: true,
            language: Language::Chinese,
        };

        if let Some(path) = initial_file {
            if let Some(parent) = path.parent() {
                app.set_directory(parent.to_path_buf());
                if let Some(idx) = app.fff_files.iter().position(|p| p == &path) {
                    app.selected_index = Some(idx);
                }
            }
        } else if let Some(home) = dirs_home() {
            app.current_dir = Some(home);
        }

        app
    }

    fn s(&self) -> &'static Strings {
        i18n::strings(self.language)
    }

    fn set_directory(&mut self, dir: PathBuf) {
        log::info!("set_directory: {}", dir.display());
        self.current_dir = Some(dir.clone());
        self.fff_files = scan_fff_files(&dir);
        self.fff_files.sort();
        log::info!("Found {} .fff files", self.fff_files.len());
        self.selected_index = None;
        self.detail = None;
        self.thumbnails.clear();
        self.thumb_pending = self.fff_files.len();
        self.loading_status = if self.fff_files.is_empty() {
            LoadingStatus::Idle
        } else {
            LoadingStatus::LoadingThumbnails
        };

        // Spawn background thread pool to load thumbnails
        let files = self.fff_files.clone();
        let tx = self.thumb_tx.clone();
        std::thread::spawn(move || {
            use rayon::prelude::*;
            files.par_iter().for_each(|path| {
                let result = if let Ok(tiff) = TiffFile::open(path) {
                    if let Some(img) = tiff.decode_thumbnail() {
                        let w = img.width();
                        let h = img.height();
                        let rgba = img.to_rgba8().into_raw();
                        ThumbResult { path: path.clone(), rgba, width: w, height: h }
                    } else {
                        ThumbResult { path: path.clone(), rgba: Vec::new(), width: 0, height: 0 }
                    }
                } else {
                    ThumbResult { path: path.clone(), rgba: Vec::new(), width: 0, height: 0 }
                };
                let _ = tx.send(result);
            });
        });
    }

    fn select_file(&mut self, index: usize, _ctx: &egui::Context) {
        if index >= self.fff_files.len() {
            log::warn!("select_file: index {} out of range ({})", index, self.fff_files.len());
            return;
        }
        self.selected_index = Some(index);
        self.expanded_setting = None;
        self.error_msg = None;
        self.use_embedded_correction = false;
        self.use_embedded_icc = false;
        self.color_status = None;

        let path = self.fff_files[index].clone();
        log::info!("select_file: [{}] {}", index, path.display());

        if let Some(detail) = &self.detail {
            if detail.path == path {
                return;
            }
        }

        // Show loading state immediately
        let file_name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.loading_status = LoadingStatus::LoadingFile(file_name);
        self.detail = None;

        // Spawn background thread for file I/O and parsing
        let tx = self.detail_tx.clone();
        std::thread::spawn(move || {
            match TiffFile::open(&path) {
                Ok(tiff) => {
                    log::info!("Opened file: {} ({} bytes)",
                        path.display(), tiff.raw_data().len());
                    let metadata = tiff.metadata_summary();
                    let all_tags = tiff.all_tags();
                    log::debug!("Parsed {} metadata entries, {} tags", metadata.len(), all_tags.len());
                    let edit_history = EditHistory::parse_from_file(tiff.raw_data());
                    log::debug!("Edit history: {} settings",
                        edit_history.as_ref().map(|h| h.settings.len()).unwrap_or(0));

                    // Extract the embedded correction from edit history (if any)
                    let embedded_correction: Option<ImageCorrection> = edit_history.as_ref().and_then(|h| {
                        if h.settings.is_empty() {
                            None
                        } else {
                            let idx = h.current_index.min(h.settings.len() - 1);
                            Some(h.settings[idx].correction.clone())
                        }
                    });

                    // Decode preview with downsampling for fast display
                    let (preview_rgba, auto_corrected) = if let Some(img) = tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
                        log::info!("Decoded downscaled preview: {}x{} {:?}",
                            img.width(), img.height(), img.color());

                        // Auto-apply embedded correction if available
                        let (processed, corrected) = if let Some(ref correction) = embedded_correction {
                            log::info!("Auto-applying embedded correction: film_type={}",
                                correction.film_type);
                            let result = color::apply_film_processing(&img, correction);
                            (result, true)
                        } else {
                            (img, false)
                        };

                        let processed = convert_16_to_8_for_display(processed);
                        let processed = clamp_image_for_gpu(processed);
                        let rgba = processed.to_rgba8();
                        let w = rgba.width();
                        let h = rgba.height();
                        (Some((rgba.into_raw(), w, h)), corrected)
                    } else {
                        log::warn!("No preview decoded for {}", path.display());
                        (None, false)
                    };

                    let embedded_icc = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
                    log::debug!("Embedded ICC: {} bytes",
                        embedded_icc.as_ref().map(|d| d.len()).unwrap_or(0));

                    let _ = tx.send(DetailMsg::Loaded(DetailResult {
                        path,
                        tiff,
                        metadata,
                        all_tags,
                        edit_history,
                        preview_rgba,
                        embedded_icc,
                        auto_corrected,
                    }));
                }
                Err(e) => {
                    log::error!("Failed to open {}: {}", path.display(), e);
                    let _ = tx.send(DetailMsg::Error(path, e.to_string()));
                }
            }
        });
    }

    /// Poll background thread results for thumbnails and detail files.
    fn poll_background_results(&mut self, ctx: &egui::Context) {
        // Poll thumbnail results
        while let Ok(result) = self.thumb_rx.try_recv() {
            self.thumb_pending = self.thumb_pending.saturating_sub(1);
            if result.width > 0 && result.height > 0 {
                let size = [result.width as usize, result.height as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &result.rgba);
                let tex = ctx.load_texture(
                    format!("thumb_{}", result.path.display()),
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                self.thumbnails.insert(result.path, ThumbEntry {
                    texture: tex,
                    width: result.width,
                    height: result.height,
                });
            }
        }
        if self.thumb_pending == 0 && matches!(self.loading_status, LoadingStatus::LoadingThumbnails) {
            self.loading_status = LoadingStatus::Idle;
        }

        // Poll detail file result
        if let Ok(msg) = self.detail_rx.try_recv() {
            match msg {
                DetailMsg::Loaded(result) => {
                    let texture = if let Some((pixels, w, h)) = result.preview_rgba {
                        let size = [w as usize, h as usize];
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                        Some(ctx.load_texture(
                            "loupe_preview",
                            color_image,
                            egui::TextureOptions::LINEAR,
                        ))
                    } else {
                        None
                    };

                    self.detail = Some(LoadedDetail {
                        path: result.path,
                        tiff: result.tiff,
                        metadata: result.metadata,
                        all_tags: result.all_tags,
                        edit_history: result.edit_history,
                        texture,
                        embedded_icc: result.embedded_icc,
                    });
                    // Set embedded correction flag if auto-applied during load
                    if result.auto_corrected {
                        self.use_embedded_correction = true;
                    }
                    self.loading_status = LoadingStatus::Idle;
                }
                DetailMsg::Error(path, e) => {
                    self.error_msg = Some(format!("{}: {}", self.s().failed_to_open, e));
                    self.detail = None;
                    self.loading_status = LoadingStatus::Idle;
                    log::error!("Background load failed for {}: {}", path.display(), e);
                }
            }
        }

        // Request repaint while background work is in progress
        if self.thumb_pending > 0
            || matches!(self.loading_status, LoadingStatus::LoadingFile(_))
        {
            ctx.request_repaint();
        }
    }

    fn open_directory_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.set_directory(dir);
        }
    }
}

// ─── eframe::App ────────────────────────────────────────────────────────────

impl eframe::App for FffViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background_results(ctx);

        // Handle drag-and-drop
        let dropped_path = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(path) = dropped_path {
            log::info!("Drag-and-drop: {}", path.display());
            if path.is_dir() {
                self.set_directory(path);
            } else if let Some(parent) = path.parent() {
                self.set_directory(parent.to_path_buf());
                if let Some(idx) = self.fff_files.iter().position(|p| p == &path) {
                    self.select_file(idx, ctx);
                }
            }
        }

        let s = self.s();

        // ── Top toolbar ─────────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(s.open_folder).clicked() {
                    self.open_directory_dialog();
                }

                ui.separator();

                ui.selectable_value(&mut self.view_mode, ViewMode::Grid, s.grid);
                ui.selectable_value(&mut self.view_mode, ViewMode::Loupe, s.loupe);

                ui.separator();

                ui.toggle_value(&mut self.show_info_panel, s.info);

                if self.show_info_panel {
                    ui.separator();
                    ui.selectable_value(&mut self.info_panel, InfoPanel::Metadata, s.metadata);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::EditHistory, s.history);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::AllTags, s.tags);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::ColorProfile, s.color_profile);
                }

                ui.separator();

                // Export buttons
                let has_selection = self.selected_index.is_some() && self.detail.is_some();
                let has_files = !self.fff_files.is_empty();

                if ui.add_enabled(has_selection, egui::Button::new(s.export_current)).clicked() {
                    self.export_current_file();
                }
                if ui.add_enabled(has_files, egui::Button::new(s.export_all)).clicked() {
                    self.export_all_files();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Language selector
                    egui::ComboBox::from_id_salt("lang_selector")
                        .selected_text(format!("{} {}", s.language, self.language.label()))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.language,
                                Language::English,
                                Language::English.label(),
                            );
                            ui.selectable_value(
                                &mut self.language,
                                Language::Chinese,
                                Language::Chinese.label(),
                            );
                        });

                    ui.separator();

                    if let Some(dir) = &self.current_dir {
                        let s2 = i18n::strings(self.language);
                        let label = format!(
                            "{} — {} {}",
                            dir.display(),
                            self.fff_files.len(),
                            s2.files_count
                        );
                        ui.label(
                            egui::RichText::new(label).small().color(ui.visuals().weak_text_color()),
                        );
                    }
                });
            });
        });

        // ── Progress bar (shown when loading) ───────────────────────────
        {
            let total = self.fff_files.len();
            let loaded = self.thumbnails.len();
            let thumbs_loading = self.thumb_pending > 0 && total > 0;

            let show_bar = thumbs_loading
                || matches!(self.loading_status,
                    LoadingStatus::LoadingFile(_)
                    | LoadingStatus::ApplyingColorProfile
                );

            if show_bar {
                egui::TopBottomPanel::top("progress_bar").show(ctx, |ui| {
                    match &self.loading_status {
                        LoadingStatus::LoadingFile(name) => {
                            ui.add(
                                egui::ProgressBar::new(0.0)
                                    .animate(true)
                                    .text(format!("⏳ {} {}…", self.s().loading_file, name)),
                            );
                        }
                        LoadingStatus::ApplyingColorProfile => {
                            ui.add(
                                egui::ProgressBar::new(0.0)
                                    .animate(true)
                                    .text("⏳ Applying color profile…"),
                            );
                        }
                        _ if thumbs_loading => {
                            let progress = loaded as f32 / total as f32;
                            let s2 = i18n::strings(self.language);
                            let text = format!(
                                "📷 {} {}/{}",
                                s2.loading_thumbnails, loaded, total
                            );
                            ui.add(egui::ProgressBar::new(progress).text(text));
                        }
                        _ => {}
                    }
                });
            }
        }

        // ── Left panel: directory tree ──────────────────────────────────
        let folders_label = i18n::strings(self.language).folders.to_string();
        egui::SidePanel::left("dir_tree_panel")
            .default_width(220.0)
            .min_width(160.0)
            .show(ctx, |ui| {
                ui.heading(&folders_label);
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        self.render_dir_tree(ui);
                    });
            });

        // ── Right panel: info (optional) ────────────────────────────────
        if self.show_info_panel {
            egui::SidePanel::right("info_panel")
                .default_width(340.0)
                .min_width(260.0)
                .show(ctx, |ui| match self.info_panel {
                    InfoPanel::Metadata => self.render_metadata_panel(ui),
                    InfoPanel::EditHistory => self.render_edit_history_panel(ui),
                    InfoPanel::AllTags => self.render_all_tags_panel(ui),
                    InfoPanel::ColorProfile => self.render_color_profile_panel(ui, ctx),
                });
        }

        // ── Bottom panel: export status ─────────────────────────────────
        let export_status = self.export_state.status.clone();
        match export_status {
            ExportStatus::Exporting { current, total, ref current_name } => {
                let s = i18n::strings(self.language);
                let label_text = format!("{} {}/{} — {}", s.exporting, current, total, current_name);
                let progress = current as f32 / total as f32;
                egui::TopBottomPanel::bottom("export_status").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(label_text)
                                .color(ui.visuals().hyperlink_color),
                        );
                    });
                    ui.add(egui::ProgressBar::new(progress).show_percentage());
                });
                ctx.request_repaint();
            }
            ExportStatus::Done { count, ref dir } => {
                let s = i18n::strings(self.language);
                let label_text = format!("✅ {} {} → {}", s.export_done, count, dir.display());
                egui::TopBottomPanel::bottom("export_status").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(label_text)
                                .color(if ui.visuals().dark_mode {
                                    egui::Color32::from_rgb(100, 255, 100)
                                } else {
                                    egui::Color32::from_rgb(0, 140, 0)
                                }),
                        );
                        if ui.small_button("✕").clicked() {
                            self.export_state.status = ExportStatus::Idle;
                        }
                    });
                });
            }
            ExportStatus::Error(ref msg) => {
                let label_text = format!("❌ {}", msg);
                egui::TopBottomPanel::bottom("export_status").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(label_text)
                                .color(if ui.visuals().dark_mode {
                                    egui::Color32::from_rgb(255, 100, 100)
                                } else {
                                    egui::Color32::from_rgb(200, 0, 0)
                                }),
                        );
                        if ui.small_button("✕").clicked() {
                            self.export_state.status = ExportStatus::Idle;
                        }
                    });
                });
            }
            ExportStatus::Idle => {}
        }

        // ── Central panel: grid or loupe ────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.fff_files.is_empty() {
                self.render_empty_state(ui);
            } else {
                match self.view_mode {
                    ViewMode::Grid => self.render_grid_view(ui, ctx),
                    ViewMode::Loupe => self.render_loupe_view(ui, ctx),
                }
            }
        });
    }
}

// ─── Directory tree ─────────────────────────────────────────────────────────

impl FffViewerApp {
    fn render_dir_tree(&mut self, ui: &mut egui::Ui) {
        let roots = get_root_dirs();
        for root in &roots {
            self.render_dir_node(ui, root, 0);
        }
    }

    fn render_dir_node(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize) {
        let is_expanded = self.expanded_dirs.contains(path);
        let is_selected = self.current_dir.as_deref() == Some(path);

        let raw_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let name = shorten_dir_name(&raw_name);

        let indent = depth as f32 * 16.0;
        ui.horizontal(|ui| {
            ui.add_space(indent);

            let arrow = if is_expanded { "▼" } else { "▶" };
            if ui
                .add(
                    egui::Label::new(egui::RichText::new(arrow).small())
                        .sense(egui::Sense::click()),
                )
                .clicked()
            {
                if is_expanded {
                    self.expanded_dirs.remove(path);
                } else {
                    self.expanded_dirs.insert(path.to_path_buf());
                }
            }

            let text = egui::RichText::new(format!("📁 {}", name)).color(if is_selected {
                ui.visuals().hyperlink_color
            } else {
                ui.visuals().text_color()
            });

            let label = egui::Label::new(if is_selected { text.strong() } else { text })
                .sense(egui::Sense::click())
                .truncate();

            let resp = ui.add(label);

            // Show full name on hover if it was shortened
            if name != raw_name {
                resp.clone().on_hover_text(&raw_name);
            }

            if resp.clicked() {
                self.set_directory(path.to_path_buf());
            }
            if resp.double_clicked() {
                if self.expanded_dirs.contains(path) {
                    self.expanded_dirs.remove(path);
                } else {
                    self.expanded_dirs.insert(path.to_path_buf());
                }
            }
        });

        if is_expanded {
            if let Ok(entries) = std::fs::read_dir(path) {
                let mut dirs: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                            && !e.file_name().to_string_lossy().starts_with('.')
                    })
                    .map(|e| e.path())
                    .collect();
                dirs.sort();
                for child in &dirs {
                    self.render_dir_node(ui, child, depth + 1);
                }
            }
        }
    }
}

// ─── Grid View ──────────────────────────────────────────────────────────────

impl FffViewerApp {
    fn render_grid_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let thumb_size = 180.0_f32;
        let spacing = 8.0_f32;
        let available_width = ui.available_width();
        let cols = ((available_width + spacing) / (thumb_size + spacing)).max(1.0) as usize;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let files = self.fff_files.clone();
                let selected = self.selected_index;

                let mut new_selection: Option<usize> = None;
                let mut double_clicked: Option<usize> = None;

                egui::Grid::new("thumb_grid")
                    .spacing([spacing, spacing])
                    .show(ui, |ui| {
                        for (idx, path) in files.iter().enumerate() {
                            let is_selected = selected == Some(idx);

                            let frame = egui::Frame::NONE
                                .fill(if is_selected {
                                    if ui.visuals().dark_mode {
                                        egui::Color32::from_rgb(40, 60, 90)
                                    } else {
                                        egui::Color32::from_rgb(200, 220, 245)
                                    }
                                } else {
                                    if ui.visuals().dark_mode {
                                        egui::Color32::from_rgb(30, 30, 30)
                                    } else {
                                        egui::Color32::from_rgb(235, 235, 235)
                                    }
                                })
                                .inner_margin(4.0)
                                .corner_radius(4.0);

                            let resp = frame.show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(thumb_size);
                                    // Fixed-height square area for thumbnail
                                    let cell_h = thumb_size;

                                    if let Some(entry) = self.thumbnails.get(path) {
                                        let aspect =
                                            entry.width as f32 / entry.height.max(1) as f32;
                                        // Fit image within square, preserving aspect ratio
                                        let (display_w, display_h) = if aspect >= 1.0 {
                                            (thumb_size, thumb_size / aspect)
                                        } else {
                                            (thumb_size * aspect, thumb_size)
                                        };
                                        // Vertical centering: pad top
                                        let pad_y = (cell_h - display_h) / 2.0;
                                        if pad_y > 1.0 {
                                            ui.add_space(pad_y);
                                        }
                                        let sized = egui::load::SizedTexture::new(
                                            entry.texture.id(),
                                            egui::vec2(display_w, display_h),
                                        );
                                        ui.with_layout(
                                            egui::Layout::top_down(egui::Align::Center),
                                            |ui| { ui.image(sized); },
                                        );
                                        let pad_bottom = cell_h - display_h - pad_y;
                                        if pad_bottom > 1.0 {
                                            ui.add_space(pad_bottom);
                                        }
                                    } else {
                                        ui.allocate_space(egui::vec2(thumb_size, cell_h));
                                        ui.spinner();
                                    }

                                    let name = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(&name)
                                                    .small()
                                                    .color(ui.visuals().weak_text_color()),
                                            );
                                        },
                                    );
                                });
                            });

                            let resp = resp.response.interact(egui::Sense::click());
                            if resp.clicked() {
                                new_selection = Some(idx);
                            }
                            if resp.double_clicked() {
                                double_clicked = Some(idx);
                            }

                            if (idx + 1) % cols == 0 {
                                ui.end_row();
                            }
                        }
                    });

                if let Some(idx) = double_clicked.or(new_selection) {
                    self.select_file(idx, ctx);
                }
                if double_clicked.is_some() {
                    self.view_mode = ViewMode::Loupe;
                }
            });
    }
}

// ─── Loupe View ─────────────────────────────────────────────────────────────

impl FffViewerApp {
    fn render_loupe_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.selected_index.is_none() && !self.fff_files.is_empty() {
            self.select_file(0, ctx);
        }

        let filmstrip_height = 100.0_f32;

        egui::TopBottomPanel::bottom("filmstrip")
            .exact_height(filmstrip_height)
            .show_inside(ui, |ui| {
                self.render_filmstrip(ui, ctx);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(20, 20, 20)
            } else {
                egui::Color32::from_rgb(240, 240, 240)
            }))
            .show_inside(ui, |ui| {
                self.render_loupe_image(ui, ctx);
            });
    }

    fn render_loupe_image(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(ref error) = self.error_msg {
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::RED, error);
            });
            return;
        }

        if let Some(detail) = &self.detail {
            if let Some(texture) = &detail.texture {
                let available = ui.available_size();
                let tex_size = texture.size_vec2();
                let scale = (available.x / tex_size.x)
                    .min(available.y / tex_size.y)
                    .min(1.0);
                let display_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);

                ui.centered_and_justified(|ui| {
                    ui.image(egui::load::SizedTexture::new(texture.id(), display_size));
                });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(self.s().no_preview);
                });
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(self.s().select_image);
            });
        }

        // Keyboard navigation
        let (left, right) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if left || right {
            let count = self.fff_files.len();
            if count > 0 {
                let cur = self.selected_index.unwrap_or(0);
                let next = if right {
                    (cur + 1) % count
                } else {
                    (cur + count - 1) % count
                };
                self.select_file(next, ctx);
            }
        }
    }

    fn render_filmstrip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let strip_h = ui.available_height() - 4.0;
        let _thumb_w = (strip_h * 0.75).max(60.0);

        let files = self.fff_files.clone();
        let selected = self.selected_index;

        egui::ScrollArea::horizontal()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut new_sel: Option<usize> = None;

                    for (idx, path) in files.iter().enumerate() {
                        let is_sel = selected == Some(idx);

                        let dark = ui.visuals().dark_mode;
                        let frame = egui::Frame::NONE
                            .fill(if is_sel {
                                if dark {
                                    egui::Color32::from_rgb(50, 80, 120)
                                } else {
                                    egui::Color32::from_rgb(190, 215, 245)
                                }
                            } else {
                                if dark {
                                    egui::Color32::from_rgb(35, 35, 35)
                                } else {
                                    egui::Color32::from_rgb(230, 230, 230)
                                }
                            })
                            .stroke(if is_sel {
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 180, 255))
                            } else {
                                egui::Stroke::NONE
                            })
                            .inner_margin(2.0)
                            .corner_radius(3.0);

                        let resp = frame
                            .show(ui, |ui| {
                                let dh = strip_h - 4.0;
                                if let Some(entry) = self.thumbnails.get(path) {
                                    let aspect =
                                        entry.width as f32 / entry.height.max(1) as f32;
                                    let dw = dh * aspect;
                                    ui.image(egui::load::SizedTexture::new(
                                        entry.texture.id(),
                                        egui::vec2(dw, dh),
                                    ));
                                } else {
                                    // Placeholder: assume portrait ~0.26 ratio for film scans
                                    ui.allocate_space(egui::vec2(dh * 0.3, dh));
                                }
                            })
                            .response
                            .interact(egui::Sense::click());

                        if resp.clicked() {
                            new_sel = Some(idx);
                        }
                    }

                    if let Some(idx) = new_sel {
                        self.select_file(idx, ctx);
                    }
                });
            });
    }
}

// ─── Empty state ────────────────────────────────────────────────────────────

impl FffViewerApp {
    fn render_empty_state(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.heading(s.app_title);
                ui.label(s.app_subtitle);
                ui.add_space(20.0);
                if self.current_dir.is_some() {
                    ui.label(s.no_fff_files);
                } else {
                    ui.label(s.select_folder_hint);
                }
                ui.add_space(10.0);
                if ui
                    .button(egui::RichText::new(s.open_folder).size(16.0))
                    .clicked()
                {
                    self.open_directory_dialog();
                }
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(s.drag_drop_hint)
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
    }
}

// ─── Right info panels ─────────────────────────────────────────────────────

impl FffViewerApp {
    fn render_metadata_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.metadata_heading);
        ui.separator();

        if let Some(detail) = &self.detail {
            ui.label(
                egui::RichText::new(
                    detail
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
                .strong()
                .color(ui.visuals().hyperlink_color),
            );
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    egui::Grid::new("metadata_grid")
                        .striped(true)
                        .num_columns(2)
                        .min_col_width(100.0)
                        .show(ui, |ui| {
                            for (key, value) in &detail.metadata {
                                ui.strong(key);
                                ui.label(egui::RichText::new(value).monospace());
                                ui.end_row();
                            }
                        });
                });
        } else {
            ui.label(s.select_file_metadata);
        }
    }

    fn render_edit_history_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.edit_history_heading);
        ui.separator();

        let history = match &self.detail {
            Some(detail) => match &detail.edit_history {
                Some(h) => h.clone(),
                None => {
                    ui.label(s.no_edit_history);
                    return;
                }
            },
            None => {
                ui.label(s.select_file_history);
                return;
            }
        };

        let s = self.s();
        ui.label(
            egui::RichText::new(format!(
                "{} {} #{}",
                history.settings.len(),
                s.settings_count,
                history.current_index
            ))
            .small()
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(4.0);

        let lang = self.language;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (idx, setting) in history.settings.iter().enumerate() {
                    let is_current = idx == history.current_index;
                    let is_expanded = self.expanded_setting == Some(idx);

                    let header_text = format!(
                        "{}{}  \"{}\"",
                        if is_current { "▶ " } else { "   " },
                        idx,
                        setting.name
                    );

                    let header = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&header_text)
                                .strong()
                                .size(14.0)
                                .color(if is_current {
                                    ui.visuals().hyperlink_color
                                } else {
                                    ui.visuals().text_color()
                                }),
                        )
                        .sense(egui::Sense::click()),
                    );

                    if header.clicked() {
                        self.expanded_setting = if is_expanded { None } else { Some(idx) };
                    }

                    let s = i18n::strings(lang);
                    ui.indent(format!("setting_basic_{}", idx), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{}: {}", s.created, setting.created))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        if setting.modified.is_valid() {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}: {}",
                                    s.modified, setting.modified
                                ))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                        }

                        let c = &setting.correction;
                        ui.horizontal_wrapped(|ui| {
                            Self::param_chip(ui, "γ", &format!("{}", c.gamma), c.gamma != 2.0);
                            Self::param_chip(ui, "EV", &format!("{}", c.ev), c.ev != 1.0);
                            Self::param_chip(
                                ui,
                                s.contrast,
                                &format!("{}", c.contrast),
                                c.contrast != 0,
                            );
                            Self::param_chip(
                                ui,
                                s.brightness,
                                &format!("{}", c.brightness),
                                c.brightness != 0,
                            );
                            Self::param_chip(
                                ui,
                                s.saturation,
                                &format!("{}", c.saturation),
                                c.saturation != 0,
                            );
                        });
                    });

                    if is_expanded {
                        ui.indent(format!("setting_detail_{}", idx), |ui| {
                            ui.add_space(4.0);
                            Self::render_correction_details(ui, &setting.correction, lang);
                        });
                    }

                    ui.add_space(8.0);
                    if idx + 1 < history.settings.len() {
                        ui.separator();
                    }
                }
            });
    }

    fn render_all_tags_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.all_tags_heading);
        ui.horizontal(|ui| {
            ui.label(s.filter);
            ui.text_edit_singleline(&mut self.tag_filter);
        });
        ui.separator();

        let Some(detail) = &self.detail else {
            ui.label(s.select_file_tags);
            return;
        };

        let s = self.s();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Grid::new("all_tags_grid")
                    .striped(true)
                    .num_columns(4)
                    .min_col_width(40.0)
                    .show(ui, |ui| {
                        ui.strong(s.ifd_header);
                        ui.strong(s.tag_header);
                        ui.strong(s.name_header);
                        ui.strong(s.value_header);
                        ui.end_row();

                        let filter = self.tag_filter.to_lowercase();
                        for (ifd_name, tag_hex, tag_name, value) in &detail.all_tags {
                            if !filter.is_empty()
                                && !tag_name.to_lowercase().contains(&filter)
                                && !tag_hex.to_lowercase().contains(&filter)
                                && !ifd_name.to_lowercase().contains(&filter)
                                && !value.to_lowercase().contains(&filter)
                            {
                                continue;
                            }

                            ui.label(
                                egui::RichText::new(ifd_name)
                                    .small()
                                    .color(ui.visuals().hyperlink_color),
                            );
                            ui.label(egui::RichText::new(tag_hex).small().monospace());
                            ui.label(tag_name);

                            let display_value = if value.len() > 60 {
                                format!("{}...", &value[..57])
                            } else {
                                value.clone()
                            };
                            ui.label(egui::RichText::new(display_value).small().monospace())
                                .on_hover_text(value);

                            ui.end_row();
                        }
                    });
            });
    }

    // ── Color Profile Panel ─────────────────────────────────────────

    fn render_color_profile_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = self.s();
        ui.heading(s.color_profile);
        ui.separator();

        // ── Input Profile ──
        ui.strong(s.input_profile);
        ui.add_space(2.0);

        let profile_names: Vec<String> = self
            .available_profiles
            .iter()
            .map(|p| format!("{} ({})", p.name, p.color_space))
            .collect();

        let current_label = self
            .selected_input_profile
            .and_then(|i| profile_names.get(i))
            .cloned()
            .unwrap_or_else(|| "—".to_string());

        egui::ComboBox::from_id_salt("input_profile_combo")
            .selected_text(&current_label)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut self.selected_input_profile, None, "— None —")
                    .clicked()
                {
                    self.color_status = None;
                }
                for (i, name) in profile_names.iter().enumerate() {
                    let cs = &self.available_profiles[i].color_space;
                    let is_rgb = cs == "RGB" || cs == "RGB ";
                    // Only allow RGB profiles as input (CMYK/GRAY incompatible with RGB images)
                    ui.add_enabled_ui(is_rgb, |ui| {
                        let label = if is_rgb {
                            name.clone()
                        } else {
                            format!("{} ⛔", name)
                        };
                        if ui
                            .selectable_value(
                                &mut self.selected_input_profile,
                                Some(i),
                                label,
                            )
                            .clicked()
                        {
                            self.color_status = None;
                        }
                    });
                }
            });

        ui.add_space(8.0);

        // ── Use Embedded ICC (only shown when file has embedded ICC) ──
        let has_embedded = self
            .detail
            .as_ref()
            .and_then(|d| d.embedded_icc.as_ref())
            .map(|d| !d.is_empty())
            .unwrap_or(false);

        if has_embedded {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.use_embedded_icc, s.use_embedded_icc);
            });
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Target Color Space ──
        ui.strong(s.target_color_space);
        ui.add_space(2.0);

        let current_target_label = self.target_color_space.label();
        egui::ComboBox::from_id_salt("target_colorspace_combo")
            .selected_text(current_target_label)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                for &space in TargetColorSpace::ALL {
                    if ui
                        .selectable_value(&mut self.target_color_space, space, space.label())
                        .clicked()
                    {
                        self.color_status = None;
                    }
                }
            });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Settings Preset ──
        ui.strong(s.settings_preset);
        ui.add_space(2.0);

        // Embedded correction option (only shown if file has edit history)
        let has_embedded_correction = self
            .detail
            .as_ref()
            .and_then(|d| d.edit_history.as_ref())
            .map(|h| !h.settings.is_empty())
            .unwrap_or(false);

        if has_embedded_correction {
            let mut use_embedded = self.use_embedded_correction;
            if ui
                .checkbox(&mut use_embedded, s.use_embedded_correction)
                .changed()
            {
                self.use_embedded_correction = use_embedded;
                if use_embedded {
                    // Deselect preset when using embedded correction
                    self.selected_preset = None;
                }
                self.color_status = None;
            }

            // Show the embedded correction name
            if use_embedded {
                if let Some(ref detail) = self.detail {
                    if let Some(ref history) = detail.edit_history {
                        let idx = history.current_index.min(history.settings.len().saturating_sub(1));
                        let setting = &history.settings[idx];
                        let label = if setting.name.is_empty() {
                            format!("#{}", idx)
                        } else {
                            setting.name.clone()
                        };
                        ui.label(
                            egui::RichText::new(format!("  📎 {}", label))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                }
            }
            ui.add_space(4.0);
        }

        // Category filter (disabled when using embedded correction)
        let preset_enabled = !self.use_embedded_correction;
        let categories: Vec<String> = {
            let mut cats: Vec<String> = self
                .available_presets
                .iter()
                .map(|p| p.category.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            cats.sort();
            cats
        };

        if !categories.is_empty() {
            ui.add_enabled_ui(preset_enabled, |ui| {
                egui::ComboBox::from_id_salt("preset_category_combo")
                    .selected_text(if self.preset_category_filter.is_empty() {
                        s.category_all
                    } else {
                        &self.preset_category_filter
                    })
                    .width(ui.available_width() - 16.0)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.preset_category_filter.is_empty(), s.category_all)
                            .clicked()
                        {
                            self.preset_category_filter.clear();
                        }
                        for cat in &categories {
                            let label = if cat.is_empty() { "Standard" } else { cat.as_str() };
                            if ui
                                .selectable_label(self.preset_category_filter == *cat, label)
                                .clicked()
                            {
                                self.preset_category_filter = cat.clone();
                            }
                        }
                    });
            });

            ui.add_space(4.0);
        }

        // Filtered presets
        let filtered_presets: Vec<(usize, &SettingsPreset)> = self
            .available_presets
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                self.preset_category_filter.is_empty()
                    || p.category == self.preset_category_filter
            })
            .collect();

        let preset_label = self
            .selected_preset
            .and_then(|i| self.available_presets.get(i))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "—".to_string());

        ui.add_enabled_ui(preset_enabled, |ui| {
            egui::ComboBox::from_id_salt("preset_combo")
                .selected_text(&preset_label)
                .width(ui.available_width() - 16.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut self.selected_preset, None, "— None —")
                        .clicked()
                    {
                        self.use_embedded_correction = false;
                        self.color_status = None;
                    }
                    for (global_idx, preset) in &filtered_presets {
                        let label = if preset.category.is_empty() {
                            preset.name.clone()
                        } else {
                            format!("{}/{}", preset.category, preset.name)
                        };
                        if ui
                            .selectable_value(
                                &mut self.selected_preset,
                                Some(*global_idx),
                                &label,
                            )
                            .clicked()
                        {
                            self.use_embedded_correction = false;
                            self.color_status = None;
                        }
                    }
                });
        });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Apply / Reset buttons ──
        ui.horizontal(|ui| {
            let can_apply = self.detail.is_some()
                && (self.selected_input_profile.is_some()
                    || self.use_embedded_icc
                    || self.selected_preset.is_some()
                    || self.use_embedded_correction);

            if ui
                .add_enabled(can_apply, egui::Button::new(s.apply_profile))
                .clicked()
            {
                self.loading_status = LoadingStatus::ApplyingColorProfile;
                self.apply_color_profile(ctx);
                self.loading_status = LoadingStatus::Idle;
            }

            if ui.button(s.reset_profile).clicked() {
                self.reset_color_profile(ctx);
            }
        });

        // Status message
        if let Some(status) = &self.color_status {
            ui.add_space(8.0);
            let is_error = status.starts_with('❌');
            let color = if is_error {
                if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(255, 100, 100)
                } else {
                    egui::Color32::from_rgb(200, 0, 0)
                }
            } else {
                if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(100, 255, 100)
                } else {
                    egui::Color32::from_rgb(0, 140, 0)
                }
            };
            ui.label(egui::RichText::new(status).color(color));
        }

        // ── Show preset details if selected ──
        if let Some(idx) = self.selected_preset {
            if let Some(preset) = self.available_presets.get(idx) {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(4.0);
                ui.strong(
                    egui::RichText::new(format!("📋 {}", preset.name))
                        .color(ui.visuals().hyperlink_color),
                );

                // Parse and show preset details
                if let Ok(xml_data) = std::fs::read_to_string(&preset.path) {
                    if let Some(correction) =
                        flexcolor::parse_settings_xml(&xml_data)
                    {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                Self::render_correction_details(
                                    ui,
                                    &correction,
                                    self.language,
                                );
                            });
                    }
                }
            }
        }
    }

    fn apply_color_profile(&mut self, ctx: &egui::Context) {
        let s = self.s();

        // Determine input ICC data
        let input_icc = if self.use_embedded_icc {
            self.detail
                .as_ref()
                .and_then(|d| d.embedded_icc.clone())
        } else if let Some(idx) = self.selected_input_profile {
            self.available_profiles
                .get(idx)
                .and_then(|p| std::fs::read(&p.path).ok())
        } else {
            None
        };

        // Load preset correction: either from embedded edit history or from selected preset file
        let preset_correction = if self.use_embedded_correction {
            self.detail.as_ref().and_then(|d| {
                d.edit_history.as_ref().and_then(|h| {
                    if h.settings.is_empty() {
                        None
                    } else {
                        let idx = h.current_index.min(h.settings.len() - 1);
                        Some(h.settings[idx].correction.clone())
                    }
                })
            })
        } else {
            self.selected_preset.and_then(|idx| {
                self.available_presets.get(idx).and_then(|p| {
                    std::fs::read_to_string(&p.path)
                        .ok()
                        .and_then(|xml| flexcolor::parse_settings_xml(&xml))
                })
            })
        };

        // Need either ICC or preset (or both)
        if input_icc.is_none() && preset_correction.is_none() {
            self.color_status = Some(format!("❌ {}: no ICC or preset", s.profile_error));
            return;
        }

        let Some(detail) = &self.detail else {
            return;
        };

        // Re-decode the preview image from the TiffFile (downscaled for display)
        let preview_img = match detail.tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
            Some(img) => img,
            None => {
                self.color_status = Some(format!("❌ {}: no preview", s.profile_error));
                return;
            }
        };

        let mut result = preview_img;

        // Step 1: Apply film processing FIRST (negative inversion + levels).
        // This must happen before ICC because the scanner ICC profile expects
        // positive/scene-referred data, not raw negative scan data with orange mask.
        if let Some(ref correction) = preset_correction {
            let film_type = correction.film_type;
            log::info!(
                "Applying film processing: FilmType={} ({}), Shadow={:?}, Highlight={:?}, Gray={:?}, \
                 RemoveCastHighlight={}, RemoveCastShadow={}, Gamma={}",
                film_type,
                flexcolor::film_type_name(film_type),
                correction.shadow,
                correction.highlight,
                correction.gray,
                correction.remove_cast_highlight,
                correction.remove_cast_shadow,
                correction.gamma,
            );
            result = color::apply_film_processing(&result, correction);
        }

        // Step 2: Apply ICC color space transform (scanner RGB → output RGB).
        // Now operating on positive/corrected data after film processing.
        if let Some(icc_data) = &input_icc {
            log::info!(
                "Applying ICC transform: input={} bytes, image={}x{}",
                icc_data.len(),
                result.width(),
                result.height()
            );
            match color::apply_icc_transform(&result, icc_data, self.target_color_space) {
                Ok(transformed) => {
                    result = transformed;
                }
                Err(e) => {
                    log::error!("ICC transform failed: {}", e);
                    self.color_status = Some(format!("❌ {}: {}", s.profile_error, e));
                    return;
                }
            }
        }

        // Step 3: Convert to display-ready 8-bit, clamp for GPU, upload texture
        let result = convert_16_to_8_for_display(result);
        let result = clamp_image_for_gpu(result);
        let rgba = result.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let pixels = rgba.into_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
        let tex = ctx.load_texture(
            "loupe_preview_icc",
            color_image,
            egui::TextureOptions::LINEAR,
        );

        if let Some(detail) = &mut self.detail {
            detail.texture = Some(tex);
        }

        let status_parts: Vec<&str> = [
            input_icc.as_ref().map(|_| "ICC"),
            preset_correction.as_ref().map(|c| {
                if c.film_type == 1 { "Neg→Pos" }
                else if c.film_type == 2 { "B&W Neg→Pos" }
                else { "Levels" }
            }),
        ]
        .into_iter()
        .flatten()
        .collect();

        self.color_status = Some(format!("✅ {} ({})", s.profile_applied, status_parts.join(" + ")));
        log::info!("Color profile applied: {}", status_parts.join(" + "));
    }

    fn reset_color_profile(&mut self, ctx: &egui::Context) {
        self.selected_input_profile = None;
        self.selected_preset = None;
        self.use_embedded_icc = false;
        self.color_status = None;

        // Re-decode preview with downscaling, auto-apply embedded correction if available
        if let Some(detail) = &mut self.detail {
            if let Some(img) = detail.tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
                // Auto-apply embedded correction (same as initial load)
                let (processed, corrected) = if let Some(ref eh) = detail.edit_history {
                    if !eh.settings.is_empty() {
                        let idx = eh.current_index.min(eh.settings.len() - 1);
                        let correction = &eh.settings[idx].correction;
                        log::info!("Reset: re-applying embedded correction");
                        (color::apply_film_processing(&img, correction), true)
                    } else {
                        (img, false)
                    }
                } else {
                    (img, false)
                };
                self.use_embedded_correction = corrected;

                let processed = convert_16_to_8_for_display(processed);
                let processed = clamp_image_for_gpu(processed);
                let rgba = processed.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let pixels = rgba.into_raw();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                detail.texture = Some(ctx.load_texture(
                    "loupe_preview",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        log::info!("Color profile reset");
    }

    // ── Export ────────────────────────────────────────────────────────

    fn export_current_file(&mut self) {
        let Some(detail) = &self.detail else { return };
        let src_path = detail.path.clone();
        log::info!("Export single: {}", src_path.display());
        let default_name = src_path
            .file_stem()
            .map(|n| format!("{}.tiff", n.to_string_lossy()))
            .unwrap_or_else(|| "export.tiff".to_string());

        if let Some(save_path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("TIFF", &["tiff", "tif"])
            .save_file()
        {
            log::info!("Exporting to: {}", save_path.display());
            match Self::export_fff_to_tiff(&src_path, &save_path) {
                Ok(()) => {
                    log::info!("Export OK: {}", save_path.display());
                    self.export_state.status = ExportStatus::Done {
                        count: 1,
                        dir: save_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                    };
                }
                Err(e) => {
                    log::error!("Export failed: {} — {}", src_path.display(), e);
                    self.export_state.status = ExportStatus::Error(format!("{}: {}", src_path.display(), e));
                }
            }
        }
    }

    fn export_all_files(&mut self) {
        if self.fff_files.is_empty() {
            return;
        }

        let Some(out_dir) = rfd::FileDialog::new()
            .set_title("Select output directory")
            .pick_folder()
        else {
            return;
        };

        let files = self.fff_files.clone();
        let total = files.len();
        log::info!("Export all: {} files → {}", total, out_dir.display());

        for (i, src_path) in files.iter().enumerate() {
            let name = src_path
                .file_stem()
                .map(|n| format!("{}.tiff", n.to_string_lossy()))
                .unwrap_or_else(|| format!("export_{}.tiff", i));

            self.export_state.status = ExportStatus::Exporting {
                current: i + 1,
                total,
                current_name: name.clone(),
            };

            let dst = out_dir.join(&name);
            log::info!("Batch export [{}/{}]: {} → {}", i + 1, total, src_path.display(), dst.display());
            if let Err(e) = Self::export_fff_to_tiff(src_path, &dst) {
                log::error!("Batch export failed at [{}/{}]: {}", i + 1, total, e);
                self.export_state.status = ExportStatus::Error(format!("{}: {}", src_path.display(), e));
                return;
            }
        }

        log::info!("Batch export complete: {} files", total);
        self.export_state.status = ExportStatus::Done {
            count: total,
            dir: out_dir,
        };
    }

    /// Export a single FFF file to standard TIFF.
    /// Reads the full-resolution image (preserving 16-bit) and writes a standard TIFF.
    /// The source file is never modified — export always writes to a new file.
    fn export_fff_to_tiff(src: &Path, dst: &Path) -> Result<(), String> {
        // Guard: never overwrite the source file
        if let (Ok(src_canon), Ok(dst_canon)) = (src.canonicalize(), dst.canonicalize()) {
            if src_canon == dst_canon {
                return Err("Cannot overwrite source file".to_string());
            }
        }

        log::info!("export_fff_to_tiff: {} → {}", src.display(), dst.display());
        let tiff = TiffFile::open(src).map_err(|e| {
            log::error!("export: failed to open {}: {}", src.display(), e);
            e.to_string()
        })?;

        let img = tiff
            .decode_for_export()
            .ok_or_else(|| {
                log::error!("export: failed to decode {}", src.display());
                "Failed to decode image data".to_string()
            })?;

        log::info!("export: decoded {}x{}, saving...", img.width(), img.height());
        img.save(dst).map_err(|e| {
            log::error!("export: failed to save {}: {}", dst.display(), e);
            e.to_string()
        })?;

        log::info!("export: saved {}", dst.display());
        Ok(())
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    fn param_chip(ui: &mut egui::Ui, label: &str, value: &str, modified: bool) {
        let color = if modified {
            if ui.visuals().dark_mode {
                egui::Color32::from_rgb(255, 200, 100)
            } else {
                egui::Color32::from_rgb(180, 120, 0)
            }
        } else {
            ui.visuals().weak_text_color()
        };
        ui.label(
            egui::RichText::new(format!("{}:{}", label, value))
                .small()
                .monospace()
                .color(color),
        );
    }

    fn render_correction_details(
        ui: &mut egui::Ui,
        corr: &flexcolor::ImageCorrection,
        lang: Language,
    ) {
        let s = i18n::strings(lang);

        egui::CollapsingHeader::new(egui::RichText::new(s.image_adjustments).small().strong())
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("adj_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(ui, s.gamma, &format!("{}", corr.gamma));
                        Self::detail_row(ui, s.ev, &format!("{}", corr.ev));
                        Self::detail_row(ui, s.contrast, &format!("{}", corr.contrast));
                        Self::detail_row(ui, s.brightness, &format!("{}", corr.brightness));
                        Self::detail_row(ui, s.lightness, &format!("{}", corr.lightness));
                        Self::detail_row(ui, s.saturation, &format!("{}", corr.saturation));
                        Self::detail_row(
                            ui,
                            s.color_temp,
                            &format!("{}", corr.color_temperature),
                        );
                        Self::detail_row(ui, s.tint, &format!("{}", corr.tint));
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.film_settings).small().strong())
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("film_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.film_curve,
                            flexcolor::film_curve_name(corr.film_curve),
                        );
                        Self::detail_row(
                            ui,
                            s.film_type,
                            flexcolor::film_type_name(corr.film_type),
                        );
                        Self::detail_row(
                            ui,
                            s.color_model,
                            flexcolor::color_model_name(corr.color_model),
                        );
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.sharpening_usm).small().strong())
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("usm_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.enabled,
                            if corr.apply_usm { s.yes } else { s.no },
                        );
                        Self::detail_row(ui, s.amount, &format!("{}", corr.usm_amount));
                        Self::detail_row(ui, s.radius, &format!("{}", corr.usm_radius));
                        Self::detail_row(ui, s.dark_limit, &format!("{}", corr.usm_dark_limit));
                        Self::detail_row(ui, s.threshold, &format!("{}", corr.threshold));
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.processing_flags).small().strong())
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("flags_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.apply_sliders,
                            &Self::bool_icon(corr.apply_sliders),
                        );
                        Self::detail_row(
                            ui,
                            s.apply_curves,
                            &Self::bool_icon(corr.apply_curves),
                        );
                        Self::detail_row(
                            ui,
                            s.apply_histogram,
                            &Self::bool_icon(corr.apply_histogram),
                        );
                        Self::detail_row(ui, s.apply_cc, &Self::bool_icon(corr.apply_cc));
                        Self::detail_row(
                            ui,
                            s.noise_filter,
                            &Self::bool_icon(corr.apply_cn_filter),
                        );
                        Self::detail_row(
                            ui,
                            s.dust_removal,
                            &Self::bool_icon(corr.apply_dust),
                        );
                        Self::detail_row(
                            ui,
                            s.embed_profile,
                            &Self::bool_icon(corr.embed_profile),
                        );
                        Self::detail_row(
                            ui,
                            s.enhanced_shadow,
                            &Self::bool_icon(corr.enhanced_shadow),
                        );
                        Self::detail_row(
                            ui,
                            s.rm_cast_highlight,
                            &Self::bool_icon(corr.remove_cast_highlight),
                        );
                        Self::detail_row(
                            ui,
                            s.rm_cast_shadow,
                            &Self::bool_icon(corr.remove_cast_shadow),
                        );
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.lens_other).small().strong())
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("lens_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.lens_correction,
                            &format!("{}", corr.lens_correction),
                        );
                        Self::detail_row(
                            ui,
                            s.vignette_amount,
                            &format!("{}", corr.vignette_amount),
                        );
                        Self::detail_row(ui, s.dust_level, &format!("{}", corr.dust_level));
                        Self::detail_row(
                            ui,
                            s.noise_radius,
                            &format!("{}", corr.color_noise_radius),
                        );
                        Self::detail_row(
                            ui,
                            s.auto_highlight,
                            &format!("{}", corr.auto_highlight),
                        );
                        Self::detail_row(
                            ui,
                            s.auto_shadow,
                            &format!("{}", corr.auto_shadow),
                        );
                    });
            });

        if !corr.gradations.is_empty() {
            egui::CollapsingHeader::new(
                egui::RichText::new(s.gradation_curves).small().strong(),
            )
            .default_open(false)
            .show(ui, |ui| {
                let channel_names = ["Master", "Red", "Green", "Blue"];
                for (ch_idx, points) in corr.gradations.iter().enumerate() {
                    let ch_name = channel_names.get(ch_idx).unwrap_or(&"?");
                    let points_str: Vec<String> = points
                        .iter()
                        .map(|(x, y, _)| format!("({},{})", x, y))
                        .collect();
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: {}",
                            ch_name,
                            points_str.join(" → ")
                        ))
                        .small()
                        .monospace(),
                    );
                }
            });
        }
    }

    fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
        ui.label(egui::RichText::new(label).small());
        ui.label(egui::RichText::new(value).small().monospace());
        ui.end_row();
    }

    fn bool_icon(v: bool) -> String {
        if v { "✅".into() } else { "—".into() }
    }
}

// ─── Utility functions ──────────────────────────────────────────────────────

fn scan_fff_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            if !path.is_file() {
                return false;
            }
            match path.extension().and_then(|ext| ext.to_str()) {
                Some(ext) => {
                    matches!(ext.to_lowercase().as_str(), "fff" | "3fr" | "tif" | "tiff")
                }
                None => false,
            }
        })
        .map(|e| e.path())
        .collect()
}

fn get_root_dirs() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(home) = dirs_home() {
        roots.push(home);
    }

    let volumes = PathBuf::from("/Volumes");
    if volumes.exists() {
        if let Ok(entries) = std::fs::read_dir(&volumes) {
            for entry in entries.filter_map(|e| e.ok()) {
                roots.push(entry.path());
            }
        }
    }

    if cfg!(not(target_os = "windows")) {
        roots.push(PathBuf::from("/"));
    }

    roots.sort();
    roots.dedup();
    roots
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Shorten verbose directory names for cleaner display in the tree.
/// Full name is still shown on hover tooltip.
fn shorten_dir_name(name: &str) -> String {
    // Adobe Creative Cloud sync folders contain long personal-info paths
    // e.g. "Creative Cloud Files - john.doe@email.com"
    // e.g. "CoreSync - john.doe@email.com - Adobe Creative Cloud"
    if let Some(idx) = name.find(" - ") {
        let prefix = &name[..idx];
        // Known verbose prefixes
        if prefix == "Creative Cloud Files"
            || prefix == "CoreSync"
            || name.contains("Adobe Creative Cloud")
            || name.contains("Creative Cloud")
        {
            return prefix.to_string();
        }
    }

    // Strip common verbose suffixes / patterns
    let patterns: &[(&str, &str)] = &[
        // "Something (john.doe@email.com)" → "Something"
        // Generic: trim trailing parenthesized email/id
    ];
    let mut result = name.to_string();
    for (find, replace) in patterns {
        result = result.replace(find, replace);
    }

    // If name contains an email address, redact it
    if let Some(at_pos) = result.find('@') {
        // Find the word containing '@'
        let start = result[..at_pos].rfind(|c: char| c == ' ' || c == '-').map(|i| i + 1).unwrap_or(0);
        let end = result[at_pos..].find(|c: char| c == ' ' || c == '-').map(|i| at_pos + i).unwrap_or(result.len());
        let email = &result[start..end];
        if email.contains('.') {
            // Replace email with "***"
            result = format!("{}***{}", &result[..start], &result[end..]);
            // Clean up double spaces / trailing dashes
            result = result.replace("  ", " ").replace(" - ***", "").replace("*** - ", "");
            result = result.trim_end_matches(" -").trim_end_matches("- ").trim().to_string();
        }
    }

    // Trim excessively long names (>40 chars)
    if result.chars().count() > 40 {
        let truncated: String = result.chars().take(37).collect();
        return format!("{}…", truncated);
    }

    result
}

/// Find a resource directory (profiles/ or settings/) relative to the executable
/// or the project source directory.
fn find_resource_dir(name: &str, exe_dir: Option<&Path>) -> Option<PathBuf> {
    // 1. Check next to the executable (for .app bundles: Contents/MacOS/../Resources/)
    if let Some(dir) = exe_dir {
        // In .app bundle: exe is at Contents/MacOS/app, resources at Contents/Resources/
        let bundle_resources = dir.join("../Resources").join(name);
        if bundle_resources.exists() {
            return Some(bundle_resources);
        }
        // Flat layout: resources next to binary
        let flat = dir.join(name);
        if flat.exists() {
            return Some(flat);
        }
    }

    // 2. Check current working directory
    let cwd = std::env::current_dir().ok()?;
    let cwd_path = cwd.join(name);
    if cwd_path.exists() {
        return Some(cwd_path);
    }

    // 3. Check CARGO_MANIFEST_DIR (dev mode)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let dev_path = PathBuf::from(manifest_dir).join(name);
        if dev_path.exists() {
            return Some(dev_path);
        }
    }

    None
}

/// Downscale an image if either dimension exceeds the GPU max texture size (16384).
/// This prevents panics in egui_glow's texture upload.
/// Convert a 16-bit image to 8-bit for display.
/// Scanner raw data is already gamma-encoded, so a simple right-shift is correct.
/// 8-bit images pass through unchanged.
/// Uses rayon parallelism for speed on large images.
fn convert_16_to_8_for_display(img: image::DynamicImage) -> image::DynamicImage {
    use rayon::prelude::*;

    match &img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();
            let mut out_pixels = vec![0u8; (w as usize) * (h as usize) * 3];
            let row_len = w as usize * 3;

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row_dst)| {
                    let row_start = y * row_len;
                    for x in 0..row_len {
                        row_dst[x] = (src[row_start + x] >> 8) as u8;
                    }
                });

            let img_buf = image::RgbImage::from_raw(w, h, out_pixels)
                .expect("convert_16_to_8: buffer size mismatch");
            image::DynamicImage::ImageRgb8(img_buf)
        }
        _ => img,
    }
}

fn clamp_image_for_gpu(img: image::DynamicImage) -> image::DynamicImage {
    const MAX_TEX: u32 = 16384;
    let (w, h) = (img.width(), img.height());
    if w <= MAX_TEX && h <= MAX_TEX {
        return img;
    }
    let scale = (MAX_TEX as f64 / w as f64).min(MAX_TEX as f64 / h as f64);
    let nw = (w as f64 * scale) as u32;
    let nh = (h as f64 * scale) as u32;
    log::warn!(
        "Image {}x{} exceeds GPU max {}; downscaling to {}x{}",
        w, h, MAX_TEX, nw, nh
    );
    img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
}
