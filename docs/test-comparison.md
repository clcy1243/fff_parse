# 测试与对比工具架构 (tif_compare v3)

> 最后更新：2026-04-18

本文档详细描述 `examples/tif_compare.rs` 的设计、使用方法和扩展点。该工具是色彩管线精度回归的核心基础设施。

---

## 设计目标

1. **严苛**：支持逐像素 16-bit 级别对比，识别 1/65535 量级的误差。
2. **可归因**：每个误差应能追到具体像素、通道、亮度区间、管线阶段。
3. **可回归**：支持基线 JSON diff，任何改动的影响一目了然。
4. **多来源预设**：同时支持 FFF 内嵌 history 和外部 FlexColor XML 预设。
5. **ref-ICC 对齐**：参考 TIF 的像素在 FlexColor 定义的色彩空间中，对比时必须在同一空间。

---

## 预设来源抽象

FlexColor 的图像校正参数（`ImageCorrection`）有三种来源：

```rust
enum PresetSource {
    EmbeddedCurrent,              // FFF 的 edit history current_index
    EmbeddedIndex(usize),         // FFF history 按索引
    EmbeddedName(String),         // FFF history 按名字匹配
    ExternalXml(PathBuf),         // settings/**/*.xml 外部预设
}
```

### 外部 XML 特殊处理

FlexColor 的 `settings/**/*.xml` 文件**不包含**以下键（FlexColor 把它们当作全局 UI 状态，不序列化进预设）：

| 缺失键 | FlexColor 默认值 | 我们的处理 |
|---|---|---|
| `FilmCurve` | 4 ("Film Auto") | `apply_xml_preset_defaults` 回填 4 |
| `ColorModel` | 依 UI 当前选择 | 按文件名推断：`cmyk/bw/rgb` 字符串匹配 |

**XML 源文件本身不修改** — 它们是 FlexColor 的原始资产。补默认只在 tif_compare 的 `resolve_preset()` 中进行。

实现：`examples/tif_compare.rs::apply_xml_preset_defaults`

---

## Manifest 驱动的批量测试

### 格式

`examples/test_cases.toml` 顶层字段：

```toml
data_dir   = "/Users/will/vmwareShare/test_image"   # 绝对路径或相对本 toml
preset_dir = "../settings"                          # XML 预设目录
```

每个 case：

```toml
[[case]]
name   = "emb_rgb_standard"              # 唯一标识
fff    = "test1_raw.fff"                 # 相对 data_dir
ref    = "test1_raw_rgb_standard_16bit.tif"
source = "embedded_name"                 # 见下表
preset = "test1_raw_rgb_standard"        # 依 source 不同含义不同
```

`source` 的取值：

| source | preset 的含义 | 用途 |
|---|---|---|
| `embedded_current` | （忽略） | FFF 的 current_index 设置 |
| `embedded_index` | 整数字符串 | FFF history 第 N 个 |
| `embedded_name` | 设置名 | FFF history 按 name 匹配 |
| `external_xml` | XML 相对 preset_dir 路径 | 外部 FlexColor 预设文件 |

### 路径解析规则

- `data_dir` / `preset_dir`：相对 manifest 文件所在目录
- case 的 `fff` / `ref`：相对 `data_dir`
- case 的 `preset`（当 source = external_xml）：相对 `preset_dir`

### 当前矩阵（29 cases）

| 类别 | 数量 | 覆盖 |
|------|------|------|
| e2e（FFF 自带 current） | 3 | default / all_config / all_config_bw |
| 内嵌 history by-name | 13 | 13 个预设变体的 "用户应用后" 快照 |
| 外部 XML | 13 | 同 13 个预设的 FlexColor 原始 XML |

对同一个预设跑 embedded 和 external 两次，可以验证 XML 加载的完整性（应收敛到相同结果）。

---

## 指标体系

### 主指标（全 16-bit 空间）

| 指标 | 含义 |
|------|------|
| **MAE_16** | 平均绝对误差（u16 量级） |
| **Mean Signed Error** | 每通道带符号均值，暴露系统性偏置方向 |
| **P50 / P95 / P99 / P999** | 误差分位数，基于 65536-bucket 直方图精确计算 |
| **Max** | 单像素最大误差 |

### 分区间 MAE（按参考亮度）

| 区间 | 条件 |
|---|---|
| shadow | ref < 20%（ref < 13107） |
| mid | 20% ≤ ref ≤ 80% |
| highlight | ref > 80%（ref > 52428） |

揭示误差是否集中在某个亮度段（例如 rgb_standard 整体 MAE 376 但 highlight MAE 10376）。

### 感知色差

| 指标 | 含义 |
|---|---|
| **ΔE2000 mean** | CIEDE2000 色差均值 |
| **ΔE2000 P95** | 95% 分位 |

采样到 500K 点以控制耗时。JND（Just Noticeable Difference）通常 ≈ 2.3，ΔE P95 < 2.3 可认为 95% 像素视觉等同。

### 8-bit 辅助

保留 MAE_8 / P99_8 / PSNR_8 作为参考（与旧版工具可比）。

---

## 评级体系

四档，**与 AND 的关系**（mae 和 p99 都满足才得该级）：

| 等级 | mae_16 | p99_16 | 视觉意义 |
|---|---|---|---|
| **STRICT** 🟢 | ≤ 100 | ≤ 500 | 逐像素近乎无差异（8-bit < 0.4） |
| **PASS** ✅ | ≤ 400 | ≤ 2000 | 整体不可察觉（8-bit < 1.5） |
| **WARN** ⚠️ | ≤ 1280 | ≤ 5000 | 细看可辨，总体仍相似 |
| **FAIL** ❌ | 其他 | | 明显色彩差异 |

当前最高档 STRICT 等级设计为"肉眼看不出区别"。更严格的档位（如逐位等同）是后续阶段目标。

---

## 测试矩阵 (T1–T4)

每个 case 跑 4 个测试：

| ID | 变体 | 作用 |
|----|------|------|
| **T1** | 完整管线（ref-ICC 或 sRGB 回退） | **主验收指标** |
| T2 | 消融：关 ICC | 证明 ICC 是否引入误差 |
| T3 | 消融：强制硬编码 LUT | 对比 LUT 来源影响 |
| T4 | 消融：完全关胶片曲线 | 验证曲线是否必要 |

### T1 ICC 选择逻辑

1. 从参考 TIF 抽出嵌入 ICC (`extract_tif_icc`)
2. 若存在且是 RGB profile → 作为输出 profile，调 `apply_icc_transform_profiles`
3. 若输入 ICC 与输出 ICC **字节完全相同** → 短路，跳过变换（避免 Lab PCS 量化误差）
4. 若输出 profile 非 RGB（CMYK / Gray） → 回退到 `TargetColorSpace::SRGB`
5. 参考 TIF 无嵌入 ICC → 也回退 sRGB

---

## 诊断输出

### 误差热图 `--dump-errmap DIR`

对 T1 每像素计算 max channel diff（u16），映射到 colormap（蓝→青→绿→黄→红），保存为 8-bit PNG。最大宽度下采样到 1200px。

颜色刻度：scale_max = 2000（u16）→ 红色。蓝色 = 差值 0，红色 = 差值 ≥2000。

### 最差像素列表 `--find-worst N`

T1 扫描过程中维护一个 size-N 的小根堆，记录 N 个最大误差像素：
```
  Worst 5 pixels (by max channel diff):
    (2139, 34) err=16718  ours=[39394,28890,19992]  ref=[56112,45608,36708]
```
可用于定位误差热区（比如是否都集中在某行/某色相/某亮度段）。

### 元信息检查 `--meta-check`

```
  ─── meta check ───
  Ref ColorType: Rgb16
  Dims:    (3601, 4489)
  Preset:  embedded[3] "test1_raw_rgb_standard"
  ICC:     yes
  Film LUT:hardcoded
  Setting: film_type=0 film_curve=4 γ=2.00 colormodel=0
```

若参考 TIF 非 16-bit，会警告"MAE16 数值是 8→16 扩展后的结果，参考价值下降"。

### Baseline 回归 `--baseline prev.json`

与之前的 JSON 输出对比，显示每个指标的 `↑/↓/=` 箭头与 delta。

---

## CLI 接口

```bash
# 单文件（默认用 FFF current 设置）
tif_compare <file.fff> <ref.tif>

# 指定预设来源
tif_compare <file.fff> <ref.tif> --setting 3
tif_compare <file.fff> <ref.tif> --setting-name "test1_raw_rgb_standard"
tif_compare <file.fff> <ref.tif> --preset-xml "settings/Standard/RGB standard.xml"

# 目录批量（按同名配对 *.fff + *.tif）
tif_compare --dir <path>

# Manifest 驱动（推荐）
tif_compare --manifest examples/test_cases.toml

# 诊断选项
tif_compare <args> -v                          # 详细每测试报告
tif_compare <args> --json                      # 机器可读输出
tif_compare <args> --find-worst 20             # 最差 20 像素
tif_compare <args> --dump-errmap /tmp/heat     # 热图 PNG
tif_compare <args> --meta-check                # 元信息
tif_compare <args> --baseline prev.json        # 回归对比
tif_compare <args> --no-lut-extract            # 强制硬编码 LUT
tif_compare <args> --trace X,Y                 # 追踪单像素各阶段值
tif_compare <args> --calibrate-usm             # 按 σ/k 标定 USM 参数
tif_compare <args> --use-ref-lut               # 用参考 TIF 反推 per-preset LUT（负片用）

# ICC 参数（实验用）
tif_compare <args> --icc-intent perceptual|relative|absolute|saturation
tif_compare <args> --icc-bpc                   # 启用黑点补偿（默认关）
tif_compare <args> --icc-no-bpc
```

**退出码**：任何 case 的任何测试 FAIL → exit 1；否则 0。可直接接 CI。

---

## 代码架构

```
examples/tif_compare.rs (~1400 行)
├─ 数据类型
│   ├─ Grade (STRICT/PASS/WARN/FAIL)
│   ├─ Stats16（16-bit 指标集合）
│   ├─ Accumulator（单-pass 统计累加器，含 65536-bucket 直方图）
│   ├─ TestResult（单次测试完整结果）
│   ├─ WorstPixel（定位误差峰值）
│   ├─ Case / Manifest（用例声明）
│   ├─ PresetSource（预设来源枚举）
│   └─ FileContext / ResolvedPreset / RunOptions
├─ 统计计算
│   ├─ Accumulator::add / to_stats
│   ├─ percentile_16 / grade_stats
│   ├─ srgb_to_lab / delta_e_2000 / delta_e_stats
│   └─ compare_images（核心对比入口）
├─ 预设处理
│   ├─ resolve_preset（三种来源统一出口）
│   ├─ apply_xml_preset_defaults（XML 缺字段补全）
│   └─ build_manual_adjust（ImageCorrection → ManualAdjust）
├─ 管线调度
│   ├─ load_context（加载 FFF+ref+ICC+LUT+preset）
│   ├─ extract_tif_icc（参考 TIF 的嵌入 ICC）
│   ├─ run_pipeline_with_output_icc（T1 专用，支持 ref-ICC 作为 output）
│   ├─ desaturate_bw_local（BW 辅助）
│   └─ run_tests（T1-T4 调度）
├─ 诊断输出
│   ├─ dump_errmap / colormap
│   ├─ print_summary / print_detail / print_json / print_meta_check
│   ├─ print_summary_with_diff（baseline 对比）
│   └─ load_baseline / extract_* / fmt_diff
├─ Manifest
│   ├─ load_manifest / resolve_path
│   └─ process_case / find_file_pairs
└─ main
    └─ CLI 解析 + dispatch
```

### 核心 Library API（src/color/transform.rs 新增）

```rust
pub enum IccIntent {
    Perceptual, RelativeColorimetric, AbsoluteColorimetric, Saturation,
}

pub struct IccSettings {
    pub intent: IccIntent,
    pub black_point_compensation: bool,  // 默认 false，对齐 lcms2 默认
}

// 任意 ICC → ICC 的变换，支持字节相等短路。
pub fn apply_icc_transform_profiles(
    img: &image::DynamicImage,
    input_icc: &[u8],
    output_icc: &[u8],
    settings: IccSettings,
) -> Result<image::DynamicImage, String>;

// 参数化版本，可控 IccSettings。
pub fn apply_icc_transform_ex(
    img: &image::DynamicImage,
    input_icc: &[u8],
    target: TargetColorSpace,
    settings: IccSettings,
) -> Result<image::DynamicImage, String>;
```

```rust
// src/color/processing.rs 新增
pub fn apply_color_pipeline_ex(
    img, adjust, curve_points, film_lut, icc_data, target_color_space,
    icc_settings: IccSettings,
) -> image::DynamicImage;
```

现有 `apply_icc_transform` / `apply_color_pipeline` 保持原签名不变，内部调 `_ex` 版本用 `IccSettings::default()`。向后兼容所有 viewer/split/examples 调用。

---

## 已知约束

1. **`image::DynamicImage::open().to_rgb16()`** 在读 CMYK 或 Gray TIF 时会转换到 RGB，损失原始精度。非 RGB 参考目前只能在 RGB 退化空间里做模糊对比。
2. **LUT 提取依赖缩略图 IFD**，若 FFF 没有或尺寸不匹配会回退硬编码。
3. **外部 XML 的 `Exposure` 键** 未解析（是扫描硬件曝光参数，与图像色彩处理无关）。
4. **`parse_image_correction`** 不解析 XML 独有的 `AdaptiveLight` / `Aperture` / `ISO` / `Descreen` / `AutoFocus` 等扫描硬件字段。
5. **`--baseline` JSON 解析** 是简易手写，只提取 (case, id, mae_16)。不做完整 JSON 校验。
