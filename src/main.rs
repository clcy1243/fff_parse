mod viewer;

use std::io::Write;

fn setup_logging() {
    let log_path = dirs_home()
        .map(|h| h.join("Library/Logs/fff_viewer.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("fff_viewer.log"));

    // Rotate: keep previous log as .log.old
    let old_path = log_path.with_extension("log.old");
    let _ = std::fs::rename(&log_path, &old_path);

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
            // Write to the log file
            if let Ok(mut f) = file.lock() {
                let _ = writeln!(f, "{}", line);
                let _ = f.flush();
            }
            writeln!(buf, "{}", line)
        })
        .init();

    log::info!("=== FFF Viewer started === (log: {})", log_path.display());
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

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

fn main() {
    setup_logging();

    let initial_file = std::env::args().nth(1).map(std::path::PathBuf::from);
    log::info!("Initial file: {:?}", initial_file);

    let icon = load_app_icon();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1400.0, 900.0])
        .with_drag_and_drop(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    if let Err(e) = eframe::run_native(
        "FFF Viewer — Flextight X5",
        native_options,
        Box::new(move |cc| Ok(Box::new(viewer::FffViewerApp::new(cc, initial_file)))),
    ) {
        log::error!("eframe exited with error: {}", e);
    }

    log::info!("=== FFF Viewer exited ===");
}
