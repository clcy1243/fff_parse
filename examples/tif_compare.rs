//! 全分辨率 TIF 导出 + 对照工具
//!
//! 用法: cargo run --release --example tif_compare -- <file.fff> <reference.tif>
//!
//! 将 FFF 文件通过我们的管线渲染为全分辨率 8-bit TIF，
//! 然后与 FlexColor 导出的 TIF 逐像素对比。

use std::env;
use std::path::Path;

use fff_viewer::color;
use fff_viewer::flexcolor::{self, EditHistory, ImageCorrection};
use fff_viewer::tiff::TiffFile;

// ─── 差异统计 ────────────────────────────────────────────────────────────────

struct ChannelStats {
    name: String,
    mae: f64,
    max_err: u32,
    psnr: f64,
    count: usize,
    err_gt5: usize,
    err_gt10: usize,
    err_gt20: usize,
    mean_ours: f64,
    mean_ref: f64,
    min_ours: u8,
    max_ours: u8,
    min_ref: u8,
    max_ref: u8,
}

fn compute_channel_stats(name: &str, ours: &[u8], reference: &[u8]) -> ChannelStats {
    assert_eq!(ours.len(), reference.len());
    let count = ours.len();
    let mut sum_abs_err: u64 = 0;
    let mut sum_sq_err: f64 = 0.0;
    let mut max_err: u32 = 0;
    let mut err_gt5 = 0usize;
    let mut err_gt10 = 0usize;
    let mut err_gt20 = 0usize;
    let mut sum_ours: u64 = 0;
    let mut sum_ref: u64 = 0;
    let mut min_ours = 255u8;
    let mut max_ours = 0u8;
    let mut min_ref = 255u8;
    let mut max_ref = 0u8;

    for i in 0..count {
        let o = ours[i];
        let r = reference[i];
        let diff = (o as i32 - r as i32).unsigned_abs();
        sum_abs_err += diff as u64;
        sum_sq_err += (diff as f64) * (diff as f64);
        if diff > max_err { max_err = diff; }
        if diff > 5 { err_gt5 += 1; }
        if diff > 10 { err_gt10 += 1; }
        if diff > 20 { err_gt20 += 1; }
        sum_ours += o as u64;
        sum_ref += r as u64;
        if o < min_ours { min_ours = o; }
        if o > max_ours { max_ours = o; }
        if r < min_ref { min_ref = r; }
        if r > max_ref { max_ref = r; }
    }

    let mae = sum_abs_err as f64 / count as f64;
    let mse = sum_sq_err / count as f64;
    let psnr = if mse > 0.0 { 10.0 * (255.0_f64 * 255.0 / mse).log10() } else { f64::INFINITY };

    ChannelStats {
        name: name.to_string(),
        mae, max_err, psnr, count,
        err_gt5, err_gt10, err_gt20,
        mean_ours: sum_ours as f64 / count as f64,
        mean_ref: sum_ref as f64 / count as f64,
        min_ours, max_ours, min_ref, max_ref,
    }
}

fn print_stats(label: &str, stats: &[ChannelStats]) {
    println!("\n┌─── {} ───", label);
    println!("│  Chan │      MAE │   MaxErr │   PSNR(dB) │ mean(ours/ref) │ range(ours/ref) │ Err>5 │ Err>10 │ Err>20");
    println!("│──────────────────────────────────────────────────────────────────────────────────────────────────────────────");
    for s in stats {
        println!("│  {:>4} │ {:>8.2} │ {:>8} │ {:>10.1} │ {:>6.1}/{:<6.1} │ {}-{}/{}-{} │ {:>5.1}% │ {:>5.1}% │ {:>5.1}%",
                 s.name, s.mae, s.max_err, s.psnr,
                 s.mean_ours, s.mean_ref,
                 s.min_ours, s.max_ours, s.min_ref, s.max_ref,
                 s.err_gt5 as f64 / s.count as f64 * 100.0,
                 s.err_gt10 as f64 / s.count as f64 * 100.0,
                 s.err_gt20 as f64 / s.count as f64 * 100.0);
    }
    println!("└──────────────────────────────────────────────────────────────────────────────────────────────────────────────");
}

// ─── ManualAdjust 构建 ───────────────────────────────────────────────────────

fn build_manual_adjust(corr: &ImageCorrection) -> color::ManualAdjust {
    let mut adj = color::ManualAdjust::default();
    adj.film_type = corr.film_type;
    adj.film_curve = corr.film_curve;
    adj.film_gamma = corr.gamma;

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
            adj.output_shadow = [corr.dot_color[0] as f32, corr.dot_color[1] as f32, corr.dot_color[2] as f32, corr.dot_color[3] as f32];
            adj.output_highlight = [corr.dot_color[7] as f32, corr.dot_color[8] as f32, corr.dot_color[9] as f32, corr.dot_color[10] as f32];
        }
    }

    if corr.apply_sliders {
        adj.saturation = corr.saturation as f32;
        if (corr.ev - 1.0).abs() > 0.001 {
            adj.exposure = corr.ev.log2() as f32;
        }
        adj.contrast = corr.contrast as f32;
        adj.brightness = corr.brightness as f32;
        adj.lightness = corr.lightness as f32;
    }

    adj.color_temperature = corr.color_temperature as f32;
    adj.tint = corr.tint as f32;

    if corr.apply_cc && corr.color_corr.len() == 36 {
        for (i, &v) in corr.color_corr.iter().enumerate() {
            adj.color_corr[i] = v;
        }
        adj.apply_color_corr = true;
    } else {
        adj.apply_color_corr = false;
    }

    adj.apply_curves = corr.apply_curves && !corr.gradations.is_empty();
    adj
}

/// 16-bit DynamicImage → 8-bit RGB (>>8 truncation)
fn to_rgb8(img: &image::DynamicImage) -> image::RgbImage {
    let rgb16 = img.to_rgb16();
    let (w, h) = (rgb16.width(), rgb16.height());
    let mut out = image::RgbImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = rgb16.get_pixel(x, y);
            out.put_pixel(x, y, image::Rgb([
                (p[0] >> 8) as u8,
                (p[1] >> 8) as u8,
                (p[2] >> 8) as u8,
            ]));
        }
    }
    out
}

fn compare_rgb_images(label: &str, ours: &image::RgbImage, reference: &image::RgbImage) {
    assert_eq!(ours.dimensions(), reference.dimensions(),
               "size mismatch: ours {:?} vs ref {:?}", ours.dimensions(), reference.dimensions());
    let pixel_count = (ours.width() * ours.height()) as usize;
    let our_raw = ours.as_raw();
    let ref_raw = reference.as_raw();

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

// ─── 主函数 ──────────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("用法: cargo run --release --example tif_compare -- <file.fff> <reference.tif>");
        std::process::exit(1);
    }

    let fff_path = &args[1];
    let ref_path = &args[2];

    println!("════════════════════════════════════════════");
    println!("  全分辨率 TIF 导出对照工具");
    println!("  FFF: {}", fff_path);
    println!("  参考: {}", ref_path);
    println!("════════════════════════════════════════════");

    // 加载参考 TIF
    let ref_img = image::open(ref_path).expect("无法打开参考 TIF");
    let ref_rgb8 = ref_img.to_rgb8();
    let (ref_w, ref_h) = ref_rgb8.dimensions();
    println!("\n参考 TIF: {}×{} ({}-bit)", ref_w, ref_h,
             if ref_img.color().bytes_per_pixel() > 3 { 16 } else { 8 });

    // 打开 FFF 文件
    let tiff = TiffFile::open(Path::new(fff_path)).expect("无法打开 FFF 文件");

    // 列出 IFD
    println!("\n=== IFD 列表 ===");
    for (idx, ifd) in tiff.ifds.iter().enumerate() {
        let w = ifd.get_u32(0x0100).unwrap_or(0);
        let h = ifd.get_u32(0x0101).unwrap_or(0);
        let bps = ifd.get_u32(0x0102).unwrap_or(8);
        let subfile = ifd.get_u32(0x00FE).unwrap_or(0);
        println!("  IFD#{}: {}×{}, {}bit, SubfileType={}", idx, w, h, bps, subfile);
    }

    // 加载全分辨率 16-bit IFD#0
    let ifd0 = &tiff.ifds[0];
    let ifd0_w = ifd0.get_u32(0x0100).unwrap_or(0);
    let ifd0_h = ifd0.get_u32(0x0101).unwrap_or(0);
    println!("\n加载 IFD#0 全分辨率: {}×{}", ifd0_w, ifd0_h);

    let raw_16 = tiff.decode_uncompressed_rgb(ifd0).expect("无法解码 IFD#0");
    println!("解码完成: {}×{}", raw_16.width(), raw_16.height());

    // 尺寸检查
    if raw_16.width() != ref_w || raw_16.height() != ref_h {
        println!("⚠ 尺寸不匹配！raw={}×{}, ref={}×{}",
                 raw_16.width(), raw_16.height(), ref_w, ref_h);
    }

    // 提取编辑历史
    let edit_history = EditHistory::parse_from_tiff(&tiff).expect("无法解析编辑历史");
    let idx = edit_history.current_index.min(edit_history.settings.len() - 1);
    let corr = &edit_history.settings[idx].correction;
    println!("\n=== 色彩方案 #{} ({}) ===", idx, edit_history.settings[idx].name);
    println!("  film_type={} ({})", corr.film_type, flexcolor::film_type_name(corr.film_type));
    println!("  DotColor={:?}", corr.dot_color);
    println!("  shadow={:?}, highlight={:?}", corr.shadow, corr.highlight);
    println!("  gray={:?}, gamma={:.4}", corr.gray, corr.gamma);
    println!("  apply: sliders={}, curves={}, histogram={}, cc={}, usm={}",
             corr.apply_sliders, corr.apply_curves, corr.apply_histogram, corr.apply_cc, corr.apply_usm);
    if corr.apply_sliders {
        println!("  contrast={}, brightness={}, saturation={}, lightness={}",
                 corr.contrast, corr.brightness, corr.saturation, corr.lightness);
    }

    // 提取 ICC 配置文件
    let all_tags = tiff.all_tags();
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
    if let Some(ref icc) = icc_data {
        println!("✓ 嵌入 ICC 配置文件: {} bytes", icc.len());
    } else {
        println!("⚠ 未找到嵌入 ICC 配置文件");
    }

    // 构建 ManualAdjust
    let adj = build_manual_adjust(corr);
    println!("\n=== ManualAdjust ===");
    println!("  levels_black={:?}", adj.levels_black);
    println!("  levels_white={:?}", adj.levels_white);
    println!("  levels_gamma={:?}", adj.levels_gamma);
    println!("  output_shadow={:?}", adj.output_shadow);
    println!("  output_highlight={:?}", adj.output_highlight);

    // 准备曲线
    let curve_points: Vec<Vec<(i64, i64, i64)>> = if adj.apply_curves && !corr.gradations.is_empty() {
        corr.gradations.clone()
    } else {
        (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect()
    };

    // 提取胶片曲线 LUT
    let thumb_img = tiff.decode_thumbnail();
    let preview_16 = tiff.decode_uncompressed_rgb(&tiff.ifds[2]).map(|img| img.to_rgb16());
    let film_lut = if let (Some(ref t), Some(ref p)) = (&thumb_img, &preview_16) {
        let t8 = t.to_rgb8();
        if corr.film_type == 1 || corr.film_type == 2 {
            color::extract_film_curve(&t8, p, corr)
        } else {
            None
        }
    } else {
        None
    };

    // ══════════════════════════════════════════════════════════════════
    //  测试不同管线配置，找出最匹配的
    // ══════════════════════════════════════════════════════════════════

    // ─── Test A: 原始（无处理） ───
    {
        let our_8 = to_rgb8(&raw_16);
        compare_rgb_images("A: Raw (无处理)", &our_8, &ref_rgb8);
    }

    // ─── Test B: 仅底片反转 ───
    let after_film = color::apply_film_processing(&raw_16, corr);
    {
        let our_8 = to_rgb8(&after_film);
        compare_rgb_images("B: Film Processing", &our_8, &ref_rgb8);
    }

    // ─── Test C: 完整管线（当前实现） ───
    {
        let result = color::apply_color_pipeline(
            after_film.clone(),
            &adj,
            &curve_points,
            film_lut.as_ref(),
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("C: Full Pipeline (当前)", &our_8, &ref_rgb8);

        // 保存我们的结果
        let out_path = "/tmp/fff_our_export.tif";
        our_8.save(out_path).expect("无法保存我们的 TIF");
        println!("  → 已保存到 {}", out_path);
    }

    // ─── Test D: 完整管线，但禁用 DotColor ───
    {
        let mut adj_no_dot = adj.clone();
        adj_no_dot.output_shadow = [0.0; 4];
        adj_no_dot.output_highlight = [255.0; 4];

        let result = color::apply_color_pipeline(
            after_film.clone(),
            &adj_no_dot,
            &curve_points,
            film_lut.as_ref(),
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("D: Full Pipeline - 无DotColor", &our_8, &ref_rgb8);
    }

    // ─── Test E: 完整管线，禁用 DotColor + 禁用色彩校正 ───
    {
        let mut adj_simple = adj.clone();
        adj_simple.output_shadow = [0.0; 4];
        adj_simple.output_highlight = [255.0; 4];
        adj_simple.apply_color_corr = false;

        let result = color::apply_color_pipeline(
            after_film.clone(),
            &adj_simple,
            &curve_points,
            film_lut.as_ref(),
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("E: Full Pipeline - 无DotColor/CC", &our_8, &ref_rgb8);
    }

    // ─── Test F: 仅色阶（无其他效果） ───
    {
        let mut adj_levels = color::ManualAdjust::default();
        adj_levels.film_type = adj.film_type;
        adj_levels.film_curve = adj.film_curve;
        adj_levels.film_gamma = adj.film_gamma;
        adj_levels.levels_black = adj.levels_black;
        adj_levels.levels_white = adj.levels_white;
        adj_levels.levels_gamma = adj.levels_gamma;
        // 不应用输出色阶
        adj_levels.apply_film_curve = adj.apply_film_curve;

        let result = color::apply_color_pipeline(
            after_film.clone(),
            &adj_levels,
            &curve_points,
            film_lut.as_ref(),
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("F: Film + Curves + Levels + ICC (无sliders)", &our_8, &ref_rgb8);
    }

    // ─── Test G: 仅底片反转 + ICC ───
    {
        let adj_identity = color::ManualAdjust::default();
        let identity_curves: Vec<Vec<(i64, i64, i64)>> = (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect();

        let result = color::apply_color_pipeline(
            after_film.clone(),
            &adj_identity,
            &identity_curves,
            None,
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("G: Film + ICC only", &our_8, &ref_rgb8);
    }

    // ─── Test I: 管线重排 — 曲线放在最后 ───
    // 假设 FlexColor 的顺序: Film → Levels → ICC → Display → Curves
    {
        let mut adj_no_curves = adj.clone();
        adj_no_curves.apply_curves = false;  // 先跳过曲线
        let identity_curves: Vec<Vec<(i64, i64, i64)>> = (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect();

        // 步骤 1-4: Film → Levels → ICC → Display (无曲线)
        let result = color::apply_color_pipeline(
            after_film.clone(),
            &adj_no_curves,
            &identity_curves,
            film_lut.as_ref(),
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        // 步骤 5: 最后应用渐变曲线
        let result = if adj.apply_curves && curve_points.len() >= 7 {
            color::apply_gradation_curves(&result, &curve_points)
        } else {
            result
        };
        let our_8 = to_rgb8(&result);
        compare_rgb_images("I: Curves LAST (Film→Levels→ICC→Adjust→Curves)", &our_8, &ref_rgb8);
    }

    // ─── Test J: 曲线在 ICC 之后、Display 之前 ───
    {
        let mut adj_no_curves = adj.clone();
        adj_no_curves.apply_curves = false;
        let identity_curves: Vec<Vec<(i64, i64, i64)>> = (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect();

        // Film → Levels → ICC (无曲线，无 display)
        let mut adj_levels_only = color::ManualAdjust::default();
        adj_levels_only.film_type = adj.film_type;
        adj_levels_only.film_curve = adj.film_curve;
        adj_levels_only.film_gamma = adj.film_gamma;
        adj_levels_only.levels_black = adj.levels_black;
        adj_levels_only.levels_white = adj.levels_white;
        adj_levels_only.levels_gamma = adj.levels_gamma;
        adj_levels_only.apply_film_curve = adj.apply_film_curve;

        let after_levels = color::apply_color_pipeline(
            after_film.clone(),
            &adj_levels_only,
            &identity_curves,
            film_lut.as_ref(),
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        // 然后曲线
        let after_curves = if adj.apply_curves && curve_points.len() >= 7 {
            color::apply_gradation_curves(&after_levels, &curve_points)
        } else {
            after_levels
        };
        // 然后 Display
        let result = color::apply_manual_adjust(&after_curves, &adj_no_curves, None);
        let our_8 = to_rgb8(&result);
        compare_rgb_images("J: Curves between ICC and Display", &our_8, &ref_rgb8);
    }

    // ─── Test K: Setting #1 参数 + setting #6 的曲线 (验证曲线是否是主要差异) ───
    {
        // Use setting #1's simple parameters but add setting #6's curves at the end
        let c1 = &edit_history.settings[1].correction;
        let mut a1 = build_manual_adjust(c1);
        a1.apply_curves = false;  // Don't apply curves in pipeline
        let identity_curves: Vec<Vec<(i64, i64, i64)>> = (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect();

        let after1 = color::apply_film_processing(&raw_16, c1);
        let result = color::apply_color_pipeline(
            after1,
            &a1,
            &identity_curves,
            film_lut.as_ref(),
            icc_data.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        // Now apply setting #6's complex curves at the end
        let result = if adj.apply_curves && curve_points.len() >= 7 {
            color::apply_gradation_curves(&result, &curve_points)
        } else {
            result
        };
        let our_8 = to_rgb8(&result);
        compare_rgb_images("K: Setting#1 params + Setting#6 curves (last)", &our_8, &ref_rgb8);
    }

    // ─── Test H: 尝试不同的编辑历史索引 ───
    println!("\n\n=== 测试不同编辑历史索引 ===");
    for (si, setting) in edit_history.settings.iter().enumerate() {
        let c = &setting.correction;
        let a = build_manual_adjust(c);
        let cp: Vec<Vec<(i64, i64, i64)>> = if a.apply_curves && !c.gradations.is_empty() {
            c.gradations.clone()
        } else {
            (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect()
        };

        let fl = if let (Some(ref t), Some(ref p)) = (&thumb_img, &preview_16) {
            let t8 = t.to_rgb8();
            if c.film_type == 1 || c.film_type == 2 {
                color::extract_film_curve(&t8, p, c)
            } else {
                None
            }
        } else {
            None
        };

        let after = color::apply_film_processing(&raw_16, c);
        let result = color::apply_color_pipeline(
            after, &a, &cp, fl.as_ref(),
            icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images(&format!("H{}: Setting #{} '{}'", si, si, setting.name), &our_8, &ref_rgb8);
    }

    println!("\n完成。");
}
