# Windows 发布执行说明（交接文档）

> 面向在 **Windows 机器**上执行构建/验收的人或 AI 助手（如 GitHub Copilot）。
> 目标：用仓库里现成的脚本构建 FFF Viewer 的 Windows 安装包并完成验收。
> 这是产品化 Sprint 的 **T60 Task 4**。计划全文见 `docs/superpowers/plans/2026-06-13-t60-windows-installer.md`，总路线见 `docs/roadmap-2026-06-productization.md`。

---

## 背景（30 秒读懂）

- 本项目是 Rust + egui 桌面应用（哈苏 FlexColor `.fff` 扫描文件查看器），二进制名 `fff_viewer`。
- 仓库里**已经写好**全部 Windows 打包工具，**不需要你写代码**，只需按下面步骤运行与验收：
  - `scripts/build-windows.ps1` —— 一键构建（`cargo build --release` → 调 Inno Setup 的 `ISCC.exe`）
  - `installer/windows/fff-viewer.iss` —— 安装包定义（装 exe + `profiles/` + `settings/` + 快捷方式 + 卸载器）
- 版本号来自 `Cargo.toml`（当前 `0.1.0`），脚本自动注入。

## ⚠️ 关键约束（务必满足，否则装出来的程序打不开图）

应用运行时通过 `src/viewer/helpers.rs::find_resource_dir` 在 **exe 同级目录**查找 `profiles/`（15 个 ICC）和 `settings/`（123 个预设）。安装包已配置好把这两个目录一并装入 `Program Files\FFF Viewer`。**验收时必须确认它们在安装目录里存在**，否则 ICC 配置与预设会是空的。

---

## 前置依赖（首次需安装）

1. **Rust（MSVC 工具链）**：https://rustup.rs/ —— 安装后确认默认 host 是 `x86_64-pc-windows-msvc`（`rustup show`）。
2. **Visual Studio Build Tools**：勾选「**使用 C++ 的桌面开发**」工作负载（提供 MSVC 链接器，`lcms2` C 依赖编译需要）。
3. **Inno Setup 6**：https://jrsoftware.org/isdl.php —— 默认装到 `C:\Program Files (x86)\Inno Setup 6\`，脚本会自动找 `ISCC.exe`。

## 执行步骤

```powershell
# 0. 取最新代码
git pull

# 1. 一键构建安装包（在仓库根目录运行）
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
#    成功时末尾打印: Installer: dist\FFF Viewer-0.1.0-setup.exe
```

```powershell
# 2. 安装：双击 dist\FFF Viewer-0.1.0-setup.exe，按向导完成（默认装到 Program Files\FFF Viewer）

# 3. 校验运行期资源就位（应分别为 15 / True）
(Get-ChildItem "$env:ProgramFiles\FFF Viewer\profiles" -Filter *.icc).Count
Test-Path "$env:ProgramFiles\FFF Viewer\settings"
```

4. **功能验收**：从开始菜单启动 FFF Viewer → 拖入一张 `.fff` 扫描文件 → 打开「色彩」面板，确认 ICC 配置下拉**非空** → 试试「放大镜 / 切割」基本功能。

   > 样本 `.fff` **不在仓库里**（体积太大，单个 ~97MB–588MB）。请从 **NAS**（或任意自有扫描文件）取一张 `.fff` 用于测试。

## 验收清单（逐项确认）

- [ ] `build-windows.ps1` 成功产出 `dist\FFF Viewer-0.1.0-setup.exe`
- [ ] 安装包能完成安装，开始菜单出现「FFF Viewer」，有卸载项
- [ ] `Program Files\FFF Viewer\profiles` 含 **15** 个 `.icc`，`settings` 目录存在
- [ ] 程序能启动并打开 `.fff`，ICC 配置下拉非空
- [ ] 切割/导出基本可用

## 已知注意点

- **虚拟机**：egui/eframe 走 wgpu，VM 虚拟显卡可能导致启动慢/卡顿，甚至选不到后端而黑屏。VMware 请开「加速 3D 图形」并分配显存；性能以物理机为准。
- **未签名**：安装包未做代码签名，Windows SmartScreen 首次运行会告警，点「更多信息 → 仍要运行」即可（本期范围是「能装能跑」，签名留待对外分发）。

## 出问题怎么报回来

> 这台机器与原开发机（macOS 上的 Claude Code）**无法直接通信**，请用下面任一方式回传，开发侧 `git pull` 后即可看到：

**方式 A（推荐，走 Git）**：把结果写进新文件 `docs/windows-release-result.md` 并提交推送，模板：

```markdown
# Windows 发布验收结果（<日期> / <机器: 物理机 or VMware VM>）

## 构建
- build-windows.ps1: 成功 / 失败
- 产物: dist\FFF Viewer-<version>-setup.exe 是否生成: 是 / 否

## 验收清单
- 安装/卸载: ✅/❌
- profiles .icc 数量: __ (期望 15)；settings 存在: ✅/❌
- 启动+开图: ✅/❌；ICC 配置下拉非空: ✅/❌
- 切割/导出: ✅/❌

## 报错/异常（原样粘贴）
```
<把 cargo / ISCC / 启动时的完整报错贴这里>
```

## 主观体验（VM 注意标注）
<流畅度、GPU、字体、DPI 等>
```

```powershell
git add docs\windows-release-result.md
git commit -m "Windows 发布验收结果回传"
git push
```

**方式 B**：直接把 PowerShell 里的完整输出/报错复制给原开发机的使用者。

## 常见报错速查

- `link.exe not found` / `error: linker ... not found` → 没装 VS Build Tools 的「C++ 桌面开发」。
- `error: Microsoft Visual C++ ... required` → 同上。
- `未找到 Inno Setup 6 (ISCC.exe)` → 没装 Inno Setup 6，或装到了非默认路径（默认 `C:\Program Files (x86)\Inno Setup 6\`）。
- `cargo: 无法将"cargo"识别为...` → Rust 未装或未加入 PATH（重开终端 / 重装 rustup）。
