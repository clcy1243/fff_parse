//! 全局应用配置，存储于 `~/fff_parse/config/settings.xml`。
//!
//! 首次启动时自动创建默认配置文件。
//! 配置项包括 GPU 加速、渲染线程数和界面语言。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

// ─── Directory layout ───────────────────────────────────────────────────────

/// 根数据目录：`~/fff_parse/`（macOS/Linux）或 `%USERPROFILE%\fff_parse\`（Windows）
pub fn app_data_dir() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join("fff_parse")
}

/// 获取用户主目录。Windows 优先使用 USERPROFILE，其次 APPDATA 的父目录；
/// macOS/Linux 使用 HOME。
fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(p) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(p));
        }
        if let Some(p) = std::env::var_os("APPDATA") {
            let p = PathBuf::from(p);
            return p.parent().map(|pp| pp.to_path_buf());
        }
    }
    std::env::var_os("HOME").map(PathBuf::from)
}

/// 日志目录：`~/fff_parse/logs/`
pub fn logs_dir() -> PathBuf {
    app_data_dir().join("logs")
}

/// 配置目录：`~/fff_parse/config/`
pub fn config_dir() -> PathBuf {
    app_data_dir().join("config")
}

/// Sidecar 目录：`~/fff_parse/sidecar/`
pub fn sidecar_dir() -> PathBuf {
    app_data_dir().join("sidecar")
}

/// 确保所有数据目录存在，不存在则自动创建。
pub fn ensure_dirs() {
    for dir in [app_data_dir(), logs_dir(), config_dir(), sidecar_dir()] {
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
        }
    }
}

/// 全局配置文件路径。
fn settings_path() -> PathBuf {
    config_dir().join("settings.xml")
}

// ─── Configuration struct ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
/// 应用全局配置。
pub struct AppConfig {
    /// 是否启用 GPU 加速渲染（默认：true）
    pub gpu_enabled: bool,
    /// 首选 GPU 设备名称（空字符串表示自动选择）
    pub gpu_device: String,
    /// 渲染/处理线程数（默认：CPU 核心数 / 4，最小为 1）
    pub render_threads: usize,
    /// 界面语言："en" 或 "zh"
    pub language: String,
    /// 收藏的目录路径列表（绝对路径，XML 中以 `|` 分隔）
    pub favorites: Vec<String>,
    /// 各目录的扫描深度：路径 → 0（仅当前层）、1（一层子目录）、2（所有子目录）
    pub dir_scan_modes: std::collections::HashMap<String, u8>,
    /// 自动色阶黑点裁切百分比（默认 0.05%，适合扫描图像的低噪声特性）
    pub auto_levels_black_pct: f32,
    /// 自动色阶白点裁切百分比（默认 0.1%，去除灰尘/划痕等亮点噪声）
    pub auto_levels_white_pct: f32,
    /// 直方图纵轴映射方式（默认：平方根）
    pub histogram_scale: String,
}

/// 直方图纵轴映射方式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HistogramScale {
    /// 线性：count / max — 忠实反映像素数量比例，低频 bin 几乎不可见
    Linear,
    /// 平方根：√count / √max — 适中压缩，业界最常用（Photoshop）
    Sqrt,
    /// 对数：ln(1+count) / ln(1+max) — 强压缩，低频 bin 清晰可见
    Log,
    /// 立方根：∛count / ∛max — 介于平方根和对数之间
    Cbrt,
}

impl HistogramScale {
    pub const ALL: &[Self] = &[Self::Linear, Self::Sqrt, Self::Log, Self::Cbrt];

    pub fn to_str(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Sqrt => "sqrt",
            Self::Log => "log",
            Self::Cbrt => "cbrt",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "linear" => Self::Linear,
            "log" => Self::Log,
            "cbrt" => Self::Cbrt,
            _ => Self::Sqrt,
        }
    }

    /// 将像素计数映射为 0-1 范围的显示高度
    pub fn map(self, count: f32, max: f32) -> f32 {
        if max <= 0.0 { return 0.0; }
        match self {
            Self::Linear => count / max,
            Self::Sqrt   => count.sqrt() / max.sqrt(),
            Self::Log    => count.ln_1p() / max.ln_1p(),
            Self::Cbrt   => count.cbrt() / max.cbrt(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gpu_enabled: true,
            gpu_device: String::new(),
            render_threads: default_thread_count(),
            language: detect_system_language(),
            favorites: Vec::new(),
            dir_scan_modes: std::collections::HashMap::new(),
            auto_levels_black_pct: 0.05,
            auto_levels_white_pct: 0.1,
            histogram_scale: "sqrt".to_string(),
        }
    }
}

/// 默认线程数：CPU 核心数 / 4，最小为 1。
fn default_thread_count() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cpus / 4).max(1)
}

/// 检测系统语言。若为中文环境则返回 "zh"，否则返回 "en"。
fn detect_system_language() -> String {
    // 通用：检查 LANG / LC_ALL / LC_MESSAGES 环境变量
    for var in ["LANG", "LC_ALL", "LC_MESSAGES"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_lowercase();
            if lower.starts_with("zh") {
                return "zh".to_string();
            }
            if !lower.is_empty() && lower != "c" && lower != "posix" {
                return "en".to_string();
            }
        }
    }

    // macOS: try AppleLocale
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
        {
            let locale = String::from_utf8_lossy(&output.stdout).to_lowercase();
            if locale.starts_with("zh") {
                return "zh".to_string();
            }
        }
    }

    // Windows: 通过 PowerShell 获取用户语言
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "(Get-Culture).Name"])
            .output()
        {
            let locale = String::from_utf8_lossy(&output.stdout).to_lowercase();
            if locale.starts_with("zh") {
                return "zh".to_string();
            }
        }
    }

    "en".to_string()
}

// ─── Load / Save ────────────────────────────────────────────────────────────

/// 从磁盘加载配置，若文件不存在或损坏则创建默认配置。
pub fn load_or_create() -> AppConfig {
    ensure_dirs();
    let path = settings_path();
    if path.exists() {
        if let Some(config) = load_from_file(&path) {
            return config;
        }
    }
    // First launch or corrupt file — create defaults
    let config = AppConfig::default();
    let _ = save(&config);
    config
}

/// 将配置保存到磁盘。
pub fn save(config: &AppConfig) -> Result<(), String> {
    ensure_dirs();
    let path = settings_path();
    let xml = to_xml(config);
    std::fs::write(&path, xml.as_bytes()).map_err(|e| format!("{}: {}", path.display(), e))
}

/// 从指定文件路径加载配置。
fn load_from_file(path: &Path) -> Option<AppConfig> {
    let xml = std::fs::read_to_string(path).ok()?;
    parse_xml(&xml)
}

// ─── XML serialization ─────────────────────────────────────────────────────

/// 将配置序列化为 XML 字符串。
fn to_xml(c: &AppConfig) -> String {
    let mut s = String::with_capacity(512);
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<fff_viewer_config>\n");
    let _ = writeln!(s, "  <gpu_enabled>{}</gpu_enabled>", c.gpu_enabled);
    let _ = writeln!(s, "  <gpu_device>{}</gpu_device>", xml_escape(&c.gpu_device));
    let _ = writeln!(s, "  <render_threads>{}</render_threads>", c.render_threads);
    let _ = writeln!(s, "  <language>{}</language>", xml_escape(&c.language));
    let favorites_str = c.favorites.iter().map(|f| xml_escape(f)).collect::<Vec<_>>().join("|");
    let _ = writeln!(s, "  <favorites>{}</favorites>", favorites_str);
    // dir_scan_modes: "path:depth" pairs joined by "|"
    let modes_str = c.dir_scan_modes.iter()
        .map(|(k, v)| format!("{}:{}", xml_escape(k), v))
        .collect::<Vec<_>>().join("|");
    let _ = writeln!(s, "  <dir_scan_modes>{}</dir_scan_modes>", modes_str);
    let _ = writeln!(s, "  <auto_levels_black_pct>{}</auto_levels_black_pct>", c.auto_levels_black_pct);
    let _ = writeln!(s, "  <auto_levels_white_pct>{}</auto_levels_white_pct>", c.auto_levels_white_pct);
    let _ = writeln!(s, "  <histogram_scale>{}</histogram_scale>", xml_escape(&c.histogram_scale));
    s.push_str("</fff_viewer_config>\n");
    s
}

/// 解析 XML 字符串为配置结构体。
fn parse_xml(xml: &str) -> Option<AppConfig> {
    if !xml.contains("<fff_viewer_config>") {
        return None;
    }
    let mut config = AppConfig::default();

    if let Some(v) = tag_content(xml, "gpu_enabled") {
        config.gpu_enabled = v == "true";
    }
    if let Some(v) = tag_content(xml, "gpu_device") {
        config.gpu_device = xml_unescape(&v);
    }
    if let Some(v) = tag_content(xml, "render_threads") {
        if let Ok(n) = v.parse::<usize>() {
            config.render_threads = n.max(1);
        }
    }
    if let Some(v) = tag_content(xml, "language") {
        config.language = xml_unescape(&v);
    }
    if let Some(v) = tag_content(xml, "favorites") {
        if !v.is_empty() {
            config.favorites = v.split('|').map(|s| xml_unescape(s.trim())).filter(|s| !s.is_empty()).collect();
        }
    }
    if let Some(v) = tag_content(xml, "dir_scan_modes") {
        if !v.is_empty() {
            for pair in v.split('|') {
                // Split on last ':' so paths with ':' (Windows drive letters) still work
                if let Some(colon) = pair.rfind(':') {
                    let path = xml_unescape(pair[..colon].trim());
                    if let Ok(depth) = pair[colon+1..].trim().parse::<u8>() {
                        config.dir_scan_modes.insert(path, depth.min(2));
                    }
                }
            }
        }
    }
    if let Some(v) = tag_content(xml, "auto_levels_black_pct") {
        if let Ok(n) = v.parse::<f32>() {
            config.auto_levels_black_pct = n.clamp(0.0, 5.0);
        }
    }
    if let Some(v) = tag_content(xml, "auto_levels_white_pct") {
        if let Ok(n) = v.parse::<f32>() {
            config.auto_levels_white_pct = n.clamp(0.0, 5.0);
        }
    }
    if let Some(v) = tag_content(xml, "histogram_scale") {
        config.histogram_scale = xml_unescape(&v);
    }
    Some(config)
}

/// 转义 XML 特殊字符（`&`、`<`、`>`、`"`）。
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 还原 XML 转义实体为原始字符。
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// 提取 XML 标签之间的文本内容。
fn tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut dir_scan_modes = std::collections::HashMap::new();
        dir_scan_modes.insert("/Users/test/Photos".to_string(), 2u8);
        let config = AppConfig {
            gpu_enabled: false,
            gpu_device: "AMD Radeon Pro 5500M".to_string(),
            render_threads: 4,
            language: "zh".to_string(),
            favorites: vec!["/Users/test/Photos".to_string(), "/Volumes/SD".to_string()],
            dir_scan_modes,
            ..Default::default()
        };
        let xml = to_xml(&config);
        let parsed = parse_xml(&xml).unwrap();
        assert_eq!(parsed.gpu_enabled, false);
        assert_eq!(parsed.gpu_device, "AMD Radeon Pro 5500M");
        assert_eq!(parsed.render_threads, 4);
        assert_eq!(parsed.language, "zh");
        assert_eq!(parsed.favorites, vec!["/Users/test/Photos", "/Volumes/SD"]);
        assert_eq!(parsed.dir_scan_modes.get("/Users/test/Photos"), Some(&2u8));
    }

    #[test]
    fn default_threads_at_least_one() {
        let t = default_thread_count();
        assert!(t >= 1);
    }

    #[test]
    fn detect_language_returns_valid() {
        let lang = detect_system_language();
        assert!(lang == "en" || lang == "zh");
    }
}
