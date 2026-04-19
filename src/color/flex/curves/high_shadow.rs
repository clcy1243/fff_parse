//! CHighShadowCurve — 3-zone shadow/highlight 曲线
//!
//! 源自 docs §28 的完整反编译。3 个输入区间各自独立处理：
//! - shadow (x < shadow_boundary): 3 种 mode（const/linear/zero）
//! - mid (shadow_boundary ≤ x < highlight_boundary): 走 sub_curve + 缩放
//! - highlight (x ≥ highlight_boundary): 3 种 mode（const/linear/max）
//!
//! ## 公式（§28.5）
//!
//! ```text
//! if x < shadow_boundary:
//!     mode 0 → shadow_out                                  # 常数
//!     mode 1 → shadow_out * x / shadow_boundary            # 0→shadow_out 线性
//!     mode 2 → 0                                           # 强制黑
//!
//! elif x < highlight_boundary:
//!     # 中间区：应用 inner_curve（完整曲线链）
//!     scaled_x = round((x - shadow_boundary) * mid_scale)  # 映射到 sub_curve 域
//!     y = sub_curve.compute_single(scaled_x)
//!     return round(shadow_out + y * mid_add_scale)
//!
//! else:  # x >= highlight_boundary
//!     mode 0 → highlight_out                                # 常数
//!     mode 1 → (x - hi_bnd) * (16383 - hi_out) / (16384 - hi_bnd) + hi_out
//!     mode 2 → 16383                                        # 强制白
//! ```
//!
//! ## 字段（§28.1）
//!
//! | field | 类型 | 含义 |
//! |-------|------|------|
//! | shadow_boundary | u16 | 输入阈值（来自 Shadow[ch] <<6） |
//! | highlight_boundary | u16 | 输入阈值（来自 Highlight[ch] <<6） |
//! | shadow_out | u16 | shadow 区输出（= EndPoints.shadow[ch] × 16383/255） |
//! | highlight_out | u16 | highlight 区输出 |
//! | mid_scale | f32 | `16383 / (hi_bnd - sh_bnd)` |
//! | mid_add_scale | f32 | `(hi_out - sh_out) / 16383` |
//! | shadow_mode | u8 | 0/1/2 |
//! | highlight_mode | u8 | 0/1/2 |
//! | sub_curve | Curve | 中间区的内层曲线（通常 inner_agg） |

use super::{Curve, MAX_14BIT};

/// Shadow 区 mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowMode {
    /// mode 0: 常数输出 `shadow_out`
    Const,
    /// mode 1: 线性 0 → shadow_out
    Linear,
    /// mode 2: 强制 0（硬截断黑）
    Zero,
}

/// Highlight 区 mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightMode {
    /// mode 0: 常数输出 `highlight_out`
    Const,
    /// mode 1: 线性 highlight_out → 16383
    Linear,
    /// mode 2: 强制 16383（硬截断白）
    Max,
}

impl ShadowMode {
    pub fn from_int(v: i32) -> Self {
        match v {
            0 => ShadowMode::Const,
            1 => ShadowMode::Linear,
            2 => ShadowMode::Zero,
            _ => ShadowMode::Const,
        }
    }
}

impl HighlightMode {
    pub fn from_int(v: i32) -> Self {
        match v {
            0 => HighlightMode::Const,
            1 => HighlightMode::Linear,
            2 => HighlightMode::Max,
            _ => HighlightMode::Const,
        }
    }
}

/// 3-zone shadow/highlight 曲线
pub struct HighShadowCurve {
    pub shadow_boundary: u16,
    pub highlight_boundary: u16,
    pub shadow_out: u16,
    pub highlight_out: u16,
    pub mid_scale: f32,
    pub mid_add_scale: f32,
    pub shadow_mode: ShadowMode,
    pub highlight_mode: HighlightMode,
    /// 中间区的内层曲线（通常是 inner_agg）
    pub sub_curve: Box<dyn Curve>,
}

impl HighShadowCurve {
    /// 用完整参数构造（通常内部调用，§28.2 set_params）
    pub fn new(
        shadow_boundary: u16,
        highlight_boundary: u16,
        shadow_out: u16,
        highlight_out: u16,
        shadow_mode: ShadowMode,
        highlight_mode: HighlightMode,
        sub_curve: Box<dyn Curve>,
    ) -> Self {
        // 参考 §28.2：运行时计算两个缩放因子
        let (mid_scale, mid_add_scale) = if highlight_boundary > shadow_boundary {
            let sh = shadow_boundary as f32;
            let hi = highlight_boundary as f32;
            let so = shadow_out as f32;
            let ho = highlight_out as f32;
            (
                MAX_14BIT as f32 / (hi - sh),
                (ho - so) / MAX_14BIT as f32,
            )
        } else {
            (1.0, 1.0) // identity fallback
        };

        Self {
            shadow_boundary,
            highlight_boundary,
            shadow_out,
            highlight_out,
            mid_scale,
            mid_add_scale,
            shadow_mode,
            highlight_mode,
            sub_curve,
        }
    }

    /// Identity-like 配置（无 histogram 调整，sub_curve 覆盖全域）
    ///
    /// 对应 §28.2 的"!ApplyHistogram" 分支：
    /// ```text
    /// shadow_out=0, highlight_out=16383
    /// shadow_boundary=0, highlight_boundary=16383
    /// mid_scale=1.0, mid_add_scale=1.0
    /// mode=0/0
    /// ```
    pub fn identity_like(sub_curve: Box<dyn Curve>) -> Self {
        Self {
            shadow_boundary: 0,
            highlight_boundary: MAX_14BIT,
            shadow_out: 0,
            highlight_out: MAX_14BIT,
            mid_scale: 1.0,
            mid_add_scale: 1.0,
            shadow_mode: ShadowMode::Const,
            highlight_mode: HighlightMode::Const,
            sub_curve,
        }
    }
}

impl Curve for HighShadowCurve {
    fn compute_single(&self, v: u16) -> u16 {
        let sh_bnd = self.shadow_boundary;
        let hi_bnd = self.highlight_boundary;

        if v < sh_bnd {
            // Shadow 区
            match self.shadow_mode {
                ShadowMode::Const => self.shadow_out,
                ShadowMode::Linear => {
                    // 0 → shadow_out 线性
                    if sh_bnd == 0 {
                        self.shadow_out
                    } else {
                        ((self.shadow_out as u32 * v as u32) / sh_bnd as u32) as u16
                    }
                }
                ShadowMode::Zero => 0,
            }
        } else if v < hi_bnd {
            // 中间区：走 sub_curve + 缩放
            let remapped = ((v - sh_bnd) as f32 * self.mid_scale).round() as i32;
            let remapped = remapped.clamp(0, MAX_14BIT as i32) as u16;
            let sub_val = self.sub_curve.compute_single(remapped);
            let out = (self.shadow_out as f32) + (sub_val as f32) * self.mid_add_scale;
            out.clamp(0.0, MAX_14BIT as f32).round() as u16
        } else {
            // Highlight 区
            match self.highlight_mode {
                HighlightMode::Const => self.highlight_out,
                HighlightMode::Linear => {
                    // hi_out → 16383 线性
                    let denom = 16384u32.saturating_sub(hi_bnd as u32);
                    if denom == 0 {
                        MAX_14BIT
                    } else {
                        let num = (v as u32 - hi_bnd as u32)
                            .saturating_mul(MAX_14BIT as u32 - self.highlight_out as u32);
                        let delta = (num / denom) as u16;
                        self.highlight_out.saturating_add(delta).min(MAX_14BIT)
                    }
                }
                HighlightMode::Max => MAX_14BIT,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// identity 子曲线（配合 identity-like 配置使测试简单）
    struct IdentityCurve;
    impl Curve for IdentityCurve {
        fn compute_single(&self, x: u16) -> u16 {
            x
        }
    }

    #[test]
    fn identity_like_passes_through() {
        // sub_curve = identity + 无 histogram 调整 → 整条应为 identity
        let hsc = HighShadowCurve::identity_like(Box::new(IdentityCurve));
        for i in (0..16384u16).step_by(173) {
            // mid 区走 scaled: (v - 0) * mid_scale=1 = v; sub(v)=v; 0 + v*1 = v
            // shadow/highlight 区：由于 shadow_boundary=0 和 highlight_boundary=MAX，
            // 实际永远进中间区（v < 16383 都在 mid 区）
            // v == MAX_14BIT 落入 highlight 区（>= 16383）
            let out = hsc.compute_single(i);
            let diff = (out as i32 - i as i32).abs();
            assert!(diff <= 1, "v={} out={} (diff {})", i, out, diff);
        }
    }

    #[test]
    fn shadow_zone_const_mode() {
        // shadow_bnd=4096, shadow_out=500, mode=Const
        let hsc = HighShadowCurve::new(
            4096,
            12288,
            500,
            MAX_14BIT,
            ShadowMode::Const,
            HighlightMode::Const,
            Box::new(IdentityCurve),
        );
        assert_eq!(hsc.compute_single(0), 500);
        assert_eq!(hsc.compute_single(2000), 500);
        assert_eq!(hsc.compute_single(4095), 500);
    }

    #[test]
    fn shadow_zone_linear_mode() {
        // shadow_bnd=4096, shadow_out=500, mode=Linear
        // 期待 v=0 → 0, v=shadow_bnd-1 ≈ 500
        let hsc = HighShadowCurve::new(
            4096,
            12288,
            500,
            MAX_14BIT,
            ShadowMode::Linear,
            HighlightMode::Const,
            Box::new(IdentityCurve),
        );
        assert_eq!(hsc.compute_single(0), 0);
        // v=2048 → 500 * 2048 / 4096 = 250
        assert_eq!(hsc.compute_single(2048), 250);
    }

    #[test]
    fn shadow_zone_zero_mode() {
        let hsc = HighShadowCurve::new(
            4096,
            12288,
            500,
            MAX_14BIT,
            ShadowMode::Zero,
            HighlightMode::Const,
            Box::new(IdentityCurve),
        );
        assert_eq!(hsc.compute_single(0), 0);
        assert_eq!(hsc.compute_single(2000), 0);
        assert_eq!(hsc.compute_single(4095), 0);
    }

    #[test]
    fn highlight_zone_const_mode() {
        let hsc = HighShadowCurve::new(
            4096,
            12288,
            0,
            15000,
            ShadowMode::Const,
            HighlightMode::Const,
            Box::new(IdentityCurve),
        );
        assert_eq!(hsc.compute_single(12288), 15000);
        assert_eq!(hsc.compute_single(14000), 15000);
        assert_eq!(hsc.compute_single(MAX_14BIT), 15000);
    }

    #[test]
    fn highlight_zone_max_mode() {
        let hsc = HighShadowCurve::new(
            4096,
            12288,
            0,
            15000,
            ShadowMode::Const,
            HighlightMode::Max,
            Box::new(IdentityCurve),
        );
        assert_eq!(hsc.compute_single(12288), MAX_14BIT);
        assert_eq!(hsc.compute_single(14000), MAX_14BIT);
    }

    #[test]
    fn mid_zone_applies_sub_curve() {
        // shadow_bnd=0, highlight_bnd=16383, shadow_out=0, hi_out=16383, identity sub
        // → mid 区应 = identity
        let hsc = HighShadowCurve::new(
            0,
            MAX_14BIT,
            0,
            MAX_14BIT,
            ShadowMode::Const,
            HighlightMode::Const,
            Box::new(IdentityCurve),
        );
        for i in (1..16383u16).step_by(173) {
            let out = hsc.compute_single(i);
            let diff = (out as i32 - i as i32).abs();
            assert!(diff <= 1, "mid identity v={} out={}", i, out);
        }
    }

    #[test]
    fn boundary_behavior() {
        let hsc = HighShadowCurve::new(
            4096,
            12288,
            100,
            15000,
            ShadowMode::Const,
            HighlightMode::Const,
            Box::new(IdentityCurve),
        );
        // shadow_bnd 临界：v=4095 → shadow 区 → 100；v=4096 → mid 区
        assert_eq!(hsc.compute_single(4095), 100);
        // hi_bnd 临界：v=12287 → mid；v=12288 → highlight 区 → 15000
        assert_eq!(hsc.compute_single(12288), 15000);
    }

    #[test]
    fn build_lut_doesnt_panic() {
        let hsc = HighShadowCurve::new(
            4096,
            12288,
            500,
            15000,
            ShadowMode::Linear,
            HighlightMode::Linear,
            Box::new(IdentityCurve),
        );
        let mut lut = Box::new([0u16; 16384]);
        hsc.build_lut(&mut *lut);
        for &v in lut.iter() {
            assert!(v <= MAX_14BIT);
        }
    }

    #[test]
    fn shadow_mode_from_int() {
        assert_eq!(ShadowMode::from_int(0), ShadowMode::Const);
        assert_eq!(ShadowMode::from_int(1), ShadowMode::Linear);
        assert_eq!(ShadowMode::from_int(2), ShadowMode::Zero);
        assert_eq!(ShadowMode::from_int(99), ShadowMode::Const); // fallback
    }

    #[test]
    fn highlight_mode_from_int() {
        assert_eq!(HighlightMode::from_int(0), HighlightMode::Const);
        assert_eq!(HighlightMode::from_int(1), HighlightMode::Linear);
        assert_eq!(HighlightMode::from_int(2), HighlightMode::Max);
    }
}
