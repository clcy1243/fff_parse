//! Focused color pipeline diagnostic for test1_all_config.fff vs test1.tif
//! Tests gamma direction and other pipeline components with Setting #1

use fff_viewer::tiff::TiffFile;
use fff_viewer::flexcolor::{self, EditHistory};
use fff_viewer::color::{self, ManualAdjust, TargetColorSpace};

fn main() {
    let fff_path = "/Users/will/vmwareShare/test_image/test1_all_config.fff";
    let tif_path = "/Users/will/vmwareShare/test_image/test1.tif";
    
    let tiff = TiffFile::open(std::path::Path::new(fff_path)).unwrap();
    let ref_img = image::open(tif_path).unwrap().to_rgb8();
    let history = EditHistory::parse_from_tiff(&tiff).unwrap();
    let corr = &history.settings[1].correction; // Setting #1 '图像 004'
    
    // ICC
    let icc = std::fs::read("profiles/Flextight X5 & 949.icc").ok();
    
    // Film processing (inversion)
    let raw = tiff.decode_preview_image().unwrap();
    let inverted = color::apply_film_processing(&raw, corr);
    
    // Extract film LUT
    let film_lut = if let Some((t8, p16)) = tiff.decode_thumbnail_pair() {
        color::extract_film_curve(&t8, &p16, corr)
    } else { None };
    
    println!("Setting #1: film_type={}, gamma={}, gray={:?}", 
             corr.film_type, corr.gamma, corr.gray);
    println!("  shadow={:?}, highlight={:?}", corr.shadow, corr.highlight);
    println!("  DotColor={:?}", &corr.dot_color);
    println!("  apply_sliders={}, apply_curves={}, apply_cc={}, apply_histogram={}",
             corr.apply_sliders, corr.apply_curves, corr.apply_cc, corr.apply_histogram);
    println!("  con={}, bri={}, lit={}, sat={}, ev={}", 
             corr.contrast, corr.brightness, corr.lightness, corr.saturation, corr.ev);
    println!("  Film LUT: {}", film_lut.is_some());
    println!();
    
    // Build base ManualAdjust from Setting #1
    let build_adj = |corr: &flexcolor::ImageCorrection| -> ManualAdjust {
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
            if (corr.ev - 1.0).abs() > 0.001 { adj.exposure = corr.ev.log2() as f32; }
            adj.contrast = corr.contrast as f32;
            adj.brightness = corr.brightness as f32;
            adj.lightness = corr.lightness as f32;
        }
        if corr.apply_cc && corr.color_corr.len() == 36 {
            let mut arr = [0i64; 36];
            for (i, &v) in corr.color_corr.iter().enumerate() { arr[i] = v; }
            adj.color_corr = arr;
            adj.apply_color_corr = true;
        }
        adj.apply_curves = corr.apply_curves;
        adj
    };
    
    let identity_curves: Vec<Vec<(i64,i64,i64)>> = (0..7).map(|_| vec![(0,0,0),(255,255,0)]).collect();
    
    // Helper: MAE + bias
    let compare = |label: &str, ours: &image::RgbImage, reference: &image::RgbImage| {
        let (w, h) = (ours.width().min(reference.width()), ours.height().min(reference.height()));
        let mut sum = [0f64; 3];
        let mut bias = [0f64; 3];
        let mut cnt = 0u64;
        for y in 0..h { for x in 0..w {
            let o = ours.get_pixel(x, y);
            let r = reference.get_pixel(x, y);
            for ch in 0..3 {
                let d = o[ch] as f64 - r[ch] as f64;
                sum[ch] += d.abs();
                bias[ch] += d;
            }
            cnt += 1;
        }}
        let mae: Vec<f64> = sum.iter().map(|s| s / cnt as f64).collect();
        let b: Vec<f64> = bias.iter().map(|s| s / cnt as f64).collect();
        let all_mae = (mae[0] + mae[1] + mae[2]) / 3.0;
        println!("  {:40} MAE={:6.2} (R={:6.2} G={:6.2} B={:6.2}) Bias=({:+.1},{:+.1},{:+.1})",
                 label, all_mae, mae[0], mae[1], mae[2], b[0], b[1], b[2]);
    };
    
    let to_8 = |img: &image::DynamicImage| -> image::RgbImage {
        match img {
            image::DynamicImage::ImageRgb16(rgb16) => {
                let (w, h) = (rgb16.width(), rgb16.height());
                let mut out = image::RgbImage::new(w, h);
                for y in 0..h { for x in 0..w {
                    let p = rgb16.get_pixel(x, y);
                    out.put_pixel(x, y, image::Rgb([
                        (p[0] >> 8) as u8, (p[1] >> 8) as u8, (p[2] >> 8) as u8
                    ]));
                }}
                out
            },
            other => other.to_rgb8(),
        }
    };
    
    // ═══ Test Suite ═══
    println!("═══ Setting #1 Pipeline Ablation vs test1.tif ═══");
    
    // T1: Full pipeline (current)
    let adj = build_adj(corr);
    let curves = if corr.apply_curves && !corr.gradations.is_empty() { &corr.gradations } else { &identity_curves };
    let result = color::apply_color_pipeline(inverted.clone(), &adj, curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
    compare("T1: Full pipeline (current)", &to_8(&result), &ref_img);
    
    // T2: No display adjustments (scanner_levels + ICC only)
    let mut adj2 = adj.clone();
    adj2.output_shadow = [0.0; 4]; adj2.output_highlight = [255.0; 4];
    adj2.saturation = 0.0; adj2.contrast = 0.0; adj2.brightness = 0.0; adj2.lightness = 0.0;
    adj2.exposure = 0.0; adj2.apply_color_corr = false; adj2.apply_curves = false;
    let result = color::apply_color_pipeline(inverted.clone(), &adj2, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
    compare("T2: Levels+LUT+ICC only (no display adj)", &to_8(&result), &ref_img);
    
    // T3: Same as T2 but with DotColor
    let mut adj3 = adj2.clone();
    adj3.output_shadow = adj.output_shadow; adj3.output_highlight = adj.output_highlight;
    let result = color::apply_color_pipeline(inverted.clone(), &adj3, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
    compare("T3: T2 + DotColor", &to_8(&result), &ref_img);
    
    // T4: Same as T2 but with saturation
    let mut adj4 = adj2.clone();
    adj4.saturation = adj.saturation;
    let result = color::apply_color_pipeline(inverted.clone(), &adj4, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
    compare("T4: T2 + saturation", &to_8(&result), &ref_img);
    
    // T5: T2 + DotColor + saturation (all display from S1)
    let mut adj5 = adj2.clone();
    adj5.output_shadow = adj.output_shadow; adj5.output_highlight = adj.output_highlight;
    adj5.saturation = adj.saturation;
    let result = color::apply_color_pipeline(inverted.clone(), &adj5, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
    compare("T5: T2 + DotColor + saturation", &to_8(&result), &ref_img);
    
    // T6: No ICC
    let result = color::apply_color_pipeline(inverted.clone(), &adj2, &identity_curves, film_lut.as_ref(), None, TargetColorSpace::SRGB);
    compare("T6: Levels+LUT only (no ICC, no display)", &to_8(&result), &ref_img);
    
    // T7: No film LUT
    let mut adj7 = adj2.clone();
    adj7.apply_film_curve = false;
    let result = color::apply_color_pipeline(inverted.clone(), &adj7, &identity_curves, None::<&[Vec<f32>; 3]>, icc.as_deref(), TargetColorSpace::SRGB);
    compare("T7: Levels only (no LUT, no ICC)", &to_8(&result), &ref_img);
    
    // ═══ Gamma direction tests ═══
    println!("\n═══ Gamma Direction Tests ═══");
    
    // G1: Current gamma (gray/128), pow(v, 1/gamma)
    compare("G1: Current gamma formula", &to_8(&{
        let adj = build_adj(corr);
        color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB)
    }), &ref_img);
    
    // G2: Reversed per-ch gamma (128/gray), pow(v, 1/gamma) → effectively pow(v, gray/128)
    {
        let mut adj = build_adj(corr);
        for i in 1..4 {
            let g = corr.gray[i] as f32 / 128.0;
            adj.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        // Keep everything else
        let result = color::apply_color_pipeline(inverted.clone(), &adj, curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("G2: Reversed per-ch gamma + full display", &to_8(&result), &ref_img);
    }
    
    // G3: Reversed per-ch gamma, no display
    {
        let mut adj = adj2.clone();
        for i in 1..4 {
            let g = corr.gray[i] as f32 / 128.0;
            adj.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("G3: Reversed per-ch gamma, no display adj", &to_8(&result), &ref_img);
    }
    
    // G4: Reversed master gamma too (1/gamma instead of gamma-1)
    {
        let mut adj = adj2.clone();
        for i in 1..4 {
            let g = corr.gray[i] as f32 / 128.0;
            adj.levels_gamma[i] = (1.0 / g.max(0.01)).clamp(0.01, 99.0);
        }
        adj.levels_gamma[0] = (1.0 / (corr.gamma as f32).max(0.01)).clamp(0.01, 99.0);
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("G4: Reversed ALL gamma, no display adj", &to_8(&result), &ref_img);
    }
    
    // G5: No gamma at all (gamma=1.0)
    {
        let mut adj = adj2.clone();
        adj.levels_gamma = [1.0; 4];
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("G5: No gamma (all 1.0), no display adj", &to_8(&result), &ref_img);
    }
    
    // G6: master gamma = gamma directly (not gamma-1)
    {
        let mut adj = adj2.clone();
        adj.levels_gamma[0] = corr.gamma as f32;
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("G6: Master gamma=gamma (not gamma-1)", &to_8(&result), &ref_img);
    }
    
    // G7: master gamma = 1/gamma
    {
        let mut adj = adj2.clone();
        adj.levels_gamma[0] = (1.0 / corr.gamma as f32).clamp(0.01, 99.0);
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("G7: Master gamma=1/gamma", &to_8(&result), &ref_img);
    }
    
    // ═══ Film LUT analysis ═══
    println!("\n═══ Film LUT Effect ═══");
    
    // L1: With extracted LUT + gamma + ICC, no display
    {
        let adj = adj2.clone();
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("L1: Extracted LUT + levels + gamma + ICC", &to_8(&result), &ref_img);
    }
    
    // L2: With extracted LUT + gamma, no ICC
    {
        let adj = adj2.clone();
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), None, TargetColorSpace::SRGB);
        compare("L2: Extracted LUT + levels + gamma (no ICC)", &to_8(&result), &ref_img);
    }
    
    // L3: No LUT, levels + gamma + ICC
    {
        let mut adj = adj2.clone();
        adj.apply_film_curve = false;
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, None::<&[Vec<f32>; 3]>, icc.as_deref(), TargetColorSpace::SRGB);
        compare("L3: No LUT, levels + gamma + ICC", &to_8(&result), &ref_img);
    }
    
    // L4: No LUT, no gamma, just levels + ICC 
    {
        let mut adj = adj2.clone();
        adj.apply_film_curve = false;
        adj.levels_gamma = [1.0; 4];
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, None::<&[Vec<f32>; 3]>, icc.as_deref(), TargetColorSpace::SRGB);
        compare("L4: No LUT, no gamma, levels + ICC", &to_8(&result), &ref_img);
    }
    
    // L5: With LUT, no gamma
    {
        let mut adj = adj2.clone();
        adj.levels_gamma = [1.0; 4];
        let result = color::apply_color_pipeline(inverted.clone(), &adj, &identity_curves, film_lut.as_ref(), icc.as_deref(), TargetColorSpace::SRGB);
        compare("L5: Extracted LUT + levels (no gamma) + ICC", &to_8(&result), &ref_img);
    }
    
    // ═══ TIF metadata check ═══
    println!("\n═══ Reference TIF Stats ═══");
    let (w, h) = (ref_img.width(), ref_img.height());
    let mut means = [0f64; 3];
    for y in 0..h { for x in 0..w {
        let p = ref_img.get_pixel(x, y);
        means[0] += p[0] as f64; means[1] += p[1] as f64; means[2] += p[2] as f64;
    }}
    let n = (w * h) as f64;
    println!("  Reference: {}x{}, mean R={:.1} G={:.1} B={:.1}", w, h, means[0]/n, means[1]/n, means[2]/n);
}
