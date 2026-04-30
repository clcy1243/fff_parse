//! USM (Unsharp Masking) 锐化。
//!
//! FlexColor 的 USM 参数语义（来自 FlexColor 手册 + 反推）：
//! - `amount`: 0-500，锐化强度百分比。100 = 标准，250 = 强
//! - `radius`: 1-20 像素，高斯模糊半径。σ ≈ radius/2
//! - `dark_limit`: 0-255，暗部低于此阈值的像素降低锐化强度（防止暗部噪声放大）
//! - `noise_limit`: 0-255，高通幅度低于此值视为噪声，不锐化
//! - `col_factor`: [R, G, B] 百分比，per-channel 锐化系数。默认 [100,100,100]
//!
//! 算法：
//!   high_pass = original − gaussian_blur(original, radius)
//!   amp       = (amount/100) × (col_factor[ch]/100)
//!   dark_gain = smoothstep 门控 @ dark_limit
//!   noise_gain = 1 if |high_pass| > noise_limit else 0  （简化：soft threshold）
//!   output    = original + amp × dark_gain × noise_gain × high_pass

use super::adjust::ManualAdjust;

/// 对 16-bit RGB 图像应用 USM 锐化。是 identity 则直接返回 clone。
///
/// 标定自 FlexColor (2026-04-18)：
///   - σ = radius / 20（`radius=10` → σ=0.5；可用 `FFF_USM_SIGMA` 覆盖）
///   - gain = amount / 67（`amount=250` → k≈3.73；可用 `FFF_USM_GAIN_DIVISOR` 覆盖）
///   - 基于 BT.601 luma 通道，对所有 R/G/B 加同一 delta
///
/// 在 rgb_standard / rgb_saturated / cmyk_standard 上 R² ≈ 0.92 拟合。
/// 负片和 dark 预设的误差来源不同，USM 贡献小。
pub fn apply_usm(img: &image::DynamicImage, adj: &ManualAdjust) -> image::DynamicImage {
    if !adj.apply_usm || adj.usm_amount == 0 || adj.usm_radius < 1 {
        return img.clone();
    }
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            image::DynamicImage::ImageRgb16(apply_usm_16(rgb16, adj))
        }
        _ => img.clone(),
    }
}

fn apply_usm_16(
    rgb16: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    adj: &ManualAdjust,
) -> image::ImageBuffer<image::Rgb<u16>, Vec<u16>> {
    let (w, h) = (rgb16.width() as usize, rgb16.height() as usize);
    let src = rgb16.as_raw();

    let radius = adj.usm_radius.max(1) as usize;
    // FlexColor radius 与 σ 的映射待定；标定实验显示 σ ≈ radius/20 拟合最佳
    // 可用 FFF_USM_SIGMA 环境变量覆盖
    let sigma: f32 = std::env::var("FFF_USM_SIGMA").ok().and_then(|s| s.parse().ok())
        .unwrap_or(radius as f32 / 20.0);
    let kernel = gaussian_kernel(sigma.max(0.1));

    // 1. 计算每像素 luminance Y = 0.299R + 0.587G + 0.114B (BT.601)
    //    FlexColor 的 USM 基于亮度通道而非 per-channel（证据：参考图的 USM 偏移在 R/G/B 上完全相同）
    let mut y_plane = vec![0f32; w * h];
    for i in 0..w * h {
        let r = src[i * 3] as f32;
        let g = src[i * 3 + 1] as f32;
        let b = src[i * 3 + 2] as f32;
        y_plane[i] = (0.299 * r + 0.587 * g + 0.114 * b) / 65535.0;
    }

    // 2. 对 luma 做高斯模糊
    let mut tmp = vec![0f32; w * h];
    let mut y_blur = vec![0f32; w * h];
    convolve_h(&y_plane, &mut tmp, w, h, &kernel);
    convolve_v(&tmp, &mut y_blur, w, h, &kernel);

    // USM 参数
    // FlexColor amount 语义：divisor=50 使 slider 值直接对应 gain (amount=250 → k=5)
    // 在色卡 c_pos_baseline / contrast / brightness 实测 MAE 最低（106 → 98/88，
    // 跨过 PASS 阈值 100）。旧 divisor=67 来自 rgb_standard/cmyk_standard 拟合，
    // 但色卡边缘/硬边场景下 50 更贴近 FlexColor。
    // 用 FFF_USM_GAIN_DIVISOR 环境变量可覆盖。
    let divisor: f32 = std::env::var("FFF_USM_GAIN_DIVISOR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50.0);
    let amount = adj.usm_amount as f32 / divisor;
    // FFF_USM_COMPRESSOR: 设为 "1" 启用实验性压缩器 gain = a / (1 + b*|y_high|)
    //   a, b 由 FFF_USM_A / FFF_USM_B 控制（默认 1.75 / 3.63, 来自两个样本像素拟合）
    let use_compressor = std::env::var("FFF_USM_COMPRESSOR").as_deref() == Ok("1");
    let comp_a: f32 = std::env::var("FFF_USM_A").ok().and_then(|s| s.parse().ok()).unwrap_or(1.75);
    let comp_b: f32 = std::env::var("FFF_USM_B").ok().and_then(|s| s.parse().ok()).unwrap_or(3.63);
    let dark_limit = adj.usm_dark_limit as f32 / 255.0;
    let noise_limit = adj.usm_noise_limit as f32 / 255.0;
    let col_factors = {
        let cf = &adj.usm_col_factor;
        let safe = |i: usize| cf.get(i).copied().unwrap_or(100) as f32 / 100.0;
        [safe(0), safe(1), safe(2)]
    };

    // Debug trace: 可通过环境变量 FFF_USM_TRACE="x,y" 打印单像素 y_orig / y_blur / y_high
    let debug_xy: Option<(usize, usize)> = std::env::var("FFF_USM_TRACE").ok().and_then(|s| {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 2 { return None; }
        Some((parts[0].parse().ok()?, parts[1].parse().ok()?))
    });
    if let Some((dx, dy)) = debug_xy {
        if dx < w && dy < h {
            let idx = dy * w + dx;
            let y_o = y_plane[idx];
            let y_b = y_blur[idx];
            let y_h = y_o - y_b;
            eprintln!("USM trace ({},{}): y_orig={:.5} y_blur={:.5} y_high={:.5} amount={} radius={}",
                dx, dy, y_o, y_b, y_h, adj.usm_amount, adj.usm_radius);
            eprintln!("  normalized delta = amount/100 * y_high = {:.5}", amount * y_h);
            eprintln!("  u16 delta        = {:.1}", amount * y_h * 65535.0);
        }
    }

    let mut out = vec![0u16; w * h * 3];
    use rayon::prelude::*;
    out.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let idx = y * w + x;

            // 亮度 high-pass 分量（同一像素三通道共用）
            let y_orig = y_plane[idx];
            let mut y_high = y_orig - y_blur[idx];

            // noise_limit 软门控
            if noise_limit > 0.0 {
                let t = (y_high.abs() - noise_limit).max(0.0) / noise_limit.max(1.0e-6);
                y_high *= t.clamp(0.0, 1.0);
            }

            // dark_limit smoothstep：暗部降低增益
            let dark_gain = if dark_limit <= 0.0 {
                1.0
            } else {
                let t = (y_orig / dark_limit).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };

            let gain_multiplier = if use_compressor {
                comp_a / (1.0 + comp_b * y_high.abs())
            } else {
                amount
            };
            let delta = gain_multiplier * dark_gain * y_high; // normalized [0,1] 单位的增量

            for ch in 0..3 {
                let orig = src[idx * 3 + ch] as f32 / 65535.0;
                let sharp = (orig + delta * col_factors[ch]).clamp(0.0, 1.0);
                row[x * 3 + ch] = (sharp * 65535.0 + 0.5) as u16;
            }
        }
    });

    image::ImageBuffer::from_raw(w as u32, h as u32, out)
        .expect("USM 16-bit buffer 尺寸不匹配")
}

/// 生成一维高斯核。kernel 长度为 2*⌈3σ⌉+1。
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let half = (3.0 * sigma).ceil().max(1.0) as i32;
    let len = (2 * half + 1) as usize;
    let two_sigma_sq = 2.0 * sigma * sigma;
    let mut k = Vec::with_capacity(len);
    let mut sum = 0.0;
    for i in -half..=half {
        let v = (-((i * i) as f32) / two_sigma_sq).exp();
        k.push(v);
        sum += v;
    }
    for v in &mut k {
        *v /= sum;
    }
    k
}

fn convolve_h(src: &[f32], dst: &mut [f32], w: usize, _h: usize, kernel: &[f32]) {
    use rayon::prelude::*;
    let half = (kernel.len() / 2) as isize;
    dst.par_chunks_mut(w).enumerate().for_each(|(y, row_dst)| {
        let row_start = y * w;
        for x in 0..w {
            let mut sum = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let xi = x as isize + (ki as isize - half);
                let xi = xi.clamp(0, w as isize - 1) as usize;
                sum += src[row_start + xi] * kv;
            }
            row_dst[x] = sum;
        }
    });
}

fn convolve_v(src: &[f32], dst: &mut [f32], w: usize, h: usize, kernel: &[f32]) {
    use rayon::prelude::*;
    let half = (kernel.len() / 2) as isize;
    dst.par_chunks_mut(w).enumerate().for_each(|(y, row_dst)| {
        for x in 0..w {
            let mut sum = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let yi = y as isize + (ki as isize - half);
                let yi = yi.clamp(0, h as isize - 1) as usize;
                sum += src[yi * w + x] * kv;
            }
            row_dst[x] = sum;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_kernel_sums_to_one() {
        let k = gaussian_kernel(2.0);
        let s: f32 = k.iter().sum();
        assert!((s - 1.0).abs() < 1e-5, "gaussian kernel sum = {}", s);
    }

    #[test]
    fn usm_identity_when_amount_zero() {
        let img = image::DynamicImage::ImageRgb16(
            image::ImageBuffer::from_pixel(10, 10, image::Rgb([30000u16, 20000, 10000]))
        );
        let mut adj = ManualAdjust::default();
        adj.apply_usm = true;
        adj.usm_amount = 0;
        let out = apply_usm(&img, &adj);
        let out_rgb = out.to_rgb16();
        let in_rgb = img.to_rgb16();
        assert_eq!(out_rgb.as_raw(), in_rgb.as_raw());
    }

    #[test]
    fn usm_preserves_uniform_region() {
        // 纯色区域 high-pass = 0 → 输出不变
        let img = image::DynamicImage::ImageRgb16(
            image::ImageBuffer::from_pixel(40, 40, image::Rgb([40000u16, 20000, 10000]))
        );
        let mut adj = ManualAdjust::default();
        adj.apply_usm = true;
        adj.usm_amount = 250;
        adj.usm_radius = 5;
        adj.usm_col_factor = [100, 100, 100];
        let out = apply_usm(&img, &adj);
        // 内部像素应该不变；边缘由于镜像边界可能略有差异，检查中心
        let out_rgb = out.to_rgb16();
        let in_rgb = img.to_rgb16();
        for y in 20..25 {
            for x in 20..25 {
                let o = out_rgb.get_pixel(x, y);
                let i = in_rgb.get_pixel(x, y);
                assert!((o[0] as i32 - i[0] as i32).abs() <= 1,
                    "pixel ({},{}) changed: {:?} vs {:?}", x, y, o, i);
            }
        }
    }
}
