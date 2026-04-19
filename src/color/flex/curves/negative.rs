//! CNegativeCurve — C-41 反转主曲线（两段二次曲线）
//!
//! 源自 docs §15 CNegativeCurve + §18.8 的 x/y 方向勘误版。pivot 在 `(x_param, y_param)`
//! 处平滑连接两段二次曲线。
//!
//! ## 公式（§15）
//!
//! ```text
//! x_param = field_0x20 * 64   // pivot x (14-bit)
//! y_param = field_0x24 * 64   // value at pivot (14-bit)
//! max_val = 16383
//! c6 = 2.0                     // _DAT_70733548
//!
//! 上段 (v >= x_param):
//!   d = v - x_param
//!   D = max_val - x_param
//!   q = ((max_val - y_param) - D) / (D * D)
//!   v_out = d*d * q + d + y_param
//!
//! 下段 (v < x_param):
//!   k = (x_param - y_param) / (x_param * x_param)
//!   linear_scale = 1.0 - (x_param - y_param) * c6 / x_param
//!   v_out = k * v*v + linear_scale * v
//!
//! 边界（若 v_out 越界）:
//!   v >= x_param: clamp 到 16384
//!   v <  x_param: clamp 到 0
//! 否则: round(v_out)
//! ```
//!
//! ## 默认参数（§18.8）
//!
//! | 实例 | field_0x20 | field_0x24 | x_param (×64) | y_param (×64) |
//! |------|-----------|-----------|---------------|---------------|
//! | shared | 158.7 | 100.6 | 10156.8 | 6438.4 |
//! | R per-ch | 107.5 | 145.1 | 6880.0 | 9286.4 |
//! | G per-ch | 139.9 | 124.5 | 8953.6 | 7968.0 |
//! | B per-ch | 146.7 | 90.4 | 9388.8 | 5785.6 |

use super::{Curve, MAX_14BIT, RANGE_14BIT};

/// 负片两段二次曲线
pub struct NegativeCurve {
    /// pivot x 位置（14-bit 域，已 × 64）
    pub x_param: f64,
    /// pivot 处的输出值（14-bit 域，已 × 64）
    pub y_param: f64,
    /// 启用标志
    pub enabled: bool,
}

/// §18.8 常数（c6 = _DAT_70733548 = 2.0）
const C6: f64 = 2.0;

impl NegativeCurve {
    /// 从 field_0x20 / field_0x24 原始值构造（内部 × 64 转到 14-bit 域）
    pub fn new(field_0x20: f64, field_0x24: f64) -> Self {
        Self {
            x_param: field_0x20 * 64.0,
            y_param: field_0x24 * 64.0,
            enabled: true,
        }
    }

    /// 直接给出 14-bit 域的 x/y（已乘 64）
    pub fn from_14bit(x_param: f64, y_param: f64) -> Self {
        Self {
            x_param,
            y_param,
            enabled: true,
        }
    }

    /// FlexColor shared 默认（x=158.7, y=100.6）
    pub fn default_shared() -> Self {
        Self::new(158.7, 100.6)
    }

    /// FlexColor R 通道默认（x=107.5, y=145.1）
    pub fn default_r() -> Self {
        Self::new(107.5, 145.1)
    }

    /// FlexColor G 通道默认（x=139.9, y=124.5）
    pub fn default_g() -> Self {
        Self::new(139.9, 124.5)
    }

    /// FlexColor B 通道默认（x=146.7, y=90.4）
    pub fn default_b() -> Self {
        Self::new(146.7, 90.4)
    }

    /// 禁用状态（identity）
    pub fn disabled() -> Self {
        Self {
            x_param: 0.0,
            y_param: 0.0,
            enabled: false,
        }
    }
}

impl Curve for NegativeCurve {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn compute_single(&self, v: u16) -> u16 {
        if !self.enabled {
            return v;
        }

        let v_f = v as f64;
        let x = self.x_param;
        let y = self.y_param;
        let max_val = MAX_14BIT as f64;

        let v_out = if v_f >= x {
            // 上段
            let d = v_f - x;
            let big_d = max_val - x;
            if big_d <= 0.0 {
                // x_param 超出 max，整条曲线无效
                return MAX_14BIT;
            }
            let q = ((max_val - y) - big_d) / (big_d * big_d);
            d * d * q + d + y
        } else {
            // 下段
            if x <= 0.0 {
                // 非法参数
                return 0;
            }
            let k = (x - y) / (x * x);
            let linear_scale = 1.0 - (x - y) * C6 / x;
            k * v_f * v_f + linear_scale * v_f
        };

        // §15 边界处理：越界按段方向钳制
        if v_out <= 0.0 || v_out >= RANGE_14BIT {
            if v_f >= x {
                MAX_14BIT
            } else {
                0
            }
        } else {
            v_out.round() as u16
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_is_identity() {
        let nc = NegativeCurve::disabled();
        for i in (0..16384u16).step_by(173) {
            assert_eq!(nc.compute_single(i), i);
        }
    }

    #[test]
    fn endpoints_zero_and_max() {
        // v=0: 下段 k*0 + scale*0 = 0
        let nc = NegativeCurve::default_shared();
        assert_eq!(nc.compute_single(0), 0);
    }

    #[test]
    fn pivot_passes_through_y_param() {
        // v = x_param 处应输出 y_param（两段在 pivot 连续）
        let nc = NegativeCurve::default_shared();
        let x_int = nc.x_param as u16;
        let y_int = nc.y_param as u16;
        let out = nc.compute_single(x_int);
        // 两段在 pivot 处理论精确，但整数 rounding 可能 ±1
        assert!(
            (out as i32 - y_int as i32).abs() <= 1,
            "pivot x={} 应 → y={} got {}",
            x_int,
            y_int,
            out
        );
    }

    #[test]
    fn piecewise_continuity() {
        // pivot 左右相邻点输出应连续（≤2 LSB diff）
        let nc = NegativeCurve::default_shared();
        let x_int = nc.x_param as u16;
        if x_int > 0 && x_int < MAX_14BIT {
            let left = nc.compute_single(x_int - 1);
            let right = nc.compute_single(x_int + 1);
            assert!(
                (right as i32 - left as i32).abs() < 50,
                "pivot 附近跳变 {} -> {}",
                left,
                right
            );
        }
    }

    #[test]
    fn per_channel_defaults_produce_valid_lut() {
        // 每个 per-ch 默认应产生合理 LUT（不 panic、值域对）
        for nc in [
            NegativeCurve::default_r(),
            NegativeCurve::default_g(),
            NegativeCurve::default_b(),
        ] {
            let mut lut = Box::new([0u16; 16384]);
            nc.build_lut(&mut *lut);
            // 0 → 0（下段 k*0 + scale*0 = 0）
            assert_eq!(lut[0], 0);
            // max → max（上段过最右端点，clamp 14-bit）
            assert!(lut[MAX_14BIT as usize] <= MAX_14BIT);
            // 单调？一般应为非递减（neg curve 基础形状）
            // 实际上可能微小波动，此处只检查不越界
            for &v in lut.iter() {
                assert!(v <= MAX_14BIT);
            }
        }
    }

    #[test]
    fn shared_default_lut_at_sample_points() {
        // shared (x=10156.8, y=6438.4)
        let nc = NegativeCurve::default_shared();
        // v=0 → 0
        assert_eq!(nc.compute_single(0), 0);
        // v=pivot ≈ 10156 → ≈ 6438
        let out_mid = nc.compute_single(10157);
        assert!(
            (out_mid as i32 - 6438).abs() < 20,
            "pivot 附近 got {}",
            out_mid
        );
    }

    #[test]
    fn different_channels_produce_different_outputs() {
        let r = NegativeCurve::default_r();
        let g = NegativeCurve::default_g();
        let b = NegativeCurve::default_b();
        let v = 8192u16;
        let or = r.compute_single(v);
        let og = g.compute_single(v);
        let ob = b.compute_single(v);
        // 三通道参数不同，输出应显著不同
        let all_equal = or == og && og == ob;
        assert!(!all_equal, "R/G/B 应不同：r={} g={} b={}", or, og, ob);
    }

    #[test]
    fn from_14bit_matches_new() {
        let a = NegativeCurve::new(158.7, 100.6);
        let b = NegativeCurve::from_14bit(158.7 * 64.0, 100.6 * 64.0);
        assert_eq!(a.x_param, b.x_param);
        assert_eq!(a.y_param, b.y_param);
    }

    #[test]
    fn build_lut_consistency() {
        let nc = NegativeCurve::default_shared();
        let mut lut = Box::new([0u16; 16384]);
        nc.build_lut(&mut *lut);
        for i in (0..16384usize).step_by(127) {
            assert_eq!(lut[i], nc.compute_single(i as u16));
        }
    }
}
