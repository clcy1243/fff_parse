//! 生成内嵌变体 XML 的 FFF 文件 —— FlexColor 打开后自动渲染变体参数。
//!
//! # 工作流
//!
//! 1. 跑本工具：`cargo run --release --example gen_variant_ffcs`
//!    → 在 `/Users/will/vmwareShare/test_image/variants/` 下生成 29 个 `*.fff`
//!    （基于 test1_raw.fff，原始扫描像素不变，仅 tag 0xC519 XML 替换为单
//!    Setting 变体）
//!
//! 2. 在 FlexColor 打开每个 `variants/*.fff`，直接导出（不需要手动选预设）
//!    → 保存为同名 `*.tif`
//!
//! 3. `cargo run --release --example tif_compare -- --manifest
//!    examples/variants_cases.toml --flex-pipeline`
//!
//! # 字节级改动
//!
//! FFF 的 tag 0xC519 布局：
//! ```
//! [offset]  4 B:  XML 长度（BE u32）
//! [+4]      N B:  XML plist 文本
//! [+4+N]   ~400 KB-4-N:  零填充
//! ```
//! 我们找到 `<?xml`，回退 4 B 读长度前缀，替换 XML 内容并更新长度前缀，
//! 剩余字节清零。IFD entry（指向 0xC519 的 offset+length）不需改动，
//! 因为 allocated size 保持不变。

use std::fs;
use std::path::PathBuf;

/// 3 个基础胶片类型模板（带期望输出 Mode）
///
/// Mode 语义（FlexColor "模式"菜单）：
/// - 0 = 8-bit RGB  ← 默认（用户已导出这批）
/// - 1 = 16-bit RGB
/// - 2 = 8-bit Gray ← 默认（BW 用户已导出这批）
/// - 4 = 8-bit CMYK
/// - 5 = 16-bit Gray
///
/// 统一强制 **16-bit 输出**（Mode=1 RGB16, Mode=5 Gray16）以获得最高拟合精度。
const TEMPLATES: &[(&str, &str, &str)] = &[
    ("pos", "/Users/will/Desktop/FFF Viewer.app/Contents/Resources/settings/Standard/RGB standard.xml", "1"),
    ("neg", "/Users/will/Desktop/FFF Viewer.app/Contents/Resources/settings/Standard Negative/Negative RGB standard.xml", "1"),
    ("bw",  "/Users/will/Desktop/FFF Viewer.app/Contents/Resources/settings/Standard Negative/B&W negative standard.xml", "5"),
];
const BASE_FFF: &str = "/Users/will/vmwareShare/test_image/test1_raw.fff";
const OUT_DIR: &str = "/Users/will/vmwareShare/test_image/variants";
const CASES_OUT: &str = "examples/variants_cases.toml";

/// 在 `<key>anchor</key>...<value>` 之后注入新的 key-value 对
/// （用于模板里缺失的字段补充）。若 anchor 未找到则原样返回。
fn insert_key_after(xml: &str, anchor: &str, new_xml: &str) -> String {
    let needle = format!("<key>{}</key>", anchor);
    let Some(pos) = xml.find(&needle) else { return xml.to_string() };
    let after_key = pos + needle.len();
    // 找下一个 <integer>/<real>/<true/>/<false/> 的闭合
    let tail = &xml[after_key..];
    // 逐个尝试四种 value 形态
    let close_patterns: &[(&str, &str)] = &[
        ("<integer>", "</integer>"),
        ("<real>", "</real>"),
        ("<true/>", ""),
        ("<false/>", ""),
        ("<string>", "</string>"),
        ("<dict>", "</dict>"),
        ("<array>", "</array>"),
    ];
    let mut end_off = None;
    for (open, close) in close_patterns {
        if let Some(o) = tail.find(open) {
            if close.is_empty() {
                // self-closing: end right after `<true/>` etc.
                end_off = Some(o + open.len());
            } else {
                // paired: find close tag
                if let Some(c) = tail[o..].find(close) {
                    end_off = Some(o + c + close.len());
                }
            }
            break;
        }
    }
    let Some(end) = end_off else { return xml.to_string() };
    let insert_at = after_key + end;
    let mut out = String::with_capacity(xml.len() + new_xml.len() + 16);
    out.push_str(&xml[..insert_at]);
    out.push_str("\n\t\t\t");
    out.push_str(new_xml);
    out.push_str(&xml[insert_at..]);
    out
}

/// 替换 plist XML 里某个 <key> 后紧邻的 <integer>/<real> 值（同 gen_variants.rs）
fn patch_scalar(xml: &str, k_name: &str, new_val: &str) -> String {
    let needle = format!("<key>{}</key>", k_name);
    let Some(key_pos) = xml.find(&needle) else {
        eprintln!("[warn] key {:?} 未找到", k_name);
        return xml.to_string();
    };
    let after_key = key_pos + needle.len();
    let tail = &xml[after_key..];
    let int_pos = tail.find("<integer>");
    let real_pos = tail.find("<real>");
    let (tag_start_off, tag_name, close_tag) = match (int_pos, real_pos) {
        (Some(i), Some(r)) if i < r => (i, "<integer>", "</integer>"),
        (Some(_), Some(r)) => (r, "<real>", "</real>"),
        (Some(i), None) => (i, "<integer>", "</integer>"),
        (None, Some(r)) => (r, "<real>", "</real>"),
        (None, None) => return xml.to_string(),
    };
    let val_start = after_key + tag_start_off + tag_name.len();
    let close_offset = tail[tag_start_off..].find(close_tag).expect("malformed");
    let val_end = after_key + tag_start_off + close_offset;
    let mut out = String::with_capacity(xml.len() + 8);
    out.push_str(&xml[..val_start]);
    out.push_str(new_val);
    out.push_str(&xml[val_end..]);
    out
}

struct Variant {
    name: String,
    patches: Vec<(String, String)>,
    /// 可选 Gradations 自定义（7 条曲线：Master / R-A / G-A / B-A / R-B / G-B / B-B）
    /// 空 Vec = 不替换（用模板默认 identity）
    gradations: Vec<Vec<(u8, u8, u8)>>, // 每条曲线 = Vec<(x, y, dy)> byte 空间
}

fn default_identity_gradation() -> Vec<(u8, u8, u8)> {
    vec![(0, 0, 1), (255, 255, 1)]
}

/// 构造 7 条 gradations，其中第 `which` 条替换为 custom curve，其余为 identity
fn build_gradations(which: usize, custom: Vec<(u8, u8, u8)>) -> Vec<Vec<(u8, u8, u8)>> {
    let mut g = vec![default_identity_gradation(); 7];
    g[which] = custom;
    g
}

/// 把 7 条 gradations 序列化为 plist XML 片段
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

/// 替换 XML 里 `<key>Gradations</key>` 后的 `<array>...</array>`（处理嵌套）
fn replace_gradations(xml: &str, new_array_xml: &str) -> String {
    let key = "<key>Gradations</key>";
    let Some(key_pos) = xml.find(key) else { return xml.to_string() };
    let after_key = key_pos + key.len();
    let tail = &xml[after_key..];
    // 找紧随的 <array>
    let Some(arr_rel) = tail.find("<array>") else { return xml.to_string() };
    let arr_start = after_key + arr_rel;
    // 配对嵌套 array
    let mut depth = 0i32;
    let mut i = arr_start;
    let bytes = xml.as_bytes();
    while i < bytes.len() {
        if xml[i..].starts_with("<array>") {
            depth += 1;
            i += 7;
        } else if xml[i..].starts_with("</array>") {
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
        } else {
            i += 1;
        }
    }
    xml.to_string()
}

/// 按变体 name 归类到子目录（~20 个/目录）
fn classify_variant(name: &str) -> &'static str {
    // name 形如 v_pos_contrast_+50 / v_pos_curve_master_scurve / v_pos_histo_grad_mid
    // 拆 segments
    if name.contains("_curve_") || name.contains("_histo_") {
        return "adv";
    }
    if name.contains("_contrast_") || name.contains("_brightness_")
        || name.contains("_gamma_") || name.contains("_baseline_") {
        return "tone";
    }
    // lightness / saturation / ev / temp / tint
    "color"
}

/// 针对某个胶片类型 (pos/neg/bw) 的变体列表
fn build_variants_for(prefix: &str) -> Vec<Variant> {
    let mut v = Vec::new();

    // 命名：v_{prefix}_{slider}_{val}
    let name = |slider: &str, v: &str| format!("v_{}_{}_{}", prefix, slider, v);

    // 稀疏档位：每个 slider 4 档，另加基线 contrast=0 用于诊断 pipeline 整体偏差
    v.push(Variant {
        name: name("baseline", "000"),
        patches: vec![],
        gradations: vec![],
    });
    // Contrast 8 档（超集包含 old ±50/±20）
    for c in [-100, -75, -50, -20, 20, 50, 75, 100] {
        v.push(Variant {
            name: name("contrast", &format!("{:+03}", c)),
            patches: vec![("Contrast".to_string(), c.to_string())],
            gradations: vec![],
        });
    }
    // Brightness 8 档
    for b in [-100, -75, -50, -20, 20, 50, 75, 100] {
        v.push(Variant {
            name: name("brightness", &format!("{:+03}", b)),
            patches: vec![("Brightness".to_string(), b.to_string())],
            gradations: vec![],
        });
    }
    // Lightness 6 档（包含 old 20/50/100）
    for l in [5, 10, 20, 50, 75, 100] {
        v.push(Variant {
            name: name("lightness", &format!("{:03}", l)),
            patches: vec![("Lightness".to_string(), l.to_string())],
            gradations: vec![],
        });
    }
    // Saturation 6 档（包含 old ±20）
    for s in [-50, -25, -20, 20, 25, 50] {
        v.push(Variant {
            name: name("saturation", &format!("{:+03}", s)),
            patches: vec![("Saturation".to_string(), s.to_string())],
            gradations: vec![],
        });
    }
    // Gamma 6 档（包含 old 1.5/2.25/2.5）
    for (tag, g) in &[
        ("050", "0.5"), ("100", "1.0"), ("150", "1.5"),
        ("225", "2.25"), ("250", "2.5"), ("300", "3.0"),
    ] {
        v.push(Variant {
            name: name("gamma", tag),
            patches: vec![("Gamma".to_string(), g.to_string())],
            gradations: vec![],
        });
    }
    // EV (exposure slider) 4 档（identity=1.0 已在 baseline 覆盖）
    for (tag, ev) in &[("050", "0.5"), ("075", "0.75"), ("150", "1.5"), ("200", "2.0")] {
        v.push(Variant {
            name: name("ev", tag),
            patches: vec![("EV".to_string(), ev.to_string())],
            gradations: vec![],
        });
    }
    // ColorTemperature 3 档
    for t in [-50, 25, 50] {
        v.push(Variant {
            name: name("temp", &format!("{:+03}", t)),
            patches: vec![("ColorTemperature".to_string(), t.to_string())],
            gradations: vec![],
        });
    }
    // Tint 3 档
    for t in [-50, 25, 50] {
        v.push(Variant {
            name: name("tint", &format!("{:+03}", t)),
            patches: vec![("Tint".to_string(), t.to_string())],
            gradations: vec![],
        });
    }

    // ── Gradations 曲线变体（CPointCurve 测试）────────────────────────
    // 索引: 0=Master, 1=R-A, 2=G-A, 3=B-A, 4=R-B, 5=G-B, 6=B-B
    let curve_cases: &[(&str, usize, Vec<(u8, u8, u8)>)] = &[
        // Master S-curve（增对比）
        ("curve_master_scurve", 0, vec![(0,0,1),(64,32,1),(192,224,1),(255,255,1)]),
        // Master 抬暗部（lift shadows）
        ("curve_master_lift", 0, vec![(0,32,1),(128,160,1),(255,255,1)]),
        // Master 压亮部（darken highlights）
        ("curve_master_darken", 0, vec![(0,0,1),(128,96,1),(255,224,1)]),
        // Master 反转（逆向）
        ("curve_master_invert", 0, vec![(0,255,1),(255,0,1)]),
        // R-A 通道 lift（偏暖）
        ("curve_r_warm", 1, vec![(0,0,1),(128,160,1),(255,255,1)]),
        // B-A 通道 lift（偏冷）
        ("curve_b_cool", 3, vec![(0,0,1),(128,160,1),(255,255,1)]),
        // G-A 通道 lift（偏绿）
        ("curve_g_lift", 2, vec![(0,0,1),(128,160,1),(255,255,1)]),
    ];
    for (cname, which, pts) in curve_cases {
        v.push(Variant {
            name: format!("v_{}_{}", prefix, cname),
            patches: vec![],
            gradations: build_gradations(*which, pts.clone()),
        });
    }

    // ── Histogram (Shadow / Gray / Highlight) 色阶 & 中间调变体 ──────
    // Shadow: ushort[4] 14-bit，模板默认 960/960/960/960（1/16 of 16384）
    // Highlight: 14400/14400/14400/14400 (7/8 of 16384)
    // Gray: byte[4] 0..255，128=identity
    // GradationSliders: [shadow, midtone, highlight] int
    //
    // 1. 抬 shadow boundary（压缩暗部）
    v.push(Variant { name: name("histo", "shadow_up"), patches: vec![
        ("Shadow".to_string(), "__ARRAY_SHADOW_UP__".to_string()),  // sentinel — 实际 patch 见 apply_array
    ], gradations: vec![] });
    // 2. 压 highlight boundary（压缩亮部）
    v.push(Variant { name: name("histo", "hl_down"), patches: vec![
        ("Highlight".to_string(), "__ARRAY_HL_DOWN__".to_string()),
    ], gradations: vec![] });
    // 3. 抬 gray midtone（暗化中间）
    v.push(Variant { name: name("histo", "gray_high"), patches: vec![
        ("Gray".to_string(), "__ARRAY_GRAY_HIGH__".to_string()),
    ], gradations: vec![] });
    // 4. 压 gray midtone（亮化中间）
    v.push(Variant { name: name("histo", "gray_low"), patches: vec![
        ("Gray".to_string(), "__ARRAY_GRAY_LOW__".to_string()),
    ], gradations: vec![] });
    // 5/6/7. GradationSliders 快捷调整（shadow/midtone/highlight ±N）
    v.push(Variant { name: name("histo", "grad_shadow"), patches: vec![
        ("GradationSliders".to_string(), "__ARRAY_GS_SHADOW__".to_string()),
    ], gradations: vec![] });
    v.push(Variant { name: name("histo", "grad_mid"), patches: vec![
        ("GradationSliders".to_string(), "__ARRAY_GS_MID__".to_string()),
    ], gradations: vec![] });
    v.push(Variant { name: name("histo", "grad_hl"), patches: vec![
        ("GradationSliders".to_string(), "__ARRAY_GS_HL__".to_string()),
    ], gradations: vec![] });

    v
}

/// 针对整数数组类 key 替换 `<array>...</array>` 内容
fn replace_int_array(xml: &str, key: &str, values: &[i32]) -> String {
    let needle = format!("<key>{}</key>", key);
    let Some(pos) = xml.find(&needle) else { return xml.to_string() };
    let after_key = pos + needle.len();
    let tail = &xml[after_key..];
    let Some(arr_rel) = tail.find("<array>") else { return xml.to_string() };
    let arr_start = after_key + arr_rel + 7;  // after <array>
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

/// 从 preset XML (ImageSetting 单体) 抽取 `<dict>...</dict>` 内容
/// —— 即 ImageSetting 的完整 dict 体（含 Created/ImageCorrection 等）。
fn extract_image_setting_dict(preset_xml: &str) -> String {
    let key = "<key>ImageSetting</key>";
    let pos = preset_xml.find(key).expect("没找到 ImageSetting key");
    // 找下一个 <dict>
    let after_key = pos + key.len();
    let dict_start = preset_xml[after_key..].find("<dict>").expect("没找到 <dict>") + after_key;
    // 配对 </dict>（考虑嵌套）
    let mut depth = 0i32;
    let mut i = dict_start;
    let bytes = preset_xml.as_bytes();
    while i < bytes.len() {
        if preset_xml[i..].starts_with("<dict>") {
            depth += 1;
            i += 6;
        } else if preset_xml[i..].starts_with("</dict>") {
            depth -= 1;
            if depth == 0 {
                return preset_xml[dict_start..i + 7].to_string();
            }
            i += 7;
        } else {
            i += 1;
        }
    }
    panic!("未找到配对 </dict>");
}

/// 把变体 XML 包装成 FFF 期望的格式（ImageSettings 数组 + CurrentIx=0）
fn build_fff_xml(image_setting_dict: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple Computer//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
  <dict>\n\
    <key>ImageSettings</key>\n\
    <array>\n\
      {}\n\
    </array>\n\
    <key>CurrentIx</key>\n\
    <integer>0</integer>\n\
  </dict>\n\
</plist>\n",
        image_setting_dict
    )
}

/// 在 FFF 二进制数据中定位 XML 区域 → 回报 (length_prefix_offset, xml_start, allocated_end)
///
/// allocated_end：原始 XML + 后续零填充的末尾（= 下一个非零 byte - 1）。
fn locate_xml_region(data: &[u8]) -> (usize, usize, usize) {
    let xml_start = data.windows(5)
        .position(|w| w == b"<?xml")
        .expect("FFF 中找不到 <?xml>");
    let prefix_off = xml_start - 4;

    // 原 XML 结束位置（`</plist>` 末尾）
    let xml_end = data[xml_start..]
        .windows(8)
        .position(|w| w == b"</plist>")
        .expect("找不到 </plist>")
        + xml_start + 8;

    // 分配边界：原 XML end 之后的零填充到下一个非零 byte
    // 为安全，从 xml_end 往前扫到 400KB 后（docs 说 ~400076 = 0xC519 结束），
    // 我们仅保证不写超过这个保守边界。
    let allocated_end = xml_start + 400_000 - 4; // 留 400KB 减去前缀
    (prefix_off, xml_start, allocated_end.min(data.len()))
}

fn main() {
    let base_data = fs::read(BASE_FFF).expect("读取基础 FFF 失败");

    let out_dir = PathBuf::from(OUT_DIR);
    fs::create_dir_all(&out_dir).unwrap();

    let (prefix_off, xml_start, allocated_end) = locate_xml_region(&base_data);
    println!("Base FFF: XML at {} (prefix at {}), allocated end {}",
        xml_start, prefix_off, allocated_end);

    let mut cases_toml = String::from(
        "# gen_variant_ffcs 生成的内嵌 XML 变体 FFF 测试用例\n\
         # 每个文件：原始扫描像素 = test1_raw.fff，仅嵌入 XML 改为单 Setting\n\
         # 前缀 v_pos/v_neg/v_bw 标识胶片类型\n\
         # 用法：FlexColor 打开 variants/*.fff，直接 Export TIF 到同名 .tif\n\n\
         data_dir = \"/Users/will/vmwareShare/test_image\"\n\
         preset_dir = \".\"\n\n",
    );

    let mut total = 0usize;

    for (prefix, tmpl_path, mode) in TEMPLATES {
        let raw_template = fs::read_to_string(tmpl_path)
            .unwrap_or_else(|e| panic!("读取模板 {} 失败: {}", tmpl_path, e));
        // 强制设置输出 Mode（pos/neg=1=RGB16, bw=5=Gray16）
        let mut template = patch_scalar(&raw_template, "Mode", mode);
        // 清零 Crop（BW 模板带非零裁切导致 FlexColor 导出缩略图尺寸）
        for k in ["CropBottom", "CropLeft", "CropRight", "CropTop"] {
            template = patch_scalar(&template, k, "0.0");
        }
        // Frame=1 表示全图（BW 模板默认 40 是帧编号？保底设 1）
        template = patch_scalar(&template, "Frame", "1");

        // 模板里 ".dp:" 和 ".dfR:" 是 FlexColor 的 profile 占位符，会在渲染时解析为
        // 用户当前默认 profile，这会导致导出 TIF 的像素值依赖全局配置。
        // 用 Setting #24 实际的具体 profile 名字替换，保证渲染一致。
        template = template.replace(
            "<string>.dp:</string>",
            "<string>Flextight X5 &amp; 949</string>",
        );
        template = template.replace(
            "<string>.dfR:</string>",
            "<string></string>",
        );

        // 补充模板缺失但 FFF 默认需要的字段（Flextight X5 scanner 默认值）
        // 对比 test1_raw.fff Setting #24 确认 9 个 key 缺失
        let missing_defaults: &[(&str, &str)] = &[
            ("FilmCurve",         "<integer>4</integer>"),      // Film Auto
            ("ApplyCNFilter",     "<true/>"),
            ("LensCorrection",    "<integer>7</integer>"),       // Flextight X5 preset
            ("VignetteAmount",    "<integer>100</integer>"),
            ("ColorModel",        "<integer>0</integer>"),       // RGB
            ("ColorTemperature",  "<integer>0</integer>"),
            ("Tint",              "<integer>0</integer>"),
            ("EV",                "<real>1.0</real>"),           // identity exposure
            ("ColorNoiseRadius",  "<integer>0</integer>"),
            ("NoiseFilterBias",   "<integer>0</integer>"),
            // GradationSliders is an array — 模板基本应有它，若缺用 [0,0,0]
        ];
        for (key, value) in missing_defaults {
            if !template.contains(&format!("<key>{}</key>", key)) {
                let insert = format!("<key>{}</key>\n\t\t\t{}", key, value);
                template = insert_key_after(&template, "ApplyCC", &insert);
            }
        }
        if !template.contains("<key>GradationSliders</key>") {
            template = insert_key_after(&template, "ApplyCC",
                "<key>GradationSliders</key>\n\t\t\t<array>\n\t\t\t\t<integer>0</integer>\n\t\t\t\t<integer>0</integer>\n\t\t\t\t<integer>0</integer>\n\t\t\t</array>");
        }
        println!("\n== 模板 [{}] Mode={} → {} ==", prefix, mode, tmpl_path);

        let variants = build_variants_for(prefix);
        for var in &variants {
            let mut patched = template.clone();
            for (k, v) in &var.patches {
                // sentinel 识别数组类 patch
                if v.starts_with("__ARRAY_") {
                    patched = match v.as_str() {
                        // Shadow 抬到 2048（1/8，压暗）
                        "__ARRAY_SHADOW_UP__" =>
                            replace_int_array(&patched, "Shadow", &[2048, 2048, 2048, 2048]),
                        // Highlight 降到 12000（压亮）
                        "__ARRAY_HL_DOWN__" =>
                            replace_int_array(&patched, "Highlight", &[12000, 12000, 12000, 12000]),
                        // Gray 抬到 160（暗化中间）
                        "__ARRAY_GRAY_HIGH__" =>
                            replace_int_array(&patched, "Gray", &[160, 160, 160, 160]),
                        // Gray 降到 96（亮化中间）
                        "__ARRAY_GRAY_LOW__" =>
                            replace_int_array(&patched, "Gray", &[96, 96, 96, 96]),
                        // GradationSliders shadow 拉（S-curve 左移）
                        "__ARRAY_GS_SHADOW__" =>
                            replace_int_array(&patched, "GradationSliders", &[-30, 0, 0]),
                        // GradationSliders mid 拉
                        "__ARRAY_GS_MID__" =>
                            replace_int_array(&patched, "GradationSliders", &[0, -30, 0]),
                        // GradationSliders hi 拉
                        "__ARRAY_GS_HL__" =>
                            replace_int_array(&patched, "GradationSliders", &[0, 0, -30]),
                        _ => patched,
                    };
                } else {
                    patched = patch_scalar(&patched, k, v);
                }
            }
            // Gradations 曲线替换（若定义）
            if !var.gradations.is_empty() {
                let grad_xml = serialize_gradations(&var.gradations);
                patched = replace_gradations(&patched, &grad_xml);
            }
            let inner = extract_image_setting_dict(&patched);
            let fff_xml = build_fff_xml(&inner);
            let fff_xml_bytes = fff_xml.as_bytes();

            if xml_start + fff_xml_bytes.len() > allocated_end {
                eprintln!("[warn] {} XML 太长 ({} B)，跳过", var.name, fff_xml_bytes.len());
                continue;
            }

            let mut new_data = base_data.clone();
            let len_be = (fff_xml_bytes.len() as u32).to_be_bytes();
            new_data[prefix_off..prefix_off + 4].copy_from_slice(&len_be);
            new_data[xml_start..xml_start + fff_xml_bytes.len()].copy_from_slice(fff_xml_bytes);
            for i in (xml_start + fff_xml_bytes.len())..allocated_end {
                new_data[i] = 0;
            }

            // 分目录（每 ~20 个文件一组，避免 FlexColor 打开目录时崩溃）
            // variants/v_{prefix}_tone/: baseline+contrast+brightness+gamma (23)
            // variants/v_{prefix}_color/: lightness+saturation+ev+temp+tint (22)
            // variants/v_{prefix}_adv/:  curve+histo (14)
            let category = classify_variant(&var.name);
            let subdir = format!("v_{}_{}", prefix, category);
            let sub_path = out_dir.join(&subdir);
            fs::create_dir_all(&sub_path).unwrap();
            let out_path = sub_path.join(format!("{}.fff", var.name));
            fs::write(&out_path, &new_data).unwrap();
            println!("✓ {} ({} B XML)", out_path.display(), fff_xml_bytes.len());

            cases_toml.push_str(&format!(
                "[[case]]\n\
                 name = \"{name}\"\n\
                 fff    = \"variants/{sub}/{name}.fff\"\n\
                 ref    = \"variants/{sub}/{name}.tif\"\n\
                 source = \"embedded_current\"\n\n",
                name = var.name, sub = subdir,
            ));
            total += 1;
        }
    }

    fs::write(CASES_OUT, cases_toml).unwrap();
    println!(
        "\n共 {} 个变体 FFF → {}（9 子目录 × ~6-23 个，避免 FlexColor 崩溃）\n\
         test_cases: {}\n\n\
         下一步：\n  1. FlexColor 依次打开 v_*/*.fff 目录导出同名 TIF\n  2. cargo run --release --example tif_compare -- --manifest {} --flex-pipeline",
        total, out_dir.display(), CASES_OUT, CASES_OUT
    );
}
