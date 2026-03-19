//! FFF（Flextight 文件格式）解析库入口。
//! 重新导出 TIFF 解析、配置、国际化、附属文件、标签名映射、色彩管理及 FlexColor 编辑历史等模块。

pub mod color;
pub mod config;
pub mod flexcolor;
pub mod i18n;
pub mod sidecar;
pub mod tags;
pub mod tiff;
