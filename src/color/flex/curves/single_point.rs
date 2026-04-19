//! CSinglePointCurve — 单点可选 LUT（§16.1）
//!
//! 源自 docs §16.1 反编译。极简结构：若 `enabled` 则查 LUT，否则直通。
//!
//! ```text
//! ushort compute(ushort x) {
//!     if (this->enabled != 0) return lut[x];
//!     return x;
//! }
//! ```
//!
//! 默认参数 field_0x20 = field_0x24 = 128.0 (`_DAT_707346dc`)，即中点。
//! FlexColor 构造时 enabled=1（默认启用）。
//!
//! ## LUT 来源（待定）
//!
//! §16.3 注："this+0x84 inline CPointCurve 可能用于 CSinglePointCurve LUT 生成"，
//! 但未完全证实。MVP 先提供 identity LUT + 接口准备。

use super::{Curve, MAX_14BIT};

/// 单点 LUT 曲线
pub struct SinglePointCurve {
    /// 预计算 14-bit LUT（若 None 则直通）
    pub lut: Option<Box<[u16; 16384]>>,
    /// 启用标志（对应 field_0x1c）
    pub enabled: bool,
    /// field_0x20（默认 128.0）— 含义待查，MVP 保留但不参与计算
    pub field_0x20: f64,
    /// field_0x24（默认 128.0）— 同上
    pub field_0x24: f64,
}

impl SinglePointCurve {
    /// FlexColor 默认构造：enabled=true，无 LUT（= identity）
    pub fn default_enabled() -> Self {
        Self {
            lut: None,
            enabled: true,
            field_0x20: 128.0,
            field_0x24: 128.0,
        }
    }

    /// 禁用状态（直通）
    pub fn disabled() -> Self {
        Self {
            lut: None,
            enabled: false,
            field_0x20: 128.0,
            field_0x24: 128.0,
        }
    }

    /// 指定 LUT 构造
    pub fn with_lut(lut: Box<[u16; 16384]>) -> Self {
        Self {
            lut: Some(lut),
            enabled: true,
            field_0x20: 128.0,
            field_0x24: 128.0,
        }
    }
}

impl Curve for SinglePointCurve {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn compute_single(&self, x: u16) -> u16 {
        if !self.enabled {
            return x;
        }
        match &self.lut {
            Some(lut) => lut[(x as usize).min(16383)],
            None => x, // 无 LUT = identity
        }
    }

    fn build_lut(&self, lut: &mut [u16; 16384]) {
        match (&self.lut, self.enabled) {
            (Some(src), true) => lut.copy_from_slice(&**src),
            _ => {
                for (i, v) in lut.iter_mut().enumerate() {
                    *v = (i as u16).min(MAX_14BIT);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_enabled_no_lut_is_identity() {
        let c = SinglePointCurve::default_enabled();
        assert!(c.enabled());
        for i in (0..16384u16).step_by(173) {
            assert_eq!(c.compute_single(i), i);
        }
    }

    #[test]
    fn disabled_is_identity() {
        let c = SinglePointCurve::disabled();
        for i in (0..16384u16).step_by(173) {
            assert_eq!(c.compute_single(i), i);
        }
    }

    #[test]
    fn with_lut_applies_lut() {
        let mut lut = Box::new([0u16; 16384]);
        for (i, v) in lut.iter_mut().enumerate() {
            *v = (i as u16).wrapping_add(1000).min(MAX_14BIT);
        }
        let c = SinglePointCurve::with_lut(lut);
        assert_eq!(c.compute_single(0), 1000);
        assert_eq!(c.compute_single(100), 1100);
    }

    #[test]
    fn disabled_with_lut_still_passthrough() {
        // 如果 enabled=false 即使有 LUT 也不应用
        let lut = Box::new([42u16; 16384]);
        let mut c = SinglePointCurve::with_lut(lut);
        c.enabled = false;
        assert_eq!(c.compute_single(100), 100); // passthrough
    }

    #[test]
    fn default_fields_are_128() {
        let c = SinglePointCurve::default_enabled();
        assert_eq!(c.field_0x20, 128.0);
        assert_eq!(c.field_0x24, 128.0);
    }
}
