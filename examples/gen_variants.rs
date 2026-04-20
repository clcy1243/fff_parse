//! 生成单 slider 隔离的预设 XML 变体，供 FlexColor 渲染 + bit-accurate 拟合用。
//!
//! # 工作流
//!
//! 1. 跑本工具：`cargo run --release --example gen_variants`
//!    → 在 `profiles/variants/` 下生成一批 `*.xml` 预设
//!    → 同时在 `examples/variants_cases.toml` 追加 test_cases.toml 兼容条目
//!
//! 2. 把 `profiles/variants/` 复制到 FlexColor 的 Settings/Standard 目录
//!    （或者 macOS 用户通过 "Load Settings" 逐个打开）
//!
//! 3. 在 FlexColor 打开 `test1_raw.fff`（或其他 base FFF），逐个载入预设、
//!    导出 TIF 到 `/Users/will/vmwareShare/test_image/variants/<name>.tif`
//!
//! 4. 运行 `cargo run --release --example tif_compare -- --manifest
//!    examples/variants_cases.toml --flex-pipeline`
//!    → 每个 slider 变体单独 MAE，快速定位实现偏差
//!
//! # 生成矩阵
//!
//! - 基线：以 `RGB standard.xml`（正片 Standard 预设）为模板
//! - 每个变体**只改一个字段**，其余保持模板值
//! - slider 档位：每个 slider 取 5-7 档（含负值 / 零 / 正值）

use std::fs;
use std::path::{Path, PathBuf};

const TEMPLATE_PATH: &str =
    "/Users/will/Desktop/FFF Viewer.app/Contents/Resources/settings/Standard/RGB standard.xml";

const OUT_DIR: &str = "profiles/variants";
const CASES_OUT: &str = "examples/variants_cases.toml";
const REF_TIF_DIR: &str = "/Users/will/vmwareShare/test_image/variants";
const BASE_FFF: &str = "test1_raw.fff";

/// 替换 plist XML 里某个 <key> 后的 <integer>/<real> 值。
///
/// `k_name` 是 key 名（如 "Contrast"），`new_val` 是新值字符串（如 "20" 或 "1.5"）。
/// 保留原 tag 类型（integer/real）。
fn patch_scalar(xml: &str, k_name: &str, new_val: &str) -> String {
    // 匹配模式：<key>Xxx</key>\n\t\t\t<integer>N</integer>
    //       或：<key>Xxx</key>\n\t\t\t<real>N</real>
    let needle = format!("<key>{}</key>", k_name);
    let Some(key_pos) = xml.find(&needle) else {
        eprintln!("[warn] key {:?} 未找到", k_name);
        return xml.to_string();
    };
    let after_key = key_pos + needle.len();

    // 在 key 后找紧邻的 <integer> 或 <real> — 谁 index 小用谁
    let tail = &xml[after_key..];
    let int_pos = tail.find("<integer>");
    let real_pos = tail.find("<real>");
    let (tag_start_off, tag_name, close_tag) = match (int_pos, real_pos) {
        (Some(i), Some(r)) if i < r => (i, "<integer>", "</integer>"),
        (Some(_), Some(r)) => (r, "<real>", "</real>"),
        (Some(i), None) => (i, "<integer>", "</integer>"),
        (None, Some(r)) => (r, "<real>", "</real>"),
        (None, None) => {
            eprintln!("[warn] key {:?} 后没有找到 <integer>/<real>", k_name);
            return xml.to_string();
        }
    };

    // 找到 close tag
    let val_start = after_key + tag_start_off + tag_name.len();
    let close_offset = tail[tag_start_off..]
        .find(close_tag)
        .expect("plist 格式错误：未闭合");
    let val_end = after_key + tag_start_off + close_offset;

    let mut out = String::with_capacity(xml.len() + 8);
    out.push_str(&xml[..val_start]);
    out.push_str(new_val);
    out.push_str(&xml[val_end..]);
    out
}

struct Variant {
    /// 文件名（不含后缀），同时用作 preset 标识
    name: String,
    /// key -> 新值字符串的 patches（单 slider 变体只有一项，
    /// 但多项组合也支持）
    patches: Vec<(String, String)>,
}

fn build_variants() -> Vec<Variant> {
    let mut v = Vec::new();

    // ── Contrast 7 档 ──────────────────────────────
    for c in [-100, -50, -20, 0, 20, 50, 100] {
        v.push(Variant {
            name: format!("v_contrast_{:+03}", c),
            patches: vec![("Contrast".to_string(), c.to_string())],
        });
    }

    // ── Brightness 7 档 ────────────────────────────
    for b in [-100, -50, -20, 0, 20, 50, 100] {
        v.push(Variant {
            name: format!("v_brightness_{:+03}", b),
            patches: vec![("Brightness".to_string(), b.to_string())],
        });
    }

    // ── Lightness 5 档（Y1=L*2.5+2 仅正值有意义）──
    for l in [0, 20, 50, 75, 100] {
        v.push(Variant {
            name: format!("v_lightness_{:03}", l),
            patches: vec![("Lightness".to_string(), l.to_string())],
        });
    }

    // ── Saturation 5 档 ────────────────────────────
    for s in [-50, -20, 0, 20, 50] {
        v.push(Variant {
            name: format!("v_saturation_{:+03}", s),
            patches: vec![("Saturation".to_string(), s.to_string())],
        });
    }

    // ── Gamma 5 档（real，非 integer）─────────────
    for (tag, g) in &[
        ("150", "1.5"),
        ("175", "1.75"),
        ("200", "2.0"),
        ("225", "2.25"),
        ("250", "2.5"),
    ] {
        v.push(Variant {
            name: format!("v_gamma_{}", tag),
            patches: vec![("Gamma".to_string(), g.to_string())],
        });
    }

    v
}

fn main() {
    let template = fs::read_to_string(TEMPLATE_PATH)
        .expect("读取模板 XML 失败，请确认 FFF Viewer.app 已在桌面");

    let out_dir = PathBuf::from(OUT_DIR);
    fs::create_dir_all(&out_dir).unwrap();

    let mut cases_toml = String::from(
        "# 自动生成（gen_variants）：单 slider 隔离的测试 case\n\
         # 用法：在 FlexColor 打开 BASE_FFF，分别载入每个 variant XML，\n\
         #      导出 TIF 到 REF_TIF_DIR；然后 tif_compare --manifest.\n\n",
    );

    let variants = build_variants();
    for var in &variants {
        let mut xml = template.clone();
        for (key, val) in &var.patches {
            xml = patch_scalar(&xml, key, val);
        }

        let xml_path = out_dir.join(format!("{}.xml", var.name));
        fs::write(&xml_path, &xml).unwrap();
        println!("✓ {}", xml_path.display());

        // toml fragment
        cases_toml.push_str(&format!(
            "[[case]]\n\
             name = \"{name}\"\n\
             fff    = \"{fff}\"\n\
             ref    = \"variants/{name}.tif\"\n\
             source = \"external_xml\"\n\
             preset = \"variants/{name}.xml\"\n\n",
            name = var.name,
            fff = BASE_FFF,
        ));
    }

    fs::write(CASES_OUT, cases_toml).unwrap();
    println!(
        "\n生成 {} 个变体 → {}\n生成 test_cases: {}\n",
        variants.len(),
        out_dir.display(),
        CASES_OUT,
    );
    println!(
        "下一步：\n  1. 在 FlexColor 打开 {}\n  2. 逐个载入 profiles/variants/*.xml 并导出 TIF 到 {}/\n  3. 跑 cargo run --release --example tif_compare -- --manifest {} --flex-pipeline",
        BASE_FFF, REF_TIF_DIR, CASES_OUT
    );

    let _ = Path::new(REF_TIF_DIR); // 文档用途
}
