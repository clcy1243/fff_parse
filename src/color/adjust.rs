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
