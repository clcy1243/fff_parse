//! 生成色卡变体 FFF —— 替换 IFD#0/1/2 全部像素为合成色卡，叠加与 gen_variant_ffcs.rs
//! 相同的 slider / 曲线 / 色阶变体矩阵。
//!
//! 色卡设计（全 IFD）：
//! - 16×16 = 256 tiles，每块 ~w/16 × h/16
//! - 64 unique colors，每色出现在 4 个不同象限位置（避免 lens/vignette 局部偏差）
//! - 64 色构成：16 灰阶 + 6 主色×8 luminance
//!
//! 4 位置布局（镜像对称）：
//! - Q-A (top-left 8×8): color i at (i/8, i%8)
//! - Q-B (top-right): h-flip
//! - Q-C (bottom-left): v-flip
//! - Q-D (bottom-right): both flip
//!
//! 变体命名：c_{prefix}_{slider}_{val}，与 v_* 对应以便与 variants_cases.toml 对比。

use std::fs;
use std::path::PathBuf;
use std::collections::HashSet;

const TEMPLATES: &[(&str, &str, &str)] = &[
    ("pos", "Standard/RGB standard.xml", "1"),
    ("neg", "Standard Negative/Negative RGB standard.xml", "1"),
    ("bw",  "Standard Negative/B&W negative standard.xml", "5"),
];
const BASE_FFF: &str = "/Users/will/vmwareShare/test_image/test1_raw.fff";
const OUT_DIR: &str = "/Users/will/vmwareShare/test_image/variants";
const CASES_OUT: &str = "examples/chart_cases.toml";

struct RunConfig {
    base_fff: PathBuf,
    out_dir: PathBuf,
    cases_out: PathBuf,
    settings_root: PathBuf,
    manifest_data_dir: String,
    prefix_filter: Option<HashSet<String>>,
    max_cases_per_prefix: Option<usize>,
}

fn env_path(name: &str, default: impl FnOnce() -> PathBuf) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or_else(default)
}

fn env_string(name: &str, default: impl FnOnce() -> String) -> String {
    std::env::var(name).unwrap_or_else(|_| default())
}

fn parse_prefix_filter() -> Option<HashSet<String>> {
    let raw = std::env::var("FFF_PREFIXES").ok()?;
    let set: HashSet<String> = raw
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if set.is_empty() { None } else { Some(set) }
}

fn parse_max_cases() -> Option<usize> {
    std::env::var("FFF_MAX_CASES")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
}

fn toml_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn load_config() -> RunConfig {
    let base_fff = env_path("FFF_BASE_FFF", || PathBuf::from(BASE_FFF));
    let out_dir = env_path("FFF_VARIANTS_OUT_DIR", || PathBuf::from(OUT_DIR));
    let cases_out = env_path("FFF_CASES_OUT", || PathBuf::from(CASES_OUT));
    let settings_root = env_path("FFF_SETTINGS_ROOT", || {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("settings")
    });
    let manifest_data_dir = env_string("FFF_DATA_DIR", || {
        let parent = out_dir.parent().unwrap_or(out_dir.as_path());
        toml_path(parent)
    });
    RunConfig {
        base_fff,
        out_dir,
        cases_out,
        settings_root,
        manifest_data_dir,
        prefix_filter: parse_prefix_filter(),
        max_cases_per_prefix: parse_max_cases(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 色卡像素生成
// ═══════════════════════════════════════════════════════════════════════════════

fn build_palette() -> [[u8; 3]; 64] {
    let mut p = [[0u8; 3]; 64];
    for i in 0..16 {
        let v = (i as u32 * 255 / 15) as u8;
        p[i] = [v, v, v];
    }
    let hues: [[u8; 3]; 6] = [
        [255, 0, 0], [0, 255, 0], [0, 0, 255],
        [255, 255, 0], [0, 255, 255], [255, 0, 255],
    ];
    let lums: [u8; 8] = [32, 64, 96, 128, 160, 192, 224, 255];
    let mut idx = 16;
    for hue in &hues {
        for &lum in &lums {
            let r = (hue[0] as u32 * lum as u32 / 255) as u8;
            let g = (hue[1] as u32 * lum as u32 / 255) as u8;
            let b = (hue[2] as u32 * lum as u32 / 255) as u8;
            p[idx] = [r, g, b];
            idx += 1;
        }
    }
    p
}

fn build_chart_u8(width: u32, height: u32) -> Vec<u8> {
    build_chart_pixels(width, height).iter().map(|&v| (v >> 8) as u8).collect()
}

fn build_chart_pixels(width: u32, height: u32) -> Vec<u16> {
    let palette = build_palette();
    let w = width as usize;
    let h = height as usize;
    let mut pixels = vec![0u16; w * h * 3];
    let qw = w / 2;
    let qh = h / 2;
    let tw = qw / 8;
    let th = qh / 8;

    let quad_permutes: &[(usize, usize, fn(usize) -> (usize, usize))] = &[
        (0, 0, |i| (i / 8, i % 8)),
        (qw, 0, |i| (i / 8, 7 - (i % 8))),
        (0, qh, |i| (7 - (i / 8), i % 8)),
        (qw, qh, |i| (7 - (i / 8), 7 - (i % 8))),
    ];

    for (qx, qy, permute) in quad_permutes {
        for i in 0..64 {
            let (row, col) = permute(i);
            let rgb = palette[i];
            let x_start = qx + col * tw;
            let y_start = qy + row * th;
            let x_end = x_start + tw;
            let y_end = y_start + th;
            let border = 10;
            for y in y_start..y_end.min(h) {
                for x in x_start..x_end.min(w) {
                    let is_border = y < y_start + border || y >= y_end.saturating_sub(border)
                        || x < x_start + border || x >= x_end.saturating_sub(border);
                    let idx = (y * w + x) * 3;
                    if is_border {
                        pixels[idx] = 0; pixels[idx + 1] = 0; pixels[idx + 2] = 0;
                    } else {
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
// XML 编辑（复用自 gen_variant_ffcs.rs）
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

fn replace_int_array(xml: &str, key: &str, values: &[i32]) -> String {
    let needle = format!("<key>{}</key>", key);
    let Some(pos) = xml.find(&needle) else { return xml.to_string() };
    let after_key = pos + needle.len();
    let tail = &xml[after_key..];
    let Some(arr_rel) = tail.find("<array>") else { return xml.to_string() };
    let arr_start = after_key + arr_rel + 7;
    let Some(end_rel) = xml[arr_start..].find("</array>") else { return xml.to_string() };
    let end = arr_start + end_rel;
    let mut body = String::from("\n");
    for v in values {
        body.push_str(&format!("\t\t\t\t<integer>{}</integer>\n", v));
    }
    body.push_str("\t\t\t");
    let mut out = String::with_capacity(xml.len() + body.len());
    out.push_str(&xml[..arr_start]);
    out.push_str(&body);
    out.push_str(&xml[end..]);
    out
}

fn replace_gradations(xml: &str, new_array_xml: &str) -> String {
    let key = "<key>Gradations</key>";
    let Some(key_pos) = xml.find(key) else { return xml.to_string() };
    let after_key = key_pos + key.len();
    let tail = &xml[after_key..];
    let Some(arr_rel) = tail.find("<array>") else { return xml.to_string() };
    let arr_start = after_key + arr_rel;
    let mut depth = 0i32;
    let mut i = arr_start;
    let bytes = xml.as_bytes();
    while i < bytes.len() {
        if xml[i..].starts_with("<array>") { depth += 1; i += 7; }
        else if xml[i..].starts_with("</array>") {
            depth -= 1;
            if depth == 0 {
                let end = i + 8;
                let mut out = String::with_capacity(xml.len() + new_array_xml.len());
                out.push_str(&xml[..arr_start]);
                out.push_str(new_array_xml);
                out.push_str(&xml[end..]);
                return out;
            }
            i += 8;
        } else { i += 1; }
    }
    xml.to_string()
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
    let _xe = data[xs..].windows(8).position(|w| w == b"</plist>").expect("no </plist>") + xs + 8;
    let alloc_end = xs + 400_000 - 4;
    (pfx, xs, alloc_end.min(data.len()))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Variant 定义（与 gen_variant_ffcs.rs 对齐）
// ═══════════════════════════════════════════════════════════════════════════════

struct Variant {
    name: String,
    patches: Vec<(String, String)>,
    gradations: Vec<Vec<(u8, u8, u8)>>,
}

fn default_identity_gradation() -> Vec<(u8, u8, u8)> {
    vec![(0, 0, 1), (255, 255, 1)]
}

fn build_gradations(which: usize, custom: Vec<(u8, u8, u8)>) -> Vec<Vec<(u8, u8, u8)>> {
    let mut g = vec![default_identity_gradation(); 7];
    g[which] = custom;
    g
}

fn serialize_gradations(grads: &[Vec<(u8, u8, u8)>]) -> String {
    let mut out = String::from("<array>\n");
    for curve in grads {
        out.push_str("\t\t\t\t<array>\n");
        for (x, y, dy) in curve {
            out.push_str(&format!(
                "\t\t\t\t\t<dict>\n\t\t\t\t\t\t<key>X</key>\n\t\t\t\t\t\t<integer>{}</integer>\n\
                 \t\t\t\t\t\t<key>Y</key>\n\t\t\t\t\t\t<integer>{}</integer>\n\
                 \t\t\t\t\t\t<key>DY</key>\n\t\t\t\t\t\t<integer>{}</integer>\n\t\t\t\t\t</dict>\n",
                x, y, dy
            ));
        }
        out.push_str("\t\t\t\t</array>\n");
    }
    out.push_str("\t\t\t</array>");
    out
}

fn build_variants_for(prefix: &str) -> Vec<Variant> {
    let mut v = Vec::new();
    let name = |slider: &str, val: &str| format!("c_{}_{}_{}", prefix, slider, val);

    v.push(Variant { name: name("baseline", "000"), patches: vec![], gradations: vec![] });

    for c in [-100, -75, -50, -20, 20, 50, 75, 100] {
        v.push(Variant {
            name: name("contrast", &format!("{:+03}", c)),
            patches: vec![("Contrast".into(), c.to_string())],
            gradations: vec![],
        });
    }
    for b in [-100, -75, -50, -20, 20, 50, 75, 100] {
        v.push(Variant {
            name: name("brightness", &format!("{:+03}", b)),
            patches: vec![("Brightness".into(), b.to_string())],
            gradations: vec![],
        });
    }
    for l in [5, 10, 20, 50, 75, 100] {
        v.push(Variant {
            name: name("lightness", &format!("{:03}", l)),
            patches: vec![("Lightness".into(), l.to_string())],
            gradations: vec![],
        });
    }
    for s in [-50, -25, -20, 20, 25, 50] {
        v.push(Variant {
            name: name("saturation", &format!("{:+03}", s)),
            patches: vec![("Saturation".into(), s.to_string())],
            gradations: vec![],
        });
    }
    for (tag, g) in &[
        ("050", "0.5"), ("100", "1.0"), ("150", "1.5"),
        ("225", "2.25"), ("250", "2.5"), ("300", "3.0"),
    ] {
        v.push(Variant {
            name: name("gamma", tag),
            patches: vec![("Gamma".into(), g.to_string())],
            gradations: vec![],
        });
    }
    for (tag, ev) in &[("050", "0.5"), ("075", "0.75"), ("150", "1.5"), ("200", "2.0")] {
        v.push(Variant {
            name: name("ev", tag),
            patches: vec![("EV".into(), ev.to_string())],
            gradations: vec![],
        });
    }
    for t in [-50, 25, 50] {
        v.push(Variant {
            name: name("temp", &format!("{:+03}", t)),
            patches: vec![("ColorTemperature".into(), t.to_string())],
            gradations: vec![],
        });
    }
    for t in [-50, 25, 50] {
        v.push(Variant {
            name: name("tint", &format!("{:+03}", t)),
            patches: vec![("Tint".into(), t.to_string())],
            gradations: vec![],
        });
    }

    // Gradations 曲线变体
    let curve_cases: &[(&str, usize, Vec<(u8, u8, u8)>)] = &[
        ("curve_master_scurve", 0, vec![(0,0,1),(64,32,1),(192,224,1),(255,255,1)]),
        ("curve_master_lift",   0, vec![(0,32,1),(128,160,1),(255,255,1)]),
        ("curve_master_darken", 0, vec![(0,0,1),(128,96,1),(255,224,1)]),
        ("curve_master_invert", 0, vec![(0,255,1),(255,0,1)]),
        ("curve_r_warm",        1, vec![(0,0,1),(128,160,1),(255,255,1)]),
        ("curve_g_lift",        2, vec![(0,0,1),(128,160,1),(255,255,1)]),
        ("curve_b_cool",        3, vec![(0,0,1),(128,160,1),(255,255,1)]),
    ];
    for (cname, which, pts) in curve_cases {
        v.push(Variant {
            name: format!("c_{}_{}", prefix, cname),
            patches: vec![],
            gradations: build_gradations(*which, pts.clone()),
        });
    }

    // Histogram 色阶 / 中间调
    v.push(Variant { name: name("histo", "shadow_up"), patches: vec![("Shadow".into(), "__ARRAY_SHADOW_UP__".into())], gradations: vec![] });
    v.push(Variant { name: name("histo", "hl_down"),   patches: vec![("Highlight".into(), "__ARRAY_HL_DOWN__".into())], gradations: vec![] });
    v.push(Variant { name: name("histo", "gray_high"), patches: vec![("Gray".into(), "__ARRAY_GRAY_HIGH__".into())], gradations: vec![] });
    v.push(Variant { name: name("histo", "gray_low"),  patches: vec![("Gray".into(), "__ARRAY_GRAY_LOW__".into())], gradations: vec![] });
    v.push(Variant { name: name("histo", "grad_shadow"), patches: vec![("GradationSliders".into(), "__ARRAY_GS_SHADOW__".into())], gradations: vec![] });
    v.push(Variant { name: name("histo", "grad_mid"),    patches: vec![("GradationSliders".into(), "__ARRAY_GS_MID__".into())], gradations: vec![] });
    v.push(Variant { name: name("histo", "grad_hl"),     patches: vec![("GradationSliders".into(), "__ARRAY_GS_HL__".into())], gradations: vec![] });

    v
}

fn apply_array_sentinel(xml: String, sentinel: &str) -> String {
    match sentinel {
        "__ARRAY_SHADOW_UP__"   => replace_int_array(&xml, "Shadow",    &[2048, 2048, 2048, 2048]),
        "__ARRAY_HL_DOWN__"     => replace_int_array(&xml, "Highlight", &[12000, 12000, 12000, 12000]),
        "__ARRAY_GRAY_HIGH__"   => replace_int_array(&xml, "Gray",      &[160, 160, 160, 160]),
        "__ARRAY_GRAY_LOW__"    => replace_int_array(&xml, "Gray",      &[96, 96, 96, 96]),
        "__ARRAY_GS_SHADOW__"   => replace_int_array(&xml, "GradationSliders", &[-30, 0, 0]),
        "__ARRAY_GS_MID__"      => replace_int_array(&xml, "GradationSliders", &[0, -30, 0]),
        "__ARRAY_GS_HL__"       => replace_int_array(&xml, "GradationSliders", &[0, 0, -30]),
        _ => xml,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 把色卡数据写入所有 3 个 IFD
// ═══════════════════════════════════════════════════════════════════════════════

fn write_chart_all_ifds(data: &mut Vec<u8>) {
    use fff_viewer::tiff::{TiffFile, ByteOrder};
    let tiff = TiffFile::parse(data).expect("parse FFF");
    let byte_order = tiff.byte_order;
    let ifds: Vec<_> = tiff.ifds.iter().map(|ifd| {
        (
            ifd.get_u32(0x0100).unwrap() as u32,
            ifd.get_u32(0x0101).unwrap() as u32,
            ifd.get(0x0102).and_then(|v| v.as_u32()).unwrap_or(8) as u32,
            ifd.get(0x0111).unwrap().as_u32_vec(),
            ifd.get(0x0117).unwrap().as_u32_vec(),
        )
    }).collect();

    for (w, h, bps, offs, cnts) in ifds.iter() {
        let pixel_bytes: Vec<u8> = if *bps == 16 {
            let p = build_chart_pixels(*w, *h);
            let mut buf = Vec::with_capacity(p.len() * 2);
            for &v in &p {
                let bytes = match byte_order {
                    ByteOrder::BigEndian => v.to_be_bytes(),
                    ByteOrder::LittleEndian => v.to_le_bytes(),
                };
                buf.extend_from_slice(&bytes);
            }
            buf
        } else {
            build_chart_u8(*w, *h)
        };
        let mut src_pos = 0;
        for (off, cnt) in offs.iter().zip(cnts.iter()) {
            let start = *off as usize;
            let end = start + *cnt as usize;
            let chunk = (end - start).min(pixel_bytes.len() - src_pos);
            if end <= data.len() && chunk > 0 {
                data[start..start + chunk].copy_from_slice(&pixel_bytes[src_pos..src_pos + chunk]);
                src_pos += chunk;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// main
// ═══════════════════════════════════════════════════════════════════════════════

fn main() {
    let cfg = load_config();
    let base_data = fs::read(&cfg.base_fff)
        .unwrap_or_else(|e| panic!("读取基础 FFF 失败 ({}): {}", cfg.base_fff.display(), e));
    fs::create_dir_all(&cfg.out_dir).unwrap();
    // 按胶片类型分目录，避免单目录文件过多导致 FlexColor 崩溃
    for sub in ["chart_pos", "chart_neg", "chart_bw"] {
        fs::create_dir_all(cfg.out_dir.join(sub)).unwrap();
    }

    let (pfx, xs, alloc_end) = locate_xml_region(&base_data);

    use fff_viewer::tiff::TiffFile;
    let tiff = TiffFile::parse(&base_data).unwrap();
    let w = tiff.ifds[0].get_u32(0x0100).unwrap();
    let h = tiff.ifds[0].get_u32(0x0101).unwrap();
    println!("Base FFF dimensions: {}×{}", w, h);

    let mut cases_toml = String::from(
        "# gen_chart_ffcs 色卡变体：IFD#0/1/2 像素替换为合成色卡 + 完整 slider/曲线/色阶矩阵\n\
         # 64 色 × 4 镜像位置 = 256 tiles；与 variants_cases.toml 一一对应（前缀 c_ vs v_）\n\n\
         data_dir = \"",
    );
    cases_toml.push_str(&cfg.manifest_data_dir);
    cases_toml.push_str(
        "\"\n\
         preset_dir = \".\"\n\n",
    );

    let mut total = 0usize;
    for (prefix, tmpl_rel, mode) in TEMPLATES {
        if cfg.prefix_filter.as_ref().is_some_and(|set| !set.contains(*prefix)) {
            continue;
        }
        let tmpl_path = cfg.settings_root.join(tmpl_rel);
        let raw_template = fs::read_to_string(&tmpl_path)
            .unwrap_or_else(|e| panic!("读取模板 {} 失败: {}", tmpl_path.display(), e));
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
        for (k, val) in missing {
            if !template.contains(&format!("<key>{}</key>", k)) {
                template = insert_key_after(&template, "ApplyCC", &format!("<key>{}</key>\n\t\t\t{}", k, val));
            }
        }
        if !template.contains("<key>GradationSliders</key>") {
            template = insert_key_after(&template, "ApplyCC",
                "<key>GradationSliders</key>\n\t\t\t<array>\n\t\t\t\t<integer>0</integer>\n\t\t\t\t<integer>0</integer>\n\t\t\t\t<integer>0</integer>\n\t\t\t</array>");
        }

        println!("\n== 色卡 [{}] Mode={} ==", prefix, mode);
        let variants = build_variants_for(prefix);
        let max_cases = cfg.max_cases_per_prefix.unwrap_or(usize::MAX);
        for var in variants.iter().take(max_cases) {
            let mut patched = template.clone();
            for (k, val) in &var.patches {
                if val.starts_with("__ARRAY_") {
                    patched = apply_array_sentinel(patched, val);
                } else {
                    patched = patch_scalar(&patched, k, val);
                }
            }
            if !var.gradations.is_empty() {
                let grad_xml = serialize_gradations(&var.gradations);
                patched = replace_gradations(&patched, &grad_xml);
            }

            let inner = extract_image_setting_dict(&patched);
            let fff_xml = build_fff_xml(&inner);
            let fff_xml_bytes = fff_xml.as_bytes();

            if xs + fff_xml_bytes.len() > alloc_end {
                eprintln!("[warn] {} XML too long ({} B)", var.name, fff_xml_bytes.len());
                continue;
            }

            let mut new_data = base_data.clone();
            let len_be = (fff_xml_bytes.len() as u32).to_be_bytes();
            new_data[pfx..pfx + 4].copy_from_slice(&len_be);
            new_data[xs..xs + fff_xml_bytes.len()].copy_from_slice(fff_xml_bytes);
            for i in (xs + fff_xml_bytes.len())..alloc_end {
                new_data[i] = 0;
            }
            write_chart_all_ifds(&mut new_data);

            let subdir = format!("chart_{}", prefix);
            let out_path = cfg.out_dir.join(&subdir).join(format!("{}.fff", var.name));
            fs::write(&out_path, &new_data).unwrap();
            total += 1;

            cases_toml.push_str(&format!(
                "[[case]]\nname = \"{name}\"\nfff    = \"variants/{sub}/{name}.fff\"\nref    = \"variants/{sub}/{name}.tif\"\nsource = \"embedded_current\"\n\n",
                name = var.name, sub = subdir,
            ));
        }
        println!("  → {} variants for {}", variants.iter().take(max_cases).count(), prefix);
    }

    let _ = w; let _ = h;
    fs::write(&cfg.cases_out, cases_toml).unwrap();
    println!(
        "\n共 {} 个色卡变体 → {}\ntest_cases: {}\n\n下一步：\n  1. FlexColor 打开 {}/c_*.fff 并导出同名 TIF\n  2. cargo run --release --example tif_compare -- --manifest {} --flex-pipeline\n  3. 单色分析：cargo run --release --example chart_analyze -- <c_xxx.fff>",
        total, cfg.out_dir.display(), cfg.cases_out.display(), cfg.out_dir.display(), cfg.cases_out.display()
    );
}
