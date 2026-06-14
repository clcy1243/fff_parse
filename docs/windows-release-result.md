# Windows 发布验收结果（2026-06-14 / 机器: 物理机 Windows）

## 构建
- build-windows.ps1: 成功
- 产物: `dist\FFF Viewer-0.1.0-setup.exe` 是否生成: 是
- 构建日志: `dist\logs\build-windows.log`

## 验收清单
- 安装/卸载: ✅（开始菜单存在 `FFF Viewer` 与 `卸载 FFF Viewer`，注册表存在卸载项）
- profiles .icc 数量: 15（期望 15）；settings 存在: ✅
- 启动+开图: ✅（用户手动打开 `.fff` 验证通过）
- ICC 配置下拉非空: ✅（运行日志显示加载 `15 ICC profiles`）
- 切割/导出: 文件处理链路无异常（用户反馈），但 UI 存在错位问题

## 日志与证据
- 构建日志：`dist\logs\build-windows.log`
- 运行日志：`C:\Users\clcy1\fff_parse\logs\fff_viewer_20260614_123113.log`
- 截图：`docs\windows-ui-misalignment.png`
- 关键日志片段（运行期）：
  - `=== FFF Viewer started ===`
  - `Found 15 ICC profiles, 123 settings presets`
  - `Found 0 .fff files`

## 报错/异常（原样粘贴）
```text
无阻塞性构建报错（本轮最终构建成功）。
人工验收反馈：文件处理没问题，UI 有错位问题。
```

## 主观体验（VM 注意标注）
- 应用可正常启动并完成基础初始化，OpenGL 初始化正常（日志显示 NVIDIA GeForce RTX 3090）。
- 人工验收确认文件处理正常；当前主要问题为界面元素存在错位，见截图 `docs/windows-ui-misalignment.png`。

## 额外说明
- 为保证当前构建机可稳定打包，已将 `installer/windows/fff-viewer.iss` 的安装语言配置改为仅使用 `compiler:Default.isl`（英文）。原因是本机 Inno Setup 6 缺失 `ChineseSimplified.isl`，会导致 ISCC 编译中断。
