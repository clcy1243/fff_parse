// ─── ICC Color Transform ────────────────────────────────────────────────────

/// Target (output) color space for ICC transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetColorSpace {
    ProPhotoRGB,
    SRGB,
    AdobeRGB,
    DisplayP3,
}

impl TargetColorSpace {
    pub const ALL: &[TargetColorSpace] = &[
        Self::ProPhotoRGB,
        Self::SRGB,
        Self::AdobeRGB,
        Self::DisplayP3,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::SRGB => "sRGB",
            Self::AdobeRGB => "Adobe RGB (1998)",
            Self::ProPhotoRGB => "ProPhoto RGB",
            Self::DisplayP3 => "Display P3",
        }
    }

    pub fn to_str(&self) -> &'static str {
        match self {
            Self::SRGB => "sRGB",
            Self::AdobeRGB => "AdobeRGB",
            Self::ProPhotoRGB => "ProPhotoRGB",
            Self::DisplayP3 => "DisplayP3",
        }
    }

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
        Self::ProPhotoRGB
    }
}

/// Create an lcms2 output profile for a target color space.
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

/// Apply ICC color transform to an image.
/// `input_icc`: scanner/input ICC profile bytes
/// `target`: output color space
/// Returns transformed DynamicImage.
pub fn apply_icc_transform(
    img: &image::DynamicImage,
    input_icc: &[u8],
    target: TargetColorSpace,
) -> Result<image::DynamicImage, String> {
    use lcms2::*;

    let input_profile = Profile::new_icc(input_icc)
        .map_err(|e| format!("Failed to load input ICC: {:?}", e))?;

    let output_profile = create_output_profile(target)?;

    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let transform = Transform::new(
                &input_profile,
                PixelFormat::RGB_16,
                &output_profile,
                PixelFormat::RGB_16,
                Intent::Perceptual,
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
            let transform = Transform::new(
                &input_profile,
                PixelFormat::RGB_8,
                &output_profile,
                PixelFormat::RGB_8,
                Intent::Perceptual,
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
            // Convert to Rgb8 first
            let rgb8 = img.to_rgb8();
            let converted = image::DynamicImage::ImageRgb8(rgb8);
            apply_icc_transform(&converted, input_icc, target)
        }
    }
}
