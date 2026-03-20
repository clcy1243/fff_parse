//! 查看器核心类型定义
//!
//! 定义查看器的所有数据结构、枚举和状态类型，包括视图模式、
//! 底片格式、分割区域、缩略图缓存、加载状态及主应用状态。

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

/// 显示预览的最大像素尺寸。
/// 超出此尺寸的图像在解码时进行降采样以提升速度。
/// 4096 在典型屏幕上画质良好，同时比全分辨率解码快约 25 倍。
pub(super) const DISPLAY_MAX_DIM: u32 = 4096;

// ─── 枚举 ───────────────────────────────────────────────────────────────────

/// 视图模式：网格缩略图或放大镜单图查看
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ViewMode {
    /// 网格缩略图模式
    Grid,
    /// 放大镜单图模式
    Loupe,
}

/// 直方图数据源：原始（未处理）或当前（已加载色彩方案后）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HistogramSource {
    /// 已加载色彩方案后的图像
    Processed,
    /// 未经任何色彩处理的原始图像
    Raw,
}

/// 每个直方图数据源独立保存的色阶参数（黑点、白点、Gamma）。
/// 切换数据源时，当前手柄状态保存到旧源，新源的手柄状态恢复到 `manual_adjust`。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HistogramLevels {
    /// 输入黑点：索引 0=总通道, 1=R, 2=G, 3=B
    pub black: [f32; 4],
    /// 中间调 Gamma：索引 0=总通道, 1=R, 2=G, 3=B
    pub gamma: [f32; 4],
    /// 输入白点：索引 0=总通道, 1=R, 2=G, 3=B
    pub white: [f32; 4],
}

impl Default for HistogramLevels {
    fn default() -> Self {
        Self {
            black: [0.0; 4],
            gamma: [1.0; 4],
            white: [255.0; 4],
        }
    }
}

/// 目录扫描深度，控制是否递归子目录查找图像文件
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DirScanDepth {
    /// 仅当前目录
    Flat,
    /// 包含一级子目录
    OneLevel,
    /// 递归所有子目录
    All,
}

impl DirScanDepth {
    /// 从 u8 值构造扫描深度
    pub(super) fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::OneLevel,
            2 => Self::All,
            _ => Self::Flat,
        }
    }
    /// 转换为 u8 值用于持久化存储
    pub(super) fn to_u8(self) -> u8 {
        match self {
            Self::Flat => 0,
            Self::OneLevel => 1,
            Self::All => 2,
        }
    }
    /// 循环切换到下一个深度级别
    pub(super) fn cycle(self) -> Self {
        match self {
            Self::Flat => Self::OneLevel,
            Self::OneLevel => Self::All,
            Self::All => Self::Flat,
        }
    }
    /// 目录树按钮上显示的简短标签
    pub(super) fn short_label(self) -> &'static str {
        match self {
            Self::Flat => "—",
            Self::OneLevel => "1",
            Self::All => "∞",
        }
    }
}

/// 右侧信息面板类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum InfoPanel {
    /// 元数据面板
    Metadata,
    /// 编辑历史面板
    EditHistory,
    /// 所有 TIFF 标签面板
    AllTags,
    /// 手动色彩调整面板
    ColorAdjust,
    /// ICC 色彩配置文件面板
    ColorProfile,
    /// 底片分割导出面板
    Split,
    /// 应用设置面板
    Settings,
}

// ─── 底片格式与导出 ─────────────────────────────────────────────────────────

/// 底片画幅格式，用于分割区域的宽高比约束
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
    /// 所有支持的画幅格式列表
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

    /// 返回格式的显示标签
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

    /// 返回横向宽高比（宽/高），自由格式返回 None
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

    /// 序列化为字符串用于持久化
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

    /// 从字符串反序列化，无法识别时返回 Free
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

/// 分割区域：描述图像上一个可旋转的矩形裁切框
#[derive(Clone)]
pub(super) struct SplitRegion {
    /// 中心点 X 坐标（归一化 0.0–1.0，相对于图像宽度）
    pub(super) cx: f32,
    /// 中心点 Y 坐标（归一化 0.0–1.0，相对于图像高度）
    pub(super) cy: f32,
    /// 宽度（归一化）
    pub(super) w: f32,
    /// 高度（归一化）
    pub(super) h: f32,
    /// 旋转角度（弧度，顺时针）
    pub(super) angle: f32,
}

impl SplitRegion {
    /// 计算屏幕坐标下的 4 个角点 [左上, 右上, 右下, 左下]
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

    /// 旋转手柄位置：位于顶部中心边缘上方的圆形
    pub(super) fn rotation_handle_screen(&self, image_rect: egui::Rect) -> egui::Pos2 {
        let cx_s = image_rect.min.x + self.cx * image_rect.width();
        let cy_s = image_rect.min.y + self.cy * image_rect.height();
        let hh = self.h * image_rect.height() / 2.0;
        let dist = hh + 22.0;
        let (sin_a, cos_a) = self.angle.sin_cos();
        // (0, -dist) rotated by angle
        egui::pos2(cx_s + dist * sin_a, cy_s - dist * cos_a)
    }

    /// 判断屏幕坐标点是否在旋转后的区域内
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

    /// 将区域约束在图像范围内，防止超出边界
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

/// 拖拽操作类型：移动、四角缩放、旋转
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DragKind {
    /// 整体移动
    Move,
    /// 左上角缩放
    ResizeTopLeft,
    /// 右上角缩放
    ResizeTopRight,
    /// 左下角缩放
    ResizeBottomLeft,
    /// 右下角缩放
    ResizeBottomRight,
    /// 旋转
    Rotate,
}

/// 分割区域的预设颜色列表，按索引循环使用
pub(super) const REGION_COLORS: &[egui::Color32] = &[
    egui::Color32::from_rgb(66, 133, 244),   // blue
    egui::Color32::from_rgb(234, 67, 53),    // red
    egui::Color32::from_rgb(52, 168, 83),    // green
    egui::Color32::from_rgb(251, 188, 4),    // yellow
    egui::Color32::from_rgb(171, 71, 188),   // purple
    egui::Color32::from_rgb(0, 188, 212),    // cyan
];

/// 底片分割状态：管理所有裁切区域及交互状态
pub(super) struct SplitState {
    /// 所有分割区域列表
    pub(super) regions: Vec<SplitRegion>,
    /// 当前选择的底片画幅格式
    pub(super) format: FilmFormat,
    /// 是否为竖向（纵向）
    pub(super) portrait: bool,
    /// 导出文件命名模式
    pub(super) naming_pattern: String,
    /// 当前正在拖拽的区域索引及拖拽类型
    pub(super) dragging: Option<(usize, DragKind)>,
    /// 当前选中的区域索引
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

// ─── 缩略图缓存条目 ─────────────────────────────────────────────────────────

/// 缩略图缓存条目：已上传到 GPU 的缩略图纹理及其尺寸
pub(super) struct ThumbEntry {
    pub(super) texture: egui::TextureHandle,
    pub(super) width: u32,
    pub(super) height: u32,
}

// ─── 选中文件的详细信息 ──────────────────────────────────────────────────────

/// 16-bit RGB 图像类型别名，用于内部管线全程保持 16-bit 精度
pub(super) type Rgb16Image = image::ImageBuffer<image::Rgb<u16>, Vec<u16>>;

/// 已加载的文件详情：包含解析后的 TIFF 数据、元数据、纹理和色彩信息
pub(super) struct LoadedDetail {
    pub(super) path: PathBuf,
    #[allow(dead_code)]
    pub(super) tiff: TiffFile,
    pub(super) metadata: Vec<(String, String)>,
    pub(super) all_tags: Vec<(String, String, String, String)>,
    pub(super) edit_history: Option<EditHistory>,
    pub(super) texture: Option<egui::TextureHandle>,
    pub(super) embedded_icc: Option<Vec<u8>>,
    /// 已处理的 16-bit 基准图像（色彩方案应用后），用于直方图计算和手动调整
    pub(super) base_rgb: Option<Rgb16Image>,
    /// 未经色彩处理的 16-bit 原始图像，用于原始直方图
    pub(super) raw_rgb: Option<Rgb16Image>,
}

// ─── 导出状态 ───────────────────────────────────────────────────────────────

/// 文件导出状态管理
pub(super) struct ExportState {
    pub(super) status: ExportStatus,
}

/// 导出状态枚举
#[derive(Debug, Clone)]
pub(super) enum ExportStatus {
    /// 空闲
    Idle,
    /// 正在导出中
    Exporting { current: usize, total: usize, current_name: String },
    /// 导出完成
    Done { count: usize, dir: PathBuf },
    /// 导出出错
    Error(String),
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            status: ExportStatus::Idle,
        }
    }
}

/// 导出处理管线参数：封装导出时需要应用的色彩处理步骤
pub(super) struct ExportPipeline {
    /// 胶片处理校正（负片反转 + 色阶）
    pub(super) correction: Option<fff_viewer::flexcolor::ImageCorrection>,
    /// ICC 配置文件数据
    pub(super) icc_data: Option<Vec<u8>>,
    /// 目标色彩空间
    pub(super) target_color_space: TargetColorSpace,
    /// 手动调整参数
    pub(super) manual_adjust: color::ManualAdjust,
}

// ─── 加载状态 ───────────────────────────────────────────────────────────────

/// 当前加载进度状态
pub(super) enum LoadingStatus {
    /// 空闲
    Idle,
    /// 正在加载缩略图
    LoadingThumbnails,
    /// 正在加载指定文件
    LoadingFile(String),
    /// 正在应用色彩配置
    ApplyingColorProfile,
}

impl Default for LoadingStatus {
    fn default() -> Self { Self::Idle }
}

// ─── 后台线程消息 ───────────────────────────────────────────────────────────

/// 后台线程加载缩略图的结果
pub(super) struct ThumbResult {
    pub(super) path: PathBuf,
    pub(super) rgba: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
}

/// 后台线程加载文件详情的结果。
/// 纹理无法在非主线程创建，因此传递原始图像数据。
pub(super) struct DetailResult {
    pub(super) path: PathBuf,
    pub(super) tiff: TiffFile,
    pub(super) metadata: Vec<(String, String)>,
    pub(super) all_tags: Vec<(String, String, String, String)>,
    pub(super) edit_history: Option<EditHistory>,
    /// 已处理的 16-bit 图像（色彩方案应用后）
    pub(super) preview_16: Option<image::DynamicImage>,
    /// 未经色彩处理的 16-bit 原始图像
    pub(super) raw_preview_16: Option<image::DynamicImage>,
    pub(super) embedded_icc: Option<Vec<u8>>,
    pub(super) auto_corrected: bool,
    pub(super) sidecar: Option<SidecarConfig>,
}

/// 文件详情加载消息
pub(super) enum DetailMsg {
    /// 加载成功
    Loaded(DetailResult),
    /// 加载失败
    Error(PathBuf, String),
}

// ─── 主应用状态 ─────────────────────────────────────────────────────────────

/// FFF 查看器主应用结构体，包含所有 UI 状态和数据
pub struct FffViewerApp {
    // 目录树
    pub(super) current_dir: Option<PathBuf>,
    pub(super) expanded_dirs: HashSet<PathBuf>,

    // 收藏夹（与 app_config.favorites 同步）
    pub(super) favorites: Vec<PathBuf>,

    // 当前目录中的文件列表
    pub(super) fff_files: Vec<PathBuf>,

    // 缩略图缓存
    pub(super) thumbnails: HashMap<PathBuf, ThumbEntry>,
    pub(super) thumb_rx: mpsc::Receiver<ThumbResult>,
    pub(super) thumb_tx: mpsc::Sender<ThumbResult>,
    pub(super) thumb_pending: usize,

    // 视图状态
    pub(super) view_mode: ViewMode,
    pub(super) selected_index: Option<usize>,

    // 选中文件的详情
    pub(super) detail: Option<LoadedDetail>,
    pub(super) detail_rx: mpsc::Receiver<DetailMsg>,
    pub(super) detail_tx: mpsc::Sender<DetailMsg>,

    // 右侧面板
    pub(super) info_panel: InfoPanel,
    pub(super) manual_adjust: color::ManualAdjust,
    pub(super) histogram: Option<Box<[[u32; 256]; 4]>>,
    pub(super) histogram_needs_update: bool,
    pub(super) histogram_source: HistogramSource,
    /// 处理后数据源的色阶手柄状态
    pub(super) levels_processed: HistogramLevels,
    /// 原始数据源的色阶手柄状态
    pub(super) levels_raw: HistogramLevels,
    pub(super) tag_filter: String,
    pub(super) expanded_setting: Option<usize>,

    // 文件列表过滤
    pub(super) file_filter: String,

    // 编辑/导出状态
    pub(super) export_state: ExportState,

    // 色彩管理
    pub(super) available_profiles: Vec<IccProfileInfo>,
    pub(super) available_presets: Vec<SettingsPreset>,
    pub(super) selected_input_profile: Option<usize>,
    pub(super) selected_preset: Option<usize>,
    pub(super) use_embedded_icc: bool,
    pub(super) use_embedded_correction: bool,
    pub(super) preset_category_filter: String,
    pub(super) color_status: Option<String>,
    pub(super) target_color_space: TargetColorSpace,

    // 底片分割与导出
    pub(super) split_state: SplitState,

    // 加载进度
    pub(super) loading_status: LoadingStatus,

    // 错误信息
    pub(super) error_msg: Option<String>,

    // UI 开关
    pub(super) show_info_panel: bool,

    // 界面语言
    pub(super) language: Language,

    // 应用配置（用于设置面板）
    pub(super) app_config: AppConfig,
    pub(super) settings_needs_restart: bool,
}

// ─── 字体加载 ───────────────────────────────────────────────────────────────

/// 加载 CJK 字体，确保中日韩文字正常显示。
/// 从系统字体路径查找并加载，作为 egui 的后备字体。
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
