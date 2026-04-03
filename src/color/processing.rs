//! 胶片类型处理：负片反转、黑白去色、统一色彩管线。
//!
//! 色阶调整（shadow/highlight/gray）和胶片曲线（film_curve LUT）已移至
//! `adjust.rs` 的 `apply_adjust_16`，由 UI 手柄控制。

// ─── Film Type Processing ───────────────────────────────────────────────────

use crate::flexcolor::ImageCorrection;
use super::adjust::ManualAdjust;
use super::transform::TargetColorSpace;

// ─── 胶片曲线 LUT ─────────────────────────────────────────────────────────
// 经验性逐通道色调曲线，适用于 FlexColor FilmCurve=4、Gamma=2 的配置。
// 通过像素级对比 16-bit 原始处理管道与 FlexColor 预渲染 8-bit 缩略图逆向工程得出，
// 测试素材为 Flextight X5 扫描的多卷 Portra 160。
//
// 将色阶输出 [0.0–1.0] 映射为显示值 [0–255]。
// 封装了：胶片响应曲线 + Gamma 编码。
// 在色阶调整之后由 apply_adjust_16 调用。

/// 红色通道胶片曲线 LUT（levels→FC→γ 空间）。
/// 从 FlexColor 导出的参考 TIF 逆向提取，适用于 levels 标准化后的输入。
pub const FILM_CURVE_LUT_R: [u8; 256] = [
      4,   4,   4,   4,   4,   4,   4,   4,   4,   4,   4,   4,   4,   4,   4,   6,
      6,   6,   6,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,
      7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   8,   8,   8,   9,   9,
     10,  10,  11,  11,  12,  12,  13,  13,  14,  15,  15,  16,  17,  18,  18,  19,
     20,  21,  22,  23,  24,  25,  26,  27,  29,  30,  31,  32,  34,  35,  37,  38,
     40,  41,  43,  44,  46,  47,  48,  49,  50,  51,  52,  53,  54,  55,  56,  57,
     58,  59,  60,  61,  63,  64,  65,  67,  68,  69,  70,  71,  72,  73,  75,  76,
     77,  78,  80,  81,  82,  84,  85,  86,  87,  88,  89,  91,  92,  93,  95,  96,
     97,  99, 100, 101, 102, 104, 105, 107, 108, 109, 111, 112, 113, 115, 116, 118,
    119, 120, 122, 123, 125, 126, 128, 129, 131, 132, 134, 135, 137, 138, 140, 141,
    143, 144, 146, 147, 149, 151, 152, 154, 156, 157, 159, 161, 162, 164, 166, 168,
    169, 171, 173, 175, 176, 178, 180, 182, 184, 185, 187, 189, 191, 193, 194, 196,
    198, 200, 202, 203, 205, 207, 209, 211, 213, 215, 217, 218, 220, 222, 224, 226,
    228, 230, 231, 233, 235, 237, 238, 240, 242, 243, 244, 245, 246, 247, 247, 248,
    248, 248, 249, 249, 249, 250, 250, 250, 250, 250, 250, 251, 251, 251, 251, 251,
    251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 254,
];

/// 绿色通道胶片曲线 LUT（levels→FC→γ 空间）。
pub const FILM_CURVE_LUT_G: [u8; 256] = [
      7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,
      7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,
      7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,
      7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,
      7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,   7,
      7,   7,   7,   7,   7,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,
      9,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,   9,
      9,  10,  10,  11,  12,  13,  14,  15,  16,  17,  19,  20,  21,  23,  24,  26,
     27,  29,  30,  32,  34,  36,  38,  40,  42,  44,  46,  48,  50,  51,  52,  54,
     55,  56,  58,  59,  61,  62,  64,  65,  67,  69,  70,  72,  73,  75,  76,  78,
     79,  81,  83,  85,  86,  88,  90,  92,  94,  96,  98,  99, 101, 103, 105, 107,
    109, 111, 113, 115, 117, 119, 121, 123, 125, 127, 129, 131, 133, 135, 138, 140,
    142, 144, 146, 149, 151, 154, 156, 159, 161, 164, 166, 169, 172, 175, 177, 180,
    183, 186, 189, 192, 195, 198, 201, 204, 207, 210, 214, 217, 220, 223, 227, 230,
    233, 237, 240, 243, 246, 249, 251, 252, 253, 253, 253, 253, 253, 253, 253, 253,
    253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 254, 255,
];

/// 蓝色通道胶片曲线 LUT（levels→FC→γ 空间）。
pub const FILM_CURVE_LUT_B: [u8; 256] = [
     14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,
     14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,
     14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,
     14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  14,  15,  15,  15,  15,
     15,  15,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,
     19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,  19,
     19,  19,  19,  19,  19,  19,  21,  22,  23,  24,  25,  26,  28,  29,  30,  31,
     33,  34,  36,  37,  39,  40,  42,  43,  45,  47,  48,  50,  52,  54,  56,  58,
     60,  62,  64,  66,  68,  70,  72,  73,  75,  76,  78,  80,  81,  83,  85,  86,
     88,  90,  92,  93,  95,  97,  98, 100, 102, 104, 106, 108, 110, 111, 113, 115,
    117, 119, 121, 123, 125, 127, 129, 131, 133, 135, 137, 139, 141, 143, 145, 147,
    149, 152, 154, 156, 158, 160, 163, 165, 167, 169, 172, 174, 176, 179, 181, 184,
    186, 189, 191, 194, 196, 199, 201, 204, 206, 209, 212, 214, 216, 219, 221, 223,
    225, 227, 229, 232, 234, 236, 238, 240, 242, 244, 245, 247, 248, 249, 250, 250,
    251, 251, 252, 253, 253, 253, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254,
    254, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
];

/// 对 256 项 LUT 进行线性插值，输入 [0, 1]，返回 16-bit 值（0–65535）。
#[inline]
pub fn lut_interp_16(val: f32, lut: &[u8; 256]) -> f32 {
    let x = val * 255.0;
    let lo = (x as usize).min(254);
    let hi = lo + 1;
    let frac = x - lo as f32;
    let out = lut[lo] as f32 * (1.0 - frac) + lut[hi] as f32 * frac;
    out * 257.0 // scale 0-255 → 0-65535
}

/// 应用胶片类型处理：负片反转 + 黑白去色。
///
/// - 彩色负片（FilmType=1）：基于 per-channel highlight 反转。
///   使用 `highlight[ch]*4 - val`（归一化到 [0, 65535]），相当于
///   测量像素值与底片基底密度（highlight）的距离。
/// - 黑白负片（FilmType=2）：同上反转后转为灰度。
/// - 正片（FilmType=0）：不做任何处理。
///
/// 色阶（shadow/highlight/gray）和胶片曲线（film_curve LUT）由 `apply_adjust_16` 处理。
pub fn apply_film_processing(
    img: &image::DynamicImage,
    correction: &ImageCorrection,
) -> image::DynamicImage {
    use rayon::prelude::*;

    let film_type = correction.film_type;
    let is_negative = film_type == 1 || film_type == 2;

    // 正片无需处理
    if !is_negative {
        return img.clone();
    }

    // 计算 per-channel 反转基准（highlight 值 × 4，14-bit → 16-bit）
    let hi_r = (correction.highlight[1] as f32) * 4.0;
    let hi_g = (correction.highlight[2] as f32) * 4.0;
    let hi_b = (correction.highlight[3] as f32) * 4.0;

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();

            let row_len = w as usize * 3;
            let mut out_pixels = vec![0u16; row_len * h as usize];

            // 预计算归一化缩放：inverted / hi * 65535
            let scale_r = if hi_r > 0.0 { 65535.0 / hi_r } else { 1.0 };
            let scale_g = if hi_g > 0.0 { 65535.0 / hi_g } else { 1.0 };
            let scale_b = if hi_b > 0.0 { 65535.0 / hi_b } else { 1.0 };

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        // highlight - val：底片基底密度减去扫描值，归一化到 [0, 65535]
                        let mut ch_f = [
                            (hi_r - src[si] as f32).max(0.0) * scale_r,
                            (hi_g - src[si + 1] as f32).max(0.0) * scale_g,
                            (hi_b - src[si + 2] as f32).max(0.0) * scale_b,
                        ];

                        if film_type == 2 {
                            let lum = 0.299 * ch_f[0] + 0.587 * ch_f[1] + 0.114 * ch_f[2];
                            ch_f = [lum, lum, lum];
                        }

                        row[base] = ch_f[0].clamp(0.0, 65535.0) as u16;
                        row[base + 1] = ch_f[1].clamp(0.0, 65535.0) as u16;
                        row[base + 2] = ch_f[2].clamp(0.0, 65535.0) as u16;
                    }
                });

            let buf = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(w, h, out_pixels)
                .expect("film_processing 16-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb16(buf)
        }
        _ => {
            let rgb8 = img.to_rgb8();
            let (w, h) = (rgb8.width(), rgb8.height());
            let src = rgb8.as_raw();
            let row_len = w as usize * 3;
            let mut out_pixels = vec![0u8; row_len * h as usize];

            // 8-bit 版本：highlight 缩放到 [0, 255]
            let hi8_r = hi_r / 257.0;
            let hi8_g = hi_g / 257.0;
            let hi8_b = hi_b / 257.0;
            let s8_r = if hi8_r > 0.0 { 255.0 / hi8_r } else { 1.0 };
            let s8_g = if hi8_g > 0.0 { 255.0 / hi8_g } else { 1.0 };
            let s8_b = if hi8_b > 0.0 { 255.0 / hi8_b } else { 1.0 };

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        let mut ch_f = [
                            (hi8_r - src[si] as f32).max(0.0) * s8_r,
                            (hi8_g - src[si + 1] as f32).max(0.0) * s8_g,
                            (hi8_b - src[si + 2] as f32).max(0.0) * s8_b,
                        ];

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

/// 应用胶片曲线 LUT（在负片反转之后、raw_rgb 保存之前调用）。
///
/// 仅当 `film_type ∈ {1,2}` 且 `film_curve == 4` 且 `gamma ≈ 2.0` 时生效。
pub fn apply_film_curve_lut(
    img: &image::DynamicImage,
    correction: &ImageCorrection,
) -> image::DynamicImage {
    let dominated = (correction.film_type == 1 || correction.film_type == 2)
        && correction.film_curve == 4
        && (correction.gamma - 2.0).abs() < 0.01;
    if !dominated {
        return img.clone();
    }

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            use rayon::prelude::*;
            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();
            let row_len = w as usize * 3;
            let mut out = vec![0u16; row_len * h as usize];

            out.par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        for ch in 0..3 {
                            let lut: &[u8; 256] = match ch {
                                0 => &FILM_CURVE_LUT_R,
                                1 => &FILM_CURVE_LUT_G,
                                _ => &FILM_CURVE_LUT_B,
                            };
                            let v = src[si + ch] as f32 / 65535.0;
                            row[base + ch] = lut_interp_16(v, lut) as u16;
                        }
                    }
                });

            let buf = image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(w, h, out)
                .expect("film_curve_lut 16-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb16(buf)
        }
        _ => {
            let rgb8 = img.to_rgb8();
            let (w, h) = (rgb8.width(), rgb8.height());
            let src = rgb8.as_raw();
            let mut out = Vec::with_capacity(src.len());
            for chunk in src.chunks_exact(3) {
                for ch in 0..3 {
                    let lut: &[u8; 256] = match ch {
                        0 => &FILM_CURVE_LUT_R,
                        1 => &FILM_CURVE_LUT_G,
                        _ => &FILM_CURVE_LUT_B,
                    };
                    out.push(lut[chunk[ch] as usize]);
                }
            }
            let buf = image::RgbImage::from_raw(w, h, out)
                .expect("film_curve_lut 8-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb8(buf)
        }
    }
}

// ─── Film Curve Extraction ───────────────────────────────────────────────────
// 从 FFF 文件的 8-bit 缩略图（FlexColor 预渲染）和 16-bit 预览（原始数据）
// 逆向提取逐通道胶片曲线 LUT。
//
// 原理：缩略图 = 全管线处理(inverted_raw)
// 反推：逆向全部处理效果后，残余映射即为纯胶片曲线。
// 反向处理包括：饱和度、CC矩阵、亮度/对比度/阴影深度、曝光、
// 输出色阶、gamma、色阶、渐变曲线。

/// 构建 256 级 LUT 的逆映射：对于每个目标输出 y，找到使 forward[x] 最接近 y 的 x。
fn invert_lut_256(forward: &[u8; 256]) -> [u8; 256] {
    let mut inv = [0u8; 256];
    for y in 0..256u16 {
        let mut best_x = 0u8;
        let mut best_dist = 256i32;
        for x in 0..256u16 {
            let dist = (forward[x as usize] as i32 - y as i32).abs();
            if dist < best_dist {
                best_dist = dist;
                best_x = x as u8;
            }
        }
        inv[y as usize] = best_x;
    }
    inv
}

/// 3×3 矩阵求逆（用于 CC 色彩校正矩阵的反向处理）。
fn invert_3x3(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-10 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let d = 1.0 / det;
    [
        [(m[1][1]*m[2][2] - m[1][2]*m[2][1])*d, (m[0][2]*m[2][1] - m[0][1]*m[2][2])*d, (m[0][1]*m[1][2] - m[0][2]*m[1][1])*d],
        [(m[1][2]*m[2][0] - m[1][0]*m[2][2])*d, (m[0][0]*m[2][2] - m[0][2]*m[2][0])*d, (m[0][2]*m[1][0] - m[0][0]*m[1][2])*d],
        [(m[1][0]*m[2][1] - m[1][1]*m[2][0])*d, (m[0][1]*m[2][0] - m[0][0]*m[2][1])*d, (m[0][0]*m[1][1] - m[0][1]*m[1][0])*d],
    ]
}

/// 从 8-bit 缩略图和同分辨率 16-bit 预览提取逐通道胶片曲线 LUT。
///
/// 返回 3 通道 × 65536 项 f32 (0.0-1.0) 的查找表。
/// 仅对负片（film_type=1 或 2）有效。
///
/// 注意：当校正包含较重的显示调整（对比度/亮度/阴影深度/CC/渐变曲线/非默认DotColor）
/// 时，反向处理不可靠（我们的公式与 FlexColor 不完全一致），此时返回 None
/// 让调用方回退到硬编码曲线。
pub fn extract_film_curve(
    thumb_8: &image::RgbImage,
    preview_16: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    correction: &crate::flexcolor::ImageCorrection,
) -> Option<[Vec<f32>; 3]> {
    let (w, h) = (thumb_8.width() as usize, thumb_8.height() as usize);
    if w != preview_16.width() as usize || h != preview_16.height() as usize {
        log::warn!("extract_film_curve: dimension mismatch {}x{} vs {}x{}",
            w, h, preview_16.width(), preview_16.height());
        return None;
    }

    let film_type = correction.film_type;
    if film_type != 1 && film_type != 2 {
        return None;
    }

    // 检测较重的显示调整 — 仅记录警告，仍然尝试提取
    // （之前会直接返回 None，导致使用不匹配的 Setting 参数时提取失败）
    let has_heavy_adjustments = {
        let has_cbl = correction.apply_sliders && (
            correction.contrast.abs() > 0
            || correction.brightness.abs() > 0
            || correction.lightness.abs() > 0
        );
        let has_cc = correction.apply_cc
            && correction.color_corr.len() == 36
            && correction.color_corr.iter().any(|&v| v != 0);
        let has_grad = correction.apply_curves
            && correction.gradations.len() >= 7
            && !correction.gradations.iter().all(|pts| {
                pts.len() == 2 && pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
            });
        let has_dot = correction.dot_color.len() >= 14
            && correction.dot_color.iter().enumerate().any(|(i, &v)| {
                // 每组前三个通道有效 [0..2]=R,G,B shadow, [7..9]=R,G,B highlight
                (i <= 2 && v != 0) || (i >= 7 && i <= 9 && v != 255)
            });
        has_cbl || has_cc || has_grad || has_dot
    };
    if has_heavy_adjustments {
        log::warn!(
            "extract_film_curve: heavy display adjustments detected \
             (contrast={}, brightness={}, lightness={}, CC={}, grad={}, dot={}) — \
             reversal may be approximate",
            correction.contrast, correction.brightness, correction.lightness,
            correction.apply_cc && correction.color_corr.iter().any(|&v| v != 0),
            correction.apply_curves,
            correction.dot_color.len() >= 14 && correction.dot_color.iter().enumerate().any(|(i, &v)| {
                (i <= 2 && v != 0) || (i >= 7 && i <= 9 && v != 255)
            }),
        );
        // 继续提取，不再返回 None
    }

    let n_pixels = w * h;
    let thumb_raw = thumb_8.as_raw();
    let prev_raw = preview_16.as_raw();

    // ── 反转参数（与 apply_film_processing 一致）──
    let hi = [
        correction.highlight[1] as f32 * 4.0,
        correction.highlight[2] as f32 * 4.0,
        correction.highlight[3] as f32 * 4.0,
    ];
    let scale = [
        if hi[0] > 0.0 { 65535.0 / hi[0] } else { 1.0 },
        if hi[1] > 0.0 { 65535.0 / hi[1] } else { 1.0 },
        if hi[2] > 0.0 { 65535.0 / hi[2] } else { 1.0 },
    ];

    // ── 色阶参数（与 load_levels_from_correction + apply_adjust_16 一致）──
    let mut bl = [0.0f32; 3];
    let mut wh_c = [0.0f32; 3];
    let mut gamma_c = [0.0f32; 3];
    for ch in 0..3 {
        bl[ch] = correction.shadow[ch + 1] as f32 * 4.0 / 65535.0;
        wh_c[ch] = correction.highlight[ch + 1] as f32 * 4.0 / 65535.0;
        gamma_c[ch] = (correction.gray[ch + 1] as f32 / 128.0).max(0.01);
    }
    let gamma_m = ((correction.gamma as f32) - 1.0).max(0.01);

    // 输出色阶 (DotColor) — 每组前三个通道有效：[0]=R, [1]=G, [2]=B; [7]=R, [8]=G, [9]=B
    let out_lo = if correction.dot_color.len() >= 14 {
        [
            correction.dot_color[0] as f32 / 255.0,
            correction.dot_color[1] as f32 / 255.0,
            correction.dot_color[2] as f32 / 255.0,
        ]
    } else { [0.0; 3] };
    let out_hi = if correction.dot_color.len() >= 14 {
        [
            correction.dot_color[7] as f32 / 255.0,
            correction.dot_color[8] as f32 / 255.0,
            correction.dot_color[9] as f32 / 255.0,
        ]
    } else { [1.0; 3] };
    let out_range = [
        (out_hi[0] - out_lo[0]).max(0.001),
        (out_hi[1] - out_lo[1]).max(0.001),
        (out_hi[2] - out_lo[2]).max(0.001),
    ];

    // 饱和度
    let sat = if correction.apply_sliders {
        correction.saturation as f32 / 100.0
    } else { 0.0 };

    // 曝光
    let exp_mult = if correction.apply_sliders && (correction.ev - 1.0).abs() > 0.001 {
        2.0f32.powf(correction.ev as f32 - 1.0)
    } else { 1.0 };

    // ── 显示调整参数（需要反向处理）──
    let contrast = if correction.apply_sliders { correction.contrast as f32 / 100.0 } else { 0.0 };
    let brightness = if correction.apply_sliders { correction.brightness as f32 / 100.0 } else { 0.0 };
    let lightness = if correction.apply_sliders { correction.lightness as f32 / 100.0 } else { 0.0 };

    // CC 矩阵逆
    let apply_cc = correction.apply_cc && correction.color_corr.len() == 36
        && correction.color_corr.iter().any(|&v| v != 0);
    let inv_cc = if apply_cc {
        let m = &correction.color_corr;
        let cc = [
            [(100 + m[0]) as f32 / 100.0, m[1] as f32 / 100.0,       m[2] as f32 / 100.0],
            [m[6] as f32 / 100.0,         (100 + m[7]) as f32 / 100.0, m[8] as f32 / 100.0],
            [m[12] as f32 / 100.0,        m[13] as f32 / 100.0,      (100 + m[14]) as f32 / 100.0],
        ];
        invert_3x3(cc)
    } else {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    };

    // 渐变曲线逆 LUT（在 scanner 空间中反向应用）
    let has_grad_curves = correction.apply_curves
        && correction.gradations.len() >= 7
        && !correction.gradations.iter().all(|pts| {
            pts.len() == 2 && pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
        });
    let inv_grad = if has_grad_curves {
        let lut_rgb = build_curve_lut(&correction.gradations[0]);
        let lut_r   = build_curve_lut(&correction.gradations[1]);
        let lut_g   = build_curve_lut(&correction.gradations[2]);
        let lut_b   = build_curve_lut(&correction.gradations[3]);
        let lut_c   = build_curve_lut(&correction.gradations[4]);
        let lut_m   = build_curve_lut(&correction.gradations[5]);
        let lut_y   = build_curve_lut(&correction.gradations[6]);
        Some([
            invert_lut_256(&lut_rgb),
            invert_lut_256(&lut_r),
            invert_lut_256(&lut_g),
            invert_lut_256(&lut_b),
            invert_lut_256(&lut_c),
            invert_lut_256(&lut_m),
            invert_lut_256(&lut_y),
        ])
    } else {
        None
    };

    // ── 构建映射 ──
    // 使用 1024 bins（缩略图像素有限，太多 bins 导致每 bin 样本不足）
    const BINS: usize = 1024;
    let mut sums = [[0.0f64; BINS]; 3];
    let mut counts = [[0u32; BINS]; 3];

    for y in 0..h {
        for x in 0..w {
            let pi = (y * w + x) * 3;

            // 1. 反转 16-bit 预览 → inverted values (0-65535)
            let mut inv = [0.0f32; 3];
            for ch in 0..3 {
                let raw_val = prev_raw[pi + ch] as f32;
                inv[ch] = ((hi[ch] - raw_val).max(0.0) * scale[ch]).clamp(0.0, 65535.0);
            }

            // B&W 负片灰度化
            if film_type == 2 {
                let lum = 0.299 * inv[0] + 0.587 * inv[1] + 0.114 * inv[2];
                inv = [lum, lum, lum];
            }

            // 2. 缩略图值 → 浮点
            let mut rgb = [
                thumb_raw[pi] as f32 / 255.0,
                thumb_raw[pi + 1] as f32 / 255.0,
                thumb_raw[pi + 2] as f32 / 255.0,
            ];

            // 3. 反向处理缩略图（从显示值恢复到胶片曲线输出）
            //    反向顺序：正向管线为 scanner→ICC→display→curves
            //    因此反向为：curves^(-1) → display^(-1) → ICC(skip) → scanner^(-1)

            // 3a. 反向渐变曲线（正向管线的最后一步，需首先反向）
            if let Some(ref ig) = inv_grad {
                let idx = |v: f32| -> usize { (v * 255.0).round().clamp(0.0, 255.0) as usize };
                // 逆 per-channel R/G/B（ig[1]=R, ig[2]=G, ig[3]=B）
                rgb[0] = ig[1][idx(rgb[0])] as f32 / 255.0;
                rgb[1] = ig[2][idx(rgb[1])] as f32 / 255.0;
                rgb[2] = ig[3][idx(rgb[2])] as f32 / 255.0;
                // 逆 CMY（C=ig[4], M=ig[5], Y=ig[6]）
                rgb[0] = 1.0 - ig[4][idx(1.0 - rgb[0])] as f32 / 255.0;
                rgb[1] = 1.0 - ig[5][idx(1.0 - rgb[1])] as f32 / 255.0;
                rgb[2] = 1.0 - ig[6][idx(1.0 - rgb[2])] as f32 / 255.0;
                // 逆 RGB 主通道
                for ch in 0..3 {
                    rgb[ch] = ig[0][idx(rgb[ch])] as f32 / 255.0;
                }
            }

            // 3b. 反向饱和度（cross-channel，正向管线中最后的 cross-channel 步骤）
            if sat.abs() > 0.001 {
                let lum = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
                for ch in 0..3 {
                    rgb[ch] = lum + (rgb[ch] - lum) / (1.0 + sat);
                }
            }

            // 3c. 反向 CC 矩阵
            if apply_cc {
                let r0 = rgb[0]; let g0 = rgb[1]; let b0 = rgb[2];
                rgb[0] = (inv_cc[0][0] * r0 + inv_cc[0][1] * g0 + inv_cc[0][2] * b0).clamp(0.0, 1.0);
                rgb[1] = (inv_cc[1][0] * r0 + inv_cc[1][1] * g0 + inv_cc[1][2] * b0).clamp(0.0, 1.0);
                rgb[2] = (inv_cc[2][0] * r0 + inv_cc[2][1] * g0 + inv_cc[2][2] * b0).clamp(0.0, 1.0);
            }

            // 3d. 反向 lightness（shadow depth）: forward = v^(1/(1+l)), reverse = v^(1+l)
            if lightness.abs() > 0.001 {
                let gamma = 1.0 / (1.0 + lightness).max(0.1);
                let inv_gamma = 1.0 / gamma;
                for ch in 0..3 {
                    rgb[ch] = rgb[ch].powf(inv_gamma).clamp(0.0, 1.0);
                }
            }

            // 3e. 反向 brightness: forward = v + b*0.5, reverse = v - b*0.5
            if brightness.abs() > 0.001 {
                for ch in 0..3 {
                    rgb[ch] = (rgb[ch] - brightness * 0.5).clamp(0.0, 1.0);
                }
            }

            // 3f. 反向 contrast: forward = (v-0.5)*scale+0.5, reverse = (v-0.5)/scale+0.5
            if contrast.abs() > 0.001 {
                let c_scale = if contrast >= 0.0 { 1.0 + contrast * 2.0 } else { 1.0 + contrast };
                let inv_scale = 1.0 / c_scale.max(0.001);
                for ch in 0..3 {
                    rgb[ch] = ((rgb[ch] - 0.5) * inv_scale + 0.5).clamp(0.0, 1.0);
                }
            }

            // 3g. 反向曝光
            if exp_mult != 1.0 {
                for ch in 0..3 {
                    rgb[ch] /= exp_mult;
                }
            }

            // 3h. 反向输出色阶 (per-channel DotColor)
            for ch in 0..3 {
                rgb[ch] = ((rgb[ch] - out_lo[ch]) / out_range[ch]).clamp(0.0, 1.0);
            }

            // 3i. 反向 master gamma
            for ch in 0..3 {
                rgb[ch] = rgb[ch].powf(gamma_m);
            }

            // 3j. 反向 per-channel gamma
            for ch in 0..3 {
                rgb[ch] = rgb[ch].powf(gamma_c[ch]);
            }

            // 注意：新管线顺序为 levels→FC→γ
            // 所以不再反向 levels（FC 操作在 levels 标准化之后的空间）

            // 4. 累积到 bins：使用 levels 标准化后的 inverted 值作为 FC 输入
            for ch in 0..3 {
                let range = (wh_c[ch] - bl[ch]).max(0.001);
                let normalized = ((inv[ch] / 65535.0 - bl[ch]) / range).clamp(0.0, 1.0);
                let bin = (normalized * (BINS - 1) as f32) as usize;
                let bin = bin.min(BINS - 1);
                sums[ch][bin] += rgb[ch] as f64;
                counts[ch][bin] += 1;
            }
        }
    }

    // ── 从 bins 构建 65536 项 LUT ──
    let mut luts: [Vec<f32>; 3] = [
        vec![0.0f32; 65536],
        vec![0.0f32; 65536],
        vec![0.0f32; 65536],
    ];

    for ch in 0..3 {
        // 计算有数据的 bin 的平均值
        let mut bin_avgs = vec![0.0f32; BINS];
        let mut valid_indices: Vec<usize> = Vec::new();
        let mut valid_values: Vec<f32> = Vec::new();
        for i in 0..BINS {
            if counts[ch][i] > 0 {
                let avg = (sums[ch][i] / counts[ch][i] as f64) as f32;
                bin_avgs[i] = avg;
                valid_indices.push(i);
                valid_values.push(avg);
            }
        }

        // 用线性插值填充空 bin（含首尾外推用边界值）
        if valid_indices.len() >= 2 {
            for i in 0..BINS {
                if counts[ch][i] == 0 {
                    // 找到左右最近的 valid bin 并线性插值
                    let right = valid_indices.partition_point(|&v| v <= i);
                    if right == 0 {
                        // 在首个 valid bin 之前：用首值
                        bin_avgs[i] = valid_values[0];
                    } else if right >= valid_indices.len() {
                        // 在末尾 valid bin 之后：用末值
                        bin_avgs[i] = *valid_values.last().unwrap();
                    } else {
                        // 在两个 valid bin 之间：线性插值
                        let li = valid_indices[right - 1];
                        let ri = valid_indices[right];
                        let frac = (i - li) as f32 / (ri - li) as f32;
                        bin_avgs[i] = valid_values[right - 1] * (1.0 - frac)
                            + valid_values[right] * frac;
                    }
                }
            }
        } else if valid_indices.len() == 1 {
            let v = valid_values[0];
            for i in 0..BINS {
                bin_avgs[i] = v;
            }
        }

        // 强制单调递增
        for i in 1..BINS {
            if bin_avgs[i] < bin_avgs[i - 1] {
                bin_avgs[i] = bin_avgs[i - 1];
            }
        }

        // 插值到 65536 项
        for i in 0..65536 {
            let pos = i as f32 / 65535.0 * (BINS - 1) as f32;
            let lo = (pos as usize).min(BINS - 2);
            let hi_idx = lo + 1;
            let frac = pos - lo as f32;
            luts[ch][i] = bin_avgs[lo] * (1.0 - frac) + bin_avgs[hi_idx] * frac;
        }
    }

    log::info!(
        "extract_film_curve: extracted from {}x{} ({} pixels), bins={}, lut[R][32768]={:.4}, lut[G][32768]={:.4}, lut[B][32768]={:.4}",
        w, h, n_pixels, BINS,
        luts[0][32768], luts[1][32768], luts[2][32768],
    );

    Some(luts)
}

// ─── Gradation Curves ───────────────────────────────────────────────────────
// FlexColor 使用 N 阶贝塞尔曲线（De Casteljau 算法）。
// 给定 n 个控制点，形成一条 (n-1) 阶贝塞尔曲线。
// 只有首尾两个控制点在曲线上，中间控制点在曲线外侧"吸引"曲线走向。
// 曲线始终在控制点的凸包内。
//
// De Casteljau 递归求值（参数 t ∈ [0,1]）：
//   第0层: P₀⁰=P₀, P₁⁰=P₁, ..., Pₙ₋₁⁰=Pₙ₋₁
//   第r层: Pᵢʳ = (1-t)·Pᵢʳ⁻¹ + t·Pᵢ₊₁ʳ⁻¹
//   最终值: P₀ⁿ⁻¹ 即为曲线上的点

/// 从控制点构建 256 级查找表（N 阶贝塞尔曲线，De Casteljau 算法）。
///
/// 控制点格式：(x, y, flag)，x/y 均为 0-255 范围。
/// 首尾点在曲线上，中间控制点在曲线外侧。
pub fn build_curve_lut(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    if n == 2 { return build_curve_lut_linear(points); }

    let num_samples = 1024;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(num_samples);

    for s in 0..num_samples {
        let t = s as f64 / (num_samples - 1) as f64;
        // De Casteljau 递归求值
        let mut work_x: Vec<f64> = pts.iter().map(|p| p.0).collect();
        let mut work_y: Vec<f64> = pts.iter().map(|p| p.1).collect();
        for level in 1..n {
            for i in 0..n - level {
                work_x[i] = (1.0 - t) * work_x[i] + t * work_x[i + 1];
                work_y[i] = (1.0 - t) * work_y[i] + t * work_y[i + 1];
            }
        }
        samples.push((work_x[0], work_y[0]));
    }

    samples_to_lut(&samples)
}

/// 预处理控制点：转 f64、排序、去重。
fn prepare_points(points: &[(i64, i64, i64)]) -> Vec<(f64, f64)> {
    let mut pts: Vec<(f64, f64)> = points.iter()
        .map(|&(x, y, _)| (x as f64, y as f64))
        .collect();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 0.5);
    pts
}

fn identity_lut() -> [u8; 256] {
    let mut lut = [0u8; 256];
    for i in 0..256 { lut[i] = i as u8; }
    lut
}

/// 二分搜索找到 x 所在区间。
fn find_segment(x: f64, pts: &[(f64, f64)]) -> (usize, usize) {
    let n = pts.len();
    let mut lo = 0;
    let mut hi = n - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if pts[mid].0 <= x { lo = mid; } else { hi = mid; }
    }
    (lo, hi)
}

fn build_curve_lut_linear(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    let mut lut = [0u8; 256];
    for i in 0..256 {
        let x = i as f64;
        if x <= pts[0].0 {
            lut[i] = pts[0].1.clamp(0.0, 255.0) as u8;
        } else if x >= pts[n - 1].0 {
            lut[i] = pts[n - 1].1.clamp(0.0, 255.0) as u8;
        } else {
            let (lo, hi) = find_segment(x, &pts);
            let dx = pts[hi].0 - pts[lo].0;
            let t = if dx.abs() < 1e-10 { 0.0 } else { (x - pts[lo].0) / dx };
            let y = pts[lo].1 * (1.0 - t) + pts[hi].1 * t;
            lut[i] = y.round().clamp(0.0, 255.0) as u8;
        }
    }
    lut
}

/// 从参数化采样点 (x, y) 构建 256 级 LUT。
fn samples_to_lut(samples: &[(f64, f64)]) -> [u8; 256] {
    let mut lut = [0u8; 256];
    let ns = samples.len();
    if ns == 0 { return identity_lut(); }
    for i in 0..256 {
        let x = i as f64;
        let y = if x <= samples[0].0 {
            samples[0].1
        } else if x >= samples[ns - 1].0 {
            samples[ns - 1].1
        } else {
            let mut lo = 0;
            let mut hi = ns - 1;
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if samples[mid].0 <= x { lo = mid; } else { hi = mid; }
            }
            let dx = samples[hi].0 - samples[lo].0;
            if dx.abs() < 1e-10 { samples[lo].1 }
            else {
                let t = (x - samples[lo].0) / dx;
                samples[lo].1 * (1.0 - t) + samples[hi].1 * t
            }
        };
        lut[i] = y.round().clamp(0.0, 255.0) as u8;
    }
    lut
}

/// 将渐变曲线应用到 RGB 图像上（自动处理 8-bit 和 16-bit）。
///
/// 渐变曲线通道顺序：[RGB主通道, R, G, B, C(青), M(品红), Y(黄)]
/// 应用顺序：先逐通道 R/G/B → CMY（反转通道）→ 主通道 RGB
pub fn apply_gradation_curves(img: &image::DynamicImage, gradations: &[Vec<(i64, i64, i64)>]) -> image::DynamicImage {
    if gradations.len() < 7 { return img.clone(); }

    let is_identity = |pts: &[(i64, i64, i64)]| -> bool {
        if pts.len() != 2 { return false; }
        pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
    };
    if gradations.iter().all(|ch| is_identity(ch)) { return img.clone(); }

    let lut_rgb = build_curve_lut(&gradations[0]);
    let lut_r   = build_curve_lut(&gradations[1]);
    let lut_g   = build_curve_lut(&gradations[2]);
    let lut_b   = build_curve_lut(&gradations[3]);
    let lut_c   = build_curve_lut(&gradations[4]);
    let lut_m   = build_curve_lut(&gradations[5]);
    let lut_y   = build_curve_lut(&gradations[6]);

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            use rayon::prelude::*;

            // 扩展 8-bit LUT 到 16-bit
            let expand = |lut8: &[u8; 256]| -> Vec<u16> {
                let mut lut16 = vec![0u16; 65536];
                for i in 0..65536u32 {
                    let pos = i as f64 / 257.0;
                    let lo = pos.floor() as usize;
                    let hi = (lo + 1).min(255);
                    let frac = pos - lo as f64;
                    let v = lut8[lo] as f64 * (1.0 - frac) + lut8[hi] as f64 * frac;
                    lut16[i as usize] = (v * 257.0).round().clamp(0.0, 65535.0) as u16;
                }
                lut16
            };

            let lr = expand(&lut_r);
            let lg = expand(&lut_g);
            let lb = expand(&lut_b);
            let lc = expand(&lut_c);
            let lm = expand(&lut_m);
            let ly = expand(&lut_y);
            let lrgb = expand(&lut_rgb);

            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();
            let row_len = w as usize * 3;
            let mut out = vec![0u16; row_len * h as usize];

            out.par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        let mut r = lr[src[si] as usize];
                        let mut g = lg[src[si + 1] as usize];
                        let mut b = lb[src[si + 2] as usize];

                        r = 65535 - lc[(65535 - r) as usize];
                        g = 65535 - lm[(65535 - g) as usize];
                        b = 65535 - ly[(65535 - b) as usize];

                        r = lrgb[r as usize];
                        g = lrgb[g as usize];
                        b = lrgb[b as usize];

                        row[base] = r;
                        row[base + 1] = g;
                        row[base + 2] = b;
                    }
                });

            let buf = image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(w, h, out)
                .expect("gradation 16-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb16(buf)
        }
        _ => {
            let rgb8 = img.to_rgb8();
            let (w, h) = (rgb8.width(), rgb8.height());
            let src = rgb8.as_raw();
            let mut out = Vec::with_capacity(src.len());
            for chunk in src.chunks_exact(3) {
                let mut r = lut_r[chunk[0] as usize];
                let mut g = lut_g[chunk[1] as usize];
                let mut b = lut_b[chunk[2] as usize];

                r = 255 - lut_c[(255 - r) as usize];
                g = 255 - lut_m[(255 - g) as usize];
                b = 255 - lut_y[(255 - b) as usize];

                r = lut_rgb[r as usize];
                g = lut_rgb[g as usize];
                b = lut_rgb[b as usize];

                out.push(r);
                out.push(g);
                out.push(b);
            }
            let buf = image::RgbImage::from_raw(w, h, out)
                .expect("gradation 8-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb8(buf)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn film_curve_lut_monotonic() {
        // 每个通道 LUT 必须单调非递减
        for (name, lut) in [("R", &FILM_CURVE_LUT_R), ("G", &FILM_CURVE_LUT_G), ("B", &FILM_CURVE_LUT_B)] {
            for i in 1..256 {
                assert!(
                    lut[i] >= lut[i - 1],
                    "{} channel LUT not monotonic at index {}: {} < {}",
                    name, i, lut[i], lut[i - 1]
                );
            }
        }
    }

    #[test]
    fn film_curve_lut_no_large_jumps() {
        // LUT 不应有超过 9 的跳变（平滑性检查）
        // 阈值 9 对应红色通道曲线末端 (index 255: 244→253) 的自然加速，
        // 为最大正常步进值。超过此值表明存在数据异常。
        for (name, lut) in [("R", &FILM_CURVE_LUT_R), ("G", &FILM_CURVE_LUT_G), ("B", &FILM_CURVE_LUT_B)] {
            for i in 1..256 {
                let diff = (lut[i] as i16 - lut[i - 1] as i16).unsigned_abs();
                assert!(
                    diff <= 9,
                    "{} channel LUT has jump of {} at index {} ({} → {})",
                    name, diff, i, lut[i - 1], lut[i]
                );
            }
        }
    }

    #[test]
    fn lut_interp_16_boundaries() {
        // 测试边界值：0.0 和 1.0
        let lut = &FILM_CURVE_LUT_G;
        let v0 = lut_interp_16(0.0, lut);
        let v1 = lut_interp_16(1.0, lut);
        assert_eq!(v0, lut[0] as f32 * 257.0);
        assert_eq!(v1, lut[255] as f32 * 257.0);
    }

    #[test]
    fn lut_interp_16_midpoint() {
        // 中间值应落在 LUT 范围内
        let lut = &FILM_CURVE_LUT_G;
        let v = lut_interp_16(0.5, lut);
        assert!(v >= 0.0 && v <= 65535.0, "midpoint value {} out of range", v);
    }

    #[test]
    fn build_curve_lut_identity() {
        // 两点对角线 [(0,0), (255,255)] 应产生恒等映射
        let pts = vec![(0i64, 0i64, 0i64), (255, 255, 0)];
        let lut = build_curve_lut(&pts);
        for i in 0..256 {
            assert_eq!(lut[i], i as u8, "identity LUT mismatch at {}", i);
        }
    }

    #[test]
    fn build_curve_lut_invert() {
        // [(0,255), (255,0)] 应产生反转映射
        let pts = vec![(0i64, 255i64, 0i64), (255, 0, 0)];
        let lut = build_curve_lut(&pts);
        assert_eq!(lut[0], 255);
        assert_eq!(lut[255], 0);
        assert!((lut[128] as i16 - 127).abs() <= 1, "invert midpoint {} != ~127", lut[128]);
    }

    #[test]
    fn build_curve_lut_monotonic_with_three_points() {
        // 三个点构成的曲线应保持单调性（Fritsch-Carlson 保证）
        let pts = vec![(0i64, 0i64, 0i64), (128, 200, 0), (255, 255, 0)];
        let lut = build_curve_lut(&pts);
        for i in 1..256 {
            assert!(
                lut[i] >= lut[i - 1],
                "three-point curve not monotonic at {}: {} < {}",
                i, lut[i], lut[i - 1]
            );
        }
    }

    #[test]
    fn build_curve_lut_single_point_identity() {
        // 单点应返回恒等映射
        let pts = vec![(128i64, 128i64, 0i64)];
        let lut = build_curve_lut(&pts);
        for i in 0..256 {
            assert_eq!(lut[i], i as u8, "single-point LUT should be identity at {}", i);
        }
    }

    #[test]
    fn gradation_curves_identity_passthrough() {
        // 恒等曲线不应修改图像
        let w = 4u32;
        let h = 2u32;
        let pixels: Vec<u8> = (0..w * h * 3).map(|i| (i % 256) as u8).collect();
        let img = image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(w, h, pixels.clone()).unwrap(),
        );
        let identity_grads: Vec<Vec<(i64, i64, i64)>> = (0..7)
            .map(|_| vec![(0, 0, 0), (255, 255, 0)])
            .collect();
        let result = apply_gradation_curves(&img, &identity_grads);
        let out = result.to_rgb8();
        assert_eq!(out.as_raw(), &pixels);
    }
}

// ─── 统一色彩处理管线 ─────────────────────────────────────────────────────

/// 统一色彩处理管线：渐变曲线 → 扫描仪色阶 → ICC → 显示调整。
///
/// 输入应为已完成胶片处理（负片反转）的 scanner 空间图像。
/// 渲染、单文件导出和分割导出均通过此函数保证管线一致。
pub fn apply_color_pipeline(
    img: image::DynamicImage,
    adjust: &ManualAdjust,
    curve_points: &[Vec<(i64, i64, i64)>],
    film_lut: Option<&[Vec<f32>; 3]>,
    icc_data: Option<&[u8]>,
    target_color_space: TargetColorSpace,
) -> image::DynamicImage {
    // 1. 扫描仪空间色阶（film_curve + levels + gamma）— 在 ICC 之前
    let img = super::adjust::apply_scanner_levels(&img, adjust, film_lut);

    // 2. ICC 色彩空间转换（扫描仪 → 输出色域）
    let img = if let Some(icc) = icc_data {
        match super::transform::apply_icc_transform(&img, icc, target_color_space) {
            Ok(transformed) => transformed,
            Err(e) => {
                log::warn!("ICC transform failed: {}", e);
                img
            }
        }
    } else {
        img
    };

    // 2b. B&W 负片：ICC 后重新去色（ICC 会将 R=G=B 灰度数据变为非灰度）
    let img = if adjust.film_type == 2 {
        desaturate_bw(&img)
    } else {
        img
    };

    // 3. 显示空间调整（曝光/对比度/亮度/饱和度等）— 在 ICC 之后
    let img = super::adjust::apply_display_adjust(&img, adjust);

    // 4. 渐变曲线 — 在显示空间最后应用（FlexColor 行为）
    let curves_are_identity = curve_points.iter().all(|pts| {
        pts.len() == 2
            && pts[0].0 == 0 && pts[0].1 == 0
            && pts[1].0 == 255 && pts[1].1 == 255
    });
    if adjust.apply_curves
        && curve_points.len() >= 7
        && !curves_are_identity
    {
        apply_gradation_curves(&img, curve_points)
    } else {
        img
    }
}

/// B&W 去色：将 RGB 转为灰度（ITU-R BT.601 权重），保持 Rgb16/Rgb8 格式
fn desaturate_bw(img: &image::DynamicImage) -> image::DynamicImage {
    use rayon::prelude::*;
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();
            let row_len = w as usize * 3;
            let mut out = vec![0u16; row_len * h as usize];
            out.par_chunks_mut(row_len).enumerate().for_each(|(y, row)| {
                let s = y * row_len;
                for x in 0..w as usize {
                    let b = x * 3;
                    let r = src[s + b] as f32;
                    let g = src[s + b + 1] as f32;
                    let bv = src[s + b + 2] as f32;
                    let lum = (0.299 * r + 0.587 * g + 0.114 * bv).round() as u16;
                    row[b] = lum;
                    row[b + 1] = lum;
                    row[b + 2] = lum;
                }
            });
            image::DynamicImage::ImageRgb16(
                image::ImageBuffer::from_raw(w, h, out).unwrap())
        },
        other => {
            let rgb8 = other.to_rgb8();
            let (w, h) = (rgb8.width(), rgb8.height());
            let mut out = image::RgbImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let p = rgb8.get_pixel(x, y);
                    let lum = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
                        .round().clamp(0.0, 255.0) as u8;
                    out.put_pixel(x, y, image::Rgb([lum, lum, lum]));
                }
            }
            image::DynamicImage::ImageRgb8(out)
        }
    }
}
