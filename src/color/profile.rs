//! ICC 配置文件管理与 FlexColor 设置预设支持。

use std::path::{Path, PathBuf};

// ─── ICC Profile descriptor ────────────────────────────────────────────────

/// 从 ICC 配置文件原始字节中解析的详细元数据。
#[derive(Debug, Clone, Default)]
pub struct IccProfileDetail {
    /// 配置文件大小（字节）
    pub size: u32,
    /// 版本号
    pub version: String,
    /// 设备类别：scnr / mntr / prtr / link / spac / abst / nmcl
    pub device_class: String,
    /// 设备类别的可读名称
    pub device_class_name: String,
    /// 输入色彩空间：RGB / CMYK / GRAY / Lab 等
    pub color_space: String,
    /// PCS（连接色彩空间）：XYZ 或 Lab
    pub pcs: String,
    /// 配置文件描述名称
    pub description: String,
    /// 创建日期时间
    pub date_time: String,
    /// 首选色彩管理模块（CMM）
    pub cmm_type: String,
    /// 渲染意图：0=感知 1=相对比色 2=饱和度 3=绝对比色
    pub rendering_intent: String,
    /// PCS 照度体 (D50 等)
    pub illuminant: String,
    /// 制造商签名
    pub manufacturer: String,
    /// 设备型号签名
    pub model: String,
    /// 标签数量
    pub tag_count: u32,
    /// 标签列表：(签名, 偏移, 大小)
    pub tags: Vec<(String, u32, u32)>,
}

/// 从 ICC 原始字节中解析详细元数据。
pub fn parse_icc_detail(data: &[u8]) -> Option<IccProfileDetail> {
    if data.len() < 132 { return None; }

    let u32be = |off: usize| -> u32 {
        u32::from_be_bytes([data[off], data[off+1], data[off+2], data[off+3]])
    };
    let sig = |off: usize| -> String {
        std::str::from_utf8(&data[off..off+4])
            .unwrap_or("????")
            .trim()
            .to_string()
    };

    let size = u32be(0);
    let cmm = sig(4);
    let ver_major = data[8];
    let ver_minor = data[9] >> 4;
    let ver_bugfix = data[9] & 0x0F;
    let version = format!("{}.{}.{}", ver_major, ver_minor, ver_bugfix);

    let device_class = sig(12);
    let device_class_name = match device_class.as_str() {
        "scnr" => "Scanner",
        "mntr" => "Monitor",
        "prtr" => "Printer",
        "link" => "Device Link",
        "spac" => "Color Space",
        "abst" => "Abstract",
        "nmcl" => "Named Color",
        _ => "Unknown",
    }.to_string();

    let color_space = sig(16);
    let pcs = sig(20);

    // Date/time at bytes 24-35 (6 × u16)
    let u16be = |off: usize| -> u16 {
        u16::from_be_bytes([data[off], data[off+1]])
    };
    let year  = u16be(24);
    let month = u16be(26);
    let day   = u16be(28);
    let hour  = u16be(30);
    let min   = u16be(32);
    let sec   = u16be(34);
    let date_time = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec);

    // Rendering intent at byte 64-67
    let intent_val = u32be(64);
    let rendering_intent = match intent_val {
        0 => "Perceptual",
        1 => "Relative Colorimetric",
        2 => "Saturation",
        3 => "Absolute Colorimetric",
        _ => "Unknown",
    }.to_string();

    // PCS illuminant at bytes 68-79 (3 × s15Fixed16)
    let fix16 = |off: usize| -> f64 {
        let raw = i32::from_be_bytes([data[off], data[off+1], data[off+2], data[off+3]]);
        raw as f64 / 65536.0
    };
    let ill_x = fix16(68);
    let ill_y = fix16(72);
    let ill_z = fix16(76);
    let illuminant = format!("X={:.4} Y={:.4} Z={:.4}", ill_x, ill_y, ill_z);

    let manufacturer = sig(48);
    let model = sig(52);

    let description = extract_profile_description(data)
        .unwrap_or_default();

    // Tags
    let tag_count = u32be(128);
    let mut tags = Vec::new();
    for i in 0..tag_count as usize {
        let base = 132 + i * 12;
        if base + 12 > data.len() { break; }
        let tag_sig = sig(base);
        let tag_off = u32be(base + 4);
        let tag_sz  = u32be(base + 8);
        tags.push((tag_sig, tag_off, tag_sz));
    }

    Some(IccProfileDetail {
        size,
        version,
        device_class,
        device_class_name,
        color_space,
        pcs,
        description,
        date_time,
        cmm_type: cmm,
        rendering_intent,
        illuminant,
        manufacturer,
        model,
        tag_count,
        tags,
    })
}

/// ICC 配置文件的描述信息。
#[derive(Debug, Clone)]
pub struct IccProfileInfo {
    /// 文件路径
    pub path: PathBuf,
    /// 配置文件显示名称
    pub name: String,
    /// 设备类别："scnr"（扫描仪）、"mntr"（显示器）、"prtr"（打印机）
    pub class: String,
    /// 色彩空间："RGB"、"CMYK"、"GRAY"
    pub color_space: String,
}

/// 扫描指定目录下的 .icc 文件，返回配置文件描述信息列表。
pub fn scan_icc_profiles(dir: &Path) -> Vec<IccProfileInfo> {
    let mut profiles = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return profiles,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("icc") {
            if let Some(info) = read_icc_info(&path) {
                profiles.push(info);
            }
        }
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    profiles
}

/// 从 ICC 文件头（128 字节）中读取基本信息。
fn read_icc_info(path: &Path) -> Option<IccProfileInfo> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 128 {
        return None;
    }

    // Bytes 12-15: profile/device class signature
    let class = std::str::from_utf8(&data[12..16])
        .unwrap_or("????")
        .trim()
        .to_string();

    // Bytes 16-19: color space
    let color_space = std::str::from_utf8(&data[16..20])
        .unwrap_or("????")
        .trim()
        .to_string();

    // Try to extract description from 'desc' tag
    let name = extract_profile_description(&data)
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".into())
        });

    Some(IccProfileInfo {
        path: path.to_path_buf(),
        name,
        class,
        color_space,
    })
}

/// 从 'desc' 标签中提取配置文件描述字符串。
fn extract_profile_description(data: &[u8]) -> Option<String> {
    if data.len() < 132 {
        return None;
    }

    // Tag table starts at offset 128
    let tag_count = u32::from_be_bytes([data[128], data[129], data[130], data[131]]) as usize;

    for i in 0..tag_count {
        let base = 132 + i * 12;
        if base + 12 > data.len() {
            break;
        }
        let sig = &data[base..base + 4];
        let offset = u32::from_be_bytes([data[base + 4], data[base + 5], data[base + 6], data[base + 7]]) as usize;
        let size = u32::from_be_bytes([data[base + 8], data[base + 9], data[base + 10], data[base + 11]]) as usize;

        if sig == b"desc" && offset + size <= data.len() && size > 12 {
            let type_sig = &data[offset..offset + 4];
            if type_sig == b"desc" {
                // ICC v2 'desc' type: offset+8 = ASCII count, offset+12 = ASCII string
                let ascii_count = u32::from_be_bytes([
                    data[offset + 8], data[offset + 9], data[offset + 10], data[offset + 11],
                ]) as usize;
                if ascii_count > 0 && offset + 12 + ascii_count <= data.len() {
                    let s = std::str::from_utf8(&data[offset + 12..offset + 12 + ascii_count - 1]).ok()?;
                    return Some(s.to_string());
                }
            } else if type_sig == b"mluc" {
                // ICC v4 'mluc' (multi-localized Unicode) type
                return extract_mluc_string(&data[offset..offset + size]);
            }
        }
    }
    None
}

/// 从 mluc（多语言 Unicode）标签中提取第一个字符串。
fn extract_mluc_string(data: &[u8]) -> Option<String> {
    if data.len() < 20 {
        return None;
    }
    // Record count at offset 8
    let _count = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    // First record at offset 16: language(2), country(2), length(4), offset(4)
    if data.len() < 28 {
        return None;
    }
    let str_len = u32::from_be_bytes([data[20], data[21], data[22], data[23]]) as usize;
    let str_off = u32::from_be_bytes([data[24], data[25], data[26], data[27]]) as usize;

    if str_off + str_len <= data.len() && str_len >= 2 {
        // UTF-16BE encoded
        let utf16: Vec<u16> = data[str_off..str_off + str_len]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&utf16).ok()
    } else {
        None
    }
}

// ─── Settings Preset ────────────────────────────────────────────────────────

/// FlexColor 设置预设，包含处理参数的 XML 文件。
#[derive(Debug, Clone)]
pub struct SettingsPreset {
    /// 预设文件路径
    pub path: PathBuf,
    /// 预设名称（文件名去扩展名）
    pub name: String,
    /// 分类（基于子目录结构）
    pub category: String,
}

/// 扫描设置目录下的 XML 预设文件，按分类和名称排序返回。
pub fn scan_settings_presets(dir: &Path) -> Vec<SettingsPreset> {
    let mut presets = Vec::new();
    scan_settings_recursive(dir, dir, &mut presets);
    presets.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    presets
}

/// 递归扫描目录树，收集 XML 预设文件。
fn scan_settings_recursive(base: &Path, dir: &Path, out: &mut Vec<SettingsPreset>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_settings_recursive(base, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("xml") {
            let category = path
                .parent()
                .and_then(|p| p.strip_prefix(base).ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = path
                .file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(SettingsPreset {
                path: path.clone(),
                name,
                category,
            });
        }
    }
}
