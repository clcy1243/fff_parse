//! 手动图像调整：曝光、对比度、高光/阴影、饱和度、色彩平衡和色阶。

/// 手动图像调整参数，在色彩管道之后应用。
#[derive(Debug, Clone, PartialEq)]
pub struct ManualAdjust {
    pub enabled: bool,
    /// 曝光补偿（档位）：-3.0 ~ 3.0
    pub exposure: f32,
    /// 对比度：-100 ~ 100
    pub contrast: f32,
    /// 高光：-100 ~ 100
    pub highlights: f32,
    /// 阴影：-100 ~ 100
    pub shadows: f32,
    /// 饱和度：-100 ~ 100
    pub saturation: f32,
    /// 红色通道色彩平衡：-100 ~ 100
    pub r_shift: f32,
    /// 绿色通道色彩平衡：-100 ~ 100
    pub g_shift: f32,
    /// 蓝色通道色彩平衡：-100 ~ 100
    pub b_shift: f32,

    // 色阶（输入范围）：索引 0=总通道, 1=R, 2=G, 3=B
    /// 输入黑点：0-255
    pub levels_black: [f32; 4],
    /// 中间调 Gamma：0.10-9.99（1.0 为中性）
    pub levels_gamma: [f32; 4],
    /// 输入白点：0-255
    pub levels_white: [f32; 4],
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
    /// 判断当前调整参数是否为恒等变换（即不产生任何效果）。
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

/// 对图像应用手动调整（曝光、对比度、阴影/高光、饱和度、色彩平衡、色阶）。
///
/// 支持 16-bit 和 8-bit 输入，始终返回与输入相同的位深。
/// 色阶参数 (levels_black/white) 范围为 0-255（用户空间），内部自动映射到实际位深。
/// 使用 65536 项 LUT（16-bit）或 256 项 LUT（8-bit）提升性能。
pub fn apply_manual_adjust(img: &image::DynamicImage, adj: &ManualAdjust) -> image::DynamicImage {
    if adj.is_identity() {
        return img.clone();
    }

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            image::DynamicImage::ImageRgb16(apply_adjust_16(rgb16, adj))
        }
        _ => {
            let rgb8 = img.to_rgb8();
            image::DynamicImage::ImageRgb8(apply_adjust_8(&rgb8, adj))
        }
    }
}

/// 16-bit 手动调整实现：65536 项 per-channel LUT + 逐像素饱和度
fn apply_adjust_16(
    rgb16: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    adj: &ManualAdjust,
) -> image::ImageBuffer<image::Rgb<u16>, Vec<u16>> {
    use rayon::prelude::*;

    let (w, h) = (rgb16.width(), rgb16.height());
    let src = rgb16.as_raw();

    let exposure_mult = 2.0_f32.powf(adj.exposure);
    let sat = adj.saturation / 100.0;

    // 色阶参数从用户空间 (0-255) 映射到归一化 (0-1)
    let bl_m = adj.levels_black[0] / 255.0;
    let wh_m = adj.levels_white[0] / 255.0;
    let range_m = (wh_m - bl_m).max(0.001);
    let gamma_m = adj.levels_gamma[0].clamp(0.01, 99.0);

    // 构建 65536 项 per-channel LUT
    let shifts = [adj.r_shift / 255.0, adj.g_shift / 255.0, adj.b_shift / 255.0];
    let mut luts: Vec<Vec<u16>> = Vec::with_capacity(3);

    for ch in 0..3 {
        let bl_c = adj.levels_black[ch + 1] / 255.0;
        let wh_c = adj.levels_white[ch + 1] / 255.0;
        let range_c = (wh_c - bl_c).max(0.001);
        let gamma_c = adj.levels_gamma[ch + 1].clamp(0.01, 99.0);

        let mut lut = vec![0u16; 65536];
        for i in 0..65536u32 {
            let mut v = i as f32 / 65535.0;

            v = ((v - bl_m) / range_m).clamp(0.0, 1.0).powf(1.0 / gamma_m);
            v = ((v - bl_c) / range_c).clamp(0.0, 1.0).powf(1.0 / gamma_c);

            v += shifts[ch];
            v *= exposure_mult;
            v = v.clamp(0.0, 1.0);

            if adj.shadows.abs() > 0.1 {
                let s = adj.shadows / 100.0;
                let t = 1.0 - v;
                v = (v + s * t * t * 0.5).clamp(0.0, 1.0);
            }
            if adj.highlights.abs() > 0.1 {
                let hi = adj.highlights / 100.0;
                let t = v;
                v = (v + hi * t * t * 0.5).clamp(0.0, 1.0);
            }
            if adj.contrast.abs() > 0.1 {
                let c = adj.contrast / 100.0;
                let scale = if c >= 0.0 { 1.0 + c * 2.0 } else { 1.0 + c };
                v = ((v - 0.5) * scale + 0.5).clamp(0.0, 1.0);
            }

            lut[i as usize] = (v * 65535.0) as u16;
        }
        luts.push(lut);
    }

    let row_len = w as usize * 3;
    let mut out = vec![0u16; row_len * h as usize];

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

                if sat.abs() > 0.001 {
                    let lum = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
                    rf = (lum + (rf - lum) * (1.0 + sat)).clamp(0.0, 65535.0);
                    gf = (lum + (gf - lum) * (1.0 + sat)).clamp(0.0, 65535.0);
                    bf = (lum + (bf - lum) * (1.0 + sat)).clamp(0.0, 65535.0);
                }

                row[base] = rf as u16;
                row[base + 1] = gf as u16;
                row[base + 2] = bf as u16;
            }
        });

    image::ImageBuffer::from_raw(w, h, out).expect("manual_adjust_16 buffer mismatch")
}

/// 8-bit 手动调整实现（保留原有逻辑）
fn apply_adjust_8(rgb8: &image::RgbImage, adj: &ManualAdjust) -> image::RgbImage {
    use rayon::prelude::*;

    let (w, h) = (rgb8.width(), rgb8.height());
    let src = rgb8.as_raw();

    let exposure_mult = 2.0_f32.powf(adj.exposure);
    let sat = adj.saturation / 100.0;

    let bl_m = adj.levels_black[0] / 255.0;
    let wh_m = adj.levels_white[0] / 255.0;
    let range_m = (wh_m - bl_m).max(0.001);
    let gamma_m = adj.levels_gamma[0].clamp(0.01, 99.0);

    let mut luts = [[0u8; 256]; 3];
    let shifts = [adj.r_shift / 255.0, adj.g_shift / 255.0, adj.b_shift / 255.0];

    for ch in 0..3 {
        let bl_c = adj.levels_black[ch + 1] / 255.0;
        let wh_c = adj.levels_white[ch + 1] / 255.0;
        let range_c = (wh_c - bl_c).max(0.001);
        let gamma_c = adj.levels_gamma[ch + 1].clamp(0.01, 99.0);

        for i in 0..=255u32 {
            let mut v = i as f32 / 255.0;
            v = ((v - bl_m) / range_m).clamp(0.0, 1.0).powf(1.0 / gamma_m);
            v = ((v - bl_c) / range_c).clamp(0.0, 1.0).powf(1.0 / gamma_c);
            v += shifts[ch];
            v *= exposure_mult;
            v = v.clamp(0.0, 1.0);
            if adj.shadows.abs() > 0.1 {
                let s = adj.shadows / 100.0;
                let t = 1.0 - v;
                v = (v + s * t * t * 0.5).clamp(0.0, 1.0);
            }
            if adj.highlights.abs() > 0.1 {
                let hi = adj.highlights / 100.0;
                let t = v;
                v = (v + hi * t * t * 0.5).clamp(0.0, 1.0);
            }
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

    image::RgbImage::from_raw(w, h, out).expect("manual_adjust_8 buffer mismatch")
}

/// 从 FFF 文件的 TIFF 标签 0xC51A（ImaconProfileData）中提取嵌入的 ICC 配置文件。
///
/// 验证数据是否为有效的 ICC 配置文件（偏移 36 处应为 "acsp" 签名）。
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

/// 从 TIFF 文件数据中读取指定标签的原始字节。
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
