use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::{Align2, Margin, Rounding, ScrollArea, Stroke, Ui};
use steam_shortcuts_util::shortcut::ShortcutOwned;

use crate::config::AppConfig;
use crate::griddb::artwork::write_artwork_bytes;
use crate::griddb::client::{GameResult, GetImagesOptions, GridDbClient, ImageKind, ImageResult};
use crate::steam::library::{list_installed_games, SteamGame};
use crate::steam::paths::{find_steam_dir, find_user_ids, grid_dir, shortcuts_path};
use crate::steam::shortcuts::read_shortcuts;
use crate::ui::theme;
use crate::ui::View;

const ALL_KINDS: &[ImageKind] = &[
    ImageKind::Cover,
    ImageKind::WideCover,
    ImageKind::Background,
    ImageKind::Logo,
    ImageKind::Icon,
];

fn kind_label(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::Cover => "Cover",
        ImageKind::WideCover => "Wide Cover",
        ImageKind::Background => "Background",
        ImageKind::Logo => "Logo",
        ImageKind::Icon => "Icon",
    }
}

#[derive(PartialEq, Clone, Copy)]
enum LibraryTab {
    NonSteam,
    SteamLibrary,
}

// ── thumbnail loading state ────────────────────────────────────────────────

struct DecodedThumb {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

enum ThumbState {
    Pending,
    Loading,
    Loaded(egui::TextureHandle),
    Failed,
}

struct GalleryEntry {
    meta: ImageResult,
    attempts: u8,
    thumb: ThumbState,
}

// ── toast notification ────────────────────────────────────────────────────

struct ToastNotification {
    message: String,
    file_path: Option<PathBuf>,
    expires_at: Instant,
}

// ── async result channels ──────────────────────────────────────────────────

type SearchRx = mpsc::Receiver<eyre::Result<Vec<GameResult>>>;
type ImagesRx = mpsc::Receiver<eyre::Result<Vec<ImageResult>>>;
type ApplyRx = mpsc::Receiver<eyre::Result<(Vec<u8>, String)>>;
type ThumbTx = mpsc::Sender<(usize, eyre::Result<DecodedThumb>)>;
type ThumbRx = mpsc::Receiver<(usize, eyre::Result<DecodedThumb>)>;

pub struct ArtworkPickerView {
    // library tab
    active_tab: LibraryTab,

    // non-steam game list
    shortcuts: Vec<ShortcutOwned>,
    shortcuts_loaded: bool,
    load_error: Option<String>,

    // steam library list
    steam_games: Vec<SteamGame>,
    steam_games_loaded: bool,

    // shared selection
    selected_game: Option<usize>,

    // image kind tabs
    active_kind: ImageKind,

    // SteamGridDB search
    search_rx: Option<SearchRx>,
    searching: bool,
    search_results: Vec<GameResult>,
    selected_sgdb_idx: usize,

    // image metadata fetch
    images_rx: Option<ImagesRx>,
    fetching: bool,
    loaded_for: Option<(usize, ImageKind)>,
    current_page: usize,
    images_has_more: bool,
    images_appending: bool,

    // thumbnail gallery
    gallery: Vec<GalleryEntry>,
    active_loads: usize,
    thumb_tx: Option<ThumbTx>,
    thumb_rx: Option<ThumbRx>,

    // apply artwork
    apply_rx: Option<ApplyRx>,
    applying: bool,

    // toast notification
    toast: Option<ToastNotification>,
}

impl ArtworkPickerView {
    pub fn new() -> Self {
        Self {
            active_tab: LibraryTab::NonSteam,
            shortcuts: vec![],
            shortcuts_loaded: false,
            load_error: None,
            steam_games: vec![],
            steam_games_loaded: false,
            selected_game: None,
            active_kind: ImageKind::Cover,
            search_rx: None,
            searching: false,
            search_results: vec![],
            selected_sgdb_idx: 0,
            images_rx: None,
            fetching: false,
            loaded_for: None,
            current_page: 0,
            images_has_more: false,
            images_appending: false,
            gallery: vec![],
            active_loads: 0,
            thumb_tx: None,
            thumb_rx: None,
            apply_rx: None,
            applying: false,
            toast: None,
        }
    }

    // ── loaders ───────────────────────────────────────────────────────────

    fn load_shortcuts(&mut self, config: &AppConfig) {
        self.shortcuts_loaded = true;
        let user_id_override = if config.steam.user_id.is_empty() {
            None
        } else {
            config.steam.user_id.parse::<u64>().ok()
        };

        let Some(steam_dir) = find_steam_dir() else {
            self.load_error = Some("Steam installation not found.".to_string());
            return;
        };

        let user_id = user_id_override.or_else(|| find_user_ids(&steam_dir).into_iter().next());

        let Some(uid) = user_id else {
            self.load_error = Some("No Steam user ID found.".to_string());
            return;
        };

        let path = shortcuts_path(&steam_dir, uid);
        if !path.exists() {
            self.load_error = Some(format!("shortcuts.vdf not found at {}", path.display()));
            return;
        }

        match read_shortcuts(&path) {
            Ok(s) => self.shortcuts = s,
            Err(e) => self.load_error = Some(format!("Failed to read shortcuts: {e}")),
        }
    }

    fn load_steam_games(&mut self) {
        self.steam_games_loaded = true;
        self.steam_games = list_installed_games();
    }

    // ── gallery management ────────────────────────────────────────────────

    fn clear_gallery(&mut self) {
        self.gallery.clear();
        self.thumb_tx = None;
        self.thumb_rx = None;
        self.active_loads = 0;
        self.current_page = 0;
        self.images_has_more = false;
        self.loaded_for = None;
    }

    // ── selection helper ─────────────────────────────────────────────────

    fn select_game(&mut self, idx: usize, name: String, config: &AppConfig) {
        self.selected_game = Some(idx);
        self.search_results.clear();
        self.clear_gallery();

        if !config.steamgriddb.api_key.is_empty() {
            self.trigger_search(config.steamgriddb.api_key.clone(), name);
        } else {
            tracing::warn!("No SteamGridDB API key — set one in Settings to search for artwork.");
        }
    }

    // ── app_id resolution ─────────────────────────────────────────────────

    fn selected_app_id(&self) -> u32 {
        let Some(idx) = self.selected_game else {
            return 0;
        };
        match self.active_tab {
            LibraryTab::NonSteam => self.shortcuts.get(idx).map(|s| s.app_id).unwrap_or(0),
            LibraryTab::SteamLibrary => self.steam_games.get(idx).map(|g| g.app_id).unwrap_or(0),
        }
    }

    // ── options builder ───────────────────────────────────────────────────

    fn selected_game_name(&self) -> String {
        let Some(idx) = self.selected_game else { return String::new() };
        match self.active_tab {
            LibraryTab::NonSteam => self.shortcuts.get(idx).map(|s| s.app_name.clone()).unwrap_or_default(),
            LibraryTab::SteamLibrary => self.steam_games.get(idx).map(|g| g.name.clone()).unwrap_or_default(),
        }
    }

    fn make_opts(config: &AppConfig, page: usize) -> GetImagesOptions {
        GetImagesOptions {
            page,
            limit: config.steamgriddb.page_size as usize,
            show_nsfw: config.steamgriddb.show_nsfw,
            show_humor: config.steamgriddb.show_humor,
        }
    }

    // ── thumb size per kind ───────────────────────────────────────────────

    fn thumb_size(&self, config: &AppConfig) -> egui::Vec2 {
        let s = match self.active_kind {
            ImageKind::Cover => config.steamgriddb.thumb_scales.cover,
            ImageKind::WideCover => config.steamgriddb.thumb_scales.wide_cover,
            ImageKind::Background => config.steamgriddb.thumb_scales.background,
            ImageKind::Logo => config.steamgriddb.thumb_scales.logo,
            ImageKind::Icon => config.steamgriddb.thumb_scales.icon,
        };
        match self.active_kind {
            ImageKind::Cover => egui::vec2(130.0 * s, 195.0 * s),
            ImageKind::WideCover => egui::vec2(200.0 * s, 93.0 * s),
            ImageKind::Background => egui::vec2(240.0 * s, 90.0 * s),
            ImageKind::Logo => egui::vec2(180.0 * s, 90.0 * s),
            ImageKind::Icon => egui::vec2(100.0 * s, 100.0 * s),
        }
    }

    // ── async dispatchers ─────────────────────────────────────────────────

    fn trigger_search(&mut self, api_key: String, game_name: String) {
        let (tx, rx) = mpsc::channel();
        self.search_rx = Some(rx);
        self.searching = true;
        self.search_results.clear();
        self.clear_gallery();

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Tokio runtime not available for search.");
            self.searching = false;
            return;
        };

        handle.spawn(async move {
            let result = GridDbClient::new(api_key).search_game(&game_name).await;
            let _ = tx.send(result);
        });
    }

    fn trigger_image_fetch(
        &mut self,
        api_key: String,
        sgdb_id: usize,
        kind: ImageKind,
        opts: GetImagesOptions,
        append: bool,
    ) {
        let (tx, rx) = mpsc::channel();
        self.images_rx = Some(rx);
        self.fetching = true;
        self.images_appending = append;

        if !append {
            self.clear_gallery();
        }

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Tokio runtime not available for image fetch.");
            self.fetching = false;
            return;
        };

        handle.spawn(async move {
            let result = GridDbClient::new(api_key)
                .get_images(sgdb_id, kind, &opts)
                .await;
            let _ = tx.send(result);
        });
    }

    fn trigger_apply(&mut self, url: String) {
        let (tx, rx) = mpsc::channel();
        self.apply_rx = Some(rx);
        self.applying = true;

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Tokio runtime not available.");
            self.applying = false;
            return;
        };

        let url_clone = url.clone();
        handle.spawn(async move {
            let result = async {
                let bytes = reqwest::get(&url_clone).await?.bytes().await?;
                Ok::<_, eyre::Report>((bytes.to_vec(), url_clone))
            }
            .await;
            let _ = tx.send(result);
        });
    }

    // ── thumbnail batch loader ────────────────────────────────────────────

    fn pump_thumb_queue(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.thumb_rx {
            while let Ok((idx, result)) = rx.try_recv() {
                self.active_loads = self.active_loads.saturating_sub(1);
                if let Some(entry) = self.gallery.get_mut(idx) {
                    match result {
                        Ok(decoded) => {
                            let color_img = egui::ColorImage::from_rgba_unmultiplied(
                                [decoded.width, decoded.height],
                                &decoded.rgba,
                            );
                            let handle = ctx.load_texture(
                                format!("vf_th_{idx}"),
                                color_img,
                                egui::TextureOptions::LINEAR,
                            );
                            entry.thumb = ThumbState::Loaded(handle);
                        }
                        Err(_) => {
                            entry.attempts += 1;
                            if entry.attempts < 3 {
                                entry.thumb = ThumbState::Pending;
                            } else {
                                entry.thumb = ThumbState::Failed;
                            }
                        }
                    }
                }
            }
        }

        for idx in 0..self.gallery.len() {
            if self.active_loads >= 5 {
                break;
            }
            if !matches!(self.gallery[idx].thumb, ThumbState::Pending) {
                continue;
            }

            let Some(tx) = self.thumb_tx.clone() else {
                break;
            };
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                break;
            };

            let url = self.gallery[idx].meta.thumb.clone();
            self.gallery[idx].thumb = ThumbState::Loading;
            self.active_loads += 1;

            handle.spawn(async move {
                let result: eyre::Result<DecodedThumb> = async {
                    let bytes = reqwest::get(&url)
                        .await
                        .map_err(eyre::Report::from)?
                        .bytes()
                        .await
                        .map_err(eyre::Report::from)?;
                    let img = image::load_from_memory(&bytes)
                        .map_err(|e| eyre::eyre!("decode: {e}"))?;
                    let rgba8 = img.to_rgba8();
                    let width = rgba8.width() as usize;
                    let height = rgba8.height() as usize;
                    Ok(DecodedThumb { width, height, rgba: rgba8.into_raw() })
                }
                .await;
                let _ = tx.send((idx, result));
            });
        }
    }

    // ── poll pending channels ─────────────────────────────────────────────

    fn poll_channels(&mut self, config: &AppConfig) {
        if let Some(rx) = &self.search_rx {
            if let Ok(result) = rx.try_recv() {
                self.searching = false;
                self.search_rx = None;
                match result {
                    Ok(results) => {
                        self.search_results = results;
                        self.selected_sgdb_idx = 0;
                        if !self.search_results.is_empty() {
                            tracing::info!(
                                "Found {} SteamGridDB match(es).",
                                self.search_results.len()
                            );
                            let id = self.search_results[0].id;
                            let opts = Self::make_opts(config, 0);
                            self.trigger_image_fetch(
                                config.steamgriddb.api_key.clone(),
                                id,
                                self.active_kind,
                                opts,
                                false,
                            );
                        } else {
                            tracing::warn!("No SteamGridDB matches found.");
                        }
                    }
                    Err(e) => {
                        tracing::error!("SteamGridDB search failed: {e}");
                    }
                }
            }
        }

        if let Some(rx) = &self.images_rx {
            if let Ok(result) = rx.try_recv() {
                self.fetching = false;
                self.images_rx = None;
                let appending = self.images_appending;
                self.images_appending = false;

                match result {
                    Ok(images) => {
                        let page_size = config.steamgriddb.page_size as usize;

                        let images: Vec<ImageResult> = images
                            .into_iter()
                            .filter(|img| config.steamgriddb.show_epilepsy || !img.epilepsy)
                            .collect();

                        let received = images.len();
                        self.images_has_more = received >= page_size;

                        if appending {
                            if self.thumb_tx.is_none() {
                                let (tx, rx) = mpsc::channel();
                                self.thumb_tx = Some(tx);
                                self.thumb_rx = Some(rx);
                            }
                            for img in images {
                                self.gallery.push(GalleryEntry {
                                    meta: img,
                                    attempts: 0,
                                    thumb: ThumbState::Pending,
                                });
                            }
                            if received > 0 {
                                self.current_page += 1;
                            }
                        } else {
                            let (tx, rx) = mpsc::channel();
                            self.thumb_tx = Some(tx);
                            self.thumb_rx = Some(rx);
                            self.gallery = images
                                .into_iter()
                                .map(|meta| GalleryEntry {
                                    meta,
                                    attempts: 0,
                                    thumb: ThumbState::Pending,
                                })
                                .collect();
                            self.current_page = 0;
                            self.loaded_for = Some((self.selected_sgdb_idx, self.active_kind));
                            tracing::info!("{received} image(s) found.");
                        }
                    }
                    Err(e) => {
                        tracing::error!("Image fetch failed: {e}");
                    }
                }
            }
        }

        if let Some(rx) = &self.apply_rx {
            if let Ok(result) = rx.try_recv() {
                self.applying = false;
                self.apply_rx = None;
                match result {
                    Ok((bytes, url)) => {
                        if let Some(steam_dir) = find_steam_dir() {
                            let uid_override = config.steam.user_id.parse::<u64>().ok();
                            let uid = uid_override
                                .or_else(|| find_user_ids(&steam_dir).into_iter().next());
                            if let Some(uid) = uid {
                                let gdir = grid_dir(&steam_dir, uid);
                                let appid = self.selected_app_id();
                                match write_artwork_bytes(
                                    appid,
                                    self.active_kind,
                                    &url,
                                    &gdir,
                                    &bytes,
                                ) {
                                    Ok(dest) => {
                                        let short_name = dest
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| dest.display().to_string());
                                        let game_name = self.selected_game_name();
                                        tracing::info!(
                                            "Artwork ({}) for '{}' saved as {}.",
                                            kind_label(self.active_kind),
                                            game_name,
                                            short_name
                                        );
                                        self.toast = Some(ToastNotification {
                                            message: format!("Artwork saved: {short_name}"),
                                            file_path: Some(dest.clone()),
                                            expires_at: Instant::now() + Duration::from_secs(6),
                                        });
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to save artwork: {e}");
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Artwork download failed: {e}");
                    }
                }
            }
        }
    }

    // ── toast ─────────────────────────────────────────────────────────────

    fn show_toast(&mut self, ctx: &egui::Context) {
        let Some(ref toast) = self.toast else { return };

        let remaining = toast
            .expires_at
            .checked_duration_since(Instant::now())
            .map(|d| d.as_secs_f32())
            .unwrap_or(0.0);

        if remaining <= 0.0 {
            self.toast = None;
            return;
        }

        ctx.request_repaint();

        let message = toast.message.clone();
        let file_path = toast.file_path.clone();

        let mut dismiss = false;
        egui::Window::new("##vf_toast")
            .anchor(Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .min_width(260.0)
            .max_width(420.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_3)
                    .rounding(Rounding::same(8.0))
                    .stroke(Stroke::new(1.5, theme::ACCENT))
                    .inner_margin(Margin::same(14.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("✓  Artwork applied")
                            .color(theme::SUCCESS)
                            .size(13.0)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            dismiss = true;
                        }
                        ui.label(
                            egui::RichText::new(format!("{:.0}s", remaining.ceil()))
                                .size(11.0)
                                .color(theme::TEXT_DIM),
                        );
                    });
                });

                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(&message)
                        .size(12.0)
                        .color(theme::TEXT_2),
                );

                if let Some(ref path) = file_path {
                    ui.add_space(6.0);
                    if ui
                        .small_button("Open in Explorer")
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        #[cfg(target_os = "windows")]
                        {
                            let _ = std::process::Command::new("explorer")
                                .arg("/select,")
                                .arg(path)
                                .spawn();
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            if let Some(parent) = path.parent() {
                                let _ = opener::open(parent);
                            }
                        }
                        dismiss = true;
                    }
                }
            });

        if dismiss {
            self.toast = None;
        }
    }

    // ── UI panels ─────────────────────────────────────────────────────────

    fn show_game_list(&mut self, ui: &mut Ui, config: &AppConfig) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (tab, label) in [
                (LibraryTab::NonSteam, "Non-Steam"),
                (LibraryTab::SteamLibrary, "Steam Library"),
            ] {
                let active = self.active_tab == tab;
                let btn =
                    egui::Button::new(egui::RichText::new(label).size(12.0).color(if active {
                        egui::Color32::WHITE
                    } else {
                        theme::TEXT_2
                    }))
                    .fill(if active {
                        theme::ACCENT
                    } else {
                        theme::SURFACE_3
                    })
                    .stroke(Stroke::new(
                        1.0,
                        if active {
                            theme::ACCENT_HOVER
                        } else {
                            theme::BORDER
                        },
                    ));
                if ui.add(btn).clicked() && !active {
                    self.active_tab = tab;
                    self.selected_game = None;
                    self.search_results.clear();
                    self.clear_gallery();
                }
            }
        });

        ui.add_space(10.0);

        match self.active_tab {
            LibraryTab::NonSteam => {
                if let Some(err) = &self.load_error {
                    ui.label(egui::RichText::new(err).color(theme::ERROR).size(12.0));
                    return;
                }
                if self.shortcuts.is_empty() {
                    ui.label(
                        egui::RichText::new("No non-Steam shortcuts found.")
                            .color(theme::TEXT_DIM)
                            .size(12.5),
                    );
                    return;
                }
                ui.label(
                    egui::RichText::new(format!("Games ({})", self.shortcuts.len()))
                        .size(11.0)
                        .color(theme::TEXT_DIM),
                );
                ui.add_space(4.0);
                ScrollArea::vertical().show(ui, |ui| {
                    let mut clicked: Option<(usize, String)> = None;
                    for (i, sc) in self.shortcuts.iter().enumerate() {
                        let selected = self.selected_game == Some(i);
                        if ui.selectable_label(selected, &sc.app_name).clicked() && !selected {
                            clicked = Some((i, sc.app_name.clone()));
                        }
                    }
                    if let Some((i, name)) = clicked {
                        self.select_game(i, name, config);
                    }
                });
            }
            LibraryTab::SteamLibrary => {
                if self.steam_games.is_empty() {
                    ui.label(
                        egui::RichText::new("No Steam games found.")
                            .color(theme::TEXT_DIM)
                            .size(12.5),
                    );
                    return;
                }
                ui.label(
                    egui::RichText::new(format!("Games ({})", self.steam_games.len()))
                        .size(11.0)
                        .color(theme::TEXT_DIM),
                );
                ui.add_space(4.0);
                ScrollArea::vertical().show(ui, |ui| {
                    let mut clicked: Option<(usize, String)> = None;
                    for (i, game) in self.steam_games.iter().enumerate() {
                        let selected = self.selected_game == Some(i);
                        if ui.selectable_label(selected, &game.name).clicked() && !selected {
                            clicked = Some((i, game.name.clone()));
                        }
                    }
                    if let Some((i, name)) = clicked {
                        self.select_game(i, name, config);
                    }
                });
            }
        }
    }

    fn show_right_panel(&mut self, ui: &mut Ui, config: &mut AppConfig) {
        let mut kind_changed: Option<ImageKind> = None;
        let mut load_more = false;
        let mut clicked_url: Option<String> = None;

        // ── Bottom: pagination bar ────────────────────────────────────────
        egui::TopBottomPanel::bottom("picker_pagination_bar")
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(Margin::symmetric(20.0, 8.0))
                    .stroke(Stroke::new(1.0, theme::BORDER)),
            )
            .min_height(38.0)
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if !self.gallery.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} image(s) · page {}",
                                self.gallery.len(),
                                self.current_page + 1,
                            ))
                            .size(12.0)
                            .color(theme::TEXT_DIM),
                        );
                    }

                    // Load More / spinner — right aligned
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.fetching && self.images_appending {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Loading more…")
                                    .color(theme::TEXT_2)
                                    .size(12.0),
                            );
                        } else if self.images_has_more {
                            let btn = egui::Button::new(
                                egui::RichText::new("Load More")
                                    .color(theme::TEXT)
                                    .size(12.5),
                            )
                            .fill(theme::SURFACE_2)
                            .stroke(Stroke::new(1.0, theme::BORDER_STRONG));
                            if ui.add(btn).clicked() {
                                load_more = true;
                            }
                        }
                    });
                });
            });

        // ── Top: kind tab bar + SGDB match + size slider ──────────────────
        egui::TopBottomPanel::top("picker_kind_bar")
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_1)
                    .inner_margin(Margin {
                        left: 20.0,
                        right: 20.0,
                        top: 10.0,
                        bottom: 8.0,
                    }),
            )
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    // Kind tabs
                    for &kind in ALL_KINDS {
                        let active = self.active_kind == kind;
                        let btn = egui::Button::new(
                            egui::RichText::new(kind_label(kind))
                                .size(12.5)
                                .color(if active {
                                    egui::Color32::WHITE
                                } else {
                                    theme::TEXT_2
                                }),
                        )
                        .fill(if active {
                            theme::ACCENT
                        } else {
                            theme::SURFACE_2
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if active {
                                theme::ACCENT_HOVER
                            } else {
                                theme::BORDER
                            },
                        ));
                        if ui.add(btn).clicked() && !active {
                            kind_changed = Some(kind);
                        }
                    }

                    // Vertical separator
                    ui.add_space(8.0);
                    ui.add(egui::Separator::default().vertical().spacing(0.0));
                    ui.add_space(8.0);

                    // SGDB match picker (shown when multiple candidates)
                    if self.search_results.len() > 1 {
                        ui.label(
                            egui::RichText::new("Game: ")
                                .color(theme::TEXT_DIM)
                                .size(12.0),
                        );
                        let current_name = self.search_results[self.selected_sgdb_idx].name.clone();
                        let mut new_idx: Option<usize> = None;
                        egui::ComboBox::from_id_salt("sgdb_pick")
                            .selected_text(&current_name)
                            .width(160.0)
                            .show_ui(ui, |ui| {
                                for (i, r) in self.search_results.iter().enumerate() {
                                    if ui
                                        .selectable_label(self.selected_sgdb_idx == i, &r.name)
                                        .clicked()
                                        && self.selected_sgdb_idx != i
                                    {
                                        new_idx = Some(i);
                                    }
                                }
                            });
                        if let Some(i) = new_idx {
                            self.selected_sgdb_idx = i;
                            if !config.steamgriddb.api_key.is_empty() {
                                let id = self.search_results[i].id;
                                let opts = Self::make_opts(config, 0);
                                self.trigger_image_fetch(
                                    config.steamgriddb.api_key.clone(),
                                    id,
                                    self.active_kind,
                                    opts,
                                    false,
                                );
                            }
                        }

                        ui.add_space(4.0);
                        ui.add(egui::Separator::default().vertical().spacing(0.0));
                        ui.add_space(4.0);
                    }

                    // Size slider — reads/writes config per active kind
                    ui.label(
                        egui::RichText::new("Size")
                            .color(theme::TEXT_DIM)
                            .size(12.0),
                    );
                    let scale = match self.active_kind {
                        ImageKind::Cover => &mut config.steamgriddb.thumb_scales.cover,
                        ImageKind::WideCover => &mut config.steamgriddb.thumb_scales.wide_cover,
                        ImageKind::Background => &mut config.steamgriddb.thumb_scales.background,
                        ImageKind::Logo => &mut config.steamgriddb.thumb_scales.logo,
                        ImageKind::Icon => &mut config.steamgriddb.thumb_scales.icon,
                    };
                    let slider_resp = ui.add(egui::Slider::new(scale, 0.5..=3.0).show_value(false));
                    if slider_resp.drag_stopped() {
                        let _ = crate::config::save(config);
                    }

                    // Applying indicator — right aligned
                    if self.applying {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new("Applying…")
                                    .color(theme::TEXT_2)
                                    .size(12.0),
                            );
                            ui.spinner();
                        });
                    }
                });
            });

        // Handle kind change after borrowing config
        if let Some(kind) = kind_changed {
            self.active_kind = kind;
            let needs_fetch = self
                .loaded_for
                .map(|(_, k)| k != kind)
                .unwrap_or(!self.search_results.is_empty());
            if needs_fetch
                && !self.search_results.is_empty()
                && !config.steamgriddb.api_key.is_empty()
            {
                let id = self.search_results[self.selected_sgdb_idx].id;
                let opts = Self::make_opts(config, 0);
                self.trigger_image_fetch(config.steamgriddb.api_key.clone(), id, kind, opts, false);
            }
        }

        // ── Central: gallery ──────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::SURFACE_1))
            .show_inside(ui, |ui| {
                self.show_gallery_area(ui, config, &mut clicked_url);
            });

        // Handle load more
        if load_more {
            let id = self.search_results[self.selected_sgdb_idx].id;
            let next_page = self.current_page + 1;
            let opts = Self::make_opts(config, next_page);
            self.trigger_image_fetch(
                config.steamgriddb.api_key.clone(),
                id,
                self.active_kind,
                opts,
                true,
            );
        }

        // Handle image clicks
        if let Some(url) = clicked_url {
            self.trigger_apply(url);
        }
    }

    fn show_gallery_area(
        &mut self,
        ui: &mut Ui,
        config: &AppConfig,
        clicked_url: &mut Option<String>,
    ) {
        if self.searching {
            ui.centered_and_justified(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("Searching SteamGridDB…")
                            .color(theme::TEXT_2)
                            .size(13.0),
                    );
                });
            });
            return;
        }
        if self.fetching && !self.images_appending {
            ui.centered_and_justified(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("Loading images…")
                            .color(theme::TEXT_2)
                            .size(13.0),
                    );
                });
            });
            return;
        }
        if self.selected_game.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("← Select a game to browse artwork.")
                        .color(theme::TEXT_DIM)
                        .size(14.0),
                );
            });
            return;
        }
        if self.gallery.is_empty() && !self.fetching {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("No images found for this game and category.")
                        .color(theme::TEXT_DIM)
                        .size(13.0),
                );
            });
            return;
        }

        let thumb_size = self.thumb_size(config);
        let gap = 8.0_f32;
        let h_pad = 20.0_f32;
        let v_pad = 12.0_f32;

        // Build render snapshot
        let entries: Vec<_> = self
            .gallery
            .iter()
            .map(|e| {
                let handle = if let ThumbState::Loaded(h) = &e.thumb {
                    Some(h.clone())
                } else {
                    None
                };
                let is_loading = matches!(e.thumb, ThumbState::Pending | ThumbState::Loading);
                let full_url = e.meta.url.clone();
                let hover = format!("{}×{}  {:?}", e.meta.width, e.meta.height, e.meta.mime);
                (is_loading, handle, full_url, hover)
            })
            .collect();

        // Compute column count from available width BEFORE the scroll area
        // so the calculation isn't affected by scroll area internals.
        let inner_w = (ui.available_width() - 2.0 * h_pad).max(thumb_size.x);
        let cols = (((inner_w + gap) / (thumb_size.x + gap)).floor() as usize).max(1);

        ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_space(v_pad);

                for chunk in entries.chunks(cols) {
                    // Each row is a plain horizontal layout — items are
                    // always at the same Y, so there is no stair effect.
                    ui.horizontal(|ui| {
                        ui.add_space(h_pad);
                        ui.spacing_mut().item_spacing.x = gap;

                        for (is_loading, handle, full_url, hover) in chunk {
                            let (tile_rect, frame_resp) =
                                ui.allocate_exact_size(thumb_size, egui::Sense::hover());

                            if ui.is_rect_visible(tile_rect) {
                                let hovered = frame_resp.hovered();
                                let fill = if hovered {
                                    theme::SURFACE_3
                                } else {
                                    theme::SURFACE_2
                                };
                                let stroke_color = if hovered {
                                    theme::ACCENT
                                } else {
                                    theme::BORDER
                                };
                                ui.painter().rect(
                                    tile_rect,
                                    Rounding::same(6.0),
                                    fill,
                                    Stroke::new(1.0, stroke_color),
                                );

                                if *is_loading {
                                    ui.put(tile_rect, egui::Spinner::new());
                                } else if let Some(tex_handle) = handle {
                                    let inner = tile_rect.shrink(2.0);
                                    let sized = egui::load::SizedTexture::from_handle(tex_handle);
                                    let img_resp = ui.put(
                                        inner,
                                        egui::Image::from_texture(sized)
                                            .fit_to_exact_size(inner.size())
                                            .sense(egui::Sense::click()),
                                    );
                                    if img_resp.clicked() {
                                        *clicked_url = Some(full_url.clone());
                                    }
                                    img_resp.on_hover_text(hover.as_str());
                                } else {
                                    ui.put(
                                        tile_rect,
                                        egui::Label::new(
                                            egui::RichText::new("⚠").color(theme::ERROR),
                                        ),
                                    );
                                }
                            }
                        }
                    });

                    ui.add_space(gap);
                }

                ui.add_space(v_pad);
            });
    }
}

impl View for ArtworkPickerView {
    fn name(&self) -> &str {
        "Artwork Manager"
    }

    fn show(&mut self, ui: &mut Ui, config: &mut AppConfig) {
        if !self.shortcuts_loaded {
            self.load_shortcuts(config);
        }
        if self.active_tab == LibraryTab::SteamLibrary && !self.steam_games_loaded {
            self.load_steam_games();
        }

        self.poll_channels(config);
        self.pump_thumb_queue(ui.ctx());

        egui::SidePanel::left("artwork_game_list")
            .resizable(true)
            .default_width(220.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::SURFACE_0)
                    .inner_margin(Margin {
                        left: 14.0,
                        right: 14.0,
                        top: 16.0,
                        bottom: 12.0,
                    }),
            )
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                self.show_game_list(ui, config);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::SURFACE_1))
            .show_inside(ui, |ui| {
                self.show_right_panel(ui, config);
            });

        self.show_toast(ui.ctx());
    }
}
