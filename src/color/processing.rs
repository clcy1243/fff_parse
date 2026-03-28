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

/// 曲线插值方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveMethod {
    /// Bézier (De Casteljau) — 单条 n 阶贝塞尔曲线，经过首尾点
    BezierDeCasteljau,
    /// Bézier (Composite Quadratic) — 分段二次贝塞尔，中间点在曲线外
    BezierCompositeQuad,
    /// Bézier (Composite Cubic) — 分段三次贝塞尔，中间点在曲线外
    BezierCompositeCubic,
    /// Clamped Cubic B-Spline — 经过首尾点的逼近曲线
    BSplineClamped,
    /// Uniform Cubic B-Spline — 均匀三次 B 样条
    BSplineUniform,
    /// Quadratic B-Spline — 二次 B 样条
    BSplineQuadratic,
    /// Catmull-Rom 样条 — 通过所有控制点
    CatmullRom,
    /// 线性插值 — 折线基准
    Linear,
}

impl CurveMethod {
    pub const ALL: [CurveMethod; 8] = [
        CurveMethod::BezierDeCasteljau,
        CurveMethod::BezierCompositeQuad,
        CurveMethod::BezierCompositeCubic,
        CurveMethod::BSplineClamped,
        CurveMethod::BSplineUniform,
        CurveMethod::BSplineQuadratic,
        CurveMethod::CatmullRom,
        CurveMethod::Linear,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            CurveMethod::BezierDeCasteljau => "Bézier (N-degree)",
            CurveMethod::BezierCompositeQuad => "Bézier (Quad)",
            CurveMethod::BezierCompositeCubic => "Bézier (Cubic)",
            CurveMethod::BSplineClamped => "B-Spline (Clamped)",
            CurveMethod::BSplineUniform => "B-Spline (Uniform)",
            CurveMethod::BSplineQuadratic => "B-Spline (Quadratic)",
            CurveMethod::CatmullRom => "Catmull-Rom",
            CurveMethod::Linear => "Linear",
        }
    }
}

/// 从控制点构建 256 级查找表，使用指定的插值方法。
pub fn build_curve_lut_with_method(points: &[(i64, i64, i64)], method: CurveMethod) -> [u8; 256] {
    match method {
        CurveMethod::BezierDeCasteljau => build_curve_lut_bezier_decasteljau(points),
        CurveMethod::BezierCompositeQuad => build_curve_lut_bezier_composite_quad(points),
        CurveMethod::BezierCompositeCubic => build_curve_lut_bezier_composite_cubic(points),
        CurveMethod::BSplineClamped => build_curve_lut_bspline_clamped(points),
        CurveMethod::BSplineUniform => build_curve_lut_bspline_uniform(points),
        CurveMethod::BSplineQuadratic => build_curve_lut_bspline_quadratic(points),
        CurveMethod::CatmullRom => build_curve_lut_catmull_rom(points),
        CurveMethod::Linear => build_curve_lut_linear(points),
    }
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

// ─── Bézier (De Casteljau) ──────────────────────────────────────────────────
// 单条 n 阶贝塞尔曲线，所有控制点作为贝塞尔控制点。
// 只有首尾点在曲线上，中间控制点在曲线外侧。

fn build_curve_lut_bezier_decasteljau(points: &[(i64, i64, i64)]) -> [u8; 256] {
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

// ─── Bézier (Composite Quadratic) ───────────────────────────────────────────
// 分段二次贝塞尔曲线（类似 TrueType 字体方式）。
// 给定控制点直接作为二次贝塞尔的控制点，段间连接点取相邻控制点中点。
// 首尾点在曲线上，中间控制点在曲线外侧。

fn build_curve_lut_bezier_composite_quad(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    if n == 2 { return build_curve_lut_linear(points); }

    // 生成 on-curve 连接点（相邻控制点中点）+ 首尾端点
    // 段 i: P_start → ctrl[i+1] → P_end
    // P_start = midpoint(ctrl[i], ctrl[i+1])  (or ctrl[0] for first)
    // P_end   = midpoint(ctrl[i+1], ctrl[i+2])  (or ctrl[n-1] for last)
    let num_samples = 1024;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(num_samples);

    let num_seg = n - 1;
    let samples_per_seg = num_samples / num_seg;

    for seg in 0..num_seg {
        // 起点
        let (sx, sy) = if seg == 0 {
            pts[0]
        } else {
            ((pts[seg].0 + pts[seg + 1].0) * 0.5, (pts[seg].1 + pts[seg + 1].1) * 0.5)
        };
        // 终点
        let (ex, ey) = if seg == num_seg - 1 {
            pts[n - 1]
        } else {
            ((pts[seg + 1].0 + pts[seg + 2].0) * 0.5, (pts[seg + 1].1 + pts[seg + 2].1) * 0.5)
        };
        // 控制点: 对于第0段用pts[1]，其余用pts[seg+1]（但第0段也是seg+1=1）
        // 第一段：start=pts[0], ctrl=pts[1], end=mid(pts[1],pts[2])
        // 中间段：start=mid(pts[seg],pts[seg+1]), ctrl=pts[seg+1]（但这不对）
        // 实际上对于 n 个控制点，有 n-2 个中间控制点形成 n-2 段（或 n-1 段？）
        // TrueType 方式：n-1 段，每段有一个控制点
        // 段 seg: 控制点是 pts[seg] 和 pts[seg+1] 之间的那个... 
        // 重新设计：控制点序列中，奇数索引是 off-curve，偶数是 on-curve
        // 但我们的情况是所有中间点都是 off-curve
        // 正确做法：对于 n 个点（首尾 on-curve，中间 off-curve），有 n-2 个 off-curve 点
        // 产生 max(1, n-2) 段。段间连接点为相邻 off-curve 点的中点。
        
        // 控制点就是 pts[seg] 到 pts[seg+1] 区间中的那个 off-curve 点
        // 简化：每段使用当前段的两端点和它们中间对应的控制点
        let (cx, cy) = if n == 3 {
            // 只有一个中间控制点
            pts[1]
        } else if seg == 0 {
            // 第一段：控制点 = pts[1]（第一个中间点）
            pts[1]
        } else if seg == num_seg - 1 {
            // 最后一段：控制点 = pts[n-2]（最后一个中间点）
            pts[n - 2]
        } else {
            // 中间段：控制点 = pts[seg+1]（注意 seg 从0开始，第一个中间控制点从1开始）
            pts[seg + 1]
        };

        let count = if seg == num_seg - 1 { num_samples - samples.len() } else { samples_per_seg };
        for i in 0..count {
            let t = i as f64 / count as f64;
            let mt = 1.0 - t;
            // 二次贝塞尔: B(t) = (1-t)²·P0 + 2(1-t)t·P1 + t²·P2
            let x = mt * mt * sx + 2.0 * mt * t * cx + t * t * ex;
            let y = mt * mt * sy + 2.0 * mt * t * cy + t * t * ey;
            samples.push((x, y));
        }
    }

    samples_to_lut(&samples)
}

// ─── Bézier (Composite Cubic) ───────────────────────────────────────────────
// 分段三次贝塞尔曲线。给定控制点序列，首尾在曲线上，中间控制点在曲线外。
// 每两个相邻控制点之间自动在中点处分段连接。

fn build_curve_lut_bezier_composite_cubic(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    if n == 2 { return build_curve_lut_linear(points); }
    if n == 3 { return build_curve_lut_bezier_composite_quad(points); }

    // 对于 n >= 4，构建分段三次贝塞尔
    // 策略：每 3 个连续中间点形成一个三次段（2 个控制点 + 2 个端点）
    // 端点取相邻控制点中点，首尾除外
    
    // 生成 on-curve 节点列表（首尾 + 中间控制点对之间的中点）
    let interior = &pts[1..n - 1]; // 中间控制点
    let ni = interior.len();
    
    // 每两个中间控制点形成一个三次段
    // 如果中间点是奇数个，最后一段退化为二次
    let num_samples = 1024;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(num_samples);

    // 构建段：每段有 start, ctrl1, ctrl2, end
    struct CubicSeg { s: (f64, f64), c1: (f64, f64), c2: (f64, f64), e: (f64, f64) }
    let mut segs: Vec<CubicSeg> = Vec::new();

    let mut i = 0;
    while i < ni {
        if i + 1 < ni {
            // 有两个控制点可用，形成一个三次段
            let start = if i == 0 {
                pts[0]
            } else {
                // 中点
                let prev = interior[i - 1];
                let cur = interior[i];
                ((prev.0 + cur.0) * 0.5, (prev.1 + cur.1) * 0.5)
            };
            let end = if i + 2 >= ni {
                pts[n - 1]
            } else {
                let cur = interior[i + 1];
                let next = interior[i + 2];
                ((cur.0 + next.0) * 0.5, (cur.1 + next.1) * 0.5)
            };
            segs.push(CubicSeg {
                s: start,
                c1: interior[i],
                c2: interior[i + 1],
                e: end,
            });
            i += 2;
        } else {
            // 奇数个中间控制点，最后一个单独形成二次段（提升为三次）
            let start = if i == 0 {
                pts[0]
            } else {
                let prev = interior[i - 1];
                let cur = interior[i];
                ((prev.0 + cur.0) * 0.5, (prev.1 + cur.1) * 0.5)
            };
            let end = pts[n - 1];
            let ctrl = interior[i];
            // 二次提升为三次：C1 = S + 2/3*(C-S), C2 = E + 2/3*(C-E)
            let c1 = (start.0 + 2.0 / 3.0 * (ctrl.0 - start.0), start.1 + 2.0 / 3.0 * (ctrl.1 - start.1));
            let c2 = (end.0 + 2.0 / 3.0 * (ctrl.0 - end.0), end.1 + 2.0 / 3.0 * (ctrl.1 - end.1));
            segs.push(CubicSeg { s: start, c1, c2, e: end });
            i += 1;
        }
    }

    if segs.is_empty() { return identity_lut(); }

    let samples_per_seg = num_samples / segs.len();
    for (si, seg) in segs.iter().enumerate() {
        let count = if si == segs.len() - 1 { num_samples - samples.len() } else { samples_per_seg };
        for j in 0..count {
            let t = j as f64 / count as f64;
            let mt = 1.0 - t;
            // 三次贝塞尔: B(t) = (1-t)³·P0 + 3(1-t)²t·P1 + 3(1-t)t²·P2 + t³·P3
            let x = mt.powi(3) * seg.s.0 + 3.0 * mt * mt * t * seg.c1.0 + 3.0 * mt * t * t * seg.c2.0 + t.powi(3) * seg.e.0;
            let y = mt.powi(3) * seg.s.1 + 3.0 * mt * mt * t * seg.c1.1 + 3.0 * mt * t * t * seg.c2.1 + t.powi(3) * seg.e.1;
            samples.push((x, y));
        }
    }

    samples_to_lut(&samples)
}

fn identity_lut() -> [u8; 256] {
    let mut lut = [0u8; 256];
    for i in 0..256 { lut[i] = i as u8; }
    lut
}

// ─── Clamped Cubic B-Spline ─────────────────────────────────────────────────
// 经过首尾控制点的三次 B 样条。通过在首尾重复节点实现端点插值。
// 控制点不在曲线上（除首尾），曲线被控制点"吸引"。

fn build_curve_lut_bspline_clamped(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    if n == 2 { return build_curve_lut_linear(points); }
    if n == 3 { return build_curve_lut_bspline_quadratic(points); }

    // Clamped B-Spline: 在首尾各重复 3 次节点
    // 节点向量: [0,0,0, t1, t2, ..., tn-2, 1,1,1] (n+4 个节点，用于 n 个控制点)
    // 参数化使用弦长参数
    let total_len: f64 = (0..n - 1).map(|i| {
        let dx = pts[i + 1].0 - pts[i].0;
        let dy = pts[i + 1].1 - pts[i].1;
        (dx * dx + dy * dy).sqrt().max(1.0)
    }).sum();

    // 累积弦长参数
    let mut chord = vec![0.0f64; n];
    for i in 1..n {
        let dx = pts[i].0 - pts[i - 1].0;
        let dy = pts[i].1 - pts[i - 1].1;
        chord[i] = chord[i - 1] + (dx * dx + dy * dy).sqrt().max(1.0);
    }
    for i in 0..n { chord[i] /= total_len; }

    // Clamped 节点向量
    let k = 4; // order (degree 3 + 1)
    let num_knots = n + k;
    let mut knots = vec![0.0f64; num_knots];
    // 前 k 个为 0，后 k 个为 1
    for i in 0..k { knots[i] = 0.0; }
    for i in (num_knots - k)..num_knots { knots[i] = 1.0; }
    // 内部节点：使用平均参数法
    for j in 1..=(n - k) {
        let mut sum = 0.0;
        for i in j..(j + k - 1) {
            sum += chord[i];
        }
        knots[j + k - 1] = sum / (k - 1) as f64;
    }

    // De Boor 算法求值
    let sample_bspline = |t: f64| -> (f64, f64) {
        let t = t.clamp(0.0, 1.0 - 1e-10);
        // 找到 t 所在的节点区间
        let mut span = k - 1;
        for i in k..num_knots - k {
            if knots[i] <= t && t < knots[i + 1] { span = i; break; }
        }

        // De Boor 递归
        let mut dx = vec![0.0f64; k];
        let mut dy = vec![0.0f64; k];
        for j in 0..k {
            let idx = (span as isize - (k as isize - 1) + j as isize) as usize;
            let idx = idx.min(n - 1);
            dx[j] = pts[idx].0;
            dy[j] = pts[idx].1;
        }

        for r in 1..k {
            for j in (r..k).rev() {
                let left = span + j - (k - 1);
                let right = span + j - r + 1;
                if right >= num_knots || left >= num_knots { continue; }
                let denom = knots[right] - knots[left];
                let alpha = if denom.abs() < 1e-10 { 0.5 } else { (t - knots[left]) / denom };
                dx[j] = (1.0 - alpha) * dx[j - 1] + alpha * dx[j];
                dy[j] = (1.0 - alpha) * dy[j - 1] + alpha * dy[j];
            }
        }

        (dx[k - 1], dy[k - 1])
    };

    // 采样足够多的点，然后映射到 LUT
    let num_samples = 1024;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let t = i as f64 / (num_samples - 1) as f64;
        samples.push(sample_bspline(t));
    }

    // 从采样点构建 LUT（x → y 映射）
    let mut lut = [0u8; 256];
    for i in 0..256 {
        let x = i as f64;
        // 找到最近的采样点
        let mut best_y = if x <= samples[0].0 { samples[0].1 }
            else if x >= samples[num_samples - 1].0 { samples[num_samples - 1].1 }
            else {
                // 二分查找
                let mut lo = 0;
                let mut hi = num_samples - 1;
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
        lut[i] = best_y.round().clamp(0.0, 255.0) as u8;
    }
    lut
}

// ─── Uniform Cubic B-Spline ─────────────────────────────────────────────────
// 均匀三次 B 样条，控制点不在曲线上（全部），使用均匀节点。

fn build_curve_lut_bspline_uniform(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    if n == 2 { return build_curve_lut_linear(points); }
    if n == 3 {
        // 对 3 个点退化为 quadratic
        return build_curve_lut_bspline_quadratic(points);
    }

    // 均匀三次 B 样条基函数（局部参数 t ∈ [0,1]）
    // N0(t) = (1 - t)^3 / 6
    // N1(t) = (3t^3 - 6t^2 + 4) / 6
    // N2(t) = (-3t^3 + 3t^2 + 3t + 1) / 6
    // N3(t) = t^3 / 6
    let basis = |t: f64| -> [f64; 4] {
        let t2 = t * t;
        let t3 = t2 * t;
        [
            (1.0 - t).powi(3) / 6.0,
            (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0,
            (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0,
            t3 / 6.0,
        ]
    };

    let num_seg = n - 3; // 有效段数
    let num_samples = 1024;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(num_samples);

    for s in 0..num_samples {
        let u = s as f64 / (num_samples - 1) as f64 * num_seg as f64;
        let seg = (u.floor() as usize).min(num_seg - 1);
        let t = u - seg as f64;
        let b = basis(t);
        let x = b[0] * pts[seg].0 + b[1] * pts[seg + 1].0 + b[2] * pts[seg + 2].0 + b[3] * pts[seg + 3].0;
        let y = b[0] * pts[seg].1 + b[1] * pts[seg + 1].1 + b[2] * pts[seg + 2].1 + b[3] * pts[seg + 3].1;
        samples.push((x, y));
    }

    samples_to_lut(&samples)
}

// ─── Quadratic B-Spline ─────────────────────────────────────────────────────

fn build_curve_lut_bspline_quadratic(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    if n == 2 { return build_curve_lut_linear(points); }

    // 均匀二次 B 样条基函数
    // N0(t) = (1-t)^2 / 2
    // N1(t) = (-2t^2 + 2t + 1) / 2
    // N2(t) = t^2 / 2
    let basis = |t: f64| -> [f64; 3] {
        let t2 = t * t;
        [
            (1.0 - t).powi(2) / 2.0,
            (-2.0 * t2 + 2.0 * t + 1.0) / 2.0,
            t2 / 2.0,
        ]
    };

    let num_seg = n - 2;
    let num_samples = 1024;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(num_samples);

    for s in 0..num_samples {
        let u = s as f64 / (num_samples - 1) as f64 * num_seg as f64;
        let seg = (u.floor() as usize).min(num_seg - 1);
        let t = u - seg as f64;
        let b = basis(t);
        let x = b[0] * pts[seg].0 + b[1] * pts[seg + 1].0 + b[2] * pts[seg + 2].0;
        let y = b[0] * pts[seg].1 + b[1] * pts[seg + 1].1 + b[2] * pts[seg + 2].1;
        samples.push((x, y));
    }

    samples_to_lut(&samples)
}

// ─── Catmull-Rom ─────────────────────────────────────────────────────────────

fn build_curve_lut_catmull_rom(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let pts = prepare_points(points);
    let n = pts.len();
    if n < 2 { return identity_lut(); }
    if n == 2 { return build_curve_lut_linear(points); }

    let mut m = vec![0.0f64; n];
    for k in 0..n {
        if k == 0 {
            m[k] = (pts[1].1 - pts[0].1) / (pts[1].0 - pts[0].0).max(1e-10);
        } else if k == n - 1 {
            m[k] = (pts[n - 1].1 - pts[n - 2].1) / (pts[n - 1].0 - pts[n - 2].0).max(1e-10);
        } else {
            let dx = pts[k + 1].0 - pts[k - 1].0;
            m[k] = if dx.abs() < 1e-10 { 0.0 } else { (pts[k + 1].1 - pts[k - 1].1) / dx };
        }
    }
    fill_lut_hermite(&pts, &m)
}

/// 用 Hermite 基函数对区间 [lo, hi] 插值。
fn hermite_interp(x: f64, pts: &[(f64, f64)], m: &[f64], lo: usize, hi: usize) -> f64 {
    let h = pts[hi].0 - pts[lo].0;
    if h.abs() < 1e-10 { return pts[lo].1; }
    let t = (x - pts[lo].0) / h;
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * pts[lo].1 + h10 * h * m[lo] + h01 * pts[hi].1 + h11 * h * m[hi]
}

/// 为所有 256 个值生成 LUT，给定点集和每点切线。
fn fill_lut_hermite(pts: &[(f64, f64)], m: &[f64]) -> [u8; 256] {
    let mut lut = [0u8; 256];
    let n = pts.len();
    for i in 0..256 {
        let x = i as f64;
        if x <= pts[0].0 {
            lut[i] = pts[0].1.clamp(0.0, 255.0) as u8;
        } else if x >= pts[n - 1].0 {
            lut[i] = pts[n - 1].1.clamp(0.0, 255.0) as u8;
        } else {
            let (lo, hi) = find_segment(x, pts);
            let y = hermite_interp(x, pts, m, lo, hi);
            lut[i] = y.round().clamp(0.0, 255.0) as u8;
        }
    }
    lut
}

// ─── Linear ──────────────────────────────────────────────────────────────────

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

// ─── 工具函数 ────────────────────────────────────────────────────────────────

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

// ─── Monotone Hermite (Fritsch-Carlson) ──────────────────────────────────────

/// 用单调三次 Hermite 插值从控制点构建 256 级查找表。
///
/// 控制点格式：(x, y, flag)，x/y 均为 0-255 范围。
/// 使用 Fritsch-Carlson 方法保证单调性，避免过冲。
pub fn build_curve_lut(points: &[(i64, i64, i64)]) -> [u8; 256] {
    let mut lut = [0u8; 256];
    if points.len() < 2 {
        for i in 0..256 { lut[i] = i as u8; }
        return lut;
    }

    let mut pts: Vec<(f64, f64)> = points.iter()
        .map(|&(x, y, _)| (x as f64, y as f64))
        .collect();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 0.5);

    let n = pts.len();
    if n < 2 {
        for i in 0..256 { lut[i] = i as u8; }
        return lut;
    }

    let mut delta = vec![0.0f64; n - 1];
    for k in 0..n - 1 {
        let dx = pts[k + 1].0 - pts[k].0;
        if dx.abs() < 1e-10 { delta[k] = 0.0; }
        else { delta[k] = (pts[k + 1].1 - pts[k].1) / dx; }
    }

    // Fritsch-Carlson 单调三次切线
    let mut m = vec![0.0f64; n];
    m[0] = delta[0];
    m[n - 1] = delta[n - 2];
    for k in 1..n - 1 {
        if delta[k - 1] * delta[k] <= 0.0 {
            m[k] = 0.0;
        } else {
            m[k] = (delta[k - 1] + delta[k]) / 2.0;
        }
    }
    for k in 0..n - 1 {
        if delta[k].abs() < 1e-10 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
        } else {
            let alpha = m[k] / delta[k];
            let beta = m[k + 1] / delta[k];
            let s2 = alpha * alpha + beta * beta;
            if s2 > 9.0 {
                let tau = 3.0 / s2.sqrt();
                m[k] = tau * alpha * delta[k];
                m[k + 1] = tau * beta * delta[k];
            }
        }
    }

    for i in 0..256 {
        let x = i as f64;
        if x <= pts[0].0 {
            lut[i] = pts[0].1.clamp(0.0, 255.0) as u8;
            continue;
        }
        if x >= pts[n - 1].0 {
            lut[i] = pts[n - 1].1.clamp(0.0, 255.0) as u8;
            continue;
        }

        let mut lo = 0;
        let mut hi = n - 1;
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if pts[mid].0 <= x { lo = mid; } else { hi = mid; }
        }

        let h = pts[hi].0 - pts[lo].0;
        if h.abs() < 1e-10 {
            lut[i] = pts[lo].1.clamp(0.0, 255.0) as u8;
            continue;
        }

        let t = (x - pts[lo].0) / h;
        let t2 = t * t;
        let t3 = t2 * t;

        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;

        let y = h00 * pts[lo].1 + h10 * h * m[lo] + h01 * pts[hi].1 + h11 * h * m[hi];
        lut[i] = y.round().clamp(0.0, 255.0) as u8;
    }

    lut
}

/// 将渐变曲线应用到 RGB 图像上（自动处理 8-bit 和 16-bit）。
///
/// 渐变曲线通道顺序：[RGB主通道, R, G, B, C(青), M(品红), Y(黄)]
/// 应用顺序：先逐通道 R/G/B → CMY（反转通道）→ 主通道 RGB
pub fn apply_gradation_curves(img: &image::DynamicImage, gradations: &[Vec<(i64, i64, i64)>], method: CurveMethod) -> image::DynamicImage {
    if gradations.len() < 7 { return img.clone(); }

    let is_identity = |pts: &[(i64, i64, i64)]| -> bool {
        if pts.len() != 2 { return false; }
        pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
    };
    if gradations.iter().all(|ch| is_identity(ch)) { return img.clone(); }

    let lut_rgb = build_curve_lut_with_method(&gradations[0], method);
    let lut_r   = build_curve_lut_with_method(&gradations[1], method);
    let lut_g   = build_curve_lut_with_method(&gradations[2], method);
    let lut_b   = build_curve_lut_with_method(&gradations[3], method);
    let lut_c   = build_curve_lut_with_method(&gradations[4], method);
    let lut_m   = build_curve_lut_with_method(&gradations[5], method);
    let lut_y   = build_curve_lut_with_method(&gradations[6], method);

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
        let result = apply_gradation_curves(&img, &identity_grads, CurveMethod::Linear);
        let out = result.to_rgb8();
        assert_eq!(out.as_raw(), &pixels);
    }
}
