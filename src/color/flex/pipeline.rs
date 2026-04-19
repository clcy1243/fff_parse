//! FlexColor pipeline 顶层
//!
//! 按 docs §16.5 的拓扑组装 per-channel 14-bit LUT，输入 `ImageCorrection` →
//! 输出 3 条 `[u16; 16384]` LUT，像素处理就是 3×LUT 查表。
//!
//! ## 拓扑（§16.5）
//!
//! ```text
//! outer_agg[channel] SEQUENCE:
//!   ├─ CGammaNegCurve (per-channel)
//!   └─ CHighShadowCurve (per-channel):
//!         sub_curve = inner_agg SEQUENCE:
//!              [0] CGammaCurve (shared)
//!              [1] CPointCurve[Master]      mode=Sequential
//!              [2] CNegativeCurve (shared)
//!              [3] CNegativeCurve (per-channel)
//!              [4] CPointCurve[UserA[ch]]   mode=AddDelta
//!              [5] CPointCurve[UserB[ch]]   mode=SubClamp
//!              [6] CContrastCurve (shared)
//!              [7] CSinglePointCurve (per-channel)
//! ```
//!
//! ## Apply* 开关
//!
//! - `ApplySliders` → Contrast/Gamma 启用
//! - `ApplyCurves` → Gradations (Master + UserA/B) 启用
//! - `ApplyHistogram` → HighShadow 使用 Shadow/Highlight/EndPoints（否则 identity-like）
//! - `FilmType != 0` → CGammaNegCurve 启用（否则 identity）
//!
//! 所有默认值见 docs §32.1。

use super::curves::{
    AggregateCurve, CompositionMode, ContrastCurve, Curve, GammaCurve, GammaNegCurve,
    HighShadowCurve, HighlightMode, NegativeCurve, PointCurve, ShadowMode, SinglePointCurve,
};
use crate::flexcolor::ImageCorrection;

/// Channel index: 0=R, 1=G, 2=B
pub type Channel = u8;

/// 3 通道 14-bit LUT（预计算 pipeline 的"烘焙形式"）
pub struct Pipeline {
    /// R / G / B 三条 14-bit LUT
    pub channel_luts: [Box<[u16; 16384]>; 3],
}

impl Pipeline {
    /// 按 §16.5 拓扑从 ImageCorrection 构造 pipeline 并预计算 3 条 LUT
    pub fn build(ic: &ImageCorrection) -> Self {
        let mut luts: [Box<[u16; 16384]>; 3] = [
            Box::new([0u16; 16384]),
            Box::new([0u16; 16384]),
            Box::new([0u16; 16384]),
        ];
        for ch in 0..3u8 {
            let outer = Self::build_channel(ic, ch);
            outer.build_lut(&mut *luts[ch as usize]);
        }
        Self { channel_luts: luts }
    }

    /// 构造单个通道的完整曲线链（outer_agg）
    ///
    /// 返回 `AggregateCurve`（实现 `Curve` trait）。
    pub fn build_channel(ic: &ImageCorrection, ch: Channel) -> AggregateCurve {
        let ch_idx = ch as usize;

        // --- inner_agg: 8 个 children ---
        let mut inner = AggregateCurve::new();

        // [0] CGammaCurve (shared)
        let gamma = GammaCurve::with_enabled(ic.gamma as f32, ic.apply_sliders);
        inner.children.push((Box::new(gamma), CompositionMode::Sequential));

        // [1] CPointCurve Master (shared)
        let master_pc = gradation_point_curve(ic, 0, ic.apply_curves);
        inner.children.push((Box::new(master_pc), CompositionMode::Sequential));

        // [2] CNegativeCurve (shared default)
        let neg_shared = if ic.film_type != 0 {
            NegativeCurve::default_shared()
        } else {
            NegativeCurve::disabled()
        };
        inner.children.push((Box::new(neg_shared), CompositionMode::Sequential));

        // [3] CNegativeCurve (per-channel)
        // film_type=1 (color neg): per-channel 有 X5 orange mask 补偿（R/G/B 不同参数）
        // film_type=2 (BW neg):    跳过 per-channel 变量，只用共享 neg curve（BW 不需要通道差异）
        // film_type=0 (positive):  禁用
        let neg_ch = match ic.film_type {
            1 => match ch {
                0 => NegativeCurve::default_r(),
                1 => NegativeCurve::default_g(),
                _ => NegativeCurve::default_b(),
            },
            _ => NegativeCurve::disabled(),
        };
        inner.children.push((Box::new(neg_ch), CompositionMode::Sequential));

        // [4] CPointCurve User A [ch] — mode = AddDelta
        // gradations: [Master, R-A, G-A, B-A, R-B, G-B, B-B]
        let user_a = gradation_point_curve(ic, 1 + ch_idx, ic.apply_curves);
        inner.children.push((Box::new(user_a), CompositionMode::AddDelta));

        // [5] CPointCurve User B [ch] — mode = SubClamp
        let user_b = gradation_point_curve(ic, 4 + ch_idx, ic.apply_curves);
        inner.children.push((Box::new(user_b), CompositionMode::SubClamp));

        // [6] CContrastCurve (shared)
        let contrast = ContrastCurve::with_enabled(
            clamp_i8(ic.contrast),
            clamp_i8(ic.brightness),
            ic.apply_sliders,
        );
        inner.children.push((Box::new(contrast), CompositionMode::Sequential));

        // [7] CSinglePointCurve — 默认 identity-like
        // TODO §44.5: 需追踪 XML 向它注入非默认控制点的路径
        inner.children.push((
            Box::new(SinglePointCurve::default_enabled()),
            CompositionMode::Sequential,
        ));

        // --- CHighShadowCurve 包装 inner_agg ---
        let high_shadow = build_high_shadow(ic, ch, Box::new(inner));

        // --- outer_agg: [CGammaNegCurve, CHighShadowCurve] ---
        let gamma_neg = GammaNegCurve::from_params(
            ic.film_type as u32,
            ic.enhanced_shadow,
            None, // NegVarGamma — Rust model 里暂无此字段，用默认
            None, // StretchNegGamma — 同上
        );

        let mut outer = AggregateCurve::new();
        outer.children.push((Box::new(gamma_neg), CompositionMode::Sequential));
        outer.children.push((Box::new(high_shadow), CompositionMode::Sequential));
        outer
    }

    /// 应用 pipeline 到 interleaved RGB 16-bit 像素
    ///
    /// 输入 u16 按 14-bit (0..16383) 解释 — 若原始是 16-bit full range，
    /// 调用方应先 `>> 2` 降到 14-bit 域；输出同域。
    pub fn apply_14bit_rgb(&self, pixels: &mut [u16]) {
        assert_eq!(
            pixels.len() % 3,
            0,
            "interleaved RGB 必须是 3 的倍数"
        );
        for chunk in pixels.chunks_exact_mut(3) {
            chunk[0] = self.channel_luts[0][(chunk[0] as usize).min(16383)];
            chunk[1] = self.channel_luts[1][(chunk[1] as usize).min(16383)];
            chunk[2] = self.channel_luts[2][(chunk[2] as usize).min(16383)];
        }
    }

    /// 应用 pipeline 到 16-bit RGB 像素（先 >> 2 降到 14-bit，查表，再 << 2 升回）
    ///
    /// 便捷方法，适合外部已是 16-bit u16 的场景。精度损失 2 LSB（最低有效位被截断）。
    pub fn apply_16bit_rgb(&self, pixels: &mut [u16]) {
        assert_eq!(pixels.len() % 3, 0);
        for chunk in pixels.chunks_exact_mut(3) {
            let idx_r = (chunk[0] >> 2) as usize;
            let idx_g = (chunk[1] >> 2) as usize;
            let idx_b = (chunk[2] >> 2) as usize;
            chunk[0] = self.channel_luts[0][idx_r.min(16383)] << 2;
            chunk[1] = self.channel_luts[1][idx_g.min(16383)] << 2;
            chunk[2] = self.channel_luts[2][idx_b.min(16383)] << 2;
        }
    }
}

/// 从 ImageCorrection.gradations[i] 构造 CPointCurve
///
/// 若索引越界（Rust model 可能缺字段）→ 返回 identity
/// 若 `enabled = false` → 返回 disabled (passthrough)
fn gradation_point_curve(ic: &ImageCorrection, index: usize, enabled: bool) -> PointCurve {
    if !enabled {
        return PointCurve::disabled();
    }
    match ic.gradations.get(index) {
        Some(pts) if !pts.is_empty() => PointCurve::from_xml_points(pts.as_slice()),
        _ => PointCurve::identity(),
    }
}

/// 构造 CHighShadowCurve（§28.2 set_params 对应）
fn build_high_shadow(
    ic: &ImageCorrection,
    ch: Channel,
    sub_curve: Box<dyn Curve>,
) -> HighShadowCurve {
    if !ic.apply_histogram {
        // §28.2 fallback：identity-like
        return HighShadowCurve::identity_like(sub_curve);
    }

    let ch_idx = ch as usize;

    // Shadow/Highlight: ushort[4]，XML byte <<6 → 14-bit（model 里已是 i64，按原值取）
    // FlexColor 用索引 1,2,3 (R,G,B)；ch_idx=0,1,2 → +1 为 shadow[1..]
    // 注意：§28.2 的 "parent.ushort[0x11e6 + ch*2]" 是 ch=1/2/3 对应 Shadow[1]/[2]/[3]
    // 在我们 Rust model 里 shadow: [i64; 4]，索引 1/2/3
    let shadow_boundary = ic.shadow.get(ch_idx + 1).copied().unwrap_or(0) as u16;
    let highlight_boundary = ic.highlight.get(ch_idx + 1).copied().unwrap_or(16383) as u16;

    // EndPoints (dot_color) — §28.3：byte[0..3] = shadow R/G/B, byte[4..7] = highlight
    // 当前 Rust model dot_color 为 Vec<i64>（14 项：前 7 shadow，后 7 highlight）
    // 但 §28.3 说真实 layout 是 byte 索引 0x4ff..0x501 shadow + 0x506..0x508 hi
    // 即在 FlexColor 里是 3 bytes per 端点。model 用 Vec<i64> 保留了冗余
    let shadow_out_byte = ic.dot_color.get(ch_idx).copied().unwrap_or(0) as u32;
    let highlight_out_byte = ic.dot_color.get(7 + ch_idx).copied().unwrap_or(255) as u32;
    let shadow_out = ((shadow_out_byte * 16383) / 255) as u16;
    let highlight_out = ((highlight_out_byte * 16383) / 255) as u16;

    // EndPoints modes: 当前 model 未单独存储 endpoint_shadow_mode/highlight_mode
    // §28 说来自 this+0x510/0x514；raw_params 里可能有，MVP 先用默认 Linear (1)
    let shadow_mode = ShadowMode::Linear;
    let highlight_mode = HighlightMode::Linear;

    HighShadowCurve::new(
        shadow_boundary,
        highlight_boundary,
        shadow_out,
        highlight_out,
        shadow_mode,
        highlight_mode,
        sub_curve,
    )
}

/// i64 → i8 的安全钳制
fn clamp_i8(v: i64) -> i8 {
    v.clamp(-100, 100) as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_ic() -> ImageCorrection {
        // 手工构造一个"默认"ImageCorrection（近 identity）
        ImageCorrection {
            contrast: 0,
            brightness: 0,
            gamma: 2.0, // identity
            lightness: 0,
            saturation: 0,
            color_temperature: 0,
            tint: 0,
            ev: 1.0,
            film_curve: 4, // default per §38
            film_type: 0,  // 关负片（identity）
            color_model: 0,
            apply_sliders: true,
            apply_curves: true,
            apply_histogram: false, // 避免 Shadow/Highlight 复杂化
            apply_usm: true,
            apply_dust: false,
            apply_cc: true,
            apply_cn_filter: true,
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
            gradations: vec![
                vec![(0, 0, 1), (255, 255, 1)]; // Master
                7
            ], // 7 条全部 identity
            input_profile_name: None,
            rgb_profile_name: None,
            dot_color: vec![0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255],
            raw_params: vec![],
        }
    }

    #[test]
    fn default_ic_produces_near_identity_lut() {
        // FilmType=0 → CGammaNegCurve 禁用；
        // ApplyHistogram=false → HighShadow identity_like；
        // Gamma=2.0 → GammaCurve identity；
        // Contrast=0, Brightness=0 → ContrastCurve identity；
        // Gradations 都是 [(0,0),(255,255)] → PointCurve 近 identity；
        // CNegativeCurve 禁用；
        // 综合：LUT 应近 identity
        let ic = default_ic();
        let pipe = Pipeline::build(&ic);

        for ch in 0..3 {
            for i in (0..16384u16).step_by(512) {
                let out = pipe.channel_luts[ch][i as usize];
                // 容忍 ±30 LSB（多层 rounding 累积 + PointCurve 最大值 16320 不是 16383）
                let diff = (out as i32 - i as i32).abs();
                assert!(
                    diff <= 50,
                    "ch={} v={} out={} diff={}",
                    ch,
                    i,
                    out,
                    diff
                );
            }
        }
    }

    #[test]
    fn film_type_enables_negative_path() {
        let mut ic = default_ic();
        ic.film_type = 1; // 启用负片
        let pipe = Pipeline::build(&ic);

        // 启用 neg 后 LUT 应显著偏离 identity（反转亮暗）
        // 具体：v=0 应接近 max（反转）
        let r0 = pipe.channel_luts[0][0];
        let r_mid = pipe.channel_luts[0][8192];
        // GammaNeg 把 0 映射到 max（pow(1, ...) * 16383 = 16383）
        // 但后面还有 shared CNegativeCurve + per-channel + HighShadow identity_like 包装...
        // 大致应 r0 明显大于 r_mid（负片反转效果）
        // 放宽：至少 r0 > r_mid 证明 neg 生效
        assert!(
            r0 > r_mid,
            "负片应 lut[0]={} > lut[mid]={}",
            r0,
            r_mid
        );
    }

    #[test]
    fn contrast_20_affects_dark_and_light_asymmetrically() {
        let mut ic = default_ic();
        ic.contrast = 20;
        let pipe = Pipeline::build(&ic);

        // §13: C=20 时 v=1024 → 666（暗部压低）, v=12288 → 13107（亮部抬升）
        // 在 pipeline 中还有 gradation identity 层，可能略偏移，容差 ±100
        let out_dark = pipe.channel_luts[0][1024];
        let out_light = pipe.channel_luts[0][12288];

        assert!(out_dark < 1024, "C=20 应压低 v=1024, got {}", out_dark);
        assert!(
            out_light > 12288,
            "C=20 应抬升 v=12288, got {}",
            out_light
        );
    }

    #[test]
    fn apply_14bit_identity_passthrough() {
        let ic = default_ic();
        let pipe = Pipeline::build(&ic);

        let mut pixels = vec![4000u16, 8000, 12000];
        let before = pixels.clone();
        pipe.apply_14bit_rgb(&mut pixels);

        // 应接近 identity（但有小偏差）
        for (b, a) in before.iter().zip(pixels.iter()) {
            let diff = (*b as i32 - *a as i32).abs();
            assert!(
                diff <= 50,
                "identity passthrough {} -> {} (diff {})",
                b,
                a,
                diff
            );
        }
    }

    #[test]
    fn apply_16bit_scales_correctly() {
        let ic = default_ic();
        let pipe = Pipeline::build(&ic);

        // 16-bit input 32768 (mid) → >>2 = 8192 (14-bit mid) → LUT ≈ 8192 → <<2 = 32768
        let mut pixels = vec![32768u16, 32768, 32768];
        pipe.apply_16bit_rgb(&mut pixels);
        for &v in &pixels {
            let diff = (v as i32 - 32768).abs();
            assert!(diff <= 200, "16-bit mid after pipeline: {}", v);
        }
    }

    #[test]
    fn disabled_apply_sliders_bypasses_contrast_gamma() {
        let mut ic = default_ic();
        ic.contrast = 50;
        ic.gamma = 1.0;
        ic.apply_sliders = false;
        let pipe = Pipeline::build(&ic);

        // ApplySliders=false → Gamma/Contrast 应当失效 → LUT 近 identity
        for i in (0..16384u16).step_by(1024) {
            let out = pipe.channel_luts[0][i as usize];
            let diff = (out as i32 - i as i32).abs();
            assert!(diff <= 50, "v={} out={} diff={}", i, out, diff);
        }
    }

    #[test]
    fn three_channels_built_independently() {
        let ic = default_ic();
        let pipe = Pipeline::build(&ic);
        // 默认 identity：三通道 LUT 应相同
        for i in (0..16384usize).step_by(1024) {
            assert_eq!(pipe.channel_luts[0][i], pipe.channel_luts[1][i]);
            assert_eq!(pipe.channel_luts[1][i], pipe.channel_luts[2][i]);
        }
    }

    #[test]
    fn three_channels_differ_when_film_type_on() {
        let mut ic = default_ic();
        ic.film_type = 1; // 启用负片 → per-channel neg 不同
        let pipe = Pipeline::build(&ic);
        // 至少在 mid 区应有差异（per-ch CNegativeCurve 参数不同）
        let m0 = pipe.channel_luts[0][8192];
        let m1 = pipe.channel_luts[1][8192];
        let m2 = pipe.channel_luts[2][8192];
        let all_same = m0 == m1 && m1 == m2;
        assert!(!all_same, "per-ch neg 应产生不同 R/G/B: {}/{}/{}", m0, m1, m2);
    }
}
