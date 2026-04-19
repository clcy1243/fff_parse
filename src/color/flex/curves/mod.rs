//! FlexColor 曲线类族
//!
//! 所有曲线在 **14-bit 域**（0..16383）工作。对应 C++ `CCurve` 基类的 vtable slot 12。

pub mod contrast;
pub mod gamma;
pub mod negative;
pub mod aggregate;
pub mod high_shadow;
pub mod point;
pub mod single_point;

pub use contrast::ContrastCurve;
pub use gamma::{GammaCurve, GammaNegCurve};
pub use negative::NegativeCurve;
pub use aggregate::AggregateCurve;
pub use high_shadow::{HighShadowCurve, ShadowMode, HighlightMode};
pub use point::{PointCurve, CurvePoint};
pub use single_point::SinglePointCurve;

/// 14-bit curve 的基础接口
///
/// 对应 C++ `CCurve` vtable：
/// - slot 12 (`compute_single`) — 单像素计算
/// - slot 8 (`build_lut`) — 预计算 16384-entry LUT（默认实现 = 逐点调用 compute_single）
pub trait Curve {
    /// 单像素 14-bit 计算
    fn compute_single(&self, x: u16) -> u16;

    /// 构建 14-bit LUT（16384 entries）
    ///
    /// 默认实现 = 逐点调用 `compute_single`。具体曲线若有更快路径可覆盖。
    fn build_lut(&self, lut: &mut [u16; 16384]) {
        for i in 0..16384 {
            lut[i] = self.compute_single(i as u16);
        }
    }

    /// 是否启用（对应 C++ `this->field_0x08` 标志）
    ///
    /// 未启用时 `build_lut` 应产生 identity。
    fn enabled(&self) -> bool {
        true
    }
}

/// CAggregateCurve 的 child composition mode
///
/// 精确语义见 docs §16.4 勘误版。关键：mode 1/2 的 child 输入是 `last_m0`
/// （上一次 mode 0 的输出，或最初的 input），不是"原始 input"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionMode {
    /// mode 0: 顺序链。`child(last_m0) → running; last_m0 = running`
    Sequential,
    /// mode 1: 加性 delta（从 last_m0）。`running += (short)child(last_m0)`
    AddDelta,
    /// mode 2: 减性 clamp（从 last_m0）。`running -= child(last_m0); clamp [0, 0x3FFF]`
    SubClamp,
}

/// 14-bit 域上限（含），= 2^14 - 1
pub const MAX_14BIT: u16 = 16383;

/// 14-bit 域范围，= 2^14
pub const RANGE_14BIT: f64 = 16384.0;

/// Pipeline 固定 pivot = 14-bit 中点，= 2^13
pub const PIVOT_14BIT: u16 = 8192;
