//! 应用核心逻辑模块
//!
//! 实现应用初始化、目录切换、文件选择、后台任务轮询、
//! 拖放处理及主界面布局（工具栏、左侧目录树、右侧面板、中央视图）。

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use super::types::*;
use super::helpers::*;

use eframe::egui;
use std::path::PathBuf;

use fff_viewer::color::{self, TargetColorSpace};
use fff_viewer::config::{self, AppConfig};
use fff_viewer::flexcolor::{EditHistory, ImageCorrection};
use fff_viewer::i18n::{self, Language, Strings};
use fff_viewer::sidecar;
use fff_viewer::tiff::TiffFile;

// ─── 应用实现 ───────────────────────────────────────────────────────────────

impl FffViewerApp {
    /// 创建应用实例，初始化字体、扫描资源目录、设置初始目录
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>, app_config: AppConfig) -> Self {
        log::info!("Initializing FffViewerApp");
        setup_cjk_fonts(&cc.egui_ctx);

        let language = Language::from_config(&app_config.language);

        // Scan for bundled ICC profiles and settings presets
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let profiles_dir = find_resource_dir("profiles", exe_dir.as_deref());
        let settings_dir = find_resource_dir("settings", exe_dir.as_deref());

        let available_profiles = profiles_dir
            .map(|d| color::scan_icc_profiles(&d))
            .unwrap_or_default();
        let available_presets = settings_dir
            .map(|d| color::scan_settings_presets(&d))
            .unwrap_or_default();
        log::info!(
            "Found {} ICC profiles, {} settings presets",
            available_profiles.len(),
            available_presets.len()
        );

        let (thumb_tx, thumb_rx) = mpsc::channel();
        let (detail_tx, detail_rx) = mpsc::channel();

        let favorites: Vec<PathBuf> = app_config
            .favorites
            .iter()
            .map(|s| PathBuf::from(s))
            .collect();

        let mut app = Self {
            current_dir: None,
            expanded_dirs: HashSet::new(),
            favorites,
            fff_files: Vec::new(),
            thumbnails: HashMap::new(),
            thumb_rx,
            thumb_tx,
            thumb_pending: 0,
            view_mode: ViewMode::Grid,
            selected_index: None,
            detail: None,
            detail_rx,
            detail_tx,
            info_panel: InfoPanel::Metadata,
            manual_adjust: color::ManualAdjust::default(),
            histogram_raw: None,
            histogram_raw_16: None,
            histogram_processed: None,
            histogram_needs_update: false,
            levels_processed: HistogramLevels::default(),
            levels_raw: HistogramLevels::default(),
            tag_filter: String::new(),
            expanded_setting: None,
            file_filter: String::new(),
            export_state: ExportState::default(),
            available_profiles,
            available_presets,
            selected_input_profile: None,
            selected_preset: None,
            use_embedded_icc: false,
            use_embedded_correction: false,
            embedded_correction_index: None,
            preset_category_filter: String::new(),
            color_status: None,
            target_color_space: TargetColorSpace::default(),
            baseline_adjust: color::ManualAdjust::default(),
            baseline_levels_processed: HistogramLevels::default(),
            baseline_levels_raw: HistogramLevels::default(),
            split_state: SplitState::default(),
            loading_status: LoadingStatus::Idle,
            error_msg: None,
            show_info_panel: true,
            language,
            app_config,
            settings_needs_restart: false,
        };

        if let Some(path) = initial_file {
            if let Some(parent) = path.parent() {
                app.set_directory(parent.to_path_buf());
                if let Some(idx) = app.fff_files.iter().position(|p| p == &path) {
                    app.selected_index = Some(idx);
                }
            }
        } else if let Some(home) = dirs_home() {
            app.set_directory(home);
        }

        app
    }

    /// 获取当前语言的国际化字符串
    pub(super) fn s(&self) -> &'static Strings {
        i18n::strings(self.language)
    }

    /// 切换当前目录：展开祖先节点、扫描文件、启动缩略图加载线程
    pub(super) fn set_directory(&mut self, dir: PathBuf) {
        log::info!("set_directory: {}", dir.display());
        self.current_dir = Some(dir.clone());

        // Find the most specific tree root that contains `dir` (longest prefix).
        // Only expand ancestors between `dir` and that root — never go above it,
        // so we don't accidentally expand e.g. "/" when home dir is also a root.
        let roots = get_root_dirs();
        let containing_root = roots
            .iter()
            .filter(|r| dir.starts_with(r.as_path()))
            .max_by_key(|r| r.components().count())
            .cloned();

        if let Some(ref root) = containing_root {
            let mut ancestor = dir.parent().map(|p| p.to_path_buf());
            while let Some(ref p) = ancestor {
                if p.starts_with(root.as_path()) {
                    self.expanded_dirs.insert(p.clone());
                    if p == root {
                        break; // reached containing root — stop
                    }
                    ancestor = p.parent().map(|q| q.to_path_buf());
                } else {
                    break; // above the containing root — don't expand
                }
            }
        }
        // Also expand the directory itself so its children are visible
        self.expanded_dirs.insert(dir.clone());
        let depth = self.dir_scan_depth(&dir);
        self.fff_files = scan_fff_files(&dir, depth);
        self.fff_files.sort();
        log::info!("Found {} .fff files", self.fff_files.len());
        self.selected_index = None;
        self.detail = None;
        self.thumbnails.clear();
        self.thumb_pending = self.fff_files.len();
        self.loading_status = if self.fff_files.is_empty() {
            LoadingStatus::Idle
        } else {
            LoadingStatus::LoadingThumbnails
        };

        // Spawn background thread pool to load thumbnails
        let files = self.fff_files.clone();
        let tx = self.thumb_tx.clone();
        std::thread::spawn(move || {
            use rayon::prelude::*;
            files.par_iter().for_each(|path| {
                // 使用轻量级方法加载缩略图，仅读取 IFD 元数据 + 缩略图像素，
                // 跳过全分辨率数据（例如 97MB 文件仅需读 ~1.3MB）
                let result = match TiffFile::open_for_thumbnail(path) {
                    Ok(Some(img)) => {
                        let w = img.width();
                        let h = img.height();
                        let rgba = img.to_rgba8().into_raw();
                        ThumbResult { path: path.clone(), rgba, width: w, height: h }
                    }
                    _ => {
                        ThumbResult { path: path.clone(), rgba: Vec::new(), width: 0, height: 0 }
                    }
                };
                let _ = tx.send(result);
            });
        });
    }

    /// 选择指定索引的文件，启动后台线程加载详情
    pub(super) fn select_file(&mut self, index: usize, _ctx: &egui::Context) {
        if index >= self.fff_files.len() {
            log::warn!("select_file: index {} out of range ({})", index, self.fff_files.len());
            return;
        }
        self.selected_index = Some(index);
        self.expanded_setting = None;
        self.error_msg = None;
        self.use_embedded_correction = false;
        self.embedded_correction_index = None;
        self.use_embedded_icc = false;
        self.color_status = None;

        let path = self.fff_files[index].clone();
        log::info!("select_file: [{}] {}", index, path.display());

        if let Some(detail) = &self.detail {
            if detail.path == path {
                return;
            }
        }

        // Show loading state immediately
        let file_name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.loading_status = LoadingStatus::LoadingFile(file_name);
        self.detail = None;

        // Spawn background thread for file I/O and parsing
        let tx = self.detail_tx.clone();
        std::thread::spawn(move || {
            match TiffFile::open(&path) {
                Ok(tiff) => {
                    log::info!("Opened file: {} ({} bytes)",
                        path.display(), tiff.raw_data().len());
                    let metadata = tiff.metadata_summary();
                    let all_tags = tiff.all_tags();
                    log::debug!("Parsed {} metadata entries, {} tags", metadata.len(), all_tags.len());
                    // 利用 tag 0xC519 精确定位 XML，避免全文件扫描
                    let edit_history = EditHistory::parse_from_tiff(&tiff);
                    log::debug!("Edit history: {} settings",
                        edit_history.as_ref().map(|h| h.settings.len()).unwrap_or(0));

                    // Extract the embedded correction from edit history (if any)
                    let embedded_correction: Option<ImageCorrection> = edit_history.as_ref().and_then(|h| {
                        if h.settings.is_empty() {
                            None
                        } else {
                            let idx = h.current_index.min(h.settings.len() - 1);
                            Some(h.settings[idx].correction.clone())
                        }
                    });

                    // Decode preview with downsampling for fast display
                    let (preview_16, raw_preview_16, auto_corrected) = if let Some(img) = tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
                        log::info!("Decoded downscaled preview: {}x{} {:?}",
                            img.width(), img.height(), img.color());

                        // 保存未经色彩处理的 16-bit 原始图像（用于原始直方图）
                        let raw_16 = img.clone();

                        // Auto-apply embedded correction if available
                        let (processed, corrected) = if let Some(ref correction) = embedded_correction {
                            log::info!("Auto-applying embedded correction: film_type={}",
                                correction.film_type);
                            let result = color::apply_film_processing(&img, correction);
                            (result, true)
                        } else {
                            (img, false)
                        };

                        (Some(processed), Some(raw_16), corrected)
                    } else {
                        log::warn!("No preview decoded for {}", path.display());
                        (None, None, false)
                    };

                    let embedded_icc = color::extract_embedded_icc(tiff.raw_data(), &all_tags);
                    log::debug!("Embedded ICC: {} bytes",
                        embedded_icc.as_ref().map(|d| d.len()).unwrap_or(0));

                    // Load sidecar XML if it exists
                    let sidecar_config = sidecar::load(&path);
                    if sidecar_config.is_some() {
                        log::info!("Loaded sidecar: {}", sidecar::sidecar_path(&path).display());
                    }

                    let _ = tx.send(DetailMsg::Loaded(DetailResult {
                        path,
                        tiff,
                        metadata,
                        all_tags,
                        edit_history,
                        preview_16,
                        raw_preview_16,
                        embedded_icc,
                        auto_corrected,
                        sidecar: sidecar_config,
                    }));
                }
                Err(e) => {
                    log::error!("Failed to open {}: {}", path.display(), e);
                    let _ = tx.send(DetailMsg::Error(path, e.to_string()));
                }
            }
        });
    }

    /// 轮询后台线程的缩略图和文件详情加载结果
    pub(super) fn poll_background_results(&mut self, ctx: &egui::Context) {
        // Poll thumbnail results
        while let Ok(result) = self.thumb_rx.try_recv() {
            self.thumb_pending = self.thumb_pending.saturating_sub(1);
            if result.width > 0 && result.height > 0 {
                let size = [result.width as usize, result.height as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &result.rgba);
                let tex = ctx.load_texture(
                    format!("thumb_{}", result.path.display()),
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                self.thumbnails.insert(result.path, ThumbEntry {
                    texture: tex,
                    width: result.width,
                    height: result.height,
                });
            }
        }
        if self.thumb_pending == 0 && matches!(self.loading_status, LoadingStatus::LoadingThumbnails) {
            self.loading_status = LoadingStatus::Idle;
        }

        // Poll detail file result
        if let Ok(msg) = self.detail_rx.try_recv() {
            match msg {
                DetailMsg::Loaded(result) => {
                    let has_sidecar = result.sidecar.is_some();
                    let sidecar = result.sidecar.clone();

                    // 从 16-bit 图像提取 base_rgb(16-bit) 并创建 8-bit 显示纹理
                    let (texture, base_rgb) = if let Some(ref img16) = result.preview_16 {
                        let rgb16 = to_rgb16(img16);
                        let tex = texture_from_16bit(&rgb16, ctx);
                        (Some(tex), Some(rgb16))
                    } else {
                        (None, None)
                    };

                    // raw_rgb 也转为 Rgb16Image
                    let raw_rgb = result.raw_preview_16.as_ref().map(|img| to_rgb16(img));

                    self.detail = Some(LoadedDetail {
                        path: result.path,
                        tiff: result.tiff,
                        metadata: result.metadata,
                        all_tags: result.all_tags,
                        edit_history: result.edit_history,
                        texture,
                        embedded_icc: result.embedded_icc,
                        base_rgb,
                        icc_rgb: None,
                        raw_rgb,
                    });

                    if has_sidecar {
                        // Restore settings from sidecar
                        self.apply_sidecar(sidecar.as_ref().unwrap(), ctx);
                    } else if result.auto_corrected {
                        // 内嵌校正已在后台线程预览中应用，这里需要重新通过完整管线处理
                        // 以正确提取色阶/饱和度等参数到 UI 手柄
                        self.use_embedded_correction = true;
                        // 自动选中当前编辑历史索引
                        if let Some(ref detail) = self.detail {
                            if let Some(ref history) = detail.edit_history {
                                if !history.settings.is_empty() {
                                    self.embedded_correction_index = Some(
                                        history.current_index.min(history.settings.len() - 1)
                                    );
                                }
                            }
                        }
                        self.apply_color_profile(ctx);
                    } else {
                        self.histogram_needs_update = true;
                    }
                    self.loading_status = LoadingStatus::Idle;
                }
                DetailMsg::Error(path, e) => {
                    self.error_msg = Some(format!("{}: {}", self.s().failed_to_open, e));
                    self.detail = None;
                    self.loading_status = LoadingStatus::Idle;
                    log::error!("Background load failed for {}: {}", path.display(), e);
                }
            }
        }

        if self.histogram_needs_update {
            self.compute_histogram();
        }

        // Request repaint while background work is in progress
        if self.thumb_pending > 0
            || matches!(self.loading_status, LoadingStatus::LoadingFile(_))
        {
            ctx.request_repaint();
        }
    }

    /// 打开目录选择对话框
    pub(super) fn open_directory_dialog(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.set_directory(dir);
        }
    }
}

// ─── eframe::App 实现 ───────────────────────────────────────────────────────

impl eframe::App for FffViewerApp {
    /// 主界面更新：处理拖放、渲染工具栏、目录树、信息面板和中央视图
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background_results(ctx);

        // Handle drag-and-drop
        let dropped_path = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
        if let Some(path) = dropped_path {
            log::info!("Drag-and-drop: {}", path.display());
            if path.is_dir() {
                self.set_directory(path);
            } else if let Some(parent) = path.parent() {
                self.set_directory(parent.to_path_buf());
                if let Some(idx) = self.fff_files.iter().position(|p| p == &path) {
                    self.select_file(idx, ctx);
                }
            }
        }

        let s = self.s();

        // ── Top toolbar ─────────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(s.open_folder).clicked() {
                    self.open_directory_dialog();
                }

                ui.separator();

                ui.selectable_value(&mut self.view_mode, ViewMode::Grid, s.grid);
                ui.selectable_value(&mut self.view_mode, ViewMode::Loupe, s.loupe);

                ui.separator();

                ui.toggle_value(&mut self.show_info_panel, s.info);

                if self.show_info_panel {
                    ui.separator();
                    ui.selectable_value(&mut self.info_panel, InfoPanel::Metadata, s.metadata);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::EditHistory, s.history);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::AllTags, s.tags);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::ColorAdjust, s.color_adjust);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::ColorProfile, s.color_profile);
                    ui.selectable_value(&mut self.info_panel, InfoPanel::Split, s.split_export);
                    ui.separator();
                    ui.selectable_value(&mut self.info_panel, InfoPanel::Settings, s.settings);
                }

                ui.separator();

                // Export buttons
                let has_selection = self.selected_index.is_some() && self.detail.is_some();
                let has_files = !self.fff_files.is_empty();

                if ui.add_enabled(has_selection, egui::Button::new(s.export_current)).clicked() {
                    self.export_current_file();
                }
                if ui.add_enabled(has_files, egui::Button::new(s.export_all)).clicked() {
                    self.export_all_files();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Language selector
                    let old_lang = self.language;
                    egui::ComboBox::from_id_salt("lang_selector")
                        .selected_text(format!("{} {}", s.language, self.language.label()))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.language,
                                Language::English,
                                Language::English.label(),
                            );
                            ui.selectable_value(
                                &mut self.language,
                                Language::Chinese,
                                Language::Chinese.label(),
                            );
                        });
                    if self.language != old_lang {
                        self.app_config.language = self.language.to_config().to_string();
                        let _ = config::save(&self.app_config);
                    }

                    ui.separator();

                    if let Some(dir) = &self.current_dir {
                        let s2 = i18n::strings(self.language);
                        let label = format!(
                            "{} — {} {}",
                            dir.display(),
                            self.fff_files.len(),
                            s2.files_count
                        );
                        ui.label(
                            egui::RichText::new(label).small().color(ui.visuals().weak_text_color()),
                        );
                    }
                });
            });
        });

        // ── Progress bar (shown when loading) ───────────────────────────
        {
            let total = self.fff_files.len();
            let loaded = self.thumbnails.len();
            let thumbs_loading = self.thumb_pending > 0 && total > 0;

            let show_bar = thumbs_loading
                || matches!(self.loading_status,
                    LoadingStatus::LoadingFile(_)
                    | LoadingStatus::ApplyingColorProfile
                );

            if show_bar {
                egui::TopBottomPanel::top("progress_bar").show(ctx, |ui| {
                    match &self.loading_status {
                        LoadingStatus::LoadingFile(name) => {
                            ui.add(
                                egui::ProgressBar::new(0.0)
                                    .animate(true)
                                    .text(format!("⏳ {} {}…", self.s().loading_file, name)),
                            );
                        }
                        LoadingStatus::ApplyingColorProfile => {
                            ui.add(
                                egui::ProgressBar::new(0.0)
                                    .animate(true)
                                    .text("⏳ Applying color profile…"),
                            );
                        }
                        _ if thumbs_loading => {
                            let progress = loaded as f32 / total as f32;
                            let s2 = i18n::strings(self.language);
                            let text = format!(
                                "📷 {} {}/{}",
                                s2.loading_thumbnails, loaded, total
                            );
                            ui.add(egui::ProgressBar::new(progress).text(text));
                        }
                        _ => {}
                    }
                });
            }
        }

        // ── Left panel: favorites + directory tree ──────────────────────
        let s = i18n::strings(self.language);
        let favorites_heading = s.favorites.to_string();
        let folders_label = s.folders.to_string();
        egui::SidePanel::left("dir_tree_panel")
            .default_width(220.0)
            .min_width(160.0)
            .show(ctx, |ui| {
                // ── Favorites section ────────────────────────────────────
                ui.heading(&favorites_heading);
                ui.separator();
                let fav_height = if self.favorites.is_empty() { 28.0 } else { (self.favorites.len() as f32 * 22.0).min(160.0) };
                egui::ScrollArea::vertical()
                    .id_salt("favs_scroll")
                    .max_height(fav_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        self.render_favorites(ui);
                    });
                ui.add_space(4.0);
                // ── Folders section ──────────────────────────────────────
                ui.heading(&folders_label);
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("tree_scroll")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        self.render_dir_tree(ui);
                    });
            });

        // ── Right panel: info (optional) ────────────────────────────────
        if self.show_info_panel {
            egui::SidePanel::right("info_panel")
                .default_width(340.0)
                .min_width(260.0)
                .max_width(360.0)
                .show(ctx, |ui| match self.info_panel {
                    InfoPanel::Metadata => self.render_metadata_panel(ui),
                    InfoPanel::EditHistory => self.render_edit_history_panel(ui),
                    InfoPanel::AllTags => self.render_all_tags_panel(ui),
                    InfoPanel::ColorAdjust => self.render_color_adjust_panel(ui, ctx),
                    InfoPanel::ColorProfile => self.render_color_profile_panel(ui, ctx),
                    InfoPanel::Split => self.render_split_panel(ui, ctx),
                    InfoPanel::Settings => self.render_settings_panel(ui),
                });
        }

        // ── Bottom panel: export status ─────────────────────────────────
        let export_status = self.export_state.status.clone();
        match export_status {
            ExportStatus::Exporting { current, total, ref current_name } => {
                let s = i18n::strings(self.language);
                let label_text = format!("{} {}/{} — {}", s.exporting, current, total, current_name);
                let progress = current as f32 / total as f32;
                egui::TopBottomPanel::bottom("export_status").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(label_text)
                                .color(ui.visuals().hyperlink_color),
                        );
                    });
                    ui.add(egui::ProgressBar::new(progress).show_percentage());
                });
                ctx.request_repaint();
            }
            ExportStatus::Done { count, ref dir } => {
                let s = i18n::strings(self.language);
                let label_text = format!("✅ {} {} → {}", s.export_done, count, dir.display());
                egui::TopBottomPanel::bottom("export_status").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(label_text)
                                .color(if ui.visuals().dark_mode {
                                    egui::Color32::from_rgb(100, 255, 100)
                                } else {
                                    egui::Color32::from_rgb(0, 140, 0)
                                }),
                        );
                        if ui.small_button("✕").clicked() {
                            self.export_state.status = ExportStatus::Idle;
                        }
                    });
                });
            }
            ExportStatus::Error(ref msg) => {
                let label_text = format!("❌ {}", msg);
                egui::TopBottomPanel::bottom("export_status").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(label_text)
                                .color(if ui.visuals().dark_mode {
                                    egui::Color32::from_rgb(255, 100, 100)
                                } else {
                                    egui::Color32::from_rgb(200, 0, 0)
                                }),
                        );
                        if ui.small_button("✕").clicked() {
                            self.export_state.status = ExportStatus::Idle;
                        }
                    });
                });
            }
            ExportStatus::Idle => {}
        }

        // ── Central panel: grid or loupe ────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.fff_files.is_empty() {
                self.render_empty_state(ui);
            } else {
                self.render_file_filter_bar(ui);
                ui.separator();
                match self.view_mode {
                    ViewMode::Grid => self.render_grid_view(ui, ctx),
                    ViewMode::Loupe => self.render_loupe_view(ui, ctx),
                }
            }
        });
    }
}
