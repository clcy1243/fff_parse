use std::env;
use std::path::Path;

use fff_viewer::color::{self, ManualAdjust, TargetColorSpace};
use fff_viewer::flexcolor::EditHistory;
use fff_viewer::tiff::TiffFile;
use image::imageops::FilterType;

#[derive(Clone)]
struct Metrics {
    mae_r: f64,
    mae_g: f64,
    mae_b: f64,
    bias_r: f64,
    bias_g: f64,
    bias_b: f64,
}

impl Metrics {
    fn mae_all(&self) -> f64 {
        (self.mae_r + self.mae_g + self.mae_b) / 3.0
    }
}

#[derive(Clone, Copy)]
struct AffineFit {
    gain: f64,
    offset: f64,
    mae: f64,
}

fn build_usm_adjust(corr: &fff_viewer::flexcolor::ImageCorrection) -> ManualAdjust {
    let mut adj = ManualAdjust::default();
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
    }
    adj
}

fn compare_rgb16(ours: &image::RgbImage, reference: &image::RgbImage) -> Metrics {
    let (w, h) = (
        ours.width().min(reference.width()),
        ours.height().min(reference.height()),
    );
    let mut abs = [0f64; 3];
    let mut signed = [0f64; 3];
    let mut n = 0f64;

    for y in 0..h {
        for x in 0..w {
            let o = ours.get_pixel(x, y);
            let r = reference.get_pixel(x, y);
            for ch in 0..3 {
                let d = o[ch] as f64 - r[ch] as f64;
                abs[ch] += d.abs();
                signed[ch] += d;
            }
            n += 1.0;
        }
    }

    Metrics {
        mae_r: abs[0] / n,
        mae_g: abs[1] / n,
        mae_b: abs[2] / n,
        bias_r: signed[0] / n,
        bias_g: signed[1] / n,
        bias_b: signed[2] / n,
    }
}

fn fit_affine_channel(ours: &image::RgbImage, reference: &image::RgbImage, ch: usize) -> AffineFit {
    let (w, h) = (
        ours.width().min(reference.width()),
        ours.height().min(reference.height()),
    );
    let mut n = 0.0f64;
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_xx = 0.0f64;
    let mut sum_xy = 0.0f64;

    for y in 0..h {
        for x in 0..w {
            let ox = ours.get_pixel(x, y)[ch] as f64;
            let ry = reference.get_pixel(x, y)[ch] as f64;
            n += 1.0;
            sum_x += ox;
            sum_y += ry;
            sum_xx += ox * ox;
            sum_xy += ox * ry;
        }
    }

    let denom = n * sum_xx - sum_x * sum_x;
    let gain = if denom.abs() < 1e-9 {
        1.0
    } else {
        (n * sum_xy - sum_x * sum_y) / denom
    };
    let offset = (sum_y - gain * sum_x) / n.max(1.0);

    let mut mae = 0.0f64;
    for y in 0..h {
        for x in 0..w {
            let ox = ours.get_pixel(x, y)[ch] as f64;
            let ry = reference.get_pixel(x, y)[ch] as f64;
            let fitted = (gain * ox + offset).clamp(0.0, 255.0);
            mae += (fitted - ry).abs();
        }
    }
    mae /= n.max(1.0);

    AffineFit { gain, offset, mae }
}

fn render_candidate(
    img: image::DynamicImage,
    corr: &fff_viewer::flexcolor::ImageCorrection,
    input_icc: Option<&[u8]>,
) -> image::DynamicImage {
    let step1 = color::apply_flex_pipeline_no_icc(img, corr);
    let step2 = if let Some(in_icc) = input_icc {
        color::apply_icc_transform_ex(&step1, in_icc, TargetColorSpace::SRGB, Default::default())
            .unwrap_or(step1)
    } else {
        step1
    };
    let usm = build_usm_adjust(corr);
    let step3 = color::apply_usm(&step2, &usm);
    if let Some(cal) = color::negative_c41_calibration(corr) {
        color::apply_affine_calibration(&step3, &cal)
    } else {
        step3
    }
}

fn render_candidate_icc_first(
    img: image::DynamicImage,
    corr: &fff_viewer::flexcolor::ImageCorrection,
    input_icc: Option<&[u8]>,
) -> image::DynamicImage {
    let stage0 = if let Some(in_icc) = input_icc {
        color::apply_icc_transform_ex(&img, in_icc, TargetColorSpace::SRGB, Default::default())
            .unwrap_or(img)
    } else {
        img
    };
    let stage1 = color::apply_flex_pipeline_no_icc(stage0, corr);
    let usm = build_usm_adjust(corr);
    let step2 = color::apply_usm(&stage1, &usm);
    if let Some(cal) = color::negative_c41_calibration(corr) {
        color::apply_affine_calibration(&step2, &cal)
    } else {
        step2
    }
}

fn estimate_film_base(img: &image::DynamicImage) -> [f64; 3] {
    let rgb = img.to_rgb16();
    let (w, h) = rgb.dimensions();
    let border = ((w.min(h) / 50).max(8)) as u32; // ~2%
    let stride = 8u32;
    let mut samples = [Vec::<u16>::new(), Vec::<u16>::new(), Vec::<u16>::new()];

    for y in (0..h).step_by(stride as usize) {
        for x in (0..w).step_by(stride as usize) {
            let on_border = x < border || x >= w.saturating_sub(border) || y < border || y >= h.saturating_sub(border);
            if !on_border {
                continue;
            }
            let p = rgb.get_pixel(x, y);
            for ch in 0..3 {
                samples[ch].push(p[ch]);
            }
        }
    }

    let mut out = [0.0f64; 3];
    for ch in 0..3 {
        if samples[ch].is_empty() {
            continue;
        }
        samples[ch].sort_unstable();
        let idx = ((samples[ch].len() - 1) as f64 * 0.90).round() as usize;
        out[ch] = samples[ch][idx] as f64;
    }
    out
}

fn apply_channel_gains(img: image::DynamicImage, gains: [f64; 3]) -> image::DynamicImage {
    match img {
        image::DynamicImage::ImageRgb16(mut rgb16) => {
            for p in rgb16.pixels_mut() {
                for ch in 0..3 {
                    let v = (p[ch] as f64 * gains[ch]).round().clamp(0.0, 65535.0) as u16;
                    p[ch] = v;
                }
            }
            image::DynamicImage::ImageRgb16(rgb16)
        }
        other => other,
    }
}

fn resize_ref_to_match(
    reference: &image::DynamicImage,
    width: u32,
    height: u32,
) -> image::DynamicImage {
    if reference.width() == width && reference.height() == height {
        reference.clone()
    } else {
        reference.resize_exact(width, height, FilterType::Triangle)
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: cargo run --release --example ifd_probe -- <file.fff> <reference.tif>");
        std::process::exit(1);
    }

    let fff_path = &args[1];
    let ref_path = &args[2];

    let reference = image::open(ref_path)
        .expect("Cannot open reference image")
        .to_rgb8();
    let reference_dyn = image::DynamicImage::ImageRgb8(reference.clone());

    let tiff = TiffFile::open(Path::new(fff_path)).expect("Cannot open FFF");
    let history = EditHistory::parse_from_tiff(&tiff).expect("Cannot parse edit history");
    let idx = history.current_index.min(history.settings.len() - 1);
    let corr = &history.settings[idx].correction;

    let all_tags = tiff.all_tags();
    let profiles_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
    let icc_data = color::extract_embedded_icc(tiff.raw_data(), &all_tags)
        .or_else(|| {
            corr.input_profile_name.as_deref().and_then(|name| {
                std::fs::read(profiles_dir.join(format!("{}.icc", name)))
                    .ok()
                    .or_else(|| std::fs::read(profiles_dir.join(format!("{}.icm", name))).ok())
            })
        })
        .or_else(|| std::fs::read(profiles_dir.join("Flextight X5 & 949.icc")).ok());

    println!("═══ IFD Probe ═══");
    println!("FFF: {}", fff_path);
    println!("Ref: {}", ref_path);
    println!(
        "Current setting: #{} '{}' film_type={} gamma={:.2}",
        idx,
        history.settings[idx].name,
        corr.film_type,
        corr.gamma
    );
    println!("ICC: {}", if icc_data.is_some() { "yes" } else { "no" });
    println!();

    let mut candidates: Vec<(String, image::DynamicImage)> = Vec::new();

    for (ifd_idx, ifd) in tiff.ifds.iter().enumerate() {
        let width = ifd.get_u32(0x0100).unwrap_or(0);
        let height = ifd.get_u32(0x0101).unwrap_or(0);
        let bps = ifd.get_u32(0x0102).unwrap_or(8);
        let compression = ifd.get_u32(0x0103).unwrap_or(1);
        let photometric = ifd.get_u32(0x0106).unwrap_or(0);
        let spp = ifd.get_u32(0x0115).unwrap_or(1);
        let subfile = ifd.get_u32(0x00FE).unwrap_or(0);
        if compression == 1 && photometric == 2 && spp >= 3 && width > 0 && height > 0 {
            if let Some(img) = tiff.decode_uncompressed_rgb(ifd) {
                candidates.push((
                    format!("IFD#{} {}x{} {}bit subfile={}", ifd_idx, width, height, bps, subfile),
                    img,
                ));
            }
        }
    }

    if let Some(img) = tiff.decode_for_export() {
        candidates.push((
            format!("decode_for_export {}x{}", img.width(), img.height()),
            img,
        ));
    }

    for (label, candidate) in candidates {
        let rendered = render_candidate(candidate.clone(), corr, icc_data.as_deref());
        let ref_match = resize_ref_to_match(&reference_dyn, rendered.width(), rendered.height()).to_rgb8();
        let ours = rendered.to_rgb8();
        let m = compare_rgb16(&ours, &ref_match);
        let fit_r = fit_affine_channel(&ours, &ref_match, 0);
        let fit_g = fit_affine_channel(&ours, &ref_match, 1);
        let fit_b = fit_affine_channel(&ours, &ref_match, 2);
        println!(
            "{:<36} MAE={:6.2}  R={:6.2} G={:6.2} B={:6.2}  Bias=({:+.1},{:+.1},{:+.1})",
            label,
            m.mae_all(),
            m.mae_r,
            m.mae_g,
            m.mae_b,
            m.bias_r,
            m.bias_g,
            m.bias_b
        );
        println!(
            "  affine-fit                         R(g={:.4}, b={:+.2}, mae={:.2})  G(g={:.4}, b={:+.2}, mae={:.2})  B(g={:.4}, b={:+.2}, mae={:.2})",
            fit_r.gain, fit_r.offset, fit_r.mae,
            fit_g.gain, fit_g.offset, fit_g.mae,
            fit_b.gain, fit_b.offset, fit_b.mae,
        );
        if icc_data.is_some() {
            let alt = render_candidate_icc_first(candidate.clone(), corr, icc_data.as_deref());
            let alt_ours = alt.to_rgb8();
            let alt_m = compare_rgb16(&alt_ours, &ref_match);
            println!(
                "  icc-first                          MAE={:6.2}  R={:6.2} G={:6.2} B={:6.2}  Bias=({:+.1},{:+.1},{:+.1})",
                alt_m.mae_all(),
                alt_m.mae_r,
                alt_m.mae_g,
                alt_m.mae_b,
                alt_m.bias_r,
                alt_m.bias_g,
                alt_m.bias_b
            );
        }
        let base = estimate_film_base(&candidate);
        if base[0] > 0.0 && base[1] > 0.0 && base[2] > 0.0 {
            let ref_base = base[1];
            let gains = [ref_base / base[0], 1.0, ref_base / base[2]];
            let balanced = render_candidate(apply_channel_gains(candidate.clone(), gains), corr, icc_data.as_deref());
            let balanced_ours = balanced.to_rgb8();
            let balanced_m = compare_rgb16(&balanced_ours, &ref_match);
            println!(
                "  base-balanced                       base=({:.0},{:.0},{:.0}) gain=({:.4},{:.4},{:.4}) MAE={:6.2}  Bias=({:+.1},{:+.1},{:+.1})",
                base[0], base[1], base[2],
                gains[0], gains[1], gains[2],
                balanced_m.mae_all(),
                balanced_m.bias_r,
                balanced_m.bias_g,
                balanced_m.bias_b
            );
        }
    }
}
