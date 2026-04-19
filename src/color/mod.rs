//! 色彩管理模块：ICC 配置文件、色彩空间转换、胶片处理和手动调整。

mod profile;
mod transform;
mod processing;
mod adjust;
mod usm;
pub mod flex;
mod flex_apply;

pub use profile::{IccProfileInfo, IccProfileDetail, SettingsPreset, scan_icc_profiles, scan_settings_presets, parse_icc_detail};
pub use transform::{TargetColorSpace, IccIntent, IccSettings, apply_icc_transform, apply_icc_transform_ex, apply_icc_transform_profiles};
pub use processing::{apply_film_processing, apply_film_curve_lut, apply_gradation_curves, apply_color_pipeline, apply_color_pipeline_ex, build_curve_lut, desaturate_bw, desaturate_bw_via_hasselblad, extract_film_curve, extract_film_curve_16, FILM_CURVE_LUT_R, FILM_CURVE_LUT_G, FILM_CURVE_LUT_B, lut_interp_16};
pub use adjust::{ManualAdjust, apply_manual_adjust, apply_scanner_levels, apply_display_adjust, extract_embedded_icc};
pub use usm::apply_usm;
pub use flex_apply::{apply_flex_pipeline, apply_flex_pipeline_no_icc};
