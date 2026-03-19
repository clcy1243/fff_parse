//! 文件列表与网格视图模块
//!
//! 实现文件过滤搜索栏和缩略图网格视图的渲染，
//! 支持模糊搜索和双击进入放大镜模式。

use super::types::*;

use eframe::egui;

use fff_viewer::i18n;

// ─── 网格视图 ───────────────────────────────────────────────────────────────

impl FffViewerApp {
    /// 渲染文件过滤搜索栏，支持模糊匹配和清除按钮
    pub(super) fn render_file_filter_bar(&mut self, ui: &mut egui::Ui) {
        let s = i18n::strings(self.language);
        ui.horizontal(|ui| {
            // Stretch the search field to fill available space minus the clear button
            let clear_width = 24.0;
            let available = ui.available_width() - clear_width - ui.spacing().item_spacing.x * 2.0;
            ui.add(
                egui::TextEdit::singleline(&mut self.file_filter)
                    .hint_text(s.file_filter_placeholder)
                    .desired_width(available),
            );
            let clear_enabled = !self.file_filter.is_empty();
            if ui
                .add_enabled(
                    clear_enabled,
                    egui::Button::new(s.file_filter_clear).min_size(egui::vec2(clear_width, 0.0)),
                )
                .clicked()
            {
                self.file_filter.clear();
            }
            // Show match count when filter is active
            if !self.file_filter.is_empty() {
                let matched = self.filtered_indices().len();
                let total = self.fff_files.len();
                let label = s
                    .files_filtered
                    .replacen("{}", &matched.to_string(), 1)
                    .replacen("{}", &total.to_string(), 1);
                ui.label(egui::RichText::new(label).weak().small());
            }
        });
    }

    /// 返回匹配当前过滤条件的文件索引列表（子序列模糊匹配）
    pub(super) fn filtered_indices(&self) -> Vec<usize> {
        if self.file_filter.is_empty() {
            return (0..self.fff_files.len()).collect();
        }
        let query = self.file_filter.to_lowercase();
        self.fff_files
            .iter()
            .enumerate()
            .filter(|(_, path)| {
                // 匹配完整文件名（名称+扩展名）
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                // 也匹配扩展名
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                // 模糊匹配：查询字符必须按顺序出现（子序列匹配）
                pub(super) fn subsequence(haystack: &str, needle: &str) -> bool {
                    let mut chars = haystack.chars();
                    needle.chars().all(|nc| chars.any(|hc| hc == nc))
                }
                subsequence(&name, &query) || subsequence(&ext, &query)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// 渲染缩略图网格视图，支持选择和双击进入放大镜模式
    pub(super) fn render_grid_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let thumb_size = 180.0_f32;
        let spacing = 8.0_f32;
        let available_width = ui.available_width();
        let cols = ((available_width + spacing) / (thumb_size + spacing)).max(1.0) as usize;

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let files = self.fff_files.clone();
                let indices = self.filtered_indices();
                let selected = self.selected_index;

                let mut new_selection: Option<usize> = None;
                let mut double_clicked: Option<usize> = None;

                egui::Grid::new("thumb_grid")
                    .spacing([spacing, spacing])
                    .show(ui, |ui| {
                        for (col_pos, idx) in indices.iter().enumerate() {
                            let idx = *idx;
                            let path = &files[idx];
                            let is_selected = selected == Some(idx);

                            let frame = egui::Frame::NONE
                                .fill(if is_selected {
                                    if ui.visuals().dark_mode {
                                        egui::Color32::from_rgb(40, 60, 90)
                                    } else {
                                        egui::Color32::from_rgb(200, 220, 245)
                                    }
                                } else {
                                    if ui.visuals().dark_mode {
                                        egui::Color32::from_rgb(30, 30, 30)
                                    } else {
                                        egui::Color32::from_rgb(235, 235, 235)
                                    }
                                })
                                .inner_margin(4.0)
                                .corner_radius(4.0);

                            let resp = frame.show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(thumb_size);
                                    // Fixed-height square area for thumbnail
                                    let cell_h = thumb_size;

                                    if let Some(entry) = self.thumbnails.get(path) {
                                        let aspect =
                                            entry.width as f32 / entry.height.max(1) as f32;
                                        // Fit image within square, preserving aspect ratio
                                        let (display_w, display_h) = if aspect >= 1.0 {
                                            (thumb_size, thumb_size / aspect)
                                        } else {
                                            (thumb_size * aspect, thumb_size)
                                        };
                                        // Vertical centering: pad top
                                        let pad_y = (cell_h - display_h) / 2.0;
                                        if pad_y > 1.0 {
                                            ui.add_space(pad_y);
                                        }
                                        let sized = egui::load::SizedTexture::new(
                                            entry.texture.id(),
                                            egui::vec2(display_w, display_h),
                                        );
                                        ui.with_layout(
                                            egui::Layout::top_down(egui::Align::Center),
                                            |ui| { ui.image(sized); },
                                        );
                                        let pad_bottom = cell_h - display_h - pad_y;
                                        if pad_bottom > 1.0 {
                                            ui.add_space(pad_bottom);
                                        }
                                    } else {
                                        ui.allocate_space(egui::vec2(thumb_size, cell_h));
                                        ui.spinner();
                                    }

                                    let name = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(&name)
                                                    .small()
                                                    .color(ui.visuals().weak_text_color()),
                                            );
                                        },
                                    );
                                });
                            });

                            let resp = resp.response.interact(egui::Sense::click());
                            if resp.clicked() {
                                new_selection = Some(idx);
                            }
                            if resp.double_clicked() {
                                double_clicked = Some(idx);
                            }

                            if (col_pos + 1) % cols == 0 {
                                ui.end_row();
                            }
                        }
                    });

                if let Some(idx) = double_clicked.or(new_selection) {
                    self.select_file(idx, ctx);
                }
                if double_clicked.is_some() {
                    self.view_mode = ViewMode::Loupe;
                }
            });
    }
}
