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

        let profile_names: Vec<String> = self
            .available_profiles
            .iter()
            .map(|p| format!("{} ({})", p.name, p.color_space))
            .collect();

        let current_label = self
            .selected_input_profile
            .and_then(|i| profile_names.get(i))
            .cloned()
            .unwrap_or_else(|| "—".to_string());

        egui::ComboBox::from_id_salt("input_profile_combo")
            .selected_text(&current_label)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(&mut self.selected_input_profile, None, "— None —")
                    .clicked()
                {
                    self.color_status = None;
                }
                for (i, name) in profile_names.iter().enumerate() {
                    let cs = &self.available_profiles[i].color_space;
                    let is_rgb = cs == "RGB" || cs == "RGB ";
                    // Only allow RGB profiles as input (CMYK/GRAY incompatible with RGB images)
                    ui.add_enabled_ui(is_rgb, |ui| {
                        let label = if is_rgb {
                            name.clone()
                        } else {
                            format!("{} ⛔", name)
                        };
                        if ui
                            .selectable_value(
                                &mut self.selected_input_profile,
                                Some(i),
                                label,
                            )
                            .clicked()
                        {
                            self.color_status = None;
                        }
                    });
                }
            });

        ui.add_space(8.0);

        // ── Use Embedded ICC (only shown when file has embedded ICC) ──
        let has_embedded = self
            .detail
            .as_ref()
            .and_then(|d| d.embedded_icc.as_ref())
            .map(|d| !d.is_empty())
            .unwrap_or(false);

        if has_embedded {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.use_embedded_icc, s.use_embedded_icc);
            });
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

        // Embedded correction option (only shown if file has edit history)
        let has_embedded_correction = self
            .detail
            .as_ref()
            .and_then(|d| d.edit_history.as_ref())
            .map(|h| !h.settings.is_empty())
            .unwrap_or(false);

        if has_embedded_correction {
            let mut use_embedded = self.use_embedded_correction;
            if ui
                .checkbox(&mut use_embedded, s.use_embedded_correction)
                .changed()
            {
                self.use_embedded_correction = use_embedded;
                if use_embedded {
                    // Deselect preset when using embedded correction
                    self.selected_preset = None;
                }
                self.color_status = None;
            }

            // Show the embedded correction name
            if use_embedded {
                if let Some(ref detail) = self.detail {
                    if let Some(ref history) = detail.edit_history {
                        let idx = history.current_index.min(history.settings.len().saturating_sub(1));
                        let setting = &history.settings[idx];
                        let label = if setting.name.is_empty() {
                            format!("#{}", idx)
                        } else {
                            setting.name.clone()
                        };
                        ui.label(
                            egui::RichText::new(format!("  📎 {}", label))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                }
            }
            ui.add_space(4.0);
        }

        // Category filter (disabled when using embedded correction)
        let preset_enabled = !self.use_embedded_correction;
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
            ui.add_enabled_ui(preset_enabled, |ui| {
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

        let preset_label = self
            .selected_preset
            .and_then(|i| self.available_presets.get(i))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "—".to_string());

        ui.add_enabled_ui(preset_enabled, |ui| {
            egui::ComboBox::from_id_salt("preset_combo")
                .selected_text(&preset_label)
                .width(ui.available_width() - 16.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut self.selected_preset, None, "— None —")
                        .clicked()
                    {
                        self.use_embedded_correction = false;
                        self.color_status = None;
                    }
                    for (global_idx, preset) in &filtered_presets {
                        let label = if preset.category.is_empty() {
                            preset.name.clone()
                        } else {
                            format!("{}/{}", preset.category, preset.name)
                        };
                        if ui
                            .selectable_value(
                                &mut self.selected_preset,
                                Some(*global_idx),
                                &label,
                            )
                            .clicked()
                        {
                            self.use_embedded_correction = false;
                            self.color_status = None;
                        }
                    }
                });
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

        // ── Show preset details if selected ──
        if let Some(idx) = self.selected_preset {
            if let Some(preset) = self.available_presets.get(idx) {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(4.0);
                ui.strong(
                    egui::RichText::new(format!("📋 {}", preset.name))
                        .color(ui.visuals().hyperlink_color),
                );

                // Parse and show preset details
                if let Ok(xml_data) = std::fs::read_to_string(&preset.path) {
                    if let Some(correction) =
                        flexcolor::parse_settings_xml(&xml_data)
                    {
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
            }
        }
    }

    /// 从 sidecar XML 恢复保存的配置到当前状态
    pub(super) fn apply_sidecar(&mut self, config: &SidecarConfig, ctx: &egui::Context) {
        log::info!("Applying sidecar config");
        self.use_embedded_correction = config.use_embedded_correction;
        self.use_embedded_icc = config.use_embedded_icc;
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
        // 加载 sidecar 时，将色阶同步到当前数据源的存储，另一个源重置
        self.save_levels_to_source(self.histogram_source);
        let other = match self.histogram_source {
            HistogramSource::Processed => &mut self.levels_raw,
            HistogramSource::Raw => &mut self.levels_processed,
        };
        *other = HistogramLevels::default();
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
            } else {
                log::warn!("Correction specifies InputProfile='{}' but not found in available profiles", name);
            }
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

        // Need either ICC or preset (or both)
        if input_icc.is_none() && preset_correction.is_none() {
            self.color_status = Some(format!("❌ {}: no ICC or preset", s.profile_error));
            return;
        }

        let Some(detail) = &self.detail else {
            return;
        };

        // Re-decode the preview image from the TiffFile (downscaled for display)
        let preview_img = match detail.tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
            Some(img) => img,
            None => {
                self.color_status = Some(format!("❌ {}: no preview", s.profile_error));
                return;
            }
        };

        let mut result = preview_img;

        // 第1步：应用 ICC 色彩空间转换（扫描仪 RGB → 输出 RGB）。
        // ICC 在胶片处理之前执行，确保色彩空间校正先于色调映射。
        if let Some(icc_data) = &input_icc {
            log::info!(
                "Applying ICC transform: input={} bytes, image={}x{}",
                icc_data.len(),
                result.width(),
                result.height()
            );
            match color::apply_icc_transform(&result, icc_data, self.target_color_space) {
                Ok(transformed) => {
                    result = transformed;
                }
                Err(e) => {
                    log::error!("ICC transform failed: {}", e);
                    self.color_status = Some(format!("❌ {}: {}", s.profile_error, e));
                    return;
                }
            }
        }

        // ICC 转换后保存为 raw_rgb（Raw 模式的基准：ICC 校正后、无胶片处理）
        let raw_after_icc = to_rgb16(&result);

        // 第2步：应用底片处理（负片反转+色阶）。
        if let Some(ref correction) = preset_correction {
            let film_type = correction.film_type;
            log::info!(
                "Applying film processing: FilmType={} ({}), Shadow={:?}, Highlight={:?}, Gray={:?}, \
                 RemoveCastHighlight={}, RemoveCastShadow={}, Gamma={}",
                film_type,
                flexcolor::film_type_name(film_type),
                correction.shadow,
                correction.highlight,
                correction.gray,
                correction.remove_cast_highlight,
                correction.remove_cast_shadow,
                correction.gamma,
            );
            result = color::apply_film_processing(&result, correction);
        }

        // 第3步：保存 16-bit 基准图像并更新 raw_rgb
        let rgb16 = to_rgb16(&result);
        if let Some(detail) = &mut self.detail {
            detail.base_rgb = Some(rgb16);
            detail.raw_rgb = Some(raw_after_icc);
        }

        self.histogram_needs_update = true;
        self.rebuild_texture_from_base(ctx);

        let status_parts: Vec<&str> = [
            input_icc.as_ref().map(|_| "ICC"),
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
        self.color_status = None;

        // Re-decode preview with downscaling, auto-apply embedded correction if available
        if let Some(detail) = &mut self.detail {
            if let Some(img) = detail.tiff.decode_preview_downscaled(DISPLAY_MAX_DIM) {
                // Auto-apply embedded correction (same as initial load)
                let (processed, corrected) = if let Some(ref eh) = detail.edit_history {
                    if !eh.settings.is_empty() {
                        let idx = eh.current_index.min(eh.settings.len() - 1);
                        let correction = &eh.settings[idx].correction;
                        log::info!("Reset: re-applying embedded correction");
                        (color::apply_film_processing(&img, correction), true)
                    } else {
                        (img, false)
                    }
                } else {
                    (img, false)
                };
                self.use_embedded_correction = corrected;

                detail.base_rgb = Some(to_rgb16(&processed));
            }
        }
        self.histogram_needs_update = true;
        self.rebuild_texture_from_base(ctx);
        log::info!("Color profile reset");
    }

    /// 根据当前直方图数据源选择对应的 16-bit 基准图像，应用手动调整后重建显示纹理。
    /// Raw 模式：raw_rgb → manual_adjust → 显示（绕过胶片处理，直接调整原始数据）
    /// Processed 模式：base_rgb → manual_adjust → 显示（在胶片处理结果上调整）
    pub(super) fn rebuild_texture_from_base(&mut self, ctx: &egui::Context) {
        let Some(detail) = &mut self.detail else { return };

        let source = match self.histogram_source {
            HistogramSource::Raw => detail.raw_rgb.as_ref().or(detail.base_rgb.as_ref()),
            HistogramSource::Processed => detail.base_rgb.as_ref(),
        };
        let Some(base) = source else { return };

        let img = image::DynamicImage::ImageRgb16(base.clone());
        let adjusted = color::apply_manual_adjust(&img, &self.manual_adjust);
        let rgb16 = to_rgb16(&adjusted);
        detail.texture = Some(texture_from_16bit(&rgb16, ctx));
    }

    /// 将当前 `manual_adjust` 中的色阶手柄保存到指定数据源的存储中。
    fn save_levels_to_source(&mut self, source: HistogramSource) {
        let store = match source {
            HistogramSource::Processed => &mut self.levels_processed,
            HistogramSource::Raw => &mut self.levels_raw,
        };
        store.black = self.manual_adjust.levels_black;
        store.gamma = self.manual_adjust.levels_gamma;
        store.white = self.manual_adjust.levels_white;
    }

    /// 从指定数据源的存储恢复色阶手柄到 `manual_adjust`。
    fn load_levels_from_source(&mut self, source: HistogramSource) {
        let store = match source {
            HistogramSource::Processed => &self.levels_processed,
            HistogramSource::Raw => &self.levels_raw,
        };
        self.manual_adjust.levels_black = store.black;
        self.manual_adjust.levels_gamma = store.gamma;
        self.manual_adjust.levels_white = store.white;
    }

    /// 从 16-bit 基准 RGB 图像计算 RGBL 四通道直方图。
    /// 内部使用 65536 bin（精确百分位计算），并派生 256 bin 版本（显示用）。
    pub(super) fn compute_histogram(&mut self) {
        let Some(detail) = &self.detail else {
            self.histogram = None;
            self.histogram_16 = None;
            return;
        };

        let source_img: Option<&Rgb16Image> = match self.histogram_source {
            HistogramSource::Raw => detail.raw_rgb.as_ref().or(detail.base_rgb.as_ref()),
            HistogramSource::Processed => detail.base_rgb.as_ref(),
        };

        let Some(base) = source_img else {
            self.histogram = None;
            self.histogram_16 = None;
            return;
        };

        // 65536-bin 精确直方图
        let mut hist_16: Vec<Vec<u32>> = vec![vec![0u32; 65536]; 4];
        for pixel in base.pixels() {
            let [r16, g16, b16] = pixel.0;
            hist_16[0][r16 as usize] += 1;
            hist_16[1][g16 as usize] += 1;
            hist_16[2][b16 as usize] += 1;
            let lum = (0.2126 * r16 as f32 + 0.7152 * g16 as f32 + 0.0722 * b16 as f32) as usize;
            hist_16[3][lum.min(65535)] += 1;
        }

        // 派生 256-bin 显示用直方图（每 256 个 16-bit bin 合并为 1 个显示 bin）
        let mut hist = Box::new([[0u32; 256]; 4]);
        for ch in 0..4 {
            for i in 0..65536 {
                hist[ch][i >> 8] += hist_16[ch][i];
            }
        }

        self.histogram = Some(hist);
        self.histogram_16 = Some(hist_16);
        self.histogram_needs_update = false;
    }

    /// 渲染 RGB 三通道叠加直方图
    pub(super) fn render_histogram(&self, ui: &mut egui::Ui) {
        let Some(ref hist) = self.histogram else { return };

        let desired_size = egui::vec2(ui.available_width(), 80.0);
        let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
        let painter = ui.painter_at(rect);

        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(20));

        let max_count = hist[0].iter().chain(hist[1].iter()).chain(hist[2].iter())
            .copied().max().unwrap_or(1).max(1) as f32;

        let w = rect.width();
        let h = rect.height();

        let colors = [
            egui::Color32::from_rgba_unmultiplied(220, 50, 50, 100),
            egui::Color32::from_rgba_unmultiplied(50, 200, 50, 100),
            egui::Color32::from_rgba_unmultiplied(50, 100, 255, 100),
        ];

        for ch in 0..3 {
            for i in 0..256 {
                let bar_h = (hist[ch][i] as f32 / max_count * h).min(h);
                if bar_h < 0.5 { continue; }
                let x = rect.left() + i as f32 / 255.0 * w;
                let bar_w = (w / 256.0).max(1.0);
                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(x, rect.bottom() - bar_h),
                    egui::vec2(bar_w, bar_h),
                );
                painter.rect_filled(bar_rect, 0.0, colors[ch]);
            }
        }
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
            for i in 0..256 {
                let count = h_arr[i] as f32;
                if count < 0.5 { continue; }
                let bar_h = (count / max_v * hist_h).ceil().min(hist_h);
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
                *gamma = (0.5_f32.ln() / t.ln()).clamp(0.10, 9.99);
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
            if cols[1].add(
                egui::DragValue::new(gamma).range(0.10..=9.99).max_decimals(2).speed(0.01),
            ).on_hover_text("Midtone gamma").changed() { changed = true; }
            if cols[2].add(
                egui::DragValue::new(white).range(min_white..=255.0).max_decimals(2).speed(0.25),
            ).on_hover_text("White point").changed() { changed = true; }
        });

        changed
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
        let adjust_enabled = s.adjust_enabled;
        let reset_adjust = s.reset_adjust;
        let exposure_str = s.exposure;
        let contrast_str = s.contrast;
        let highlights_str = s.highlights;
        let shadows_str = s.shadows;
        let saturation_str = s.saturation_label;
        let color_balance_str = s.color_balance;
        let hist_rgb = s.hist_rgb;
        let hist_r = s.hist_r;
        let hist_g = s.hist_g;
        let hist_b = s.hist_b;
        let hist_source_raw = s.hist_source_raw;
        let hist_source_processed = s.hist_source_processed;

        // ── Header + toggle (not scrollable) ────────────────────────────
        ui.heading(adjust_heading);
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if ui.checkbox(&mut self.manual_adjust.enabled, adjust_enabled).changed() {
                rebuild = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(reset_adjust).clicked() {
                    let enabled = self.manual_adjust.enabled;
                    self.manual_adjust = color::ManualAdjust { enabled, ..Default::default() };
                    self.levels_processed = HistogramLevels::default();
                    self.levels_raw = HistogramLevels::default();
                    rebuild = true;
                }
            });
        });

        ui.add_space(4.0);

        // ── 直方图数据源切换 ────────────────────────────────────────────
        // 切换时将当前色阶手柄保存到旧数据源，从新数据源恢复手柄状态
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("📊").small());
            let is_raw = self.histogram_source == HistogramSource::Raw;
            if ui.selectable_label(!is_raw, hist_source_processed).clicked() && is_raw {
                // 保存当前手柄到 Raw 存储
                self.save_levels_to_source(HistogramSource::Raw);
                self.histogram_source = HistogramSource::Processed;
                // 从 Processed 存储恢复手柄
                self.load_levels_from_source(HistogramSource::Processed);
                self.histogram_needs_update = true;
                rebuild = true;
            }
            ui.label("|");
            if ui.selectable_label(is_raw, hist_source_raw).clicked() && !is_raw {
                // 保存当前手柄到 Processed 存储
                self.save_levels_to_source(HistogramSource::Processed);
                self.histogram_source = HistogramSource::Raw;
                // 从 Raw 存储恢复手柄
                self.load_levels_from_source(HistogramSource::Raw);
                self.histogram_needs_update = true;
                rebuild = true;
            }
        });

        // 数据源切换后立即重算直方图
        if self.histogram_needs_update {
            self.compute_histogram();
        }

        // 读取直方图数据用于渲染
        let hist_data: Option<[[u32; 256]; 4]> = self.histogram.as_deref().copied();

        // 从 65536-bin 直方图预计算各通道精确黑白点（避免渲染循环中的借用冲突）
        let clip_pct = (
            self.app_config.auto_levels_black_pct,
            self.app_config.auto_levels_white_pct,
        );
        // sections 顺序: [RGB合并(hist ch3), R(ch0), G(ch1), B(ch2)]
        let hist_ch_order = [3usize, 0, 1, 2];
        let auto_bw: [Option<(f32, f32)>; 4] = if let Some(ref h16) = self.histogram_16 {
            std::array::from_fn(|i| {
                Some(Self::auto_percentile_levels(&h16[hist_ch_order[i]], clip_pct.0, clip_pct.1))
            })
        } else {
            [None; 4]
        };

        ui.add_space(2.0);

        // ── 4 histogram sections (outside ScrollArea so scrollbar can't overlap) ──
        let sections: [(&str, usize, egui::Color32); 4] = [
            (hist_rgb, 3usize, egui::Color32::from_gray(160)),
            (hist_r,   0usize, egui::Color32::from_rgb(200, 50, 50)),
            (hist_g,   1usize, egui::Color32::from_rgb(50, 180, 50)),
            (hist_b,   2usize, egui::Color32::from_rgb(60, 100, 220)),
        ];
        let levels_idx = [0usize, 1usize, 2usize, 3usize];

        for (section_pos, (title, hist_ch, bar_color)) in sections.iter().enumerate() {
            let lvl_idx = levels_idx[section_pos];
            let hist = hist_data.as_ref().map(|hd| &hd[*hist_ch]);
            let section_id = ui.id().with(section_pos);
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
            ) {
                rebuild = true;
            }
            ui.add_space(4.0);
        }

        ui.separator();
        ui.add_space(4.0);

        // ── Basic adjustment sliders (scrollable) ───────────────────────
        egui::ScrollArea::vertical().id_salt("adjust_sliders").show(ui, |ui| {
            let adj = &mut self.manual_adjust;

            ui.label(exposure_str);
            if ui.add(egui::Slider::new(&mut adj.exposure, -3.0..=3.0).step_by(0.05).text("stops")).changed() {
                rebuild = true;
            }

            ui.label(contrast_str);
            if ui.add(egui::Slider::new(&mut adj.contrast, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label(highlights_str);
            if ui.add(egui::Slider::new(&mut adj.highlights, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label(shadows_str);
            if ui.add(egui::Slider::new(&mut adj.shadows, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.label(saturation_str);
            if ui.add(egui::Slider::new(&mut adj.saturation, -100.0..=100.0).text("")).changed() {
                rebuild = true;
            }

            ui.add_space(8.0);
            ui.label(egui::RichText::new(color_balance_str).strong());

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
        });

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

        // Detect changes and save
        let gpu_changed = self.app_config.gpu_enabled != old_gpu
            || self.app_config.gpu_device != old_device;
        let threads_changed = self.app_config.render_threads != old_threads;
        let lang_changed = self.language != old_lang;
        let levels_pct_changed = self.app_config.auto_levels_black_pct != old_black_pct
            || self.app_config.auto_levels_white_pct != old_white_pct;

        if gpu_changed || threads_changed || lang_changed || levels_pct_changed {
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
        if ui.button(s.ctx_reveal_in_finder).clicked() {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open")
                    .arg("-R")
                    .arg(path)
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
            let _ = std::process::Command::new("open").arg(path).spawn();
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
                let channel_names = ["Master", "Red", "Green", "Blue"];
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
