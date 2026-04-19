//! ICC 色彩空间转换，将图像从扫描仪输入色彩空间转换到目标色彩空间。

// ─── ICC Color Transform ────────────────────────────────────────────────────

/// 渲染意图（rendering intent），影响 ICC 变换时的色域映射策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IccIntent {
    Perceptual,
    RelativeColorimetric,
    AbsoluteColorimetric,
    Saturation,
}

impl Default for IccIntent {
    fn default() -> Self { Self::Perceptual }
}

impl IccIntent {
    pub fn to_lcms(&self) -> lcms2::Intent {
        match self {
            Self::Perceptual            => lcms2::Intent::Perceptual,
            Self::RelativeColorimetric  => lcms2::Intent::RelativeColorimetric,
            Self::AbsoluteColorimetric  => lcms2::Intent::AbsoluteColorimetric,
            Self::Saturation            => lcms2::Intent::Saturation,
        }
    }
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Perceptual            => "perceptual",
            Self::RelativeColorimetric  => "relative",
            Self::AbsoluteColorimetric  => "absolute",
            Self::Saturation            => "saturation",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "perceptual" | "p" => Some(Self::Perceptual),
            "relative"   | "r" => Some(Self::RelativeColorimetric),
            "absolute"   | "a" => Some(Self::AbsoluteColorimetric),
            "saturation" | "s" => Some(Self::Saturation),
            _ => None,
        }
    }
}

/// ICC 变换的完整参数。使用 Default 保持向后兼容（Perceptual + BPC on）。
#[derive(Debug, Clone, Copy)]
pub struct IccSettings {
    pub intent: IccIntent,
    /// 是否启用黑点补偿 (Black Point Compensation)。Perceptual 意图默认会启用。
    pub black_point_compensation: bool,
}

impl Default for IccSettings {
    fn default() -> Self {
        // 默认关 BPC，和重构前的 Transform::new() 一致（lcms2 默认不开 BPC）。
        Self { intent: IccIntent::default(), black_point_compensation: false }
    }
}

/// 目标（输出）色彩空间，用于 ICC 色彩转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetColorSpace {
    ProPhotoRGB,
    SRGB,
    AdobeRGB,
    DisplayP3,
}

impl TargetColorSpace {
    /// 所有支持的色彩空间列表。
    pub const ALL: &[TargetColorSpace] = &[
        Self::ProPhotoRGB,
        Self::SRGB,
        Self::AdobeRGB,
        Self::DisplayP3,
    ];

    /// 返回用于 UI 显示的色彩空间名称。
    pub fn label(&self) -> &'static str {
        match self {
            Self::SRGB => "sRGB",
            Self::AdobeRGB => "Adobe RGB (1998)",
            Self::ProPhotoRGB => "ProPhoto RGB",
            Self::DisplayP3 => "Display P3",
        }
    }

    /// 返回用于序列化/持久化的字符串标识。
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::SRGB => "sRGB",
            Self::AdobeRGB => "AdobeRGB",
            Self::ProPhotoRGB => "ProPhotoRGB",
            Self::DisplayP3 => "DisplayP3",
        }
    }

    /// 从字符串解析色彩空间，无法识别时默认为 ProPhoto RGB。
    pub fn from_str(s: &str) -> Self {
        match s {
            "sRGB" => Self::SRGB,
            "AdobeRGB" => Self::AdobeRGB,
            "DisplayP3" => Self::DisplayP3,
            _ => Self::ProPhotoRGB,
        }
    }
}

impl Default for TargetColorSpace {
    fn default() -> Self {
        // egui 不做显示色彩管理，像素直接发送到屏幕。
        // 默认使用 sRGB 以匹配大多数显示器的色彩空间。
        Self::SRGB
    }
}

/// 对图像应用"输入 ICC → 任意输出 ICC"的变换（不走 TargetColorSpace 枚举）。
///
/// 当我们知道参考文件用的是什么 ICC 时，用这个版本能得到与参考严格一致的像素值。
///
/// - 输入位深保持（16/8-bit RGB）
/// - 仅支持 RGB → RGB（如果 output profile 不是 RGB 色彩空间会返回错误）
pub fn apply_icc_transform_profiles(
    img: &image::DynamicImage,
    input_icc: &[u8],
    output_icc: &[u8],
    settings: IccSettings,
) -> Result<image::DynamicImage, String> {
    use lcms2::*;

    // 两个 profile 字节完全相同 → 真正的 identity，不过 lcms2 会因 PCS (Lab) 量化引入小误差，
    // 所以这里提前短路、直接克隆返回。
    if input_icc == output_icc {
        return Ok(img.clone());
    }

    let input_profile = Profile::new_icc(input_icc)
        .map_err(|e| format!("Failed to load input ICC: {:?}", e))?;
    let output_profile = Profile::new_icc(output_icc)
        .map_err(|e| format!("Failed to load output ICC: {:?}", e))?;

    // 非 RGB 输出 profile 由调用方决定回退策略，这里直接返回 Err
    if output_profile.color_space() != ColorSpaceSignature::RgbData {
        return Err(format!(
            "output profile color space is not RGB: {:?}",
            output_profile.color_space()
        ));
    }

    let flags = if settings.black_point_compensation {
        Flags::BLACKPOINT_COMPENSATION
    } else {
        Flags::default()
    };
    let intent = settings.intent.to_lcms();

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let transform = Transform::new_flags(
                &input_profile, PixelFormat::RGB_16,
                &output_profile, PixelFormat::RGB_16,
                intent, flags,
            ).map_err(|e| format!("Failed to create transform: {:?}", e))?;
            let pixels: Vec<[u16; 3]> = rgb16.pixels().map(|p| [p[0], p[1], p[2]]).collect();
            let mut output = pixels.clone();
            transform.transform_pixels(&pixels, &mut output);
            let flat: Vec<u16> = output.into_iter().flat_map(|p| p).collect();
            let result = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                rgb16.width(), rgb16.height(), flat,
            ).ok_or_else(|| "Failed to build output buffer".to_string())?;
            Ok(image::DynamicImage::ImageRgb16(result))
        }
        image::DynamicImage::ImageRgb8(rgb8) => {
            let transform = Transform::new_flags(
                &input_profile, PixelFormat::RGB_8,
                &output_profile, PixelFormat::RGB_8,
                intent, flags,
            ).map_err(|e| format!("Failed to create transform: {:?}", e))?;
            let pixels: Vec<[u8; 3]> = rgb8.pixels().map(|p| [p[0], p[1], p[2]]).collect();
            let mut output = pixels.clone();
            transform.transform_pixels(&pixels, &mut output);
            let flat: Vec<u8> = output.into_iter().flat_map(|p| p).collect();
            let result = image::RgbImage::from_raw(rgb8.width(), rgb8.height(), flat)
                .ok_or_else(|| "Failed to build output buffer".to_string())?;
            Ok(image::DynamicImage::ImageRgb8(result))
        }
        _ => {
            let rgb8 = img.to_rgb8();
            apply_icc_transform_profiles(&image::DynamicImage::ImageRgb8(rgb8), input_icc, output_icc, settings)
        }
    }
}

/// RGB → Gray ICC 变换。输出用 R=G=B 填充的 Rgb16（1 通道逻辑、3 通道存储）
/// 以便下游 RGB-based 比较/显示流水线无需改动。
///
/// ⚠️ 实验性：应用 Flextight→HasselbladGray 的 lcms2 变换，在实测中比简单的
/// sRGB + luma 提取 **更差**（因为 FlexColor 实际并不做真正的 Gray ICC 变换，
/// 只把 Gray profile 作为 ICC tag 元数据）。保留 API 供未来需要真变换场景使用。
#[allow(dead_code)]
fn apply_icc_rgb_to_gray(
    img: &image::DynamicImage,
    input_icc: &[u8],
    output_icc: &[u8],
    settings: IccSettings,
) -> Result<image::DynamicImage, String> {
    use lcms2::*;

    let input_profile = Profile::new_icc(input_icc)
        .map_err(|e| format!("Failed to load input ICC: {:?}", e))?;
    let output_profile = Profile::new_icc(output_icc)
        .map_err(|e| format!("Failed to load output ICC: {:?}", e))?;

    let flags = if settings.black_point_compensation {
        Flags::BLACKPOINT_COMPENSATION
    } else {
        Flags::default()
    };
    let intent = settings.intent.to_lcms();

    let rgb16 = match img {
        image::DynamicImage::ImageRgb16(b) => b.clone(),
        _ => img.to_rgb16(),
    };

    let transform = Transform::new_flags(
        &input_profile, PixelFormat::RGB_16,
        &output_profile, PixelFormat::GRAY_16,
        intent, flags,
    ).map_err(|e| format!("Failed to create RGB→Gray transform: {:?}", e))?;

    let pixels: Vec<[u16; 3]> = rgb16.pixels().map(|p| [p[0], p[1], p[2]]).collect();
    let mut gray_out: Vec<u16> = vec![0u16; pixels.len()];
    transform.transform_pixels(&pixels, &mut gray_out);

    // 展开成 R=G=B 的 16-bit RGB，便于下游比较
    let mut flat = Vec::with_capacity(gray_out.len() * 3);
    for g in gray_out {
        flat.extend_from_slice(&[g, g, g]);
    }
    let result = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
        rgb16.width(), rgb16.height(), flat,
    ).ok_or_else(|| "Failed to build RGB buffer from gray output".to_string())?;
    Ok(image::DynamicImage::ImageRgb16(result))
}

/// 根据目标色彩空间创建 lcms2 输出配置文件。
fn create_output_profile(space: TargetColorSpace) -> Result<lcms2::Profile, String> {
    use lcms2::*;

    match space {
        TargetColorSpace::SRGB => Ok(Profile::new_srgb()),

        TargetColorSpace::AdobeRGB => {
            let d65 = CIExyY { x: 0.3127, y: 0.3290, Y: 1.0 };
            let primaries = CIExyYTRIPLE {
                Red: CIExyY { x: 0.6400, y: 0.3300, Y: 0.0 },
                Green: CIExyY { x: 0.2100, y: 0.7100, Y: 0.0 },
                Blue: CIExyY { x: 0.1500, y: 0.0600, Y: 0.0 },
            };
            let gamma = ToneCurve::new(2.19921875);
            Profile::new_rgb(&d65, &primaries, &[&gamma, &gamma, &gamma])
                .map_err(|e| format!("Failed to create Adobe RGB profile: {:?}", e))
        }

        TargetColorSpace::ProPhotoRGB => {
            let d50 = CIExyY { x: 0.3457, y: 0.3585, Y: 1.0 };
            let primaries = CIExyYTRIPLE {
                Red: CIExyY { x: 0.7347, y: 0.2653, Y: 0.0 },
                Green: CIExyY { x: 0.1596, y: 0.8404, Y: 0.0 },
                Blue: CIExyY { x: 0.0366, y: 0.0001, Y: 0.0 },
            };
            let gamma = ToneCurve::new(1.8);
            Profile::new_rgb(&d50, &primaries, &[&gamma, &gamma, &gamma])
                .map_err(|e| format!("Failed to create ProPhoto RGB profile: {:?}", e))
        }

        TargetColorSpace::DisplayP3 => {
            let d65 = CIExyY { x: 0.3127, y: 0.3290, Y: 1.0 };
            let primaries = CIExyYTRIPLE {
                Red: CIExyY { x: 0.6800, y: 0.3200, Y: 0.0 },
                Green: CIExyY { x: 0.2650, y: 0.6900, Y: 0.0 },
                Blue: CIExyY { x: 0.1500, y: 0.0600, Y: 0.0 },
            };
            let gamma = ToneCurve::new(2.2);
            Profile::new_rgb(&d65, &primaries, &[&gamma, &gamma, &gamma])
                .map_err(|e| format!("Failed to create Display P3 profile: {:?}", e))
        }
    }
}

/// 对图像应用 ICC 色彩转换（使用默认 IccSettings = Perceptual + BPC）。
pub fn apply_icc_transform(
    img: &image::DynamicImage,
    input_icc: &[u8],
    target: TargetColorSpace,
) -> Result<image::DynamicImage, String> {
    apply_icc_transform_ex(img, input_icc, target, IccSettings::default())
}

/// 对图像应用 ICC 色彩转换，可指定 rendering intent 与 BPC。
///
/// - `input_icc`：扫描仪/输入设备的 ICC 配置文件数据
/// - `target`：目标输出色彩空间
/// - `settings`：intent 与黑点补偿开关
///
/// 保持原图位深（16-bit/8-bit），不支持的格式会先转为 RGB8 再处理。
pub fn apply_icc_transform_ex(
    img: &image::DynamicImage,
    input_icc: &[u8],
    target: TargetColorSpace,
    settings: IccSettings,
) -> Result<image::DynamicImage, String> {
    use lcms2::*;

    let input_profile = Profile::new_icc(input_icc)
        .map_err(|e| format!("Failed to load input ICC: {:?}", e))?;

    let output_profile = create_output_profile(target)?;

    let flags = if settings.black_point_compensation {
        Flags::BLACKPOINT_COMPENSATION
    } else {
        Flags::default()
    };
    let intent = settings.intent.to_lcms();

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let transform = Transform::new_flags(
                &input_profile,
                PixelFormat::RGB_16,
                &output_profile,
                PixelFormat::RGB_16,
                intent,
                flags,
            ).map_err(|e| format!("Failed to create transform: {:?}", e))?;

            let pixels: Vec<[u16; 3]> = rgb16
                .pixels()
                .map(|p| [p[0], p[1], p[2]])
                .collect();

            let mut output = pixels.clone();
            transform.transform_pixels(&pixels, &mut output);

            let flat: Vec<u16> = output.into_iter().flat_map(|p| p).collect();
            let result = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(
                rgb16.width(),
                rgb16.height(),
                flat,
            )
            .ok_or_else(|| "Failed to create output image".to_string())?;

            Ok(image::DynamicImage::ImageRgb16(result))
        }
        image::DynamicImage::ImageRgb8(rgb8) => {
            let transform = Transform::new_flags(
                &input_profile,
                PixelFormat::RGB_8,
                &output_profile,
                PixelFormat::RGB_8,
                intent,
                flags,
            ).map_err(|e| format!("Failed to create transform: {:?}", e))?;

            let pixels: Vec<[u8; 3]> = rgb8
                .pixels()
                .map(|p| [p[0], p[1], p[2]])
                .collect();

            let mut output = pixels.clone();
            transform.transform_pixels(&pixels, &mut output);

            let flat: Vec<u8> = output.into_iter().flat_map(|p| p).collect();
            let result = image::RgbImage::from_raw(rgb8.width(), rgb8.height(), flat)
                .ok_or_else(|| "Failed to create output image".to_string())?;

            Ok(image::DynamicImage::ImageRgb8(result))
        }
        _ => {
            let rgb8 = img.to_rgb8();
            let converted = image::DynamicImage::ImageRgb8(rgb8);
            apply_icc_transform_ex(&converted, input_icc, target, settings)
        }
    }
}
