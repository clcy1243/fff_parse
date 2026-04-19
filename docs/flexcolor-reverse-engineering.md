# FlexColor 互操作研究笔记

> 目的：理解 FlexColor v4.8.9.1 的色彩处理管线，用于与 FFF 文件互操作研究。

## 📌 阅读索引（最新成果优先）

**复刻参考权威**：
- **§16.5** — 完整 pipeline 数据流（14-bit 域，per-channel）
- **§18.7** (勘误版) — CGammaNegCurve 完整公式 `pow(1-v², 1/γ) × 16383`
- **§16.4** (勘误版) — CAggregateCurve mode 0/1/2 精确语义
- **§25** — CImageCorrection 全字段布局（offsets + types）
- **§28** — CHighShadowCurve 三点 histogram 算法
- **§32** — 所有字段默认值表

**新挖的高价值细节**：
- **§19** — 11 个 CGammaNeg 常量（0.099/10.1 合法区间；0.2 默认 γ；17700/16383 scale）
- **§18.8** (勘误版) — CNegativeCurve 默认 x/y 参数（每通道）
- **§23** — FilmType (this+0x51c) 是 CGammaNeg 主开关，默认 0（关闭）
- **§24/26** — InputProfile/RGBProfile/CMYKProfile/GrayProfile 是 CFileSpec 子对象
- **§27** — XML reader/writer vtable slot 语义
- **§29** — RTTI 类目录（发现 CImageConverter, CColorCorrection, CColorTempConversion 等）
- **§31** — CImageCorrection ctor 完整展开
- **§32.5** — Profile 默认标识符（`.dp:`/`.dfR:`/`.dfC:`/`.dfG:`）

**工具**（`tools/ghidra_query/`）：
- `./run.sh decompile/vtable/disasm/read-const/find-str-xrefs`
- `./run.sh dump-xml-registry` — 全部 825 个 XML key 注册
- `./run.sh list-rtti-classes [prefix]` — 从 RTTI 扫类名（新）

---

> 合法性基础：DMCA 1201(f) / EU 计算机程序指令 Article 6 — 允许为互操作目的的逆向分析。
>
> **范围**：仅分析算法/数据结构，不复制 FlexColor 的源码或二进制。本文内容来自：
> 1. 开放 XML 配置文件的 plist 结构分析
> 2. 可执行文件的公开字符串（C++ RTTI 类型名）
> 3. 通过 pixel-pair 黑盒对比观察输入输出关系

---

## 一、包结构总览

`FlexColor v4.8.9.1/`
- `FlexColor.exe` (203 KB) — GUI 前端
- `DLLS/FlexColor.dll` (10.6 MB) — 主要图像处理逻辑
- `Colormaps/LUTTable*.xml` (1.6 MB × 7) — **相机色度 LUT（Cb/Cr 空间）**
- `Profiles/*.icc` — ICC 配置文件（Flextight 扫描仪 + Hasselblad 输出空间）
- `Settings/Flextight/` — 扫描仪预设（含 Film Specific 子目录按胶片型号）
- `Settings/FlexFrame/` — 相机后背预设
- `Misc/FlexColor read me.rtf` — 官方文档（可读）

---

## 二、Colormaps LUT 结构（重大发现）

**位置**：`Colormaps/LUTTable{22MPC,31MP,31MPC,39MP,39MPC,Ixpress,leica}.xml` — Hasselblad 相机后背的色彩查找表。

**格式**：Apple plist，关键字段：

| 键 | 类型 | 含义 |
|---|---|---|
| `tf` / `tt` | integer | 两个参考色温（典型 5130K / 3238K）|
| `vf` / `vt` | array[3] real | 对应色温的 RGB 白点向量 |
| `mf` / `mfd` / `mt` | array[9] real | 3×3 矩阵（色彩校正） |
| `DivFactor` | integer (=32) | LUT 索引缩放因子 |
| `CbE` / `CbS` | integer | Cb 维度范围（如 -20..84，共 105 格）|
| `CrE` / `CrS` | integer | Cr 维度范围（如 -32..56，共 89 格）|
| `LUTTableFlashStd` / `LUTTableFlashRepro` / `LUTTableTSStd` / `LUTTableTSRepro` | array of 18690 real | **2D Cb/Cr → (dCb, dCr) LUT** |

**18690 = 105 × 89 × 2** 验证了结构：Cb 格数 × Cr 格数 × 2 输出（ΔCb, ΔCr）。

### 关键推论

**FlexColor 的色彩校正运行在 YCbCr 色度空间**：
1. RGB → Y, Cb, Cr（亮度 + 色度）
2. (Cb/DivFactor, Cr/DivFactor) 查 2D LUT → 得到色度修正 ΔCb, ΔCr
3. 应用修正：Cb' = Cb + ΔCb, Cr' = Cr + ΔCr
4. (Y, Cb', Cr') → RGB

LUT 表的 4 个变体（Flash/TS × Std/Repro）对应不同光源/用途。

### 对我们的启示

我们的 `FILM_CURVE_LUT_R/G/B`（RGB 空间 256-entry LUT）**空间根本不对**。FlexColor 在 **chrominance 空间** 做色彩修正，luma 基本不动。这解释了为什么：
- 我们的 hardcoded LUT 只对特定材料（Portra 160 + X5）拟合得上 — 那是针对单一 (Y, Cb, Cr) 分布的近似
- 换其他材料（不同 Cb/Cr 分布）就完全错位
- `--use-ref-lut` 反推能提升精度，但本质还是 RGB-空间近似

---

## 三、C++ 类结构（来自 DLL 的 RTTI 字符串）

FlexColor.dll 暴露了以下类名（mangled `.?AV...`），揭示内部架构：

### 色彩处理核心

| 类 | 推测职责 |
|---|---|
| **`CColorWorld`** | 色彩空间上下文（色温、白点）|
| **`CCachedColorWorld`** | 色彩空间缓存 |
| **`CICMColorManager`** | ICC 色彩管理 |
| **`CColorManager`** | 色彩管理调度 |
| **`CColorCorrection`** | 色彩校正主类 |
| **`CColorTempConversion`** | 色温转换 |
| **`CDeviceColorProfileCache`** | 设备 profile 缓存 |

### 曲线类（Curve 对象，非标量公式！）

| 类 | 含义 |
|---|---|
| **`CCurve`** | 曲线基类 |
| **`CCurvePoint`** | 曲线控制点 |
| **`CAggregateCurve`** | 复合曲线（组合多条）|
| **`CBufferedCurve`** | 缓存曲线 |
| **`CFilmCurve`** | **胶片响应曲线**（film_type/curve 参数产生）|
| **`CGammaCurve`** | γ 曲线（正片用）|
| **`CGammaNegCurve`** | **负片专用 γ 曲线**（密度对数反转）|
| **`CContrastCurve`** | **对比度曲线**（不是我们当前的 scale 公式！）|
| **`CContrastBrightnessCurve`** | 亮度-对比度联合曲线 |
| **`CHighShadowCurve`** | 高光/阴影联动曲线 |

### 胶片专用

| 类 | 含义 |
|---|---|
| **`CFilmDetector`** | 胶片类型自动识别 |
| **`CFilmRepairDust/Scratch/Hole/Damage`** | 划痕/灰尘/洞修复 |

### 数据结构

| 类 | 含义 |
|---|---|
| `CImageCorrection` | 与 FFF 里嵌入的 `ImageCorrection` 对应（参数容器）|

### 字符串常量

- `"CONTRAST PARAMETERS"`, `"FilmHighlightComp"`, `"FilmShadowComp"`, `"FilmMidpoint"` — FilmCurve 的 3 个 shape 参数
- `"gamma = (%d/100000)"` — gamma 内部用 5 位定点整数
- `"Curves timeout %ld"` — 曲线计算有超时保护
- `"Gamma %.2e"` — γ 以科学计数法输出

---

## 四、关键架构假设（基于上面证据）

### 假设 1：管线是 **Curve 合成**

所有调整（contrast, lightness, shadow, highlight, gamma, film curve）**都是 `CCurve` 对象**，通过 `CAggregateCurve` 合并成一条总曲线。一次应用 = 一次 LUT 查询。

这解释了：
- 为什么 "contrast=20" 的效果不能用 `scale = 1 + c*2` 线性公式匹配（它是 curve shape 变化，不是均匀拉伸）
- 为什么不同参数组合的最优 `mult` 不同（各 curve 在 aggregate 中**非线性组合**）
- 为什么 `CGammaNegCurve` 单独存在（负片的 γ 作用在密度域，与正片不同）

### 假设 2：色彩校正在 **YCbCr 色度空间**

> ❌ **已证伪（2026-04-19）**：实际 pipeline 在 **14-bit per-channel RGB LUT** 域运行。见 §16.5 完整数据流与 §25 字段布局。下述内容仅为历史记录。

基于 Colormaps 的证据，色彩校正的 LUT 阶段**不在 RGB**。`CColorCorrection` 可能：
1. RGB → YCbCr（标准矩阵）
2. 对 (Cb, Cr) 查 2D LUT 得到 (ΔCb, ΔCr)
3. 色温调整也在 YCbCr 域（`mf/mt` 矩阵插值）
4. 合并后转回 RGB

### 假设 3：胶片反转用**密度域**（对数）

> ❌ **已证伪（2026-04-19）**：真实公式是 `LUT[i] = pow(1 − (i/scale)², 1/γ) × 16383`（见 §18.7），**无 log/exp**。C-41 per-channel 非对称由 CNegativeCurve 的 per-channel 二段二次曲线参数实现（见 §18.8）。下述内容仅为历史推测。

`CGammaNegCurve` 暗示负片处理**不是线性 (highlight - raw)/highlight**，而是：
1. 密度 d = -log(raw / orange_mask)
2. 归一化 d_rel = (d - d_min) / (d_max - d_min)
3. 通过 `CGammaNegCurve` 应用非线性映射
4. 输出 positive = d_rel（在某个转换后）

这解释了为什么 **C-41 负片的 per-channel 处理非对称** — 橙色蒙版的 R/G/B 在 log 域表现完全不同。

---

## 五、后续可做的深度研究

### 低成本
- [ ] 用 Ghidra 打开 `FlexColor.dll` 分析 `CFilmCurve::Apply` 等具体函数（`CCachedColorWorld` 构造器里的设置值尤其重要）
- [ ] 扫描 DLL 的 `.rdata` 段寻找 **Flextight LUT 常量**（scanner 的 film curve 可能也在 Cb/Cr 域，但作为常量嵌入 DLL）
- [ ] 验证 YCbCr 假设：把 `extract_film_curve_16` 从 RGB 域改到 Cb/Cr 域，看能否拟合出更平滑/稀疏的 LUT

### 高价值
- [ ] 读 `FlexColor read me.rtf` — 可能有色彩处理的用户面描述，透露内部公式名
- [ ] 研究 `Plugins/` 和 `PluginsX64/` — Photoshop 插件可能暴露更简单的 API
- [ ] 分析 `HasDeviceLink64.dll` (`HasDeviceLinkMFC64.exe`) — 设备链接（DeviceLink ICC）说不定是 FlexColor 输出色彩变换的真身

### 高成本（不一定回报）
- [ ] 完整逆向 `FlexColor.dll`（10 MB，数周级工作量）
- [ ] 用 Frida 等运行时插桩跟踪关键函数参数/返回值（需要 Windows 环境）

---

## 六、对当前项目的影响

### 立即可改进

1. **`CContrastBrightnessCurve` 证据**：我们的线性 `scale = 1 + c*2` 公式不可能精确匹配。应该改为**构建一条 tone curve**，以 4-5 个控制点表示 contrast 曲线的形状。
2. **`CGammaNegCurve` 证据**：负片的密度域反转与我们的线性 `(hi - raw)/hi` 不同。实现 `positive = ((log(hi/raw)) / (log(hi/shadow)))^gamma` 可能更接近。
3. **色度空间 LUT**：对 `--use-ref-lut` 改用 Cb/Cr 空间反推，可能得到更平滑的 LUT（尤其对负片 G/B 通道不对称的情形）。

### 长期方向

> ❌ **方向已废（2026-04-19）**：此前基于 YCbCr 假设（已证伪）。正确方向见 §16.5（14-bit per-channel LUT pipeline）与 §25（字段布局）。下方 YCbCr 架构图仅历史留存。

把当前的 RGB-空间、标量-组合管线，逐步重构为 **YCbCr + Curve 合成** 架构：
```
RGB
  → (matrix) → YCbCr
  → AggregateCurve(Y) 应用于 Y 通道
  → 2D LUT(Cb, Cr) 应用于色度
  → (matrix) → RGB
```

这更贴近 FlexColor 的实际架构，长期看会让所有 preset 类型的精度一起提升，不再需要 per-preset 标定。

---

## 七、深度逆向 Roadmap（按效果排序）

> 📋 **历史记录（2026-04-19）**：下列 T1-T9 roadmap 及 §九 时间表均已完成或作废，状态列"⏳ 待做"过期。实际状态见 TaskList（T1/T8 因 YCbCr 假设作废而关闭；其余 T2/T3/T4/T5/T7/T9 已完成）。**当前活动任务**见 §30.2（T10 Rust 复刻、T14 pipeline 入口、T16 slider 算法、T18 FilmCurve preset 等）。


> 目标：从 FlexColor.dll 提取 **算法思想和公式**（不复制代码），用于 clean-room 式重写到 Rust。
> 工具：**Ghidra**（NSA 免费，反编译质量好，macOS 原生，支持 C++ RTTI）。
> 法律边界：仅做 interoperability 目的的算法提取；公式可实现，不直接拷贝二进制或源码。

### T9 · Ghidra 环境搭建 · 🔧 所有后续任务的前置
**时间**：1–2 小时（一次性）

**步骤**：
1. 下载 Ghidra release（免费，zip 直接运行）
2. `brew install openjdk` 如果没 Java
3. 新建 project，导入 `FlexColor.dll` + `HasDeviceLink64.dll`
4. 选 "Analyze All" → 等 20–30 分钟（10MB DLL 首次分析）
5. 保存 project（后续打开零等待）

**产出**：可 navigate 的反编译环境、完整函数列表、RTTI 类图

---

### T7 · 读 `Misc/FlexColor read me.rtf` · 💰 最低成本
**时间**：30 分钟

**目标**：扫官方用户文档找算法相关描述。

**方法**：搜 "Contrast"/"Curve"/"Gamma"/"Color Space"/"Adjustment" 等关键词，记录任何提到算法名、公式参考、参数语义的段落。

**产出**：
- 可能性低但有可能：官方文档写出 "我们用 XXX 算法"
- 即便文档没公式，也能澄清 UI slider 的 0–100 数值是什么内部语义

---

### T1 · 验证 YCbCr 色空间假设 · 🎯 影响**全部 29 case**
**时间**：6–10 小时

**目标**：确认 FlexColor 色彩管线是否真在 YCbCr 空间。决定整个项目架构方向。

**方法**：
1. Ghidra 找 `CColorWorld` 构造器 + `CColorCorrection::Apply`
2. 识别 RGB→X 的 3×3 矩阵（9 个 float 常数）
3. 对比标准 BT.601/709 YCbCr 矩阵系数：
   ```
   Y  = 0.299 R + 0.587 G + 0.114 B
   Cb = -0.169R − 0.331G + 0.500B (+ 128)
   Cr = 0.500 R − 0.419G − 0.081B (+ 128)
   ```
4. 如果系数不是 BT.601/709，查是否自定义矩阵（Hasselblad 可能有私有空间）

**产出**：
- 报告："FlexColor 在 XXX 空间工作，矩阵系数是 YYY"
- 若证实 YCbCr：进入架构重构路线
- 若证否：找出真实空间（Lab？专有？），缩小后续搜索

---

### T2 · 提取 Contrast/Lightness/Brightness 曲线公式 · 🎯 影响 **8 个 dark/saturated case**
**时间**：3–5 小时

**目标**：得到 FlexColor 对比度、亮度、明度的**精确公式**。

**关注类**：
- `CContrastCurve` · `Apply` 方法
- `CContrastBrightnessCurve` · `Apply` 方法
- `CHighShadowCurve` · `Apply` 方法

**重点弄清**：
- 输入/输出值域（0..1、-128..127、还是其他）
- pivot 点位置（0.5 中灰？0.18？自适应？）
- 公式形态（线性 scale / S-curve / gamma 混合 / 贝塞尔曲线？）
- 当有多个 slider 同时非零时，它们是**串联**还是**合成到一条曲线**

**产出**：
- 3 个公式的 Rust 实现（独立重写）
- 参数映射表（slider 0–100 → 公式参数）
- **预期**：rgb_dark / cmyk_dark / rgb_saturated 系列 FAIL → WARN 或 PASS

---

### T3 · 提取 `CGammaNegCurve` 密度域负片反转 · 🎯 影响 **7 个负片 case**
**时间**：2–4 小时

**目标**：弄清 C-41 / BW 负片真实反转算法（对比当前线性 `(hi-raw)/hi` 公式）。

**方法**：
1. 找 `CGammaNegCurve` vtable
2. 对比 `CGammaCurve`（正片）vs `CGammaNegCurve`（负片）的 `Apply` 差异
3. 关注点：
   - 有 `log`/`exp` 调用？→ 密度域特征
   - shadow/highlight/gamma 如何组合
   - 橙色蒙版偏移是否独立参数（每通道 offset 常数）

**产出**：
- C-41 反转精确公式
- **预期**：`emb_neg_rgb_standard` 从 653 WARN 下降到 PASS 或 STRICT，**不再依赖** `--use-ref-lut`

---

### T4 · 扫描 `.rdata` 段 dump 嵌入 LUT · 🎯 影响**胶片曲线 4 档选项的精度**
**时间**：2–3 小时

**目标**：从 DLL 数据段提取 FlexColor 的 **Linear / Film Std / Film High / Film Low / Film Auto 五档曲线 LUT**。

**方法**：
1. Ghidra Data Type Manager 定位 `.rdata` 段
2. 写 Ghidra Python 脚本扫描连续 float 数组，候选长度：
   - 65536（1D LUT）
   - 105×89×2 = 18690（2D Cb/Cr LUT，与 Colormaps XML 同结构）
   - 256×3 = 768（简单 RGB LUT）
   - 1024（线性 curve）
3. 值范围过滤：[0, 1]、[-10, 10]、或 [0, 65535]
4. Cross-reference：在 `FilmCurveMenu`、`FilmHighlightComp` 字符串引用站点附近找这些数组

**产出**：
- 5 份 LUT 数据文件（按 FilmCurve 参数值 0–4 分）
- 替换 `src/color/processing.rs::FILM_CURVE_LUT_{R,G,B}` 中的 256-entry 硬编码小表
- **预期**：硬编码路径精度大幅提升，不再只适配 Portra 160

---

### T5 · `CAggregateCurve` pipeline 合成顺序 · 🎯 影响**架构正确性**
**时间**：2–3 小时

**目标**：确定 FlexColor 真实应用各 curve 的顺序和合成方式。

**方法**：
1. 找 `CAggregateCurve::Apply`
2. 看它持有哪些子 curve（成员变量或动态 vector）
3. 追 `CImageCorrection::Apply`，看 contrast/gamma/film curve 如何被传入 aggregate
4. 对比当前 `apply_color_pipeline_ex` 的顺序

**产出**：
- 正确的 pipeline 顺序图
- 可能纠正若干阶段先后
- **预期**：消除 "改对一个，另一个反而更差" 现象

---

### T6 · 分析 `HasDeviceLink64.dll` · 🎲 可能一锤定音
**时间**：2–3 小时

**目标**：验证 FlexColor 是否通过 **DeviceLink ICC profile** 一次性应用所有变换（如是，可直接解析得到 LUT）。

**方法**：
1. Ghidra 开 `HasDeviceLink64.dll`（小文件，分析快）
2. 看导出函数、类名、strings
3. 看是否生成 / 读取 ICC LUT8/LUT16 CLUT tag
4. 追它被 FlexColor.dll 调用的站点

**产出**：
- 最佳情况：实现 DeviceLink ICC 构建 → **一条 3D LUT 搞定整个 pipeline**
- 次佳情况：排除这条路径，方向聚焦 Ghidra decompile

---

### T8 · tif_compare YCbCr 空间 ref-LUT 反推 · 🧪 实验验证，独立可做
**时间**：3–4 小时

**目标**：**不等 Ghidra 结果**，假设 YCbCr 为真，改造 `extract_film_curve_16` 到 YCbCr 空间，看是否大幅改善。

**方法**：
1. 新写 `extract_chroma_lut`：
   - 读 (raw, ref) pair
   - 各自 RGB→YCbCr
   - 对 (Cb, Cr) 建 2D LUT 输出 (ΔCb, ΔCr)
2. 新写 `apply_chroma_lut` 插入到 scanner_levels 后
3. 重跑 manifest

**产出**：
- YCbCr 假设的实验性证据（**不依赖 Ghidra**）
- 若大幅改善：YCbCr 假设基本坐实，全力转向重构
- 若无改善或更差：空间假设错，缩小后续搜索

---

## 八、推荐执行顺序

```
T9 环境搭建
 │
 ├─ T7 读 RTF（30 分钟零成本）
 │
 ├─ T1 YCbCr 验证 ←──┐
 │                   │
 │   T8 YCbCr 实验 ──┘ 可与 T1 并行（T8 不依赖 Ghidra）
 │
 ├─ T2 Contrast 公式
 ├─ T3 Negative Gamma
 ├─ T4 Dump 嵌入 LUT
 │
 ├─ T5 Pipeline 顺序
 └─ T6 DeviceLink DLL
```

**第一批推荐**：T9 + T7 + T1 + T8（~11 小时）— 这组合出结果后，后续方向会清晰很多。

---

## 九、时间总账

| 任务 | 时间 | 状态 |
|---|---|---|
| T9 · Ghidra 环境 | 1–2 小时 | ⏳ 待做 |
| T7 · 读 RTF | 30 分钟 | ⏳ 待做 |
| T1 · YCbCr 验证 | 6–10 小时 | ⏳ 待做 |
| T2 · Contrast 公式 | 3–5 小时 | ⏳ 待做 |
| T3 · Negative Gamma | 2–4 小时 | ⏳ 待做 |
| T4 · LUT dump | 2–3 小时 | ⏳ 待做 |
| T5 · Pipeline 顺序 | 2–3 小时 | ⏳ 待做 |
| T6 · DeviceLink | 2–3 小时 | ⏳ 待做 |
| T8 · YCbCr 实验 | 3–4 小时 | ⏳ 待做 |
| **合计** | **21.5–36.5 小时** | |

---

## 十、能提取 vs. 提不出

### 💯 几乎一定能拿到

- Pipeline 合成顺序（curves 按什么顺序 chain）
- Contrast/Gamma/Lightness 具体公式（通常 10–30 行闭式表达）
- 色彩空间矩阵系数（RGB↔YCbCr 的 9 个数）
- LUT 维度和索引 scheme（1D/2D，轴是什么）
- 参数范围和 scale factor（slider 0–100 到内部 0.0–1.0 的具体映射）
- 嵌入 LUT 数据（如果是常量，直接 dump）

### ⚠️ 较难但不是不可能

- 经验 LUT 的生成逻辑（不直接存在代码里，在胶片标定的研究结果里）
- SIMD 批处理的具体像素级操作（decompile 出来是繁琐的 intrinsic 形式，可读但费时）
- Virtual function dispatch（需要看 vtable 确定是哪个派生类 Apply 被调用）

### 🚫 不需要提取

- UI、对话框逻辑
- Scanner SCSI/FireWire 驱动
- 文件 I/O（我们自己写的 TIFF parser 够用了）
- 帮助文档/本地化
- 相机/后背支持（我们只做扫描仪部分）

---

## 十一、常见坑

1. **MSVC release 构建内联激进**：`Apply` 可能被内联进调用者，不是独立函数。要在调用站点读取。
2. **C++ 异常处理展开**：Ghidra 反编译里会有大量 `__CxxFrameHandler` 相关冗余代码，可忽略。
3. **模板展开**：`CMenuHelper<eFilmCurveType>` 这种模板类会生成多份实例。看 RTTI 能分辨。
4. **浮点常数识别**：反编译器有时把 `3F000000h` 不识别为 float `0.5`，要 right-click → "Convert to float"。
5. **数学库调用**：`powf`, `logf` 等会显示为 `@___imp__powf` 链接，要跟进到 import 表确认是标准 math。
6. **虚函数解析**：`this->apply()` 实际调哪个派生类，需要追到构造器看 vtable 设置。

---

## 十二、T7 实施记录（2026-04-19）

### 资料来源

| 文件 | 类型 | 信息量 |
|---|---|---|
| `Misc/FlexColor read me.rtf` | ChangeLog | 零算法信息（仅版本历史）|
| `Language/English/Cross/messages.xml` | UI string plist | ★★ 菜单名、分类 |
| `DLLS/English/FlexColorLang.dll` | 本地化 DLL | ★★★ 全量 UI strings (UTF-16) |
| `DLLS/FlexColor.dll` | 主 DLL | ★★★ 内部标识符名（未 strip）|

### 🎯 FilmCurve 语义修正（重大）

`messages.xml::FilmCurveMenu` 显示 FlexColor **UI 实际菜单**是 6 项：

| 值 | FlexColor UI 名 | 我们之前认为 |
|---|---|---|
| 0 | **As Shot**（使用扫描时记录的）| `Linear` ❌ |
| 1 | **Standard** | `Film Std` ≈ |
| 2 | **Low Contrast** | `Film High` ❌（顺序反了）|
| 3 | **High Contrast** | `Film Low` ❌（顺序反了）|
| 4 | **Old Standard**（legacy 兼容）| `Film Auto` ❌ |
| 5 | **Linear** | `Unknown` |

**我们数据里 `FilmCurve=4` 实际是 "Old Standard"**（旧版兼容固定曲线），不是我们一直理解的 "Film Auto"（自适应）。这合理地解释了为什么硬编码 256-entry LUT 只能拟合一种材料——它本就是**固定的 legacy curve**。

**待办**：修正 `src/flexcolor/model.rs::film_curve_name`。

### 🎯 Gradation Curves 是 Hermite/三次样条（重大）

> ⚠️ **勘误（2026-04-19）**：实际是 **Cox-de Boor B-spline**（非 Hermite），见 §16.9 的 FUN_702699e0 反编译。`dy` 字段可能是 knot-vector 元素或端点标记，非 Hermite 切线。

`FlexColorLang.dll` 里的字段名：
- `Gradation CurvePoints x`
- `Gradation CurvePoints y`
- `Gradation CurvePoints dy` ← **切线分量！**

我们之前把 gradation 当成 256-entry 线性插值 LUT。实际上 FlexColor 用**带切线的三次 Hermite 样条**（或 Catmull-Rom / Bezier），这是为什么用户画的曲线看起来光滑、而我们的线性插值在某些点有棱角。

**待办**：`src/color/processing.rs::build_curve_lut` 改为 Hermite 插值（使用 x/y/dy）。

### 🎯 内部参数名（直接暴露 pipeline 构造）

从 FlexColor.dll 的 ASCII 标识符里看到：

| 标识符 | 含义 |
|---|---|
| `FilmHighlightComp` | 胶片曲线的**高光补偿**参数 |
| `FilmShadowComp` | 胶片曲线的**阴影补偿**参数 |
| `FilmMidpoint` | 胶片曲线的**中点**位置（不一定 0.5）|
| `FilmCurveMenu` | FilmCurve 选项枚举 |
| `FilmTypeOption` | FilmType 选项 |
| **`StretchNegGamma`** | **负片 gamma 拉伸**——专用函数，证实负片有单独处理路径 |
| `ColorResponseUnit` | 色彩响应单位（tone curve 包装类）|
| `GrayResponseCurve` | 灰度响应曲线（BW 专用）|
| `GradationLines` | 曲线（画出来的线）|
| `GradationSlider` / `GradationSliders` | 滑杆（macro 控制）|
| `AdaptiveLight` | 自适应光照校正（未实现特性）|
| `DefaultApprovalLevel` | 审批级别（per-image state）|
| `CSetupPageContrast`, `CSetupPageDot` | UI 标签页类，说明 Contrast、DotColor 是独立参数组 |

### 🎯 Pipeline 子系统分类（CorrPartNames）

`messages.xml::CorrPartNames` 列出 7 个可调整**独立模块**：
1. **Gradation Sliders**（宏控制：亮度/对比度/饱和度/明度/色温/色调）
2. **Gradation Curves**（RGB 7 通道 Hermite 曲线）
3. **Histogram**（Shadow / Gray / Highlight / DotColor 白黑点）
4. **Color Corrections**（6×6 CCM 矩阵）
5. **Sharpness**（USM）
6. **FlexTouch**（除尘/划痕修复）
7. **Color Noise Filter**（色彩噪声去除）

每组在 `ImageCorrection` 里有独立的 `Apply{Sliders,Curves,Histogram,CC,USM,Dust,CNFilter}` 布尔开关。

### 🎯 Lightness 就是 Shadow Depth

FlexColor UI 里**没有"Lightness" slider**——只有 `Shadow Depth`（tooltip: "increases detail in dark areas"）。我们模型里 `Lightness` 参数对应的就是 "Shadow Depth"。

公式方向确认：gamma < 1（提亮暗部），与我们当前实现一致。

### 🎯 ICC 后端：Windows ICM API，不是 lcms2

`FlexColor.dll` import 了一堆 Windows ICM API：
- `OpenColorProfileW`, `CloseColorProfile`
- `AssociateColorProfileWithDeviceW`
- `TranslateColors` ← 核心 ICC 变换 API
- `GetColorProfileElement`, `SetColorProfileElement`
- `IsColorProfileValid`, `IsColorProfileTagPresent`

**影响**：Windows ICM 和 lcms2 在 PCS 量化、rendering intent 实现细节上**可能有细微差异**。对严格像素对齐而言，用 lcms2 无法完美复制 Windows ICM 的行为。但差异一般在 P99 量级，STRICT 级大部分 case 应该够用。

### 其他杂项发现

- `Remove Cast Highlight` / `Remove Cast Shadow` 是独立 bool 开关（与 levels 联动去色偏）
- `Enhanced shadow detail` 是独立特性，与 Shadow Depth **不同**（布尔 vs 数值）
- `Lineart Threshold` 存在——BW 二值化阈值
- `AdaptiveLight` 自适应光照——我们**完全没实现**
- **无 "Lightness" slider in UI**——UI 里"明度"表现为 Shadow Depth

### T7 产出清单（立即可执行的改动）

- [ ] 修正 `film_curve_name` 的映射顺序（As Shot/Standard/Low/High/Old/Linear）
- [ ] `build_curve_lut` 改 Hermite 插值（消费 `dy` 而不是忽略）
- [ ] 文档内 `lightness` 注释改为 "Shadow Depth"
- [ ] 添加 `AdaptiveLight` 到 TODO（未来特性）
- [ ] `docs/pipeline-status.md` 中 pipeline 图按 CorrPartNames 顺序重写

---

## 十三、反编译结果 · 已破解的算法

> 所有反编译通过 `tools/ghidra_query/` 自动化流程得到：
> `./run.sh list-methods <Class>` → `./run.sh decompile 0xADDR` → `./run.sh read-const 0xDATADDR`

### 共同观察

- **工作空间：14-bit（0–16383 = 0x4000）**，不是 16-bit
  - 常量常量 `16383.0`, `16384.0`, `8192.0`（pivot/mid）反复出现
  - 这解释了为什么 Shadow/Highlight 参数需要乘 4 才能到 16-bit 域
- **架构：x86 32-bit**，`__thiscall` 调用约定（`this` 在 `ecx`）
- **Slider 归一化**：多数 slider 值除以 **50.0**（不是 100.0），然后乘 2 得到最终系数。意思是 slider 值 100 → 系数 4。
- **参数 struct 布局**：`this->field_0x1c` 是指向参数结构体的指针，offset 0x4fe/0x8d/... 内的字节是各 slider 值

---

### ✅ CContrastCurve::Apply @ 0x70267220

**功能**：对比度 + 亮度联合曲线（14-bit 输入 → 14-bit 输出的 LUT，共 0x4000=16384 项）。

**公式**（已从反编译完全还原）：

```python
# 从 params 结构读取
contrast   = params[0x4fe]   # 带符号 byte, -100..100
brightness = params[0x8d]    # 带符号 byte, -100..100

C = (contrast / 50.0) * 2.0        # 归一化 → [-4..4]
B = (brightness / 50.0) * 8192.0   # 绝对亮度偏移（14-bit 单位）

# 暗部（v ∈ [0, 8192)）:
for v in 0..8192:
    if brightness <= 0:
        b = 1.0
    else:
        b = v / 8192.0
    new_v = v + B * b + C * v * (v - 8192) / 16384.0
    new_v = clamp(new_v, 0, 16383)

# 亮部（v ∈ [8192, 16384)）:
for v in 8192..16384:
    if brightness <= 0:
        b = (16383 - v) / 8192.0
    else:
        b = 1.0
    new_v = v + B * b + C * (v - 8192) * (16384 - v) / 16384.0
    new_v = clamp(new_v, 0, 16383)
```

**关键结构**：
- **S-curve contrast**：`v * (v - pivot)` 在暗部产生负形状（darker darks）、`(v - pivot) * (max - v)` 在亮部产生正形状（brighter lights）
- **Pivot = 8192**（14-bit mid，绝对居中，不自适应）
- **Brightness** 是**带形状的加性偏移**：brightness > 0 时亮部全量加、暗部逐渐减到 0；brightness < 0 反之——实现"brightness 在阳光下不烧高光、在阴影里不压死黑"的直觉
- **Slider → 系数**：50 是归一化基数，100 → 系数 2（不是 1）

**对比我们当前实现** (`src/color/adjust.rs::apply_display_adjust`)：
```rust
let scale = if c >= 0.0 { 1.0 + c * 2.0 } else { 1.0 + c };
v = ((v - 0.5) * scale + 0.5).clamp(0.0, 1.0);
```
- 线性**拉伸**（不保留极端值），FlexColor 用**加性 S-curve**（保留极端）
- 我们 pivot 在 0.5；FlexColor 在 8192/16383 ≈ 0.5001（一致）
- slider 归一化：我们 `c/100`，FlexColor `c/50`；slider 100 → 我们 scale=3，FlexColor `C=4` 但效果小得多（因为是加性而非乘性）

**常量表**（`tools/ghidra_query/` 实测）：

| 地址 | 值 | 用途 |
|---|---|---|
| `_DAT_70734060` | 8192.0 | pivot |
| `_DAT_70733528` | 50.0 | slider 归一化 |
| `_DAT_70734058` | 1/8192 = 0.0001220703125 | 1/pivot |
| `_DAT_70733988` | 16383.0 | 14-bit max |
| `_DAT_707338b8` | 16384.0 | 14-bit range |
| `_DAT_70733990` | 1/16384 = 6.103515625e-05 | 1/range |

**实测对比**（slider 值 contrast=20, brightness=0）：

| 输入 v | 我们公式输出 | FlexColor 公式输出 | 差异 |
|---|---|---|---|
| 1024（暗部） | **0**（被截断）| 666 | 我们完全丢失细节 |
| 4096（中暗） | 2621 | 3277 | 我们过黑 −20% |
| 8192（mid） | 8192 | 8192 | 相同 |
| 12288（中亮） | 13763 | 13107 | 我们过亮 +5% |

**直接影响**：
- **rgb_dark / cmyk_dark / 所有含 contrast=20 的 preset** 可望从 FAIL 降到 PASS/WARN
- 先实现 FlexColor 公式 → 跑 manifest 回归对比

---

### ✅ CGammaCurve::Apply @ 0x702667e0 → 0x70266830

**功能**：正片 gamma 曲线（14-bit LUT builder，大小 0x4000）。

**结构**：Apply 是一个 dispatcher
```c
void CGammaCurve::Apply(short *lut) {
    if (this->flag_0x08 != 0) FUN_70266830(this, lut);   // 内部实现
    else                       FUN_7026a300(this, lut);   // 调 CAggregateCurve::Apply
}
```

**公式**（从 x87 FPU 汇编还原）：

```python
# 读取参数
G = params[0x52c]  # float gamma slider，典型值 1.5–2.5，默认 2.0

# 计算实际 pow 指数（分两路）
if G >= 2.0:
    exponent = 1.0 / (G - 1.0)           # 简化路径
else:
    exponent = 1.0 / (1.0 - (2.0 - G) * 0.8)  # G < 2 的平滑路径

# 填 16384-entry LUT
for i in 0..16384:
    v = i / 16383.0                       # 归一化到 [0, 1]
    lut[i] = round(pow(v, exponent) * 16383.0)
```

**关键观察**：

- **G = 2.0 是中性（identity）**：exponent = 1.0 → `pow(v, 1) = v`
- **G > 2 亮化**（简化公式，`1/(G-1) < 1`）
- **G < 2 暗化**（平滑公式避免 G → 1 时指数爆炸）
- **G 有效下界 ~0.75**：低于此值 `1 - (2-G)*0.8 ≤ 0`，奇点

**对比我们当前 `levels_gamma[0] = gamma - 1`**：

| G | 我们 exp = 1/(G-1) | FlexColor exp | 备注 |
|---|---|---|---|
| 2.5 | 0.667 | 0.667 | ✓ 一致（都在 ≥2 路径） |
| 2.0 | 1.0 | 1.0 | ✓ identity |
| 1.5 | 2.0 | **1.667** | ❌ 我们过黑 |
| 1.0 | ∞（clamp 0.01→1/0.01=100）| **5.0** | ❌ 我们极端黑 |

**常量**：

| 地址 | 值 | 用途 |
|---|---|---|
| `_DAT_70734070` | 0.8 | G<2 路径的调整因子 |
| `_DAT_70733988` | 16383.0 | 14-bit max |

**参数位置**：`params->field_0x52c` 是 gamma slider（float）。

---

### ✅ CGammaNegCurve::Apply @ 0x70266310 （8-bit LUT 变体）

**功能**：负片 gamma — 但这个方法输出 **256-entry 字节 LUT**（用于缩略图/直方图渲染），不是主管线。

**公式**：
```python
# ⚠️ 勘误（2026-04-19）：公式漏了 1 − v² 项（与 14-bit 变体同构造，见 §18.7）
if params is None or params[0x51c] == 0:
    return identity_256byte_lut()

# 读配置
gamma = registry_get("NegVarGamma")  # 浮点值，若非 (0.099, 10.1) 则用默认 0.2
input_scale = 255.0
if params[0x518] != 0:
    input_scale = 275.5                            # EnhancedShadow
stretch = registry_get("StretchNegGamma")          # 若在 (1.01, 1.11)，input_scale *= stretch
output_scale = 255.0                               # 8-bit max

exponent = 1.0 / gamma

for i in 0..256:
    v = i / input_scale
    out = pow(1.0 - v*v, exponent) * output_scale  # ★ 1 − v² 反转 + pow
    lut[i] = clamp_byte(round(out))
```

**关键观察**：
- **这不是主 16384-LUT 路径** — 负片的大 LUT 由 CAggregateCurve 合成
- **NegVarGamma 默认 0.2** → exponent = 5.0（非常陡，匹配 C-41 负片的对数密度响应）
- **StretchNegGamma 范围 1.01–1.11** → 轻微输出范围扩张（最多 11%）
- **8-bit 输出 scale 275.5 > 255**：有意允许 overflow 再 clamp，而不是线性压缩

**常量**：

| 地址 | 值 | 用途 |
|---|---|---|
| `_DAT_707338d0` | 0.2 | NegVarGamma 默认值 |
| `_DAT_70735140` / `_DAT_70735138` | 0.099 / 10.1 | gamma 有效范围 |
| `_DAT_707336f8` | 255.0 | 默认 scale |
| `_DAT_70735148` | 275.5 | stretch scale |
| `_DAT_70734640` / `_DAT_70735128` | 1.01 / 1.11 | StretchNegGamma 有效范围 |

**待办**：
- [ ] 查 `NegVarGamma` / `StretchNegGamma` 这些 registry key 的实际值（参考 Windows 注册表或 FFF 里的设置）

---

### ✅ CGammaNegCurve 16384-LUT Builder @ 0x702664e0 （真正的 C-41 反转）

**公式**（从 x87 FPU 汇编完全还原）：

```python
gamma = registry_get("NegVarGamma") or 0.2   # 默认 0.2
out_scale = 16383.0                           # 默认
if params[0x518]:
    out_scale = 17700.0                       # stretched output flag
stretch = registry_get("StretchNegGamma")     # 若在 (1.01, 1.11)，有效
if 1.01 < stretch < 1.11:
    out_scale *= stretch                      # 最多 ~19600

exponent = 1.0 / gamma                        # 默认 5.0

# 填 16384-entry LUT（14-bit 输入 → 14-bit 输出）
for i in 0..16384:
    v = i / out_scale                         # 归一化
    y = pow(1.0 - v*v, exponent)              # ⭐ 1-v² 的幂次方
    lut[i] = clamp_ushort(round(y * 16383.0))
```

**这就是真正的 C-41 负片反转曲线**。结构上是：
1. **线性归一化** `v = i / out_scale`
2. **平方 + 反相**：`1 - v²` — 在 v≈0（暗部负片，亮部正片）处平坦（保留亮部细节），v→1 处陡降
3. **幂次压缩**：`pow(..., 1/gamma)` — gamma=0.2 时指数 5，非常陡
4. **输出量化**：乘 16383 映射到 14-bit 输出

**典型值**（gamma=0.2, out_scale=16383）：
- `lut[0] = pow(1, 5) * 16383 = 16383` — 极暗输入 → 极亮输出
- `lut[4096] = pow(1 - 0.0625, 5) * 16383 = 11956` — 暗部输入 → 中亮
- `lut[8192] = pow(1 - 0.25, 5) * 16383 = 3887` — 中部 → 中暗
- `lut[12288] = pow(1 - 0.5625, 5) * 16383 = 298` — 亮部输入 → 几乎黑
- `lut[16383] = pow(0, 5) * 16383 = 0` — 极亮输入 → 黑

**对比我们当前的线性公式** `(hi - raw) / hi * 65535`：我们没有 squaring、没有 pow、没有 gamma — **根本不是同一曲线**。这彻底解释了 C-41 负片管线为什么差异巨大。

**常量**：

| 地址 | 值 | 用途 |
|---|---|---|
| `_DAT_707338d0` | 0.2 | NegVarGamma 默认 gamma |
| `_DAT_70733988` | 16383.0 | 14-bit max (output scale, default) |
| `_DAT_70735130` | 17700.0 | stretch output scale |
| `_DAT_70733750` | 100.0 | 某初始化常量（待查） |
| `_DAT_70734640` / `_DAT_70735128` | 1.01 / 1.11 | StretchNegGamma 有效区间 |

**关键观察**：
- **14-bit 工作域**（again），i ∈ [0, 16384)
- `stretch` 最多让 out_scale 变 17700 × 1.11 ≈ 19647 → i/out_scale 最大 ≈ 0.834 → v² 最多 0.695 → 允许输出不降到 0（保留阴影细节）
- **这是单一固定曲线**：不同 preset（C-41 的 Std/Sat/Dark）并不改这个曲线，而是改其上下游的 histogram、sliders、CC 矩阵

---

### ✅ CAggregateCurve · Pipeline 合成器

**职责**：把多条子曲线按特定模式**组合**成单一 16384-LUT。

**主 LUT builder** @ 0x7026a300：
```c
void Apply(short *out) {
    if (this->flag_0x08) {
        for (short i = 0; i < 0x4000; i++)
            out[i] = compute_single(this, i);   // 调 slot-12 虚函数
    } else {
        for (short i = 0; i < 0x4000; i++) out[i] = i;   // identity
    }
}
```

**单像素 compute** @ 0x70268210（完整解码）：
```python
def compute_single(agg, v_in):
    v = v_in
    n = agg.field_0x100            # child count (short)
    for i in 0..n:
        mode = agg.field_0x88[i*4]              # int mode per child
        child = agg.field_0x10[i*4]              # CCurve* child
        if mode == 0:
            # ① SEQUENCE: 串联。上一个 curve 输出 → 下一个输入
            v = child.compute_single(v)
        elif mode == 1:
            # ② ADD: 并联加。v_in + child(v_in)
            v = clamp_u16(v_in + child.compute_single(v_in))
        elif mode == 2:
            # ③ SUBTRACT: 并联减。v_in - child(v_in), clamp [0, 16383]
            r = v_in - child.compute_single(v_in)
            v = clamp(r, 0, 16383)
    return v
```

**内存布局**：
- `0x10..0x88`：最多 30 个 `CCurve*` 子指针（`(0x88-0x10)/4 = 30`）
- `0x88..0x100`：30 个 `int mode` 对应
- `0x100`：short count

**意义**：
- FlexColor 的整条 pipeline（contrast + gamma + curves + shadow/highlight + ...）最终被**合并为一张 16384-entry LUT**，每像素一次查表搞定
- 模式 0 (串联) 是默认（effects 叠加）；模式 1/2 用于特殊组合（可能用于 DotColor 等）
- **这解释了为什么 FlexColor 快**：pipeline 再复杂，应用时只是 LUT 查找

**待确认**：谁把子曲线添加到 `agg.field_0x10`？这回答了"执行顺序"问题。应在 `CImageCorrection` 的构造或某 `Build`/`Setup` 函数里。

---

### ✅ CHighShadowCurve · 阴影/高光三区间曲线

**单像素 compute** @ 0x70267720（解码后）：

```python
def compute_single(self, v):
    shadow_thr    = self.field_0x1c   # ushort, 阴影阈值
    highlight_thr = self.field_0x1e   # ushort, 高光阈值
    shadow_out    = self.field_0x20   # ushort, 阴影区输出
    highlight_out = self.field_0x22   # ushort, 高光区输出
    shadow_mode   = self.field_0x2c   # int: 0=flat, 1=linear, 2=clip-to-0
    highlight_mode = self.field_0x30  # int: 0=flat, 1=linear, 2=clip-to-max
    sub_curve     = self.field_0x18   # CCurve* 中间段子曲线
    scale_mid_in  = self.field_0x24   # float, 中段输入缩放
    scale_mid_out = self.field_0x28   # float, 中段输出缩放

    if v < shadow_thr:
        if shadow_mode == 0: return shadow_out            # 平钳
        if shadow_mode == 1: return shadow_out * v / shadow_thr  # 线性下降
        if shadow_mode == 2: return 0                     # 硬钳零
    elif v < highlight_thr:
        # 中段：映射到子曲线再缩放回来
        remapped = round((v - shadow_thr) * scale_mid_in)
        sub_val  = sub_curve.compute_single(remapped)
        return round(shadow_out + sub_val * scale_mid_out)
    else:  # v >= highlight_thr
        if highlight_mode == 0: return highlight_out      # 平钳
        if highlight_mode == 1:
            # 线性到 max
            return round(
                highlight_out
                + (v - highlight_thr) * (16383 - highlight_out) / (16384 - highlight_thr)
            )
        if highlight_mode == 2: return 16383              # 硬钳 max
    return v  # 默认 identity
```

**这是"Shadow Depth"功能的底层**：
- 三区间（暗 / 中 / 亮）各自独立处理
- 每端有 3 种模式（clamp / linear / hard-cut）
- 中段允许嵌套一条子曲线做 S-curve / gamma 等

**对应 FFF 预设字段**：
> ⚠️ **勘误（2026-04-19）**：下列映射应为 **EndPoints**（不是 DotColor）。正确映射见 §28.3：
> - `EndPoints.shadow[ch]` (byte) @ this+0x4ff/0x500/0x501 → shadow_out (scale × 16383/255)
> - `EndPoints.highlight[ch]` (byte) @ this+0x506/0x507/0x508 → highlight_out
> - `Shadow[ch]` (ushort) @ this+0x11e8/0x11ea/0x11ec → shadow_boundary
> - `Highlight[ch]` (ushort) @ this+0x540/0x542/0x544 → highlight_boundary
> - `EndPoints.shadow_mode` (int32) @ this+0x510 → shadow_mode
> - `EndPoints.highlight_mode` (int32) @ this+0x514 → highlight_mode

- `DotColor[0..3]`（RGB shadow_out）→ shadow_out
- `DotColor[7..10]`（RGB highlight_out）→ highlight_out
- `shadow` / `highlight` 数组 → thresholds
- `Enhanced Shadow` 标志可能切换 shadow_mode

---

### ✅ CContrastBrightnessCurve · 是 CContrastCurve 的 clamp 包装

**单像素 compute** @ 0x70267010 — 只是一层薄 wrapper：
```c
ushort compute_single(...) {
    ushort r = FUN_7061c3c0(param1, param2);   // 委托给底层（与 CContrastCurve 共用）
    return clamp(r, 0, 16383);
}
```

**结论**：`CContrastBrightnessCurve` **不是独立算法**，它复用 CContrastCurve 的内部计算，只多一层 u14 clamp。我们之前 T2 的公式对它也成立。

---

### 📊 已知 pipeline 组件全景

| Class | 主 LUT 地址 | 单像素地址 | 状态 |
|---|---|---|---|
| `CContrastCurve` | @ 0x70267220 | @ 0x70268770（lut[v]>>6 shortcut） | ✅ 公式解 |
| `CContrastBrightnessCurve` | — | @ 0x70267010（wrapper）| ✅ 同上 |
| `CGammaCurve` | @ 0x70266830 | @ (未单独测) | ✅ 公式解 |
| `CGammaNegCurve` | @ 0x702664e0 | @ 0x702661f0（lut[v]）| ✅ 公式解 |
| `CHighShadowCurve` | 继承 Aggregate @ 0x7026a300 | @ 0x70267720 | ✅ 三区间解 |
| `CAggregateCurve` | @ 0x7026a300 | @ 0x70268210 | ✅ 3-mode composer 解 |
| `CImageCorrection` | - | @ 0x702d4f80（top entry） | ⚠️ 部分理解 |
| `CFilmCurve` | (vtable slot 12) | (见下) | ⏳ 待分析 |

---

### ✅ CFilmCurve · 纯数据，非代码

**发现**：CFilmCurve 的 vtable **只含 1 个方法（析构）**。RTTI 存在但**无 compute 逻辑**：
```
类 CFilmCurve 的所有符号: 7
  [Label] RTTI_Type_Descriptor
  [Label] RTTI_Complete_Object_Locator
  [Label] vftable  (仅 1 slot: destructor)
  ...
扫描 parent namespace == CFilmCurve 的函数: 0
```

**结论**：胶片曲线不是代码 — 是**数据驱动**。

FlexColor 的 `FilmCurve` 参数（0–5 六档：As Shot / Standard / Low Contrast / High Contrast / Old Standard / Linear）不对应 6 个算法公式，而是**选择 6 份预存 LUT**（或更少，几个共享）之一，由 CAggregateCurve 应用。

**对应我们工作**：
- 与其尝试"反推 FilmCurve 公式"（公式不存在），不如**直接 dump `.rdata` 里的 LUT 数据**（T4 任务）
- LUT 很可能就是 16384-entry 的 ushort 数组，按 FilmCurve 索引挑选

**待办**：T4 · `find-luts` 扫描时，特别标记 **长度 == 16384** 的 ushort 数组并比对其 vftable/起始地址上下文。

---

### ✅ CColorWorld · 只有析构

和 CFilmCurve 一样，`CColorWorld` 和 `CColorCorrection` 的 vtable 只各含 1 slot（析构）。它们是**色彩空间上下文容器**，承载参数与状态，**处理逻辑由调用方（CImageCorrection 等）使用**。

进一步查这些类的数据字段和使用位置（xref），可以弄清 FlexColor 的色空间管理（YCbCr? 还是其它），但不会像 Curve 类那样直接出公式。

> ⚠️ **勘误（2026-04-19）**：CColorCorrection 内部结构已解（§29.3）—含 6×6 double 矩阵 + 2×64K ushort LUT。**不是 YCbCr**，而是 16-bit RGB 的矩阵 + pre/post LUT 变换（典型 ICC 风格）。色彩校正实质走 CImageCorrection 的 14-bit per-channel curve pipeline（§16.5）+ 可选 CColorCorrection 的 16-bit 后处理（§29.3）。

---

## 十四、总结 · pyghidra + Ghidra 已破解的知识地图

> ⚠️ **注意**：本节为早期阶段性总结。pipeline 拓扑图已被 **§16.5** 完全取代（Section 14 的 children 列表含过期成分，如 CFilmCurve/CContrastBrightnessCurve 实际不在运行时 pipeline）。复刻时以 §16.5 + §25 为准。

### Pipeline 核心工作域：14-bit（0–16383）

FlexColor 内部**统一用 14-bit 做所有 curve 计算**：
- Pivot = 8192（绝对中心，不自适应）
- Max = 16383
- 我们 16-bit 数据**必须除 4** 进 FlexColor 域，出来再乘 4

这解释了 FFF 参数里 `shadow × 4`、`highlight × 4` 的惯例 — 它们是 **FlexColor 14-bit 参数转 16-bit 存储**。

### Curve 架构层次

```
CImageCorrection (顶层)
  └── per-channel per-pixel dispatch @ 0x702d4f80
        └── CAggregateCurve (16384-LUT 合成器)
              │ 3-mode chain: SEQUENCE / ADD / SUBTRACT
              ├── CFilmCurve LUT（5 档预存数据）
              ├── CGammaCurve  · pow(v, 1/[func(G)])
              ├── CGammaNegCurve · pow(1 - v², 1/gamma)
              ├── CContrastCurve · v + C·v·(v−pivot)/16384 + B·zone
              ├── CContrastBrightnessCurve · 同上 + clamp 包装
              ├── CHighShadowCurve · 3 区间（shadow / mid / highlight）+ 子曲线
              └── ...（其他 curves，待挖）
```

### 公式表（立即可重写到 Rust）

> ⚠️ **勘误（2026-04-19）**：Neg Gamma 公式行已修为 `pow(1−(v/scale)², 1/γ)`，与 §18.7 一致。"我们当前"列描述的仍是旧 Rust 实现（未动）。

| 参数 | FlexColor 公式 | 我们当前 | 匹配度 |
|---|---|---|---|
| Contrast | v + C·v·(v−8192)/16384 + B·zone(v) | (v−0.5)·(1+2c) | ❌ 完全不同 |
| Gamma (G≥2) | pow(v/16383, 1/(G−1)) × 16383 | 同（×65535）| ✓ 一致 |
| Gamma (G<2) | pow(v/16383, 1/(1−(2−G)·0.8)) × 16383 | pow + clamp | ❌ 公式不同 |
| Neg Gamma | pow(1−(v/scale)², 1/0.2) × 16383 | (hi−raw)/hi × 65535 | ❌ 完全不同 |
| Shadow/Highlight | 3-zone 分段 + 子曲线 | 线性钳位 | ❌ 粗得多 |

### Slider 统一规则

- **除数 50**（不是 100）：slider 100 → 内部系数 2
- **Pivot = 8192**（不是 0.5 × 65535 = 32767.5）
- **输出 clamp** 到 [0, 16383]（not [0, 65535]）

### 下一步研究方向

1. **T4 · Dump `.rdata` 找 5 份 FilmCurve LUT**（CFilmCurve 是数据，不是代码）
2. **CImageCorrection::Apply 完整 trace** — top-level dispatch 逻辑
3. **CAggregateCurve::SetChildren 路径** — 谁注册 child curves 及顺序
4. **CColorWorld 数据字段分析** — 确认 YCbCr 矩阵（T1）
5. **实现 Contrast + GammaNeg 两条公式到 Rust** — 最大 ROI，直接改善 8+ 个 case

### pyghidra 工具状态

`tools/ghidra_query/` 已稳定工作，本次新增命令：
- `run-rtti` · 跑 Windows RTTI 分析器恢复 C++ 类（首次用，后续不再需要）
- `disasm <ADDR>` · 反汇编到指令级（处理 x87 FPU）
- `read-const <ADDR>` · 读 `_DAT_xxx` 常量的 double/float/int 值
- `class-methods-all <ClassName>` · 列类的所有符号 + vtable xref
- 及之前的 `list-classes`, `list-methods`, `decompile`, `vtable`, `dump-pipeline`, `find-luts`

每条 curve 算法 30–60 分钟内破解，效率已验证。

---

## 十五、第二轮深挖 · Pipeline 构造器 + CNegativeCurve（2026-04-19）

### ✅ CNegativeCurve @ vftable 0x707198fc — 真正的 C-41 反转主曲线

**不是我们之前以为的 CGammaNegCurve**！pipeline 构造里显式用的是 `CNegativeCurve`，其 16384-LUT 在 slot 8 (0x70266ac0)。

**公式**（2 段 quadratic，pivot 在 x_param 处平滑连接）：

```python
# 参数（per-channel 存在 CNegativeCurve 对象的 field_0x20/0x24）
x_param = field_0x20 * 64.0   # shadow pivot position (14-bit 域)
y_param = field_0x24 * 64.0   # value at pivot
max_val = 16383.0             # _DAT_70733988
c6 = 2.0                      # _DAT_70733548

# 检查"NoNegCurve" registry 禁用开关
if registry_get("NoNegCurve"):
    return identity_lut()

# 填 16384 LUT
for v in 0..16384:
    if v >= x_param:
        # 上段：(x, y) → (max_val, *) 的 quadratic
        d = v - x_param
        D = max_val - x_param
        q = ((max_val - y_param) - D) / (D * D)   # 负数，凸曲线
        v_out = d*d * q + d + y_param
    else:
        # 下段：(0, 0) → (x, y) 的 quadratic
        k = (x_param - y_param) / (x_param * x_param)
        linear_scale = 1.0 - (x_param - y_param) * c6 / x_param
        v_out = k * v*v + linear_scale * v

    # 边界钳制
    if v_out <= 0 or v_out >= 16384.0:  # _DAT_70733970 as float = 16384
        lut[v] = 16384 if v >= x_param else 0
    else:
        lut[v] = round(v_out)
```

**边界验证**：
- `v = 0` → lower 段，`k*0 + linear*0 = 0` ✓
- `v = x_param` → lower 段 `k*x² + linear*x`，代入化简 = `y_param` ✓
- `v = x_param` → upper 段 `0 + 0 + y_param` = `y_param` ✓ (连续)
- **两段在 pivot 处一阶导数连续**（凸性不同但值连续）

**默认参数**（对三通道 R/G/B 有不同）：
- 构造时从 `_DAT_7073529c/98, _DAT_707352a4/a0, _DAT_707352ac/a8` 取
- ⚠️ **勘误（见 §18.8）**：此前写的 x/y 方向颠倒。正确赋值：
  - `puVar1[8]` → field_0x20 ← `_DAT_707352b4 = 158.7` → **x_param = 158.7 × 64 = 10156**
  - `puVar1[9]` → field_0x24 ← `_DAT_707352b0 = 100.6` → **y_param = 100.6 × 64 = 6438**
- 意义：pivot 在 x=10156 (14-bit)，输出 y=6438。**不是抬升阴影，而是降低 "中高光"**（因为 pivot 处输出比 identity 低 3718）。

**含义**：CNegativeCurve 是**负片的 tone 曲线 base**，针对不同胶片/设置可调整 x/y 参数。FilmCurve 5 档（Standard/Low/High/Old/Linear）**很可能就是通过调整 x/y 实现**，不需要独立 LUT。

> ⚠️ **勘误（2026-04-19）**：FilmCurve 实际是 **6 档**（见 §十二）：As Shot / Standard / Low Contrast / High Contrast / Old Standard / Linear。此外，FilmCurve 是否通过调整 CNegativeCurve 的 x/y 参数实现 —— T18 正在查，暂未证实。

---

### ✅ CImageCorrection 构造器 @ 0x702d5a20 — Pipeline 完整拓扑

**发现**：调用 FlexColor 的**实际 pipeline 图谱**，由 CImageCorrection 的构造函数 FUN_702d5a20 完整揭示。

#### 共享曲线对象（构造一次）

```c
this->field_0x18 = new CContrastCurve      // 共享
this->field_0x14 = new CGammaCurve         // 共享
this->field_0x74 = new CNegativeCurve      // 共享，默认 (100.6, 158.7)
this->field_0x84 = new CAggregateCurve(0xf8)   // ???
this->field_0x84 + curvepoints via FUN_702693f0(CCurvePoint, x=0x202, y=const, dy=1.0)  // Gradation curve
this->field_0x84 + curvepoints(x=0x6464, dy=1.0)  // 另一个点
```

#### 逐通道构造（3 次循环：R / G / B）

```python
# 每个通道 local_28 ∈ {1, 2, 3}：
channel[i] = {
  # 创建一个 CGammaNegCurve 实例（每通道独立）
  gamma_neg_per_channel = new CGammaNegCurve()
      gamma_neg.field_0x1c = this (backptr)
      gamma_neg.field_0x20 = i (channel index!)

  # 创建一个 CAggregateCurve（容量 30 child，0x104 bytes）
  inner_agg = new CAggregateCurve
      .children = []
      .add(this.CGammaCurve,    mode=0 SEQUENCE)
      .add(this.field_0x28,     mode=0)  # 未知类型，待查
      .add(this.CNegativeCurve, mode=0)  # 共享 neg curve
  
  # 每通道独立 CNegativeCurve（用 R/G/B 专用 x/y 参数）
  neg_per_channel = new CNegativeCurve
      .field_0x20 = per_channel_x  # R: 不同值，G: 不同，B: 不同
      .field_0x24 = per_channel_y
  inner_agg.add(neg_per_channel, mode=0)
  inner_agg.add(earlier_curve_A, mode=1 ADD)
  inner_agg.add(earlier_curve_B, mode=2 SUBTRACT)  
  inner_agg.add(this.CContrastCurve, mode=0)   # ← Contrast 在这里！neg 反转之后
  
  # CSinglePointCurve（新类！）
  single_pt = new CSinglePointCurve (0x28 bytes)
      .field_0x20 = _DAT_707346dc
      .field_0x24 = _DAT_707346dc
      .field_0x1c = 1  # flag
  inner_agg.add(single_pt, mode=0)
  
  # CHighShadowCurve（0x34 bytes）
  high_shadow = new CHighShadowCurve
      .field_0x18 = inner_agg      # sub_curve = 上面的 inner agg!
      .field_0x22 = 0x4000         # highlight_out = 16384
      .field_0x20 = 1              # shadow_mode = linear
      .field_0x14 = this
      .field_0x10 = inner_agg
  
  # 外层 aggregate（只 2 个 child）
  outer_agg = new CAggregateCurve
      .add(gamma_neg_per_channel,  mode=0)
      .add(high_shadow,            mode=0)  # 内含 inner_agg
  
  channel[i] = outer_agg
}
```

#### Pipeline 总图

```
Input RGB (per pixel, per channel) [14-bit, 0..16383]
   │
   ▼
┌─────────────────────────────────────────────────┐
│ channel[i].outer_agg (SEQUENCE):                │
│   ├─ (1) CGammaNegCurve (per-channel)           │ ← 每通道独立 gamma 配置
│   │                                             │
│   └─ (2) CHighShadowCurve (per-channel):        │ ← 3-zone tone
│         │                                       │
│         └─ sub_curve = inner_agg (SEQUENCE):    │
│              ├─ [0] CGammaCurve (shared)        │ ← 正片 gamma pow
│              ├─ [1] this->field_0x28 (?)        │ ← 待查
│              ├─ [2] CNegativeCurve (shared)     │ ← C-41 反转 (默认)
│              ├─ [3] CNegativeCurve (per-ch)     │ ← per-channel 微调
│              ├─ [4] earlier_curve_A (+ADD)      │ ← 待查（可能 DotColor）
│              ├─ [5] earlier_curve_B (-SUB)      │ ← 待查
│              ├─ [6] CContrastCurve (shared)     │ ← 对比度！
│              ├─ [7] CSinglePointCurve           │ ← 单点钳制?
│   (next curves not yet read)                    │
└─────────────────────────────────────────────────┘
   │
   ▼
Output [14-bit]
```

### 关键洞察

1. **Contrast 在 C-41 反转 AFTER**：负片先反为正像，再应用 Contrast。我们目前架构反了：Contrast 在 display_adjust 阶段，在 ICC 之后。
2. **Per-channel 有独立副本**：CGammaNegCurve（每通道）、CNegativeCurve（每通道）。共享的只有 CContrastCurve、CGammaCurve、CNegativeCurve 的 default。
3. **Shadow/Highlight (CHighShadowCurve) 是包装容器**：实际 curve 操作通过其 sub_curve 完成，它只做 3-zone clamp/extrapolate。
4. **CCurvePoint** 出现在构造中 — 用户画的 Gradation Curves 也是此类的集合，应该用 Hermite 插值。
5. **两种未知类**：`this->field_0x28`（待查）、`earlier_curve_A/B`（modes 1/2 用于加减）、`CSinglePointCurve`（新类）。

### 下一步

> ✅ **全部已完成（2026-04-19）**：见 §16.1-16.3。CSinglePointCurve 解码（16.1），field_0x28 是 Master CPointCurve（16.3），earlier_A/B 是 per-channel 用户曲线同址（16.3 注），FUN_70268c40 是 **CPointCurve ctor**（非 CAggregateCurve，16.2）。

1. ~~**CSinglePointCurve** — 查 vtable 和 compute_single~~ → §16.1
2. ~~**this->field_0x28** — 看构造函数更早部分（在 FUN_702d5a20 调用之前）~~ → §16.3 Master CPointCurve
3. ~~**earlier_curve_A/B** — 同样查它们什么时候被赋值~~ → §16.3 per-channel CPointCurve
4. ~~**FUN_70268c40** (aggregate 的真实 ctor，带 0xf8 size) — 搞清 0xf8 vs 0x104 区别~~ → §16.2

---

## 16. 完整 Pipeline 拓扑（2026-04-19 更新）

**前置谜团全部揭示**：CSinglePointCurve 解码、0xf8 class 识别、7 个 user curve slot 的归属。

### 16.1 CSinglePointCurve (vtable @ 0x7071986c)

Slot 12 `compute_single` @ `0x70268b30`：
```c
ushort compute(ushort x) {
    if (this->enabled != 0) return lut[x];  // offset 0x14 指向 u16 LUT
    return x;  // 未启用 → 直通
}
```
- 大小 0x28 bytes
- 构造时 `enabled` (offset 0x1c) = 1，默认启用
- 字段 `0x20 / 0x24` 分别存两个 double（初始值 `_DAT_707346dc`），与 CNegativeCurve 的参数布局一致 → 推测它是 "单点可调节" curve，可能对应 UI 的 "Offset / Neutral" 滑块

### 16.2 0xf8-aggregate 实际是 CPointCurve（不是 CAggregateCurve！）

FUN_70268c40 的 vtable 是 `CPointCurve::vftable` (@ 0x707198b4)，**非** CAggregateCurve！两者本源不同：

| 类 | 大小 | 含义 | slot 12 compute_single |
|----|------|------|------------------------|
| `CAggregateCurve` | 0x104 | 最多 30 个 children + 每 child 一个 mode flag | 串联 child 处理 |
| `CPointCurve` | 0xf8 | 最多 10 个 CCurvePoint（用户控制点） | 纯 LUT 查表 @ offset 0x14 |

CPointCurve::compute_single @ `0x702661f0`：
```c
ushort compute(ushort x) {
    return lut[x];  // 直接返回 14-bit LUT 第 x 项
}
```
LUT 通过 FUN_70269600（build_lut from points + hermite/spline）预计算，并在 user 调整曲线时重建。

### 16.3 CImageCorrection 字段地图

外层 ctor FUN_702d4360 创建 7 个 CPointCurve，内层 ctor FUN_702d5a20 引用它们 + 额外创建一个（this+0x84）。最终：

| 偏移 | 类 | 用途（推断） |
|------|-----|------|
| this+0x14 | CGammaCurve (shared) | 正片 gamma (default=1.0) |
| this+0x18 | CContrastCurve (shared) | 全局对比度 |
| this+0x28 | **CPointCurve (shared)** | ★ Master 曲线（无通道区分） |
| this+0x2c | CPointCurve (R) | R 通道独立曲线 A |
| this+0x30 | CPointCurve (G) | G 通道独立曲线 A |
| this+0x34 | CPointCurve (B) | B 通道独立曲线 B |
| this+0x38 | CPointCurve (R) | R 通道独立曲线 B |
| this+0x3c | CPointCurve (G) | G 通道独立曲线 B |
| this+0x40 | CPointCurve (B) | B 通道独立曲线 B |
| this+0x74 | CNegativeCurve (shared) | C-41 default (707352b4/b0) |
| this+0x84 | CPointCurve (inline) | 内嵌点曲线，初始化为 [(2,1.0),(100,1.0)] — 估计用于 CSinglePointCurve LUT 生成 |
| this+0x1c..0x20 | per-channel CGammaNegCurve | ch=1,2,3 |
| this+0x2c..0x34（同上）| 注意 0x2c 起的 slot 兼作 earlier_A 数组 |
| this+0x50..0x58 | per-channel CHighShadowCurve | |
| this+0x78..0x80 | per-channel 外层 CAggregateCurve | |

**注意**：分别在 `0x2c/0x30/0x34` 的 CPointCurve 与 earlier_A 其实是**同一对象** —— 内层循环的 `local_2c[-0xc]` 索引它们到 inner_agg 作为 mode=1 child。即 "per-channel 用户曲线" = "earlier_A"。同样 `0x38/0x3c/0x40` = earlier_B（mode=2）。

### 16.4 CAggregateCurve compute_single 的三种 mode（精确定义，2026-04-19 勘误）

⚠️ **勘误**：此前把 mode 1/2 的 child 输入标为"原始入口 input"是错的。实际反汇编（`./run.sh disasm 0x70268210`）显示 EDI 寄存器的行为：mode 0 的 child 返回值**同时**写回 EDI，而 mode 1/2 **读 EDI 但不更新**。所以 mode 1/2 的 child 输入是 "最近一次 mode 0 child 输出"（若尚无 mode 0 则是原始 input）。

更精确的伪代码：
```c
ushort compute_single(ushort input) {
    ushort running = input;     // 在 [ESP+0x10]
    ushort last_m0 = input;     // 在 EDI（mode 0 的 child 输入 + mode 1/2 的 child 输入）
    for (i = 0; i < count; i++) {
        child = this.children[i];
        mode = this.modes[i];
        if (mode == 0) {              // SEQUENTIAL
            ushort y = child(last_m0);   // 注意：传 last_m0，不是 running
            running = y;
            last_m0 = y;                  // 同时更新
        } else if (mode == 1) {       // ADD_DELTA
            ushort y = child(last_m0);   // 读 last_m0，不更新
            running = (ushort)(running + (short)y);
        } else if (mode == 2) {       // SUB_CLAMP
            ushort y = child(last_m0);
            int tmp = running - y;
            running = (tmp < 0) ? 0 : (tmp > 0x3FFF ? 0x3FFF : tmp);
        }
    }
    return running;
}
```

**关键差别**（与旧伪代码）：
- mode 1/2 的 child 输入是 `last_m0`（"上一次 mode 0 产生的值，或最初的 input"），**不是"原始 input"**。
- 在当前 FlexColor pipeline 中，inner_agg 的前 4 个 child 都 mode 0，所以到 mode 1 时 `last_m0` 已经是"经过 Gamma/Master/Neg_shared/Neg_ch 后的值"，**不等于**原始 input。这是 user 曲线 A/B 被应用于"反转后的正像域"的机制。
- 复刻时不可假设 mode 1/2 的 child 输入是原始像素；必须跟踪 `last_m0` 变量。

在 8-bit 路径（slot 10）与 14-bit 路径（slot 12）语义一致，只是 clamp 上限和 child 调用的 vtable slot 不同（slot 0x28 vs 0x30）。

### 16.5 完整数据流（14-bit domain, 0..16383, clamp at 0x3FFF）

```
input_14bit (per pixel, per channel R/G/B)
   │
   ▼ outer_agg[channel] SEQUENCE:
   ├─ CGammaNegCurve[channel]  (channel-specific gamma-neg)
   │
   └─ CHighShadowCurve[channel]  (3-zone shadow/highlight tone)
         │
         └─ sub_curve = inner_agg SEQUENCE:
              [0] CGammaCurve (shared)       ← pow(x, gamma)
              [1] CPointCurve[0x28] (shared) ← Master 用户曲线 LUT
              [2] CNegativeCurve[0x74] (shared)  ← C-41 default: (field_0x20=158.7, field_0x24=100.6)
              [3] CNegativeCurve (channel)   ← 每通道 neg 参数
              [4] CPointCurve[0x2c|0x30|0x34] mode=1 (ADD_DELTA from original)
              [5] CPointCurve[0x38|0x3c|0x40] mode=2 (SUB_CLAMP)
              [6] CContrastCurve (shared)    ← 对比度/亮度/cast
              [7] CSinglePointCurve (channel) ← 单点 LUT（可能 enabled=1，默认启用）
output_14bit
```

### 16.6 我们与 FlexColor 的差异点（排序后）

1. **顺序错位**：我们的 pipeline 是 `raw → Contrast → Neg`，FlexColor 是 `Gamma → Master → Neg(shared) → Neg(ch) → UserA → UserB → Contrast → SinglePoint`。Contrast 应在 Neg 之后。
2. **Master 曲线 vs per-ch 曲线分离**：我们没有区分。UI 的 "Gradation" 曲线实际对应 `this+0x28`，应在 gamma 之后、neg 之前应用；per-channel 曲线是后处理 delta。
3. **CSinglePointCurve**：我们完全忽略。默认 enabled=1 时它会应用一次 LUT — 需查它的 LUT 内容（由 `this+0x84` 点驱动）。
4. **mode=1/mode=2 双输入语义**：per-channel 用户曲线是 delta，不是串联。这解释了为什么直接把 user curve 塞进 pipeline 会过强。
5. **CGammaNegCurve + CHighShadowCurve 嵌套**：我们仅做单次 ICC transform（经 lcms2 走 sRGB profile）；FlexColor 嵌两层（外 channel-specific neg-gamma，内 shared linear gamma）。⚠️ *"sRGB gamma" 措辞不准确 — 实际我们走的是 sRGB ICC profile 而非直接 pow(1/2.2)*

### 16.7 CPointCurve 的两套 LUT（8-bit & 14-bit）

CPointCurve 内部同时维护两张 LUT，根据消费者精度选用：

| 字段 | 类型 | 构建函数 | 规模 | 用途 |
|-----|------|---------|------|------|
| `this+0x10` | `u8*` (256 B) | slot 7 @ `0x70268d60` | 256 entries, byte out | 8-bit 预览 / 缩略图管线 |
| `this+0x14` | `u16*` (32 KB) | slot 8 @ `0x70268fc0` | 256 entries × 64 step = 16384 out range | 14-bit 主管线（compute_single 用这个） |

两者都从同一组 CCurvePoint 采样，差异仅在 y 值的缩放：
- slot 7 传 `param_7 = 1` 给插值函数 → 输出 `y ∈ [0..255]`
- slot 8 传 `param_7 = 0x40 (64)` → 输出 `y ∈ [0..16320]`，再 clamp 到 `sVar2 = param_7 * 0x100 - 1 = 0x3FFF`（精确 14-bit max）

插值器 FUN_702698c0 + FUN_702699e0：对每个控制点 i，根据参数 `t` 计算一个权重（形状函数），与 `point[i].y * scale` 加权求和。控制点结构体 0x14 bytes：
```
offset 0x00: vtable (CCurvePoint)
offset 0x04: type/enabled
offset 0x08-0x20: coefs / data (具体含义待深入)
```
此为**基函数求和**形式，很可能是 Hermite / cubic B-spline（不是 Catmull-Rom——后者用差分）。

### 16.8 索引 vs 采样点数

关键数字：LUT 驱动时，循环变量 `local_50` 从 0 步进 `local_48`，直到 255；每次插值得到一个 (x, y) pair，然后在输出 LUT 上填充 `x_prev..x_cur` 区间的线性插值（slot 7 见 `*pcVar12 = (char)iVar9 + (char)sVar10`）。所以：

- 每条 CCurvePoint 控制一段曲线
- 相邻控制点之间用**线性填充**（非 spline）到 LUT
- 控制点本身的 (x, y) 由基函数在参数 t 处求值得到

结论：FlexColor 的用户 Gradation Curve 实为 **"参数采样 + 线性填充"** 的 hybrid，不是纯 Hermite spline。复刻时应用同样的近似。

### 16.9 B-spline 基函数（确认！）

FUN_702699e0 是 **Cox-de Boor 递归**，构造 B-spline basis function：
```
N(i, 1, t) = 1  if knot[i] <= t < knot[i+1]
           = 0  otherwise

N(i, k, t) = (t - knot[i])     / (knot[i+k-1] - knot[i])     * N(i, k-1, t)
           + (knot[i+k] - t)   / (knot[i+k]   - knot[i+1])   * N(i+1, k-1, t)
```

param_1 = i (control point index), param_2 = k (order, 剩余递归深度), param_3 = knot vector (short*), param_4 = t。

**结论：FlexColor 的 Gradation Curves 是 B-spline 曲线。** 复刻时：
1. 用户拖的 (x, y) 点作为 control points
2. 按 knot 向量（可能 uniform clamped knots）求值 B-spline basis
3. 参数 t 从 0 步进到 255，每步采样得 (x_t, y_t)
4. 相邻采样点之间用**线性内插**填充 LUT

Cox-de Boor 的 `k` 值即 B-spline 阶数。需进一步确认构造时传入的 `param_3`（`param_2 + iVar5` 计算 knot 数），但从控制点 10 个、knot 向量至少 `n+k+1` 个角度推断，阶数可能为 4（cubic）。

### 16.10 下一步（给逆向继续用）

1. **CCurvePoint 字段 0x08-0x20 布局** — 其中一个是 x 坐标 (0..255 byte)，一个是 y 坐标 (double)，其余可能是插值系数
2. **CHighShadowCurve LUT 构建** — slot 7/8 应该也有对应的批 build 接口，我们目前只读到 compute_single
3. **查 XML → CImageCorrection setter** — 从哪些 XML 节点填 this+0x28 的 points 与 neg 参数（搜 "Curve"、"Points"、"NegAmount" 等字符串 xref）
4. **this+0x84 CPointCurve 的作用** — 其 hardcoded (2,1), (100,1) 仅为 endpoints，后续 user 调整（如 Neutral slider）写入这里
5. **验证**：写 test 用 Rust 管线按 16.5 顺序重新组装，观察是否与 FlexColor 输出吻合

---

## 17. XML 解析注册机制（部分追踪，2026-04-19）

### 17.1 注册模式

每个 XML key 对应一个 7 指令的"注册 thunk"，分布在 `.text` 段 0x7069xxxx 区域：

```asm
PUSH <key_string_addr>      ; 键名
MOV  ECX, <parser_instance> ; 特定 key-parser 单例
CALL [0x706b34dc]           ; CRegistry::Register(key)
PUSH <destructor_ptr>
CALL _atexit                ; 注册单例析构
POP  ECX
RET
```

找到的 thunk：
| Key | Thunk Addr | Parser Instance |
|-----|-----------|-----------------|
| `PhysicalWidth` | 0x7069f7d0 | 0x70a5f074 |
| `Gradations` | 0x7069f810 | 0x70a5f0ac |
| `Gamma` | 0x7069f870 | 0x70a5f0ac (同 Gradations) |
| `streamableVersion` | 0x7069ce00 | 0x70a5ca30 |
| `Points` | 0x7069ce20 | 0x70a5ca4c |

**关键观察**：Gamma 和 Gradations 共用同一 parser instance (0x70a5f0ac) — 说明 ImageCorrection 层级下的所有字段由一个 CImageCorrectionXMLParser 处理；Points 独立为 CGradationXMLParser (0x70a5ca4c)，被 Gradations parser 作为子 parser 调用。

### 17.2 卡点

Handler thunk 跳转到 `[0x706b34d4]`，这是 Ghidra 未解析的 IAT entry（值 0x007ac30c 不在任何映射段）。PE loader 运行时修补，静态分析无法追踪。

**绕行方案**：
- **动态分析**：x64dbg 挂 FlexColor.exe，在 0x706b34d4 下硬件断点，读修补后的指针
- **经验反推**：构造 minimal XML diff，跑 FlexColor 观察输出，反推 key→slot 映射（见 T11）

### 17.3 Gradations 数组语义（推测，待 T11 验证）

test XML 的 `<Gradations>` 是长度 7 的 array，与外层 ctor 创建的 7 个 CPointCurve 数目吻合：

| 索引 | 假设对应字段 | 语义 |
|-----|-------------|------|
| [0] | this+0x28 | Master 曲线（跨通道）|
| [1] | this+0x2c | R UserA（mode 1 delta）|
| [2] | this+0x30 | G UserA |
| [3] | this+0x34 | B UserA |
| [4] | this+0x38 | R UserB（mode 2 clamp）|
| [5] | this+0x3c | G UserB |
| [6] | this+0x40 | B UserB |

Point 的 `DY` 字段（值 1）可能是 B-spline knot weight / 端点类型标记。

---

## 18. 完整 XML 字段映射（2026-04-19 重大突破）

### 18.1 发现路径

`CImageCorrection::vtable` 8 slot：
- slot 0 (0x702d5f70) — XML 完整读/写入口
- slot 1 (0x702d6f20) — Gradations 数组迭代（7 次循环）
- **slot 2 (0x702d5540) 返回常量 11 = `streamableVersion` 的值**
- slot 3 返回 2 — 另一版本号
- slot 4 (0x702d4580) — destructor
- slot 5 (0x702d7590) — **XML 写出主体**，调用 FUN_702d81f0 按 version 分派
- slot 6 (0x702d79a0) — 未详查
- slot 7 (0x702d4f80) — per-pixel **"中性平衡"** 函数（不是 curve apply！用 this+0x11e8 的 3 个参考色做 white balance）

slot 5 调 `FUN_702d81f0(version, writer)`，后者按 case 0..11 序列化不同 version 引入的字段 — 这给出完整字段偏移表。

### 18.2 XML 注册表（825 个 thunk，过滤 ImageCorrection 子集）

用新工具 `./run.sh dump-xml-registry` 扫出全部 XML key 注册 thunk（pattern: `PUSH key_str; MOV ECX, parser_DAT; CALL [0x706b34dc]`）。ImageCorrection 子集共 52 keys，parser_DAT 分布在 0x70a5f0xx..0x70a5f6xx。

### 18.3 XML Key → this+offset 完整映射

> ⚠️ **勘误（2026-04-19）**："since_ver" 列的 version 编号**不可信**。实测 streamableVersion=11 的 XML 也包含 case 0xc (12) 的字段 `<FilmType>`（见 `settings/Standard Negative/Negative RGB standard.xml`）。真相：FUN_702d81f0 的 switch(in_EAX) 不是按 streamableVersion 分派，更可能是**按 UI 功能组**（sliders / curves / histogram / USM / profile ...）分批写出，所有组都会触发。偏移 / 类型 / XML key 列**仍准确**，只是 "since_ver" 列须忽略。

从 FUN_702d81f0 的 version dispatch 反推：

| XML Key | offset | type | since_ver | 说明 |
|---------|--------|------|-----------|------|
| **Gradations** | 0x28 | 7 × CPointCurve | 1 | ★ 7 条用户曲线（Master+R/G/B×2） |
| ApplyCC | 0x88 | byte (bool) | 3 | 是否应用 ColorCorrection |
| ApplyUSM | 0x89 | byte (bool) | 4 | 是否应用 USM 锐化 |
| ApplyDust | 0x8a | byte (bool) | 5 | 是否应用除尘 |
| **Brightness** | 0x8d | int8 | 0 | -100..+100 |
| **Saturation** | 0x4fc | int16 | 3 | |
| **Contrast** | 0x4fe | int8 | 0 | -100..+100 |
| EnhancedShadow | 0x518 | byte (bool) | 6 | 决定 CGammaNegCurve 走哪套 scale 常量 |
| **Gamma** | 0x52c | float32 | 0 | 默认 1.0/2.0 |
| Gray | 0x536 | struct | 2 | 灰场 RGB 参考点（this+0x11e8 正是此组数据！）|
| Highlight | 0x53e | struct | 2 | 高光 RGB 参考点 |
| **Lightness** | 0x96c | int16 | 0 | |
| Mode | 0x970 | int | 10 | 处理模式（Normal/BW/…）|
| Shadow | 0x11e6 | struct | 2 | 暗部 RGB 参考点（与 slot 7 的 this+0x11e8 吻合，存在 0x11e6/e8/ea 三 short）|
| Threshold | 0x11f0 | int16 | 1 | |
| USMAmount | 0x11f2 | int16 | 4 | 锐化强度 |
| USMColFactor | 0x11f4 | struct (8B?) | 4 | 色通道因子 |
| USMDarkLimit | 0x11fa | int16 | 4 | |
| USMNoiseLimit | 0x11fc | byte | 4 | |
| USMRadius | 0x11fe | int16 | 4 | 锐化半径 |
| DustLevel | 0x1214 | byte | 5 | |
| **ApplySliders** | 0x1218 | byte (bool) | 0 | 是否应用 Gamma/Contrast/Brightness 等滑块 |
| ApplyCurves | 0x1219 | byte (bool) | 1 | 是否应用 Gradations 曲线 |
| ApplyHistogram | 0x121a | byte (bool) | 2 | 是否应用直方图（Shadow/Highlight/Gray）|
| ColorNoiseRadius | 0x121c | int32 | 6 | |
| ApplyCNFilter | 0x1220 | byte (bool) | 6 | |
| ColorTemperature | 0x1224 | int32 | 8 | |
| Tint | 0x1228? | ? | 8 | （case 8 第二个 write，待确认偏移）|
| ColorModel | 0x1238 | int32 | 3 | RGB/CMYK/Gray enum |
| FilmCurve | 0x123c | int32 | 1 | 胶片曲线 preset 索引 |
| NoiseFilterBias | 0x1240 | int16 | 5 | |
| ? (case 7) | 0x1244 | int32 | 7 | DAT_70a5f608（未映射 key）|
| VignetteAmount? | 0x1248 | int32 | 7 | DAT_70a5f624 |
| **CMYKProfile** | 0x90 | CString | 11 | ICC profile path |
| **SoftProof** | 0x11ee | byte (bool) | 11 | 软打样开关 |
| **Convert** | 0x1200 | byte (bool) | 11 | 颜色转换开关 |
| **EmbedProfile** | 0x1201 | byte (bool) | 11 | 是否嵌入 profile |
| **InputProfile** | 0x548 | CString | 11 | 输入 ICC 路径 |
| **RGBProfile** | 0x99c | CString | 11 | RGB 输出 ICC 路径 |
| **GrayProfile** | 0xdc0 | CString | 11 | Gray 输出 ICC 路径 |
| **FilmType** | 0x51c | uint32 | 12 | ★ 胶片类型 enum（0=关 / 非 0=启用 Negative）|

### 18.4 关键洞察：ApplySliders / ApplyCurves / ApplyHistogram / ApplyUSM / ApplyCC / ApplyCNFilter / ApplyDust

每个主要功能模块都有独立 bool 开关。Pipeline 执行顺序受这些开关控制：
- `ApplySliders=true` → 应用 Contrast/Brightness/Gamma/Lightness/Saturation
- `ApplyCurves=true` → 应用 7 条 Gradations
- `ApplyHistogram=true` → 应用 Shadow/Gray/Highlight 三点映射
- `ApplyCC=true` → 应用 Color Correction (ICC-based)
- `ApplyUSM=true` → 应用 USM 锐化
- `ApplyCNFilter=true` → 应用色噪过滤
- `ApplyDust=true` → 应用除尘

复刻时每个开关对应独立 pipeline 阶段启用/跳过。

### 18.5 Color Model enum (this+0x1238)

从 XML `<ColorModel>0</ColorModel>` 测试：0 = RGB，可能 1/2 = CMYK/Gray。需动态验证。

### 18.6 Film Curve (this+0x123c)

`FilmCurve` 是预设索引 (int32)，从 UI 下拉选择胶片类型。默认 0 = "None"，具体值对应哪种胶片待查（可能触发不同的 CGammaNegCurve 默认参数）。

### 18.7 CGammaNegCurve 公式（完整，2026-04-19 勘误）

**⚠️ 审查勘误**：此前版本漏写了 `1 − v²` 反转项。真实公式（由 FUN_702664e0 内循环 x87 指令直接读出）：

```asm
FILD i               ; ST0 = i (int→float)
FDIV input_scale     ; ST0 = i / input_scale  ≡  v
FMUL ST0             ; ST0 = v²
FLD1                 ; ST0 = 1, ST1 = v²
FSUBRP               ; ST0 = 1 − v²            ★ 关键：反转
FLD exponent         ; exponent = 1/gamma
CALL _CIpow          ; ST0 = (1 − v²) ^ (1/gamma)
FMUL 16383.0         ; ST0 *= output_scale
; round and store
```

即：
```
for i in 0..N:
    v = i / input_scale
    LUT[i] = round( pow(1 - v*v, 1/gamma) * 16383.0 )
```

**语义**：这不是纯 gamma，而是 **"1 − v² 负片反转" 与 "pow(·, 1/gamma)" 的复合**。反转项 `1 − v²` 天然完成"亮→暗"的底片→正像映射：
- v=0 (输入 0) → `1−0=1` → `1^(1/γ) × 16383 = 16383` （最亮）
- v=1 (输入满) → `1−1=0` → `0 × 16383 = 0` （最暗）
- 中间走二次曲线的 gamma power，暗部被提升（因为 `1−v²` 在 v≈1 区域斜率大）

**默认 gamma=0.2（即 exponent=5）时**：
| v | 1 − v² | (1−v²)^5 | LUT[i] |
|---|--------|----------|--------|
| 0.00 | 1.000 | 1.000 | 16383 |
| 0.25 | 0.938 | 0.724 | 11860 |
| 0.50 | 0.750 | 0.237 | 3885 |
| 0.75 | 0.438 | 0.016 | 264 |
| 1.00 | 0.000 | 0.000 | 0 |

这是典型 C-41 负片曲线：暗部压缩比线性强得多。

**完整流程（正确版）**：
```
if (!this->enabled) → LUT = identity
if (parent && parent->field_0x51c != 0):       # FilmType 门控
    gamma = XML["NegVarGamma"] if in (0.099, 10.1) else 0.2   # _DAT_707338d0
    input_scale = parent->field_0x518 ? 17700 : 16383          # EnhancedShadow
    stretch = XML["StretchNegGamma"] in (1.01, 1.11)
    if stretch valid: input_scale *= stretch
    for i in 0..16384:
        v = i / input_scale
        LUT[i] = round( pow(1 - v*v, 1/gamma) * 16383.0 )
```

8-bit 变体 (FUN_70266310) 使用 scale=255（主）/ 275.5（EnhancedShadow）、output_scale=255。

### 18.8 CNegativeCurve 默认参数（更正）

（Section 15 曾将 x/y 颠倒，此为更正）

| 实例 | field_0x20 | field_0x24 | 来源 |
|------|-----------|-----------|------|
| shared (this+0x74) | 158.7 | 100.6 | _DAT_707352b4 / _DAT_707352b0 |
| R per-ch | 107.5 | 145.1 | _DAT_7073529c / _DAT_70735298 |
| G per-ch | 139.9 | 124.5 | _DAT_707352a4 / _DAT_707352a0 |
| B per-ch | 146.7 | 90.4 | _DAT_707352ac / _DAT_707352a8 |

CSinglePointCurve 默认 field_0x20 = field_0x24 = 128.0 (_DAT_707346dc)。

### 18.9 新工具：dump-xml-registry

路径：`tools/ghidra_query/scripts/dump_xml_registry.py`。
扫 `.text` 段的 7-指令注册 thunk pattern，一次性输出 (thunk_addr, key_string, parser_DAT_addr) 三元组。825 个 key 总量，按 DAT 聚类能快速识别不同子对象的 schema。

### 18.10 下一步

1. **slot 0 (FUN_702d5f70) 完整读取** — XML read 对应 setter，反推能确认字段偏移
2. **slot 6 (FUN_702d79a0) 用途** — 循环 14 次 FUN_702d7ba0，未明
3. ~~**读所有 CGammaNeg 常量**~~ — 见 Section 19
4. **FUN_702d81f0 case 7/11** — 补完 VignetteAmount 等剩余字段
5. **Shadow/Gray/Highlight 的 struct 布局**（FUN_702d8cf0）— 3 个 short RGB？8 字节？
6. **USMColFactor struct** — 类似上

---

## 19. CGammaNeg + slot 7 常量表（2026-04-19）

全部为 double (8 bytes)。

### 19.1 Gamma-Neg pipeline 常量

| 地址 | 值 | 角色 |
|------|-----|------|
| 0x70735140 | **0.099** | NegVarGamma 合法下限（slider 最小值）|
| 0x70735138 | **10.1** | NegVarGamma 合法上限 |
| 0x707338d0 | **0.2** | NegVarGamma 默认值（XML 缺失或越界时用）|
| 0x707338d8 | **-1.0** | 8-bit 版本 NegVarGamma 默认（特殊值：标志位？）|
| 0x70733750 | **100.0** | XML "NegVarGamma" 读取前的初始值（若失败则 100 → 越界 → fallback）|
| 0x70734640 | **1.01** | StretchNegGamma 合法下限 |
| 0x70735128 | **1.11** | StretchNegGamma 合法上限（意味着 stretch 只在 1.01..1.11 生效，微调区间）|
| 0x70733988 | **16383.0** | 14-bit 主 output/input_scale（= 2^14-1）|
| 0x707336f8 | **255.0** | 8-bit 主 output/input_scale |
| 0x70735130 | **17700.0** | 14-bit EnhancedShadow input_scale（略大于 16383）|
| 0x70735148 | **275.5** | 8-bit EnhancedShadow input_scale（略大于 255）|

### 19.2 CGammaNegCurve 最终公式（2026-04-19 勘误版）

⚠️ **勘误**：此前公式缺少 `1 - v²` 反转项。参见 Section 18.7 的 x87 指令级反编译证据。

```
# 14-bit 版本 (FUN_702664e0)
if !enabled || !parent || parent.field_0x51c == 0:
    LUT = identity[0..16384]
else:
    gamma = XML["NegVarGamma"]
    if !xml_read_ok or gamma <= 0.099 or gamma >= 10.1:
        gamma = 0.2
    input_scale = 17700.0 if parent.field_0x518 != 0 (EnhancedShadow) else 16383.0
    stretch = XML["StretchNegGamma"] ∈ (1.01, 1.11)
    if stretch valid: input_scale *= stretch
    for i in 0..16384:
        v = i / input_scale
        LUT[i] = round( pow(1 - v*v, 1/gamma) * 16383.0 )
```

**EnhancedShadow 分支**：启用后 input_scale 16383→17700，`v_max = 16383/17700 ≈ 0.926`，`1 - v²_max = 1 - 0.857 ≈ 0.143`，`0.143^5 ≈ 6e-5`，`×16383 ≈ 1`。即"最暗输入 → 输出 1（近 0，但不真 0）"，**避免了 input_scale=16383 时 v=1 → LUT[16383]=0 的硬截断**，给阴影区留了更多动态范围。这就是 "Enhanced Shadow" 的实际作用。

### 19.3 slot 7 (CImageCorrection 白平衡) 常量

| 地址 | 值 | 角色 |
|------|-----|------|
| 0x70988690 | 3.0 | 除数（RGB 三通道平均）|
| 0x706e0650 | 10.0 | 加法 offset |
| 0x70734cd0 | -4.0 | 乘数 |
| 0x7073366c | 原始 `00 00 80 4f 00 00 00 00` | `(float)4294967296.0` = 2^32，用于 signed→unsigned 修正（`if (x < 0) x += 2^32`）|

slot 7 的白平衡阈值公式（从 FUN_702d4f80 第一部分）：
```
ref[0..2] = *(short *)(this+0x11e8), *(short *)(this+0x11ea), *(short *)(this+0x11ec)
tmp[c] = ref[c]² / 16383
mean_tmp = (tmp[0] + tmp[1] + tmp[2]) / 3
fVar = FUN_7027c660()  # 未知调用（可能是读 XML 或 this 成员，输出参数 alpha）
upper_threshold = round(alpha * 3.0 + 10.0) + mean_tmp
lower_threshold = mean_tmp - round(alpha * 3.0 + 10.0) * (-4.0)
                = mean_tmp + 4 * round(alpha * 3.0 + 10.0)
```

(Note: signs and ordering are tentative; needs FUN_7027c660 decode.)

### 19.4 FUN_7027c660 = sqrt

已确认：FUN_7027c660 是对 `_CIsqrt` 的简单包装，返回 float。slot 7 里 `fVar18 = FUN_7027c660();` 后续用 `fVar18 * 3.0 + 10.0` → 所以 **slot 7 的 alpha = sqrt(FPU_stack_top_value)**。具体 sqrt 参数由 x87 FPU stack 传递，decompiler 丢失了源。从上下文（调用前的 FPU 操作）推测 alpha = √mean_tmp 或 √(Σref²)/3。

### 19.5 下一步（Section 19 补充）

1. **x87 FPU stack 追踪** — 反汇编 0x702d4f80 确认 sqrt 的确切参数
2. **slot 7 是否被主 pipeline 调用** — 如果是，白平衡在 CImageCorrection::Apply 之外独立调用

---

## 20. CAggregateCurve 的多精度 vtable slots

### 20.1 三套 compute_single

| slot | 地址 | 精度 | 返回 | 备注 |
|------|------|------|------|------|
| 10 | 0x70268360 | 8-bit in → 8-bit out | uint (低 8 bit) | mode 0/1/2 同 14-bit 版本；clamp 到 0xff |
| 11 | 0x70268430 | 14-bit in → 8-bit out | uint | `return slot_12(x) >> 6` — 直接位移降精度 |
| 12 | 0x70268210 | 14-bit in → 14-bit out | uint | ★ 主管线用这个（Section 16.4） |

**重要**：slot 11 通过移位降精度给 8-bit 输出，不是走 slot 10 的独立 8-bit 路径。这意味着 **14-bit LUT 是主数据源，8-bit 只是 downsample**。

### 20.2 mode 语义在两套间一致

slot 10（8-bit）的分支结构与 slot 12（14-bit）完全一致：
- mode 0：sequential chain（child(running)）
- mode 1：`running = (ushort)(original + (byte)child(original))` — ADD_DELTA from original
- mode 2：`running = running - (child(original) & 0xff)`，clamp [0, ∞)（8-bit 不 clamp 上限，让 ushort 自然溢出？不完全，代码里有负数 clamp 为 0）

### 20.3 对 Rust 复刻的启示

我们的 14-bit 实现只需复刻 slot 12。8-bit 路径（slot 10/11）只给 thumbnail/preview 用。

### 20.4 CPointCurve 同样有多精度

回忆 Section 16.7：CPointCurve slot 7 = 8-bit LUT builder (×1 scale), slot 8 = 14-bit LUT builder (×64 scale)。同样 **14-bit 是主，8-bit 是 downscale**。整个 pipeline 统一在 14-bit 域。

---

## 21. FUN_702d8cf0 = ushort 数组写出器

`void FUN_702d8cf0(writer, key_DAT, ushort *arr)` 拷贝 `unaff_ESI`（调用者压栈的长度）个 ushort 到新 uint buffer，然后 `writer.write(key_DAT, buf)`。

**含义**：Shadow / Gray / Highlight / USMColFactor 都是 **ushort 数组**（不是 single scalar）。长度由调用者定，通常是 3 (RGB triple)。

### 21.1 Shadow/Gray/Highlight 精确布局

| 字段 | 偏移 | 类型 | 长度 |
|------|------|------|------|
| Shadow | this+0x11e6 | ushort[3] | 6 bytes — RGB 暗部参考点 |
| Gray | this+0x536 | ushort[3] | 6 bytes — 中灰参考点 |
| Highlight | this+0x53e | ushort[3] | 6 bytes — 高光参考点 |
| USMColFactor | this+0x11f4 | ushort[3] | 6 bytes — USM 通道因子 |

**交叉确认**：slot 7 (white balance) 读 `this+0x11e8, 0x11ea, 0x11ec` 为 3 个 ushort — 即 Shadow 的 R/G/B 分量（偏移 0x11e6 + 2/4/6 = e8/ea/ec）。

等等，this+0x11e6 的 ushort[3] 是 0x11e6/e8/ea 而不是 e8/ea/ec。这个差一组说明：**Shadow 字段起始偏移是 0x11e6**，包含 3 个 ushort 占 0x11e6..0x11eb，slot 7 读的是 **Highlight-的前 3 个 ushort？不对，Highlight 起于 0x53e。这里有矛盾，下一轮需澄清**：
- 可能 slot 7 读的 this+0x11e8/ea/ec 其实不是 Shadow 而是 0x11e6 Shadow 之后的另一字段（也许 "Threshold/EndPoints" 之类，占 0x11ec..）
- 或 FUN_702d8cf0 写的长度不是 3 (RGB) 而是其他

**待澄清**：回头解析 FUN_702d8cf0 的压栈参数（长度值）和 0x11ec 偏移的字段。

---

## 22. 重新整理：slot 7 (FUN_702d4f80) 的完整伪代码

基于 0x11e8/ea/ec 读取 + sqrt + 阈值混合，还原为：

```python
def slot7_white_balance(this, pixels: ushort[], count: ushort):
    # this+0x518 是启用标志（EnhancedShadow 同一位？或另一个？）
    # this+0x51c 必须为 0（否则跳过此阶段，由其他阶段处理）
    if this.field_0x518 == 0 or this.field_0x51c != 0:
        return  # 不做
    
    # Step 1: 计算三参考色平方
    ref = [*(ushort *)(this + 0x11e8 + 2*c) for c in 0..3]  # shadow RGB or ref triple
    tmp = [(ref[c] * ref[c]) // 16383 for c in 0..3]         # squared/scaled
    mean_tmp = (tmp[0] + tmp[1] + tmp[2]) // 3               # via magic multiply
    
    # Step 2: 阈值计算（含 sqrt）
    alpha = sqrt( ... FPU_stack value ... )   # 未知具体参数（推测 mean_tmp 或 sum）
    upper_threshold = round(alpha * 3.0 + 10.0) + mean_tmp
    lower_threshold = mean_tmp - round(alpha * 3.0 + 10.0) * (-4.0)
                    = mean_tmp + 4 * round(alpha * 3.0 + 10.0)
    
    # Step 3: 按组 3 (RGB) 处理像素
    pixel_count = (count - 1) // 3 + 1  # 上取整
    for _ in range(pixel_count):
        p = pixels[0..2]
        mean = (p[0] + p[1] + p[2]) // 3
        delta_sum = sum(p[c] - tmp[c] for c in 0..3)
        
        if mean < upper_threshold:
            t = 0.0
            apply = True
        elif mean < lower_threshold:
            t = (mean - upper_threshold) / (lower_threshold - upper_threshold)
            apply = (t < 1.0)
        else:
            apply = False
        
        if apply:
            for c in 0..3:
                new_c = tmp[c] + (1-t) * delta_sum/3 + t * (original_c - tmp[c])
                pixels[c] = clamp(new_c, 0, 16383)
        pixels += 3
```

**语义解读**：
- 当像素 mean 低于 `upper_threshold`（暗部）：**完全对齐 tmp 参考值 + 通道间 delta 平均**，强力去色偏
- mean 在 [upper, lower] 之间：按 `t` 线性混合"去色偏模式" → "保留模式"
- mean 高于 `lower_threshold`（亮部）：**不动**

即：**暗部做去色偏，亮部保留**。这就是扫描 C-41 底片常需要的 "shadow cast removal"。配合 XML 字段 **RemoveCastShadow**（Section 17 中的 key）—极可能此 slot 由 `RemoveCastShadow=true` 触发。

参考点是 Shadow（this+0x11e6/e8/ea 或附近）的 RGB，意味着用户通过 Shadow 滑块指定的参考色决定了去色偏的目标值。

---

## 23. FilmType = CGammaNegCurve 主开关（关键）

> ⚠️ **勘误（2026-04-19）**：本节声称 "FilmType 是 streamableVersion 12 引入" **错误**。实测 streamableVersion=11 的 XML（`Standard Negative/Negative RGB standard.xml`）已含 `<FilmType>1</FilmType>`。"since_ver" 编号不可信，见 §18.3 勘误。FilmType 作为 CGammaNegCurve 启用门控的结论**仍成立**（通过 CGammaNegCurve build 里 `parent->field_0x51c != 0` 检查已证实，§18.7）。

### 23.1 发现

FUN_702d81f0 case 12 (version 12)：
```c
writer.write(&DAT_70a5f330 /* "FilmType" */, *(uint*)(this + 0x51c));
```

**this+0x51c 就是 CGammaNegCurve build 函数里检查的 `parent->field_0x51c`**（Section 18.7 的 `parent.field_0x51c != 0` 分支）！

也就是说：
- `FilmType == 0` (XML `<FilmType>0</FilmType>`) → CGammaNegCurve 禁用，LUT = identity
- `FilmType != 0` → CGammaNegCurve 启用，生成真实的 neg gamma LUT

这解释了为什么正片（"RGB standard" 这种 preset）和负片（"negative rgb standard"）的管线差异如此大——**FilmType 开关直接决定 gamma-neg 是否被应用**。

### 23.2 EnhancedShadow = this+0x518

case 6 写：`writer.write("EnhancedShadow", *(byte*)(this+0x518))`。这匹配 Section 18.7 的 `parent.field_0x518` — 即 CGammaNegCurve build 里决定 input_scale 是 16383 还是 17700 的那个开关。

### 23.3 FilmType 的具体枚举值（待查）

XML 写 `<FilmType>11</FilmType>` 或类似——实际值含义需：
1. 找 CGammaNegCurve 的 build 函数里是否按 FilmType 值选择不同的 gamma 默认
2. 或找 UI 字符串（"Negative", "Positive", "Kodak Portra" 等）对应的枚举数值

---

## 24. Case 11 新增字段（streamableVersion=11，我们 XML 用的版本）

当 XML 里 `<streamableVersion>11</streamableVersion>` 时，会同时触发 case 0..11 的全部字段反/序列化：

| 字段 | offset | 类型 | 含义 |
|------|--------|------|------|
| CMYKProfile | 0x90 | CString (~32 B) | ICC path |
| InputProfile | 0x548 | CString | 输入 profile |
| RGBProfile | 0x99c | CString | RGB 输出 profile |
| GrayProfile | 0xdc0 | CString | Gray 输出 profile |
| SoftProof | 0x11ee | byte (bool) | |
| Convert | 0x1200 | byte (bool) | |
| EmbedProfile | 0x1201 | byte (bool) | |

**关键**：ICC profile 路径是**字符串**（不是嵌入二进制），意味着 FlexColor 运行时按文件名加载 profile。我们无法从 DLL 里直接提取 profile；需找到对应 ICC 文件或通过 Windows ICM API 查询。

`writer.write` 调 slot 0x18 而非 slot 4/8/0xc — 说明 slot 0x18 专门处理字符串。前面的调用：
- slot 4 (int16/int32) — 整数
- slot 8 (double) — 浮点
- slot 0xc (byte/bool) — 布尔
- slot 0x18 (CString*) — 字符串 ★
- slot 0x5c (curve array) — 曲线数组（Gradations 专用）
- slot 0x40 (ushort array) — ushort 数组（Shadow/Gray/Highlight）

### 24.1 CImageCorrection 的 CString 字段完整列表

| 偏移 | 字段 | Case |
|------|------|------|
| 0x90 | CMYKProfile | 11 |
| 0x548 | InputProfile | 11 |
| 0x99c | RGBProfile | 11 |
| 0xdc0 | GrayProfile | 11 |

可能还有 SoftProofProfile、ColorCorr (0x170 的 DAT_70a5f170 映射 "ColorCorr"——此字段在 FUN_702d81f0 没出现，暗示它在 slot 0 read 路径或 slot 5 writer 未调用的分支)。

---

## 25. CImageCorrection 完整字段布局（汇总表）

从 ctor（FUN_702d4360）+ inner_ctor（FUN_702d5a20）+ writer（FUN_702d7590/81f0）+ XML 注册映射三角验证：

| 偏移 | 字段 | 类型 | 来源 |
|------|------|------|------|
| 0x00 | vftable | ptr | ctor |
| 0x14 | CGammaCurve (shared) | ptr | inner ctor |
| 0x18 | CContrastCurve (shared) | ptr | inner ctor |
| 0x1c | CGammaNegCurve R (per-ch) | ptr | inner ctor (loop) |
| 0x20 | CGammaNegCurve G | ptr | inner ctor |
| 0x24 | CGammaNegCurve B | ptr | inner ctor |
| **0x28** | Gradations[0] = Master CPointCurve | ptr | ctor + writer case 1 |
| **0x2c** | Gradations[1] = R UserA CPointCurve | ptr | ctor |
| **0x30** | Gradations[2] = G UserA CPointCurve | ptr | ctor |
| **0x34** | Gradations[3] = B UserA CPointCurve | ptr | ctor |
| **0x38** | Gradations[4] = R UserB CPointCurve | ptr | ctor |
| **0x3c** | Gradations[5] = G UserB CPointCurve | ptr | ctor |
| **0x40** | Gradations[6] = B UserB CPointCurve | ptr | ctor |
| 0x50 | CHighShadowCurve R (per-ch) | ptr | inner ctor |
| 0x54 | CHighShadowCurve G | ptr | inner ctor |
| 0x58 | CHighShadowCurve B | ptr | inner ctor |
| 0x74 | CNegativeCurve (shared, 158.7/100.6) | ptr | inner ctor |
| 0x78 | outer_agg R | ptr | inner ctor |
| 0x7c | outer_agg G | ptr | inner ctor |
| 0x80 | outer_agg B | ptr | inner ctor |
| 0x84 | inline CPointCurve (2,1)(100,1) | ptr | inner ctor |
| **0x88** | ApplyCC | bool | writer case 3 |
| **0x89** | ApplyUSM | bool | writer case 4 |
| **0x8a** | ApplyDust | bool | writer case 5 |
| **0x8d** | Brightness | int8 | writer case 0 |
| **0x90** | CMYKProfile | CString | writer case 11 |
| (0x90 + str_size) | - | - | 待测 |
| **0x4fc** | Saturation | int16 | writer case 3 |
| **0x4fe** | Contrast | int8 | writer case 0 |
| **0x518** | EnhancedShadow | bool | writer case 6 + CGammaNeg 读 |
| **0x51c** | FilmType | uint32 | writer case 12 + CGammaNeg 读 |
| **0x52c** | Gamma | float32 | writer case 0 |
| **0x536** | Gray | ushort[3] | writer case 2 |
| **0x53e** | Highlight | ushort[3] | writer case 2 |
| **0x548** | InputProfile | CString | writer case 11 |
| **0x96c** | Lightness | int16 | writer case 0 |
| **0x970** | Mode | int32 | writer case 10 |
| **0x99c** | RGBProfile | CString | writer case 11 |
| **0xdc0** | GrayProfile | CString | writer case 11 |
| **0x11e6** | Shadow | ushort[3] | writer case 2 |
| **0x11e8** | Shadow[1] (=slot 7 读点) | ushort | slot 7 交叉确认 |
| **0x11ee** | SoftProof | bool | writer case 11 |
| **0x11f0** | Threshold | int16 | writer case 1 |
| **0x11f2** | USMAmount | int16 | writer case 4 |
| **0x11f4** | USMColFactor | ushort[3] | writer case 4 |
| **0x11fa** | USMDarkLimit | int16 | writer case 4 |
| **0x11fc** | USMNoiseLimit | byte | writer case 4 |
| **0x11fe** | USMRadius | int16 | writer case 4 |
| **0x1200** | Convert | bool | writer case 11 |
| **0x1201** | EmbedProfile | bool | writer case 11 |
| **0x1214** | DustLevel | byte | writer case 5 |
| **0x1218** | ApplySliders | bool | writer case 0 |
| **0x1219** | ApplyCurves | bool | writer case 1 |
| **0x121a** | ApplyHistogram | bool | writer case 2 |
| **0x121c** | ColorNoiseRadius | int32 | writer case 6 |
| **0x1220** | ApplyCNFilter | bool | writer case 6 |
| **0x1224** | ColorTemperature | int32 | writer case 8 |
| **0x1238** | ColorModel | int32 | writer case 3 |
| **0x123c** | FilmCurve | int32 | writer case 1 |
| **0x1240** | NoiseFilterBias | int16 | writer case 5 |
| **0x1244** | ?? (DAT_70a5f608) | int32 | writer case 7 |
| **0x1248** | VignetteAmount | int32 | writer case 7 |

### 25.1 CString 大小测算

InputProfile 位于 0x548，RGBProfile 位于 0x99c，差 = 0x99c - 0x548 = 0x454 (1108 bytes)。这中间必然有其他字段。std::string 在 MSVC 2008 典型 24 字节 header + 最多 16 字节 SSO 或外部堆。如每 CString 占 ~32 字节，中间 1076 字节包含很多其他东西（例如 image metadata、per-channel curve caches）。

GrayProfile 位于 0xdc0，距 RGBProfile = 0xdc0 - 0x99c = 0x424 (1060 bytes)。相似跨度。

这说明 CImageCorrection 对象非常大（超过 4KB）。

---

## 26. slot 0 = CImageCorrection XML 读取器

FUN_702d5f70 是双向 XML 序列化的 READ 方向。按顺序读取，初始化默认值，然后被 XML 覆盖。

### 26.1 默认值

```c
this->ApplyHistogram = 1;  // 默认开
this->ApplySliders = 1;    // 默认开
this->ApplyCurves = 1;     // 默认开
// 其他字段默认值未显式设置（依赖 ctor 的 0 初始化）
```

### 26.2 Gradations 读取

对 this+0x28..0x40 的 7 个指针：
- 指针为 NULL → 调 `FUN_70265dd0(xml_reader)` 构造新 CPointCurve
- 已存在 → 调 existing curve's vtable slot 0 (XML reader method)

**推论**：CPointCurve 也有 XML reader（slot 0 via CCurveBase 基类继承）。所以曲线的 `<Points>` 数组由曲线对象自己解析。

### 26.3 6×6 ColorCorr 矩阵定位

36 shorts (72 bytes) 读入 this+0x4b4..0x4fb。正好在 Saturation (0x4fc) 之前。

XML key `<ColorCorr>` (DAT_70a5f170) 对应此矩阵。6×6 short 暗示 **6通道色彩校正**（可能 RGB + CMY 或 RGB + skin/sky/foliage 三组 memory colors）。典型的 FlexColor "Color Correction" 面板。

### 26.4 Shadow/Gray/Highlight 实为 4 分量（不是 3！）

XML 读取外层循环 `iVar6 = 4`，而非 3。所以 Shadow/Gray/Highlight 每个是 **4 个值**：

```c
puVar8 = this + 0x536;  // Gray 起始
for i in 0..4:
    // Shadow: byte from XML, <<6 scale to 14-bit (即 0..255 → 0..16320)
    *(ushort*)(this + 0x11e6 + 2*i) = xml_read_byte() << 6;
    
    // Gray: byte from XML, no scale (0..255)
    *(ushort*)(this + 0x536 + 2*i) = xml_read_byte();
    
    // Highlight: byte from XML, <<6 scale
    *(ushort*)(this + 0x53e + 2*i) = xml_read_byte() << 6;
```

所以：
- Shadow at **this+0x11e6** = ushort[4]（8 bytes）— XML 存 byte，内部 <<6 到 14-bit
- Gray at **this+0x536** = ushort[4]（8 bytes）— XML 存 byte，内部保留 0..255
- Highlight at **this+0x53e** = ushort[4]（8 bytes）— XML 存 byte，内部 <<6 到 14-bit

**4 分量推测**：RGB + K (alpha/达到)？或 RGB + gray luminance？XML 里具体 schema 待看：
```xml
<Shadow>
  <byte>0</byte>     <!-- R -->
  <byte>0</byte>     <!-- G -->
  <byte>0</byte>     <!-- B -->
  <byte>0</byte>     <!-- 第 4 个：K/Gray? -->
</Shadow>
```

**关键更正**：Section 22 假设 Shadow[1]=this+0x11e8 对应 slot 7 的读取，仍成立（iter 1 写 Shadow[1]=this+0x11e6+2=0x11e8）。但 Section 18.3 的 "ushort[3]" 应更正为 "ushort[4]"。

### 26.5 ApplyCC / USM 字段读取顺序

```c
this->ApplyCC = xml_read_bool();          // 0x88
this->USMAmount = xml_read_short();       // 0x11f2
this->USMDarkLimit = xml_read_short();    // 0x11fa
this->USMRadius = xml_read_short();       // 0x11fe
this->field_0x8c = xml_read_byte();        // 未识别字段！
this->field_0x8b = xml_read_byte();        // 未识别字段！
this->field_0x975 = xml_read_bool();       // 未识别！
this->field_0x974 = xml_read_bool();       // 未识别！
```

新发现 4 个未识别的字段（不在 slot 5 writer 的任何 case 里）。可能是：
- **this+0x8b, 0x8c**：紧邻 ApplyCC(0x88)/ApplyUSM(0x89)/ApplyDust(0x8a)/Brightness(0x8d) 之间的 2 字节。很可能是 Apply* 族的额外 bool（比如 ApplyInputProfile, ApplyOutputProfile 之类）
- **this+0x974, 0x975**：紧邻 Lightness(0x96c) 和 Mode(0x970) 区域。可能是另外的 flag

### 26.6 2×7 DotColor/EndPoints struct 

```c
puStack_248 = this + 0x510;  // 2 entries × uint
iStack_250 = this + 0x4ff;   // 2 entries × 7 bytes + padding
for (outer = 2; outer > 0; outer--):
    xml_read_byte(&stack);     // 1 byte 
    for (inner = 3; inner > 0; inner--):
        *puVar9++ = xml_read_byte();  // 3 bytes RGB
    xml_read_byte(&stack);     // 1 more byte
    iStack_250 += 7;
    *puStack_248++ = ...;      // 1 uint
```

2 × (1 + 3 + 1) = 10 bytes stored at this+0x4ff..0x512 per iter? Plus 2 × uint at this+0x510..0x517.

这读取 **DotColor 和 EndPoints**（XML keys 来自 Section 18.2）的 struct：
- 每个 5 bytes (flag + RGB + padding) + 1 uint (4 bytes) = 9 bytes
- 2 × 9 = 18 bytes at this+0x4ff..0x510 (approximately)

### 26.7 InputProfile 字符串 + "FlexTight Input" 默认比对

```c
this->Convert = xml_read_bool();   // 0x1200
// 读 null-terminated 字符串，最多 256 字符，存入 stack buffer
strncmp(stack_buffer, "FlexTight Input") 
// 根据比对结果设 flag
```

**"FlexTight Input" 是默认 InputProfile**（即 Flextight 输入色彩空间）。当 XML 指定了不同的 profile，会用 XML 里的路径。

### 26.8 slot 0 未读完的部分

函数共约 260 行，上面只读了约 140 行。后续还有：
- 剩余 USM 字段 (USMNoiseLimit, USMColFactor, ApplyUSM)
- Histogram (ApplyHistogram) — 已默认 true
- Saturation (0x4fc)、ColorModel (0x1238)、FilmCurve (0x123c)
- Lightness (0x96c), Mode (0x970)
- FilmType (0x51c), EnhancedShadow (0x518)
- ColorTemperature (0x1224), Tint
- ColorNoiseRadius (0x121c), ApplyCNFilter
- ApplyDust, DustLevel, NoiseFilterBias, VignetteAmount
- SoftProof, EmbedProfile, InputProfile/RGBProfile/CMYKProfile/GrayProfile 字符串
- 可能还有未暴露的隐藏字段（类似 0x8b/0x8c/0x974/0x975 这种）

---

## 27. 写/读 slot 语义汇总（CXMLReader vtable）

从 slot 0 和 slot 5 的调用对，总结 XML reader/writer 对象的 vtable slot 语义：

| slot | 用途 | writer | reader |
|------|------|--------|--------|
| 0x00 (`*param_1`) | 读单个 int8/byte + 出栈 | - | **param_1->()** 直接读 |
| 0x04 | 写/读 int16 / int32 | `write(key, int)` | - |
| 0x08 | 写/读 double | `write(key, double)` | `read(key, &double)` |
| 0x0c | 写/读 byte (bool) | `write(key, byte)` | `read(key, &byte)` |
| 0x18 | 写/读 CString | `write(key, CString*)` | `read(key, &CString)` |
| 0x40 | 写/读 ushort 数组 | `write(key, uint[], len)` | - |
| 0x5c | 写/读 curve 数组 | `write(key, curve[], 7)` | - |

### 27.1 为什么 slot 0 reader 看不到 key strings

slot 0 是 **流式** 读取（不 key-lookup），字段按固定顺序。而 slot 5 writer 走 FUN_702d81f0 case dispatch（按 version）写 key=value。两者都是同一 XML schema，只是 read 按顺序、write 按 key。

这意味着：
- **Reader 顺序决定字段解析顺序，和 XML 里实际字段顺序解耦**（plist 是 dict，顺序无所谓）
- 猜测：XML reader 对象是 dict-style，按 key 查询；param_1 vtable slot 可能带 key 参数，只是被 decompiler 优化掉了

需进一步分析 param_1 类型（应是 CPListReader 之类）才能完全搞清。

---

## 28. CHighShadowCurve::set_params — 揭示 Histogram 3-point 完整机制

slot 14 链：`FUN_70268060` → `FUN_70268080` (实体 set_params)。

### 28.1 字段

CHighShadowCurve 实例布局（0x34 字节）：

| offset | 类型 | 含义 |
|--------|------|------|
| 0x00 | vftable | CHighShadowCurve::vftable |
| 0x0c | heap ptr | LUT 缓存（destructor free 它）|
| 0x10 | ptr | **parent** (CImageCorrection *) |
| 0x14 | int | **channel index** (1=R, 2=G, 3=B) |
| 0x18 | ptr | **inner_agg** (CAggregateCurve *) |
| 0x1c | ushort | shadow_boundary (14-bit)，输入阈值 |
| 0x1e | ushort | highlight_boundary (14-bit) |
| 0x20 | ushort | shadow_out (14-bit)，shadow 区输出值 |
| 0x22 | ushort | highlight_out (14-bit) |
| 0x24 | float | mid_scale = 16383 / (hi_bnd - sh_bnd) |
| 0x28 | float | mid_add_scale = (hi_out - sh_out) / 16383 |
| 0x2c | int | shadow_mode (0=const / 1=linear / 2=zero) |
| 0x30 | int | highlight_mode (0=const / 1=linear / 2=max) |

### 28.2 set_params 逻辑

```c
void CHighShadowCurve::set_params():
    parent = this.parent
    ch = this.channel  # 1, 2, 3
    
    if (parent.ApplyHistogram):
        # 从 CImageCorrection 读 per-channel 参数
        this.shadow_out     = parent.byte[0x4fe + ch] * 16383 / 255   # EndPoints shadow RGB (byte)
        this.highlight_out  = parent.byte[0x505 + ch] * 16383 / 255   # EndPoints hi RGB (byte)
        this.shadow_boundary    = parent.ushort[0x11e6 + ch*2]        # Shadow XML RGB (14-bit)
        this.highlight_boundary = parent.ushort[0x53e + ch*2]         # Highlight XML RGB (14-bit)
        this.mid_scale      = 16383.0 / (hi_bnd - sh_bnd)
        this.mid_add_scale  = (hi_out - sh_out) / 16383.0
        this.shadow_mode    = parent.int[0x510]     # EndPoints shadow_mode (shared)
        this.highlight_mode = parent.int[0x514]     # EndPoints hi_mode
    else:
        # 默认 identity
        this.shadow_out = 0;    this.highlight_out = 16383;
        this.shadow_boundary = 0; this.highlight_boundary = 16383;
        this.mid_scale = 1.0;    this.mid_add_scale = 1.0;
        this.shadow_mode = 0;    this.highlight_mode = 0;
```

然后调 `inner_agg.build_lut()` (slot 14 = slot 0x38) 递归准备内层 LUT。

### 28.3 交叉确认：Shadow/Highlight 字段布局

结合 §26.4 (slot 0 reader) + §28.2 (consumer)：

| 字段 | XML Key | 偏移 | 类型 | 通道编码 |
|------|---------|------|------|----------|
| Shadow | `<Shadow>` | this+0x11e6 | ushort[4] | XML 存 byte，<<6 载入为 14-bit；ch=1,2,3 取第 2/3/4 个（不是第 1 个，因为 `+ ch*2`） |
| Highlight | `<Highlight>` | this+0x53e | ushort[4] | 同上 |
| Gray | `<Gray>` | this+0x536 | ushort[4] | XML 存 byte，保留 0..255（不左移）|
| EndPoints shadow | `<EndPoints>` 的 shadow 子项 | this+0x4fe..501 | byte[4]（第 0 字节是 Contrast）| 0x4ff/0x500/0x501 = R/G/B shadow_out |
| EndPoints highlight | 同上 hi 子项 | this+0x505..508 | byte[4] | 0x506/0x507/0x508 = R/G/B hi_out |
| EndPoints shadow_mode | 同上 | this+0x510 | int32 | 0/1/2 |
| EndPoints highlight_mode | 同上 | this+0x514 | int32 | 0/1/2 |

**关键发现**：
- **Shadow/Highlight 是 boundary (14-bit)，EndPoints 是 output values (byte 0-255)**。概念上独立：boundary 定义"Histogram 三区的边界在哪里"，endpoints 定义"三区输出值是什么"
- **第 0 分量（ch=0）不用**。可能是保留给 luma/Gray 使用，或是历史兼容
- **shadow_mode / highlight_mode 是全局（不分通道）**

### 28.4 修正 §25 字段表

- Shadow at this+0x11e6 = **ushort[4]** (Section 26 已更正)
- Highlight at this+0x53e = **ushort[4]**
- Gray at this+0x536 = **ushort[4]**（byte 不左移）
- **EndPoints struct**（XML key DAT_70a5f288）实际位于 this+0x4ff..0x517（约 18 bytes）：
  - 0x4ff/0x500/0x501: shadow R/G/B (byte)
  - 0x506/0x507/0x508: highlight R/G/B (byte)  
  - 0x510: shadow_mode (int)
  - 0x514: highlight_mode (int)
  - 0x4ff 之前的 0x4fe 是 Contrast（独立字段）
  - 0x502..0x505, 0x509..0x50f: 填充或其他字段

### 28.5 CHighShadowCurve::compute_single 完整语义（结合 set_params）

```c
ushort compute(ushort x):
    if (x < shadow_boundary):  # 来自 XML Shadow RGB[channel]（<<6 后值）
        if (shadow_mode == 0): return shadow_out   # 常数输出（来自 EndPoints.shadow[ch] × 16383/255）
        if (shadow_mode == 1): return shadow_out * x / shadow_boundary  # 0→shadow_out 线性
        if (shadow_mode == 2): return 0  # 黑

    elif (x < highlight_boundary):  # 来自 XML Highlight RGB[channel]
        # 中间区：应用 inner_curve（完整曲线链）
        scaled_x = round((x - shadow_boundary) * mid_scale)  # 映射 [sh_bnd, hi_bnd] → [0, 16383]
        y = inner_curve.compute_single(scaled_x)
        return round(shadow_out + y * mid_add_scale)  # 输出在 [shadow_out, highlight_out] 间

    else:  # x >= highlight_boundary
        if (highlight_mode == 0): return highlight_out   # 常数输出
        if (highlight_mode == 1): return (x - hi_bnd) * (16383 - hi_out) / (16384 - hi_bnd) + hi_out  # hi_out→16383 线性
        if (highlight_mode == 2): return 16383  # 白
```

这是 FlexColor **Histogram 三点映射**的完整实现。Shadow/Highlight 边界由用户通过 "Set Shadow Point / Highlight Point" 工具设定（通常从图像点击取值，自动写入 XML Shadow/Highlight 数组）。EndPoints 是输出目标（通常固定为 0/255 或 black point / white point）。

### 28.6 下一步

1. **EndPoints 第 0 字节和 0x502..0x505 填充区** — slot 0 reader 里具体如何读？
2. **inner_agg 的 apply_array (slot 14? FUN_70268060 调了 `(**(code **)(**inner_agg + 0x38))()`)** — slot 14 = 构造 inner LUT？这是递归准备 LUT 的链条。
3. **CHighShadowCurve 是否还有单独的 build_lut** — slot 7/8 的 inherited 版本足够？
4. **CHighShadowCurve.field_0x0c heap buffer** — 可能存着 14-bit LUT cache，每次 set_params 重建

---

## 29. FlexColor RTTI 类目录（关键类，2026-04-19 新工具）

### 29.1 新工具 list-rtti-classes

`tools/ghidra_query/scripts/list_rtti_classes.py` 扫 `.data/.rdata` 段的 `.?AV<ClassName>@@` MSVC type descriptor 字符串，直接列出所有 C++ 类名（不依赖 Ghidra 的 namespace 解析）。

### 29.2 Pipeline 相关类（从 RTTI 筛出）

| 类名 | vtable | 推测用途 |
|------|--------|---------|
| **CImageConverter** | 0x7071a83c | ★ 顶层 pipeline 基类（线程化 message loop）|
| **CImageConverterOffscreen** | 0x7071a854 | ★ offscreen rect 处理变体 |
| CImageConverterOffscreenPC | — | PC 平台特化 |
| **CColorCorrection** | 0x7071d83c | ★ 6×6 matrix + 2 × 64K LUT 的色彩校正实体 |
| **CColorTempConversion** | 0x70719248 | ★ 色温/白平衡转换 |
| **CCachedColorWorld** | — | 色空间 cache |
| **CColorWorld** | — | 色空间 base class（仅析构函数，抽象）|
| **CColorManager** | — | 色彩管理总控 |
| **CICMColorManager** | — | Windows ICM API wrapper |
| CFullImageReader | — | FFF 文件读取 |
| CFullImageReaderPC | — | PC 平台特化 |
| CImageData / CImageBuffer | — | 图像数据缓冲区 |
| CCameraPixelCorrection | — | 相机校正（暗电流/偏置等）|
| CCorrectChromaticAberration | — | CA 校正 |
| CColumnCorrection / CLineCorrection | — | 列/线 defect 修复 |
| CFilmDetector | — | 自动胶片类型检测 |
| CFilmRepairDust / Dust2 / Scratch / Hole / Damage | — | 除尘/划痕/孔洞 |
| CMosaicImage | — | Bayer 去马赛克？|
| CGenImage | — | 通用图像对象 |
| CImageCorrection | 0x7071aaec | ★ 调色设置（curve + slider + Shadow/Highlight 参数）|
| CImageSetting | — | 单帧设置（含 CImageCorrection + 其他）|
| CImageDescription | — | 图像元信息 |

### 29.3 CColorCorrection 内部结构（从 ctor 读出）

```
offset 0x00: vftable
offset 0x04..0x108: 6×6 double 矩阵（36 doubles = 288 bytes）
  - 部分对角线设为 1.0 (0x3ff0000000000000)，对应 identity
  - 约 3 个 off-diagonal sets (在 +9, +0x11 被置 1.0，可能是次对角)
offset 0x13c..0x140: 8 bytes 从 DAT_70997928+0x1c8/0x1d0 初始化（外部配置指针）
offset 0x100: field_0x52 = 0
offset 0x100: field_0x40 = 0
offset 0x104: field_0x41 double = 1.0
offset 0x10c [0x43]: LUT_in ptr = operator_new(0x20000)  ; 64K ushort，初始 identity
offset 0x110 [0x44]: LUT_out ptr = operator_new(0x20000) ; 64K ushort，初始 identity
```

**关键**：CColorCorrection 工作在 **16-bit 域**（LUT 有 65536 条），比 CImageCorrection 的 14-bit curves 精度更高。这可能意味着 FlexColor 的 pipeline 是：
1. 14-bit curves (CImageCorrection 的 Gradation chain)
2. 升 16-bit
3. 16-bit ICC / color correction (CColorCorrection)
4. 16-bit USM
5. 16-bit → 8/16 输出 via output profile

### 29.4 CImageConverter 线程架构

slot 1（FUN_702d34c0）= 主循环：
```c
do {
    msg = param_1[5]->get_message(...)
    if (msg.type == 1): param_1[6]->process(msg)  // 处理一行像素
    else if (msg.type == 2): {                      // 退出
        this->exit_flag = 1
        param_1[6]->process(msg)
    }
    else if (msg.type == 3): (**this + 0x10)()   // ConvertRow (slot 4)
} while (!this->exit_flag)
```

这是**生产者-消费者**架构：扫描硬件产生行，转换线程消费。param_1[6] 很可能是 CImageCorrection 的 apply wrapper。

### 29.5 CImageConverterOffscreen::slot 4 = 处理矩形

FUN_702d3cf0 接收矩形参数 (param_3..param_6 = x1/y1/x2/y2)，然后：
1. `FUN_703cf560` 构造临时 CXRect（矩形对象）
2. 调 `this->v_slot_8(local_28)` 虚拟函数（offscreen 子类的 init 步骤）
3. `FUN_702e1b20(...)` 看起来接 rect + 几个 ptr，可能是 **pipeline 前置 setup**
4. `FUN_7031ad90(...)` 传入 `this_00 = iStack_78 - iVar2`（可能是 row buffer 指针）+ 尺寸 + count

FUN_7031ad90 很可能就是**主像素处理内层**。下次可深挖。

### 29.6 CColorTempConversion xref

4 个构造者：
- 0x70596d00, 0x705973e0, 0x70597350 — 在 .text 段 0x705xxxx，属 UI/dialog 模块
- 0x70260320 — 在 0x7026xxxx 段（curve 模块），**值得深挖**

后者很可能是 pipeline 的 WB 步骤。`ColorTemperature` (this+0x1224 int) 和 `Tint` (相邻) 字段驱动这里。

---

## 30. 下一步汇总（2026-04-19 睡前记录）

### 30.1 已有（可立即开始 Rust 复刻）

1. 完整 pipeline 拓扑（§16.5, §25）
2. 全部曲线公式（§13, §18.7 勘误版）
3. XML ↔ CImageCorrection 字段映射（§18.3 + §23-§26 补完）
4. CHighShadowCurve Histogram 3-point 机制（§28）
5. B-spline basis（§16.9）
6. Default 常量全（§18.8, §19）
7. CAggregateCurve mode 1/2 真实语义（§16.4 勘误版）

### 30.2 仍需挖（按优先级）

| 优先 | 任务 | 入口 | 估计 |
|-----|------|------|------|
| 🔥 | **FUN_7031ad90 主像素处理** — 可能是 outer_agg LUT × 像素的核心 | 0x7031ad90 | 1-2 h |
| 🔥 | **FUN_702e1b20 pipeline setup** — 触发 CHighShadow/CGammaNeg 的 set_params 和 build_lut | 0x702e1b20 | 1-2 h |
| 🔥 | **FUN_70260320 CColorTempConversion curve-side 调用** — ColorTemperature/Tint 白平衡算法 | 0x70260320 | 1 h |
| 🎯 | **CColorCorrection::build_transform** — 6×6 matrix × ICC profile 组合路径 | 非虚函数，需搜 | 2 h |
| 🎯 | **USM Apply**（USMRadius/USMAmount 的真实使用路径） | 搜 this+0x11f2 读者 | 1 h |
| 💡 | FilmCurve preset 数组（胶片类型 → gamma preset 映射）| 搜 this+0x123c 读者 | 1 h |
| 💡 | Lightness/Saturation 算法（this+0x96c / this+0x4fc 读者）| 搜 | 1 h |
| 🔧 | inner_agg slot 14 (recurse_build_lut) —set_params 链条 | CAggregateCurve vtable slot 14 FUN_70268730 | 0.5 h |
| 🔧 | Case 11 EndPoints/DotColor 字段 0x502..0x50f 细节 | slot 0 XML reader tail | 0.5 h |
| 🔧 | ColorModel enum values（RGB/CMYK/Gray）| 搜 this+0x1238 reader | 0.5 h |

### 30.3 审查 agent 的 4 个必修问题 — 已修

- ✅ E1 CGammaNeg 公式 `1 − v²` (§18.7, §19.2)
- ✅ E2 mode 1/2 child 输入语义 (§16.4)
- ✅ E3 CNegativeCurve x/y 方向 (§15, §16.5)
- ✅ E4 case 11/12 字段补完（§23-§26，§18.3 表未回头补，但 §25 汇总已全）

### 30.4 工具状态

- ✅ `./run.sh decompile|vtable|disasm|read-const|find-str-xrefs` — 单 addr / 类查询
- ✅ `./run.sh dump-xml-registry` — 一键扫 825 个 XML key thunk
- ✅ `./run.sh list-rtti-classes [prefix]` — 扫 RTTI class 名
- ⚠️ Ghidra project lock — 单实例，不可并发。Agent 审查时暴露此限制

### 30.5 Ghidra 知识库
项目状态保存在 `/Users/will/Projects/Ghidra-Projects/FlexColor-RE/`。RTTI 已运行（run-rtti 803 classes 已恢复 namespace）。

---

## 31. CImageCorrection ctor 完整展开 — ICC profile 子对象确认

### 31.1 4 个 CFileSpec 子对象

从 FUN_702d4360 ctor 尾部：
```c
param_1[0x24] = CFileSpec::vftable;    // offset 0x24*4 = 0x90  → CMYKProfile 位置
param_1[0x152] = CFileSpec::vftable;   // offset 0x152*4 = 0x548 → InputProfile
param_1[0x267] = CFileSpec::vftable;   // offset 0x267*4 = 0x99c → RGBProfile
param_1[0x370] = CFileSpec::vftable;   // offset 0x370*4 = 0xdc0 → GrayProfile
```

**完全印证了 §25 的 CString 偏移推断**！每个 profile 其实不是 `std::string`，而是 MFC 的 `CFileSpec` 对象（从 FlexColor SDK 继承的 file path 类）。大小看 ctor：相邻实例差 0xbe = 190 字节。CFileSpec 实例布局：
- offset 0x00: vftable
- offset 0x04: path string (CString 内嵌？)
- offset 0x98 或附近: flag / length 等元数据

每个 profile 占 `0x370 - 0x267 = 0x109 * 4 = 0x424 = 1060` 字节？但 `0x267 - 0x152 = 0x115 * 4 = 0x454 = 1108`。差距略不同，说明每个 CFileSpec 后跟不同 padding/其他字段。

### 31.2 此前疑问字段确认

| offset | 字段 | 内容 |
|--------|------|------|
| 0x1224 | ColorTemperature | int32 |
| 0x1228 | (推测 Tint) | int32 - ctor 置 0 |
| 0x1230 | (推测 EV) | double - ctor 置 1.0 (0x3ff0000000000000) |
| 0x120c | ? | ptr → operator_new(0x4010 = 16400 bytes) |
| 0x1210 | ? | = 0x2008 = 8200 |
| 0x121b | ? | byte = 0 |

其中 0x120c 的 16KB 缓冲区可能是**直方图缓冲**（4 通道 × 4K bins × 2 bytes = 32KB 不对；或 3 通道 × 2K ushort × 2 = 12KB；或 1 × 16K × 1 = 16KB = **单通道 14-bit 直方图**）。

### 31.3 ctor 内的 3-iteration 循环（可能 per-channel 历史/cache）

```c
iVar4 = 3;  // R, G, B
puVar2 = param_1 + 0x1e;
do {
    puVar2[-0x17] = 0;  // offset 0x1e+0x1e*(-0x17) = +0x1c → this+0x1c+... actually offset 0x1e-0x17=0x07, *4=0x1c = per-channel CGammaNeg 指针
    *puVar2 = 0;
    puVar2[-4] = 0;
    puVar2[-10] = 0;
    puVar2[0x12a] = 0;
    puVar2[-7] = 0;
    puVar2[-0x1c] = 0;
    puVar2[-0xd] = 0;
    puVar2 = puVar2 + 1;
    iVar4 = iVar4 + -1;
} while (iVar4 != 0);
```

这段初始化 8 个 per-channel 指针 × 3 通道 = 24 个指针字段。匹配我们 §25 已知的 CGammaNeg (0x1c/0x20/0x24)、CHighShadowCurve (0x50/0x54/0x58)、outer_agg (0x78/0x7c/0x80)，以及其他未知指针。

### 31.4 初始化调用

ctor 末尾调 `FUN_702d67b0()` + `FUN_702d6700()`，分别可能是：
- FUN_702d67b0 — 初始化某 per-channel 数据
- FUN_702d6700 — 初始化 histogram / EV 默认值

内部 FUN_702d5a20 (主 pipeline 构造) 只在**存在时**由 copy ctor 或专门初始化调用。普通 ctor 到此刚好初始化 CFileSpec profiles、7 Gradations、指针清零，并没有立即创建 CGammaNeg/CHighShadow/等。

### 31.5 结论

- **ICC profile 是 CFileSpec 对象**，不是简单字符串 — 访问 path 需走 CFileSpec 的方法（可能 GetPath / operator= 等）
- **ColorTemperature/Tint/EV** 布局紧凑（0x1224 / 0x1228 / 0x1230）
- **16KB 缓冲 @ 0x120c** 可能是直方图数据
- 复刻时需处理 CFileSpec 路径读取（或直接用 ICC 文件路径字符串）

---

## 32. CImageCorrection::InitDefaults — 所有字段默认值表

FUN_702d67b0（在 ctor 末尾调用）设定每个字段的初始值。复刻时从这里拷贝默认值即可，不必每次解析 XML。

### 32.1 默认值表

| 偏移 | 字段 | 默认值 | 备注 |
|------|------|--------|------|
| 0x88 | ApplyCC | **1 (true)** | |
| 0x89 | ApplyUSM | **1 (true)** | |
| 0x8a | ApplyDust | ctor 零 | InitDefaults 未改 |
| 0x8b | AutoHighlight | **0** | |
| 0x8c | AutoShadow | **0** | |
| 0x8d | Brightness | ctor 零 | |
| 0x4b4..0x4fb | ColorCorr matrix (6×6 short) | **全 0** | FUN_702d6630 显式清零 |
| 0x4fc | Saturation | **0** | |
| 0x4ff/0x500/0x501 | EndPoints shadow RGB | **(10, 10, 10)** | byte 0..255 |
| 0x502..0x505 | EndPoints shadow padding/flag | **0x0a0a0a0a** (all 0x0a) | |
| 0x506/0x507/0x508 | EndPoints highlight RGB | **(255, 255, 255)** | |
| 0x509..0x50d | EndPoints highlight padding | **0xffffffff** (all 0xff) | |
| 0x510 | shadow_mode | **1** | 线性 |
| 0x514 | highlight_mode | **1** | 线性 |
| 0x518 | EnhancedShadow | **0** | |
| 0x51c | **FilmType** | **0** | ★ 默认关闭胶片反转！|
| 0x52c | Gamma | ctor 零 (0.0f) | 异常：文档推测 1.0，实为 0 |
| 0x530/0x531/0x532 | per-channel ColorCorr flag? | 0 | |
| 0x536..0x53d | Gray ushort[4] | ctor 零 | |
| 0x53e..0x545 | Highlight ushort[4] | ctor 零 | |
| 0x558 | InputProfile path (内嵌到 CFileSpec) | **".dp:"** | 默认 profile 标识 |
| 0x96c | Lightness | ctor 零 | |
| 0x970 | Mode | **0** | |
| 0x9ac | RGBProfile path | **".dfR:"** | 默认 RGB profile 标识 |
| 0xa0 | CMYKProfile path | **".dfC:"** | 默认 CMYK profile 标识 |
| 0xdd0 | GrayProfile path | **".dfG:"** | 默认 Gray profile 标识 |
| 0x11e6..0x11ed | Shadow ushort[4] | ctor 零 | |
| 0x11ee | SoftProof | **0** | |
| 0x11f0 | Threshold | ctor 零 | |
| 0x11f2 | **USMAmount** | **0** | ★ 默认无锐化 |
| 0x11f4/f6/f8 | USMColFactor ushort[3] | **(100, 100, 100)** | |
| 0x11fa | USMDarkLimit | **10** | |
| 0x11fc | USMNoiseLimit | **0** | |
| 0x11fe | USMRadius | **10** | |
| 0x1200 | Convert | **1 (true)** | |
| 0x1201 | EmbedProfile | **0** | |
| 0x1202 | ? | 0x32 (50) | 内部字段 |
| 0x1204 | ? | 0x32 (50) | |
| 0x1206 | ? | 0x32 (50) | |
| 0x1208 | ? | 1 | |
| 0x121c | ColorNoiseRadius | **0** | |
| 0x1218 | ApplySliders | **1 (true)** | |
| 0x1219 | ApplyCurves | **1 (true)** | |
| 0x121a | ApplyHistogram | **1 (true)** | |
| 0x1220 | ApplyCNFilter | **1 (true)** | |
| 0x1224 | ColorTemperature | ctor 零 | |
| 0x1228 | (Tint 推测) | ctor 零 | |
| 0x1230 | (EV 推测) | **1.0** (double) | |
| 0x1238 | ColorModel | **0** (RGB) | |
| 0x123c | FilmCurve | ctor 零 | |
| 0x1240 | NoiseFilterBias | ctor 零 | |
| 0x1244 | ? (case 7 第一字段) | **7** | |
| 0x1248 | VignetteAmount | **100** | |

### 32.2 关键默认值结论

1. **FilmType = 0** 默认 — 对应 `<FilmType>0</FilmType>`，**C-41 负片处理默认关闭**。需要 XML 显式设 FilmType != 0 才触发 CGammaNegCurve。匹配我们测试数据"positive"/"RGB standard" case 走正片路径的观察。

2. **所有 Apply* 默认 true**：ApplyCC, ApplyUSM, ApplySliders, ApplyCurves, ApplyHistogram, ApplyCNFilter, Convert 全 1。**ApplyDust 例外**（默认 0）。

3. **USMAmount = 0** 默认 — 默认锐化量为零，即使 ApplyUSM=1 也不会改变图像。

4. **Profile 默认是标识符而非路径**：`.dp:` / `.dfR:` / `.dfC:` / `.dfG:` — 这些是 FlexColor 内部 profile identifier，运行时映射到**内置 ICC profile**。复刻时需要决定用 Flextight Input/sRGB IEC61966-2.1/ProPhoto RGB 还是其他。

5. **EndPoints 默认 (10,10,10)-(255,255,255), mode=1**：意味着默认 histogram 映射为：
   - 输入 [0, Shadow_boundary) → 线性到 `shadow_out=10×16383/255 ≈ 642`
   - 输入 [Shadow_boundary, Highlight_boundary] → 走 inner_curve
   - 输入 [Highlight_boundary, max] → 线性到 `highlight_out=255×16383/255 = 16383`
   - 但因为 Shadow (0x11e6) 和 Highlight (0x53e) 默认 0，实际触发需要 XML 提供边界。

6. **ColorCorr 6×6 matrix 默认全 0** — 不是 identity！这意味着：
   - 若 XML 不提供 `<ColorCorr>`，矩阵全 0 → 颜色变换输出 0（黑）
   - ApplyCC=true 但 XML 无 matrix 会 broken
   - 推测：matrix 全 0 时 ApplyCC 自动跳过；或 XML 必须提供，否则 ApplyCC 失效
   - 复刻时：ApplyCC=true 且 matrix 全 0 → treat as identity/skip

### 32.3 FUN_702d6630 额外初始化

同时设：
- `this+0x44/0x48/0x4c`（3 个指针）指向的对象的 `field_0x20 = 128.0`, `field_0x24 = 127.0`
  - 这 3 个对象身份未知，可能是另一组 per-channel CSinglePointCurve
  - 128/127 接近中点，典型 "neutral" 默认

### 32.4 CImageCorrection 里还有 2 个（未知）per-channel 对象组

- this+0x44/0x48/0x4c（3 指针）—— 对象类型未知，field_0x20 是 128.0，field_0x24 是 127.0
- 已知的 CGammaNeg (0x1c/20/24)、CHighShadow (0x50/54/58)、outer_agg (0x78/7c/80)、CSinglePointCurve (0x68/6c/70 从 §15)
- 共 5 组 per-channel 数组（15 个指针）

### 32.5 Profile 默认标识符解读

| 标识 | 对应 | 推测含义 |
|------|------|---------|
| `.dp:` | InputProfile | default profile — Flextight Input? |
| `.dfR:` | RGBProfile | default RGB — sRGB/Adobe RGB? |
| `.dfC:` | CMYKProfile | default CMYK — SWOP? |
| `.dfG:` | GrayProfile | default Gray — Gray Gamma 2.2? |

FlexColor 运行时解析这些标识符到实际 profile。Windows ICM 层可能有查询表。复刻时：
- 需要实测每个标识符对应哪个具体 ICC profile（可能在 `Misc/` 目录找到）
- 或手动指定等价 profile（如 sRGB for `.dfR:`）

---

## 33. 测试验证策略（MVP 路线）

基于 `examples/test_cases.toml` 的 29 个 case，设计**分层 MVP 验证**，确保每个 pipeline 阶段可独立验证、bug 定位精确。

### 33.1 分层优先级

**Tier 0 · Baseline Identity（首个要调通）**
- 目标：pure sRGB ICC + ApplySliders=1（但所有 slider = 默认）+ ApplyCurves=0 → 应输出近 identity
- 已有 case 近似：`emb_rgb_standard` / `ext_rgb_standard`
- 验证要素：ICC 装载、CImageCorrection field 解析、CPointCurve 空曲线（直线）

**Tier 1 · 单 slider 隔离**（每个独立调通）
| 特性 | 最佳 case | 验证算法 |
|------|----------|---------|
| Contrast | `ext_rgb_saturated` 或自构造 XML 仅 `Contrast=20` | §13 CContrastCurve 公式（pivot=8192，C=contrast/50） |
| Brightness | 自构造 XML 仅 `Brightness=20` | §13 CContrastCurve 公式的 B 项 |
| Gamma | 自构造 XML 仅 `Gamma=1.5` / `Gamma=2.5` | §13 CGammaCurve G≥2 / G<2 两路 |
| Lightness | `emb_rgb_dark` | **T16 待查**（this+0x96c 读者未定位） |
| Saturation | `emb_rgb_saturated` | **T16 待查** |

**Tier 2 · 曲线隔离**
| 特性 | 最佳 case | 验证算法 |
|------|----------|---------|
| ApplyCurves (用户 Gradation) | 自构造 `<Gradations>` 只改 Master[0] | §16.9 Cox-de Boor B-spline + §16.5 inner_agg[1] |
| ApplyHistogram (Shadow/Hi/EP) | 自构造 `<Shadow>`/`<Highlight>` 只调 R 通道 | §28 CHighShadowCurve 3-zone |

**Tier 3 · 负片管线**
| 特性 | 最佳 case | 验证算法 |
|------|----------|---------|
| CGammaNegCurve + CNegativeCurve | `emb_neg_rgb_standard` | §18.7 `pow(1-v², 1/γ)` + §15 CNegativeCurve 2 段二次 |
| Per-channel neg 微调 | `emb_neg_rgb_saturated` | §15 R/G/B 不同 x/y 参数 |
| B&W 负片 | `emb_bw_neg_standard` | Mode=BW + 灰色 profile |

**Tier 4 · Output profile 分支**
| 特性 | 最佳 case | 验证算法 |
|------|----------|---------|
| CMYK 输出 | `emb_cmyk_standard` | ColorModel=1 + CMYKProfile 应用 |
| Dark + CMYK 组合 | `emb_cmyk_dark` | Lightness + CMYK |
| Dark saturated + CMYK | `emb_cmyk_dark_saturated` | 三 slider 组合，最复杂 |

**Tier 5 · 端到端完整**
- `e2e_default` — FFF 自带默认 ImageCorrection
- `e2e_all_config` — FFF 自带含所有特性激活
- `e2e_all_config_bw` — B&W 版

### 33.2 验证执行顺序

```
Tier 0 (Identity)
    │ PASS → 证明 "ICC + 空管线 + 字段解析"
    ▼
Tier 1 (单 slider)
    │ 每项独立 PASS → 每个 slider 公式正确
    │  └─ FAIL 某项 → 精确定位到 Contrast / Gamma / Lightness / Saturation 某个
    ▼
Tier 2 (曲线)
    │ PASS → Cox-de Boor / 3-zone 实现正确
    ▼
Tier 3 (负片)
    │ PASS → CGammaNeg + CNegativeCurve + per-channel neg 链条正确
    ▼
Tier 4 (输出)
    │ PASS → ICC output profile 装载正确
    ▼
Tier 5 (e2e)
    │ PASS → 完整 pipeline 正确
```

### 33.3 差分测试（隔离 bug 位置）

当某 case FAIL 时，用以下对比定位问题：

| 对比 A vs B | 差异 | 定位 |
|------------|------|------|
| `emb_rgb_standard` vs `emb_rgb_saturated` | 只 Saturation 值不同 | Saturation 算法 |
| `emb_rgb_standard` vs `emb_rgb_dark` | 只 Lightness 值不同 | Lightness 算法 |
| `emb_rgb_standard` vs `emb_neg_rgb_standard` | FilmType 0→非0 | CGammaNeg 分支 |
| `emb_rgb_*` vs `emb_cmyk_*` | ColorModel / Output profile | ICC 出路 |
| `emb_rgb_saturated` vs `ext_rgb_saturated` | 同效果不同 XML 来源 | XML 解析（内嵌 vs 外部）|

### 33.4 自构造 minimal XML

`ext_*` 用的 `Standard/*.xml` 是 FlexColor 发布预设，含**所有字段**。为精准隔离算法，建议额外生成 **minimal preset**（只动一个字段）：

```xml
<!-- test_minimal_contrast_20.xml -->
<ImageCorrection>
  <streamableVersion>11</streamableVersion>
  <ApplySliders>true</ApplySliders>  <!-- 只启用 sliders -->
  <ApplyCurves>false</ApplyCurves>   <!-- 关闭用户曲线 -->
  <ApplyHistogram>false</ApplyHistogram>  <!-- 关闭直方图 -->
  <ApplyCC>false</ApplyCC>           <!-- 关闭 6×6 矩阵 -->
  <ApplyUSM>false</ApplyUSM>         <!-- 关闭锐化 -->
  <Contrast>20</Contrast>            <!-- 唯一变化 -->
  <Brightness>0</Brightness>
  <Gamma>2.0</Gamma>                 <!-- identity -->
  <Lightness>0</Lightness>
  <Saturation>0</Saturation>
  <FilmType>0</FilmType>             <!-- 正片 -->
  <!-- ... 其他字段用默认 -->
</ImageCorrection>
```

这样 FAIL 的唯一可能就是 Contrast 算法本身。

### 33.5 数据收集建议

跑 minimal XML 时额外保存：
1. **中间 LUT**（outer_agg 构建出的 14-bit 表）— 若我们能独立计算 LUT 并与 FlexColor 输出"同 input, 同 LUT"验证，隔离掉 pixel-apply 阶段
2. **几个关键像素的 trace**（原 RGB → 每阶段后的值）— 定位哪个阶段偏离
3. **参数 dump**（CImageCorrection 所有字段的实际值）— 确认 XML 解析正确

`tif_compare` 工具现有 `trace_pixel` / 指标功能可复用（见 §现有代码）。

### 33.6 已知陷阱

1. **FFF embedded history 的 preset** 可能与 **external XML** 的同名 preset 内容**不完全相同**（FlexColor 保存历史时会快照当前状态）。要确认 `emb_rgb_standard` ↔ `ext_rgb_standard` 的 XML 是 byte-identical，否则会被误判为"两路有 bug"
2. **16-bit ref.tif 取值域**：FlexColor 输出 14-bit 后通常 ×4 升到 16-bit；若 ref 是 8-bit TIF 则需 ÷256 回推。`tif_compare` 应自动处理
3. **ICC profile 差异**：即使算法完全一致，lcms2 vs Windows ICM 在 PCS 量化上微差可能出现 1-2 LSB。STRICT 级验证时用 WARN band 容忍
4. **ApplySliders=true 默认**：若 XML 不写 `<ApplySliders>`，默认 true（§32）— minimal XML 必须显式写 `false` 才能关闭

### 33.7 验证进度跟踪

建议 `tif_compare` 输出增加"分层标签"：
```
[T0 Identity]        ext_rgb_standard        STRICT 2/65536
[T1 Contrast]        ext_rgb_dark            WARN  ...
[T1 Saturation]      ext_rgb_saturated       FAIL  ...
[T3 Neg]             ext_neg_rgb_standard    FAIL  ...
```

便于一眼看出"Tier 0 PASS 后 Tier 1 的 Saturation 卡住" → 精力集中到该公式。

---

## 34. Rust 实现设计（T10 路线图）

基于 §16.5 / §25 / §32 的完整理解，设计 Rust 复刻的模块结构、类型 API、集成点。**纯设计文档，不写代码**。

### 34.1 模块拓扑

现有 `src/color/` 结构：
```
src/color/
├── mod.rs          公共 re-export
├── profile.rs      ICC profile 扫描
├── transform.rs    lcms2 ICC transform
├── processing.rs   当前 apply_color_pipeline_ex（需重构）
├── adjust.rs       ManualAdjust + scanner_levels
└── usm.rs          USM 锐化
```

**新增子模块**（复刻 FlexColor pipeline）：
```
src/color/
└── flex/                   新模块（或 flexcolor_pipeline/）
    ├── mod.rs              pub API
    ├── curves/
    │   ├── mod.rs          Curve trait
    │   ├── contrast.rs     CContrastCurve
    │   ├── gamma.rs        CGammaCurve (pos) + CGammaNegCurve (neg, 1-v²)
    │   ├── negative.rs     CNegativeCurve (2-piece quadratic)
    │   ├── point.rs        CPointCurve (用户曲线 + LUT)
    │   ├── aggregate.rs    CAggregateCurve (3-mode composer)
    │   ├── high_shadow.rs  CHighShadowCurve (3-zone)
    │   └── single_point.rs CSinglePointCurve
    ├── bspline.rs          Cox-de Boor B-spline basis
    ├── pipeline.rs         ★ 顶层 Pipeline struct + build + apply
    ├── constants.rs        §19 的 15 个常量
    └── field_map.rs        CImageCorrection→Rust 映射
```

### 34.2 Curve trait

```rust
/// 14-bit pipeline 的基本曲线接口（对应 C++ CCurve 基类）
pub trait Curve {
    /// 单像素计算（对应 vtable slot 12）
    fn compute_single(&self, x: u16) -> u16;
    
    /// 构建 14-bit LUT（16384 entries，对应 vtable slot 8 = build_14bit_lut）
    fn build_lut(&self, lut: &mut [u16; 16384]) {
        // 默认实现：逐点调用 compute_single
        for i in 0..16384 {
            lut[i] = self.compute_single(i as u16);
        }
    }
    
    /// 启用标志（对应 this+0x08，未启用时返回 identity）
    fn enabled(&self) -> bool { true }
}
```

### 34.3 主要类型

```rust
// CContrastCurve: parent+0x4fe (Contrast) + parent+0x8d (Brightness)
pub struct ContrastCurve<'a> { parent: &'a ImageCorrection }

// CGammaCurve: parent+0x52c (Gamma)
pub struct GammaCurve<'a> { parent: &'a ImageCorrection }

// CGammaNegCurve: 读 parent+0x51c (FilmType) 决定启用;
// 公式 pow(1-v², 1/γ) × 16383 (§18.7)
pub struct GammaNegCurve<'a> { parent: &'a ImageCorrection, channel: u8 }

// CNegativeCurve: 2 段二次曲线，pivot 在 (x_param, y_param)
pub struct NegativeCurve {
    pub x_param: f32,  // field_0x20 × 64
    pub y_param: f32,  // field_0x24 × 64
}

// CPointCurve: 最多 10 个用户控制点 + 预计算 LUT (14-bit)
pub struct PointCurve {
    pub points: Vec<CurvePoint>,  // 控制点 (x, y, dy)
    pub lut: Box<[u16; 16384]>,   // 预计算 LUT (u16 at field_0x14)
    pub enabled: bool,
}

pub struct CurvePoint {
    pub x: u8,     // 0..255
    pub y: f64,    // double
    pub dy: u8,    // knot/端点标记（非 Hermite 切线）
}

// CAggregateCurve: 顺序 + delta 组合
pub struct AggregateCurve {
    pub children: Vec<(Box<dyn Curve>, CompositionMode)>,
}

pub enum CompositionMode {
    Sequential,   // mode 0: chain
    AddDelta,     // mode 1: running + child(last_m0)
    SubClamp,     // mode 2: running - child(last_m0), clamp [0, 0x3FFF]
}

// CHighShadowCurve: 3-zone，sub_curve = inner_agg
pub struct HighShadowCurve {
    pub shadow_boundary: u16,   // field_0x1c（来自 Shadow[ch] <<6）
    pub highlight_boundary: u16, // field_0x1e（来自 Highlight[ch] <<6）
    pub shadow_out: u16,         // field_0x20（EndPoints.shadow[ch] × 16383/255）
    pub highlight_out: u16,      // field_0x22
    pub mid_scale: f32,          // 16383 / (hi_bnd - sh_bnd)
    pub mid_add_scale: f32,      // (hi_out - sh_out) / 16383
    pub shadow_mode: u8,         // 0/1/2
    pub highlight_mode: u8,      // 0/1/2
    pub sub_curve: AggregateCurve,
}

// CSinglePointCurve: 可选单点 LUT
pub struct SinglePointCurve {
    pub enabled: bool,           // field_0x1c
    pub lut: Option<Box<[u16; 16384]>>,  // field_0x14 (nullable)
}
```

### 34.4 Pipeline 顶层

```rust
pub struct Pipeline {
    pub ic: ImageCorrection,     // 已有的 flexcolor::model::ImageCorrection
    /// 构建好的 per-channel LUT (14-bit in → 14-bit out)
    pub channel_luts: [Box<[u16; 16384]>; 3],  // R, G, B
}

impl Pipeline {
    /// 从 ImageCorrection 构建完整 pipeline（按 §16.5 拓扑）
    pub fn build(ic: ImageCorrection) -> Self {
        let mut luts = [empty_lut(), empty_lut(), empty_lut()];
        for channel in 0..3 {
            let outer_agg = Self::build_outer_agg(&ic, channel);
            outer_agg.build_lut(&mut luts[channel]);
        }
        Self { ic, channel_luts: luts }
    }
    
    /// 按 §16.5 组装 outer_agg (per-channel)
    fn build_outer_agg(ic: &ImageCorrection, ch: u8) -> AggregateCurve {
        let inner_agg = AggregateCurve {
            children: vec![
                (Box::new(GammaCurve::new(ic)), Sequential),           // [0]
                (Box::new(PointCurve::master(&ic.gradations[0])), Sequential),  // [1]
                (Box::new(NegativeCurve::shared()), Sequential),        // [2]
                (Box::new(NegativeCurve::per_channel(ch)), Sequential), // [3]
                (Box::new(PointCurve::user_a(&ic.gradations[1+ch])), AddDelta),   // [4]
                (Box::new(PointCurve::user_b(&ic.gradations[4+ch])), SubClamp),   // [5]
                (Box::new(ContrastCurve::new(ic)), Sequential),         // [6]
                (Box::new(SinglePointCurve::default()), Sequential),    // [7]
            ],
        };
        let high_shadow = HighShadowCurve::from_ic(ic, ch, inner_agg);
        AggregateCurve {
            children: vec![
                (Box::new(GammaNegCurve::new(ic, ch)), Sequential),
                (Box::new(high_shadow), Sequential),
            ],
        }
    }
    
    /// 应用到 RGB u16 图像（3 × LUT lookup）
    pub fn apply(&self, img: &mut [u16], width: usize, height: usize) {
        // 假设 interleaved RGB16; 逐 pixel 3 次查表
        for chunk in img.chunks_exact_mut(3) {
            // 注意：14-bit LUT, 需先 >> 2 (16→14) 或用 17-bit LUT
            chunk[0] = self.channel_luts[0][(chunk[0] >> 2) as usize] << 2;
            chunk[1] = self.channel_luts[1][(chunk[1] >> 2) as usize] << 2;
            chunk[2] = self.channel_luts[2][(chunk[2] >> 2) as usize] << 2;
        }
    }
}
```

### 34.5 与现有代码的集成

**`processing.rs::apply_color_pipeline_ex`** 当前负责 display_adjust + scanner_levels + C-41 invert + gradation + ICC。建议改为：

```
raw 16-bit
  → scanner_levels (暂保留，可能 FlexColor 在 Gamma 前做等价操作)
  → ICC Input (lcms2, 用 .dp:/InputProfile)
  → Pipeline::apply (14-bit per-channel LUT, 我们的 §16.5 复刻)
  → ICC Output (lcms2, 用 .dfR:/RGBProfile)
  → USM (可选)
  → 16-bit output
```

**关键**：`Pipeline::apply` 取代了现有的 manual adjust + gradation + film invert。Contrast/Brightness/Gamma/Lightness/Saturation 都被烘焙进 channel_luts。

**现有 `model.rs::ImageCorrection` struct** 已有大部分字段，只需补：
- 确认 `shadow/gray/highlight: [i64; 4]` 数组顺序与我们 §26.4 认知一致（目前 comment 说 `[RGB, R, G, B]`，但我们的分析是 ch=1,2,3 用第 2/3/4 个，即 [?, R, G, B]；第 0 个用途未明）
- `dot_color` 应重命名为 `end_points`（§28 确认）
- 增加 `endpoint_shadow_mode: i32` / `endpoint_highlight_mode: i32`（对应 this+0x510/0x514）

### 34.6 分阶段落地

**Phase 1 · Curve trait + 基础曲线**（1-2 天）
- `curves/` 下所有类，各自带单测（比对 §13 / §18.7 公式）
- `AggregateCurve` 带单测（构造简单组合，验证 mode 0/1/2）
- `bspline.rs` Cox-de Boor 带单测

**Phase 2 · Pipeline 构造 + LUT 生成**（1 天）
- `Pipeline::build` 完整 §16.5 拓扑
- 测：给定 default ImageCorrection，输出应 ≈ identity LUT

**Phase 3 · 集成到 `apply_color_pipeline_ex`**（0.5 天）
- 新建 `apply_flex_pipeline`，逐个 case 替换现有路径

**Phase 4 · 验证迭代**（按 §33 分层）
- Tier 0 → Tier 1 → ... → Tier 5
- 每失败一个，回到对应 Section 检查算法细节

### 34.7 已知难点

1. **CColorCorrection 6×6 matrix** 尚未完全解（§29.3），ApplyCC=true 时应用位置未定。MVP 可先跳过 (ApplyCC=false)
2. **USM 算法**（`this+0x11f2..0x11fe`）真实公式待挖。MVP 可沿用现有 luma-based USM
3. **CSinglePointCurve 的 LUT 构建**：它 field_0x14 的 LUT 如何生成？可能来自 this+0x84（待查）。MVP 可 `enabled=false` 跳过
4. **ICC profile 标识符 .dp:/.dfR:/...**：MVP 可 fallback 到默认 sRGB + Flextight Input（已嵌入）
5. **ColorTemperature/Tint/Saturation/Lightness 算法未解** — T16 agent 可能带回答。MVP 先置零
6. **mode 1/2 的 `last_m0` 语义**（§16.4）—  Rust 实现需同时维护 `running` 和 `last_m0` 两个变量

### 34.8 依赖与 breaking changes

- **新增依赖**：无（仅用 std；lcms2 已有）
- **现有 API 影响**：
  - `apply_color_pipeline_ex` 签名不变，内部重写
  - `ImageCorrection` struct 重命名 `dot_color` → `end_points`（**breaking change**，需同步更新 `src/viewer/` 和 `flexcolor/parser.rs`）
  - `build_curve_lut` 改为 B-spline（**breaking**，若外部 API 使用）
- **废弃**：
  - 当前 `FILM_CURVE_LUT_R/G/B` 硬编码数组（被 dynamic Pipeline 替代）
  - `apply_manual_adjust` 的部分功能（被 Pipeline 覆盖）

### 34.9 验证前置

Phase 1 测试只需复制 §13 的 Python 伪代码转 Rust，对相同输入应 bit-identical。这是**离线可验证**的（不需要 FlexColor 跑起来）。

Phase 2 的 LUT 生成与 FlexColor 比对需要：
1. 跑 FlexColor 跑出具体的 input 像素 → 输出，以 XML 为准
2. 我们的 Pipeline::build 生成同 LUT，对比 per-entry diff
3. 先跑 minimal XML（§33.4）缩小差异来源

### 34.10 时间估计

| 阶段 | 估时 | 产出 |
|------|------|------|
| Phase 1 Curves | 8-12 h | 9 个 curve 类 + 单测 |
| Phase 2 Pipeline | 4-6 h | Pipeline struct + build |
| Phase 3 集成 | 2-4 h | `apply_flex_pipeline` |
| Phase 4 验证 | 8-16 h | Tier 0-5 all PASS |
| **总计** | **22-38 h** | MVP 可用 |

已知未解算法（Saturation, ColorTemp, USM, ColorCorr）留给后续迭代，MVP 先不管。

---

## 35. 差距清单 · 已知 vs 未知（Rust 复刻阻塞点）

按 **"能否立即开写"** 分类。🟢 = 直接实现；🟡 = 能写但需运行时验证；🔴 = 缺信息不能动。

### 35.1 🟢 高置信度 · 可直接实现

| 组件 | 来源 | 状态 |
|------|------|------|
| **CContrastCurve 公式** | §13 x87 反编译还原 | ✅ 含 8192 pivot + brightness zone 形状 |
| **CGammaCurve 公式**（G≥2 / G<2 两路） | §13 | ✅ 含 `1/(G-1)` 和 `1/(1-(2-G)*0.8)` 分支 |
| **CGammaNegCurve 公式** `pow(1-v², 1/γ) × 16383` | §18.7 x87 反编译勘误 | ✅ 含 EnhancedShadow 分支（input_scale 17700 vs 16383）|
| **CNegativeCurve 公式**（2 段二次）| §15 | ✅ 含 per-channel 默认 x/y 参数（§18.8）|
| **CHighShadowCurve 3-zone + set_params** | §28 | ✅ 含 Shadow/Highlight/EndPoints 字段→参数映射 |
| **CAggregateCurve mode 0/1/2 语义** | §16.4 勘误版 | ✅ 含 `last_m0` 变量跟踪 |
| **CSinglePointCurve compute** | §16.1 | ✅ 简单 LUT 查表 + enable flag |
| **CPointCurve LUT 查表** | §16.2 | ✅ pre-built LUT @ field_0x14 |
| **B-spline 基函数**（Cox-de Boor）| §16.9 | ✅ 递归定义完整 |
| **CImageCorrection 字段布局**（50+ 字段）| §25 + §32 | ✅ 偏移 + 类型 + 默认值 |
| **所有 Apply\* bool 默认值**（ApplyHistogram/Sliders/Curves 默认 true 等）| §32.1 | ✅ |
| **Shadow/Gray/Highlight ushort[4] 布局**（XML byte <<6 到 14-bit） | §26.4 | ✅ |
| **EndPoints byte + mode 字段**（0x4ff.., 0x510） | §28.3 | ✅ |
| **9 个 Neg 默认常量**（shared + R/G/B × x/y） | §18.8 | ✅ |
| **15 个 gamma/scale 常量**（16383, 0.099, 10.1, 0.2, 17700, ...） | §19 | ✅ |
| **pipeline 拓扑**（outer_agg [GammaNeg + HighShadow] + inner_agg [8 children]） | §16.5 | ✅ per-channel 链条清晰 |

**结论**：§34 Phase 1 + 2（曲线类 + Pipeline::build）**全部可动**，无阻塞。这也是 MVP 核心。

### 35.2 🟡 可实现但需运行时验证

| 组件 | 已知 | 需验证 |
|------|------|--------|
| **B-spline 阶数 k** | 已证是 Cox-de Boor 递归（§16.9） | k=4 (cubic) 是推测，未实测 |
| **B-spline knot 向量** | 递归结构清 | 具体 knot 值（uniform / clamped / custom？）未读 |
| **CPointCurve 插值细节** | "参数采样 + 线性填充"（§16.8） | 采样密度（256 步）、边界行为待写单测比对 FlexColor |
| **`last_m0` 初始状态** | mode 1/2 使用 last_m0（§16.4） | 当第一个 child 就是 mode 1 时，last_m0 = 原始 input（根据 EDI 初始值推断） |
| **CSinglePointCurve LUT 构造** | field_0x14 是预计算 LUT | 来源（this+0x84 inline CPointCurve？还是 SingleSet set_param？）未证 |
| **this+0x44/0x48/0x4c 的 3 个指针** | 被 FUN_702d6630 初始化 field_0x20=128, field_0x24=127 | 对象类型不明；可能是第二组 per-channel curve |
| **FilmCurve 与 NegVarGamma 的关系** | FilmCurve 是 XML int；NegVarGamma 是 registry 字段 | 可能 UI 选 FilmCurve=1 (Standard) 写入 NegVarGamma=某值；或独立路径 — T18 agent 在查 |
| **FUN_702d81f0 case 编号语义** | 不是 streamableVersion 分派（用户指出矛盾）| 真实含义待重查（UI 功能组？批次？）|
| **InputProfile 默认 `.dp:` 解析** | 字符串确认 | FlexColor 运行时如何解析到实际 ICC 未知；MVP 可 fallback Flextight Input profile |

**策略**：Phase 1 测试时对拿到 FlexColor 输出做 `input→output` bit 对比，发现差异再回查。上述项不是阻塞，是"边跑边修"。

### 35.3 🔴 关键缺失 · 阻塞或大幅影响

| 缺失 | 影响 | MVP 对策 |
|------|------|---------|
| **Saturation 算法**（this+0x4fc）| 所有 `saturated` case 无法匹配 | MVP 跳过（置 0）；等 T16 agent 回 |
| **Lightness 算法**（this+0x96c）| 所有 `dark` case 无法匹配 | MVP 跳过；等 T16 |
| **ColorTemperature/Tint**（0x1224/0x1228）| 用户调 WB 的情况 | MVP 跳过（XML 默认 0）；等 T16 |
| **EV 算法**（0x1230，默认 1.0）| 曝光补偿 | MVP 跳过（默认 identity） |
| **USM 真实公式**（USMRadius/Amount/DarkLimit/NoiseLimit/ColFactor）| ApplyUSM=true 的 case | MVP 沿用我们现有 luma-based USM；失败则等 T16/T17 |
| **CColorCorrection 6×6 matrix 应用** | ApplyCC=true + ColorCorr 非零 | MVP 跳过（ApplyCC=false 时曲线仍生效）；等 T17 |
| **Pipeline 实际调用点 FUN_7031ad90** | 不影响 LUT 正确性，但影响"LUT 之后做什么"（ICC 升 16bit？USM？输出）| MVP 假设 LUT 后 ICC output profile 直接出 —— 等 T14 回 |
| **`.dp:` / `.dfR:` profile 标识符解析** | ICC 应用正确与否 | MVP fallback sRGB + Flextight Input；验证不匹配再查 |

**关键判断**：以上 8 项缺失中，**只有"USM 算法"和"Saturation"** 会让大量 case FAIL。其他（ColorTemp/Tint/EV/ColorCorr）在当前 29 test case 里**大多默认关**，不影响 MVP 达成 Tier 0-4（除 `saturated` 系列）。

### 35.4 🔵 已知不需要（范围外）

| 项 | 原因 |
|----|------|
| CFilmRepairDust / Scratch / Damage / Hole | ApplyDust=false 默认，特殊功能不需要 |
| CCorrectChromaticAberration | 相机特性，非 pipeline |
| CFilmDetector | 自动胶片识别，UI 辅助 |
| CLineCorrection / CColumnCorrection | 扫描硬件 defect 修复 |
| CImageConverter 线程框架 | 单线程 Rust 直接算即可 |

### 35.5 MVP 成就矩阵（预测）

给定 §34 Phase 1-3 完成，不引入任何 🔴 项的 workaround，各 Tier 的预期：

| Tier | case 数 | 预期通过 | 可能卡点 |
|------|--------|---------|---------|
| **T0 Identity** | 2 (rgb_standard emb + ext) | **PASS** | ICC profile 匹配 |
| **T1 单 slider** | 8（包含 dark/saturated） | **部分 PASS**（Contrast/Gamma/Brightness）；Lightness/Saturation **FAIL** | 两个 slider 算法未解 |
| **T2 曲线/直方图** | 0（现 preset 都用默认）| N/A | Hermite vs B-spline 差异待测 |
| **T3 负片** | 5 | **大概率 PASS** | CGammaNeg + CNegativeCurve 公式完整 |
| **T4 CMYK/BW** | 10 | ICC 相关；**部分 PASS** | Output profile 装载 |
| **T5 e2e** | 3 | 含所有特性 | 多个 FAIL，逐一回查 |

**预期 MVP 完成后**: ~15/29 PASS，~8/29 WARN（接近但非 STRICT），~6/29 FAIL（Saturation/ColorTemp/USM 相关）。等 T16/T17 回补后可继续提升。

### 35.6 并行 agent 完成后的更新入口

3 个后台 agent 可能带回：
- **T14 (FUN_7031ad90)** — 若是主 pixel loop，§35.3 "Pipeline 实际调用点" 清 → Rust Phase 3 集成少走弯路
- **T16 (Saturation/ColorTemp/Tint/Lightness)** — 若带回公式，§35.3 前 4 项 🔴 → 🟢
- **T18 (FilmCurve preset)** — 若找到数组 → §35.2 "FilmCurve 与 NegVarGamma" 清 + §33.3 差分测试多一维

待 agent 完成，合并报告为 Section 36/37/38，重扫 🔴 项。

### 35.7 优先级明确化

**MVP Phase 1（高 ROI）**：
1. 实现 §35.1 所有 🟢 组件 + Phase 2 Pipeline::build
2. 跑 Tier 0 validation
3. 按 §33.4 构造 minimal XML 跑 Tier 1 单 slider
4. 迭代到 Tier 3（负片）

**Phase 1 完成后再决策**：
- 若 Tier 3 全 PASS → 走 Tier 4（CMYK/BW 输出 profile）
- 若 Tier 3 部分 FAIL → 回查 pipeline 组装顺序 / B-spline / mode 1/2 细节

**Phase 2（T16 agent 结果回来后）**：
- 补 Saturation / Lightness / ColorTemp 算法
- 重跑 Tier 1 `saturated` / `dark` 变体

**Phase 3（T16/T17 完整后）**：
- USM / ColorCorr 对齐 FlexColor
- 达到 STRICT 级

---

## 36. ICC Profiles 磁盘清单（16 个文件，2026-04-19）

位于 `/Users/will/vmwareShare/FlexColor v4.8.9.1/Profiles/`：

| 文件 | 大小 | class | space→PCS | ver | 说明 |
|------|-----|-------|-----------|-----|------|
| **Flextight Input.icc** | 613 KB | scnr | RGB→Lab | 2.0.0 | ★ 默认输入 profile（`.dp:` 对应）|
| Flextight X5 & 949.icc | 113 KB | scnr | RGB→Lab | 2.0.0 | X5 扫描仪 |
| Flextight X1 848 & 646.icc | 113 KB | scnr | RGB→Lab | 2.0.0 | X1 扫描仪 |
| Flextight 2848 v2.icc | 113 KB | scnr | RGB→Lab | 2.0.0 | 2848 扫描仪 |
| Flextight 343 v2.icc | 113 KB | scnr | RGB→Lab | 2.0.0 | 343 扫描仪 |
| Flextight 2848 ref.icc | 244 KB | scnr | RGB→Lab | 2.0.3 | 参考 + K-tag 扩展 |
| **Hasselblad RGB.icc** | **532 B** | scnr | RGB→XYZ | 2.0.0 | ★ 默认 RGB 输出（`.dfR:` 对应，仅 TRC + 矩阵）|
| **Hasselblad Gray.icc** | **364 B** | mntr | GRAY→XYZ | 2.1.0 | ★ 默认 Gray（`.dfG:` 对应，仅 kTRC + wtpt）|
| Hasselblad 250GCR 40K90.icc | 599 KB | prtr | **CMYK**→Lab | 2.0.0 | ★ CMYK 候选（250 线 GCR 40% K 限 90%）|
| Hasselblad 280GCR 30K90.icc | 599 KB | prtr | **CMYK**→Lab | 2.0.0 | ★ CMYK 候选（280 线 GCR 30% K 限 90%）|
| Hasselblad 330GCR 20K95.icc | 599 KB | prtr | **CMYK**→Lab | 2.0.0 | ★ CMYK 候选（330 线 GCR 20% K 限 95%）|
| Hasselblad 330Skel 30K75.icc | 599 KB | prtr | **CMYK**→Lab | 2.0.0 | Skeleton black 变体 |
| Hasselblad 350GCR 20K95.icc | 599 KB | prtr | **CMYK**→Lab | 2.0.0 | 350 线 |
| Hasselblad 350Skel 30K90.icc | 599 KB | prtr | **CMYK**→Lab | 2.0.0 | 350 Skeleton |
| Leica DMR.icc | 536 B | scnr | RGB→XYZ | 2.0.0 | Leica 相机后背 |

### 36.1 默认标识符映射（基本定型）

| 标识符 | 对应 ICC | 大小/类型 |
|--------|---------|----------|
| **.dp:** | `Flextight Input.icc` | 613KB scanner profile，A2B0/A2B1/A2B2 全 CLUT |
| **.dfR:** | `Hasselblad RGB.icc` | 532B 矩阵 + TRC（仅 rTRC/gTRC/bTRC + rXYZ/gXYZ/bXYZ）|
| **.dfC:** | `Hasselblad 280GCR 30K90.icc`（推测默认）| 599KB CMYK→Lab CLUT。实际值需查 Registry/默认 preset |
| **.dfG:** | `Hasselblad Gray.icc` | 364B 仅 kTRC + wtpt |

### 36.2 Hasselblad RGB 的 Primary + TRC（可直接提取）

`Hasselblad RGB.icc` 只有 532 字节，是 **Matrix + 1D TRC** 类型 profile。MVP 可直接硬编码：
- rXYZ / gXYZ / bXYZ（9 个定点数，定义 RGB→XYZ 矩阵）
- rTRC / gTRC / bTRC（可能单个 gamma 数或 ≤ 16 字节 curve）
- wtpt（白点 XYZ）
- bkpt（黑点 XYZ）

**复刻策略**：用 `lcms2::Profile::from_memory(bytes)` 直接载入文件，无需手工解析。或用 `qcms` Rust 原生库。

### 36.3 Flextight Input 的 A2B0/A2B1/A2B2 全 CLUT

`Flextight Input.icc` 613KB 包含 3 个 rendering intent 的 3D CLUT：
- A2B0 (Perceptual): 设备→PCS
- A2B1 (Relative Colorimetric)
- A2B2 (Saturation)

这是核心。lcms2 可直接使用 — 不用手工解析 CLUT。

### 36.4 验证路径

在 Rust 测试中，加载 `Flextight Input.icc` 作为 input profile，`Hasselblad RGB.icc` 作为 output profile，对比 FlexColor 默认 (`.dp:` / `.dfR:`) 渲染。若像素差异 &lt;1 LSB，证明 profile 映射正确。

---

## 37. T14 Agent 成果 — 真实像素处理路径

agent `a806658dc1e1b6628` 挖到关键发现：

### 37.1 真正的像素循环：FUN_702f4270

**不是** 通过 CAggregateCurve.build_lut → compute_single 的路径。主像素循环：

```c
// In FUN_702f4270 (pipeline apply for one tile)

// 1) 前置校正（slot at this+0x8 的 vtable+0x1c）
(**(this+8)->vtable + 0x1c)(buf, W*3, scale);

// 2) 主 LUT apply — 每 RGB triple 独立查表
for (i = 0; i < W*3; i += 3) {
    buf[i+0] = LUT_R[buf[i+0]];   // this+0x1c = ushort LUT[16384]
    buf[i+1] = LUT_G[buf[i+1]];   // this+0x20
    buf[i+2] = LUT_B[buf[i+2]];   // this+0x24
}

// 3) FUN_702d4b50 — post-LUT 色彩变换（可能 ColorCorrection matrix 应用）
// 4) FUN_702d4720 — per-row saturation/Lightness curve
// 5) FUN_702f4690 — 水平翻转（根据 transform flag）
// 6) FUN_705146a0 — USM 锐化
```

### 37.2 LUT 构建：FUN_705142a0（orchestrator）

关键：**LUT 不通过 CAggregateCurve 的 8-child 链构建**！`FUN_705142a0` 直接从 CImageCorrection 参数用 `FUN_70512fb0`（quadratic shoulder curve builder）生成 3 条 14-bit LUT。

```c
FUN_705142a0(tmp, src_CImageCorrection, ...)
  ├─ 读取 src+0x11fe (USMRadius), src+0x11fa (USMDarkLimit), src+0x1220 (ApplyCNFilter) 等
  ├─ FUN_70512fb0(1, ...) // 构建 14-bit shoulder LUT → tmp+0xc (0x10000 字节 = 4×16384 int)
  ├─ FUN_70512f10(tmp)    // 构建 14→8-bit knee LUT → tmp+0xbc
  └─ FUN_70513890()       // black/white-point 校准采样
```

### 37.3 Pipeline 拓扑重新认识（关键！）

**§16.5 的 8-child inner_agg 拓扑可能是 GUI 预览/实时曲线编辑路径**，**不是**主输出的像素处理。主输出通过 `FUN_705142a0` 直接基于同一组参数重构等价 LUT，走 quadratic shoulder 方案而非 B-spline + curve chain。

这一点需要在 Rust 复刻时注意：
- 若目标是**复刻 FlexColor 输出 TIFF**（bit-accurate）→ 应实现 `FUN_70512fb0` 的 quadratic shoulder 方案
- 若目标是**GUI 互动曲线**（用户拖动 CPointCurve 实时反馈）→ 实现 §16.5 的 B-spline + aggregate 方案
- MVP 先做前者（精确复刻），后做后者（用户体验）

### 37.4 Run 循环：FUN_702d9e60（全分辨率）vs FUN_702d3cf0（预览）

- **FUN_702d9e60** = `CImageConverter::Run`，全分辨率 tile pump 循环
- **FUN_702d3cf0** = `CImageConverterOffscreen::slot4`，下采样/预览路径（对用户图像先缩小再处理）

两者都调 `FUN_702f4270(tile_idx)` 做单 tile 处理。共 8 个 call site。

### 37.5 ICC 路径分支

通过 `this+0x42` 开关：
- **正常路径 (this+0x42==0)**: `RAW → 前置校正 → R/G/B 独立 14-bit LUT → post_color_correction → saturation → flip → USM → 输出`
- **ICC 路径 (this+0x42!=0)**: `RAW → 16-bit per-sample profile transform (vtable+0x30 call) → USM → 输出`（**跳过** LUT + saturation）

即 **ICC 和 LUT 是 OR 关系，不是串联**。`CImageConverter` 根据某个配置位选一条。

### 37.6 新函数/类索引（供后续深挖）

| 地址 | 作用 |
|------|------|
| **0x702f4270** | ★ 主 pixel apply 循环（含 LUT lookup + sat + USM 调用）|
| **0x705142a0** | ★ LUT 构建 orchestrator |
| **0x70512fb0** | ★ 14-bit quadratic shoulder LUT builder |
| 0x70512f10 | 14→8-bit knee LUT builder |
| 0x70513890 | 黑白点校准采样 |
| **0x702d9e60** | ★ CImageConverter::Run 全分辨率 tile pump |
| 0x702d4b50 | post_LUT color correction（未解码，可能 ColorCorrection matrix）|
| 0x702d4720 | per-row saturation/Lightness curve |
| 0x705146a0 | USM 锐化（本项目已有等价实现）|

---

## 38. T18 Agent 成果 — FilmCurve 不参与 Pipeline

agent `a1e6b4faa35bb8ad7` 确认：

### 38.1 FilmCurve 是 UI 状态，非 pipeline 参数

- **`.rdata` 中没有 FilmCurve 索引的 preset 数组** — 5 档 `Standard/Low/High/Old/Linear` 不对应代码里的数据表
- `CFilmCurve` 类只有析构函数（vtable 1 slot），是纯数据 stub
- 字段 `this+0x123c` 在整个 DLL 只有 **7 条机器指令引用**：2 个写（reset + XML 缺省）、1 个 XML 读入、2 个序列化（writer）、2 个 pipeline 消费

### 38.2 真实消费：4 → 2 重映射

Pipeline 中唯一使用 FilmCurve 的地方（`FUN_70245cc0 @ 0x70246359`, `FUN_702dcc60 @ 0x702dccef`）都是同一模式：

```c
int fc = this->FilmCurve;    // 0x123c
if (fc == 4) fc = 2;         // 或 FUN_70250620() 的分支
downstream_ctx->field_0x170 = fc;  // 写入下游 metadata
```

**没有** switch 按 0/1/2/3 选择不同 preset。也**没有** FilmCurve → gamma/stretch 的映射代码。

### 38.3 默认值其实是 4（不是 0）

- `FUN_702d6a30 @ 0x702d6a69`: reset 路径写 `*(+0x123c) = 4`
- `0x702d7cc4`: XML reader fallback（XML 无 `<FilmCurve>` 时）写 4
- ctor 零置只是 memset 副作用，不是"语义默认"

由于 `if (fc == 4) fc = 2;`，**下游实际值 = 2**。

### 38.4 FilmCurve 与 NegVarGamma 独立

CGammaNegCurve 的 build 只读 XML `NegVarGamma` / `StretchNegGamma`，**完全不引用 this+0x123c**。所以 FilmCurve **不 override** NegVarGamma。

### 38.5 对 Rust 复刻的影响

- **MVP 可完全跳过 FilmCurve 字段**（UI 展示目的除外）
- 若需 bit-精确序列化 3F metadata，`FilmCurve` 硬编码为 4（UI state）或 2（effective downstream）均可
- `T18 · FilmCurve preset 数组` 结论：**不存在**

---

## 39. Colormaps YCbCr LUT 解析（A 任务，2026-04-19）

### 39.1 7 个 Colormap 文件概览

位于 `/Users/will/vmwareShare/FlexColor v4.8.9.1/Colormaps/`：

| 文件 | 大小 | tf (daylight) | tt (tungsten) | 用途（从文件名推断）|
|------|-----|----|----|-----|
| LUTTable22MPC.xml | 1.6 MB | 5130 K | 3238 K | 22 MP 后背（CFA 变体 C）|
| LUTTable31MP.xml | 1.6 MB | 5400 K | 3200 K | 31 MP 后背 |
| LUTTable31MPC.xml | 1.6 MB | 5130 K | 3238 K | 31 MP-C |
| LUTTable39MP.xml | 1.6 MB | 5000 K | 2900 K | 39 MP 后背（CFV-39 等）|
| LUTTable39MPC.xml | 1.6 MB | 5130 K | 3238 K | 39 MP-C |
| LUTTableIxpress.xml | 1.6 MB | 5000 K | 2900 K | Ixpress 系列（legacy）|
| LUTTableleica.xml | 1.7 MB | 5000 K | 2900 K | Leica DMR 后背 |

### 39.2 每文件的 keys 和结构

**一致 schema**（除 leica 的一项 LUT 缺 1 entry）：
```
TimeStamp  : "2007-11-08 14:08:03"
Version    : "2.0"
tf, tt     : int (两个参考色温)
vf         : [real, real, real]  (tf 色温下的 RGB 偏移/增益)
vt         : [real, real, real]  (tt 色温下)
mf         : [real × 9]           (tf 色温下 3×3 矩阵)
mfd        : [real × 9]           (中间色温 3×3 矩阵，"daylight" 可能是 "default"?)
mt         : [real × 9]           (tt 色温下)
CbS, CbE   : -20, 84   (Cb 范围, 共 105 格)
CrS, CrE   : -32, 56   (Cr 范围, 共 89 格)
DivFactor  : 32         (Cb/Cr 输入的缩放因子)
LUTTableFlashStd   : [real × 18690]  (flash + standard intent)
LUTTableTSStd      : [real × 18690]  (TS = tungsten, standard intent)
LUTTableFlashRepro : [real × 18690]  (flash + reproduction)
LUTTableTSRepro    : [real × 18690]  (tungsten + reproduction)
```

**18690 = 105 × 89 × 2** （Cb 格数 × Cr 格数 × 2 输出 ΔCb/ΔCr）

### 39.3 数据解读

**vf/vt**：三通道向量。示例 22MPC 的 `vf = [2.1825, 0.0, 0.8732]` —— 中间 0.0 暗示可能是**对数增益**（或者**相对于 G 的 log 比值**）：
- 实际上 `vf[0] = log2(R_gain/G_gain)`, `vf[1] = 0`（参考通道）, `vf[2] = log2(B_gain/G_gain)`
- 验证：`2.1825` 对应 R 增益 2^2.18 ≈ 4.5，在日光下合理（R 补偿紫外滤光）

**mf/mfd/mt**：3 个 3×3 矩阵，对应 **3 个参考色温**下的 sensor→sRGB（或类似）变换矩阵。运行时按实际 CT 插值：
- `CT < tt` 或 `CT == tt` → 用 mt
- `CT == tf` → 用 mf
- `tt < CT < tf` → 用 mfd，或在 mf/mfd/mt 间双线性插值

**LUT 4 变体**：
- **Flash vs TS**：场景光源（Flash 日光 / TS 钨丝）决定选哪条 LUT
- **Std vs Repro**：rendering intent（standard = 扫描忠实 / reproduction = 广色域打印）

LUT indexing（推测）：
```python
# 输入：YCbCr 空间的 (Cb, Cr)
cb_idx = (Cb / DivFactor) - CbS    # 0..104
cr_idx = (Cr / DivFactor) - CrS    # 0..88
# LUT 是 row-major 还是 col-major 未定，需实验确认
idx = (cr_idx * 105 + cb_idx) * 2
delta_cb = LUT[idx]
delta_cr = LUT[idx + 1]
```

### 39.4 🚨 关键判断：这些 LUT 适用于**相机后背**，不是**扫描仪**

**文件名全部指向相机后背**（22MP, 31MP, 39MP, Ixpress, Leica DMR），**没有** Flextight 扫描仪的 Colormap。

回忆 §36 的 ICC profile：
- **扫描仪 profile**: Flextight Input.icc, Flextight 2848/343/X1/X5
- **后背 profile**: Hasselblad RGB.icc, Leica DMR.icc

**双条独立路径**：
- **相机后背 FFF**: raw → Colormap YCbCr correction → sRGB
- **扫描仪 FFF** (我们的 test 数据): raw → Flextight Input ICC → ImageCorrection curves → 输出

**结论**：
- `fff_parse` 项目处理**扫描仪 FFF**（文件名 test1_raw.fff 属 Flextight X5 等）
- **Colormaps 与我们的 pipeline 无关**
- §二 的 YCbCr 色度 LUT 假设**只适用于相机后背文件**，不是扫描仪文件，这也是 §二 假设在扫描仪 test 数据上失败的根本原因

### 39.5 如果未来支持相机后背 FFF

若 `fff_parse` 扩展支持相机后背文件（Hasselblad H-series, Ixpress, etc.），则需：
1. 从 FFF 的 camera metadata 识别后背型号
2. 加载对应 `LUTTable{model}.xml`
3. RGB → YCbCr（BT.601/709 或 Hasselblad 自定义矩阵）
4. `(Cb, Cr)` 查 LUT → `(ΔCb, ΔCr)` → 应用
5. 按场景光源 + intent 选 4 种 LUT 之一
6. 按色温插值 mf/mfd/mt 矩阵

当前范围外，仅留作参考。

### 39.6 FlexColor.dll 里是否 xref 这些 Colormap 文件？

字符串 `LUTTable22MPC.xml` 等应出现在 DLL 的 string 表里。待查（可用 `./run.sh find-str-xrefs "LUTTable"` 确认使用路径）。

---

## 40. Colormaps XML 子组件存储位置（§二 新发现）

§二 里我们注意到 ColorMap XML 的存在但未确认使用。结合 §39 判断：

- **适用对象**：相机后背 FFF（非 Flextight 扫描仪）
- **触发路径**：FFF header 的 camera model 字段 → 加载对应 LUT → 在 raw 解析阶段（**而非** CImageCorrection 阶段）应用 YCbCr 色度校正
- **是否在 CImageCorrection 之前 or 之后**：推测在之前（作为 raw→linear RGB 的一部分），但未证实
- **测试数据中会不会见到**：只有当用户加载来自 H3D/Ixpress 等后背的 3F 文件时触发。test1*.fff 是 Flextight 扫的胶片，不会走这条路径

对当前 Rust MVP：**完全可以忽略 Colormaps**。

---

## 41. T16 Agent 中期成果 — ColorTemperature/Tint/Saturation/Lightness 算法

agent `a1d403090c6d3a4e5` 仍在跑，下列为中期提取。

### 41.1 ColorTemperature + Tint → RGB gains（已完全解）

**入口**：`FUN_702dcc60` 是 WB/ColorModel 总调度（在 CImageConverter 的 early-stage 调用）：
```c
if (*(char*)(this+0x91) != 0) {   // WB 启用标志
    FUN_702406a0(
        *(int*)(this->ic + 0x1224),      // ColorTemperature int32
        *(double*)(this->ic + 0x1228)    // Tint double
    );
    *(double*)(output + 0x28) = *(double*)(this->ic + 0x1230);  // EV double
    *(int*)(output + 0x38) = *(int*)(this->ic + 0x1238);        // ColorModel int32
    // FilmCurve 4→2 remap 在这里
}
```

**算法** (`FUN_702406a0` → `FUN_70596d50`)：

```python
def ct_tint_to_rgb_gains(ct: int, tint: float) -> (float, float, float):
    # Step 1: CT → Mired
    mired = 1_000_000 / ct   # 倒数色温（Mired）
    
    # Step 2: 查 Planckian locus 表
    # 表位于 0x706e2410, stride 0x20 (4 doubles/entry), 至少 29 项
    # 每项：[mired_low, u_coef_low, v_coef_low, ..., mired_high, ...]
    i = 0
    while mired >= planckian_table[i].mired:
        i += 1
    # Step 3: 线性插值
    t = (planckian_table[i].mired - mired) / (planckian_table[i].mired - planckian_table[i-1].mired)
    
    # Step 4: (u, v) 色度 + Tint offset
    # Tint 通过 sqrt 归一化后沿 isotemperature direction 加到 (u, v)
    u = table[i].u * t + table[i+1].u * (1 - t)
    v = table[i].v * t + table[i+1].v * (1 - t)
    tint_mag = ... sqrt(u² + v²) ...
    u += tint * u_perp_direction
    v += tint * v_perp_direction
    
    # Step 5: 转 (x, y) 色度
    x = 3u / (2u - 8v + 4)
    y = 2v / (2u - 8v + 4)
    
    # Step 6: 调 FUN_702fe3b0(x, y, 1-x-y) → 2×3 matrix → 每行 dot product → 得 RGB 3 doubles
    rgb_raw = matrix_apply(x, y, 1-x-y)
    
    # Step 7: 归一化（每通道除以最大值）
    rgb_max = max(rgb_raw)
    return [rgb_max / r for r in rgb_raw]    # 每通道倒数，WB 是对角 multiply
```

**结论**：**WB 是 RGB per-channel 对角 gains（3 doubles），不是 3×3 色度旋转矩阵**。复刻可直接用 Planckian locus + (x,y)→RGB 公式，或用 lcms2 内建 CCT→chromaticity。

### 41.2 Saturation 与 6×6 ColorCorr 矩阵关系

**FUN_702d4f30** 反汇编（精确）：
```asm
; loop 6×6 = 36 iterations
;   eax = matrix[i,j] (signed short)
;   ebp = -saturation
;   if (eax != -sat) AL |= 1
; returns AL in caller's bool
```

即 `IsSaturationMatrixCustomized() = (any 6×6 entry != -saturation)`。

**解读假说**（需进一步确认）：
- 6×6 矩阵的条目存储的是 **(actual_coef + saturation)** 的偏移
- 即 `effective_coef[i,j] = matrix[i,j] - saturation`
- 当 user 调 Saturation 时，矩阵不重建，**只是运行时减 saturation**
- 这解释了为什么"all entries = -sat → effective all = 0（无色彩校正）"：是 identity fast-path 触发条件

**这是 Saturation 的主要作用机制**：不是 HSV 饱和度变换，而是**色彩校正矩阵的全局偏移**。

### 41.3 Tint 的具体读者

找到 11 处 0x1228 引用：
- ctor/reset 写 (702d4547, 702d6b61, 702d8011, 702def6c)
- Serializer 写 (702d7590 @ 702d781d, 702d81f0 @ 702d8537)
- 管线读者 (702dcc60 @ 702dcca1, **70245cc0 @ 70246093, 702500b0 @ 70250383, 702de9d0**)

`FUN_70245cc0 @ 0x70246093` 是**主 CImageConverter pipeline 入口**，在这里 `FLD double [ecx+0x1228]; CALL FUN_702406a0` → 调用 CT/Tint→gains 转换。这**确证 WB 在 pipeline 早期执行**。

### 41.4 Lightness (0x96c) 消费

找到 11 处 0x96c 引用，除序列化外，唯一运行时消费者：
- **FUN_702d4720** (`MOV AL, [EBP+0x96c]` 读 + 0x96c ≠ 0 判断)
- **FUN_702d6ce0** (preset 曲线 descriptor 读)

`FUN_702d4720` 是 §37.1 看到的 "per-row saturation curve"。它同时读 Lightness — 说明 **Lightness 与 Saturation 在同一 post-LUT 阶段处理**（都是 per-row 调用 curve vtable+0x30）。

### 41.5 EV (0x1230) 传递

`FUN_702dcc60` 直接 copy `*(double*)(this+0x1230) → output+0x28`。即 EV 被传递到下游作为曝光乘数 context，**不在 CImageCorrection 内做 pow(2, EV) 运算**，而是在下游 expose step。具体 expose 函数待查。

### 41.6 对 Rust MVP 的影响

| 缺失 (§35.3) | 现状 | 可实现度 |
|------|------|---------|
| ColorTemperature/Tint | ✅ 算法完整（Planckian + chromaticity + RGB gains）| 🟢 可实现（用 `colorimetry` crate 或手撸）|
| Saturation 算法 | 🟡 机制基本明确（矩阵偏移），需验证 `effective = matrix - sat` 假设 | 🟡 简单但未完全证实 |
| Lightness 算法 | 🟡 位于 FUN_702d4720（per-row curve），需深挖其具体曲线 | 🟡 FUN_702d4720 内部未完全解 |
| EV 算法 | 🟡 不在 CImageCorrection 内，在下游 expose；可能是简单 `output *= pow(2, EV)` | 🟡 推测 |

T16 仍在跑，可能还会带更多 Tint/Lightness 细节。待最终报告发布后更新此 section。

---

## 42. T16 Agent 最终报告 — 全部 4 个 slider 算法完整

### 42.1 汇总表

| Slider | offset | type | pipeline 消费者 | identity fast-path |
|--------|--------|------|-----------------|--------------------|
| Saturation | 0x4fc | int16 | **无直接像素级 reader**；折入 6×6 矩阵 @ 0x4b4 | `FUN_702d4f30` 返回 bool |
| Lightness | 0x96c | int16 | `FUN_702d4720`（像素循环）+ `FUN_702d6ce0`（preset）| `>0` 硬门控 |
| ColorTemperature | 0x1224 | int32 | `FUN_702406a0 → FUN_70596d50` | `CT==0` 或 `FCOMP 常量` |
| Tint | 0x1228 | **double** (不是 int32！) | 同 CT 路径 | 与 CT 绑定 gate |
| EV | 0x1230 | double | Pass-through 到 pipeline+0x28 | 无 gate |

### 42.2 Saturation = 6×6 矩阵全局偏移（基本确认）

**没有**独立 per-pixel Saturation 公式。所有 0x4fc 的读取都在：
- XML 序列化（writers）
- 矩阵 identity 检查 `FUN_702d4f30`：
  ```c
  bool matrix_customized = any((matrix[i,j] != -saturation) for i,j in 6×6)
  ```

**编码假说**（需验证）：6×6 矩阵存储 `cell = actual_coef + saturation`。当 user 调 Saturation 时，矩阵**不重建**，仅 identity 检查用 `-sat` 对照。实际 apply 时 effective = `cell - sat`（或等价运算）。

**实质**：Saturation 就是 ColorCorrection 6×6 矩阵的 "中性偏移量"。Saturation=0 + 矩阵全 0 → 色彩校正关闭。

### 42.3 Lightness ≡ Shadow Depth + CPointCurve 叠加（真公式，§44 勘误版）

> ⚠️ **勘误（2026-04-19，T20 agent）**：此前写 "CFilmCurve.compute_single" **错**。CFilmCurve 是僵尸类（只剩析构）。实际被调用的是 **CPointCurve** at `this+0x84`。详见 §44。

**核心公式**：FUN_702d4720 像素循环：

```c
if (*(CPointCurve**)(this+0x84) != NULL && Lightness > 0 && ApplySliders) {
    for (i = 0; i < pixel_count; i++) {
        ushort raw_val = src[i];                       // 原像素值
        ushort curve_val = point_curve_at_0x84.compute_14bit(raw_val);  // CPointCurve slot 12
        short delta = curve_val - raw_val;             // CPointCurve 曲线偏差
        // 对 RGB 3 通道**同时加**同一 delta（单色加）
        for (ch = 0; ch < 3; ch++) {
            out[ch] = clamp(out[ch] + delta, 0, 0x3fff);
        }
    }
}
```

**关键点**：
- **非对称 `Lightness > 0`**：Lightness 只能**加亮阴影**，不能压暗。Negative slider 无效 → 证实 "Lightness = Shadow Depth"
- **曲线来源**：`this+0x84` 的 CPointCurve 实例。默认控制点是 `(2, 2)` 和 `(100, 100)`（§16.3）—— 几乎是 identity，所以 **默认 Lightness slider 无效果**
- **何时生效**：需 XML 或 UI 向 `this+0x84` 的 CPointCurve 加载非 identity 控制点。控制点加载路径未完全追踪（T20 记录）
- **RGB 同加**（不是 per-channel）：alpha/achromatic 贡献，避免色偏
- Lightness 数值本身**不在 FUN_702d4720 内缩放**；它的强度已烘焙进 CPointCurve LUT 构建时（controlpoint + spline）

次消费者 `FUN_702d6ce0`：
```c
iVar4 = Lightness * 250;        // *0xfa
offset = iVar4/100 - (iVar4>>31) - ... + 2;
*(byte*)(curve_desc + 0x39) = offset;  // 写 curve preset 索引调整
```
是 **curve preset 选择**（不是像素级公式），可能对应 "Shadow Depth 强度档位 0-5"。

### 42.4 CT + Tint → 3 个 RGB per-channel gains（完整）

算法流程：
```
CT (int) → Mired = 1e6 / CT
 ↓
查 Planckian locus 表 @ 0x706e2410（29 行 × 32 字节 = 4 doubles/row）
 ↓
线性插值 (t = ...) 得 (X_coef, Y_coef)
 ↓
X = X_coef*t + X_base*(1-t) + Tint * (sqrt-based perpendicular offset)
Y = Y_coef*t + Y_base*(1-t) + Tint * (同上)
 ↓
(x, y) = (X_coef_normalized, Y_normalized)   # CIE 1931 色度坐标
 ↓
FUN_702fe3b0(CColorTempConversion, x, y, 1-x-y) → 应用 {M, M^-1} 转换
 ↓
RGB_raw = 3 doubles
 ↓
归一化：RGB_gains[c] = max(RGB_raw) / RGB_raw[c]   （每通道倒数，对角 multiply）
```

**CColorTempConversion**: 存 `{M (3×3 double), M^-1 (3×3 double)}`（§41.1 已解）。M 可能是相机/扫描仪的 **camera-to-XYZ** 矩阵，M^-1 反之。

### 42.5 CT 的 fast-path 清晰

```c
// FUN_702de6a0: 判断是否跳过
*(bool*)(pipeline+0x90) = (CT == 0);

// FUN_702de9d0: pipeline 运行前后
if (skip_flag) {
    saved_CT = *(int*)(this+0x1224);
    saved_Tint = *(double*)(this+0x1228);
    *(int*)(this+0x1224) = 0;        // 清零
    *(double*)(this+0x1228) = 0;     // 清零
    // ...pipeline 运行...
    *(int*)(this+0x1224) = saved_CT;   // 恢复
    *(double*)(this+0x1228) = saved_Tint;
}
```

即 **CT=0 → 跳过 WB**（Tint 也强制 0），pipeline 结束后恢复字段值（给 UI 显示）。

另外 `FUN_702500b0` 有 `FCOMP double[0x70733588]` 对 CT 做次级比较（可能是"CT < 1000K 无效"的保护）。

### 42.6 EV 在 FlexColor 外处理

```c
// FUN_702dcc60 @ 0x702dccbf:
FLD double [+0x1230];
FSTP double [pipeline_output + 0x28];   // 直接 copy 到下游 context
```

**FlexColor 不做 `pow(2, EV)`**。它把 EV 作为 double 传给下游（可能是 raw 读取层 / HasDeviceLink64.dll / 扫描仪硬件层）。默认 1.0。

### 42.7 对 Rust MVP 的直接影响

更新 §35.3 🔴 阻塞项：

| 原 🔴 项 | 新状态 | Rust MVP 实现路径 |
|----------|--------|------------------|
| Saturation 算法 | 🟢 就绪 | 6×6 矩阵 apply 实现；identity 检查 `all(cell == -sat)` |
| Lightness 算法 | 🟢 就绪 | 调 CFilmCurve LUT + 加 delta 到 RGB 等同；**需 CFilmCurve LUT 数据（待 dump .rdata）** |
| ColorTemperature/Tint | 🟢 就绪 | Planckian locus + (x,y) + {M, M^-1} → RGB gains。可用标准 color science 库或手撸 |
| EV | 🟡 跳过 | 不在 CImageCorrection 内，无需实现 |

**仅剩实际问题**：
- CFilmCurve LUT 数据（Lightness 用）— 未 dump `.rdata` 里的 CFilmCurve 实例数据
- Planckian 表内容（29 行 × 4 doubles）— 未 dump（Ghidra 可 read-const 拉出来）
- 6×6 矩阵应用位置（post-LUT 14-bit 阶段？还是独立阶段？）— 待进一步确认

### 42.8 agent 新增工具

agent 新建 `tools/ghidra_query/scripts/find_offset_refs.py`，支持按 struct field 偏移量搜索所有反汇编引用（对这次调查关键）。

**用法**：
```
./run.sh find-offset-refs <offset> [segment_prefix]
# 例：./run.sh find-offset-refs 0x4fc           # 所有 0x4fc 引用
# 例：./run.sh find-offset-refs 0x4fc 0x702     # 限定 0x702xxxxx 段
```

---

## 43. 🔄 方向转变 · 从"反推 LUT"到"前向计算 LUT"（2026-04-19）

**重要勘误性笔记**：项目早期的"从 ref TIF / FFF thumbnail 反推 LUT"路径**方向错了**。记录于此以防后人重走老路。

### 43.1 早期思路（现已作废）

```
raw 16-bit → 我们的 pipeline 猜测 → our_output
                                    ↓（与 ref 比对）
                                  diff 大
                                    ↓
                          从 ref 反推 LUT（extract_film_curve_16）
                                    ↓
                            修正 pipeline 参数
```

用于此路径的工具：
- `extract_film_curve_16()` —— 从 (raw, ref) pair 构建 LUT
- `--use-ref-lut` flag —— tif_compare 里"作弊"模式直接用反推 LUT
- `FILM_CURVE_LUT_R/G/B` —— 从 Portra 160 + X5 标定得到的硬编码 256-entry LUT
- 从 thumbnail 提取曲线（给 C-41 负片用）

### 43.2 为什么方向错了

**根本问题**：ref TIF 和 FFF thumbnail **都是 FlexColor 自己用它的内部 LUT 生成的输出**。从它们反推 LUT，本质是用"结果"反推"工具"——这条路径：
1. **需要的信息已经在输入端（XML）**：我们有 `<ImageCorrection>` 的完整字段（§25），只要按 FlexColor 公式前向计算即可得到完全同一条 LUT
2. **反推引入不必要的 noise**：采样 pair、插值、局部拟合误差累积，永远达不到 bit-accurate
3. **通用性极差**：针对每个 preset 都要重新标定；换胶片/相机就失效；§二 里硬编码 LUT 只对 Portra 160 + X5 工作就是例证
4. **掩盖了真实管线结构**：把"Gamma + Master + Neg + Contrast + SinglePoint"8 步链路塌缩为一条 256-entry 拟合 LUT，丢失了全部中间结构

### 43.3 新思路（基于 §37 + §42 的认识）

```
FlexColor XML (ImageCorrection 50+ 字段)
        ↓（按 §18.7 / §15 / §42 公式计算）
构造 3 条 14-bit LUT[16384] per-channel
        ↓（应用到 raw 像素）
our_output
        ↓（tif_compare 比对）
ref  ← STRICT bit-accurate 目标
```

这是**正向推导**：输入 XML 就是 FlexColor 当时生成 ref 所用的参数，按相同公式前向计算理论上完全一致。

### 43.4 弃用项（MVP 不再使用）

| 项目 | 处理 |
|------|------|
| `src/color/processing.rs::extract_film_curve_16` | 弃用主路径 |
| `src/color/processing.rs::FILM_CURVE_LUT_{R,G,B}` 硬编码常量 | **完全删除**（错误方向的遗物）|
| `--use-ref-lut` CLI flag（tif_compare） | MVP 验证期后移除 |
| C-41 从 thumbnail 提取曲线（最近 commit `7bd48d0`）| 弃用，CGammaNegCurve 公式 §18.7 已有 |
| "near-identity extraction" 近似检测 | 不再需要 |

### 43.5 保留项（价值不变）

| 项目 | 新定位 |
|------|--------|
| **`tif_compare` 工具** | ★ 从"逆向反推工具"升级为 **validation harness**。所有 16-bit 指标、manifest、trace_pixel 完全保留 |
| `ΔE2000` + banded MAE + signed ME 指标 | MVP 验证核心 |
| `examples/test_cases.toml` (29 case) | Tier 0-5 验证矩阵 |
| `trace_pixel` 单像素追踪 | 关键 bug 定位工具 |
| USM calibration（σ=radius/20, gain=amount/67）| 沿用到 USM 真实公式回来前的占位 |

### 43.6 保留作"诊断工具"（非生产路径）

`extract_film_curve_16` 等反推代码**可保留**作 **error attribution** 工具：

- **用途**：当我们的 forward-computed output 与 ref 偏差 > 阈值时，从 ref 反推 LUT，与我们计算的 LUT 做 per-entry diff → 定位"曲线哪一段偏差最大"
- **定位**：放在 `tools/debug/` 或 `examples/` 下，加明确"诊断用途"注释
- **不进入生产 pipeline**

### 43.7 对 §33 测试策略的影响

§33 的 Tier 0-5 验证路径不变，但评估标准从"WARN 可接受"强化为"STRICT 目标"：

- **Tier 0 (Identity)** 必须 STRICT — 任何偏差都是 ICC profile 装载 bug
- **Tier 1 (单 slider)** 应 STRICT — 偏差说明公式实现有 bug
- **Tier 3 (负片)** 应 STRICT — CGammaNeg/CNegativeCurve 公式已全解
- **Tier 4 (CMYK/BW 输出)** 多为 WARN — ICC 输出 profile 的 lcms2 vs Windows ICM 差异

"STRICT 不可达就接受 WARN" 的旧思维在**公式完整已知**后应放弃。

### 43.8 对架构认识的启示

这个 reframe 揭示了一个 RE 普适教训：

> 当你能反推一个函数的行为，又同时有能力静态分析它的实现时，**静态分析永远优先**。反推只能得到"当前输入下的近似",静态分析得到"所有输入下的精确".

早期反推路径不是完全浪费——
- 它搭建了 `tif_compare`（现在是关键 validation tool）
- 它暴露了"不同 preset 无法用单一 LUT 覆盖"的问题，促使转向深度逆向
- 它证明了 "YCbCr 假设"（§二）是错的——在扫描仪数据上 ref-LUT 反推永远是 RGB-空间的近似

但继续走下去会不断标定新 preset、修补 LUT、永远达不到 bit-accurate。**是 §37/§42 的深度逆向让我们看到了正确的路**。

### 43.9 行动清单

Rust MVP 实施时（§34 Phase 3 集成阶段）：
1. 新 pipeline 按 §42 forward-compute LUT
2. 用 tif_compare 验证 bit-accurate 匹配
3. 通过 Tier 0-3 的所有 case（除 🟡 USM/ColorCorr 相关外）
4. 删除 `FILM_CURVE_LUT_{R,G,B}` + `extract_film_curve_16` 的生产引用
5. `--use-ref-lut` 保留到 diagnostic，或完全删除

---

## 44. T20 Agent 成果 — Planckian 表完整 dump + CFilmCurve 僵尸确认

agent `a0e044252c9ff5a98` 产出。

### 44.1 Planckian locus 表 @ 0x706e2410（CIE 1960 UCS u,v 空间）

**29 行 × 4 doubles = 928 bytes**。每行 = `(mired, u, v, slope)`，对应 Robertson 1968 等温线表。

```rust
/// FlexColor 的 Planckian locus 表（CIE 1960 UCS u,v 空间）
/// 每行 = (mired, u, v, slope)，mired = 1e6/K
pub const PLANCKIAN_LOCUS: [(f64, f64, f64, f64); 29] = [
    (  0.0, 0.18006, 0.26352,  -0.24341),   // K=∞
    ( 10.0, 0.18066, 0.26589,  -0.25479),   // K=100000
    ( 20.0, 0.18133, 0.26846,  -0.26876),
    ( 30.0, 0.18208, 0.27119,  -0.28539),
    ( 40.0, 0.18293, 0.27407,  -0.30470),
    ( 50.0, 0.18388, 0.27709,  -0.32675),
    ( 60.0, 0.18494, 0.28021,  -0.35156),
    ( 70.0, 0.18611, 0.28342,  -0.37915),
    ( 80.0, 0.18740, 0.28668,  -0.40955),
    ( 90.0, 0.18880, 0.28997,  -0.44278),
    (100.0, 0.19032, 0.29326,  -0.47888),   // K=10000
    (125.0, 0.19462, 0.30141,  -0.58204),
    (150.0, 0.19962, 0.30921,  -0.70471),
    (175.0, 0.20525, 0.31647,  -0.84901),
    (200.0, 0.21142, 0.32312,  -1.01820),   // K=5000
    (225.0, 0.21807, 0.32909,  -1.21680),
    (250.0, 0.22511, 0.33439,  -1.45120),   // K=4000
    (275.0, 0.23247, 0.33904,  -1.72980),
    (300.0, 0.24010, 0.34308,  -2.06370),   // K≈3333
    (325.0, 0.24702, 0.34655,  -2.46810),
    (350.0, 0.25591, 0.34951,  -2.96410),
    (375.0, 0.26400, 0.35200,  -3.58140),
    (400.0, 0.27218, 0.35407,  -4.36330),   // K=2500
    (425.0, 0.28039, 0.35577,  -5.37620),
    (450.0, 0.28863, 0.35714,  -6.72620),
    (475.0, 0.29685, 0.35823,  -8.59550),
    (500.0, 0.30505, 0.35907, -11.32400),   // K=2000
    (525.0, 0.31320, 0.35968, -15.62800),
    (550.0, 0.32129, 0.36011, -23.32500),   // K≈1818
];
```

**算法**（§42.4 完整版）：
```
1. mired = 1e6 / CT
2. 二分找 row_i 使 row_i.mired ≤ mired < row_{i+1}.mired
3. t = (row_{i+1}.mired - mired) / (row_{i+1}.mired - row_i.mired)
4. u = lerp(row_i.u, row_{i+1}.u, 1-t)
   v = lerp(row_i.v, row_{i+1}.v, 1-t)
   slope = lerp(row_i.slope, row_{i+1}.slope, 1-t)
5. Tint 偏移：
   # slope 是等温线切线方向；沿法线 + Tint 平移
   tint_mag = Tint / sqrt(1 + slope²)
   u += tint_mag
   v += tint_mag * slope
6. 转 (x, y): x = 3u / (2u - 8v + 4), y = 2v / (2u - 8v + 4)
7. chromaticity → RGB gains（经 {M, M^-1} matrix pair on CColorTempConversion）
```

### 44.2 CFilmCurve 是僵尸类 — Lightness 实际走 CPointCurve

**T18 与 T16 貌似矛盾的真相**：

- T18 正确：`CFilmCurve::vftable @ 0x7071d84c` 只有 1 slot（scalar deleting destructor）
- T16 误认：`FUN_702d4720` 里调的 slot 12 是 **CPointCurve** 的，不是 CFilmCurve 的
- 对 `CFilmCurve::vftable` 的全局 xref **只有 1 处**（CFilmCurve 自己的析构 thunk）

**CFilmCurve 从未被实例化**。至少在 v4.8.9.1 里，它是僵尸类，可能是旧版本残留。

### 44.3 CImageCorrection+0x84 确认是 CPointCurve

从 `FUN_702d5a20`（CImageCorrection 曲线初始化）：
```c
piVar2 = operator_new(0xf8);           // 0xf8 = CPointCurve 实例大小
piVar2 = FUN_70268c40(piVar2);         // CPointCurve::ctor
*(int**)(this + 0x84) = piVar2;        // 写入 this+0x84
FUN_702693f0(piVar2, CCurvePoint::vftable, 0, 0x0202, 1.0);   // 点 (2, 2)
FUN_702693f0(piVar2, CCurvePoint::vftable, 0, 0x6464, 1.0);   // 点 (100, 100)
```

所以 `CImageCorrection.field_0x84` = **CPointCurve** 实例，默认 2 个端点 (2, 2) 和 (100, 100) — 近 identity。

### 44.4 CPointCurve LUT 是动态构建

**关键**：LUT **不在 .rdata，也不能静态 dump**。每次按控制点由 `FUN_70268fc0`（spline 插值）在堆上构建：

```c
// CPointCurve slot 13 (0x70269f50) — 懒加载
if (!this.ready) {
    if (this.lut_ptr == NULL) this.lut_ptr = malloc(0x8000);   // 32768 bytes = 16384 × u16
    this.vtable[slot_8_evaluate](this.lut_ptr);                // 按控制点填 LUT
    this.ready = true;
}
```

slot 12 (`0x702661f0`) 读该堆 LUT：`return lut[x]`。

### 44.5 默认 Lightness 为何"无效"

如果 XML 没给 CPointCurve at 0x84 加额外控制点，默认只有 (2, 2) 和 (100, 100) — 近 identity 线段。此时 `curve(raw) - raw ≈ 0`，即 `delta ≈ 0`，Lightness 公式的"加 delta 到 RGB"没效果。

**这解释了为什么 Lightness 调到非 0 时仍看似"无效"** — 只有当 XML/UI 往 CPointCurve 注入更复杂的控制点（画出了 Shadow Depth 曲线），才会产生非零 delta。

**Rust MVP 需确认**：XML 的哪个字段把控制点注入 CPointCurve+0x84。可能是：
- Settings preset XML 里某个 `<PointCurve>` 节点
- 或 UI 预设（菜单选项）触发时动态加载

### 44.6 Rust MVP 可用性汇总

| 数据 | 可用性 | 如何用 |
|------|-------|-------|
| **Planckian locus 表** | 🟢 完全可用 | Copy §44.1 的 Rust 常量，§42.4 算法 |
| **CPointCurve slot 8 LUT 构建** | 🟡 算法未完全读 | MVP 先用 Catmull-Rom 或线性插值占位，tif_compare 验证后调整 |
| **CPointCurve 控制点加载源** | 🔴 未追踪 | 默认情况 Lightness 无效；完整实现需找 XML setter |

### 44.7 关键地址速查

| 符号 | 地址 |
|------|------|
| Planckian locus 表起始 | 0x706e2410 |
| Planckian locus 表末 | 0x706e27b0 |
| CFilmCurve vftable (僵尸) | 0x7071d84c |
| CPointCurve vftable | 0x707198b4 |
| CPointCurve ctor | 0x70268c40 |
| CPointCurve slot 12 (查 LUT) | 0x702661f0 |
| CPointCurve slot 13 (懒加载) | 0x70269f50 |
| CPointCurve slot 8 (LUT filler) | 0x70268fc0 |
| CImageCorrection ctor | 0x702d4360 |
| CImageCorrection curve init | 0x702d5a20 |
| Lightness 主循环 | 0x702d4720 |

---

## 45. T6 Agent — HasDeviceLink64.dll 排除（非 holy grail）

agent `a6e64289bac6d40c0` 产出。**结论：此 DLL 与色彩管线无关**。

### 45.1 真实身份

**HasDeviceLink64.dll (64 KB) 是 Hasselblad Flextight 扫描仪的 SCSI 硬件 I/O 桥**。"DeviceLink" 是 "设备连接" 不是 ICC DeviceLink profile。

- 路径：`/Users/will/vmwareShare/FlexColor v4.8.9.1/Misc/HasDeviceLink64.dll`
- PDB：`E:\FlexColor488\FLEXCOLOR\NextGenWin\bin\x64\Release\HasDeviceLink64.pdb`
- 配套：`HasDeviceLinkMFC64.exe`（托盘小程序 + MFC GUI）

### 45.2 铁证：5 个导出全部 IPC

```
InitServer / Service / CloseServer / SendCloseServer / IsServerRuning
```

**零 ICC 相关**。导入表只有 kernel32（`CreateFileW` + `DeviceIoControl` 等）、user32 (`wsprintfW`)、MSVCR90/MSVCP90。**无 mscms.dll / icm32.dll / gdi32.dll**。没 `CreateColorTransform*`、`OpenColorProfile*`、`TranslateColors`、`IccSaveProfile`。

### 45.3 架构图（FlexColor 的硬件访问）

```
FlexColor.exe / FlexColor.dll  (CHWInterfaceSCSIScanPC client)
         │
         │  共享内存 + 信号量 IPC
         ▼
HasDeviceLinkMFC64.exe  (托盘 server 进程)
         │
         │  InitServer/Service exports
         ▼
HasDeviceLink64.dll  (server 实现)
         │
         │  DeviceIoControl 到 \\.\ScsiScanX
         ▼
scsiscan.sys  (INF/ 目录下的内核驱动)
         │
         ▼
Flextight 扫描仪硬件
```

### 45.4 对项目的影响

- **Holy grail 路径否定**：无法用 DeviceLink ICC 烘焙整条 pipeline
- **研究范围收敛**：HasDeviceLink* 系列可从 RE 范围里移除
- **架构澄清**：FlexColor 的硬件 I/O 与色彩处理完全解耦，Rust 复刻只需关注 `FlexColor.dll` 的色彩算法
- **测试数据来源明确**：test1*.fff 的 raw 像素已经是扫描仪输出；色彩还原就是 FlexColor.dll 的工作

### 45.5 副产物

agent 装了 `pefile` Python 库（`pip3 install --user --break-system-packages pefile`），后续若需要分析其他 PE 文件可复用。

---

## 46. T10 Rust 实现进度（2026-04-19）

### 46.1 已完成：Phase 1-4

**Phase 1** — Curve 类族（`src/color/flex/curves/`）：
- `contrast.rs` ContrastCurve — §13 S-curve + brightness zone
- `gamma.rs` GammaCurve + GammaNegCurve — §13 正片 pow + §18.7 `pow(1-v², 1/γ)` 勘误版
- `negative.rs` NegativeCurve — §15 2 段二次 + §18.8 per-channel 默认
- `aggregate.rs` AggregateCurve — §16.4 勘误版 3-mode composer（`last_m0` 正确）
- `high_shadow.rs` HighShadowCurve — §28 3-zone + §28.2 set_params
- `point.rs` PointCurve — §16.2 用户曲线 + §16.8 "采样+线性填充"
- `single_point.rs` SinglePointCurve — §16.1

**Phase 2** — 顶层 Pipeline（`src/color/flex/pipeline.rs`）：
- `Pipeline::build(ic)` 按 §16.5 拓扑组装 outer_agg×3 通道
- `Pipeline::apply_14bit_rgb` / `apply_16bit_rgb` 像素应用

**Phase 3** — 集成（`src/color/flex_apply.rs` + `examples/tif_compare.rs`）：
- `apply_flex_pipeline(img, ic, icc, target, settings)` DynamicImage 桥接
- `--flex-pipeline` CLI flag 启用 T6 测试
- T6 链：flex::Pipeline → ICC (含 CMYK 回退) → BW desat → USM

**Phase 4** — 验证（29 case manifest）：当前结果见 §46.3

**测试**：96 个单元测试全绿（covers 所有曲线 + Pipeline + flex_apply 桥接）。

### 46.2 B-spline 依赖

Cox-de Boor 递归已实现在 `src/color/flex/bspline.rs`（§16.9）。当前 PointCurve 对 2 点（identity 常见）走纯线性；对 >2 点走 cubic B-spline clamped knots。**FlexColor 实际 knot vector 与 order 未完全证实**，多点情况可能有偏差。

### 46.3 29 case 结果汇总（迭代版）

**v1** = 纯 Pipeline（Phase 3 集成后）。**v4** = 加 T21 BW gate 修复 + T24 Lightness，T22 ColorCorr 保留代码但禁用。

| Grade | v1 | v4 | 变化 |
|-------|----|----|------|
| 🟢 STRICT | 2 | 2 | rgb_standard×2 稳定 |
| ⚠️ WARN | 1 | **2** | 新：e2e_all_config_bw (-92%!) |
| ❌ FAIL | 26 | 25 | 整体 MAE 下降 |

**关键 case 对比（MAE16）**：

| Case | v1 baseline | v4 | Δ |
|------|-----|-----|----|
| emb_rgb_standard | 92 STRICT | 92 STRICT | — |
| emb_rgb_dark | 2369 | **1940** | **-18%** |
| **emb_bw_neg_standard** | **13046** | **3844** | **-70%** ✨ |
| **e2e_all_config_bw** | **12629** | **990 WARN** | **-92%** ✨ |
| emb_cmyk_dark | 4148 | 3569 | -14% |
| emb_cmyk_dark_saturated | 3467 | 2890 | -17% |
| e2e_all_config | 3798 | 3798 | — (T22 禁用) |

### 46.3.1 实际跑 tif_compare 命令

```bash
cargo run --release --example tif_compare -- --manifest examples/test_cases.toml --flex-pipeline
```

### 46.4 T6 vs T1（ref-LUT cheat）对比 — §43 验证

| Case | T1 (cheat) | T6 (flex) | 改善 |
|------|---|---|---|
| emb_rgb_standard | **92** | **92** | 持平（都 STRICT）|
| emb_rgb_dark | 5771 | **2369** | **-59%** |
| emb_rgb_saturated | 3992 | **2570** | **-36%** |
| emb_neg_rgb_standard | 4323 | **931** | **-78%** |
| **e2e_all_config** | **13768** | **3798** | **-72%** ✨ |

**结论**：§43 的"前向计算优于从 ref 反推"**得到强力验证**。flex Pipeline 在非 identity case 上**全面优于 T1 cheat**。`e2e_all_config`（最复杂 case，激活所有 FlexColor 特性）T6 MAE **仅为 T1 的 28%**。

### 46.5 已知缺失（待办）

按 MAE 影响从大到小：

| 缺失项 | 影响 case | 预期降低 MAE |
|---------|---------|--------------|
| **BW negative 专用路径** | 2 个 BW case (~13000 MAE) | → 2000 以下 |
| **ApplyCC 6×6 matrix** (§29.3, §42.2) | saturated/dark/all_config 共 10+ cases | → 50% 降低 |
| **EndPoints shadow_out > 0** | complex configs（e2e_all_config 等）| → 30% 降低 |
| **Lightness via CPointCurve@0x84** | dark cases (~2300 MAE) | → 500 以下 |
| **CPointCurve 多点 B-spline 精度** | user-editing cases（当前 test 多为 identity）| MVP 后再看 |
| **ColorTemperature/Tint WB** (§42.4) | 用户调 WB 的场景 | 视 case 而定 |

### 46.6 代码组织一览

```
src/color/
├── mod.rs                    # pub re-exports
├── profile.rs                # ICC 扫描（不动）
├── transform.rs              # ICC transform（不动）
├── processing.rs             # 旧 pipeline（保留作对比基线）
├── adjust.rs                 # ManualAdjust（桥接用）
├── usm.rs                    # USM（flex 仍用此）
├── flex/                     # ★ T10 新模块
│   ├── mod.rs
│   ├── bspline.rs            # Cox-de Boor
│   ├── pipeline.rs           # Pipeline 顶层
│   └── curves/
│       ├── mod.rs            # Curve trait + CompositionMode
│       ├── contrast.rs
│       ├── gamma.rs
│       ├── negative.rs
│       ├── aggregate.rs
│       ├── high_shadow.rs
│       ├── point.rs
│       └── single_point.rs
└── flex_apply.rs             # ★ DynamicImage 桥接
```

---

## 47. 待研究：FlexColor 如何做剩余 3 大 feature

基于当前 §46.5 缺失清单，规划下一阶段的研究问题。

### 47.1 BW Negative 专用路径（最大单项影响）

**现象**（§46.3 trace）：
- 参考：`[48522, 48522, 48522]`（gray）
- T6 flex: `[19348, 33348, 51024]`（per-channel 巨大差异）
- 即便后续 BW desat 平均化，整体过暗 ~14000 LSB

**疑问**：
- FlexColor 对 BW negative (film_type=2) 用什么公式？`pow(1-v², 1/γ)` 还是 linear `(hi-v)/hi`？
- BW negative 是否跳过 CGammaNegCurve 走另一路径？
- 是否在 raw → positive 前先做 luma collapse？

**研究方法**：
1. Ghidra 查 `film_type` 字段读者（非 CGammaNeg build）
2. 搜字符串 `"film_type"`, `"BW"`, `"Grayscale"`, `"LumaConversion"`
3. 看 `FUN_70512fb0`（§37 shoulder LUT builder）是否按 film_type 分支
4. 对比 FlexColor 的 Hasselblad Gray profile apply 时机

**输出**：BW negative 从 raw → positive 的精确算法。

### 47.2 ApplyCC 6×6 Matrix 应用时机与公式

**现状** (§29.3, §42.2)：
- CColorCorrection 类包含 6×6 double matrix + 2 × 64K ushort LUT
- Saturation 可能作为矩阵的"中性偏移"（§42.2：identity check 是"all cells == -sat"）
- 但**没确认**：矩阵在 pipeline 哪一步 apply？输入是 RGB 还是 6 通道（RGB + CMY）？

**疑问**：
- Matrix 作用空间：RGB？6 通道（扩展的 CMYRGB）？
- Apply 时机：post-LUT？pre-LUT？独立阶段？
- Saturation 如何与 matrix 数据融合（替换/叠加）？
- Matrix 应用的具体运算：`out = M × [R, G, B, C, M, Y]ᵀ`？还是更复杂？

**研究方法**：
1. Ghidra 找读 `this+0x4b4..0x4fb`（36 short 矩阵）的像素路径
2. 搜 `CColorCorrection` 的非虚函数（类只有 1 个 slot 析构，真正的计算可能在独立 function）
3. 检查 `FUN_702d4b50` (§37.6 post_LUT color correction 推测) 是否消费此矩阵
4. 理解 R/G/B/C/M/Y 6 通道的构造（可能 `C=1-R, M=1-G, Y=1-B`？）

**输出**：6×6 matrix apply 的精确算法 + Saturation 融合方式。

### 47.3 EndPoints 与 Shadow/Highlight 协同

**现状** (§28.3)：
- 当前 Pipeline 里 shadow_out/highlight_out 来自 dot_color 的前 3 字节 + 7-9 字节
- 默认 dot_color=[0,0,0,0,...0, 255,255,255,...255] 正常工作
- 但 e2e_all_config 有 dot_color=[60,60,60,...180,180,180] 未必是我们实现的"shadow_out = 60 × 16383/255 = 3856"

**疑问**：
- DotColor 数组的 14 个元素的完整语义（前 7 个 shadow 块 vs 后 7 个 highlight 块）
- shadow_out 映射是 `byte × 16383/255` 线性还是有 gamma?
- DotColor 是否还影响 HighShadowCurve 之外的 pipeline 阶段？

**研究方法**：
1. Ghidra 找 this+0x510/0x514（§28.2 的 shadow_mode/hi_mode）读者
2. DotColor XML 字段的字节级布局（结合 §26.6 slot 0 XML reader 的相关分析）
3. 对比 e2e_all_config 的 ref 输出 vs 我们修正后的输出

**输出**：DotColor 14 字节精确布局 + shadow_out/hi_out 映射公式。

### 47.4 Lightness via CPointCurve @ +0x84

**现状** (§42.3, §44.3)：
- Lightness 公式已解：`out += (point_curve(raw) - raw)`，RGB 同加 delta
- CPointCurve at `this+0x84` 默认 (2, 2) 和 (100, 100) 两个点（近 identity）
- **未知**：XML/UI 如何注入额外控制点让 Lightness 产生效果

**疑问**：
- Lightness slider 值如何转化为 CPointCurve 控制点？
  - 可能：slider 值 → preset 查表（类似 FilmCurve 的映射）
  - 可能：Shadow Depth UI 加非默认点
- 控制点注入路径是内部（从 Lightness int）还是外部（XML `<PointCurve>` 节点）？

**研究方法**：
1. Ghidra 找 `this+0x96c`（Lightness）的所有读者（§42.4 已列 FUN_702d4720 和 FUN_702d6ce0）
2. `FUN_702d6ce0` — T16 说它是 preset 曲线 descriptor，`iVar4 = Lightness * 250; offset = iVar4/100 - ... + 2`。这个 offset 可能是 preset 索引 → 查表得控制点！
3. 如果找到控制点查表，整个 Lightness pipeline 就通了

**输出**：Lightness 值 → CPointCurve@+0x84 控制点的映射算法。

### 47.5 优先级建议

| # | 任务 | 预期 MAE 降低 | 工作量 |
|---|------|--------|--------|
| 1 | BW negative 专用路径（T21）| 2 case × 11000 = 22000 | 中（Ghidra 挖 + Rust 实现）|
| 2 | ApplyCC 6×6 matrix（T22）| 10 case × 1000 ≈ 10000 | 中+（复杂数学 + Ghidra）|
| 3 | DotColor shadow_out>0（T23）| 5 case × 500 ≈ 2500 | 低（主要 Rust 小改）|
| 4 | Lightness CPointCurve 注入（T24）| 6 case × 800 ≈ 4800 | 中（Ghidra 挖 FUN_702d6ce0）|

建议顺序：**3 → 4 → 1 → 2**（先改简单、累积见效、再攻坚难点）。

### 47.6 研究资源

- **Ghidra 查询工具**：`tools/ghidra_query/run.sh`（含 §42.8 新增 `find-offset-refs`）
- **测试 harness**：`tif_compare --flex-pipeline --trace x,y` 单像素定位
- **可构造 minimal XML**（§33.4）隔离单一特性
- **已完成 agent 模板**（审查/深挖类任务可复用）

---

## 48. T21 · BW Negative 完整路径（2026-04-19 完成）

agent `a0733a49abb89d0e9` 产出。

### 48.1 BW 与 color neg 的**唯一算法差异**

FlexColor 对 BW（film_type=2）与 color neg（film_type=1）**共用 CGammaNegCurve**（`pow(1-v², 1/γ) × 16383`）。差异只在两处：

1. **CNegativeCurve gate 严格 `== 1`**（FUN_70266ac0）：BW **跳过** 2 段二次曲线
2. **Mode==2/5 → RGB→Gray collapse**（FUN_702d90b0）：输出走 Hasselblad Gray.icc 的 kTRC + vtable[0x48] 灰度 writer

### 48.2 核心 bug 修复（已应用）

```rust
// 错误：过去 shared Neg 对 BW 也启用
if ic.film_type != 0 { NegativeCurve::default_shared() } else { disabled() }

// 正确：严格 FilmType==1 才启用
if ic.film_type == 1 { NegativeCurve::default_shared() } else { disabled() }
```

`src/color/flex/pipeline.rs` 已修。per-channel Neg 本来就是 `== 1` gate，无需改。

### 48.3 Mode 枚举完整解读

| Mode | 用途 | output writer |
|------|------|--------------|
| 0 | RGB 标准 | vtable[0x10] 3-channel |
| 2 | **Grayscale（BW output）** ★ | vtable[0x48] 1-channel via vtable[0x1c] collapse |
| 3 | Lineart 二值化 | Threshold (0x11f0) 硬阈值 |
| 4 | 8-bit SoftProof 分支 | (特殊) |
| 5 | Grayscale 变体 | 同 Mode=2 |

**BW preset XML**（`Standard Negative/B&W negative standard.xml`）：`<FilmType>2</FilmType>` + `<Mode>2</Mode>`。两者配对但在代码中**独立生效**。

### 48.4 Gray ICC 输出（待 Rust 集成）

vtable[0x1c] RGB→scalar 实为 **Hasselblad Gray.icc kTRC**。MVP 可用简单的 BT.601 luma 近似；完美对齐需 lcms2 装载 GrayProfile 字段（默认 `.dfG:`）。

### 48.5 关键地址

| 地址 | 角色 | gate |
|------|------|------|
| **0x70266ac0** | CNegativeCurve 14-bit builder | `FilmType == 1` |
| 0x70266ca0 | 8-bit 变体 | 同上 |
| 0x702664e0 | CGammaNegCurve 14-bit builder | `FilmType != 0`（color + BW 都用）|
| **0x702d90b0** | 主 16-bit pipeline loop + Mode gate | `Mode == 2 \|\| Mode == 5` |
| 0x702dc370 | 8-bit CMYK tile loop + Mode gate | 同上 |
| 0x7026a460 | Mode=3 lineart threshold | `Mode == 3` |

---

## 49. T22 · ApplyCC 6×6 ColorCorr Matrix 完整机制（2026-04-19 完成）

agent `af96a792dc6e99e57` 产出。**重大发现：不是 6 通道矩阵应用，而是 3 通道 opponent-excess 色彩减法**。

### 49.1 真实算法

**步骤 1（setup 期）**：`FUN_702d57b0` 编译 **6×6 → 3×6**

`this+0x4b4..0x4fb` 的 36 个 int16 + Saturation (`this+0x4fc`) → `this+0x976..0x999` 的 **18 个 int16 编译矩阵**。每个编译单元 = 3 个源单元之和，Saturation 烘焙进特定单元。

**步骤 2（每像素 apply 期）**：`FUN_702d4b50` 

```python
def apply_color_correction(rgb: [u16; 3], M3x6: [[i16; 6]; 3]) -> [u16; 3]:
    r, g, b = rgb
    # 从 RGB 计算 6 个 opponent-excess 色彩项
    c = [
        max(0, b - max(r, g)),        # pure blue excess
        max(0, g - max(r, b)),        # pure green excess
        max(0, min(r, g) - b),        # yellow content
        max(0, min(r, b) - g),        # magenta content
        max(0, min(g, b) - r),        # cyan content
        max(0, r - max(g, b)),        # pure red excess
    ]
    # 每通道：delta = -dot(M3x6[ch], c) / 100
    out = []
    for ch in [0, 1, 2]:
        delta = round(sum(M3x6[ch][k] * c[k] for k in 0..6) / 100.0)
        out.append(clamp14(rgb[ch] - delta))
    return out
```

### 49.2 关键观察

- **不是 `M × [RGB CMY]ᵀ` 6 通道**，是 **M × [chroma_excess_6 项]** → 3 通道 delta
- **Saturation 烘焙进矩阵**（半数编译单元 +Sat，半数不变），fast-path: `all cells == -Sat`
- **矩阵单元 = 百分比单位**（除以 100.0 from `_DAT_70733750`）
- **减法式**（`out = in - delta`，不是线性变换）

### 49.3 Apply 时机

```text
per-channel LUT apply (§37.1 flex::Pipeline)
    ↓
FUN_702d4b50 (ApplyCC)           ← 这一阶段
    ↓
FUN_702d4720 (Lightness/Shadow Depth)
    ↓
Flip + USM
```

即 **post-LUT，pre-Lightness**。

### 49.4 Identity fast-path

`FUN_702d4f30` 检查 `any(M6x6[i][j] != -Sat)`。默认矩阵全 0 + Sat=0 → 所有 cells == 0 == -0 → fast-path 触发（**跳过**整个 apply）。

### 49.5 Rust 实现建议

新建 `src/color/flex/color_correction.rs`：

```rust
pub struct ColorCorrParams {
    pub matrix: [[i16; 6]; 6],    // ImageCorrection.color_corr parsed to 6×6
    pub saturation: i16,           // ImageCorrection.saturation
    pub apply_cc: bool,            // ImageCorrection.apply_cc
}

pub fn is_customized(p: &ColorCorrParams) -> bool {
    p.matrix.iter().flatten().any(|&c| c != -p.saturation)
}

/// Compile 6×6 + Sat → 3×6 (镜像 FUN_702d57b0)
pub fn compile_3x6(p: &ColorCorrParams) -> [[i16; 6]; 3] { ... }

/// Apply per-pixel (镜像 FUN_702d4b50)，14-bit domain
pub fn apply(pixels: &mut [u16], m3x6: &[[i16; 6]; 3]) { ... }
```

在 `Pipeline::apply_16bit_rgb` 里，**LUT 应用后**调 ColorCorrection.apply（前提 ApplyCC && is_customized）。

### 49.6 待确认点

**推测部分**（需 round-trip pixel test 确认）：
- 6 个 opponent-excess 项的确切计算顺序与编译矩阵列的对应关系
- Sat 添加到哪些具体的 3×6 cells（pattern 已知，但第几行第几列待验）

### 49.7 关键地址

| 地址 | 角色 |
|------|------|
| **0x702d4b50** | ★ Per-pixel apply |
| **0x702d57b0** | ★ 6×6 → 3×6 编译器 |
| 0x702d4f30 | Identity check (`any != -Sat`) |
| 0x702d6630 | InitDefaults 零矩阵 |
| 0x70733750 | const 100.0 divisor |
| this+0x4b4 | 6×6 源矩阵（36 × int16）|
| this+0x4fc | Saturation (int16) |
| this+0x976 | 3×6 编译矩阵（18 × int16）|
| this+0x88 | ApplyCC (byte) |

---

## 50. T24 · Lightness CPointCurve@+0x84 精确公式（2026-04-19 完成）

agent `a8173627d4a5bf273` 产出。**无 preset 表，硬编码 4 点 + Lightness 驱动 Point[1].Y**。

### 50.1 CPointCurve@+0x84 的真实初始化

**两阶段追加**（T20 早期报告不完整）：

1. `FUN_70269600` (CPointCurve::Init): 插入 (0, 0) 和 (255, 255)
2. `FUN_702d5a20` (CImageCorrection setup): 追加 (2, 2) 和 (100, 100)

最终 **4 点曲线**（sorted by X）：
```
[(0, 0), (2, 2), (100, 100), (255, 255)]
```

### 50.2 Lightness 注入公式（FUN_702d6ce0）

```c
void apply_lightness_to_curve(CImageCorrection* self) {
    int Lightness = (short)self->[0x96c];
    int mid_y = floor(Lightness * 2.5) + 2;    // = floor(Lightness * 250 / 100) + 2
    
    CPointCurve* cpc = self->[0x84];
    if (cpc->nPoints > 1) {
        cpc->points[1].X = 2;          // 保持 X=2
        cpc->points[1].Y = (u8)mid_y;  // ★ Lightness 改写的唯一 Y 值
    }
    if (cpc->nPoints > 2) {
        cpc->points[2].X = 50;         // (或 DAT_707b6b60 非零时用它)
        cpc->points[2].Y = 50;         // 固定 shadow anchor
    }
    // 触发 LUT rebuild (vtable[13])
}
```

**Lightness → Point[1].Y 表**：
| Lightness | Point[1] = (2, Y) | 意义 |
|-----------|-------------------|------|
| 0 | (2, 2) | identity（默认）|
| 20 | (2, 52) | 暗部 lift 25% |
| 50 | (2, 127) | 暗部 lift 50%（"中等"）|
| 100 | (2, 252) | 暗部 lift 全量（饱和前）|

### 50.3 pipeline apply（FUN_702d4720）

gate: `ApplySliders && Lightness > 0 && CPointCurve@+0x84 != NULL`

```c
for each pixel {
    ushort raw_curve_input = ... // 某通道或 luma
    ushort y = CPointCurve_lookup(cpc, raw_curve_input);
    short delta = y - raw_curve_input;
    // RGB 三通道同加 delta
    out[0] = clamp14(out[0] + delta);
    out[1] = clamp14(out[1] + delta);
    out[2] = clamp14(out[2] + delta);
}
```

**关键**：Lightness=0 严格跳过（> 0 strict gate）。

### 50.4 "Shadow Depth" 形态

因为 Point[1].X=2（byte 空间，14-bit ≈ 128），所以 Lightness 只 **lift 极暗像素（14-bit 0..128）**。中间/高光（>= 50 byte = 3200 14-bit）走 (50,50)→(255,255) 近 identity。这完全匹配 "Shadow Depth" UI 名称。

### 50.5 Rust 实现建议

```rust
// 在 Pipeline 或单独模块
pub struct LightnessCurve {
    pub lightness: i16,       // ImageCorrection.lightness
    pub apply_sliders: bool,  // ImageCorrection.apply_sliders
    lut: [u16; 16384],        // 预计算 (byte domain 0..255 ↔ 14-bit 0..16383)
}

impl LightnessCurve {
    pub fn new(lightness: i16, apply_sliders: bool) -> Self {
        // 构造 4 点曲线：(0,0), (2, Y1), (50, 50), (255, 255)
        // Y1 = min(Lightness * 2.5 + 2, 255)
        // 线性分段插值填 16384 entries
        ...
    }
    
    pub fn apply(&self, rgb: &mut [u16]) {
        if !self.apply_sliders || self.lightness <= 0 { return; }
        for chunk in rgb.chunks_exact_mut(3) {
            // raw_curve_input: 某通道或 luma（待确认）
            let luma = (chunk[0] as u32 + chunk[1] as u32 + chunk[2] as u32) / 3;
            let delta = self.lut[luma.min(16383) as usize] as i32 - luma as i32;
            chunk[0] = (chunk[0] as i32 + delta).clamp(0, 16383) as u16;
            chunk[1] = (chunk[1] as i32 + delta).clamp(0, 16383) as u16;
            chunk[2] = (chunk[2] as i32 + delta).clamp(0, 16383) as u16;
        }
    }
}
```

在 Pipeline apply 里 **ColorCorrection 之后、USM 之前** 调用。

### 50.6 关键地址

| 地址 | 角色 |
|------|------|
| **0x702d6ce0** | Lightness inject (改写 Point[1]) |
| **0x702d4720** | Apply per-row (读 CPointCurve，加 delta 到 RGB) |
| 0x702d5a20 | curve setup（追加 2 点）|
| 0x70269600 | CPointCurve::Init（初始 2 点）|
| 0x702693f0 | CPointCurve::AddPoint (sorted insert) |
| 0x70268fc0 | BuildLUT14 (vtable[8]) |
| 0x702661f0 | LUT query (vtable[12]) |
| this+0x96c | Lightness (int16) |
| this+0x1218 | ApplySliders (byte) |
| this+0x84 | CPointCurve* ptr |

---

---

## 51. T22-follow 验证结果（2026-04-20）

### 51.1 T22-follow 反编译结果

T22-follow agent 完整反编译 `FUN_702d57b0`，给出 18-cell compile pattern：

```
cols[0] = {1, 2, 3}   // R output: M3x6[0][k] = Σ_{j∈cols[0]} M6x6[k][j] + (Sat if k∈cols[0] else 0)
cols[1] = {0, 2, 4}   // G output
cols[2] = {0, 1, 5}   // B output
```

对称式：`cols[o] = ({0,1,2}\{o}) ∪ {o+3}`。已实现于 `color_correction.rs::ColorCorrection::compile`。

### 51.2 Apply 测试结果

开启 `should_apply = apply_cc && is_customized` 后 manifest 结果：

| 指标 | 禁用（baseline） | T22 启用 | Δ |
|------|-----------------|----------|----|
| STRICT | 6 | 6 | 0 |
| WARN   | 5 | 7 | +2 |
| FAIL   | 134 | 132 | -2 |
| T6 e2e_all_config MAE | 3798 | **8780** | +4982（回归）|
| T6 e2e_default MAE | 2009 | 1396 | -613 |
| T6 其他大多数 cases | — | 略有改善（~-100..200）| |

**结论**：compile pattern 对**绝大多数 cases**（ApplyCC=false 或小 Sat）保持不变或有小幅改善，
但 **e2e_all_config**（ApplyCC=true + 极端 Sat）产生 4982 MAE 大幅回归。

### 51.3 推测错误来源

候选问题（按可能性排序）：

1. **Per-pixel apply 公式 scale**：`delta = dot(M,chromas) / 100`。分母可能不是 100
   （可能 10000、128、256、或 bit-shift）。`FUN_702d4b50` 需要追看 arithmetic scale。
2. **Sign**：`out = in - delta`。可能应该是 `+ delta`，或因为 Sat 正负语义翻转。
3. **6 个 opponent-excess 公式**：对 R/G/B 三组 3 通道 opponent / 3 个 primary excess
   的划分、符号或 clamp 可能误解。
4. **compile pattern 小差异**：Sat 加到哪些 cell（currently `k ∈ cols[o]`），也可能是
   `k = o + 3` 单一 cell 或 inverted。

### 51.4 本轮处置

`should_apply` 强制返回 `false`（MVP），compile pattern 保留供 round-trip 验证。
这保持了 e2e_all_config = 3798 baseline，并避免 categorical 回归。

### 51.5 下一步研究方向

要 lock down 正确公式，需要：

**A. Round-trip 实验**：
1. 在 test XML 上手动设置 `ApplyCC=true, Sat=小值, 矩阵=单 cell`。
2. 跑 FlexColor 生成 ref TIFF。
3. 比较 ours vs ref 找出 delta scale / sign。

**B. Ghidra 继续追**：
1. `FUN_702d4b50` 逐行 decompile（特别是 6 chroma 项构造 + 除法/移位）。
2. 确认 Sat 进入 compile 时的 sign convention。

**C. 参考 FilmCurve preset 类比**：
本项目 FilmCurve 的常数分母是 10000（§T18），可能 ColorCorr 也用 10000。

### 51.6 影响 & 风险

- MVP 禁用无副作用（identity STRICT 保留，e2e_all_config baseline 保留）。
- 启用错误公式的风险：e2e_all_config 大幅回归，但多数其他 cases 小幅改善。净值为负（回归大于改善）。
- 正确启用的上限：**无法确定** — 需要 round-trip 才能估算。


---

## 52. T25 · 定位真实 ColorCorrection per-pixel apply（2026-04-20 完成）

### 52.1 Agent 结果（`FUN_702d4b50` R-row 字节级 decompile）

Agent 跑了 pyghidra `disasm` + `decompile`，在 R 输出行（0x702d4cba..0x702d4d6e）逐条 trace
FPU 栈，得出**正确的 chroma 顺序 + dot-product permutation**。

### 52.2 DLL chroma 栈槽顺序（c0..c5）

| 索引 | 公式 | 含义 |
|------|------|------|
| c0 | max(0, B − max(R,G)) | pure blue excess |
| c1 | max(0, G − max(R,B)) | pure green excess |
| c2 | max(0, R − max(G,B)) | **pure red excess** |
| c3 | max(0, min(R,G) − B) | **yellow** |
| c4 | max(0, min(R,B) − G) | **magenta** |
| c5 | max(0, min(G,B) − R) | **cyan** |

差异于我们原实现：c2..c5 的角色被错换。

### 52.3 Dot-product permutation（R row 验证，G/B 推定同）

```
delta_R = Σ_{k=0..5} m3x6[R][k] · c[PERM[k]] / 100.0
PERM = [2, 1, 0, 5, 4, 3]
```

即 M3x6 的 6 列对应 chroma：`[R_exc, G_exc, B_exc, cyan, magenta, yellow]`。
与 T22 compile pattern 的语义一致：
- cols[0]={1,2,3} (R out) = G_exc + B_exc + cyan — R 受"非 R"与其互补色调制
- cols[1]={0,2,4} (G out) = R_exc + B_exc + magenta
- cols[2]={0,1,5} (B out) = R_exc + G_exc + yellow

### 52.4 Rust 修复（已提交）

`src/color/flex/color_correction.rs::apply_rgb_chunk`：
- 重排 chroma 数组到 DLL 顺序
- dot-product 应用 PERM `[2,1,0,5,4,3]`
- 保留 `-delta`、`/100.0`、14-bit clamp

### 52.5 实测影响

| 指标 | T22 禁用（baseline） | T22 错 perm | T25 修复 |
|------|---------------------|-------------|----------|
| STRICT | 6 | 6 | **7** |
| PASS | 0 | 0 | **4** |
| WARN | 5 | 7 | 4 |
| FAIL | 134 | 132 | **130** |
| T6 e2e_all_config | 3798 | 8780 | 7203 (仍回归) |
| T6 emb_neg_rgb_standard | 931 | 1386 | **276 PASS** |
| T6 emb_neg_rgb_saturated | 1513 | 1889 | **304 PASS** |
| T6 ext_neg_rgb_standard  | 931 | 1386 | **276 PASS** |
| T6 ext_neg_rgb_saturated | 1513 | 1889 | **304 PASS** |

**净值正**：neg RGB 4 个 case 进入 PASS tier，1 个 ext_rgb_saturated 升 STRICT。

### 52.6 未验证 / 剩余研究

1. **G/B row permutation**：agent 只逐行验证 R-row FPU trace；G 从 0x702d4d84 开始带 `FMUL ST4`（依赖 R block 留在 FPU 栈的数据），**可能 permutation 不同**。
2. **FSUBRP 符号**：G block 0x702d4eae 是 `FSUBRP` 而非 `FSUBP`，可能 sign convention 翻转。
3. **e2e_all_config 残余 3405 回归**：可能源自 (1) 或 (2)，或 e2e_all_config 本身还有未模拟的步骤（ApplyCC XML 值极端）。

下一步：继续 agent 跑 `FUN_702d4b50` 的 G/B block 完整 FPU trace（0x702d4d84 + 0x702d4e27）。

### 52.7 关键地址

| 地址 | 角色 |
|------|------|
| **0x702d4b50** | ColorCorrection per-pixel apply loop |
| 0x702d4cba..0x702d4d6e | R-row dot-product（已验证 PERM=[2,1,0,5,4,3]）|
| 0x702d4d84+ | G-row dot-product（未 byte-trace）|
| 0x702d4e27+ | B-row dot-product（未 byte-trace）|
| 0x702d4ebd | 最终 clamp + pack to 14-bit |
| 0x70733750 | double 100.0（delta 分母）|
| this+0x976..0x998 | M3x6 (18 × int16)，行主 |

