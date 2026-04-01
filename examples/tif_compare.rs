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
            adj.output_shadow = [0.0, corr.dot_color[0] as f32, corr.dot_color[1] as f32, corr.dot_color[2] as f32];
            adj.output_highlight = [255.0, corr.dot_color[7] as f32, corr.dot_color[8] as f32, corr.dot_color[9] as f32];
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

fn image_to_rgb8(img: &image::DynamicImage) -> image::RgbImage {
    img.to_rgb8()
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

    // 提取 ICC 配置文件（嵌入或从磁盘加载）
    let all_tags = tiff.all_tags();
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
    let icc_data = if icc_data.is_none() {
        let profile_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("profiles")
            .join("Flextight X5 & 949.icc");
        if profile_path.exists() {
            let data = std::fs::read(&profile_path).ok();
            if let Some(ref d) = data {
                println!("✓ ICC (from disk): {} bytes — {}", d.len(), profile_path.display());
            }
            data
        } else {
            println!("⚠ ICC profile not found at {}", profile_path.display());
            None
        }
    } else {
        println!("✓ ICC (embedded): {} bytes", icc_data.as_ref().unwrap().len());
        icc_data
    };

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
        let c1 = &edit_history.settings[1].correction;
        let mut a1 = build_manual_adjust(c1);
        a1.apply_curves = false;
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
        let result = if adj.apply_curves && curve_points.len() >= 7 {
            color::apply_gradation_curves(&result, &curve_points)
        } else {
            result
        };
        let our_8 = to_rgb8(&result);
        compare_rgb_images("K: Setting#1 params + Setting#6 curves (last)", &our_8, &ref_rgb8);
    }

    // ══════════════════════════════════════════════════════════════════
    //  消融测试：从 Setting#1 逐步添加 Setting#6 的特征
    // ══════════════════════════════════════════════════════════════════
    println!("\n\n=== 消融测试: Setting#1 基础 + 逐步添加 Setting#7 特征 ===");
    let c1 = &edit_history.settings[1].correction;
    let c7 = corr;  // Current (latest) setting
    let after_film_c1 = color::apply_film_processing(&raw_16, c1);
    let identity_curves: Vec<Vec<(i64, i64, i64)>> = (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect();

    // 使用 c1 的 correction 提取 film_lut（确保与 H1 一致）
    let film_lut_c1 = if let (Some(ref t), Some(ref p)) = (&thumb_img, &preview_16) {
        let t8 = t.to_rgb8();
        if c1.film_type == 1 || c1.film_type == 2 {
            color::extract_film_curve(&t8, p, c1)
        } else {
            None
        }
    } else {
        None
    };

    // L1: Setting#1 基础 (baseline)
    {
        let a = build_manual_adjust(c1);
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &identity_curves,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L1: Setting#1 baseline", &our_8, &ref_rgb8);
    }

    // L2: + gamma/gray 变化 (1.89844 + new gray)
    {
        let mut a = build_manual_adjust(c1);
        a.film_gamma = c7.gamma;
        a.levels_gamma[0] = ((c7.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 {
            a.levels_gamma[i] = (c7.gray[i] as f32 / 128.0).clamp(0.01, 99.0);
        }
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &identity_curves,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L2: +gamma/gray变化", &our_8, &ref_rgb8);
    }

    // L3: + contrast=41
    {
        let mut a = build_manual_adjust(c1);
        a.film_gamma = c7.gamma;
        a.levels_gamma[0] = ((c7.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 { a.levels_gamma[i] = (c7.gray[i] as f32 / 128.0).clamp(0.01, 99.0); }
        a.contrast = 41.0;
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &identity_curves,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L3: +contrast=41", &our_8, &ref_rgb8);
    }

    // L4: + brightness=42
    {
        let mut a = build_manual_adjust(c1);
        a.film_gamma = c7.gamma;
        a.levels_gamma[0] = ((c7.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 { a.levels_gamma[i] = (c7.gray[i] as f32 / 128.0).clamp(0.01, 99.0); }
        a.contrast = 41.0;
        a.brightness = 42.0;
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &identity_curves,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L4: +brightness=42", &our_8, &ref_rgb8);
    }

    // L5: + lightness=43
    {
        let mut a = build_manual_adjust(c1);
        a.film_gamma = c7.gamma;
        a.levels_gamma[0] = ((c7.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 { a.levels_gamma[i] = (c7.gray[i] as f32 / 128.0).clamp(0.01, 99.0); }
        a.contrast = 41.0;
        a.brightness = 42.0;
        a.lightness = 43.0;
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &identity_curves,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L5: +lightness=43", &our_8, &ref_rgb8);
    }

    // L6: + DotColor [60,180]
    {
        let mut a = build_manual_adjust(c1);
        a.film_gamma = c7.gamma;
        a.levels_gamma[0] = ((c7.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 { a.levels_gamma[i] = (c7.gray[i] as f32 / 128.0).clamp(0.01, 99.0); }
        a.contrast = 41.0;
        a.brightness = 42.0;
        a.lightness = 43.0;
        if c7.dot_color.len() >= 14 {
            a.output_shadow = [0.0, c7.dot_color[0] as f32, c7.dot_color[1] as f32, c7.dot_color[2] as f32];
            a.output_highlight = [255.0, c7.dot_color[7] as f32, c7.dot_color[8] as f32, c7.dot_color[9] as f32];
        }
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &identity_curves,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L6: +DotColor[60,180]", &our_8, &ref_rgb8);
    }

    // L7: + color_corr
    {
        let mut a = build_manual_adjust(c1);
        a.film_gamma = c7.gamma;
        a.levels_gamma[0] = ((c7.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 { a.levels_gamma[i] = (c7.gray[i] as f32 / 128.0).clamp(0.01, 99.0); }
        a.contrast = 41.0;
        a.brightness = 42.0;
        a.lightness = 43.0;
        if c7.dot_color.len() >= 14 {
            a.output_shadow = [0.0, c7.dot_color[0] as f32, c7.dot_color[1] as f32, c7.dot_color[2] as f32];
            a.output_highlight = [255.0, c7.dot_color[7] as f32, c7.dot_color[8] as f32, c7.dot_color[9] as f32];
        }
        if c7.apply_cc && c7.color_corr.len() == 36 {
            for (i, &v) in c7.color_corr.iter().enumerate() { a.color_corr[i] = v; }
            a.apply_color_corr = true;
        }
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &identity_curves,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L7: +color_corr", &our_8, &ref_rgb8);
    }

    // L8: + gradation curves (全特征)
    {
        let mut a = build_manual_adjust(c1);
        a.film_gamma = c7.gamma;
        a.levels_gamma[0] = ((c7.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 { a.levels_gamma[i] = (c7.gray[i] as f32 / 128.0).clamp(0.01, 99.0); }
        a.contrast = 41.0;
        a.brightness = 42.0;
        a.lightness = 43.0;
        if c7.dot_color.len() >= 14 {
            a.output_shadow = [0.0, c7.dot_color[0] as f32, c7.dot_color[1] as f32, c7.dot_color[2] as f32];
            a.output_highlight = [255.0, c7.dot_color[7] as f32, c7.dot_color[8] as f32, c7.dot_color[9] as f32];
        }
        if c7.apply_cc && c7.color_corr.len() == 36 {
            for (i, &v) in c7.color_corr.iter().enumerate() { a.color_corr[i] = v; }
            a.apply_color_corr = true;
        }
        a.apply_curves = true;
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &curve_points,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L8: +curves (全特征)", &our_8, &ref_rgb8);
    }

    // L9: 仅 Setting#1 + curves (不加其它 slider)
    {
        let mut a = build_manual_adjust(c1);
        a.apply_curves = true;
        let result = color::apply_color_pipeline(
            after_film_c1.clone(), &a, &curve_points,
            film_lut_c1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_8 = to_rgb8(&result);
        compare_rgb_images("L9: Setting#1 + curves only", &our_8, &ref_rgb8);
    }

    // ═══════════════════════════════════════════════════════════════════
    // M 测试: 公式变体搜索 — 寻找正确的 FlexColor 调整公式
    // ═══════════════════════════════════════════════════════════════════
    println!("\n\n=== 公式变体测试: 使用 Setting#7 全参数 ===");

    // 提取 Setting#7 的 film_lut
    let film_lut_c7 = if let (Some(ref t), Some(ref p)) = (&thumb_img, &preview_16) {
        let t8 = t.to_rgb8();
        if c7.film_type == 1 || c7.film_type == 2 {
            color::extract_film_curve(&t8, p, c7)
        } else {
            None
        }
    } else {
        None
    };
    let after_film_c7 = color::apply_film_processing(&raw_16, c7);

    // M1: 反转 per-channel gamma 方向 (v^gamma 而非 v^(1/gamma))
    // 实现方式: 将 levels_gamma[i] 设为 1/original，使得 v^(1/(1/g)) = v^g
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M1: 反转 per-ch gamma", &to_rgb8(&result), &ref_rgb8);
    }

    // M2: 反转 per-ch gamma + 反转 master gamma
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        let mg = (c7.gamma as f32 - 1.0).max(0.01);
        a.levels_gamma[0] = (1.0 / mg).clamp(0.01, 99.0);
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M2: 反转 per-ch + master gamma", &to_rgb8(&result), &ref_rgb8);
    }

    // M3: 原始 gamma + 禁用 color_corr
    {
        let mut a = build_manual_adjust(c7);
        a.apply_color_corr = false;
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M3: 禁用 color_corr", &to_rgb8(&result), &ref_rgb8);
    }

    // M4: 反转 gamma + 禁用 color_corr
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M4: 反转gamma + 禁用cc", &to_rgb8(&result), &ref_rgb8);
    }

    // M5: 反转 gamma + 禁用 color_corr + 弱化 brightness (×0.25)
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        a.brightness *= 0.25;
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M5: M4 + brightness×0.25", &to_rgb8(&result), &ref_rgb8);
    }

    // M6: 反转 gamma + 禁用 color_corr + 弱化 brightness + 弱化 contrast
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        a.brightness *= 0.25;
        a.contrast *= 0.5;
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M6: M5 + contrast×0.5", &to_rgb8(&result), &ref_rgb8);
    }

    // M7: 反转 gamma + color_corr ÷1000 (而非÷100)
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        // color_corr 保留但需要在 pipeline 中用不同缩放 — 无法通过参数修改
        // 改为禁用 cc 并手动计算效果大致方向
        a.apply_color_corr = false;
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M7: 反转gamma + 无cc (=M4重复,用作基准)", &to_rgb8(&result), &ref_rgb8);
    }

    // M8: 反转 gamma + 禁用 cc + 禁用 curves
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        a.apply_curves = false;
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &identity_curves,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M8: 反转gamma + 无cc + 无curves", &to_rgb8(&result), &ref_rgb8);
    }

    // M9: 反转 gamma + 禁用 cc + 禁用 brightness/contrast/lightness
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        a.brightness = 0.0;
        a.contrast = 0.0;
        a.lightness = 0.0;
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M9: 反转gamma + 无cc/bri/con/lit", &to_rgb8(&result), &ref_rgb8);
    }

    // M10: 仅反转 gamma (保留所有其它原样，包括 cc)
    // 与 H7 (原始) 对比，看 gamma 方向的纯影响
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("M10: 仅反转 per-ch gamma (=M1)", &to_rgb8(&result), &ref_rgb8);
    }

    // 打印曲线控制点
    println!("\n--- 曲线控制点 (Setting#7) ---");
    let ch_names = ["RGB(master)", "R", "G", "B", "C", "M", "Y"];
    for (ci, pts) in curve_points.iter().enumerate() {
        if ci < ch_names.len() {
            let pts_str: Vec<String> = pts.iter().map(|&(x,y,dy)| format!("({},{},{})", x, y, dy)).collect();
            println!("  {}: {}", ch_names[ci], pts_str.join(" "));
        }
    }

    // P 测试: DotColor 公式修正 — 使用 per-channel 值而非 max/min(master, channel)
    println!("\n\n=== DotColor 公式修正测试 ===");

    // P1: 仅修正 DotColor 公式 (per-channel 直接使用)
    {
        let mut a = build_manual_adjust(c7);
        if c7.dot_color.len() >= 14 {
            // 直接使用 per-channel 值，不与 master 做 max/min
            a.output_shadow[0] = 0.0;  // master shadow 清零
            a.output_highlight[0] = 255.0;  // master highlight 满
            // per-channel 保持原样
            a.output_shadow[1] = c7.dot_color[0] as f32;  // R shadow
            a.output_shadow[2] = c7.dot_color[1] as f32;  // G shadow
            a.output_shadow[3] = c7.dot_color[2] as f32;  // B shadow
            a.output_highlight[1] = c7.dot_color[7] as f32;  // R highlight
            a.output_highlight[2] = c7.dot_color[8] as f32;  // G highlight
            a.output_highlight[3] = c7.dot_color[9] as f32;  // B highlight
        }
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("P1: DotColor per-ch 直接 (当前管线)", &to_rgb8(&result), &ref_rgb8);
    }

    // P2: P1 + 反转 gamma
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        if c7.dot_color.len() >= 14 {
            a.output_shadow[0] = 0.0;
            a.output_highlight[0] = 255.0;
            a.output_shadow[1] = c7.dot_color[0] as f32;
            a.output_shadow[2] = c7.dot_color[1] as f32;
            a.output_shadow[3] = c7.dot_color[2] as f32;
            a.output_highlight[1] = c7.dot_color[7] as f32;
            a.output_highlight[2] = c7.dot_color[8] as f32;
            a.output_highlight[3] = c7.dot_color[9] as f32;
        }
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("P2: P1 + 反转gamma", &to_rgb8(&result), &ref_rgb8);
    }

    // P3: P2 + 禁用 cc
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        if c7.dot_color.len() >= 14 {
            a.output_shadow[0] = 0.0;
            a.output_highlight[0] = 255.0;
            a.output_shadow[1] = c7.dot_color[0] as f32;
            a.output_shadow[2] = c7.dot_color[1] as f32;
            a.output_shadow[3] = c7.dot_color[2] as f32;
            a.output_highlight[1] = c7.dot_color[7] as f32;
            a.output_highlight[2] = c7.dot_color[8] as f32;
            a.output_highlight[3] = c7.dot_color[9] as f32;
        }
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("P3: P2 + 禁用cc", &to_rgb8(&result), &ref_rgb8);
    }

    // P4: P3 + brightness×0.25
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        a.brightness *= 0.25;
        if c7.dot_color.len() >= 14 {
            a.output_shadow[0] = 0.0;
            a.output_highlight[0] = 255.0;
            a.output_shadow[1] = c7.dot_color[0] as f32;
            a.output_shadow[2] = c7.dot_color[1] as f32;
            a.output_shadow[3] = c7.dot_color[2] as f32;
            a.output_highlight[1] = c7.dot_color[7] as f32;
            a.output_highlight[2] = c7.dot_color[8] as f32;
            a.output_highlight[3] = c7.dot_color[9] as f32;
        }
        let result = color::apply_color_pipeline(
            after_film_c7.clone(), &a, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("P4: P3 + brightness×0.25", &to_rgb8(&result), &ref_rgb8);
    }

    // P5: DotColor 公式修正 + curves→display→DotColor 管线
    {
        let mut a = build_manual_adjust(c7);
        let oshadow = if c7.dot_color.len() >= 14 {
            [0.0, c7.dot_color[0] as f32, c7.dot_color[1] as f32, c7.dot_color[2] as f32]
        } else { [0.0; 4] };
        let ohigh = if c7.dot_color.len() >= 14 {
            [255.0, c7.dot_color[7] as f32, c7.dot_color[8] as f32, c7.dot_color[9] as f32]
        } else { [255.0; 4] };
        a.output_shadow = [0.0; 4];
        a.output_highlight = [255.0; 4];
        let img = color::apply_scanner_levels(&after_film_c7, &a, film_lut_c7.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        let img = color::apply_gradation_curves(&img, &curve_points);
        let img = color::apply_display_adjust(&img, &a);
        let img = apply_dotcolor_last(&img, &oshadow, &ohigh);
        compare_rgb_images("P5: curves→display→DotColor(per-ch)", &to_rgb8(&img), &ref_rgb8);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Q 测试: 反向求解 — 从参考 TIF 反推各阶段应有的正确值
    // ═══════════════════════════════════════════════════════════════════
    println!("\n\n=== Q: 反向求解测试 ===");
    {
        // 打印 S7 参数摘要
        println!("Setting#7 参数:");
        println!("  film_type={}, gamma={}", c7.film_type, c7.gamma);
        println!("  shadow={:?}", &c7.shadow);
        println!("  highlight={:?}", &c7.highlight);
        println!("  gray={:?}", &c7.gray);
        println!("  saturation={}, contrast={}, brightness={}, lightness={}",
            c7.saturation, c7.contrast, c7.brightness, c7.lightness);
        println!("  dot_color={:?}", &c7.dot_color);
        println!("  color_corr[0..12]={:?}", &c7.color_corr[..12.min(c7.color_corr.len())]);
        println!("  ev={}", c7.ev);

        // 构建 7 通道 curves LUT (256 entries each for 8-bit)
        let curve_luts: Vec<Vec<u8>> = curve_points.iter().map(|pts| {
            let lut = color::build_curve_lut(pts);
            lut.to_vec()
        }).collect();

        // 构建反转 LUT: 对于 master curve + per-channel curve, 找出 output→input 映射
        // curves 应用顺序: master first, then per-channel
        // combined(x) = ch_curve(master_curve(x))
        // 构建组合 LUT
        let mut combined_luts: Vec<Vec<u8>> = Vec::new();
        for ch in 0..3 {
            let master = &curve_luts[0];
            let ch_lut = &curve_luts[ch + 1]; // R=1, G=2, B=3
            let combined: Vec<u8> = (0..256).map(|i| {
                let after_master = master[i] as usize;
                ch_lut[after_master.min(255)]
            }).collect();
            combined_luts.push(combined);
        }

        // 构建反转 LUT (output → input)
        // 由于曲线可能不是单调的，我们使用最近匹配
        let mut inv_luts: Vec<Vec<u8>> = Vec::new();
        for ch in 0..3 {
            let forward = &combined_luts[ch];
            let mut inv = vec![0u8; 256];
            for out in 0..256u32 {
                let mut best_in = 0u8;
                let mut best_diff = 256i32;
                for inp in 0..256u32 {
                    let diff = (forward[inp as usize] as i32 - out as i32).abs();
                    if diff < best_diff {
                        best_diff = diff;
                        best_in = inp as u8;
                    }
                }
                inv[out as usize] = best_in;
            }
            inv_luts.push(inv);
        }

        // 反转参考 TIF: 得到 curves 之前的值
        let ref_img = image::open(&ref_path).unwrap().to_rgb8();
        let (w, h) = (ref_img.width(), ref_img.height());
        let mut pre_curves_ref = vec![0u8; (w * h * 3) as usize];
        for i in 0..(w * h) as usize {
            let px = ref_img.as_raw();
            for ch in 0..3 {
                pre_curves_ref[i * 3 + ch] = inv_luts[ch][px[i * 3 + ch] as usize];
            }
        }

        // 打印 pre-curves 参考值统计
        let mut ch_means = [0.0f64; 3];
        let mut ch_min = [255u8; 3];
        let mut ch_max = [0u8; 3];
        for i in 0..(w * h) as usize {
            for ch in 0..3 {
                let v = pre_curves_ref[i * 3 + ch];
                ch_means[ch] += v as f64;
                ch_min[ch] = ch_min[ch].min(v);
                ch_max[ch] = ch_max[ch].max(v);
            }
        }
        let n = (w * h) as f64;
        println!("参考TIF 反推 pre-curves 值:");
        println!("  R: mean={:.1}, range=[{}-{}]", ch_means[0] / n, ch_min[0], ch_max[0]);
        println!("  G: mean={:.1}, range=[{}-{}]", ch_means[1] / n, ch_min[1], ch_max[1]);
        println!("  B: mean={:.1}, range=[{}-{}]", ch_means[2] / n, ch_min[2], ch_max[2]);

        // 打印 curves 单调性分析
        for ch in 0..3 {
            let f = &combined_luts[ch];
            let mut non_mono = 0;
            for i in 1..256 {
                if f[i] < f[i-1] { non_mono += 1; }
            }
            let ch_name = ["R", "G", "B"][ch];
            println!("  {ch_name} curve: 非单调段数={non_mono}, range=[{}-{}]", f.iter().min().unwrap(), f.iter().max().unwrap());
        }

        // 我们管线的 pre-curves 值 (scanner→ICC→DotColor→sliders, 无curves)
        let a7 = build_manual_adjust(c7);
        let mut a_no_curves = a7.clone();
        a_no_curves.apply_curves = false;
        let result_no_curves = color::apply_color_pipeline(
            after_film_c7.clone(), &a_no_curves, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_pre_curves = to_rgb8(&result_no_curves);

        // 比较
        let pre_ref_img = image::RgbImage::from_raw(w, h, pre_curves_ref.clone()).unwrap();
        let pre_ref_dyn = image::DynamicImage::ImageRgb8(pre_ref_img);
        compare_rgb_images("Q1: 我们的pre-curves vs 参考的pre-curves", &our_pre_curves, &image_to_rgb8(&pre_ref_dyn));

        // 也计算: 无 DotColor、无 cc、无 sliders 的 pre-curves
        let mut a_base = a7.clone();
        a_base.apply_curves = false;
        a_base.apply_contrast = false;
        a_base.apply_brightness = false;
        a_base.apply_shadow_depth = false;
        a_base.apply_color_corr = false;
        a_base.output_shadow = [0.0; 4];
        a_base.output_highlight = [255.0; 4];
        let result_base = color::apply_color_pipeline(
            after_film_c7.clone(), &a_base, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        let our_base = to_rgb8(&result_base);
        compare_rgb_images("Q2: 仅scanner+ICC (无显示调整) vs 参考pre-curves", &our_base, &image_to_rgb8(&pre_ref_dyn));

        // Q3: 仅scanner+ICC+DotColor (无sliders)
        let mut a_dot_only = a7.clone();
        a_dot_only.apply_curves = false;
        a_dot_only.apply_contrast = false;
        a_dot_only.apply_brightness = false;
        a_dot_only.apply_shadow_depth = false;
        a_dot_only.apply_color_corr = false;
        let result_dot = color::apply_color_pipeline(
            after_film_c7.clone(), &a_dot_only, &curve_points,
            film_lut_c7.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb_images("Q3: scanner+ICC+DotColor vs 参考pre-curves", &to_rgb8(&result_dot), &image_to_rgb8(&pre_ref_dyn));
    }

    // ═══════════════════════════════════════════════════════════════════
    // N 测试: 管线顺序变体 — 曲线在 display_adjust 之前，DotColor 在最后
    // ═══════════════════════════════════════════════════════════════════
    println!("\n\n=== 管线顺序变体测试 ===");

    // 辅助函数: 将 DotColor 作为最后一步应用
    fn apply_dotcolor_last(img: &image::DynamicImage, oshadow: &[f32; 4], ohigh: &[f32; 4]) -> image::DynamicImage {
        if let image::DynamicImage::ImageRgb16(ref rgb16) = img {
            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();
            let mut out = vec![0u16; src.len()];
            for ch in 0..3 {
                let lo = oshadow[0].max(oshadow[ch + 1]) / 255.0;
                let hi = ohigh[0].min(ohigh[ch + 1]) / 255.0;
                let range = (hi - lo).max(0.001);
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let i = (y * w as usize + x) * 3 + ch;
                        let v = src[i] as f32 / 65535.0;
                        out[i] = ((lo + v * range).clamp(0.0, 1.0) * 65535.0) as u16;
                    }
                }
            }
            let buf = image::ImageBuffer::from_raw(w, h, out).unwrap();
            image::DynamicImage::ImageRgb16(buf)
        } else {
            img.clone()
        }
    }

    let real_oshadow = build_manual_adjust(c7).output_shadow;
    let real_ohigh = build_manual_adjust(c7).output_highlight;

    // N1: scanner→ICC→curves→display(无DotColor)→DotColor
    {
        let mut a = build_manual_adjust(c7);
        a.output_shadow = [0.0; 4];
        a.output_highlight = [255.0; 4];
        let img = color::apply_scanner_levels(&after_film_c7, &a, film_lut_c7.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        let img = color::apply_gradation_curves(&img, &curve_points);
        let img = color::apply_display_adjust(&img, &a);
        let img = apply_dotcolor_last(&img, &real_oshadow, &real_ohigh);
        compare_rgb_images("N1: curves→display→DotColor", &to_rgb8(&img), &ref_rgb8);
    }

    // N2: N1 + 反转 per-ch gamma
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.output_shadow = [0.0; 4];
        a.output_highlight = [255.0; 4];
        let img = color::apply_scanner_levels(&after_film_c7, &a, film_lut_c7.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        let img = color::apply_gradation_curves(&img, &curve_points);
        let img = color::apply_display_adjust(&img, &a);
        let img = apply_dotcolor_last(&img, &real_oshadow, &real_ohigh);
        compare_rgb_images("N2: N1 + 反转gamma", &to_rgb8(&img), &ref_rgb8);
    }

    // N3: N2 + 禁用 color_corr
    {
        let mut a = build_manual_adjust(c7);
        for i in 1..4 {
            let g = c7.gray[i] as f32 / 128.0;
            a.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        a.apply_color_corr = false;
        a.output_shadow = [0.0; 4];
        a.output_highlight = [255.0; 4];
        let img = color::apply_scanner_levels(&after_film_c7, &a, film_lut_c7.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        let img = color::apply_gradation_curves(&img, &curve_points);
        let img = color::apply_display_adjust(&img, &a);
        let img = apply_dotcolor_last(&img, &real_oshadow, &real_ohigh);
        compare_rgb_images("N3: N2 + 禁用cc", &to_rgb8(&img), &ref_rgb8);
    }

    // N4: 原始gamma + curves→display→DotColor + 禁用cc
    {
        let mut a = build_manual_adjust(c7);
        a.apply_color_corr = false;
        a.output_shadow = [0.0; 4];
        a.output_highlight = [255.0; 4];
        let img = color::apply_scanner_levels(&after_film_c7, &a, film_lut_c7.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        let img = color::apply_gradation_curves(&img, &curve_points);
        let img = color::apply_display_adjust(&img, &a);
        let img = apply_dotcolor_last(&img, &real_oshadow, &real_ohigh);
        compare_rgb_images("N4: 原始gamma + curves→disp→Dot, 无cc", &to_rgb8(&img), &ref_rgb8);
    }

    // N5: scanner→ICC→DotColor→curves (当前顺序但 DotColor 先于 curves)
    {
        let mut a = build_manual_adjust(c7);
        let img = color::apply_scanner_levels(&after_film_c7, &a, film_lut_c7.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        // display_adjust (includes DotColor as step ①)
        let img = color::apply_display_adjust(&img, &a);
        // curves AFTER display_adjust (current order)
        let img = color::apply_gradation_curves(&img, &curve_points);
        compare_rgb_images("N5: display(+Dot)→curves (当前顺序)", &to_rgb8(&img), &ref_rgb8);
    }

    // N6: scanner→ICC→DotColor→curves→brightness/contrast/lightness
    // DotColor before curves, display sliders after curves
    {
        let mut a = build_manual_adjust(c7);
        let saved_bri = a.brightness;
        let saved_con = a.contrast;
        let saved_lit = a.lightness;
        let saved_sat = a.saturation;
        // Phase 1: DotColor only
        a.brightness = 0.0;
        a.contrast = 0.0;
        a.lightness = 0.0;
        a.saturation = 0.0;
        a.apply_color_corr = false;
        let img = color::apply_scanner_levels(&after_film_c7, &a, film_lut_c7.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        let img = color::apply_display_adjust(&img, &a); // only DotColor
        // Phase 2: curves
        let img = color::apply_gradation_curves(&img, &curve_points);
        // Phase 3: brightness/contrast/lightness/saturation
        a.output_shadow = [0.0; 4];
        a.output_highlight = [255.0; 4];
        a.brightness = saved_bri;
        a.contrast = saved_con;
        a.lightness = saved_lit;
        a.saturation = saved_sat;
        let img = color::apply_display_adjust(&img, &a);
        compare_rgb_images("N6: Dot→curves→sliders, 无cc", &to_rgb8(&img), &ref_rgb8);
    }

    // ─── Test H: 尝试不同的编辑历史索引 ───
    println!("\n\n=== 测试不同编辑历史索引 ===");

    // 先打印所有 settings 的关键参数
    println!("\n--- 各 Setting 参数对比 ---");
    for (si, setting) in edit_history.settings.iter().enumerate() {
        let c = &setting.correction;
        println!("  S{}: '{}' ft={} gamma={:.2} shadow={:?} highlight={:?} gray={:?} con={} bri={} lit={} sat={} ev={:.2}",
            si, setting.name, c.film_type, c.gamma,
            &c.shadow, &c.highlight, &c.gray,
            c.contrast, c.brightness, c.lightness, c.saturation, c.ev);
        if !c.dot_color.is_empty() {
            let dc = &c.dot_color;
            let has_dc = dc.iter().any(|&v| v != 0 && v != 255);
            if has_dc {
                println!("       dot=[s:{},{},{},{} h:{},{},{},{}]",
                    dc[0], dc[1], dc[2], dc[3], dc[7], dc[8], dc[9], dc[10]);
            }
        }
    }
    println!();

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
