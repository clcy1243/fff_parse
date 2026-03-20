//! 工具函数模块
//!
//! 提供文件扫描、目录获取、名称缩短、资源查找、
//! 图像位深转换和 GPU 纹理尺寸限制等辅助功能。

use super::types::*;

use eframe::egui;
use std::path::Path;
use std::path::PathBuf;

// ─── 工具函数 ───────────────────────────────────────────────────────────────

/// 扫描目录中的图像文件（FFF/3FR/TIFF），按指定深度递归子目录
pub(super) fn scan_fff_files(dir: &Path, depth: DirScanDepth) -> Vec<PathBuf> {
    /// 判断文件是否为支持的图像格式
    fn is_image_file(path: &Path) -> bool {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => matches!(ext.to_lowercase().as_str(), "fff" | "3fr" | "tif" | "tiff"),
            None => false,
        }
    }

    /// 递归收集文件
    fn collect(dir: &Path, remaining: Option<usize>, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if is_image_file(&path) {
                    out.push(path);
                }
            } else if path.is_dir() {
                let is_hidden = path.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false);
                if !is_hidden {
                    match remaining {
                        Some(0) => {} // depth exhausted
                        Some(n) => collect(&path, Some(n - 1), out),
                        None => collect(&path, None, out), // unlimited
                    }
                }
            }
        }
    }

    let mut files = Vec::new();
    match depth {
        DirScanDepth::Flat => collect(dir, Some(0), &mut files),
        DirScanDepth::OneLevel => collect(dir, Some(1), &mut files),
        DirScanDepth::All => collect(dir, None, &mut files),
    }
    files
}

/// 获取根目录列表：用户主目录 + 外接卷宗 + 系统根目录
pub(super) fn get_root_dirs() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Some(home) = dirs_home() {
        roots.push(home);
    }

    let volumes = PathBuf::from("/Volumes");
    if volumes.exists() {
        if let Ok(entries) = std::fs::read_dir(&volumes) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                // Skip symlinks — "Macintosh HD" is a symlink to "/" on macOS.
                // Real external / removable drives are actual directories.
                let is_symlink = path
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                // Skip hidden system volumes (e.g. .Spotlight-V100, .timemachine)
                let is_hidden = path
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false);
                // Skip volumes that are not readable (e.g. "Macintosh HD - Data"
                // has d--x--x--x permissions — only the kernel can list it).
                // External USB drives always have normal read permissions.
                let is_readable = std::fs::read_dir(&path).is_ok();
                if !is_symlink && !is_hidden && is_readable {
                    roots.push(path);
                }
            }
        }
    }

    if cfg!(not(target_os = "windows")) {
        roots.push(PathBuf::from("/"));
    }

    roots.sort();
    roots.dedup();
    roots
}

/// 获取用户主目录路径
pub(super) fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// 缩短冗长的目录名以便在树形视图中展示。
/// 完整名称仍会在悬停提示中显示。
pub(super) fn shorten_dir_name(name: &str) -> String {
    // Adobe Creative Cloud sync folders contain long personal-info paths
    // e.g. "Creative Cloud Files - john.doe@email.com"
    // e.g. "CoreSync - john.doe@email.com - Adobe Creative Cloud"
    if let Some(idx) = name.find(" - ") {
        let prefix = &name[..idx];
        // Known verbose prefixes
        if prefix == "Creative Cloud Files"
            || prefix == "CoreSync"
            || name.contains("Adobe Creative Cloud")
            || name.contains("Creative Cloud")
        {
            return prefix.to_string();
        }
    }

    // Strip common verbose suffixes / patterns
    let patterns: &[(&str, &str)] = &[
        // "Something (john.doe@email.com)" → "Something"
        // Generic: trim trailing parenthesized email/id
    ];
    let mut result = name.to_string();
    for (find, replace) in patterns {
        result = result.replace(find, replace);
    }

    // If name contains an email address, redact it
    if let Some(at_pos) = result.find('@') {
        // Find the word containing '@'
        let start = result[..at_pos].rfind(|c: char| c == ' ' || c == '-').map(|i| i + 1).unwrap_or(0);
        let end = result[at_pos..].find(|c: char| c == ' ' || c == '-').map(|i| at_pos + i).unwrap_or(result.len());
        let email = &result[start..end];
        if email.contains('.') {
            // Replace email with "***"
            result = format!("{}***{}", &result[..start], &result[end..]);
            // Clean up double spaces / trailing dashes
            result = result.replace("  ", " ").replace(" - ***", "").replace("*** - ", "");
            result = result.trim_end_matches(" -").trim_end_matches("- ").trim().to_string();
        }
    }

    // Trim excessively long names (>40 chars)
    if result.chars().count() > 40 {
        let truncated: String = result.chars().take(37).collect();
        return format!("{}…", truncated);
    }

    result
}

/// 查找资源目录（profiles/ 或 settings/），依次检查可执行文件旁、
/// 当前工作目录和 CARGO_MANIFEST_DIR。
pub(super) fn find_resource_dir(name: &str, exe_dir: Option<&Path>) -> Option<PathBuf> {
    // 1. Check next to the executable (for .app bundles: Contents/MacOS/../Resources/)
    if let Some(dir) = exe_dir {
        // In .app bundle: exe is at Contents/MacOS/app, resources at Contents/Resources/
        let bundle_resources = dir.join("../Resources").join(name);
        if bundle_resources.exists() {
            return Some(bundle_resources);
        }
        // Flat layout: resources next to binary
        let flat = dir.join(name);
        if flat.exists() {
            return Some(flat);
        }
    }

    // 2. Check current working directory
    let cwd = std::env::current_dir().ok()?;
    let cwd_path = cwd.join(name);
    if cwd_path.exists() {
        return Some(cwd_path);
    }

    // 3. Check CARGO_MANIFEST_DIR (dev mode)
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let dev_path = PathBuf::from(manifest_dir).join(name);
        if dev_path.exists() {
            return Some(dev_path);
        }
    }

    None
}

/// 将 16 位图像转换为 8 位用于显示。
/// 扫描仪原始数据已经过伽马编码，简单右移即可。
/// 8 位图像直接返回。使用 rayon 并行加速处理大图像。
pub(super) fn convert_16_to_8_for_display(img: image::DynamicImage) -> image::DynamicImage {
    use rayon::prelude::*;

    match &img {
        image::DynamicImage::ImageRgb16(rgb16) => {
            let (w, h) = (rgb16.width(), rgb16.height());
            let src = rgb16.as_raw();
            let mut out_pixels = vec![0u8; (w as usize) * (h as usize) * 3];
            let row_len = w as usize * 3;

            out_pixels
                .par_chunks_mut(row_len)
                .enumerate()
                .for_each(|(y, row_dst)| {
                    let row_start = y * row_len;
                    for x in 0..row_len {
                        row_dst[x] = (src[row_start + x] >> 8) as u8;
                    }
                });

            let img_buf = image::RgbImage::from_raw(w, h, out_pixels)
                .expect("convert_16_to_8: buffer size mismatch");
            image::DynamicImage::ImageRgb8(img_buf)
        }
        _ => img,
    }
}

/// 缩放超出 GPU 最大纹理尺寸 (16384) 的图像，防止上传纹理时崩溃
pub(super) fn clamp_image_for_gpu(img: image::DynamicImage) -> image::DynamicImage {
    const MAX_TEX: u32 = 16384;
    let (w, h) = (img.width(), img.height());
    if w <= MAX_TEX && h <= MAX_TEX {
        return img;
    }
    let scale = (MAX_TEX as f64 / w as f64).min(MAX_TEX as f64 / h as f64);
    let nw = (w as f64 * scale) as u32;
    let nh = (h as f64 * scale) as u32;
    log::warn!(
        "Image {}x{} exceeds GPU max {}; downscaling to {}x{}",
        w, h, MAX_TEX, nw, nh
    );
    img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
}

/// 将 DynamicImage 转换为 Rgb16Image（16-bit RGB）。
/// 如果输入已是 ImageRgb16 则零拷贝提取；否则从 8-bit 上移。
pub(super) fn to_rgb16(img: &image::DynamicImage) -> Rgb16Image {
    match img {
        image::DynamicImage::ImageRgb16(rgb16) => rgb16.clone(),
        _ => {
            let rgb8 = img.to_rgb8();
            let (w, h) = rgb8.dimensions();
            let pixels: Vec<u16> = rgb8.as_raw().iter().map(|&v| (v as u16) << 8 | v as u16).collect();
            Rgb16Image::from_raw(w, h, pixels).expect("to_rgb16: buffer size mismatch")
        }
    }
}

/// 从 16-bit RGB 图像创建 egui 显示纹理（>>8 转 8-bit）
pub(super) fn texture_from_16bit(rgb16: &Rgb16Image, ctx: &egui::Context) -> egui::TextureHandle {
    use rayon::prelude::*;
    let (w, h) = (rgb16.width() as usize, rgb16.height() as usize);
    let src = rgb16.as_raw();
    let row_len = w * 3;
    // 并行 >>8 转换 + 添加 alpha 通道
    let mut rgba = vec![255u8; w * h * 4];
    rgba.par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row_dst)| {
            let row_start = y * row_len;
            for x in 0..w {
                let si = row_start + x * 3;
                let di = x * 4;
                row_dst[di]     = (src[si]     >> 8) as u8;
                row_dst[di + 1] = (src[si + 1] >> 8) as u8;
                row_dst[di + 2] = (src[si + 2] >> 8) as u8;
                // alpha = 255 (already set)
            }
        });
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    ctx.load_texture("loupe_preview", color_image, egui::TextureOptions::LINEAR)
}

