# 推进计划 · 2026-06 产品化 Sprint（T59–T70）

> 制定日期：2026-06-13
> 战略重心：**产品化 / UX 优先**（正片/负片色彩已 97% 对齐，转向「让软件真正可日常使用」）
> 排序策略：**A 基础设施先行** —— 先搭一键打包，之后每步都在真实 app 里 dogfood 验收
> BW 色彩根因：**非阻塞可调度项**（待 Windows 环境，见 Track P）

---

## 背景与定位

fff_parse 是 Rust + egui 桌面应用，通过 Ghidra 逆向复刻哈苏 Flextight X5 扫描仪 `.fff` 文件的色彩管线。
当前状态（详见 `docs/pipeline-status.md`）：

- 真实扫描 177 case：64% PASS+，**POS/NEG 97% 对齐**（肉眼无差），BW 全 FAIL（结构性、环境阻塞）
- 解析层、GUI 查看器成熟；色彩管线复刻 ~60–70% 且核心 bit-accurate
- **产品化缺口**：打包几乎为零（`.app` 手动构建）、调色面板体验待打磨、部分高级滤镜「只存不算」

本 sprint 不做高级滤镜（暗角/除尘/色偏/色彩噪声/阴影增强 —— 推迟到下期），聚焦三条产品化主线 + 一条可调度的 BW 取证线。

---

## Sprint 目标（按执行顺序）

### 🅐 Track 1 · 一键打包发布（基础，最先做）

打包骨架先行，使后续每个改动都能在真实安装产物里验收。

| ID | 任务 | 验收标准 |
|---|---|---|
| **T59** ✅ | macOS 打包：`scripts/build-macos.sh`，产出带图标 + 版本号的 `.app` 与 `.dmg`（自写脚本，零额外依赖） | 双击 `.dmg` 装入 app，能启动并打开一张 `.fff` |
| **T60** | Windows installer：复用现有 `winresource` 图标，产出 `.exe` + NSIS/Inno installer | Windows 上能安装并打开 `.fff` |
| **T61** | Linux 构建验证 + **版本号统一**（`Cargo.toml` 当前 `0.1.0`，与文档 v0.8/v0.9 不符，统一为单一真源并在 UI 关于页展示） | 三平台 `cargo build --release` 全过；三处版本号一致 |

### 🅑 Track 2 · 调色面板体验打磨（主体）

`调整(ColorAdjust)` / `色彩(ColorProfile)` 两面板已接线可见，本轨提升其可用性。
`render_color_adjust_panel` 单函数约 1100 行，是主要复杂度来源。

| ID | 任务 | 验收标准 |
|---|---|---|
| **T62** | Dogfood 巡检：在真实 app（Track 1 产物）里走查两面板，输出带优先级的 UX 痛点清单至 `docs/` | ≥10 条痛点，每条标 P0/P1/P2 |
| **T63** | 拆分 `render_color_adjust_panel` 为子组件（色阶 / 曲线编辑 / 滑块组 / 6×6 矩阵），降复杂度 | 单函数 < 300 行；行为不变（视觉回归对照截图一致） |
| **T64** | 重置/保存/基线语义统一 + 面板内调整前后对比（before/after toggle） | 重置可回基线；sidecar delta 写入正确；before/after 可切换 |
| **T65** | 滑块实时预览防抖（拖动不卡顿）+ 参数旁标注对应 FlexColor 语义（如 Lightness=Shadow Depth，仅 >0 生效） | 拖动大图不掉帧；关键滑块有语义 tooltip |
| **T66** | 调整操作的撤销/重做 | Ctrl+Z / Ctrl+Y（或 Cmd）对滑块与曲线编辑生效 |

### 🅒 Track 3 · 性能 + 工程债（穿插于 1/2 之间）

| ID | 任务 | 验收标准 |
|---|---|---|
| **T67** | 清 5 条 dead-code 警告（`apply_pipeline_to_image` / `render_histogram_bars` / `convert_16_to_8_for_display` 等） | `cargo build` 0 warning |
| **T68** | 评估并移除/标注 zombie `CFilmCurve`（T56 已确认 Mode=5 BW 不用 FilmCurve） | 移除该路径，或在代码 + 文档明确标注「保留原因」 |
| **T69** | 大图导出/内存优化 + 缩略图缓存（延续切图 3× 提速方向） | 350MB+ 图导出内存峰值下降；缩略图二次加载命中缓存 |

### 🅟 Track P · BW 色彩根因取证（待 Windows，非阻塞，可随时插入）

BW 所需 gamma=0.231 数据在 FlexColor DLL `.rdata` 不可达，推测在 scanner 注册表/config。

| ID | 任务 | 验收标准 |
|---|---|---|
| **T70** | 我先写好**精确取证步骤清单**（`reg query "HKCU\Software\Hasselblad\FlexColor"` dump + 备选：深解 `profiles/Flextight X5 & 949.icc` 的 DevD/CIED/Pmtr 私有 tag）。用户在 Windows 上照做回传，据此定位 BW gamma 来源 | 取证清单可独立执行；回传数据后能判定 0.231 来源或确认为结构性限制 |

---

## 执行顺序总览

```
T59 → T60 → T61        (Track 1 打包，产出可安装 app)
   └─ T67 穿插          (顺手清 warning)
T62 → T63 → T64 → T65 → T66   (Track 2 面板打磨，在真实 app 里验收)
   └─ T68 / T69 穿插    (工程债 / 性能)
T70 (Parked) ─────────  (用户拿到 Windows 时随时插入)
```

---

## 验收与质量约束

- 每个任务完成后在真实安装产物里人工视觉验收（延续项目「发布到桌面看效果」习惯）
- 色彩管线相关改动（T68）须跑 `tif_compare --manifest` 回归，确保不破坏已 STRICT/PASS 的 case
- FlexColor 批量导出约束仍生效：每子目录 ≤20 张（见 memory）
- 状态判断以 `docs/pipeline-status.md` + git log 为准

## 不在本期范围（下期候选）

- 高级滤镜像素实现：EnhancedShadow(P1)、Vignette/LensCorrection、RemoveCast、ColorNoise、DustLevel
- 继续推 ICC 跨通道残差 / gamma_050 / CMYK·Gray 目标空间对比 / e2e_all_config 回归
- 逆向遗留组件补全：CSinglePointCurve、多点 B-spline 精度验证（`point.rs`）、`pipeline.rs:121` 单点曲线注入路径
