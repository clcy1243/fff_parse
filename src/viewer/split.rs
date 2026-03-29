//! 底片分割与导出模块
//!
//! 实现底片分割功能：分割面板 UI、区域交互（移动/缩放/旋转）、
//! 分割区域叠加绘制、裁切导出及双线性插值采样。

use super::types::*;

use eframe::egui;
use std::path::Path;

use fff_viewer::color;
use fff_viewer::flexcolor;
use fff_viewer::tiff::TiffFile;

impl FffViewerApp {

    // ── 导出 ─────────────────────────────────────────────────────────

    /// 导出当前选中的单个文件为标准 TIFF（应用当前色彩调整）
    pub(super) fn export_current_file(&mut self) {
        let Some(detail) = &self.detail else { return };
        let src_path = detail.path.clone();
        log::info!("Export single: {}", src_path.display());
        let default_name = src_path
            .file_stem()
            .map(|n| format!("{}.tiff", n.to_string_lossy()))
            .unwrap_or_else(|| "export.tiff".to_string());

        let pipeline = self.build_export_pipeline();

        if let Some(save_path) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .add_filter("TIFF", &["tiff", "tif"])
            .save_file()
        {
            log::info!("Exporting to: {}", save_path.display());
            match Self::export_fff_to_tiff(&src_path, &save_path, &pipeline) {
                Ok(()) => {
                    log::info!("Export OK: {}", save_path.display());
                    self.export_state.status = ExportStatus::Done {
                        count: 1,
                        dir: save_path.parent().unwrap_or(Path::new(".")).to_path_buf(),
                    };
                }
                Err(e) => {
                    log::error!("Export failed: {} — {}", src_path.display(), e);
                    self.export_state.status = ExportStatus::Error(format!("{}: {}", src_path.display(), e));
                }
            }
        }
    }

    /// 批量导出当前目录下所有文件为标准 TIFF（应用当前色彩调整）
    pub(super) fn export_all_files(&mut self) {
        if self.fff_files.is_empty() {
            return;
        }

        let Some(out_dir) = rfd::FileDialog::new()
            .set_title("Select output directory")
            .pick_folder()
        else {
            return;
        };

        let pipeline = self.build_export_pipeline();
        let files = self.fff_files.clone();
        let total = files.len();
        log::info!("Export all: {} files → {}", total, out_dir.display());

        for (i, src_path) in files.iter().enumerate() {
            let name = src_path
                .file_stem()
                .map(|n| format!("{}.tiff", n.to_string_lossy()))
                .unwrap_or_else(|| format!("export_{}.tiff", i));

            self.export_state.status = ExportStatus::Exporting {
                current: i + 1,
                total,
                current_name: name.clone(),
            };

            let dst = out_dir.join(&name);
            log::info!("Batch export [{}/{}]: {} → {}", i + 1, total, src_path.display(), dst.display());
            if let Err(e) = Self::export_fff_to_tiff(&src_path, &dst, &pipeline) {
                log::error!("Batch export failed at [{}/{}]: {}", i + 1, total, e);
                self.export_state.status = ExportStatus::Error(format!("{}: {}", src_path.display(), e));
                return;
            }
        }

        log::info!("Batch export complete: {} files", total);
        self.export_state.status = ExportStatus::Done {
            count: total,
            dir: out_dir,
        };
    }

    /// 构建当前色彩处理管线参数，供导出使用
    fn build_export_pipeline(&self) -> ExportPipeline {
        // 胶片处理校正参数
        let correction = if self.use_embedded_correction {
            // 使用嵌入的编辑历史校正
            self.detail.as_ref().and_then(|d| {
                d.edit_history.as_ref().and_then(|h| {
                    if h.settings.is_empty() {
                        None
                    } else {
                        let idx = h.current_index.min(h.settings.len() - 1);
                        Some(h.settings[idx].correction.clone())
                    }
                })
            })
        } else {
            self.selected_preset.and_then(|idx| {
                self.available_presets.get(idx).and_then(|p| {
                    std::fs::read_to_string(&p.path)
                        .ok()
                        .and_then(|xml| flexcolor::parse_settings_xml(&xml))
                })
            })
        };

        // ICC 配置文件数据
        let icc_data = if self.use_embedded_icc {
            self.detail
                .as_ref()
                .and_then(|d| d.embedded_icc.clone())
        } else if let Some(idx) = self.selected_input_profile {
            self.available_profiles
                .get(idx)
                .and_then(|p| std::fs::read(&p.path).ok())
        } else {
            None
        };

        ExportPipeline {
            correction,
            icc_data,
            target_color_space: self.target_color_space,
            manual_adjust: self.manual_adjust.clone(),
            film_lut: self.extracted_film_lut.clone(),
            curve_points: self.curve_points.clone(),
        }
    }

    /// 将单个 FFF 文件导出为标准 TIFF。
    /// 解码全分辨率 16-bit 图像，应用与预览相同的色彩处理管线，写入新文件。
    pub(super) fn export_fff_to_tiff(
        src: &Path,
        dst: &Path,
        pipeline: &ExportPipeline,
    ) -> Result<(), String> {
        // Guard: never overwrite the source file
        if let (Ok(src_canon), Ok(dst_canon)) = (src.canonicalize(), dst.canonicalize()) {
            if src_canon == dst_canon {
                return Err("Cannot overwrite source file".to_string());
            }
        }

        log::info!("export_fff_to_tiff: {} → {}", src.display(), dst.display());
        let tiff = TiffFile::open(src).map_err(|e| {
            log::error!("export: failed to open {}: {}", src.display(), e);
            e.to_string()
        })?;

        let mut img = tiff
            .decode_for_export()
            .ok_or_else(|| {
                log::error!("export: failed to decode {}", src.display());
                "Failed to decode image data".to_string()
            })?;

        log::info!("export: decoded {}x{} {:?}", img.width(), img.height(), img.color());

        // 应用与预览相同的色彩处理管线（全分辨率）

        // 1. 胶片处理（负片反转）— 使用实际校正参数（与加载渲染一致）
        if let Some(ref correction) = pipeline.correction {
            log::info!("export: applying film processing (film_type={})", correction.film_type);
            img = color::apply_film_processing(&img, correction);
        }

        // 2-4. 统一色彩管线（渐变曲线 → 色阶 → ICC → 显示调整）
        img = color::apply_color_pipeline(
            img,
            &pipeline.manual_adjust,
            &pipeline.curve_points,
            pipeline.film_lut.as_ref(),
            pipeline.icc_data.as_deref(),
            pipeline.target_color_space,
        );

        img.save(dst).map_err(|e| {
            log::error!("export: failed to save {}: {}", dst.display(), e);
            e.to_string()
        })?;

        log::info!("export: saved {}", dst.display());
        Ok(())
    }

    // ── 分割与导出 ─────────────────────────────────────────────────

    /// 渲染分割面板：画幅选择、区域列表、命名模式和导出按钮
    pub(super) fn render_split_panel(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let s = self.s();
        ui.heading(s.split_export);
        ui.separator();

        // Film format selector
        ui.strong(s.film_format);
        ui.add_space(2.0);
        let old_format = self.split_state.format;
        egui::ComboBox::from_id_salt("film_format_combo")
            .selected_text(self.split_state.format.label())
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                for &fmt in FilmFormat::ALL {
                    ui.selectable_value(&mut self.split_state.format, fmt, fmt.label());
                }
            });

        // Orientation toggle (only for non-free, non-square formats)
        let old_portrait = self.split_state.portrait;
        if let Some(ratio) = self.split_state.format.ratio() {
            if (ratio - 1.0).abs() > 0.01 {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(s.orientation_label);
                    ui.selectable_value(&mut self.split_state.portrait, false, s.landscape_label);
                    ui.selectable_value(&mut self.split_state.portrait, true, s.portrait_label);
                });
            }
        }
        if self.split_state.format != old_format || self.split_state.portrait != old_portrait {
            self.save_sidecar();
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Add / Clear buttons
        let has_detail = self.detail.is_some();
        ui.horizontal(|ui| {
            if ui.add_enabled(has_detail, egui::Button::new(s.add_region)).clicked() {
                self.add_split_region();
                self.save_sidecar();
            }
            if ui.add_enabled(!self.split_state.regions.is_empty(), egui::Button::new(s.clear_all)).clicked() {
                self.split_state.regions.clear();
                self.split_state.dragging = None;
                self.split_state.selected = None;
                self.save_sidecar();
            }
        });

        ui.add_space(8.0);

        // Region list
        if self.split_state.regions.is_empty() {
            ui.label(
                egui::RichText::new(s.no_regions)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        } else {
            let mut remove_idx = None;
            let n_regions = self.split_state.regions.len();
            for i in 0..n_regions {
                let color = REGION_COLORS[i % REGION_COLORS.len()];
                let is_active = self.split_state.selected == Some(i);

                ui.horizontal(|ui| {
                    // Color swatch
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, color);

                    let label = format!("{} #{}", s.region_label, i + 1);
                    let rt = if is_active {
                        egui::RichText::new(&label).strong()
                    } else {
                        egui::RichText::new(&label)
                    };
                    if ui.selectable_label(is_active, rt).clicked() {
                        self.split_state.selected = Some(i);
                    }

                    // Show dimensions and angle
                    let r = &self.split_state.regions[i];
                    let angle_deg = r.angle.to_degrees();
                    let info = if angle_deg.abs() > 0.1 {
                        format!("{:.0}%×{:.0}% ∠{:.1}°", r.w * 100.0, r.h * 100.0, angle_deg)
                    } else {
                        format!("{:.0}%×{:.0}%", r.w * 100.0, r.h * 100.0)
                    };
                    ui.label(
                        egui::RichText::new(info)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );

                    if ui.small_button("🗑").clicked() {
                        remove_idx = Some(i);
                    }
                });
            }
            if let Some(idx) = remove_idx {
                self.split_state.regions.remove(idx);
                // Fix selected index
                if let Some(sel) = self.split_state.selected {
                    if sel == idx {
                        self.split_state.selected = None;
                    } else if sel > idx {
                        self.split_state.selected = Some(sel - 1);
                    }
                }
                // Fix dragging index
                if let Some((drag_idx, _)) = &self.split_state.dragging {
                    if *drag_idx == idx {
                        self.split_state.dragging = None;
                    } else if *drag_idx > idx {
                        self.split_state.dragging = Some((*drag_idx - 1, DragKind::Move));
                    }
                }
                self.save_sidecar();
            }
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // Naming pattern
        ui.strong(s.naming_pattern);
        ui.add_space(2.0);
        let old_pattern = self.split_state.naming_pattern.clone();
        let resp = ui.text_edit_singleline(&mut self.split_state.naming_pattern);
        if resp.lost_focus() && self.split_state.naming_pattern != old_pattern {
            self.save_sidecar();
        }
        ui.label(
            egui::RichText::new("{name} = filename, {n} = index")
                .small()
                .color(ui.visuals().weak_text_color()),
        );

        // Preview of naming
        if !self.split_state.regions.is_empty() {
            if let Some(detail) = &self.detail {
                let stem = detail.path.file_stem()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file".to_string());
                let preview = self.split_state.naming_pattern
                    .replace("{name}", &stem)
                    .replace("{n}", "1");
                ui.label(
                    egui::RichText::new(format!("→ {}.tif", preview))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            }
        }

        ui.add_space(12.0);

        // Export button
        let can_export = has_detail && !self.split_state.regions.is_empty();
        if ui.add_enabled(can_export, egui::Button::new(s.export_splits)).clicked() {
            self.export_split_regions();
        }

        // Show export status
        if let ExportStatus::Done { count, ref dir } = self.export_state.status {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("✅ {} {} → {}", s.split_exported, count, dir.display()))
                    .color(if ui.visuals().dark_mode {
                        egui::Color32::from_rgb(100, 255, 100)
                    } else {
                        egui::Color32::from_rgb(0, 140, 0)
                    }),
            );
        }
        if let ExportStatus::Error(ref e) = self.export_state.status {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::RED, format!("❌ {}", e));
        }
    }

    /// 添加一个新的分割区域，根据画幅格式计算初始大小
    pub(super) fn add_split_region(&mut self) {
        let effective_ratio = self.split_state.format.ratio().map(|r| {
            if self.split_state.portrait { 1.0 / r } else { r }
        });

        // Default region: centered, ~30% of image height
        let h = 0.3_f32.min(1.0);
        let w = if let Some(ratio) = effective_ratio {
            let img_aspect = self.detail.as_ref()
                .and_then(|d| d.texture.as_ref())
                .map(|t| t.size_vec2().x / t.size_vec2().y)
                .unwrap_or(1.0);
            (h * ratio / img_aspect).min(0.95)
        } else {
            0.3
        };

        let n = self.split_state.regions.len();
        let y_offset = (n as f32 * 0.05) % 0.5;
        let mut region = SplitRegion {
            cx: 0.5,
            cy: (0.1 + y_offset + h / 2.0).min(1.0 - h / 2.0),
            w: w.min(1.0),
            h: h.min(1.0),
            angle: 0.0,
        };
        region.clamp_to_image();
        let new_idx = self.split_state.regions.len();
        self.split_state.selected = Some(new_idx);
        self.split_state.regions.push(region);
    }

    /// 处理分割区域的鼠标交互：悬停光标、拖拽开始/进行/结束
    pub(super) fn handle_split_interactions(&mut self, response: &egui::Response, image_rect: egui::Rect, ctx: &egui::Context) {
        let handle_radius = 10.0_f32;
        let rot_handle_radius = 8.0_f32;

        // --- Hover cursor ---
        if response.hovered() && self.split_state.dragging.is_none() {
            if let Some(mouse_pos) = response.hover_pos() {
                let mut cursor = None;
                'outer: for (_i, region) in self.split_state.regions.iter().enumerate().rev() {
                    let corners = region.corners_screen(image_rect);

                    // Check rotation handle
                    let rot_pos = region.rotation_handle_screen(image_rect);
                    if mouse_pos.distance(rot_pos) <= rot_handle_radius + 4.0 {
                        cursor = Some(egui::CursorIcon::Alias);
                        break;
                    }

                    // Check corner handles [TL, TR, BR, BL]
                    for (ci, corner) in corners.iter().enumerate() {
                        if mouse_pos.distance(*corner) <= handle_radius + 2.0 {
                            cursor = Some(resize_cursor_for_corner(ci, region.angle));
                            break 'outer;
                        }
                    }

                    // Check body
                    if region.contains_screen_point(mouse_pos, image_rect) {
                        cursor = Some(egui::CursorIcon::Grab);
                        break;
                    }
                }
                if let Some(c) = cursor {
                    ctx.set_cursor_icon(c);
                }
            }
        }

        // --- Drag start ---
        if response.drag_started() {
            if let Some(mouse_pos) = response.interact_pointer_pos() {
                let mut found = None;
                for (i, region) in self.split_state.regions.iter().enumerate().rev() {
                    let corners = region.corners_screen(image_rect);

                    // Rotation handle
                    let rot_pos = region.rotation_handle_screen(image_rect);
                    if mouse_pos.distance(rot_pos) <= rot_handle_radius + 4.0 {
                        found = Some((i, DragKind::Rotate));
                        break;
                    }

                    // Corner handles [TL, TR, BR, BL]
                    let corner_kinds = [
                        DragKind::ResizeTopLeft,
                        DragKind::ResizeTopRight,
                        DragKind::ResizeBottomRight,
                        DragKind::ResizeBottomLeft,
                    ];
                    let mut hit_corner = false;
                    for (ci, corner) in corners.iter().enumerate() {
                        if mouse_pos.distance(*corner) <= handle_radius + 2.0 {
                            found = Some((i, corner_kinds[ci]));
                            hit_corner = true;
                            break;
                        }
                    }
                    if hit_corner {
                        break;
                    }

                    // Body
                    if region.contains_screen_point(mouse_pos, image_rect) {
                        found = Some((i, DragKind::Move));
                        break;
                    }
                }
                self.split_state.dragging = found;
                self.split_state.selected = found.map(|(i, _)| i);
            }
        }

        // --- Dragging ---
        if response.dragged() {
            if let Some((idx, kind)) = self.split_state.dragging {
                if idx < self.split_state.regions.len() {
                    let iw = image_rect.width();
                    let ih = image_rect.height();
                    if iw > 0.0 && ih > 0.0 {
                        match kind {
                            DragKind::Move => {
                                let delta = response.drag_delta();
                                let dx = delta.x / iw;
                                let dy = delta.y / ih;
                                let region = &mut self.split_state.regions[idx];
                                region.cx += dx;
                                region.cy += dy;
                                region.clamp_to_image();
                                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                            }
                            DragKind::Rotate => {
                                if let Some(mouse_pos) = response.interact_pointer_pos() {
                                    let cx_s = image_rect.min.x + self.split_state.regions[idx].cx * iw;
                                    let cy_s = image_rect.min.y + self.split_state.regions[idx].cy * ih;
                                    let angle = (mouse_pos.x - cx_s).atan2(-(mouse_pos.y - cy_s));
                                    self.split_state.regions[idx].angle = angle;
                                    self.split_state.regions[idx].clamp_to_image();
                                    ctx.set_cursor_icon(egui::CursorIcon::Alias);
                                }
                            }
                            resize_kind => {
                                let delta = response.drag_delta();
                                let dx = delta.x / iw;
                                let dy = delta.y / ih;
                                let from_left = matches!(resize_kind, DragKind::ResizeTopLeft | DragKind::ResizeBottomLeft);
                                let from_top = matches!(resize_kind, DragKind::ResizeTopLeft | DragKind::ResizeTopRight);
                                self.resize_region(idx, dx, dy, from_left, from_top, image_rect);
                                let ci = match resize_kind {
                                    DragKind::ResizeTopLeft => 0,
                                    DragKind::ResizeTopRight => 1,
                                    DragKind::ResizeBottomRight => 2,
                                    DragKind::ResizeBottomLeft => 3,
                                    _ => 0,
                                };
                                ctx.set_cursor_icon(resize_cursor_for_corner(ci, self.split_state.regions[idx].angle));
                            }
                        }
                    }
                }
            }
        }

        if response.drag_stopped() {
            if self.split_state.dragging.is_some() {
                self.save_sidecar();
            }
            self.split_state.dragging = None;
        }
    }

    /// 调整分割区域大小，支持宽高比锁定和旋转坐标变换
    pub(super) fn resize_region(
        &mut self,
        idx: usize,
        dx: f32,
        dy: f32,
        from_left: bool,
        from_top: bool,
        image_rect: egui::Rect,
    ) {
        let effective_ratio = self.split_state.format.ratio().map(|r| {
            if self.split_state.portrait { 1.0 / r } else { r }
        });
        let img_aspect = image_rect.width() / image_rect.height();

        let region = &mut self.split_state.regions[idx];

        // Transform the screen-space delta into the region's local (rotated) space
        let (sin_a, cos_a) = region.angle.sin_cos();
        let local_dx = dx * cos_a + dy * sin_a;
        let local_dy = -dx * sin_a + dy * cos_a;

        // Determine sign: left/top edges move opposite to right/bottom
        let sx = if from_left { -1.0 } else { 1.0 };
        let dw = sx * local_dx;

        let new_w = (region.w + dw).max(0.01);
        let actual_dw = new_w - region.w;

        // Shift center: when resizing from one side, center moves by half the delta
        // in the region's local coordinate system, then rotated back
        let shift_local_x = actual_dw / 2.0 * sx;
        region.w = new_w;

        if let Some(ratio) = effective_ratio {
            // Constrain aspect ratio: h = w * img_aspect / ratio
            let new_h = region.w * img_aspect / ratio;
            let actual_dh = new_h - region.h;
            let sy = if from_top { -1.0 } else { 1.0 };
            let shift_local_y = actual_dh / 2.0 * sy;
            region.h = new_h;
            // Rotate shift back to image coordinate system
            region.cx += shift_local_x * cos_a - shift_local_y * sin_a;
            region.cy += shift_local_x * sin_a + shift_local_y * cos_a;
        } else {
            let sy = if from_top { -1.0 } else { 1.0 };
            let dh_local = sy * local_dy;
            let new_h = (region.h + dh_local).max(0.01);
            let actual_dh = new_h - region.h;
            let shift_local_y = actual_dh / 2.0 * sy;
            region.h = new_h;
            region.cx += shift_local_x * cos_a - shift_local_y * sin_a;
            region.cy += shift_local_x * sin_a + shift_local_y * cos_a;
        }

        region.clamp_to_image();
    }

    /// 导出所有分割区域为独立的 TIFF 文件（应用当前色彩调整）
    pub(super) fn export_split_regions(&mut self) {
        let s = self.s();
        let Some(detail) = &self.detail else { return };
        if self.split_state.regions.is_empty() {
            return;
        }

        let Some(out_dir) = rfd::FileDialog::new()
            .set_title("Select output directory")
            .pick_folder()
        else {
            return;
        };

        let src_path = detail.path.clone();
        let stem = src_path.file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        // Decode full-resolution image for export
        let mut img = match detail.tiff.decode_for_export() {
            Some(img) => img,
            None => {
                self.export_state.status = ExportStatus::Error("Failed to decode image".into());
                return;
            }
        };

        // 应用色彩处理管线（全分辨率）：胶片处理 → 色阶 → ICC → 显示调整
        let pipeline = self.build_export_pipeline();

        // 1. 胶片处理（负片反转）— 使用实际校正参数（与渲染管线一致）
        if let Some(ref correction) = pipeline.correction {
            img = color::apply_film_processing(&img, correction);
        }

        // 2-4. 统一色彩管线（渐变曲线 → 色阶 → ICC → 显示调整）
        img = color::apply_color_pipeline(
            img,
            &pipeline.manual_adjust,
            &pipeline.curve_points,
            pipeline.film_lut.as_ref(),
            pipeline.icc_data.as_deref(),
            pipeline.target_color_space,
        );

        let img_w = img.width();
        let img_h = img.height();
        let mut exported = 0;

        for (i, region) in self.split_state.regions.iter().enumerate() {
            let name = self.split_state.naming_pattern
                .replace("{name}", &stem)
                .replace("{n}", &(i + 1).to_string());
            let out_path = out_dir.join(format!("{}.tif", name));

            // Guard: never overwrite source
            if let (Ok(src_c), Ok(dst_c)) = (src_path.canonicalize(), out_path.canonicalize()) {
                if src_c == dst_c {
                    self.export_state.status = ExportStatus::Error(s.cannot_overwrite_source.to_string());
                    return;
                }
            }

            let cropped = crop_rotated_region(&img, region, img_w, img_h);

            match cropped.save(&out_path) {
                Ok(()) => {
                    let cw = cropped.width();
                    let ch = cropped.height();
                    log::info!("Split export: {} ({}x{})", out_path.display(), cw, ch);
                    exported += 1;
                }
                Err(e) => {
                    log::error!("Split export failed: {} — {}", out_path.display(), e);
                    self.export_state.status = ExportStatus::Error(format!("{}: {}", out_path.display(), e));
                    return;
                }
            }
        }

        self.export_state.status = ExportStatus::Done {
            count: exported,
            dir: out_dir,
        };
        log::info!("Split export: {} files", exported);
    }


}

// ─── 分割叠加层绘制 ─────────────────────────────────────────────────────────

/// 在图像上绘制所有分割区域的叠加层（边框、角点手柄、旋转手柄、编号标签）
pub(super) fn draw_split_overlays(
    painter: &egui::Painter,
    image_rect: egui::Rect,
    regions: &[SplitRegion],
    selected_idx: Option<usize>,
) {
    let handle_size = 8.0_f32;
    let rot_handle_radius = 6.0_f32;

    for (i, region) in regions.iter().enumerate() {
        let corners = region.corners_screen(image_rect);
        let color = REGION_COLORS[i % REGION_COLORS.len()];
        let is_selected = selected_idx == Some(i);
        let stroke_width = if is_selected { 3.0 } else { 2.0 };

        // Semi-transparent fill (rotated quad)
        let fill = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 20);
        let mut quad_mesh = egui::Mesh::default();
        for c in &corners {
            quad_mesh.colored_vertex(*c, fill);
        }
        quad_mesh.add_triangle(0, 1, 2);
        quad_mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(quad_mesh));

        // Border (4 line segments)
        let stroke = egui::Stroke::new(stroke_width, color);
        for j in 0..4 {
            painter.line_segment([corners[j], corners[(j + 1) % 4]], stroke);
        }

        // Corner handles
        for corner in &corners {
            let handle_rect =
                egui::Rect::from_center_size(*corner, egui::vec2(handle_size, handle_size));
            painter.rect_filled(handle_rect, 2.0, color);
            painter.rect_stroke(
                handle_rect,
                2.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }

        // Rotation handle (circle above top edge)
        let rot_pos = region.rotation_handle_screen(image_rect);
        // Line from top-center to rotation handle
        let top_center = egui::pos2(
            (corners[0].x + corners[1].x) / 2.0,
            (corners[0].y + corners[1].y) / 2.0,
        );
        painter.line_segment(
            [top_center, rot_pos],
            egui::Stroke::new(1.5, color),
        );
        painter.circle_filled(rot_pos, rot_handle_radius, color);
        painter.circle_stroke(
            rot_pos,
            rot_handle_radius,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
        );
        // Rotation icon: small arc arrow
        painter.text(
            rot_pos,
            egui::Align2::CENTER_CENTER,
            "↻",
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );

        // Region number label (at first corner)
        let label_pos = egui::pos2(corners[0].x + 4.0, corners[0].y + 2.0);
        let galley = painter.layout_no_wrap(
            format!("#{}", i + 1),
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
        let label_rect = egui::Rect::from_min_size(
            label_pos,
            galley.size() + egui::vec2(6.0, 2.0),
        );
        painter.rect_filled(
            label_rect,
            3.0,
            egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 180),
        );
        painter.galley(label_pos + egui::vec2(3.0, 1.0), galley, egui::Color32::WHITE);
    }
}

/// 根据角点索引和区域旋转角度选择合适的缩放光标图标
fn resize_cursor_for_corner(corner_idx: usize, angle: f32) -> egui::CursorIcon {
    // Base diagonal angles for corners [TL, TR, BR, BL]
    // TL/BR → NwSe (\), TR/BL → NeSw (/)
    let base_deg = [-45.0_f32, 45.0, 135.0, 225.0];
    let total = base_deg[corner_idx] + angle.to_degrees();
    // Normalize to [0, 180)
    let norm = ((total % 180.0) + 180.0) % 180.0;
    if norm < 22.5 || norm >= 157.5 {
        egui::CursorIcon::ResizeHorizontal
    } else if norm < 67.5 {
        egui::CursorIcon::ResizeNeSw
    } else if norm < 112.5 {
        egui::CursorIcon::ResizeVertical
    } else {
        egui::CursorIcon::ResizeNwSe
    }
}

/// 从源图像中裁切旋转区域，使用双线性插值保证质量
fn crop_rotated_region(
    img: &image::DynamicImage,
    region: &SplitRegion,
    img_w: u32,
    img_h: u32,
) -> image::DynamicImage {
    let out_w = (region.w * img_w as f32).round() as u32;
    let out_h = (region.h * img_h as f32).round() as u32;
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);

    let cx_px = region.cx * img_w as f32;
    let cy_px = region.cy * img_h as f32;
    let (sin_a, cos_a) = region.angle.sin_cos();

    // For angle ≈ 0, use fast axis-aligned crop
    if region.angle.abs() < 0.001 {
        let px = ((cx_px - out_w as f32 / 2.0).round() as u32).min(img_w.saturating_sub(out_w));
        let py = ((cy_px - out_h as f32 / 2.0).round() as u32).min(img_h.saturating_sub(out_h));
        return img.crop_imm(px, py, out_w.min(img_w - px), out_h.min(img_h - py));
    }

    // Use 16-bit path if source is 16-bit
    match img {
        image::DynamicImage::ImageRgb16(src) => {
            let mut out = image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::new(out_w, out_h);
            let hw = out_w as f32 / 2.0;
            let hh = out_h as f32 / 2.0;
            for oy in 0..out_h {
                for ox in 0..out_w {
                    let lx = ox as f32 - hw + 0.5;
                    let ly = oy as f32 - hh + 0.5;
                    let sx = cx_px + lx * cos_a - ly * sin_a;
                    let sy = cy_px + lx * sin_a + ly * cos_a;
                    let pixel = bilinear_sample_rgb16(src, sx, sy);
                    out.put_pixel(ox, oy, pixel);
                }
            }
            image::DynamicImage::ImageRgb16(out)
        }
        _ => {
            let src = img.to_rgb8();
            let mut out = image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::new(out_w, out_h);
            let hw = out_w as f32 / 2.0;
            let hh = out_h as f32 / 2.0;
            for oy in 0..out_h {
                for ox in 0..out_w {
                    let lx = ox as f32 - hw + 0.5;
                    let ly = oy as f32 - hh + 0.5;
                    let sx = cx_px + lx * cos_a - ly * sin_a;
                    let sy = cy_px + lx * sin_a + ly * cos_a;
                    let pixel = bilinear_sample_rgb8(&src, sx, sy);
                    out.put_pixel(ox, oy, pixel);
                }
            }
            image::DynamicImage::ImageRgb8(out)
        }
    }
}

/// 16 位 RGB 图像的双线性插值采样
pub(super) fn bilinear_sample_rgb16(
    img: &image::ImageBuffer<image::Rgb<u16>, Vec<u16>>,
    x: f32,
    y: f32,
) -> image::Rgb<u16> {
    let w = img.width() as f32;
    let h = img.height() as f32;
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return image::Rgb([0, 0, 0]);
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(img.width() - 1);
    let y1 = (y0 + 1).min(img.height() - 1);
    let fx = x - x.floor();
    let fy = y - y.floor();

    let p00 = img.get_pixel(x0, y0).0;
    let p10 = img.get_pixel(x1, y0).0;
    let p01 = img.get_pixel(x0, y1).0;
    let p11 = img.get_pixel(x1, y1).0;

    let mut out = [0u16; 3];
    for c in 0..3 {
        let v = p00[c] as f32 * (1.0 - fx) * (1.0 - fy)
            + p10[c] as f32 * fx * (1.0 - fy)
            + p01[c] as f32 * (1.0 - fx) * fy
            + p11[c] as f32 * fx * fy;
        out[c] = v.round().clamp(0.0, 65535.0) as u16;
    }
    image::Rgb(out)
}

/// 8 位 RGB 图像的双线性插值采样
pub(super) fn bilinear_sample_rgb8(
    img: &image::ImageBuffer<image::Rgb<u8>, Vec<u8>>,
    x: f32,
    y: f32,
) -> image::Rgb<u8> {
    let w = img.width() as f32;
    let h = img.height() as f32;
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return image::Rgb([0, 0, 0]);
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(img.width() - 1);
    let y1 = (y0 + 1).min(img.height() - 1);
    let fx = x - x.floor();
    let fy = y - y.floor();

    let p00 = img.get_pixel(x0, y0).0;
    let p10 = img.get_pixel(x1, y0).0;
    let p01 = img.get_pixel(x0, y1).0;
    let p11 = img.get_pixel(x1, y1).0;

    let mut out = [0u8; 3];
    for c in 0..3 {
        let v = p00[c] as f32 * (1.0 - fx) * (1.0 - fy)
            + p10[c] as f32 * fx * (1.0 - fy)
            + p01[c] as f32 * (1.0 - fx) * fy
            + p11[c] as f32 * fx * fy;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    image::Rgb(out)
}
