//! FlexColor 数据模型定义
//!
//! 定义解析 Imacon FFF 标签 0xC519 中 XML plist 所需的数据结构，
//! 包括图像设置、日期时间、图像校正参数和编辑历史。

use std::fmt;

use super::parser;

/// 单条图像设置（编辑历史条目）
#[derive(Debug, Clone)]
pub struct ImageSetting {
    /// 设置名称
    pub name: String,
    /// 附加信息
    pub info: String,
    /// 标志位
    pub flags: i64,
    /// 创建时间
    pub created: DateTime,
    /// 修改时间
    pub modified: DateTime,
    /// 图像校正参数
    pub correction: ImageCorrection,
}

/// plist 中的日期时间
#[derive(Debug, Clone, Default)]
pub struct DateTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

impl DateTime {
    /// 判断日期是否有效（年份大于 0）
    pub fn is_valid(&self) -> bool {
        self.year > 0
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_valid() {
            write!(
                f,
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                self.year, self.month, self.day, self.hour, self.minute, self.second
            )
        } else {
            write!(f, "—")
        }
    }
}

/// 图像校正参数
#[derive(Debug, Clone, Default)]
pub struct ImageCorrection {
    /// 对比度
    pub contrast: i64,
    /// 亮度
    pub brightness: i64,
    /// 伽马值
    pub gamma: f64,
    /// 明度
    pub lightness: i64,
    /// 饱和度
    pub saturation: i64,
    /// 色温
    pub color_temperature: i64,
    /// 色调偏移
    pub tint: i64,
    /// 曝光补偿值
    pub ev: f64,
    /// 胶片曲线类型
    pub film_curve: i64,
    /// 胶片类型
    pub film_type: i64,
    /// 颜色模型
    pub color_model: i64,
    /// 是否应用滑块调整
    pub apply_sliders: bool,
    /// 是否应用曲线调整
    pub apply_curves: bool,
    /// 是否应用直方图调整
    pub apply_histogram: bool,
    /// 是否应用 USM 锐化
    pub apply_usm: bool,
    /// 是否应用除尘
    pub apply_dust: bool,
    /// 是否应用色彩校正
    pub apply_cc: bool,
    /// 是否应用色彩噪声滤镜
    pub apply_cn_filter: bool,
    /// USM 锐化强度
    pub usm_amount: i64,
    /// USM 锐化半径
    pub usm_radius: i64,
    /// USM 暗部限制
    pub usm_dark_limit: i64,
    /// USM 噪声限制
    pub usm_noise_limit: i64,
    /// 阈值
    pub threshold: i64,
    /// 除尘级别
    pub dust_level: i64,
    /// 色彩噪声半径
    pub color_noise_radius: i64,
    /// 噪声滤镜偏移
    pub noise_filter_bias: i64,
    /// 镜头校正
    pub lens_correction: i64,
    /// 暗角校正量
    pub vignette_amount: i64,
    /// 是否增强阴影
    pub enhanced_shadow: bool,
    /// 是否去除高光色偏
    pub remove_cast_highlight: bool,
    /// 是否去除阴影色偏
    pub remove_cast_shadow: bool,
    /// 是否嵌入 ICC 配置文件
    pub embed_profile: bool,
    /// 是否转换色彩空间
    pub convert: bool,
    /// 是否启用软打样
    pub soft_proof: bool,
    /// 自动高光值
    pub auto_highlight: i64,
    /// 自动阴影值
    pub auto_shadow: i64,
    /// 处理模式
    pub mode: i64,
    /// USM 色彩因子 [R, G, B]
    pub usm_col_factor: Vec<i64>,
    /// 直方图色阶：暗部，按通道 [RGB, R, G, B]
    pub shadow: [i64; 4],
    /// 直方图色阶：中间调，按通道 [RGB, R, G, B]
    pub gray: [i64; 4],
    /// 直方图色阶：高光，按通道 [RGB, R, G, B]
    pub highlight: [i64; 4],
    /// 色彩校正矩阵：36 个值（通道 × 分量）
    pub color_corr: Vec<i64>,
    /// 色调滑块 [对比度, 亮度, 阴影深度]
    pub gradation_sliders: [i64; 3],
    /// 各通道色调曲线控制点：[主通道, R, G, B, ...] 每个含 [(x, y, dy), ...]
    pub gradations: Vec<Vec<(i64, i64, i64)>>,
    /// 输入 ICC 配置文件名称（如 "Flextight Input"）
    pub input_profile_name: Option<String>,
    /// 输出 RGB 配置文件名称（如 "sRGB Color Space Profile.icm"）
    pub rgb_profile_name: Option<String>,
    /// 输出色阶端点 DotColor [14 值]：前 7 个为暗部端点，后 7 个为亮部端点
    pub dot_color: Vec<i64>,
    /// 所有原始键值对（用于展示未知字段）
    pub raw_params: Vec<(String, String)>,
}

/// 已解析的 FlexColor 编辑历史
#[derive(Debug, Clone)]
pub struct EditHistory {
    /// 所有图像设置条目
    pub settings: Vec<ImageSetting>,
    /// 当前选中的设置索引
    pub current_index: usize,
}

impl EditHistory {
    /// 从标签 0xC519 的原始字节数据中解析编辑历史
    pub fn parse(data: &[u8]) -> Option<Self> {
        // Find the XML plist within the data
        let xml_start = find_subsequence(data, b"<?xml")?;
        let plist_end_marker = b"</plist>";
        let xml_end = find_subsequence(&data[xml_start..], plist_end_marker)?
            + xml_start
            + plist_end_marker.len();

        let xml_str = std::str::from_utf8(&data[xml_start..xml_end]).ok()?;
        Self::parse_xml(xml_str)
    }

    /// 从完整文件数据中解析编辑历史（自动查找 XML plist）。
    ///
    /// 优先使用 `parse_from_tiff` 以避免全文件扫描。
    pub fn parse_from_file(data: &[u8]) -> Option<Self> {
        Self::parse(data)
    }

    /// 从已解析的 TiffFile 中提取编辑历史。
    ///
    /// 利用 tag 0xC519 的精确偏移和长度前缀直接定位 XML，
    /// 避免对整个文件数据进行线性扫描。
    pub fn parse_from_tiff(tiff: &crate::tiff::TiffFile) -> Option<Self> {
        if let Some(xml) = tiff.settings_xml() {
            Self::parse_xml(&xml)
        } else {
            // 回退：扫描原始数据
            Self::parse(tiff.raw_data())
        }
    }

    /// 解析 Apple plist XML 字符串
    fn parse_xml(xml: &str) -> Option<Self> {
        // Simple XML parser for Apple plist format
        let nodes = parser::parse_plist_nodes(xml)?;

        // The root should be: plist > dict > {key: ImageSettings, array: [...], key: CurrentIx, integer: N}
        let dict_node = parser::find_child_element(&nodes, "dict")?;

        let mut settings = Vec::new();
        let mut current_index = 0usize;

        let dict_children = parser::element_children(dict_node);
        let mut i = 0;
        while i + 1 < dict_children.len() {
            let key = parser::get_element_text(dict_children[i]);
            let val = dict_children[i + 1];

            match key.as_deref() {
                Some("ImageSettings") => {
                    if let Some(entries) = parser::parse_image_settings_array(val) {
                        settings = entries;
                    }
                }
                Some("CurrentIx") => {
                    if let Some(idx) = parser::get_element_text(val).and_then(|s| s.parse::<usize>().ok())
                    {
                        current_index = idx;
                    }
                }
                _ => {}
            }

            i += 2;
        }

        if settings.is_empty() {
            return None;
        }

        Some(EditHistory {
            settings,
            current_index,
        })
    }
}

/// 在字节序列中查找子序列的起始位置
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// 获取胶片曲线类型的可读名称
pub fn film_curve_name(v: i64) -> &'static str {
    match v {
        0 => "Linear",
        1 => "Film Std",
        2 => "Film High",
        3 => "Film Low",
        4 => "Film Auto",
        _ => "Unknown",
    }
}

/// 获取胶片类型的可读名称
pub fn film_type_name(v: i64) -> &'static str {
    match v {
        0 => "Positive E-6",
        1 => "Negative C-41",
        2 => "B&W",
        _ => "Unknown",
    }
}

/// 获取颜色模型的可读名称
pub fn color_model_name(v: i64) -> &'static str {
    match v {
        0 => "RGB",
        1 => "CMYK",
        2 => "Grayscale",
        _ => "Unknown",
    }
}
