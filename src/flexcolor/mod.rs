//! FlexColor 编辑历史解析模块
//!
//! 解析 Imacon/Hasselblad FlexColor 软件在 FFF 文件中嵌入的编辑参数（标签 0xC519）。
//! 数据以 Apple plist XML 格式存储，包含图像校正参数、色阶曲线、ICC 配置文件等信息。

mod model;
mod parser;

pub use model::{ImageSetting, DateTime, ImageCorrection, EditHistory};
pub use model::{film_curve_name, film_type_name, color_model_name};
pub use parser::parse_settings_xml;
