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

    /// 是否启用 apply（§52 chroma permutation 修复后启用）
    pub fn should_apply(&self) -> bool {
        self.apply_cc && self.is_customized()
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
    /// 从 params 预编译（镜像 FUN_702d57b0，T22-follow 完整反编译验证）
    ///
    /// 公式（每 cell 独立，全部 18 个从 Ghidra 1:1 decode）：
    ///
    /// ```text
    /// M3x6[o][k] = Σ_{j ∈ cols[o]} M6x6[k][j] + (Sat if k ∈ cols[o] else 0)
    ///
    /// cols[0] = {1, 2, 3}   // R output: 副列{1,2} + primary{3}
    /// cols[1] = {0, 2, 4}   // G output: 副列{0,2} + primary{4}
    /// cols[2] = {0, 1, 5}   // B output: 副列{0,1} + primary{5}
    /// ```
    ///
    /// 对称模式：`cols[o] = ({0,1,2} \ {o}) ∪ {o + 3}`
    pub fn compile(params: &ColorCorrParams) -> Self {
        let enabled = params.should_apply();

        // T22-follow 反编译得出的精确列选择
        const COLS: [[usize; 3]; 3] = [
            [1, 2, 3], // out=0 (R)
            [0, 2, 4], // out=1 (G)
            [0, 1, 5], // out=2 (B)
        ];

        let mut m3x6 = [[0i16; 6]; 3];
        let sat = params.saturation;
        let m = &params.matrix;

        for o in 0..3 {
            for k in 0..6 {
                // 3 列求和
                let sum: i32 = COLS[o].iter().map(|&j| m[k][j] as i32).sum();
                // Sat 加到 k ∈ cols[o] 的 cell
                let sat_add = if COLS[o].contains(&k) { sat as i32 } else { 0 };
                // i16 wrap (与原 x86 short 加法一致)
                m3x6[o][k] = (sum + sat_add) as i16;
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

    /// 单 RGB 像素 apply（§52 `FUN_702d4b50` 核心循环，agent 验证）
    ///
    /// DLL 把 6 个 chroma 按 `[B_exc, G_exc, R_exc, yellow, magenta, cyan]` 存栈，
    /// 但 M3x6 列索引 k=0..5 按 `[R_exc, G_exc, B_exc, cyan, magenta, yellow]` 访问。
    /// 等价于对 dot-product 应用 permutation `[2,1,0,5,4,3]`。
    pub fn apply_rgb_chunk(&self, chunk: &mut [u16]) {
        debug_assert_eq!(chunk.len(), 3);
        if !self.enabled {
            return;
        }

        let r = chunk[0] as i32;
        let g = chunk[1] as i32;
        let b = chunk[2] as i32;

        // DLL chroma 顺序（c0..c5），与 FUN_702d4b50 栈槽一致
        let c = [
            (b - r.max(g)).max(0) as f32,         // c0: pure blue excess
            (g - r.max(b)).max(0) as f32,         // c1: pure green excess
            (r - g.max(b)).max(0) as f32,         // c2: pure red excess
            (r.min(g) - b).max(0) as f32,         // c3: yellow content
            (r.min(b) - g).max(0) as f32,         // c4: magenta content
            (g.min(b) - r).max(0) as f32,         // c5: cyan content
        ];

        // DLL M3x6 列 → chroma permutation (agent 验证 R-row；G/B 推定同)
        const PERM: [usize; 6] = [2, 1, 0, 5, 4, 3];

        for (ch, val) in chunk.iter_mut().enumerate() {
            let mut sum = 0.0f32;
            for k in 0..6 {
                sum += (self.m3x6[ch][k] as f32) * c[PERM[k]];
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
    fn saturation_only_triggers_apply() {
        let mut p = zero_params();
        p.saturation = 20;
        assert!(p.is_customized());
        assert!(p.should_apply());
    }

    #[test]
    fn matrix_nonzero_triggers_apply() {
        let mut p = zero_params();
        p.matrix[2][3] = 50;
        assert!(p.is_customized());
        assert!(p.should_apply());
    }

    #[test]
    fn compile_pattern_out0_r() {
        // T22-follow: cols[0] = {1, 2, 3}
        // Put distinct values at m[0][1], m[0][2], m[0][3]; Sat=0
        let mut p = zero_params();
        p.saturation = 0;
        p.matrix[0][1] = 10;
        p.matrix[0][2] = 20;
        p.matrix[0][3] = 30;
        let cc = ColorCorrection::compile(&p);
        // M3x6[0][0] = M6x6[0][1] + M6x6[0][2] + M6x6[0][3] = 10+20+30 = 60
        assert_eq!(cc.m3x6[0][0], 60);
        // M3x6[1][0] = cols[1]={0,2,4}: 0 + 20 + 0 = 20
        assert_eq!(cc.m3x6[1][0], 20);
        // M3x6[2][0] = cols[2]={0,1,5}: 0 + 10 + 0 = 10
        assert_eq!(cc.m3x6[2][0], 10);
    }

    #[test]
    fn compile_saturation_pattern() {
        // Sat 加到 k ∈ cols[o] 的 cell
        let mut p = zero_params();
        p.saturation = 100;
        // matrix 全 0，所以 sum=0, cell = sat_add
        let cc = ColorCorrection::compile(&p);
        // out=0, cols={1,2,3}: Sat 加到 k∈{1,2,3}
        assert_eq!(cc.m3x6[0][0], 0);
        assert_eq!(cc.m3x6[0][1], 100);
        assert_eq!(cc.m3x6[0][2], 100);
        assert_eq!(cc.m3x6[0][3], 100);
        assert_eq!(cc.m3x6[0][4], 0);
        assert_eq!(cc.m3x6[0][5], 0);
        // out=1, cols={0,2,4}
        assert_eq!(cc.m3x6[1][0], 100);
        assert_eq!(cc.m3x6[1][1], 0);
        assert_eq!(cc.m3x6[1][2], 100);
        assert_eq!(cc.m3x6[1][3], 0);
        assert_eq!(cc.m3x6[1][4], 100);
        assert_eq!(cc.m3x6[1][5], 0);
        // out=2, cols={0,1,5}
        assert_eq!(cc.m3x6[2][0], 100);
        assert_eq!(cc.m3x6[2][1], 100);
        assert_eq!(cc.m3x6[2][2], 0);
        assert_eq!(cc.m3x6[2][3], 0);
        assert_eq!(cc.m3x6[2][4], 0);
        assert_eq!(cc.m3x6[2][5], 100);
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
        // 灰度像素 (R=G=B) → 所有 6 个 chroma 项都是 0 → 无 delta → 输出不变
        let mut p = zero_params();
        p.saturation = 50;
        for row in &mut p.matrix {
            for cell in row {
                *cell = 1000;
            }
        }
        let cc = ColorCorrection::compile(&p);
        assert!(cc.enabled);
        // 灰度像素 → 所有 6 chroma 项为 0 → delta=0 → 输出不变
        let mut px = [5000u16, 5000, 5000];
        cc.apply_rgb_chunk(&mut px);
        assert_eq!(px, [5000, 5000, 5000]);
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
