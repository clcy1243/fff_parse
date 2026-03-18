use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;

use fff_viewer::color::{self, IccProfileInfo, SettingsPreset, TargetColorSpace};
use fff_viewer::config::AppConfig;
use fff_viewer::flexcolor::EditHistory;
use fff_viewer::i18n::Language;
use fff_viewer::sidecar::SidecarConfig;
use fff_viewer::tiff::TiffFile;

/// Maximum pixel dimension for display preview.
/// Larger images are subsampled during decode for speed.
/// 4096 gives good quality for typical screen sizes while being ~25× faster
/// than full-resolution decode of large scanner images.
pub(super) const DISPLAY_MAX_DIM: u32 = 4096;

// ─── Enums ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ViewMode {
    Grid,
    Loupe,
}

/// Per-directory subdirectory scan depth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DirScanDepth {
    Flat,     // 0 — current folder only
    OneLevel, // 1 — one level of subdirectories
    All,      // 2 — all subdirectories recursively
}

impl DirScanDepth {
    pub(super) fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::OneLevel,
            2 => Self::All,
            _ => Self::Flat,
        }
    }
    pub(super) fn to_u8(self) -> u8 {
        match self {
            Self::Flat => 0,
            Self::OneLevel => 1,
            Self::All => 2,
        }
    }
    pub(super) fn cycle(self) -> Self {
        match self {
            Self::Flat => Self::OneLevel,
            Self::OneLevel => Self::All,
            Self::All => Self::Flat,
        }
    }
    /// Short label shown in the tree button
    pub(super) fn short_label(self) -> &'static str {
        match self {
            Self::Flat => "—",
            Self::OneLevel => "1",
            Self::All => "∞",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum InfoPanel {
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
pub(super) enum FilmFormat {
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
    pub(super) const ALL: &[Self] = &[
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

    pub(super) fn label(&self) -> &'static str {
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
    pub(super) fn ratio(&self) -> Option<f32> {
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

    pub(super) fn to_str(&self) -> &'static str {
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

    pub(super) fn from_str(s: &str) -> Self {
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
pub(super) struct SplitRegion {
    /// Center coordinates (normalized 0.0–1.0 relative to image dimensions)
    pub(super) cx: f32,
    pub(super) cy: f32,
    /// Half-extents (normalized)
    pub(super) w: f32,
    pub(super) h: f32,
    /// Rotation angle in radians (clockwise)
    pub(super) angle: f32,
}

impl SplitRegion {
    /// Get the 4 corners in screen coordinates [TL, TR, BR, BL]
    pub(super) fn corners_screen(&self, image_rect: egui::Rect) -> [egui::Pos2; 4] {
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
    pub(super) fn rotation_handle_screen(&self, image_rect: egui::Rect) -> egui::Pos2 {
        let cx_s = image_rect.min.x + self.cx * image_rect.width();
        let cy_s = image_rect.min.y + self.cy * image_rect.height();
        let hh = self.h * image_rect.height() / 2.0;
        let dist = hh + 22.0;
        let (sin_a, cos_a) = self.angle.sin_cos();
        // (0, -dist) rotated by angle
        egui::pos2(cx_s + dist * sin_a, cy_s - dist * cos_a)
    }

    /// Check if a screen-space point is inside the rotated region
    pub(super) fn contains_screen_point(&self, point: egui::Pos2, image_rect: egui::Rect) -> bool {
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

    pub(super) fn clamp_to_image(&mut self) {
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
pub(super) enum DragKind {
    Move,
    ResizeTopLeft,
    ResizeTopRight,
    ResizeBottomLeft,
    ResizeBottomRight,
    Rotate,
}

pub(super) const REGION_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(66, 133, 244),   // blue
    egui::Color32::from_rgb(234, 67, 53),    // red
    egui::Color32::from_rgb(52, 168, 83),    // green
    egui::Color32::from_rgb(251, 188, 4),    // yellow
    egui::Color32::from_rgb(171, 71, 188),   // purple
    egui::Color32::from_rgb(0, 188, 212),    // cyan
];

pub(super) struct SplitState {
    pub(super) regions: Vec<SplitRegion>,
    pub(super) format: FilmFormat,
    pub(super) portrait: bool,
    pub(super) naming_pattern: String,
    pub(super) dragging: Option<(usize, DragKind)>,
    pub(super) selected: Option<usize>,
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

pub(super) struct ThumbEntry {
    pub(super) texture: egui::TextureHandle,
    pub(super) width: u32,
    pub(super) height: u32,
}

// ─── Loaded detail for the selected file ────────────────────────────────────

pub(super) struct LoadedDetail {
    pub(super) path: PathBuf,
    #[allow(dead_code)]
    pub(super) tiff: TiffFile,
    pub(super) metadata: Vec<(String, String)>,
    pub(super) all_tags: Vec<(String, String, String, String)>,
    pub(super) edit_history: Option<EditHistory>,
    pub(super) texture: Option<egui::TextureHandle>,
    pub(super) embedded_icc: Option<Vec<u8>>,
    pub(super) base_rgb: Option<image::RgbImage>,
}

// ─── Export state ───────────────────────────────────────────────────────────

pub(super) struct ExportState {
    pub(super) status: ExportStatus,
}

#[derive(Debug, Clone)]
pub(super) enum ExportStatus {
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

pub(super) enum LoadingStatus {
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
pub(super) struct ThumbResult {
    pub(super) path: PathBuf,
    pub(super) rgba: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
}

/// Result of loading a full detail file on a background thread.
/// Textures cannot be created off the main thread, so we send raw image data.
pub(super) struct DetailResult {
    pub(super) path: PathBuf,
    pub(super) tiff: TiffFile,
    pub(super) metadata: Vec<(String, String)>,
    pub(super) all_tags: Vec<(String, String, String, String)>,
    pub(super) edit_history: Option<EditHistory>,
    pub(super) preview_rgba: Option<(Vec<u8>, u32, u32)>, // (pixels, width, height)
    pub(super) embedded_icc: Option<Vec<u8>>,
    pub(super) auto_corrected: bool, // true if embedded correction was auto-applied
    pub(super) sidecar: Option<SidecarConfig>, // persisted settings from XML sidecar
}

pub(super) enum DetailMsg {
    Loaded(DetailResult),
    Error(PathBuf, String),
}

// ─── App state ──────────────────────────────────────────────────────────────

pub struct FffViewerApp {
    // Directory tree
    pub(super) current_dir: Option<PathBuf>,
    pub(super) expanded_dirs: HashSet<PathBuf>,

    // Favorites (synced with app_config.favorites)
    pub(super) favorites: Vec<PathBuf>,

    // File list in current directory
    pub(super) fff_files: Vec<PathBuf>,

    // Thumbnails cache
    pub(super) thumbnails: HashMap<PathBuf, ThumbEntry>,
    pub(super) thumb_rx: mpsc::Receiver<ThumbResult>,
    pub(super) thumb_tx: mpsc::Sender<ThumbResult>,
    pub(super) thumb_pending: usize,

    // View state
    pub(super) view_mode: ViewMode,
    pub(super) selected_index: Option<usize>,

    // Detail of selected file
    pub(super) detail: Option<LoadedDetail>,
    pub(super) detail_rx: mpsc::Receiver<DetailMsg>,
    pub(super) detail_tx: mpsc::Sender<DetailMsg>,

    // Right panel
    pub(super) info_panel: InfoPanel,
    pub(super) manual_adjust: color::ManualAdjust,
    pub(super) histogram: Option<Box<[[u32; 256]; 4]>>,
    pub(super) histogram_needs_update: bool,
    pub(super) tag_filter: String,
    pub(super) expanded_setting: Option<usize>,

    // File list filter
    pub(super) file_filter: String,

    // Editing state
    pub(super) export_state: ExportState,

    // Color management
    pub(super) available_profiles: Vec<IccProfileInfo>,
    pub(super) available_presets: Vec<SettingsPreset>,
    pub(super) selected_input_profile: Option<usize>,
    pub(super) selected_preset: Option<usize>,
    pub(super) use_embedded_icc: bool,
    pub(super) use_embedded_correction: bool,
    pub(super) preset_category_filter: String,
    pub(super) color_status: Option<String>,
    pub(super) target_color_space: TargetColorSpace,

    // Split & export
    pub(super) split_state: SplitState,

    // Loading progress
    pub(super) loading_status: LoadingStatus,

    // Error
    pub(super) error_msg: Option<String>,

    // UI toggles
    pub(super) show_info_panel: bool,

    // Language
    pub(super) language: Language,

    // App config (for settings panel)
    pub(super) app_config: AppConfig,
    pub(super) settings_needs_restart: bool,
}

// ─── Font loading ───────────────────────────────────────────────────────────

pub(super) fn setup_cjk_fonts(ctx: &egui::Context) {
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
