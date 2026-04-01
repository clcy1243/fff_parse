//! B&W rendering diagnostic: compare our pipeline output against FlexColor TIF export
//!
//! Usage: cargo run --release --example bw_diag -- <file.fff> <reference.tif>

use std::path::Path;

use fff_viewer::color::{self, ManualAdjust, TargetColorSpace};
use fff_viewer::flexcolor::{self, EditHistory, ImageCorrection};
use fff_viewer::tiff::TiffFile;

fn build_adj(corr: &ImageCorrection) -> ManualAdjust {
    let mut adj = ManualAdjust::default();
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

fn to_8bit(img: &image::DynamicImage) -> image::RgbImage {
    match img {
        image::DynamicImage::ImageRgb16(img16) => {
            let (w, h) = (img16.width(), img16.height());
            let mut out = image::RgbImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let p = img16.get_pixel(x, y);
                    out.put_pixel(x, y, image::Rgb([
                        (p[0] >> 8) as u8, (p[1] >> 8) as u8, (p[2] >> 8) as u8,
                    ]));
                }
            }
            out
        },
        other => other.to_rgb8(),
    }
}

fn compare(ours: &image::RgbImage, refs: &image::RgbImage, label: &str) {
    let (w, h) = (ours.width().min(refs.width()), ours.height().min(refs.height()));
    let mut ch_mae = [0.0f64; 3];
    let mut ch_bias = [0.0f64; 3];
    let mut max_err = 0u32;
    let mut count = 0u64;
    for y in 0..h {
        for x in 0..w {
            let o = ours.get_pixel(x, y);
            let r = refs.get_pixel(x, y);
            for ch in 0..3 {
                let diff = o[ch] as f64 - r[ch] as f64;
                ch_mae[ch] += diff.abs();
                ch_bias[ch] += diff;
                let ae = diff.abs() as u32;
                if ae > max_err { max_err = ae; }
            }
            count += 1;
        }
    }
    let mae = (ch_mae[0] + ch_mae[1] + ch_mae[2]) / (count as f64 * 3.0);
    let our_mean = ch_bias.iter().map(|b| *b / count as f64).collect::<Vec<_>>();
    println!("  {}: MAE={:.2}, MaxErr={}, Bias=({:+.1},{:+.1},{:+.1})",
        label, mae, max_err, our_mean[0], our_mean[1], our_mean[2]);
}

fn compare_dyn(ours: &image::DynamicImage, refs: &image::RgbImage, label: &str) {
    let o8 = to_8bit(ours);
    compare(&o8, refs, label);
}

fn region_compare(ours: &image::RgbImage, refs: &image::RgbImage) {
    let (w, h) = (ours.width().min(refs.width()), ours.height().min(refs.height()));
    let gw = w / 3;
    let gh = h / 3;
    for gy in 0..3 {
        for gx in 0..3 {
            let x0 = gx * gw;
            let y0 = gy * gh;
            let rw = if gx == 2 { w - x0 } else { gw };
            let rh = if gy == 2 { h - y0 } else { gh };
            let mut sum_err = 0.0f64;
            let mut our_sum = 0.0f64;
            let mut ref_sum = 0.0f64;
            let mut cnt = 0u64;
            for y in y0..y0+rh {
                for x in x0..x0+rw {
                    let o = ours.get_pixel(x, y);
                    let r = refs.get_pixel(x, y);
                    let ov = (o[0] as f64 + o[1] as f64 + o[2] as f64) / 3.0;
                    let rv = (r[0] as f64 + r[1] as f64 + r[2] as f64) / 3.0;
                    sum_err += (ov - rv).abs();
                    our_sum += ov;
                    ref_sum += rv;
                    cnt += 1;
                }
            }
            println!("  Grid[{},{}]: MAE={:.2}, OurMean={:.1}, RefMean={:.1}",
                gx, gy, sum_err / cnt as f64, our_sum / cnt as f64, ref_sum / cnt as f64);
        }
    }
}

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: cargo run --release --example bw_diag -- <file.fff> <reference.tif>");
        std::process::exit(1);
    }
    let fff_path = &args[1];
    let ref_path = &args[2];

    let ref_img = image::open(ref_path).expect("Cannot open reference TIF");
    let ref_8 = ref_img.to_rgb8();
    println!("Reference: {}×{}", ref_8.width(), ref_8.height());

    let tiff = TiffFile::open(Path::new(fff_path)).expect("Cannot open FFF");
    let ifd0 = &tiff.ifds[0];
    let raw_img = tiff.decode_uncompressed_rgb(ifd0).expect("Cannot decode IFD#0");
    let (rw, rh) = (raw_img.width(), raw_img.height());
    println!("Raw: {}×{}", rw, rh);

    let hist = EditHistory::parse_from_tiff(&tiff).expect("Cannot parse edit history");
    let cur_idx = hist.current_index.min(hist.settings.len() - 1);
    let corr = &hist.settings[cur_idx].correction;
    println!("Setting: #{} '{}', FilmType={} ({}), FilmCurve={}, Gamma={:.4}",
        cur_idx + 1, hist.settings[cur_idx].name,
        corr.film_type, flexcolor::film_type_name(corr.film_type),
        corr.film_curve, corr.gamma);
    println!("Highlight={:?}, Shadow={:?}, Gray={:?}", corr.highlight, corr.shadow, corr.gray);
    if corr.dot_color.len() >= 14 {
        println!("DotColor sh=[{},{},{}] hi=[{},{},{}]",
            corr.dot_color[0], corr.dot_color[1], corr.dot_color[2],
            corr.dot_color[7], corr.dot_color[8], corr.dot_color[9]);
    }
    println!("C/B/L={}/{}/{} Sat={} EV={:.3} apply_sliders={} apply_curves={} apply_cc={}",
        corr.contrast, corr.brightness, corr.lightness,
        corr.saturation, corr.ev, corr.apply_sliders, corr.apply_curves, corr.apply_cc);
    if corr.apply_curves && corr.gradations.len() >= 4 {
        for (i, name) in ["RGB", "R", "G", "B"].iter().enumerate() {
            let pts = &corr.gradations[i];
            let is_identity = pts.len() == 2
                && pts[0].0 == 0 && pts[0].1 == 0
                && pts[1].0 == 255 && pts[1].1 == 255;
            if !is_identity {
                println!("  Curve {}: {} pts {:?}", name, pts.len(), pts);
            }
        }
    }

    // Film processing
    let film_img = color::apply_film_processing(&raw_img, corr);

    // Extract film curve
    let extracted_lut = if corr.film_type == 1 || corr.film_type == 2 {
        tiff.decode_thumbnail_pair().and_then(|(t8, p16)| {
            println!("Thumbnail pair: {}×{}", t8.width(), t8.height());
            color::extract_film_curve(&t8, &p16, corr)
        })
    } else { None };
    println!("Extracted LUT: {}", if extracted_lut.is_some() { "yes" } else { "no" });

    let icc_data = std::fs::read("profiles/Flextight X5 & 949.icc").ok();
    let icc_ref = icc_data.as_deref();

    let identity_curves: Vec<Vec<(i64,i64,i64)>> = vec![
        vec![(0,0,0),(255,255,0)]; 7
    ];
    let curves = if corr.apply_curves && corr.gradations.len() >= 7 {
        &corr.gradations
    } else {
        &identity_curves
    };

    // ── Full pipeline ──
    let adj_full = build_adj(corr);
    let result = color::apply_color_pipeline(
        film_img.clone(), &adj_full, curves,
        extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB,
    );
    let result_8 = to_8bit(&result);

    println!("\n═══ Full pipeline vs reference ═══");
    compare(&result_8, &ref_8, "Full pipeline");

    println!("\n═══ 3×3 Grid (full pipeline) ═══");
    region_compare(&result_8, &ref_8);

    // ── Channel analysis ──
    println!("\n═══ Channel analysis ═══");
    {
        let (w, h) = (result_8.width().min(ref_8.width()), result_8.height().min(ref_8.height()));
        let mut r_diff_sq = [0.0f64; 3];
        let mut cnt = 0u64;
        for y in 0..h {
            for x in 0..w {
                let o = result_8.get_pixel(x, y);
                let r = ref_8.get_pixel(x, y);
                for ch in 0..3 {
                    r_diff_sq[ch] += (o[ch] as f64 - r[ch] as f64).powi(2);
                }
                cnt += 1;
            }
        }
        // Check if ref is truly grayscale
        let mut ref_color_diff = 0.0f64;
        for y in 0..h {
            for x in 0..w {
                let r = ref_8.get_pixel(x, y);
                ref_color_diff += (r[0] as f64 - r[1] as f64).abs()
                    + (r[1] as f64 - r[2] as f64).abs();
            }
        }
        let ref_gs = (ref_color_diff / cnt as f64) < 1.0;
        println!("  Ref is grayscale: {} (avg channel diff={:.3})",
            ref_gs, ref_color_diff / cnt as f64);
        // Check if our output is truly grayscale
        let mut our_color_diff = 0.0f64;
        for y in 0..h {
            for x in 0..w {
                let o = result_8.get_pixel(x, y);
                our_color_diff += (o[0] as f64 - o[1] as f64).abs()
                    + (o[1] as f64 - o[2] as f64).abs();
            }
        }
        let our_gs = (our_color_diff / cnt as f64) < 1.0;
        println!("  Our is grayscale: {} (avg channel diff={:.3})",
            our_gs, our_color_diff / cnt as f64);
    }

    // ── Histogram comparison ──
    println!("\n═══ Histogram percentiles ═══");
    {
        let (w, h) = (result_8.width().min(ref_8.width()), result_8.height().min(ref_8.height()));
        let mut hist_ours = [0u64; 256];
        let mut hist_ref = [0u64; 256];
        for y in 0..h {
            for x in 0..w {
                hist_ours[result_8.get_pixel(x, y)[0] as usize] += 1;
                hist_ref[ref_8.get_pixel(x, y)[0] as usize] += 1;
            }
        }
        let total_o: u64 = hist_ours.iter().sum();
        let total_r: u64 = hist_ref.iter().sum();
        for pct in [1, 5, 25, 50, 75, 95, 99] {
            let op = percentile(&hist_ours, total_o, pct);
            let rp = percentile(&hist_ref, total_r, pct);
            println!("  P{:2}: ours={:3}, ref={:3}, diff={:+}", pct, op, rp, op as i32 - rp as i32);
        }
    }

    // ── Ablation tests ──
    println!("\n═══ Ablation tests ═══");

    // A1: Scanner levels only (no display adj)
    let mut a1 = build_adj(corr);
    a1.apply_exposure = false; a1.apply_contrast = false;
    a1.apply_brightness = false; a1.apply_shadow_depth = false;
    a1.apply_saturation = false; a1.apply_color_corr = false;
    a1.apply_curves = false;
    a1.output_shadow = [0.0; 4]; a1.output_highlight = [255.0; 4];
    let r_a1 = color::apply_color_pipeline(
        film_img.clone(), &a1, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a1, &ref_8, "A1: Scanner+ICC only");

    // A1b: No ICC
    let r_a1b = color::apply_color_pipeline(
        film_img.clone(), &a1, &identity_curves, extracted_lut.as_ref(), None, TargetColorSpace::SRGB);
    compare_dyn(&r_a1b, &ref_8, "A1b: Scanner only (no ICC)");

    // A1c: No extracted LUT
    let r_a1c = color::apply_color_pipeline(
        film_img.clone(), &a1, &identity_curves, None, icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a1c, &ref_8, "A1c: Scanner+ICC (hardcoded LUT)");

    // A1d: No LUT at all
    let mut a1d = a1.clone();
    a1d.apply_film_curve = false;
    let r_a1d = color::apply_color_pipeline(
        film_img.clone(), &a1d, &identity_curves, None, icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a1d, &ref_8, "A1d: Scanner+ICC (no LUT)");

    // A2: + DotColor
    let mut a2 = a1.clone();
    if corr.dot_color.len() >= 14 {
        a2.output_shadow = [0.0, corr.dot_color[0] as f32, corr.dot_color[1] as f32, corr.dot_color[2] as f32];
        a2.output_highlight = [255.0, corr.dot_color[7] as f32, corr.dot_color[8] as f32, corr.dot_color[9] as f32];
    }
    let r_a2 = color::apply_color_pipeline(
        film_img.clone(), &a2, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a2, &ref_8, "A2: + DotColor");

    // A3: + exposure
    let mut a3 = a2.clone();
    if corr.apply_sliders { a3.apply_exposure = true; a3.exposure = (corr.ev as f32) - 1.0; }
    let r_a3 = color::apply_color_pipeline(
        film_img.clone(), &a3, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a3, &ref_8, "A3: + Exposure");

    // A4: + contrast
    let mut a4 = a3.clone();
    if corr.apply_sliders { a4.apply_contrast = true; a4.contrast = corr.contrast as f32; }
    let r_a4 = color::apply_color_pipeline(
        film_img.clone(), &a4, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a4, &ref_8, "A4: + Contrast");

    // A5: + brightness
    let mut a5 = a4.clone();
    if corr.apply_sliders { a5.apply_brightness = true; a5.brightness = corr.brightness as f32; }
    let r_a5 = color::apply_color_pipeline(
        film_img.clone(), &a5, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a5, &ref_8, "A5: + Brightness");

    // A6: + lightness
    let mut a6 = a5.clone();
    if corr.apply_sliders { a6.apply_shadow_depth = true; a6.lightness = corr.lightness as f32; }
    let r_a6 = color::apply_color_pipeline(
        film_img.clone(), &a6, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a6, &ref_8, "A6: + Lightness");

    // A7: + saturation
    let mut a7 = a6.clone();
    if corr.apply_sliders { a7.apply_saturation = true; a7.saturation = corr.saturation as f32; }
    let r_a7 = color::apply_color_pipeline(
        film_img.clone(), &a7, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a7, &ref_8, "A7: + Saturation");

    // A8: + CC
    let mut a8 = a7.clone();
    if corr.apply_cc && corr.color_corr.len() == 36 {
        a8.apply_color_corr = true;
        a8.color_corr = corr.color_corr.clone().try_into().unwrap_or([0i64; 36]);
    }
    let r_a8 = color::apply_color_pipeline(
        film_img.clone(), &a8, &identity_curves, extracted_lut.as_ref(), icc_ref, TargetColorSpace::SRGB);
    compare_dyn(&r_a8, &ref_8, "A8: + CC");

    // A9: + curves (= full)
    compare_dyn(&result, &ref_8, "A9: + Curves (full)");

    // ── Ground-truth LUT ──
    println!("\n═══ Ground-truth transfer function ═══");
    let film_16 = match &film_img {
        image::DynamicImage::ImageRgb16(img) => img.clone(),
        _ => panic!("Expected Rgb16"),
    };
    let (w, h) = (film_16.width().min(ref_8.width()), film_16.height().min(ref_8.height()));
    const BINS: usize = 1024;
    let mut gt_sums = [0.0f64; BINS];
    let mut gt_counts = [0u32; BINS];
    for y in 0..h {
        for x in 0..w {
            let inv = film_16.get_pixel(x, y)[0] as usize;
            let ref_val = ref_8.get_pixel(x, y)[0];
            let bin = inv * (BINS - 1) / 65535;
            gt_sums[bin] += ref_val as f64;
            gt_counts[bin] += 1;
        }
    }
    let mut gt_lut = vec![0.0f32; 65536];
    let mut last_valid = 0.0f32;
    for bin in 0..BINS {
        let val = if gt_counts[bin] > 0 {
            last_valid = gt_sums[bin] as f32 / gt_counts[bin] as f32 / 255.0;
            last_valid
        } else { last_valid };
        let start = bin * 65536 / BINS;
        let end = ((bin + 1) * 65536 / BINS).min(65536);
        for i in start..end { gt_lut[i] = val; }
    }
    // Render GT
    let mut gt_mae = 0.0f64;
    let mut gt_count = 0u64;
    for y in 0..h {
        for x in 0..w {
            let inv = film_16.get_pixel(x, y)[0] as usize;
            let val = (gt_lut[inv.min(65535)] * 255.0).clamp(0.0, 255.0) as u8;
            let ref_val = ref_8.get_pixel(x, y)[0];
            gt_mae += (val as f64 - ref_val as f64).abs();
            gt_count += 1;
        }
    }
    println!("  GT LUT MAE: {:.2}", gt_mae / gt_count as f64);

    println!("  GT LUT at key points (inverted → ref):");
    for i in (0..=255).step_by(16) {
        let idx = (i as usize * 65535 / 255).min(65535);
        println!("    inv={:3} → ref={:.1}", i, gt_lut[idx] * 255.0);
    }
}

fn percentile(hist: &[u64; 256], total: u64, pct: u64) -> u8 {
    let target = total * pct / 100;
    let mut cum = 0u64;
    for (i, &c) in hist.iter().enumerate() {
        cum += c;
        if cum >= target { return i as u8; }
    }
    255
}
