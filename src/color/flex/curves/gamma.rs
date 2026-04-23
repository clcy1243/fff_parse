//! Gamma 曲线族
//!
//! - `GammaCurve` — 正片 gamma（docs §13 CGammaCurve::Apply @ 0x70266830）
//! - `GammaNegCurve` — 负片 gamma（docs §18.7 CGammaNegCurve @ 0x702664e0 勘误版）
//!
//! ## GammaCurve 公式（§13）
//!
//! ```text
//! if G >= 2.0:
//!     exponent = 1.0 / (G - 1.0)
//! else:
//!     exponent = 1.0 / (1.0 - (2.0 - G) * 0.8)    // G < 2 的平滑路径，常数 0.8
//!
//! LUT[i] = round( pow(i / 16383.0, exponent) * 16383.0 )
//! ```
//!
//! **G=2.0 是 identity**。G>2 亮化，G<2 暗化。G 下界 ~0.75（低于 = 1-(2-G)*0.8 奇点）。
//!
//! ## GammaNegCurve 公式（§18.7）
//!
//! ```text
//! if !enabled || parent.FilmType == 0:
//!     return identity
//!
//! gamma = XML["NegVarGamma"] (若在 (0.099, 10.1) 区间) else 0.2
//! input_scale = 17700.0 (EnhancedShadow) or 16383.0
//! stretch = XML["StretchNegGamma"] (若在 (1.01, 1.11) 区间) → input_scale *= stretch
//!
//! for i in 0..16384:
//!     v = i / input_scale
//!     LUT[i] = round( pow(1 - v*v, 1/gamma) * 16383.0 )
//! ```
//!
//! **关键**：有 `1 - v²` 反转项（C-41 负片→正像），然后 pow。默认 gamma=0.2 → exponent=5。

use super::{Curve, MAX_14BIT};

/// 正片 gamma 曲线
///
/// 公式：分段 `pow(v / 16383, f(G))`，两种指数计算方式。
pub struct GammaCurve {
    /// Gamma slider 值，来自 `ImageCorrection.gamma` @ offset 0x52c (f32)。
    /// 典型范围 1.0..3.0，默认 2.0（= identity）。
    pub gamma: f32,
    /// 启用标志（典型 `ApplySliders`）
    pub enabled: bool,
}

impl GammaCurve {
    /// 新建，默认启用
    pub fn new(gamma: f32) -> Self {
        Self {
            gamma,
            enabled: true,
        }
    }

    /// 新建，带启用标志
    pub fn with_enabled(gamma: f32, enabled: bool) -> Self {
        Self { gamma, enabled }
    }

    /// 计算 pow 的指数（按 §13 分段）
    #[inline]
    fn exponent(&self) -> f64 {
        let g = self.gamma as f64;
        if g >= 2.0 {
            1.0 / (g - 1.0)
        } else if g >= 0.76 {
            // G ∈ [0.76, 2)：§13 smooth path
            let denom = 1.0 - (2.0 - g) * 0.8;
            1.0 / denom
        } else {
            // G < 0.76：原公式奇点（FlexColor UI 不允许），fallback 用标准 gamma
            // 定义 `exp = 1/G`（经典 gamma 反向），避免 LUT 全零。非 bit-accurate。
            1.0 / g.max(0.01)
        }
    }
}

impl Curve for GammaCurve {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn compute_single(&self, v: u16) -> u16 {
        if !self.enabled {
            return v;
        }
        let exp = self.exponent();
        let v_f = v as f64 / MAX_14BIT as f64;
        let out = v_f.powf(exp) * MAX_14BIT as f64;
        out.clamp(0.0, MAX_14BIT as f64).round() as u16
    }
}

/// 负片 gamma 曲线（C-41 反转）
///
/// 公式 `pow(1 - v², 1/γ) × 16383`，含 `1 - v²` 反转项。
/// 见 docs §18.7（勘误版）。
pub struct GammaNegCurve {
    /// gamma 值（来自 XML `NegVarGamma`），默认 0.2（即 exponent = 5）
    pub gamma: f64,
    /// 输入归一化分母（16383 或 17700 如 EnhancedShadow）
    pub input_scale: f64,
    /// 输出乘子（通常固定 16383）
    pub output_scale: f64,
    /// 启用标志（= `parent.FilmType != 0`）
    pub enabled: bool,
}

impl GammaNegCurve {
    /// 用 FlexColor 的默认参数构造（gamma=0.2, input_scale=16383, output_scale=16383）
    pub fn default_enabled() -> Self {
        Self {
            gamma: 0.2,
            input_scale: 16383.0,
            output_scale: 16383.0,
            enabled: true,
        }
    }

    /// 从 ImageCorrection 字段构造
    ///
    /// - `film_type`: `parent+0x51c`（0 = 禁用）
    /// - `enhanced_shadow`: `parent+0x518`（切换 input_scale 16383 / 17700）
    /// - `neg_var_gamma`: 来自 XML（None = 使用默认 0.2）
    /// - `stretch_neg_gamma`: 来自 XML（None 或 非法区间 = 不应用）
    pub fn from_params(
        film_type: u32,
        enhanced_shadow: bool,
        neg_var_gamma: Option<f64>,
        stretch_neg_gamma: Option<f64>,
    ) -> Self {
        let enabled = film_type != 0;
        let gamma = match neg_var_gamma {
            Some(g) if g > 0.099 && g < 10.1 => g,
            _ => 0.2, // _DAT_707338d0
        };
        let mut input_scale = if enhanced_shadow {
            17700.0 // _DAT_70735130
        } else {
            16383.0 // _DAT_70733988
        };
        if let Some(s) = stretch_neg_gamma {
            if s > 1.01 && s < 1.11 {
                input_scale *= s;
            }
        }
        Self {
            gamma,
            input_scale,
            output_scale: 16383.0,
            enabled,
        }
    }

    /// 禁用状态（FilmType==0），LUT=identity
    pub fn disabled() -> Self {
        Self {
            gamma: 0.2,
            input_scale: 16383.0,
            output_scale: 16383.0,
            enabled: false,
        }
    }
}

impl Curve for GammaNegCurve {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn compute_single(&self, i: u16) -> u16 {
        if !self.enabled {
            return i;
        }
        let v = i as f64 / self.input_scale;
        let one_minus_v2 = 1.0 - v * v;
        // 保护：若 1 - v² 为负（i 超过 input_scale），clamp 为 0
        let base = one_minus_v2.max(0.0);
        let out = base.powf(1.0 / self.gamma) * self.output_scale;
        out.clamp(0.0, MAX_14BIT as f64).round() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== GammaCurve 测试 =====

    #[test]
    fn gamma_2_is_identity() {
        let gc = GammaCurve::new(2.0);
        for i in (0..16384u16).step_by(173) {
            assert_eq!(gc.compute_single(i), i, "v={}", i);
        }
    }

    #[test]
    fn gamma_endpoints_preserved() {
        for g in [1.5f32, 2.0, 2.5, 3.0] {
            let gc = GammaCurve::new(g);
            assert_eq!(gc.compute_single(0), 0, "G={} at v=0", g);
            assert_eq!(gc.compute_single(MAX_14BIT), MAX_14BIT, "G={} at v=max", g);
        }
    }

    #[test]
    fn gamma_gt2_brightens() {
        // G=2.5 → exp = 1/(2.5-1) = 0.667 → pow(0.5, 0.667) ≈ 0.63 > 0.5
        let gc = GammaCurve::new(2.5);
        let mid = gc.compute_single(8192);
        assert!(mid > 8192, "G=2.5 应使 mid 变亮，got {}", mid);
    }

    #[test]
    fn gamma_lt2_darkens() {
        // G=1.5 → exp = 1/(1 - (0.5)*0.8) = 1/0.6 = 1.667 → pow(0.5, 1.667) ≈ 0.315 < 0.5
        let gc = GammaCurve::new(1.5);
        let mid = gc.compute_single(8192);
        assert!(mid < 8192, "G=1.5 应使 mid 变暗，got {}", mid);
    }

    #[test]
    fn gamma_exponent_matches_docs_table() {
        // docs §13 exponent 对照
        let tolerance = 0.01;
        let cases = [
            (2.5f32, 0.667f64),
            (2.0, 1.0),
            (1.5, 1.667), // 1/(1 - 0.5*0.8) = 1/0.6
            (1.0, 5.0),   // 1/(1 - 1.0*0.8) = 1/0.2
        ];
        for (g, expected_exp) in cases {
            let gc = GammaCurve::new(g);
            let got = gc.exponent();
            assert!(
                (got - expected_exp).abs() < tolerance,
                "G={} exp={}, expected {}",
                g,
                got,
                expected_exp
            );
        }
    }

    #[test]
    fn gamma_below_0_75_clamps_exponent() {
        // G=0.5 会使 1 - 1.5*0.8 = -0.2 出现奇点，should clamp
        let gc = GammaCurve::new(0.5);
        let exp = gc.exponent();
        assert!(exp > 0.0, "应有保护，exponent={}", exp);
        // 不应 panic
        let _ = gc.compute_single(8192);
    }

    #[test]
    fn gamma_disabled_is_passthrough() {
        let gc = GammaCurve::with_enabled(2.5, false);
        for i in (0..16384u16).step_by(173) {
            assert_eq!(gc.compute_single(i), i);
        }
    }

    // ===== GammaNegCurve 测试 =====

    #[test]
    fn neg_disabled_is_identity() {
        let gnc = GammaNegCurve::disabled();
        for i in (0..16384u16).step_by(173) {
            assert_eq!(gnc.compute_single(i), i, "v={}", i);
        }
    }

    #[test]
    fn neg_default_inverts_endpoints() {
        let gnc = GammaNegCurve::default_enabled();
        // v=0 → 1 - 0 = 1 → pow(1, 5) * 16383 = 16383 （最暗输入 → 最亮输出）
        assert_eq!(gnc.compute_single(0), MAX_14BIT);
        // v=max → 1 - 1 = 0 → pow(0, 5) * 16383 = 0
        assert_eq!(gnc.compute_single(MAX_14BIT), 0);
    }

    #[test]
    fn neg_default_matches_docs_table() {
        // docs §18.7 实测表（gamma=0.2 默认）
        // 容差 ±20 LSB —— docs 表是手算近似，f64 精确计算有小差异
        let gnc = GammaNegCurve::default_enabled();
        let tol = 20i32;
        let cases = [
            (0u16, 16383u16),
            (4096, 11860),
            (8192, 3885),
            (12288, 264),
            (MAX_14BIT, 0),
        ];
        for (input, expected) in cases {
            let out = gnc.compute_single(input);
            let diff = (out as i32 - expected as i32).abs();
            assert!(
                diff <= tol,
                "v={} got {}, expected {} (diff {})",
                input,
                out,
                expected,
                diff
            );
        }
    }

    #[test]
    fn neg_exact_values_from_f64() {
        // f64 精确计算的期望值（用 docs 公式直接算出，作为回归基线）
        let gnc = GammaNegCurve::default_enabled();
        // 严格相等
        assert_eq!(gnc.compute_single(0), 16383);
        assert_eq!(gnc.compute_single(MAX_14BIT), 0);

        // 用与实现相同的 f64 公式算期望（自洽测试）
        let expected = |i: u16| -> u16 {
            let v = i as f64 / 16383.0;
            let r = (1.0 - v * v).max(0.0).powf(5.0) * 16383.0;
            r.clamp(0.0, 16383.0).round() as u16
        };
        for i in (0..16384u16).step_by(1024) {
            assert_eq!(gnc.compute_single(i), expected(i), "v={}", i);
        }
    }

    #[test]
    fn neg_from_params_disabled_when_film_type_zero() {
        let gnc = GammaNegCurve::from_params(0, false, None, None);
        assert!(!gnc.enabled());
        // identity
        for i in (0..16384u16).step_by(973) {
            assert_eq!(gnc.compute_single(i), i);
        }
    }

    #[test]
    fn neg_from_params_enabled_when_film_type_nonzero() {
        let gnc = GammaNegCurve::from_params(1, false, None, None);
        assert!(gnc.enabled());
        assert_eq!(gnc.gamma, 0.2);
        assert_eq!(gnc.input_scale, 16383.0);
    }

    #[test]
    fn neg_from_params_enhanced_shadow_changes_scale() {
        let gnc = GammaNegCurve::from_params(1, true, None, None);
        assert_eq!(gnc.input_scale, 17700.0);
        // v_max = 16383/17700 ≈ 0.9257
        // 1 - 0.857 = 0.143
        // 0.143^5 ≈ 5.97e-5
        // × 16383 ≈ 1
        let out = gnc.compute_single(MAX_14BIT);
        assert!(out <= 5, "enhanced shadow v=max 应 ≈ 1, got {}", out);
        // v=0 仍然是 max
        assert_eq!(gnc.compute_single(0), MAX_14BIT);
    }

    #[test]
    fn neg_from_params_stretch_applied_in_valid_range() {
        let g1 = GammaNegCurve::from_params(1, false, None, None);
        let g2 = GammaNegCurve::from_params(1, false, None, Some(1.05));
        // stretch 1.05 在区间 (1.01, 1.11) 应生效
        assert!(g2.input_scale > g1.input_scale);
        assert!((g2.input_scale - 16383.0 * 1.05).abs() < 1e-6);
    }

    #[test]
    fn neg_from_params_stretch_rejected_out_of_range() {
        let g_oob_lo = GammaNegCurve::from_params(1, false, None, Some(1.0));
        let g_oob_hi = GammaNegCurve::from_params(1, false, None, Some(1.12));
        assert_eq!(g_oob_lo.input_scale, 16383.0);
        assert_eq!(g_oob_hi.input_scale, 16383.0);
    }

    #[test]
    fn neg_from_params_gamma_rejected_out_of_range() {
        // NegVarGamma 有效区间 (0.099, 10.1)
        let g_oob_lo = GammaNegCurve::from_params(1, false, Some(0.05), None);
        let g_oob_hi = GammaNegCurve::from_params(1, false, Some(10.5), None);
        assert_eq!(g_oob_lo.gamma, 0.2, "低越界应 fallback 默认");
        assert_eq!(g_oob_hi.gamma, 0.2, "高越界应 fallback 默认");
    }

    #[test]
    fn neg_from_params_gamma_accepted_in_range() {
        let g = GammaNegCurve::from_params(1, false, Some(0.5), None);
        assert_eq!(g.gamma, 0.5);
    }

    #[test]
    fn neg_monotonically_decreasing() {
        // 负片曲线应严格递减
        let gnc = GammaNegCurve::default_enabled();
        let mut prev = u16::MAX as i32 + 1;
        for i in (0..16384u16).step_by(11) {
            let out = gnc.compute_single(i) as i32;
            assert!(out <= prev, "非单调: v={} out={} prev={}", i, out, prev);
            prev = out;
        }
    }

    #[test]
    fn build_lut_consistency() {
        let gnc = GammaNegCurve::default_enabled();
        let mut lut = Box::new([0u16; 16384]);
        gnc.build_lut(&mut *lut);
        for i in (0..16384usize).step_by(97) {
            assert_eq!(lut[i], gnc.compute_single(i as u16));
        }
    }
}
