//! FFF Pipeline Test & Compare Tool (v2)
//!
//! Compares our color pipeline output against FlexColor-exported reference TIFs.
//!
//! Usage:
//!   cargo run --release --example tif_compare -- <file.fff> <reference.tif> [-v] [--json] [--setting N]
//!   cargo run --release --example tif_compare -- --dir <path> [-v] [--json]

use std::env;
use std::path::{Path, PathBuf};

use fff_viewer::color;
use fff_viewer::flexcolor::{EditHistory, ImageCorrection};
use fff_viewer::tiff::TiffFile;

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq)]
enum Grade { Pass, Warn, Fail }

impl Grade {
    fn icon(&self) -> &'static str {
        match self { Self::Pass => "✅", Self::Warn => "⚠️ ", Self::Fail => "❌" }
    }
    fn label(&self) -> &'static str {
        match self { Self::Pass => "PASS", Self::Warn => "WARN", Self::Fail => "FAIL" }
    }
}

#[derive(Clone)]
struct ChannelStats {
    mae_16: f64,
    mae_8: f64,
    p95_8: u32,
    p99_8: u32,
    max_err_8: u32,
    psnr_8: f64,
}

struct TestResult {
    id: String,
    name: String,
    ch_r: ChannelStats,
    ch_g: ChannelStats,
    ch_b: ChannelStats,
    ch_all: ChannelStats,
    delta_e: f64,
    grade: Grade,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Statistics Computation (16-bit + 8-bit dual precision)
// ═══════════════════════════════════════════════════════════════════════════════

struct Accumulator {
    sum_abs_16: u64,
    sum_abs_8: u64,
    sum_sq_8: f64,
    max_8: u32,
    hist_8: [u64; 256],
    count: u64,
}

impl Accumulator {
    fn new() -> Self {
        Self { sum_abs_16: 0, sum_abs_8: 0, sum_sq_8: 0.0, max_8: 0, hist_8: [0; 256], count: 0 }
    }

    fn add(&mut self, ours_16: u16, ref_16: u16) {
        let d16 = (ours_16 as i32 - ref_16 as i32).unsigned_abs();
        let o8 = (ours_16 >> 8) as u8;
        let r8 = (ref_16 >> 8) as u8;
        let d8 = (o8 as i32 - r8 as i32).unsigned_abs();

        self.sum_abs_16 += d16 as u64;
        self.sum_abs_8 += d8 as u64;
        self.sum_sq_8 += (d8 as f64) * (d8 as f64);
        if d8 > self.max_8 { self.max_8 = d8; }
        self.hist_8[d8.min(255) as usize] += 1;
        self.count += 1;
    }

    fn to_stats(&self) -> ChannelStats {
        let n = self.count.max(1) as f64;
        let mse = self.sum_sq_8 / n;
        ChannelStats {
            mae_16: self.sum_abs_16 as f64 / n,
            mae_8: self.sum_abs_8 as f64 / n,
            p95_8: percentile(&self.hist_8, self.count, 95.0),
            p99_8: percentile(&self.hist_8, self.count, 99.0),
            max_err_8: self.max_8,
            psnr_8: if mse > 0.0 { 10.0 * (255.0f64 * 255.0 / mse).log10() } else { f64::INFINITY },
        }
    }
}

fn percentile(hist: &[u64; 256], total: u64, pct: f64) -> u32 {
    let target = (total as f64 * pct / 100.0).ceil() as u64;
    let mut cum = 0u64;
    for (val, &count) in hist.iter().enumerate() {
        cum += count;
        if cum >= target { return val as u32; }
    }
    255
}

fn grade_stats(s: &ChannelStats) -> Grade {
    if s.mae_8 <= 5.0 && s.p99_8 <= 20 { Grade::Pass }
    else if s.mae_8 <= 10.0 && s.p99_8 <= 40 { Grade::Warn }
    else { Grade::Fail }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ΔE76 (CIE L*a*b* Euclidean distance)
// ═══════════════════════════════════════════════════════════════════════════════

fn srgb_to_lab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let lin = |v: f32| -> f32 {
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (rl, gl, bl) = (lin(r), lin(g), lin(b));

    let x = 0.4124564 * rl + 0.3575761 * gl + 0.1804375 * bl;
    let y = 0.2126729 * rl + 0.7151522 * gl + 0.0721750 * bl;
    let z = 0.0193339 * rl + 0.1191920 * gl + 0.9503041 * bl;

    let f = |t: f32| -> f32 {
        if t > 0.008856 { t.cbrt() } else { 7.787037 * t + 16.0 / 116.0 }
    };
    let l = 116.0 * f(y) - 16.0;
    let a = 500.0 * (f(x / 0.95047) - f(y));
    let b_val = 200.0 * (f(y) - f(z / 1.08883));
    (l, a, b_val)
}

fn mean_delta_e(ours: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>, reference: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>) -> f64 {
    use rayon::prelude::*;
    let o = ours.as_raw();
    let r = reference.as_raw();
    let n = o.len() / 3;

    let sum: f64 = (0..n).into_par_iter().map(|i| {
        let j = i * 3;
        let (ol, oa, ob) = srgb_to_lab(
            o[j] as f32 / 65535.0, o[j+1] as f32 / 65535.0, o[j+2] as f32 / 65535.0);
        let (rl, ra, rb) = srgb_to_lab(
            r[j] as f32 / 65535.0, r[j+1] as f32 / 65535.0, r[j+2] as f32 / 65535.0);
        ((ol-rl).powi(2) + (oa-ra).powi(2) + (ob-rb).powi(2)).sqrt() as f64
    }).sum();

    sum / n as f64
}

// ═══════════════════════════════════════════════════════════════════════════════
// Image Comparison
// ═══════════════════════════════════════════════════════════════════════════════

fn compare_images(id: &str, name: &str, ours: &image::DynamicImage, reference: &image::DynamicImage) -> TestResult {
    let ours_16 = ours.to_rgb16();
    let ref_16 = reference.to_rgb16();
    assert_eq!(ours_16.dimensions(), ref_16.dimensions(),
        "size mismatch: ours {:?} vs ref {:?}", ours_16.dimensions(), ref_16.dimensions());

    let o = ours_16.as_raw();
    let r = ref_16.as_raw();
    let n = o.len();

    // Single-pass: accumulate R/G/B/All channel stats
    let mut acc = [Accumulator::new(), Accumulator::new(), Accumulator::new(), Accumulator::new()];
    for i in 0..n {
        let ch = i % 3;
        acc[ch].add(o[i], r[i]);
        acc[3].add(o[i], r[i]);
    }

    let ch_all = acc[3].to_stats();
    let grade = grade_stats(&ch_all);
    let delta_e = mean_delta_e(&ours_16, &ref_16);

    TestResult {
        id: id.to_string(), name: name.to_string(),
        ch_r: acc[0].to_stats(), ch_g: acc[1].to_stats(), ch_b: acc[2].to_stats(),
        ch_all, delta_e, grade,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ManualAdjust Builder (from plist ImageCorrection → pipeline params)
// ═══════════════════════════════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════════════════════════════
// Test Runner
// ═══════════════════════════════════════════════════════════════════════════════

fn identity_curves() -> Vec<Vec<(i64, i64, i64)>> {
    (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect()
}

fn run_tests(
    raw_16: &image::DynamicImage,
    reference: &image::DynamicImage,
    edit_history: &EditHistory,
    setting_idx: usize,
    icc_data: Option<&[u8]>,
    film_lut: Option<&[Vec<f32>; 3]>,
) -> Vec<TestResult> {
    let corr = &edit_history.settings[setting_idx].correction;
    let adj = build_manual_adjust(corr);
    let id_curves = identity_curves();
    let after_film = color::apply_film_processing(raw_16, corr);

    let mut results = Vec::new();

    // ─── Group 1: Full pipeline variants (Setting vs Reference) ─────────

    // T1: Full pipeline with extracted LUT + ICC
    {
        let r = color::apply_color_pipeline(
            after_film.clone(), &adj, &id_curves,
            film_lut, icc_data, color::TargetColorSpace::SRGB);
        results.push(compare_images("T1", "完整管线 (提取LUT+ICC)", &r, reference));
    }

    // T2: Extracted LUT, no ICC
    {
        let r = color::apply_color_pipeline(
            after_film.clone(), &adj, &id_curves,
            film_lut, None, color::TargetColorSpace::SRGB);
        results.push(compare_images("T2", "提取LUT, 无ICC", &r, reference));
    }

    // T3: Hardcoded LUT + ICC
    {
        let r = color::apply_color_pipeline(
            after_film.clone(), &adj, &id_curves,
            None, icc_data, color::TargetColorSpace::SRGB);
        results.push(compare_images("T3", "硬编码LUT + ICC", &r, reference));
    }

    // T4: No film curve at all
    {
        let mut adj_no_fc = adj.clone();
        adj_no_fc.apply_film_curve = false;
        let r = color::apply_color_pipeline(
            after_film.clone(), &adj_no_fc, &id_curves,
            None, icc_data, color::TargetColorSpace::SRGB);
        results.push(compare_images("T4", "无胶片曲线", &r, reference));
    }

    // ─── Group 2: Pipeline stage diagnostics ────────────────────────────

    // T5: Scanner levels only (levels + film_curve + gamma)
    let stage_scanner = color::apply_scanner_levels(&after_film, &adj, film_lut);
    results.push(compare_images("T5", "阶段: scanner levels", &stage_scanner, reference));

    // T6: + ICC transform
    let stage_icc = if let Some(icc) = icc_data {
        match color::apply_icc_transform(&stage_scanner, icc, color::TargetColorSpace::SRGB) {
            Ok(t) => t,
            Err(_) => stage_scanner.clone(),
        }
    } else {
        stage_scanner.clone()
    };
    results.push(compare_images("T6", "阶段: + ICC", &stage_icc, reference));

    // T7: + Display adjust (saturation/brightness/contrast/etc)
    let stage_display = color::apply_display_adjust(&stage_icc, &adj);
    results.push(compare_images("T7", "阶段: + display adjust", &stage_display, reference));

    // T8: + Gradation curves (= full pipeline)
    let user_curves = if adj.apply_curves && !corr.gradations.is_empty() {
        corr.gradations.clone()
    } else {
        id_curves.clone()
    };
    let curves_are_identity = user_curves.iter().all(|pts| {
        pts.len() == 2 && pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
    });
    let stage_final = if adj.apply_curves && user_curves.len() >= 7 && !curves_are_identity {
        color::apply_gradation_curves(&stage_display, &user_curves)
    } else {
        stage_display.clone()
    };
    results.push(compare_images("T8", "阶段: + curves (完整)", &stage_final, reference));

    results
}

// ═══════════════════════════════════════════════════════════════════════════════
// Output Formatting
// ═══════════════════════════════════════════════════════════════════════════════

fn count_grades(results: &[TestResult]) -> (usize, usize, usize) {
    results.iter().fold((0, 0, 0), |(p, w, f), r| match r.grade {
        Grade::Pass => (p + 1, w, f),
        Grade::Warn => (p, w + 1, f),
        Grade::Fail => (p, w, f + 1),
    })
}

fn print_summary(results: &[TestResult]) {
    let (pass, warn, fail) = count_grades(results);
    println!("\n  SUMMARY: {} PASS / {} WARN / {} FAIL ({} tests)\n",
        pass, warn, fail, results.len());

    println!("  {:>3} {:<30} {:>7} {:>5} {:>5} {:>5} {:>7} {:>5}",
        "ID", "Name", "MAE(8)", "P95", "P99", "Max", "ΔE76", "");
    println!("  ─── ────────────────────────────── ─────── ───── ───── ───── ─────── ─────");

    for r in results {
        let s = &r.ch_all;
        println!("  {} {:>3} {:<30} {:>7.2} {:>5} {:>5} {:>5} {:>7.2} {}",
            r.grade.icon(), r.id, r.name,
            s.mae_8, s.p95_8, s.p99_8, s.max_err_8,
            r.delta_e, r.grade.label());
    }
}

fn print_detail(results: &[TestResult]) {
    for r in results {
        println!("\n  ┌─── {} {} ─── [{}]", r.id, r.name, r.grade.label());
        println!("  │  {:>4} │ {:>10} │ {:>8} │ {:>5} │ {:>5} │ {:>5} │ {:>8}",
            "Chan", "MAE(16b)", "MAE(8b)", "P95", "P99", "Max", "PSNR");
        println!("  │  ─────┼────────────┼──────────┼───────┼───────┼───────┼─────────");
        for (name, ch) in [("R", &r.ch_r), ("G", &r.ch_g), ("B", &r.ch_b), ("All", &r.ch_all)] {
            println!("  │  {:>4} │ {:>10.1} │ {:>8.2} │ {:>5} │ {:>5} │ {:>5} │ {:>7.1}dB",
                name, ch.mae_16, ch.mae_8, ch.p95_8, ch.p99_8, ch.max_err_8, ch.psnr_8);
        }
        println!("  │  ΔE76(mean) = {:.2}", r.delta_e);
        println!("  └────────────────────────────────────────────");
    }
}

fn print_json(results: &[TestResult]) {
    println!("[");
    for (i, r) in results.iter().enumerate() {
        let s = &r.ch_all;
        let comma = if i + 1 < results.len() { "," } else { "" };
        println!("  {{\"id\":\"{}\",\"name\":\"{}\",\"mae_8\":{:.2},\"mae_16\":{:.1},\
            \"p95\":{},\"p99\":{},\"max\":{},\"psnr\":{:.1},\
            \"delta_e\":{:.2},\"grade\":\"{}\",\
            \"channels\":{{\"r\":{{\"mae_8\":{:.2},\"p99\":{}}},\
            \"g\":{{\"mae_8\":{:.2},\"p99\":{}}},\
            \"b\":{{\"mae_8\":{:.2},\"p99\":{}}}}}}}{}",
            r.id, r.name, s.mae_8, s.mae_16,
            s.p95_8, s.p99_8, s.max_err_8, s.psnr_8,
            r.delta_e, r.grade.label(),
            r.ch_r.mae_8, r.ch_r.p99_8,
            r.ch_g.mae_8, r.ch_g.p99_8,
            r.ch_b.mae_8, r.ch_b.p99_8,
            comma);
    }
    println!("]");
}

// ═══════════════════════════════════════════════════════════════════════════════
// File Processing
// ═══════════════════════════════════════════════════════════════════════════════

fn find_file_pairs(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut pairs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let fff_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |e| e.eq_ignore_ascii_case("fff")))
            .collect();
        for fff in &fff_files {
            let tif = fff.with_extension("tif");
            if tif.exists() {
                pairs.push((fff.clone(), tif));
            }
        }
    }
    pairs.sort();
    pairs
}

struct FileContext {
    raw_16: image::DynamicImage,
    reference: image::DynamicImage,
    edit_history: EditHistory,
    icc_data: Option<Vec<u8>>,
    film_lut: Option<[Vec<f32>; 3]>,
    setting_idx: usize,
}

fn load_file_pair(fff_path: &Path, ref_path: &Path, setting_override: Option<usize>) -> Result<FileContext, String> {
    let reference = image::open(ref_path).map_err(|e| format!("Cannot open reference TIF: {}", e))?;
    let tiff = TiffFile::open(fff_path).map_err(|e| format!("Cannot open FFF file: {}", e))?;

    let ifd0 = &tiff.ifds[0];
    let raw_16 = tiff.decode_uncompressed_rgb(ifd0).ok_or_else(|| "Cannot decode IFD#0".to_string())?;

    let edit_history = EditHistory::parse_from_tiff(&tiff).ok_or_else(|| "Cannot parse edit history".to_string())?;
    let setting_idx = setting_override.unwrap_or_else(|| {
        if edit_history.settings.len() > 1 { 1 } else { 0 }
    });
    let setting_idx = setting_idx.min(edit_history.settings.len() - 1);
    let corr = &edit_history.settings[setting_idx].correction;

    // ICC profile
    let all_tags = tiff.all_tags();
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags).or_else(|| {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("profiles")
            .join("Flextight X5 & 949.icc");
        std::fs::read(&path).ok()
    });

    // Film LUT extraction from thumbnail/preview
    let thumb_img: Option<image::DynamicImage> = tiff.decode_thumbnail();
    let preview_16: Option<image::ImageBuffer<image::Rgb<u16>, Vec<u16>>> = tiff.ifds.get(2)
        .and_then(|ifd| tiff.decode_uncompressed_rgb(ifd))
        .map(|img: image::DynamicImage| img.to_rgb16());
    let film_lut = if let (Some(ref t), Some(ref p)) = (&thumb_img, &preview_16) {
        let t8: image::RgbImage = t.to_rgb8();
        if corr.film_type == 1 || corr.film_type == 2 {
            color::extract_film_curve(&t8, p, corr)
        } else { None }
    } else { None };

    Ok(FileContext { raw_16, reference, edit_history, icc_data, film_lut, setting_idx })
}

fn process_file_pair(
    fff_path: &Path, ref_path: &Path,
    setting_override: Option<usize>,
    verbose: bool, json_output: bool,
) -> Option<Vec<TestResult>> {
    let ctx = match load_file_pair(fff_path, ref_path, setting_override) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  ⚠ Skipping {}: {}", fff_path.file_name().unwrap_or_default().to_string_lossy(), e);
            return None;
        }
    };
    let sname = &ctx.edit_history.settings[ctx.setting_idx].name;
    let corr = &ctx.edit_history.settings[ctx.setting_idx].correction;

    if !json_output {
        println!("\n═══════════════════════════════════════════════════");
        println!("  FFF Pipeline Test Report");
        println!("  File: {}", fff_path.file_name().unwrap_or_default().to_string_lossy());
        println!("  Ref:  {}", ref_path.file_name().unwrap_or_default().to_string_lossy());
        println!("  Setting #{} \"{}\" (film_type={}, γ={:.2})",
            ctx.setting_idx, sname, corr.film_type, corr.gamma);
        println!("  Film LUT: {}", if ctx.film_lut.is_some() { "extracted" } else { "hardcoded" });
        println!("  ICC: {}", if ctx.icc_data.is_some() { "yes" } else { "no" });
        println!("═══════════════════════════════════════════════════");
    }

    let results = run_tests(
        &ctx.raw_16, &ctx.reference, &ctx.edit_history,
        ctx.setting_idx, ctx.icc_data.as_deref(), ctx.film_lut.as_ref(),
    );

    if json_output {
        print_json(&results);
    } else {
        print_summary(&results);
        if verbose {
            print_detail(&results);
        }
    }

    Some(results)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Main
// ═══════════════════════════════════════════════════════════════════════════════

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();

    // Parse arguments
    let mut verbose = false;
    let mut json_output = false;
    let mut dir_path: Option<String> = None;
    let mut setting_override: Option<usize> = None;
    let mut positional = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--verbose" => verbose = true,
            "--json" => json_output = true,
            "--dir" => { i += 1; dir_path = args.get(i).cloned(); }
            "--setting" => { i += 1; setting_override = args.get(i).and_then(|s| s.parse().ok()); }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    let pairs: Vec<(PathBuf, PathBuf)> = if let Some(ref dir) = dir_path {
        let pairs = find_file_pairs(Path::new(dir));
        if pairs.is_empty() {
            eprintln!("No .fff + .tif pairs found in {}", dir);
            std::process::exit(1);
        }
        pairs
    } else if positional.len() >= 2 {
        vec![(PathBuf::from(&positional[0]), PathBuf::from(&positional[1]))]
    } else {
        eprintln!("Usage: tif_compare <file.fff> <ref.tif> [-v] [--json] [--setting N]");
        eprintln!("       tif_compare --dir <path> [-v] [--json]");
        std::process::exit(1);
    };

    let mut all_results: Vec<(String, Vec<TestResult>)> = Vec::new();

    for (fff, tif) in &pairs {
        if let Some(results) = process_file_pair(fff, tif, setting_override, verbose, json_output) {
            all_results.push((fff.file_name().unwrap_or_default().to_string_lossy().into_owned(), results));
        }
    }

    // Grand summary for multi-file mode
    if all_results.len() > 1 && !json_output {
        println!("\n\n═══════════════════════════════════════════════════════════════════");
        println!("  GRAND SUMMARY");
        println!("═══════════════════════════════════════════════════════════════════");
        println!("  {:<35} {:>5} {:>5} {:>5} {:>5} {:>8}",
            "FILE", "TESTS", "PASS", "WARN", "FAIL", "BEST_MAE");
        println!("  ─────────────────────────────────── ───── ───── ───── ───── ────────");
        let (mut tp, mut tw, mut tf, mut tt) = (0, 0, 0, 0);
        for (name, results) in &all_results {
            let (p, w, f) = count_grades(results);
            let best = results.iter().map(|r| r.ch_all.mae_8)
                .min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(f64::NAN);
            println!("  {:<35} {:>5} {:>5} {:>5} {:>5} {:>8.2}",
                name, results.len(), p, w, f, best);
            tp += p; tw += w; tf += f; tt += results.len();
        }
        println!("  ─────────────────────────────────── ───── ───── ───── ───── ────────");
        println!("  {:<35} {:>5} {:>5} {:>5} {:>5}", "TOTAL", tt, tp, tw, tf);
    }

    // Exit with error if any test failed
    let any_fail = all_results.iter().any(|(_, rs)| rs.iter().any(|r| r.grade == Grade::Fail));
    if any_fail {
        std::process::exit(1);
    }
}