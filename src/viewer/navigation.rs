use super::types::*;
use super::helpers::*;

use eframe::egui;
use std::path::{Path, PathBuf};

use fff_viewer::config;
use fff_viewer::i18n;
use fff_viewer::tiff::TiffFile;

// ─── Directory tree ─────────────────────────────────────────────────────────

impl FffViewerApp {
    pub(super) fn render_favorites(&mut self, ui: &mut egui::Ui) {
        let s = i18n::strings(self.language);
        if self.favorites.is_empty() {
            ui.label(egui::RichText::new(s.no_favorites).weak().italics());
            return;
        }
        let favorites = self.favorites.clone();
        let mut remove_idx: Option<usize> = None;
        for (i, fav) in favorites.iter().enumerate() {
            let is_selected = self.current_dir.as_deref() == Some(fav.as_path());
            let name = fav
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| fav.to_string_lossy().to_string());
            let name = shorten_dir_name(&name);

            ui.horizontal(|ui| {
                // Remove-from-favorites button
                let star_resp = ui.add(
                    egui::Label::new(egui::RichText::new("★").color(egui::Color32::from_rgb(255, 200, 0)))
                        .sense(egui::Sense::click()),
                );
                if star_resp.on_hover_text(s.remove_favorite).clicked() {
                    remove_idx = Some(i);
                }

                let text = egui::RichText::new(format!("📁 {}", name)).color(if is_selected {
                    ui.visuals().hyperlink_color
                } else {
                    ui.visuals().text_color()
                });
                let label = egui::Label::new(if is_selected { text.strong() } else { text })
                    .sense(egui::Sense::click())
                    .truncate();
                let resp = ui.add(label);
                let resp = resp.on_hover_text(fav.to_string_lossy());
                if resp.clicked() {
                    self.set_directory(fav.clone());
                }
            });
        }
        if let Some(idx) = remove_idx {
            self.favorites.remove(idx);
            self.save_favorites();
        }
    }

    pub(super) fn save_favorites(&mut self) {
        self.app_config.favorites = self.favorites.iter().map(|p| p.to_string_lossy().to_string()).collect();
        let _ = config::save(&self.app_config);
    }

    pub(super) fn dir_scan_depth(&self, dir: &Path) -> DirScanDepth {
        let key = dir.to_string_lossy();
        DirScanDepth::from_u8(
            self.app_config.dir_scan_modes.get(key.as_ref()).copied().unwrap_or(0)
        )
    }

    pub(super) fn set_dir_scan_depth(&mut self, dir: &Path, depth: DirScanDepth) {
        let key = dir.to_string_lossy().to_string();
        if depth == DirScanDepth::Flat {
            self.app_config.dir_scan_modes.remove(&key);
        } else {
            self.app_config.dir_scan_modes.insert(key, depth.to_u8());
        }
        let _ = config::save(&self.app_config);
    }

    pub(super) fn render_dir_tree(&mut self, ui: &mut egui::Ui) {
        let roots = get_root_dirs();
        for root in &roots {
            self.render_dir_node(ui, root, 0);
        }
    }

    pub(super) fn render_dir_node(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize) {
        let is_expanded = self.expanded_dirs.contains(path);
        let is_selected = self.current_dir.as_deref() == Some(path);
        let is_fav = self.favorites.contains(&path.to_path_buf());
        let scan_depth = self.dir_scan_depth(path);

        let raw_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let name = shorten_dir_name(&raw_name);

        let indent = depth as f32 * 16.0;
        let mut toggle_fav = false;
        let mut cycle_depth = false;
        ui.horizontal(|ui| {
            ui.add_space(indent);

            let arrow = if is_expanded { "▼" } else { "▶" };
            if ui
                .add(
                    egui::Label::new(egui::RichText::new(arrow).small())
                        .sense(egui::Sense::click()),
                )
                .clicked()
            {
                if is_expanded {
                    self.expanded_dirs.remove(path);
                } else {
                    self.expanded_dirs.insert(path.to_path_buf());
                }
            }

            // Star icon: filled gold if favorited, dim outline if not
            let star_char = if is_fav { "★" } else { "☆" };
            let star_color = if is_fav {
                egui::Color32::from_rgb(255, 200, 0)
            } else {
                ui.visuals().weak_text_color()
            };
            let s = i18n::strings(self.language);
            let hint = if is_fav { s.remove_favorite } else { s.add_favorite };
            let star_resp = ui.add(
                egui::Label::new(egui::RichText::new(star_char).color(star_color).small())
                    .sense(egui::Sense::click()),
            );
            if star_resp.on_hover_text(hint).clicked() {
                toggle_fav = true;
            }

            // Right portion: depth button (right-aligned) + folder label (left-aligned, fills rest).
            // Outer RTL pins depth button to far right; inner LTR keeps folder name left-aligned.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Depth cycle button — appears on far right (first in RTL)
                let depth_hint = match scan_depth {
                    DirScanDepth::Flat => s.scan_depth_flat,
                    DirScanDepth::OneLevel => s.scan_depth_one,
                    DirScanDepth::All => s.scan_depth_all,
                };
                let depth_color = match scan_depth {
                    DirScanDepth::Flat => ui.visuals().weak_text_color(),
                    DirScanDepth::OneLevel => egui::Color32::from_rgb(80, 160, 255),
                    DirScanDepth::All => egui::Color32::from_rgb(80, 200, 120),
                };
                let btn = ui.add(
                    egui::Label::new(
                        egui::RichText::new(scan_depth.short_label())
                            .small()
                            .monospace()
                            .color(depth_color),
                    )
                    .sense(egui::Sense::click()),
                );
                if btn.on_hover_text(depth_hint).clicked() {
                    cycle_depth = true;
                }

                // Folder label — left-aligned in remaining space, truncates if needed
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let text = egui::RichText::new(format!("📁 {}", name)).color(if is_selected {
                        ui.visuals().hyperlink_color
                    } else {
                        ui.visuals().text_color()
                    });
                    let label = egui::Label::new(if is_selected { text.strong() } else { text })
                        .sense(egui::Sense::click())
                        .truncate();
                    let resp = ui.add(label);
                    if name != raw_name {
                        resp.clone().on_hover_text(&raw_name);
                    }
                    if resp.clicked() {
                        self.set_directory(path.to_path_buf());
                    }
                    if resp.double_clicked() {
                        if self.expanded_dirs.contains(path) {
                            self.expanded_dirs.remove(path);
                        } else {
                            self.expanded_dirs.insert(path.to_path_buf());
                        }
                    }
                });
            });
        });

        if toggle_fav {
            let pb = path.to_path_buf();
            if is_fav {
                self.favorites.retain(|f| f != &pb);
            } else {
                self.favorites.push(pb);
            }
            self.save_favorites();
        }
        if cycle_depth {
            let new_depth = scan_depth.cycle();
            self.set_dir_scan_depth(path, new_depth);
            // If this is the currently selected directory, re-scan with new depth
            if self.current_dir.as_deref() == Some(path) {
                let dir = path.to_path_buf();
                self.fff_files = scan_fff_files(&dir, new_depth);
                self.fff_files.sort();
                self.thumbnails.clear();
                self.thumb_pending = self.fff_files.len();
                self.loading_status = if self.fff_files.is_empty() {
                    LoadingStatus::Idle
                } else {
                    LoadingStatus::LoadingThumbnails
                };
                let files = self.fff_files.clone();
                let tx = self.thumb_tx.clone();
                std::thread::spawn(move || {
                    use rayon::prelude::*;
                    files.par_iter().for_each(|path| {
                        let result = if let Ok(tiff) = TiffFile::open(path) {
                            if let Some(img) = tiff.decode_thumbnail() {
                                let w = img.width();
                                let h = img.height();
                                let rgba = img.to_rgba8().into_raw();
                                ThumbResult { path: path.clone(), rgba, width: w, height: h }
                            } else {
                                ThumbResult { path: path.clone(), rgba: Vec::new(), width: 0, height: 0 }
                            }
                        } else {
                            ThumbResult { path: path.clone(), rgba: Vec::new(), width: 0, height: 0 }
                        };
                        let _ = tx.send(result);
                    });
                });
            }
        }

        if is_expanded {
            if let Ok(entries) = std::fs::read_dir(path) {
                let mut dirs: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                            && !e.file_name().to_string_lossy().starts_with('.')
                    })
                    .map(|e| e.path())
                    .collect();
                dirs.sort();
                for child in &dirs {
                    self.render_dir_node(ui, child, depth + 1);
                }
            }
        }
    }
}
