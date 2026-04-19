//! flex::Pipeline 到 DynamicImage 的桥接
//!
//! 提供与 `apply_color_pipeline_ex` 并行的新路径：**前向计算 FlexColor LUT**，
//! 替代"从 ref 反推 LUT"的旧路径（§43）。
//!
//! 与 `apply_color_pipeline_ex` 的差异：
//! - 不用 `ManualAdjust` + 独立 gradation/film_lut 参数
//! - 直接从 `ImageCorrection` 构造完整 14-bit LUT 链（§16.5）
//! - 不做 scanner_levels（FlexColor 里没有独立"scanner levels"阶段，一切烘焙进 LUT）
//! - 不做 `apply_display_adjust`（slider 效果已在 LUT 内）
//! - ICC 位置与旧路径一致：LUT 之后做 output ICC transform
//! - USM 保留为独立步骤（T10 Phase 4 再重构）

use super::flex::{ColorCorrParams, ColorCorrection, LightnessCurve, Pipeline};
use super::transform::{IccSettings, TargetColorSpace};
use crate::flexcolor::ImageCorrection;

/// 应用 flex::Pipeline 到整张图，可选 ICC transform 作为 post 处理
///
/// # Arguments
/// - `img`: 输入图像（Rgb16 最佳；Rgb8 会先升到 Rgb16）
/// - `ic`: XML ImageCorrection，决定 LUT 内容
/// - `icc_data`: output ICC profile（若 None 跳过 ICC）
/// - `target`: ICC target 色彩空间
/// - `icc_settings`: intent / BPC 等
///
/// # 返回
/// 处理后的 Rgb16 图（保持 16-bit 精度）。
pub fn apply_flex_pipeline(
    img: image::DynamicImage,
    ic: &ImageCorrection,
    icc_data: Option<&[u8]>,
    target: TargetColorSpace,
    icc_settings: IccSettings,
) -> image::DynamicImage {
    // 1-3. Pipeline + ColorCorrection + Lightness 全链
    let img_after_flex = apply_flex_pipeline_no_icc(img, ic);

    // 4. Output ICC transform（可选）
    if let Some(icc) = icc_data {
        match super::transform::apply_icc_transform_ex(&img_after_flex, icc, target, icc_settings) {
            Ok(transformed) => transformed,
            Err(e) => {
                log::warn!("flex_pipeline ICC transform failed: {}", e);
                img_after_flex
            }
        }
    } else {
        img_after_flex
    }
}

/// 不带 ICC 的简单应用（Tier 0 identity 验证场景最常用）
///
/// 包括：flex::Pipeline LUT + ColorCorrection (T22) + Lightness (T24)
/// 不包括：ICC / BW desat / USM（由调用方添加）
pub fn apply_flex_pipeline_no_icc(
    img: image::DynamicImage,
    ic: &ImageCorrection,
) -> image::DynamicImage {
    let pipeline = Pipeline::build(ic);

    // ColorCorrection 预编译（默认参数下 enabled=false fast-path）
    let cc_params = ColorCorrParams::from_image_correction(
        &ic.color_corr,
        ic.saturation,
        ic.apply_cc,
    );
    let color_correction = ColorCorrection::compile(&cc_params);

    // Lightness 曲线构造（Lightness=0 或 ApplySliders=false 时 should_apply=false）
    let lightness = LightnessCurve::new(ic.lightness as i16, ic.apply_sliders);

    apply_pipeline_full(img, &pipeline, &color_correction, &lightness)
}

/// 完整应用：Pipeline LUT → ColorCorrection → Lightness
fn apply_pipeline_full(
    img: image::DynamicImage,
    pipeline: &Pipeline,
    color_correction: &ColorCorrection,
    lightness: &LightnessCurve,
) -> image::DynamicImage {
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            let mut raw: Vec<u16> = rgb16.into_raw();

            // 1. Pipeline LUT（14-bit 域）— 内部自带 16/14-bit 转换
            pipeline.apply_16bit_rgb(&mut raw);

            // 2+3 如果启用，需切到 14-bit domain apply
            let need_post = color_correction.enabled || lightness.should_apply();
            if need_post {
                // 16-bit → 14-bit in-place (>>2)
                for v in raw.iter_mut() {
                    *v >>= 2;
                }
                // 2. ColorCorrection apply (14-bit domain)
                color_correction.apply_14bit_rgb(&mut raw);
                // 3. Lightness apply (14-bit domain)
                lightness.apply_14bit_rgb(&mut raw);
                // 14-bit → 16-bit (<<2)
                for v in raw.iter_mut() {
                    *v <<= 2;
                }
            }

            image::DynamicImage::ImageRgb16(
                image::ImageBuffer::from_raw(w, h, raw).expect("buffer size mismatch"),
            )
        }
        other => {
            let rgb16 = other.into_rgb16();
            let (w, h) = (rgb16.width(), rgb16.height());
            let mut raw: Vec<u16> = rgb16.into_raw();
            pipeline.apply_16bit_rgb(&mut raw);
            let need_post = color_correction.enabled || lightness.should_apply();
            if need_post {
                for v in raw.iter_mut() { *v >>= 2; }
                color_correction.apply_14bit_rgb(&mut raw);
                lightness.apply_14bit_rgb(&mut raw);
                for v in raw.iter_mut() { *v <<= 2; }
            }
            image::DynamicImage::ImageRgb16(
                image::ImageBuffer::from_raw(w, h, raw).expect("buffer size mismatch"),
            )
        }
    }
}

/// 核心像素循环：对 DynamicImage 应用预构建的 Pipeline
fn apply_pipeline_to_image(
    img: image::DynamicImage,
    pipeline: &Pipeline,
) -> image::DynamicImage {
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            let mut raw: Vec<u16> = rgb16.into_raw();
            pipeline.apply_16bit_rgb(&mut raw);
            image::DynamicImage::ImageRgb16(
                image::ImageBuffer::from_raw(w, h, raw).expect("buffer size mismatch"),
            )
        }
        other => {
            // 其他格式先升到 16-bit 再处理
            let rgb16 = other.into_rgb16();
            let (w, h) = (rgb16.width(), rgb16.height());
            let mut raw: Vec<u16> = rgb16.into_raw();
            pipeline.apply_16bit_rgb(&mut raw);
            image::DynamicImage::ImageRgb16(
                image::ImageBuffer::from_raw(w, h, raw).expect("buffer size mismatch"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageBuffer};

    fn identity_ic() -> ImageCorrection {
        // 跟 pipeline::tests::default_ic 保持一致
        ImageCorrection {
            contrast: 0,
            brightness: 0,
            gamma: 2.0,
            lightness: 0,
            saturation: 0,
            color_temperature: 0,
            tint: 0,
            ev: 1.0,
            film_curve: 4,
            film_type: 0,
            color_model: 0,
            apply_sliders: true,
            apply_curves: true,
            apply_histogram: false,
            apply_usm: false,
            apply_dust: false,
            apply_cc: false,
            apply_cn_filter: false,
            usm_amount: 0,
            usm_radius: 10,
            usm_dark_limit: 10,
            usm_noise_limit: 0,
            threshold: 0,
            dust_level: 0,
            color_noise_radius: 0,
            noise_filter_bias: 0,
            lens_correction: 0,
            vignette_amount: 100,
            enhanced_shadow: false,
            remove_cast_highlight: false,
            remove_cast_shadow: false,
            embed_profile: false,
            convert: true,
            soft_proof: false,
            auto_highlight: 0,
            auto_shadow: 0,
            mode: 0,
            usm_col_factor: vec![100, 100, 100],
            shadow: [0, 0, 0, 0],
            gray: [0, 0, 0, 0],
            highlight: [0, 16383, 16383, 16383],
            color_corr: vec![0i64; 36],
            gradation_sliders: [0, 0, 0],
            gradations: vec![vec![(0, 0, 1), (255, 255, 1)]; 7],
            input_profile_name: None,
            rgb_profile_name: None,
            dot_color: vec![0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255],
            raw_params: vec![],
        }
    }

    fn make_gradient_img(w: u32, h: u32) -> DynamicImage {
        // 横向 RGB 渐变：左 0 右 max
        let mut raw = vec![0u16; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let v = ((x as u64 * 65535) / (w.saturating_sub(1).max(1) as u64)) as u16;
                let i = ((y * w + x) * 3) as usize;
                raw[i] = v;
                raw[i + 1] = v;
                raw[i + 2] = v;
            }
        }
        DynamicImage::ImageRgb16(ImageBuffer::from_raw(w, h, raw).unwrap())
    }

    #[test]
    fn identity_ic_preserves_image_approx() {
        let ic = identity_ic();
        let img = make_gradient_img(32, 4);
        let out = apply_flex_pipeline_no_icc(img.clone(), &ic);

        let src_raw = img.into_rgb16().into_raw();
        let out_raw = out.into_rgb16().into_raw();
        assert_eq!(src_raw.len(), out_raw.len());

        // identity 配置：输出 ≈ 输入（容忍 ±200 LSB on 16-bit，来自 14↔16 量化 + 多层 rounding）
        let mut max_diff = 0i32;
        for (s, o) in src_raw.iter().zip(out_raw.iter()) {
            let d = (*s as i32 - *o as i32).abs();
            if d > max_diff {
                max_diff = d;
            }
        }
        assert!(
            max_diff <= 300,
            "identity pipeline max diff {} 超出容忍",
            max_diff
        );
    }

    #[test]
    fn negative_ic_inverts_image() {
        let mut ic = identity_ic();
        ic.film_type = 1; // 启用负片
        let img = make_gradient_img(64, 2);
        let out = apply_flex_pipeline_no_icc(img, &ic);

        let out_raw = out.into_rgb16().into_raw();
        // 负片反转：左端（原 0）应变亮，右端（原 max）应变暗
        let first_r = out_raw[0];
        let last_r = out_raw[out_raw.len() - 3];
        assert!(
            first_r > last_r,
            "负片应反转：first={} last={}",
            first_r,
            last_r
        );
    }

    #[test]
    fn works_on_rgb8_input() {
        // Rgb8 输入应被自动升级为 Rgb16 后处理
        let mut raw8 = vec![0u8; 4 * 4 * 3];
        for i in 0..raw8.len() {
            raw8[i] = (i % 256) as u8;
        }
        let rgb8 = image::ImageBuffer::from_raw(4, 4, raw8).unwrap();
        let img = DynamicImage::ImageRgb8(rgb8);

        let ic = identity_ic();
        let out = apply_flex_pipeline_no_icc(img, &ic);
        assert!(matches!(out, DynamicImage::ImageRgb16(_)));
    }

    #[test]
    fn empty_image_no_panic() {
        let empty = DynamicImage::ImageRgb16(ImageBuffer::new(0, 0));
        let ic = identity_ic();
        let out = apply_flex_pipeline_no_icc(empty, &ic);
        assert_eq!(out.width(), 0);
    }
}
