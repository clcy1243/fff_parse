//! Sidecar XML 文件：为每个源文件持久化颜色设置和分割区域。
//!
//! 每个源文件对应的 sidecar 存储在
//! `~/fff_parse/sidecar/{路径哈希}.xml`。
//!
//! # 新语义（T58，2026-05-01）
//!
//! 历史上 sidecar 保存 ManualAdjust **全部字段**（~50 项），加载时整体覆盖。
//! 这导致：
//! 1. 切换色彩方案（preset）后保留旧的"全量快照"，preset 变化无法生效
//! 2. sidecar XML 体积大，dirty-diff 不直观
//!
//! 新流程：
//! - **保存**：`ManualAdjust` 字段**与 baseline（preset 加载后的状态）做 diff**，
//!   仅写不同的字段到 sidecar
//! - **加载**：先 apply preset/embedded 得到 baseline，再把 sidecar 里 `Some(...)`
//!   的字段**叠加**到 `manual_adjust`
//! - **重置**：删除 sidecar 文件 + 恢复 baseline
//!
//! `SidecarConfig` 因此包含：
//! - 色彩方案选择（preset_name/input_profile_name/embedded_*/target_color_space）
//!   —— 总是全量写，这些决定 baseline
//! - `SidecarAdjustOverrides` —— 用 `Option<T>` 表示"仅此字段被显式覆盖"
//! - `curve_points_override: Option<Vec<Vec<(i64,i64,i64)>>>` —— 7 条曲线只要任一偏离
//!   默认 identity，就全量写 7 条
//! - split_regions —— 无 baseline 概念，总是全量写

use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::color::{self, ManualAdjust};
use crate::config;

/// 需要跨会话持久化的单文件配置。
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    // Color settings
    pub use_embedded_correction: bool,
    pub use_embedded_icc: bool,
    /// 选中的内嵌编辑历史索引
    pub embedded_correction_index: Option<usize>,
    pub input_profile_name: Option<String>,
    pub preset_name: Option<String>,
    pub target_color_space: String,

    // Split settings
    pub split_format: String,
    pub split_portrait: bool,
    pub split_naming_pattern: String,
    pub split_regions: Vec<SidecarRegion>,

    /// ManualAdjust 的 overrides（只包含与 baseline 不同的字段）
    pub adjust: SidecarAdjustOverrides,

    /// 7 条渐变曲线（Master/R-A/G-A/B-A/R-B/G-B/B-B）。
    /// 若任一条偏离 identity 默认，则整体存入 `Some`。
    pub curve_points_override: Option<Vec<Vec<(i64, i64, i64)>>>,
}

impl SidecarConfig {
    /// 空 config（无任何改动）
    pub fn empty() -> Self {
        Self {
            use_embedded_correction: false,
            use_embedded_icc: false,
            embedded_correction_index: None,
            input_profile_name: None,
            preset_name: None,
            target_color_space: "ProPhotoRGB".to_string(),
            split_format: "Free".to_string(),
            split_portrait: false,
            split_naming_pattern: "{name}_{n}".to_string(),
            split_regions: Vec::new(),
            adjust: SidecarAdjustOverrides::default(),
            curve_points_override: None,
        }
    }
}

/// ManualAdjust 的部分覆盖：每个字段 `Option<T>`，只有 `Some` 的字段会被写入/应用。
#[derive(Debug, Clone, Default)]
pub struct SidecarAdjustOverrides {
    pub film_type: Option<i64>,
    pub film_curve: Option<i64>,
    pub film_gamma: Option<f64>,

    pub apply_levels: Option<bool>,
    pub apply_curves: Option<bool>,
    pub apply_exposure: Option<bool>,
    pub apply_brightness: Option<bool>,
    pub apply_shadow_depth: Option<bool>,
    pub apply_midtone: Option<bool>,
    pub apply_contrast: Option<bool>,
    pub apply_highlights: Option<bool>,
    pub apply_shadows: Option<bool>,
    pub apply_saturation: Option<bool>,
    pub apply_color_balance: Option<bool>,
    pub apply_color_temp: Option<bool>,
    pub apply_color_corr: Option<bool>,
    pub apply_film_curve: Option<bool>,

    pub exposure: Option<f32>,
    pub brightness: Option<f32>,
    pub lightness: Option<f32>,
    pub midtone: Option<f32>,
    pub contrast: Option<f32>,
    pub highlights: Option<f32>,
    pub shadows: Option<f32>,
    pub saturation: Option<f32>,
    pub color_temperature: Option<f32>,
    pub tint: Option<f32>,
    pub r_shift: Option<f32>,
    pub g_shift: Option<f32>,
    pub b_shift: Option<f32>,

    pub levels_black: Option<[f32; 4]>,
    pub levels_gamma: Option<[f32; 4]>,
    pub levels_white: Option<[f32; 4]>,
    pub output_shadow: Option<[f32; 4]>,
    pub output_highlight: Option<[f32; 4]>,

    pub color_corr: Option<[i64; 36]>,

    pub apply_usm: Option<bool>,
    pub usm_amount: Option<i64>,
    pub usm_radius: Option<i64>,
    pub usm_dark_limit: Option<i64>,
    pub usm_noise_limit: Option<i64>,
    pub usm_col_factor: Option<[i64; 3]>,

    pub apply_dust: Option<bool>,
    pub dust_level: Option<i64>,

    pub apply_cn_filter: Option<bool>,
    pub color_noise_radius: Option<i64>,
    pub noise_filter_bias: Option<i64>,

    pub lens_correction: Option<i64>,
    pub vignette_amount: Option<i64>,

    pub enhanced_shadow: Option<bool>,
    pub remove_cast_highlight: Option<bool>,
    pub remove_cast_shadow: Option<bool>,
}

impl SidecarAdjustOverrides {
    /// 计算 diff：对每个字段，若 current != baseline 则置 `Some(current)`。
    pub fn from_diff(current: &ManualAdjust, baseline: &ManualAdjust) -> Self {
        macro_rules! diff {
            ($field:ident) => {
                if current.$field != baseline.$field {
                    Some(current.$field.clone())
                } else {
                    None
                }
            };
        }
        Self {
            film_type: diff!(film_type),
            film_curve: diff!(film_curve),
            film_gamma: diff!(film_gamma),
            apply_levels: diff!(apply_levels),
            apply_curves: diff!(apply_curves),
            apply_exposure: diff!(apply_exposure),
            apply_brightness: diff!(apply_brightness),
            apply_shadow_depth: diff!(apply_shadow_depth),
            apply_midtone: diff!(apply_midtone),
            apply_contrast: diff!(apply_contrast),
            apply_highlights: diff!(apply_highlights),
            apply_shadows: diff!(apply_shadows),
            apply_saturation: diff!(apply_saturation),
            apply_color_balance: diff!(apply_color_balance),
            apply_color_temp: diff!(apply_color_temp),
            apply_color_corr: diff!(apply_color_corr),
            apply_film_curve: diff!(apply_film_curve),
            exposure: diff!(exposure),
            brightness: diff!(brightness),
            lightness: diff!(lightness),
            midtone: diff!(midtone),
            contrast: diff!(contrast),
            highlights: diff!(highlights),
            shadows: diff!(shadows),
            saturation: diff!(saturation),
            color_temperature: diff!(color_temperature),
            tint: diff!(tint),
            r_shift: diff!(r_shift),
            g_shift: diff!(g_shift),
            b_shift: diff!(b_shift),
            levels_black: diff!(levels_black),
            levels_gamma: diff!(levels_gamma),
            levels_white: diff!(levels_white),
            output_shadow: diff!(output_shadow),
            output_highlight: diff!(output_highlight),
            color_corr: diff!(color_corr),
            apply_usm: diff!(apply_usm),
            usm_amount: diff!(usm_amount),
            usm_radius: diff!(usm_radius),
            usm_dark_limit: diff!(usm_dark_limit),
            usm_noise_limit: diff!(usm_noise_limit),
            usm_col_factor: if current.usm_col_factor != baseline.usm_col_factor {
                Some(current.usm_col_factor)
            } else {
                None
            },
            apply_dust: diff!(apply_dust),
            dust_level: diff!(dust_level),
            apply_cn_filter: diff!(apply_cn_filter),
            color_noise_radius: diff!(color_noise_radius),
            noise_filter_bias: diff!(noise_filter_bias),
            lens_correction: diff!(lens_correction),
            vignette_amount: diff!(vignette_amount),
            enhanced_shadow: diff!(enhanced_shadow),
            remove_cast_highlight: diff!(remove_cast_highlight),
            remove_cast_shadow: diff!(remove_cast_shadow),
        }
    }

    /// 把 `Some(...)` 的字段叠加到 target。
    pub fn apply_to(&self, target: &mut ManualAdjust) {
        macro_rules! apply {
            ($field:ident) => {
                if let Some(v) = self.$field.clone() { target.$field = v; }
            };
        }
        apply!(film_type);
        apply!(film_curve);
        apply!(film_gamma);
        apply!(apply_levels);
        apply!(apply_curves);
        apply!(apply_exposure);
        apply!(apply_brightness);
        apply!(apply_shadow_depth);
        apply!(apply_midtone);
        apply!(apply_contrast);
        apply!(apply_highlights);
        apply!(apply_shadows);
        apply!(apply_saturation);
        apply!(apply_color_balance);
        apply!(apply_color_temp);
        apply!(apply_color_corr);
        apply!(apply_film_curve);
        apply!(exposure);
        apply!(brightness);
        apply!(lightness);
        apply!(midtone);
        apply!(contrast);
        apply!(highlights);
        apply!(shadows);
        apply!(saturation);
        apply!(color_temperature);
        apply!(tint);
        apply!(r_shift);
        apply!(g_shift);
        apply!(b_shift);
        apply!(levels_black);
        apply!(levels_gamma);
        apply!(levels_white);
        apply!(output_shadow);
        apply!(output_highlight);
        apply!(color_corr);
        apply!(apply_usm);
        apply!(usm_amount);
        apply!(usm_radius);
        apply!(usm_dark_limit);
        apply!(usm_noise_limit);
        if let Some(cf) = self.usm_col_factor {
            target.usm_col_factor = cf;
        }
        apply!(apply_dust);
        apply!(dust_level);
        apply!(apply_cn_filter);
        apply!(color_noise_radius);
        apply!(noise_filter_bias);
        apply!(lens_correction);
        apply!(vignette_amount);
        apply!(enhanced_shadow);
        apply!(remove_cast_highlight);
        apply!(remove_cast_shadow);
    }

    /// 是否有任何字段被设置
    pub fn any_set(&self) -> bool {
        // 任一 Option 为 Some 即视为"有值"
        self.film_type.is_some() || self.film_curve.is_some() || self.film_gamma.is_some()
        || self.apply_levels.is_some() || self.apply_curves.is_some()
        || self.apply_exposure.is_some() || self.apply_brightness.is_some()
        || self.apply_shadow_depth.is_some() || self.apply_midtone.is_some()
        || self.apply_contrast.is_some() || self.apply_highlights.is_some()
        || self.apply_shadows.is_some() || self.apply_saturation.is_some()
        || self.apply_color_balance.is_some() || self.apply_color_temp.is_some()
        || self.apply_color_corr.is_some() || self.apply_film_curve.is_some()
        || self.exposure.is_some() || self.brightness.is_some() || self.lightness.is_some()
        || self.midtone.is_some() || self.contrast.is_some() || self.highlights.is_some()
        || self.shadows.is_some() || self.saturation.is_some()
        || self.color_temperature.is_some() || self.tint.is_some()
        || self.r_shift.is_some() || self.g_shift.is_some() || self.b_shift.is_some()
        || self.levels_black.is_some() || self.levels_gamma.is_some()
        || self.levels_white.is_some() || self.output_shadow.is_some()
        || self.output_highlight.is_some() || self.color_corr.is_some()
        || self.apply_usm.is_some() || self.usm_amount.is_some() || self.usm_radius.is_some()
        || self.usm_dark_limit.is_some() || self.usm_noise_limit.is_some()
        || self.usm_col_factor.is_some()
        || self.apply_dust.is_some() || self.dust_level.is_some()
        || self.apply_cn_filter.is_some() || self.color_noise_radius.is_some()
        || self.noise_filter_bias.is_some()
        || self.lens_correction.is_some() || self.vignette_amount.is_some()
        || self.enhanced_shadow.is_some() || self.remove_cast_highlight.is_some()
        || self.remove_cast_shadow.is_some()
    }
}

/// 分割区域，包含中心坐标、尺寸和旋转角度。
#[derive(Debug, Clone)]
pub struct SidecarRegion {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub angle: f32,
}

/// 计算 sidecar 路径：`~/fff_parse/sidecar/{hash}.xml`，
/// 其中哈希值由源文件的绝对路径生成。
pub fn sidecar_path(source: &Path) -> PathBuf {
    let abs = std::fs::canonicalize(source)
        .unwrap_or_else(|_| source.to_path_buf());
    let mut hasher = DefaultHasher::new();
    abs.to_string_lossy().as_ref().hash(&mut hasher);
    let hash = hasher.finish();
    config::sidecar_dir().join(format!("{:016x}.xml", hash))
}

/// 从磁盘加载 sidecar 配置。文件不存在或格式无效时返回 `None`。
pub fn load(source: &Path) -> Option<SidecarConfig> {
    let path = sidecar_path(source);
    let xml = std::fs::read_to_string(&path).ok()?;
    parse_xml(&xml)
}

/// 将 sidecar 配置保存到磁盘的 sidecar 目录中。
pub fn save(source: &Path, config: &SidecarConfig) -> Result<(), String> {
    crate::config::ensure_dirs();
    let path = sidecar_path(source);
    let xml = to_xml(config);
    std::fs::write(&path, xml.as_bytes()).map_err(|e| format!("{}: {}", path.display(), e))
}

/// 删除 sidecar 文件（重置按钮用）。文件不存在时返回 Ok(())。
pub fn delete(source: &Path) -> Result<(), String> {
    let path = sidecar_path(source);
    match std::fs::remove_file(&path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {}", path.display(), e)),
    }
}

// ─── XML serialization ─────────────────────────────────────────────────────

/// 将 `SidecarConfig` 序列化为 XML 字符串（只写 Some 字段）。
fn to_xml(c: &SidecarConfig) -> String {
    let mut s = String::with_capacity(1024);
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<fff_viewer>\n");

    // Color section — 选择类字段总是全量写
    s.push_str("  <color>\n");
    let _ = writeln!(s, "    <use_embedded_correction>{}</use_embedded_correction>", c.use_embedded_correction);
    let _ = writeln!(s, "    <use_embedded_icc>{}</use_embedded_icc>", c.use_embedded_icc);
    if let Some(idx) = c.embedded_correction_index {
        let _ = writeln!(s, "    <embedded_correction_index>{}</embedded_correction_index>", idx);
    }
    if let Some(ref name) = c.input_profile_name {
        let _ = writeln!(s, "    <input_profile>{}</input_profile>", xml_escape(name));
    }
    if let Some(ref name) = c.preset_name {
        let _ = writeln!(s, "    <preset>{}</preset>", xml_escape(name));
    }
    let _ = writeln!(s, "    <target_color_space>{}</target_color_space>", xml_escape(&c.target_color_space));
    s.push_str("  </color>\n");

    // Adjust section — 仅写 Some 字段
    let a = &c.adjust;
    if a.any_set() || c.curve_points_override.is_some() {
        s.push_str("  <adjust>\n");
        macro_rules! w {
            ($field:ident, $tag:literal) => {
                if let Some(ref v) = a.$field {
                    let _ = writeln!(s, "    <{}>{}</{}>", $tag, v, $tag);
                }
            };
        }
        w!(film_type, "adj_film_type");
        w!(film_curve, "adj_film_curve");
        w!(film_gamma, "adj_film_gamma");
        w!(apply_levels, "adj_apply_levels");
        w!(apply_curves, "adj_apply_curves");
        w!(apply_exposure, "adj_apply_exposure");
        w!(apply_brightness, "adj_apply_brightness");
        w!(apply_shadow_depth, "adj_apply_shadow_depth");
        w!(apply_midtone, "adj_apply_midtone");
        w!(apply_contrast, "adj_apply_contrast");
        w!(apply_highlights, "adj_apply_highlights");
        w!(apply_shadows, "adj_apply_shadows");
        w!(apply_saturation, "adj_apply_saturation");
        w!(apply_color_balance, "adj_apply_color_balance");
        w!(apply_color_temp, "adj_apply_color_temp");
        w!(apply_color_corr, "adj_apply_color_corr");
        w!(apply_film_curve, "adj_apply_film_curve");
        w!(exposure, "adj_exposure");
        w!(brightness, "adj_brightness");
        w!(lightness, "adj_lightness");
        w!(midtone, "adj_midtone");
        w!(contrast, "adj_contrast");
        w!(highlights, "adj_highlights");
        w!(shadows, "adj_shadows");
        w!(saturation, "adj_saturation");
        w!(color_temperature, "adj_color_temperature");
        w!(tint, "adj_tint");
        w!(r_shift, "adj_r_shift");
        w!(g_shift, "adj_g_shift");
        w!(b_shift, "adj_b_shift");
        if let Some(ref v) = a.levels_black {
            let _ = writeln!(s, "    <adj_levels_black>{},{},{},{}</adj_levels_black>", v[0], v[1], v[2], v[3]);
        }
        if let Some(ref v) = a.levels_gamma {
            let _ = writeln!(s, "    <adj_levels_gamma>{},{},{},{}</adj_levels_gamma>", v[0], v[1], v[2], v[3]);
        }
        if let Some(ref v) = a.levels_white {
            let _ = writeln!(s, "    <adj_levels_white>{},{},{},{}</adj_levels_white>", v[0], v[1], v[2], v[3]);
        }
        if let Some(ref v) = a.output_shadow {
            let _ = writeln!(s, "    <adj_output_shadow>{},{},{},{}</adj_output_shadow>", v[0], v[1], v[2], v[3]);
        }
        if let Some(ref v) = a.output_highlight {
            let _ = writeln!(s, "    <adj_output_highlight>{},{},{},{}</adj_output_highlight>", v[0], v[1], v[2], v[3]);
        }
        if let Some(ref v) = a.color_corr {
            let joined: Vec<String> = v.iter().map(|n| n.to_string()).collect();
            let _ = writeln!(s, "    <adj_color_corr>{}</adj_color_corr>", joined.join(","));
        }
        w!(apply_usm, "adj_apply_usm");
        w!(usm_amount, "adj_usm_amount");
        w!(usm_radius, "adj_usm_radius");
        w!(usm_dark_limit, "adj_usm_dark_limit");
        w!(usm_noise_limit, "adj_usm_noise_limit");
        if let Some(ref v) = a.usm_col_factor {
            let _ = writeln!(s, "    <adj_usm_col_factor>{},{},{}</adj_usm_col_factor>", v[0], v[1], v[2]);
        }
        w!(apply_dust, "adj_apply_dust");
        w!(dust_level, "adj_dust_level");
        w!(apply_cn_filter, "adj_apply_cn_filter");
        w!(color_noise_radius, "adj_color_noise_radius");
        w!(noise_filter_bias, "adj_noise_filter_bias");
        w!(lens_correction, "adj_lens_correction");
        w!(vignette_amount, "adj_vignette_amount");
        w!(enhanced_shadow, "adj_enhanced_shadow");
        w!(remove_cast_highlight, "adj_remove_cast_highlight");
        w!(remove_cast_shadow, "adj_remove_cast_shadow");

        // 渐变曲线（任一 ≠ identity 即全量写 7 条）
        if let Some(ref curves) = c.curve_points_override {
            s.push_str("    <curve_points>\n");
            for (i, curve) in curves.iter().enumerate() {
                let pts: Vec<String> = curve.iter()
                    .map(|(x, y, dy)| format!("{},{},{}", x, y, dy))
                    .collect();
                let _ = writeln!(s, "      <curve ch=\"{}\">{}</curve>", i, pts.join(";"));
            }
            s.push_str("    </curve_points>\n");
        }
        s.push_str("  </adjust>\n");
    }

    // Split section — 总是全量写（用户明确选项，无 baseline 概念）
    s.push_str("  <split>\n");
    let _ = writeln!(s, "    <format>{}</format>", xml_escape(&c.split_format));
    let _ = writeln!(s, "    <portrait>{}</portrait>", c.split_portrait);
    let _ = writeln!(s, "    <naming_pattern>{}</naming_pattern>", xml_escape(&c.split_naming_pattern));
    if !c.split_regions.is_empty() {
        s.push_str("    <regions>\n");
        for r in &c.split_regions {
            let _ = writeln!(
                s,
                "      <region cx=\"{:.6}\" cy=\"{:.6}\" w=\"{:.6}\" h=\"{:.6}\" angle=\"{:.6}\"/>",
                r.cx, r.cy, r.w, r.h, r.angle
            );
        }
        s.push_str("    </regions>\n");
    }
    s.push_str("  </split>\n");

    s.push_str("</fff_viewer>\n");
    s
}

/// 对字符串进行 XML 转义（`&`、`<`、`>`、`"`）。
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 对 XML 转义字符串进行反转义。
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

// ─── XML parsing ────────────────────────────────────────────────────────────

/// 将 XML 字符串解析为 `SidecarConfig`，格式无效时返回 `None`。
///
/// 本实现把每个 tag 视为"Optional"：tag 存在则 `Some`，否则 `None`。
fn parse_xml(xml: &str) -> Option<SidecarConfig> {
    // Validate root element
    if !xml.contains("<fff_viewer>") {
        return None;
    }

    let mut config = SidecarConfig::empty();

    // Parse color section
    if let Some(v) = tag_content(xml, "use_embedded_correction") {
        config.use_embedded_correction = v == "true";
    }
    if let Some(v) = tag_content(xml, "use_embedded_icc") {
        config.use_embedded_icc = v == "true";
    }
    if let Some(v) = tag_content(xml, "embedded_correction_index") {
        if let Ok(idx) = v.parse::<usize>() {
            config.embedded_correction_index = Some(idx);
        }
    }
    if let Some(v) = tag_content(xml, "input_profile") {
        config.input_profile_name = Some(xml_unescape(&v));
    }
    if let Some(v) = tag_content(xml, "preset") {
        config.preset_name = Some(xml_unescape(&v));
    }
    if let Some(v) = tag_content(xml, "target_color_space") {
        config.target_color_space = xml_unescape(&v);
    }
    if let Some(v) = tag_content(xml, "format") {
        config.split_format = xml_unescape(&v);
    }
    if let Some(v) = tag_content(xml, "portrait") {
        config.split_portrait = v == "true";
    }
    if let Some(v) = tag_content(xml, "naming_pattern") {
        config.split_naming_pattern = xml_unescape(&v);
    }

    // Parse adjust section via macros
    let a = &mut config.adjust;
    macro_rules! r_i64 {
        ($field:ident, $tag:literal) => {
            if let Some(v) = tag_content(xml, $tag) {
                if let Ok(n) = v.parse::<i64>() { a.$field = Some(n); }
            }
        };
    }
    macro_rules! r_f32 {
        ($field:ident, $tag:literal) => {
            if let Some(v) = tag_content(xml, $tag) {
                if let Ok(n) = v.parse::<f32>() { a.$field = Some(n); }
            }
        };
    }
    macro_rules! r_f64 {
        ($field:ident, $tag:literal) => {
            if let Some(v) = tag_content(xml, $tag) {
                if let Ok(n) = v.parse::<f64>() { a.$field = Some(n); }
            }
        };
    }
    macro_rules! r_bool {
        ($field:ident, $tag:literal) => {
            if let Some(v) = tag_content(xml, $tag) {
                a.$field = Some(v == "true");
            }
        };
    }

    r_i64!(film_type, "adj_film_type");
    r_i64!(film_curve, "adj_film_curve");
    r_f64!(film_gamma, "adj_film_gamma");
    r_bool!(apply_levels, "adj_apply_levels");
    r_bool!(apply_curves, "adj_apply_curves");
    r_bool!(apply_exposure, "adj_apply_exposure");
    r_bool!(apply_brightness, "adj_apply_brightness");
    r_bool!(apply_shadow_depth, "adj_apply_shadow_depth");
    r_bool!(apply_midtone, "adj_apply_midtone");
    r_bool!(apply_contrast, "adj_apply_contrast");
    r_bool!(apply_highlights, "adj_apply_highlights");
    r_bool!(apply_shadows, "adj_apply_shadows");
    r_bool!(apply_saturation, "adj_apply_saturation");
    r_bool!(apply_color_balance, "adj_apply_color_balance");
    r_bool!(apply_color_temp, "adj_apply_color_temp");
    r_bool!(apply_color_corr, "adj_apply_color_corr");
    r_bool!(apply_film_curve, "adj_apply_film_curve");
    // exposure: 过去可能存 -inf/NaN，读取时防御
    if let Some(v) = tag_content(xml, "adj_exposure") {
        if let Ok(f) = v.parse::<f32>() {
            if f.is_finite() { a.exposure = Some(f); }
        }
    }
    r_f32!(brightness, "adj_brightness");
    r_f32!(lightness, "adj_lightness");
    r_f32!(midtone, "adj_midtone");
    r_f32!(contrast, "adj_contrast");
    r_f32!(highlights, "adj_highlights");
    r_f32!(shadows, "adj_shadows");
    r_f32!(saturation, "adj_saturation");
    r_f32!(color_temperature, "adj_color_temperature");
    r_f32!(tint, "adj_tint");
    r_f32!(r_shift, "adj_r_shift");
    r_f32!(g_shift, "adj_g_shift");
    r_f32!(b_shift, "adj_b_shift");

    let parse_f32_arr4 = |xml: &str, tag: &str| -> Option<[f32; 4]> {
        let v = tag_content(xml, tag)?;
        let vals: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if vals.len() == 4 { Some([vals[0], vals[1], vals[2], vals[3]]) }
        else if vals.len() == 1 { Some([vals[0]; 4]) }
        else { None }
    };
    a.levels_black = parse_f32_arr4(xml, "adj_levels_black");
    a.levels_gamma = parse_f32_arr4(xml, "adj_levels_gamma");
    a.levels_white = parse_f32_arr4(xml, "adj_levels_white");
    a.output_shadow = parse_f32_arr4(xml, "adj_output_shadow");
    a.output_highlight = parse_f32_arr4(xml, "adj_output_highlight");

    if let Some(v) = tag_content(xml, "adj_color_corr") {
        let vals: Vec<i64> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if vals.len() == 36 {
            let mut arr = [0i64; 36];
            for (i, &val) in vals.iter().enumerate() { arr[i] = val; }
            a.color_corr = Some(arr);
        }
    }

    r_bool!(apply_usm, "adj_apply_usm");
    r_i64!(usm_amount, "adj_usm_amount");
    r_i64!(usm_radius, "adj_usm_radius");
    r_i64!(usm_dark_limit, "adj_usm_dark_limit");
    r_i64!(usm_noise_limit, "adj_usm_noise_limit");
    if let Some(v) = tag_content(xml, "adj_usm_col_factor") {
        let vals: Vec<i64> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if vals.len() == 3 { a.usm_col_factor = Some([vals[0], vals[1], vals[2]]); }
    }
    r_bool!(apply_dust, "adj_apply_dust");
    r_i64!(dust_level, "adj_dust_level");
    r_bool!(apply_cn_filter, "adj_apply_cn_filter");
    r_i64!(color_noise_radius, "adj_color_noise_radius");
    r_i64!(noise_filter_bias, "adj_noise_filter_bias");
    r_i64!(lens_correction, "adj_lens_correction");
    r_i64!(vignette_amount, "adj_vignette_amount");
    r_bool!(enhanced_shadow, "adj_enhanced_shadow");
    r_bool!(remove_cast_highlight, "adj_remove_cast_highlight");
    r_bool!(remove_cast_shadow, "adj_remove_cast_shadow");

    // Parse curve_points (若存在)
    if xml.contains("<curve_points>") {
        let mut curves: Vec<Vec<(i64, i64, i64)>> = vec![vec![(0, 0, 0), (255, 255, 0)]; 7];
        let mut search_from = 0;
        while let Some(start) = xml[search_from..].find("<curve ch=\"") {
            let abs_start = search_from + start;
            let close_tag = "</curve>";
            let end = match xml[abs_start..].find(close_tag) {
                Some(e) => abs_start + e + close_tag.len(),
                None => break,
            };
            let tag = &xml[abs_start..end];
            // <curve ch="N">pts</curve>
            if let Some(ch_str) = attr_str(tag, "ch") {
                if let Ok(ch) = ch_str.parse::<usize>() {
                    if ch < 7 {
                        let body_start = tag.find('>').map(|p| p + 1).unwrap_or(0);
                        let body_end = tag.rfind("</curve>").unwrap_or(tag.len());
                        let body = &tag[body_start..body_end];
                        let mut pts = Vec::new();
                        for triple in body.split(';') {
                            let nums: Vec<i64> = triple.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                            if nums.len() == 3 {
                                pts.push((nums[0], nums[1], nums[2]));
                            }
                        }
                        if !pts.is_empty() {
                            curves[ch] = pts;
                        }
                    }
                }
            }
            search_from = end;
        }
        config.curve_points_override = Some(curves);
    }

    // Parse region elements: <region cx="..." cy="..." w="..." h="..." angle="..."/>
    let mut search_from = 0;
    while let Some(start) = xml[search_from..].find("<region ") {
        let abs_start = search_from + start;
        let tag_end = match xml[abs_start..].find("/>") {
            Some(e) => abs_start + e + 2,
            None => break,
        };
        let tag = &xml[abs_start..tag_end];
        if let Some(region) = parse_region_tag(tag) {
            config.split_regions.push(region);
        }
        search_from = tag_end;
    }

    Some(config)
}

/// 提取 `<tag>` 与 `</tag>` 之间的文本内容。
fn tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].trim().to_string())
}

/// 解析自闭合的 `<region ... />` 标签。
fn parse_region_tag(tag: &str) -> Option<SidecarRegion> {
    Some(SidecarRegion {
        cx: attr_f32(tag, "cx")?,
        cy: attr_f32(tag, "cy")?,
        w: attr_f32(tag, "w")?,
        h: attr_f32(tag, "h")?,
        angle: attr_f32(tag, "angle").unwrap_or(0.0),
    })
}

/// 提取浮点属性值，如 `name="1.234"`。
fn attr_f32(tag: &str, name: &str) -> Option<f32> {
    attr_str(tag, name)?.parse().ok()
}

/// 提取属性字符串，如 `name="foo"`。
fn attr_str(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_adjust_modified() -> ManualAdjust {
        let mut a = ManualAdjust::default();
        a.contrast = 30.0;
        a.saturation = 10.0;
        a.exposure = 0.5;
        a
    }

    #[test]
    fn diff_empty_when_equal() {
        let baseline = ManualAdjust::default();
        let current = ManualAdjust::default();
        let d = SidecarAdjustOverrides::from_diff(&current, &baseline);
        assert!(!d.any_set());
    }

    #[test]
    fn diff_captures_only_changed() {
        let baseline = ManualAdjust::default();
        let current = sample_adjust_modified();
        let d = SidecarAdjustOverrides::from_diff(&current, &baseline);
        assert_eq!(d.contrast, Some(30.0));
        assert_eq!(d.saturation, Some(10.0));
        assert_eq!(d.exposure, Some(0.5));
        assert!(d.brightness.is_none());
        assert!(d.lightness.is_none());
    }

    #[test]
    fn apply_only_sets_some_fields() {
        let baseline = ManualAdjust::default();
        let current = sample_adjust_modified();
        let d = SidecarAdjustOverrides::from_diff(&current, &baseline);

        let mut target = ManualAdjust::default();
        target.brightness = 42.0; // untouched by sidecar
        d.apply_to(&mut target);
        assert_eq!(target.contrast, 30.0);
        assert_eq!(target.saturation, 10.0);
        assert_eq!(target.exposure, 0.5);
        assert_eq!(target.brightness, 42.0, "non-overridden 字段不应变动");
    }

    #[test]
    fn roundtrip_preserves_overrides() {
        let baseline = ManualAdjust::default();
        let current = sample_adjust_modified();
        let config = SidecarConfig {
            adjust: SidecarAdjustOverrides::from_diff(&current, &baseline),
            preset_name: Some("Test".to_string()),
            ..SidecarConfig::empty()
        };
        let xml = to_xml(&config);
        let parsed = parse_xml(&xml).expect("valid XML");
        assert_eq!(parsed.preset_name, Some("Test".to_string()));
        assert_eq!(parsed.adjust.contrast, Some(30.0));
        assert_eq!(parsed.adjust.saturation, Some(10.0));
        assert!(parsed.adjust.brightness.is_none());
    }

    #[test]
    fn empty_sidecar_has_no_adjust_section() {
        let config = SidecarConfig::empty();
        let xml = to_xml(&config);
        assert!(!xml.contains("<adjust>"), "empty config 不应写 adjust 段");
    }

    #[test]
    fn curve_points_roundtrip() {
        let mut curves = vec![vec![(0, 0, 0), (255, 255, 0)]; 7];
        curves[0] = vec![(0, 0, 0), (64, 32, 0), (192, 224, 0), (255, 255, 0)];
        let config = SidecarConfig {
            curve_points_override: Some(curves.clone()),
            ..SidecarConfig::empty()
        };
        let xml = to_xml(&config);
        let parsed = parse_xml(&xml).expect("valid XML");
        assert_eq!(parsed.curve_points_override, Some(curves));
    }

    #[test]
    fn avoid_unused_color_warning() {
        // Ensures `use` of color module is referenced (for future-compat)
        let _ = color::ManualAdjust::default();
    }
}
