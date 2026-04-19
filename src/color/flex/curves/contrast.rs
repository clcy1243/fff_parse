//! CContrastCurve — 对比度 + 亮度联合曲线
//!
//! 源自 docs §13 的完整反编译公式。产生一条 14-bit LUT，应用 "S-curve contrast"
//! + "形状化 brightness offset"，pivot 固定在 8192（14-bit 中点）。
//!
//! ## 公式（直接对应 C++ FUN_70267220）
//!
//! ```text
//! C = (contrast / 50.0) * 2.0             // -100..100 → -4..4
//! B = (brightness / 50.0) * 8192.0        // -100..100 → -16384..16384 (14-bit abs offset)
//!
//! 暗部 (v < 8192):
//!   b_shape = 1.0              (brightness <= 0)
//!          or v / 8192.0       (brightness > 0)
//!   out = v + B * b_shape + C * v * (v - 8192) / 16384
//!
//! 亮部 (v >= 8192):
//!   b_shape = (16383 - v) / 8192   (brightness <= 0)
//!          or 1.0                    (brightness > 0)
//!   out = v + B * b_shape + C * (v - 8192) * (16384 - v) / 16384
//!
//! out = clamp(out, 0, 16383)
//! ```
//!
//! ## Slider 归一化
//!
//! - Contrast/Brightness 在 XML 里是 int8，范围 -100..100
//! - 归一化除数是 **50**（不是 100），所以 slider=100 → C=4（系数 4）
//! - Brightness 最大偏移是 ±16384 全程（压到 0 或顶到 max）
//!
//! 参考：docs §13 "CContrastCurve::Apply @ 0x70267220"

use super::{Curve, MAX_14BIT, PIVOT_14BIT};

/// 对比度 + 亮度曲线
pub struct ContrastCurve {
    /// Contrast slider (-100..100)，来自 `ImageCorrection.contrast` @ offset 0x4fe
    pub contrast: i8,
    /// Brightness slider (-100..100)，来自 `ImageCorrection.brightness` @ offset 0x8d
    pub brightness: i8,
    /// 启用标志（典型 `ApplySliders`）
    pub enabled: bool,
}

impl ContrastCurve {
    /// 新建，默认启用
    pub fn new(contrast: i8, brightness: i8) -> Self {
        Self {
            contrast,
            brightness,
            enabled: true,
        }
    }

    /// 新建，带启用标志
    pub fn with_enabled(contrast: i8, brightness: i8, enabled: bool) -> Self {
        Self {
            contrast,
            brightness,
            enabled,
        }
    }

    /// 计算归一化后的 C/B 系数
    #[inline]
    fn coefficients(&self) -> (f64, f64) {
        let c = (self.contrast as f64 / 50.0) * 2.0;
        let b = (self.brightness as f64 / 50.0) * PIVOT_14BIT as f64;
        (c, b)
    }
}

impl Curve for ContrastCurve {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn compute_single(&self, v: u16) -> u16 {
        if !self.enabled {
            return v;
        }

        let (c, b) = self.coefficients();
        let v_f = v as f64;
        let pivot = PIVOT_14BIT as f64; // 8192
        let range = super::RANGE_14BIT; // 16384
        let max_val = MAX_14BIT as f64; // 16383

        let out = if v_f < pivot {
            // 暗部
            let b_shape = if self.brightness <= 0 {
                1.0
            } else {
                v_f / pivot
            };
            v_f + b * b_shape + c * v_f * (v_f - pivot) / range
        } else {
            // 亮部
            let b_shape = if self.brightness <= 0 {
                (max_val - v_f) / pivot
            } else {
                1.0
            };
            v_f + b * b_shape + c * (v_f - pivot) * (range - v_f) / range
        };

        // clamp to 14-bit
        out.clamp(0.0, max_val).round() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs §13 实测表的容忍值（±2 LSB，x87 FPU 与 f64 精度差异）
    const TOLERANCE: i32 = 2;

    fn assert_close(actual: u16, expected: i32, label: &str) {
        let diff = (actual as i32 - expected).abs();
        assert!(
            diff <= TOLERANCE,
            "{}: got {}, expected {} (diff {})",
            label,
            actual,
            expected,
            diff
        );
    }

    #[test]
    fn zero_sliders_is_identity() {
        let cc = ContrastCurve::new(0, 0);
        for i in (0..16384u16).step_by(173) {
            let out = cc.compute_single(i);
            assert_eq!(out, i, "v={} should be identity", i);
        }
    }

    #[test]
    fn pivot_fixed_at_8192() {
        // pivot 处始终输出自己（contrast 无效，brightness 才能移动它）
        for c in [-100i8, -50, 0, 50, 100] {
            let cc = ContrastCurve::new(c, 0);
            assert_eq!(cc.compute_single(PIVOT_14BIT), PIVOT_14BIT, "C={}", c);
        }
    }

    #[test]
    fn endpoints_preserved_for_contrast_only() {
        // contrast-only 不移动 0 和 max
        let cc = ContrastCurve::new(50, 0);
        assert_eq!(cc.compute_single(0), 0);
        assert_eq!(cc.compute_single(MAX_14BIT), MAX_14BIT);
    }

    #[test]
    fn contrast_20_matches_docs_table() {
        // docs §13 实测表 (C=20, B=0)
        let cc = ContrastCurve::new(20, 0);
        assert_close(cc.compute_single(1024), 666, "v=1024");
        assert_close(cc.compute_single(4096), 3277, "v=4096");
        assert_eq!(cc.compute_single(8192), 8192); // pivot
        assert_close(cc.compute_single(12288), 13107, "v=12288");
    }

    #[test]
    fn contrast_negative_inverts_s_curve() {
        // C < 0 应该把暗部抬高，亮部压低（inverse S-curve）
        let cc = ContrastCurve::new(-20, 0);
        assert!(cc.compute_single(4096) > 4096, "C<0 应抬暗部");
        assert!(cc.compute_single(12288) < 12288, "C<0 应压亮部");
    }

    #[test]
    fn brightness_positive_lifts_whole_curve() {
        // B > 0: 亮部全量加，暗部线性淡入到 0
        let cc = ContrastCurve::new(0, 50);
        // v=0 时 b_shape=0 → 不变
        assert_eq!(cc.compute_single(0), 0);
        // v=8192 mid → b_shape=1 → +B/1=+8192
        let mid = cc.compute_single(8192);
        assert!(mid > 8192);
    }

    #[test]
    fn brightness_negative_darkens() {
        let cc = ContrastCurve::new(0, -50);
        // v=max 时 b_shape=0 → 不变
        assert_eq!(cc.compute_single(MAX_14BIT), MAX_14BIT);
        // v=mid → 偏移为负
        let mid = cc.compute_single(8192);
        assert!(mid < 8192);
    }

    #[test]
    fn clamp_at_bounds() {
        // 极端参数下不应超出 14-bit 域
        let cc = ContrastCurve::new(100, 100);
        for i in (0..16384u16).step_by(97) {
            let out = cc.compute_single(i);
            assert!(out <= MAX_14BIT, "v={} out={} 超出 14-bit max", i, out);
        }
    }

    #[test]
    fn build_lut_matches_compute_single() {
        let cc = ContrastCurve::new(20, 0);
        let mut lut = Box::new([0u16; 16384]);
        cc.build_lut(&mut *lut);
        for i in (0..16384usize).step_by(97) {
            assert_eq!(lut[i], cc.compute_single(i as u16), "i={}", i);
        }
    }

    #[test]
    fn disabled_is_passthrough() {
        let cc = ContrastCurve::with_enabled(50, 50, false);
        for i in (0..16384u16).step_by(163) {
            assert_eq!(cc.compute_single(i), i);
        }
    }
}
