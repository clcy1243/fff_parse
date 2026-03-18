// ─── Film Type Processing ───────────────────────────────────────────────────

use crate::flexcolor::ImageCorrection;

/// Compute per-channel auto shadow/highlight for negative film by analyzing
/// the raw image histogram. Excludes saturated pixels (film borders, sprocket
/// holes) to find the actual image content range per channel.
///
/// Returns per-channel (shadow, highlight) in 16-bit scale for the INVERTED
/// (positive) data space.
fn compute_neg_auto_levels_16(src: &[u16], width: usize, height: usize) -> ([f32; 3], [f32; 3]) {
    use rayon::prelude::*;

    const BINS: usize = 4096;
    const SHIFT: u32 = 4; // 16-bit >> 4 = 4096 bins
    const SATURATION_THRESHOLD: u16 = 45000;
    const BASE_PCT: f64 = 0.995; // film base = 99.5th percentile of image content
    const DEEP_PCT: f64 = 0.005; // deep shadow = 0.5th percentile

    // Build per-channel histograms in parallel by row
    // Use Vec to avoid stack overflow on rayon worker threads (3×4096×4 = 48KB per thread)
    let row_len = width * 3;
    let num_rows = height;

    let (hist_r, hist_g, hist_b, total) = (0..num_rows)
        .into_par_iter()
        .fold(
            || (vec![0u32; BINS], vec![0u32; BINS], vec![0u32; BINS], 0u64),
            |mut acc, y| {
                let start = y * row_len;
                for x in 0..width {
                    let si = start + x * 3;
                    let r = src[si];
                    let g = src[si + 1];
                    let b = src[si + 2];
                    // Exclude saturated pixels (borders, sprocket holes)
                    if r < SATURATION_THRESHOLD && g < SATURATION_THRESHOLD && b < SATURATION_THRESHOLD {
                        acc.0[(r >> SHIFT) as usize] += 1;
                        acc.1[(g >> SHIFT) as usize] += 1;
                        acc.2[(b >> SHIFT) as usize] += 1;
                        acc.3 += 1;
                    }
                }
                acc
            },
        )
        .reduce(
            || (vec![0u32; BINS], vec![0u32; BINS], vec![0u32; BINS], 0u64),
            |mut a, b| {
                for i in 0..BINS {
                    a.0[i] += b.0[i];
                    a.1[i] += b.1[i];
                    a.2[i] += b.2[i];
                }
                a.3 += b.3;
                a
            },
        );

    if total == 0 {
        return ([0.0; 3], [65535.0; 3]);
    }

    let find_pct = |hist: &[u32], pct: f64| -> f32 {
        let target = (total as f64 * pct) as u64;
        let mut count = 0u64;
        for i in 0..BINS {
            count += hist[i] as u64;
            if count >= target {
                return ((i as u32) << SHIFT) as f32;
            }
        }
        65535.0
    };

    // Film base = highest raw values among image content (per channel)
    let base_r = find_pct(&hist_r, BASE_PCT);
    let base_g = find_pct(&hist_g, BASE_PCT);
    let base_b = find_pct(&hist_b, BASE_PCT);

    // Deep shadow = lowest raw values among image content (per channel)
    let deep_r = find_pct(&hist_r, DEEP_PCT);
    let deep_g = find_pct(&hist_g, DEEP_PCT);
    let deep_b = find_pct(&hist_b, DEEP_PCT);

    // Convert to inverted (positive) space
    let shadow = [65535.0 - base_r, 65535.0 - base_g, 65535.0 - base_b];
    let highlight = [65535.0 - deep_r, 65535.0 - deep_g, 65535.0 - deep_b];

    log::info!(
        "Auto-levels: film_base raw R={:.0},G={:.0},B={:.0}, deep R={:.0},G={:.0},B={:.0} \
         → inv shadow R={:.0},G={:.0},B={:.0}, highlight R={:.0},G={:.0},B={:.0} ({}px, {} excluded)",
        base_r, base_g, base_b, deep_r, deep_g, deep_b,
        shadow[0], shadow[1], shadow[2], highlight[0], highlight[1], highlight[2],
        total, width as u64 * height as u64 - total,
    );

    (shadow, highlight)
}

// ─── Film curve LUT ─────────────────────────────────────────────────────────
// Empirical per-channel tone curves for FlexColor FilmCurve=4, Gamma=2.
// These were reverse-engineered from pixel-level comparison of the 16-bit raw
// processing pipeline against FlexColor's pre-rendered 8-bit thumbnails across
// multiple Portra 160 scans on a Flextight X5.
//
// Maps linear per-channel levels output [0.0–1.0] → display value [0–255].
// Encapsulates: film response curve + ICC transform (Flextight Input → sRGB) +
// Gamma encoding.  Applied AFTER shadow/highlight levels but WITHOUT per-channel
// gray midtone gamma (the gray shift is minor and already baked into the average).

const FILM_CURVE_LUT_R: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   1,   1,   1,   1,   1,   1,   2,   2,   2,   3,   4,   5,   5,   5,
      6,   7,   8,   9,  10,  11,  12,  13,  13,  14,  14,  15,  16,  17,  17,  19,
     21,  24,  26,  28,  30,  32,  34,  36,  38,  40,  41,  41,  41,  41,  41,  41,
     41,  41,  41,  41,  41,  41,  41,  41,  44,  46,  47,  49,  50,  52,  54,  57,
     59,  61,  63,  65,  67,  68,  69,  71,  73,  75,  76,  77,  78,  80,  82,  83,
     85,  87,  89,  91,  92,  94,  96,  97,  99, 101, 104, 106, 108, 111, 113, 115,
    117, 119, 121, 124, 126, 128, 130, 133, 135, 137, 139, 141, 144, 146, 148, 150,
    152, 154, 156, 158, 159, 162, 164, 167, 169, 172, 174, 177, 180, 182, 185, 187,
    190, 192, 195, 197, 199, 201, 203, 205, 207, 209, 211, 212, 213, 213, 214, 215,
    216, 217, 219, 220, 221, 223, 225, 227, 228, 230, 233, 236, 237, 238, 244, 253,
];

const FILM_CURVE_LUT_G: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   1,   1,   1,   1,   1,   1,   1,   1,
      1,   1,   1,   1,   1,   1,   1,   1,   1,   1,   1,   2,   2,   2,   2,   2,
      2,   2,   2,   2,   2,   2,   3,   3,   3,   3,   3,   3,   3,   4,   4,   4,
      4,   4,   4,   4,   5,   5,   5,   6,   6,   7,   7,   7,   8,   9,  10,  12,
     13,  14,  16,  17,  18,  20,  21,  23,  24,  26,  28,  30,  31,  33,  35,  36,
     38,  39,  41,  43,  44,  46,  48,  50,  52,  54,  55,  57,  59,  60,  62,  63,
     65,  67,  69,  71,  72,  74,  76,  78,  80,  81,  83,  85,  88,  90,  92,  94,
     96,  99, 101, 103, 105, 107, 110, 112, 114, 116, 118, 120, 122, 124, 127, 129,
    131, 134, 137, 139, 142, 145, 148, 150, 153, 156, 159, 162, 164, 167, 170, 172,
    175, 178, 180, 183, 185, 188, 191, 193, 196, 199, 202, 205, 207, 210, 213, 215,
    218, 221, 225, 228, 231, 234, 236, 239, 242, 245, 248, 251, 253, 254, 254, 254,
    254, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
];

const FILM_CURVE_LUT_B: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   1,   1,   2,   2,
      3,   3,   4,   5,   5,   6,   7,   7,   8,   9,  10,  11,  12,  13,  14,  15,
     17,  18,  20,  21,  23,  25,  26,  28,  30,  32,  33,  35,  37,  39,  41,  43,
     45,  47,  49,  51,  52,  54,  56,  58,  59,  61,  63,  64,  66,  67,  69,  71,
     73,  75,  77,  79,  81,  83,  86,  88,  90,  92,  94,  96,  98, 100, 102, 105,
    107, 109, 112, 114, 117, 119, 122, 125, 127, 130, 133, 136, 139, 142, 145, 148,
    150, 154, 157, 160, 163, 166, 170, 173, 176, 179, 182, 186, 189, 192, 195, 198,
    201, 204, 207, 210, 213, 216, 220, 223, 226, 229, 233, 236, 239, 242, 245, 247,
    250, 252, 253, 253, 254, 254, 254, 254, 255, 255, 255, 255, 255, 255, 255, 255,
    255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
];

/// Linearly interpolate a 256-entry LUT for a floating-point input in [0, 1].
/// Returns 16-bit value (0–65535).
#[inline]
fn lut_interp_16(val: f32, lut: &[u8; 256]) -> f32 {
    let x = val * 255.0;
    let lo = (x as usize).min(254);
    let hi = lo + 1;
    let frac = x - lo as f32;
    let out = lut[lo] as f32 * (1.0 - frac) + lut[hi] as f32 * frac;
    out * 257.0 // scale 0-255 → 0-65535
}

/// Linearly interpolate a 256-entry LUT for a floating-point input in [0, 1].
/// Returns 8-bit value (0–255) as f32.
#[inline]
fn lut_interp_8(val: f32, lut: &[u8; 256]) -> f32 {
    let x = val * 255.0;
    let lo = (x as usize).min(254);
    let hi = lo + 1;
    let frac = x - lo as f32;
    lut[lo] as f32 * (1.0 - frac) + lut[hi] as f32 * frac
}

/// Apply film type processing: negative inversion + per-channel levels.
///
/// For negative film (FilmType=1, C-41): invert pixels then remap per-channel.
/// When `remove_cast_shadow`/`remove_cast_highlight` are enabled, automatically
/// computes per-channel shadow/highlight from the image histogram to remove the
/// orange mask color cast. The preset's Gray values still control midtone gamma.
///
/// For B&W negative (FilmType=2): same inversion + convert to grayscale.
/// For positive film (FilmType=0): only apply levels adjustment using preset values.
///
/// When FilmCurve=4 and Gamma=2 (typical Portra/color negative), applies an
/// empirical per-channel tone curve that matches FlexColor's rendering.
///
/// Preserves bit depth: 16-bit input → 16-bit output, 8-bit → 8-bit.
/// Uses rayon for parallel row processing on large images.
pub fn apply_film_processing(
    img: &image::DynamicImage,
    correction: &ImageCorrection,
) -> image::DynamicImage {
    use rayon::prelude::*;

    let film_type = correction.film_type;
    let is_negative = film_type == 1 || film_type == 2;

    let shadow = correction.shadow;
    let highlight = correction.highlight;
    let gray = correction.gray;

    // Detect if preset has meaningful per-channel levels or just defaults/zeros.
    // FlexColor saves auto-computed levels into the active correction when the
    // user processes a negative. If the per-channel values (R,G,B = indices 1,2,3)
    // are all identical, the preset is unprocessed and we need auto-computation.
    let needs_auto_levels = is_negative
        && (correction.remove_cast_shadow || correction.remove_cast_highlight)
        && shadow[1] == shadow[2] && shadow[2] == shadow[3]
        && highlight[1] == highlight[2] && highlight[2] == highlight[3];

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            const SCALE: f32 = 4.0;
            const MAX_VAL: f32 = 65535.0;

            let src = rgb16.as_raw();

            // Compute per-channel level params.
            // If the active correction already has per-channel values (FlexColor
            // saved auto-computed levels), use them directly. Otherwise, compute
            // from the image histogram to remove the negative film orange mask.
            let (auto_shadow, auto_highlight) = if needs_auto_levels {
                log::info!("Preset has identical per-channel levels, computing auto-levels");
                compute_neg_auto_levels_16(src, w as usize, h as usize)
            } else {
                ([0.0f32; 3], [0.0f32; 3])
            };

            let mut ch_s = [0.0f32; 4];
            let mut ch_range = [1.0f32; 4];
            let mut ch_gamma = [1.0f32; 4];

            if needs_auto_levels {
                // Master channel: use preset values
                ch_s[0] = (shadow[0] as f32 * SCALE).clamp(0.0, MAX_VAL);
                let h0 = (highlight[0] as f32 * SCALE).clamp(1.0, MAX_VAL);
                ch_range[0] = (h0 - ch_s[0]).max(1.0);
                ch_gamma[0] = 1.0 / (gray[0] as f32 / 128.0).clamp(0.01, 10.0);

                // Per-channel: use auto-computed values with preset gamma
                for ch in 0..3 {
                    ch_s[ch + 1] = auto_shadow[ch];
                    ch_range[ch + 1] = (auto_highlight[ch] - auto_shadow[ch]).max(1.0);
                    ch_gamma[ch + 1] = 1.0 / (gray[ch + 1] as f32 / 128.0).clamp(0.01, 10.0);
                }
            } else {
                // Use preset values directly (FlexColor already computed per-channel levels)
                for i in 0..4 {
                    ch_s[i] = (shadow[i] as f32 * SCALE).clamp(0.0, MAX_VAL);
                    let h_val = (highlight[i] as f32 * SCALE).clamp(1.0, MAX_VAL);
                    ch_range[i] = (h_val - ch_s[i]).max(1.0);
                    ch_gamma[i] = 1.0 / (gray[i] as f32 / 128.0).clamp(0.01, 10.0);
                }
            }

            log::debug!(
                "Film processing levels: shadow=[{:.0},{:.0},{:.0},{:.0}] highlight=[{:.0},{:.0},{:.0},{:.0}] gamma=[{:.3},{:.3},{:.3},{:.3}]",
                ch_s[0], ch_s[1], ch_s[2], ch_s[3],
                ch_s[0]+ch_range[0], ch_s[1]+ch_range[1], ch_s[2]+ch_range[2], ch_s[3]+ch_range[3],
                ch_gamma[0], ch_gamma[1], ch_gamma[2], ch_gamma[3],
            );

            // Use empirical film curve LUT when FilmCurve=4 + Gamma≈2 (color negative).
            // The LUT was derived from pixel comparisons with FlexColor's pre-rendered
            // thumbnails.  It replaces per-channel gray gamma, master levels, and
            // encapsulates the film response + ICC + gamma encoding in one step.
            let use_film_lut = is_negative
                && correction.film_curve == 4
                && (correction.gamma - 2.0).abs() < 0.01;

            let apply_master = !use_film_lut
                && ((ch_s[0] > 4.0)
                    || ((highlight[0] as f32 * SCALE) < MAX_VAL - 4.0)
                    || (ch_gamma[0] - 1.0).abs() > 0.01);

            if use_film_lut {
                log::info!("Using empirical film curve LUT (FilmCurve=4, Gamma=2)");
            }

            // Saturation: FlexColor range is -100..+100, 0 = neutral
            let sat_factor = 1.0 + correction.saturation as f32 / 100.0;

            let row_len = w as usize * 3;
            let mut out_pixels = vec![0u16; row_len * h as usize];

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        let mut ch_f = [
                            src[si] as f32,
                            src[si + 1] as f32,
                            src[si + 2] as f32,
                        ];

                        if is_negative {
                            ch_f[0] = MAX_VAL - ch_f[0];
                            ch_f[1] = MAX_VAL - ch_f[1];
                            ch_f[2] = MAX_VAL - ch_f[2];
                        }

                        if use_film_lut {
                            // Linear per-channel levels (no gray gamma) → LUT
                            let luts: [&[u8; 256]; 3] = [
                                &FILM_CURVE_LUT_R,
                                &FILM_CURVE_LUT_G,
                                &FILM_CURVE_LUT_B,
                            ];
                            for ch in 0..3 {
                                let ci = ch + 1;
                                let n = ((ch_f[ch] - ch_s[ci]) / ch_range[ci]).clamp(0.0, 1.0);
                                ch_f[ch] = lut_interp_16(n, luts[ch]);
                            }
                        } else {
                            // Original pipeline: per-channel levels with gray gamma
                            for ch in 0..3 {
                                let ci = ch + 1;
                                let n = ((ch_f[ch] - ch_s[ci]) / ch_range[ci]).clamp(0.0, 1.0);
                                ch_f[ch] = n.powf(ch_gamma[ci]) * MAX_VAL;
                            }

                            if apply_master {
                                for ch in 0..3 {
                                    let n = ((ch_f[ch] - ch_s[0]) / ch_range[0]).clamp(0.0, 1.0);
                                    ch_f[ch] = n.powf(ch_gamma[0]) * MAX_VAL;
                                }
                            }
                        }

                        if film_type == 2 {
                            let lum = 0.299 * ch_f[0] + 0.587 * ch_f[1] + 0.114 * ch_f[2];
                            ch_f = [lum, lum, lum];
                        }

                        // Apply saturation adjustment
                        if (sat_factor - 1.0).abs() > 0.001 {
                            let lum = 0.299 * ch_f[0] + 0.587 * ch_f[1] + 0.114 * ch_f[2];
                            ch_f[0] = lum + (ch_f[0] - lum) * sat_factor;
                            ch_f[1] = lum + (ch_f[1] - lum) * sat_factor;
                            ch_f[2] = lum + (ch_f[2] - lum) * sat_factor;
                        }

                        row[base] = ch_f[0].clamp(0.0, MAX_VAL) as u16;
                        row[base + 1] = ch_f[1].clamp(0.0, MAX_VAL) as u16;
                        row[base + 2] = ch_f[2].clamp(0.0, MAX_VAL) as u16;
                    }
                });

            let buf = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(w, h, out_pixels)
                .expect("film_processing 16-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb16(buf)
        }
        _ => {
            // 8-bit fallback: convert to 16-bit when auto-levels needed
            let rgb8 = img.to_rgb8();
            let (w, h) = (rgb8.width(), rgb8.height());

            if needs_auto_levels {
                let src8 = rgb8.as_raw();
                let src16: Vec<u16> = src8.iter().map(|&v| (v as u16) << 8).collect();
                let img16 = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(w, h, src16)
                    .expect("8→16 upscale failed");
                let dyn16 = image::DynamicImage::ImageRgb16(img16);
                let result16 = apply_film_processing(&dyn16, correction);
                return image::DynamicImage::ImageRgb8(result16.to_rgb8());
            }

            const SCALE8: f32 = 64.0;

            let mut ch_s = [0.0f32; 4];
            let mut ch_range = [1.0f32; 4];
            let mut ch_gamma = [1.0f32; 4];
            for i in 0..4 {
                ch_s[i] = (shadow[i] as f32 / SCALE8).clamp(0.0, 255.0);
                let h_val = (highlight[i] as f32 / SCALE8).clamp(1.0, 255.0);
                ch_range[i] = (h_val - ch_s[i]).max(1.0);
                ch_gamma[i] = 1.0 / (gray[i] as f32 / 128.0).clamp(0.01, 10.0);
            }
            let use_film_lut_8 = is_negative
                && correction.film_curve == 4
                && (correction.gamma - 2.0).abs() < 0.01;

            let apply_master = !use_film_lut_8
                && ((ch_s[0] > 1.0) || (ch_range[0] < 253.0) || (ch_gamma[0] - 1.0).abs() > 0.01);

            let src = rgb8.as_raw();
            let row_len = w as usize * 3;
            let mut out_pixels = vec![0u8; row_len * h as usize];

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row)| {
                    let src_start = y * row_len;
                    for x in 0..w as usize {
                        let base = x * 3;
                        let si = src_start + base;
                        let mut ch_f = [src[si] as f32, src[si + 1] as f32, src[si + 2] as f32];

                        if is_negative {
                            ch_f[0] = 255.0 - ch_f[0];
                            ch_f[1] = 255.0 - ch_f[1];
                            ch_f[2] = 255.0 - ch_f[2];
                        }

                        if use_film_lut_8 {
                            let luts: [&[u8; 256]; 3] = [
                                &FILM_CURVE_LUT_R,
                                &FILM_CURVE_LUT_G,
                                &FILM_CURVE_LUT_B,
                            ];
                            for ch in 0..3 {
                                let ci = ch + 1;
                                let n = ((ch_f[ch] - ch_s[ci]) / ch_range[ci]).clamp(0.0, 1.0);
                                ch_f[ch] = lut_interp_8(n, luts[ch]);
                            }
                        } else {
                            for ch in 0..3 {
                                let ci = ch + 1;
                                let n = ((ch_f[ch] - ch_s[ci]) / ch_range[ci]).clamp(0.0, 1.0);
                                ch_f[ch] = n.powf(ch_gamma[ci]) * 255.0;
                            }

                            if apply_master {
                                for ch in 0..3 {
                                    let n = ((ch_f[ch] - ch_s[0]) / ch_range[0]).clamp(0.0, 1.0);
                                    ch_f[ch] = n.powf(ch_gamma[0]) * 255.0;
                                }
                            }
                        }

                        if film_type == 2 {
                            let lum = 0.299 * ch_f[0] + 0.587 * ch_f[1] + 0.114 * ch_f[2];
                            ch_f = [lum, lum, lum];
                        }

                        row[base] = ch_f[0].clamp(0.0, 255.0) as u8;
                        row[base + 1] = ch_f[1].clamp(0.0, 255.0) as u8;
                        row[base + 2] = ch_f[2].clamp(0.0, 255.0) as u8;
                    }
                });

            let buf = image::RgbImage::from_raw(w, h, out_pixels)
                .expect("film_processing 8-bit: buffer size mismatch");
            image::DynamicImage::ImageRgb8(buf)
        }
    }
}
