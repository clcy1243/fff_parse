use crate::flexcolor::ImageCorrection;

#[derive(Debug, Clone, Copy)]
pub struct AffineCalibration {
    pub gains: [f32; 3],
    pub offsets_8bit: [f32; 3],
}

pub fn negative_c41_calibration(corr: &ImageCorrection) -> Option<AffineCalibration> {
    if corr.film_type != 1 {
        return None;
    }

    let has_profile = corr
        .input_profile_name
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if has_profile {
        Some(AffineCalibration {
            gains: [1.06, 1.04, 0.97],
            offsets_8bit: [-17.54, -13.26, 17.28],
        })
    } else {
        Some(AffineCalibration {
            gains: [1.01, 1.05, 0.96],
            offsets_8bit: [-3.07, -16.64, 8.66],
        })
    }
}

pub fn apply_affine_calibration(
    img: &image::DynamicImage,
    calibration: &AffineCalibration,
) -> image::DynamicImage {
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let mut out = rgb16.clone();
            for pixel in out.pixels_mut() {
                for ch in 0..3 {
                    let v = pixel[ch] as f32 * calibration.gains[ch]
                        + calibration.offsets_8bit[ch] * 257.0;
                    pixel[ch] = v.clamp(0.0, 65535.0) as u16;
                }
            }
            image::DynamicImage::ImageRgb16(out)
        }
        image::DynamicImage::ImageRgb8(rgb8) => {
            let mut out = rgb8.clone();
            for pixel in out.pixels_mut() {
                for ch in 0..3 {
                    let v = pixel[ch] as f32 * calibration.gains[ch]
                        + calibration.offsets_8bit[ch];
                    pixel[ch] = v.clamp(0.0, 255.0) as u8;
                }
            }
            image::DynamicImage::ImageRgb8(out)
        }
        _ => {
            let rgb16 = img.to_rgb16();
            apply_affine_calibration(&image::DynamicImage::ImageRgb16(rgb16), calibration)
        }
    }
}
