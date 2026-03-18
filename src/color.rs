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

// ─── ICC Color Transform ────────────────────────────────────────────────────

/// Target (output) color space for ICC transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetColorSpace {
    ProPhotoRGB,
    SRGB,
    AdobeRGB,
    DisplayP3,
}

impl TargetColorSpace {
    pub const ALL: &[TargetColorSpace] = &[
        Self::ProPhotoRGB,
        Self::SRGB,
        Self::AdobeRGB,
        Self::DisplayP3,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::SRGB => "sRGB",
            Self::AdobeRGB => "Adobe RGB (1998)",
            Self::ProPhotoRGB => "ProPhoto RGB",
            Self::DisplayP3 => "Display P3",
        }
    }

    pub fn to_str(&self) -> &'static str {
        match self {
            Self::SRGB => "sRGB",
            Self::AdobeRGB => "AdobeRGB",
            Self::ProPhotoRGB => "ProPhotoRGB",
            Self::DisplayP3 => "DisplayP3",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "sRGB" => Self::SRGB,
            "AdobeRGB" => Self::AdobeRGB,
            "DisplayP3" => Self::DisplayP3,
            _ => Self::ProPhotoRGB,
        }
    }
}

impl Default for TargetColorSpace {
    fn default() -> Self {
        Self::ProPhotoRGB
    }
}

/// Create an lcms2 output profile for a target color space.
fn create_output_profile(space: TargetColorSpace) -> Result<lcms2::Profile, String> {
    use lcms2::*;

    match space {
        TargetColorSpace::SRGB => Ok(Profile::new_srgb()),

        TargetColorSpace::AdobeRGB => {
            let d65 = CIExyY { x: 0.3127, y: 0.3290, Y: 1.0 };
            let primaries = CIExyYTRIPLE {
                Red: CIExyY { x: 0.6400, y: 0.3300, Y: 0.0 },
                Green: CIExyY { x: 0.2100, y: 0.7100, Y: 0.0 },
                Blue: CIExyY { x: 0.1500, y: 0.0600, Y: 0.0 },
            };
            let gamma = ToneCurve::new(2.19921875);
            Profile::new_rgb(&d65, &primaries, &[&gamma, &gamma, &gamma])
                .map_err(|e| format!("Failed to create Adobe RGB profile: {:?}", e))
        }

        TargetColorSpace::ProPhotoRGB => {
            let d50 = CIExyY { x: 0.3457, y: 0.3585, Y: 1.0 };
            let primaries = CIExyYTRIPLE {
                Red: CIExyY { x: 0.7347, y: 0.2653, Y: 0.0 },
                Green: CIExyY { x: 0.1596, y: 0.8404, Y: 0.0 },
                Blue: CIExyY { x: 0.0366, y: 0.0001, Y: 0.0 },
            };
            let gamma = ToneCurve::new(1.8);
            Profile::new_rgb(&d50, &primaries, &[&gamma, &gamma, &gamma])
                .map_err(|e| format!("Failed to create ProPhoto RGB profile: {:?}", e))
        }

        TargetColorSpace::DisplayP3 => {
            let d65 = CIExyY { x: 0.3127, y: 0.3290, Y: 1.0 };
            let primaries = CIExyYTRIPLE {
                Red: CIExyY { x: 0.6800, y: 0.3200, Y: 0.0 },
                Green: CIExyY { x: 0.2650, y: 0.6900, Y: 0.0 },
                Blue: CIExyY { x: 0.1500, y: 0.0600, Y: 0.0 },
            };
            let gamma = ToneCurve::new(2.2);
            Profile::new_rgb(&d65, &primaries, &[&gamma, &gamma, &gamma])
                .map_err(|e| format!("Failed to create Display P3 profile: {:?}", e))
        }
    }
}

/// Manual image adjustment parameters applied after the color profile pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct ManualAdjust {
    pub enabled: bool,
    pub exposure: f32,      // stops: -3.0..3.0
    pub contrast: f32,      // -100..100
    pub highlights: f32,    // -100..100
    pub shadows: f32,       // -100..100
    pub saturation: f32,    // -100..100
    pub r_shift: f32,       // color balance red:   -100..100
    pub g_shift: f32,       // color balance green: -100..100
    pub b_shift: f32,       // color balance blue:  -100..100

    // Levels (input range): index 0=master, 1=R, 2=G, 3=B
    pub levels_black: [f32; 4],  // input black point: 0-255
    pub levels_gamma: [f32; 4],  // midtone gamma: 0.10-9.99 (1.0=neutral)
    pub levels_white: [f32; 4],  // input white point: 0-255
}

impl Default for ManualAdjust {
    fn default() -> Self {
        Self {
            enabled: false,
            exposure: 0.0, contrast: 0.0, highlights: 0.0, shadows: 0.0,
            saturation: 0.0, r_shift: 0.0, g_shift: 0.0, b_shift: 0.0,
            levels_black: [0.0; 4],
            levels_gamma: [1.0; 4],
            levels_white: [255.0; 4],
        }
    }
}

impl ManualAdjust {
    pub fn is_identity(&self) -> bool {
        !self.enabled
            || (self.exposure.abs() < 0.001
                && self.contrast.abs() < 0.1
                && self.highlights.abs() < 0.1
                && self.shadows.abs() < 0.1
                && self.saturation.abs() < 0.1
                && self.r_shift.abs() < 0.1
                && self.g_shift.abs() < 0.1
                && self.b_shift.abs() < 0.1
                && self.levels_black.iter().all(|&v| v < 0.5)
                && self.levels_gamma.iter().all(|&v| (v - 1.0).abs() < 0.01)
                && self.levels_white.iter().all(|&v| v > 254.5))
    }
}

/// Apply ICC color transform to an image.
/// `input_icc`: scanner/input ICC profile bytes
/// `target`: output color space
/// Returns transformed DynamicImage.
pub fn apply_icc_transform(
    img: &image::DynamicImage,
    input_icc: &[u8],
    target: TargetColorSpace,
) -> Result<image::DynamicImage, String> {
    use lcms2::*;

    let input_profile = Profile::new_icc(input_icc)
        .map_err(|e| format!("Failed to load input ICC: {:?}", e))?;

    let output_profile = create_output_profile(target)?;

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let transform = Transform::new(
                &input_profile,
                PixelFormat::RGB_16,
                &output_profile,
                PixelFormat::RGB_16,
                Intent::Perceptual,
            ).map_err(|e| format!("Failed to create transform: {:?}", e))?;

            let pixels: Vec<[u16; 3]> = rgb16
                .pixels()
                .map(|p| [p[0], p[1], p[2]])
                .collect();

            let mut output = pixels.clone();
            transform.transform_pixels(&pixels, &mut output);

            let flat: Vec<u16> = output.into_iter().flat_map(|p| p).collect();
            let result = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                rgb16.width(),
                rgb16.height(),
                flat,
            )
            .ok_or_else(|| "Failed to create output image".to_string())?;

            Ok(image::DynamicImage::ImageRgb16(result))
        }
        image::DynamicImage::ImageRgb8(rgb8) => {
            let transform = Transform::new(
                &input_profile,
                PixelFormat::RGB_8,
                &output_profile,
                PixelFormat::RGB_8,
                Intent::Perceptual,
            ).map_err(|e| format!("Failed to create transform: {:?}", e))?;

            let pixels: Vec<[u8; 3]> = rgb8
                .pixels()
                .map(|p| [p[0], p[1], p[2]])
                .collect();

            let mut output = pixels.clone();
            transform.transform_pixels(&pixels, &mut output);

            let flat: Vec<u8> = output.into_iter().flat_map(|p| p).collect();
            let result = image::RgbImage::from_raw(rgb8.width(), rgb8.height(), flat)
                .ok_or_else(|| "Failed to create output image".to_string())?;

            Ok(image::DynamicImage::ImageRgb8(result))
        }
        _ => {
            // Convert to Rgb8 first
            let rgb8 = img.to_rgb8();
            let converted = image::DynamicImage::ImageRgb8(rgb8);
            apply_icc_transform(&converted, input_icc, target)
        }
    }
}

// ─── Film Type Processing ───────────────────────────────────────────────────

use crate::flexcolor::ImageCorrection;

/// Compute per-channel auto shadow/highlight for negative film by analyzing
/// the raw image histogram. Excludes saturated pixels (film borders, sprocket
/// holes) to find the actual image content range per channel.
///
/// Returns per-channel (shadow, highlight) in 16-bit scale for the INVERTED
/// (positive) data space.
fn compute_neg_auto_levels_16(src: &[u16], width: usize, height: usize) -> ([f32; 3], [f32; 3]) {
    use rayon::prelude::*;

    const BINS: usize = 4096;
    const SHIFT: u32 = 4; // 16-bit >> 4 = 4096 bins
    const SATURATION_THRESHOLD: u16 = 45000;
    const BASE_PCT: f64 = 0.995; // film base = 99.5th percentile of image content
    const DEEP_PCT: f64 = 0.005; // deep shadow = 0.5th percentile

    // Build per-channel histograms in parallel by row
    // Use Vec to avoid stack overflow on rayon worker threads (3×4096×4 = 48KB per thread)
    let row_len = width * 3;
    let num_rows = height;

    let (hist_r, hist_g, hist_b, total) = (0..num_rows)
        .into_par_iter()
        .fold(
            || (vec![0u32; BINS], vec![0u32; BINS], vec![0u32; BINS], 0u64),
            |mut acc, y| {
                let start = y * row_len;
                for x in 0..width {
                    let si = start + x * 3;
                    let r = src[si];
                    let g = src[si + 1];
                    let b = src[si + 2];
                    // Exclude saturated pixels (borders, sprocket holes)
                    if r < SATURATION_THRESHOLD && g < SATURATION_THRESHOLD && b < SATURATION_THRESHOLD {
                        acc.0[(r >> SHIFT) as usize] += 1;
                        acc.1[(g >> SHIFT) as usize] += 1;
                        acc.2[(b >> SHIFT) as usize] += 1;
                        acc.3 += 1;
                    }
                }
                acc
            },
        )
        .reduce(
            || (vec![0u32; BINS], vec![0u32; BINS], vec![0u32; BINS], 0u64),
            |mut a, b| {
                for i in 0..BINS {
                    a.0[i] += b.0[i];
                    a.1[i] += b.1[i];
                    a.2[i] += b.2[i];
                }
                a.3 += b.3;
                a
            },
        );

    if total == 0 {
        return ([0.0; 3], [65535.0; 3]);
    }

    let find_pct = |hist: &[u32], pct: f64| -> f32 {
        let target = (total as f64 * pct) as u64;
        let mut count = 0u64;
        for i in 0..BINS {
            count += hist[i] as u64;
            if count >= target {
                return ((i as u32) << SHIFT) as f32;
            }
        }
        65535.0
    };

    // Film base = highest raw values among image content (per channel)
    let base_r = find_pct(&hist_r, BASE_PCT);
    let base_g = find_pct(&hist_g, BASE_PCT);
    let base_b = find_pct(&hist_b, BASE_PCT);

    // Deep shadow = lowest raw values among image content (per channel)
    let deep_r = find_pct(&hist_r, DEEP_PCT);
    let deep_g = find_pct(&hist_g, DEEP_PCT);
    let deep_b = find_pct(&hist_b, DEEP_PCT);

    // Convert to inverted (positive) space
    let shadow = [65535.0 - base_r, 65535.0 - base_g, 65535.0 - base_b];
    let highlight = [65535.0 - deep_r, 65535.0 - deep_g, 65535.0 - deep_b];

    log::info!(
        "Auto-levels: film_base raw R={:.0},G={:.0},B={:.0}, deep R={:.0},G={:.0},B={:.0} \
         → inv shadow R={:.0},G={:.0},B={:.0}, highlight R={:.0},G={:.0},B={:.0} ({}px, {} excluded)",
        base_r, base_g, base_b, deep_r, deep_g, deep_b,
        shadow[0], shadow[1], shadow[2], highlight[0], highlight[1], highlight[2],
        total, width as u64 * height as u64 - total,
    );

    (shadow, highlight)
}

/// Apply film type processing: negative inversion + per-channel levels.
///
/// For negative film (FilmType=1, C-41): invert pixels then remap per-channel.
/// When `remove_cast_shadow`/`remove_cast_highlight` are enabled, automatically
/// computes per-channel shadow/highlight from the image histogram to remove the
/// orange mask color cast. The preset's Gray values still control midtone gamma.
///
/// For B&W negative (FilmType=2): same inversion + convert to grayscale.
/// For positive film (FilmType=0): only apply levels adjustment using preset values.
///
/// Preserves bit depth: 16-bit input → 16-bit output, 8-bit → 8-bit.
/// Uses rayon for parallel row processing on large images.
pub fn apply_film_processing(
    img: &image::DynamicImage,
    correction: &ImageCorrection,
) -> image::DynamicImage {
    use rayon::prelude::*;

    let film_type = correction.film_type;
    let is_negative = film_type == 1 || film_type == 2;

    let shadow = correction.shadow;
    let highlight = correction.highlight;
    let gray = correction.gray;

    // Detect if preset has meaningful per-channel levels or just defaults/zeros.
    // FlexColor saves auto-computed levels into the active correction when the
    // user processes a negative. If the per-channel values (R,G,B = indices 1,2,3)
    // are all identical, the preset is unprocessed and we need auto-computation.
    let needs_auto_levels = is_negative
        && (correction.remove_cast_shadow || correction.remove_cast_highlight)
        && shadow[1] == shadow[2] && shadow[2] == shadow[3]
        && highlight[1] == highlight[2] && highlight[2] == highlight[3];

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            const SCALE: f32 = 4.0;
            const MAX_VAL: f32 = 65535.0;

            let src = rgb16.as_raw();

            // Compute per-channel level params.
            // If the active correction already has per-channel values (FlexColor
            // saved auto-computed levels), use them directly. Otherwise, compute
            // from the image histogram to remove the negative film orange mask.
            let (auto_shadow, auto_highlight) = if needs_auto_levels {
                log::info!("Preset has identical per-channel levels, computing auto-levels");
                compute_neg_auto_levels_16(src, w as usize, h as usize)
            } else {
                ([0.0f32; 3], [0.0f32; 3])
            };

            let mut ch_s = [0.0f32; 4];
            let mut ch_range = [1.0f32; 4];
            let mut ch_gamma = [1.0f32; 4];

            if needs_auto_levels {
                // Master channel: use preset values
                ch_s[0] = (shadow[0] as f32 * SCALE).clamp(0.0, MAX_VAL);
                let h0 = (highlight[0] as f32 * SCALE).clamp(1.0, MAX_VAL);
                ch_range[0] = (h0 - ch_s[0]).max(1.0);
                ch_gamma[0] = 1.0 / (gray[0] as f32 / 128.0).clamp(0.01, 10.0);

                // Per-channel: use auto-computed values with preset gamma
                for ch in 0..3 {
                    ch_s[ch + 1] = auto_shadow[ch];
                    ch_range[ch + 1] = (auto_highlight[ch] - auto_shadow[ch]).max(1.0);
                    ch_gamma[ch + 1] = 1.0 / (gray[ch + 1] as f32 / 128.0).clamp(0.01, 10.0);
                }
            } else {
                // Use preset values directly (FlexColor already computed per-channel levels)
                for i in 0..4 {
                    ch_s[i] = (shadow[i] as f32 * SCALE).clamp(0.0, MAX_VAL);
                    let h_val = (highlight[i] as f32 * SCALE).clamp(1.0, MAX_VAL);
                    ch_range[i] = (h_val - ch_s[i]).max(1.0);
                    ch_gamma[i] = 1.0 / (gray[i] as f32 / 128.0).clamp(0.01, 10.0);
                }
            }

            log::debug!(
                "Film processing levels: shadow=[{:.0},{:.0},{:.0},{:.0}] highlight=[{:.0},{:.0},{:.0},{:.0}] gamma=[{:.3},{:.3},{:.3},{:.3}]",
                ch_s[0], ch_s[1], ch_s[2], ch_s[3],
                ch_s[0]+ch_range[0], ch_s[1]+ch_range[1], ch_s[2]+ch_range[2], ch_s[3]+ch_range[3],
                ch_gamma[0], ch_gamma[1], ch_gamma[2], ch_gamma[3],
            );

            let apply_master = (ch_s[0] > 4.0)
                || ((highlight[0] as f32 * SCALE) < MAX_VAL - 4.0)
                || (ch_gamma[0] - 1.0).abs() > 0.01;

            let row_len = w as usize * 3;
            let mut out_pixels = vec![0u16; row_len * h as usize];

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        let mut ch_f = [
                            src[si] as f32,
                            src[si + 1] as f32,
                            src[si + 2] as f32,
                        ];

                        if is_negative {
                            ch_f[0] = MAX_VAL - ch_f[0];
                            ch_f[1] = MAX_VAL - ch_f[1];
                            ch_f[2] = MAX_VAL - ch_f[2];
                        }

                        for ch in 0..3 {
                            let ci = ch + 1;
                            let n = ((ch_f[ch] - ch_s[ci]) / ch_range[ci]).clamp(0.0, 1.0);
                            ch_f[ch] = n.powf(ch_gamma[ci]) * MAX_VAL;
                        }

                        if apply_master {
                            for ch in 0..3 {
                                let n = ((ch_f[ch] - ch_s[0]) / ch_range[0]).clamp(0.0, 1.0);
                                ch_f[ch] = n.powf(ch_gamma[0]) * MAX_VAL;
                            }
                        }

                        if film_type == 2 {
                            let lum = 0.299 * ch_f[0] + 0.587 * ch_f[1] + 0.114 * ch_f[2];
                            ch_f = [lum, lum, lum];
                        }

                        row[base] = ch_f[0].clamp(0.0, MAX_VAL) as u16;
                        row[base + 1] = ch_f[1].clamp(0.0, MAX_VAL) as u16;
                        row[base + 2] = ch_f[2].clamp(0.0, MAX_VAL) as u16;
                    }
                });

            let buf = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(w, h, out_pixels)
                .expect("film_processing 16-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb16(buf)
        }
        _ => {
            // 8-bit fallback: convert to 16-bit when auto-levels needed
            let rgb8 = img.to_rgb8();
            let (w, h) = (rgb8.width(), rgb8.height());

            if needs_auto_levels {
                let src8 = rgb8.as_raw();
                let src16: Vec<u16> = src8.iter().map(|&v| (v as u16) << 8).collect();
                let img16 = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(w, h, src16)
                    .expect("8→16 upscale failed");
                let dyn16 = image::DynamicImage::ImageRgb16(img16);
                let result16 = apply_film_processing(&dyn16, correction);
                return image::DynamicImage::ImageRgb8(result16.to_rgb8());
            }

            const SCALE8: f32 = 64.0;

            let mut ch_s = [0.0f32; 4];
            let mut ch_range = [1.0f32; 4];
            let mut ch_gamma = [1.0f32; 4];
            for i in 0..4 {
                ch_s[i] = (shadow[i] as f32 / SCALE8).clamp(0.0, 255.0);
                let h_val = (highlight[i] as f32 / SCALE8).clamp(1.0, 255.0);
                ch_range[i] = (h_val - ch_s[i]).max(1.0);
                ch_gamma[i] = 1.0 / (gray[i] as f32 / 128.0).clamp(0.01, 10.0);
            }
            let apply_master = (ch_s[0] > 1.0) || (ch_range[0] < 253.0) || (ch_gamma[0] - 1.0).abs() > 0.01;

            let src = rgb8.as_raw();
            let row_len = w as usize * 3;
            let mut out_pixels = vec![0u8; row_len * h as usize];

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        let mut ch_f = [src[si] as f32, src[si + 1] as f32, src[si + 2] as f32];

                        if is_negative {
                            ch_f[0] = 255.0 - ch_f[0];
                            ch_f[1] = 255.0 - ch_f[1];
                            ch_f[2] = 255.0 - ch_f[2];
                        }

                        for ch in 0..3 {
                            let ci = ch + 1;
                            let n = ((ch_f[ch] - ch_s[ci]) / ch_range[ci]).clamp(0.0, 1.0);
                            ch_f[ch] = n.powf(ch_gamma[ci]) * 255.0;
                        }

                        if apply_master {
                            for ch in 0..3 {
                                let n = ((ch_f[ch] - ch_s[0]) / ch_range[0]).clamp(0.0, 1.0);
                                ch_f[ch] = n.powf(ch_gamma[0]) * 255.0;
                            }
                        }

                        if film_type == 2 {
                            let lum = 0.299 * ch_f[0] + 0.587 * ch_f[1] + 0.114 * ch_f[2];
                            ch_f = [lum, lum, lum];
                        }

                        row[base] = ch_f[0].clamp(0.0, 255.0) as u8;
                        row[base + 1] = ch_f[1].clamp(0.0, 255.0) as u8;
                        row[base + 2] = ch_f[2].clamp(0.0, 255.0) as u8;
                    }
                });

            let buf = image::RgbImage::from_raw(w, h, out_pixels)
                .expect("film_processing 8-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb8(buf)
        }
    }
}

/// Apply manual adjustments (exposure, contrast, shadows/highlights, saturation, color balance)
/// to an 8-bit image. Uses per-channel LUTs for performance.
pub fn apply_manual_adjust(img: &image::DynamicImage, adj: &ManualAdjust) -> image::DynamicImage {
    if adj.is_identity() {
        return img.clone();
    }

    let rgb8 = img.to_rgb8();
    let (w, h) = (rgb8.width(), rgb8.height());
    let src = rgb8.as_raw();

    let exposure_mult = 2.0_f32.powf(adj.exposure);
    let sat = adj.saturation / 100.0;

    // Master levels params
    let bl_m = adj.levels_black[0] / 255.0;
    let wh_m = adj.levels_white[0] / 255.0;
    let range_m = (wh_m - bl_m).max(0.001);
    let gamma_m = adj.levels_gamma[0].clamp(0.01, 99.0);

    // Build per-channel LUTs: levels → exposure/color-balance → shadows/highlights → contrast
    let mut luts = [[0u8; 256]; 3];
    let shifts = [adj.r_shift / 255.0, adj.g_shift / 255.0, adj.b_shift / 255.0];

    for ch in 0..3 {
        let bl_c = adj.levels_black[ch + 1] / 255.0;
        let wh_c = adj.levels_white[ch + 1] / 255.0;
        let range_c = (wh_c - bl_c).max(0.001);
        let gamma_c = adj.levels_gamma[ch + 1].clamp(0.01, 99.0);

        for i in 0..=255u32 {
            let mut v = i as f32 / 255.0;

            // Step 1: Apply master levels (affects all channels equally)
            v = ((v - bl_m) / range_m).clamp(0.0, 1.0).powf(1.0 / gamma_m);

            // Step 2: Apply per-channel levels
            v = ((v - bl_c) / range_c).clamp(0.0, 1.0).powf(1.0 / gamma_c);

            // Step 3: Color balance + exposure
            v += shifts[ch];
            v *= exposure_mult;
            v = v.clamp(0.0, 1.0);

            // Step 4: Shadows rolloff
            if adj.shadows.abs() > 0.1 {
                let s = adj.shadows / 100.0;
                let t = 1.0 - v;
                v = (v + s * t * t * 0.5).clamp(0.0, 1.0);
            }
            // Step 5: Highlights rolloff
            if adj.highlights.abs() > 0.1 {
                let hi = adj.highlights / 100.0;
                let t = v;
                v = (v + hi * t * t * 0.5).clamp(0.0, 1.0);
            }
            // Step 6: Contrast S-curve
            if adj.contrast.abs() > 0.1 {
                let c = adj.contrast / 100.0;
                let scale = if c >= 0.0 { 1.0 + c * 2.0 } else { 1.0 + c };
                v = ((v - 0.5) * scale + 0.5).clamp(0.0, 1.0);
            }

            luts[ch][i as usize] = (v * 255.0) as u8;
        }
    }

    let row_len = w as usize * 3;
    let mut out = vec![0u8; row_len * h as usize];

    use rayon::prelude::*;
    out.par_chunks_mut(row_len)
        .enumerate()
        .for_each(|(y, row)| {
            let src_start = y * row_len;
            for x in 0..w as usize {
                let base = x * 3;
                let si = src_start + base;
                let mut rf = luts[0][src[si] as usize] as f32;
                let mut gf = luts[1][src[si + 1] as usize] as f32;
                let mut bf = luts[2][src[si + 2] as usize] as f32;

                // Saturation: desaturate/saturate using luminance
                if sat.abs() > 0.001 {
                    let lum = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
                    rf = (lum + (rf - lum) * (1.0 + sat)).clamp(0.0, 255.0);
                    gf = (lum + (gf - lum) * (1.0 + sat)).clamp(0.0, 255.0);
                    bf = (lum + (bf - lum) * (1.0 + sat)).clamp(0.0, 255.0);
                }

                row[base] = rf as u8;
                row[base + 1] = gf as u8;
                row[base + 2] = bf as u8;
            }
        });

    let buf = image::RgbImage::from_raw(w, h, out).expect("manual_adjust buffer mismatch");
    image::DynamicImage::ImageRgb8(buf)
}

/// Extract embedded ICC profile data from FFF tag 0xC51A.
pub fn extract_embedded_icc(tiff_data: &[u8], tags: &[(String, String, String, String)]) -> Option<Vec<u8>> {
    // Look for tag 0xC51A (ImaconProfileData)
    for (_, tag_hex, _, _value) in tags {
        if tag_hex == "0xC51A" {
            // Extract raw tag data
            let data = extract_tag_data(tiff_data, 0xC51A)?;

            // Validate: a real ICC profile has "acsp" signature at offset 36
            if data.len() > 40 && &data[36..40] == b"acsp" {
                log::info!("Embedded ICC profile found: {} bytes, valid ICC", data.len());
                return Some(data);
            } else {
                log::info!(
                    "Tag 0xC51A contains Imacon proprietary data ({} bytes), not a standard ICC profile",
                    data.len()
                );
                return None;
            }
        }
    }
    None
}

/// Read raw bytes for a given TIFF tag from the file data.
fn extract_tag_data(data: &[u8], target_tag: u16) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }

    let big_endian = data[0] == b'M' && data[1] == b'M';

    let read_u16 = |off: usize| -> u16 {
        if big_endian {
            u16::from_be_bytes([data[off], data[off + 1]])
        } else {
            u16::from_le_bytes([data[off], data[off + 1]])
        }
    };
    let read_u32 = |off: usize| -> u32 {
        if big_endian {
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
        } else {
            u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
        }
    };

    let mut ifd_offset = read_u32(4) as usize;

    while ifd_offset > 0 && ifd_offset + 2 <= data.len() {
        let entry_count = read_u16(ifd_offset) as usize;
        for i in 0..entry_count {
            let entry_off = ifd_offset + 2 + i * 12;
            if entry_off + 12 > data.len() {
                break;
            }
            let tag = read_u16(entry_off);
            if tag == target_tag {
                let typ = read_u16(entry_off + 2);
                let count = read_u32(entry_off + 4) as usize;
                let byte_size = match typ {
                    1 | 6 | 7 => count,          // BYTE, SBYTE, UNDEFINED
                    2 => count,                   // ASCII
                    3 | 8 => count * 2,           // SHORT, SSHORT
                    4 | 9 => count * 4,           // LONG, SLONG
                    5 | 10 => count * 8,          // RATIONAL, SRATIONAL
                    _ => count,
                };

                let value_offset = if byte_size <= 4 {
                    entry_off + 8
                } else {
                    read_u32(entry_off + 8) as usize
                };

                if value_offset + byte_size <= data.len() {
                    return Some(data[value_offset..value_offset + byte_size].to_vec());
                }
            }
        }
        // Next IFD
        let next_off = ifd_offset + 2 + entry_count * 12;
        if next_off + 4 <= data.len() {
            ifd_offset = read_u32(next_off) as usize;
        } else {
            break;
        }
    }
    None
}
