//! Lightness（Shadow Depth）模块 — T24 实现
//!
//! 源自 docs §50（T24 agent 报告）。Lightness slider 通过 CPointCurve@+0x84 的
//! Point[1].Y 写入，生成 4 点曲线后对 RGB 像素逐 pixel 加 delta。
//!
//! ## 公式（§50.2）
//!
//! ```text
//! Point[1].Y = floor(Lightness * 2.5) + 2      (byte 空间 0..255)
//! 4 点曲线 = [(0,0), (2, Y1), (50, 50), (255, 255)]
//!
//! per-pixel:
//!   luma = (R + G + B) / 3      # 或某通道，待验证
//!   delta = lut_14bit[luma] - luma
//!   out_R = clamp14(R + delta)
//!   out_G = clamp14(G + delta)
//!   out_B = clamp14(B + delta)
//! ```
//!
//! Gate: `apply_sliders && lightness > 0`
//!
//! 只影响**极暗像素**（14-bit 0..128），中间/高光近 identity（Shadow Depth 效果）。

use super::curves::{Curve, MAX_14BIT};

/// Lightness 曲线（4 点 byte 空间 → 14-bit LUT）
pub struct LightnessCurve {
    pub lightness: i16,
    pub apply_sliders: bool,
    /// 预计算 14-bit LUT (16384 entries)
    pub lut: Box<[u16; 16384]>,
}

impl LightnessCurve {
    /// 从 ImageCorrection 字段构造
    pub fn new(lightness: i16, apply_sliders: bool) -> Self {
        let lut = build_lightness_lut(lightness);
        Self {
            lightness,
            apply_sliders,
            lut,
        }
    }

    /// 是否需要应用（gate 检查）
    #[inline]
    pub fn should_apply(&self) -> bool {
        self.apply_sliders && self.lightness > 0
    }

    /// 单像素 luma-based delta 应用
    ///
    /// rgb 是 interleaved 14-bit (0..16383)
    pub fn apply_rgb_chunk(&self, chunk: &mut [u16]) {
        debug_assert_eq!(chunk.len(), 3);
        if !self.should_apply() {
            return;
        }
        // 用 luma 查 LUT（§50.3 推测：vtbl[0x34] 预计算，当前用 BT.601 近似）
        let r = chunk[0] as u32;
        let g = chunk[1] as u32;
        let b = chunk[2] as u32;
        let luma = ((r * 299 + g * 587 + b * 114) / 1000).min(MAX_14BIT as u32) as usize;

        let curve_val = self.lut[luma] as i32;
        let delta = curve_val - luma as i32;

        chunk[0] = (chunk[0] as i32 + delta).clamp(0, MAX_14BIT as i32) as u16;
        chunk[1] = (chunk[1] as i32 + delta).clamp(0, MAX_14BIT as i32) as u16;
        chunk[2] = (chunk[2] as i32 + delta).clamp(0, MAX_14BIT as i32) as u16;
    }

    /// 应用到整张 interleaved RGB 14-bit 数组
    pub fn apply_14bit_rgb(&self, pixels: &mut [u16]) {
        if !self.should_apply() {
            return;
        }
        for chunk in pixels.chunks_exact_mut(3) {
            self.apply_rgb_chunk(chunk);
        }
    }
}

/// 构造 4 点曲线 + 线性分段插值到 14-bit LUT
///
/// 4 points: `[(0, 0), (2, Y1), (50, 50), (255, 255)]`
/// `Y1 = min(Lightness * 2.5 + 2, 255)`
fn build_lightness_lut(lightness: i16) -> Box<[u16; 16384]> {
    // Y1 公式（§50.2）
    let y1 = ((lightness as i32 * 250 / 100) + 2).clamp(0, 255) as u8;

    // 4 控制点（byte 空间 0..255），CPointCurve 内部 <<6 到 14-bit
    // T38：用 PointCurve B-spline 替代线性分段，与 FlexColor CPointCurve 一致
    let pts: Vec<(i64, i64, i64)> = vec![
        (0, 0, 1),
        (2, y1 as i64, 1),
        (50, 50, 1),
        (255, 255, 1),
    ];
    let pc = super::curves::PointCurve::from_xml_points(&pts);
    let mut lut = Box::new([0u16; 16384]);
    pc.build_lut(&mut *lut);
    lut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightness_zero_is_identity() {
        let lc = LightnessCurve::new(0, true);
        assert!(!lc.should_apply());
        // LUT 应近 identity
        for i in (0..16384u16).step_by(1024) {
            let got = lc.lut[i as usize];
            let diff = (got as i32 - i as i32).abs();
            assert!(diff <= 2, "Lightness=0, i={} got={}, diff={}", i, got, diff);
        }
    }

    #[test]
    fn lightness_disabled_sliders_gate() {
        let lc = LightnessCurve::new(50, false);
        assert!(!lc.should_apply());
        let mut px = [1000u16, 2000, 3000];
        let before = px;
        lc.apply_rgb_chunk(&mut px);
        assert_eq!(px, before); // 未改变
    }

    #[test]
    fn lightness_50_lifts_shadows() {
        // Lightness=50 → Y1=127
        let lc = LightnessCurve::new(50, true);
        assert!(lc.should_apply());

        // 极暗像素（< 128 14-bit，对应 byte < 2）应被显著 lift
        let dark_in = 64u16; // 14-bit
        let dark_out = lc.lut[dark_in as usize];
        assert!(
            dark_out > dark_in + 1000,
            "Lightness=50 at v={} 应大幅 lift, got {}",
            dark_in,
            dark_out
        );

        // 中等亮度（byte ≈ 50, 14-bit ≈ 3200）B-spline 接近但不精确通过 (50,50)
        // T38：B-spline 容差放宽（控制点不是插值节点）
        let mid_in = 3200u16;
        let mid_out = lc.lut[mid_in as usize];
        assert!(mid_out > mid_in, "mid 应略 lift, got {}", mid_out);
        assert!(mid_out < mid_in + 4000, "mid 不应过度 lift, got {}", mid_out);

        // 高光应不受影响
        let hi_in = 14000u16;
        let hi_out = lc.lut[hi_in as usize];
        let hi_diff = (hi_out as i32 - hi_in as i32).abs();
        assert!(hi_diff <= 100, "hi 应 identity, got {}", hi_out);
    }

    #[test]
    fn lightness_y1_formula_matches_docs() {
        // docs §50.2 表
        let cases = [
            (0, 2),
            (20, 52),
            (50, 127),
            (100, 252),
        ];
        for (l, expected_y1) in cases {
            let y1 = ((l as i32 * 250 / 100) + 2).clamp(0, 255);
            assert_eq!(y1, expected_y1, "Lightness={}", l);
        }
    }

    #[test]
    fn rgb_chunk_apply_preserves_hue() {
        // 同加 delta 不改变色相
        let lc = LightnessCurve::new(50, true);
        let mut px = [2000u16, 4000, 6000];
        lc.apply_rgb_chunk(&mut px);
        // 差值应相同（保色相）
        let d_rg = px[1] as i32 - px[0] as i32;
        let d_gb = px[2] as i32 - px[1] as i32;
        // 原始差：4000-2000=2000, 6000-4000=2000
        // clip 后可能略有差异，但差应接近
        assert_eq!(d_rg, 2000);
        assert_eq!(d_gb, 2000);
    }

    #[test]
    fn rgb_chunk_clamps_at_max() {
        // 高亮像素 + 大 lightness 应 clamp
        let lc = LightnessCurve::new(100, true);
        let mut px = [MAX_14BIT; 3];
        lc.apply_rgb_chunk(&mut px);
        for &v in &px {
            assert!(v <= MAX_14BIT);
        }
    }
}
