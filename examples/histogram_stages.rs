//! 直方图阶段对照工具：在色彩管线的各个阶段输出直方图 SVG
//!
//! 用法: cargo run --release --example histogram_stages -- <file.fff> [output_dir]
//!
//! 输出 SVG 文件（默认当前目录）：
//!   0_raw_decoded.svg        — 原始解码（无任何处理）
//!   1_film_processed.svg     — 胶片反转后（负片→正片）
//!   2_gradation_curves.svg   — 渐变曲线后
//!   3_scanner_levels.svg     — 扫描仪色阶（胶片曲线 + 色阶 + 伽马）后
//!   4_icc_transformed.svg    — ICC 色彩空间变换后
//!   5_display_adjusted.svg   — 显示调整（曝光/对比度/饱和度等）后

use std::env;
use std::path::{Path, PathBuf};

use fff_viewer::color;
use fff_viewer::flexcolor::{self, EditHistory};
use fff_viewer::tiff::TiffFile;

type Rgb16Image = image::ImageBuffer<image::Rgb<u16>, Vec<u16>>;

const DISPLAY_MAX_DIM: u32 = 4096;

// ─── 直方图计算 ──────────────────────────────────────────────────────────────

/// 自动检测实际位深：找到最大值来判断是 14-bit (≤16383) 还是 16-bit
fn detect_bit_depth(img: &Rgb16Image) -> (u8, u16) {
    let mut max_val: u16 = 0;
    for pixel in img.pixels() {
        for &v in &pixel.0 {
            if v > max_val { max_val = v; }
        }
    }
    let bits = if max_val <= 4095 { 12 }
        else if max_val <= 16383 { 14 }
        else { 16 };
    (bits, max_val)
}

/// 打印每个通道的统计信息
fn print_channel_stats(img: &Rgb16Image, label: &str) {
    let mut stats = [(u16::MAX, 0u16, 0u64, 0u64); 3]; // (min, max, sum, count)
    for pixel in img.pixels() {
        for ch in 0..3 {
            let v = pixel.0[ch];
            stats[ch].0 = stats[ch].0.min(v);
            stats[ch].1 = stats[ch].1.max(v);
            stats[ch].2 += v as u64;
            stats[ch].3 += 1;
        }
    }
    let names = ["R", "G", "B"];
    println!("  [{label}] 通道统计:");
    for ch in 0..3 {
        let mean = stats[ch].2 as f64 / stats[ch].3.max(1) as f64;
        println!("    {}: min={} max={} mean={:.1}", names[ch],
            stats[ch].0, stats[ch].1, mean);
    }
}

/// 计算直方图，可指定位深映射到 256 bin
fn compute_histogram_16_bits(img: &Rgb16Image, bit_depth: u8) -> [[u32; 256]; 4] {
    let shift = match bit_depth {
        12 => 4,  // 4096 → 256
        14 => 6,  // 16384 → 256
        16 => 8,  // 65536 → 256
        _ => 8,
    };
    let mut hist = [[0u32; 256]; 4];
    for pixel in img.pixels() {
        let [r, g, b] = pixel.0;
        let ri = ((r >> shift) as usize).min(255);
        let gi = ((g >> shift) as usize).min(255);
        let bi = ((b >> shift) as usize).min(255);
        hist[0][ri] += 1;
        hist[1][gi] += 1;
        hist[2][bi] += 1;
    }
    for i in 0..256 {
        hist[3][i] = hist[0][i].max(hist[1][i]).max(hist[2][i]);
    }
    hist
}

fn compute_histogram_dyn(img: &image::DynamicImage, bit_depth: u8) -> [[u32; 256]; 4] {
    compute_histogram_16_bits(&to_rgb16(img), bit_depth)
}

/// 找到直方图中的峰值位置（局部最大值）
fn find_peaks(hist: &[u32; 256], min_count_ratio: f64) -> Vec<(usize, u32)> {
    let max_val = *hist.iter().max().unwrap_or(&1) as f64;
    let threshold = (max_val * min_count_ratio) as u32;
    let mut peaks = Vec::new();

    for i in 1..255 {
        if hist[i] >= threshold
            && hist[i] >= hist[i - 1]
            && hist[i] >= hist[i + 1]
            && (i < 2 || hist[i] > hist[i - 2] || hist[i] > hist[i + 1])
        {
            // 检查是否是真正的局部最大值（5-bin 窗口）
            let window_max = (i.saturating_sub(2)..=(i + 2).min(255))
                .map(|j| hist[j])
                .max()
                .unwrap_or(0);
            if hist[i] == window_max {
                peaks.push((i, hist[i]));
            }
        }
    }
    peaks
}

/// 查找谷值：两个峰值之间的局部最小值
fn find_valleys(hist: &[u32; 256], peaks: &[(usize, u32)]) -> Vec<(usize, u32)> {
    let mut valleys = Vec::new();
    if peaks.len() < 2 { return valleys; }

    for w in peaks.windows(2) {
        let (p1, _) = w[0];
        let (p2, _) = w[1];
        if p2 <= p1 + 1 { continue; }
        // 在两个峰之间找最小值
        let mut min_idx = p1 + 1;
        let mut min_val = hist[p1 + 1];
        for i in (p1 + 1)..p2 {
            if hist[i] < min_val {
                min_val = hist[i];
                min_idx = i;
            }
        }
        valleys.push((min_idx, min_val));
    }
    valleys
}

/// 打印直方图峰值信息
fn print_peaks(hist: &[[u32; 256]; 4], label: &str) {
    let names = ["R", "G", "B"];
    println!("  [{label}] 峰值:");
    for ch in 0..3 {
        let peaks = find_peaks(&hist[ch], 0.05); // 阈值 = 最大值的 5%
        if peaks.is_empty() {
            println!("    {}: 无显著峰值", names[ch]);
        } else {
            let peak_strs: Vec<String> = peaks.iter()
                .map(|&(pos, count)| format!("{}({})", pos, count))
                .collect();
            println!("    {}: {}", names[ch], peak_strs.join(", "));
        }
    }
}

fn to_rgb16(img: &image::DynamicImage) -> Rgb16Image {
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => rgb16.clone(),
        _ => {
            let rgb8 = img.to_rgb8();
            let (w, h) = rgb8.dimensions();
            let pixels: Vec<u16> = rgb8.as_raw()
                .iter()
                .map(|&v| (v as u16) << 8 | v as u16)
                .collect();
            Rgb16Image::from_raw(w, h, pixels).expect("to_rgb16 failed")
        }
    }
}

// ─── SVG 生成 ────────────────────────────────────────────────────────────────

fn histogram_to_svg(hist: &[[u32; 256]; 4], title: &str) -> String {
    let w = 900;
    let h = 480;
    let margin_left = 50;
    let margin_top = 40;
    let margin_bottom = 100; // 增大底部空间放标注
    let margin_right = 20;
    let plot_w = w - margin_left - margin_right;
    let plot_h = h - margin_top - margin_bottom;

    // 找最大值（忽略两端各 1 bin 以避免端点噪声干扰缩放）
    let max_val = hist.iter()
        .flat_map(|ch| ch[1..255].iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" style="background:#1a1a1a">
  <text x="{}" y="24" fill="#ccc" font-size="14" font-family="monospace" text-anchor="middle">{title}</text>
  <rect x="{margin_left}" y="{margin_top}" width="{plot_w}" height="{plot_h}" fill="#222" stroke="#444" stroke-width="1"/>
"##,
        w / 2,
    );

    // 网格线
    for i in 1..4 {
        let x = margin_left + plot_w * i / 4;
        svg.push_str(&format!(
            r##"  <line x1="{x}" y1="{margin_top}" x2="{x}" y2="{}" stroke="#333" stroke-width="0.5"/>
"##,
            margin_top + plot_h,
        ));
        svg.push_str(&format!(
            r##"  <text x="{x}" y="{}" fill="#666" font-size="10" font-family="monospace" text-anchor="middle">{}</text>
"##,
            margin_top + plot_h + 15,
            i * 64,
        ));
    }
    // 0 和 255 标签
    svg.push_str(&format!(
        r##"  <text x="{margin_left}" y="{}" fill="#666" font-size="10" font-family="monospace" text-anchor="middle">0</text>
"##,
        margin_top + plot_h + 15,
    ));
    svg.push_str(&format!(
        r##"  <text x="{}" y="{}" fill="#666" font-size="10" font-family="monospace" text-anchor="middle">255</text>
"##,
        margin_left + plot_w,
        margin_top + plot_h + 15,
    ));

    // 通道颜色和名称
    let channels_info = [
        (0, "rgba(255,80,80,0.6)", "#ff5050", "R"),
        (1, "rgba(80,200,80,0.6)", "#50c850", "G"),
        (2, "rgba(80,120,255,0.6)", "#5078ff", "B"),
    ];

    // 先绘制填充区域
    for &(ch, fill_color, _, label) in &channels_info {
        let mut points = Vec::with_capacity(258);
        points.push(format!("{},{}", margin_left, margin_top + plot_h));
        for i in 0..256 {
            let x = margin_left as f64 + (i as f64 / 255.0) * plot_w as f64;
            let val = (hist[ch][i] as f64 / max_val).sqrt().min(1.0);
            let y = margin_top as f64 + plot_h as f64 * (1.0 - val);
            points.push(format!("{:.1},{:.1}", x, y));
        }
        points.push(format!("{},{}", margin_left + plot_w, margin_top + plot_h));

        svg.push_str(&format!(
            "  <polygon points=\"{}\" fill=\"{fill_color}\" stroke=\"none\"/>\n",
            points.join(" "),
        ));

        // 图例
        let lx = margin_left + 10 + ch * 40;
        svg.push_str(&format!(
            "  <rect x=\"{lx}\" y=\"{}\" width=\"10\" height=\"10\" fill=\"{fill_color}\"/>\n\
             \x20\x20<text x=\"{}\" y=\"{}\" fill=\"#ccc\" font-size=\"11\" font-family=\"monospace\">{label}</text>\n",
            margin_top + 6,
            lx + 14,
            margin_top + 15,
        ));
    }

    // ── 峰谷标注 ──
    let bin_to_x = |bin: usize| -> f64 {
        margin_left as f64 + (bin as f64 / 255.0) * plot_w as f64
    };
    let val_to_y = |count: u32| -> f64 {
        let val = (count as f64 / max_val).sqrt().min(1.0);
        margin_top as f64 + plot_h as f64 * (1.0 - val)
    };

    for &(ch, _, stroke_color, ch_label) in &channels_info {
        let peaks = find_peaks(&hist[ch], 0.05);
        let valleys = find_valleys(&hist[ch], &peaks);

        // 底部标注区域: R 在第1行, G 第2行, B 第3行
        let anno_base_y = (margin_top + plot_h + 28 + ch * 22) as f64;

        // 峰值标注 (▲)
        for &(bin, count) in &peaks {
            let x = bin_to_x(bin);
            let y = val_to_y(count);

            // 从峰顶引线到底部标注区
            svg.push_str(&format!(
                "  <line x1=\"{x:.1}\" y1=\"{y:.1}\" x2=\"{x:.1}\" y2=\"{anno_base_y:.1}\" \
                 stroke=\"{stroke_color}\" stroke-width=\"0.5\" stroke-dasharray=\"2,2\" opacity=\"0.6\"/>\n"
            ));
            // 峰顶小圆点
            svg.push_str(&format!(
                "  <circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3\" fill=\"{stroke_color}\" opacity=\"0.9\"/>\n"
            ));
            // 底部标注文字: ▲bin
            svg.push_str(&format!(
                "  <text x=\"{x:.1}\" y=\"{:.1}\" fill=\"{stroke_color}\" font-size=\"9\" \
                 font-family=\"monospace\" text-anchor=\"middle\" opacity=\"0.9\">▲{bin}</text>\n",
                anno_base_y + 4.0,
            ));
        }

        // 谷值标注 (▽)
        for &(bin, count) in &valleys {
            let x = bin_to_x(bin);
            let y = val_to_y(count);

            // 谷底小方块
            svg.push_str(&format!(
                "  <rect x=\"{:.1}\" y=\"{:.1}\" width=\"4\" height=\"4\" fill=\"none\" \
                 stroke=\"{stroke_color}\" stroke-width=\"1\" opacity=\"0.7\"/>\n",
                x - 2.0, y - 2.0,
            ));
            // 引线到底部
            svg.push_str(&format!(
                "  <line x1=\"{x:.1}\" y1=\"{y:.1}\" x2=\"{x:.1}\" y2=\"{anno_base_y:.1}\" \
                 stroke=\"{stroke_color}\" stroke-width=\"0.3\" stroke-dasharray=\"1,3\" opacity=\"0.5\"/>\n"
            ));
            // 底部标注文字: ▽bin
            svg.push_str(&format!(
                "  <text x=\"{x:.1}\" y=\"{:.1}\" fill=\"{stroke_color}\" font-size=\"8\" \
                 font-family=\"monospace\" text-anchor=\"middle\" opacity=\"0.7\">▽{bin}</text>\n",
                anno_base_y + 4.0,
            ));
        }

        // 通道范围标注 (左右边界)
        let mut lo = 256usize;
        let mut hi_b = 0usize;
        for i in 0..256 {
            if hist[ch][i] > 0 {
                lo = lo.min(i);
                hi_b = hi_b.max(i);
            }
        }
        // 范围标注在标注行最左和最右
        let lo_x = bin_to_x(lo);
        let hi_x = bin_to_x(hi_b);
        svg.push_str(&format!(
            "  <text x=\"{lo_x:.1}\" y=\"{:.1}\" fill=\"{stroke_color}\" font-size=\"8\" \
             font-family=\"monospace\" text-anchor=\"middle\" font-weight=\"bold\" opacity=\"0.9\">[{lo}</text>\n",
            anno_base_y - 6.0,
        ));
        svg.push_str(&format!(
            "  <text x=\"{hi_x:.1}\" y=\"{:.1}\" fill=\"{stroke_color}\" font-size=\"8\" \
             font-family=\"monospace\" text-anchor=\"middle\" font-weight=\"bold\" opacity=\"0.9\">{hi_b}]</text>\n",
            anno_base_y - 6.0,
        ));
        // 通道名称标注在最左侧
        svg.push_str(&format!(
            "  <text x=\"{}\" y=\"{:.1}\" fill=\"{stroke_color}\" font-size=\"9\" \
             font-family=\"monospace\" text-anchor=\"end\" font-weight=\"bold\">{ch_label}</text>\n",
            margin_left - 4,
            anno_base_y + 4.0,
        ));
    }

    // Y 轴标签（max count）
    svg.push_str(&format!(
        "  <text x=\"{}\" y=\"{}\" fill=\"#666\" font-size=\"9\" font-family=\"monospace\" text-anchor=\"end\">{:.0}</text>\n\
         \x20\x20<text x=\"{}\" y=\"{}\" fill=\"#666\" font-size=\"9\" font-family=\"monospace\" text-anchor=\"end\">0</text>\n",
        margin_left - 4,
        margin_top + 10,
        max_val,
        margin_left - 4,
        margin_top + plot_h,
    ));

    // 统计信息
    let total_pixels: u64 = hist[0].iter().map(|&v| v as u64).sum();
    svg.push_str(&format!(
        "  <text x=\"{}\" y=\"{}\" fill=\"#888\" font-size=\"10\" font-family=\"monospace\" text-anchor=\"end\">pixels: {} | scale: sqrt</text>\n",
        w - margin_right,
        margin_top + plot_h + 15,
        total_pixels,
    ));

    svg.push_str("</svg>\n");
    svg
}

// ─── 辅助: SVG 保存 + 替代反转直方图 ─────────────────────────────────────────

fn save_svg_file(
    hist: &[[u32; 256]; 4],
    idx: u32,
    name: &str,
    title: &str,
    file_stem: &str,
    output_dir: &Path,
) {
    let svg = histogram_to_svg(hist, &format!("{:02} {} ({})", idx, title, file_stem));
    let path = output_dir.join(format!("{}_{:02}_{}.svg", file_stem, idx, name));
    std::fs::write(&path, &svg).unwrap();
    let range = |ch: usize| -> String {
        let lo = (0..256).find(|&i| hist[ch][i] > 0).unwrap_or(0);
        let hi = (0..256).rev().find(|&i| hist[ch][i] > 0).unwrap_or(255);
        format!("{}-{}", lo, hi)
    };
    println!("  {:02} {:<35} R={:<8} G={:<8} B={}", idx, name, range(0), range(1), range(2));
}

fn compute_alt_histogram(
    data: &Rgb16Image,
    formula: &dyn Fn(u16, usize) -> u8,
) -> [[u32; 256]; 4] {
    let mut h = [[0u32; 256]; 4];
    for pixel in data.pixels() {
        for ch in 0..3 {
            let bin = formula(pixel.0[ch], ch) as usize;
            h[ch][bin.min(255)] += 1;
        }
    }
    for i in 0..256 {
        h[3][i] = h[0][i].max(h[1][i]).max(h[2][i]);
    }
    h
}

fn hist_from_8bit(img: &image::RgbImage) -> [[u32; 256]; 4] {
    let mut h = [[0u32; 256]; 4];
    for pixel in img.pixels() {
        h[0][pixel.0[0] as usize] += 1;
        h[1][pixel.0[1] as usize] += 1;
        h[2][pixel.0[2] as usize] += 1;
    }
    for i in 0..256 {
        h[3][i] = h[0][i].max(h[1][i]).max(h[2][i]);
    }
    h
}

// ─── 主程序 ──────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: cargo run --release --example histogram_stages -- <file.fff> [output_dir]");
        std::process::exit(1);
    }

    let file_path = Path::new(&args[1]);
    let output_dir = if args.len() > 2 {
        PathBuf::from(&args[2])
    } else {
        file_path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    std::fs::create_dir_all(&output_dir).expect("创建输出目录失败");

    let file_stem = file_path.file_stem().unwrap().to_string_lossy();
    let fs = file_stem.as_ref();

    println!("=== 直方图阶段对照工具 ===");
    println!("输入: {}", file_path.display());
    println!("输出目录: {}", output_dir.display());
    println!();

    // ── 解析 FFF 文件 ──
    let tiff = TiffFile::open(file_path).expect("打开 FFF 文件失败");
    println!("文件: {} 个 IFD", tiff.ifds.len());

    let edit_history = EditHistory::parse_from_tiff(&tiff);
    let correction = edit_history
        .as_ref()
        .and_then(|h| {
            if h.settings.is_empty() {
                None
            } else {
                let idx = h.current_index.min(h.settings.len() - 1);
                Some(h.settings[idx].correction.clone())
            }
        })
        .expect("未找到编辑历史校正数据");

    println!(
        "FilmType={} ({}), FilmCurve={}, Gamma={:.2}",
        correction.film_type,
        flexcolor::film_type_name(correction.film_type),
        correction.film_curve,
        correction.gamma,
    );
    println!("  Shadow:    {:?}", correction.shadow);
    println!("  Highlight: {:?}", correction.highlight);
    println!("  Gray:      {:?}", correction.gray);
    println!();

    // ── 解码所有数据源 ──
    let ifd0_img = tiff
        .decode_preview_downscaled(DISPLAY_MAX_DIM)
        .expect("解码 IFD#0 失败");
    let ifd0_rgb16 = to_rgb16(&ifd0_img);

    let (thumb_8, ifd2_rgb16) = tiff.decode_thumbnail_pair().expect("解码缩略图对失败");
    let ifd2_img = image::DynamicImage::ImageRgb16(ifd2_rgb16.clone());

    println!(
        "IFD#0: {}x{} (16-bit, downsampled)",
        ifd0_rgb16.width(),
        ifd0_rgb16.height()
    );
    println!(
        "IFD#1: {}x{} (8-bit, FlexColor)",
        thumb_8.width(),
        thumb_8.height()
    );
    println!(
        "IFD#2: {}x{} (16-bit, preview)",
        ifd2_rgb16.width(),
        ifd2_rgb16.height()
    );
    print_channel_stats(&ifd0_rgb16, "IFD#0 raw");
    print_channel_stats(&ifd2_rgb16, "IFD#2 raw");
    println!();

    // ── 提取胶片曲线和 ICC ──
    let film_lut = if correction.film_type == 1 || correction.film_type == 2 {
        let lut = color::extract_film_curve(&thumb_8, &ifd2_rgb16, &correction);
        println!(
            "  胶片曲线: {}",
            if lut.is_some() {
                "提取成功"
            } else {
                "提取失败，使用硬编码"
            }
        );
        lut
    } else {
        println!("  正片，无需胶片曲线");
        None
    };

    let all_tags = tiff.all_tags();
    let embedded_icc = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
    println!(
        "  ICC: {}",
        if embedded_icc.is_some() {
            "已提取"
        } else {
            "未找到"
        }
    );

    // ── 构建 ManualAdjust ──
    let mut adj = color::ManualAdjust::default();
    adj.film_type = correction.film_type;
    adj.film_curve = correction.film_curve;
    adj.film_gamma = correction.gamma;

    if correction.apply_histogram {
        for i in 0..4 {
            adj.levels_black[i] =
                (correction.shadow[i] as f32 * 4.0 / 65535.0 * 255.0).clamp(0.0, 255.0);
            adj.levels_white[i] =
                (correction.highlight[i] as f32 * 4.0 / 65535.0 * 255.0).clamp(0.0, 255.0);
        }
        adj.levels_gamma[0] = ((correction.gamma as f32) - 1.0).clamp(0.01, 3.00);
        for i in 1..4 {
            adj.levels_gamma[i] = (correction.gray[i] as f32 / 128.0).clamp(0.01, 99.0);
        }
        adj.levels_black[0] = adj.levels_black[1]
            .min(adj.levels_black[2])
            .min(adj.levels_black[3]);
        adj.levels_white[0] = adj.levels_white[1]
            .max(adj.levels_white[2])
            .max(adj.levels_white[3]);
        if correction.dot_color.len() >= 14 {
            adj.output_shadow = [correction.dot_color[0] as f32, correction.dot_color[1] as f32,
                correction.dot_color[2] as f32, correction.dot_color[3] as f32];
            adj.output_highlight = [correction.dot_color[7] as f32, correction.dot_color[8] as f32,
                correction.dot_color[9] as f32, correction.dot_color[10] as f32];
        }
    }

    if correction.apply_sliders {
        adj.saturation = correction.saturation as f32;
        if (correction.ev - 1.0).abs() > 0.001 && correction.ev > 0.0 {
            adj.exposure = correction.ev.log2() as f32;
        }
        adj.contrast = correction.contrast as f32;
        adj.brightness = correction.brightness as f32;
        adj.lightness = correction.lightness as f32;
    }
    adj.color_temperature = correction.color_temperature as f32;
    adj.tint = correction.tint as f32;
    if correction.apply_cc && correction.color_corr.len() == 36 {
        let mut arr = [0i64; 36];
        for (i, &v) in correction.color_corr.iter().enumerate() {
            arr[i] = v;
        }
        adj.color_corr = arr;
        adj.apply_color_corr = true;
    }
    adj.apply_curves = correction.apply_curves && !correction.gradations.is_empty();

    let curve_points = if !correction.gradations.is_empty() {
        let mut pts = correction.gradations.clone();
        while pts.len() < 7 {
            pts.push(vec![(0, 0, 0), (255, 255, 0)]);
        }
        pts
    } else {
        vec![vec![(0, 0, 0), (255, 255, 0)]; 7]
    };

    println!();
    println!("ManualAdjust:");
    println!("  levels_black: {:?}", adj.levels_black);
    println!("  levels_white: {:?}", adj.levels_white);
    println!("  levels_gamma: {:?}", adj.levels_gamma);
    println!(
        "  exposure={:.3} contrast={} brightness={} saturation={}",
        adj.exposure, adj.contrast, adj.brightness, adj.saturation
    );
    println!();

    // ── 计数器 ──
    let mut idx = 0u32;

    // ═══════════════════════════════════════════════════════════════════════════
    //  第一部分：数据源 (原始数据，无处理)
    // ═══════════════════════════════════════════════════════════════════════════
    println!("═══ 数据源 ═══");

    // 01: IFD#0 raw (16-bit mapping)
    idx += 1;
    save_svg_file(
        &compute_histogram_16_bits(&ifd0_rgb16, 16),
        idx, "ifd0_raw", "IFD#0 Raw 16bit", fs, &output_dir,
    );

    // 02: IFD#2 raw (16-bit mapping)
    idx += 1;
    save_svg_file(
        &compute_histogram_16_bits(&ifd2_rgb16, 16),
        idx, "ifd2_raw", "IFD#2 Raw 16bit", fs, &output_dir,
    );

    // 03: IFD#1 FlexColor 8-bit thumbnail (REFERENCE)
    idx += 1;
    save_svg_file(
        &hist_from_8bit(&thumb_8),
        idx, "ifd1_fc_ref", "IFD#1 FlexColor 8bit ★REFERENCE★", fs, &output_dir,
    );

    // ═══════════════════════════════════════════════════════════════════════════
    //  第二部分：管线阶段 (IFD#0 全分辨率源)
    // ═══════════════════════════════════════════════════════════════════════════
    println!("═══ 管线阶段 (IFD#0) ═══");

    // 04: Film Processing (negative inversion)
    let ifd0_film = color::apply_film_processing(&ifd0_img, &correction);
    idx += 1;
    save_svg_file(
        &compute_histogram_dyn(&ifd0_film, 16),
        idx, "ifd0_film", "IFD#0 → Film Processing", fs, &output_dir,
    );

    // 05: Film → apply_film_curve_lut (standalone hardcoded film curve)
    let ifd0_film_fc_standalone = color::apply_film_curve_lut(&ifd0_film, &correction);
    idx += 1;
    save_svg_file(
        &compute_histogram_dyn(&ifd0_film_fc_standalone, 16),
        idx, "ifd0_film_fclut", "IFD#0 → Film → FilmCurveLUT (standalone)", fs, &output_dir,
    );

    // 06: Film → Film Curve only (via scanner_levels, no levels/gamma)
    {
        let mut a = color::ManualAdjust::default();
        a.film_type = adj.film_type;
        a.film_curve = adj.film_curve;
        a.film_gamma = adj.film_gamma;
        a.apply_film_curve = true;
        a.apply_levels = false;
        let img = color::apply_scanner_levels(&ifd0_film, &a, film_lut.as_ref());
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd0_film_fc", "IFD#0 → Film → FC LUT (scanner_levels)", fs, &output_dir,
        );
    }

    // 07: Film → Levels clip only (no FC, no gamma)
    {
        let mut a = adj.clone();
        a.apply_film_curve = false;
        a.levels_gamma = [1.0; 4];
        let img = color::apply_scanner_levels(&ifd0_film, &a, None);
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd0_film_levels", "IFD#0 → Film → Levels only", fs, &output_dir,
        );
    }

    // 08: Film → Per-channel Gamma only (no FC, no levels)
    {
        let mut a = color::ManualAdjust::default();
        a.apply_film_curve = false;
        a.apply_levels = true;
        a.levels_black = [0.0; 4];
        a.levels_white = [255.0; 4];
        a.levels_gamma = [1.0, adj.levels_gamma[1], adj.levels_gamma[2], adj.levels_gamma[3]];
        let img = color::apply_scanner_levels(&ifd0_film, &a, None);
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd0_film_chgamma", "IFD#0 → Film → Per-ch Gamma", fs, &output_dir,
        );
    }

    // 09: Film → Direct Gamma 2.0 (v^0.5, alternative gamma interpretation)
    {
        let mut a = color::ManualAdjust::default();
        a.apply_film_curve = false;
        a.apply_levels = true;
        a.levels_black = [0.0; 4];
        a.levels_white = [255.0; 4];
        a.levels_gamma = [2.0, 1.0, 1.0, 1.0]; // v^(1/2.0) = √v
        let img = color::apply_scanner_levels(&ifd0_film, &a, None);
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd0_film_sqrt", "IFD#0 → Film → √v (gamma 2.0)", fs, &output_dir,
        );
    }

    // 10: Film → FC + Levels (no gamma)
    {
        let mut a = adj.clone();
        a.levels_gamma = [1.0; 4];
        let img = color::apply_scanner_levels(&ifd0_film, &a, film_lut.as_ref());
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd0_film_fc_levels", "IFD#0 → Film → FC+Levels (no γ)", fs, &output_dir,
        );
    }

    // 11: Film → Full Scanner Levels (current gamma interpretation: gamma-1)
    let ifd0_scanlevels = color::apply_scanner_levels(&ifd0_film, &adj, film_lut.as_ref());
    idx += 1;
    save_svg_file(
        &compute_histogram_dyn(&ifd0_scanlevels, 16),
        idx, "ifd0_film_scanlevels", "IFD#0 → Film → Full Scanner Levels", fs, &output_dir,
    );

    // 12: Film → Scanner Levels (direct gamma 2.0 interpretation)
    {
        let mut a = adj.clone();
        a.levels_gamma[0] = correction.gamma as f32; // 2.0 → v^(1/2.0)
        let img = color::apply_scanner_levels(&ifd0_film, &a, film_lut.as_ref());
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd0_film_scanlevels_g2", "IFD#0 → Film → ScanLevels (γ=2.0)", fs, &output_dir,
        );
    }

    // 13: Film → Gradation Curves (before scanner levels)
    let ifd0_grad = color::apply_gradation_curves(&ifd0_film, &curve_points);
    idx += 1;
    save_svg_file(
        &compute_histogram_dyn(&ifd0_grad, 16),
        idx, "ifd0_film_grad", "IFD#0 → Film → Gradation Curves", fs, &output_dir,
    );

    // 14: Film → Gradation → Scanner Levels
    let ifd0_grad_scan = color::apply_scanner_levels(&ifd0_grad, &adj, film_lut.as_ref());
    idx += 1;
    save_svg_file(
        &compute_histogram_dyn(&ifd0_grad_scan, 16),
        idx, "ifd0_film_grad_scan", "IFD#0 → Film → Grad → ScanLevels", fs, &output_dir,
    );

    // 15: Film → Gradation → Scanner → ICC
    if let Some(icc_data) = embedded_icc.as_deref() {
        if let Ok(img) = color::apply_icc_transform(
            &ifd0_grad_scan,
            icc_data,
            color::TargetColorSpace::SRGB,
        ) {
            idx += 1;
            save_svg_file(
                &compute_histogram_dyn(&img, 16),
                idx, "ifd0_grad_scan_icc", "IFD#0 → … → ScanLevels → ICC", fs, &output_dir,
            );

            // 16: Film → Gradation → Scanner → ICC → Display Adjust
            let img2 = color::apply_display_adjust(&img, &adj);
            idx += 1;
            save_svg_file(
                &compute_histogram_dyn(&img2, 16),
                idx, "ifd0_full_pipeline", "IFD#0 → Full Pipeline", fs, &output_dir,
            );
        }
    } else {
        // No ICC, just apply display adjust after scanner levels
        let img = color::apply_display_adjust(&ifd0_grad_scan, &adj);
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd0_full_pipeline", "IFD#0 → Full Pipeline (no ICC)", fs, &output_dir,
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    //  第三部分：管线阶段 (IFD#2 预览源)
    // ═══════════════════════════════════════════════════════════════════════════
    println!("═══ 管线阶段 (IFD#2) ═══");

    // 17: Film Processing on IFD#2
    let ifd2_film = color::apply_film_processing(&ifd2_img, &correction);
    idx += 1;
    save_svg_file(
        &compute_histogram_dyn(&ifd2_film, 16),
        idx, "ifd2_film", "IFD#2 → Film Processing", fs, &output_dir,
    );

    // 18: IFD#2 → Film → FC LUT only
    {
        let mut a = color::ManualAdjust::default();
        a.film_type = adj.film_type;
        a.film_curve = adj.film_curve;
        a.film_gamma = adj.film_gamma;
        a.apply_film_curve = true;
        a.apply_levels = false;
        let img = color::apply_scanner_levels(&ifd2_film, &a, film_lut.as_ref());
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd2_film_fc", "IFD#2 → Film → FC LUT", fs, &output_dir,
        );
    }

    // 19: IFD#2 → Film → Full Scanner Levels
    {
        let img = color::apply_scanner_levels(&ifd2_film, &adj, film_lut.as_ref());
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd2_film_scanlevels", "IFD#2 → Film → Scanner Levels", fs, &output_dir,
        );
    }

    // 20: IFD#2 → Film → Grad → Scanner Levels
    {
        let ifd2_grad = color::apply_gradation_curves(&ifd2_film, &curve_points);
        let img = color::apply_scanner_levels(&ifd2_grad, &adj, film_lut.as_ref());
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd2_film_grad_scan", "IFD#2 → Film → Grad → ScanLevels", fs, &output_dir,
        );
    }

    // 21: IFD#2 → Full Pipeline
    {
        let img = color::apply_color_pipeline(
            ifd2_img.clone(),
            &adj,
            &curve_points,
            film_lut.as_ref(),
            embedded_icc.as_deref(),
            color::TargetColorSpace::SRGB,
        );
        idx += 1;
        save_svg_file(
            &compute_histogram_dyn(&img, 16),
            idx, "ifd2_full_pipeline", "IFD#2 → Full Pipeline", fs, &output_dir,
        );
    }

    // ═══════════════════════════════════════════════════════════════════════════
    //  第四部分：替代反转公式 (直接从原始 16-bit 数据计算直方图)
    // ═══════════════════════════════════════════════════════════════════════════
    println!("═══ 替代反转公式 ═══");

    let hi_16 = [
        correction.highlight[1] as u32 * 4, // R highlight in 16-bit
        correction.highlight[2] as u32 * 4, // G
        correction.highlight[3] as u32 * 4, // B
    ];
    let r_hi = hi_16[0];
    println!(
        "  Highlight (16-bit): R={}, G={}, B={}",
        hi_16[0], hi_16[1], hi_16[2]
    );

    // ── IFD#2 替代反转 ──

    // 22: 255 - (raw >> 8) on IFD#2
    idx += 1;
    save_svg_file(
        &compute_alt_histogram(&ifd2_rgb16, &|v, _ch| {
            255u8.saturating_sub((v >> 8) as u8)
        }),
        idx, "ifd2_inv255", "IFD#2: 255-(raw>>8)", fs, &output_dir,
    );

    // 23: (r_hi - raw) * 255 / r_hi on IFD#2 (H2 formula)
    idx += 1;
    save_svg_file(
        &compute_alt_histogram(&ifd2_rgb16, &|v, _ch| {
            if (v as u32) >= r_hi {
                return 0;
            }
            ((r_hi - v as u32) * 255 / r_hi) as u8
        }),
        idx, "ifd2_rhi", "IFD#2: (r_hi-raw)*255/r_hi", fs, &output_dir,
    );

    // 24: (r_hi - raw) / max_inv * 255 on IFD#2
    {
        let mut max_inv = 0u32;
        for pixel in ifd2_rgb16.pixels() {
            for ch in 0..3 {
                max_inv = max_inv.max(r_hi.saturating_sub(pixel.0[ch] as u32));
            }
        }
        println!("  IFD#2 max_inv (r_hi - min_raw) = {}", max_inv);
        idx += 1;
        save_svg_file(
            &compute_alt_histogram(&ifd2_rgb16, &|v, _ch| {
                let inv = r_hi.saturating_sub(v as u32);
                (inv * 255 / max_inv.max(1)) as u8
            }),
            idx, "ifd2_rhi_maxinv", "IFD#2: (r_hi-raw)/max_inv*255", fs, &output_dir,
        );
    }

    // 25: per-channel (hi_ch - raw) * 255 / hi_ch on IFD#2
    idx += 1;
    save_svg_file(
        &compute_alt_histogram(&ifd2_rgb16, &|v, ch| {
            let h = hi_16[ch];
            if (v as u32) >= h {
                return 0;
            }
            ((h - v as u32) * 255 / h) as u8
        }),
        idx, "ifd2_perch_hi", "IFD#2: per-ch (hi-raw)*255/hi", fs, &output_dir,
    );

    // 26: per-channel (hi_ch - raw) / max_inv_ch * 255 on IFD#2
    {
        let mut max_inv_ch = [0u32; 3];
        for pixel in ifd2_rgb16.pixels() {
            for ch in 0..3 {
                max_inv_ch[ch] =
                    max_inv_ch[ch].max(hi_16[ch].saturating_sub(pixel.0[ch] as u32));
            }
        }
        idx += 1;
        save_svg_file(
            &compute_alt_histogram(&ifd2_rgb16, &|v, ch| {
                let inv = hi_16[ch].saturating_sub(v as u32);
                (inv * 255 / max_inv_ch[ch].max(1)) as u8
            }),
            idx, "ifd2_perch_maxinv", "IFD#2: per-ch (hi-raw)/max_inv*255", fs, &output_dir,
        );
    }

    // ── IFD#0 替代反转 ──

    // 27: 255 - (raw >> 8) on IFD#0
    idx += 1;
    save_svg_file(
        &compute_alt_histogram(&ifd0_rgb16, &|v, _ch| {
            255u8.saturating_sub((v >> 8) as u8)
        }),
        idx, "ifd0_inv255", "IFD#0: 255-(raw>>8)", fs, &output_dir,
    );

    // 28: (r_hi - raw) * 255 / r_hi on IFD#0
    idx += 1;
    save_svg_file(
        &compute_alt_histogram(&ifd0_rgb16, &|v, _ch| {
            if (v as u32) >= r_hi {
                return 0;
            }
            ((r_hi - v as u32) * 255 / r_hi) as u8
        }),
        idx, "ifd0_rhi", "IFD#0: (r_hi-raw)*255/r_hi", fs, &output_dir,
    );

    // 29: (r_hi - raw) / max_inv * 255 on IFD#0
    {
        let mut max_inv = 0u32;
        for pixel in ifd0_rgb16.pixels() {
            for ch in 0..3 {
                max_inv = max_inv.max(r_hi.saturating_sub(pixel.0[ch] as u32));
            }
        }
        println!("  IFD#0 max_inv (r_hi - min_raw) = {}", max_inv);
        idx += 1;
        save_svg_file(
            &compute_alt_histogram(&ifd0_rgb16, &|v, _ch| {
                let inv = r_hi.saturating_sub(v as u32);
                (inv * 255 / max_inv.max(1)) as u8
            }),
            idx, "ifd0_rhi_maxinv", "IFD#0: (r_hi-raw)/max_inv*255", fs, &output_dir,
        );
    }

    // 30: per-channel (hi_ch - raw) * 255 / hi_ch on IFD#0
    idx += 1;
    save_svg_file(
        &compute_alt_histogram(&ifd0_rgb16, &|v, ch| {
            let h = hi_16[ch];
            if (v as u32) >= h {
                return 0;
            }
            ((h - v as u32) * 255 / h) as u8
        }),
        idx, "ifd0_perch_hi", "IFD#0: per-ch (hi-raw)*255/hi", fs, &output_dir,
    );

    // 31: per-channel (hi_ch - raw) / max_inv_ch * 255 on IFD#0
    {
        let mut max_inv_ch = [0u32; 3];
        for pixel in ifd0_rgb16.pixels() {
            for ch in 0..3 {
                max_inv_ch[ch] =
                    max_inv_ch[ch].max(hi_16[ch].saturating_sub(pixel.0[ch] as u32));
            }
        }
        idx += 1;
        save_svg_file(
            &compute_alt_histogram(&ifd0_rgb16, &|v, ch| {
                let inv = hi_16[ch].saturating_sub(v as u32);
                (inv * 255 / max_inv_ch[ch].max(1)) as u8
            }),
            idx, "ifd0_perch_maxinv", "IFD#0: per-ch (hi-raw)/max_inv*255", fs, &output_dir,
        );
    }

    println!();
    println!("=== 完成: {} 个 SVG ===", idx);
    println!("目标参考: FlexColor 范围 R=26-252, G=89-252, B=151-252");
}
