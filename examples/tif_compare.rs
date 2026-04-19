//! FFF Pipeline Test & Compare Tool (v3)
//!
//! 对比我们的管线输出与 FlexColor 导出的参考 TIF，用于逐像素精度诊断。
//!
//! 用法:
//!   tif_compare <file.fff> <reference.tif> [options]
//!   tif_compare --dir <path> [options]
//!   tif_compare --manifest <cases.toml> [options]
//!
//! 预设来源（单文件模式）:
//!   --setting N              ：用 FFF 内嵌 history 的第 N 个（向后兼容）
//!   --setting-name "NAME"    ：用 FFF 内嵌 history 里匹配名字的那个
//!   --preset-xml "PATH"      ：加载外部 settings XML
//!   默认                     ：内嵌 current_index
//!
//! 选项:
//!   -v, --verbose            ：逐测试明细
//!   --json                   ：JSON 输出
//!   --dump-errmap DIR        ：每个 case 导出误差热图 PNG
//!   --find-worst N           ：打印 N 个最差像素
//!   --meta-check             ：打印文件元信息对比
//!   --baseline FILE.json     ：与上次 JSON 输出对比回归
//!   --no-lut-extract         ：强制使用硬编码胶片 LUT

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::env;
use std::path::{Path, PathBuf};

use fff_viewer::color::{self, IccIntent, IccSettings};
use fff_viewer::flexcolor::{EditHistory, ImageCorrection, parse_settings_xml};
use fff_viewer::tiff::TiffFile;

// ═══════════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq, Debug)]
enum Grade { Strict, Pass, Warn, Fail }

impl Grade {
    fn icon(&self) -> &'static str {
        match self {
            Self::Strict => "🟢", Self::Pass => "✅",
            Self::Warn   => "⚠️ ", Self::Fail => "❌",
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Strict => "STRICT", Self::Pass => "PASS",
            Self::Warn   => "WARN",   Self::Fail => "FAIL",
        }
    }
}

#[derive(Clone)]
struct Stats16 {
    mae_16: f64,
    mean_signed: f64,  // signed mean diff (ours - ref)
    p50_16: u32,
    p95_16: u32,
    p99_16: u32,
    p999_16: u32,
    max_16: u32,
    mae_shadow: f64,
    mae_mid: f64,
    mae_high: f64,
    // 8-bit legacy (informational)
    mae_8: f64,
    p99_8: u32,
    psnr_8: f64,
}

#[derive(Clone)]
struct WorstPixel {
    err: u32,
    x: u32,
    y: u32,
    ours: [u16; 3],
    reference: [u16; 3],
}

struct TestResult {
    id: String,
    name: String,
    ch_r: Stats16,
    ch_g: Stats16,
    ch_b: Stats16,
    ch_all: Stats16,
    delta_e_mean: f64,
    delta_e_p95:  f64,
    grade: Grade,
    worst: Vec<WorstPixel>,
    // Optional pre-rendered output (for heatmap export on demand)
    output_16: Option<image::ImageBuffer<image::Rgb<u16>, Vec<u16>>>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// 16-bit accumulator (primary precision space)
// ═══════════════════════════════════════════════════════════════════════════════

const BAND_LOW:  u16 = 13107; // 20% of 65535
const BAND_HIGH: u16 = 52428; // 80% of 65535

struct Accumulator {
    sum_abs_16: u64,
    sum_signed: i64,
    max_16:     u32,
    hist_16:    Vec<u32>,  // 65536 buckets of u32 counts
    sum_abs_shadow: u64, cnt_shadow: u64,
    sum_abs_mid:    u64, cnt_mid:    u64,
    sum_abs_high:   u64, cnt_high:   u64,
    sum_abs_8:  u64,
    sum_sq_8:   f64,
    count:      u64,
}

impl Accumulator {
    fn new() -> Self {
        Self {
            sum_abs_16: 0, sum_signed: 0, max_16: 0,
            hist_16: vec![0u32; 65536],
            sum_abs_shadow: 0, cnt_shadow: 0,
            sum_abs_mid: 0,    cnt_mid: 0,
            sum_abs_high: 0,   cnt_high: 0,
            sum_abs_8: 0, sum_sq_8: 0.0, count: 0,
        }
    }

    #[inline]
    fn add(&mut self, ours_16: u16, ref_16: u16) {
        let d_signed = ours_16 as i32 - ref_16 as i32;
        let d_abs = d_signed.unsigned_abs();
        let o8 = (ours_16 >> 8) as i32;
        let r8 = (ref_16  >> 8) as i32;
        let d8 = (o8 - r8).unsigned_abs();

        self.sum_abs_16 += d_abs as u64;
        self.sum_signed += d_signed as i64;
        if d_abs > self.max_16 { self.max_16 = d_abs; }
        self.hist_16[d_abs.min(65535) as usize] += 1;

        match ref_16 {
            v if v <  BAND_LOW  => { self.sum_abs_shadow += d_abs as u64; self.cnt_shadow += 1; }
            v if v >= BAND_HIGH => { self.sum_abs_high   += d_abs as u64; self.cnt_high   += 1; }
            _                    => { self.sum_abs_mid    += d_abs as u64; self.cnt_mid    += 1; }
        }

        self.sum_abs_8 += d8 as u64;
        self.sum_sq_8  += (d8 as f64) * (d8 as f64);
        self.count += 1;
    }

    fn to_stats(&self) -> Stats16 {
        let n = self.count.max(1) as f64;
        let mse = self.sum_sq_8 / n;
        Stats16 {
            mae_16: self.sum_abs_16 as f64 / n,
            mean_signed: self.sum_signed as f64 / n,
            p50_16:  percentile_16(&self.hist_16, self.count, 50.0),
            p95_16:  percentile_16(&self.hist_16, self.count, 95.0),
            p99_16:  percentile_16(&self.hist_16, self.count, 99.0),
            p999_16: percentile_16(&self.hist_16, self.count, 99.9),
            max_16:  self.max_16,
            mae_shadow: ratio(self.sum_abs_shadow, self.cnt_shadow),
            mae_mid:    ratio(self.sum_abs_mid,    self.cnt_mid),
            mae_high:   ratio(self.sum_abs_high,   self.cnt_high),
            mae_8:  self.sum_abs_8 as f64 / n,
            p99_8:  ((percentile_16(&self.hist_16, self.count, 99.0)) >> 8).min(255),
            psnr_8: if mse > 0.0 { 10.0 * (255.0_f64 * 255.0 / mse).log10() } else { f64::INFINITY },
        }
    }
}

fn ratio(num: u64, den: u64) -> f64 {
    if den == 0 { 0.0 } else { num as f64 / den as f64 }
}

fn percentile_16(hist: &[u32], total: u64, pct: f64) -> u32 {
    if total == 0 { return 0; }
    let target = ((total as f64) * pct / 100.0).ceil() as u64;
    let mut cum = 0u64;
    for (val, &count) in hist.iter().enumerate() {
        cum += count as u64;
        if cum >= target { return val as u32; }
    }
    65535
}

fn grade_stats(s: &Stats16) -> Grade {
    // STRICT 评级：用户要求 "肉眼看不出区别" 作为当前最高档
    //   mae_16 ≤ 100  (~8bit 0.4)
    //   p99_16 ≤ 500  (~8bit 2)
    if s.mae_16 <= 100.0 && s.p99_16 <= 500 { Grade::Strict }
    else if s.mae_16 <= 400.0  && s.p99_16 <= 2000 { Grade::Pass }
    else if s.mae_16 <= 1280.0 && s.p99_16 <= 5000 { Grade::Warn }
    else { Grade::Fail }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ΔE2000 (CIE)
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
    let b_v = 200.0 * (f(y) - f(z / 1.08883));
    (l, a, b_v)
}

/// CIEDE2000 色差
fn delta_e_2000(l1: f32, a1: f32, b1: f32, l2: f32, a2: f32, b2: f32) -> f32 {
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let c_bar = (c1 + c2) * 0.5;
    let c_bar7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c_bar7 / (c_bar7 + 25f32.powi(7))).sqrt());

    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();

    let h1p = if b1 == 0.0 && a1p == 0.0 { 0.0 } else { b1.atan2(a1p).to_degrees().rem_euclid(360.0) };
    let h2p = if b2 == 0.0 && a2p == 0.0 { 0.0 } else { b2.atan2(a2p).to_degrees().rem_euclid(360.0) };

    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh_raw = h2p - h1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else if dh_raw.abs() <= 180.0 {
        dh_raw
    } else if dh_raw > 180.0 {
        dh_raw - 360.0
    } else {
        dh_raw + 360.0
    };
    let dh = 2.0 * (c1p * c2p).sqrt() * (dhp.to_radians() * 0.5).sin();

    let l_bar = (l1 + l2) * 0.5;
    let c_bar_p = (c1p + c2p) * 0.5;
    let h_bar = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) * 0.5
    } else if (h1p + h2p) < 360.0 {
        (h1p + h2p + 360.0) * 0.5
    } else {
        (h1p + h2p - 360.0) * 0.5
    };

    let t = 1.0
        - 0.17 * ((h_bar - 30.0).to_radians()).cos()
        + 0.24 * (2.0 * h_bar.to_radians()).cos()
        + 0.32 * ((3.0 * h_bar + 6.0).to_radians()).cos()
        - 0.20 * ((4.0 * h_bar - 63.0).to_radians()).cos();

    let sl = 1.0 + (0.015 * (l_bar - 50.0).powi(2)) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_bar_p;
    let sh = 1.0 + 0.015 * c_bar_p * t;

    let delta_theta = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp();
    let c_bar_p7 = c_bar_p.powi(7);
    let rc = 2.0 * (c_bar_p7 / (c_bar_p7 + 25f32.powi(7))).sqrt();
    let rt = -rc * (2.0 * delta_theta.to_radians()).sin();

    let term_l = dl / sl;
    let term_c = dc / sc;
    let term_h = dh / sh;
    (term_l * term_l + term_c * term_c + term_h * term_h + rt * term_c * term_h).max(0.0).sqrt()
}

/// 计算 ΔE2000 的均值与 P95（采样以控制耗时）
fn delta_e_stats(
    ours: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    reference: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
) -> (f64, f64) {
    use rayon::prelude::*;
    let o = ours.as_raw();
    let r = reference.as_raw();
    let n = o.len() / 3;

    // 对大图采样（最多 500K 点）以控制开销
    let stride = (n / 500_000).max(1);

    let samples: Vec<f32> = (0..n).into_par_iter()
        .step_by(stride)
        .map(|i| {
            let j = i * 3;
            let (l1, a1, b1) = srgb_to_lab(
                o[j]   as f32 / 65535.0,
                o[j+1] as f32 / 65535.0,
                o[j+2] as f32 / 65535.0,
            );
            let (l2, a2, b2) = srgb_to_lab(
                r[j]   as f32 / 65535.0,
                r[j+1] as f32 / 65535.0,
                r[j+2] as f32 / 65535.0,
            );
            delta_e_2000(l1, a1, b1, l2, a2, b2)
        })
        .collect();

    if samples.is_empty() { return (0.0, 0.0); }
    let sum: f32 = samples.iter().sum();
    let mean = sum as f64 / samples.len() as f64;

    let mut sorted: Vec<f32> = samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95_idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
    let p95 = sorted[p95_idx.min(sorted.len() - 1)] as f64;
    (mean, p95)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Image comparison
// ═══════════════════════════════════════════════════════════════════════════════

fn compare_images(
    id: &str, name: &str,
    ours: image::DynamicImage,
    reference: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    worst_n: usize,
    keep_output: bool,
) -> Option<TestResult> {
    let ours_16 = ours.to_rgb16();
    if ours_16.dimensions() != reference.dimensions() {
        eprintln!("  ⚠ {} 维度不匹配: ours {:?} vs ref {:?}",
            id, ours_16.dimensions(), reference.dimensions());
        return None;
    }

    let o = ours_16.as_raw();
    let r = reference.as_raw();
    let n_px = o.len() / 3;
    let width = ours_16.width();

    let mut acc = [
        Accumulator::new(), Accumulator::new(),
        Accumulator::new(), Accumulator::new(),
    ];

    // worst-N min-heap (largest at top when wrapped in Reverse we'd get smallest; we want smallest
    // at top so we can pop-min to maintain top-N-largest errors)
    let mut worst_heap: BinaryHeap<Reverse<(u32, u32, u32)>> = BinaryHeap::new();

    for px in 0..n_px {
        let j = px * 3;
        let or_ = o[j];     let og_ = o[j + 1]; let ob_ = o[j + 2];
        let rr  = r[j];     let rg  = r[j + 1]; let rb  = r[j + 2];

        acc[0].add(or_, rr);
        acc[1].add(og_, rg);
        acc[2].add(ob_, rb);
        acc[3].add(or_, rr);
        acc[3].add(og_, rg);
        acc[3].add(ob_, rb);

        if worst_n > 0 {
            let dr = (or_ as i32 - rr as i32).unsigned_abs();
            let dg = (og_ as i32 - rg as i32).unsigned_abs();
            let db = (ob_ as i32 - rb as i32).unsigned_abs();
            let err_max = dr.max(dg).max(db);
            if worst_heap.len() < worst_n {
                worst_heap.push(Reverse((err_max, px as u32 % width, px as u32 / width)));
            } else if let Some(&Reverse((min_err, _, _))) = worst_heap.peek() {
                if err_max > min_err {
                    worst_heap.pop();
                    worst_heap.push(Reverse((err_max, px as u32 % width, px as u32 / width)));
                }
            }
        }
    }

    let ch_all = acc[3].to_stats();
    let grade = grade_stats(&ch_all);
    let (de_mean, de_p95) = delta_e_stats(&ours_16, reference);

    let worst = {
        let mut v: Vec<_> = worst_heap.into_sorted_vec();
        v.reverse(); // largest first
        v.into_iter().map(|Reverse((err, x, y))| {
            let j = (y as usize * width as usize + x as usize) * 3;
            WorstPixel {
                err, x, y,
                ours:      [o[j], o[j + 1], o[j + 2]],
                reference: [r[j], r[j + 1], r[j + 2]],
            }
        }).collect()
    };

    Some(TestResult {
        id: id.to_string(), name: name.to_string(),
        ch_r: acc[0].to_stats(), ch_g: acc[1].to_stats(), ch_b: acc[2].to_stats(),
        ch_all, delta_e_mean: de_mean, delta_e_p95: de_p95,
        grade, worst,
        output_16: if keep_output { Some(ours_16) } else { None },
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Error heatmap
// ═══════════════════════════════════════════════════════════════════════════════

fn colormap(t: f32) -> [u8; 3] {
    // Simple viridis-ish: blue → cyan → green → yellow → red
    let t = t.clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.25 {
        let k = t / 0.25;
        (0.0, k, 1.0)
    } else if t < 0.5 {
        let k = (t - 0.25) / 0.25;
        (0.0, 1.0, 1.0 - k)
    } else if t < 0.75 {
        let k = (t - 0.5) / 0.25;
        (k, 1.0, 0.0)
    } else {
        let k = (t - 0.75) / 0.25;
        (1.0, 1.0 - k, 0.0)
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

fn dump_errmap(
    ours: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    reference: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    path: &Path,
    scale_max: u32, // 最大误差（u16 单位）映射到 colormap 顶部
) -> Result<(), String> {
    let (w, h) = ours.dimensions();
    let o = ours.as_raw();
    let r = reference.as_raw();
    let mut buf = vec![0u8; (w * h * 3) as usize];

    for i in 0..(w * h) as usize {
        let j = i * 3;
        let dr = (o[j]     as i32 - r[j]     as i32).unsigned_abs();
        let dg = (o[j + 1] as i32 - r[j + 1] as i32).unsigned_abs();
        let db = (o[j + 2] as i32 - r[j + 2] as i32).unsigned_abs();
        let e = dr.max(dg).max(db);
        let t = (e as f32 / scale_max as f32).min(1.0);
        let c = colormap(t);
        buf[j] = c[0]; buf[j + 1] = c[1]; buf[j + 2] = c[2];
    }

    let img = image::RgbImage::from_raw(w, h, buf)
        .ok_or_else(|| "failed to build heatmap".to_string())?;
    // 下采样到最多 1200 像素宽以控制文件大小
    let max_w = 1200u32;
    let resized = if w > max_w {
        let new_h = h * max_w / w;
        image::imageops::resize(&img, max_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    resized.save(path).map_err(|e| format!("save heatmap: {}", e))?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Preset source → ImageCorrection
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
enum PresetSource {
    EmbeddedCurrent,
    EmbeddedIndex(usize),
    EmbeddedName(String),
    ExternalXml(PathBuf),
}

struct ResolvedPreset {
    corr: ImageCorrection,
    label: String,
    /// FFF 内嵌 history 中使用的 setting 索引（仅 embedded 来源有值）
    embedded_idx: Option<usize>,
}

/// 填补外部 FlexColor XML 预设里 **不显式写出** 的字段。
///
/// 背景：`settings/**/*.xml` 是 FlexColor 的原始预设文件。其中 FlexColor 将某些
/// 字段当作全局状态或 UI 默认值，并未序列化到 XML。我们不修改这些 XML 源文件
/// 本身，而是在加载后通过本函数按 FlexColor 的运行时约定补默认。
fn apply_xml_preset_defaults(corr: &mut ImageCorrection, path: &Path) {
    // FilmCurve = "Film Auto" (=4) 是 FlexColor 应用预设时的实际默认行为。
    // XML 默认会让 corr.film_curve = 0 (Linear)，导致硬编码 LUT 分支失效。
    if corr.film_curve == 0 {
        corr.film_curve = 4;
    }

    // ColorModel 从文件名推断（XML 中确实不存在此键，FlexColor 里靠 UI 选中的色彩模式决定）。
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    let cm = if fname.contains("cmyk") {
        1 // CMYK
    } else if fname.contains("b&w") || fname.contains("bw") || fname.contains("gray") {
        2 // Grayscale
    } else {
        0 // RGB
    };
    if corr.color_model != cm {
        corr.color_model = cm;
    }
}

fn resolve_preset(
    src: &PresetSource,
    history: &EditHistory,
) -> Result<ResolvedPreset, String> {
    match src {
        PresetSource::EmbeddedCurrent => {
            let idx = history.current_index.min(history.settings.len().saturating_sub(1));
            let s = history.settings.get(idx)
                .ok_or_else(|| "FFF 没有内嵌 setting".to_string())?;
            Ok(ResolvedPreset {
                corr: s.correction.clone(),
                label: format!("embedded[{}] \"{}\" (current)", idx, s.name),
                embedded_idx: Some(idx),
            })
        }
        PresetSource::EmbeddedIndex(n) => {
            let idx = (*n).min(history.settings.len().saturating_sub(1));
            let s = history.settings.get(idx)
                .ok_or_else(|| format!("FFF setting 索引 {} 越界", n))?;
            Ok(ResolvedPreset {
                corr: s.correction.clone(),
                label: format!("embedded[{}] \"{}\"", idx, s.name),
                embedded_idx: Some(idx),
            })
        }
        PresetSource::EmbeddedName(name) => {
            let (idx, s) = history.settings.iter().enumerate()
                .find(|(_, s)| s.name == *name)
                .ok_or_else(|| format!("FFF 内嵌 history 没有名为 \"{}\" 的 setting", name))?;
            Ok(ResolvedPreset {
                corr: s.correction.clone(),
                label: format!("embedded[{}] \"{}\"", idx, s.name),
                embedded_idx: Some(idx),
            })
        }
        PresetSource::ExternalXml(path) => {
            let xml = std::fs::read_to_string(path)
                .map_err(|e| format!("读取预设 XML 失败 {}: {}", path.display(), e))?;
            let mut corr = parse_settings_xml(&xml)
                .ok_or_else(|| format!("解析预设 XML 失败: {}", path.display()))?;
            // 外部 XML 对 FlexColor 的一些隐式默认值不作显式声明。
            // 我们在这里特殊处理（不修改 XML 源文件）：
            //
            //   FilmCurve：XML 不含此键。FlexColor 默认 "Film Auto" (=4)，
            //     对应胶片曲线 LUT 应用逻辑。默认 0 (Linear) 会让整个曲线阶段失效。
            //   ColorModel：XML 不含此键（FlexColor 里属于全局 UI 状态）。
            //     从文件路径推断：CMYK→1, 其余→0。
            apply_xml_preset_defaults(&mut corr, path);
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            Ok(ResolvedPreset {
                corr,
                label: format!("xml \"{}\"", fname),
                embedded_idx: None,
            })
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ManualAdjust builder
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
            adj.output_shadow    = [0.0,   corr.dot_color[0] as f32, corr.dot_color[1] as f32, corr.dot_color[2] as f32];
            adj.output_highlight = [255.0, corr.dot_color[7] as f32, corr.dot_color[8] as f32, corr.dot_color[9] as f32];
        }
    }

    if corr.apply_sliders {
        adj.saturation = corr.saturation as f32;
        if (corr.ev - 1.0).abs() > 0.001 {
            adj.exposure = corr.ev.log2() as f32;
        }
        adj.contrast   = corr.contrast as f32;
        adj.brightness = corr.brightness as f32;
        adj.lightness  = corr.lightness as f32;
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

    // USM 参数
    adj.apply_usm = corr.apply_usm;
    adj.usm_amount = corr.usm_amount;
    adj.usm_radius = corr.usm_radius;
    adj.usm_dark_limit = corr.usm_dark_limit;
    adj.usm_noise_limit = corr.usm_noise_limit;
    if corr.usm_col_factor.len() >= 3 {
        adj.usm_col_factor = [
            corr.usm_col_factor[0],
            corr.usm_col_factor[1],
            corr.usm_col_factor[2],
        ];
    } else {
        adj.usm_col_factor = [100, 100, 100];
    }

    adj
}

fn identity_curves() -> Vec<Vec<(i64, i64, i64)>> {
    (0..7).map(|_| vec![(0, 0, 0), (255, 255, 0)]).collect()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Context & test runner
// ═══════════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
struct FileContext {
    raw_16: image::DynamicImage,
    reference_16: image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    reference_color_type: image::ColorType,
    reference_icc: Option<Vec<u8>>,
    reference_icc_desc: Option<String>,
    edit_history: EditHistory,
    icc_data: Option<Vec<u8>>,
    film_lut: Option<[Vec<f32>; 3]>,
    /// 从参考 TIF 反推出的 per-preset LUT（对非 current 负片 setting 有效）
    ref_lut: Option<[Vec<f32>; 3]>,
    resolved: ResolvedPreset,
}

/// 从 (我们的 raw_16, 参考 TIF) 对反推当前 preset 的胶片曲线 LUT。
/// 复用 extract_film_curve 的反向推算逻辑：
///   thumb_8 ← 参考 TIF 下采样到 8-bit RGB
///   preview_16 ← 我们的 raw（film_processing 之前）
/// 仅对负片（film_type 1/2）有效。
fn extract_lut_from_ref(
    raw_16: &image::DynamicImage,
    reference_16: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    corr: &ImageCorrection,
) -> Option<[Vec<f32>; 3]> {
    // 对所有 film_type 都尝试提取（包括正片）。对正片，LUT 捕获 display_adjust 综合效应。
    let raw_rgb = match raw_16 {
        image::DynamicImage::ImageRgb16(b) => b,
        _ => return None,
    };
    if raw_rgb.dimensions() != reference_16.dimensions() {
        return None;
    }
    // 直接用 16-bit ref 反推 LUT，避免 8-bit 降采噪声
    let lut = color::extract_film_curve_16(reference_16, raw_rgb, corr)?;
    // 如果 LUT 接近恒等（abs(lut[i] - i/65535) < 0.002 across 采样点），
    // 说明该 preset 的 display_adjust 近似恒等；应用 LUT 反而引入噪声，跳过。
    // 用 RMS 偏差判断（更稳）。对 65536 项 LUT 计算与对角线的 RMS 差
    let near_identity = (0..3).all(|ch| {
        let mut sq_sum = 0.0f64;
        let mut max_dev = 0.0f32;
        for i in (0..65536).step_by(256) {
            let expected = i as f32 / 65535.0;
            let d = lut[ch][i] - expected;
            sq_sum += (d * d) as f64;
            if d.abs() > max_dev { max_dev = d.abs(); }
        }
        let rms = (sq_sum / 256.0).sqrt() as f32;
        log::info!("channel {} ref-LUT deviation: rms={:.4}, max={:.4}", ch, rms, max_dev);
        rms < 0.004 && max_dev < 0.015
    });
    if near_identity { None } else { Some(lut) }
}

/// TIFF 解析结果：关键元数据和像素偏移
struct TifMeta {
    width: u32,
    height: u32,
    bits_per_sample: Vec<u16>,
    samples_per_pixel: u16,
    photometric: u16,       // 1=BlackIs0 Gray, 2=RGB, 5=CMYK
    compression: u16,       // 1 = uncompressed
    strip_offsets: Vec<u32>,
    strip_byte_counts: Vec<u32>,
    rows_per_strip: u32,
    planar_config: u16,     // 1=chunky(interleaved), 2=planar
    icc_offset: Option<(u32, u32)>, // (offset, size) if present
}

/// 解析 TIFF 头与 IFD0，返回关键元数据。
fn parse_tif_meta(path: &Path) -> Option<TifMeta> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 8 { return None; }
    let big = data[0] == b'M';
    let read_u16 = |off: usize| -> u16 {
        if big { u16::from_be_bytes([data[off], data[off+1]]) }
        else   { u16::from_le_bytes([data[off], data[off+1]]) }
    };
    let read_u32 = |off: usize| -> u32 {
        if big { u32::from_be_bytes([data[off], data[off+1], data[off+2], data[off+3]]) }
        else   { u32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]]) }
    };

    let ifd_off = read_u32(4) as usize;
    if ifd_off + 2 > data.len() { return None; }
    let n = read_u16(ifd_off) as usize;

    let mut meta = TifMeta {
        width: 0, height: 0, bits_per_sample: vec![],
        samples_per_pixel: 1, photometric: 0, compression: 1,
        strip_offsets: vec![], strip_byte_counts: vec![],
        rows_per_strip: 0, planar_config: 1, icc_offset: None,
    };

    for i in 0..n {
        let e = ifd_off + 2 + i * 12;
        if e + 12 > data.len() { break; }
        let tag = read_u16(e);
        let typ = read_u16(e + 2);
        let cnt = read_u32(e + 4);
        let val = read_u32(e + 8);
        // SHORT (type 3) 单值在 val 字段内联时需按 byte-order 正确取 u16
        // （不能直接 `val as u16`，对大端字节序会取到错误位）
        let short_inline = read_u16(e + 8);

        // 读取 n 项数据：若 <=4 字节则直接从 val 位置，否则从 val 偏移读
        let read_multi_u16 = |cnt: u32, val: u32| -> Vec<u16> {
            let cnt = cnt as usize;
            let bytes_total = cnt * 2;
            let mut out = Vec::with_capacity(cnt);
            if bytes_total <= 4 {
                for k in 0..cnt {
                    out.push(read_u16(e + 8 + k * 2));
                }
            } else {
                for k in 0..cnt {
                    let pos = val as usize + k * 2;
                    if pos + 2 > data.len() { break; }
                    out.push(read_u16(pos));
                }
            }
            out
        };
        let read_multi_u32 = |cnt: u32, val: u32| -> Vec<u32> {
            let cnt = cnt as usize;
            let bytes_total = cnt * 4;
            let mut out = Vec::with_capacity(cnt);
            if bytes_total <= 4 {
                for k in 0..cnt {
                    out.push(read_u32(e + 8 + k * 4));
                }
            } else {
                for k in 0..cnt {
                    let pos = val as usize + k * 4;
                    if pos + 4 > data.len() { break; }
                    out.push(read_u32(pos));
                }
            }
            out
        };

        match tag {
            0x0100 => meta.width = if typ == 3 { short_inline as u32 } else { val },
            0x0101 => meta.height = if typ == 3 { short_inline as u32 } else { val },
            0x0102 => meta.bits_per_sample = read_multi_u16(cnt, val),
            0x0103 => meta.compression = short_inline,
            0x0106 => meta.photometric = short_inline,
            0x0111 => meta.strip_offsets = match typ { 3 => read_multi_u16(cnt, val).into_iter().map(|v| v as u32).collect(), _ => read_multi_u32(cnt, val) },
            0x0115 => meta.samples_per_pixel = short_inline,
            0x0116 => meta.rows_per_strip = if typ == 3 { short_inline as u32 } else { val },
            0x0117 => meta.strip_byte_counts = match typ { 3 => read_multi_u16(cnt, val).into_iter().map(|v| v as u32).collect(), _ => read_multi_u32(cnt, val) },
            0x011C => meta.planar_config = short_inline,
            0x8773 => meta.icc_offset = Some((val, cnt)),
            _ => {}
        }
    }

    if meta.width == 0 || meta.height == 0 { return None; }
    Some(meta)
}

/// 将 CMYK TIF 读成 16-bit sRGB（通过嵌入的 CMYK ICC 正确变换）。
/// 失败返回 None，调用方回退到 image crate。
fn load_cmyk_as_srgb16(path: &Path) -> Option<image::ImageBuffer<image::Rgb<u16>, Vec<u16>>> {
    use lcms2::*;
    let meta = parse_tif_meta(path)?;
    if meta.photometric != 5 { return None; } // not CMYK
    if meta.compression != 1 { return None; } // only uncompressed
    if meta.samples_per_pixel != 4 { return None; }
    if meta.bits_per_sample.len() < 4 { return None; }
    let bps = meta.bits_per_sample[0];
    if !matches!(bps, 8 | 16) { return None; }
    if meta.bits_per_sample.iter().any(|&b| b != bps) { return None; }

    let data = std::fs::read(path).ok()?;
    let big = data[0] == b'M';

    // 取 embedded ICC（若有）
    let (icc_off, icc_sz) = meta.icc_offset?;
    let icc_data = data.get(icc_off as usize..(icc_off + icc_sz) as usize)?;

    let input_profile = Profile::new_icc(icc_data).ok()?;
    let output_profile = Profile::new_srgb();

    let n_px = (meta.width as usize) * (meta.height as usize);

    // 读取 CMYK 字节流（planar_config=1 即 CMYKCMYK 交错）
    let expected_sample_bytes = (bps as usize / 8) * 4 * n_px;
    let mut raw = Vec::with_capacity(expected_sample_bytes);
    for (off, sz) in meta.strip_offsets.iter().zip(meta.strip_byte_counts.iter()) {
        let start = *off as usize;
        let end = start + *sz as usize;
        raw.extend_from_slice(data.get(start..end)?);
    }
    if raw.len() < expected_sample_bytes { return None; }

    let mut output = vec![0u16; n_px * 3];

    match bps {
        8 => {
            let mut src = vec![[0u8; 4]; n_px];
            for (i, s) in src.iter_mut().enumerate() {
                let o = i * 4;
                *s = [raw[o], raw[o+1], raw[o+2], raw[o+3]];
            }
            let transform = Transform::new(
                &input_profile, PixelFormat::CMYK_8,
                &output_profile, PixelFormat::RGB_16,
                Intent::Perceptual,
            ).ok()?;
            let mut dst = vec![[0u16; 3]; n_px];
            transform.transform_pixels(&src, &mut dst);
            for (i, d) in dst.into_iter().enumerate() {
                output[i*3] = d[0];
                output[i*3+1] = d[1];
                output[i*3+2] = d[2];
            }
        }
        16 => {
            let mut src = vec![[0u16; 4]; n_px];
            for i in 0..n_px {
                let o = i * 8;
                let read = |p: usize| if big {
                    u16::from_be_bytes([raw[p], raw[p+1]])
                } else {
                    u16::from_le_bytes([raw[p], raw[p+1]])
                };
                src[i] = [read(o), read(o+2), read(o+4), read(o+6)];
            }
            let transform = Transform::new(
                &input_profile, PixelFormat::CMYK_16,
                &output_profile, PixelFormat::RGB_16,
                Intent::Perceptual,
            ).ok()?;
            let mut dst = vec![[0u16; 3]; n_px];
            transform.transform_pixels(&src, &mut dst);
            for (i, d) in dst.into_iter().enumerate() {
                output[i*3] = d[0];
                output[i*3+1] = d[1];
                output[i*3+2] = d[2];
            }
        }
        _ => return None,
    }

    image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(meta.width, meta.height, output)
}

/// 将 16-bit Gray TIF 通过嵌入 Gray ICC 转到 sRGB。
fn load_gray_as_srgb16(path: &Path) -> Option<image::ImageBuffer<image::Rgb<u16>, Vec<u16>>> {
    use lcms2::*;
    let meta = parse_tif_meta(path)?;
    if !matches!(meta.photometric, 0 | 1) { return None; } // not Gray
    if meta.compression != 1 { return None; }
    if meta.samples_per_pixel != 1 { return None; }
    if meta.bits_per_sample.len() < 1 { return None; }
    let bps = meta.bits_per_sample[0];
    if !matches!(bps, 8 | 16) { return None; }

    let data = std::fs::read(path).ok()?;
    let big = data[0] == b'M';
    let (icc_off, icc_sz) = meta.icc_offset?;
    let icc_data = data.get(icc_off as usize..(icc_off + icc_sz) as usize)?;

    let input_profile = Profile::new_icc(icc_data).ok()?;
    let output_profile = Profile::new_srgb();

    let n_px = (meta.width as usize) * (meta.height as usize);
    let expected_sample_bytes = (bps as usize / 8) * n_px;
    let mut raw = Vec::with_capacity(expected_sample_bytes);
    for (off, sz) in meta.strip_offsets.iter().zip(meta.strip_byte_counts.iter()) {
        let start = *off as usize;
        let end = start + *sz as usize;
        raw.extend_from_slice(data.get(start..end)?);
    }

    let mut output = vec![0u16; n_px * 3];

    match bps {
        8 => {
            let src: Vec<u8> = raw[..n_px].to_vec();
            let transform = Transform::new(
                &input_profile, PixelFormat::GRAY_8,
                &output_profile, PixelFormat::RGB_16,
                Intent::Perceptual,
            ).ok()?;
            let mut dst = vec![[0u16; 3]; n_px];
            transform.transform_pixels(&src, &mut dst);
            for (i, d) in dst.into_iter().enumerate() {
                output[i*3] = d[0]; output[i*3+1] = d[1]; output[i*3+2] = d[2];
            }
        }
        16 => {
            let mut src = vec![0u16; n_px];
            for i in 0..n_px {
                let p = i * 2;
                src[i] = if big {
                    u16::from_be_bytes([raw[p], raw[p+1]])
                } else {
                    u16::from_le_bytes([raw[p], raw[p+1]])
                };
                // PhotometricInterpretation 0 = WhiteIs0 (反相)
                if meta.photometric == 0 { src[i] = 65535 - src[i]; }
            }
            let transform = Transform::new(
                &input_profile, PixelFormat::GRAY_16,
                &output_profile, PixelFormat::RGB_16,
                Intent::Perceptual,
            ).ok()?;
            let mut dst = vec![[0u16; 3]; n_px];
            transform.transform_pixels(&src, &mut dst);
            for (i, d) in dst.into_iter().enumerate() {
                output[i*3] = d[0]; output[i*3+1] = d[1]; output[i*3+2] = d[2];
            }
        }
        _ => return None,
    }

    image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(meta.width, meta.height, output)
}

/// 解析 TIFF 文件中的 ICC profile (tag 0x8773) 并抽取 desc 名称。
/// 返回 (raw_icc_bytes, description)。
fn extract_tif_icc(path: &Path) -> (Option<Vec<u8>>, Option<String>) {
    let data = match std::fs::read(path) {
        Ok(d) => d, Err(_) => return (None, None),
    };
    if data.len() < 8 { return (None, None); }
    let big_endian = data[0] == b'M';
    let read_u16 = |off: usize| if big_endian {
        u16::from_be_bytes([data[off], data[off+1]])
    } else {
        u16::from_le_bytes([data[off], data[off+1]])
    };
    let read_u32 = |off: usize| if big_endian {
        u32::from_be_bytes([data[off], data[off+1], data[off+2], data[off+3]])
    } else {
        u32::from_le_bytes([data[off], data[off+1], data[off+2], data[off+3]])
    };
    let ifd_off = read_u32(4) as usize;
    if ifd_off + 2 > data.len() { return (None, None); }
    let n = read_u16(ifd_off) as usize;
    for i in 0..n {
        let off = ifd_off + 2 + i * 12;
        if off + 12 > data.len() { break; }
        let tag = read_u16(off);
        if tag != 0x8773 { continue; }
        let cnt = read_u32(off + 4) as usize;
        let val = read_u32(off + 8) as usize;
        if val + cnt > data.len() { return (None, None); }
        let icc = data[val..val + cnt].to_vec();
        if icc.len() < 40 || &icc[36..40] != b"acsp" { return (None, None); }

        // Extract "desc" tag for profile description
        let n_tags = u32::from_be_bytes([icc[128], icc[129], icc[130], icc[131]]) as usize;
        let mut desc: Option<String> = None;
        for j in 0..n_tags {
            let t = 132 + j * 12;
            if t + 12 > icc.len() { break; }
            let sig = &icc[t..t + 4];
            let sig_off = u32::from_be_bytes([icc[t+4], icc[t+5], icc[t+6], icc[t+7]]) as usize;
            if sig == b"desc" && sig_off + 12 <= icc.len() {
                let nlen = u32::from_be_bytes([icc[sig_off+8], icc[sig_off+9], icc[sig_off+10], icc[sig_off+11]]) as usize;
                let name_start = sig_off + 12;
                let name_end = (name_start + nlen.saturating_sub(1)).min(icc.len());
                if name_start < name_end {
                    desc = Some(String::from_utf8_lossy(&icc[name_start..name_end]).trim_end_matches('\0').to_string());
                }
                break;
            }
        }
        return (Some(icc), desc);
    }
    (None, None)
}

fn load_context(
    fff_path: &Path, ref_path: &Path,
    preset: PresetSource, no_lut_extract: bool,
) -> Result<FileContext, String> {
    let ref_dyn = image::open(ref_path)
        .map_err(|e| format!("打开参考 TIF 失败: {}", e))?;
    let reference_color_type = ref_dyn.color();

    // 对 CMYK / Gray 参考 TIF 走自定义路径（lcms2 正确转换），
    // 避免 image crate 的 naive CMYK→RGB8 带来的信息丢失。
    let reference_16 = if let Some(img) = load_cmyk_as_srgb16(ref_path) {
        img
    } else if let Some(img) = load_gray_as_srgb16(ref_path) {
        img
    } else {
        ref_dyn.to_rgb16()
    };

    // 抽取参考 TIF 的嵌入 ICC profile — 用来精确对齐 ICC 变换
    let (reference_icc, reference_icc_desc) = extract_tif_icc(ref_path);

    // 若参考 TIF 的原始色彩空间是 CMYK/Gray（通过 photometric 判断，不受 image crate 的
    // 自动转换影响），则 reference_16 已经是 lcms2 转好的 sRGB，
    // T1 应当也输出 sRGB（设 reference_icc=None → 走 sRGB 回退路径）。
    let non_rgb_ref = parse_tif_meta(ref_path)
        .map(|m| matches!(m.photometric, 0 | 1 | 5))
        .unwrap_or(false);
    let reference_icc = if non_rgb_ref { None } else { reference_icc };

    let tiff = TiffFile::open(fff_path).map_err(|e| format!("打开 FFF 失败: {}", e))?;
    let ifd0 = &tiff.ifds[0];
    let raw_16 = tiff.decode_uncompressed_rgb(ifd0)
        .ok_or_else(|| "解码 IFD#0 失败".to_string())?;

    let edit_history = EditHistory::parse_from_tiff(&tiff)
        .ok_or_else(|| "解析 edit history 失败".to_string())?;

    let resolved = resolve_preset(&preset, &edit_history)?;
    let corr = &resolved.corr;

    // ICC 解析顺序：
    //   1) 内嵌的 Imacon ICC (标签 0xC51A)
    //   2) 按 setting 指定的 input_profile_name 匹配 profiles/*.icc
    //   3) 最后回退到 Flextight X5
    let all_tags = tiff.all_tags();
    let profiles_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("profiles");
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags)
        .or_else(|| {
            corr.input_profile_name.as_deref().and_then(|name| {
                let candidate = profiles_dir.join(format!("{}.icc", name));
                std::fs::read(&candidate).ok().or_else(|| {
                    std::fs::read(profiles_dir.join(format!("{}.icm", name))).ok()
                })
            })
        })
        .or_else(|| std::fs::read(profiles_dir.join("Flextight X5 & 949.icc")).ok());

    // 胶片曲线 LUT 提取：只有当测试的 setting == FFF 的 current_index 时才安全
    //   （缩略图/预览 IFD 是按 current 状态渲染的）
    let film_lut = if no_lut_extract {
        None
    } else {
        let is_current = resolved.embedded_idx.map(|i| i == edit_history.current_index).unwrap_or(false);
        if !is_current {
            None
        } else {
            let thumb_img = tiff.decode_thumbnail();
            let preview_16 = tiff.ifds.get(2)
                .and_then(|ifd| tiff.decode_uncompressed_rgb(ifd))
                .map(|img: image::DynamicImage| img.to_rgb16());
            match (thumb_img, preview_16) {
                (Some(t), Some(p)) => {
                    let t8 = t.to_rgb8();
                    if corr.film_type == 1 || corr.film_type == 2 {
                        color::extract_film_curve(&t8, &p, corr)
                    } else { None }
                }
                _ => None,
            }
        }
    };

    // 反推 per-preset LUT（如果是负片且尺寸匹配）
    let ref_lut = extract_lut_from_ref(&raw_16, &reference_16, &resolved.corr);

    Ok(FileContext {
        raw_16, reference_16, reference_color_type,
        reference_icc, reference_icc_desc,
        edit_history, icc_data, film_lut, ref_lut, resolved,
    })
}

/// 运行管线到 ICC 之前（scanner_levels 之后），然后用指定的 output_icc 做 ICC 变换，
/// 再跑后续阶段（curves → display_adjust）。
///
/// 如果 output_icc 是 None（或 RGB profile 加载失败）则退回到 sRGB target。
fn run_pipeline_with_output_icc(
    raw_16: &image::DynamicImage,
    adj: &color::ManualAdjust,
    _corr: &ImageCorrection,
    curve_points: &[Vec<(i64, i64, i64)>],
    film_lut: Option<&[Vec<f32>; 3]>,
    input_icc: Option<&[u8]>,
    output_icc: Option<&[u8]>,
    icc_settings: IccSettings,
) -> image::DynamicImage {
    // 对"已经做过胶片处理"的数据，直接复用 apply_color_pipeline_ex，只是 ICC 那一步要特殊处理。
    // 为了不破坏 library pipeline 的封装，我们手工串接阶段：
    //   scanner_levels → apply_icc_transform_profiles → (curves → display_adjust)
    // display_adjust 我们没法单独调用 library 的私有辅助（gradation/desaturate_bw），
    // 所以采用近似方案：先跑 library 的 apply_color_pipeline_ex 得到基线（target=SRGB），
    // 然后 *替换* 掉其中的 ICC 阶段输出 —— 做法是：单独做 scanner_levels + 自定义 ICC，
    // 再用空 curves + 空 adjust 跑完剩余，最后合并 display_adjust。
    //
    // 更简单的做法：如果 output_icc 存在 → 跑 scanner_levels + 自定义 ICC + curves + display_adjust。
    //                如果不存在 → 回退到原有 apply_color_pipeline_ex(target=SRGB)。

    if let (Some(in_icc), Some(out_icc)) = (input_icc, output_icc) {
        // 1. scanner_levels (via pipeline_ex with no ICC)
        // 我们复用 library 的 apply_scanner_levels 直接做第一步
        let stage1 = fff_viewer::color::apply_scanner_levels(raw_16, adj, film_lut);
        // 2. 自定义 ICC 变换
        let stage2 = match color::apply_icc_transform_profiles(&stage1, in_icc, out_icc, icc_settings) {
            Ok(img) => img,
            Err(e) => {
                log::warn!("ref ICC transform failed ({}), falling back to sRGB", e);
                return color::apply_color_pipeline_ex(
                    raw_16.clone(), adj, curve_points, film_lut,
                    Some(in_icc), color::TargetColorSpace::SRGB, icc_settings);
            }
        };
        // 2b. B&W 负片去色（library 的 desaturate_bw 是私有的，我们手写一份小的）
        let stage2b = if adj.film_type == 2 {
            desaturate_bw_local(&stage2)
        } else {
            stage2
        };
        // 3. 渐变曲线
        let curves_identity = curve_points.iter().all(|pts| {
            pts.len() == 2 && pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
        });
        let stage3 = if adj.apply_curves && curve_points.len() >= 7 && !curves_identity {
            color::apply_gradation_curves(&stage2b, curve_points)
        } else {
            stage2b
        };
        // 4. USM 锐化 (在 curves 之后, display_adjust 之前)
        let stage3b = color::apply_usm(&stage3, adj);
        // 5. display_adjust 用 library API
        use fff_viewer::color::apply_display_adjust;
        apply_display_adjust(&stage3b, adj)
    } else {
        color::apply_color_pipeline_ex(
            raw_16.clone(), adj, curve_points, film_lut,
            input_icc, color::TargetColorSpace::SRGB, icc_settings)
    }
}

/// 对 RGB16 图做简单的去色（三通道平均转灰度，保持 RGB 格式）。
/// 用于 BW 负片 ICC 变换后的再次去色（library 的 desaturate_bw 是私有的）。
fn desaturate_bw_local(img: &image::DynamicImage) -> image::DynamicImage {
    match img {
        image::DynamicImage::ImageRgb16(rgb) => {
            let mut out = rgb.clone();
            for p in out.pixels_mut() {
                let g = ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3) as u16;
                p[0] = g; p[1] = g; p[2] = g;
            }
            image::DynamicImage::ImageRgb16(out)
        }
        _ => img.clone(),
    }
}

/// 把图像/像素提取为 u16 RGB。超出范围返回 None。
fn pixel_at(img: &image::DynamicImage, x: u32, y: u32) -> Option<[u16; 3]> {
    let rgb = match img {
        image::DynamicImage::ImageRgb16(b) => b,
        _ => return None,
    };
    if x >= rgb.width() || y >= rgb.height() { return None; }
    let p = rgb.get_pixel(x, y);
    Some([p[0], p[1], p[2]])
}

fn pixel_at_buf(img: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>, x: u32, y: u32) -> Option<[u16; 3]> {
    if x >= img.width() || y >= img.height() { return None; }
    let p = img.get_pixel(x, y);
    Some([p[0], p[1], p[2]])
}

/// 追踪单像素在管线各阶段的值并打印。
fn trace_pixel(ctx: &FileContext, x: u32, y: u32, icc_settings: IccSettings) {
    let corr = &ctx.resolved.corr;
    let adj = build_manual_adjust(corr);
    let id_curves = identity_curves();
    let icc = ctx.icc_data.as_deref();
    let ref_icc = ctx.reference_icc.as_deref();
    let lut = ctx.film_lut.as_ref();

    println!("\n  ═════ pixel trace ({}, {}) ═════", x, y);

    let ref_px = pixel_at_buf(&ctx.reference_16, x, y);
    if let Some(r) = ref_px {
        println!("  参考像素      = [{:>5}, {:>5}, {:>5}]", r[0], r[1], r[2]);
    }
    println!();

    let raw_px = pixel_at(&ctx.raw_16, x, y);
    if let Some(p) = raw_px {
        println!("  [0] RAW                            = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
    }

    let after_film = color::apply_film_processing(&ctx.raw_16, corr);
    if let Some(p) = pixel_at(&after_film, x, y) {
        println!("  [1] apply_film_processing          = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
    }

    let after_scanner = color::apply_scanner_levels(&after_film, &adj, lut);
    if let Some(p) = pixel_at(&after_scanner, x, y) {
        println!("  [2] apply_scanner_levels           = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
    }

    // ICC step (via our ref-ICC aware path)
    let after_icc = if let (Some(in_icc), Some(out_icc)) = (icc, ref_icc) {
        match color::apply_icc_transform_profiles(&after_scanner, in_icc, out_icc, icc_settings) {
            Ok(img) => {
                if in_icc == out_icc {
                    println!("  [3] ICC (ref-ICC, 字节相等, 短路)  = [same as [2]]");
                } else {
                    let p = pixel_at(&img, x, y).unwrap_or([0, 0, 0]);
                    println!("  [3] ICC (in → ref-ICC)             = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
                }
                img
            }
            Err(e) => {
                println!("  [3] ICC 失败 ({})，回退 sRGB", e);
                if let Some(ic) = icc {
                    color::apply_icc_transform_ex(&after_scanner, ic, color::TargetColorSpace::SRGB, icc_settings)
                        .unwrap_or(after_scanner.clone())
                } else { after_scanner.clone() }
            }
        }
    } else if let Some(ic) = icc {
        let img = color::apply_icc_transform_ex(&after_scanner, ic, color::TargetColorSpace::SRGB, icc_settings)
            .unwrap_or(after_scanner.clone());
        let p = pixel_at(&img, x, y).unwrap_or([0, 0, 0]);
        println!("  [3] ICC (→ sRGB)                   = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
        img
    } else {
        after_scanner.clone()
    };

    // BW desaturate
    let after_bw = if adj.film_type == 2 {
        let img = desaturate_bw_local(&after_icc);
        if let Some(p) = pixel_at(&img, x, y) {
            println!("  [3b] desaturate_bw                 = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
        }
        img
    } else {
        after_icc
    };

    // Curves
    let curves_identity = id_curves.iter().all(|pts| {
        pts.len() == 2 && pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
    });
    let after_curves = if adj.apply_curves && id_curves.len() >= 7 && !curves_identity {
        let img = color::apply_gradation_curves(&after_bw, &id_curves);
        if let Some(p) = pixel_at(&img, x, y) {
            println!("  [4] apply_gradation_curves         = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
        }
        img
    } else {
        println!("  [4] apply_gradation_curves 跳过 (identity curves)");
        after_bw
    };

    // USM
    let after_usm = color::apply_usm(&after_curves, &adj);
    if adj.apply_usm && adj.usm_amount != 0 {
        if let Some(p) = pixel_at(&after_usm, x, y) {
            println!("  [5] apply_usm (amount={} radius={}) = [{:>5}, {:>5}, {:>5}]",
                adj.usm_amount, adj.usm_radius, p[0], p[1], p[2]);
        }
    } else {
        println!("  [5] apply_usm 跳过 (amount=0 or disabled)");
    }

    // Display adjust
    use fff_viewer::color::apply_display_adjust;
    let after_display = apply_display_adjust(&after_usm, &adj);
    if let Some(p) = pixel_at(&after_display, x, y) {
        println!("  [6] apply_display_adjust           = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
    }

    // Summary vs reference
    if let (Some(ours), Some(r)) = (pixel_at(&after_display, x, y), ref_px) {
        let d0 = ours[0] as i32 - r[0] as i32;
        let d1 = ours[1] as i32 - r[1] as i32;
        let d2 = ours[2] as i32 - r[2] as i32;
        println!();
        println!("  最终 ours     = [{:>5}, {:>5}, {:>5}]", ours[0], ours[1], ours[2]);
        println!("  参考         = [{:>5}, {:>5}, {:>5}]", r[0], r[1], r[2]);
        println!("  diff         = [{:>+5}, {:>+5}, {:>+5}]", d0, d1, d2);
        println!("  比例 ours/ref = [{:.3}, {:.3}, {:.3}]",
            ours[0] as f64 / r[0].max(1) as f64,
            ours[1] as f64 / r[1].max(1) as f64,
            ours[2] as f64 / r[2].max(1) as f64);
    }

    // ═════ flex::Pipeline trace（§34，T10 Phase 3+4）═════
    // 独立一路：直接从 XML 参数前向构造 LUT，与旧路径 T1 对比
    println!();
    println!("  ───── flex::Pipeline (§34) ─────");
    let flex_img = color::apply_flex_pipeline_no_icc(ctx.raw_16.clone(), corr);
    let flex_img_w_icc = color::apply_flex_pipeline(
        ctx.raw_16.clone(),
        corr,
        ref_icc,
        color::TargetColorSpace::SRGB,
        icc_settings,
    );
    if let Some(p) = pixel_at(&flex_img, x, y) {
        println!("  [F0] flex LUT 应用 (pre-ICC)       = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
    }
    if let Some(p) = pixel_at(&flex_img_w_icc, x, y) {
        println!("  [F1] flex + ICC(ref-ICC→sRGB)      = [{:>5}, {:>5}, {:>5}]", p[0], p[1], p[2]);
    }
    if let (Some(ours), Some(r)) = (pixel_at(&flex_img_w_icc, x, y), ref_px) {
        let d0 = ours[0] as i32 - r[0] as i32;
        let d1 = ours[1] as i32 - r[1] as i32;
        let d2 = ours[2] as i32 - r[2] as i32;
        println!("  flex diff vs ref                   = [{:>+5}, {:>+5}, {:>+5}]", d0, d1, d2);
    }

    // Scanner levels parameters at this pixel (just metadata)
    println!();
    println!("  管线参数:");
    println!("    film_type={} film_curve={} gamma={:.2}", corr.film_type, corr.film_curve, corr.gamma);
    println!("    shadow  ={:?}", corr.shadow);
    println!("    gray    ={:?}", corr.gray);
    println!("    highlight={:?}", corr.highlight);
    println!("    DotColor={:?}", corr.dot_color);
    println!("    ApplyHistogram={} ApplySliders={} ApplyCurves={} ApplyCC={} ApplyUSM={}",
        corr.apply_histogram, corr.apply_sliders, corr.apply_curves, corr.apply_cc, corr.apply_usm);
}

fn run_tests(
    ctx: &FileContext, worst_n: usize, keep_output_t1: bool,
    icc_settings: IccSettings, use_ref_lut: bool, flex_pipeline: bool,
) -> Vec<TestResult> {
    let corr = &ctx.resolved.corr;
    let adj = build_manual_adjust(corr);
    let id_curves = identity_curves();
    let after_film = color::apply_film_processing(&ctx.raw_16, corr);
    let icc = ctx.icc_data.as_deref();
    let ref_icc = ctx.reference_icc.as_deref();
    // LUT 选择：若 use_ref_lut 并且 ref_lut 存在，优先用 ref_lut；否则 film_lut（extracted）；都没有就 hardcoded
    let lut = if use_ref_lut {
        ctx.ref_lut.as_ref().or(ctx.film_lut.as_ref())
    } else {
        ctx.film_lut.as_ref()
    };

    let mut results = Vec::new();

    // T1 — 完整管线（LUT + ICC，ICC output 用 ref TIF 内嵌 profile 对齐）
    {
        let r = run_pipeline_with_output_icc(
            &after_film, &adj, corr, &id_curves,
            lut, icc, ref_icc, icc_settings);
        let label = if ref_icc.is_some() { "完整管线 (ref-ICC)" } else { "完整管线 (sRGB 回退)" };
        if let Some(res) = compare_images("T1", label,
            r, &ctx.reference_16, worst_n, keep_output_t1) {
            results.push(res);
        }
    }

    // T2 — 消融：关 ICC
    {
        let r = color::apply_color_pipeline_ex(
            after_film.clone(), &adj, &id_curves,
            lut, None, color::TargetColorSpace::SRGB, icc_settings);
        if let Some(res) = compare_images("T2", "消融: 无 ICC",
            r, &ctx.reference_16, 0, false) {
            results.push(res);
        }
    }

    // T3 — 消融：强制硬编码 LUT
    {
        let r = color::apply_color_pipeline_ex(
            after_film.clone(), &adj, &id_curves,
            None, icc, color::TargetColorSpace::SRGB, icc_settings);
        if let Some(res) = compare_images("T3", "消融: 硬编码 LUT",
            r, &ctx.reference_16, 0, false) {
            results.push(res);
        }
    }

    // T4 — 消融：完全关胶片曲线
    {
        let mut adj_no_fc = adj.clone();
        adj_no_fc.apply_film_curve = false;
        let r = color::apply_color_pipeline_ex(
            after_film.clone(), &adj_no_fc, &id_curves,
            None, icc, color::TargetColorSpace::SRGB, icc_settings);
        if let Some(res) = compare_images("T4", "消融: 无胶片曲线",
            r, &ctx.reference_16, 0, false) {
            results.push(res);
        }
    }

    // T6 — flex::Pipeline 新路径（§34 T10 Phase 3 集成测试）
    // 用 XML ImageCorrection 直接前向计算 LUT，跳过 scanner_levels + gradation + display_adjust
    // 与 T1 差异：T1 走"反推 LUT → 旧 pipeline"；T6 走"XML → flex curves → LUT"
    if flex_pipeline {
        // 架构：flex::Pipeline 已覆盖 scanner_levels + curves + display_adjust
        // 剩余步骤：ICC（若 input != ref）+ BW desat（若 film_type=2）+ USM
        let step1 = color::apply_flex_pipeline_no_icc(ctx.raw_16.clone(), corr);

        // BW (film_type=2): T37 直接 input_icc → Hasselblad Gray，跳过 sRGB 中间态
        let step2b = if adj.film_type == 2 {
            let gray_icc_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("profiles").join("Hasselblad Gray.icc");
            if let Ok(gray_data) = std::fs::read(&gray_icc_path) {
                color::desaturate_bw_via_gray_icc(&step1, icc, &gray_data)
            } else {
                desaturate_bw_local(&step1)
            }
        } else {
            // 非 BW：标准 ICC step
            if let (Some(in_icc), Some(out_icc)) = (icc, ref_icc) {
                match color::apply_icc_transform_profiles(&step1, in_icc, out_icc, icc_settings) {
                    Ok(img) => img,
                    Err(_) => color::apply_icc_transform_ex(
                        &step1, in_icc, color::TargetColorSpace::SRGB, icc_settings,
                    ).unwrap_or(step1),
                }
            } else if let Some(in_icc) = icc {
                color::apply_icc_transform_ex(
                    &step1, in_icc, color::TargetColorSpace::SRGB, icc_settings,
                ).unwrap_or(step1)
            } else {
                step1
            }
        };

        // USM（外部步骤，flex 暂未吸收）
        let step3 = color::apply_usm(&step2b, &adj);

        if let Some(res) = compare_images(
            "T6",
            "flex::Pipeline + ICC + BW + USM",
            step3,
            &ctx.reference_16,
            worst_n,
            false,
        ) {
            results.push(res);
        }
    }

    results
}

// ═══════════════════════════════════════════════════════════════════════════════
// Output formatting
// ═══════════════════════════════════════════════════════════════════════════════

fn count_grades(results: &[TestResult]) -> (usize, usize, usize, usize) {
    results.iter().fold((0, 0, 0, 0), |(s, p, w, f), r| match r.grade {
        Grade::Strict => (s + 1, p, w, f),
        Grade::Pass   => (s, p + 1, w, f),
        Grade::Warn   => (s, p, w + 1, f),
        Grade::Fail   => (s, p, w, f + 1),
    })
}

fn print_summary(results: &[TestResult]) {
    let (st, pa, wa, fa) = count_grades(results);
    println!("\n  SUMMARY: {} STRICT / {} PASS / {} WARN / {} FAIL ({} tests)\n",
        st, pa, wa, fa, results.len());

    println!("  {:>3} {:<28} {:>8} {:>6} {:>6} {:>7} {:>6} {:>6}",
        "ID", "Name", "MAE16", "P95_16", "P99_16", "Signed", "ΔE00", "");
    println!("  ─── ──────────────────────────── ──────── ────── ────── ─────── ────── ──────");
    for r in results {
        let s = &r.ch_all;
        println!("  {} {:>3} {:<28} {:>8.1} {:>6} {:>6} {:>+7.1} {:>6.2} {}",
            r.grade.icon(), r.id, r.name,
            s.mae_16, s.p95_16, s.p99_16,
            s.mean_signed, r.delta_e_mean, r.grade.label());
    }
}

fn print_detail(results: &[TestResult], find_worst: usize) {
    for r in results {
        println!("\n  ┌─── {} {} ─── [{}]", r.id, r.name, r.grade.label());
        println!("  │  {:>4} │ {:>9} │ {:>+7} │ {:>5} │ {:>6} │ {:>6} │ {:>6}",
            "Chan", "MAE16", "Signed", "P50", "P95", "P99", "Max");
        println!("  │  ─────┼───────────┼─────────┼───────┼────────┼────────┼────────");
        for (name, ch) in [("R", &r.ch_r), ("G", &r.ch_g), ("B", &r.ch_b), ("All", &r.ch_all)] {
            println!("  │  {:>4} │ {:>9.1} │ {:>+7.1} │ {:>5} │ {:>6} │ {:>6} │ {:>6}",
                name, ch.mae_16, ch.mean_signed,
                ch.p50_16, ch.p95_16, ch.p99_16, ch.max_16);
        }
        let a = &r.ch_all;
        println!("  │  bands MAE16: shadow={:.1}  mid={:.1}  highlight={:.1}",
            a.mae_shadow, a.mae_mid, a.mae_high);
        println!("  │  8-bit: MAE={:.2}  P99={}  PSNR={:.1}dB",
            a.mae_8, a.p99_8, a.psnr_8);
        println!("  │  ΔE2000 mean={:.2}  P95={:.2}", r.delta_e_mean, r.delta_e_p95);
        if find_worst > 0 && !r.worst.is_empty() {
            println!("  │  Worst {} pixels (by max channel diff):", r.worst.len());
            for w in r.worst.iter().take(find_worst) {
                println!("  │    ({:>4},{:>4}) err={:>5}  ours=[{:>5},{:>5},{:>5}]  ref=[{:>5},{:>5},{:>5}]",
                    w.x, w.y, w.err,
                    w.ours[0], w.ours[1], w.ours[2],
                    w.reference[0], w.reference[1], w.reference[2]);
            }
        }
        println!("  └────────────────────────────────────────────");
    }
}

fn print_json(case_name: &str, results: &[TestResult]) {
    print!("{{\"case\":\"{}\",\"tests\":[", case_name);
    for (i, r) in results.iter().enumerate() {
        let s = &r.ch_all;
        if i > 0 { print!(","); }
        print!("{{\"id\":\"{}\",\"name\":\"{}\",\
            \"mae_16\":{:.2},\"mean_signed\":{:.2},\
            \"p50_16\":{},\"p95_16\":{},\"p99_16\":{},\"p999_16\":{},\"max_16\":{},\
            \"mae_shadow\":{:.2},\"mae_mid\":{:.2},\"mae_high\":{:.2},\
            \"delta_e_mean\":{:.3},\"delta_e_p95\":{:.3},\
            \"grade\":\"{}\"}}",
            r.id, r.name,
            s.mae_16, s.mean_signed,
            s.p50_16, s.p95_16, s.p99_16, s.p999_16, s.max_16,
            s.mae_shadow, s.mae_mid, s.mae_high,
            r.delta_e_mean, r.delta_e_p95,
            r.grade.label());
    }
    println!("]}}");
}

fn print_meta_check(ctx: &FileContext, fff: &Path, refp: &Path) {
    println!("  ─── meta check ───");
    println!("  FFF:     {}", fff.display());
    println!("  Ref:     {}", refp.display());
    println!("  Ref ColorType: {:?}", ctx.reference_color_type);
    let ref_16bit = matches!(ctx.reference_color_type,
        image::ColorType::Rgb16 | image::ColorType::Rgba16 | image::ColorType::L16 | image::ColorType::La16);
    if !ref_16bit {
        println!("  ⚠  参考 TIF 非 16-bit，MAE16 数值是 8→16 扩展后的结果，参考价值下降");
    }
    println!("  Dims:    {:?}", ctx.reference_16.dimensions());
    println!("  Preset:  {}", ctx.resolved.label);
    println!("  ICC:     {}", if ctx.icc_data.is_some() { "yes" } else { "no" });
    println!("  Film LUT:{}", if ctx.film_lut.is_some() { "extracted" } else { "hardcoded" });
    let corr = &ctx.resolved.corr;
    println!("  Setting: film_type={} film_curve={} γ={:.2} colormodel={}",
        corr.film_type, corr.film_curve, corr.gamma, corr.color_model);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Manifest
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug)]
struct Case {
    name: String,
    fff: PathBuf,
    reference: PathBuf,
    preset: PresetSource,
}

fn resolve_path(base: &Path, p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() { pb } else { base.join(pb) }
}

fn load_manifest(manifest_path: &Path) -> Result<Vec<Case>, String> {
    let content = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("读取 manifest: {}", e))?;
    let value: toml::Value = toml::from_str(&content)
        .map_err(|e| format!("解析 manifest: {}", e))?;

    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let data_dir = value.get("data_dir").and_then(|v| v.as_str())
        .map(|s| resolve_path(manifest_dir, s))
        .ok_or_else(|| "manifest 缺少 data_dir".to_string())?;
    let preset_dir = value.get("preset_dir").and_then(|v| v.as_str())
        .map(|s| resolve_path(manifest_dir, s))
        .unwrap_or_else(|| manifest_dir.to_path_buf());

    let cases_val = value.get("case").and_then(|v| v.as_array())
        .ok_or_else(|| "manifest 缺少 [[case]]".to_string())?;

    let mut cases = Vec::new();
    for (idx, cv) in cases_val.iter().enumerate() {
        let t = cv.as_table().ok_or_else(|| format!("case #{} 不是 table", idx))?;
        let name = t.get("name").and_then(|v| v.as_str())
            .ok_or_else(|| format!("case #{} 缺 name", idx))?.to_string();
        let fff_rel = t.get("fff").and_then(|v| v.as_str())
            .ok_or_else(|| format!("case {} 缺 fff", name))?;
        let ref_rel = t.get("ref").and_then(|v| v.as_str())
            .ok_or_else(|| format!("case {} 缺 ref", name))?;
        let fff = resolve_path(&data_dir, fff_rel);
        let reference = resolve_path(&data_dir, ref_rel);

        let source = t.get("source").and_then(|v| v.as_str()).unwrap_or("embedded_current");
        let preset_val = t.get("preset").and_then(|v| v.as_str());

        let preset = match source {
            "embedded_current" => PresetSource::EmbeddedCurrent,
            "embedded_index" => {
                let n: usize = preset_val.ok_or_else(|| format!("case {} 缺 preset (index)", name))?
                    .parse().map_err(|_| format!("case {} preset 不是整数", name))?;
                PresetSource::EmbeddedIndex(n)
            }
            "embedded_name" => {
                let s = preset_val.ok_or_else(|| format!("case {} 缺 preset (name)", name))?;
                PresetSource::EmbeddedName(s.to_string())
            }
            "external_xml" => {
                let s = preset_val.ok_or_else(|| format!("case {} 缺 preset (xml)", name))?;
                PresetSource::ExternalXml(resolve_path(&preset_dir, s))
            }
            other => return Err(format!("case {} 未知 source: {}", name, other)),
        };

        cases.push(Case { name, fff, reference, preset });
    }
    Ok(cases)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Baseline JSON diff
// ═══════════════════════════════════════════════════════════════════════════════

/// 超简单的 JSON parser：只从我们自己输出的 NDJSON 里抽取 (case, id, mae_16)
/// 每行一个 case 对象。
fn load_baseline(path: &Path) -> HashMap<(String, String), f64> {
    let mut out = HashMap::new();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t, Err(_) => return out,
    };
    for line in text.lines() {
        // 粗略抽取，匹配 "case":"xxx" 和 {"id":"Tn", ..., "mae_16":v, ...}
        let case = extract_string_field(line, "\"case\"");
        if case.is_none() { continue; }
        let case = case.unwrap();
        for frag in line.split("{\"id\":").skip(1) {
            let id = extract_string_field_raw(&format!("\"id\":{}", frag), "\"id\"");
            let mae = extract_number_field(frag, "\"mae_16\"");
            if let (Some(id), Some(mae)) = (id, mae) {
                out.insert((case.clone(), id), mae);
            }
        }
    }
    out
}

fn extract_string_field(line: &str, key: &str) -> Option<String> {
    let i = line.find(key)?;
    let rest = &line[i + key.len()..];
    let q1 = rest.find('"')?;
    let q2 = rest[q1 + 1..].find('"')?;
    Some(rest[q1 + 1..q1 + 1 + q2].to_string())
}
fn extract_string_field_raw(line: &str, key: &str) -> Option<String> {
    extract_string_field(line, key)
}
fn extract_number_field(line: &str, key: &str) -> Option<f64> {
    let i = line.find(key)?;
    let rest = &line[i + key.len()..];
    let colon = rest.find(':')?;
    let tail = &rest[colon + 1..];
    let end = tail.find(|c: char| c == ',' || c == '}').unwrap_or(tail.len());
    tail[..end].trim().parse().ok()
}

fn fmt_diff(new: f64, old: Option<&f64>) -> String {
    match old {
        None => format!("{:>8.1}", new),
        Some(&o) => {
            let d = new - o;
            let arrow = if d.abs() < 0.05 { "=" }
                else if d < 0.0 { "↓" } else { "↑" };
            format!("{:>8.1} {}{:+.1}", new, arrow, d)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Single-file / dir / manifest runner
// ═══════════════════════════════════════════════════════════════════════════════

struct RunOptions {
    verbose: bool,
    json: bool,
    meta_check: bool,
    dump_errmap_dir: Option<PathBuf>,
    find_worst: usize,
    baseline: HashMap<(String, String), f64>,
    no_lut_extract: bool,
    icc_settings: IccSettings,
    trace_pixel: Option<(u32, u32)>,
    calibrate_usm: bool,
    use_ref_lut: bool,
    /// T10 Phase 3：启用 FlexColor flex::Pipeline 新路径（§34）
    flex_pipeline: bool,
}

/// 对 Y 做可分离高斯模糊（带 σ），返回 Y_blur
fn gaussian_blur_luma(y: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.1 {
        return y.to_vec();
    }
    let half = (3.0 * sigma).ceil().max(1.0) as i32;
    let len = (2 * half + 1) as usize;
    let two_sigma_sq = 2.0 * sigma * sigma;
    let mut kernel = Vec::with_capacity(len);
    let mut ksum = 0.0f32;
    for i in -half..=half {
        let v = (-((i * i) as f32) / two_sigma_sq).exp();
        kernel.push(v);
        ksum += v;
    }
    for v in &mut kernel { *v /= ksum; }
    let half_u = kernel.len() / 2;

    use rayon::prelude::*;
    let mut tmp = vec![0.0f32; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(yy, row)| {
        let rs = yy * w;
        for x in 0..w {
            let mut s = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let xi = (x as isize + ki as isize - half_u as isize).clamp(0, w as isize - 1) as usize;
                s += y[rs + xi] * kv;
            }
            row[x] = s;
        }
    });
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(yy, row)| {
        for x in 0..w {
            let mut s = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let yi = (yy as isize + ki as isize - half_u as isize).clamp(0, h as isize - 1) as usize;
                s += tmp[yi * w + x] * kv;
            }
            row[x] = s;
        }
    });
    out
}

/// USM 标定：枚举多个 σ，最小二乘拟合增益 k 使得 diff ≈ k·(Y_our − blur_σ(Y_our))。
fn calibrate_usm(ctx: &FileContext, icc_settings: IccSettings) {
    let corr = &ctx.resolved.corr;
    let adj = build_manual_adjust(corr);
    let icc = ctx.icc_data.as_deref();
    let ref_icc = ctx.reference_icc.as_deref();
    let lut = ctx.film_lut.as_ref();

    // 跑完管线到 ICC 之后，但不做 curves/USM/display_adjust（rgb_standard 这些都是 identity）
    let after_film = color::apply_film_processing(&ctx.raw_16, corr);
    let scanner_out = color::apply_scanner_levels(&after_film, &adj, lut);
    let pre_usm = if let (Some(in_icc), Some(out_icc)) = (icc, ref_icc) {
        color::apply_icc_transform_profiles(&scanner_out, in_icc, out_icc, icc_settings)
            .unwrap_or(scanner_out)
    } else {
        scanner_out
    };

    let ours_rgb = match &pre_usm {
        image::DynamicImage::ImageRgb16(b) => b.clone(),
        _ => { eprintln!("calibrate: 预期 Rgb16"); return; }
    };
    let w = ours_rgb.width() as usize;
    let h = ours_rgb.height() as usize;
    assert_eq!(ctx.reference_16.dimensions(), ours_rgb.dimensions());

    // 计算 luma 与 diff（基于 ref - ours 的亮度差）
    let ours = ours_rgb.as_raw();
    let refp = ctx.reference_16.as_raw();
    let mut y_ours = vec![0.0f32; w * h];
    let mut delta_y = vec![0.0f32; w * h];
    let mut diff_uniformity_sum = 0.0f64; // |dR-dG| + |dG-dB|
    for i in 0..w * h {
        let r_o = ours[i * 3] as f32;
        let g_o = ours[i * 3 + 1] as f32;
        let b_o = ours[i * 3 + 2] as f32;
        let r_r = refp[i * 3] as f32;
        let g_r = refp[i * 3 + 1] as f32;
        let b_r = refp[i * 3 + 2] as f32;
        y_ours[i] = (0.299 * r_o + 0.587 * g_o + 0.114 * b_o) / 65535.0;
        let dr = r_r - r_o;
        let dg = g_r - g_o;
        let db = b_r - b_o;
        // 用 luma 权重平均（如果是纯 luma USM，三通道应当近似相等）
        delta_y[i] = (0.299 * dr + 0.587 * dg + 0.114 * db) / 65535.0;
        diff_uniformity_sum += ((dr - dg).abs() + (dg - db).abs()) as f64;
    }
    let diff_non_uniformity_u16 = diff_uniformity_sum / (w as f64 * h as f64) / 2.0;

    println!("\n  ═════ USM 标定 ═════");
    println!("  case: {}", ctx.resolved.label);
    println!("  dims: {}×{}", w, h);
    println!("  channel 非一致性 mean|dR-dG|+|dG-dB|/2 = {:.2} u16", diff_non_uniformity_u16);
    if diff_non_uniformity_u16 > 200.0 {
        println!("  ⚠️  非一致性较高，可能不是纯 luma USM");
    } else {
        println!("  ✓ 三通道 diff 高度一致 → 确认 luma-based 操作");
    }

    // σ 扫描
    let sigmas = [0.5f32, 1.0, 2.0, 3.0, 5.0, 7.5, 10.0, 15.0, 20.0, 30.0, 50.0, 75.0, 100.0];
    println!("\n  {:>6} {:>8} {:>10} {:>10} {:>10}", "σ", "k (gain)", "residual", "R²", "hp_mag");
    println!("  ────── ──────── ────────── ────────── ──────────");

    let mut best: Option<(f32, f32, f64)> = None;
    for &sigma in &sigmas {
        let y_blur = gaussian_blur_luma(&y_ours, w, h, sigma);
        // high_pass = y - y_blur
        // fit k: minimize sum((delta - k*hp)^2) → k = sum(delta * hp) / sum(hp^2)
        let mut sum_dh = 0.0f64;
        let mut sum_hh = 0.0f64;
        let mut sum_dd = 0.0f64;
        for i in 0..y_ours.len() {
            let hp = y_ours[i] - y_blur[i];
            let d = delta_y[i];
            sum_dh += (d * hp) as f64;
            sum_hh += (hp * hp) as f64;
            sum_dd += (d * d) as f64;
        }
        let k = if sum_hh > 1e-20 { sum_dh / sum_hh } else { 0.0 };
        // residual normalized
        let mut rss = 0.0f64;
        for i in 0..y_ours.len() {
            let hp = y_ours[i] - y_blur[i];
            let r = delta_y[i] as f64 - k * hp as f64;
            rss += r * r;
        }
        let r_squared = 1.0 - rss / sum_dd.max(1e-20);
        let hp_mag = (sum_hh / (w * h) as f64).sqrt();
        println!("  {:>6.1} {:>8.4} {:>10.4e} {:>10.4} {:>10.4e}", sigma, k, rss, r_squared, hp_mag);
        if best.as_ref().map_or(true, |&(_, _, r)| rss < r) {
            best = Some((sigma, k as f32, rss));
        }
    }

    if let Some((s, k, r)) = best {
        println!("\n  最佳 σ = {:.1}, k = {:.4}, RSS = {:.4e}", s, k, r);
        // amount映射: flexcolor amount=X 对应实际 k。推算 divisor:
        //   k = amount / divisor  →  divisor = amount / k
        if corr.usm_amount != 0 {
            let divisor = corr.usm_amount as f32 / k;
            println!("  反推 FFF_USM_GAIN_DIVISOR = {:.1} (当前 amount={})", divisor, corr.usm_amount);
        }
    }
}

fn process_case(case_name: &str, fff: &Path, refp: &Path, preset: PresetSource, opts: &RunOptions) -> Vec<TestResult> {
    let ctx = match load_context(fff, refp, preset, opts.no_lut_extract) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("  ⚠ {}: {}", case_name, e);
            return Vec::new();
        }
    };

    if !opts.json {
        println!("\n═══════════════════════════════════════════════════");
        println!("  Case: {}", case_name);
        println!("  FFF:  {}", fff.file_name().unwrap_or_default().to_string_lossy());
        println!("  Ref:  {}", refp.file_name().unwrap_or_default().to_string_lossy());
        println!("  Preset: {}", ctx.resolved.label);
        println!("  LUT: {}  ICC: {}",
            if ctx.film_lut.is_some() { "extracted" } else { "hardcoded" },
            if ctx.icc_data.is_some() { "yes" } else { "no" });
        if opts.meta_check {
            print_meta_check(&ctx, fff, refp);
        }
        println!("═══════════════════════════════════════════════════");
    }

    if let Some((x, y)) = opts.trace_pixel {
        trace_pixel(&ctx, x, y, opts.icc_settings);
    }

    if opts.calibrate_usm {
        calibrate_usm(&ctx, opts.icc_settings);
        return Vec::new();
    }

    let keep_t1 = opts.dump_errmap_dir.is_some();
    let results = run_tests(&ctx, opts.find_worst, keep_t1, opts.icc_settings, opts.use_ref_lut, opts.flex_pipeline);

    // Error heatmap (T1 only)
    if let Some(dir) = &opts.dump_errmap_dir {
        if let Some(first) = results.first() {
            if let Some(out) = first.output_16.as_ref() {
                std::fs::create_dir_all(dir).ok();
                let path = dir.join(format!("{}_errmap.png", case_name));
                match dump_errmap(out, &ctx.reference_16, &path, 2000) {
                    Ok(_) => {
                        if !opts.json {
                            println!("  → heatmap: {}", path.display());
                        }
                    }
                    Err(e) => eprintln!("  ⚠ heatmap 失败: {}", e),
                }
            }
        }
    }

    if opts.json {
        print_json(case_name, &results);
    } else {
        // Print with optional baseline diff
        if opts.baseline.is_empty() {
            print_summary(&results);
        } else {
            print_summary_with_diff(case_name, &results, &opts.baseline);
        }
        if opts.verbose {
            print_detail(&results, opts.find_worst);
        }
    }

    results
}

fn print_summary_with_diff(
    case_name: &str, results: &[TestResult],
    baseline: &HashMap<(String, String), f64>,
) {
    let (st, pa, wa, fa) = count_grades(results);
    println!("\n  SUMMARY: {} STRICT / {} PASS / {} WARN / {} FAIL ({} tests)\n",
        st, pa, wa, fa, results.len());
    println!("  {:>3} {:<28} {:>17} {:>6} {:>6} {:>6}",
        "ID", "Name", "MAE16 (vs baseline)", "P95", "P99", "ΔE00");
    println!("  ─── ──────────────────────────── ───────────────── ────── ────── ──────");
    for r in results {
        let s = &r.ch_all;
        let key = (case_name.to_string(), r.id.clone());
        let diff = fmt_diff(s.mae_16, baseline.get(&key));
        println!("  {} {:>3} {:<28} {} {:>6} {:>6} {:>6.2}",
            r.grade.icon(), r.id, r.name, diff, s.p95_16, s.p99_16, r.delta_e_mean);
    }
}

fn find_file_pairs(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut pairs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let fff_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok()).map(|e| e.path())
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

// ═══════════════════════════════════════════════════════════════════════════════
// Main
// ═══════════════════════════════════════════════════════════════════════════════

fn print_usage() {
    eprintln!("用法:");
    eprintln!("  tif_compare <file.fff> <ref.tif> [--setting N | --setting-name NAME | --preset-xml PATH]");
    eprintln!("  tif_compare --dir <path>");
    eprintln!("  tif_compare --manifest <cases.toml>");
    eprintln!("  选项: -v --json --dump-errmap DIR --find-worst N --meta-check --baseline FILE.json --no-lut-extract");
}

fn main() {
    env_logger::init();
    let args: Vec<String> = env::args().collect();

    let mut verbose = false;
    let mut json_output = false;
    let mut meta_check = false;
    let mut dir_path:   Option<String> = None;
    let mut manifest:   Option<String> = None;
    let mut preset_idx: Option<usize>  = None;
    let mut preset_name: Option<String> = None;
    let mut preset_xml:  Option<String> = None;
    let mut errmap_dir:  Option<String> = None;
    let mut find_worst: usize = 0;
    let mut baseline_path: Option<String> = None;
    let mut no_lut_extract = false;
    let mut icc_intent = IccIntent::Perceptual;
    let mut icc_bpc = false;
    let mut trace_pixel: Option<(u32, u32)> = None;
    let mut calibrate_usm_flag = false;
    let mut use_ref_lut_flag = false;
    let mut flex_pipeline_flag = false;
    let mut positional = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--verbose" => verbose = true,
            "--json" => json_output = true,
            "--meta-check" => meta_check = true,
            "--no-lut-extract" => no_lut_extract = true,
            "--icc-intent" => {
                i += 1;
                if let Some(s) = args.get(i) {
                    icc_intent = IccIntent::from_str(s).unwrap_or_else(|| {
                        eprintln!("未知 intent: {} (用 perceptual|relative|absolute|saturation)", s);
                        std::process::exit(2);
                    });
                }
            }
            "--icc-no-bpc" => icc_bpc = false,
            "--icc-bpc"    => icc_bpc = true,
            "--trace" => {
                i += 1;
                if let Some(s) = args.get(i) {
                    let parts: Vec<&str> = s.split(',').collect();
                    if parts.len() == 2 {
                        if let (Ok(x), Ok(y)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                            trace_pixel = Some((x, y));
                        } else {
                            eprintln!("--trace 需要 x,y 整数，例如 --trace 2139,34");
                            std::process::exit(2);
                        }
                    } else {
                        eprintln!("--trace 需要 x,y 格式，例如 --trace 2139,34");
                        std::process::exit(2);
                    }
                }
            }
            "--calibrate-usm" => calibrate_usm_flag = true,
            "--use-ref-lut" => use_ref_lut_flag = true,
            "--flex-pipeline" => flex_pipeline_flag = true,
            "--dir" =>          { i += 1; dir_path    = args.get(i).cloned(); }
            "--manifest" =>     { i += 1; manifest    = args.get(i).cloned(); }
            "--setting" =>      { i += 1; preset_idx  = args.get(i).and_then(|s| s.parse().ok()); }
            "--setting-name" => { i += 1; preset_name = args.get(i).cloned(); }
            "--preset-xml" =>   { i += 1; preset_xml  = args.get(i).cloned(); }
            "--dump-errmap" =>  { i += 1; errmap_dir  = args.get(i).cloned(); }
            "--find-worst" =>   { i += 1; find_worst  = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0); }
            "--baseline" =>     { i += 1; baseline_path = args.get(i).cloned(); }
            "-h" | "--help" => { print_usage(); return; }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    let baseline = baseline_path.as_ref()
        .map(|p| load_baseline(Path::new(p)))
        .unwrap_or_default();

    let opts = RunOptions {
        verbose, json: json_output, meta_check,
        dump_errmap_dir: errmap_dir.map(PathBuf::from),
        find_worst, baseline,
        no_lut_extract,
        icc_settings: IccSettings { intent: icc_intent, black_point_compensation: icc_bpc },
        trace_pixel,
        calibrate_usm: calibrate_usm_flag,
        use_ref_lut: use_ref_lut_flag,
        flex_pipeline: flex_pipeline_flag,
    };

    // Build single preset source from CLI (for single-file / dir mode)
    let cli_preset = if let Some(xml) = &preset_xml {
        PresetSource::ExternalXml(PathBuf::from(xml))
    } else if let Some(name) = &preset_name {
        PresetSource::EmbeddedName(name.clone())
    } else if let Some(idx) = preset_idx {
        PresetSource::EmbeddedIndex(idx)
    } else {
        PresetSource::EmbeddedCurrent
    };

    // Dispatch
    let mut all_results: Vec<(String, Vec<TestResult>)> = Vec::new();

    if let Some(mp) = manifest {
        let path = PathBuf::from(&mp);
        let cases = match load_manifest(&path) {
            Ok(c) => c,
            Err(e) => { eprintln!("manifest 加载失败: {}", e); std::process::exit(1); }
        };
        if !opts.json {
            println!("Loaded {} cases from {}", cases.len(), path.display());
        }
        for c in &cases {
            let results = process_case(&c.name, &c.fff, &c.reference, c.preset.clone(), &opts);
            if !results.is_empty() {
                all_results.push((c.name.clone(), results));
            }
        }
    } else if let Some(dir) = dir_path {
        let pairs = find_file_pairs(Path::new(&dir));
        if pairs.is_empty() {
            eprintln!("在 {} 未找到 .fff + .tif 配对", dir);
            std::process::exit(1);
        }
        for (fff, tif) in &pairs {
            let case_name = fff.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let results = process_case(&case_name, fff, tif, cli_preset.clone(), &opts);
            if !results.is_empty() {
                all_results.push((case_name, results));
            }
        }
    } else if positional.len() >= 2 {
        let fff = PathBuf::from(&positional[0]);
        let tif = PathBuf::from(&positional[1]);
        let case_name = fff.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let results = process_case(&case_name, &fff, &tif, cli_preset, &opts);
        if !results.is_empty() {
            all_results.push((case_name, results));
        }
    } else {
        print_usage();
        std::process::exit(1);
    }

    // Grand summary for multi-case
    if all_results.len() > 1 && !opts.json {
        println!("\n\n═══════════════════════════════════════════════════════════════════");
        println!("  GRAND SUMMARY");
        println!("═══════════════════════════════════════════════════════════════════");
        println!("  {:<30} {:>5} {:>6} {:>6} {:>6} {:>6} {:>9} {:>7}",
            "CASE", "TESTS", "STRICT", "PASS", "WARN", "FAIL", "T1_MAE16", "T1_ΔE");
        println!("  ────────────────────────────── ───── ────── ────── ────── ────── ───────── ───────");
        let (mut tst, mut tpa, mut twa, mut tfa, mut tot) = (0usize, 0, 0, 0, 0);
        for (name, results) in &all_results {
            let (st, pa, wa, fa) = count_grades(results);
            let (t1_mae, t1_de) = results.first()
                .map(|r| (r.ch_all.mae_16, r.delta_e_mean))
                .unwrap_or((f64::NAN, f64::NAN));
            println!("  {:<30} {:>5} {:>6} {:>6} {:>6} {:>6} {:>9.1} {:>7.2}",
                name, results.len(), st, pa, wa, fa, t1_mae, t1_de);
            tst += st; tpa += pa; twa += wa; tfa += fa; tot += results.len();
        }
        println!("  ────────────────────────────── ───── ────── ────── ────── ────── ───────── ───────");
        println!("  {:<30} {:>5} {:>6} {:>6} {:>6} {:>6}",
            "TOTAL", tot, tst, tpa, twa, tfa);
    }

    // Exit code: FAIL if any test FAIL; otherwise 0
    let any_fail = all_results.iter().any(|(_, rs)| rs.iter().any(|r| r.grade == Grade::Fail));
    if any_fail {
        std::process::exit(1);
    }
}
