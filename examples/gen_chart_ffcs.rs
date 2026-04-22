//! 生成色卡变体 FFF —— 替换 IFD#0 全分辨率 RGB 像素为合成色卡
//!
//! 色卡设计（3601×4489）：
//! - 16×16 = 256 tiles，每块 ~225×280 像素
//! - 64 unique colors，每色出现在 4 个不同象限位置（避免 lens/vignette 局部偏差）
//! - 64 色构成：16 灰阶 + 6 主色×8 luminance = 16+48
//!
//! 4 位置布局（镜像对称）：
//! - Q-A (top-left 8×8): color i at (i/8, i%8)
//! - Q-B (top-right): vertical flip
//! - Q-C (bottom-left): horizontal flip
//! - Q-D (bottom-right): both flip

use std::fs;
use std::path::PathBuf;

const BASE_FFF: &str = "/Users/will/vmwareShare/test_image/test1_raw.fff";
const OUT_DIR: &str = "/Users/will/vmwareShare/test_image/variants";
const CASES_OUT: &str = "examples/chart_cases.toml";

const TEMPLATES: &[(&str, &str, &str)] = &[
    ("pos", "/Users/will/Desktop/FFF Viewer.app/Contents/Resources/settings/Standard/RGB standard.xml", "1"),
    ("neg", "/Users/will/Desktop/FFF Viewer.app/Contents/Resources/settings/Standard Negative/Negative RGB standard.xml", "1"),
    ("bw",  "/Users/will/Desktop/FFF Viewer.app/Contents/Resources/settings/Standard Negative/B&W negative standard.xml", "5"),
];

/// 生成 64 色调色板（每色 RGB 8-bit，会 <<8 到 16-bit）
fn build_palette() -> [[u8; 3]; 64] {
    let mut p = [[0u8; 3]; 64];
    // 16 灰阶：0..255 均匀（含首末）
    for i in 0..16 {
        let v = (i as u32 * 255 / 15) as u8;
        p[i] = [v, v, v];
    }
    // 48 hue × luminance：6 hue × 8 luminance step
    // hue: R, G, B, Y, C, M
    let hues: [[u8; 3]; 6] = [
        [255, 0, 0],   // R
        [0, 255, 0],   // G
        [0, 0, 255],   // B
        [255, 255, 0], // Y
        [0, 255, 255], // C
        [255, 0, 255], // M
    ];
    let lums: [u8; 8] = [32, 64, 96, 128, 160, 192, 224, 255];
    let mut idx = 16;
    for hue in &hues {
        for &lum in &lums {
            // lum 作比例因子
            let r = (hue[0] as u32 * lum as u32 / 255) as u8;
            let g = (hue[1] as u32 * lum as u32 / 255) as u8;
            let b = (hue[2] as u32 * lum as u32 / 255) as u8;
            p[idx] = [r, g, b];
            idx += 1;
        }
    }
    p
}

/// 构造 3601×4489 RGB16 色卡像素（interleaved u16）
fn build_chart_pixels(width: u32, height: u32) -> Vec<u16> {
    let palette = build_palette();
    let w = width as usize;
    let h = height as usize;
    let mut pixels = vec![0u16; w * h * 3];

    // 4 象限，每象限 8x8 tiles
    let tiles_per_q = 8; // 每象限 8 列 × 8 行
    let qw = w / 2; // 象限宽
    let qh = h / 2; // 象限高
    let tw = qw / tiles_per_q; // tile 宽
    let th = qh / tiles_per_q; // tile 高

    // 象限定义：4 个 (quadrant_x_offset, quadrant_y_offset, permute_fn)
    // Permute: 从 color index i → 象限内 (row, col)
    let quad_permutes: &[(usize, usize, fn(usize) -> (usize, usize))] = &[
        (0, 0, |i| (i / 8, i % 8)),             // Q-A: linear
        (qw, 0, |i| (i / 8, 7 - (i % 8))),      // Q-B: h-flip
        (0, qh, |i| (7 - (i / 8), i % 8)),      // Q-C: v-flip
        (qw, qh, |i| (7 - (i / 8), 7 - (i % 8))), // Q-D: both flip
    ];

    for (qx, qy, permute) in quad_permutes {
        for i in 0..64 {
            let (row, col) = permute(i);
            let rgb = palette[i];
            // tile 在全图里的起止
            let x_start = qx + col * tw;
            let y_start = qy + row * th;
            let x_end = x_start + tw;
            let y_end = y_start + th;
            // 填充 tile（保留 10px 边框做黑线分隔，方便视觉辨认）
            let border = 10;
            for y in y_start..y_end.min(h) {
                for x in x_start..x_end.min(w) {
                    let is_border = y < y_start + border || y >= y_end.saturating_sub(border)
                        || x < x_start + border || x >= x_end.saturating_sub(border);
                    let idx = (y * w + x) * 3;
                    if is_border {
                        // 黑色边框（区分相邻 tile）
                        pixels[idx] = 0;
                        pixels[idx + 1] = 0;
                        pixels[idx + 2] = 0;
                    } else {
                        // 16-bit: 8-bit <<8 + 8-bit 补低位（近 max-fill）
                        pixels[idx] = (rgb[0] as u16) << 8 | rgb[0] as u16;
                        pixels[idx + 1] = (rgb[1] as u16) << 8 | rgb[1] as u16;
                        pixels[idx + 2] = (rgb[2] as u16) << 8 | rgb[2] as u16;
                    }
                }
            }
        }
    }
    pixels
}

// ═══════════════════════════════════════════════════════════════════════════════
// 从 gen_variant_ffcs.rs 复用的 XML 编辑函数（简化版）
// ═══════════════════════════════════════════════════════════════════════════════

fn patch_scalar(xml: &str, k: &str, new_val: &str) -> String {
    let needle = format!("<key>{}</key>", k);
    let Some(kp) = xml.find(&needle) else { return xml.to_string() };
    let after = kp + needle.len();
    let tail = &xml[after..];
    let int_pos = tail.find("<integer>");
    let real_pos = tail.find("<real>");
    let (off, open, close) = match (int_pos, real_pos) {
        (Some(i), Some(r)) if i < r => (i, "<integer>", "</integer>"),
        (Some(_), Some(r)) => (r, "<real>", "</real>"),
        (Some(i), None) => (i, "<integer>", "</integer>"),
        (None, Some(r)) => (r, "<real>", "</real>"),
        (None, None) => return xml.to_string(),
    };
    let vs = after + off + open.len();
    let ve = after + off + tail[off..].find(close).unwrap();
    let mut o = String::with_capacity(xml.len() + 8);
    o.push_str(&xml[..vs]);
    o.push_str(new_val);
    o.push_str(&xml[ve..]);
    o
}

fn insert_key_after(xml: &str, anchor: &str, new_xml: &str) -> String {
    let needle = format!("<key>{}</key>", anchor);
    let Some(p) = xml.find(&needle) else { return xml.to_string() };
    let after = p + needle.len();
    let tail = &xml[after..];
    let close_patterns: &[(&str, &str)] = &[
        ("<integer>", "</integer>"), ("<real>", "</real>"),
        ("<true/>", ""), ("<false/>", ""),
        ("<string>", "</string>"), ("<dict>", "</dict>"), ("<array>", "</array>"),
    ];
    let mut end_off = None;
    for (open, close) in close_patterns {
        if let Some(o) = tail.find(open) {
            if close.is_empty() { end_off = Some(o + open.len()); }
            else if let Some(c) = tail[o..].find(close) { end_off = Some(o + c + close.len()); }
            break;
        }
    }
    let Some(end) = end_off else { return xml.to_string() };
    let ins_at = after + end;
    let mut out = String::with_capacity(xml.len() + new_xml.len() + 16);
    out.push_str(&xml[..ins_at]);
    out.push_str("\n\t\t\t");
    out.push_str(new_xml);
    out.push_str(&xml[ins_at..]);
    out
}

fn extract_image_setting_dict(preset_xml: &str) -> String {
    let key = "<key>ImageSetting</key>";
    let pos = preset_xml.find(key).expect("no ImageSetting");
    let after = pos + key.len();
    let start = preset_xml[after..].find("<dict>").expect("no <dict>") + after;
    let mut depth = 0i32;
    let mut i = start;
    while i < preset_xml.len() {
        if preset_xml[i..].starts_with("<dict>") { depth += 1; i += 6; }
        else if preset_xml[i..].starts_with("</dict>") {
            depth -= 1;
            if depth == 0 { return preset_xml[start..i + 7].to_string(); }
            i += 7;
        } else { i += 1; }
    }
    panic!("unbalanced dict")
}

fn build_fff_xml(inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple Computer//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n  <dict>\n    <key>ImageSettings</key>\n    <array>\n      {}\n    </array>\n\
    <key>CurrentIx</key>\n    <integer>0</integer>\n  </dict>\n</plist>\n",
        inner
    )
}

fn locate_xml_region(data: &[u8]) -> (usize, usize, usize) {
    let xs = data.windows(5).position(|w| w == b"<?xml").expect("no <?xml");
    let pfx = xs - 4;
    let xe = data[xs..].windows(8).position(|w| w == b"</plist>").expect("no </plist>") + xs + 8;
    let alloc_end = xs + 400_000 - 4;
    let _ = xe;
    (pfx, xs, alloc_end.min(data.len()))
}

/// 把色卡 RGB16 interleaved 数据写入 FFF 的 IFD#0 strip offsets（in-place）
///
/// 需要 fff_viewer::tiff::TiffFile 解析 IFD 获取 StripOffsets + StripByteCounts。
/// 然后按字节序序列化 u16 → bytes，分段写入。
fn write_chart_pixels(data: &mut Vec<u8>, pixels: &[u16]) {
    use fff_viewer::tiff::TiffFile;
    // 解析 FFF 获取 strip offsets
    let tiff = TiffFile::parse(data).expect("parse FFF");
    let ifd0 = &tiff.ifds[0];
    let strip_offs = ifd0.get(0x0111).expect("no StripOffsets").as_u32_vec();
    let strip_cnts = ifd0.get(0x0117).expect("no StripByteCounts").as_u32_vec();
    let width = ifd0.get_u32(0x0100).unwrap() as usize;
    let height = ifd0.get_u32(0x0101).unwrap() as usize;
    let bps = ifd0.get(0x0102).and_then(|v| v.as_u32()).unwrap_or(16);
    assert_eq!(bps, 16, "expected 16-bit IFD0");

    let byte_order = tiff.byte_order;
    let total_pixels = width * height * 3;
    assert_eq!(pixels.len(), total_pixels, "pixel count mismatch");

    // 序列化 u16 → bytes（按 byte_order）
    let mut pixel_bytes = Vec::with_capacity(total_pixels * 2);
    for &p in pixels {
        let bytes = match byte_order {
            fff_viewer::tiff::ByteOrder::BigEndian => p.to_be_bytes(),
            fff_viewer::tiff::ByteOrder::LittleEndian => p.to_le_bytes(),
        };
        pixel_bytes.extend_from_slice(&bytes);
    }

    // 分段写入 strips
    let mut src_pos = 0;
    for (off, cnt) in strip_offs.iter().zip(strip_cnts.iter()) {
        let start = *off as usize;
        let end = start + *cnt as usize;
        let chunk_size = (end - start).min(pixel_bytes.len() - src_pos);
        if end <= data.len() {
            data[start..start + chunk_size].copy_from_slice(&pixel_bytes[src_pos..src_pos + chunk_size]);
            src_pos += chunk_size;
        }
    }
}

fn main() {
    let base_data = fs::read(BASE_FFF).unwrap();
    let out_dir = PathBuf::from(OUT_DIR);
    fs::create_dir_all(&out_dir).unwrap();

    let (pfx, xs, alloc_end) = locate_xml_region(&base_data);

    // 解析 base 获取维度
    use fff_viewer::tiff::TiffFile;
    let tiff = TiffFile::parse(&base_data).unwrap();
    let w = tiff.ifds[0].get_u32(0x0100).unwrap();
    let h = tiff.ifds[0].get_u32(0x0101).unwrap();
    println!("Base FFF dimensions: {}×{}", w, h);

    let chart_pixels = build_chart_pixels(w, h);
    println!("Chart pixels built: {} u16 values ({} MB)",
        chart_pixels.len(), chart_pixels.len() * 2 / 1024 / 1024);

    let mut cases_toml = String::from(
        "# gen_chart_ffcs 色卡变体：IFD#0 像素替换为合成色卡\n\
         # 64 色 × 4 镜像位置 = 256 tiles；每色在 4 个不同位置可做统计平均\n\n\
         data_dir = \"/Users/will/vmwareShare/test_image\"\n\
         preset_dir = \".\"\n\n",
    );

    for (prefix, tmpl_path, mode) in TEMPLATES {
        let raw_template = fs::read_to_string(tmpl_path).unwrap();
        let mut template = patch_scalar(&raw_template, "Mode", mode);
        for k in ["CropBottom", "CropLeft", "CropRight", "CropTop"] {
            template = patch_scalar(&template, k, "0.0");
        }
        template = patch_scalar(&template, "Frame", "1");
        template = template.replace("<string>.dp:</string>", "<string>Flextight X5 &amp; 949</string>");
        template = template.replace("<string>.dfR:</string>", "<string></string>");

        let missing: &[(&str, &str)] = &[
            ("FilmCurve", "<integer>4</integer>"),
            ("ApplyCNFilter", "<true/>"),
            ("LensCorrection", "<integer>7</integer>"),
            ("VignetteAmount", "<integer>100</integer>"),
            ("ColorModel", "<integer>0</integer>"),
            ("ColorTemperature", "<integer>0</integer>"),
            ("Tint", "<integer>0</integer>"),
            ("EV", "<real>1.0</real>"),
            ("ColorNoiseRadius", "<integer>0</integer>"),
            ("NoiseFilterBias", "<integer>0</integer>"),
        ];
        for (k, v) in missing {
            if !template.contains(&format!("<key>{}</key>", k)) {
                template = insert_key_after(&template, "ApplyCC", &format!("<key>{}</key>\n\t\t\t{}", k, v));
            }
        }
        if !template.contains("<key>GradationSliders</key>") {
            template = insert_key_after(&template, "ApplyCC",
                "<key>GradationSliders</key>\n\t\t\t<array>\n\t\t\t\t<integer>0</integer>\n\t\t\t\t<integer>0</integer>\n\t\t\t\t<integer>0</integer>\n\t\t\t</array>");
        }

        let inner = extract_image_setting_dict(&template);
        let fff_xml = build_fff_xml(&inner);
        let fff_xml_bytes = fff_xml.as_bytes();

        if xs + fff_xml_bytes.len() > alloc_end {
            eprintln!("[warn] XML too long");
            continue;
        }

        let mut new_data = base_data.clone();
        // 写 XML
        let len_be = (fff_xml_bytes.len() as u32).to_be_bytes();
        new_data[pfx..pfx + 4].copy_from_slice(&len_be);
        new_data[xs..xs + fff_xml_bytes.len()].copy_from_slice(fff_xml_bytes);
        for i in (xs + fff_xml_bytes.len())..alloc_end {
            new_data[i] = 0;
        }
        // 写色卡像素
        write_chart_pixels(&mut new_data, &chart_pixels);

        let name = format!("c_{}_baseline_000", prefix);
        let out_path = out_dir.join(format!("{}.fff", name));
        fs::write(&out_path, &new_data).unwrap();
        println!("✓ {}", out_path.display());

        cases_toml.push_str(&format!(
            "[[case]]\nname = \"{name}\"\nfff    = \"variants/{name}.fff\"\nref    = \"variants/{name}.tif\"\nsource = \"embedded_current\"\n\n",
        ));
    }

    fs::write(CASES_OUT, cases_toml).unwrap();
    println!("\n色卡 baseline 生成完毕。下一步：\n  1. FlexColor 打开 c_*_baseline_000.fff 导出 TIF\n  2. 跑 tif_compare --manifest {} --flex-pipeline\n  3. 色卡效果 OK 后扩展 slider 变体", CASES_OUT);
}
