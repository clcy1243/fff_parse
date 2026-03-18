use super::types::*;
use super::split::draw_split_overlays;

use eframe::egui;

// ─── Loupe View ─────────────────────────────────────────────────────────────

impl FffViewerApp {
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

    pub(super) fn render_loupe_image(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(ref error) = self.error_msg {
            ui.centered_and_justified(|ui| {
                ui.colored_label(egui::Color32::RED, error);
            });
            return;
        }

        if let Some(detail) = &self.detail {
            if let Some(texture) = &detail.texture {
                let available = ui.available_size();
                let tex_size = texture.size_vec2();
                let scale = (available.x / tex_size.x)
                    .min(available.y / tex_size.y)
                    .min(1.0);
                let display_size = egui::vec2(tex_size.x * scale, tex_size.y * scale);

                // Allocate full area with click+drag sensing for split interaction
                let (full_rect, response) = ui.allocate_exact_size(
                    available,
                    if self.info_panel == InfoPanel::Split {
                        egui::Sense::click_and_drag()
                    } else {
                        egui::Sense::hover()
                    },
                );

                // Center the image within the allocated area
                let image_rect =
                    egui::Align2::CENTER_CENTER.align_size_within_rect(display_size, full_rect);

                // Draw the image
                let painter = ui.painter_at(full_rect);
                painter.image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Draw split overlays and handle interactions
                if self.info_panel == InfoPanel::Split {
                    self.handle_split_interactions(&response, image_rect, ctx);
                    let painter = ui.painter_at(full_rect);
                    draw_split_overlays(
                        &painter,
                        image_rect,
                        &self.split_state.regions,
                        self.split_state.selected,
                    );
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

        // Keyboard navigation
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
        // Delete selected split region with Delete/Backspace key
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
