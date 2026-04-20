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
}

/// 针对某个胶片类型 (pos/neg/bw) 的变体列表
fn build_variants_for(prefix: &str) -> Vec<Variant> {
    let mut v = Vec::new();

    // 命名：v_{prefix}_{slider}_{val}
    let name = |slider: &str, v: &str| format!("v_{}_{}_{}", prefix, slider, v);

    // 稀疏档位：每个 slider 4 档，足够拟合公式（避免 102MB × 87 文件超盘）
    for c in [-50, -20, 20, 50] {
        v.push(Variant {
            name: name("contrast", &format!("{:+03}", c)),
            patches: vec![("Contrast".to_string(), c.to_string())],
        });
    }
    for b in [-50, -20, 20, 50] {
        v.push(Variant {
            name: name("brightness", &format!("{:+03}", b)),
            patches: vec![("Brightness".to_string(), b.to_string())],
        });
    }
    for l in [20, 50, 100] {
        v.push(Variant {
            name: name("lightness", &format!("{:03}", l)),
            patches: vec![("Lightness".to_string(), l.to_string())],
        });
    }
    // BW 模式下 Saturation 影响 ColorCorr 进链（最终 desaturate 成 gray）
    for s in [-20, 20] {
        v.push(Variant {
            name: name("saturation", &format!("{:+03}", s)),
            patches: vec![("Saturation".to_string(), s.to_string())],
        });
    }
    for (tag, g) in &[("150", "1.5"), ("225", "2.25"), ("250", "2.5")] {
        v.push(Variant {
            name: name("gamma", tag),
            patches: vec![("Gamma".to_string(), g.to_string())],
        });
    }
    v
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

        // 补充模板缺失但 FFF 默认需要的字段（否则 FlexColor 渲染偏亮）
        //   FilmCurve=4 (Film Auto) — 胶片曲线 LUT 必须应用
        //   ApplyCNFilter=true — 色彩噪声滤镜（默认开）
        if !template.contains("<key>FilmCurve</key>") {
            template = insert_key_after(&template, "ApplyCC",
                "<key>FilmCurve</key>\n\t\t\t<integer>4</integer>");
        }
        if !template.contains("<key>ApplyCNFilter</key>") {
            template = insert_key_after(&template, "ApplyCC",
                "<key>ApplyCNFilter</key>\n\t\t\t<true/>");
        }
        println!("\n== 模板 [{}] Mode={} → {} ==", prefix, mode, tmpl_path);

        let variants = build_variants_for(prefix);
        for var in &variants {
            let mut patched = template.clone();
            for (k, v) in &var.patches {
                patched = patch_scalar(&patched, k, v);
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

            let out_path = out_dir.join(format!("{}.fff", var.name));
            fs::write(&out_path, &new_data).unwrap();
            println!("✓ {} ({} B XML)", out_path.display(), fff_xml_bytes.len());

            cases_toml.push_str(&format!(
                "[[case]]\n\
                 name = \"{name}\"\n\
                 fff    = \"variants/{name}.fff\"\n\
                 ref    = \"variants/{name}.tif\"\n\
                 source = \"embedded_current\"\n\n",
                name = var.name,
            ));
            total += 1;
        }
    }

    fs::write(CASES_OUT, cases_toml).unwrap();
    println!(
        "\n共 {} 个变体 FFF → {}\ntest_cases: {}\n\n下一步：\n  1. FlexColor 打开 {}/v_*.fff 并导出同名 TIF\n  2. cargo run --release --example tif_compare -- --manifest {} --flex-pipeline",
        total, out_dir.display(), CASES_OUT, OUT_DIR, CASES_OUT
    );
}
