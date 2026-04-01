/// 从多个 FFF 文件中提取彩色负片的 film curve LUT。
///
/// 策略：
/// 1. 扫描指定目录下的所有 FFF 文件
/// 2. 筛选彩色负片（film_type == 1），且 current setting 无重度显示调整
/// 3. 对每个干净样本，反转 ICC + levels + gamma，提取 film curve
/// 4. 跨样本平均，输出最终 LUT
///
/// 用法: cargo run --release --example extract_lut -- <目录路径> [<目录路径2> ...]

use fff_viewer::flexcolor::{self, EditHistory};
use fff_viewer::color;
use fff_viewer::tiff::TiffFile;
use lcms2::{Profile, Transform, PixelFormat, Intent};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: {} <目录路径> [<目录路径2> ...]", args[0]);
        std::process::exit(1);
    }

    // 收集所有 FFF 文件路径
    let mut fff_paths: Vec<std::path::PathBuf> = Vec::new();
    for dir_arg in &args[1..] {
        let dir = std::path::Path::new(dir_arg);
        if dir.is_file() && dir.extension().map_or(false, |e| e.eq_ignore_ascii_case("fff")) {
            fff_paths.push(dir.to_path_buf());
        } else if dir.is_dir() {
            collect_fff_files(dir, &mut fff_paths);
        } else {
            eprintln!("⚠ 跳过: {}", dir_arg);
        }
    }

    fff_paths.sort();
    println!("找到 {} 个 FFF 文件\n", fff_paths.len());

    // 第一遍：扫描所有文件，筛选可用样本
    let mut samples: Vec<Sample> = Vec::new();
    let mut stats = ScanStats::default();

    for path in &fff_paths {
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  ✗ 读取失败 {}: {}", filename, e);
                stats.read_errors += 1;
                continue;
            }
        };

        let tiff = match TiffFile::parse(&data) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  ✗ 解析失败 {}: {}", filename, e);
                stats.parse_errors += 1;
                continue;
            }
        };

        let edit_history = match EditHistory::parse_from_tiff(&tiff) {
            Some(h) => h,
            None => {
                eprintln!("  ✗ 无编辑历史 {}", filename);
                stats.no_history += 1;
                continue;
            }
        };

        let current_idx = edit_history.current_index.min(edit_history.settings.len().saturating_sub(1));
        let current = &edit_history.settings[current_idx].correction;

        stats.total_scanned += 1;

        // 只关注彩色负片
        if current.film_type != 1 {
            match current.film_type {
                0 => stats.positive += 1,
                2 => stats.bw_negative += 1,
                _ => stats.other_type += 1,
            }
            continue;
        }
        stats.color_negative += 1;

        // 检查 current setting 是否 "干净"（无重度显示调整）
        let is_clean = is_clean_setting(current);
        let setting_name = &edit_history.settings[current_idx].name;

        if !is_clean {
            println!("  ⚠ {} — current setting '{}' (#{}) 有显示调整，跳过",
                filename, setting_name, current_idx);
            println!("    contrast={}, brightness={}, lightness={}, sat={}, ev={:.3}",
                current.contrast, current.brightness, current.lightness,
                current.saturation, current.ev);
            stats.has_adjustments += 1;
            continue;
        }

        // 解码缩略图对
        let (thumb_8, preview_16) = match tiff.decode_thumbnail_pair() {
            Some(pair) => pair,
            None => {
                eprintln!("  ✗ 无缩略图对 {}", filename);
                stats.no_thumbnail += 1;
                continue;
            }
        };

        // 检查 shadow 是否接近零（低 shadow 样本有更完整的 film curve 覆盖）
        let max_shadow = current.shadow[1..4].iter().map(|&s| s as f32).fold(0.0f32, f32::max);
        let shadow_tag = if max_shadow < 100.0 { "★" } else { " " };

        println!("  ✓{} {} — '{}' (#{}) gamma={:.3} gray={:?}",
            shadow_tag, filename, setting_name, current_idx, current.gamma, &current.gray[1..4]);
        println!("    highlight={:?} shadow={:?}",
            &current.highlight[1..4], &current.shadow[1..4]);

        samples.push(Sample {
            path: path.clone(),
            filename: filename.to_string(),
            thumb_8,
            preview_16,
            correction: current.clone(),
            low_shadow: max_shadow < 100.0,
        });
    }

    let low_shadow_count = samples.iter().filter(|s| s.low_shadow).count();
    println!("\n=== 扫描统计 ===");
    println!("总文件: {}, 彩色负片: {}, 干净样本: {} (低shadow★: {})",
        stats.total_scanned, stats.color_negative, samples.len(), low_shadow_count);
    println!("正片: {}, B&W: {}, 有调整: {}, 无缩略图: {}",
        stats.positive, stats.bw_negative, stats.has_adjustments, stats.no_thumbnail);

    // 只使用低 shadow 样本来获得完整的 film curve 覆盖
    let samples: Vec<Sample> = samples.into_iter().filter(|s| s.low_shadow).collect();

    if samples.is_empty() {
        eprintln!("\n❌ 没有找到低 shadow 的干净彩色负片样本");
        std::process::exit(1);
    }

    println!("使用 {} 个低 shadow 样本进行提取\n", samples.len());

    // 第二遍：从每个样本提取 film curve
    println!("\n=== 提取 Film Curve ===");

    // 加载 ICC profile 用于反转
    let profile_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("profiles")
        .join("Flextight X5 & 949.icc");
    let icc_data = std::fs::read(&profile_path)
        .expect(&format!("无法读取 ICC 配置文件: {}", profile_path.display()));
    println!("✓ ICC profile loaded: {} bytes", icc_data.len());

    // 创建反向 ICC 变换 (sRGB → scanner RGB)
    let input_profile = Profile::new_icc(&icc_data)
        .expect("Failed to load input ICC profile");
    let srgb_profile = Profile::new_srgb();
    let reverse_icc = Transform::new(
        &srgb_profile,
        PixelFormat::RGB_8,
        &input_profile,
        PixelFormat::RGB_16,
        Intent::Perceptual,
    ).expect("Failed to create reverse ICC transform");

    const BINS: usize = 256;

    // 两组统计：with_icc（反转ICC）和 no_icc（不反转ICC）
    let mut g_sums_icc = [[0.0f64; BINS]; 3];
    let mut g_counts_icc = [[0u32; BINS]; 3];
    let mut g_sums_no = [[0.0f64; BINS]; 3];
    let mut g_counts_no = [[0u32; BINS]; 3];
    let mut all_sums_icc: Vec<[[f64; BINS]; 3]> = Vec::new();
    let mut all_counts_icc: Vec<[[u32; BINS]; 3]> = Vec::new();
    let mut all_sums_no: Vec<[[f64; BINS]; 3]> = Vec::new();
    let mut all_counts_no: Vec<[[u32; BINS]; 3]> = Vec::new();

    for (si, sample) in samples.iter().enumerate() {
        let c = &sample.correction;
        let (w, h) = (sample.thumb_8.width() as usize, sample.thumb_8.height() as usize);
        let thumb_raw = sample.thumb_8.as_raw();
        let prev_raw = sample.preview_16.as_raw();

        // 反转 ICC: 将 8-bit sRGB 缩略图转回 16-bit scanner RGB
        let npix = w * h;
        let thumb_pixels: Vec<[u8; 3]> = (0..npix)
            .map(|i| [thumb_raw[i*3], thumb_raw[i*3+1], thumb_raw[i*3+2]])
            .collect();
        let mut reversed_pixels: Vec<[u16; 3]> = vec![[0u16; 3]; npix];
        reverse_icc.transform_pixels(&thumb_pixels, &mut reversed_pixels);

        // 反转参数
        let hi = [
            c.highlight[1] as f32 * 4.0,
            c.highlight[2] as f32 * 4.0,
            c.highlight[3] as f32 * 4.0,
        ];
        let scale = [
            if hi[0] > 0.0 { 65535.0 / hi[0] } else { 1.0 },
            if hi[1] > 0.0 { 65535.0 / hi[1] } else { 1.0 },
            if hi[2] > 0.0 { 65535.0 / hi[2] } else { 1.0 },
        ];

        // 色阶参数
        let mut bl = [0.0f32; 3];
        let mut wh_c = [0.0f32; 3];
        let mut gamma_c = [0.0f32; 3];
        for ch in 0..3 {
            bl[ch] = c.shadow[ch + 1] as f32 * 4.0 / 65535.0;
            wh_c[ch] = c.highlight[ch + 1] as f32 * 4.0 / 65535.0;
            gamma_c[ch] = (c.gray[ch + 1] as f32 / 128.0).max(0.01);
        }
        let gamma_m = ((c.gamma as f32) - 1.0).max(0.01);

        let mut sums_icc = [[0.0f64; BINS]; 3];
        let mut counts_icc = [[0u32; BINS]; 3];
        let mut sums_no = [[0.0f64; BINS]; 3];
        let mut counts_no = [[0u32; BINS]; 3];

        for y in 0..h {
            for x in 0..w {
                let pi = y * w + x;

                for ch in 0..3 {
                    // 1. 反转 16-bit 预览 → inverted value (归一化 0-1)
                    let raw_val = prev_raw[pi * 3 + ch] as f32;
                    let inv = ((hi[ch] - raw_val).max(0.0) * scale[ch]).clamp(0.0, 65535.0) / 65535.0;

                    let range = (wh_c[ch] - bl[ch]).max(0.001);
                    let bin = ((inv * (BINS - 1) as f32) as usize).min(BINS - 1);

                    // Mode A: 反转 ICC
                    {
                        let thumb_scanner = reversed_pixels[pi][ch] as f32 / 65535.0;
                        let mut v = thumb_scanner;
                        v = v.powf(gamma_m);
                        v = v.powf(gamma_c[ch]);
                        v = v * range + bl[ch];
                        v = v.clamp(0.0, 1.0);
                        sums_icc[ch][bin] += v as f64;
                        counts_icc[ch][bin] += 1;
                        g_sums_icc[ch][bin] += v as f64;
                        g_counts_icc[ch][bin] += 1;
                    }
                    // Mode B: 不反转 ICC（直接用 8-bit 缩略图）
                    {
                        let thumb_val = thumb_raw[pi * 3 + ch] as f32 / 255.0;
                        let mut v = thumb_val;
                        v = v.powf(gamma_m);
                        v = v.powf(gamma_c[ch]);
                        v = v * range + bl[ch];
                        v = v.clamp(0.0, 1.0);
                        sums_no[ch][bin] += v as f64;
                        counts_no[ch][bin] += 1;
                        g_sums_no[ch][bin] += v as f64;
                        g_counts_no[ch][bin] += 1;
                    }
                }
            }
        }

        let mut valid_bins = [0usize; 3];
        for ch in 0..3 {
            valid_bins[ch] = counts_icc[ch].iter().filter(|&&c| c > 0).count();
        }
        println!("  [{:2}] {} — valid bins R:{}/G:{}/B:{}, pixels={}",
            si, sample.filename, valid_bins[0], valid_bins[1], valid_bins[2], w * h);

        all_sums_icc.push(sums_icc);
        all_counts_icc.push(counts_icc);
        all_sums_no.push(sums_no);
        all_counts_no.push(counts_no);
    }

    // 对两种模式分别计算平均 LUT
    for (mode_name, global_sums, global_counts, all_sums, all_counts) in [
        ("A: 反转ICC", &g_sums_icc, &g_counts_icc, &all_sums_icc, &all_counts_icc),
        ("B: 不反转ICC", &g_sums_no, &g_counts_no, &all_sums_no, &all_counts_no),
    ] {
        println!("\n{}", "=".repeat(60));
        println!("=== [{}] 全局平均 Film Curve LUT ({}个样本) ===", mode_name, samples.len());

        let mut avg_lut = [[0u8; BINS]; 3];
        for ch in 0..3 {
            let ch_name = ["R", "G", "B"][ch];
            let mut bin_avgs = vec![0.0f32; BINS];
            let mut valid_indices: Vec<usize> = Vec::new();
            let mut valid_values: Vec<f32> = Vec::new();

            for i in 0..BINS {
                if global_counts[ch][i] > 0 {
                    let avg = (global_sums[ch][i] / global_counts[ch][i] as f64) as f32;
                    bin_avgs[i] = avg;
                    valid_indices.push(i);
                    valid_values.push(avg);
                }
            }

            if valid_indices.len() >= 2 {
                for i in 0..BINS {
                    if global_counts[ch][i] == 0 {
                        let right = valid_indices.partition_point(|&v| v <= i);
                        if right == 0 {
                            bin_avgs[i] = valid_values[0];
                        } else if right >= valid_indices.len() {
                            bin_avgs[i] = *valid_values.last().unwrap();
                        } else {
                            let li = valid_indices[right - 1];
                            let ri = valid_indices[right];
                            let frac = (i - li) as f32 / (ri - li) as f32;
                            bin_avgs[i] = valid_values[right - 1] * (1.0 - frac)
                                + valid_values[right] * frac;
                        }
                    }
                }
            }

            for i in 1..BINS {
                if bin_avgs[i] < bin_avgs[i - 1] {
                    bin_avgs[i] = bin_avgs[i - 1];
                }
            }

            for i in 0..BINS {
                avg_lut[ch][i] = (bin_avgs[i] * 255.0).round().clamp(0.0, 255.0) as u8;
            }

            let first_nonzero = (0..BINS).find(|&i| avg_lut[ch][i] > 0).unwrap_or(BINS);
            let reaches_255 = (0..BINS).rev().find(|&i| avg_lut[ch][i] < 255).unwrap_or(0);
            println!("{}: first_nonzero=bin {}, last<255=bin {}, valid_bins={}/{}",
                ch_name, first_nonzero, reaches_255, valid_indices.len(), BINS);
        }

        println!("\nRust 代码:");
        for (ch, name) in [(0, "R"), (1, "G"), (2, "B")] {
            println!("pub const FILM_CURVE_LUT_{}: [u8; 256] = [", name);
            for row in 0..16 {
                let start = row * 16;
                let vals: Vec<String> = (start..start + 16)
                    .map(|i| format!("{:3}", avg_lut[ch][i]))
                    .collect();
                println!("    {},", vals.join(", "));
            }
            println!("];\n");
        }

        println!("与现有硬编码 LUT 比较:");
        let existing = [
            ("R", &color::FILM_CURVE_LUT_R),
            ("G", &color::FILM_CURVE_LUT_G),
            ("B", &color::FILM_CURVE_LUT_B),
        ];
        for (ch_idx, (name, old_lut)) in existing.iter().enumerate() {
            let mut sum_diff = 0i64;
            let mut max_diff = 0i32;
            for i in 0..256 {
                let diff = avg_lut[ch_idx][i] as i32 - old_lut[i] as i32;
                sum_diff += diff.abs() as i64;
                max_diff = max_diff.max(diff.abs());
            }
            println!("{}: MAE={:.1}, MaxDiff={}", name, sum_diff as f64 / 256.0, max_diff);
        }

        // 方差
        println!("\n样本间一致性:");
        for ch in 0..3 {
            let ch_name = ["R", "G", "B"][ch];
            let mut max_std = 0.0f64;
            let mut total_std = 0.0f64;
            let mut std_count = 0;
            for bin in 0..BINS {
                if global_counts[ch][bin] < 10 { continue; }
                let mean = global_sums[ch][bin] / global_counts[ch][bin] as f64;
                let mut var_sum = 0.0f64;
                let mut n_samples = 0;
                for si in 0..samples.len() {
                    if all_counts[si][ch][bin] > 0 {
                        let sample_avg = all_sums[si][ch][bin] / all_counts[si][ch][bin] as f64;
                        var_sum += (sample_avg - mean).powi(2);
                        n_samples += 1;
                    }
                }
                if n_samples > 1 {
                    let std = (var_sum / (n_samples - 1) as f64).sqrt();
                    max_std = max_std.max(std);
                    total_std += std;
                    std_count += 1;
                }
            }
            let avg_std = if std_count > 0 { total_std / std_count as f64 } else { 0.0 };
            println!("{}: avg_std={:.4}, max_std={:.4} ({} bins)", ch_name, avg_std, max_std, std_count);
        }
    }
}

struct Sample {
    path: std::path::PathBuf,
    filename: String,
    thumb_8: image::RgbImage,
    preview_16: image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    correction: flexcolor::ImageCorrection,
    low_shadow: bool,
}

#[derive(Default)]
struct ScanStats {
    total_scanned: usize,
    color_negative: usize,
    positive: usize,
    bw_negative: usize,
    other_type: usize,
    has_adjustments: usize,
    no_thumbnail: usize,
    read_errors: usize,
    parse_errors: usize,
    no_history: usize,
}

fn is_clean_setting(c: &flexcolor::ImageCorrection) -> bool {
    // 无对比度/亮度/明度调整
    let no_cbl = !c.apply_sliders || (
        c.contrast == 0 && c.brightness == 0 && c.lightness == 0
    );
    // 曝光中性
    let no_ev = !c.apply_sliders || (c.ev - 1.0).abs() < 0.01;
    // 无 CC 矩阵
    let no_cc = !c.apply_cc || c.color_corr.iter().all(|&v| v == 0);
    // 恒等渐变曲线
    let no_grad = !c.apply_curves || c.gradations.is_empty()
        || c.gradations.iter().all(|pts| {
            pts.len() == 2 && pts[0].0 == 0 && pts[0].1 == 0 && pts[1].0 == 255 && pts[1].1 == 255
        });
    // DotColor 默认 (0-255)
    let no_dot = c.dot_color.len() < 14
        || c.dot_color.iter().enumerate().all(|(i, &v)| {
            if i <= 2 { v == 0 } else if i >= 7 && i <= 9 { v == 255 } else { true }
        });

    no_cbl && no_ev && no_cc && no_grad && no_dot
}

fn collect_fff_files(dir: &std::path::Path, result: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_fff_files(&path, result);
            } else if path.extension().map_or(false, |e| e.eq_ignore_ascii_case("fff")) {
                result.push(path);
            }
        }
    }
}
