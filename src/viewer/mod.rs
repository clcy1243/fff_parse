//! FFF 图像查看器模块
//!
//! 提供基于 egui 的桌面图像浏览应用，支持 Hasselblad FFF/3FR 和 TIFF 文件的
//! 缩略图浏览、放大查看、色彩管理、底片分割导出等功能。

mod types;
mod app;
mod navigation;
mod file_list;
mod loupe;
mod panels;
mod split;
mod helpers;

pub use types::FffViewerApp;
