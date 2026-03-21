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

### 渐变曲线编辑（v0.6.0 新增）

- **7 通道独立编辑** — 支持 RGB / R / G / B / C / M / Y 七个通道的独立曲线调整，各通道可独立重置
- **交互式控制点** — 在 256×256 曲线图上点击添加控制点、拖拽移动、右键或双击删除；端点 X 坐标固定，不可越过相邻控制点
- **平滑插值** — 采用 Fritsch-Carlson 单调三次样条插值，保证曲线平滑且无过冲
- **动态应用** — 曲线从 `raw_rgb` 动态计算，不烘焙到基础数据；直方图始终基于曲线前的原始数据
- **色彩方案联动** — 加载 FlexColor 色彩方案时自动提取并应用嵌入的曲线控制点

### TIFF 导出

- **导出当前文件** — 将选中的 FFF 文件导出为标准 TIFF（保留 16-bit 色深）
- **批量导出** — 一键导出当前目录中的所有 FFF 文件
- **进度显示** — 批量导出时在底部状态栏显示进度条和当前文件名

## 系统要求

- Rust 1.70+（推荐 1.94+）
- macOS / Windows / Linux（egui 跨平台）
- Little CMS 2（`lcms2`，通过 Cargo 自动编译）
- Windows 额外依赖：MSVC 构建工具链（通过 Visual Studio Build Tools 安装 "C++ 桌面开发" 工作负载）

## 构建与运行

```bash
# 构建（推荐 release 模式，图像解码速度更快）
cargo build --release

# 直接运行
cargo run --release --bin fff_viewer

# 打开指定文件（macOS / Linux）
cargo run --release --bin fff_viewer -- "/path/to/scan.fff"

# 打开指定文件（Windows）
cargo run --release --bin fff_viewer -- "C:\path\to\scan.fff"
```

### macOS 打包发布

构建后手动创建 `.app` 包：

```bash
cargo build --release

APP="FFF Viewer.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/fff_viewer "$APP/Contents/MacOS/"
cp icons/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cp -R profiles "$APP/Contents/Resources/profiles"
cp -R settings "$APP/Contents/Resources/settings"

# 创建 Info.plist
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
  <key>CFBundleVersion</key>          <string>0.6.0</string>
  <key>CFBundlePackageType</key>      <string>APPL</string>
  <key>NSHighResolutionCapable</key>  <true/>
</dict>
</plist>
EOF

echo "打包完成: $APP"
```

分发时将整个 `FFF Viewer.app` 文件夹压缩为 zip 即可。

### Windows 打包发布

#### 前置条件

1. 安装 [Rust](https://rustup.rs/)（选择 MSVC 工具链）
2. 安装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)，勾选 **"C++ 桌面开发"** 工作负载

#### 编译

```powershell
cargo build --release
```

生成的可执行文件位于 `target\release\fff_viewer.exe`。

#### 打包为便携式发行包

```powershell
# 创建发行目录
$dist = "FFF_Viewer_Windows"
New-Item -ItemType Directory -Force -Path $dist

# 复制可执行文件
Copy-Item target\release\fff_viewer.exe $dist\

# 复制资源目录（ICC 配置文件和设置预设）
Copy-Item -Recurse profiles $dist\profiles
Copy-Item -Recurse settings $dist\settings

# 复制图标（可选）
Copy-Item icons\icon.ico $dist\

Write-Host "打包完成: $dist"
```

将 `FFF_Viewer_Windows` 文件夹压缩为 zip 即可分发。用户解压后双击 `fff_viewer.exe` 运行。

> **提示：** Windows 版为绿色便携式软件，无需安装。`profiles/` 和 `settings/` 目录需与 `fff_viewer.exe` 放在同一文件夹中。

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
| **🎨 色彩** | ICC 配置文件选择、设置预设浏览与应用、渐变曲线编辑 |

## 项目结构

```
src/
├── main.rs            # 入口，解析命令行参数，日志初始化
├── lib.rs             # 公共模块导出
├── tiff.rs            # TIFF/FFF 二进制解析器（IFD、EXIF、MakerNote）
├── tags.rs            # 标签名称查找表与值解释器
├── i18n.rs            # 国际化（英语 / 简体中文）
├── config.rs          # 配置文件读写
├── sidecar.rs         # Sidecar 文件处理
├── parse_test.rs      # CLI 解析测试工具
├── color/             # ICC 色彩管理模块
│   ├── mod.rs         #   模块导出
│   ├── profile.rs     #   ICC 配置文件加载与扫描
│   ├── transform.rs   #   色彩空间变换
│   ├── processing.rs  #   图像处理、渐变曲线 LUT 生成
│   └── adjust.rs      #   手动调整（曝光、色阶、曲线等）
├── flexcolor/         # FlexColor 编辑历史 & 设置预设
│   ├── mod.rs         #   模块导出
│   ├── model.rs       #   数据结构定义
│   └── parser.rs      #   plist XML 解析器
└── viewer/            # egui GUI
    ├── mod.rs          #   模块导出
    ├── app.rs          #   应用状态与主逻辑
    ├── panels.rs       #   右侧信息面板（元数据、历史、标签、色彩、曲线编辑）
    ├── types.rs        #   类型定义
    ├── file_list.rs    #   目录树与文件浏览器
    ├── loupe.rs        #   放大查看视图
    ├── navigation.rs   #   图像导航
    ├── split.rs        #   分屏视图
    └── helpers.rs      #   UI 工具函数
icons/
├── AppIcon.icns       # 应用图标（macOS）
├── icon_256.png       # 应用图标 256×256 PNG 源文件
└── icon.ico           # 应用图标（Windows）
profiles/              # FlexColor ICC 配置文件（15 个）
settings/              # FlexColor 设置预设（123 个 XML 文件）
```

## 已知限制

- FFF 原始图像通常为 3996×15178 16-bit RGB（约 350MB），导出单张 TIFF 需数秒（release 模式），内存占用约 800MB
- 目前仅支持未压缩 RGB 和 JPEG 两种编码方式
- FlexColor 编辑历史依赖 tag 0xC519 中嵌入的 plist XML，不同固件/软件版本的结构可能有差异
- ICC 色彩变换目前应用于预览图（非全分辨率原图），导出暂不应用色彩变换

## Changelog

### v0.6.0

- **渐变曲线编辑**
  - 新增交互式曲线编辑器，支持 RGB / R / G / B / C / M / Y 七个通道独立编辑
  - 256×256 曲线图：网格参考线、对角恒等线、Fritsch-Carlson 单调三次插值平滑曲线
  - 控制点交互：点击添加、拖拽移动、右键或双击删除；端点 X 固定，不可越过相邻点
  - 各通道独立重置为线性曲线
  - 加载 FlexColor 色彩方案时自动提取嵌入的曲线控制点
- **渲染管线重构**
  - 曲线不再静态烘焙到 `base_rgb`，改由 `rebuild_texture_from_base()` 从 `raw_rgb` 动态应用
  - 直方图始终基于原始数据（曲线前），不受曲线调整影响
  - 16-bit 图像曲线应用改为 rayon 并行处理，提升性能
- `build_curve_lut` 改为 `pub`，供曲线编辑器预览渲染使用
- 新增 `curve_reset` 中英文翻译
- 新增 `FffViewerApp` 曲线状态字段：`curve_points`、`curve_channel`、`curve_dragging`
- 新增 `color::adjust` 模块单元测试（恒等变换、色阶黑白点、曝光等）
- 修正 `FILM_CURVE_LUT_R` 查找表中间段平滑度

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
