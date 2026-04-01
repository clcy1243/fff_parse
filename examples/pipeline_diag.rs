//! Pipeline diagnostic: extract ground-truth transfer function from reference TIF
//!
//! Usage: cargo run --release --example pipeline_diag -- <file.fff> <test1.tif> [test1_all_config.tif]
//!
//! Compares inverted raw pixels → reference TIF to find the exact transfer function.

use std::env;
use std::path::Path;

use fff_viewer::color;
use fff_viewer::flexcolor::{self, EditHistory};
use fff_viewer::tiff::TiffFile;

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: cargo run --release --example pipeline_diag -- <file.fff> <test1.tif> [test1_all_config.tif]");
        std::process::exit(1);
    }

    let fff_path = &args[1];
    let ref1_path = &args[2];
    let ref2_path = args.get(3);

    println!("═══ Pipeline Diagnostic ═══");
    println!("FFF: {}", fff_path);
    println!("Ref1 (S1 export): {}", ref1_path);

    // Load reference TIFs
    let ref1_img = image::open(ref1_path).expect("Cannot open ref1 TIF");
    let ref1_8 = ref1_img.to_rgb8();
    let ref2_8 = ref2_path.map(|p| {
        println!("Ref2 (S7 export): {}", p);
        image::open(p).expect("Cannot open ref2 TIF").to_rgb8()
    });

    // Load FFF
    let tiff = TiffFile::open(Path::new(fff_path)).expect("Cannot open FFF");
    let ifd0 = &tiff.ifds[0];
    let raw_16 = tiff.decode_uncompressed_rgb(ifd0).expect("Cannot decode IFD#0");
    println!("Raw: {}×{}", raw_16.width(), raw_16.height());

    // Parse edit history
    let edit_history = EditHistory::parse_from_tiff(&tiff).expect("Cannot parse edit history");
    let current_idx = edit_history.current_index.min(edit_history.settings.len() - 1);
    let c_current = &edit_history.settings[current_idx].correction;
    let c1 = &edit_history.settings[1].correction; // Setting #1 (图像 004)

    println!("\nCurrent setting: #{} '{}'", current_idx, edit_history.settings[current_idx].name);
    println!("S1: '{}', ft={}, gamma={:.4}", edit_history.settings[1].name, c1.film_type, c1.gamma);
    println!("S1 shadow={:?}, highlight={:?}", c1.shadow, c1.highlight);
    println!("S1 gray={:?}, gamma={:.4}", c1.gray, c1.gamma);
    println!("S1 sat={}, con={}, bri={}, lit={}", c1.saturation, c1.contrast, c1.brightness, c1.lightness);

    // ICC
    let all_tags = tiff.all_tags();
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
    // If no embedded ICC, try loading from profiles directory
    let icc_data = if icc_data.is_none() {
        let profile_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("profiles")
            .join("Flextight X5 & 949.icc");
        if profile_path.exists() {
            let data = std::fs::read(&profile_path).ok();
            if let Some(ref d) = data {
                println!("ICC (from disk): {} bytes — {}", d.len(), profile_path.display());
            }
            data
        } else {
            println!("ICC profile not found at {}", profile_path.display());
            None
        }
    } else {
        println!("ICC (embedded): {} bytes", icc_data.as_ref().unwrap().len());
        icc_data
    };

    // Film processing with S1's highlight values
    let after_film = color::apply_film_processing(&raw_16, c1);
    let after_film_16 = after_film.to_rgb16();
    println!("After film_processing: {}×{}", after_film_16.width(), after_film_16.height());

    let (w, h) = (after_film_16.width() as usize, after_film_16.height() as usize);
    let n_pixels = w * h;
    let af_raw = after_film_16.as_raw();
    let r1_raw = ref1_8.as_raw();

    // ═══════════════════════════════════════════════════════
    // Step 1: Extract ground-truth transfer function
    //    inverted_16bit → reference_8bit (the overall mapping)
    // ═══════════════════════════════════════════════════════
    println!("\n═══ Step 1: Ground-truth transfer function (inverted → ref1) ═══");

    const BINS: usize = 256;
    let mut sums = [[0.0f64; BINS]; 3];
    let mut counts = [[0u32; BINS]; 3];

    for y in 0..h {
        for x in 0..w {
            let pi = (y * w + x) * 3;
            for ch in 0..3 {
                // Inverted 16-bit value → bin (0-255 range)
                let inv_val = af_raw[pi + ch] as f32;
                let bin = (inv_val / 65535.0 * 255.0) as usize;
                let bin = bin.min(255);
                let ref_val = r1_raw[pi + ch] as f32;
                sums[ch][bin] += ref_val as f64;
                counts[ch][bin] += 1;
            }
        }
    }

    // Build ground-truth LUT (256 entries, input: inverted_normalized, output: ref TIF value)
    let mut gt_lut = [[0.0f32; BINS]; 3];
    for ch in 0..3 {
        for i in 0..BINS {
            if counts[ch][i] > 0 {
                gt_lut[ch][i] = (sums[ch][i] / counts[ch][i] as f64) as f32;
            }
        }
        // Fill gaps by interpolation
        let mut last_valid = 0;
        let mut first_valid = BINS;
        for i in 0..BINS {
            if counts[ch][i] > 0 {
                if first_valid == BINS { first_valid = i; }
                last_valid = i;
            }
        }
        if first_valid < BINS {
            for i in 0..first_valid { gt_lut[ch][i] = gt_lut[ch][first_valid]; }
            for i in (last_valid + 1)..BINS { gt_lut[ch][i] = gt_lut[ch][last_valid]; }
            let mut prev = first_valid;
            for i in (first_valid + 1)..=last_valid {
                if counts[ch][i] == 0 {
                    // interpolate
                    let mut next = i + 1;
                    while next <= last_valid && counts[ch][next] == 0 { next += 1; }
                    if next <= last_valid {
                        let frac = (i - prev) as f32 / (next - prev) as f32;
                        gt_lut[ch][i] = gt_lut[ch][prev] * (1.0 - frac) + gt_lut[ch][next] * frac;
                    }
                } else {
                    prev = i;
                }
            }
        }
    }

    println!("Ground-truth LUT (inverted_8bit → ref_8bit):");
    for &idx in &[0, 32, 64, 96, 128, 160, 192, 224, 255] {
        println!("  [{:3}] R={:5.1} G={:5.1} B={:5.1} (counts R={:6} G={:6} B={:6})",
            idx, gt_lut[0][idx], gt_lut[1][idx], gt_lut[2][idx],
            counts[0][idx], counts[1][idx], counts[2][idx]);
    }

    // ═══════════════════════════════════════════════════════
    // Step 2: Apply ground-truth LUT and compare
    // ═══════════════════════════════════════════════════════
    println!("\n═══ Step 2: Render with ground-truth LUT ═══");

    let mut gt_out = vec![0u8; n_pixels * 3];
    for i in 0..n_pixels {
        for ch in 0..3 {
            let inv = af_raw[i * 3 + ch] as f32 / 65535.0 * 255.0;
            let lo = (inv as usize).min(254);
            let frac = inv - lo as f32;
            let val = gt_lut[ch][lo] * (1.0 - frac) + gt_lut[ch][lo + 1] * frac;
            gt_out[i * 3 + ch] = val.clamp(0.0, 255.0) as u8;
        }
    }
    let gt_img = image::RgbImage::from_raw(w as u32, h as u32, gt_out).unwrap();
    compare_rgb("GT LUT (inverted → ref) vs ref1", &gt_img, &ref1_8);

    // ═══════════════════════════════════════════════════════
    // Step 3: Compare film curve extractions
    // ═══════════════════════════════════════════════════════
    println!("\n═══ Step 3: Film curve comparison ═══");

    let thumb_pair = tiff.decode_thumbnail_pair();
    let (thumb_img, preview_16) = match thumb_pair {
        Some((t, p)) => (Some(t), Some(p)),
        None => (None, None),
    };

    // Extract with S1's params
    let film_lut_s1 = if let (Some(ref t), Some(ref p)) = (&thumb_img, &preview_16) {
        color::extract_film_curve(t, p, c1)
    } else { None };
    println!("Film LUT S1: {}", if film_lut_s1.is_some() { "extracted" } else { "None" });

    // Extract with current setting params
    let film_lut_current = if let (Some(ref t), Some(ref p)) = (&thumb_img, &preview_16) {
        color::extract_film_curve(t, p, c_current)
    } else { None };
    println!("Film LUT current (S{}): {}", current_idx,
        if film_lut_current.is_some() { "extracted" } else { "None (heavy adj)" });

    // Compare LUT values at key points
    if let Some(ref lut) = film_lut_s1 {
        println!("\nExtracted LUT (S1 params) — values at key points:");
        for &idx in &[0usize, 8192, 16384, 24576, 32768, 40960, 49152, 57344, 65535] {
            println!("  [{:5}] R={:.4} G={:.4} B={:.4}",
                idx, lut[0][idx], lut[1][idx], lut[2][idx]);
        }
    }

    // Compare extracted LUT with hardcoded LUT
    println!("\nHardcoded LUT — values at key points:");
    for &idx8 in &[0, 32, 64, 96, 128, 160, 192, 224, 255] {
        let r = color::FILM_CURVE_LUT_R[idx8] as f32 / 255.0;
        let g = color::FILM_CURVE_LUT_G[idx8] as f32 / 255.0;
        let b = color::FILM_CURVE_LUT_B[idx8] as f32 / 255.0;
        println!("  [{:3}] R={:.4} G={:.4} B={:.4}", idx8, r, g, b);
    }

    // ═══════════════════════════════════════════════════════
    // Step 4: Pipeline step-by-step with S1 params
    // ═══════════════════════════════════════════════════════
    println!("\n═══ Step 4: S1 pipeline step-by-step vs ref1 ═══");

    let mut adj_s1 = build_manual_adjust(c1);
    let identity_curves: Vec<Vec<(i64, i64, i64)>> = (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect();

    // 4a: Film processing only (inverted, no curve/levels/gamma)
    compare_rgb("4a: Film processing only", &to_rgb8(&after_film), &ref1_8);

    // 4b: + scanner_levels with S1's levels + NO film curve
    {
        let mut adj_no_curve = adj_s1.clone();
        adj_no_curve.apply_film_curve = false;
        let img = color::apply_scanner_levels(&after_film, &adj_no_curve, None);
        compare_rgb("4b: + levels/gamma (no film curve)", &to_rgb8(&img), &ref1_8);
    }

    // 4c: + scanner_levels with hardcoded film curve
    {
        let img = color::apply_scanner_levels(&after_film, &adj_s1, None);
        compare_rgb("4c: + levels/gamma + hardcoded curve", &to_rgb8(&img), &ref1_8);
    }

    // 4d: + scanner_levels with extracted film curve (S1)
    {
        let img = color::apply_scanner_levels(&after_film, &adj_s1, film_lut_s1.as_ref());
        compare_rgb("4d: + levels/gamma + extracted S1 curve", &to_rgb8(&img), &ref1_8);
    }

    // 4e: 4d + ICC
    {
        let img = color::apply_scanner_levels(&after_film, &adj_s1, film_lut_s1.as_ref());
        let img = if let Some(icc) = icc_data.as_deref() {
            color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
        } else { img };
        compare_rgb("4e: + ICC", &to_rgb8(&img), &ref1_8);
    }

    // 4f: 4e + display adjust (S1 has only saturation=15)
    {
        let img = color::apply_color_pipeline(
            after_film.clone(), &adj_s1, &identity_curves,
            film_lut_s1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
        );
        compare_rgb("4f: Full S1 pipeline", &to_rgb8(&img), &ref1_8);
    }

    // ═══════════════════════════════════════════════════════
    // Step 5: Test with CORRECTED levels for negative film
    //   For negative film, after normalization:
    //   - shadow in inverted space = 0
    //   - highlight in inverted space = 1 - shadow/highlight
    // ═══════════════════════════════════════════════════════
    println!("\n═══ Step 5: Corrected levels for negative film ═══");

    if c1.film_type == 1 || c1.film_type == 2 {
        let mut adj_fixed = adj_s1.clone();
        // For negative film, fix the levels to account for film_processing normalization
        for ch in 1..4 {
            let shadow = c1.shadow[ch] as f32;
            let highlight = c1.highlight[ch] as f32;
            adj_fixed.levels_black[ch] = 0.0;
            adj_fixed.levels_white[ch] = ((1.0 - shadow / highlight.max(1.0)) * 255.0).clamp(1.0, 255.0);
        }
        adj_fixed.levels_black[0] = 0.0;
        adj_fixed.levels_white[0] = adj_fixed.levels_white[1].max(adj_fixed.levels_white[2]).max(adj_fixed.levels_white[3]);

        println!("Fixed levels_black={:?}", adj_fixed.levels_black);
        println!("Fixed levels_white={:?}", adj_fixed.levels_white);
        println!("Original levels_black={:?}", adj_s1.levels_black);
        println!("Original levels_white={:?}", adj_s1.levels_white);

        // 5a: Fixed levels, no film curve
        {
            let mut adj = adj_fixed.clone();
            adj.apply_film_curve = false;
            let img = color::apply_scanner_levels(&after_film, &adj, None);
            compare_rgb("5a: Fixed levels, no curve", &to_rgb8(&img), &ref1_8);
        }

        // 5b: Fixed levels + hardcoded curve
        {
            let img = color::apply_scanner_levels(&after_film, &adj_fixed, None);
            compare_rgb("5b: Fixed levels + hardcoded curve", &to_rgb8(&img), &ref1_8);
        }

        // 5c: Fixed levels + extracted S1 curve
        {
            let img = color::apply_scanner_levels(&after_film, &adj_fixed, film_lut_s1.as_ref());
            compare_rgb("5c: Fixed levels + extracted S1 curve", &to_rgb8(&img), &ref1_8);
        }

        // 5d: Fixed levels + no curve + ICC
        {
            let mut adj = adj_fixed.clone();
            adj.apply_film_curve = false;
            let img = color::apply_scanner_levels(&after_film, &adj, None);
            let img = if let Some(icc) = icc_data.as_deref() {
                color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
            } else { img };
            compare_rgb("5d: Fixed levels + ICC (no curve)", &to_rgb8(&img), &ref1_8);
        }

        // 5e: Fixed levels, full pipeline
        {
            let img = color::apply_color_pipeline(
                after_film.clone(), &adj_fixed, &identity_curves,
                film_lut_s1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
            );
            compare_rgb("5e: Fixed levels full pipeline", &to_rgb8(&img), &ref1_8);
        }
    }

    // ═══════════════════════════════════════════════════════
    // Step 6: Test WITHOUT film_processing normalization
    //   (remove the 65535/hi scaling, just do hi - raw)
    // ═══════════════════════════════════════════════════════
    println!("\n═══ Step 6: No normalization in film processing ═══");

    if c1.film_type == 1 || c1.film_type == 2 {
        // Manually invert WITHOUT the 65535/hi scaling
        let hi_r = c1.highlight[1] as f32 * 4.0;
        let hi_g = c1.highlight[2] as f32 * 4.0;
        let hi_b = c1.highlight[3] as f32 * 4.0;
        let raw_rgb16 = raw_16.to_rgb16();
        let raw_raw = raw_rgb16.as_raw();
        let mut unnorm_pixels = vec![0u16; (w * h * 3) as usize];
        for i in 0..(w * h) {
            let si = i * 3;
            unnorm_pixels[si] = (hi_r - raw_raw[si] as f32).max(0.0).min(65535.0) as u16;
            unnorm_pixels[si + 1] = (hi_g - raw_raw[si + 1] as f32).max(0.0).min(65535.0) as u16;
            unnorm_pixels[si + 2] = (hi_b - raw_raw[si + 2] as f32).max(0.0).min(65535.0) as u16;
        }
        let unnorm_img = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
            w as u32, h as u32, unnorm_pixels
        ).unwrap();
        let unnorm_dyn = image::DynamicImage::ImageRgb16(unnorm_img);

        // Print value stats
        let un_raw = unnorm_dyn.to_rgb16();
        let un_data = un_raw.as_raw();
        let mut sums_ch = [0.0f64; 3];
        let mut max_ch = [0u16; 3];
        for i in 0..(w * h) {
            for ch in 0..3 {
                let v = un_data[i * 3 + ch];
                sums_ch[ch] += v as f64;
                if v > max_ch[ch] { max_ch[ch] = v; }
            }
        }
        println!("Unnormalized inversion stats:");
        println!("  R: mean={:.0}, max={} (hi_r={})", sums_ch[0] / n_pixels as f64, max_ch[0], hi_r);
        println!("  G: mean={:.0}, max={} (hi_g={})", sums_ch[1] / n_pixels as f64, max_ch[1], hi_g);
        println!("  B: mean={:.0}, max={} (hi_b={})", sums_ch[2] / n_pixels as f64, max_ch[2], hi_b);

        // 6a: Unnormalized + original levels (no curve)
        {
            let mut adj = adj_s1.clone();
            adj.apply_film_curve = false;
            let img = color::apply_scanner_levels(&unnorm_dyn, &adj, None);
            compare_rgb("6a: Unnorm + original levels, no curve", &to_rgb8(&img), &ref1_8);
        }

        // 6b: Unnormalized + original levels + ICC
        {
            let mut adj = adj_s1.clone();
            adj.apply_film_curve = false;
            let img = color::apply_scanner_levels(&unnorm_dyn, &adj, None);
            let img = if let Some(icc) = icc_data.as_deref() {
                color::apply_icc_transform(&img, icc, color::TargetColorSpace::SRGB).unwrap_or(img)
            } else { img };
            compare_rgb("6b: Unnorm + original levels + ICC", &to_rgb8(&img), &ref1_8);
        }

        // 6c: Unnormalized + original levels + ICC + sat
        {
            let mut adj = adj_s1.clone();
            adj.apply_film_curve = false;
            let img = color::apply_color_pipeline(
                unnorm_dyn.clone(), &adj, &identity_curves,
                None, icc_data.as_deref(), color::TargetColorSpace::SRGB,
            );
            compare_rgb("6c: Unnorm + original levels + ICC + sat", &to_rgb8(&img), &ref1_8);
        }
    }

    // ═══════════════════════════════════════════════════════
    // Step 7: Also compare against test1_all_config.tif if provided
    // ═══════════════════════════════════════════════════════
    if let Some(ref ref2) = ref2_8 {
        println!("\n═══ Step 7: Compare vs test1_all_config.tif (S7 export) ═══");

        // H1: S1 rendering vs ref2
        {
            let a = build_manual_adjust(c1);
            let img = color::apply_color_pipeline(
                after_film.clone(), &a, &identity_curves,
                film_lut_s1.as_ref(), icc_data.as_deref(), color::TargetColorSpace::SRGB,
            );
            compare_rgb("H1: S1 pipeline vs ref2", &to_rgb8(&img), ref2);
        }

        // Ref1 vs Ref2 directly
        compare_rgb("Ref1 vs Ref2 (TIF difference)", &ref1_8, ref2);
    }

    println!("\nDone.");
}

// ─── Helper functions ────────────────────────────────────────────────────────

fn build_manual_adjust(corr: &flexcolor::ImageCorrection) -> color::ManualAdjust {
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

    adj.apply_curves = corr.apply_curves && !corr.gradations.is_empty();
    adj
}

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

fn compare_rgb(label: &str, ours: &image::RgbImage, reference: &image::RgbImage) {
    if ours.dimensions() != reference.dimensions() {
        println!("  {} — size mismatch!", label);
        return;
    }
    let n = (ours.width() * ours.height()) as usize;
    let our_raw = ours.as_raw();
    let ref_raw = reference.as_raw();

    let mut mae = [0.0f64; 3];
    let mut mean_ours = [0.0f64; 3];
    let mut mean_ref = [0.0f64; 3];

    for i in 0..n {
        for ch in 0..3 {
            let o = our_raw[i * 3 + ch] as f64;
            let r = ref_raw[i * 3 + ch] as f64;
            mae[ch] += (o - r).abs();
            mean_ours[ch] += o;
            mean_ref[ch] += r;
        }
    }

    let nf = n as f64;
    println!("┌─── {} ───", label);
    println!("│  R: MAE={:6.2}  mean={:5.1}/{:5.1}", mae[0] / nf, mean_ours[0] / nf, mean_ref[0] / nf);
    println!("│  G: MAE={:6.2}  mean={:5.1}/{:5.1}", mae[1] / nf, mean_ours[1] / nf, mean_ref[1] / nf);
    println!("│  B: MAE={:6.2}  mean={:5.1}/{:5.1}", mae[2] / nf, mean_ours[2] / nf, mean_ref[2] / nf);
    let mae_all = (mae[0] + mae[1] + mae[2]) / 3.0 / nf;
    println!("│  All: MAE={:6.2}", mae_all);
    println!("└──────");
}
