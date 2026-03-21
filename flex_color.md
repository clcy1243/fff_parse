## 参考资料文档
- https://cdn.hasselblad.com/manuals/flextight/Flexcolor-Manual-Scanners.pdf
- https://cdn.hasselblad.com/manuals/flextight/Flexcolor-4.5-Addendum.pdf

## FFF 文件调整项实现计划

FlexColor 在 FFF 文件的标签 0xC519 (ImaconCalibration) 中嵌入编辑参数，
以 Apple plist XML 格式存储在 `ImageCorrection` 结构中。
下表列出了所有调整项及其在 `ManualAdjust` 模块中的实现状态。

### 已实现的调整项（✅）

| 调整项 | FlexColor 参数 | ManualAdjust 字段 | 说明 |
|--------|----------------|-------------------|------|
| 胶片类型 | FilmType | film_type | 0=正片 E-6, 1=负片 C-41, 2=黑白 |
| 胶片曲线 | FilmCurve | film_curve | 0=Linear, 1=Std, 2=High, 3=Low, 4=Auto |
| 胶片 Gamma | Gamma | film_gamma | 默认 2.0 |
| 曝光补偿 | EV | exposure | ±3.0 档 (log2 换算) |
| 亮度 | Brightness | brightness | -100 ~ 100 |
| 阴影深度 | Lightness | lightness | -100 ~ 100 |
| 中间调 | Gamma - 1.0 | midtone | 0.1 ~ 4.0, 默认 1.0 |
| 对比度 | Contrast | contrast | -100 ~ 100 |
| 高光 | — (手动) | highlights | -100 ~ 100 |
| 阴影 | — (手动) | shadows | -100 ~ 100 |
| 饱和度 | Saturation | saturation | -100 ~ 100 |
| 色温 | ColorTemperature | color_temperature | -100 ~ 100 |
| 色调偏移 | Tint | tint | -100 ~ 100 |
| 色彩平衡 R/G/B | — (手动) | r/g/b_shift | -100 ~ 100 |
| 色彩校正矩阵 | ColorCorr | color_corr | 6×6 RGBCMY 矩阵 |
| 输入色阶 | Shadow/Gray/Highlight | levels_black/gamma/white | 4 通道 (RGB+R+G+B) |
| 渐变曲线 | Gradations | apply_curves + curve editor | 多通道控制点 |

### 已加载但尚未实现处理的调整项（🔧）

以下调整项已添加字段定义、嵌入校正加载、sidecar 序列化和 UI 控件，
但实际的图像处理算法尚未实现。

| 调整项 | FlexColor 参数 | ManualAdjust 字段 | 实现难度 | 备注 |
|--------|----------------|-------------------|----------|------|
| USM 锐化 | USMAmount/USMRadius/USMDarkLimit/USMNoiseLimit/USMColFactor | apply_usm, usm_amount/radius/dark_limit/noise_limit/col_factor | 高 | 需实现 Unsharp Mask 卷积 |
| 除尘 | DustLevel | apply_dust, dust_level | 高 | 需实现中值滤波或类似算法 |
| 色彩噪声滤镜 | ColorNoiseRadius/NoiseFilterBias | apply_cn_filter, color_noise_radius/noise_filter_bias | 高 | 需实现色度通道降噪 |
| 镜头校正 | LensCorrection | lens_correction | 中 | 需镜头校正数据库或几何变换 |
| 暗角校正 | VignetteAmount | vignette_amount | 中 | 径向亮度补偿 |
| 阴影增强 | EnhancedShadow | enhanced_shadow | 低 | 暗部 Gamma 增强 |
| 去除高光色偏 | RemoveCastHighlight | remove_cast_highlight | 中 | 高光中性化处理 |
| 去除阴影色偏 | RemoveCastShadow | remove_cast_shadow | 中 | 暗部中性化处理 |

### 仅用于展示/元数据的参数（📋）

以下参数在 `ImageCorrection` 中已解析并在元数据面板展示，但不需要在调整模块中实现：

| 参数 | 说明 |
|------|------|
| ColorModel | 颜色模型 (RGB/CMYK/Grayscale) — 仅展示 |
| ApplySliders | 是否应用滑块 — 控制加载行为 |
| ApplyCurves | 是否应用曲线 — 控制加载行为 |
| ApplyHistogram | 是否应用色阶 — 控制加载行为 |
| EmbedProfile | 是否嵌入 ICC — 导出选项 |
| Convert | 是否转换色彩空间 — 导出选项 |
| SoftProof | 软打样 — 显示选项 |
| AutoHighlight/AutoShadow | 自动曝光参考值 — 仅参考 |
| Mode | 处理模式 — 仅展示 |
| GradationSliders | 色调滑块 [对比度, 亮度, 阴影深度] — 与单独参数重复 |
| InputProfile/RGBProfile | ICC 配置文件名 — 已在色彩管理面板处理 |
| Threshold | 阈值 — 与 USM 相关 |

### 后续实现优先级建议

1. **P1 (高优先)**: 阴影增强 (EnhancedShadow) — 算法简单，效果明显
2. **P2 (中优先)**: 暗角校正 (VignetteAmount) — 径向补偿算法成熟
3. **P2 (中优先)**: 去除色偏 (RemoveCastHighlight/Shadow) — 可用简单白平衡方法
4. **P3 (低优先)**: USM 锐化 — 需要卷积实现，考虑使用 image crate 或自行实现
5. **P3 (低优先)**: 色彩噪声滤镜 — 需要色度域降噪算法
6. **P4 (待定)**: 除尘 — 需要复杂的图像修复算法
7. **P4 (待定)**: 镜头校正 — 需要镜头畸变参数数据

## 直方图功能

主要内容在第一个文档的第85页，以及第二个文档的第14页。


The Histogram window contains a graph that indicates the tonal range
of your image. The graph displays the number of pixels (on the vertical
axis) of each brightness (on the horizontal axis). Pixels with a value of 0
(black) are shown on the left; pixels with a value of 255 (white) are shown
on the right.

直方图的内容分四块：
- 第一块为 RGB 通道调整，有五个滑块，横轴三个，从左到右依次为 Shadow、Midtone、Highlight，纵轴两个，我并没有找到官方对这两个滑块的说明，但看起来是调整色阶的，从上到下的调整范围分别是 255-180 和 60 - 0
- 下面三个滑块分别为 R、G、B通道，均有三个滑块，都在横轴，从左到右依次为 Shadow、Neutral、Highlight