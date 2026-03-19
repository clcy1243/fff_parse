//! ICC 配置文件管理与 FlexColor 设置预设支持。

use std::path::{Path, PathBuf};

// ─── ICC Profile descriptor ────────────────────────────────────────────────

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
