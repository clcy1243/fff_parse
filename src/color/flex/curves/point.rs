//! CPointCurve — 用户 Gradation 曲线
//!
//! 源自 docs §16.2 + §16.7 + §16.8 + §16.9。用户画的曲线，由最多 10 个
//! CurvePoint (x, y, dy) 控制，预计算为 14-bit LUT 存于 `this+0x14`。
//!
//! ## XML 与内存表示
//!
//! XML `<Points>` 中 x/y/dy 都是 byte (0..255)。FlexColor 内部 × 64 升到 14-bit 域
//! （<<6；0..16320，注意不是 0..16383 而是 0..16320）。
//!
//! ## LUT 构建（§16.8）
//!
//! ```text
//! # 对每个 256 * step 的参数 t：
//! for t in 0..=255 (步进 step_size):
//!     (x_t, y_t) = B-spline 基函数 weighted sum over control points
//!     # (x_t, y_t) 是 14-bit 域的坐标（已 × 64）
//!
//! # 相邻 t-sample 之间，在 LUT 上用**线性填充**区间 [prev_x, x_t]：
//!     for k in prev_x..x_t:
//!         lut[k] = lerp(prev_y, y_t, (k - prev_x) / (x_t - prev_x))
//! ```
//!
//! ## MVP 简化
//!
//! - 2 点（identity 常见情形）→ 纯线性填充，跳过 spline
//! - 多点 → B-spline 采样（见 `super::super::bspline`）+ 线性填充
//!
//! TODO: 多点 case 需与 FlexColor 输出对齐验证（knot vector / order / step 精度）

use super::super::bspline;
use super::{Curve, MAX_14BIT};

/// XML `<Points>` 中的控制点
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurvePoint {
    /// x 坐标，0..255（XML byte）
    pub x: u8,
    /// y 坐标，0..255（XML byte）
    pub y: u8,
    /// dy 标记（knot weight / endpoint marker，详情待考）
    pub dy: u8,
}

impl CurvePoint {
    pub fn new(x: u8, y: u8, dy: u8) -> Self {
        Self { x, y, dy }
    }

    /// 8-bit x → 14-bit 域（× 64）
    pub fn x_14bit(&self) -> u32 {
        (self.x as u32) << 6
    }

    /// 8-bit y → 14-bit 域（× 64）
    pub fn y_14bit(&self) -> u32 {
        (self.y as u32) << 6
    }
}

/// CPointCurve — 用户曲线 + 预计算 LUT
pub struct PointCurve {
    /// 控制点列表（最多 10 个，§16.2）
    pub points: Vec<CurvePoint>,
    /// 预计算 14-bit LUT (16384 entries)，对应 `this+0x14`
    pub lut: Box<[u16; 16384]>,
    /// 启用标志
    pub enabled: bool,
}

impl PointCurve {
    /// 默认 identity 曲线（2 个端点 (0,0) 和 (255,255)）
    pub fn identity() -> Self {
        Self::from_points(vec![CurvePoint::new(0, 0, 1), CurvePoint::new(255, 255, 1)])
    }

    /// 从控制点列表构造（会立即 build LUT）
    pub fn from_points(points: Vec<CurvePoint>) -> Self {
        let mut lut = Box::new([0u16; 16384]);
        build_lut(&points, &mut *lut);
        Self {
            points,
            lut,
            enabled: true,
        }
    }

    /// 从 XML `<Points>` array 构造
    /// 格式：`Vec<(x, y, dy)>`
    pub fn from_xml_points(xml_points: &[(i64, i64, i64)]) -> Self {
        let points: Vec<CurvePoint> = xml_points
            .iter()
            .map(|&(x, y, dy)| {
                CurvePoint::new(
                    x.clamp(0, 255) as u8,
                    y.clamp(0, 255) as u8,
                    dy.clamp(0, 255) as u8,
                )
            })
            .collect();
        Self::from_points(points)
    }

    /// 禁用状态（identity LUT）
    pub fn disabled() -> Self {
        let mut s = Self::identity();
        s.enabled = false;
        s
    }

    /// 检测当前点集是否为 identity（2 点 (0,0)-(255,255)）
    pub fn is_identity(&self) -> bool {
        self.points.len() == 2
            && self.points[0].x == 0
            && self.points[0].y == 0
            && self.points[1].x == 255
            && self.points[1].y == 255
    }
}

impl Curve for PointCurve {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn compute_single(&self, x: u16) -> u16 {
        if !self.enabled {
            return x;
        }
        let idx = (x as usize).min(16383);
        self.lut[idx]
    }

    fn build_lut(&self, lut: &mut [u16; 16384]) {
        if self.enabled {
            lut.copy_from_slice(&*self.lut);
        } else {
            for (i, v) in lut.iter_mut().enumerate() {
                *v = i as u16;
            }
        }
    }
}

/// 从控制点构建 14-bit LUT
fn build_lut(points: &[CurvePoint], lut: &mut [u16; 16384]) {
    if points.is_empty() {
        // 空曲线 → identity
        for (i, v) in lut.iter_mut().enumerate() {
            *v = i as u16;
        }
        return;
    }

    if points.len() == 1 {
        // 单点：常数 LUT
        let y = points[0].y_14bit() as u16;
        for v in lut.iter_mut() {
            *v = y.min(MAX_14BIT);
        }
        return;
    }

    if points.len() == 2 {
        // 两点：纯线性填充（最常见 identity case 走这里）
        fill_linear_between(lut, &points[0], &points[1]);
        // LUT 末尾（x 超出最后一点）用最后一点的 y
        let last_x_14 = points[1].x_14bit() as usize;
        let last_y_14 = points[1].y_14bit().min(MAX_14BIT as u32) as u16;
        for v in lut.iter_mut().skip(last_x_14.max(16384)).take(0) {
            *v = last_y_14;
        }
        // 确保 LUT 完全覆盖（[last_x_14, 16384)）
        for v in lut.iter_mut().skip(last_x_14 + 1) {
            *v = last_y_14;
        }
        return;
    }

    // 多点：B-spline 采样 + 线性填充
    // k=4 (cubic), uniform clamped knots, 256 sample steps（推测 FlexColor 用此参数）
    build_lut_multipoint(points, lut);
}

/// 2 点之间纯线性填充
fn fill_linear_between(lut: &mut [u16; 16384], p0: &CurvePoint, p1: &CurvePoint) {
    let x0 = p0.x_14bit() as i64;
    let y0 = p0.y_14bit() as i64;
    let x1 = p1.x_14bit() as i64;
    let y1 = p1.y_14bit() as i64;

    // [0, x0) 全部 = y0
    let x0_idx = x0.max(0).min(16384) as usize;
    for v in lut[..x0_idx].iter_mut() {
        *v = y0.min(MAX_14BIT as i64) as u16;
    }

    // [x0, x1] 线性插值
    let dx = x1 - x0;
    if dx <= 0 {
        // 退化
        for v in lut.iter_mut() {
            *v = y0.min(MAX_14BIT as i64) as u16;
        }
        return;
    }
    let x1_idx = x1.max(0).min(16384) as usize;
    for i in x0_idx..=x1_idx.min(16383) {
        let t = (i as i64 - x0) as f64 / dx as f64;
        let y = y0 as f64 + t * (y1 - y0) as f64;
        lut[i] = y.clamp(0.0, MAX_14BIT as f64).round() as u16;
    }

    // (x1, 16384) 全部 = y1
    let y1_val = y1.min(MAX_14BIT as i64) as u16;
    for v in lut.iter_mut().skip(x1_idx + 1) {
        *v = y1_val;
    }
}

/// 多点 B-spline 采样 + 线性填充（TODO: 验证 FlexColor 精确参数）
fn build_lut_multipoint(points: &[CurvePoint], lut: &mut [u16; 16384]) {
    let n = points.len();
    let k = 4.min(n); // cubic 或退化
    let knots = bspline::uniform_clamped_knots(n, k);

    // 控制点 x/y 各自的 14-bit 值
    let xs: Vec<f64> = points.iter().map(|p| p.x_14bit() as f64).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.y_14bit() as f64).collect();

    // 采样步数（256 × 4 = 1024，足够精细）
    const STEPS: usize = 1024;
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(STEPS + 1);
    for s in 0..=STEPS {
        let t = s as f64 / STEPS as f64;
        let x_t = bspline::evaluate(&xs, &knots, k, t);
        let y_t = bspline::evaluate(&ys, &knots, k, t);
        samples.push((x_t, y_t));
    }

    // 按 x 排序（B-spline 可能产生非单调 x，尽管通常单调）
    samples.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // [0, first_x) 前填 first_y
    let first_y = samples[0].1.clamp(0.0, MAX_14BIT as f64).round() as u16;
    let first_x_idx = (samples[0].0 as i64).max(0).min(16384) as usize;
    for v in lut[..first_x_idx].iter_mut() {
        *v = first_y;
    }

    // 相邻 sample 之间线性填充
    for w in samples.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        let x0_i = (x0 as i64).max(0).min(16383) as usize;
        let x1_i = (x1 as i64).max(0).min(16383) as usize;
        if x1_i <= x0_i {
            continue;
        }
        let dx = (x1 - x0).max(1e-9);
        for i in x0_i..=x1_i {
            let t = (i as f64 - x0) / dx;
            let y = y0 + t * (y1 - y0);
            lut[i] = y.clamp(0.0, MAX_14BIT as f64).round() as u16;
        }
    }

    // (last_x, 16384) 后填 last_y
    let last_y = samples.last().unwrap().1.clamp(0.0, MAX_14BIT as f64).round() as u16;
    let last_x_idx = (samples.last().unwrap().0 as i64).max(0).min(16384) as usize;
    for v in lut.iter_mut().skip(last_x_idx + 1) {
        *v = last_y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_curve_lut_is_linear() {
        let pc = PointCurve::identity();
        assert!(pc.is_identity());
        // LUT 应接近 identity（x/y=0..255, 14-bit 映射 x*64..y*64）
        // 所以 lut[i] ≈ i for i in 0..16320
        assert_eq!(pc.compute_single(0), 0);
        // lut[16320] ≈ 16320（xml y=255 → 14bit 16320）
        let at_end = pc.compute_single(16320);
        assert!(
            (at_end as i32 - 16320).abs() <= 1,
            "at 16320 got {}",
            at_end
        );
        // [16320, 16383] 应 flat at 16320（超出最后控制点）
        assert_eq!(pc.compute_single(16383), 16320);
    }

    #[test]
    fn identity_curve_monotonic() {
        let pc = PointCurve::identity();
        let mut prev = 0i32;
        for i in (0..16384u16).step_by(97) {
            let v = pc.compute_single(i) as i32;
            assert!(v >= prev, "non-monotonic at {}", i);
            prev = v;
        }
    }

    #[test]
    fn disabled_is_passthrough() {
        let pc = PointCurve::disabled();
        for i in (0..16384u16).step_by(173) {
            assert_eq!(pc.compute_single(i), i);
        }
    }

    #[test]
    fn single_point_curve_is_constant() {
        let pc = PointCurve::from_points(vec![CurvePoint::new(100, 150, 1)]);
        let expected = 150 * 64; // 9600
        for i in (0..16384u16).step_by(173) {
            assert_eq!(pc.compute_single(i), expected);
        }
    }

    #[test]
    fn two_point_non_identity() {
        // (0, 0) → (128, 200)
        let pc = PointCurve::from_points(vec![
            CurvePoint::new(0, 0, 1),
            CurvePoint::new(128, 200, 1),
        ]);
        // x0=0, y0=0, x1=128*64=8192, y1=200*64=12800
        assert_eq!(pc.compute_single(0), 0);
        assert_eq!(pc.compute_single(8192), 12800);
        // 中点 v=4096 → y = 0 + 0.5 * 12800 = 6400
        let mid = pc.compute_single(4096);
        assert!((mid as i32 - 6400).abs() <= 1);
        // 超出 x1=8192 后 flat
        assert_eq!(pc.compute_single(10000), 12800);
    }

    #[test]
    fn from_xml_points_identity() {
        // 最常见的 XML 默认：两点 identity
        let pc = PointCurve::from_xml_points(&[(0, 0, 1), (255, 255, 1)]);
        assert!(pc.is_identity());
    }

    #[test]
    fn multipoint_curve_passes_through_endpoints() {
        // 4 个控制点
        let pc = PointCurve::from_points(vec![
            CurvePoint::new(0, 0, 1),
            CurvePoint::new(64, 80, 1),
            CurvePoint::new(192, 200, 1),
            CurvePoint::new(255, 255, 1),
        ]);
        // clamped B-spline 应穿过首末控制点
        assert_eq!(pc.compute_single(0), 0);
        // 末点 x=255 → 14bit=16320
        let at_end = pc.compute_single(16320);
        assert!(
            (at_end as i32 - 16320).abs() <= 5,
            "multipoint end at 16320 got {}",
            at_end
        );
    }

    #[test]
    fn build_lut_returns_same_data() {
        let pc = PointCurve::identity();
        let mut lut = Box::new([0u16; 16384]);
        pc.build_lut(&mut *lut);
        for i in (0..16384usize).step_by(97) {
            assert_eq!(lut[i], pc.compute_single(i as u16));
        }
    }

    #[test]
    fn curve_point_14bit_conversion() {
        let p = CurvePoint::new(100, 200, 1);
        assert_eq!(p.x_14bit(), 6400);
        assert_eq!(p.y_14bit(), 12800);
    }
}
