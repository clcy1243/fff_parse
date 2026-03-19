//! 色彩管理模块：ICC 配置文件、色彩空间转换、胶片处理和手动调整。

mod profile;
mod transform;
mod processing;
mod adjust;

pub use profile::{IccProfileInfo, SettingsPreset, scan_icc_profiles, scan_settings_presets};
pub use transform::{TargetColorSpace, apply_icc_transform};
pub use processing::apply_film_processing;
pub use adjust::{ManualAdjust, apply_manual_adjust, extract_embedded_icc};
