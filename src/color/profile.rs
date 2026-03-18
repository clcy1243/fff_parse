/// ICC color management and FlexColor settings preset support.

use std::path::{Path, PathBuf};

// ─── ICC Profile descriptor ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct IccProfileInfo {
    pub path: PathBuf,
    pub name: String,
    /// Profile class: "scnr" (scanner/input), "mntr" (monitor), "prtr" (printer)
    pub class: String,
    /// Color space: "RGB", "CMYK", "GRAY"
    pub color_space: String,
}

/// Scan a directory for .icc files and return descriptors.
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

/// Read basic info from an ICC profile header (128 bytes).
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

/// Extract the profile description string from the 'desc' tag.
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

/// Extract first string from an mluc (multi-localized Unicode) tag.
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

#[derive(Debug, Clone)]
pub struct SettingsPreset {
    pub path: PathBuf,
    pub name: String,
    pub category: String,
}

/// Scan settings directory for XML preset files.
pub fn scan_settings_presets(dir: &Path) -> Vec<SettingsPreset> {
    let mut presets = Vec::new();
    scan_settings_recursive(dir, dir, &mut presets);
    presets.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    presets
}

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
