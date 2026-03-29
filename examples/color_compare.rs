//! 色彩管线对照工具：逐步对比 16-bit 管线输出与 FlexColor 8-bit 缩略图
//!
//! 用法: cargo run --example color_compare -- <file.fff> [--dump-pixels N]
//!
//! 缩略图 = FlexColor 预渲染结果（IFD SubfileType=1, 8-bit），是 ground truth。
//! 本工具将 16-bit 原始数据经过管线各步骤后降采样到缩略图尺寸，逐步对比。

use std::env;
use std::path::Path;

use fff_viewer::color;
use fff_viewer::flexcolor::{self, EditHistory, ImageCorrection};
use fff_viewer::tiff::TiffFile;

// ─── 差异统计 ───────────────────────────────────────────────────────────────

struct ChannelStats {
    name: &'static str,
    mae: f64,      // Mean Absolute Error
    max_err: u8,   // Max absolute error
    psnr: f64,     // Peak Signal-to-Noise Ratio
    count: usize,  // Pixel count
    err_gt5: usize,  // Pixels with error > 5
    err_gt10: usize, // Pixels with error > 10
}

fn compute_channel_stats(name: &'static str, ours: &[u8], reference: &[u8]) -> ChannelStats {
    assert_eq!(ours.len(), reference.len());
    let count = ours.len();
    let mut sum_abs_err: u64 = 0;
    let mut sum_sq_err: f64 = 0.0;
    let mut max_err: u8 = 0;
    let mut err_gt5 = 0usize;
    let mut err_gt10 = 0usize;

    for i in 0..count {
        let diff = (ours[i] as i16 - reference[i] as i16).unsigned_abs() as u8;
        sum_abs_err += diff as u64;
        sum_sq_err += (diff as f64) * (diff as f64);
        if diff > max_err { max_err = diff; }
        if diff > 5 { err_gt5 += 1; }
        if diff > 10 { err_gt10 += 1; }
    }

    let mae = sum_abs_err as f64 / count as f64;
    let mse = sum_sq_err / count as f64;
    let psnr = if mse > 0.0 { 10.0 * (255.0_f64 * 255.0 / mse).log10() } else { f64::INFINITY };

    ChannelStats { name, mae, max_err, psnr, count, err_gt5, err_gt10 }
}

fn print_stats(label: &str, stats: &[ChannelStats]) {
    println!("\n┌─── {} ───", label);
    println!("│ {:>5} │ {:>8} │ {:>8} │ {:>10} │ {:>10} │ {:>10}",
             "Chan", "MAE", "MaxErr", "PSNR(dB)", "Err>5", "Err>10");
    println!("│{}", "─".repeat(70));
    for s in stats {
        println!("│ {:>5} │ {:>8.3} │ {:>8} │ {:>10.2} │ {:>9} │ {:>9}",
                 s.name, s.mae, s.max_err,
                 if s.psnr.is_infinite() { "∞".to_string() } else { format!("{:.2}", s.psnr) },
                 format!("{} ({:.1}%)", s.err_gt5, s.err_gt5 as f64 / s.count as f64 * 100.0),
                 format!("{} ({:.1}%)", s.err_gt10, s.err_gt10 as f64 / s.count as f64 * 100.0));
    }
    println!("└{}", "─".repeat(72));
}

// ─── 图像工具 ───────────────────────────────────────────────────────────────

/// 将 16-bit DynamicImage 转为 RGB8（>>8 截断，与 texture_from_16bit 一致）
fn to_rgb8_truncate(img: &image::DynamicImage) -> image::RgbImage {
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();
            let mut out = vec![0u8; w as usize * h as usize * 3];
            for (i, &v) in src.iter().enumerate() {
                out[i] = (v >> 8) as u8;
            }
            image::RgbImage::from_raw(w, h, out).unwrap()
        }
        _ => img.to_rgb8(),
    }
}

/// Nearest-neighbor 降采样到目标尺寸
fn downsample_nearest(img: &image::RgbImage, target_w: u32, target_h: u32) -> image::RgbImage {
    let (sw, sh) = (img.width(), img.height());
    let mut out = image::RgbImage::new(target_w, target_h);
    for y in 0..target_h {
        let sy = (y as u64 * sh as u64 / target_h as u64) as u32;
        for x in 0..target_w {
            let sx = (x as u64 * sw as u64 / target_w as u64) as u32;
            out.put_pixel(x, y, *img.get_pixel(sx.min(sw - 1), sy.min(sh - 1)));
        }
    }
    out
}

/// 16-bit DynamicImage 的 nearest-neighbor 降采样
fn downsample_16bit_nearest(img: &image::DynamicImage, target_w: u32, target_h: u32) -> image::DynamicImage {
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (sw, sh) = (rgb16.width(), rgb16.height());
            let mut out = vec![0u16; target_w as usize * target_h as usize * 3];
            for y in 0..target_h {
                let sy = (y as u64 * sh as u64 / target_h as u64) as u32;
                for x in 0..target_w {
                    let sx = (x as u64 * sw as u64 / target_w as u64) as u32;
                    let src_idx = (sy as usize * sw as usize + sx.min(sw - 1) as usize) * 3;
                    let dst_idx = (y as usize * target_w as usize + x as usize) * 3;
                    let src = rgb16.as_raw();
                    out[dst_idx] = src[src_idx];
                    out[dst_idx + 1] = src[src_idx + 1];
                    out[dst_idx + 2] = src[src_idx + 2];
                }
            }
            let buf = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                target_w, target_h, out
            ).unwrap();
            image::DynamicImage::ImageRgb16(buf)
        }
        _ => {
            let rgb8 = img.to_rgb8();
            let ds = downsample_nearest(&rgb8, target_w, target_h);
            image::DynamicImage::ImageRgb8(ds)
        }
    }
}

/// 比较两个 RGB8 图像，返回 per-channel 统计
fn compare_images(label: &str, ours: &image::RgbImage, reference: &image::RgbImage) {
    assert_eq!(ours.dimensions(), reference.dimensions(),
               "size mismatch: ours {:?} vs ref {:?}", ours.dimensions(), reference.dimensions());
    let our_raw = ours.as_raw();
    let ref_raw = reference.as_raw();
    let pixel_count = (ours.width() * ours.height()) as usize;

    // 分离通道
    let (mut our_r, mut our_g, mut our_b) = (vec![0u8; pixel_count], vec![0u8; pixel_count], vec![0u8; pixel_count]);
    let (mut ref_r, mut ref_g, mut ref_b) = (vec![0u8; pixel_count], vec![0u8; pixel_count], vec![0u8; pixel_count]);
    for i in 0..pixel_count {
        our_r[i] = our_raw[i * 3];
        our_g[i] = our_raw[i * 3 + 1];
        our_b[i] = our_raw[i * 3 + 2];
        ref_r[i] = ref_raw[i * 3];
        ref_g[i] = ref_raw[i * 3 + 1];
        ref_b[i] = ref_raw[i * 3 + 2];
    }

    let stats = vec![
        compute_channel_stats("R", &our_r, &ref_r),
        compute_channel_stats("G", &our_g, &ref_g),
        compute_channel_stats("B", &our_b, &ref_b),
        compute_channel_stats("All", our_raw, ref_raw),
    ];
    print_stats(label, &stats);
}

// ─── 管线复现（从 panels.rs 提取的逻辑） ────────────────────────────────────

/// 从 ImageCorrection 构建 ManualAdjust（复现 panels.rs 的加载逻辑）
fn build_manual_adjust(corr: &ImageCorrection) -> color::ManualAdjust {
    let mut adj = color::ManualAdjust::default();

    // 胶片参数
    adj.film_type = corr.film_type;
    adj.film_curve = corr.film_curve;
    adj.film_gamma = corr.gamma;

    // 色阶（来自 load_levels_from_correction）
    if corr.apply_histogram {
        for i in 0..4 {
            adj.levels_black[i] = (corr.shadow[i] as f32 * 4.0 / 65535.0 * 255.0).clamp(0.0, 255.0);
            adj.levels_white[i] = (corr.highlight[i] as f32 * 4.0 / 65535.0 * 255.0).clamp(0.0, 255.0);
        }
        adj.levels_gamma[0] = ((corr.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 {
            adj.levels_gamma[i] = (corr.gray[i] as f32 / 128.0).clamp(0.01, 99.0);
        }
        adj.levels_black[0] = adj.levels_black[1].min(adj.levels_black[2]).min(adj.levels_black[3]);
        adj.levels_white[0] = adj.levels_white[1].max(adj.levels_white[2]).max(adj.levels_white[3]);
        if corr.dot_color.len() >= 14 {
            adj.output_shadow = corr.dot_color[0] as f32;
            adj.output_highlight = corr.dot_color[7] as f32;
        }
    }

    // 滑块参数
    if corr.apply_sliders {
        adj.saturation = corr.saturation as f32;
        if (corr.ev - 1.0).abs() > 0.001 {
            adj.exposure = corr.ev.log2() as f32;
        }
        adj.contrast = corr.contrast as f32;
        adj.brightness = corr.brightness as f32;
        adj.lightness = corr.lightness as f32;
    }

    // 色温/色调
    adj.color_temperature = corr.color_temperature as f32;
    adj.tint = corr.tint as f32;

    // 色彩校正矩阵
    if corr.apply_cc && corr.color_corr.len() == 36 {
        for (i, &v) in corr.color_corr.iter().enumerate() {
            adj.color_corr[i] = v;
        }
        adj.apply_color_corr = true;
    } else {
        adj.apply_color_corr = false;
    }

    // 渐变曲线开关
    adj.apply_curves = corr.apply_curves && !corr.gradations.is_empty();

    adj
}

/// 打印 ManualAdjust 状态概要
fn print_adjust_summary(adj: &color::ManualAdjust) {
    println!("\n=== ManualAdjust 参数 ===");
    println!("  film_type={}, film_curve={}, film_gamma={:.2}", adj.film_type, adj.film_curve, adj.film_gamma);
    println!("  levels_black={:?}", adj.levels_black);
    println!("  levels_white={:?}", adj.levels_white);
    println!("  levels_gamma={:?}", adj.levels_gamma);
    println!("  output_shadow={:.0}, output_highlight={:.0}", adj.output_shadow, adj.output_highlight);
    println!("  exposure={:.3}, brightness={:.1}, contrast={:.1}", adj.exposure, adj.brightness, adj.contrast);
    println!("  saturation={:.1}, lightness={:.1}, midtone={:.2}", adj.saturation, adj.lightness, adj.midtone);
    println!("  color_temp={:.1}, tint={:.1}", adj.color_temperature, adj.tint);
    println!("  apply: levels={}, film_curve={}, curves={}, color_corr={}",
             adj.apply_levels, adj.apply_film_curve, adj.apply_curves, adj.apply_color_corr);
    if adj.apply_color_corr {
        println!("  color_corr (top-left 3x3): [{},{},{}  {},{},{}  {},{},{}]",
                 adj.color_corr[0], adj.color_corr[1], adj.color_corr[2],
                 adj.color_corr[6], adj.color_corr[7], adj.color_corr[8],
                 adj.color_corr[12], adj.color_corr[13], adj.color_corr[14]);
    }
}

// ─── 逐步管线测试 ──────────────────────────────────────────────────────────

fn run_pipeline_test(
    raw_16: &image::DynamicImage,
    corr: &ImageCorrection,
    thumb_ref: &image::RgbImage,
    dump_pixels: usize,
    icc_data: Option<&[u8]>,
) {
    let (tw, th) = thumb_ref.dimensions();
    let (rw, rh) = (raw_16.width(), raw_16.height());
    println!("\n参考缩略图尺寸: {}×{}", tw, th);
    println!("原始 16-bit 尺寸: {}×{}", rw, rh);
    let need_downsample = rw != tw || rh != th;
    if need_downsample {
        println!("⚠ 尺寸不匹配，需要降采样（可能引入对齐误差）");
    }

    /// 将处理结果转为与缩略图同尺寸的 8-bit 图像
    fn to_comparable(img: &image::DynamicImage, tw: u32, th: u32, need_ds: bool) -> image::RgbImage {
        if need_ds {
            let ds = downsample_16bit_nearest(img, tw, th);
            to_rgb8_truncate(&ds)
        } else {
            to_rgb8_truncate(img)
        }
    }

    let adj = build_manual_adjust(corr);
    print_adjust_summary(&adj);

    // ─── Step 0: 原始 16-bit → 降采样 → 8-bit (无任何处理) ───
    {
        let our_8 = to_comparable(raw_16, tw, th, need_downsample);
        compare_images("Step 0: Raw (无处理)", &our_8, thumb_ref);
    }

    // ─── Step 0.5: ICC 色彩空间转换 ───
    let after_icc = if let Some(icc) = icc_data {
        // 尝试多种目标色彩空间
        for &target in &[
            color::TargetColorSpace::SRGB,
            color::TargetColorSpace::AdobeRGB,
            color::TargetColorSpace::ProPhotoRGB,
        ] {
            if let Ok(transformed) = color::apply_icc_transform(raw_16, icc, target) {
                let our_8 = to_comparable(&transformed, tw, th, need_downsample);
                compare_images(&format!("Step 0.5: ICC → {:?}", target), &our_8, thumb_ref);
            }
        }
        // 使用 sRGB 作为默认目标（FlexColor 通常输出 sRGB）
        match color::apply_icc_transform(raw_16, icc, color::TargetColorSpace::SRGB) {
            Ok(t) => { println!("  → 使用 sRGB 继续后续步骤"); t }
            Err(e) => { println!("  ⚠ ICC 转换失败: {}, 跳过", e); raw_16.clone() }
        }
    } else {
        println!("\n  ⚠ 未找到嵌入 ICC 配置文件，跳过 ICC 转换");
        raw_16.clone()
    };

    // ─── Step 1: 底片反转 ───
    let after_film = color::apply_film_processing(&after_icc, corr);
    {
        let our_8 = to_comparable(&after_film, tw, th, need_downsample);
        compare_images("Step 1: Film Processing (底片反转)", &our_8, thumb_ref);
    }

    // ─── Step 2: 底片反转 + 渐变曲线 ───
    let curve_points: Vec<Vec<(i64, i64, i64)>> = if adj.apply_curves && !corr.gradations.is_empty() {
        corr.gradations.clone()
    } else {
        // 恒等曲线
        (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect()
    };
    let after_curves = if adj.apply_curves && curve_points.len() >= 7 {
        color::apply_gradation_curves(&after_film, &curve_points)
    } else {
        after_film.clone()
    };
    {
        let our_8 = to_comparable(&after_curves, tw, th, need_downsample);
        compare_images("Step 2: + Gradation Curves (渐变曲线)", &our_8, thumb_ref);
    }

    // ─── Step 3: + 仅色阶 (levels only) ───
    {
        let mut adj_levels_only = color::ManualAdjust::default();
        adj_levels_only.film_type = adj.film_type;
        adj_levels_only.film_curve = adj.film_curve;
        adj_levels_only.film_gamma = adj.film_gamma;
        adj_levels_only.levels_black = adj.levels_black;
        adj_levels_only.levels_white = adj.levels_white;
        adj_levels_only.levels_gamma = adj.levels_gamma;
        adj_levels_only.output_shadow = adj.output_shadow;
        adj_levels_only.output_highlight = adj.output_highlight;
        // 其他全部禁用
        adj_levels_only.apply_film_curve = false;
        adj_levels_only.apply_exposure = false;
        adj_levels_only.apply_brightness = false;
        adj_levels_only.apply_shadow_depth = false;
        adj_levels_only.apply_midtone = false;
        adj_levels_only.apply_contrast = false;
        adj_levels_only.apply_highlights = false;
        adj_levels_only.apply_shadows = false;
        adj_levels_only.apply_saturation = false;
        adj_levels_only.apply_color_balance = false;
        adj_levels_only.apply_color_temp = false;
        adj_levels_only.apply_color_corr = false;

        let result = color::apply_manual_adjust(&after_curves, &adj_levels_only, None);
        let our_8 = to_comparable(&result, tw, th, need_downsample);
        compare_images("Step 3: + Levels Only (仅色阶+gamma+output)", &our_8, thumb_ref);
    }

    // ─── Step 4: + 色阶 + Film Curve LUT ───
    {
        let mut adj_lf = color::ManualAdjust::default();
        adj_lf.film_type = adj.film_type;
        adj_lf.film_curve = adj.film_curve;
        adj_lf.film_gamma = adj.film_gamma;
        adj_lf.levels_black = adj.levels_black;
        adj_lf.levels_white = adj.levels_white;
        adj_lf.levels_gamma = adj.levels_gamma;
        adj_lf.output_shadow = adj.output_shadow;
        adj_lf.output_highlight = adj.output_highlight;
        adj_lf.apply_film_curve = true;
        adj_lf.apply_exposure = false;
        adj_lf.apply_brightness = false;
        adj_lf.apply_shadow_depth = false;
        adj_lf.apply_midtone = false;
        adj_lf.apply_contrast = false;
        adj_lf.apply_highlights = false;
        adj_lf.apply_shadows = false;
        adj_lf.apply_saturation = false;
        adj_lf.apply_color_balance = false;
        adj_lf.apply_color_temp = false;
        adj_lf.apply_color_corr = false;

        let result = color::apply_manual_adjust(&after_curves, &adj_lf, None);
        let our_8 = to_comparable(&result, tw, th, need_downsample);
        compare_images("Step 4: + Film Curve LUT", &our_8, thumb_ref);
    }

    // ─── Step 5: + 色阶 + Film LUT + Exposure/ColorTemp ───
    {
        let mut adj5 = color::ManualAdjust::default();
        adj5.film_type = adj.film_type;
        adj5.film_curve = adj.film_curve;
        adj5.film_gamma = adj.film_gamma;
        adj5.levels_black = adj.levels_black;
        adj5.levels_white = adj.levels_white;
        adj5.levels_gamma = adj.levels_gamma;
        adj5.output_shadow = adj.output_shadow;
        adj5.output_highlight = adj.output_highlight;
        adj5.apply_film_curve = true;
        adj5.exposure = adj.exposure;
        adj5.apply_exposure = true;
        adj5.color_temperature = adj.color_temperature;
        adj5.tint = adj.tint;
        adj5.apply_color_temp = true;
        adj5.apply_brightness = false;
        adj5.apply_shadow_depth = false;
        adj5.apply_midtone = false;
        adj5.apply_contrast = false;
        adj5.apply_highlights = false;
        adj5.apply_shadows = false;
        adj5.apply_saturation = false;
        adj5.apply_color_balance = false;
        adj5.apply_color_corr = false;

        let result = color::apply_manual_adjust(&after_curves, &adj5, None);
        let our_8 = to_comparable(&result, tw, th, need_downsample);
        compare_images("Step 5: + Exposure + ColorTemp", &our_8, thumb_ref);
    }

    // ─── Step 6: + 所有调整项（对比度/亮度/阴影/高光/饱和度...） ───
    {
        let mut adj6 = adj.clone();
        adj6.apply_color_corr = false; // 先不加色彩矩阵
        let result = color::apply_manual_adjust(&after_curves, &adj6, None);
        let our_8 = to_comparable(&result, tw, th, need_downsample);
        compare_images("Step 6: + All Sliders (无色彩矩阵)", &our_8, thumb_ref);
    }

    // ─── Step 7: 全管线（含色彩矩阵） ───
    {
        let result = color::apply_manual_adjust(&after_curves, &adj, None);
        let our_8 = to_comparable(&result, tw, th, need_downsample);
        compare_images("Step 7: Full Pipeline (全管线)", &our_8, thumb_ref);

        // 可选：打印差异最大的 N 个像素 + 原始16bit值
        if dump_pixels > 0 {
            let our_raw = our_8.as_raw();
            let ref_raw = thumb_ref.as_raw();
            let total_px = our_raw.len() / 3;

            // 获取原始 16-bit 数据（用于分析）
            let raw_16_data: Option<&[u16]> = match raw_16 {
                image::DynamicImage::ImageRgb16(ref rgb16) => Some(rgb16.as_raw()),
                _ => None,
            };

            // 收集所有非黑像素的差异
            let mut diffs: Vec<(usize, u8, u8, u8, u8, u8, u8, i16, i16, i16, u16)> = Vec::new();
            for i in 0..total_px {
                let (or, og, ob) = (our_raw[i*3], our_raw[i*3+1], our_raw[i*3+2]);
                let (rr, rg, rb) = (ref_raw[i*3], ref_raw[i*3+1], ref_raw[i*3+2]);
                if or == 0 && og == 0 && ob == 0 && rr == 0 && rg == 0 && rb == 0 { continue; }
                let (dr, dg, db) = (or as i16 - rr as i16, og as i16 - rg as i16, ob as i16 - rb as i16);
                let total_err = dr.unsigned_abs() + dg.unsigned_abs() + db.unsigned_abs();
                diffs.push((i, or, og, ob, rr, rg, rb, dr, dg, db, total_err));
            }
            diffs.sort_by(|a, b| b.10.cmp(&a.10));
            let n = dump_pixels.min(diffs.len());
            println!("\n=== 差异最大的 {} 个像素（共 {} 个非黑像素） ===", n, diffs.len());
            println!("{:>7} │ {:>12} │ {:>12} │ {:>12} │ {:>5} │ {:>18}",
                     "Idx", "Ours(R,G,B)", "Ref(R,G,B)", "Diff(R,G,B)", "Total", "Raw16(R,G,B)");
            for d in diffs.iter().take(n) {
                let (i, or, og, ob, rr, rg, rb, dr, dg, db, te) = *d;
                let x = i % tw as usize;
                let y = i / tw as usize;
                let raw_str = if let Some(r16) = raw_16_data {
                    let ri = i * 3;
                    if ri + 2 < r16.len() {
                        format!("({:>5},{:>5},{:>5})", r16[ri], r16[ri+1], r16[ri+2])
                    } else {
                        String::from("N/A")
                    }
                } else {
                    String::from("N/A")
                };
                println!("{:>3},{:>3} │ ({:>3},{:>3},{:>3}) │ ({:>3},{:>3},{:>3}) │ ({:>+4},{:>+4},{:>+4}) │ {:>5} │ {}",
                         x, y, or, og, ob, rr, rg, rb, dr, dg, db, te, raw_str);
            }

            // 还打印一些中等差异的像素来看关系
            println!("\n=== 中等差异像素采样（第50-70百分位） ===");
            let mid_start = diffs.len() / 2;
            let mid_n = 10.min(diffs.len() - mid_start);
            println!("{:>7} │ {:>12} │ {:>12} │ {:>12} │ {:>18} │ {:>12}",
                     "Idx", "Ours(R,G,B)", "Ref(R,G,B)", "Diff(R,G,B)", "Raw16(R,G,B)", "Raw>>8");
            for d in diffs.iter().skip(mid_start).take(mid_n) {
                let (i, or, og, ob, rr, rg, rb, dr, dg, db, _te) = *d;
                let x = i % tw as usize;
                let y = i / tw as usize;
                let (raw_str, trunc_str) = if let Some(r16) = raw_16_data {
                    let ri = i * 3;
                    if ri + 2 < r16.len() {
                        (format!("({:>5},{:>5},{:>5})", r16[ri], r16[ri+1], r16[ri+2]),
                         format!("({:>3},{:>3},{:>3})", r16[ri]>>8, r16[ri+1]>>8, r16[ri+2]>>8))
                    } else {
                        (String::from("N/A"), String::from("N/A"))
                    }
                } else {
                    (String::from("N/A"), String::from("N/A"))
                };
                println!("{:>3},{:>3} │ ({:>3},{:>3},{:>3}) │ ({:>3},{:>3},{:>3}) │ ({:>+4},{:>+4},{:>+4}) │ {} │ {}",
                         x, y, or, og, ob, rr, rg, rb, dr, dg, db, raw_str, trunc_str);
            }
        }
    }
}

// ─── 主函数 ─────────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: cargo run --example color_compare -- <file.fff> [--dump-pixels N]");
        std::process::exit(1);
    }

    let path = &args[1];
    let dump_pixels = args.iter().position(|a| a == "--dump-pixels")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    println!("════════════════════════════════════════════");
    println!("  色彩管线对照工具");
    println!("  文件: {}", path);
    println!("════════════════════════════════════════════");

    // 打开 FFF 文件
    let tiff = TiffFile::open(Path::new(path)).expect("无法打开 FFF 文件");

    // 列出所有 IFD
    println!("\n=== IFD 列表 ===");
    for (idx, ifd) in tiff.ifds.iter().enumerate() {
        let w = ifd.get_u32(0x0100).unwrap_or(0);
        let h = ifd.get_u32(0x0101).unwrap_or(0);
        let bps = ifd.get_u32(0x0102).unwrap_or(8);
        let subfile = ifd.get_u32(0x00FE).unwrap_or(0);
        println!("  IFD#{}: {}×{}, {}bit, SubfileType={}", idx, w, h, bps, subfile);
    }

    // 提取缩略图（SubfileType=1，8-bit reference）
    let thumb = tiff.decode_thumbnail().expect("无法解码缩略图");
    let thumb_rgb8 = thumb.to_rgb8();
    let (tw, th) = thumb_rgb8.dimensions();
    println!("\n缩略图: {}×{} (8-bit reference)", tw, th);

    // 查找与缩略图同尺寸的 16-bit IFD（优先使用，避免降采样误差）
    let raw_16 = {
        let mut found_matching = None;
        for (idx, ifd) in tiff.ifds.iter().enumerate() {
            let w = ifd.get_u32(0x0100).unwrap_or(0);
            let h = ifd.get_u32(0x0101).unwrap_or(0);
            let bps = ifd.get_u32(0x0102).unwrap_or(8);
            if w == tw && h == th && bps == 16 {
                println!("找到匹配的 16-bit IFD#{}: {}×{}", idx, w, h);
                found_matching = tiff.decode_uncompressed_rgb(ifd);
                break;
            }
        }
        if let Some(img) = found_matching {
            img
        } else {
            println!("未找到匹配尺寸的 16-bit IFD，使用降采样");
            let max_dim = tw.max(th);
            tiff.decode_preview_downscaled(max_dim)
                .expect("无法解码 16-bit 预览")
        }
    };
    println!("16-bit 预览: {}×{}", raw_16.width(), raw_16.height());

    // 提取编辑历史
    let edit_history = EditHistory::parse_from_tiff(&tiff)
        .expect("无法解析编辑历史");
    println!("\n=== 编辑历史 ===");
    println!("  {} 组设置, 当前索引: {}", edit_history.settings.len(), edit_history.current_index);

    // 使用当前色彩方案
    let idx = edit_history.current_index.min(edit_history.settings.len() - 1);
    let setting = &edit_history.settings[idx];
    let corr = &setting.correction;

    println!("\n=== 色彩方案 #{} ===", idx);
    println!("  film_type={} ({})", corr.film_type, flexcolor::film_type_name(corr.film_type));
    println!("  film_curve={}, gamma={:.4}", corr.film_curve, corr.gamma);
    println!("  apply: sliders={}, curves={}, histogram={}, cc={}, usm={}",
             corr.apply_sliders, corr.apply_curves, corr.apply_histogram, corr.apply_cc, corr.apply_usm);
    println!("  shadow={:?}", corr.shadow);
    println!("  highlight={:?}", corr.highlight);
    println!("  gray={:?}", corr.gray);
    println!("  ev={:.4}, contrast={}, brightness={}, saturation={}",
             corr.ev, corr.contrast, corr.brightness, corr.saturation);
    println!("  color_temperature={}, tint={}, lightness={}",
             corr.color_temperature, corr.tint, corr.lightness);
    if !corr.dot_color.is_empty() {
        println!("  dot_color={:?}", corr.dot_color);
    }
    if !corr.gradations.is_empty() {
        println!("  gradations: {} 通道", corr.gradations.len());
        for (ch, pts) in corr.gradations.iter().enumerate() {
            let names = ["RGB", "R", "G", "B", "C", "M", "Y"];
            let name = names.get(ch).unwrap_or(&"?");
            println!("    {}: {} 控制点", name, pts.len());
        }
    }

    // 提取嵌入的 ICC 配置文件
    let all_tags = tiff.all_tags();
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
    if icc_data.is_some() {
        println!("\n✓ 找到嵌入 ICC 配置文件 ({} bytes)", icc_data.as_ref().unwrap().len());
    } else {
        println!("\n⚠ 未找到嵌入 ICC 配置文件");
    }

    // 执行逐步管线测试
    run_pipeline_test(&raw_16, corr, &thumb_rgb8, dump_pixels, icc_data.as_deref());

    println!("\n完成。");
}
