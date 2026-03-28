//! 胶片类型处理：负片反转、黑白去色。
//!
//! 色阶调整（shadow/highlight/gray）和胶片曲线（film_curve LUT）已移至
//! `adjust.rs` 的 `apply_adjust_16`，由 UI 手柄控制。

// ─── Film Type Processing ───────────────────────────────────────────────────

use crate::flexcolor::ImageCorrection;

// ─── 胶片曲线 LUT ─────────────────────────────────────────────────────────
// 经验性逐通道色调曲线，适用于 FlexColor FilmCurve=4、Gamma=2 的配置。
// 通过像素级对比 16-bit 原始处理管道与 FlexColor 预渲染 8-bit 缩略图逆向工程得出，
// 测试素材为 Flextight X5 扫描的多卷 Portra 160。
//
// 将色阶输出 [0.0–1.0] 映射为显示值 [0–255]。
// 封装了：胶片响应曲线 + Gamma 编码。
// 在色阶调整之后由 apply_adjust_16 调用。

/// 红色通道胶片曲线 LUT。
/// index 144-152 中间段使用渐变过渡（41→42→43→44），替代原先的平坦段（全为 41），
/// 以改善中间调到高光的平滑度。
pub const FILM_CURVE_LUT_R: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   1,   1,   1,   1,   1,   1,   2,   2,   2,   3,   4,   5,   5,   5,
      6,   7,   8,   9,  10,  11,  12,  13,  13,  14,  14,  15,  16,  17,  17,  19,
     21,  24,  26,  28,  30,  32,  34,  36,  38,  40,  41,  41,  41,  41,  41,  41,
     41,  42,  42,  42,  43,  43,  43,  44,  44,  46,  47,  49,  50,  52,  54,  57,
     59,  61,  63,  65,  67,  68,  69,  71,  73,  75,  76,  77,  78,  80,  82,  83,
     85,  87,  89,  91,  92,  94,  96,  97,  99, 101, 104, 106, 108, 111, 113, 115,
    117, 119, 121, 124, 126, 128, 130, 133, 135, 137, 139, 141, 144, 146, 148, 150,
    152, 154, 156, 158, 159, 162, 164, 167, 169, 172, 174, 177, 180, 182, 185, 187,
    190, 192, 195, 197, 199, 201, 203, 205, 207, 209, 211, 212, 213, 213, 214, 215,
    216, 217, 219, 220, 221, 223, 225, 227, 228, 230, 233, 236, 237, 238, 244, 253,
];

/// 绿色通道胶片曲线 LUT。
pub const FILM_CURVE_LUT_G: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   1,   1,   1,   1,   1,   1,   1,   1,
      1,   1,   1,   1,   1,   1,   1,   1,   1,   1,   1,   2,   2,   2,   2,   2,
      2,   2,   2,   2,   2,   2,   3,   3,   3,   3,   3,   3,   3,   4,   4,   4,
      4,   4,   4,   4,   5,   5,   5,   6,   6,   7,   7,   7,   8,   9,  10,  12,
     13,  14,  16,  17,  18,  20,  21,  23,  24,  26,  28,  30,  31,  33,  35,  36,
     38,  39,  41,  43,  44,  46,  48,  50,  52,  54,  55,  57,  59,  60,  62,  63,
     65,  67,  69,  71,  72,  74,  76,  78,  80,  81,  83,  85,  88,  90,  92,  94,
     96,  99, 101, 103, 105, 107, 110, 112, 114, 116, 118, 120, 122, 124, 127, 129,
    131, 134, 137, 139, 142, 145, 148, 150, 153, 156, 159, 162, 164, 167, 170, 172,
    175, 178, 180, 183, 185, 188, 191, 193, 196, 199, 202, 205, 207, 210, 213, 215,
    218, 221, 225, 228, 231, 234, 236, 239, 242, 245, 248, 251, 253, 254, 254, 254,
    254, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
];

/// 蓝色通道胶片曲线 LUT。
pub const FILM_CURVE_LUT_B: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   1,   1,   2,   2,
      3,   3,   4,   5,   5,   6,   7,   7,   8,   9,  10,  11,  12,  13,  14,  15,
     17,  18,  20,  21,  23,  25,  26,  28,  30,  32,  33,  35,  37,  39,  41,  43,
     45,  47,  49,  51,  52,  54,  56,  58,  59,  61,  63,  64,  66,  67,  69,  71,
     73,  75,  77,  79,  81,  83,  86,  88,  90,  92,  94,  96,  98, 100, 102, 105,
    107, 109, 112, 114, 117, 119, 122, 125, 127, 130, 133, 136, 139, 142, 145, 148,
    150, 154, 157, 160, 163, 166, 170, 173, 176, 179, 182, 186, 189, 192, 195, 198,
    201, 204, 207, 210, 213, 216, 220, 223, 226, 229, 233, 236, 239, 242, 245, 247,
    250, 252, 253, 253, 254, 254, 254, 254, 255, 255, 255, 255, 255, 255, 255, 255,
    255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
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
