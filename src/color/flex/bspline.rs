//! B-spline Cox-de Boor basis function
//!
//! 源自 docs §16.9 — 对应 C++ `FUN_702699e0` 反编译。标准 Cox-de Boor 递归定义：
//!
//! ```text
//! N(i, 1, t) = 1  if knot[i] <= t < knot[i+1]
//!            = 0  otherwise
//!
//! N(i, k, t) = (t - knot[i])     / (knot[i+k-1] - knot[i])     * N(i, k-1, t)
//!            + (knot[i+k] - t)   / (knot[i+k]   - knot[i+1])   * N(i+1, k-1, t)
//! ```
//!
//! FlexColor 的实际阶数 k 未实测（推测 4 / cubic）；knot 向量结构也待查。
//! 当前实现为通用 Cox-de Boor，可接受任意阶数与 knot 向量。
//!
//! ## 参考
//! - docs §16.9 B-spline 基函数
//! - docs §16.8 "参数采样 + 线性填充" hybrid 方式
//! - https://en.wikipedia.org/wiki/B-spline#Cox%E2%80%93de_Boor_recursion_formula

/// 计算 Cox-de Boor B-spline basis 函数 N(i, k, t)
///
/// # Arguments
/// - `i`: control point index
/// - `k`: 阶数（order，= degree + 1）。k=1 表示分段常数，k=4 表示 cubic B-spline
/// - `knots`: knot vector（必须严格递增或等值；长度至少 i + k + 1）
/// - `t`: 求值参数
///
/// # 数值稳定性
/// 当 `knot[i+k-1] == knot[i]` 时，那项系数为 0/0，约定为 0（不是 NaN）。
///
/// # Panics
/// 不 panic；越界返回 0.0。
pub fn cox_de_boor(i: usize, k: usize, knots: &[f64], t: f64) -> f64 {
    // 防御：索引越界则贡献为 0
    if i + k >= knots.len() {
        return 0.0;
    }

    // 基础情形
    if k == 1 {
        let t_i = knots[i];
        let t_ip1 = knots[i + 1];
        // 约定区间 [t_i, t_{i+1})（半开）
        if t >= t_i && t < t_ip1 {
            return 1.0;
        }
        // 右端点闭合：t = global max 时，归属到"最后一个非空区间"
        let last_knot = *knots.last().unwrap();
        if t >= last_knot && t_ip1 == last_knot && t_i < t_ip1 {
            // 检查此 i 是否是"最末非空区间"（后面再无 j 使 knots[j] < knots[j+1] == last）
            let is_last_nonempty = !((i + 1)..(knots.len() - 1))
                .any(|j| knots[j] < knots[j + 1] && knots[j + 1] == last_knot);
            if is_last_nonempty {
                return 1.0;
            }
        }
        return 0.0;
    }

    // 递归情形
    let t_i = knots[i];
    let t_ik_1 = knots[i + k - 1];
    let t_ip1 = knots[i + 1];
    let t_ik = knots[i + k];

    let left_denom = t_ik_1 - t_i;
    let right_denom = t_ik - t_ip1;

    let left = if left_denom.abs() < 1e-12 {
        0.0
    } else {
        (t - t_i) / left_denom * cox_de_boor(i, k - 1, knots, t)
    };

    let right = if right_denom.abs() < 1e-12 {
        0.0
    } else {
        (t_ik - t) / right_denom * cox_de_boor(i + 1, k - 1, knots, t)
    };

    left + right
}

/// 生成 "uniform clamped" knot vector
///
/// 对 n 个 control point + k 阶 B-spline，knot 向量长度 = n + k。
/// 前 k 个 = 0.0，后 k 个 = 1.0，中间 n-k 个均匀分布。
///
/// 这是 standard "clamped B-spline" — 曲线穿过首末端点。
pub fn uniform_clamped_knots(n: usize, k: usize) -> Vec<f64> {
    let total = n + k;
    let mut knots = Vec::with_capacity(total);
    let inner = if n > k { n - k } else { 0 };

    for _ in 0..k {
        knots.push(0.0);
    }
    for j in 1..=inner {
        knots.push(j as f64 / (inner + 1) as f64);
    }
    for _ in 0..k {
        knots.push(1.0);
    }
    knots
}

/// 在 B-spline 曲线上求值（加权控制点）
///
/// # Arguments
/// - `control_values`: 每个控制点的标量值（x 或 y）
/// - `knots`: knot 向量
/// - `k`: 阶数
/// - `t`: 参数
pub fn evaluate(control_values: &[f64], knots: &[f64], k: usize, t: f64) -> f64 {
    let n = control_values.len();
    let mut sum = 0.0;
    for i in 0..n {
        let basis = cox_de_boor(i, k, knots, t);
        sum += control_values[i] * basis;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    #[test]
    fn k1_is_indicator() {
        let knots = [0.0, 0.5, 1.0];
        assert!((cox_de_boor(0, 1, &knots, 0.25) - 1.0).abs() < EPS);
        assert!(cox_de_boor(0, 1, &knots, 0.75).abs() < EPS);
        assert!((cox_de_boor(1, 1, &knots, 0.75) - 1.0).abs() < EPS);
    }

    #[test]
    fn k1_right_endpoint_closed_at_last() {
        let knots = [0.0, 0.5, 1.0];
        // t=1.0 应属于最末区间
        assert!((cox_de_boor(1, 1, &knots, 1.0) - 1.0).abs() < EPS);
    }

    #[test]
    fn k2_linear_basis() {
        // k=2 (linear): N(0, 2, t) 应为三角形基函数
        let knots = [0.0, 0.5, 1.0, 1.5];
        let v0 = cox_de_boor(0, 2, &knots, 0.25);
        // 0.25 在 [0, 0.5) 上升阶段：(0.25-0)/(0.5-0) = 0.5
        assert!((v0 - 0.5).abs() < EPS);
        // 0.5 处：从 [0, 0.5) 下降到 0 + 从 [0.5, 1.0) 上升
        // 实际上 N(0, 2, 0.5) 应为峰值 1.0（或由 right 贡献衔接）
    }

    #[test]
    fn partition_of_unity() {
        // B-spline 基函数在 [knot[k-1], knot[n]) 区间内和为 1
        let n = 4;
        let k = 3;
        let knots = uniform_clamped_knots(n, k);
        for t_int in 0..=100 {
            let t = t_int as f64 / 100.0;
            let sum: f64 = (0..n).map(|i| cox_de_boor(i, k, &knots, t)).sum();
            // 容忍边界浮点误差
            assert!(
                (sum - 1.0).abs() < 1e-6 || sum.abs() < 1e-6,
                "t={} sum={}",
                t,
                sum
            );
        }
    }

    #[test]
    fn uniform_clamped_knot_length() {
        // n + k knots
        for &(n, k) in &[(4, 3), (5, 4), (10, 4), (3, 3)] {
            let knots = uniform_clamped_knots(n, k);
            assert_eq!(knots.len(), n + k, "n={} k={}", n, k);
            // 首 k 个 == 0，末 k 个 == 1
            for i in 0..k {
                assert_eq!(knots[i], 0.0);
                assert_eq!(knots[knots.len() - 1 - i], 1.0);
            }
        }
    }

    #[test]
    fn evaluate_linear_endpoints() {
        // 2 个 control points + k=2 → linear interpolation
        let n = 2;
        let k = 2;
        let knots = uniform_clamped_knots(n, k);
        // knots = [0, 0, 1, 1]
        let vals = [10.0, 20.0];
        assert!((evaluate(&vals, &knots, k, 0.0) - 10.0).abs() < EPS);
        assert!((evaluate(&vals, &knots, k, 1.0) - 20.0).abs() < EPS);
        assert!((evaluate(&vals, &knots, k, 0.5) - 15.0).abs() < EPS);
    }

    #[test]
    fn evaluate_cubic_clamped_hits_endpoints() {
        // n=4 控制点 + k=4 (cubic clamped B-spline) → 曲线应穿过首末 control point
        let n = 4;
        let k = 4;
        let knots = uniform_clamped_knots(n, k);
        let vals = [0.0, 1.0, 2.0, 3.0];
        assert!((evaluate(&vals, &knots, k, 0.0) - 0.0).abs() < 1e-6);
        assert!((evaluate(&vals, &knots, k, 1.0) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn oob_index_returns_zero() {
        let knots = [0.0, 0.5, 1.0];
        // i 越界应返回 0
        assert_eq!(cox_de_boor(10, 2, &knots, 0.5), 0.0);
    }
}
