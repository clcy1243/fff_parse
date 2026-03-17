# FFF Viewer

一个用于查看 Hasselblad / Imacon Flextight X5 扫描仪输出的 `.fff`（3F / Flexible File Format）文件的桌面应用程序。

基于 Rust + [egui](https://github.com/emilk/egui) 构建，无需依赖任何商业软件即可浏览扫描图像、查看 TIFF/EXIF 元数据、回溯 FlexColor 的完整编辑历史，应用 ICC 色彩配置文件，并导出为标准 TIFF 格式。

## 功能

### 浏览与查看

- **Lightroom 风格界面** — 左侧目录树、中央网格 / 胶片浏览双视图、右侧信息面板
- **图像预览** — 自动解码 FFF 文件中的未压缩 RGB 图像（8-bit / 16-bit），缩略图渐进式加载
- **元数据浏览** — 图像尺寸、色彩空间、扫描仪型号、软件版本等关键信息
- **FlexColor 编辑历史** — 解析嵌入的 Apple plist XML，展示每次编辑设置的名称、时间戳及详细校正参数
- **全部标签** — 列出所有 TIFF/EXIF 标签及 Hasselblad MakerNote 私有标签
- **国际化** — 支持英语和简体中文界面切换

### ICC 色彩管理（v0.5.0 新增）

- **输入配置文件** — 从内置的 15 个 FlexColor ICC 配置文件中选择（Flextight X5、Hasselblad RGB 等）
- **设置预设** — 从 123 个 FlexColor 设置预设中选择（Standard、Film Specific 等分类），查看详细参数
- **内嵌 ICC** — 使用 FFF 文件中嵌入的 ICC 配置文件（tag 0xC51A）
- **实时预览** — 选择配置文件后点击应用，立即在预览中显示色彩变换效果

### TIFF 导出

- **导出当前文件** — 将选中的 FFF 文件导出为标准 TIFF（保留 16-bit 色深）
- **批量导出** — 一键导出当前目录中的所有 FFF 文件
- **进度显示** — 批量导出时在底部状态栏显示进度条和当前文件名

## 系统要求

- Rust 1.70+（推荐 1.94+）
- macOS / Linux / Windows（egui 跨平台）
- Little CMS 2（`lcms2`，通过 Cargo 自动编译）

## 构建与运行

```bash
# 构建（推荐 release 模式，图像解码速度更快）
cargo build --release

# 直接运行
cargo run --release --bin fff_viewer

# 打开指定文件
cargo run --release --bin fff_viewer -- "/path/to/scan.fff"
```

### macOS App 打包

```bash
# 构建后手动创建 .app 包
APP="FFF Viewer.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/fff_viewer "$APP/Contents/MacOS/"
cp icons/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp -R profiles "$APP/Contents/Resources/profiles"
cp -R settings "$APP/Contents/Resources/settings"
# 添加 Info.plist（参见项目内脚本或手动编写）
```

### CLI 解析工具

项目附带一个命令行解析工具，可在终端中快速查看文件结构：

```bash
cargo run --release --bin parse_test -- "/path/to/scan.fff"
```

输出包括 IFD 结构、所有标签值、图像解码测试结果和 FlexColor 编辑历史。

## 界面说明

工具栏提供视图和面板切换：

| 按钮 | 功能 |
|------|------|
| **📂 打开文件夹** | 选择包含 FFF 文件的目录 |
| **▦ 网格 / 🔍 放大** | 切换缩略图网格和胶片浏览视图 |
| **ℹ 信息** | 显示/隐藏右侧信息面板 |
| **📤 导出** | 将当前选中的 FFF 文件导出为 TIFF |
| **📤 全部导出** | 批量导出当前目录中所有 FFF 文件 |
| **🌐 语言** | 切换英语/简体中文 |

**信息面板：**

| 标签页 | 内容 |
|--------|------|
| **📋 元数据** | 图像尺寸、色彩模式、扫描仪信息等摘要 |
| **📝 历史** | FlexColor 编辑设置列表，点击可展开查看详细校正参数 |
| **🏷 标签** | 所有 IFD 中的原始标签名称与值（支持筛选） |
| **🎨 色彩** | ICC 配置文件选择、设置预设浏览与应用 |

## 项目结构

```
src/
├── main.rs        # 入口，解析命令行参数，日志初始化
├── lib.rs         # 公共模块导出
├── viewer.rs      # egui GUI（目录树 + 网格/胶片视图 + 信息面板 + 导出 + 色彩管理）
├── tiff.rs        # TIFF/FFF 二进制解析器（IFD、EXIF、MakerNote）
├── flexcolor.rs   # FlexColor 编辑历史 & 设置预设 plist XML 解析器
├── color.rs       # ICC 色彩管理模块（配置文件加载、变换、内嵌 ICC 提取）
├── i18n.rs        # 国际化（英语 / 简体中文）
├── tags.rs        # 标签名称查找表与值解释器
└── parse_test.rs  # CLI 解析测试工具
icons/
└── AppIcon.icns   # 应用图标（FlexColor 风格）
profiles/          # FlexColor ICC 配置文件（15 个）
settings/          # FlexColor 设置预设（123 个 XML 文件）
```

## 已知限制

- FFF 原始图像通常为 3996×15178 16-bit RGB（约 350MB），导出单张 TIFF 需数秒（release 模式），内存占用约 800MB
- 目前仅支持未压缩 RGB 和 JPEG 两种编码方式
- FlexColor 编辑历史依赖 tag 0xC519 中嵌入的 plist XML，不同固件/软件版本的结构可能有差异
- ICC 色彩变换目前应用于预览图（非全分辨率原图），导出暂不应用色彩变换

## Changelog

### v0.5.0

- **ICC 色彩管理**
  - 新增色彩管理面板（🎨 色彩标签页）
  - 内置 15 个 FlexColor ICC 配置文件（Flextight X5/949、Hasselblad RGB/CMYK 等）
  - 内置 123 个 FlexColor 设置预设（Standard、Film Specific 分类）
  - 支持从 FFF 文件中提取内嵌 ICC 配置文件（tag 0xC51A）
  - ICC 色彩变换基于 Little CMS 2（`lcms2` crate）
  - 选择输入配置文件后可实时应用到预览图
  - 设置预设选择器支持分类筛选，选中时显示详细校正参数
- 新增 `src/color.rs` 模块
- 新增色彩管理相关 i18n 字符串
- .app 打包现在包含 profiles/ 和 settings/ 资源目录

### v0.4.0

- **TIFF 导出功能**
  - 导出当前选中文件为标准 TIFF（保留原始 16-bit 色深）
  - 批量导出当前目录中所有 FFF 文件到指定文件夹
  - 底部状态栏显示导出进度、完成/错误状态
- **移除编辑面板** — 移除了 v0.3.0 中的质感/颜色校正/层次/直方图编辑面板，聚焦于查看和导出
- **崩溃日志** — 文件日志 + panic hook + backtrace 捕获
- **浅色主题修复** — 所有硬编码颜色改为主题感知
- 清理未使用的 i18n 字符串，新增导出相关翻译

### v0.3.0

- 修复 XML plist 解析器嵌套深度 bug（`find("</dict>")` 匹配到嵌套子元素），改用消耗量追踪
- 编辑历史现在正确显示所有设置
- 目录树最小宽度调整，支持长路径名精简显示（Creative Cloud 等特殊路径自动缩写）

### v0.2.0

- **全新 Lightroom 风格界面**
  - 左侧目录树：浏览文件系统，点击目录加载其中的 FFF 文件
  - 网格视图（Grid）：缩略图网格，点击选中，双击进入 Loupe 视图
  - 胶片浏览视图（Loupe）：选中图片放大显示，底部单行缩略图胶片条
  - 左右方向键切换图片
  - Grid / Loupe 视图自由切换
- 国际化支持（英语 / 简体中文），CJK 字体加载
- 应用图标（FlexColor 风格）
- 缩略图保持原始比例，网格格子为正方形
- 缩略图渐进式加载（每帧加载 2 张，不阻塞 UI）
- 支持打开文件夹（对话框 + 拖放目录）
- macOS .app 打包

### v0.1.0

- 实现 TIFF/FFF 二进制解析器，支持大/小端字节序、所有 12 种 TIFF 数据类型
- 支持 IFD 链、Sub-IFD、EXIF IFD 和 Hasselblad MakerNote 解析
- 实现未压缩 RGB（8-bit / 16-bit）图像解码与预览
- 内置 60+ 标准 TIFF/EXIF 标签和 15 个 Hasselblad MakerNote 标签的名称映射
- 基于 egui 构建 GUI，支持图像缩放显示与元数据面板
- 支持文件选择对话框、拖放打开和命令行参数
- 解析 FlexColor 编辑历史（嵌入式 Apple plist XML）
- 编辑历史详情展示：图像校正参数、胶片设置、USM 锐化、处理标志、镜头校正、渐变曲线
- 附带 `parse_test` CLI 工具用于调试和验证

## 许可证

MIT
