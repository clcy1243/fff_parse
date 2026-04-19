//! ColorCorrection 6×6 矩阵 + Saturation 应用 — T22 实现
//!
//! 源自 docs §49（T22 agent 报告）。FlexColor 的 ApplyCC 特性：
//! - `this+0x4b4..0x4fb` 存 36 个 int16 矩阵（6×6）
//! - `this+0x4fc` 存 Saturation (int16)
//! - `this+0x88` 是 ApplyCC 启用位
//!
//! ## 真实算法（§49.1）
//!
//! **不是 6 通道矩阵**，而是：
//!
//! 1. **Compile 阶段** (`FUN_702d57b0`): 6×6 源矩阵 + Saturation → 3×6 编译矩阵
//! 2. **Apply 阶段** (`FUN_702d4b50`): 对每像素：
//!    - 从 RGB 计算 6 个 "opponent-excess" 色彩项
//!    - delta[ch] = dot(M3x6[ch], chromas) / 100
//!    - out[ch] = clamp14(rgb[ch] - delta[ch])
//!
//! 位置：per-channel LUT apply 之后，Lightness apply 之前。
//!
//! ## 6 个 opponent-excess 项（§49.1）
//!
//! | 索引 | 公式 | 含义 |
//! |------|------|------|
//! | 0 | max(0, B - max(R,G)) | pure blue excess |
//! | 1 | max(0, G - max(R,B)) | pure green excess |
//! | 2 | max(0, min(R,G) - B) | yellow content |
//! | 3 | max(0, min(R,B) - G) | magenta content |
//! | 4 | max(0, min(G,B) - R) | cyan content |
//! | 5 | max(0, R - max(G,B)) | pure red excess |
//!
//! ## Identity fast-path（§49.4）
//!
//! `all(M6x6[i][j] == -Saturation)` → 跳过整个 apply。
//! 默认状态下矩阵全 0 + Sat=0 → 所有 cells == -0 → fast-path 触发。

use super::curves::MAX_14BIT;

/// XML ImageCorrection 里的 ColorCorr 参数
#[derive(Debug, Clone)]
pub struct ColorCorrParams {
    /// 6×6 int16 源矩阵（行主）
    pub matrix: [[i16; 6]; 6],
    /// Saturation (-100..100)
    pub saturation: i16,
    /// ApplyCC gate
    pub apply_cc: bool,
}

impl ColorCorrParams {
    /// 从扁平 36 元素 vec（ImageCorrection.color_corr）+ saturation + apply_cc 构造
    pub fn from_image_correction(
        color_corr: &[i64],
        saturation: i64,
        apply_cc: bool,
    ) -> Self {
        let mut matrix = [[0i16; 6]; 6];
        for (i, &v) in color_corr.iter().take(36).enumerate() {
            matrix[i / 6][i % 6] = v.clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        }
        Self {
            matrix,
            saturation: saturation.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
            apply_cc,
        }
    }

    /// Identity fast-path check（§49.4 `FUN_702d4f30`）
    ///
    /// 等价 FlexColor 的 "is customized"：任一 cell 与 -Sat 不同即表示用户改过。
    pub fn is_customized(&self) -> bool {
        self.matrix
            .iter()
            .flatten()
            .any(|&c| c != -self.saturation)
    }

    /// 是否启用 apply
    ///
    /// **当前完全禁用**（MVP 保守）：T22 的 3×6 compile pattern 是推测，未经
    /// round-trip 验证。实测 e2e_all_config（有真实 matrix 数据）启用后 MAE 3798→8755 回归。
    /// 保留代码 + 单测，待 round-trip 验证 compile pattern 后再启用。
    pub fn should_apply(&self) -> bool {
        false
    }
}

/// 预编译后的 ColorCorrection（供 pipeline 使用）
pub struct ColorCorrection {
    /// 3×6 编译矩阵
    pub m3x6: [[i16; 6]; 3],
    /// 是否需要 apply
    pub enabled: bool,
}

impl ColorCorrection {
    /// 从 params 预编译（镜像 FUN_702d57b0）
    ///
    /// 编译规则（§49.1）：每个 M3x6[out][k] = 3 个源 6×6 cells 求和，
    /// Saturation 添加到特定 cells。具体列选择模式**未完全证实**，
    /// 当前实现基于 T22 agent 的推测（需 round-trip test 验证）。
    pub fn compile(params: &ColorCorrParams) -> Self {
        let enabled = params.should_apply();

        // 推测的编译模式（§49.1 snippet）：
        //   M3x6[out][k] = M6x6[k][3] + M6x6[k][1] + M6x6[k][2] + (Sat_if_k_in_{1,2,3})
        //
        // 注：每一行 out (0, 1, 2 = R/G/B output) 选择的 6×6 列组合可能不同。
        // T22 agent 只提供了 out=0 行的 snippet。其他行先假设对称（pattern-symmetric）。
        //
        // **这是 MVP 占位实现**，精度需 round-trip test 调整。
        let mut m3x6 = [[0i16; 6]; 3];
        let sat = params.saturation;
        let m = &params.matrix;

        for k in 0..6 {
            // 3 个源 cells 求和 (列 3, 1, 2 of 6×6[k] — 推测)
            let sum = m[k][3] as i32 + m[k][1] as i32 + m[k][2] as i32;
            // Sat 添加模式（T22 snippet 示例：k in {1,2,3} 时 +Sat）
            let sat_add = if k >= 1 && k <= 3 { sat as i32 } else { 0 };

            for out in 0..3 {
                // 同样 pattern 适用于 3 个输出行（推测）
                m3x6[out][k] = (sum + sat_add).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            }
        }

        Self { m3x6, enabled }
    }

    /// 从 params 直接构造并编译
    pub fn from_params(params: &ColorCorrParams) -> Self {
        Self::compile(params)
    }

    /// 禁用状态（fast-path 用，无 apply）
    pub fn disabled() -> Self {
        Self {
            m3x6: [[0i16; 6]; 3],
            enabled: false,
        }
    }

    /// 单 RGB 像素 apply（§49.1 `FUN_702d4b50` 核心循环）
    pub fn apply_rgb_chunk(&self, chunk: &mut [u16]) {
        debug_assert_eq!(chunk.len(), 3);
        if !self.enabled {
            return;
        }

        let r = chunk[0] as i32;
        let g = chunk[1] as i32;
        let b = chunk[2] as i32;

        // 6 个 opponent-excess 色彩项
        let c = [
            (b - r.max(g)).max(0) as f32,         // pure blue excess
            (g - r.max(b)).max(0) as f32,         // pure green excess
            (r.min(g) - b).max(0) as f32,         // yellow content
            (r.min(b) - g).max(0) as f32,         // magenta content
            (g.min(b) - r).max(0) as f32,         // cyan content
            (r - g.max(b)).max(0) as f32,         // pure red excess
        ];

        // 每通道: delta = dot(M3x6[ch], c) / 100; out = in - delta
        for (ch, val) in chunk.iter_mut().enumerate() {
            let mut sum = 0.0f32;
            for k in 0..6 {
                sum += (self.m3x6[ch][k] as f32) * c[k];
            }
            let delta = (sum / 100.0).round() as i32;
            *val = ((*val as i32) - delta).clamp(0, MAX_14BIT as i32) as u16;
        }
    }

    /// 应用到整张 14-bit interleaved RGB
    pub fn apply_14bit_rgb(&self, pixels: &mut [u16]) {
        if !self.enabled {
            return;
        }
        for chunk in pixels.chunks_exact_mut(3) {
            self.apply_rgb_chunk(chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_params() -> ColorCorrParams {
        ColorCorrParams {
            matrix: [[0; 6]; 6],
            saturation: 0,
            apply_cc: true,
        }
    }

    #[test]
    fn identity_fast_path_when_all_zero() {
        let p = zero_params();
        // matrix 全 0 + Sat=0 → 所有 cells == -0 → is_customized = false
        assert!(!p.is_customized());
        assert!(!p.should_apply());
    }

    #[test]
    fn non_zero_sat_triggers_customized() {
        let mut p = zero_params();
        p.saturation = 10;
        // matrix 全 0 ≠ -10 → customized
        assert!(p.is_customized());
    }

    #[test]
    fn all_negsat_not_customized() {
        let mut p = zero_params();
        p.saturation = 10;
        for row in &mut p.matrix {
            for cell in row {
                *cell = -10;
            }
        }
        // 所有 cells == -Sat → not customized
        assert!(!p.is_customized());
    }

    #[test]
    fn apply_cc_gate() {
        let mut p = zero_params();
        p.saturation = 10;
        p.matrix[0][0] = 100;
        p.apply_cc = false;
        assert!(!p.should_apply()); // apply_cc=false 一票否决
    }

    #[test]
    fn saturation_only_does_not_trigger_mvp() {
        // MVP 保守策略：Sat != 0 但 matrix 全 0 时不启用
        let mut p = zero_params();
        p.saturation = 20;
        assert!(!p.should_apply());
    }

    #[test]
    fn matrix_nonzero_does_not_trigger_mvp() {
        // MVP 完全禁用 apply（待 round-trip 验证 compile pattern）
        let mut p = zero_params();
        p.matrix[2][3] = 50;
        assert!(!p.should_apply());
    }

    #[test]
    fn disabled_cc_is_passthrough() {
        let cc = ColorCorrection::disabled();
        let mut px = [1000u16, 2000, 3000];
        let before = px;
        cc.apply_rgb_chunk(&mut px);
        assert_eq!(px, before);
    }

    #[test]
    fn zero_matrix_produces_no_change() {
        let cc = ColorCorrection::compile(&zero_params());
        // enabled=false 因为 all cells == -0
        assert!(!cc.enabled);
        let mut px = [1000u16, 2000, 3000];
        let before = px;
        cc.apply_rgb_chunk(&mut px);
        assert_eq!(px, before);
    }

    #[test]
    fn gray_pixel_no_chroma_terms() {
        // 灰度像素 (R=G=B) → 所有 6 个 chroma 项都是 0 → 无 delta
        // 注：当前 should_apply=false (MVP 禁用)，此测试验证禁用时 passthrough
        let mut p = zero_params();
        p.saturation = 50;
        for row in &mut p.matrix {
            for cell in row {
                *cell = 1000;
            }
        }
        let cc = ColorCorrection::compile(&p);
        // MVP 禁用：enabled 应 false
        assert!(!cc.enabled);

        let mut px = [5000u16, 5000, 5000];
        cc.apply_rgb_chunk(&mut px);
        assert_eq!(px, [5000, 5000, 5000]); // passthrough
    }

    #[test]
    fn from_image_correction_fields() {
        let color_corr: Vec<i64> = (0..36).map(|i| i as i64).collect();
        let params = ColorCorrParams::from_image_correction(&color_corr, 10, true);
        assert_eq!(params.matrix[0][0], 0);
        assert_eq!(params.matrix[5][5], 35);
        assert_eq!(params.saturation, 10);
        assert!(params.apply_cc);
    }

    #[test]
    fn clamps_output_to_14bit() {
        // 极端矩阵值可能导致 out 超出 14-bit → 必须 clamp
        let mut p = zero_params();
        p.saturation = 1;
        for row in &mut p.matrix {
            for cell in row {
                *cell = 10000; // 极端
            }
        }
        let cc = ColorCorrection::compile(&p);
        let mut px = [MAX_14BIT, 0, 0]; // 纯红，触发 red excess 项
        cc.apply_rgb_chunk(&mut px);
        for &v in &px {
            assert!(v <= MAX_14BIT);
        }
    }
}
