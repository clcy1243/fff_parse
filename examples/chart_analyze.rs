//! 色卡分析：按 64 色 × 4 位置读取，计算每色的均值/方差，识别偏差大的色域。
//!
//! 输入：
//!   1. 我们 flex pipeline 的输出（从 FFF 处理）
//!   2. ref TIF（FlexColor 导出）
//!
//! 输出：按偏差排序的 64 色列表 + 位置相关性分析。

use fff_viewer::{color, flexcolor, tiff::TiffFile};

/// 64 色 palette（同 gen_chart_ffcs.rs）
fn build_palette() -> [[u8; 3]; 64] {
    let mut p = [[0u8; 3]; 64];
    for i in 0..16 {
        let v = (i as u32 * 255 / 15) as u8;
        p[i] = [v, v, v];
    }
    let hues: [[u8; 3]; 6] = [
        [255, 0, 0], [0, 255, 0], [0, 0, 255],
        [255, 255, 0], [0, 255, 255], [255, 0, 255],
    ];
    let lums: [u8; 8] = [32, 64, 96, 128, 160, 192, 224, 255];
    let mut idx = 16;
    for hue in &hues {
        for &lum in &lums {
            p[idx] = [
                (hue[0] as u32 * lum as u32 / 255) as u8,
                (hue[1] as u32 * lum as u32 / 255) as u8,
                (hue[2] as u32 * lum as u32 / 255) as u8,
            ];
            idx += 1;
        }
    }
    p
}

fn color_label(i: usize) -> String {
    if i < 16 {
        format!("Gray {}/15", i)
    } else {
        let hue_names = ["R", "G", "B", "Y", "C", "M"];
        let h = (i - 16) / 8;
        let l = (i - 16) % 8;
        format!("{}-lum{}", hue_names[h], l)
    }
}

fn sample_tile_u16(img: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>, x: u32, y: u32, w: u32, h: u32) -> [f64; 3] {
    // 取 tile 中心 50% × 50% 区域均值（避开 10px 边框 + 抗锯齿）
    let mx = w / 4;
    let my = h / 4;
    let x0 = x + mx;
    let y0 = y + my;
    let x1 = (x + w - mx).min(img.width());
    let y1 = (y + h - my).min(img.height());
    let mut sr = 0.0f64; let mut sg = 0.0f64; let mut sb = 0.0f64; let mut n = 0.0;
    for yy in y0..y1 {
        for xx in x0..x1 {
            let p = img.get_pixel(xx, yy);
            sr += p[0] as f64; sg += p[1] as f64; sb += p[2] as f64; n += 1.0;
        }
    }
    [sr / n, sg / n, sb / n]
}

/// 手动 BW 去色：每通道 gamma-2.2 linearize → 简单平均 → gamma-2.2 re-encode。
/// 不使用 ICC primaries 矩阵 —— 假设 FlexColor 对 BW 的 collapse 忽略 primaries。
fn manual_bw_g22(img: &image::DynamicImage) -> image::DynamicImage {
    let rgb16 = img.clone().into_rgb16();
    let (w, h) = (rgb16.width(), rgb16.height());
    let mut raw: Vec<u16> = rgb16.into_raw();
    let max = 65535.0_f64;
    let g = 2.2_f64;
    let inv_g = 1.0 / g;
    for chunk in raw.chunks_exact_mut(3) {
        let r = (chunk[0] as f64 / max).powf(g);
        let g_ = (chunk[1] as f64 / max).powf(g);
        let b = (chunk[2] as f64 / max).powf(g);
        let y = (r + g_ + b) / 3.0;
        let enc = y.powf(inv_g).clamp(0.0, 1.0) * max;
        let v = enc.round() as u16;
        chunk[0] = v; chunk[1] = v; chunk[2] = v;
    }
    image::DynamicImage::ImageRgb16(
        image::ImageBuffer::from_raw(w, h, raw).unwrap())
}

/// FlexColor 的 BW RGB→Gray collapse (FUN_7025b1f0)：BT.601 整数 luma，
/// 工作在 14-bit 空间。我们的 flex pipeline 输出是 u16 全程；先 >>2 到 14-bit，
/// 应用 299/587/114/1000 整数权重，<<2 回 16-bit。
fn bt601_luma(img: &image::DynamicImage, apply_gamma22: bool) -> image::DynamicImage {
    let rgb16 = img.clone().into_rgb16();
    let (w, h) = (rgb16.width(), rgb16.height());
    let mut raw: Vec<u16> = rgb16.into_raw();
    for chunk in raw.chunks_exact_mut(3) {
        let r14 = (chunk[0] >> 2) as i32;
        let g14 = (chunk[1] >> 2) as i32;
        let b14 = (chunk[2] >> 2) as i32;
        let y14 = (r14 * 299 + g14 * 587 + b14 * 114) / 1000;
        let y14 = y14.clamp(0, 16383) as u16;
        let y16 = if apply_gamma22 {
            let y = y14 as f64 / 16383.0;
            (y.powf(1.0 / 2.2) * 65535.0).round() as u16
        } else {
            y14 << 2
        };
        chunk[0] = y16; chunk[1] = y16; chunk[2] = y16;
    }
    image::DynamicImage::ImageRgb16(
        image::ImageBuffer::from_raw(w, h, raw).unwrap())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: chart_analyze <c_xxx_baseline.fff>");
        std::process::exit(1);
    }
    let fff_path = &args[1];
    let ref_path = fff_path.replacen(".fff", ".tif", 1);

    // 加载 FFF → flex pipeline 输出
    let tiff = TiffFile::open(fff_path).unwrap();
    let hist = flexcolor::EditHistory::parse_from_tiff(&tiff).unwrap();
    let corr = &hist.settings[hist.current_index.min(hist.settings.len() - 1)].correction;
    let raw = tiff.decode_uncompressed_rgb(&tiff.ifds[0]).unwrap();
    let our = color::apply_flex_pipeline_no_icc(raw, corr);
    // ICC → 输出空间（BW 用 Hasselblad Gray，其他用 in_icc → ref_icc）
    let all_tags = tiff.all_tags();
    let profiles_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("profiles");
    // FFF 通常无 embedded ICC；优先读 ref TIF 的 embedded profile（= FlexColor 本次
    // 导出实际用的 scanner ICC），再回退到 bundled Flextight X5 & 949。这样
    // in_icc 与 out_icc 字节一致 → `apply_icc_transform_profiles` 走 identity 短路。
    let ref_embedded_icc = {
        let ref_data = std::fs::read(&ref_path).ok();
        ref_data.and_then(|d| {
            TiffFile::parse(&d).ok().and_then(|rt| {
                color::extract_embedded_icc(rt.raw_data(), &rt.all_tags())
            })
        })
    };
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags)
        .or_else(|| ref_embedded_icc.clone())
        .or_else(|| std::fs::read(profiles_dir.join("Flextight X5 & 949.icc")).ok());
    let skip_icc = args.iter().any(|a| a == "--no-icc");
    let skip_usm = args.iter().any(|a| a == "--no-usm");
    // T47 诊断：BW 去色时是否传 scanner ICC 作为 input profile（默认 true = 旧行为）。
    // 假设：flex pipeline 输出已在 sRGB-ish 空间，再次传 scanner ICC 会造成二次线性化。
    let no_scanner_icc = args.iter().any(|a| a == "--no-scanner-icc");
    let bw_manual_g22 = args.iter().any(|a| a == "--bw-manual-g22");
    let bw_bt601 = args.iter().any(|a| a == "--bw-bt601");
    let bw_bt601_g22 = args.iter().any(|a| a == "--bw-bt601-g22");
    let our_final: image::DynamicImage = if skip_icc {
        println!("[diag] 跳过 ICC transform");
        our.clone()
    } else if corr.film_type == 2 {
        // T47: BT.601 整数 luma（FUN_7025b1f0）为默认
        let use_old_icc = args.iter().any(|a| a == "--bw-old-icc");
        if use_old_icc {
            let gray_icc = std::fs::read(profiles_dir.join("Hasselblad Gray.icc")).unwrap();
            let input_icc = if no_scanner_icc { None } else { icc_data.as_deref() };
            println!("[diag] BW 旧 ICC 路径（对比用）");
            color::desaturate_bw_via_gray_icc(&our, input_icc, &gray_icc)
        } else if bw_bt601_g22 {
            println!("[diag] BW BT.601 + gamma-2.2 再编码");
            bt601_luma(&our, true)
        } else if bw_manual_g22 {
            println!("[diag] BW 手动 gamma-2.2");
            manual_bw_g22(&our)
        } else {
            // 默认：BT.601
            let _ = bw_bt601;
            bt601_luma(&our, false)
        }
    } else {
        // 用 ref_icc 作 target（T6 等价路径）
        let ref_data = std::fs::read(&ref_path).unwrap();
        let ref_tiff = TiffFile::parse(&ref_data).unwrap();
        let ref_icc = color::extract_embedded_icc(ref_tiff.raw_data(), &ref_tiff.all_tags());
        // ICC intent / BPC 诊断 flag
        let icc_settings = {
            use color::{IccIntent, IccSettings};
            let intent = if args.iter().any(|a| a == "--icc-rel") { IccIntent::RelativeColorimetric }
                else if args.iter().any(|a| a == "--icc-sat") { IccIntent::Saturation }
                else if args.iter().any(|a| a == "--icc-abs") { IccIntent::AbsoluteColorimetric }
                else { IccIntent::Perceptual };
            let bpc = args.iter().any(|a| a == "--icc-bpc");
            let s = IccSettings { intent, black_point_compensation: bpc };
            println!("[diag] ICC intent={:?} bpc={}", intent, bpc);
            s
        };
        if let (Some(in_icc), Some(out_icc)) = (&icc_data, &ref_icc) {
            color::apply_icc_transform_profiles(&our, in_icc, out_icc, icc_settings).unwrap_or(our)
        } else if let Some(in_icc) = &icc_data {
            color::apply_icc_transform_ex(&our, in_icc, color::TargetColorSpace::SRGB, icc_settings).unwrap_or(our)
        } else {
            our
        }
    };
    // 可选跳过 USM
    let our_final2 = if skip_usm {
        println!("[diag] 跳过 USM");
        our_final
    } else {
        use fff_viewer::color::ManualAdjust;
        let mut adj = ManualAdjust::default();
        adj.apply_usm = corr.apply_usm;
        adj.usm_amount = corr.usm_amount;
        adj.usm_radius = corr.usm_radius;
        adj.usm_dark_limit = corr.usm_dark_limit;
        adj.usm_noise_limit = corr.usm_noise_limit;
        color::apply_usm(&our_final, &adj)
    };
    let our_rgb = our_final2.into_rgb16();

    // 加载 ref TIF
    let ref_img = image::open(&ref_path).unwrap().into_rgb16();

    println!("Our:  {}×{} {:?}", our_rgb.width(), our_rgb.height(), our_rgb.dimensions());
    println!("Ref:  {}×{}", ref_img.width(), ref_img.height());

    // 色卡布局（匹配 gen_chart_ffcs.rs）
    let w = our_rgb.width() as u32;
    let h = our_rgb.height() as u32;
    let qw = w / 2;
    let qh = h / 2;
    let tw = qw / 8;
    let th = qh / 8;

    let palette = build_palette();
    // 每色 4 个象限位置
    let quad_permutes: &[(u32, u32, fn(usize) -> (usize, usize))] = &[
        (0, 0, |i| (i / 8, i % 8)),
        (qw as usize as u32, 0, |i| (i / 8, 7 - (i % 8))),
        (0, qh as usize as u32, |i| (7 - (i / 8), i % 8)),
        (qw, qh, |i| (7 - (i / 8), 7 - (i % 8))),
    ];

    let mut results: Vec<(usize, [u8; 3], [f64; 3], [f64; 3], f64, f64)> = Vec::with_capacity(64);
    for i in 0..64 {
        let mut ours_samples = [0.0f64; 3];
        let mut ref_samples = [0.0f64; 3];
        for (qx, qy, permute) in quad_permutes {
            let (row, col) = permute(i);
            let x = qx + (col as u32) * tw;
            let y = qy + (row as u32) * th;
            let our_avg = sample_tile_u16(&our_rgb, x, y, tw, th);
            let ref_avg = sample_tile_u16(&ref_img, x, y, tw, th);
            for c in 0..3 { ours_samples[c] += our_avg[c] / 4.0; ref_samples[c] += ref_avg[c] / 4.0; }
        }
        let delta: [f64; 3] = [
            ours_samples[0] - ref_samples[0],
            ours_samples[1] - ref_samples[1],
            ours_samples[2] - ref_samples[2],
        ];
        let mag = ((delta[0].powi(2) + delta[1].powi(2) + delta[2].powi(2)) / 3.0).sqrt();
        let signed = (delta[0] + delta[1] + delta[2]) / 3.0;
        results.push((i, palette[i], ours_samples, ref_samples, mag, signed));
    }
    results.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap());

    println!("\n=== 偏差最大的 20 色（按 RMS 排序）===");
    println!("{:3} {:15} {:20} {:20} {:20} {:>8} {:>8}",
        "idx", "label", "ref input (byte)", "ref out (byte)", "ours out (byte)", "rms_16", "signed");
    for (i, rgb, ours, r, mag, sig) in results.iter().take(20) {
        println!("{:3} {:15} R={:3} G={:3} B={:3}          R={:5.0} G={:5.0} B={:5.0}    R={:5.0} G={:5.0} B={:5.0}    {:8.1} {:+8.1}",
            i, color_label(*i),
            rgb[0], rgb[1], rgb[2],
            r[0]/257.0, r[1]/257.0, r[2]/257.0,
            ours[0]/257.0, ours[1]/257.0, ours[2]/257.0,
            mag, sig
        );
    }

    // 分组统计
    let mut gray_mag = 0.0; let mut gray_n = 0;
    let mut hue_mag = [0.0; 6]; let mut hue_n = [0; 6];
    for (i, _, _, _, mag, _) in &results {
        if *i < 16 { gray_mag += mag; gray_n += 1; }
        else { let h = (i - 16) / 8; hue_mag[h] += mag; hue_n[h] += 1; }
    }
    println!("\n=== 分组 RMS ===");
    println!("Gray:    avg={:.1} (n={})", gray_mag / gray_n as f64, gray_n);
    for (h, name) in ["R","G","B","Y","C","M"].iter().enumerate() {
        if hue_n[h] > 0 {
            println!("{}:      avg={:.1} (n={})", name, hue_mag[h] / hue_n[h] as f64, hue_n[h]);
        }
    }
}

