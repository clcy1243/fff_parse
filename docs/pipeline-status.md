# 色彩管线还原方案与进度

> 最后更新：2026-04-17  
> 对应提交：`7bd48d0` (C-41 启用提取LUT)

## 目标

逆向 FlexColor 的色彩处理管线，使本应用的渲染输出与 FlexColor 导出的 TIFF 逐像素对齐。

验收标准：所有测试文件的完整管线（T1）达到 **PASS**（MAE ≤ 5, P99 ≤ 20）。

---

## 管线架构

FFF 文件中存储的是扫描仪原始 RGB 数据（16-bit），需经过以下管线还原为可视图像：

```
原始 RGB
  │
  ├─ 1. 负片反转 (apply_film_processing)
  │     根据 film_type 和 highlight 参数反转负片
  │
  ├─ 2. 扫描仪色阶 (apply_scanner_levels)
  │     levels 归一化 → 胶片曲线 LUT → gamma 校正
  │
  ├─ 3. ICC 色彩转换 (apply_icc_transform)
  │     扫描仪色彩空间 → sRGB（使用 Imacon ICC 配置文件）
  │
  ├─ 4. BW 去饱和 (仅 film_type=2)
  │     彩色负片基底 → 灰度
  │
  ├─ 5. 用户渐变曲线 (apply_gradation_curves)
  │     RGB 分通道曲线调整
  │
  └─ 6. 显示调整 (apply_display_adjust)
        亮度/对比度/饱和度/DotColor/color_corr
```

### 胶片曲线来源

管线第 2 步中的胶片曲线 LUT 有两种来源：

| 来源 | 方法 | 适用场景 |
|------|------|----------|
| **提取 LUT** | 从 FFF 内嵌的 8-bit 缩略图和 16-bit 预览对比逆推 | ✅ 当前所有负片类型 |
| **硬编码 LUT** | 代码中内置的固定曲线 | ❌ 已弃用，MAE 远高于提取方案 |

---

## 测试工具 (tif_compare)

位于 `examples/tif_compare.rs`，通过将管线输出与 FlexColor 导出的参考 TIFF 逐像素对比来量化还原精度。

### 指标体系

| 指标 | 含义 | PASS | WARN | FAIL |
|------|------|------|------|------|
| **MAE(8-bit)** | 平均绝对误差（0-255 量级） | ≤ 5 | ≤ 10 | > 10 |
| **P95** | 95% 像素的误差上界 | — | — | — |
| **P99** | 99% 像素的误差上界 | ≤ 20 | ≤ 40 | > 40 |
| **Max** | 最大单像素误差 | — | — | — |
| **ΔE76** | CIE L\*a\*b\* 色差（感知距离） | — | — | — |
| **PSNR** | 峰值信噪比 | — | — | — |

Grade 判定：同时满足 MAE 和 P99 阈值才为该等级。

### 测试矩阵 (T1–T8)

| ID | 测试内容 | 目的 |
|----|----------|------|
| T1 | 完整管线（提取LUT + ICC） | **主要验收指标** |
| T2 | 提取LUT，跳过 ICC | 隔离 ICC 影响 |
| T3 | 硬编码LUT + ICC | 对比 LUT 来源 |
| T4 | 无胶片曲线 | 验证曲线必要性 |
| T5 | 阶段截断：scanner levels | 管线逐步追踪起点 |
| T6 | 阶段：+ ICC | ICC 对误差的贡献 |
| T7 | 阶段：+ display adjust | 显示调整的影响 |
| T8 | 阶段：+ curves（完整） | 用户曲线的影响 |

### 用法

```bash
# 单文件详细报告
cargo run --release --example tif_compare -- <file.fff> <file.tif> -v

# 目录批量测试
cargo run --release --example tif_compare -- --dir <path>

# JSON 输出（可供 CI 解析）
cargo run --release --example tif_compare -- <file.fff> <file.tif> --json

# 指定使用编辑历史中的第 N 个设置
cargo run --release --example tif_compare -- <file.fff> <file.tif> --setting 3
```

---

## 当前测试结果

> 测试日期：2026-04-17 | 提交：`7bd48d0`

### 总览

| 文件 | 类型 | 最佳 MAE | T1(完整管线) MAE | T1 Grade |
|------|------|----------|------------------|----------|
| test1.fff | C-41 彩负 | 6.05 (T5) | 16.74 | FAIL |
| test1_all_config.fff | C-41 彩负 | 7.84 (T2) | 9.26 | FAIL |
| test1_all_config_bw.fff | BW 黑白 | 7.96 (T5) | 14.23 | FAIL |

### 关键发现

1. **ICC 变换是当前主要误差来源**
   - T5（scanner levels 后截断）MAE 6-9，加 ICC 后（T6）跳到 10-14
   - R/G 通道被 ICC 显著恶化，B 通道反而改善
   - 所有文件中 T2（无 ICC）均优于 T1（有 ICC）

2. **提取 LUT 远优于硬编码 LUT**
   - test1_all_config: 提取 MAE=7.84 vs 硬编码 MAE=62.20
   - test1: 两者接近（8.05 vs 7.51），但 all_config 差距巨大

3. **R/G 通道误差远大于 B 通道**
   - T1 典型分布：R≈11.5, G≈12.8, B≈3.4
   - 指向胶片曲线在 R/G 通道的提取精度不足，或 ICC 配置文件不匹配

4. **Display adjust（饱和度）在 ICC 后部分补偿了误差**
   - T6→T7: MAE 从 9.96 降到 9.26（test1_all_config 为例）

---

## 已完成的工作

| 日期 | 提交 | 内容 |
|------|------|------|
| 04-17 | `7bd48d0` | C-41 启用提取 LUT（弃用硬编码） |
| 04-17 | `86a8d73` | tif_compare 重写：2164→597行，新指标体系 |
| 04-16 | `06c6a2a` | 状态泄漏修复 + 管线顺序修正 + 颜色校正矩阵修复 |
| 04-15 | `79a5afe` | 彩色负片管线重大修复（MAE 62→2，特定文件） |
| 04-14 | `b20c9c6` | BW 二次灰度化 + DotColor 修复 |

---

## 待解决问题

### 高优先级

- [ ] **ICC 配置文件优化** — 当前 ICC 变换使 R/G 通道误差加倍，需要：
  - 确认使用的 ICC 配置文件是否匹配扫描仪型号
  - 考虑 ICC 渲染意图（Perceptual vs Relative Colorimetric）
  - 可能需要调整 ICC 应用时的白点适配

- [ ] **R/G 通道精度** — 胶片曲线在 R/G 通道的提取精度不足，需要：
  - 分析缩略图提取算法在 R/G 通道的行为差异
  - 确认 FlexColor 是否对不同通道使用不同的曲线参数

### 中优先级

- [ ] **6×6 CMY 颜色校正矩阵** — 当前仅支持部分矩阵参数
- [ ] **USM 锐化** — FlexColor 导出 TIFF 时应用了 USM，当前未实现
- [ ] **16-bit 参考 TIFF** — 当前参考文件为 8-bit，限制了比较精度

### 低优先级

- [ ] **CI 集成** — tif_compare 已支持退出码，可接入 CI 管线
- [ ] **更多测试文件** — E-6 正片、不同扫描仪型号的覆盖

---

## 文件索引

| 文件 | 说明 |
|------|------|
| `examples/tif_compare.rs` | 管线验证工具（597行） |
| `src/color/processing.rs` | 核心管线：反转 + 曲线提取 + LUT |
| `src/color/adjust.rs` | ManualAdjust 结构 + scanner levels + display adjust |
| `src/color/transform.rs` | ICC 色彩空间变换 |
| `src/viewer/panels.rs` | 查看器 UI + 管线调用 |
| `profiles/` | 内置 ICC 配置文件 |
