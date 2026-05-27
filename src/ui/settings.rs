use egui::{Margin, ScrollArea, Ui};

use crate::config::AppConfig;
use crate::ui::theme;
use crate::ui::View;

pub struct SettingsView {
    initialized: bool,

    steam_user_id: String,
    scan_dirs: Vec<String>,
    blocklist: Vec<String>,
    api_key: String,
    auto_artwork: bool,
    page_size: u8,
    show_nsfw: bool,
    show_humor: bool,
    show_epilepsy: bool,

    new_scan_dir: String,
    new_blocklist: String,
}

impl SettingsView {
    pub fn new() -> Self {
        Self {
            initialized: false,
            steam_user_id: String::new(),
            scan_dirs: vec![],
            blocklist: vec![],
            api_key: String::new(),
            auto_artwork: true,
            page_size: 25,
            show_nsfw: false,
            show_humor: false,
            show_epilepsy: false,
            new_scan_dir: String::new(),
            new_blocklist: String::new(),
        }
    }

    fn sync_from(&mut self, config: &AppConfig) {
        self.steam_user_id = config.steam.user_id.clone();
        self.scan_dirs = config.scanner.scan_dirs.clone();
        self.blocklist = config.scanner.blocklist.clone();
        self.api_key = config.steamgriddb.api_key.clone();
        self.auto_artwork = config.steamgriddb.auto_artwork;
        self.page_size = config.steamgriddb.page_size;
        self.show_nsfw = config.steamgriddb.show_nsfw;
        self.show_humor = config.steamgriddb.show_humor;
        self.show_epilepsy = config.steamgriddb.show_epilepsy;
    }

    fn apply_to(&self, config: &mut AppConfig) {
        config.steam.user_id = self.steam_user_id.clone();
        config.scanner.scan_dirs = self.scan_dirs.clone();
        config.scanner.blocklist = self.blocklist.clone();
        config.steamgriddb.api_key = self.api_key.clone();
        config.steamgriddb.auto_artwork = self.auto_artwork;
        config.steamgriddb.page_size = self.page_size;
        config.steamgriddb.show_nsfw = self.show_nsfw;
        config.steamgriddb.show_humor = self.show_humor;
        config.steamgriddb.show_epilepsy = self.show_epilepsy;
    }
}

impl View for SettingsView {
    fn name(&self) -> &str {
        "Settings"
    }

    fn show(&mut self, ui: &mut Ui, config: &mut AppConfig) {
        if !self.initialized {
            self.sync_from(config);
            self.initialized = true;
        }

        // ── Static bottom bar: action buttons ─────────────────────────────
        egui::TopBottomPanel::bottom("settings_action_bar")
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(egui::Margin::symmetric(20.0, 10.0))
                    .stroke(egui::Stroke::new(1.0, theme::BORDER)),
            )
            .min_height(48.0)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let save_btn =
                        egui::Button::new(egui::RichText::new("Save").color(egui::Color32::WHITE))
                            .fill(theme::ACCENT)
                            .stroke(egui::Stroke::new(1.0, theme::ACCENT_HOVER));
                    if ui.add(save_btn).clicked() {
                        self.apply_to(config);
                        match crate::config::save(config) {
                            Ok(()) => tracing::info!("Settings saved."),
                            Err(e) => tracing::error!("Settings save failed: {e}"),
                        }
                    }

                    if ui.button("Reload from Disk").clicked() {
                        match crate::config::load() {
                            Ok(loaded) => {
                                *config = loaded;
                                self.sync_from(config);
                                tracing::info!("Config reloaded from disk.");
                            }
                            Err(e) => tracing::error!("Config reload failed: {e}"),
                        }
                    }

                    if ui.button("Reset to Defaults").clicked() {
                        *config = AppConfig::default();
                        self.sync_from(config);
                        tracing::info!("Config reset to defaults (not yet saved).");
                    }

                    ui.add_space(16.0);
                    ui.add(egui::Separator::default().vertical().spacing(0.0));
                    ui.add_space(8.0);

                    if ui.button("Show Config File").clicked() {
                        match crate::config::config_path() {
                            Ok(path) => {
                                if !path.exists() {
                                    let _ = crate::config::save(config);
                                }
                                #[cfg(target_os = "windows")]
                                {
                                    if let Err(e) = std::process::Command::new("explorer")
                                        .arg("/select,")
                                        .arg(&path)
                                        .spawn()
                                    {
                                        tracing::error!("Cannot open Explorer: {e}");
                                    }
                                }
                                #[cfg(not(target_os = "windows"))]
                                {
                                    if let Some(parent) = path.parent() {
                                        let _ = opener::open(parent);
                                    }
                                }
                            }
                            Err(e) => tracing::error!("Cannot get config path: {e}"),
                        }
                    }
                });
            });

        // ── Scrollable settings content ───────────────────────────────────
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let content_margin = Margin {
                left: 24.0,
                right: 24.0,
                top: 20.0,
                bottom: 20.0,
            };
            egui::Frame::none()
                .inner_margin(content_margin)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    self.show_content(ui);
                });
        });
    }
}

// Helper that renders a full-width card.
fn full_card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    theme::card().show(ui, |ui| {
        ui.set_min_width(w - 2.0 * 14.0); // subtract card inner_margin on each side
        add(ui);
    });
}

impl SettingsView {
    fn show_content(&mut self, ui: &mut Ui) {
        // ── SteamGridDB ──────────────────────────────────────────────────
        full_card(ui, |ui| {
            theme::section_header(ui, "SteamGridDB");

            // Stacked label + full-width input so all inputs share the same width
            ui.label(
                egui::RichText::new("API Key")
                    .size(12.0)
                    .color(theme::TEXT_2),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.api_key)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("Enter your SteamGridDB API key"),
            );

            ui.add_space(8.0);
            ui.checkbox(
                &mut self.auto_artwork,
                "Auto-apply artwork when adding non-Steam games",
            );

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // Slider row — label left, slider right is fine for sliders
            ui.horizontal(|ui| {
                ui.label("Images per page");
                let mut size = self.page_size as i32;
                if ui
                    .add(egui::Slider::new(&mut size, 10..=50).suffix(" images"))
                    .changed()
                {
                    self.page_size = size as u8;
                }
            });
            ui.label(
                egui::RichText::new("How many images to load per page in Artwork Manager.")
                    .small()
                    .color(theme::TEXT_DIM),
            );

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(egui::RichText::new("Content filters").color(theme::TEXT_2));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_nsfw, "NSFW");
                ui.add_space(8.0);
                ui.checkbox(&mut self.show_humor, "Humor");
                ui.add_space(8.0);
                ui.checkbox(&mut self.show_epilepsy, "Epilepsy warning");
            });
            ui.label(
                egui::RichText::new("Disabled filters are excluded from artwork search results.")
                    .small()
                    .color(theme::TEXT_DIM),
            );
        });

        ui.add_space(12.0);

        // ── Steam ────────────────────────────────────────────────────────
        full_card(ui, |ui| {
            theme::section_header(ui, "Steam");

            ui.label(
                egui::RichText::new("User ID override")
                    .size(12.0)
                    .color(theme::TEXT_2),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.steam_user_id).desired_width(f32::INFINITY),
            );
            ui.label(
                egui::RichText::new("Leave blank to auto-detect the first account found.")
                    .small()
                    .color(theme::TEXT_DIM),
            );
        });

        ui.add_space(12.0);

        // ── Scanner ──────────────────────────────────────────────────────
        full_card(ui, |ui| {
            theme::section_header(ui, "Scanner");

            ui.label(egui::RichText::new("Scan directories").color(theme::TEXT_2));
            ui.add_space(4.0);

            let mut remove_dir: Option<usize> = None;
            for (i, dir) in self.scan_dirs.iter_mut().enumerate() {
                let row_h = ui.spacing().interact_size.y;
                let row_w = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(row_w, row_h),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("Remove").clicked() {
                            remove_dir = Some(i);
                        }
                        ui.add(egui::TextEdit::singleline(dir).desired_width(f32::INFINITY));
                    },
                );
            }
            if let Some(i) = remove_dir {
                self.scan_dirs.remove(i);
            }

            {
                let row_h = ui.spacing().interact_size.y;
                let row_w = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(row_w, row_h),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("Add Directory").clicked() && !self.new_scan_dir.is_empty() {
                            self.scan_dirs.push(std::mem::take(&mut self.new_scan_dir));
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_scan_dir)
                                .desired_width(f32::INFINITY),
                        );
                    },
                );
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.label(egui::RichText::new("Blocklist (folder names to skip)").color(theme::TEXT_2));
            ui.add_space(4.0);

            let mut remove_block: Option<usize> = None;
            for (i, entry) in self.blocklist.iter_mut().enumerate() {
                let row_h = ui.spacing().interact_size.y;
                let row_w = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(row_w, row_h),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("Remove").clicked() {
                            remove_block = Some(i);
                        }
                        ui.add(egui::TextEdit::singleline(entry).desired_width(f32::INFINITY));
                    },
                );
            }
            if let Some(i) = remove_block {
                self.blocklist.remove(i);
            }

            {
                let row_h = ui.spacing().interact_size.y;
                let row_w = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(row_w, row_h),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("Add Entry").clicked() && !self.new_blocklist.is_empty() {
                            self.blocklist.push(std::mem::take(&mut self.new_blocklist));
                        }
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_blocklist)
                                .desired_width(f32::INFINITY),
                        );
                    },
                );
            }
        });
    }
}
