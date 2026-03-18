use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use fff_viewer::color::{self, IccProfileInfo, SettingsPreset, TargetColorSpace};
use fff_viewer::config::{self, AppConfig};
use fff_viewer::flexcolor::{self, EditHistory, ImageCorrection};
use fff_viewer::i18n::{self, Language, Strings};
use fff_viewer::sidecar::{self, SidecarConfig, SidecarRegion as SidecarRegionData};
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

/// Per-directory subdirectory scan depth.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DirScanDepth {
    Flat,     // 0 — current folder only
    OneLevel, // 1 — one level of subdirectories
    All,      // 2 — all subdirectories recursively
}

impl DirScanDepth {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::OneLevel,
            2 => Self::All,
            _ => Self::Flat,
        }
    }
    fn to_u8(self) -> u8 {
        match self {
            Self::Flat => 0,
            Self::OneLevel => 1,
            Self::All => 2,
        }
    }
    fn cycle(self) -> Self {
        match self {
            Self::Flat => Self::OneLevel,
            Self::OneLevel => Self::All,
            Self::All => Self::Flat,
        }
    }
    /// Short label shown in the tree button
    fn short_label(self) -> &'static str {
        match self {
            Self::Flat => "—",
            Self::OneLevel => "1",
            Self::All => "∞",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum InfoPanel {
    Metadata,
    EditHistory,
    AllTags,
    ColorAdjust,
    ColorProfile,
    Split,
    Settings,
}

// ─── Film split & export ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum FilmFormat {
    Free,
    Full35mm,
    Medium645,
    Medium6x6,
    Medium6x7,
    Medium6x9,
    Medium6x12,
    Medium6x17,
    LargeFormat4x5,
}

impl FilmFormat {
    const ALL: &[Self] = &[
        Self::Free,
        Self::Full35mm,
        Self::Medium645,
        Self::Medium6x6,
        Self::Medium6x7,
        Self::Medium6x9,
        Self::Medium6x12,
        Self::Medium6x17,
        Self::LargeFormat4x5,
    ];

    fn label(&self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Full35mm => "35mm (3:2)",
            Self::Medium645 => "6×4.5 (4:3)",
            Self::Medium6x6 => "6×6 (1:1)",
            Self::Medium6x7 => "6×7 (7:6)",
            Self::Medium6x9 => "6×9 (3:2)",
            Self::Medium6x12 => "6×12 (2:1)",
            Self::Medium6x17 => "6×17 (3:1)",
            Self::LargeFormat4x5 => "4×5 (5:4)",
        }
    }

    /// Aspect ratio as width/height for landscape orientation. None for Free.
    fn ratio(&self) -> Option<f32> {
        match self {
            Self::Free => None,
            Self::Full35mm | Self::Medium6x9 => Some(3.0 / 2.0),
            Self::Medium645 => Some(4.0 / 3.0),
            Self::Medium6x6 => Some(1.0),
            Self::Medium6x7 => Some(7.0 / 6.0),
            Self::Medium6x12 => Some(2.0),
            Self::Medium6x17 => Some(3.0),
            Self::LargeFormat4x5 => Some(5.0 / 4.0),
        }
    }

    fn to_str(&self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Full35mm => "Full35mm",
            Self::Medium645 => "Medium645",
            Self::Medium6x6 => "Medium6x6",
            Self::Medium6x7 => "Medium6x7",
            Self::Medium6x9 => "Medium6x9",
            Self::Medium6x12 => "Medium6x12",
            Self::Medium6x17 => "Medium6x17",
            Self::LargeFormat4x5 => "LargeFormat4x5",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "Full35mm" => Self::Full35mm,
            "Medium645" => Self::Medium645,
            "Medium6x6" => Self::Medium6x6,
            "Medium6x7" => Self::Medium6x7,
            "Medium6x9" => Self::Medium6x9,
            "Medium6x12" => Self::Medium6x12,
            "Medium6x17" => Self::Medium6x17,
            "LargeFormat4x5" => Self::LargeFormat4x5,
            _ => Self::Free,
        }
    }
}

#[derive(Clone)]
struct SplitRegion {
    /// Center coordinates (normalized 0.0–1.0 relative to image dimensions)
    cx: f32,
    cy: f32,
    /// Half-extents (normalized)
    w: f32,
    h: f32,
    /// Rotation angle in radians (clockwise)
    angle: f32,
}

impl SplitRegion {
    /// Get the 4 corners in screen coordinates [TL, TR, BR, BL]
    fn corners_screen(&self, image_rect: egui::Rect) -> [egui::Pos2; 4] {
        let cx_s = image_rect.min.x + self.cx * image_rect.width();
        let cy_s = image_rect.min.y + self.cy * image_rect.height();
        let hw = self.w * image_rect.width() / 2.0;
        let hh = self.h * image_rect.height() / 2.0;
        let (sin_a, cos_a) = self.angle.sin_cos();
        [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].map(|(dx, dy)| {
            egui::pos2(
                cx_s + dx * cos_a - dy * sin_a,
                cy_s + dx * sin_a + dy * cos_a,
            )
        })
    }

    /// Rotation handle position: circle above top-center edge
    fn rotation_handle_screen(&self, image_rect: egui::Rect) -> egui::Pos2 {
        let cx_s = image_rect.min.x + self.cx * image_rect.width();
        let cy_s = image_rect.min.y + self.cy * image_rect.height();
        let hh = self.h * image_rect.height() / 2.0;
        let dist = hh + 22.0;
        let (sin_a, cos_a) = self.angle.sin_cos();
        // (0, -dist) rotated by angle
        egui::pos2(cx_s + dist * sin_a, cy_s - dist * cos_a)
    }

    /// Check if a screen-space point is inside the rotated region
    fn contains_screen_point(&self, point: egui::Pos2, image_rect: egui::Rect) -> bool {
        let cx_s = image_rect.min.x + self.cx * image_rect.width();
        let cy_s = image_rect.min.y + self.cy * image_rect.height();
        let dx = point.x - cx_s;
        let dy = point.y - cy_s;
        let (sin_a, cos_a) = self.angle.sin_cos();
        let local_x = dx * cos_a + dy * sin_a;
        let local_y = -dx * sin_a + dy * cos_a;
        let hw = self.w * image_rect.width() / 2.0;
        let hh = self.h * image_rect.height() / 2.0;
        local_x.abs() <= hw && local_y.abs() <= hh
    }

    fn clamp_to_image(&mut self) {
        self.w = self.w.clamp(0.01, 1.0);
        self.h = self.h.clamp(0.01, 1.0);
        // Compute axis-aligned bounding box of rotated region
        let (sin_a, cos_a) = self.angle.sin_cos();
        let hw = self.w / 2.0;
        let hh = self.h / 2.0;
        let aabb_hw = hw * cos_a.abs() + hh * sin_a.abs();
        let aabb_hh = hw * sin_a.abs() + hh * cos_a.abs();
        self.cx = self.cx.clamp(aabb_hw, (1.0 - aabb_hw).max(aabb_hw));
        self.cy = self.cy.clamp(aabb_hh, (1.0 - aabb_hh).max(aabb_hh));
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragKind {
    Move,
    ResizeTopLeft,
    ResizeTopRight,
    ResizeBottomLeft,
    ResizeBottomRight,
    Rotate,
}

const REGION_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(66, 133, 244),   // blue
    egui::Color32::from_rgb(234, 67, 53),    // red
    egui::Color32::from_rgb(52, 168, 83),    // green
    egui::Color32::from_rgb(251, 188, 4),    // yellow
    egui::Color32::from_rgb(171, 71, 188),   // purple
    egui::Color32::from_rgb(0, 188, 212),    // cyan
];

struct SplitState {
    regions: Vec<SplitRegion>,
    format: FilmFormat,
    portrait: bool,
    naming_pattern: String,
    dragging: Option<(usize, DragKind)>,
    selected: Option<usize>,
}

impl Default for SplitState {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
            format: FilmFormat::Full35mm,
            portrait: false,
            naming_pattern: "{name}_{n}".to_string(),
            dragging: None,
            selected: None,
        }
    }
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
    base_rgb: Option<image::RgbImage>,
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
    sidecar: Option<SidecarConfig>, // persisted settings from XML sidecar
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

    // Favorites (synced with app_config.favorites)
    favorites: Vec<PathBuf>,

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
    manual_adjust: color::ManualAdjust,
    histogram: Option<Box<[[u32; 256]; 4]>>,
    histogram_needs_update: bool,
    tag_filter: String,
    expanded_setting: Option<usize>,

    // File list filter
    file_filter: String,

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

    // Split & export
    split_state: SplitState,

    // Loading progress
    loading_status: LoadingStatus,

    // Error
    error_msg: Option<String>,

    // UI toggles
    show_info_panel: bool,

    // Language
    language: Language,

    // App config (for settings panel)
    app_config: AppConfig,
    settings_needs_restart: bool,
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
            // CJK fonts (e.g. Hiragino Sans GB) have a higher ascent ratio (~0.88)
            // than Ubuntu-Light (~0.83), causing CJK glyphs to sit visually higher.
            // y_offset_factor pushes glyphs down to align with the primary font's
            // visual center in buttons.
            let fd = egui::FontData::from_owned(font_data).tweak(egui::FontTweak {
                scale: 1.0,
                y_offset_factor: 0.2,
                y_offset: 0.0,
                baseline_offset_factor: 0.0,
            });
            fonts.font_data.insert("cjk".to_owned(), fd.into());

            // Adjust emoji fonts' y_offset to align with shifted CJK text.
            if let Some(emoji_data) = fonts.font_data.get_mut("NotoEmoji-Regular") {
                let fd = std::sync::Arc::make_mut(emoji_data);
                fd.tweak.y_offset_factor = -0.15;
            }
            if let Some(emoji_data) = fonts.font_data.get_mut("emoji-icon-font") {
                let fd = std::sync::Arc::make_mut(emoji_data);
                fd.tweak.y_offset_factor = -0.15;
            }

            // Insert CJK as SECOND font (after Ubuntu-Light, before emoji fonts).
            // This keeps Ubuntu-Light as primary for proper button/line metrics,
            // while CJK characters fall back to this font, and emoji still use
            // the built-in NotoEmoji/emoji-icon-font.
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                // Default order: ["Ubuntu-Light", "NotoEmoji-Regular", "emoji-icon-font"]
                // Insert at position 1 → ["Ubuntu-Light", "cjk", "NotoEmoji-Regular", "emoji-icon-font"]
                let pos = 1.min(family.len());
                family.insert(pos, "cjk".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                let pos = 1.min(family.len());
                family.insert(pos, "cjk".to_owned());
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
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>, app_config: AppConfig) -> Self {
        log::info!("Initializing FffViewerApp");
        setup_cjk_fonts(&cc.egui_ctx);

        let language = Language::from_config(&app_config.language);

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

        let favorites: Vec<PathBuf> = app_config
            .favorites
            .iter()
            .map(|s| PathBuf::from(s))
            .collect();

        let mut app = Self {
            current_dir: None,
            expanded_dirs: HashSet::new(),
            favorites,
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
            manual_adjust: color::ManualAdjust::default(),
            histogram: None,
            histogram_needs_update: false,
            tag_filter: String::new(),
            expanded_setting: None,
            file_filter: String::new(),
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
            split_state: SplitState::default(),
            loading_status: LoadingStatus::Idle,
            error_msg: None,
            show_info_panel: true,
            language,
            app_config,
            settings_needs_restart: false,
        };

        if let Some(path) = initial_file {
            if let Some(parent) = path.parent() {
                app.set_directory(parent.to_path_buf());
                if let Some(idx) = app.fff_files.iter().position(|p| p == &path) {
                    app.selected_index = Some(idx);
                }
            }
        } else if let Some(home) = dirs_home() {
            app.set_directory(home);
        }

        app
    }

    fn s(&self) -> &'static Strings {
        i18n::strings(self.language)
    }

    fn set_directory(&mut self, dir: PathBuf) {
        log::info!("set_directory: {}", dir.display());
        self.current_dir = Some(dir.clone());

        // Find the most specific tree root that contains `dir` (longest prefix).
        // Only expand ancestors between `dir` and that root — never go above it,
        // so we don't accidentally expand e.g. "/" when home dir is also a root.
        let roots = get_root_dirs();
        let containing_root = roots
            .iter()
            .filter(|r| dir.starts_with(r.as_path()))
            .max_by_key(|r| r.components().count())
            .cloned();

        if let Some(ref root) = containing_root {
            let mut ancestor = dir.parent().map(|p| p.to_path_buf());
            while let Some(ref p) = ancestor {
                if p.starts_with(root.as_path()) {
                    self.expanded_dirs.insert(p.clone());
                    if p == root {
                        break; // reached containing root — stop
                    }
                    ancestor = p.parent().map(|q| q.to_path_buf());
                } else {
                    break; // above the containing root — don't expand
                }
            }
        }
        // Also expand the directory itself so its children are visible
        self.expanded_dirs.insert(dir.clone());
        let depth = self.dir_scan_depth(&dir);
        self.fff_files = scan_fff_files(&dir, depth);
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

                    // Load sidecar XML if it exists
                    let sidecar_config = sidecar::load(&path);
                    if sidecar_config.is_some() {
                        log::info!("Loaded sidecar: {}", sidecar::sidecar_path(&path).display());
                    }

                    let _ = tx.send(DetailMsg::Loaded(DetailResult {
                        path,
                        tiff,
                        metadata,
                        all_tags,
                        edit_history,
                        preview_rgba,
                        embedded_icc,
                        auto_corrected,
                        sidecar: sidecar_config,
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
                    let has_sidecar = result.sidecar.is_some();
                    let sidecar = result.sidecar.clone();

                    let (texture, base_rgb) = if let Some((ref pixels, w, h)) = result.preview_rgba {
                        let size = [w as usize, h as usize];
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels);
                        let tex = ctx.load_texture(
                            "loupe_preview",
                            color_image,
                            egui::TextureOptions::LINEAR,
                        );
                        let rgb_pixels: Vec<u8> = pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
                        let rgb_img = image::RgbImage::from_raw(w, h, rgb_pixels);
                        (Some(tex), rgb_img)
                    } else {
                        (None, None)
                    };

                    self.detail = Some(LoadedDetail {
                        path: result.path,
                        tiff: result.tiff,
                        metadata: result.metadata,
                        all_tags: result.all_tags,
                        edit_history: result.edit_history,
                        texture,
                        embedded_icc: result.embedded_icc,
                        base_rgb,
                    });

                    if has_sidecar {
                        // Restore settings from sidecar
                        self.apply_sidecar(sidecar.as_ref().unwrap(), ctx);
                    } else {
                        // Default: auto-apply embedded correction
                        if result.auto_corrected {
                            self.use_embedded_correction = true;
                        }
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

        if self.histogram_needs_update {
            self.compute_histogram();
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
                    ui.selectable_value(&mut self.info_panel, InfoPanel::ColorAdjust, s.color_adjust);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::ColorProfile, s.color_profile);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::Split, s.split_export);
                    ui.separator();
                    ui.selectable_value(&mut self.info_panel, InfoPanel::Settings, s.settings);
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
                    let old_lang = self.language;
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
                    if self.language != old_lang {
                        self.app_config.language = self.language.to_config().to_string();
                        let _ = config::save(&self.app_config);
                    }

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

        // ── Left panel: favorites + directory tree ──────────────────────
        let s = i18n::strings(self.language);
        let favorites_heading = s.favorites.to_string();
        let folders_label = s.folders.to_string();
        egui::SidePanel::left("dir_tree_panel")
            .default_width(220.0)
            .min_width(160.0)
            .show(ctx, |ui| {
                // ── Favorites section ────────────────────────────────────
                ui.heading(&favorites_heading);
                ui.separator();
                let fav_height = if self.favorites.is_empty() { 28.0 } else { (self.favorites.len() as f32 * 22.0).min(160.0) };
                egui::ScrollArea::vertical()
                    .id_salt("favs_scroll")
                    .max_height(fav_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        self.render_favorites(ui);
                    });
                ui.add_space(4.0);
                // ── Folders section ──────────────────────────────────────
                ui.heading(&folders_label);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("tree_scroll")
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
                    InfoPanel::ColorAdjust => self.render_color_adjust_panel(ui, ctx),
                    InfoPanel::ColorProfile => self.render_color_profile_panel(ui, ctx),
                    InfoPanel::Split => self.render_split_panel(ui, ctx),
                    InfoPanel::Settings => self.render_settings_panel(ui),
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
                self.render_file_filter_bar(ui);
                ui.separator();
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
    fn render_favorites(&mut self, ui: &mut egui::Ui) {
        let s = i18n::strings(self.language);
        if self.favorites.is_empty() {
            ui.label(egui::RichText::new(s.no_favorites).weak().italics());
            return;
        }
        let favorites = self.favorites.clone();
        let mut remove_idx: Option<usize> = None;
        for (i, fav) in favorites.iter().enumerate() {
            let is_selected = self.current_dir.as_deref() == Some(fav.as_path());
            let name = fav
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| fav.to_string_lossy().to_string());
            let name = shorten_dir_name(&name);

            ui.horizontal(|ui| {
                // Remove-from-favorites button
                let star_resp = ui.add(
                    egui::Label::new(egui::RichText::new("★").color(egui::Color32::from_rgb(255, 200, 0)))
                        .sense(egui::Sense::click()),
                );
                if star_resp.on_hover_text(s.remove_favorite).clicked() {
                    remove_idx = Some(i);
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
                let resp = resp.on_hover_text(fav.to_string_lossy());
                if resp.clicked() {
                    self.set_directory(fav.clone());
                }
            });
        }
        if let Some(idx) = remove_idx {
            self.favorites.remove(idx);
            self.save_favorites();
        }
    }

    fn save_favorites(&mut self) {
        self.app_config.favorites = self.favorites.iter().map(|p| p.to_string_lossy().to_string()).collect();
        let _ = config::save(&self.app_config);
    }

    fn dir_scan_depth(&self, dir: &Path) -> DirScanDepth {
        let key = dir.to_string_lossy();
        DirScanDepth::from_u8(
            self.app_config.dir_scan_modes.get(key.as_ref()).copied().unwrap_or(0)
        )
    }

    fn set_dir_scan_depth(&mut self, dir: &Path, depth: DirScanDepth) {
        let key = dir.to_string_lossy().to_string();
        if depth == DirScanDepth::Flat {
            self.app_config.dir_scan_modes.remove(&key);
        } else {
            self.app_config.dir_scan_modes.insert(key, depth.to_u8());
        }
        let _ = config::save(&self.app_config);
    }

    fn render_dir_tree(&mut self, ui: &mut egui::Ui) {
        let roots = get_root_dirs();
        for root in &roots {
            self.render_dir_node(ui, root, 0);
        }
    }

    fn render_dir_node(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize) {
        let is_expanded = self.expanded_dirs.contains(path);
        let is_selected = self.current_dir.as_deref() == Some(path);
        let is_fav = self.favorites.contains(&path.to_path_buf());
        let scan_depth = self.dir_scan_depth(path);

        let raw_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let name = shorten_dir_name(&raw_name);

        let indent = depth as f32 * 16.0;
        let mut toggle_fav = false;
        let mut cycle_depth = false;
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

            // Star icon: filled gold if favorited, dim outline if not
            let star_char = if is_fav { "★" } else { "☆" };
            let star_color = if is_fav {
                egui::Color32::from_rgb(255, 200, 0)
            } else {
                ui.visuals().weak_text_color()
            };
            let s = i18n::strings(self.language);
            let hint = if is_fav { s.remove_favorite } else { s.add_favorite };
            let star_resp = ui.add(
                egui::Label::new(egui::RichText::new(star_char).color(star_color).small())
                    .sense(egui::Sense::click()),
            );
            if star_resp.on_hover_text(hint).clicked() {
                toggle_fav = true;
            }

            // Right portion: depth button (right-aligned) + folder label (left-aligned, fills rest).
            // Outer RTL pins depth button to far right; inner LTR keeps folder name left-aligned.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Depth cycle button — appears on far right (first in RTL)
                let depth_hint = match scan_depth {
                    DirScanDepth::Flat => s.scan_depth_flat,
                    DirScanDepth::OneLevel => s.scan_depth_one,
                    DirScanDepth::All => s.scan_depth_all,
                };
                let depth_color = match scan_depth {
                    DirScanDepth::Flat => ui.visuals().weak_text_color(),
                    DirScanDepth::OneLevel => egui::Color32::from_rgb(80, 160, 255),
                    DirScanDepth::All => egui::Color32::from_rgb(80, 200, 120),
                };
                let btn = ui.add(
                    egui::Label::new(
                        egui::RichText::new(scan_depth.short_label())
                            .small()
                            .monospace()
                            .color(depth_color),
                    )
                    .sense(egui::Sense::click()),
                );
                if btn.on_hover_text(depth_hint).clicked() {
                    cycle_depth = true;
                }

                // Folder label — left-aligned in remaining space, truncates if needed
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let text = egui::RichText::new(format!("📁 {}", name)).color(if is_selected {
                        ui.visuals().hyperlink_color
                    } else {
                        ui.visuals().text_color()
                    });
                    let label = egui::Label::new(if is_selected { text.strong() } else { text })
                        .sense(egui::Sense::click())
                        .truncate();
                    let resp = ui.add(label);
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
            });
        });

        if toggle_fav {
            let pb = path.to_path_buf();
            if is_fav {
                self.favorites.retain(|f| f != &pb);
            } else {
                self.favorites.push(pb);
            }
            self.save_favorites();
        }
        if cycle_depth {
            let new_depth = scan_depth.cycle();
            self.set_dir_scan_depth(path, new_depth);
            // If this is the currently selected directory, re-scan with new depth
            if self.current_dir.as_deref() == Some(path) {
                let dir = path.to_path_buf();
                self.fff_files = scan_fff_files(&dir, new_depth);
                self.fff_files.sort();
                self.thumbnails.clear();
                self.thumb_pending = self.fff_files.len();
                self.loading_status = if self.fff_files.is_empty() {
                    LoadingStatus::Idle
                } else {
                    LoadingStatus::LoadingThumbnails
                };
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
        }

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
    fn render_file_filter_bar(&mut self, ui: &mut egui::Ui) {
        let s = i18n::strings(self.language);
        ui.horizontal(|ui| {
            // Stretch the search field to fill available space minus the clear button
            let clear_width = 24.0;
            let available = ui.available_width() - clear_width - ui.spacing().item_spacing.x * 2.0;
            ui.add(
                egui::TextEdit::singleline(&mut self.file_filter)
                    .hint_text(s.file_filter_placeholder)
                    .desired_width(available),
            );
            let clear_enabled = !self.file_filter.is_empty();
            if ui
                .add_enabled(
                    clear_enabled,
                    egui::Button::new(s.file_filter_clear).min_size(egui::vec2(clear_width, 0.0)),
                )
                .clicked()
            {
                self.file_filter.clear();
            }
            // Show match count when filter is active
            if !self.file_filter.is_empty() {
                let matched = self.filtered_indices().len();
                let total = self.fff_files.len();
                let label = s
                    .files_filtered
                    .replacen("{}", &matched.to_string(), 1)
                    .replacen("{}", &total.to_string(), 1);
                ui.label(egui::RichText::new(label).weak().small());
            }
        });
    }

    /// Returns indices into `self.fff_files` that match the current filter.
    fn filtered_indices(&self) -> Vec<usize> {
        if self.file_filter.is_empty() {
            return (0..self.fff_files.len()).collect();
        }
        let query = self.file_filter.to_lowercase();
        self.fff_files
            .iter()
            .enumerate()
            .filter(|(_, path)| {
                // Match against the full filename (name + extension)
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                // Also match against just the extension
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                // Fuzzy: all query chars must appear in order (subsequence match)
                fn subsequence(haystack: &str, needle: &str) -> bool {
                    let mut chars = haystack.chars();
                    needle.chars().all(|nc| chars.any(|hc| hc == nc))
                }
                subsequence(&name, &query) || subsequence(&ext, &query)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    fn render_grid_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let thumb_size = 180.0_f32;
        let spacing = 8.0_f32;
        let available_width = ui.available_width();
        let cols = ((available_width + spacing) / (thumb_size + spacing)).max(1.0) as usize;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let files = self.fff_files.clone();
                let indices = self.filtered_indices();
                let selected = self.selected_index;

                let mut new_selection: Option<usize> = None;
                let mut double_clicked: Option<usize> = None;

                egui::Grid::new("thumb_grid")
                    .spacing([spacing, spacing])
                    .show(ui, |ui| {
                        for (col_pos, idx) in indices.iter().enumerate() {
                            let idx = *idx;
                            let path = &files[idx];
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

                            if (col_pos + 1) % cols == 0 {
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

                // Allocate full area with click+drag sensing for split interaction
                let (full_rect, response) = ui.allocate_exact_size(
                    available,
                    if self.info_panel == InfoPanel::Split {
                        egui::Sense::click_and_drag()
                    } else {
                        egui::Sense::hover()
                    },
                );

                // Center the image within the allocated area
                let image_rect =
                    egui::Align2::CENTER_CENTER.align_size_within_rect(display_size, full_rect);

                // Draw the image
                let painter = ui.painter_at(full_rect);
                painter.image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Draw split overlays and handle interactions
                if self.info_panel == InfoPanel::Split {
                    self.handle_split_interactions(&response, image_rect, ctx);
                    let painter = ui.painter_at(full_rect);
                    draw_split_overlays(
                        &painter,
                        image_rect,
                        &self.split_state.regions,
                        self.split_state.selected,
                    );
                }
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
        let (left, right, delete) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
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
        // Delete selected split region with Delete/Backspace key
        if delete && self.info_panel == InfoPanel::Split {
            if let Some(idx) = self.split_state.selected {
                if idx < self.split_state.regions.len() {
                    self.split_state.regions.remove(idx);
                    self.split_state.selected = None;
                    self.split_state.dragging = None;
                    self.save_sidecar();
                }
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

        let old_target = self.target_color_space;
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
        if self.target_color_space != old_target {
            self.save_sidecar();
        }

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
                self.save_sidecar();
            }

            if ui.button(s.reset_profile).clicked() {
                self.reset_color_profile(ctx);
                self.save_sidecar();
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

    /// Apply saved sidecar configuration to current state
    fn apply_sidecar(&mut self, config: &SidecarConfig, ctx: &egui::Context) {
        log::info!("Applying sidecar config");
        self.use_embedded_correction = config.use_embedded_correction;
        self.use_embedded_icc = config.use_embedded_icc;
        self.target_color_space = TargetColorSpace::from_str(&config.target_color_space);

        // Look up profile by name
        self.selected_input_profile = config.input_profile_name.as_ref().and_then(|name| {
            self.available_profiles.iter().position(|p| &p.name == name)
        });

        // Look up preset by name
        self.selected_preset = config.preset_name.as_ref().and_then(|name| {
            self.available_presets.iter().position(|p| &p.name == name)
        });

        // Restore split state
        self.split_state.format = FilmFormat::from_str(&config.split_format);
        self.split_state.portrait = config.split_portrait;
        self.split_state.naming_pattern = config.split_naming_pattern.clone();
        self.split_state.regions = config.split_regions.iter().map(|r| {
            SplitRegion { cx: r.cx, cy: r.cy, w: r.w, h: r.h, angle: r.angle }
        }).collect();
        self.split_state.selected = None;
        self.split_state.dragging = None;

        // Re-apply color profile if any profile/preset was selected
        self.manual_adjust = config.manual_adjust.clone();
        self.histogram_needs_update = true;
        if self.selected_input_profile.is_some() || self.selected_preset.is_some()
            || self.use_embedded_icc || self.use_embedded_correction {
            self.apply_color_profile(ctx);
        } else {
            self.rebuild_texture_from_base(ctx);
        }
    }

    /// Save current state to sidecar XML file
    fn save_sidecar(&self) {
        let path = match &self.detail {
            Some(d) => &d.path,
            None => return,
        };

        let input_profile_name = self.selected_input_profile
            .and_then(|i| self.available_profiles.get(i))
            .map(|p| p.name.clone());
        let preset_name = self.selected_preset
            .and_then(|i| self.available_presets.get(i))
            .map(|p| p.name.clone());

        let config = SidecarConfig {
            use_embedded_correction: self.use_embedded_correction,
            use_embedded_icc: self.use_embedded_icc,
            input_profile_name,
            preset_name,
            target_color_space: self.target_color_space.to_str().to_string(),
            split_format: self.split_state.format.to_str().to_string(),
            split_portrait: self.split_state.portrait,
            split_naming_pattern: self.split_state.naming_pattern.clone(),
            split_regions: self.split_state.regions.iter().map(|r| {
                SidecarRegionData { cx: r.cx, cy: r.cy, w: r.w, h: r.h, angle: r.angle }
            }).collect(),
            manual_adjust: self.manual_adjust.clone(),
        };

        if let Err(e) = sidecar::save(path, &config) {
            log::error!("Failed to save sidecar: {}", e);
        } else {
            log::debug!("Saved sidecar: {}", sidecar::sidecar_path(path).display());
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

        // Step 3: Convert to display-ready 8-bit, clamp for GPU, store base + upload texture
        let result = convert_16_to_8_for_display(result);
        let result = clamp_image_for_gpu(result);
        if let Some(detail) = &mut self.detail {
            detail.base_rgb = Some(result.to_rgb8());
        }
        self.rebuild_texture_from_base(ctx);

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
                detail.base_rgb = Some(processed.to_rgb8());
            }
        }
        self.rebuild_texture_from_base(ctx);
        log::info!("Color profile reset");
    }

    fn rebuild_texture_from_base(&mut self, ctx: &egui::Context) {
        let Some(detail) = &mut self.detail else { return };
        let Some(ref base) = detail.base_rgb else { return };

        let img = image::DynamicImage::ImageRgb8(base.clone());
        let adjusted = color::apply_manual_adjust(&img, &self.manual_adjust);
        let rgba = adjusted.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let pixels = rgba.into_raw();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
        detail.texture = Some(ctx.load_texture(
            "loupe_preview_adjusted",
            color_image,
            egui::TextureOptions::LINEAR,
        ));
        self.histogram_needs_update = true;
    }

    fn compute_histogram(&mut self) {
        let Some(detail) = &self.detail else {
            self.histogram = None;
            return;
        };
        let Some(ref base) = detail.base_rgb else {
            self.histogram = None;
            return;
        };

        let mut hist = Box::new([[0u32; 256]; 4]);
        let img = image::DynamicImage::ImageRgb8(base.clone());
        let adjusted = color::apply_manual_adjust(&img, &self.manual_adjust);
        let rgb = adjusted.to_rgb8();

        for pixel in rgb.pixels() {
            let [r, g, b] = pixel.0;
            hist[0][r as usize] += 1;
            hist[1][g as usize] += 1;
            hist[2][b as usize] += 1;
            let lum = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) as u8;
            hist[3][lum as usize] += 1;
        }
        self.histogram = Some(hist);
        self.histogram_needs_update = false;
    }

    fn render_histogram(&self, ui: &mut egui::Ui) {
        let Some(ref hist) = self.histogram else { return };

        let desired_size = egui::vec2(ui.available_width(), 80.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));

        let max_count = hist[0].iter().chain(hist[1].iter()).chain(hist[2].iter())
            .copied().max().unwrap_or(1).max(1) as f32;

        let w = rect.width();
        let h = rect.height();

        let colors = [
            egui::Color32::from_rgba_unmultiplied(220, 50, 50, 100),
            egui::Color32::from_rgba_unmultiplied(50, 200, 50, 100),
            egui::Color32::from_rgba_unmultiplied(50, 100, 255, 100),
        ];

        for ch in 0..3 {
            for i in 0..256 {
                let bar_h = (hist[ch][i] as f32 / max_count * h).min(h);
                if bar_h < 0.5 { continue; }
                let x = rect.left() + i as f32 / 255.0 * w;
                let bar_w = (w / 256.0).max(1.0);
                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(x, rect.bottom() - bar_h),
                    egui::vec2(bar_w, bar_h),
                );
                painter.rect_filled(bar_rect, 0.0, colors[ch]);
            }
        }
    }

    fn render_color_adjust_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = self.s();

        if self.detail.is_none() {
            ui.label(s.select_image);
            return;
        }

        let mut rebuild = false;

        let adjust_heading = s.adjust_heading;
        let adjust_enabled = s.adjust_enabled;
        let reset_adjust = s.reset_adjust;
        let exposure_str = s.exposure;
        let contrast_str = s.contrast;
        let highlights_str = s.highlights;
        let shadows_str = s.shadows;
        let saturation_str = s.saturation_label;
        let color_balance_str = s.color_balance;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(adjust_heading);
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.checkbox(&mut self.manual_adjust.enabled, adjust_enabled).changed() {
                    rebuild = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(reset_adjust).clicked() {
                        let enabled = self.manual_adjust.enabled;
                        self.manual_adjust = color::ManualAdjust { enabled, ..Default::default() };
                        rebuild = true;
                    }
                });
            });

            ui.add_space(4.0);

            // Render histogram inline to avoid borrow conflicts with self.manual_adjust below
            if let Some(ref hist) = self.histogram {
                let desired_size = egui::vec2(ui.available_width(), 80.0);
                let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));
                let max_count = hist[0].iter().chain(hist[1].iter()).chain(hist[2].iter())
                    .copied().max().unwrap_or(1).max(1) as f32;
                let w = rect.width();
                let h = rect.height();
                let colors = [
                    egui::Color32::from_rgba_unmultiplied(220, 50, 50, 100),
                    egui::Color32::from_rgba_unmultiplied(50, 200, 50, 100),
                    egui::Color32::from_rgba_unmultiplied(50, 100, 255, 100),
                ];
                for ch in 0..3 {
                    for i in 0..256 {
                        let bar_h = (hist[ch][i] as f32 / max_count * h).min(h);
                        if bar_h < 0.5 { continue; }
                        let x = rect.left() + i as f32 / 255.0 * w;
                        let bar_w = (w / 256.0).max(1.0);
                        let bar_rect = egui::Rect::from_min_size(
                            egui::pos2(x, rect.bottom() - bar_h),
                            egui::vec2(bar_w, bar_h),
                        );
                        painter.rect_filled(bar_rect, 0.0, colors[ch]);
                    }
                }
            }

            ui.add_space(8.0);
            ui.separator();

            let adj = &mut self.manual_adjust;

            ui.label(exposure_str);
            if ui.add(egui::Slider::new(&mut adj.exposure, -3.0..=3.0).step_by(0.05).text("stops")).changed() {
                rebuild = true;
            }

            ui.label(contrast_str);
            if ui.add(egui::Slider::new(&mut adj.contrast, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label(highlights_str);
            if ui.add(egui::Slider::new(&mut adj.highlights, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label(shadows_str);
            if ui.add(egui::Slider::new(&mut adj.shadows, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label(saturation_str);
            if ui.add(egui::Slider::new(&mut adj.saturation, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.add_space(8.0);
            ui.label(egui::RichText::new(color_balance_str).strong());

            ui.label("R");
            if ui.add(egui::Slider::new(&mut adj.r_shift, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label("G");
            if ui.add(egui::Slider::new(&mut adj.g_shift, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label("B");
            if ui.add(egui::Slider::new(&mut adj.b_shift, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }
        });

        if rebuild {
            self.rebuild_texture_from_base(ctx);
            self.save_sidecar();
        }
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

    // ── Settings ──────────────────────────────────────────────────────

    fn render_settings_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.settings_heading);
        ui.separator();
        ui.add_space(8.0);

        // ── GPU Acceleration ──
        ui.strong(s.gpu_acceleration);
        ui.add_space(2.0);
        let old_gpu = self.app_config.gpu_enabled;
        ui.checkbox(&mut self.app_config.gpu_enabled, s.gpu_acceleration);

        ui.add_space(4.0);
        ui.strong(s.gpu_device_label);
        ui.add_space(2.0);
        let old_device = self.app_config.gpu_device.clone();
        ui.text_edit_singleline(&mut self.app_config.gpu_device);
        ui.label(
            egui::RichText::new(s.gpu_auto_select)
                .small()
                .color(ui.visuals().weak_text_color()),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Render Threads ──
        ui.strong(s.render_threads);
        ui.add_space(2.0);
        let max_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(16);
        let old_threads = self.app_config.render_threads;
        let mut threads = self.app_config.render_threads as u32;
        ui.add(egui::Slider::new(&mut threads, 1..=(max_cpus as u32)).suffix(" threads"));
        self.app_config.render_threads = threads as usize;
        ui.label(
            egui::RichText::new(s.render_threads_hint)
                .small()
                .color(ui.visuals().weak_text_color()),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Language ──
        ui.strong(s.ui_language);
        ui.add_space(2.0);
        let old_lang = self.language;
        egui::ComboBox::from_id_salt("settings_lang_selector")
            .selected_text(self.language.label())
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.language, Language::English, Language::English.label());
                ui.selectable_value(&mut self.language, Language::Chinese, Language::Chinese.label());
            });

        // Detect changes and save
        let gpu_changed = self.app_config.gpu_enabled != old_gpu
            || self.app_config.gpu_device != old_device;
        let threads_changed = self.app_config.render_threads != old_threads;
        let lang_changed = self.language != old_lang;

        if gpu_changed || threads_changed || lang_changed {
            if lang_changed {
                self.app_config.language = self.language.to_config().to_string();
            }
            let _ = config::save(&self.app_config);

            if gpu_changed || threads_changed {
                self.settings_needs_restart = true;
            }
        }

        // Restart hint
        if self.settings_needs_restart {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(s.restart_required)
                    .color(egui::Color32::from_rgb(255, 180, 0)),
            );
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // Data directory info
        ui.label(
            egui::RichText::new(format!("📁 {}", config::app_data_dir().display()))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }

    // ── Split & Export ────────────────────────────────────────────────

    fn render_split_panel(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let s = self.s();
        ui.heading(s.split_export);
        ui.separator();

        // Film format selector
        ui.strong(s.film_format);
        ui.add_space(2.0);
        let old_format = self.split_state.format;
        egui::ComboBox::from_id_salt("film_format_combo")
            .selected_text(self.split_state.format.label())
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                for &fmt in FilmFormat::ALL {
                    ui.selectable_value(&mut self.split_state.format, fmt, fmt.label());
                }
            });

        // Orientation toggle (only for non-free, non-square formats)
        let old_portrait = self.split_state.portrait;
        if let Some(ratio) = self.split_state.format.ratio() {
            if (ratio - 1.0).abs() > 0.01 {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(s.orientation_label);
                    ui.selectable_value(&mut self.split_state.portrait, false, s.landscape_label);
                    ui.selectable_value(&mut self.split_state.portrait, true, s.portrait_label);
                });
            }
        }
        if self.split_state.format != old_format || self.split_state.portrait != old_portrait {
            self.save_sidecar();
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Add / Clear buttons
        let has_detail = self.detail.is_some();
        ui.horizontal(|ui| {
            if ui.add_enabled(has_detail, egui::Button::new(s.add_region)).clicked() {
                self.add_split_region();
                self.save_sidecar();
            }
            if ui.add_enabled(!self.split_state.regions.is_empty(), egui::Button::new(s.clear_all)).clicked() {
                self.split_state.regions.clear();
                self.split_state.dragging = None;
                self.split_state.selected = None;
                self.save_sidecar();
            }
        });

        ui.add_space(8.0);

        // Region list
        if self.split_state.regions.is_empty() {
            ui.label(
                egui::RichText::new(s.no_regions)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        } else {
            let mut remove_idx = None;
            let n_regions = self.split_state.regions.len();
            for i in 0..n_regions {
                let color = REGION_COLORS[i % REGION_COLORS.len()];
                let is_active = self.split_state.selected == Some(i);

                ui.horizontal(|ui| {
                    // Color swatch
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, color);

                    let label = format!("{} #{}", s.region_label, i + 1);
                    let rt = if is_active {
                        egui::RichText::new(&label).strong()
                    } else {
                        egui::RichText::new(&label)
                    };
                    if ui.selectable_label(is_active, rt).clicked() {
                        self.split_state.selected = Some(i);
                    }

                    // Show dimensions and angle
                    let r = &self.split_state.regions[i];
                    let angle_deg = r.angle.to_degrees();
                    let info = if angle_deg.abs() > 0.1 {
                        format!("{:.0}%×{:.0}% ∠{:.1}°", r.w * 100.0, r.h * 100.0, angle_deg)
                    } else {
                        format!("{:.0}%×{:.0}%", r.w * 100.0, r.h * 100.0)
                    };
                    ui.label(
                        egui::RichText::new(info)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );

                    if ui.small_button("🗑").clicked() {
                        remove_idx = Some(i);
                    }
                });
            }
            if let Some(idx) = remove_idx {
                self.split_state.regions.remove(idx);
                // Fix selected index
                if let Some(sel) = self.split_state.selected {
                    if sel == idx {
                        self.split_state.selected = None;
                    } else if sel > idx {
                        self.split_state.selected = Some(sel - 1);
                    }
                }
                // Fix dragging index
                if let Some((drag_idx, _)) = &self.split_state.dragging {
                    if *drag_idx == idx {
                        self.split_state.dragging = None;
                    } else if *drag_idx > idx {
                        self.split_state.dragging = Some((*drag_idx - 1, DragKind::Move));
                    }
                }
                self.save_sidecar();
            }
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // Naming pattern
        ui.strong(s.naming_pattern);
        ui.add_space(2.0);
        let old_pattern = self.split_state.naming_pattern.clone();
        let resp = ui.text_edit_singleline(&mut self.split_state.naming_pattern);
        if resp.lost_focus() && self.split_state.naming_pattern != old_pattern {
            self.save_sidecar();
        }
        ui.label(
            egui::RichText::new("{name} = filename, {n} = index")
                .small()
                .color(ui.visuals().weak_text_color()),
        );

        // Preview of naming
        if !self.split_state.regions.is_empty() {
            if let Some(detail) = &self.detail {
                let stem = detail.path.file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file".to_string());
                let preview = self.split_state.naming_pattern
                    .replace("{name}", &stem)
                    .replace("{n}", "1");
                ui.label(
                    egui::RichText::new(format!("→ {}.tif", preview))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            }
        }

        ui.add_space(12.0);

        // Export button
        let can_export = has_detail && !self.split_state.regions.is_empty();
        if ui.add_enabled(can_export, egui::Button::new(s.export_splits)).clicked() {
            self.export_split_regions();
        }

        // Show export status
        if let ExportStatus::Done { count, ref dir } = self.export_state.status {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("✅ {} {} → {}", s.split_exported, count, dir.display()))
                    .color(if ui.visuals().dark_mode {
                        egui::Color32::from_rgb(100, 255, 100)
                    } else {
                        egui::Color32::from_rgb(0, 140, 0)
                    }),
            );
        }
        if let ExportStatus::Error(ref e) = self.export_state.status {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, format!("❌ {}", e));
        }
    }

    fn add_split_region(&mut self) {
        let effective_ratio = self.split_state.format.ratio().map(|r| {
            if self.split_state.portrait { 1.0 / r } else { r }
        });

        // Default region: centered, ~30% of image height
        let h = 0.3_f32.min(1.0);
        let w = if let Some(ratio) = effective_ratio {
            let img_aspect = self.detail.as_ref()
                .and_then(|d| d.texture.as_ref())
                .map(|t| t.size_vec2().x / t.size_vec2().y)
                .unwrap_or(1.0);
            (h * ratio / img_aspect).min(0.95)
        } else {
            0.3
        };

        let n = self.split_state.regions.len();
        let y_offset = (n as f32 * 0.05) % 0.5;
        let mut region = SplitRegion {
            cx: 0.5,
            cy: (0.1 + y_offset + h / 2.0).min(1.0 - h / 2.0),
            w: w.min(1.0),
            h: h.min(1.0),
            angle: 0.0,
        };
        region.clamp_to_image();
        let new_idx = self.split_state.regions.len();
        self.split_state.selected = Some(new_idx);
        self.split_state.regions.push(region);
    }

    fn handle_split_interactions(&mut self, response: &egui::Response, image_rect: egui::Rect, ctx: &egui::Context) {
        let handle_radius = 10.0_f32;
        let rot_handle_radius = 8.0_f32;

        // --- Hover cursor ---
        if response.hovered() && self.split_state.dragging.is_none() {
            if let Some(mouse_pos) = response.hover_pos() {
                let mut cursor = None;
                'outer: for (_i, region) in self.split_state.regions.iter().enumerate().rev() {
                    let corners = region.corners_screen(image_rect);

                    // Check rotation handle
                    let rot_pos = region.rotation_handle_screen(image_rect);
                    if mouse_pos.distance(rot_pos) <= rot_handle_radius + 4.0 {
                        cursor = Some(egui::CursorIcon::Crosshair);
                        break;
                    }

                    // Check corner handles [TL, TR, BR, BL]
                    for (ci, corner) in corners.iter().enumerate() {
                        if mouse_pos.distance(*corner) <= handle_radius + 2.0 {
                            cursor = Some(resize_cursor_for_corner(ci, region.angle));
                            break 'outer;
                        }
                    }

                    // Check body
                    if region.contains_screen_point(mouse_pos, image_rect) {
                        cursor = Some(egui::CursorIcon::Grab);
                        break;
                    }
                }
                if let Some(c) = cursor {
                    ctx.set_cursor_icon(c);
                }
            }
        }

        // --- Drag start ---
        if response.drag_started() {
            if let Some(mouse_pos) = response.interact_pointer_pos() {
                let mut found = None;
                for (i, region) in self.split_state.regions.iter().enumerate().rev() {
                    let corners = region.corners_screen(image_rect);

                    // Rotation handle
                    let rot_pos = region.rotation_handle_screen(image_rect);
                    if mouse_pos.distance(rot_pos) <= rot_handle_radius + 4.0 {
                        found = Some((i, DragKind::Rotate));
                        break;
                    }

                    // Corner handles [TL, TR, BR, BL]
                    let corner_kinds = [
                        DragKind::ResizeTopLeft,
                        DragKind::ResizeTopRight,
                        DragKind::ResizeBottomRight,
                        DragKind::ResizeBottomLeft,
                    ];
                    let mut hit_corner = false;
                    for (ci, corner) in corners.iter().enumerate() {
                        if mouse_pos.distance(*corner) <= handle_radius + 2.0 {
                            found = Some((i, corner_kinds[ci]));
                            hit_corner = true;
                            break;
                        }
                    }
                    if hit_corner {
                        break;
                    }

                    // Body
                    if region.contains_screen_point(mouse_pos, image_rect) {
                        found = Some((i, DragKind::Move));
                        break;
                    }
                }
                self.split_state.dragging = found;
                self.split_state.selected = found.map(|(i, _)| i);
            }
        }

        // --- Dragging ---
        if response.dragged() {
            if let Some((idx, kind)) = self.split_state.dragging {
                if idx < self.split_state.regions.len() {
                    let iw = image_rect.width();
                    let ih = image_rect.height();
                    if iw > 0.0 && ih > 0.0 {
                        match kind {
                            DragKind::Move => {
                                let delta = response.drag_delta();
                                let dx = delta.x / iw;
                                let dy = delta.y / ih;
                                let region = &mut self.split_state.regions[idx];
                                region.cx += dx;
                                region.cy += dy;
                                region.clamp_to_image();
                                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                            }
                            DragKind::Rotate => {
                                if let Some(mouse_pos) = response.interact_pointer_pos() {
                                    let cx_s = image_rect.min.x + self.split_state.regions[idx].cx * iw;
                                    let cy_s = image_rect.min.y + self.split_state.regions[idx].cy * ih;
                                    let angle = (mouse_pos.x - cx_s).atan2(-(mouse_pos.y - cy_s));
                                    self.split_state.regions[idx].angle = angle;
                                    self.split_state.regions[idx].clamp_to_image();
                                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                                }
                            }
                            resize_kind => {
                                let delta = response.drag_delta();
                                let dx = delta.x / iw;
                                let dy = delta.y / ih;
                                let from_left = matches!(resize_kind, DragKind::ResizeTopLeft | DragKind::ResizeBottomLeft);
                                let from_top = matches!(resize_kind, DragKind::ResizeTopLeft | DragKind::ResizeTopRight);
                                self.resize_region(idx, dx, dy, from_left, from_top, image_rect);
                                let ci = match resize_kind {
                                    DragKind::ResizeTopLeft => 0,
                                    DragKind::ResizeTopRight => 1,
                                    DragKind::ResizeBottomRight => 2,
                                    DragKind::ResizeBottomLeft => 3,
                                    _ => 0,
                                };
                                ctx.set_cursor_icon(resize_cursor_for_corner(ci, self.split_state.regions[idx].angle));
                            }
                        }
                    }
                }
            }
        }

        if response.drag_stopped() {
            if self.split_state.dragging.is_some() {
                self.save_sidecar();
            }
            self.split_state.dragging = None;
        }
    }

    fn resize_region(
        &mut self,
        idx: usize,
        dx: f32,
        dy: f32,
        from_left: bool,
        from_top: bool,
        image_rect: egui::Rect,
    ) {
        let effective_ratio = self.split_state.format.ratio().map(|r| {
            if self.split_state.portrait { 1.0 / r } else { r }
        });
        let img_aspect = image_rect.width() / image_rect.height();

        let region = &mut self.split_state.regions[idx];

        // Transform the screen-space delta into the region's local (rotated) space
        let (sin_a, cos_a) = region.angle.sin_cos();
        let local_dx = dx * cos_a + dy * sin_a;
        let local_dy = -dx * sin_a + dy * cos_a;

        // Determine sign: left/top edges move opposite to right/bottom
        let sx = if from_left { -1.0 } else { 1.0 };
        let dw = sx * local_dx;

        let new_w = (region.w + dw).max(0.01);
        let actual_dw = new_w - region.w;

        // Shift center: when resizing from one side, center moves by half the delta
        // in the region's local coordinate system, then rotated back
        let shift_local_x = actual_dw / 2.0 * sx;
        region.w = new_w;

        if let Some(ratio) = effective_ratio {
            // Constrain aspect ratio: h = w * img_aspect / ratio
            let new_h = region.w * img_aspect / ratio;
            let actual_dh = new_h - region.h;
            let sy = if from_top { -1.0 } else { 1.0 };
            let shift_local_y = actual_dh / 2.0 * sy;
            region.h = new_h;
            // Rotate shift back to image coordinate system
            region.cx += shift_local_x * cos_a - shift_local_y * sin_a;
            region.cy += shift_local_x * sin_a + shift_local_y * cos_a;
        } else {
            let sy = if from_top { -1.0 } else { 1.0 };
            let dh_local = sy * local_dy;
            let new_h = (region.h + dh_local).max(0.01);
            let actual_dh = new_h - region.h;
            let shift_local_y = actual_dh / 2.0 * sy;
            region.h = new_h;
            region.cx += shift_local_x * cos_a - shift_local_y * sin_a;
            region.cy += shift_local_x * sin_a + shift_local_y * cos_a;
        }

        region.clamp_to_image();
    }

    fn export_split_regions(&mut self) {
        let s = self.s();
        let Some(detail) = &self.detail else { return };
        if self.split_state.regions.is_empty() {
            return;
        }

        let Some(out_dir) = rfd::FileDialog::new()
            .set_title("Select output directory")
            .pick_folder()
        else {
            return;
        };

        let src_path = detail.path.clone();
        let stem = src_path.file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        // Decode full-resolution image for export
        let img = match detail.tiff.decode_for_export() {
            Some(img) => img,
            None => {
                self.export_state.status = ExportStatus::Error("Failed to decode image".into());
                return;
            }
        };

        let img_w = img.width();
        let img_h = img.height();
        let mut exported = 0;

        for (i, region) in self.split_state.regions.iter().enumerate() {
            let name = self.split_state.naming_pattern
                .replace("{name}", &stem)
                .replace("{n}", &(i + 1).to_string());
            let out_path = out_dir.join(format!("{}.tif", name));

            // Guard: never overwrite source
            if let (Ok(src_c), Ok(dst_c)) = (src_path.canonicalize(), out_path.canonicalize()) {
                if src_c == dst_c {
                    self.export_state.status = ExportStatus::Error(s.cannot_overwrite_source.to_string());
                    return;
                }
            }

            let cropped = crop_rotated_region(&img, region, img_w, img_h);

            match cropped.save(&out_path) {
                Ok(()) => {
                    let cw = cropped.width();
                    let ch = cropped.height();
                    log::info!("Split export: {} ({}x{})", out_path.display(), cw, ch);
                    exported += 1;
                }
                Err(e) => {
                    log::error!("Split export failed: {} — {}", out_path.display(), e);
                    self.export_state.status = ExportStatus::Error(format!("{}: {}", out_path.display(), e));
                    return;
                }
            }
        }

        self.export_state.status = ExportStatus::Done {
            count: exported,
            dir: out_dir,
        };
        log::info!("Split export: {} files", exported);
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

// ─── Split overlay rendering ────────────────────────────────────────────────

fn draw_split_overlays(
    painter: &egui::Painter,
    image_rect: egui::Rect,
    regions: &[SplitRegion],
    selected_idx: Option<usize>,
) {
    let handle_size = 8.0_f32;
    let rot_handle_radius = 6.0_f32;

    for (i, region) in regions.iter().enumerate() {
        let corners = region.corners_screen(image_rect);
        let color = REGION_COLORS[i % REGION_COLORS.len()];
        let is_selected = selected_idx == Some(i);
        let stroke_width = if is_selected { 3.0 } else { 2.0 };

        // Semi-transparent fill (rotated quad)
        let fill = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 20);
        let mut quad_mesh = egui::Mesh::default();
        for c in &corners {
            quad_mesh.colored_vertex(*c, fill);
        }
        quad_mesh.add_triangle(0, 1, 2);
        quad_mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(quad_mesh));

        // Border (4 line segments)
        let stroke = egui::Stroke::new(stroke_width, color);
        for j in 0..4 {
            painter.line_segment([corners[j], corners[(j + 1) % 4]], stroke);
        }

        // Corner handles
        for corner in &corners {
            let handle_rect =
                egui::Rect::from_center_size(*corner, egui::vec2(handle_size, handle_size));
            painter.rect_filled(handle_rect, 2.0, color);
            painter.rect_stroke(
                handle_rect,
                2.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }

        // Rotation handle (circle above top edge)
        let rot_pos = region.rotation_handle_screen(image_rect);
        // Line from top-center to rotation handle
        let top_center = egui::pos2(
            (corners[0].x + corners[1].x) / 2.0,
            (corners[0].y + corners[1].y) / 2.0,
        );
        painter.line_segment(
            [top_center, rot_pos],
            egui::Stroke::new(1.5, color),
        );
        painter.circle_filled(rot_pos, rot_handle_radius, color);
        painter.circle_stroke(
            rot_pos,
            rot_handle_radius,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
        );
        // Rotation icon: small arc arrow
        painter.text(
            rot_pos,
            egui::Align2::CENTER_CENTER,
            "↻",
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );

        // Region number label (at first corner)
        let label_pos = egui::pos2(corners[0].x + 4.0, corners[0].y + 2.0);
        let galley = painter.layout_no_wrap(
            format!("#{}", i + 1),
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
        let label_rect = egui::Rect::from_min_size(
            label_pos,
            galley.size() + egui::vec2(6.0, 2.0),
        );
        painter.rect_filled(
            label_rect,
            3.0,
            egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 180),
        );
        painter.galley(label_pos + egui::vec2(3.0, 1.0), galley, egui::Color32::WHITE);
    }
}

/// Choose resize cursor based on the corner index and region rotation angle
fn resize_cursor_for_corner(corner_idx: usize, angle: f32) -> egui::CursorIcon {
    // Base diagonal angles for corners [TL, TR, BR, BL]
    let base_deg = [-135.0_f32, -45.0, 45.0, 135.0];
    let total = base_deg[corner_idx] + angle.to_degrees();
    // Normalize to [0, 180)
    let norm = ((total % 180.0) + 180.0) % 180.0;
    if norm < 22.5 || norm >= 157.5 {
        egui::CursorIcon::ResizeHorizontal
    } else if norm < 67.5 {
        egui::CursorIcon::ResizeNeSw
    } else if norm < 112.5 {
        egui::CursorIcon::ResizeVertical
    } else {
        egui::CursorIcon::ResizeNwSe
    }
}

/// Crop a rotated region from the source image using bilinear interpolation
fn crop_rotated_region(
    img: &image::DynamicImage,
    region: &SplitRegion,
    img_w: u32,
    img_h: u32,
) -> image::DynamicImage {
    let out_w = (region.w * img_w as f32).round() as u32;
    let out_h = (region.h * img_h as f32).round() as u32;
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);

    let cx_px = region.cx * img_w as f32;
    let cy_px = region.cy * img_h as f32;
    let (sin_a, cos_a) = region.angle.sin_cos();

    // For angle ≈ 0, use fast axis-aligned crop
    if region.angle.abs() < 0.001 {
        let px = ((cx_px - out_w as f32 / 2.0).round() as u32).min(img_w.saturating_sub(out_w));
        let py = ((cy_px - out_h as f32 / 2.0).round() as u32).min(img_h.saturating_sub(out_h));
        return img.crop_imm(px, py, out_w.min(img_w - px), out_h.min(img_h - py));
    }

    // Use 16-bit path if source is 16-bit
    match img {
        image::DynamicImage::ImageRgb16(src) => {
            let mut out = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::new(out_w, out_h);
            let hw = out_w as f32 / 2.0;
            let hh = out_h as f32 / 2.0;
            for oy in 0..out_h {
                for ox in 0..out_w {
                    let lx = ox as f32 - hw + 0.5;
                    let ly = oy as f32 - hh + 0.5;
                    let sx = cx_px + lx * cos_a - ly * sin_a;
                    let sy = cy_px + lx * sin_a + ly * cos_a;
                    let pixel = bilinear_sample_rgb16(src, sx, sy);
                    out.put_pixel(ox, oy, pixel);
                }
            }
            image::DynamicImage::ImageRgb16(out)
        }
        _ => {
            let src = img.to_rgb8();
            let mut out = image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::new(out_w, out_h);
            let hw = out_w as f32 / 2.0;
            let hh = out_h as f32 / 2.0;
            for oy in 0..out_h {
                for ox in 0..out_w {
                    let lx = ox as f32 - hw + 0.5;
                    let ly = oy as f32 - hh + 0.5;
                    let sx = cx_px + lx * cos_a - ly * sin_a;
                    let sy = cy_px + lx * sin_a + ly * cos_a;
                    let pixel = bilinear_sample_rgb8(&src, sx, sy);
                    out.put_pixel(ox, oy, pixel);
                }
            }
            image::DynamicImage::ImageRgb8(out)
        }
    }
}

fn bilinear_sample_rgb16(
    img: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    x: f32,
    y: f32,
) -> image::Rgb<u16> {
    let w = img.width() as f32;
    let h = img.height() as f32;
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return image::Rgb([0, 0, 0]);
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(img.width() - 1);
    let y1 = (y0 + 1).min(img.height() - 1);
    let fx = x - x.floor();
    let fy = y - y.floor();

    let p00 = img.get_pixel(x0, y0).0;
    let p10 = img.get_pixel(x1, y0).0;
    let p01 = img.get_pixel(x0, y1).0;
    let p11 = img.get_pixel(x1, y1).0;

    let mut out = [0u16; 3];
    for c in 0..3 {
        let v = p00[c] as f32 * (1.0 - fx) * (1.0 - fy)
            + p10[c] as f32 * fx * (1.0 - fy)
            + p01[c] as f32 * (1.0 - fx) * fy
            + p11[c] as f32 * fx * fy;
        out[c] = v.round().clamp(0.0, 65535.0) as u16;
    }
    image::Rgb(out)
}

fn bilinear_sample_rgb8(
    img: &image::ImageBuffer<image::Rgb<u8>, Vec<u8>>,
    x: f32,
    y: f32,
) -> image::Rgb<u8> {
    let w = img.width() as f32;
    let h = img.height() as f32;
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return image::Rgb([0, 0, 0]);
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(img.width() - 1);
    let y1 = (y0 + 1).min(img.height() - 1);
    let fx = x - x.floor();
    let fy = y - y.floor();

    let p00 = img.get_pixel(x0, y0).0;
    let p10 = img.get_pixel(x1, y0).0;
    let p01 = img.get_pixel(x0, y1).0;
    let p11 = img.get_pixel(x1, y1).0;

    let mut out = [0u8; 3];
    for c in 0..3 {
        let v = p00[c] as f32 * (1.0 - fx) * (1.0 - fy)
            + p10[c] as f32 * fx * (1.0 - fy)
            + p01[c] as f32 * (1.0 - fx) * fy
            + p11[c] as f32 * fx * fy;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    image::Rgb(out)
}

// ─── Utility functions ──────────────────────────────────────────────────────

fn scan_fff_files(dir: &Path, depth: DirScanDepth) -> Vec<PathBuf> {
    fn is_image_file(path: &Path) -> bool {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => matches!(ext.to_lowercase().as_str(), "fff" | "3fr" | "tif" | "tiff"),
            None => false,
        }
    }

    fn collect(dir: &Path, remaining: Option<usize>, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if is_image_file(&path) {
                    out.push(path);
                }
            } else if path.is_dir() {
                let is_hidden = path.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false);
                if !is_hidden {
                    match remaining {
                        Some(0) => {} // depth exhausted
                        Some(n) => collect(&path, Some(n - 1), out),
                        None => collect(&path, None, out), // unlimited
                    }
                }
            }
        }
    }

    let mut files = Vec::new();
    match depth {
        DirScanDepth::Flat => collect(dir, Some(0), &mut files),
        DirScanDepth::OneLevel => collect(dir, Some(1), &mut files),
        DirScanDepth::All => collect(dir, None, &mut files),
    }
    files
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
                let path = entry.path();
                // Skip symlinks — "Macintosh HD" is a symlink to "/" on macOS.
                // Real external / removable drives are actual directories.
                let is_symlink = path
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                // Skip hidden system volumes (e.g. .Spotlight-V100, .timemachine)
                let is_hidden = path
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false);
                // Skip volumes that are not readable (e.g. "Macintosh HD - Data"
                // has d--x--x--x permissions — only the kernel can list it).
                // External USB drives always have normal read permissions.
                let is_readable = std::fs::read_dir(&path).is_ok();
                if !is_symlink && !is_hidden && is_readable {
                    roots.push(path);
                }
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
