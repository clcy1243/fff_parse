//! 手动图像调整：曝光、对比度、高光/阴影、饱和度、色彩平衡、色阶及锐化/降噪/镜头校正。

/// 手动图像调整参数，在色彩管道之后应用。
#[derive(Debug, Clone, PartialEq)]
pub struct ManualAdjust {
    /// 胶片类型：0=正片, 1=彩色负片, 2=黑白负片
    pub film_type: i64,
    /// 胶片曲线类型（来自 correction.film_curve）
    pub film_curve: i64,
    /// 胶片 Gamma 值（来自 correction.gamma）
    pub film_gamma: f64,

    // ── 各调整项独立启用开关（用于调试/排查渲染问题） ──
    pub apply_levels: bool,
    pub apply_film_curve: bool,
    pub apply_curves: bool,
    pub apply_exposure: bool,
    pub apply_brightness: bool,
    pub apply_shadow_depth: bool,
    pub apply_midtone: bool,
    pub apply_contrast: bool,
    pub apply_highlights: bool,
    pub apply_shadows: bool,
    pub apply_saturation: bool,
    pub apply_color_balance: bool,
    pub apply_color_temp: bool,
    pub apply_color_corr: bool,

    // ── 调整值 ──
    /// 曝光补偿（档位）：-3.0 ~ 3.0
    pub exposure: f32,
    /// 亮度：-100 ~ 100
    pub brightness: f32,
    /// 阴影深度：-100 ~ 100（对应 FlexColor 的 Lightness/阴影深度）
    pub lightness: f32,
    /// 中间调：对应 FlexColor 的 Gamma，默认 1.0 表示 Gamma=2.0
    pub midtone: f32,
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
    /// 色温：-100 ~ 100
    pub color_temperature: f32,
    /// 色调偏移：-100 ~ 100
    pub tint: f32,

    /// 色彩校正矩阵 6×6 (RGBCMY)，整数值，默认全零(无校正)
    pub color_corr: [i64; 36],

    // 色阶（输入范围）：索引 0=总通道, 1=R, 2=G, 3=B
    /// 输入黑点：0-255
    pub levels_black: [f32; 4],
    /// 中间调 Gamma：0.10-9.99（1.0 为中性）
    pub levels_gamma: [f32; 4],
    /// 输入白点：0-255
    pub levels_white: [f32; 4],

    // ── USM 锐化参数（尚未实现处理，仅保存/加载） ──
    /// 是否应用 USM 锐化
    pub apply_usm: bool,
    /// USM 锐化强度：0-500
    pub usm_amount: i64,
    /// USM 锐化半径：1-20
    pub usm_radius: i64,
    /// USM 暗部限制：0-255
    pub usm_dark_limit: i64,
    /// USM 噪声限制：0-255
    pub usm_noise_limit: i64,
    /// USM 色彩因子 [R, G, B]
    pub usm_col_factor: [i64; 3],

    // ── 除尘参数（尚未实现处理，仅保存/加载） ──
    /// 是否应用除尘
    pub apply_dust: bool,
    /// 除尘级别：0-100
    pub dust_level: i64,

    // ── 色彩噪声滤镜参数（尚未实现处理，仅保存/加载） ──
    /// 是否应用色彩噪声滤镜
    pub apply_cn_filter: bool,
    /// 色彩噪声半径
    pub color_noise_radius: i64,
    /// 噪声滤镜偏移
    pub noise_filter_bias: i64,

    // ── 镜头/暗角校正参数（尚未实现处理，仅保存/加载） ──
    /// 镜头校正
    pub lens_correction: i64,
    /// 暗角校正量
    pub vignette_amount: i64,

    // ── 阴影增强与色偏去除（尚未实现处理，仅保存/加载） ──
    /// 是否增强阴影
    pub enhanced_shadow: bool,
    /// 是否去除高光色偏
    pub remove_cast_highlight: bool,
    /// 是否去除阴影色偏
    pub remove_cast_shadow: bool,
}

impl Default for ManualAdjust {
    fn default() -> Self {
        Self {
            film_type: 0,
            film_curve: 0,
            film_gamma: 2.0,
            apply_levels: true, apply_film_curve: true, apply_curves: true, apply_exposure: true,
            apply_brightness: true, apply_shadow_depth: true, apply_midtone: true,
            apply_contrast: true, apply_highlights: true, apply_shadows: true,
            apply_saturation: true, apply_color_balance: true, apply_color_temp: true,
            apply_color_corr: true,
            exposure: 0.0, brightness: 0.0, lightness: 0.0, midtone: 1.0, contrast: 0.0,
            highlights: 0.0, shadows: 0.0,
            saturation: 0.0, r_shift: 0.0, g_shift: 0.0, b_shift: 0.0,
            color_temperature: 0.0, tint: 0.0,
            color_corr: [0i64; 36],
            levels_black: [0.0; 4],
            levels_gamma: [1.0; 4],
            levels_white: [255.0; 4],
            // USM 锐化
            apply_usm: false,
            usm_amount: 0, usm_radius: 1, usm_dark_limit: 0, usm_noise_limit: 0,
            usm_col_factor: [100, 100, 100],
            // 除尘
            apply_dust: false, dust_level: 0,
            // 色彩噪声滤镜
            apply_cn_filter: false, color_noise_radius: 0, noise_filter_bias: 0,
            // 镜头/暗角校正
            lens_correction: 0, vignette_amount: 0,
            // 阴影增强与色偏去除
            enhanced_shadow: false, remove_cast_highlight: false, remove_cast_shadow: false,
        }
    }
}

impl ManualAdjust {
    /// 判断当前调整参数是否为恒等变换（即不产生任何效果）。
    pub fn is_identity(&self) -> bool {
        let levels_id = !self.apply_levels
            || (self.levels_black.iter().all(|&v| v < 0.5)
                && self.levels_gamma.iter().all(|&v| (v - 1.0).abs() < 0.01)
                && self.levels_white.iter().all(|&v| v > 254.5));
        let exposure_id = !self.apply_exposure || self.exposure.abs() < 0.001;
        let brightness_id = !self.apply_brightness || self.brightness.abs() < 0.1;
        let shadow_depth_id = !self.apply_shadow_depth || self.lightness.abs() < 0.1;
        let midtone_id = !self.apply_midtone || (self.midtone - 1.0).abs() < 0.01;
        let contrast_id = !self.apply_contrast || self.contrast.abs() < 0.1;
        let highlights_id = !self.apply_highlights || self.highlights.abs() < 0.1;
        let shadows_id = !self.apply_shadows || self.shadows.abs() < 0.1;
        let saturation_id = !self.apply_saturation || self.saturation.abs() < 0.1;
        let color_balance_id = !self.apply_color_balance
            || (self.r_shift.abs() < 0.1 && self.g_shift.abs() < 0.1 && self.b_shift.abs() < 0.1);
        let color_temp_id = !self.apply_color_temp
            || (self.color_temperature.abs() < 0.1 && self.tint.abs() < 0.1);
        let color_corr_id = !self.apply_color_corr
            || self.color_corr.iter().all(|&v| v == 0);

        // film_curve 不算恒等——只要 apply_film_curve=true 就需要处理
        let film_curve_id = !self.apply_film_curve;

        levels_id && film_curve_id && exposure_id && brightness_id && shadow_depth_id && midtone_id
            && contrast_id && highlights_id && shadows_id && saturation_id
            && color_balance_id && color_temp_id && color_corr_id
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

    let exposure_mult = if adj.apply_exposure { 2.0_f32.powf(adj.exposure) } else { 1.0 };
    let sat = if adj.apply_saturation { adj.saturation / 100.0 } else { 0.0 };

    // 色阶参数从用户空间 (0-255) 映射到归一化 (0-1)
    let (bl_m, wh_m, gamma_m) = if adj.apply_levels {
        (adj.levels_black[0] / 255.0, adj.levels_white[0] / 255.0, adj.levels_gamma[0].clamp(0.01, 99.0))
    } else {
        (0.0, 1.0, 1.0)
    };
    let range_m = (wh_m - bl_m).max(0.001);

    // 色温/色调 per-channel 乘数
    let temp_mults = if adj.apply_color_temp && (adj.color_temperature.abs() > 0.1 || adj.tint.abs() > 0.1) {
        let t = adj.color_temperature / 100.0;
        let tn = adj.tint / 100.0;
        [1.0 + t * 0.15, 1.0 - tn * 0.15, 1.0 - t * 0.15]
    } else {
        [1.0, 1.0, 1.0]
    };

    // 构建 65536 项 per-channel LUT
    let shifts = if adj.apply_color_balance {
        [adj.r_shift / 255.0, adj.g_shift / 255.0, adj.b_shift / 255.0]
    } else {
        [0.0; 3]
    };
    let mut luts: Vec<Vec<u16>> = Vec::with_capacity(3);

    let use_film_lut = adj.apply_film_curve
        && (adj.film_type == 1 || adj.film_type == 2)
        && adj.film_curve == 4
        && (adj.film_gamma - 2.0).abs() < 0.01;

    for ch in 0..3 {
        let (bl_c, wh_c, gamma_c) = if adj.apply_levels {
            (adj.levels_black[ch + 1] / 255.0, adj.levels_white[ch + 1] / 255.0,
             adj.levels_gamma[ch + 1].clamp(0.01, 99.0))
        } else {
            (0.0, 1.0, 1.0)
        };
        let range_c = (wh_c - bl_c).max(0.001);

        let mut lut = vec![0u16; 65536];
        for i in 0..65536u32 {
            let mut v = i as f32 / 65535.0;

            v = ((v - bl_m) / range_m).clamp(0.0, 1.0).powf(1.0 / gamma_m);
            v = ((v - bl_c) / range_c).clamp(0.0, 1.0).powf(1.0 / gamma_c);

            if use_film_lut {
                let lut_table: &[u8; 256] = match ch {
                    0 => &crate::color::FILM_CURVE_LUT_R,
                    1 => &crate::color::FILM_CURVE_LUT_G,
                    _ => &crate::color::FILM_CURVE_LUT_B,
                };
                v = crate::color::lut_interp_16(v, lut_table) / 65535.0;
            }

            v += shifts[ch];
            v *= exposure_mult;
            v *= temp_mults[ch];
            v = v.clamp(0.0, 1.0);

            if adj.apply_shadows && adj.shadows.abs() > 0.1 {
                let s = adj.shadows / 100.0;
                let t = 1.0 - v;
                v = (v + s * t * t * 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_highlights && adj.highlights.abs() > 0.1 {
                let hi = adj.highlights / 100.0;
                let t = v;
                v = (v + hi * t * t * 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_contrast && adj.contrast.abs() > 0.1 {
                let c = adj.contrast / 100.0;
                let scale = if c >= 0.0 { 1.0 + c * 2.0 } else { 1.0 + c };
                v = ((v - 0.5) * scale + 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_brightness && adj.brightness.abs() > 0.1 {
                let b = adj.brightness / 100.0;
                v = (v + b * 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_shadow_depth && adj.lightness.abs() > 0.1 {
                let l = adj.lightness / 100.0;
                let gamma = 1.0 / (1.0 + l).max(0.1);
                v = v.powf(gamma).clamp(0.0, 1.0);
            }
            // 中间调：midtone=1.0 为中性(Gamma=2.0)，>1 提亮中间调，<1 压暗
            if adj.apply_midtone && (adj.midtone - 1.0).abs() > 0.01 {
                let g = adj.midtone.clamp(0.1, 10.0);
                v = v.powf(1.0 / g).clamp(0.0, 1.0);
            }

            lut[i as usize] = (v * 65535.0) as u16;
        }
        luts.push(lut);
    }

    let row_len = w as usize * 3;
    let mut out = vec![0u16; row_len * h as usize];

    // 色彩校正矩阵 (6×6 RGBCMY, 只使用 RGB 部分 3×3)
    // cc[row][col]: row=输出通道(R/G/B), col=输入通道(R/G/B)
    // 对角线 += 100 表示原色保留, 其余值为百分比混入
    let apply_cc = adj.apply_color_corr && adj.color_corr.iter().any(|&v| v != 0);
    let cc: [[f32; 3]; 3] = if apply_cc {
        let m = &adj.color_corr;
        // 矩阵前 3 行 × 前 3 列即 RGB→RGB 部分
        [
            [(100 + m[0]) as f32 / 100.0, m[1] as f32 / 100.0,       m[2] as f32 / 100.0],
            [m[6] as f32 / 100.0,         (100 + m[7]) as f32 / 100.0, m[8] as f32 / 100.0],
            [m[12] as f32 / 100.0,        m[13] as f32 / 100.0,      (100 + m[14]) as f32 / 100.0],
        ]
    } else {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    };

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

                if apply_cc {
                    let r0 = rf; let g0 = gf; let b0 = bf;
                    rf = (cc[0][0] * r0 + cc[0][1] * g0 + cc[0][2] * b0).clamp(0.0, 65535.0);
                    gf = (cc[1][0] * r0 + cc[1][1] * g0 + cc[1][2] * b0).clamp(0.0, 65535.0);
                    bf = (cc[2][0] * r0 + cc[2][1] * g0 + cc[2][2] * b0).clamp(0.0, 65535.0);
                }

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

    let exposure_mult = if adj.apply_exposure { 2.0_f32.powf(adj.exposure) } else { 1.0 };
    let sat = if adj.apply_saturation { adj.saturation / 100.0 } else { 0.0 };

    let (bl_m, wh_m, gamma_m) = if adj.apply_levels {
        (adj.levels_black[0] / 255.0, adj.levels_white[0] / 255.0, adj.levels_gamma[0].clamp(0.01, 99.0))
    } else {
        (0.0, 1.0, 1.0)
    };
    let range_m = (wh_m - bl_m).max(0.001);

    let temp_mults = if adj.apply_color_temp && (adj.color_temperature.abs() > 0.1 || adj.tint.abs() > 0.1) {
        let t = adj.color_temperature / 100.0;
        let tn = adj.tint / 100.0;
        [1.0 + t * 0.15, 1.0 - tn * 0.15, 1.0 - t * 0.15]
    } else {
        [1.0, 1.0, 1.0]
    };

    let mut luts = [[0u8; 256]; 3];
    let shifts = if adj.apply_color_balance {
        [adj.r_shift / 255.0, adj.g_shift / 255.0, adj.b_shift / 255.0]
    } else {
        [0.0; 3]
    };

    let use_film_lut = adj.apply_film_curve
        && (adj.film_type == 1 || adj.film_type == 2)
        && adj.film_curve == 4
        && (adj.film_gamma - 2.0).abs() < 0.01;

    for ch in 0..3 {
        let (bl_c, wh_c, gamma_c) = if adj.apply_levels {
            (adj.levels_black[ch + 1] / 255.0, adj.levels_white[ch + 1] / 255.0,
             adj.levels_gamma[ch + 1].clamp(0.01, 99.0))
        } else {
            (0.0, 1.0, 1.0)
        };
        let range_c = (wh_c - bl_c).max(0.001);

        for i in 0..=255u32 {
            let mut v = i as f32 / 255.0;
            v = ((v - bl_m) / range_m).clamp(0.0, 1.0).powf(1.0 / gamma_m);
            v = ((v - bl_c) / range_c).clamp(0.0, 1.0).powf(1.0 / gamma_c);

            if use_film_lut {
                let lut_table: &[u8; 256] = match ch {
                    0 => &crate::color::FILM_CURVE_LUT_R,
                    1 => &crate::color::FILM_CURVE_LUT_G,
                    _ => &crate::color::FILM_CURVE_LUT_B,
                };
                let x = v * 255.0;
                let lo = (x as usize).min(254);
                let hi = lo + 1;
                let frac = x - lo as f32;
                v = (lut_table[lo] as f32 * (1.0 - frac) + lut_table[hi] as f32 * frac) / 255.0;
            }

            v += shifts[ch];
            v *= exposure_mult;
            v *= temp_mults[ch];
            v = v.clamp(0.0, 1.0);
            if adj.apply_shadows && adj.shadows.abs() > 0.1 {
                let s = adj.shadows / 100.0;
                let t = 1.0 - v;
                v = (v + s * t * t * 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_highlights && adj.highlights.abs() > 0.1 {
                let hi = adj.highlights / 100.0;
                let t = v;
                v = (v + hi * t * t * 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_contrast && adj.contrast.abs() > 0.1 {
                let c = adj.contrast / 100.0;
                let scale = if c >= 0.0 { 1.0 + c * 2.0 } else { 1.0 + c };
                v = ((v - 0.5) * scale + 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_brightness && adj.brightness.abs() > 0.1 {
                let b = adj.brightness / 100.0;
                v = (v + b * 0.5).clamp(0.0, 1.0);
            }
            if adj.apply_shadow_depth && adj.lightness.abs() > 0.1 {
                let l = adj.lightness / 100.0;
                let gamma = 1.0 / (1.0 + l).max(0.1);
                v = v.powf(gamma).clamp(0.0, 1.0);
            }
            if adj.apply_midtone && (adj.midtone - 1.0).abs() > 0.01 {
                let g = adj.midtone.clamp(0.1, 10.0);
                v = v.powf(1.0 / g).clamp(0.0, 1.0);
            }
            luts[ch][i as usize] = (v * 255.0) as u8;
        }
    }

    let row_len = w as usize * 3;
    let mut out = vec![0u8; row_len * h as usize];

    let apply_cc = adj.apply_color_corr && adj.color_corr.iter().any(|&v| v != 0);
    let cc: [[f32; 3]; 3] = if apply_cc {
        let m = &adj.color_corr;
        [
            [(100 + m[0]) as f32 / 100.0, m[1] as f32 / 100.0,       m[2] as f32 / 100.0],
            [m[6] as f32 / 100.0,         (100 + m[7]) as f32 / 100.0, m[8] as f32 / 100.0],
            [m[12] as f32 / 100.0,        m[13] as f32 / 100.0,      (100 + m[14]) as f32 / 100.0],
        ]
    } else {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    };

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

                if apply_cc {
                    let r0 = rf; let g0 = gf; let b0 = bf;
                    rf = (cc[0][0] * r0 + cc[0][1] * g0 + cc[0][2] * b0).clamp(0.0, 255.0);
                    gf = (cc[1][0] * r0 + cc[1][1] * g0 + cc[1][2] * b0).clamp(0.0, 255.0);
                    bf = (cc[2][0] * r0 + cc[2][1] * g0 + cc[2][2] * b0).clamp(0.0, 255.0);
                }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_adjust_is_not_identity_with_film_curve() {
        // 默认 ManualAdjust 因 apply_film_curve=true 不是恒等
        let adj = ManualAdjust::default();
        assert!(!adj.is_identity(), "default with film_curve enabled is not identity");
    }

    #[test]
    fn adjust_without_film_curve_is_identity() {
        let mut adj = ManualAdjust::default();
        adj.apply_film_curve = false;
        assert!(adj.is_identity(), "default with film_curve disabled should be identity");
    }

    #[test]
    fn identity_adjust_preserves_image() {
        // 恒等变换不应修改图像
        let mut adj = ManualAdjust::default();
        // 关闭 film_curve 使其为恒等
        adj.apply_film_curve = false;
        let w = 4u32;
        let h = 2u32;
        let pixels: Vec<u8> = (0..w * h * 3).map(|i| (i % 256) as u8).collect();
        let img = image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(w, h, pixels.clone()).unwrap(),
        );
        let result = apply_manual_adjust(&img, &adj);
        let out = result.to_rgb8();
        assert_eq!(out.as_raw(), &pixels, "identity adjust should not change pixels");
    }

    #[test]
    fn levels_black_clips_dark() {
        // 设置黑点=128 应将低于 128 的值映射到 0
        let mut adj = ManualAdjust::default();
        adj.apply_film_curve = false;
        adj.levels_black = [128.0; 4];
        let w = 4u32;
        let h = 1u32;
        let pixels = vec![0u8, 0, 0, 64, 64, 64, 128, 128, 128, 255, 255, 255];
        let img = image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(w, h, pixels).unwrap(),
        );
        let result = apply_manual_adjust(&img, &adj);
        let out = result.to_rgb8();
        let raw = out.as_raw();
        // 值 0 和 64 应映射到 0（低于黑点）
        assert_eq!(raw[0], 0, "value 0 with black=128 should be 0");
        assert_eq!(raw[3], 0, "value 64 with black=128 should be 0");
        // 值 128 应映射到 0（刚好在黑点）
        assert_eq!(raw[6], 0, "value 128 with black=128 should be 0");
        // 值 255 应映射到 255（白点保持）
        assert_eq!(raw[9], 255, "value 255 should stay 255");
    }

    #[test]
    fn levels_white_clips_bright() {
        // 设置白点=128 应将高于 128 的值映射到 255
        let mut adj = ManualAdjust::default();
        adj.apply_film_curve = false;
        adj.levels_white = [128.0; 4];
        let w = 4u32;
        let h = 1u32;
        let pixels = vec![0u8, 0, 0, 64, 64, 64, 128, 128, 128, 255, 255, 255];
        let img = image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(w, h, pixels).unwrap(),
        );
        let result = apply_manual_adjust(&img, &adj);
        let out = result.to_rgb8();
        let raw = out.as_raw();
        // 值 0 应映射到 0
        assert_eq!(raw[0], 0, "value 0 should stay 0");
        // 值 128 应映射到 255（刚好在白点）
        assert_eq!(raw[6], 255, "value 128 with white=128 should be 255");
        // 值 255 应映射到 255（超过白点）
        assert_eq!(raw[9], 255, "value 255 should be 255");
    }

    #[test]
    fn exposure_positive_brightens() {
        let mut adj = ManualAdjust::default();
        adj.apply_film_curve = false;
        adj.exposure = 1.0; // +1 stop → 2x
        let w = 1u32;
        let h = 1u32;
        let pixels = vec![64u8, 64, 64];
        let img = image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(w, h, pixels).unwrap(),
        );
        let result = apply_manual_adjust(&img, &adj);
        let out = result.to_rgb8();
        let raw = out.as_raw();
        // 64 * 2 = 128
        assert_eq!(raw[0], 128, "exposure +1 stop should double value 64 to 128");
    }

    #[test]
    fn manual_adjust_16bit_roundtrip() {
        // 16-bit 恒等变换也应保持不变
        let mut adj = ManualAdjust::default();
        adj.apply_film_curve = false;
        let w = 2u32;
        let h = 2u32;
        let pixels: Vec<u16> = vec![0, 0, 0, 32768, 32768, 32768, 65535, 65535, 65535, 16384, 32768, 49152];
        let img = image::DynamicImage::ImageRgb16(
            image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(w, h, pixels.clone()).unwrap(),
        );
        let result = apply_manual_adjust(&img, &adj);
        match result {
            image::DynamicImage::ImageRgb16(buf) => {
                assert_eq!(buf.as_raw(), &pixels, "16-bit identity should preserve pixels");
            }
            _ => panic!("16-bit input should produce 16-bit output"),
        }
    }

    #[test]
    fn new_fields_have_correct_defaults() {
        let adj = ManualAdjust::default();
        // USM 默认关闭
        assert!(!adj.apply_usm);
        assert_eq!(adj.usm_amount, 0);
        assert_eq!(adj.usm_radius, 1);
        assert_eq!(adj.usm_dark_limit, 0);
        assert_eq!(adj.usm_noise_limit, 0);
        assert_eq!(adj.usm_col_factor, [100, 100, 100]);
        // 除尘默认关闭
        assert!(!adj.apply_dust);
        assert_eq!(adj.dust_level, 0);
        // 降噪默认关闭
        assert!(!adj.apply_cn_filter);
        assert_eq!(adj.color_noise_radius, 0);
        assert_eq!(adj.noise_filter_bias, 0);
        // 镜头/暗角校正默认为0
        assert_eq!(adj.lens_correction, 0);
        assert_eq!(adj.vignette_amount, 0);
        // 阴影增强与色偏去除默认关闭
        assert!(!adj.enhanced_shadow);
        assert!(!adj.remove_cast_highlight);
        assert!(!adj.remove_cast_shadow);
    }

    #[test]
    fn new_fields_do_not_affect_identity() {
        // 新增字段不影响恒等判断（它们不参与图像处理）
        let mut adj = ManualAdjust::default();
        adj.apply_film_curve = false;
        // 设置新增字段的值
        adj.apply_usm = true;
        adj.usm_amount = 100;
        adj.apply_dust = true;
        adj.dust_level = 50;
        adj.apply_cn_filter = true;
        adj.enhanced_shadow = true;
        // 恒等判断不受影响（新字段未参与 is_identity 判断）
        assert!(adj.is_identity(), "new fields should not affect identity check");
    }
}

