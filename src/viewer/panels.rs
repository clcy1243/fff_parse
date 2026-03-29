//! 信息面板模块
//!
//! 实现右侧所有信息面板的渲染：元数据、编辑历史、标签浏览、
//! 色彩调整、ICC 色彩配置、分割导出和应用设置面板。

use super::types::*;
use super::helpers::*;

use eframe::egui;
use std::collections::HashSet;

use fff_viewer::color::{self, SettingsPreset, TargetColorSpace};
use fff_viewer::config;
use fff_viewer::flexcolor;
use fff_viewer::i18n::{self, Language};
use fff_viewer::sidecar::{self, SidecarConfig, SidecarRegion as SidecarRegionData};

// ─── 空状态 ─────────────────────────────────────────────────────────────────

impl FffViewerApp {
    /// 渲染无文件时的空状态页面：应用标题、打开文件夹按钮和拖放提示
    pub(super) fn render_empty_state(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.heading(s.app_title);
                ui.label(s.app_subtitle);
                ui.add_space(20.0);
                if self.current_dir.is_some() {
                    ui.label(s.no_fff_files);
                } else {
                    ui.label(s.select_folder_hint);
                }
                ui.add_space(10.0);
                if ui
                    .button(egui::RichText::new(s.open_folder).size(16.0))
                    .clicked()
                {
                    self.open_directory_dialog();
                }
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(s.drag_drop_hint)
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
    }
}

// ─── 右侧信息面板 ──────────────────────────────────────────────────────────

impl FffViewerApp {
    /// 从 correction 的 shadow/highlight/gray 加载色阶手柄值。
    /// shadow/highlight 值始终在后处理空间（对负片已是反转后的值）。
    /// FlexColor 模式：Master shadow = min(R,G,B)，Master highlight = max(R,G,B)。
    fn load_levels_from_correction(adj: &mut color::ManualAdjust, corr: &flexcolor::ImageCorrection) {
        if !corr.apply_histogram { return; }
        // 先加载 per-channel (R=1, G=2, B=3) shadow/highlight
        for i in 0..4 {
            adj.levels_black[i] = (corr.shadow[i] as f32 * 4.0 / 65535.0 * 255.0).clamp(0.0, 255.0);
            adj.levels_white[i] = (corr.highlight[i] as f32 * 4.0 / 65535.0 * 255.0).clamp(0.0, 255.0);
        }
        // Master gamma 来自 Gamma 字段：FlexColor Gamma 2.0 → 显示 1.0（中性）
        adj.levels_gamma[0] = ((corr.gamma as f32) - 1.0).clamp(0.01, 3.00);
        // Per-channel gamma 来自 Gray 字段（0-255 位置，128=中性→gamma=1.0）
        for i in 1..4 {
            adj.levels_gamma[i] = (corr.gray[i] as f32 / 128.0).clamp(0.01, 99.0);
        }
        // Master shadow/highlight 从 per-channel 派生（仅用于 UI 显示，不参与图像计算）
        adj.levels_black[0] = adj.levels_black[1].min(adj.levels_black[2]).min(adj.levels_black[3]);
        adj.levels_white[0] = adj.levels_white[1].max(adj.levels_white[2]).max(adj.levels_white[3]);
        // 输出色阶 (DotColor)
        if corr.dot_color.len() >= 14 {
            adj.output_shadow = corr.dot_color[0] as f32;
            adj.output_highlight = corr.dot_color[7] as f32;
        }
    }

    /// 渲染元数据面板：显示选中文件的关键 TIFF/EXIF 元数据
    pub(super) fn render_metadata_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.metadata_heading);
        ui.separator();

        if let Some(detail) = &self.detail {
            ui.label(
                egui::RichText::new(
                    detail
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
                .strong()
                .color(ui.visuals().hyperlink_color),
            );
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    egui::Grid::new("metadata_grid")
                        .striped(true)
                        .num_columns(2)
                        .min_col_width(100.0)
                        .show(ui, |ui| {
                            for (key, value) in &detail.metadata {
                                ui.strong(key);
                                ui.label(egui::RichText::new(value).monospace());
                                ui.end_row();
                            }
                        });
                });
        } else {
            ui.label(s.select_file_metadata);
        }
    }

    /// 渲染编辑历史面板：展示 FlexColor 的编辑设置记录
    pub(super) fn render_edit_history_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.edit_history_heading);
        ui.separator();

        let history = match &self.detail {
            Some(detail) => match &detail.edit_history {
                Some(h) => h.clone(),
                None => {
                    ui.label(s.no_edit_history);
                    return;
                }
            },
            None => {
                ui.label(s.select_file_history);
                return;
            }
        };

        let s = self.s();
        ui.label(
            egui::RichText::new(format!(
                "{} {} #{}",
                history.settings.len(),
                s.settings_count,
                history.current_index
            ))
            .small()
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(4.0);

        let lang = self.language;
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (idx, setting) in history.settings.iter().enumerate() {
                    let is_current = idx == history.current_index;
                    let is_expanded = self.expanded_setting == Some(idx);

                    let header_text = format!(
                        "{}{}  \"{}\"",
                        if is_current { "▶ " } else { "   " },
                        idx,
                        setting.name
                    );

                    let header = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&header_text)
                                .strong()
                                .size(14.0)
                                .color(if is_current {
                                    ui.visuals().hyperlink_color
                                } else {
                                    ui.visuals().text_color()
                                }),
                        )
                        .sense(egui::Sense::click()),
                    );

                    if header.clicked() {
                        self.expanded_setting = if is_expanded { None } else { Some(idx) };
                    }

                    let s = i18n::strings(lang);
                    ui.indent(format!("setting_basic_{}", idx), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{}: {}", s.created, setting.created))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        if setting.modified.is_valid() {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}: {}",
                                    s.modified, setting.modified
                                ))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                        }

                        let c = &setting.correction;
                        ui.horizontal_wrapped(|ui| {
                            Self::param_chip(ui, "γ", &format!("{}", c.gamma), c.gamma != 2.0);
                            Self::param_chip(ui, "EV", &format!("{}", c.ev), c.ev != 1.0);
                            Self::param_chip(
                                ui,
                                s.contrast,
                                &format!("{}", c.contrast),
                                c.contrast != 0,
                            );
                            Self::param_chip(
                                ui,
                                s.brightness,
                                &format!("{}", c.brightness),
                                c.brightness != 0,
                            );
                            Self::param_chip(
                                ui,
                                s.saturation,
                                &format!("{}", c.saturation),
                                c.saturation != 0,
                            );
                        });
                    });

                    if is_expanded {
                        ui.indent(format!("setting_detail_{}", idx), |ui| {
                            ui.add_space(4.0);
                            Self::render_correction_details(ui, &setting.correction, lang);
                        });
                    }

                    ui.add_space(8.0);
                    if idx + 1 < history.settings.len() {
                        ui.separator();
                    }
                }
            });
    }

    /// 渲染所有标签面板：可搜索过滤的完整 TIFF/IFD 标签表
    pub(super) fn render_all_tags_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.all_tags_heading);
        ui.horizontal(|ui| {
            ui.label(s.filter);
            ui.text_edit_singleline(&mut self.tag_filter);
        });
        ui.separator();

        let Some(detail) = &self.detail else {
            ui.label(s.select_file_tags);
            return;
        };

        let s = self.s();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                egui::Grid::new("all_tags_grid")
                    .striped(true)
                    .num_columns(4)
                    .min_col_width(40.0)
                    .show(ui, |ui| {
                        ui.strong(s.ifd_header);
                        ui.strong(s.tag_header);
                        ui.strong(s.name_header);
                        ui.strong(s.value_header);
                        ui.end_row();

                        let filter = self.tag_filter.to_lowercase();
                        for (ifd_name, tag_hex, tag_name, value) in &detail.all_tags {
                            if !filter.is_empty()
                                && !tag_name.to_lowercase().contains(&filter)
                                && !tag_hex.to_lowercase().contains(&filter)
                                && !ifd_name.to_lowercase().contains(&filter)
                                && !value.to_lowercase().contains(&filter)
                            {
                                continue;
                            }

                            ui.label(
                                egui::RichText::new(ifd_name)
                                    .small()
                                    .color(ui.visuals().hyperlink_color),
                            );
                            ui.label(egui::RichText::new(tag_hex).small().monospace());
                            ui.label(tag_name);

                            let display_value = if value.len() > 60 {
                                format!("{}...", &value[..57])
                            } else {
                                value.clone()
                            };
                            ui.label(egui::RichText::new(display_value).small().monospace())
                                .on_hover_text(value);

                            ui.end_row();
                        }
                    });
            });
    }

    // ── ICC 色彩配置面板 ──────────────────────────────────────────

    /// 渲染色彩配置面板：输入 ICC 配置文件、目标色彩空间、预设和应用/重置按钮
    pub(super) fn render_color_profile_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = self.s();
        ui.heading(s.color_profile);
        ui.separator();

        // ── Input Profile ──
        ui.strong(s.input_profile);
        ui.add_space(2.0);

        let has_embedded_icc = self
            .detail
            .as_ref()
            .and_then(|d| d.embedded_icc.as_ref())
            .map(|d| !d.is_empty())
            .unwrap_or(false);

        let profile_names: Vec<String> = self
            .available_profiles
            .iter()
            .map(|p| format!("{} ({})", p.name, p.color_space))
            .collect();

        // 当前选中标签：内嵌 ICC、外部配置文件或无
        let current_label = if self.use_embedded_icc && has_embedded_icc {
            format!("📎 {}", s.use_embedded_icc)
        } else {
            self.selected_input_profile
                .and_then(|i| profile_names.get(i))
                .cloned()
                .unwrap_or_else(|| "—".to_string())
        };

        egui::ComboBox::from_id_salt("input_profile_combo")
            .selected_text(&current_label)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                // "— None —" 选项
                let is_none = !self.use_embedded_icc && self.selected_input_profile.is_none();
                if ui.selectable_label(is_none, "— None —").clicked() && !is_none {
                    self.selected_input_profile = None;
                    self.use_embedded_icc = false;
                    self.color_status = None;
                }
                // 📎 内嵌 ICC 选项（仅当文件有内嵌 ICC 时显示）
                if has_embedded_icc {
                    if ui.selectable_label(
                        self.use_embedded_icc,
                        format!("📎 {}", s.use_embedded_icc),
                    ).clicked() && !self.use_embedded_icc {
                        self.use_embedded_icc = true;
                        self.selected_input_profile = None;
                        self.color_status = None;
                    }
                }
                // 外部 ICC 配置文件
                for (i, name) in profile_names.iter().enumerate() {
                    let cs = &self.available_profiles[i].color_space;
                    let is_rgb = cs == "RGB" || cs == "RGB ";
                    ui.add_enabled_ui(is_rgb, |ui| {
                        let label = if is_rgb {
                            name.clone()
                        } else {
                            format!("{} ⛔", name)
                        };
                        let is_selected = !self.use_embedded_icc
                            && self.selected_input_profile == Some(i);
                        if ui.selectable_label(is_selected, label).clicked() && !is_selected {
                            self.selected_input_profile = Some(i);
                            self.use_embedded_icc = false;
                            self.color_status = None;
                        }
                    });
                }
            });

        // 显示选中的 ICC 配置详情（内嵌或外部）
        {
            let icc_detail = if self.use_embedded_icc {
                self.detail.as_ref()
                    .and_then(|d| d.embedded_icc.as_ref())
                    .and_then(|bytes| color::parse_icc_detail(bytes))
            } else if let Some(idx) = self.selected_input_profile {
                self.available_profiles.get(idx)
                    .and_then(|p| std::fs::read(&p.path).ok())
                    .and_then(|bytes| color::parse_icc_detail(&bytes))
            } else {
                None
            };

            if let Some(detail) = icc_detail {
                egui::CollapsingHeader::new(
                    egui::RichText::new(if detail.description.is_empty() {
                        format!("ℹ ICC ({} {})", detail.color_space, detail.device_class_name)
                    } else {
                        format!("ℹ {}", detail.description)
                    }).small()
                )
                .id_salt("input_icc_detail")
                .show(ui, |ui| {
                    egui::Grid::new("input_icc_grid")
                        .num_columns(2)
                        .spacing([8.0, 2.0])
                        .show(ui, |ui| {
                            let row = |ui: &mut egui::Ui, label: &str, value: &str| {
                                ui.label(egui::RichText::new(label).small());
                                ui.label(egui::RichText::new(value).small().monospace());
                                ui.end_row();
                            };
                            if !detail.description.is_empty() {
                                row(ui, s.icc_detail_name, &detail.description);
                            }
                            row(ui, s.icc_detail_version, &detail.version);
                            row(ui, s.icc_detail_class,
                                &format!("{} ({})", detail.device_class_name, detail.device_class));
                            row(ui, s.icc_detail_color_space, &detail.color_space);
                            row(ui, s.icc_detail_pcs, &detail.pcs);
                            row(ui, s.icc_detail_intent, &detail.rendering_intent);
                            row(ui, s.icc_detail_date, &detail.date_time);
                            if !detail.cmm_type.is_empty() {
                                row(ui, s.icc_detail_cmm, &detail.cmm_type);
                            }
                            row(ui, s.icc_detail_illuminant, &detail.illuminant);
                            row(ui, s.icc_detail_size,
                                &format!("{} bytes", detail.size));
                            row(ui, s.icc_detail_tags, &detail.tag_count.to_string());
                        });

                    if !detail.tags.is_empty() {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(
                            detail.tags.iter()
                                .map(|(sig, _, sz)| format!("{sig}({sz})"))
                                .collect::<Vec<_>>()
                                .join("  ")
                        ).small().weak());
                    }
                });
            }
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Target Color Space ──
        ui.strong(s.target_color_space);
        ui.add_space(2.0);

        let old_target = self.target_color_space;
        let current_target_label = self.target_color_space.label();
        egui::ComboBox::from_id_salt("target_colorspace_combo")
            .selected_text(current_target_label)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                for &space in TargetColorSpace::ALL {
                    if ui
                        .selectable_value(&mut self.target_color_space, space, space.label())
                        .clicked()
                    {
                        self.color_status = None;
                    }
                }
            });
        if self.target_color_space != old_target {
            self.save_sidecar();
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Settings Preset ──
        ui.strong(s.settings_preset);
        ui.add_space(2.0);

        // 收集内嵌编辑历史条目
        let embedded_entries: Vec<(usize, String)> = self.detail.as_ref()
            .and_then(|d| d.edit_history.as_ref())
            .map(|h| {
                h.settings.iter().enumerate().map(|(i, setting)| {
                    let label = if setting.name.is_empty() {
                        format!("📎 #{}", i)
                    } else {
                        format!("📎 {}", setting.name)
                    };
                    (i, label)
                }).collect()
            })
            .unwrap_or_default();

        // Category filter
        let categories: Vec<String> = {
            let mut cats: Vec<String> = self
                .available_presets
                .iter()
                .map(|p| p.category.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            cats.sort();
            cats
        };

        if !categories.is_empty() {
            egui::ComboBox::from_id_salt("preset_category_combo")
                .selected_text(if self.preset_category_filter.is_empty() {
                    s.category_all
                } else {
                    &self.preset_category_filter
                })
                .width(ui.available_width() - 16.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.preset_category_filter.is_empty(), s.category_all)
                        .clicked()
                    {
                        self.preset_category_filter.clear();
                    }
                    for cat in &categories {
                        let label = if cat.is_empty() { "Standard" } else { cat.as_str() };
                        if ui
                            .selectable_label(self.preset_category_filter == *cat, label)
                            .clicked()
                        {
                            self.preset_category_filter = cat.clone();
                        }
                    }
                });

            ui.add_space(4.0);
        }

        // Filtered presets
        let filtered_presets: Vec<(usize, &SettingsPreset)> = self
            .available_presets
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                self.preset_category_filter.is_empty()
                    || p.category == self.preset_category_filter
            })
            .collect();

        // 当前选中标签
        let preset_label = if self.use_embedded_correction {
            self.embedded_correction_index
                .and_then(|idx| embedded_entries.iter().find(|(i, _)| *i == idx))
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| format!("📎 #{}", self.embedded_correction_index.unwrap_or(0)))
        } else {
            self.selected_preset
                .and_then(|i| self.available_presets.get(i))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "—".to_string())
        };

        egui::ComboBox::from_id_salt("preset_combo")
            .selected_text(&preset_label)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                // "— None —" 选项
                let is_none = !self.use_embedded_correction && self.selected_preset.is_none();
                if ui.selectable_label(is_none, "— None —").clicked() && !is_none {
                    self.selected_preset = None;
                    self.use_embedded_correction = false;
                    self.embedded_correction_index = None;
                    self.color_status = None;
                }

                // 内嵌编辑历史条目
                if !embedded_entries.is_empty() {
                    ui.separator();
                    for (idx, label) in &embedded_entries {
                        let is_selected = self.use_embedded_correction
                            && self.embedded_correction_index == Some(*idx);
                        if ui.selectable_label(is_selected, label).clicked() && !is_selected {
                            self.use_embedded_correction = true;
                            self.embedded_correction_index = Some(*idx);
                            self.selected_preset = None;
                            self.color_status = None;
                        }
                    }
                    if !filtered_presets.is_empty() {
                        ui.separator();
                    }
                }

                // 外部预设
                for (global_idx, preset) in &filtered_presets {
                    let label = if preset.category.is_empty() {
                        preset.name.clone()
                    } else {
                        format!("{}/{}", preset.category, preset.name)
                    };
                    let is_selected = !self.use_embedded_correction
                        && self.selected_preset == Some(*global_idx);
                    if ui.selectable_label(is_selected, &label).clicked() && !is_selected {
                        self.selected_preset = Some(*global_idx);
                        self.use_embedded_correction = false;
                        self.embedded_correction_index = None;
                        self.color_status = None;
                    }
                }
            });

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Apply / Reset buttons ──
        ui.horizontal(|ui| {
            let can_apply = self.detail.is_some()
                && (self.selected_input_profile.is_some()
                    || self.use_embedded_icc
                    || self.selected_preset.is_some()
                    || self.use_embedded_correction);

            if ui
                .add_enabled(can_apply, egui::Button::new(s.apply_profile))
                .clicked()
            {
                self.loading_status = LoadingStatus::ApplyingColorProfile;
                self.apply_color_profile(ctx);
                self.loading_status = LoadingStatus::Idle;
                self.save_sidecar();
            }

            if ui.button(s.reset_profile).clicked() {
                self.reset_color_profile(ctx);
                self.save_sidecar();
            }
        });

        // Status message
        if let Some(status) = &self.color_status {
            ui.add_space(8.0);
            let is_error = status.starts_with('❌');
            let color = if is_error {
                if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(255, 100, 100)
                } else {
                    egui::Color32::from_rgb(200, 0, 0)
                }
            } else {
                if ui.visuals().dark_mode {
                    egui::Color32::from_rgb(100, 255, 100)
                } else {
                    egui::Color32::from_rgb(0, 140, 0)
                }
            };
            ui.label(egui::RichText::new(status).color(color));
        }

        // ── Show correction details: embedded or selected preset ──
        let active_correction: Option<(String, flexcolor::ImageCorrection)> = if self.use_embedded_correction {
            let emb_idx = self.embedded_correction_index;
            self.detail.as_ref()
                .and_then(|d| d.edit_history.as_ref())
                .and_then(|h| {
                    let idx = emb_idx.unwrap_or(
                        h.current_index.min(h.settings.len().saturating_sub(1))
                    );
                    h.settings.get(idx).map(|setting| {
                        let label = if setting.name.is_empty() {
                            format!("📎 #{}", idx)
                        } else {
                            format!("📎 {}", setting.name)
                        };
                        (label, setting.correction.clone())
                    })
                })
        } else if let Some(idx) = self.selected_preset {
            self.available_presets.get(idx).and_then(|preset| {
                std::fs::read_to_string(&preset.path).ok().and_then(|xml_data| {
                    flexcolor::parse_settings_xml(&xml_data)
                        .map(|corr| (format!("📋 {}", preset.name), corr))
                })
            })
        } else {
            None
        };

        if let Some((title, correction)) = active_correction {
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);
            ui.strong(
                egui::RichText::new(&title)
                    .color(ui.visuals().hyperlink_color),
            );

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    Self::render_correction_details(
                        ui,
                        &correction,
                        self.language,
                    );
                });
        }
    }

    /// 从 sidecar XML 恢复保存的配置到当前状态
    pub(super) fn apply_sidecar(&mut self, config: &SidecarConfig, ctx: &egui::Context) {
        log::info!("Applying sidecar config");
        self.use_embedded_correction = config.use_embedded_correction;
        self.use_embedded_icc = config.use_embedded_icc;
        self.embedded_correction_index = config.embedded_correction_index;
        self.target_color_space = TargetColorSpace::from_str(&config.target_color_space);

        // Look up profile by name
        self.selected_input_profile = config.input_profile_name.as_ref().and_then(|name| {
            self.available_profiles.iter().position(|p| &p.name == name)
        });

        // Look up preset by name
        self.selected_preset = config.preset_name.as_ref().and_then(|name| {
            self.available_presets.iter().position(|p| &p.name == name)
        });

        // Restore split state
        self.split_state.format = FilmFormat::from_str(&config.split_format);
        self.split_state.portrait = config.split_portrait;
        self.split_state.naming_pattern = config.split_naming_pattern.clone();
        self.split_state.regions = config.split_regions.iter().map(|r| {
            SplitRegion { cx: r.cx, cy: r.cy, w: r.w, h: r.h, angle: r.angle }
        }).collect();
        self.split_state.selected = None;
        self.split_state.dragging = None;

        // Re-apply color profile if any profile/preset was selected
        self.manual_adjust = config.manual_adjust.clone();
        self.histogram_needs_update = true;

        if self.selected_input_profile.is_some() || self.selected_preset.is_some()
            || self.use_embedded_icc || self.use_embedded_correction {
            self.apply_color_profile(ctx);
        } else {
            self.rebuild_texture_from_base(ctx);
        }
    }

    /// 将当前状态保存为 sidecar XML 文件
    pub(super) fn save_sidecar(&self) {
        let path = match &self.detail {
            Some(d) => &d.path,
            None => return,
        };

        let input_profile_name = self.selected_input_profile
            .and_then(|i| self.available_profiles.get(i))
            .map(|p| p.name.clone());
        let preset_name = self.selected_preset
            .and_then(|i| self.available_presets.get(i))
            .map(|p| p.name.clone());

        let config = SidecarConfig {
            use_embedded_correction: self.use_embedded_correction,
            use_embedded_icc: self.use_embedded_icc,
            embedded_correction_index: self.embedded_correction_index,
            input_profile_name,
            preset_name,
            target_color_space: self.target_color_space.to_str().to_string(),
            split_format: self.split_state.format.to_str().to_string(),
            split_portrait: self.split_state.portrait,
            split_naming_pattern: self.split_state.naming_pattern.clone(),
            split_regions: self.split_state.regions.iter().map(|r| {
                SidecarRegionData { cx: r.cx, cy: r.cy, w: r.w, h: r.h, angle: r.angle }
            }).collect(),
            manual_adjust: self.manual_adjust.clone(),
        };

        if let Err(e) = sidecar::save(path, &config) {
            log::error!("Failed to save sidecar: {}", e);
        } else {
            log::debug!("Saved sidecar: {}", sidecar::sidecar_path(path).display());
        }
    }

    /// 根据嵌入校正的 InputProfile 字段自动选择输入 ICC 配置文件
    pub(super) fn auto_select_input_profile(&mut self) {
        // 1. 优先使用色彩方案中的 InputProfile 名称
        let profile_name = self.detail.as_ref()
            .and_then(|d| d.edit_history.as_ref())
            .and_then(|h| {
                let idx = h.current_index.min(h.settings.len().saturating_sub(1));
                h.settings.get(idx)
            })
            .and_then(|s| s.correction.input_profile_name.clone());

        if let Some(ref name) = profile_name {
            if let Some(pos) = self.available_profiles.iter().position(|p| {
                p.name == *name || p.name == format!("{}.icc", name)
            }) {
                log::info!("Auto-selected input profile: {} (index {})", name, pos);
                self.selected_input_profile = Some(pos);
                return;
            }
            log::info!("InputProfile='{}' not found, trying scanner model match", name);
        }

        // 2. 回退：根据扫描仪型号从 ICC 名称中匹配
        let scanner_model = self.detail.as_ref()
            .and_then(|d| d.metadata.iter().find(|(k, _)| k == "Model").map(|(_, v)| v.clone()));

        if let Some(ref model) = scanner_model {
            // 从型号中提取关键字（如 "Flextight X5" → 搜索 "X5"）
            let model_lower = model.to_lowercase();
            if let Some(pos) = self.available_profiles.iter().position(|p| {
                let name_lower = p.name.to_lowercase();
                // 匹配策略：ICC 名称包含扫描仪型号关键部分
                if model_lower.contains("x5") && name_lower.contains("x5") { return true; }
                if model_lower.contains("x1") && name_lower.contains("x1") { return true; }
                if model_lower.contains("848") && name_lower.contains("848") { return true; }
                if model_lower.contains("949") && name_lower.contains("949") { return true; }
                if model_lower.contains("flextight") && name_lower.contains("flextight") { return true; }
                false
            }) {
                log::info!("Auto-selected input profile by scanner model '{}': {} (index {})",
                    model, self.available_profiles[pos].name, pos);
                self.selected_input_profile = Some(pos);
                return;
            }
            log::warn!("No ICC profile matching scanner model '{}'", model);
        }
    }

    /// 应用色彩配置：先做底片处理（负片反转+色阶），再做 ICC 色彩空间转换
    pub(super) fn apply_color_profile(&mut self, ctx: &egui::Context) {
        let s = self.s();

        // Determine input ICC data
        let input_icc = if self.use_embedded_icc {
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

        // Load preset correction: either from embedded edit history or from selected preset file
        let preset_correction = if self.use_embedded_correction {
            let emb_idx = self.embedded_correction_index;
            self.detail.as_ref().and_then(|d| {
                d.edit_history.as_ref().and_then(|h| {
                    if h.settings.is_empty() {
                        None
                    } else {
                        let idx = emb_idx.unwrap_or(
                            h.current_index.min(h.settings.len() - 1)
                        );
                        h.settings.get(idx).map(|s| s.correction.clone())
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

        // Need either ICC or preset (or both)
        if input_icc.is_none() && preset_correction.is_none() {
            self.color_status = Some(format!("❌ {}: no ICC or preset", s.profile_error));
            return;
        }

        let Some(detail) = &self.detail else {
            return;
        };

        // 第1步：解码原始预览图像
        let preview_img = match detail.tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
            Some(img) => img,
            None => {
                self.color_status = Some(format!("❌ {}: no preview", s.profile_error));
                return;
            }
        };

        // 保存原始预览（用于切换胶片类型时重新处理）
        let preview_raw = to_rgb16(&preview_img);
        let mut result = preview_img;

        // 第2步：应用胶片处理（负片反转 + B&W 去色）— 在 scanner 空间中进行
        if let Some(ref correction) = preset_correction {
            let film_type = correction.film_type;
            log::info!(
                "Applying film processing: FilmType={} ({}), Shadow={:?}, Highlight={:?}, Gray={:?}, \
                 FilmCurve={}, Gamma={}",
                film_type,
                flexcolor::film_type_name(film_type),
                correction.shadow,
                correction.highlight,
                correction.gray,
                correction.film_curve,
                correction.gamma,
            );

            result = color::apply_film_processing(&result, correction);

            // 提取负片胶片曲线（从 8-bit 缩略图 + 16-bit 预览逆向）
            if correction.film_type == 1 || correction.film_type == 2 {
                if let Some((thumb_8, preview_16)) = detail.tiff.decode_thumbnail_pair() {
                    self.extracted_film_lut = color::extract_film_curve(
                        &thumb_8, &preview_16, correction,
                    );
                    if self.extracted_film_lut.is_some() {
                        log::info!("Film curve extracted from thumbnail pair");
                    } else {
                        log::warn!("Film curve extraction returned None");
                    }
                } else {
                    log::warn!("No thumbnail pair found for film curve extraction");
                    self.extracted_film_lut = None;
                }
            } else {
                self.extracted_film_lut = None;
            }

            // 从色彩方案加载色阶到手柄（负片自动翻转）
            Self::load_levels_from_correction(&mut self.manual_adjust, correction);

            // 提取胶片类型、胶片曲线、Gamma
            self.manual_adjust.film_type = correction.film_type;
            self.manual_adjust.film_curve = correction.film_curve;
            self.manual_adjust.film_gamma = correction.gamma;

            // 将滑块参数（饱和度/EV/对比度/亮度/阴影深度/中间调）映射到手柄
            if correction.apply_sliders {
                self.manual_adjust.saturation = correction.saturation as f32;
                if (correction.ev - 1.0).abs() > 0.001 {
                    self.manual_adjust.exposure = correction.ev.log2() as f32;
                }
                self.manual_adjust.contrast = correction.contrast as f32;
                self.manual_adjust.brightness = correction.brightness as f32;
                self.manual_adjust.lightness = correction.lightness as f32;
            }
            // 提取色温/色调
            self.manual_adjust.color_temperature = correction.color_temperature as f32;
            self.manual_adjust.tint = correction.tint as f32;
            // 提取色彩校正矩阵
            if correction.apply_cc && correction.color_corr.len() == 36 {
                let mut arr = [0i64; 36];
                for (i, &v) in correction.color_corr.iter().enumerate() {
                    arr[i] = v;
                }
                self.manual_adjust.color_corr = arr;
                self.manual_adjust.apply_color_corr = true;
            } else {
                self.manual_adjust.color_corr = [0i64; 36];
                self.manual_adjust.apply_color_corr = false;
            }
            // 提取渐变曲线开关
            self.manual_adjust.apply_curves = correction.apply_curves && !correction.gradations.is_empty();
            // 提取 USM 锐化参数
            self.manual_adjust.apply_usm = correction.apply_usm;
            self.manual_adjust.usm_amount = correction.usm_amount;
            self.manual_adjust.usm_radius = correction.usm_radius;
            self.manual_adjust.usm_dark_limit = correction.usm_dark_limit;
            self.manual_adjust.usm_noise_limit = correction.usm_noise_limit;
            if correction.usm_col_factor.len() >= 3 {
                self.manual_adjust.usm_col_factor = [
                    correction.usm_col_factor[0],
                    correction.usm_col_factor[1],
                    correction.usm_col_factor[2],
                ];
            }
            // 提取除尘参数
            self.manual_adjust.apply_dust = correction.apply_dust;
            self.manual_adjust.dust_level = correction.dust_level;
            // 提取色彩噪声滤镜参数
            self.manual_adjust.apply_cn_filter = correction.apply_cn_filter;
            self.manual_adjust.color_noise_radius = correction.color_noise_radius;
            self.manual_adjust.noise_filter_bias = correction.noise_filter_bias;
            // 提取镜头/暗角校正参数
            self.manual_adjust.lens_correction = correction.lens_correction;
            self.manual_adjust.vignette_amount = correction.vignette_amount;
            // 提取阴影增强与色偏去除参数
            self.manual_adjust.enhanced_shadow = correction.enhanced_shadow;
            self.manual_adjust.remove_cast_highlight = correction.remove_cast_highlight;
            self.manual_adjust.remove_cast_shadow = correction.remove_cast_shadow;
            // 加载渐变曲线控制点到编辑器
            if !correction.gradations.is_empty() {
                self.curve_points = correction.gradations.clone();
                while self.curve_points.len() < 7 {
                    self.curve_points.push(vec![(0, 0, 0), (255, 255, 0)]);
                }
            } else {
                self.curve_points = Self::default_curve_points();
            }
            self.curve_channel = 0;
            self.curve_dragging = None;
            // 同步到两组手柄状态
            self.levels_processed = HistogramLevels {
                black: self.manual_adjust.levels_black,
                gamma: self.manual_adjust.levels_gamma,
                white: self.manual_adjust.levels_white,
            };
            self.levels_raw = self.levels_processed.clone();
        }

        // 第3步：保存各阶段 16-bit 基准图像
        // preview_raw: 原始解码预览（用于切换胶片类型重新处理）
        // raw_rgb: 反转后的 scanner 空间数据（rebuild 起点）
        let scanner_rgb = to_rgb16(&result);
        if let Some(detail) = &mut self.detail {
            detail.preview_raw = Some(preview_raw);
            detail.raw_rgb = Some(scanner_rgb);
            detail.base_rgb = detail.raw_rgb.clone();
        }

        // 缓存 ICC 数据（供 rebuild_texture_from_base 使用）
        self.active_icc_data = input_icc;

        // 保存当前调整状态为基线（重置按钮恢复到此状态）
        self.baseline_adjust = self.manual_adjust.clone();
        self.baseline_levels_processed = self.levels_processed.clone();
        self.baseline_levels_raw = self.levels_raw.clone();
        self.baseline_curve_points = self.curve_points.clone();

        self.histogram_needs_update = true;
        self.rebuild_texture_from_base(ctx);

        let status_parts: Vec<&str> = [
            self.active_icc_data.as_ref().map(|_| "ICC"),
            preset_correction.as_ref().map(|c| {
                if c.film_type == 1 { "Neg→Pos" }
                else if c.film_type == 2 { "B&W Neg→Pos" }
                else { "Levels" }
            }),
        ]
        .into_iter()
        .flatten()
        .collect();

        self.color_status = Some(format!("✅ {} ({})", s.profile_applied, status_parts.join(" + ")));
        log::info!("Color profile applied: {}", status_parts.join(" + "));
    }

    /// 重置色彩配置到默认状态，重新解码预览并应用嵌入校正
    pub(super) fn reset_color_profile(&mut self, ctx: &egui::Context) {
        self.selected_input_profile = None;
        self.selected_preset = None;
        self.use_embedded_icc = false;
        self.embedded_correction_index = None;
        self.color_status = None;
        self.extracted_film_lut = None;

        // Re-decode preview with downscaling, auto-apply embedded correction if available
        if let Some(detail) = &mut self.detail {
            if let Some(ref eh) = detail.edit_history {
                if !eh.settings.is_empty() {
                    let idx = eh.current_index.min(eh.settings.len() - 1);
                    self.use_embedded_correction = true;
                    self.embedded_correction_index = Some(idx);
                }
            }
        }

        // 根据嵌入校正的 InputProfile 自动匹配 ICC 配置文件
        self.auto_select_input_profile();

        if self.selected_input_profile.is_some() || self.use_embedded_correction {
            // 有 ICC 或嵌入校正时走完整管线
            self.apply_color_profile(ctx);
        } else {
            // 无 ICC 无嵌入校正：回退到原始预览
            if let Some(detail) = &mut self.detail {
                if let Some(img) = detail.tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
                    detail.raw_rgb = Some(to_rgb16(&img));
                    detail.base_rgb = Some(to_rgb16(&img));
                }
            }
            self.histogram_needs_update = true;
            self.rebuild_texture_from_base(ctx);
        }
        log::info!("Color profile reset");
    }

    /// 当用户更改胶片类型时，使用新的 film_type 重新处理管线。
    /// 从 preview_raw（原始解码预览）重新开始胶片处理。
    pub(super) fn reprocess_with_film_type(&mut self, ctx: &egui::Context) {
        let new_film_type = self.manual_adjust.film_type;
        log::info!("Reprocessing with film_type={}, use_embedded={}, emb_idx={:?}",
            new_film_type, self.use_embedded_correction, self.embedded_correction_index);

        let Some(detail) = &self.detail else {
            log::warn!("reprocess_with_film_type: no detail");
            return;
        };
        let Some(preview_raw) = detail.preview_raw.as_ref() else {
            log::warn!("reprocess_with_film_type: no preview_raw, falling back to apply_color_profile");
            self.apply_color_profile(ctx);
            return;
        };

        let mut result = image::DynamicImage::ImageRgb16(preview_raw.clone());

        // 构建临时 correction，使用新的 film_type
        let correction = if self.use_embedded_correction {
            let emb_idx = self.embedded_correction_index;
            detail.edit_history.as_ref().and_then(|h| {
                if h.settings.is_empty() { return None; }
                let idx = emb_idx.unwrap_or(h.current_index.min(h.settings.len() - 1));
                h.settings.get(idx).map(|s| {
                    let mut c = s.correction.clone();
                    c.film_type = new_film_type;
                    c
                })
            })
        } else if let Some(preset_idx) = self.selected_preset {
            self.available_presets.get(preset_idx).and_then(|p| {
                std::fs::read_to_string(&p.path)
                    .ok()
                    .and_then(|xml| flexcolor::parse_settings_xml(&xml))
                    .map(|mut c| { c.film_type = new_film_type; c })
            })
        } else {
            detail.edit_history.as_ref().and_then(|h| {
                if h.settings.is_empty() { return None; }
                let idx = self.embedded_correction_index
                    .unwrap_or(h.current_index.min(h.settings.len() - 1));
                h.settings.get(idx).map(|s| {
                    let mut c = s.correction.clone();
                    c.film_type = new_film_type;
                    c
                })
            })
        };

        if let Some(ref correction) = correction {
            log::info!("reprocess_with_film_type: applying film processing with film_type={}, highlight={:?}",
                correction.film_type, correction.highlight);
            result = color::apply_film_processing(&result, correction);

            let scanner_rgb = to_rgb16(&result);

            // 重新提取负片胶片曲线
            if correction.film_type == 1 || correction.film_type == 2 {
                if let Some(detail) = &self.detail {
                    if let Some((thumb_8, preview_16)) = detail.tiff.decode_thumbnail_pair() {
                        self.extracted_film_lut = color::extract_film_curve(
                            &thumb_8, &preview_16, correction,
                        );
                    }
                }
            } else {
                self.extracted_film_lut = None;
            }

            // 存储 scanner 空间数据（反转后、色阶前）
            if let Some(detail) = &mut self.detail {
                detail.raw_rgb = Some(scanner_rgb.clone());
                detail.base_rgb = Some(scanner_rgb);
            }

            // 从色彩方案加载色阶到手柄（负片自动翻转）
            Self::load_levels_from_correction(&mut self.manual_adjust, correction);
            self.manual_adjust.film_curve = correction.film_curve;
            self.manual_adjust.film_gamma = correction.gamma;
        } else {
            log::warn!("reprocess_with_film_type: no correction found");
            let rgb16 = to_rgb16(&result);
            if let Some(detail) = &mut self.detail {
                detail.raw_rgb = Some(rgb16.clone());
                detail.base_rgb = Some(rgb16);
            }
        }

        self.histogram_needs_update = true;
        self.rebuild_texture_from_base(ctx);
    }

    /// 新管线：raw_rgb（scanner 空间）→ 渐变曲线 → 扫描仪色阶 → ICC → 显示调整 → 纹理。
    /// 色阶（film_curve + levels + gamma）在 ICC 之前应用，确保正确的色彩映射。
    pub(super) fn rebuild_texture_from_base(&mut self, ctx: &egui::Context) {
        let Some(detail) = &mut self.detail else { return };

        let source = detail.raw_rgb.as_ref().or(detail.base_rgb.as_ref());
        let Some(base) = source else { return };

        // 统一色彩管线（渐变曲线 → 色阶 → ICC → 显示调整）
        let adjusted = color::apply_color_pipeline(
            image::DynamicImage::ImageRgb16(base.clone()),
            &self.manual_adjust,
            &self.curve_points,
            self.extracted_film_lut.as_ref(),
            self.active_icc_data.as_deref(),
            self.target_color_space,
        );
        let rgb16 = to_rgb16(&adjusted);

        // 从最终渲染结果计算处理后直方图
        let mut proc_hist = Box::new([[0u32; 256]; 4]);
        for pixel in rgb16.pixels() {
            let [r16, g16, b16] = pixel.0;
            proc_hist[0][r16 as usize >> 8] += 1;
            proc_hist[1][g16 as usize >> 8] += 1;
            proc_hist[2][b16 as usize >> 8] += 1;
        }
        // RGB 合成：三通道叠加，每个 bin 取 max(R,G,B)
        for i in 0..256 {
            proc_hist[3][i] = proc_hist[0][i].max(proc_hist[1][i]).max(proc_hist[2][i]);
        }
        self.histogram_processed = Some(proc_hist);

        detail.texture = Some(texture_from_16bit(&rgb16, ctx));
    }

    /// 将当前 `manual_adjust` 中的色阶手柄保存到两组存储（raw/processed 始终同步）。
    #[allow(dead_code)]
    fn save_levels_to_source(&mut self) {
        let levels = HistogramLevels {
            black: self.manual_adjust.levels_black,
            gamma: self.manual_adjust.levels_gamma,
            white: self.manual_adjust.levels_white,
        };
        self.levels_processed = levels.clone();
        self.levels_raw = levels;
    }

    /// 从存储恢复色阶手柄到 `manual_adjust`（两组存储始终同步，使用 processed）。
    #[allow(dead_code)]
    fn load_levels_from_source(&mut self) {
        self.manual_adjust.levels_black = self.levels_processed.black;
        self.manual_adjust.levels_gamma = self.levels_processed.gamma;
        self.manual_adjust.levels_white = self.levels_processed.white;
    }

    /// 计算原始直方图（用于色阶调整的自动功能和显示）。
    /// 直方图始终基于 raw_rgb（原始数据，不含渐变曲线），不受曲线调整影响。
    /// 处理后直方图在 rebuild_texture_from_base() 中从渲染结果计算。
    pub(super) fn compute_histogram(&mut self) {
        let Some(detail) = &self.detail else {
            self.histogram_raw = None;
            self.histogram_raw_16 = None;
            return;
        };

        // 直方图始终基于 raw_rgb（ICC + 胶片处理，不含渐变曲线），
        // 反映原始像素分布，不受曲线调整影响。
        let source_img = detail.raw_rgb.as_ref().or(detail.base_rgb.as_ref());

        let Some(base) = source_img else {
            self.histogram_raw = None;
            self.histogram_raw_16 = None;
            return;
        };

        // 65536-bin 精确直方图（R/G/B 三通道）
        let mut hist_16: Vec<Vec<u32>> = vec![vec![0u32; 65536]; 3];
        for pixel in base.pixels() {
            let [r16, g16, b16] = pixel.0;
            {
                hist_16[0][r16 as usize] += 1;
                hist_16[1][g16 as usize] += 1;
                hist_16[2][b16 as usize] += 1;
            }
        }

        // 派生 256-bin 显示用直方图
        let mut hist = Box::new([[0u32; 256]; 4]);
        for ch in 0..3 {
            for i in 0..65536 {
                hist[ch][i >> 8] += hist_16[ch][i];
            }
        }
        // RGB 合成直方图：三通道叠加，每个 bin 取 max(R,G,B) 计数
        // 这样 R/G/B 任一通道有高度的区域 RGB 通道都会有高度
        for i in 0..256 {
            hist[3][i] = hist[0][i].max(hist[1][i]).max(hist[2][i]);
        }

        // 诊断日志
        for ch in 0..3 {
            let min_8 = hist[ch].iter().position(|&c| c > 0).unwrap_or(0);
            let max_8 = hist[ch].iter().rposition(|&c| c > 0).unwrap_or(0);
            let ch_name = ["R", "G", "B"][ch];
            log::info!("Raw histogram {}: 256bin=[{}-{}]",
                ch_name, min_8, max_8);
        }

        self.histogram_raw = Some(hist);
        self.histogram_raw_16 = Some(hist_16);
        self.histogram_needs_update = false;
    }

    /// 根据 65536-bin 直方图按指定百分位计算黑白点（返回 0-255 浮点精度值）。
    /// `black_pct`：黑点裁切百分比（如 0.05 表示 0.05%）。
    /// `white_pct`：白点裁切百分比（如 0.1 表示 0.1%）。
    pub(super) fn auto_percentile_levels(hist: &[u32], black_pct: f32, white_pct: f32) -> (f32, f32) {
        let total: u64 = hist.iter().map(|&c| c as u64).sum();
        if total == 0 { return (0.0, 255.0); }
        let bins = hist.len() as f32;
        let scale = 255.0 / (bins - 1.0); // 映射到 0-255 空间

        let lo_target = ((total as f64 * black_pct as f64 / 100.0) as u64).max(1);
        let hi_target = ((total as f64 * white_pct as f64 / 100.0) as u64).max(1);

        let mut b = 0.0f32;
        let mut cumsum = 0u64;
        for (i, &count) in hist.iter().enumerate() {
            cumsum += count as u64;
            if cumsum >= lo_target { b = i as f32 * scale; break; }
        }

        let mut w = 255.0f32;
        cumsum = 0;
        for (i, &count) in hist.iter().enumerate().rev() {
            cumsum += count as u64;
            if cumsum >= hi_target { w = i as f32 * scale; break; }
        }
        (b, w.max(b + 0.1))
    }

    /// 渲染单通道色阶区段：直方图 + 渐变轨道 + 可拖拽黑/灰/白三角手柄。
    /// `auto_bw`：从 65536-bin 直方图预计算的精确黑白点 (black, white)。
    /// `is_master`：true 时 Gamma 显示为 0.01-3.00（左高右低），false 时显示为 0-255 位置值。
    /// 返回 true 表示有值被修改。
    pub(super) fn render_levels_section(
        ui: &mut egui::Ui,
        section_id: egui::Id,
        title: &str,
        hist: Option<&[u32; 256]>,
        bar_color: egui::Color32,
        auto_bw: Option<(f32, f32)>,
        black: &mut f32,
        gamma: &mut f32,
        white: &mut f32,
        hist_scale: config::HistogramScale,
        is_master: bool,
    ) -> bool {
        let mut changed = false;
        let avail_w = ui.available_width();

        // Title row with "A" (auto-levels) button on the right
        let auto_clicked = ui.horizontal(|ui| {
            ui.label(egui::RichText::new(title).small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::Button::new(egui::RichText::new("A").small())
                    .min_size(egui::vec2(16.0, 0.0)))
                    .on_hover_text("Auto-set levels (16-bit precision)")
                    .clicked()
            }).inner
        }).inner;

        if auto_clicked {
            if let Some((b, w)) = auto_bw {
                *black = b;
                *white = w;
                *gamma = 1.0;
                changed = true;
            }
        }

        // ── Histogram bars ───────────────────────────────────────────────
        let hist_h = 55.0_f32;
        let (hist_rect, _) = ui.allocate_exact_size(egui::vec2(avail_w, hist_h), egui::Sense::hover());
        let painter = ui.painter_at(hist_rect);
        painter.rect_filled(hist_rect, 2.0, egui::Color32::from_gray(18));

        if let Some(h_arr) = hist {
            // Highlight the actual data range with a slightly lighter background
            let first_nz = h_arr.iter().position(|&c| c > 0);
            let last_nz  = h_arr.iter().rposition(|&c| c > 0);
            if let (Some(lo), Some(hi)) = (first_nz, last_nz) {
                let x1 = hist_rect.left() + lo as f32 / 255.0 * hist_rect.width();
                let x2 = hist_rect.left() + (hi + 1) as f32 / 255.0 * hist_rect.width();
                painter.rect_filled(
                    egui::Rect::from_x_y_ranges(x1..=x2, hist_rect.top()..=hist_rect.bottom()),
                    0.0,
                    egui::Color32::from_gray(28),
                );
            }

            let max_v = h_arr.iter().copied().max().unwrap_or(1).max(1) as f32;
            let w = hist_rect.width();
            let bar_w = (w / 256.0).max(1.0);
            let scale = hist_scale;
            for i in 0..256 {
                let count = h_arr[i] as f32;
                if count < 0.5 { continue; }
                let bar_h = (scale.map(count, max_v) * hist_h).ceil().min(hist_h);
                let x = hist_rect.left() + i as f32 / 255.0 * w;
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, hist_rect.bottom() - bar_h),
                        egui::vec2(bar_w, bar_h),
                    ),
                    0.0,
                    bar_color,
                );
            }
        }

        // Marker lines on histogram for current black/white points
        let bx_hist = hist_rect.left() + *black / 255.0 * hist_rect.width();
        let wx_hist = hist_rect.left() + *white / 255.0 * hist_rect.width();
        painter.line_segment(
            [egui::pos2(bx_hist, hist_rect.top()), egui::pos2(bx_hist, hist_rect.bottom())],
            egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(80, 130, 255, 220)),
        );
        painter.line_segment(
            [egui::pos2(wx_hist, hist_rect.top()), egui::pos2(wx_hist, hist_rect.bottom())],
            egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 230, 50, 220)),
        );

        // ── Gradient track ───────────────────────────────────────────────
        let track_h = 22.0_f32;
        let (track_rect, _) = ui.allocate_exact_size(egui::vec2(avail_w, track_h), egui::Sense::hover());
        let track_painter = ui.painter_at(track_rect);

        // Draw black→white gradient strip
        let steps = 64u32;
        let step_w = track_rect.width() / steps as f32;
        for i in 0..steps {
            let t = i as f32 / (steps - 1) as f32;
            let gray = (t * 255.0) as u8;
            track_painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(track_rect.left() + i as f32 * step_w, track_rect.top()),
                    egui::vec2(step_w + 0.5, track_h),
                ),
                0.0,
                egui::Color32::from_gray(gray),
            );
        }

        // Triangle handle positions
        let bx = track_rect.left() + *black / 255.0 * track_rect.width();
        let wx = track_rect.left() + *white / 255.0 * track_rect.width();
        // 统一公式：t = 0.5^(1/gamma)，gamma=1.0 → t=0.5（中性）
        let t_gamma = 0.5_f32.powf(1.0 / (*gamma).max(0.01));
        let gx = bx + t_gamma * (wx - bx);
        let center_y = track_rect.center().y;
        let tri_h = track_h * 0.75;
        let tri_w = tri_h * 0.8;

        // Draw gamma handle first (sits between black and white)
        track_painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(gx, center_y - tri_h / 2.0),
                egui::pos2(gx - tri_w / 2.0, center_y + tri_h / 2.0),
                egui::pos2(gx + tri_w / 2.0, center_y + tri_h / 2.0),
            ],
            egui::Color32::from_gray(190),
            egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
        ));
        // Black handle
        track_painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(bx, center_y - tri_h / 2.0),
                egui::pos2(bx - tri_w / 2.0, center_y + tri_h / 2.0),
                egui::pos2(bx + tri_w / 2.0, center_y + tri_h / 2.0),
            ],
            egui::Color32::from_rgb(30, 60, 180),
            egui::Stroke::new(1.0, egui::Color32::WHITE),
        ));
        // White handle
        track_painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(wx, center_y - tri_h / 2.0),
                egui::pos2(wx - tri_w / 2.0, center_y + tri_h / 2.0),
                egui::pos2(wx + tri_w / 2.0, center_y + tri_h / 2.0),
            ],
            egui::Color32::from_rgb(240, 215, 50),
            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
        ));

        // ── Drag interaction ─────────────────────────────────────────────
        let hit_w = tri_w + 6.0;

        let r_white = ui.interact(
            egui::Rect::from_center_size(egui::pos2(wx, center_y), egui::vec2(hit_w, track_h)),
            section_id.with("w"),
            egui::Sense::drag(),
        );
        if r_white.dragged() {
            *white = (*white + r_white.drag_delta().x / track_rect.width() * 255.0)
                .clamp(*black + 0.1, 255.0);
            changed = true;
        }

        let r_black = ui.interact(
            egui::Rect::from_center_size(egui::pos2(bx, center_y), egui::vec2(hit_w, track_h)),
            section_id.with("b"),
            egui::Sense::drag(),
        );
        if r_black.dragged() {
            *black = (*black + r_black.drag_delta().x / track_rect.width() * 255.0)
                .clamp(0.0, *white - 0.1);
            changed = true;
        }

        let r_gamma = ui.interact(
            egui::Rect::from_center_size(egui::pos2(gx, center_y), egui::vec2(hit_w, track_h)),
            section_id.with("g"),
            egui::Sense::drag(),
        );
        if r_gamma.dragged() {
            let range = (wx - bx).max(1.0);
            let new_gx = (gx + r_gamma.drag_delta().x).clamp(bx + 1.0, wx - 1.0);
            let t = (new_gx - bx) / range;
            if t > 0.001 && t < 0.999 {
                let gamma_max = if is_master { 3.00 } else { 99.0 };
                *gamma = (0.5_f32.ln() / t.ln()).clamp(0.01, gamma_max);
            }
            changed = true;
        }

        // ── Compact numeric inputs below the track ───────────────────────
        let max_black = (*white - 0.1).max(0.0);
        let min_white = (*black + 0.1).min(255.0);
        ui.columns(3, |cols| {
            if cols[0].add(
                egui::DragValue::new(black).range(0.0..=max_black).max_decimals(2).speed(0.25),
            ).on_hover_text("Black point").changed() { changed = true; }

            if is_master {
                // RGB Master: 显示 Gamma 值，范围 0.01-3.00（左=3.00亮, 右=0.01暗）
                if cols[1].add(
                    egui::DragValue::new(gamma).range(0.01..=3.00).max_decimals(2).speed(0.01),
                ).on_hover_text("Midtone gamma").changed() { changed = true; }
            } else {
                // R/G/B 通道: 显示 Gray 值（gamma × 128），范围 0-255
                let mut gray_val = (*gamma * 128.0).clamp(0.0, 255.0);
                let resp = cols[1].add(
                    egui::DragValue::new(&mut gray_val).range(0.0..=255.0).max_decimals(0).speed(0.5),
                );
                if resp.on_hover_text("Midtone gray").changed() {
                    *gamma = (gray_val / 128.0).clamp(0.01, 99.0);
                    changed = true;
                }
            }

            if cols[2].add(
                egui::DragValue::new(white).range(min_white..=255.0).max_decimals(2).speed(0.25),
            ).on_hover_text("White point").changed() { changed = true; }
        });

        changed
    }

    /// 渲染曲线编辑器：通道选择 + 可视化曲线图 + 控制点拖拽交互。
    /// 返回 true 表示曲线数据已修改，需要重建纹理。
    fn render_curve_editor(
        ui: &mut egui::Ui,
        curve_points: &mut Vec<Vec<(i64, i64, i64)>>,
        curve_channel: &mut usize,
        curve_dragging: &mut Option<usize>,
        reset_label: &str,
    ) -> bool {
        let mut changed = false;

        // ── 通道选择按钮 ──
        ui.horizontal(|ui| {
            let channels = ["RGB", "R", "G", "B", "C", "M", "Y"];
            let colors = [
                egui::Color32::from_gray(200),
                egui::Color32::from_rgb(255, 80, 80),
                egui::Color32::from_rgb(80, 200, 80),
                egui::Color32::from_rgb(80, 120, 255),
                egui::Color32::from_rgb(0, 200, 200),
                egui::Color32::from_rgb(200, 0, 200),
                egui::Color32::from_rgb(200, 200, 0),
            ];
            for (i, &label) in channels.iter().enumerate() {
                let selected = *curve_channel == i;
                let text = egui::RichText::new(label).small();
                let text = if selected { text.color(colors[i]) } else { text };
                if ui.selectable_label(selected, text).clicked() {
                    *curve_channel = i;
                    *curve_dragging = None;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(reset_label).clicked() {
                    let ch = *curve_channel;
                    if ch < curve_points.len() {
                        curve_points[ch] = vec![(0, 0, 0), (255, 255, 0)];
                        changed = true;
                    }
                }
            });
        });

        // ── 曲线图绘制 ──
        let avail_w = ui.available_width();
        let graph_size = avail_w.min(256.0).max(120.0);
        let (graph_rect, response) = ui.allocate_exact_size(
            egui::vec2(graph_size, graph_size),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(graph_rect);

        // 背景
        painter.rect_filled(graph_rect, 2.0, egui::Color32::from_gray(24));

        // 网格线（4×4）
        let grid_color = egui::Color32::from_gray(45);
        for i in 1..4 {
            let frac = i as f32 / 4.0;
            let x = graph_rect.left() + frac * graph_rect.width();
            let y = graph_rect.top() + frac * graph_rect.height();
            painter.line_segment(
                [egui::pos2(x, graph_rect.top()), egui::pos2(x, graph_rect.bottom())],
                egui::Stroke::new(0.5, grid_color),
            );
            painter.line_segment(
                [egui::pos2(graph_rect.left(), y), egui::pos2(graph_rect.right(), y)],
                egui::Stroke::new(0.5, grid_color),
            );
        }

        // 对角线（恒等映射参考线）
        painter.line_segment(
            [egui::pos2(graph_rect.left(), graph_rect.bottom()),
             egui::pos2(graph_rect.right(), graph_rect.top())],
            egui::Stroke::new(0.5, egui::Color32::from_gray(60)),
        );

        let ch = *curve_channel;
        if ch >= curve_points.len() { return changed; }

        // 曲线通道颜色
        let curve_color = match ch {
            1 => egui::Color32::from_rgb(255, 80, 80),
            2 => egui::Color32::from_rgb(80, 200, 80),
            3 => egui::Color32::from_rgb(80, 120, 255),
            4 => egui::Color32::from_rgb(0, 200, 200),
            5 => egui::Color32::from_rgb(200, 0, 200),
            6 => egui::Color32::from_rgb(200, 200, 0),
            _ => egui::Color32::from_gray(220),
        };

        // 绘制曲线（用 LUT 生成平滑曲线）
        let lut = color::build_curve_lut(&curve_points[ch]);
        let mut curve_line: Vec<egui::Pos2> = Vec::with_capacity(256);
        for i in 0..256 {
            let x = graph_rect.left() + i as f32 / 255.0 * graph_rect.width();
            let y = graph_rect.bottom() - lut[i] as f32 / 255.0 * graph_rect.height();
            curve_line.push(egui::pos2(x, y));
        }
        // 逐段绘制曲线
        for pair in curve_line.windows(2) {
            painter.line_segment([pair[0], pair[1]], egui::Stroke::new(1.5, curve_color));
        }

        // 绘制控制点
        let point_radius = 4.0;
        let pts = &curve_points[ch];
        for (i, &(px, py, _)) in pts.iter().enumerate() {
            let x = graph_rect.left() + px as f32 / 255.0 * graph_rect.width();
            let y = graph_rect.bottom() - py as f32 / 255.0 * graph_rect.height();
            let is_dragging = *curve_dragging == Some(i);
            let fill = if is_dragging { egui::Color32::WHITE } else { curve_color };
            painter.circle_filled(egui::pos2(x, y), point_radius, fill);
            painter.circle_stroke(
                egui::pos2(x, y), point_radius,
                egui::Stroke::new(1.0, egui::Color32::from_gray(180)),
            );
        }

        // ── 交互：拖拽控制点 / 添加 / 删除 ──
        let to_curve = |pos: egui::Pos2| -> (i64, i64) {
            let x = ((pos.x - graph_rect.left()) / graph_rect.width() * 255.0)
                .round().clamp(0.0, 255.0) as i64;
            let y = ((graph_rect.bottom() - pos.y) / graph_rect.height() * 255.0)
                .round().clamp(0.0, 255.0) as i64;
            (x, y)
        };

        let hit_radius = 10.0;
        let find_closest_point = |pos: egui::Pos2, pts: &[(i64, i64, i64)]| -> Option<usize> {
            let mut best_idx = None;
            let mut best_dist = f32::MAX;
            for (i, &(px, py, _)) in pts.iter().enumerate() {
                let x = graph_rect.left() + px as f32 / 255.0 * graph_rect.width();
                let y = graph_rect.bottom() - py as f32 / 255.0 * graph_rect.height();
                let dist = pos.distance(egui::pos2(x, y));
                if dist < hit_radius && dist < best_dist {
                    best_dist = dist;
                    best_idx = Some(i);
                }
            }
            best_idx
        };

        // 右键或双击删除控制点
        if response.double_clicked() || response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(idx) = find_closest_point(pos, &curve_points[ch]) {
                    // 不允许删除头尾端点
                    if idx > 0 && idx < curve_points[ch].len() - 1 {
                        curve_points[ch].remove(idx);
                        *curve_dragging = None;
                        changed = true;
                    }
                }
            }
        }

        // 开始拖拽
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                *curve_dragging = find_closest_point(pos, &curve_points[ch]);

                // 如果没有命中已有点，在曲线上添加新控制点
                if curve_dragging.is_none() && graph_rect.contains(pos) {
                    let (cx, cy) = to_curve(pos);
                    // 按 x 坐标插入，保持排序
                    let insert_pos = curve_points[ch]
                        .iter()
                        .position(|&(x, _, _)| x > cx)
                        .unwrap_or(curve_points[ch].len());
                    curve_points[ch].insert(insert_pos, (cx, cy, 0));
                    *curve_dragging = Some(insert_pos);
                    changed = true;
                }
            }
        }

        // 拖拽中
        if response.dragged() {
            if let Some(drag_idx) = *curve_dragging {
                if let Some(pos) = response.interact_pointer_pos() {
                    let (mut cx, cy) = to_curve(pos);
                    let pts = &curve_points[ch];

                    // 限制拖拽范围：不能越过相邻控制点
                    // 起点和终点仅锁定 x 坐标，y 坐标可自由调整
                    if drag_idx == 0 {
                        cx = 0; // 起点 x 固定为 0
                    } else if drag_idx == pts.len() - 1 {
                        cx = 255; // 终点 x 固定为 255
                    } else {
                        let x_min = pts[drag_idx - 1].0 + 1;
                        let x_max = pts[drag_idx + 1].0 - 1;
                        cx = cx.clamp(x_min, x_max);
                    }

                    curve_points[ch][drag_idx] = (cx, cy, 0);
                    changed = true;
                }
            }
        }

        // 结束拖拽
        if response.drag_stopped() {
            *curve_dragging = None;
        }

        changed
    }

    /// 仅渲染直方图条形图（无色阶控制），用于处理后直方图的只读显示。
    fn render_histogram_bars(
        ui: &mut egui::Ui,
        hist: Option<&[u32; 256]>,
        bar_color: egui::Color32,
        hist_scale: config::HistogramScale,
    ) {
        let avail_w = ui.available_width();
        let hist_h = 40.0_f32;
        let (hist_rect, _) = ui.allocate_exact_size(egui::vec2(avail_w, hist_h), egui::Sense::hover());
        let painter = ui.painter_at(hist_rect);
        painter.rect_filled(hist_rect, 2.0, egui::Color32::from_gray(18));

        if let Some(h_arr) = hist {
            let max_v = h_arr.iter().copied().max().unwrap_or(1).max(1) as f32;
            let w = hist_rect.width();
            let bar_w = (w / 256.0).max(1.0);
            for i in 0..256 {
                let count = h_arr[i] as f32;
                if count < 0.5 { continue; }
                let bar_h = (hist_scale.map(count, max_v) * hist_h).ceil().min(hist_h);
                let x = hist_rect.left() + i as f32 / 255.0 * w;
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, hist_rect.bottom() - bar_h),
                        egui::vec2(bar_w, bar_h),
                    ),
                    0.0,
                    bar_color,
                );
            }
        }
    }

    /// 渲染手动色彩调整面板：四通道色阶 + 曝光/对比度/高光/阴影/饱和度/色彩平衡滑块
    pub(super) fn render_color_adjust_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let s = self.s();

        if self.detail.is_none() {
            ui.label(s.select_image);
            return;
        }

        let mut rebuild = false;

        let adjust_heading = s.adjust_heading;
        let reset_adjust = s.reset_adjust;
        let exposure_str = s.exposure;
        let brightness_str = s.brightness;
        let lightness_str = s.lightness;
        let midtone_str = s.midtone_adj;
        let contrast_str = s.contrast;
        let highlights_str = s.highlights;
        let shadows_str = s.shadows;
        let saturation_str = s.saturation_label;
        let color_balance_str = s.color_balance;
        let hist_rgb = s.hist_rgb;
        let hist_r = s.hist_r;
        let hist_g = s.hist_g;
        let hist_b = s.hist_b;

        // ── Header + reset (not scrollable) ────────────────────────────
        ui.heading(adjust_heading);
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(reset_adjust).clicked() {
                    // 重置到色彩方案加载后的基线状态（包括曲线控制点）
                    self.manual_adjust = self.baseline_adjust.clone();
                    self.levels_processed = self.baseline_levels_processed.clone();
                    self.levels_raw = self.baseline_levels_raw.clone();
                    self.curve_points = self.baseline_curve_points.clone();
                    self.curve_channel = 0;
                    self.curve_dragging = None;
                    rebuild = true;
                }
            });
        });

        ui.add_space(4.0);

        // ── 胶片类型选择 ────────────────────────────────────────────────
        {
            let film_type_label = match self.manual_adjust.film_type {
                0 => s.film_type_positive,
                1 => s.film_type_negative,
                2 => s.film_type_bw_negative,
                _ => "?",
            };
            ui.horizontal(|ui| {
                ui.label(s.film_type_label);
                egui::ComboBox::from_id_salt("film_type_combo")
                    .selected_text(film_type_label)
                    .width(ui.available_width() - 16.0)
                    .show_ui(ui, |ui| {
                        for (val, label) in [
                            (0i64, s.film_type_positive),
                            (1, s.film_type_negative),
                            (2, s.film_type_bw_negative),
                        ] {
                            if ui.selectable_value(&mut self.manual_adjust.film_type, val, label).clicked() {
                                // 胶片类型变更需要重新处理整个管线
                                self.reprocess_with_film_type(ctx);
                                rebuild = false; // reprocess 已经重建纹理
                            }
                        }
                    });
            });
        }

        ui.add_space(4.0);

        // 数据源变更后重算原始直方图
        if self.histogram_needs_update {
            self.compute_histogram();
        }

        // 读取原始直方图数据
        let hist_raw_data: Option<[[u32; 256]; 4]> = self.histogram_raw.as_deref().copied();

        // 从 65536-bin 直方图预计算各通道精确黑白点
        let clip_pct = (
            self.app_config.auto_levels_black_pct,
            self.app_config.auto_levels_white_pct,
        );
        let hist_ch_order = [3usize, 0, 1, 2];
        let auto_bw: [Option<(f32, f32)>; 4] = if let Some(ref h16) = self.histogram_raw_16 {
            std::array::from_fn(|i| {
                if hist_ch_order[i] == 3 {
                    // RGB master: 从 R/G/B 三通道取 min(shadow) / max(highlight)
                    let (b0, w0) = Self::auto_percentile_levels(&h16[0], clip_pct.0, clip_pct.1);
                    let (b1, w1) = Self::auto_percentile_levels(&h16[1], clip_pct.0, clip_pct.1);
                    let (b2, w2) = Self::auto_percentile_levels(&h16[2], clip_pct.0, clip_pct.1);
                    Some((b0.min(b1).min(b2), w0.max(w1).max(w2)))
                } else {
                    Some(Self::auto_percentile_levels(&h16[hist_ch_order[i]], clip_pct.0, clip_pct.1))
                }
            })
        } else {
            [None; 4]
        };

        ui.add_space(2.0);

        // ── 整个调整区域放入可滚动容器 ──
        egui::ScrollArea::vertical().id_salt("adjust_scroll").show(ui, |ui| {

        // ── 原始直方图色阶（可调整） ──
        if ui.checkbox(&mut self.manual_adjust.apply_levels, s.histogram_levels).changed() {
            rebuild = true;
        }
        // ── 胶片曲线开关 ──
        if ui.checkbox(&mut self.manual_adjust.apply_film_curve, s.film_curve).changed() {
            rebuild = true;
        }

        let sections: [(&str, usize, egui::Color32); 4] = [
            (hist_rgb, 3usize, egui::Color32::from_gray(160)),
            (hist_r,   0usize, egui::Color32::from_rgb(200, 50, 50)),
            (hist_g,   1usize, egui::Color32::from_rgb(50, 180, 50)),
            (hist_b,   2usize, egui::Color32::from_rgb(60, 100, 220)),
        ];
        let levels_idx = [0usize, 1usize, 2usize, 3usize];

        // 保存当前值用于检测联动变化
        let prev_black = self.manual_adjust.levels_black;
        let prev_white = self.manual_adjust.levels_white;

        for (section_pos, (title, hist_ch, bar_color)) in sections.iter().enumerate() {
            let lvl_idx = levels_idx[section_pos];
            let hist = hist_raw_data.as_ref().map(|hd| &hd[*hist_ch]);
            let section_id = ui.id().with(section_pos);
            // RGB 通道默认展开，其余默认折叠
            let default_open = section_pos == 0;
            egui::CollapsingHeader::new(egui::RichText::new(*title).small().strong())
                .id_salt(format!("levels_{}", section_pos))
                .default_open(default_open)
                .show(ui, |ui| {
                    if Self::render_levels_section(
                        ui,
                        section_id,
                        title,
                        hist,
                        *bar_color,
                        auto_bw[section_pos],
                        &mut self.manual_adjust.levels_black[lvl_idx],
                        &mut self.manual_adjust.levels_gamma[lvl_idx],
                        &mut self.manual_adjust.levels_white[lvl_idx],
                        config::HistogramScale::from_str(&self.app_config.histogram_scale),
                        section_pos == 0, // is_master: 第一个 section 是 RGB Master
                    ) {
                        rebuild = true;
                    }
                });
        }

        // ── FlexColor 式联动：Master ↔ R/G/B shadow/highlight ──
        if rebuild {
            let b = &mut self.manual_adjust.levels_black;
            let w = &mut self.manual_adjust.levels_white;

            // Master (索引 0) 发生变化 → 同步偏移 R(1)/G(2)/B(3)
            let db = b[0] - prev_black[0];
            let dw = w[0] - prev_white[0];
            if db.abs() > 0.001 {
                for ch in 1..=3 {
                    b[ch] = (b[ch] + db).clamp(0.0, w[ch] - 0.1);
                }
            }
            if dw.abs() > 0.001 {
                for ch in 1..=3 {
                    w[ch] = (w[ch] + dw).clamp(b[ch] + 0.1, 255.0);
                }
            }

            // R/G/B 发生变化 → Master 更新为 min(shadow) / max(highlight)
            let any_ch_black_changed = (1..=3).any(|ch| (b[ch] - prev_black[ch]).abs() > 0.001);
            let any_ch_white_changed = (1..=3).any(|ch| (w[ch] - prev_white[ch]).abs() > 0.001);
            if any_ch_black_changed && db.abs() < 0.001 {
                b[0] = b[1].min(b[2]).min(b[3]);
            }
            if any_ch_white_changed && dw.abs() < 0.001 {
                w[0] = w[1].max(w[2]).max(w[3]);
            }
        }

        // ── 输出色阶 (DotColor) ──
        ui.add_space(2.0);
        egui::CollapsingHeader::new(egui::RichText::new(s.output_levels).small().strong())
            .id_salt("output_levels")
            .default_open(true)
            .show(ui, |ui| {
                let avail_w = ui.available_width();
                // 渐变条：从 output_shadow 到 output_highlight
                let track_h = 18.0_f32;
                let (track_rect, _) = ui.allocate_exact_size(egui::vec2(avail_w, track_h), egui::Sense::hover());
                let painter = ui.painter_at(track_rect);
                let steps = 64u32;
                let step_w = track_rect.width() / steps as f32;
                let os = self.manual_adjust.output_shadow;
                let oh = self.manual_adjust.output_highlight;
                for i in 0..steps {
                    let t = i as f32 / (steps - 1) as f32;
                    let gray = (os + t * (oh - os)).clamp(0.0, 255.0) as u8;
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(track_rect.left() + i as f32 * step_w, track_rect.top()),
                            egui::vec2(step_w + 0.5, track_h),
                        ),
                        0.0,
                        egui::Color32::from_gray(gray),
                    );
                }

                ui.columns(2, |cols| {
                    cols[0].horizontal(|ui| {
                        ui.label(egui::RichText::new(s.output_shadow).small());
                        if ui.add(
                            egui::DragValue::new(&mut self.manual_adjust.output_shadow)
                                .range(0.0..=255.0).max_decimals(0).speed(0.5),
                        ).changed() {
                            rebuild = true;
                        }
                    });
                    cols[1].horizontal(|ui| {
                        ui.label(egui::RichText::new(s.output_highlight).small());
                        if ui.add(
                            egui::DragValue::new(&mut self.manual_adjust.output_highlight)
                                .range(0.0..=255.0).max_decimals(0).speed(0.5),
                        ).changed() {
                            rebuild = true;
                        }
                    });
                });
            });

        ui.separator();
        ui.add_space(2.0);

        // ── 处理后直方图（仅显示，不可调整） ──
        let hist_proc_data: Option<[[u32; 256]; 4]> = self.histogram_processed.as_deref().copied();
        let hist_scale = config::HistogramScale::from_str(&self.app_config.histogram_scale);
        egui::CollapsingHeader::new(egui::RichText::new(s.histogram_output).small().strong())
            .default_open(false)
            .show(ui, |ui| {
                for (_section_pos, (_title, hist_ch, bar_color)) in sections.iter().enumerate() {
                    let hist = hist_proc_data.as_ref().map(|hd| &hd[*hist_ch]);
                    Self::render_histogram_bars(ui, hist, *bar_color, hist_scale);
                    ui.add_space(2.0);
                }
            });

        ui.separator();
        ui.add_space(4.0);

        // ── Basic adjustment sliders ───────────────────────
        {
            // 渐变曲线开关（直方图始终显示原始数据，不随曲线变化）
            if ui.checkbox(&mut self.manual_adjust.apply_curves, s.gradation_curves).changed() {
                rebuild = true;
            }

            // 曲线编辑器（仅当曲线开启时显示）
            if self.manual_adjust.apply_curves {
                if Self::render_curve_editor(
                    ui,
                    &mut self.curve_points,
                    &mut self.curve_channel,
                    &mut self.curve_dragging,
                    s.curve_reset,
                ) {
                    rebuild = true;
                }
            }
            ui.add_space(4.0);

            let adj = &mut self.manual_adjust;

            // 曝光
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_exposure, "").changed() { rebuild = true; }
                ui.label(exposure_str);
            });
            if ui.add(egui::Slider::new(&mut adj.exposure, -3.0..=3.0).step_by(0.05).text("stops")).changed() {
                rebuild = true;
            }

            // 亮度
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_brightness, "").changed() { rebuild = true; }
                ui.label(brightness_str);
            });
            if ui.add(egui::Slider::new(&mut adj.brightness, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            // 阴影深度
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_shadow_depth, "").changed() { rebuild = true; }
                ui.label(lightness_str);
            });
            if ui.add(egui::Slider::new(&mut adj.lightness, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            // 中间调
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_midtone, "").changed() { rebuild = true; }
                ui.label(midtone_str);
            });
            if ui.add(egui::Slider::new(&mut adj.midtone, 0.1..=4.0).step_by(0.01).text("")).changed() {
                rebuild = true;
            }

            // 对比度
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_contrast, "").changed() { rebuild = true; }
                ui.label(contrast_str);
            });
            if ui.add(egui::Slider::new(&mut adj.contrast, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            // 高光
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_highlights, "").changed() { rebuild = true; }
                ui.label(highlights_str);
            });
            if ui.add(egui::Slider::new(&mut adj.highlights, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            // 阴影
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_shadows, "").changed() { rebuild = true; }
                ui.label(shadows_str);
            });
            if ui.add(egui::Slider::new(&mut adj.shadows, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            // 饱和度
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_saturation, "").changed() { rebuild = true; }
                ui.label(saturation_str);
            });
            if ui.add(egui::Slider::new(&mut adj.saturation, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.add_space(8.0);

            // 色温/色调
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_color_temp, "").changed() { rebuild = true; }
                ui.label(egui::RichText::new(s.color_temp).strong());
            });
            if ui.add(egui::Slider::new(&mut adj.color_temperature, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }
            ui.label(s.tint);
            if ui.add(egui::Slider::new(&mut adj.tint, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.add_space(8.0);

            // 色彩平衡
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_color_balance, "").changed() { rebuild = true; }
                ui.label(egui::RichText::new(color_balance_str).strong());
            });

            ui.label("R");
            if ui.add(egui::Slider::new(&mut adj.r_shift, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label("G");
            if ui.add(egui::Slider::new(&mut adj.g_shift, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label("B");
            if ui.add(egui::Slider::new(&mut adj.b_shift, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.add_space(8.0);

            // 色彩校正矩阵
            let cc_labels = ["R", "G", "B", "C", "M", "Y"];
            ui.horizontal(|ui| {
                if ui.checkbox(&mut adj.apply_color_corr, "").changed() { rebuild = true; }
                ui.label(egui::RichText::new(s.apply_cc).strong());
            });
            egui::Grid::new("cc_matrix_grid")
                .striped(true)
                .num_columns(7)
                .show(ui, |ui| {
                    // header row
                    ui.label("");
                    for label in &cc_labels {
                        ui.label(egui::RichText::new(*label).small().strong());
                    }
                    ui.end_row();
                    for row in 0..6 {
                        ui.label(egui::RichText::new(cc_labels[row]).small().strong());
                        for col in 0..6 {
                            let idx = row * 6 + col;
                            let mut val = adj.color_corr[idx] as i32;
                            if ui.add(egui::DragValue::new(&mut val).range(-100..=100).speed(1)).changed() {
                                adj.color_corr[idx] = val as i64;
                                rebuild = true;
                            }
                        }
                        ui.end_row();
                    }
                });

            // ── USM 锐化 / 除尘 / 降噪 / 镜头校正 / 阴影增强 ──
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            egui::CollapsingHeader::new(egui::RichText::new(s.sharpening_usm).strong())
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut adj.apply_usm, "").changed() { rebuild = true; }
                        ui.label(s.enabled);
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.amount);
                        let mut val = adj.usm_amount as i32;
                        if ui.add(egui::Slider::new(&mut val, 0..=500).text("")).changed() {
                            adj.usm_amount = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.radius);
                        let mut val = adj.usm_radius as i32;
                        if ui.add(egui::Slider::new(&mut val, 1..=20).text("")).changed() {
                            adj.usm_radius = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.dark_limit);
                        let mut val = adj.usm_dark_limit as i32;
                        if ui.add(egui::Slider::new(&mut val, 0..=255).text("")).changed() {
                            adj.usm_dark_limit = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.noise_limit);
                        let mut val = adj.usm_noise_limit as i32;
                        if ui.add(egui::Slider::new(&mut val, 0..=255).text("")).changed() {
                            adj.usm_noise_limit = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("⚠ USM sharpening not yet implemented")
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    );
                });

            ui.add_space(4.0);
            egui::CollapsingHeader::new(egui::RichText::new(s.dust_removal).strong())
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut adj.apply_dust, "").changed() { rebuild = true; }
                        ui.label(s.enabled);
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.dust_level);
                        let mut val = adj.dust_level as i32;
                        if ui.add(egui::Slider::new(&mut val, 0..=100).text("")).changed() {
                            adj.dust_level = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("⚠ Dust removal not yet implemented")
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    );
                });

            ui.add_space(4.0);
            egui::CollapsingHeader::new(egui::RichText::new(s.noise_filter).strong())
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut adj.apply_cn_filter, "").changed() { rebuild = true; }
                        ui.label(s.enabled);
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.noise_radius);
                        let mut val = adj.color_noise_radius as i32;
                        if ui.add(egui::Slider::new(&mut val, 0..=20).text("")).changed() {
                            adj.color_noise_radius = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.noise_bias);
                        let mut val = adj.noise_filter_bias as i32;
                        if ui.add(egui::Slider::new(&mut val, -100..=100).text("")).changed() {
                            adj.noise_filter_bias = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("⚠ Noise filter not yet implemented")
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    );
                });

            ui.add_space(4.0);
            egui::CollapsingHeader::new(egui::RichText::new(s.lens_correction).strong())
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(s.lens_correction);
                        let mut val = adj.lens_correction as i32;
                        if ui.add(egui::Slider::new(&mut val, 0..=100).text("")).changed() {
                            adj.lens_correction = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(s.vignette_amount);
                        let mut val = adj.vignette_amount as i32;
                        if ui.add(egui::Slider::new(&mut val, 0..=100).text("")).changed() {
                            adj.vignette_amount = val as i64;
                            rebuild = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("⚠ Lens/vignette correction not yet implemented")
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    );
                });

            ui.add_space(4.0);
            egui::CollapsingHeader::new(egui::RichText::new(s.enhanced_shadow).strong())
                .default_open(false)
                .show(ui, |ui| {
                    if ui.checkbox(&mut adj.enhanced_shadow, s.enhanced_shadow).changed() { rebuild = true; }
                    if ui.checkbox(&mut adj.remove_cast_highlight, s.rm_cast_highlight).changed() { rebuild = true; }
                    if ui.checkbox(&mut adj.remove_cast_shadow, s.rm_cast_shadow).changed() { rebuild = true; }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("⚠ Shadow/cast processing not yet implemented")
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    );
                });
        }

        }); // ScrollArea

        if rebuild {
            self.rebuild_texture_from_base(ctx);
            self.save_sidecar();
        }
    }

    // ── 设置 ─────────────────────────────────────────────────────────

    /// 渲染应用设置面板：GPU 加速、渲染线程数、界面语言
    pub(super) fn render_settings_panel(&mut self, ui: &mut egui::Ui) {
        let s = self.s();
        ui.heading(s.settings_heading);
        ui.separator();
        ui.add_space(8.0);

        // ── GPU Acceleration ──
        ui.strong(s.gpu_acceleration);
        ui.add_space(2.0);
        let old_gpu = self.app_config.gpu_enabled;
        ui.checkbox(&mut self.app_config.gpu_enabled, s.gpu_acceleration);

        ui.add_space(4.0);
        ui.strong(s.gpu_device_label);
        ui.add_space(2.0);
        let old_device = self.app_config.gpu_device.clone();
        ui.text_edit_singleline(&mut self.app_config.gpu_device);
        ui.label(
            egui::RichText::new(s.gpu_auto_select)
                .small()
                .color(ui.visuals().weak_text_color()),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Render Threads ──
        ui.strong(s.render_threads);
        ui.add_space(2.0);
        let max_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(16);
        let old_threads = self.app_config.render_threads;
        let mut threads = self.app_config.render_threads as u32;
        ui.add(egui::Slider::new(&mut threads, 1..=(max_cpus as u32)).suffix(" threads"));
        self.app_config.render_threads = threads as usize;
        ui.label(
            egui::RichText::new(s.render_threads_hint)
                .small()
                .color(ui.visuals().weak_text_color()),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Language ──
        ui.strong(s.ui_language);
        ui.add_space(2.0);
        let old_lang = self.language;
        egui::ComboBox::from_id_salt("settings_lang_selector")
            .selected_text(self.language.label())
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.language, Language::English, Language::English.label());
                ui.selectable_value(&mut self.language, Language::Chinese, Language::Chinese.label());
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Auto Levels Clipping ──
        ui.strong(s.auto_levels_clip);
        ui.add_space(2.0);
        let old_black_pct = self.app_config.auto_levels_black_pct;
        let old_white_pct = self.app_config.auto_levels_white_pct;
        ui.horizontal(|ui| {
            ui.label(s.auto_levels_black_pct);
            ui.add(egui::DragValue::new(&mut self.app_config.auto_levels_black_pct)
                .range(0.0..=5.0)
                .speed(0.01)
                .suffix("%")
                .max_decimals(2));
        });
        ui.horizontal(|ui| {
            ui.label(s.auto_levels_white_pct);
            ui.add(egui::DragValue::new(&mut self.app_config.auto_levels_white_pct)
                .range(0.0..=5.0)
                .speed(0.01)
                .suffix("%")
                .max_decimals(2));
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // ── Histogram Scale ──
        ui.strong(s.histogram_scale_label);
        ui.add_space(2.0);
        let old_hist_scale = self.app_config.histogram_scale.clone();
        let cur_scale = config::HistogramScale::from_str(&self.app_config.histogram_scale);
        let scale_label = match cur_scale {
            config::HistogramScale::Linear => s.histogram_scale_linear,
            config::HistogramScale::Sqrt   => s.histogram_scale_sqrt,
            config::HistogramScale::Log    => s.histogram_scale_log,
            config::HistogramScale::Cbrt   => s.histogram_scale_cbrt,
        };
        egui::ComboBox::from_id_salt("settings_hist_scale")
            .selected_text(scale_label)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                for &sc in config::HistogramScale::ALL {
                    let (label, desc) = match sc {
                        config::HistogramScale::Linear => (s.histogram_scale_linear, s.histogram_scale_linear_desc),
                        config::HistogramScale::Sqrt   => (s.histogram_scale_sqrt,   s.histogram_scale_sqrt_desc),
                        config::HistogramScale::Log    => (s.histogram_scale_log,    s.histogram_scale_log_desc),
                        config::HistogramScale::Cbrt   => (s.histogram_scale_cbrt,   s.histogram_scale_cbrt_desc),
                    };
                    let mut selected = cur_scale == sc;
                    let resp = ui.selectable_label(selected, format!("{label}  {desc}"));
                    if resp.clicked() && !selected {
                        selected = true;
                        self.app_config.histogram_scale = sc.to_str().to_string();
                    }
                    let _ = selected;
                }
            });

        // Detect changes and save
        let gpu_changed = self.app_config.gpu_enabled != old_gpu
            || self.app_config.gpu_device != old_device;
        let threads_changed = self.app_config.render_threads != old_threads;
        let lang_changed = self.language != old_lang;
        let levels_pct_changed = self.app_config.auto_levels_black_pct != old_black_pct
            || self.app_config.auto_levels_white_pct != old_white_pct;
        let hist_scale_changed = self.app_config.histogram_scale != old_hist_scale;

        if gpu_changed || threads_changed || lang_changed || levels_pct_changed || hist_scale_changed {
            if lang_changed {
                self.app_config.language = self.language.to_config().to_string();
            }
            let _ = config::save(&self.app_config);

            if gpu_changed || threads_changed {
                self.settings_needs_restart = true;
            }
        }

        // Restart hint
        if self.settings_needs_restart {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(s.restart_required)
                    .color(egui::Color32::from_rgb(255, 180, 0)),
            );
        }

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);

        // Data directory info
        ui.label(
            egui::RichText::new(format!("📁 {}", config::app_data_dir().display()))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }

    /// 文件右键菜单：复制路径、复制文件名、在 Finder 中显示、用默认应用打开。
    pub(super) fn file_context_menu(&self, ui: &mut egui::Ui, path: &std::path::Path) {
        let s = self.s();

        if ui.button(s.ctx_copy_path).clicked() {
            ui.ctx().copy_text(path.to_string_lossy().into_owned());
            ui.close_menu();
        }
        if ui.button(s.ctx_copy_filename).clicked() {
            let name = path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.ctx().copy_text(name);
            ui.close_menu();
        }
        ui.separator();
        if ui.button(s.ctx_reveal_in_file_manager).clicked() {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open")
                    .arg("-R")
                    .arg(path)
                    .spawn();
            }
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("explorer")
                    .arg(format!("/select,{}", path.to_string_lossy()))
                    .spawn();
            }
            #[cfg(target_os = "linux")]
            {
                if let Some(dir) = path.parent() {
                    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                }
            }
            ui.close_menu();
        }
        if ui.button(s.ctx_open_default).clicked() {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open").arg(path).spawn();
            }
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("cmd")
                    .arg("/c")
                    .arg("start")
                    .arg("")
                    .arg(path)
                    .spawn();
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                let _ = std::process::Command::new("xdg-open").arg(path).spawn();
            }
            ui.close_menu();
        }
    }

    // ── 辅助函数 ─────────────────────────────────────────────────────

    /// 渲染参数标签芯片（label:value 格式），已修改时高亮显示
    pub(super) fn param_chip(ui: &mut egui::Ui, label: &str, value: &str, modified: bool) {
        let color = if modified {
            if ui.visuals().dark_mode {
                egui::Color32::from_rgb(255, 200, 100)
            } else {
                egui::Color32::from_rgb(180, 120, 0)
            }
        } else {
            ui.visuals().weak_text_color()
        };
        ui.label(
            egui::RichText::new(format!("{}:{}", label, value))
                .small()
                .monospace()
                .color(color),
        );
    }

    /// 渲染 FlexColor ImageCorrection 的详细参数：调整、胶片、锐化、处理标志等
    pub(super) fn render_correction_details(
        ui: &mut egui::Ui,
        corr: &flexcolor::ImageCorrection,
        lang: Language,
    ) {
        let s = i18n::strings(lang);

        egui::CollapsingHeader::new(egui::RichText::new(s.image_adjustments).small().strong())
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("adj_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(ui, s.gamma, &format!("{}", corr.gamma));
                        Self::detail_row(ui, s.midtone_adj, &format!("{:.1}", corr.gamma - 1.0));
                        Self::detail_row(ui, s.ev, &format!("{}", corr.ev));
                        Self::detail_row(ui, s.contrast, &format!("{}", corr.contrast));
                        Self::detail_row(ui, s.brightness, &format!("{}", corr.brightness));
                        Self::detail_row(ui, s.lightness, &format!("{}", corr.lightness));
                        Self::detail_row(ui, s.saturation, &format!("{}", corr.saturation));
                        Self::detail_row(
                            ui,
                            s.color_temp,
                            &format!("{}", corr.color_temperature),
                        );
                        Self::detail_row(ui, s.tint, &format!("{}", corr.tint));
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.film_settings).small().strong())
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("film_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.film_curve,
                            flexcolor::film_curve_name(corr.film_curve),
                        );
                        Self::detail_row(
                            ui,
                            s.film_type,
                            flexcolor::film_type_name(corr.film_type),
                        );
                        Self::detail_row(
                            ui,
                            s.color_model,
                            flexcolor::color_model_name(corr.color_model),
                        );
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.sharpening_usm).small().strong())
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("usm_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.enabled,
                            if corr.apply_usm { s.yes } else { s.no },
                        );
                        Self::detail_row(ui, s.amount, &format!("{}", corr.usm_amount));
                        Self::detail_row(ui, s.radius, &format!("{}", corr.usm_radius));
                        Self::detail_row(ui, s.dark_limit, &format!("{}", corr.usm_dark_limit));
                        Self::detail_row(ui, s.threshold, &format!("{}", corr.threshold));
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.processing_flags).small().strong())
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("flags_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.apply_sliders,
                            &Self::bool_icon(corr.apply_sliders),
                        );
                        Self::detail_row(
                            ui,
                            s.apply_curves,
                            &Self::bool_icon(corr.apply_curves),
                        );
                        Self::detail_row(
                            ui,
                            s.apply_histogram,
                            &Self::bool_icon(corr.apply_histogram),
                        );
                        Self::detail_row(ui, s.apply_cc, &Self::bool_icon(corr.apply_cc));

                        // 色彩校正矩阵 6×6
                        if !corr.color_corr.is_empty() {
                            ui.label(egui::RichText::new("CC Matrix").small());
                            ui.end_row();
                            let cols = 6;
                            for row in 0..(corr.color_corr.len() / cols) {
                                let start = row * cols;
                                let end = (start + cols).min(corr.color_corr.len());
                                let vals: Vec<String> = corr.color_corr[start..end]
                                    .iter()
                                    .map(|v| format!("{}", v))
                                    .collect();
                                Self::detail_row(ui, &format!("  R{}", row + 1), &vals.join(", "));
                            }
                        }
                        Self::detail_row(
                            ui,
                            s.noise_filter,
                            &Self::bool_icon(corr.apply_cn_filter),
                        );
                        Self::detail_row(
                            ui,
                            s.dust_removal,
                            &Self::bool_icon(corr.apply_dust),
                        );
                        Self::detail_row(
                            ui,
                            s.embed_profile,
                            &Self::bool_icon(corr.embed_profile),
                        );
                        Self::detail_row(
                            ui,
                            s.enhanced_shadow,
                            &Self::bool_icon(corr.enhanced_shadow),
                        );
                        Self::detail_row(
                            ui,
                            s.rm_cast_highlight,
                            &Self::bool_icon(corr.remove_cast_highlight),
                        );
                        Self::detail_row(
                            ui,
                            s.rm_cast_shadow,
                            &Self::bool_icon(corr.remove_cast_shadow),
                        );
                    });
            });

        egui::CollapsingHeader::new(egui::RichText::new(s.lens_other).small().strong())
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("lens_grid")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        Self::detail_row(
                            ui,
                            s.lens_correction,
                            &format!("{}", corr.lens_correction),
                        );
                        Self::detail_row(
                            ui,
                            s.vignette_amount,
                            &format!("{}", corr.vignette_amount),
                        );
                        Self::detail_row(ui, s.dust_level, &format!("{}", corr.dust_level));
                        Self::detail_row(
                            ui,
                            s.noise_radius,
                            &format!("{}", corr.color_noise_radius),
                        );
                        Self::detail_row(
                            ui,
                            s.auto_highlight,
                            &format!("{}", corr.auto_highlight),
                        );
                        Self::detail_row(
                            ui,
                            s.auto_shadow,
                            &format!("{}", corr.auto_shadow),
                        );
                    });
            });

        if !corr.gradations.is_empty() {
            egui::CollapsingHeader::new(
                egui::RichText::new(s.gradation_curves).small().strong(),
            )
            .default_open(false)
            .show(ui, |ui| {
                let channel_names = ["RGB", "R", "G", "B", "C", "M", "Y"];
                for (ch_idx, points) in corr.gradations.iter().enumerate() {
                    let ch_name = channel_names.get(ch_idx).unwrap_or(&"?");
                    let points_str: Vec<String> = points
                        .iter()
                        .map(|(x, y, _)| format!("({},{})", x, y))
                        .collect();
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: {}",
                            ch_name,
                            points_str.join(" → ")
                        ))
                        .small()
                        .monospace(),
                    );
                }
            });
        }

        // ── Histogram Levels (shadow / gray / highlight per channel) ──
        {
            egui::CollapsingHeader::new(
                egui::RichText::new(s.histogram_levels).small().strong(),
            )
            .default_open(true)
            .show(ui, |ui| {
                let ch = ["RGB", "R", "G", "B"];
                egui::Grid::new("hist_levels_grid")
                    .striped(true)
                    .num_columns(4)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("").small());
                        ui.label(egui::RichText::new(s.levels_shadow).small().strong());
                        ui.label(egui::RichText::new(s.levels_midtone).small().strong());
                        ui.label(egui::RichText::new(s.levels_highlight).small().strong());
                        ui.end_row();
                        for i in 0..4 {
                            ui.label(egui::RichText::new(ch[i]).small().strong());
                            // Shadow/Highlight: raw × 4 = 16-bit value, show mapped to 0-255
                            let s_val = corr.shadow[i] as f64 * 4.0 / 65535.0 * 255.0;
                            let h_val = corr.highlight[i] as f64 * 4.0 / 65535.0 * 255.0;
                            // Gray: 128 = gamma 1.0
                            let g_gamma = 1.0 / (corr.gray[i] as f64 / 128.0).clamp(0.01, 10.0);
                            ui.label(egui::RichText::new(
                                format!("{} (≈{:.1})", corr.shadow[i], s_val)
                            ).small().monospace());
                            ui.label(egui::RichText::new(
                                format!("{} (γ{:.2})", corr.gray[i], g_gamma)
                            ).small().monospace());
                            ui.label(egui::RichText::new(
                                format!("{} (≈{:.1})", corr.highlight[i], h_val)
                            ).small().monospace());
                            ui.end_row();
                        }
                    });
            });
        }
    }

    /// 渲染一行参数详情（标签 + 值）
    pub(super) fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
        ui.label(egui::RichText::new(label).small());
        ui.label(egui::RichText::new(value).small().monospace());
        ui.end_row();
    }

    /// 布尔值转图标：✅ 或 —
    pub(super) fn bool_icon(v: bool) -> String {
        if v { "✅".into() } else { "—".into() }
    }
}
