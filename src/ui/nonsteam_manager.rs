use std::path::{Path, PathBuf};
use std::sync::mpsc;

use egui::{Margin, RichText, ScrollArea, Ui};
use steam_shortcuts_util::shortcut::ShortcutOwned;

use crate::config::AppConfig;
use crate::griddb::artwork::apply_artwork;
use crate::griddb::client::{GetImagesOptions, GridDbClient, ImageKind};
use crate::scanner::detector::scan_dirs;
use crate::steam::appid::calculate_app_id;
use crate::steam::paths::{find_steam_dir, find_user_ids, grid_dir, shortcuts_path};
use crate::steam::shortcuts::{read_shortcuts, write_shortcuts};
use crate::ui::theme;
use crate::ui::View;

// ── scan result ───────────────────────────────────────────────────────────

struct ScanResult {
    name: String,
    exe: String,
    start_dir: String,
    launch_options: String,
    app_id: u32,
    include: bool,
}

type ScanRx = mpsc::Receiver<eyre::Result<Vec<ScanResult>>>;
type ArtworkRx = mpsc::Receiver<()>;

// ── view ──────────────────────────────────────────────────────────────────

pub struct NonSteamManagerView {
    shortcuts: Vec<ShortcutOwned>,
    loaded: bool,
    load_error: Option<String>,
    selected_idx: Option<usize>,

    edit_name: String,
    edit_exe: String,
    edit_launch_options: String,
    edit_dirty: bool,

    exe_list: Vec<PathBuf>,
    exe_list_for: Option<usize>,

    scan_rx: Option<ScanRx>,
    scanning: bool,
    scan_results: Vec<ScanResult>,

    // selected scan result for pre-edit before adding
    selected_scan_idx: Option<usize>,
    scan_exe_list: Vec<PathBuf>,
    scan_exe_list_for: Option<usize>,

    artwork_rx: Option<ArtworkRx>,
    artwork_applying: bool,
}

impl NonSteamManagerView {
    pub fn new() -> Self {
        Self {
            shortcuts: vec![],
            loaded: false,
            load_error: None,
            selected_idx: None,
            edit_name: String::new(),
            edit_exe: String::new(),
            edit_launch_options: String::new(),
            edit_dirty: false,
            exe_list: vec![],
            exe_list_for: None,
            scan_rx: None,
            scanning: false,
            scan_results: vec![],
            selected_scan_idx: None,
            scan_exe_list: vec![],
            scan_exe_list_for: None,
            artwork_rx: None,
            artwork_applying: false,
        }
    }

    // ── loading ───────────────────────────────────────────────────────────

    fn load(&mut self, config: &AppConfig) {
        self.loaded = true;
        let Some(steam_dir) = find_steam_dir() else {
            self.load_error = Some("Steam installation not found.".to_string());
            return;
        };
        let uid_override = config.steam.user_id.parse::<u64>().ok();
        let uid = uid_override.or_else(|| find_user_ids(&steam_dir).into_iter().next());
        let Some(uid) = uid else {
            self.load_error = Some("No Steam user ID found.".to_string());
            return;
        };
        let path = shortcuts_path(&steam_dir, uid);
        if !path.exists() {
            return;
        }
        match read_shortcuts(&path) {
            Ok(s) => self.shortcuts = s,
            Err(e) => self.load_error = Some(format!("Failed to read shortcuts.vdf: {e}")),
        }
    }

    // ── selection / edit ──────────────────────────────────────────────────

    fn select_game(&mut self, idx: usize) {
        self.selected_idx = Some(idx);
        self.selected_scan_idx = None;
        let sc = &self.shortcuts[idx];
        self.edit_name = sc.app_name.clone();
        self.edit_exe = strip_quotes(&sc.exe);
        self.edit_launch_options = sc.launch_options.clone();
        self.edit_dirty = false;

        if self.exe_list_for != Some(idx) {
            let install_dir = strip_quotes(&sc.start_dir);
            self.exe_list = find_exes(&install_dir);
            self.exe_list_for = Some(idx);
        }
    }

    fn select_scan_result(&mut self, idx: usize) {
        self.selected_scan_idx = Some(idx);
        self.selected_idx = None;
        self.edit_dirty = false;

        if self.scan_exe_list_for != Some(idx) {
            let dir = strip_quotes(&self.scan_results[idx].start_dir);
            self.scan_exe_list = find_exes(&dir);
            self.scan_exe_list_for = Some(idx);
        }
    }

    fn save_edits(&mut self, config: &AppConfig) {
        let Some(idx) = self.selected_idx else { return };
        let sc = &mut self.shortcuts[idx];
        sc.app_name = self.edit_name.clone();
        sc.exe = add_quotes(&self.edit_exe);
        sc.launch_options = self.edit_launch_options.clone();

        match write_to_steam(&self.shortcuts, config) {
            Ok(()) => {
                self.edit_dirty = false;
                tracing::info!("Changes saved. Restart Steam to apply.");
            }
            Err(e) => tracing::error!("Save failed: {e}"),
        }
    }

    fn remove_game(&mut self, config: &AppConfig) {
        let Some(idx) = self.selected_idx else { return };
        let name = self.shortcuts[idx].app_name.clone();
        self.shortcuts.remove(idx);
        self.selected_idx = None;
        self.exe_list.clear();
        self.exe_list_for = None;
        self.edit_dirty = false;

        for (i, sc) in self.shortcuts.iter_mut().enumerate() {
            sc.order = i.to_string();
        }
        match write_to_steam(&self.shortcuts, config) {
            Ok(()) => tracing::info!("'{}' removed. Restart Steam to apply.", name),
            Err(e) => tracing::error!("Failed to save shortcuts: {e}"),
        }
    }

    // ── scanning ──────────────────────────────────────────────────────────

    fn trigger_scan(&mut self, config: &AppConfig) {
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.scanning = true;
        self.scan_results.clear();
        self.selected_scan_idx = None;

        let scan_dir_list = config.scanner.scan_dirs.clone();
        let blocklist = config.scanner.blocklist.clone();
        let known_dirs: Vec<String> = self.shortcuts
            .iter()
            .map(|s| norm(&strip_quotes(&s.start_dir)))
            .collect();

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Tokio runtime not available.");
            self.scanning = false;
            return;
        };

        handle.spawn_blocking(move || {
            let detected = scan_dirs(&scan_dir_list, &blocklist);
            let results: Vec<ScanResult> = detected
                .into_iter()
                .filter_map(|game| {
                    let game_root = norm(&game.install_dir.to_string_lossy());
                    let already_added = known_dirs
                        .iter()
                        .any(|d| Path::new(d).starts_with(Path::new(&game_root)));
                    if already_added {
                        return None;
                    }
                    let exe_q = format!("\"{}\"", game.exe_path.display());
                    let dir_q = format!("\"{}\"", game.install_dir.display());
                    let app_id = calculate_app_id(&exe_q, &game.name);
                    Some(ScanResult {
                        name: game.name,
                        exe: exe_q,
                        start_dir: dir_q,
                        launch_options: String::new(),
                        app_id,
                        include: true,
                    })
                })
                .collect();
            let _ = tx.send(Ok(results));
        });
    }

    fn add_selected(&mut self, config: &AppConfig) {
        let count = self.scan_results.iter().filter(|r| r.include).count();
        if count == 0 {
            tracing::warn!("No games selected to add.");
            return;
        }
        let new_games: Vec<(u32, String)> = self.scan_results
            .iter()
            .filter(|r| r.include)
            .map(|r| (r.app_id, r.name.clone()))
            .collect();
        for result in self.scan_results.iter().filter(|r| r.include) {
            self.shortcuts.push(ShortcutOwned {
                order: self.shortcuts.len().to_string(),
                app_id: result.app_id,
                app_name: result.name.clone(),
                exe: result.exe.clone(),
                start_dir: result.start_dir.clone(),
                icon: String::new(),
                shortcut_path: String::new(),
                launch_options: result.launch_options.clone(),
                is_hidden: false,
                allow_desktop_config: true,
                allow_overlay: true,
                open_vr: 0,
                dev_kit: 0,
                dev_kit_game_id: String::new(),
                dev_kit_overrite_app_id: 0,
                last_play_time: 0,
                tags: vec![],
            });
        }
        match write_to_steam(&self.shortcuts, config) {
            Ok(()) => {
                self.scan_results.clear();
                self.selected_scan_idx = None;
                tracing::info!("{count} game(s) added to Steam shortcuts. Restart Steam to apply.");
                if config.steamgriddb.auto_artwork {
                    self.trigger_artwork(config, new_games);
                }
            }
            Err(e) => tracing::error!("Failed to save shortcuts: {e}"),
        }
    }

    // ── auto-artwork ──────────────────────────────────────────────────────

    fn trigger_artwork(&mut self, config: &AppConfig, games: Vec<(u32, String)>) {
        let api_key = config.steamgriddb.api_key.clone();
        if api_key.is_empty() || games.is_empty() {
            return;
        }
        let Some(steam_dir) = find_steam_dir() else { return };
        let uid_override = config.steam.user_id.parse::<u64>().ok();
        let uid = uid_override.or_else(|| find_user_ids(&steam_dir).into_iter().next());
        let Some(uid) = uid else { return };
        let gdir = grid_dir(&steam_dir, uid);

        let (tx, rx) = mpsc::channel::<()>();
        self.artwork_rx = Some(rx);
        self.artwork_applying = true;

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Tokio runtime not available for artwork.");
            self.artwork_applying = false;
            return;
        };

        let game_count = games.len();
        handle.spawn(async move {
            let client = GridDbClient::new(api_key);
            let kinds = [
                ImageKind::Cover,
                ImageKind::WideCover,
                ImageKind::Background,
                ImageKind::Logo,
            ];
            let mut applied = 0usize;

            for (app_id, name) in &games {
                let search = match client.search_game(name).await {
                    Ok(r) if !r.is_empty() => r,
                    _ => continue,
                };
                let sgdb_id = search[0].id;

                for &kind in &kinds {
                    let images = match client
                        .get_images(sgdb_id, kind, &GetImagesOptions::default())
                        .await
                    {
                        Ok(imgs) if !imgs.is_empty() => imgs,
                        _ => continue,
                    };
                    if apply_artwork(*app_id, kind, &images[0].url, &gdir)
                        .await
                        .is_ok()
                    {
                        applied += 1;
                    }
                }
            }
            tracing::info!(
                "Auto-artwork: applied {applied} image(s) for {game_count} game(s). Restart Steam to see changes."
            );
            let _ = tx.send(());
        });
    }

    fn poll_channels(&mut self) {
        if let Some(rx) = &self.scan_rx {
            if let Ok(result) = rx.try_recv() {
                self.scanning = false;
                self.scan_rx = None;
                match result {
                    Ok(results) => {
                        let n = results.len();
                        self.scan_results = results;
                        if n == 0 {
                            tracing::info!("Scan complete — no new games found.");
                        } else {
                            tracing::info!("Scan complete — found {n} new game(s).");
                        }
                    }
                    Err(e) => tracing::error!("Scan failed: {e}"),
                }
            }
        }

        if let Some(rx) = &self.artwork_rx {
            if rx.try_recv().is_ok() {
                self.artwork_rx = None;
                self.artwork_applying = false;
            }
        }
    }

    // ── panels ────────────────────────────────────────────────────────────

    fn show_game_list(&mut self, ui: &mut Ui, config: &AppConfig) {
        ui.label(
            RichText::new(format!("Non-Steam Games ({})", self.shortcuts.len()))
                .size(13.0)
                .color(theme::TEXT_2),
        );
        ui.add_space(6.0);

        if let Some(err) = &self.load_error {
            ui.label(RichText::new(err).color(theme::ERROR).small());
        }

        let mut click_game: Option<usize> = None;
        let mut do_scan = false;

        // Leave room for the static bottom bar.
        let list_height = ui.available_height() - 52.0;

        ScrollArea::vertical()
            .max_height(list_height)
            .id_salt("nsm_list")
            .show(ui, |ui| {
                if self.shortcuts.is_empty() {
                    ui.label(
                        RichText::new("No non-Steam games added yet.")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                } else {
                    for (i, sc) in self.shortcuts.iter().enumerate() {
                        let sel = self.selected_idx == Some(i);
                        let resp = ui.selectable_label(sel, &sc.app_name);
                        if resp.clicked() && !sel {
                            click_game = Some(i);
                        }
                    }
                }
            });

        ui.separator();
        ui.add_space(4.0);

        if self.scanning {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Scanning…").color(theme::TEXT_DIM).size(13.0));
            });
        } else {
            if ui.button("Scan for Games").clicked() {
                do_scan = true;
            }
        }

        if self.artwork_applying {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Applying artwork…").color(theme::TEXT_DIM).size(13.0));
            });
        }

        if let Some(i) = click_game { self.select_game(i); }
        if do_scan { self.trigger_scan(config); }
    }

    fn show_scan_panel(&mut self, ui: &mut Ui, config: &AppConfig) {
        let mut do_add = false;
        let mut dismiss = false;
        let mut click_scan: Option<usize> = None;
        let mut select_all = false;
        let mut select_none = false;

        // ── static bottom action bar ──────────────────────────────────────
        egui::TopBottomPanel::bottom("scan_action_bar")
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0))
                    .stroke(egui::Stroke::new(1.0, theme::BORDER)),
            )
            .min_height(48.0)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let count = self.scan_results.iter().filter(|r| r.include).count();
                    let add_label = format!(
                        "Add {} Game{}",
                        count,
                        if count == 1 { "" } else { "s" }
                    );
                    let add_btn = egui::Button::new(
                        RichText::new(add_label).color(egui::Color32::WHITE),
                    )
                    .fill(theme::ACCENT)
                    .stroke(egui::Stroke::new(1.0, theme::ACCENT_HOVER));
                    if ui.add_enabled(!self.scanning && count > 0, add_btn).clicked() {
                        do_add = true;
                    }

                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }

                    if self.artwork_applying {
                        ui.add_space(8.0);
                        ui.add(egui::Separator::default().vertical().spacing(0.0));
                        ui.add_space(8.0);
                        ui.spinner();
                        ui.label(
                            RichText::new("Applying artwork…")
                                .color(theme::TEXT_DIM)
                                .size(13.0),
                        );
                    }
                });
            });

        // ── scanning with no results yet: show full-area spinner ──────────
        if self.scanning && self.scan_results.is_empty() {
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show_inside(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.spinner();
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("Scanning for new games…")
                                    .color(theme::TEXT_DIM)
                                    .size(14.0),
                            );
                        });
                    });
                });
            if dismiss {
                self.scanning = false;
                self.scan_rx = None;
            }
            return;
        }

        // ── left sub-panel: found games list ──────────────────────────────
        egui::SidePanel::left("scan_results_list")
            .resizable(true)
            .default_width(220.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(Margin { left: 12.0, right: 12.0, top: 14.0, bottom: 10.0 }),
            )
            .show_inside(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} game{} found",
                        self.scan_results.len(),
                        if self.scan_results.len() == 1 { "" } else { "s" }
                    ))
                    .size(13.0)
                    .color(theme::TEXT_2),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.small_button("Select all").clicked() { select_all = true; }
                    if ui.small_button("Select none").clicked() { select_none = true; }
                });
                ui.add_space(6.0);

                ScrollArea::vertical().id_salt("scan_result_list").show(ui, |ui| {
                    for (i, result) in self.scan_results.iter_mut().enumerate() {
                        let sel = self.selected_scan_idx == Some(i);
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut result.include, "");
                            let resp = ui.selectable_label(
                                sel,
                                RichText::new(&result.name).size(13.0),
                            );
                            if resp.clicked() {
                                click_scan = Some(i);
                            }
                        });
                    }
                });
            });

        // ── central area: per-game config editor ──────────────────────────
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_1)
                    .inner_margin(Margin { left: 20.0, right: 20.0, top: 16.0, bottom: 12.0 }),
            )
            .show_inside(ui, |ui| {
                if let Some(idx) = self.selected_scan_idx {
                    self.show_scan_result_editor(ui, idx);
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Select a game from the list to configure it before adding")
                                .color(theme::TEXT_DIM)
                                .size(14.0),
                        );
                    });
                }
            });

        // ── deferred actions ──────────────────────────────────────────────
        if select_all  { for r in &mut self.scan_results { r.include = true; } }
        if select_none { for r in &mut self.scan_results { r.include = false; } }
        if let Some(i) = click_scan { self.select_scan_result(i); }
        if do_add   { self.add_selected(config); }
        if dismiss  { self.scan_results.clear(); self.selected_scan_idx = None; }
    }

    fn show_game_editor(&mut self, ui: &mut Ui, config: &AppConfig) {
        if self.selected_idx.is_none() && self.selected_scan_idx.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Select a game from the list to edit")
                        .color(theme::TEXT_DIM)
                        .size(14.0),
                );
            });
            return;
        }

        // Editing an existing shortcut
        if self.selected_idx.is_some() {
            self.show_shortcut_editor(ui, config);
            return;
        }

        // Editing a scan result before adding
        if let Some(idx) = self.selected_scan_idx {
            self.show_scan_result_editor(ui, idx);
        }
    }

    fn show_shortcut_editor(&mut self, ui: &mut Ui, config: &AppConfig) {
        let mut do_save = false;
        let mut do_remove = false;
        let mut new_exe: Option<String> = None;

        ScrollArea::vertical().id_salt("nsm_editor").show(ui, |ui| {
            theme::card().show(ui, |ui| {
                theme::section_header(ui, "Game Name");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.edit_name)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    self.edit_dirty = true;
                }
            });

            ui.add_space(8.0);

            theme::card().show(ui, |ui| {
                theme::section_header(ui, "Executable");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.edit_exe)
                            .desired_width(f32::INFINITY)
                            .hint_text("Path to .exe"),
                    )
                    .changed()
                {
                    self.edit_dirty = true;
                }

                if !self.exe_list.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Executables found in install directory:")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                    ScrollArea::vertical()
                        .max_height(160.0)
                        .id_salt("nsm_exe_picker")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            for path in &self.exe_list {
                                let path_str = path.to_string_lossy();
                                let is_selected = path_str.as_ref() == self.edit_exe.as_str();
                                let label = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy())
                                    .unwrap_or_default();
                                if ui
                                    .selectable_label(is_selected, label.as_ref())
                                    .on_hover_text(path_str.as_ref())
                                    .clicked()
                                    && !is_selected
                                {
                                    new_exe = Some(path_str.into_owned());
                                }
                            }
                        });
                }
            });

            ui.add_space(8.0);

            theme::card().show(ui, |ui| {
                theme::section_header(ui, "Launch Options");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.edit_launch_options)
                            .desired_width(f32::INFINITY)
                            .hint_text("Optional launch arguments"),
                    )
                    .changed()
                {
                    self.edit_dirty = true;
                }
            });

            ui.add_space(12.0);

            ui.horizontal(|ui| {
                let save_label = if self.edit_dirty { "Save Changes  •" } else { "Save Changes" };
                let save_btn = egui::Button::new(
                    RichText::new(save_label).color(egui::Color32::WHITE),
                )
                .fill(theme::ACCENT)
                .stroke(egui::Stroke::new(1.0, theme::ACCENT_HOVER));
                if ui.add(save_btn).clicked() { do_save = true; }

                if ui.button("Remove Game").clicked() { do_remove = true; }
            });

            if self.edit_dirty {
                ui.add_space(2.0);
                ui.label(
                    RichText::new("You have unsaved changes")
                        .small()
                        .color(theme::TEXT_DIM),
                );
            }
        });

        if let Some(exe) = new_exe {
            self.edit_exe = exe;
            self.edit_dirty = true;
        }
        if do_save   { self.save_edits(config); }
        if do_remove { self.remove_game(config); }
    }

    fn show_scan_result_editor(&mut self, ui: &mut Ui, idx: usize) {
        let mut new_exe: Option<String> = None;

        ScrollArea::vertical().id_salt("nsm_scan_editor").show(ui, |ui| {
            theme::card().show(ui, |ui| {
                theme::section_header(ui, "Game Name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.scan_results[idx].name)
                        .desired_width(f32::INFINITY),
                );
            });

            ui.add_space(8.0);

            theme::card().show(ui, |ui| {
                theme::section_header(ui, "Executable");
                ui.add(
                    egui::TextEdit::singleline(&mut self.scan_results[idx].exe)
                        .desired_width(f32::INFINITY)
                        .hint_text("Path to .exe"),
                );

                if !self.scan_exe_list.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Executables found in install directory:")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                    let current_exe = self.scan_results[idx].exe.clone();
                    ScrollArea::vertical()
                        .max_height(280.0)
                        .id_salt("nsm_scan_exe_picker")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            for path in &self.scan_exe_list {
                                let path_str = path.to_string_lossy();
                                let is_selected = path_str.as_ref() == current_exe.as_str();
                                let label = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy())
                                    .unwrap_or_default();
                                if ui
                                    .selectable_label(is_selected, label.as_ref())
                                    .on_hover_text(path_str.as_ref())
                                    .clicked()
                                    && !is_selected
                                {
                                    new_exe = Some(path_str.into_owned());
                                }
                            }
                        });
                }
            });

            ui.add_space(8.0);

            theme::card().show(ui, |ui| {
                theme::section_header(ui, "Launch Options");
                ui.add(
                    egui::TextEdit::singleline(&mut self.scan_results[idx].launch_options)
                        .desired_width(f32::INFINITY)
                        .hint_text("Optional launch arguments"),
                );
            });
        });

        if let Some(exe) = new_exe {
            self.scan_results[idx].exe = exe;
        }
    }
}

impl View for NonSteamManagerView {
    fn name(&self) -> &str {
        "Non-Steam Games"
    }

    fn show(&mut self, ui: &mut Ui, config: &mut AppConfig) {
        if !self.loaded {
            self.load(config);
        }
        self.poll_channels();

        egui::SidePanel::left("nsm_list_panel")
            .resizable(true)
            .default_width(250.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(Margin { left: 14.0, right: 14.0, top: 16.0, bottom: 12.0 }),
            )
            .show_inside(ui, |ui| {
                self.show_game_list(ui, config);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::SURFACE_1))
            .show_inside(ui, |ui| {
                if self.scanning || !self.scan_results.is_empty() {
                    self.show_scan_panel(ui, config);
                } else {
                    egui::Frame::none()
                        .inner_margin(Margin { left: 20.0, right: 20.0, top: 16.0, bottom: 12.0 })
                        .show(ui, |ui| {
                            self.show_game_editor(ui, config);
                        });
                }
            });
    }
}

// ── helpers ───────────────────────────────────────────────────────────────

fn strip_quotes(s: &str) -> String { s.trim_matches('"').to_string() }
fn add_quotes(s: &str) -> String   { format!("\"{s}\"") }

fn norm(s: &str) -> String {
    s.to_lowercase().replace('/', "\\")
}

fn find_exes(dir: &str) -> Vec<PathBuf> {
    let path = std::path::Path::new(dir);
    if !path.is_dir() { return vec![]; }
    let mut exes: Vec<PathBuf> = walkdir::WalkDir::new(path)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .map(|x| x.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    exes.sort();
    exes
}

fn write_to_steam(shortcuts: &[ShortcutOwned], config: &AppConfig) -> eyre::Result<()> {
    let steam_dir =
        find_steam_dir().ok_or_else(|| eyre::eyre!("Steam installation not found"))?;
    let uid_override = config.steam.user_id.parse::<u64>().ok();
    let uid = uid_override
        .or_else(|| find_user_ids(&steam_dir).into_iter().next())
        .ok_or_else(|| eyre::eyre!("No Steam user ID found"))?;
    let path = shortcuts_path(&steam_dir, uid);
    write_shortcuts(&path, shortcuts)?;
    Ok(())
}
