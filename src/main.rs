//! FFF Viewer 应用程序入口，基于 egui/eframe 构建的 Flextight X5 扫描仪文件查看器。

// Windows: 隐藏控制台窗口（双击运行 .exe 时不弹出黑色终端）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod viewer;

use std::io::Write;

use fff_viewer::config;

/// 初始化日志系统，创建带时间戳的日志文件并清理过期日志。
fn setup_logging() {
    let log_dir = config::logs_dir();
    config::ensure_dirs();

    // Create timestamped log file: fff_viewer_20260318_041200.log
    let now = chrono::Local::now();
    let filename = now.format("fff_viewer_%Y%m%d_%H%M%S.log").to_string();
    let log_path = log_dir.join(&filename);

    // Clean up logs older than 3 days
    cleanup_old_logs(&log_dir, 3);

    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .expect("Failed to open log file");

    let file = std::sync::Mutex::new(file);

    // Install panic hook that writes to the log file before aborting
    let panic_log_path = log_path.clone();
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!(
            "[PANIC] {}\nBacktrace:\n{:?}\n",
            info,
            std::backtrace::Backtrace::force_capture()
        );
        eprintln!("{}", msg);
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&panic_log_path) {
            let _ = writeln!(f, "{}", msg);
        }
    }));

    // Build env_logger writing to both stderr and the log file
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .format(move |buf, record| {
            let ts = buf.timestamp_millis();
            let line = format!(
                "[{} {} {}:{}] {}",
                ts,
                record.level(),
                record.file().unwrap_or("?"),
                record.line().unwrap_or(0),
                record.args()
            );
            if let Ok(mut f) = file.lock() {
                let _ = writeln!(f, "{}", line);
                let _ = f.flush();
            }
            writeln!(buf, "{}", line)
        })
        .init();

    log::info!("=== FFF Viewer started === (log: {})", log_path.display());
}

/// 从 `dir` 目录中删除超过 `days` 天的 fff_viewer_*.log 日志文件。
fn cleanup_old_logs(dir: &std::path::Path, days: u64) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(days * 24 * 60 * 60);

    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.starts_with("fff_viewer_") || !name.ends_with(".log") {
            continue;
        }
        if let Ok(meta) = path.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

/// 从内嵌的 PNG 资源加载应用程序图标。
fn load_app_icon() -> Option<egui::IconData> {
    let png_bytes = include_bytes!("../icons/icon_256.png");
    let img = image::load_from_memory(png_bytes).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

/// 应用程序主函数，初始化日志、加载配置、设置窗口并启动 eframe 图形界面。
fn main() {
    setup_logging();

    let initial_file = std::env::args().nth(1).map(std::path::PathBuf::from);
    log::info!("Initial file: {:?}", initial_file);
    log::info!("Data dir: {}", config::app_data_dir().display());

    // Load global configuration (creates default on first launch)
    let app_config = config::load_or_create();
    log::info!(
        "Config: gpu={}, device={:?}, threads={}, lang={}",
        app_config.gpu_enabled,
        app_config.gpu_device,
        app_config.render_threads,
        app_config.language
    );

    // Configure rayon global thread pool
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(app_config.render_threads)
        .build_global();

    let icon = load_app_icon();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1400.0, 900.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let hardware_accel = if app_config.gpu_enabled {
        eframe::HardwareAcceleration::Preferred
    } else {
        eframe::HardwareAcceleration::Off
    };

    let native_options = eframe::NativeOptions {
        viewport,
        hardware_acceleration: hardware_accel,
        ..Default::default()
    };

    let config_clone = app_config.clone();
    if let Err(e) = eframe::run_native(
        "FFF Viewer — Flextight X5",
        native_options,
        Box::new(move |cc| Ok(Box::new(viewer::FffViewerApp::new(cc, initial_file, config_clone)))),
    ) {
        log::error!("eframe exited with error: {}", e);
    }

    log::info!("=== FFF Viewer exited ===");
}
