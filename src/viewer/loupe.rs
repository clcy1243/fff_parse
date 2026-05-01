//! 放大镜视图模块
//!
//! 实现单图放大查看，包含主图显示、底部胶片条导航、
//! 键盘左右箭头切换及分割区域叠加显示。

use super::types::*;
use super::split::draw_split_overlays;

use eframe::egui;

// ─── 放大镜视图 ─────────────────────────────────────────────────────────────

impl FffViewerApp {
    /// 渲染放大镜视图：主图 + 底部胶片条
    pub(super) fn render_loupe_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.selected_index.is_none() && !self.fff_files.is_empty() {
            self.select_file(0, ctx);
        }

        let filmstrip_height = 100.0_f32;

        egui::TopBottomPanel::bottom("filmstrip")
            .exact_height(filmstrip_height)
            .show_inside(ui, |ui| {
                self.render_filmstrip(ui, ctx);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(20, 20, 20)
            } else {
                egui::Color32::from_rgb(240, 240, 240)
            }))
            .show_inside(ui, |ui| {
                self.render_loupe_image(ui, ctx);
            });
    }

    /// 渲染放大镜主图区域，处理分割叠加和键盘导航
    pub(super) fn render_loupe_image(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(ref error) = self.error_msg {
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::RED, error);
            });
            return;
        }

        // 为避免后续 self 借用冲突，先取出 texture 的 id 和尺寸（只读信息）
        let texture_info = self.detail.as_ref()
            .and_then(|d| d.texture.as_ref())
            .map(|t| (t.id(), t.size_vec2()));
        if self.detail.is_some() {
            if let Some((texture_id, tex_size)) = texture_info {
                let available = ui.available_size();
                let fit_scale = (available.x / tex_size.x)
                    .min(available.y / tex_size.y)
                    .min(1.0);
                // loupe_zoom: -1.0 = sentinel 代表 "100% 实际像素"；正值 = fit 的倍率
                let scale = if self.loupe_zoom < 0.0 {
                    // 点击 "100%" 后，display_size = 图像原始像素
                    1.0
                } else {
                    fit_scale * self.loupe_zoom
                };
                let display_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);

                // Allocate full area with click+drag sensing for split interaction
                let sense = if self.info_panel == InfoPanel::Split {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::click_and_drag() | egui::Sense::hover()
                };
                let (full_rect, response) = ui.allocate_exact_size(available, sense);

                let split_mode = self.info_panel == InfoPanel::Split;

                // 先用当前（未更新）pan 算出 image_rect 供区域交互 hit-test
                let mut image_rect =
                    egui::Align2::CENTER_CENTER.align_size_within_rect(display_size, full_rect);
                image_rect = image_rect.translate(egui::vec2(self.loupe_pan.0, self.loupe_pan.1));

                // Split 模式：优先处理区域交互（可能在本帧 drag_started 时把 dragging 置为 Some）
                if split_mode {
                    self.handle_split_interactions(&response, image_rect, ctx);
                }

                // "是否正在操作切割框" —— 只有此刻为 true 时才阻止图像 pan/zoom；
                // 光标在空白处时即便在 Split 标签下仍允许缩放/平移
                let region_busy = split_mode && self.split_state.dragging.is_some();

                // 当 scale 超出 fit 时允许 pan（否则 pan 固定为 0）
                let can_pan = display_size.x > full_rect.width() || display_size.y > full_rect.height();
                if !region_busy && can_pan && response.dragged() {
                    let delta = response.drag_delta();
                    self.loupe_pan.0 += delta.x;
                    self.loupe_pan.1 += delta.y;
                }
                if !can_pan {
                    self.loupe_pan = (0.0, 0.0);
                }

                // Clamp pan
                let max_pan_x = ((display_size.x - full_rect.width()) / 2.0).max(0.0);
                let max_pan_y = ((display_size.y - full_rect.height()) / 2.0).max(0.0);
                self.loupe_pan.0 = self.loupe_pan.0.clamp(-max_pan_x, max_pan_x);
                self.loupe_pan.1 = self.loupe_pan.1.clamp(-max_pan_y, max_pan_y);

                // 更新 image_rect（pan 可能刚被改写）
                image_rect =
                    egui::Align2::CENTER_CENTER.align_size_within_rect(display_size, full_rect);
                image_rect = image_rect.translate(egui::vec2(self.loupe_pan.0, self.loupe_pan.1));

                // 鼠标滚轮缩放（正在操作区域时跳过）
                if !region_busy && response.hovered() {
                    let scroll_y = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll_y.abs() > 0.1 {
                        let factor = (scroll_y * 0.003).exp();
                        let new_zoom = if self.loupe_zoom < 0.0 {
                            (1.0 / fit_scale) * factor
                        } else {
                            self.loupe_zoom * factor
                        };
                        self.loupe_zoom = new_zoom.clamp(0.1, 8.0);
                    }
                }

                // Draw the image
                let painter = ui.painter_at(full_rect);
                painter.image(
                    texture_id,
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Draw split overlays（交互已在上面处理过）
                if split_mode {
                    let painter = ui.painter_at(full_rect);
                    draw_split_overlays(
                        &painter,
                        image_rect,
                        &self.split_state.regions,
                        self.split_state.selected,
                    );
                }

                // 右键菜单
                if let Some(idx) = self.selected_index {
                    let path = self.fff_files[idx].clone();
                    response.context_menu(|ui| {
                        self.file_context_menu(ui, &path);
                    });
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(self.s().no_preview);
                });
            }
        } else {
            ui.centered_and_justified(|ui| {
                ui.label(self.s().select_image);
            });
        }

        // 键盘导航
        let (left, right, delete) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            )
        });
        if left || right {
            let count = self.fff_files.len();
            if count > 0 {
                let cur = self.selected_index.unwrap_or(0);
                let next = if right {
                    (cur + 1) % count
                } else {
                    (cur + count - 1) % count
                };
                self.select_file(next, ctx);
            }
        }
        // Delete/Backspace 键删除选中的分割区域
        if delete && self.info_panel == InfoPanel::Split {
            if let Some(idx) = self.split_state.selected {
                if idx < self.split_state.regions.len() {
                    self.split_state.regions.remove(idx);
                    self.split_state.selected = None;
                    self.split_state.dragging = None;
                    self.save_sidecar();
                }
            }
        }
    }

    /// 渲染底部胶片条，显示所有文件的缩略图并支持点击选择
    pub(super) fn render_filmstrip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let strip_h = ui.available_height() - 4.0;
        let _thumb_w = (strip_h * 0.75).max(60.0);

        let files = self.fff_files.clone();
        let selected = self.selected_index;

        egui::ScrollArea::horizontal()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut new_sel: Option<usize> = None;

                    for (idx, path) in files.iter().enumerate() {
                        let is_sel = selected == Some(idx);

                        let dark = ui.visuals().dark_mode;
                        let frame = egui::Frame::NONE
                            .fill(if is_sel {
                                if dark {
                                    egui::Color32::from_rgb(50, 80, 120)
                                } else {
                                    egui::Color32::from_rgb(190, 215, 245)
                                }
                            } else {
                                if dark {
                                    egui::Color32::from_rgb(35, 35, 35)
                                } else {
                                    egui::Color32::from_rgb(230, 230, 230)
                                }
                            })
                            .stroke(if is_sel {
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 180, 255))
                            } else {
                                egui::Stroke::NONE
                            })
                            .inner_margin(2.0)
                            .corner_radius(3.0);

                        let resp = frame
                            .show(ui, |ui| {
                                let dh = strip_h - 4.0;
                                if let Some(entry) = self.thumbnails.get(path) {
                                    let aspect =
                                        entry.width as f32 / entry.height.max(1) as f32;
                                    let dw = dh * aspect;
                                    ui.image(egui::load::SizedTexture::new(
                                        entry.texture.id(),
                                        egui::vec2(dw, dh),
                                    ));
                                } else {
                                    // Placeholder: assume portrait ~0.26 ratio for film scans
                                    ui.allocate_space(egui::vec2(dh * 0.3, dh));
                                }
                            })
                            .response
                            .interact(egui::Sense::click());

                        if resp.clicked() {
                            new_sel = Some(idx);
                        }
                    }

                    if let Some(idx) = new_sel {
                        self.select_file(idx, ctx);
                    }
                });
            });
    }
}
