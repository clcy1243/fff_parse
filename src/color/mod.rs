//! 色彩管理模块：ICC 配置文件、色彩空间转换、胶片处理和手动调整。

mod profile;
mod transform;
mod processing;
mod adjust;

pub use profile::{IccProfileInfo, IccProfileDetail, SettingsPreset, scan_icc_profiles, scan_settings_presets, parse_icc_detail};
pub use transform::{TargetColorSpace, apply_icc_transform};
pub use processing::{apply_film_processing, apply_film_curve_lut, apply_gradation_curves, build_curve_lut, extract_film_curve, FILM_CURVE_LUT_R, FILM_CURVE_LUT_G, FILM_CURVE_LUT_B, lut_interp_16};
pub use adjust::{ManualAdjust, apply_manual_adjust, extract_embedded_icc};
