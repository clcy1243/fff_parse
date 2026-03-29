# FFF Viewer

一个用于查看 Hasselblad / Imacon Flextight X5 扫描仪输出的 `.fff`（3F / Flexible File Format）文件的桌面应用程序。

基于 Rust + [egui](https://github.com/emilk/egui) 构建，无需依赖任何商业软件即可浏览扫描图像、查看 TIFF/EXIF 元数据、回溯 FlexColor 的完整编辑历史，应用 ICC 色彩配置文件，并导出为标准 TIFF 格式。通过逆向 FlexColor 的色彩处理管线，本应用可复现 FlexColor 对正片、彩色负片和黑白负片的渲染效果。

---

## 功能概览

### 浏览与查看

- **Lightroom 风格界面** — 左侧目录树（含收藏夹）、中央网格 / 胶片浏览双视图、右侧信息面板
- **图像预览** — 自动解码 FFF 文件中的未压缩 RGB 图像（8-bit / 16-bit），缩略图渐进式加载
- **元数据浏览** — 图像尺寸、色彩空间、扫描仪型号、软件版本等关键信息
- **FlexColor 编辑历史** — 解析嵌入的 Apple plist XML，展示每次编辑设置的名称、时间戳及详细校正参数
- **全部标签** — 列出所有 TIFF/EXIF 标签及 Hasselblad MakerNote 私有标签（支持筛选）
- **国际化** — 支持英语和简体中文界面切换（200+ UI 字符串）

### 色彩管理

- **ICC 配置文件** — 内置 15 个 FlexColor ICC 配置文件，支持从 FFF 文件中提取内嵌 ICC
- **设置预设** — 内置 123 个 FlexColor 设置预设（Standard、Film Specific 等分类）
- **目标色彩空间** — sRGB、Adobe RGB、ProPhoto RGB、Display P3
- **胶片类型** — 正片 (E-6)、彩色负片 (C-41)、黑白负片 三种胶片处理模式
- **实时预览** — 所有色彩调整实时反映到预览画面

### 色彩调整

- **色阶（Levels）** — Master + R/G/B 四通道独立调整，含自动色阶（基于 65536-bin 直方图百分位计算）
- **渐变曲线（Gradation Curves）** — RGB / R / G / B / C / M / Y 七通道交互式曲线编辑
- **曝光** — ±3.0 EV 档位（指数缩放 2^EV）
- **亮度 / 对比度 / 高光 / 阴影 / 中间调** — 全套色调控制
- **饱和度** — ITU-R BT.709 亮度保持模式
- **色温 / 色调** — 暖冷色温与色调偏移
- **色彩平衡** — R/G/B 三通道独立偏移
- **颜色校正矩阵** — 6×6 RGBCMY 矩阵（使用 3×3 RGB 部分）
- **输出色阶（DotColor）** — 输出范围黑白点映射

### 分割导出

- **胶片格式** — 24 种预设格式（35mm、6×6、6×7、4×5 等）+ 自由格式
- **多区域管理** — 添加、移动、调整大小、旋转裁切区域
- **双线性插值** — 旋转裁切时使用双线性插值保证质量
- **独立导出** — 每个区域独立导出为 TIFF，支持自定义命名模板

### TIFF 导出

- **单文件导出** — 保留 16-bit 色深，应用完整色彩管线
- **批量导出** — 一键导出当前目录所有 FFF 文件
- **进度显示** — 底部状态栏显示导出进度条和当前文件名

---

## 代码架构

### 项目结构

```
src/
├── main.rs              # 入口：命令行参数、日志系统、panic hook、eframe 窗口
├── lib.rs               # 公共模块导出
├── tiff.rs              # TIFF/FFF 二进制解析器（1911 行）
├── tags.rs              # 标签名称查找表：170+ 标准标签、15 个 MakerNote 标签
├── i18n.rs              # 国际化：200+ UI 字符串（英语 / 简体中文）
├── config.rs            # 全局配置：GPU、线程数、语言、收藏夹、直方图设置
├── sidecar.rs           # 逐文件持久化：色彩设置、分割区域、手动调整
├── parse_test.rs        # CLI 解析工具
├── color/               # 色彩处理引擎
│   ├── mod.rs           #   模块导出（含 apply_color_pipeline）
│   ├── profile.rs       #   ICC 配置文件扫描与元数据解析
│   ├── transform.rs     #   lcms2 色彩空间变换
│   ├── processing.rs    #   胶片处理、曲线 LUT、色彩管线（1069 行）
│   └── adjust.rs        #   ManualAdjust：色阶、曝光、对比度等（1058 行）
├── flexcolor/           # FlexColor 数据解析
│   ├── mod.rs           #   模块导出
│   ├── model.rs         #   数据结构：ImageCorrection（70+ 字段）、EditHistory
│   └── parser.rs        #   Apple plist XML 递归下降解析器
└── viewer/              # egui GUI 界面
    ├── mod.rs            #   模块导出
    ├── app.rs            #   应用主循环、事件处理、布局渲染（713 行）
    ├── types.rs          #   类型定义：FffViewerApp（~100 个状态字段）
    ├── panels.rs         #   信息面板：元数据、历史、标签、色彩调整（核心 UI）
    ├── file_list.rs      #   文件筛选与网格缩略图视图
    ├── loupe.rs          #   单图放大查看视图
    ├── navigation.rs     #   目录树与键盘导航
    ├── split.rs          #   分割区域管理与导出（~900 行）
    └── helpers.rs        #   UI 工具函数

profiles/                # 内置 FlexColor ICC 配置文件（15 个 .icc）
settings/                # 内置 FlexColor 设置预设（123 个 XML）
icons/                   # 应用图标（macOS .icns / Windows .ico / PNG 源文件）
```

### 模块职责

| 模块 | 行数 | 职责 |
|------|------|------|
| `tiff.rs` | 1911 | TIFF/FFF 二进制解析：IFD 链、EXIF、MakerNote、图像解码、降采样预览 |
| `color/processing.rs` | 1069 | 色彩管线核心：胶片反转、曲线提取、渐变曲线、管线编排 |
| `color/adjust.rs` | 1058 | 手动调整处理：色阶、曝光、对比度、饱和度、色彩校正矩阵 |
| `color/transform.rs` | ~200 | lcms2 ICC 变换：设备空间 → 输出色彩空间 |
| `color/profile.rs` | ~150 | ICC 文件扫描与预设 XML 索引 |
| `viewer/panels.rs` | ~2500 | 色彩调整 UI：直方图、色阶、曲线编辑器、全部滑块 |
| `viewer/app.rs` | 713 | 应用框架：初始化、后台任务轮询、拖放、布局 |
| `viewer/types.rs` | 640 | 状态定义：所有枚举、结构体、缓存字段 |
| `viewer/split.rs` | ~900 | 分割裁切：区域管理、画布交互、旋转导出 |
| `flexcolor/parser.rs` | 497 | plist XML 解析：递归下降、70+ 字段提取 |
| `flexcolor/model.rs` | 286 | 数据模型：ImageCorrection、EditHistory、FilmType |
| `i18n.rs` | 691 | 国际化：Language 枚举、Strings 结构体 |
| `sidecar.rs` | 584 | 逐文件持久化：XML 读写、路径哈希 |
| `config.rs` | 387 | 全局配置：GPU、线程、语言、收藏夹 |

### 线程模型

- **主线程** — egui 事件循环与 UI 渲染
- **缩略图线程** — 后台逐张解码低分辨率预览（通过 mpsc 通道传回）
- **文件详情线程** — 加载完整元数据、编辑历史、基础图像
- **rayon 全局线程池** — 并行色彩处理（线程数可配置）

### 数据持久化

| 类型 | 路径 | 内容 |
|------|------|------|
| 全局配置 | `~/fff_parse/config/settings.xml` | GPU、线程数、语言、收藏夹、直方图设置 |
| 逐文件 Sidecar | `~/fff_parse/sidecar/{hash}.xml` | 色彩设置、分割区域、手动调整参数 |
| 日志 | `~/fff_parse/logs/fff_viewer_*.log` | 运行日志 + panic backtrace（自动清理 >3 天） |

---

## 色彩处理管线

FFF Viewer 的色彩管线通过逆向 FlexColor 的处理流程实现，所有渲染、单文件导出和分割导出共用同一个 `apply_color_pipeline()` 函数，保证输出一致性。

### 管线总览

```
┌────────────────────────────────────────────────────────────┐
│                    FFF 文件加载                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ TiffFile::open()                                     │  │
│  │  ├─ 解码 IFD#0: 全分辨率 16-bit RGB (~92 MB)         │  │
│  │  ├─ 解码 IFD#1: 8-bit 缩略图（FlexColor 已处理）      │  │
│  │  ├─ 解码 IFD#2: 16-bit 降采样预览 (~2.6 MB)          │  │
│  │  ├─ 提取 tag 0xC519: FlexColor XML 编辑历史          │  │
│  │  └─ 提取 tag 0xC51A: 内嵌 ICC / CCD 校准数据         │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────────┬───────────────────────────────────┘
                         │
                         ▼
┌────────────────────────────────────────────────────────────┐
│              阶段 1: 胶片处理 (Film Processing)              │
│                                                            │
│  apply_film_processing(preview, correction)                │
│                                                            │
│  正片 (E-6):  不做处理，直接输出                              │
│  彩色负片 (C-41):                                           │
│    inverted[ch] = (highlight[ch] × 4 − raw) × scale       │
│  黑白负片:                                                  │
│    同上反转后 → 灰度: 0.299R + 0.587G + 0.114B              │
│                                                            │
│  → 输出: raw_rgb（扫描仪色彩空间，后续调整的基础数据）          │
└────────────────────────┬───────────────────────────────────┘
                         │
                         ▼
┌────────────────────────────────────────────────────────────┐
│          阶段 1.5: 胶片曲线提取（仅负片）                      │
│                                                            │
│  extract_film_curve(thumb_8bit, preview_16bit, correction) │
│                                                            │
│  通过对比 FlexColor 已处理的 8-bit 缩略图和原始 16-bit        │
│  预览，逆向推导出每通道 65536 级胶片响应曲线 LUT              │
│                                                            │
│  过程:                                                     │
│   1. 从缩略图逆向还原所有显示调整（饱和度、CC 矩阵、         │
│      亮度、对比度、曝光、输出色阶、主伽马、渐变曲线）          │
│   2. 应用逐通道色阶（shadow/highlight 映射）                 │
│   3. 反向应用渐变曲线（RGB→CMY→逐通道）                      │
│   4. 将反转后的原始值累积到 1024 个 bin 中                   │
│   5. 插值为 65536 级 LUT，保证单调性                         │
│                                                            │
│  回退: 若检测到重度调整（对比度/亮度/CC矩阵/非默认曲线       │
│        /非默认DotColor），返回 None，使用硬编码 LUT          │
│                                                            │
│  → 输出: extracted_film_lut（Optional，3 通道 × 65536 级）  │
└────────────────────────┬───────────────────────────────────┘
                         │
                         ▼
┌────────────────────────────────────────────────────────────┐
│     apply_color_pipeline() — 统一色彩管线入口                 │
│     渲染 / 单文件导出 / 分割导出 共用此函数                    │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 阶段 2: 渐变曲线 (Gradation Curves)                   │  │
│  │                                                      │  │
│  │ apply_gradation_curves(img, curve_points)             │  │
│  │                                                      │  │
│  │ 7 通道: [RGB主通道, R, G, B, C, M, Y]                 │  │
│  │ 应用顺序: R→G→B → C→M→Y → RGB主通道                   │  │
│  │ 每通道 256 级 LUT (De Casteljau 贝塞尔插值)            │  │
│  │ 16-bit 图像: 256 级 LUT 扩展到 65536 级               │  │
│  │ 跳过条件: 所有曲线为恒等映射 (首点=0, 末点=255)         │  │
│  └──────────────────────────┬───────────────────────────┘  │
│                             ▼                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 阶段 3: 扫描仪色阶 (Scanner Levels)                   │  │
│  │                                                      │  │
│  │ apply_scanner_levels_16(rgb16, adjust, film_lut)      │  │
│  │                                                      │  │
│  │ 处理顺序:                                             │  │
│  │  1. 应用胶片曲线 LUT（提取的或硬编码的）                │  │
│  │  2. 逐通道色阶裁切:                                   │  │
│  │     v = (v − shadow) / (highlight − shadow)           │  │
│  │  3. 逐通道伽马: v = v^(1/gamma_ch)                    │  │
│  │  4. 主伽马: v = v^(1/gamma_master)                    │  │
│  │                                                      │  │
│  │ 构建 65536 级逐通道 LUT，一次性应用                    │  │
│  └──────────────────────────┬───────────────────────────┘  │
│                             ▼                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 阶段 4: ICC 色彩空间变换                               │  │
│  │                                                      │  │
│  │ apply_icc_transform(img, input_icc, target_space)     │  │
│  │                                                      │  │
│  │ 引擎: Little CMS 2 (lcms2 crate)                     │  │
│  │ 渲染意图: Perceptual（感知）                           │  │
│  │ 位深保持: 16-bit → 16-bit                             │  │
│  │                                                      │  │
│  │ 目标色彩空间:                                         │  │
│  │  sRGB        — 内置 lcms2 配置文件, D65               │  │
│  │  Adobe RGB   — gamma 2.19921875, D65                  │  │
│  │  ProPhoto    — gamma 1.8, D50                         │  │
│  │  Display P3  — gamma 2.2, D65                         │  │
│  │                                                      │  │
│  │ 跳过条件: 无 ICC 数据                                  │  │
│  └──────────────────────────┬───────────────────────────┘  │
│                             ▼                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 阶段 5: 显示调整 (Display Adjustments)                │  │
│  │                                                      │  │
│  │ apply_display_adjust_16(rgb16, adjust)                │  │
│  │                                                      │  │
│  │ 以下调整按序构建 65536 级逐通道 LUT:                   │  │
│  │  1. 输出色阶: [0,1] → [shadow/255, highlight/255]     │  │
│  │  2. 曝光: v × 2^exposure_stops                        │  │
│  │  3. 色彩平衡: 逐通道 R/G/B 偏移                       │  │
│  │  4. 色温/色调: 暖冷色偏移                              │  │
│  │  5. 阴影/高光: 二次曲线提升/压缩                       │  │
│  │  6. 对比度: (v−0.5)×scale + 0.5                       │  │
│  │  7. 亮度: 线性偏移                                    │  │
│  │  8. 明度（暗部深度）: 伽马曲线                         │  │
│  │  9. 中间调: 伽马调整                                  │  │
│  │                                                      │  │
│  │ LUT 应用后:                                           │  │
│  │  10. 颜色校正矩阵: 3×3 RGB 线性变换                   │  │
│  │  11. 饱和度: lum + (v−lum) × (1+sat)                  │  │
│  │      (ITU-R BT.709 亮度: 0.2126R + 0.7152G + 0.0722B)│  │
│  └──────────────────────────────────────────────────────┘  │
│                                                            │
│  → 输出: 最终 16-bit RGB                                   │
│  → 计算处理后直方图（256-bin）                              │
│  → 转换为 GPU 纹理显示                                     │
└────────────────────────────────────────────────────────────┘
```

### 内嵌校正 vs 外部预设

当用户选择外部 FlexColor 色彩方案（设置预设）时，管线对两种校正数据做不同处理：

| 数据来源 | 胶片反转（highlight 值） | 胶片曲线提取 | 显示调整（色阶/对比度等） |
|----------|------------------------|-------------|------------------------|
| **内嵌校正**（编辑历史中的当前条目） | ✅ 始终使用（扫描仪校准数据） | ✅ 始终使用 | 仅在 `use_embedded_correction=true` 时 |
| **外部预设**（XML 文件） | ❌ 不使用 | ❌ 不使用 | ✅ 使用预设中的值 |

这一设计的原因是：`correction.highlight` 字段在胶片反转阶段编码的是胶片底基密度，与具体扫描硬件和胶片有关（扫描特异性数据），不能被外部预设覆盖；而色阶、对比度等显示调整参数则可安全地从预设加载。

### 关键函数调用图

```
apply_color_profile()                    ← 文件加载 / 色彩方案切换
  ├── apply_film_processing()            ← 胶片反转（正片/负片/黑白）
  ├── extract_film_curve()               ← 从缩略图对逆向胶片曲线
  │     ├── build_curve_lut() ×7         ← 构建渐变曲线 LUT
  │     └── invert_lut_256() ×7          ← 反向 LUT
  ├── load_levels_from_correction()      ← FlexColor 校正 → UI 滑块
  └── rebuild_texture_from_base()        ← 触发管线重建

rebuild_texture_from_base()              ← 任何滑块/曲线变化时调用
  └── apply_color_pipeline()             ← 统一管线入口
        ├── apply_gradation_curves()     ← 渐变曲线（7 通道）
        │     └── build_curve_lut() ×7
        ├── apply_scanner_levels()       ← 胶片曲线 + 色阶 + 伽马
        │     └── apply_scanner_levels_16()
        ├── apply_icc_transform()        ← ICC 色彩空间变换
        │     └── create_output_profile()
        └── apply_display_adjust()       ← 曝光/对比度/饱和度/CC 矩阵
              └── apply_display_adjust_16()

reprocess_with_film_type()               ← 用户切换胶片类型时
  ├── apply_film_processing()            ← 用新胶片类型重新处理
  ├── extract_film_curve()               ← 重新提取胶片曲线
  └── rebuild_texture_from_base()        ← 触发管线重建
```

### 扫描仪空间 vs 显示空间

管线在 ICC 变换前后分为两个色彩空间：

- **扫描仪空间**（ICC 变换前）：渐变曲线、胶片曲线 LUT、色阶/伽马裁切在此空间执行，这些操作与扫描仪特性相关
- **显示空间**（ICC 变换后）：曝光、对比度、亮度、饱和度、色彩平衡等在输出色彩空间中执行

---

## 编辑模块详解

### 色阶（Levels）

**原理：** 四通道（Master + R/G/B）的黑点/伽马/白点裁切，将输入范围 [shadow, highlight] 映射到 [0, 1]，然后应用伽马校正。

```
v = clamp((v − shadow) / (highlight − shadow), 0, 1)
v = v^(1/gamma)
```

- **UI 空间：** 0–255（与 Photoshop 一致）
- **内部空间：** 0.0–1.0
- **伽马：** 1.0 = 中性，< 1.0 压暗中间调，> 1.0 提亮中间调
- **应用顺序：** 逐通道伽马 → 主伽马
- **自动色阶：** 基于 65536-bin 直方图的百分位计算，默认黑点 0.05%、白点 0.1%（可在设置中调整）

**状态：** ✅ 完整实现

### 输出色阶（DotColor）

**原理：** 将最终输出的 [0, 1] 范围映射到 [output_shadow/255, output_highlight/255]，用于控制打印或显示的动态范围。

**状态：** ✅ 完整实现

### 渐变曲线（Gradation Curves）

**原理：** 7 个独立通道的色调映射曲线，通过 N 个控制点定义 0–255 的输入→输出映射。

- **通道：** RGB（主通道）、R、G、B、C（青）、M（品红）、Y（黄）
- **插值：** De Casteljau 递归贝塞尔插值（1024 采样 → 256 级 LUT）
- **单调性：** Fritsch-Carlson 算法保证曲线平滑且无过冲
- **应用顺序：** R→G→B → C→M→Y → RGB 主通道
- **CMY 处理：** C = 1−R 空间中操作，即 `output = 1 − curve(1 − input)`
- **16-bit 扩展：** 256 级 LUT 通过线性插值扩展到 65536 级
- **交互：** 点击添加控制点、拖拽移动、右键/双击删除；端点 X 固定

**状态：** ✅ 完整实现

### 胶片曲线（Film Curve）

**原理：** 模拟 FlexColor 的胶片特性曲线，将线性扫描数据映射为胶片的非线性响应。

两种来源：
1. **提取曲线** — 从 FFF 文件中的 8-bit 缩略图（FlexColor 已处理）与 16-bit 原始预览的对比中逆向推导，生成 3 通道 × 65536 级 LUT，精确还原特定图像的胶片特性
2. **硬编码曲线** — 从 FlexColor 的 Portra 160 样本中经验提取的 3 × 256 级 LUT（`FILM_CURVE_LUT_R/G/B`），仅在 `film_curve==4 && gamma≈2.0` 时作为回退使用

**重度调整检测：** 当校正参数包含非零对比度/亮度/明度、非恒等 CC 矩阵、非默认渐变曲线或非默认 DotColor 时，胶片曲线提取返回 None（此时缩略图中混入了过多显示调整成分，无法可靠分离胶片特性）

**状态：** ✅ 完整实现

### 曝光（Exposure）

**原理：** 指数缩放，模拟胶片曝光补偿。

```
output = input × 2^exposure_stops
```

- **范围：** −3.0 到 +3.0 EV
- **默认：** 0.0（无变化）
- **EV 字段注意：** FlexColor 预设的 `EV` 字段默认为 1.0（unity），缺失时解析器自动设为 1.0 以避免 `log2(0) = −∞`

**状态：** ✅ 完整实现

### 亮度（Brightness）

**原理：** 线性偏移，均匀提亮或压暗所有色调。

```
output = input + brightness / 200.0
```

- **范围：** −100 到 +100
- **默认：** 0

**状态：** ✅ 完整实现

### 对比度（Contrast）

**原理：** 以 0.5（中灰）为中心的非对称缩放。

```
scale = 1 + contrast/100 × (contrast ≥ 0 ? 2 : 1)
output = (input − 0.5) × scale + 0.5
```

- **范围：** −100 到 +100
- **默认：** 0

**状态：** ✅ 完整实现

### 高光 / 阴影（Highlights / Shadows）

**原理：** 使用二次曲线分别处理亮部和暗部区域。

- **高光（Highlights）：** 在亮部区域应用二次曲线压缩/提升
- **阴影（Shadows）：** 在暗部区域应用二次曲线提升/压缩
- **范围：** −100 到 +100
- **默认：** 0

**状态：** ✅ 完整实现

### 明度 / 暗部深度（Lightness / Shadow Depth）

**原理：** 伽马曲线调整，主要影响暗部细节。

```
output = input^(1 / (1 + lightness/100))
```

- **范围：** −100 到 +100
- **默认：** 0

**状态：** ✅ 完整实现

### 中间调（Midtone）

**原理：** 灵活的伽马调整，1.0 为中性。

```
output = input^(1/midtone)
```

- **范围：** 0.1 到 4.0
- **默认：** 1.0

**状态：** ✅ 完整实现

### 饱和度（Saturation）

**原理：** 基于 ITU-R BT.709 亮度的色彩浓度调整。

```
lum = 0.2126×R + 0.7152×G + 0.0722×B
output = lum + (input − lum) × (1 + saturation/100)
```

- **正值增加色彩浓度，负值趋向灰度**
- **范围：** −100 到 +100
- **默认：** 0

**状态：** ✅ 完整实现

### 色温 / 色调（Color Temperature / Tint）

**原理：** 逐通道乘法偏移，模拟暖冷色温变化。

- **色温：** 正值偏暖（增加 R，减少 B），负值偏冷
- **色调：** 正值偏绿，负值偏品红
- **缩放因子：** ×0.15（柔和调整）
- **范围：** −100 到 +100
- **默认：** 0

**状态：** ✅ 完整实现

### 色彩平衡（Color Balance）

**原理：** R/G/B 三通道独立线性偏移。

- **范围：** 每通道 −100 到 +100
- **默认：** 0

**状态：** ✅ 完整实现

### 颜色校正矩阵（Color Correction Matrix）

**原理：** 6×6 RGBCMY 矩阵的线性 RGB 变换（实际使用 3×3 RGB 子矩阵）。

```
[R']   [cc[0]+100  cc[1]    cc[2]  ] [R]
[G'] = [cc[6]    cc[7]+100  cc[8]  ] [G] ÷ 100
[B']   [cc[12]   cc[13]   cc[14]+100] [B]
```

- **对角线 +100 表示恒等**（100 = 100%）
- **范围：** 每个元素 −100 到 +100
- **默认：** 全部为 0（恒等矩阵）

**状态：** ✅ 完整实现

### 直方图（Histogram）

**原理：** 双直方图系统。

- **原始直方图（Raw）：** 256-bin 显示直方图 + 65536-bin 精确直方图
  - **显示用 256-bin：** 对负片使用 per-channel highlight 反转公式 `bin = (hi_ch - raw) * 255 / hi_ch`，
    直接从原始扫描数据（`preview_raw`，未经任何处理）计算。此公式模拟 FlexColor 的原始直方图显示风格，
    其中 `hi_ch` 为内嵌编辑历史中各通道的 highlight 值（14-bit × 4 → 16-bit）。
    正片模式下退化为简单的 `raw >> 8` 线性映射。
    （公式来源：通过 `examples/histogram_stages.rs` 诊断工具对比 FlexColor 界面逐阶段排查确定，
    对应 `ifd2_perch_hi` 变体，虽非精确匹配但为最接近的可复现公式。）
  - **自动色阶用 65536-bin：** 基于 `raw_rgb`（胶片反转后、曲线前），用于百分位黑白点计算
- **处理后直方图（Processed）：** 基于最终输出像素，256-bin，仅供显示
- **显示模式：** 线性 / 平方根 / 对数 / 立方根（可在设置中切换）
- **通道：** R / G / B 独立 + RGB 合成（取各 bin 的 max(R,G,B)）

**状态：** ✅ 完整实现

### 分割裁切（Split / Crop）

**原理：** 在图像上定义多个可旋转的矩形区域，每个区域独立导出为 TIFF。

- **格式预设：** 24 种胶片格式（35mm 全画幅、6×6、6×7、6×9、6×12、6×17、4×5 等）+ 自由格式
- **区域交互：** 拖拽移动、8 方向角点/边缘缩放、边缘旋转
- **旋转导出：** 使用双线性插值在亚像素精度下裁切旋转区域
- **命名模板：** 自定义导出文件名模式（`{name}_{n}.tif`）
- **持久化：** 区域信息保存到 Sidecar XML

**状态：** ✅ 完整实现

---

## 未实现功能

以下功能的参数可在 FlexColor 校正数据中解析和保存，UI 中有占位控件和提示信息，但尚未实现实际的像素处理：

| 功能 | UI 控件 | 说明 |
|------|---------|------|
| **USM 锐化** | 强度、半径、暗部限制、噪声限制 滑块 | 需要实现 Unsharp Mask 卷积核 |
| **灰尘去除** | 灰尘等级 滑块 | 需要实现斑点检测与修复算法 |
| **颜色噪声滤波** | 噪声半径、噪声偏置 滑块 | 需要实现色度降噪算法 |
| **镜头/暗角校正** | 镜头校正、暗角量 滑块 | 需要实现几何畸变与暗角补偿 |
| **增强阴影 / 去色偏** | 启用/禁用 复选框 | 需要实现局部阴影增强与色偏移除 |

---

## FFF 文件格式

FFF（Flexible File Format）基于 TIFF 结构，由 Imacon/Hasselblad 扫描仪软件 FlexColor 生成。

### 文件布局

```
偏移量              内容
─────────────────────────────────────────────────────────
[0..8]              TIFF 头 (MM 大端序, magic 0x55, IFD#0 偏移)
[~8..~76]           Tag 0xB4C7: FlexColor 版本 / 扫描仪序列号 (~67 B)
[~76..~400076]      Tag 0xC519: XML plist 编辑历史 (~400 KB, 零填充)
[~400076..~600076]  Tag 0xB4C5: 二进制设置副本 (~195 KB)
[~600076..]         IFD#0: 全分辨率 16-bit RGB (~92 MB/帧)
[...]               IFD#1: 8-bit 缩略图 (FlexColor 已处理, ~1.3 MB)
[...]               Tag 0xC51A: CCD 校准数据 (~211 KB)
[...]               IFD#2: 16-bit 降采样预览 (~2.6 MB)
[末尾]              IPTC / 元数据 (~200 B)
```

### 自定义标签

| 标签 ID | 名称 | 内容 |
|---------|------|------|
| `0xB4C7` | FlexColor Version | 扫描仪序列号和 FlexColor 版本 |
| `0xC519` | Settings XML | Apple plist XML，包含完整编辑历史（多个 ImageSetting） |
| `0xB4C5` | Binary Settings | 二进制格式的设置副本 |
| `0xC51A` | Imacon Profile Data | CCD 校准数据或内嵌 ICC 配置文件 |

---

## 系统要求

- Rust 1.70+（推荐 1.94+）
- macOS / Windows（已测试）/ Linux（egui 跨平台，未测试）
- Little CMS 2（`lcms2`，通过 Cargo 自动编译）
- Windows 额外依赖：MSVC 构建工具链

### 依赖

| Crate | 版本 | 用途 |
|-------|------|------|
| `eframe` | 0.31 | egui 窗口框架 |
| `egui` | 0.31 | UI 工具包 |
| `image` | 0.25 | JPEG/PNG/TIFF 编解码 |
| `lcms2` | 6 | ICC 色彩变换引擎 |
| `rayon` | 1 | 数据并行处理 |
| `rfd` | 0.15 | 文件/文件夹对话框 |
| `log` + `env_logger` | 0.4 / 0.11 | 日志系统 |
| `chrono` | 0.4 | 时间戳处理 |
| `winresource` | 0.1 | Windows 图标嵌入（仅构建依赖） |

## 构建与运行

```bash
# 构建（推荐 release 模式，图像处理速度提升显著）
cargo build --release

# 运行测试（25 个单元测试）
cargo test

# 直接运行
cargo run --release --bin fff_viewer

# 打开指定文件
cargo run --release --bin fff_viewer -- "/path/to/scan.fff"
```

### macOS 打包发布

```bash
cargo build --release

APP="FFF Viewer.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/fff_viewer "$APP/Contents/MacOS/"
cp icons/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp -R profiles "$APP/Contents/Resources/profiles"
cp -R settings "$APP/Contents/Resources/settings"

cat > "$APP/Contents/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>       <string>fff_viewer</string>
  <key>CFBundleIconFile</key>         <string>AppIcon</string>
  <key>CFBundleIdentifier</key>       <string>com.fff-viewer.app</string>
  <key>CFBundleName</key>             <string>FFF Viewer</string>
  <key>CFBundleVersion</key>          <string>0.7.0</string>
  <key>CFBundlePackageType</key>      <string>APPL</string>
  <key>NSHighResolutionCapable</key>  <true/>
</dict>
</plist>
EOF
```

### Windows 打包发布

```powershell
# 前置: Rust MSVC 工具链 + Visual Studio Build Tools "C++ 桌面开发"
cargo build --release

$dist = "FFF_Viewer_Windows"
New-Item -ItemType Directory -Force -Path $dist
Copy-Item target\release\fff_viewer.exe $dist\
Copy-Item -Recurse profiles $dist\profiles
Copy-Item -Recurse settings $dist\settings
Copy-Item icons\icon.ico $dist\
```

> Windows 版为绿色便携式软件，`profiles/` 和 `settings/` 需与 `fff_viewer.exe` 同目录。

### CLI 解析工具

```bash
cargo run --release --bin parse_test -- "/path/to/scan.fff"
```

输出 IFD 结构、所有标签值、图像解码测试结果和 FlexColor 编辑历史。

---

## 界面说明

### 布局

```
┌──────────────────────────────────────────────────────┐
│ 📂 打开  ▦ 网格  🔍 放大  ℹ 信息  📤 导出  🌐 语言  │ 工具栏
├──────────┬────────────────────────┬───────────────────┤
│ ★ 收藏夹 │                        │  📋 元数据        │
│          │   主视图               │  📝 历史          │
│ 📁 目录树 │   (网格 / 放大)        │  🏷 标签          │
│          │                        │  🎨 色彩调整      │
│          ├────────────────────────┤  🖼 色彩方案       │
│          │   胶片条 (放大模式)     │  ✂ 分割           │
│          │                        │  ⚙ 设置          │
├──────────┴────────────────────────┴───────────────────┤
│ 状态栏: 加载进度 │ 文件匹配数 │ 导出进度              │
└──────────────────────────────────────────────────────┘
```

### 信息面板标签页

| 标签页 | 内容 |
|--------|------|
| **📋 元数据** | 图像尺寸、色彩模式、扫描仪信息等摘要 |
| **📝 历史** | FlexColor 编辑设置列表，点击展开查看详细校正参数 |
| **🏷 标签** | 所有 IFD 中的原始标签（支持筛选） |
| **🎨 色彩** | 色阶、渐变曲线、曝光等全部色彩调整滑块和控件 |
| **🖼 色彩方案** | ICC 配置文件选择、设置预设浏览与应用 |
| **✂ 分割** | 胶片格式选择、区域管理、分割导出 |
| **⚙ 设置** | GPU、线程数、语言、直方图显示、自动色阶阈值 |

---

## 已知限制

- FFF 原始图像通常为 3996×15178 16-bit RGB（约 350MB），导出单张 TIFF 需数秒，内存占用约 800MB
- 仅支持未压缩 RGB 和 JPEG 两种编码方式
- FlexColor 编辑历史依赖 tag 0xC519 中的 plist XML，不同固件/软件版本可能有结构差异
- 胶片曲线提取在重度调整的图像上可能不准确（此时回退到硬编码 LUT）
- USM 锐化、灰尘去除、噪声滤波、镜头校正等高级功能尚未实现像素处理

---

## Changelog

### v0.7.0

- **色彩管线统一**
  - 提取 `apply_color_pipeline()` 统一函数，渲染/导出/分割导出共用同一管线
  - 修复导出与渲染不一致的问题
- **色彩方案切换修复**
  - 分离内嵌校正与预设校正：胶片反转始终使用内嵌校正（扫描仪校准数据）
  - 修复外部预设缺少 EV 字段导致 `log2(0) = −∞` 全黑的问题
- **胶片曲线提取增强**
  - 新增重度调整检测（对比度/亮度/CC 矩阵/非默认曲线/DotColor），检测到时自动回退
- **曲线编辑器防崩溃**
  - 拖拽索引越界检查、通道重置时清除拖拽状态、加载时保证每通道 ≥ 2 控制点
- **分割区域光标修复**
  - 修正角点缩放光标方向、旋转手柄显示旋转光标
- **Panic Hook 增强**
  - 提取并记录实际 panic 消息（payload + location），改善崩溃诊断
- 移除 `film_gamma ≈ 2.0` 限制，扩展扫描仪色阶的适用范围
- Sidecar 加载时自动修复非有限曝光值

### v0.6.0

- **渐变曲线编辑**
  - 新增交互式曲线编辑器，支持 RGB / R / G / B / C / M / Y 七个通道独立编辑
  - 256×256 曲线图：网格参考线、对角恒等线、Fritsch-Carlson 单调三次插值
  - 控制点交互：点击添加、拖拽移动、右键或双击删除；端点 X 固定
  - 加载 FlexColor 色彩方案时自动提取嵌入的曲线控制点
- **渲染管线重构**
  - 曲线从 `raw_rgb` 动态应用，不烘焙到基础数据
  - 直方图始终基于原始数据（曲线前）
  - 16-bit 曲线应用改为 rayon 并行处理
- 新增 `color::adjust` 模块单元测试（25 个测试用例）

### v0.5.0

- **ICC 色彩管理**
  - 新增色彩管理面板
  - 内置 15 个 FlexColor ICC 配置文件、123 个设置预设
  - 支持内嵌 ICC 提取（tag 0xC51A）
  - ICC 变换基于 Little CMS 2
  - 设置预设选择器支持分类筛选

### v0.4.0

- **TIFF 导出功能** — 单文件/批量导出，16-bit 色深，进度显示
- **崩溃日志** — 文件日志 + panic hook + backtrace 捕获
- **浅色主题修复**

### v0.3.0

- 修复 XML plist 解析器嵌套深度 bug
- 编辑历史正确显示所有设置

### v0.2.0

- **Lightroom 风格界面** — 目录树、网格视图、胶片浏览视图
- 国际化支持、CJK 字体、应用图标
- 缩略图渐进式加载、macOS .app 打包

### v0.1.0

- TIFF/FFF 二进制解析器（大/小端、12 种数据类型、IFD 链）
- 未压缩 RGB 解码与预览
- FlexColor 编辑历史解析
- `parse_test` CLI 工具

## 许可证

MIT
