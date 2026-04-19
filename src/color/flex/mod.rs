//! FlexColor pipeline 复刻模块
//!
//! 基于逆向工程 `docs/flexcolor-reverse-engineering.md` 的结论，
//! 从 XML `ImageCorrection` 参数前向计算 14-bit per-channel LUT。
//!
//! 架构参考 §16.5 / §25 / §42。

pub mod curves;
pub mod bspline;
pub mod pipeline;

pub use curves::{Curve, CompositionMode};
pub use pipeline::Pipeline;
