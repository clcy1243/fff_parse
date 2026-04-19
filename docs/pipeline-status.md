# 色彩管线还原方案与进度

> 最后更新：2026-04-18  
> 当前工具版本：tif_compare v3

## 目标

逆向 FlexColor 的色彩处理管线，使本应用的渲染输出与 FlexColor 导出的 TIFF **逐像素对齐**。

验收等级：
- **STRICT**（当前最高档，对应"肉眼看不出区别"）— mae_16 ≤ 100 AND p99_16 ≤ 500
- **PASS** — mae_16 ≤ 400 AND p99_16 ≤ 2000
- **WARN** — mae_16 ≤ 1280 AND p99_16 ≤ 5000
- **FAIL** — 超出上述

---

## 管线架构

FFF 文件中存储的是扫描仪原始 RGB 数据（16-bit），需经过以下管线还原为可视图像：

```
原始 RGB (16-bit)
  │
  ├─ 1. 胶片处理 (apply_film_processing)
  │     根据 film_type 和 highlight 参数反转负片 / BW 去饱和
  │
  ├─ 2. 扫描仪色阶 (apply_scanner_levels)
  │     per-channel 色阶归一化 → 胶片曲线 LUT → gamma 校正
  │
  ├─ 3. ICC 色彩空间变换 (apply_icc_transform_profiles)
  │     扫描仪色彩空间 → 目标 ICC 空间
  │     ★ 目标 profile = 参考 TIF 嵌入的 ICC（若有），否则回退 sRGB
  │     ★ 输入==输出 profile 时直接短路（避免 Lab PCS 量化）
  │
  ├─ 4. BW 去饱和 (仅 film_type=2)
  │     ICC 变换会让灰度数据变非灰度，这一步再次去色
  │
  ├─ 5. 用户渐变曲线 (apply_gradation_curves)
  │     RGB + R + G + B + C + M + Y 七通道曲线
  │
  └─ 6. 显示调整 (apply_display_adjust)
        曝光 / 对比度 / 亮度 / 饱和度 / DotColor / color_corr 矩阵
```

### 关键架构洞察（2026-04-18 发现）

**FlexColor 导出 TIF 时不做色彩空间变换，只把扫描仪 ICC 嵌入为 TIFF tag**。

这意味着参考 TIF 的**像素值**并非 sRGB，而是对应各预设的原生色彩空间：

| 参考 TIF 类别 | 嵌入 ICC | 像素所在空间 |
|---|---|---|
| `test1.tif` / `test1_all_config.tif`（主图导出） | sRGB IEC61966-2.1 | sRGB |
| `test1_raw_rgb_*.tif` | Flextight X5 & 949 | 扫描仪 RGB |
| `test1_raw_negative_rgb_*.tif` | Flextight Input | 扫描仪 RGB |
| `test1_raw_cmyk_*.tif` | Hasselblad 330Skel 30K75 | CMYK |
| `test1_raw_bw_*.tif` / `test1_all_config_bw.tif` | Hasselblad Gray | Gray |

**推论**：为做严谨的像素级对比，ICC 变换的 **output profile 必须等于参考 TIF 的嵌入 ICC**。硬编码 sRGB 作为目标会造成双重变换误差。

### 胶片曲线来源

| 来源 | 方法 | 适用场景 |
|------|------|----------|
| **提取 LUT** | 从 FFF 内嵌的 8-bit 缩略图和 16-bit 预览逆向 | 负片且测试的 setting == FFF current_index |
| **硬编码 LUT** | 代码中内置的 Film Auto 曲线 | 其他所有情况（正片、非-current 的负片 setting） |

提取 LUT 仅当 `resolved.embedded_idx == edit_history.current_index` 才启用（因为缩略图只对应 current 状态）。外部 XML 预设永远使用硬编码。

---

## 测试工具 (tif_compare v3)

完整架构文档见 [`docs/test-comparison.md`](test-comparison.md)。

### 核心设计

- **三种预设来源**：FFF 内嵌 current / FFF 内嵌 by-name / 外部 XML 文件
- **Manifest 驱动**：`examples/test_cases.toml` 声明 29 个测试用例（3 e2e + 13 embedded + 13 external）
- **全 16-bit 精度指标**：MAE16 / P50 / P95 / P99 / P999 / Max / 每通道 Signed ME
- **分区间 MAE**：shadow (<20%) / mid / highlight (>80%)
- **ΔE2000** 感知色差
- **ref-ICC 自动对齐**：从参考 TIF 抽取嵌入 ICC 作为 ICC 变换的 output profile

### 测试矩阵 (T1–T4)

| ID | 变体 | 作用 |
|----|------|------|
| T1 | 完整管线（ref-ICC 或 sRGB 回退） | **主验收指标** |
| T2 | 消融：关 ICC | 隔离 ICC 影响 |
| T3 | 消融：强制硬编码 LUT | 对比 LUT 来源 |
| T4 | 消融：完全关胶片曲线 | 验证曲线必要性 |

---

## 当前测试结果

> 测试日期：2026-04-18 | manifest v1 (29 cases)

### 等级分布

| 变体 | STRICT | PASS | WARN | FAIL |
|------|--------|------|------|------|
| T1 默认 | 2 | 0 | 1 | 26 |
| T1 `--use-ref-lut`（仅负片）| 2 | 0 | 5 | 22 |
| T1 `--use-ref-lut`（全类型）| 0 | **4** | 4 | 21 |

说明：
- 默认版：rgb_standard 达 STRICT (MAE 92, ΔE 0.13)
- 扩展到全类型 ref-LUT 后：更多 case 进入 "above-FAIL" 档（8 个 vs 7 个），但 STRICT 降为 PASS（LUT 提取固有量化噪声）
- 不同需求选不同路径：追 STRICT → 不加 flag；追 WARN/PASS 面积 → `--use-ref-lut`

### 🟢 STRICT 等级的 case（视觉完全不可区分）

| Case | MAE16 | P99_16 | ΔE2000 mean | ΔE2000 P95 |
|---|---|---|---|---|
| `emb_rgb_standard` | **92** | 426 | **0.13** | - |
| `ext_rgb_standard` | **92** | 426 | **0.13** | - |

ΔE 0.13 远低于人眼可察觉阈值 (JND ≈ 2.3) → 100% 像素视觉等同。

### WARN 的 case

| Case | MAE16 | P99_16 | ΔE2000 mean |
|---|---|---|---|
| `ext_rgb_saturated` | 865 | - | - |

### 关键发现

1. **ref-ICC 对齐是 STRICT 级对比的前提**  
   未做 ref-ICC 匹配前所有 case 都 FAIL。修正后 rgb_standard 从 1912 → 376 MAE。

2. **输入==输出 ICC 短路必要**  
   即便输入/输出是同一个 profile（字节相同），lcms2 往返 Lab PCS 仍会引入数百 MAE 的量化误差。必须短路跳过。

3. **USM 是 PASS → STRICT 的关键**（2026-04-18 校准完成）  
   Luma-based USM + σ=radius/20 + gain=amount/67：
   - rgb_standard: MAE 376 → **92** (STRICT)
   - 两个 `rgb_standard` case 达到 ΔE2000=0.13（视觉等同）
   - FlexColor 的 `radius=10` 参数对应 **σ=0.5**（不是 σ=10），这是最大的反直觉发现
   - 校准方法：提取 (ref − our_scanner_output) 作为 delta，与 `our_Y − blur_σ(our_Y)` 做最小二乘拟合，rgb_standard R²=0.92

4. **"dark" 预设暴露 scanner_levels 偏差**（USM 后仍未解决）  
   rgb_standard STRICT（92），但 rgb_dark 仍 ~5800。USM 标定显示 dark 预设的误差**不是** luma-based（channel 非一致性 2217 u16，而 rgb_standard 是 1.48 u16）。说明 `apply_scanner_levels` 在 shadow 裁切上与 FlexColor 存在差异。

5. **非 RGB 参考（CMYK/Gray）目前无法精准对比**  
   占 8 个 case。需要实现 Flextight→Hasselblad-CMYK / Flextight→Hasselblad-Gray 真实转换。

6. **负片类预设可能还有胶片曲线 LUT 问题**  
   USM 标定对 neg_rgb_standard 拟合残差极大，且 amount=-60 为软化而非锐化。问题可能在 film curve 提取精度而非 USM。

---

## 完成的工作

### 2026-04-18（本轮）

| 改动 | 文件 | 效果 |
|------|------|------|
| **tif_compare 全面重写（v3）** | `examples/tif_compare.rs` | 三种预设源 + manifest + 全 16-bit 指标 + ΔE2000 + 热图 + worst-pixel + baseline diff |
| **引入 `IccIntent` / `IccSettings`** | `src/color/transform.rs` | 支持 4 种 rendering intent + BPC 开关 |
| **新增 `apply_icc_transform_profiles`** | `src/color/transform.rs` | 任意输入/输出 ICC profile 变换，字节相等短路 |
| **新增 `apply_color_pipeline_ex`** | `src/color/processing.rs` | 可显式指定 IccSettings 的管线入口 |
| **导出 `parse_settings_xml`** | `src/flexcolor/mod.rs` | 加载外部 FlexColor 预设 XML |
| **外部 XML 默认字段补全** | `examples/tif_compare.rs::apply_xml_preset_defaults` | `FilmCurve=4` (Film Auto) + `ColorModel` 按文件名推断。不修改 XML 源文件。|
| **ICC 按名加载** | `examples/tif_compare.rs::load_context` | 优先匹配 `corr.input_profile_name` 而非硬编码 X5 |
| **参考 TIF ICC 抽取 + 用作 output profile** | `examples/tif_compare.rs::extract_tif_icc` | 消除 Flextight→sRGB 的无关变换误差 |
| **tif_compare 增加 `--trace x,y`** | `examples/tif_compare.rs::trace_pixel` | 单像素各阶段追踪，诊断精度问题 |
| **新增 `apply_usm` 亮度 USM** | `src/color/usm.rs` | BT.601 luma 基础上的可分离高斯 USM；opt-in via `FFF_USM_ENABLE=1` |
| **USM 参数 ManualAdjust 接入** | `examples/tif_compare.rs::build_manual_adjust` | 从 ImageCorrection 传递 amount/radius/dark_limit/noise_limit/col_factor |
| **测试 manifest** | `examples/test_cases.toml` | 29 个用例（e2e × 3 + emb × 13 + ext × 13）|
| **dev-dependencies 加 toml** | `Cargo.toml` | manifest 解析 |

### 之前的里程碑

| 日期 | 提交 | 内容 |
|------|------|------|
| 04-17 | `8112ddf` | 添加色彩管线还原进度文档 |
| 04-17 | `7bd48d0` | C-41 启用提取 LUT（弃用硬编码） |
| 04-16 | `86a8d73` | tif_compare 首次重写（2164→597 行） |
| 04-16 | `06c6a2a` | 状态泄漏修复 + 管线顺序修正 + 颜色校正矩阵修复 |
| 04-14 | `79a5afe` | 彩色负片管线大修（某些文件 MAE 62→2）|
| 04-14 | `b20c9c6` | BW 二次灰度化 + DotColor 修复 |

---

## 待解决问题

### ✅ 已完成 — USM 精准校准（2026-04-18）

通过 `tif_compare --calibrate-usm` 做最小二乘 σ/k 拟合：

| Case | σ | k (gain) | R² | divisor |
|---|---|---|---|---|
| rgb_standard | 0.5 | 3.72 | 0.92 | 67.1 |
| rgb_saturated | 0.5 | 3.52 | — | 71.0 |
| cmyk_standard | 0.5 | 4.57 | — | 54.7 |

结论：
- σ 映射 `sigma = radius / 20`（FlexColor radius=10 对应 σ=0.5）
- 增益 `k = amount / 67`
- 基于 BT.601 luma，对三通道加同一 delta

实现：`src/color/usm.rs::apply_usm`，默认启用（无环境变量时用标定值）。

### P1 — 让剩余 26 个 FAIL 进 WARN 或更好

本轮（2026-04-18 下半）深挖了四个 FAIL 类别，部分得出结论，部分需后续：

#### ✅ 已诊断（根因清晰，未改代码以避免回归）

- **[dark 预设，4 case]** Contrast 公式过于激进
  - 当前公式 `scale = 1 + c*2`（c=0.2 → 1.4×），对 v=0.07 的 B 通道计算出 `(0.07-0.5)*1.4+0.5 = -0.10 → clamp 到 0`，丢失信息
  - 扫描 `FFF_CONTRAST_MULT` 显示 **每个预设最优值不同**（rgb_dark: 0.2 最好；cmyk_dark: 2.0 最好；rgb_saturated: 1.0 最好）→ 说明公式本身不对，单参数调不了
  - 保留 `FFF_CONTRAST_MULT` 环境变量可覆盖默认 2.0，方便后续研究
  - 下一步：研究 FlexColor 实际 contrast 公式（可能是 S-curve 或与 lightness/其他阶段耦合）

- **[BW Gray 目标，3 case]** ICC 直转不匹配 FlexColor 行为
  - 实现了 `apply_icc_rgb_to_gray`（Flextight→HasselbladGray via lcms2）
  - 实验发现：直转比 sRGB 回退 + desaturate **更差**（all_config_bw: 874→3945 回归）
  - 结论：FlexColor 不做真正的 Gray ICC 变换，只用 ICC tag 元数据
  - 撤销了 Gray 自动路由，保留 `apply_icc_rgb_to_gray` API 供需要真变换时使用
  - 剩余 BW 误差（5708 for bw_neg_standard, 4194 for all_config_bw）源于 `desaturate_bw` 的位置（pre-scanner_levels vs post-scanner_levels）：试过调换，一个 case 更好另一个更差，说明 FlexColor BW 处理逻辑更复杂

- **[负片 + 正片含 display_adjust，10+ case]** 硬编码 LUT 仅适配特定材料，且 display_adjust 公式不够准确（**2026-04-18/19 系列改进**）
  - 原问题：LUT 从 Portra 160 + Flextight X5 反推，对其他场景失真；正片 Contrast/Lightness 公式非单参数可校
  - 解法：`--use-ref-lut` 从参考 TIF 反推 per-preset **综合 LUT**（捕获胶片曲线 + display_adjust + USM）
  - 关键改动：
    1. `extract_film_curve_16` 接受 16-bit thumb（精度提升，无 8-bit 降采噪声）
    2. `extract_film_curve` 扩展到 `film_type=0`（正片路径跳过负片反转）
    3. `extract_lut_from_ref` 输出近恒等检测（避免破坏已 STRICT 的 case）
  - 效果：
    - `neg_rgb_standard`: 4324 FAIL → **653 WARN** → 821 FAIL (v14 精度权衡)
    - `rgb_dark_saturated`: 2372 FAIL → **274 PASS** (ΔE 0.59)
    - `rgb_saturated ext`: 865 WARN → **176 PASS**
    - `cmyk_dark`: 6195 → 1948 (大幅降，接近 WARN)
    - `rgb_dark`: 5772 → 2192 (大幅降)
  - 局限：
    - rgb_standard 从 STRICT (92) 微降至 PASS (115)，LUT 拟合引入 ~20 MAE 噪声
    - 不是生产方案 — 生产 viewer 无参考 TIF 可用；真正要应用需要离线批量标定材料/预设的 LUT 并随软件发布

#### ✅ 已实现（2026-04-18 第二轮）：手动 CMYK/Gray TIF 加载器

- **手动 TIFF 解析**（`examples/tif_compare.rs::parse_tif_meta`）：绕过 `image` crate 在 16-bit CMYK 上的 naive 8-bit RGB 降级
- **`load_cmyk_as_srgb16`** / **`load_gray_as_srgb16`**：用 lcms2 将原始像素正确转到 sRGB 16-bit 作为对比基准
- **关键 bug 修复**：TIFF SHORT 类型的大端字节序 inline 值正确读取（之前 `val as u16` 对大端取错位导致 photometric=0 而非 2）

效果（对比 v8 naive 路径）：

| Case | v8 (naive) | v10 (lcms2) | 评价 |
|---|---|---|---|
| cmyk_standard | 3401 | **1864** (−1537) | 接近 WARN 阈值 |
| cmyk_saturated | 4227 | **1501** (−2726) | 接近 WARN 阈值 |
| cmyk_dark | 3030 | 6195 (+3165) | 暴露 contrast 公式真实问题 |
| cmyk_dark_saturated | 3138 | 5365 (+2227) | 同上 |
| neg_cmyk_* | 5400 | ~6000 (+500~600) | 暴露负片 LUT 问题 |

**更重要的改进：测量本身更诚实**。之前 naive 路径在某些 case 上碰巧数字好看，掩盖了管线真实误差。现在等级虽然相同（26 FAIL），但每个 case 的 MAE 反映了真实的 pipeline-vs-FlexColor 差距。

- **`apply_icc_rgb_to_gray`**（library 层）：保留 RGB→Gray ICC 变换 API 供未来使用

### P1 — 让更多 case 进 PASS

- [ ] **"dark" 预设 scanner_levels 差异**（impact：~4 case，rgb_dark / rgb_dark_saturated × emb/ext）
  - 现状：rgb_standard MAE 376，rgb_dark MAE 5787（15×）
  - 疑点：shadow 阈值极低时的处理路径。
  - 行动：对比两个预设的 `ManualAdjust` 参数差，看 shadow[] 数组与 DotColor 关系。

- [ ] **外部 XML `Saturated` 变体与 emb 版本差异**（impact：~2 case）
  - 现状：`emb_rgb_saturated=4002` vs `ext_rgb_saturated=893`（ext 反而更好！）
  - 疑点：FFF 内嵌 history 的 "saturated" 条目可能混入了用户后续其他编辑状态，不是纯预设应用的结果
  - 行动：对比 emb[10] 和 XML 内容，找出额外字段。

### P2 — 支持非 RGB 输出目标

- [ ] **CMYK 目标转换**（impact：6 case，所有 cmyk_* 变体）
  - 现状：参考 TIF 是 Hasselblad CMYK 空间，我们对比时被 `image::open().to_rgb16()` 转回 RGB，精度不对等
  - 方案：`apply_icc_transform_profiles` 支持 RGB→CMYK transform，产出 4 通道数据；tif_compare 读参考 TIF 的原始 CMYK 数据做 CMYK 空间对比
  - 难点：`image` crate 对 CMYK 支持有限

- [ ] **Gray 目标转换**（impact：2 case，bw_neg / all_config_bw）
  - 类似 CMYK，目标是 `Hasselblad Gray.icc`
  - 更简单，单通道

### P3 — 持续测量与基础设施

- [ ] **Worst-pixel tracking 覆盖 T2/T3/T4**（现在只有 T1）
- [ ] **JSON 输出加入每通道统计**（现在只有 "all" 聚合）
- [ ] **Baseline diff 模式对接 CI**（`--baseline prev.json` 已实现）
- [ ] **色卡（IT8 / ColorChecker）专项测试**（可用 FlexColor 扫描色卡，评估绝对色彩准确度）

---

## 文件索引

| 文件 | 说明 |
|------|------|
| `examples/tif_compare.rs` | 逐像素对比工具（v3） |
| `examples/test_cases.toml` | 测试用例 manifest |
| `src/color/transform.rs` | ICC 色彩空间变换（含 IccIntent/IccSettings） |
| `src/color/processing.rs` | 核心管线 + 胶片反转 + 曲线提取 |
| `src/color/adjust.rs` | ManualAdjust + scanner_levels + display_adjust |
| `src/flexcolor/parser.rs` | plist XML 解析（含外部 XML 入口） |
| `profiles/` | 内置 ICC 配置文件（Flextight X5 / Input / Hasselblad CMYK / Gray） |
| `settings/` | FlexColor 原生预设 XML（只读，代码层做默认字段补全） |
| `docs/pipeline-status.md` | 本文件 |
| `docs/test-comparison.md` | 测试工具架构详细文档 |

---

## v5 阶段：T22-follow compile pattern 验证未通过（2026-04-20）

- T22-follow agent 反编译 `FUN_702d57b0` 完整 18-cell compile pattern：`cols = [{1,2,3},{0,2,4},{0,1,5}]`。
- 启用 `should_apply = apply_cc && is_customized` + 新 compile：
  - e2e_all_config T6 MAE **3798 → 8780**（大幅回归）
  - 多数其他 cases 小幅改善（~-100..200）
  - WARN 计数 5 → 7，FAIL 计数 134 → 132
- **净值为负**：e2e_all_config 单一 case 的 +4982 MAE 超过多 case 合计改善。
- **处置**：`should_apply` 强制 false，compile pattern 保留待 round-trip 验证。
- **下一步**：需 round-trip 实验（手动 XML + FlexColor 生成 ref）lock down apply 公式的 scale/sign/chromas。

### v5 T6 基线快照（保持 §v4 水平）

| case | MAE16 | status |
|------|-------|--------|
| emb_rgb_standard | 92 | STRICT ✓ |
| ext_rgb_standard | 92 | STRICT ✓ |
| e2e_all_config_bw | 990 | WARN |
| ext_rgb_saturated | 865 | WARN |
| e2e_all_config | 3798 | FAIL |
| emb_bw_neg_standard | 3844 | FAIL |

TOTAL: 6 STRICT / 0 PASS / 5 WARN / 134 FAIL (145 tests over 29 cases)

