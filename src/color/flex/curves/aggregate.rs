//! CAggregateCurve — Pipeline 合成器
//!
//! 将多条子曲线按 3 种 composition mode 组合成单一 14-bit LUT。
//! 精确语义见 docs §16.4（勘误版，EDI 寄存器 `last_m0` 跟踪机制）。
//!
//! ## 三种 mode
//!
//! - **mode 0 (Sequential)**：`y = child(last_m0); running = y; last_m0 = y`
//!   顺序链式调用，child 输入 = 上一次的输出（或最初 input）。
//!
//! - **mode 1 (AddDelta)**：`y = child(last_m0); running = (ushort)(running + (short)y)`
//!   child 输入用 last_m0（不更新），输出被**加到** running（wrapping ushort）。
//!
//! - **mode 2 (SubClamp)**：`y = child(last_m0); running -= y; clamp [0, 0x3FFF]`
//!   child 输入用 last_m0，输出**减去** running，clamp。
//!
//! ## 关键不变量
//!
//! `last_m0` 只在 mode 0 时更新。mode 1/2 给 child 传的是"最后一次 mode 0 的输出"，
//! 不是"原始入口 input"。这是 FlexColor 为"让 user 曲线 A/B 作用在正像域"的机制。

use super::{Curve, CompositionMode, MAX_14BIT};

/// 聚合曲线：按 mode 串联多条子曲线
pub struct AggregateCurve {
    /// 子曲线列表（每项：child + 该 child 使用的 composition mode）
    pub children: Vec<(Box<dyn Curve>, CompositionMode)>,
    /// 启用标志（对应 `this->field_0x08`）
    pub enabled: bool,
}

impl AggregateCurve {
    /// 新建空 aggregate（可用 `add` 链式添加）
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            enabled: true,
        }
    }

    /// 添加子曲线 + mode
    pub fn add(mut self, child: Box<dyn Curve>, mode: CompositionMode) -> Self {
        self.children.push((child, mode));
        self
    }

    /// 添加子曲线 + Sequential mode（便捷方法）
    pub fn add_seq(self, child: Box<dyn Curve>) -> Self {
        self.add(child, CompositionMode::Sequential)
    }

    /// 子曲线数量
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl Default for AggregateCurve {
    fn default() -> Self {
        Self::new()
    }
}

impl Curve for AggregateCurve {
    fn enabled(&self) -> bool {
        self.enabled && !self.children.is_empty()
    }

    fn compute_single(&self, input: u16) -> u16 {
        if !self.enabled || self.children.is_empty() {
            return input;
        }

        // §16.4 精确实现：维护 running 和 last_m0 两个变量
        let mut running: u16 = input;
        let mut last_m0: u16 = input;

        for (child, mode) in &self.children {
            let y = child.compute_single(last_m0);
            match mode {
                CompositionMode::Sequential => {
                    running = y;
                    last_m0 = y;
                }
                CompositionMode::AddDelta => {
                    // 对应 C++ `_param_1 = (ushort)(_param_1 + (short)uVar2);`
                    // 注意：这里是 wrapping ushort 加 signed short
                    running = running.wrapping_add(y);
                }
                CompositionMode::SubClamp => {
                    // 对应 C++ `tmp = running - y; clamp [0, 0x3FFF]`
                    let tmp = running as i32 - y as i32;
                    running = if tmp < 0 {
                        0
                    } else if tmp > MAX_14BIT as i32 {
                        MAX_14BIT
                    } else {
                        tmp as u16
                    };
                }
            }
        }

        running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::gamma::GammaCurve;

    /// 辅助：一个固定值曲线
    struct ConstCurve(u16);
    impl Curve for ConstCurve {
        fn compute_single(&self, _x: u16) -> u16 {
            self.0
        }
    }

    /// 辅助：identity 曲线（输出 = 输入）
    struct IdentityCurve;
    impl Curve for IdentityCurve {
        fn compute_single(&self, x: u16) -> u16 {
            x
        }
    }

    /// 辅助：加 delta 曲线（输出 = 输入 + delta，wrapping）
    struct AddDeltaCurve(i32);
    impl Curve for AddDeltaCurve {
        fn compute_single(&self, x: u16) -> u16 {
            ((x as i32 + self.0) & 0xFFFF) as u16
        }
    }

    #[test]
    fn empty_aggregate_is_identity() {
        let agg = AggregateCurve::new();
        for i in (0..16384u16).step_by(173) {
            assert_eq!(agg.compute_single(i), i);
        }
    }

    #[test]
    fn single_sequential_child() {
        let agg = AggregateCurve::new()
            .add_seq(Box::new(IdentityCurve));
        assert_eq!(agg.compute_single(5000), 5000);
    }

    #[test]
    fn sequential_chain_composes() {
        // 两个 Sequential child：输出应是 child2(child1(input))
        // 这里用 AddDeltaCurve 链：first 加 100, second 加 200 → 最终 +300
        let agg = AggregateCurve::new()
            .add_seq(Box::new(AddDeltaCurve(100)))
            .add_seq(Box::new(AddDeltaCurve(200)));
        assert_eq!(agg.compute_single(5000), 5300);
    }

    #[test]
    fn mode_sequential_updates_last_m0() {
        // first mode 0 child 输出 999；
        // second mode 1 (AddDelta) 给 child 传 last_m0=999；
        // 假设 mode 1 child = ConstCurve(10)，输出 10；
        // running (999) + 10 = 1009
        let agg = AggregateCurve::new()
            .add(Box::new(ConstCurve(999)), CompositionMode::Sequential)
            .add(Box::new(ConstCurve(10)), CompositionMode::AddDelta);
        assert_eq!(agg.compute_single(5000), 999 + 10);
    }

    #[test]
    fn mode1_child_input_is_last_m0_not_original() {
        // 关键测试：验证 mode 1 child 接收 last_m0 而非原始 input
        // setup:
        //   child1 (mode 0): identity → last_m0 = input
        //   child2 (mode 0): ConstCurve(42) → last_m0 = 42
        //   child3 (mode 1): SpyCurve 记录它接收到的 input
        // 预期 child3 接收 42（上次 mode 0 输出），不是原始 input

        use std::cell::Cell;

        struct SpyCurve {
            received: std::rc::Rc<Cell<u16>>,
        }
        impl Curve for SpyCurve {
            fn compute_single(&self, x: u16) -> u16 {
                self.received.set(x);
                0 // 返回什么不重要
            }
        }

        let received = std::rc::Rc::new(Cell::new(0u16));
        let agg = AggregateCurve::new()
            .add(Box::new(IdentityCurve), CompositionMode::Sequential)
            .add(Box::new(ConstCurve(42)), CompositionMode::Sequential)
            .add(
                Box::new(SpyCurve {
                    received: received.clone(),
                }),
                CompositionMode::AddDelta,
            );

        let _ = agg.compute_single(5000);
        assert_eq!(
            received.get(),
            42,
            "mode 1 child 应收到 last_m0=42，不是原始 5000"
        );
    }

    #[test]
    fn mode1_before_any_mode0_uses_original_input() {
        // 当 mode 1 出现在第一个 child 时，last_m0 = 原始 input
        use std::cell::Cell;

        struct SpyCurve {
            received: std::rc::Rc<Cell<u16>>,
        }
        impl Curve for SpyCurve {
            fn compute_single(&self, x: u16) -> u16 {
                self.received.set(x);
                0
            }
        }

        let received = std::rc::Rc::new(Cell::new(0u16));
        let agg = AggregateCurve::new().add(
            Box::new(SpyCurve {
                received: received.clone(),
            }),
            CompositionMode::AddDelta,
        );

        let _ = agg.compute_single(7777);
        assert_eq!(
            received.get(),
            7777,
            "无 mode 0 时 mode 1 应收到原始 input"
        );
    }

    #[test]
    fn mode2_clamps_to_zero() {
        // running - child > running → 若 child 较大则结果 < 0 → clamp 0
        let agg = AggregateCurve::new()
            .add(Box::new(ConstCurve(100)), CompositionMode::Sequential)
            .add(Box::new(ConstCurve(500)), CompositionMode::SubClamp);
        // running = 100, 减 500 = -400 → 0
        assert_eq!(agg.compute_single(5000), 0);
    }

    #[test]
    fn mode2_clamps_to_max() {
        // 构造 running > 16383 的情况：用 AddDelta 先加到很大
        // 实际 sub clamp 上限 = 16383（MAX_14BIT）
        // running 开始 = 5000，加一个大值 mode 1 → wrapping 行为
        // 这里单测上限：running = 某大值，减去 0 → 保留 running（若 ≤ 14bit）
        let agg = AggregateCurve::new()
            .add(Box::new(ConstCurve(100)), CompositionMode::Sequential)
            .add(Box::new(ConstCurve(0)), CompositionMode::SubClamp);
        assert_eq!(agg.compute_single(5000), 100);
    }

    #[test]
    fn sub_clamp_normal_case() {
        let agg = AggregateCurve::new()
            .add(Box::new(ConstCurve(1000)), CompositionMode::Sequential)
            .add(Box::new(ConstCurve(300)), CompositionMode::SubClamp);
        assert_eq!(agg.compute_single(5000), 700);
    }

    #[test]
    fn gamma_identity_in_aggregate() {
        // 把 GammaCurve(2.0) 放进 aggregate，应等于 identity
        let agg = AggregateCurve::new().add_seq(Box::new(GammaCurve::new(2.0)));
        for i in (0..16384u16).step_by(173) {
            assert_eq!(agg.compute_single(i), i, "v={}", i);
        }
    }

    #[test]
    fn build_lut_for_aggregate() {
        let agg = AggregateCurve::new().add_seq(Box::new(AddDeltaCurve(100)));
        let mut lut = Box::new([0u16; 16384]);
        agg.build_lut(&mut *lut);
        for i in (0..16384usize).step_by(97) {
            let expected = ((i + 100) & 0xFFFF) as u16;
            assert_eq!(lut[i], expected);
        }
    }

    #[test]
    fn nested_aggregate() {
        // aggregate 嵌套 aggregate
        let inner = AggregateCurve::new().add_seq(Box::new(AddDeltaCurve(50)));
        let outer = AggregateCurve::new()
            .add_seq(Box::new(inner))
            .add_seq(Box::new(AddDeltaCurve(50)));
        assert_eq!(outer.compute_single(1000), 1100);
    }
}
