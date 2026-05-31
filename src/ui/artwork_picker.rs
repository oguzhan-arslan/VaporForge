use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::{Align2, Margin, Rounding, ScrollArea, Stroke, Ui};
use steam_shortcuts_util::shortcut::ShortcutOwned;

use crate::config::AppConfig;
use crate::griddb::artwork::{delete_all_artwork, delete_artwork, write_artwork_bytes};
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

// ── Steam Store search result ──────────────────────────────────────────────

struct SteamSearchResult {
    app_id: u32,
    name: String,
}

// ── async result channels ──────────────────────────────────────────────────

type SearchRx = mpsc::Receiver<eyre::Result<Vec<GameResult>>>;
type ImagesRx = mpsc::Receiver<eyre::Result<Vec<ImageResult>>>;
type ApplyRx = mpsc::Receiver<eyre::Result<PathBuf>>;
type SteamRx = mpsc::Receiver<eyre::Result<Vec<SteamSearchResult>>>;
type SgdbIdRx = mpsc::Receiver<eyre::Result<Option<usize>>>;
type ThumbTx = mpsc::Sender<(usize, eyre::Result<DecodedThumb>)>;
type ThumbRx = mpsc::Receiver<(usize, eyre::Result<DecodedThumb>)>;

pub struct ArtworkPickerView {
    // local game lists (both loaded eagerly)
    shortcuts: Vec<ShortcutOwned>,
    shortcuts_loaded: bool,
    load_error: Option<String>,
    steam_games: Vec<SteamGame>,
    steam_games_loaded: bool,

    // left-panel search bar (drives both local filter and debounced SGDB lookup)
    search_query: String,

    // selection — mutually exclusive: local game (by app_id) or fetch result
    selected_app_id: Option<u32>,
    is_fetch_selection: bool,
    selected_fetch_idx: usize,

    // image kind tabs
    active_kind: ImageKind,

    // SteamGridDB search (for local game auto-search)
    search_rx: Option<SearchRx>,
    searching: bool,
    search_results: Vec<GameResult>,
    selected_sgdb_idx: usize,

    // image metadata fetch
    images_rx: Option<ImagesRx>,
    fetching: bool,
    loaded_for: Option<(usize, ImageKind)>,
    current_page: usize,
    fetching_page: usize,
    images_has_more: bool,

    // thumbnail gallery
    gallery: Vec<GalleryEntry>,
    active_loads: usize,
    thumb_tx: Option<ThumbTx>,
    thumb_rx: Option<ThumbRx>,

    // apply artwork
    apply_rx: Option<ApplyRx>,
    applying: bool,

    // SGDB ID resolution for "Other Matches" (Steam AppID → SGDB ID → images)
    sgdb_id_rx: Option<SgdbIdRx>,
    resolving_sgdb_id: bool,
    resolved_sgdb_id: Option<usize>,

    // debounced Steam Store search driven by search_query
    fetch_pending_since: Option<Instant>,
    fetch_rx: Option<SteamRx>,
    fetch_searching: bool,
    fetch_results: Vec<SteamSearchResult>,

    // toast notification
    toast: Option<ToastNotification>,
}

impl ArtworkPickerView {
    pub fn new() -> Self {
        Self {
            shortcuts: vec![],
            shortcuts_loaded: false,
            load_error: None,
            steam_games: vec![],
            steam_games_loaded: false,
            search_query: String::new(),
            selected_app_id: None,
            is_fetch_selection: false,
            selected_fetch_idx: 0,
            active_kind: ImageKind::Cover,
            search_rx: None,
            searching: false,
            search_results: vec![],
            selected_sgdb_idx: 0,
            images_rx: None,
            fetching: false,
            loaded_for: None,
            current_page: 0,
            fetching_page: 0,
            images_has_more: false,
            gallery: vec![],
            active_loads: 0,
            thumb_tx: None,
            thumb_rx: None,
            apply_rx: None,
            applying: false,
            sgdb_id_rx: None,
            resolving_sgdb_id: false,
            resolved_sgdb_id: None,
            fetch_pending_since: None,
            fetch_rx: None,
            fetch_searching: false,
            fetch_results: vec![],
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

    // ── selection helpers ─────────────────────────────────────────────────

    fn select_local_game(&mut self, app_id: u32, name: &str, config: &AppConfig) {
        self.selected_app_id = Some(app_id);
        self.is_fetch_selection = false;
        self.resolved_sgdb_id = None;
        self.resolving_sgdb_id = false;
        self.sgdb_id_rx = None;
        self.search_results.clear();
        self.clear_gallery();

        if !config.steamgriddb.api_key.is_empty() {
            self.trigger_search(config.steamgriddb.api_key.clone(), name.to_string());
        } else {
            tracing::warn!("No SteamGridDB API key — set one in Settings to search for artwork.");
        }
    }

    fn select_fetch_result(&mut self, idx: usize, config: &AppConfig) {
        self.is_fetch_selection = true;
        self.selected_fetch_idx = idx;
        self.resolved_sgdb_id = None;
        self.search_results.clear();
        self.selected_sgdb_idx = 0;
        self.clear_gallery();

        if let Some(r) = self.fetch_results.get(idx) {
            self.selected_app_id = Some(r.app_id);
            if !config.steamgriddb.api_key.is_empty() {
                self.trigger_sgdb_id_resolve(r.app_id, config.steamgriddb.api_key.clone());
            }
        }
    }

    // ── app_id / name resolution ──────────────────────────────────────────

    fn selected_app_id(&self) -> u32 {
        self.selected_app_id.unwrap_or(0)
    }

    // ── options builder ───────────────────────────────────────────────────

    fn selected_game_name(&self) -> String {
        if self.is_fetch_selection {
            return self
                .fetch_results
                .get(self.selected_fetch_idx)
                .map(|r| r.name.clone())
                .unwrap_or_default();
        }
        let Some(app_id) = self.selected_app_id else {
            return String::new();
        };
        if let Some(sc) = self.shortcuts.iter().find(|s| s.app_id == app_id) {
            return sc.app_name.clone();
        }
        if let Some(g) = self.steam_games.iter().find(|g| g.app_id == app_id) {
            return g.name.clone();
        }
        String::new()
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
    ) {
        let (tx, rx) = mpsc::channel();
        self.images_rx = Some(rx);
        self.fetching = true;
        self.fetching_page = opts.page;
        self.clear_gallery();

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

    fn trigger_apply(&mut self, url: String, appid: u32, kind: ImageKind, grid_dir: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.apply_rx = Some(rx);
        self.applying = true;

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Tokio runtime not available.");
            self.applying = false;
            return;
        };

        handle.spawn(async move {
            let result: eyre::Result<PathBuf> = async {
                let bytes = reqwest::get(&url).await?.bytes().await?;
                write_artwork_bytes(appid, kind, &url, &grid_dir, &bytes)
            }
            .await;
            let _ = tx.send(result);
        });
    }

    fn trigger_sgdb_id_resolve(&mut self, steam_appid: u32, api_key: String) {
        let (tx, rx) = mpsc::channel();
        self.sgdb_id_rx = Some(rx);
        self.resolving_sgdb_id = true;

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.resolving_sgdb_id = false;
            return;
        };

        handle.spawn(async move {
            let result: eyre::Result<Option<usize>> = GridDbClient::new(api_key)
                .game_by_steam_appid(steam_appid)
                .await;
            let _ = tx.send(result);
        });
    }

    fn trigger_fetch_search(&mut self) {
        let query = self.search_query.clone();
        if query.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.fetch_rx = Some(rx);
        self.fetch_searching = true;
        self.fetch_results.clear();

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::error!("Tokio runtime not available for Steam search.");
            self.fetch_searching = false;
            return;
        };

        handle.spawn(async move {
            let result: eyre::Result<Vec<SteamSearchResult>> = async {
                let resp = reqwest::Client::new()
                    .get("https://store.steampowered.com/api/storesearch/")
                    .query(&[
                        ("term", &query),
                        ("l", &"english".to_string()),
                        ("cc", &"US".to_string()),
                    ])
                    .send()
                    .await?
                    .json::<serde_json::Value>()
                    .await?;
                let items = resp["items"].as_array().cloned().unwrap_or_default();
                Ok(items
                    .into_iter()
                    .filter_map(|item| {
                        Some(SteamSearchResult {
                            app_id: item["id"].as_u64()? as u32,
                            name: item["name"].as_str()?.to_string(),
                        })
                    })
                    .collect())
            }
            .await;
            let _ = tx.send(result);
        });
    }

    fn check_fetch_debounce(&mut self, _config: &AppConfig) {
        let Some(since) = self.fetch_pending_since else {
            return;
        };
        if since.elapsed() >= Duration::from_millis(500) {
            self.fetch_pending_since = None;
            self.trigger_fetch_search();
        }
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
                    let img =
                        image::load_from_memory(&bytes).map_err(|e| eyre::eyre!("decode: {e}"))?;
                    let rgba8 = img.to_rgba8();
                    let width = rgba8.width() as usize;
                    let height = rgba8.height() as usize;
                    Ok(DecodedThumb {
                        width,
                        height,
                        rgba: rgba8.into_raw(),
                    })
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
                match result {
                    Ok(images) => {
                        let page_size = config.steamgriddb.page_size as usize;

                        let images: Vec<ImageResult> = images
                            .into_iter()
                            .filter(|img| config.steamgriddb.show_epilepsy || !img.epilepsy)
                            .collect();

                        let received = images.len();
                        self.images_has_more = received >= page_size;

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
                        self.current_page = self.fetching_page;
                        self.loaded_for = Some((self.selected_sgdb_idx, self.active_kind));
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
                            short_name,
                        );
                        self.toast = Some(ToastNotification {
                            message: format!("Artwork saved: {short_name}"),
                            file_path: Some(dest),
                            expires_at: Instant::now() + Duration::from_secs(6),
                        });
                    }
                    Err(e) => tracing::error!("Artwork apply failed: {e}"),
                }
            }
        }

        // ── SGDB ID resolution for "Other Matches" ────────────────────────
        if let Some(rx) = &self.sgdb_id_rx {
            if let Ok(result) = rx.try_recv() {
                self.resolving_sgdb_id = false;
                self.sgdb_id_rx = None;
                match result {
                    Ok(Some(sgdb_id)) => {
                        tracing::info!("Resolved SGDB ID: {sgdb_id}");
                        self.resolved_sgdb_id = Some(sgdb_id);
                        if !config.steamgriddb.api_key.is_empty() {
                            let opts = Self::make_opts(config, 0);
                            self.trigger_image_fetch(
                                config.steamgriddb.api_key.clone(),
                                sgdb_id,
                                self.active_kind,
                                opts,
                            );
                        }
                    }
                    Ok(None) => tracing::warn!("No SGDB entry found for this Steam game."),
                    Err(e) => tracing::error!("SGDB ID resolve failed: {e}"),
                }
            }
        }

        // ── Steam Store search results ────────────────────────────────────
        if let Some(rx) = &self.fetch_rx {
            if let Ok(result) = rx.try_recv() {
                self.fetch_searching = false;
                self.fetch_rx = None;
                match result {
                    Ok(results) => {
                        let n = results.len();
                        self.fetch_results = results;
                        if n == 0 {
                            tracing::info!("Steam search: no results.");
                        } else {
                            tracing::info!("Steam search: {n} result(s).");
                        }
                    }
                    Err(e) => tracing::error!("Steam search failed: {e}"),
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
                        egui::RichText::new("Artwork applied")
                            .color(theme::SUCCESS)
                            .size(13.0)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("X").clicked() {
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
        let mut local_clicked: Option<(u32, String)> = None;
        let mut fetch_clicked: Option<usize> = None;

        // ── header ────────────────────────────────────────────────────────
        let total = self.shortcuts.len() + self.steam_games.len();
        let filter_lc = self.search_query.to_lowercase();
        let filtered = if filter_lc.is_empty() {
            total
        } else {
            self.shortcuts
                .iter()
                .filter(|s| s.app_name.to_lowercase().contains(&filter_lc))
                .count()
                + self
                    .steam_games
                    .iter()
                    .filter(|g| g.name.to_lowercase().contains(&filter_lc))
                    .count()
        };
        let count_label = if filter_lc.is_empty() {
            format!("Found Games ({})", total)
        } else {
            format!("Found Games ({} of {})", filtered, total)
        };
        ui.label(
            egui::RichText::new(count_label)
                .size(13.0)
                .color(theme::TEXT_2),
        );
        ui.add_space(6.0);

        if let Some(err) = &self.load_error {
            ui.label(egui::RichText::new(err).color(theme::ERROR).size(12.0));
        }

        // ── single search bar ─────────────────────────────────────────────
        let search_changed = ui
            .add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search games...")
                    .desired_width(f32::INFINITY),
            )
            .changed();

        if search_changed {
            self.fetch_results.clear();
            self.fetch_searching = false;
            if self.search_query.is_empty() {
                self.fetch_pending_since = None;
            } else {
                self.fetch_pending_since = Some(Instant::now());
            }
            if self.is_fetch_selection {
                self.is_fetch_selection = false;
                self.selected_app_id = None;
                self.search_results.clear();
                self.clear_gallery();
            }
        }

        ui.add_space(6.0);

        // ── combined scrollable list ──────────────────────────────────────
        let mut all_games: Vec<(u32, &str)> = self
            .shortcuts
            .iter()
            .map(|s| (s.app_id, s.app_name.as_str()))
            .chain(self.steam_games.iter().map(|g| (g.app_id, g.name.as_str())))
            .filter(|(_, name)| filter_lc.is_empty() || name.to_lowercase().contains(&filter_lc))
            .collect();
        all_games.sort_unstable_by_key(|(_, name)| name.to_ascii_lowercase());

        ScrollArea::vertical()
            .id_salt("artwork_game_list_scroll")
            .show(ui, |ui| {
                if all_games.is_empty() && !filter_lc.is_empty() {
                    ui.label(
                        egui::RichText::new("No local games match.")
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                } else {
                    for (app_id, name) in &all_games {
                        let sel = self.selected_app_id == Some(*app_id) && !self.is_fetch_selection;
                        if ui.selectable_label(sel, *name).clicked() && !sel {
                            local_clicked = Some((*app_id, name.to_string()));
                        }
                    }
                }

                // ── SteamGridDB results (only when a query is active) ─────
                if !self.search_query.is_empty() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Other Matches")
                            .size(11.5)
                            .color(theme::TEXT_DIM),
                    );
                    ui.add_space(4.0);

                    if self.fetch_pending_since.is_some() || self.fetch_searching {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Searching...")
                                    .color(theme::TEXT_DIM)
                                    .size(11.5),
                            );
                        });
                        ui.ctx().request_repaint_after(Duration::from_millis(50));
                    } else if !self.fetch_results.is_empty() {
                        for (i, result) in self.fetch_results.iter().enumerate() {
                            let sel = self.is_fetch_selection && self.selected_fetch_idx == i;
                            if ui
                                .selectable_label(sel, egui::RichText::new(&result.name).size(12.5))
                                .clicked()
                                && !sel
                            {
                                fetch_clicked = Some(i);
                            }
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("No results.")
                                .small()
                                .color(theme::TEXT_DIM),
                        );
                    }
                }
            });

        // ── deferred actions ──────────────────────────────────────────────
        if let Some((app_id, name)) = local_clicked {
            self.select_local_game(app_id, &name, config);
        }
        if let Some(i) = fetch_clicked {
            self.select_fetch_result(i, config);
        }
    }

    fn show_right_panel(&mut self, ui: &mut Ui, config: &mut AppConfig) {
        let mut kind_changed: Option<ImageKind> = None;
        let mut load_prev = false;
        let mut load_next = false;
        let mut clicked_url: Option<String> = None;
        let mut reset_kind = false;
        let mut reset_all = false;

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
                    // Size slider — left side
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

                    // Pagination — right aligned
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.fetching {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Loading...")
                                    .color(theme::TEXT_2)
                                    .size(12.0),
                            );
                        } else if !self.gallery.is_empty() {
                            // Next
                            let next_btn = egui::Button::new(
                                egui::RichText::new("Next").size(12.5).color(theme::TEXT),
                            )
                            .fill(theme::SURFACE_2)
                            .stroke(Stroke::new(1.0, theme::BORDER_STRONG));
                            if ui.add_enabled(self.images_has_more, next_btn).clicked() {
                                load_next = true;
                            }

                            // Page indicator
                            ui.label(
                                egui::RichText::new(format!("Page {}", self.current_page + 1))
                                    .size(12.0)
                                    .color(theme::TEXT_DIM),
                            );

                            // Prev
                            let prev_btn = egui::Button::new(
                                egui::RichText::new("Prev").size(12.5).color(theme::TEXT),
                            )
                            .fill(theme::SURFACE_2)
                            .stroke(Stroke::new(1.0, theme::BORDER_STRONG));
                            if ui.add_enabled(self.current_page > 0, prev_btn).clicked() {
                                load_prev = true;
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

                    // SGDB match picker — only for local games (multiple SGDB candidates)
                    if !self.is_fetch_selection && self.search_results.len() > 1 {
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
                                );
                            }
                        }

                        ui.add_space(4.0);
                        ui.add(egui::Separator::default().vertical().spacing(0.0));
                        ui.add_space(4.0);
                    }

                    // Right-aligned: reset buttons + status indicators
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.applying {
                            ui.label(
                                egui::RichText::new("Applying...")
                                    .color(theme::TEXT_2)
                                    .size(12.0),
                            );
                            ui.spinner();
                        } else if self.resolving_sgdb_id {
                            ui.label(
                                egui::RichText::new("Finding artwork source...")
                                    .color(theme::TEXT_DIM)
                                    .size(12.0),
                            );
                            ui.spinner();
                        } else if self.is_fetch_selection {
                            if let Some(id) = self.selected_app_id {
                                ui.label(
                                    egui::RichText::new(format!("Steam AppID: {id}"))
                                        .color(theme::TEXT_DIM)
                                        .size(11.5),
                                );
                            }
                        }

                        // Reset buttons — only when a game is selected
                        if self.selected_app_id.is_some() {
                            ui.add_space(8.0);
                            ui.add(egui::Separator::default().vertical().spacing(0.0));
                            ui.add_space(4.0);

                            let reset_all_btn = egui::Button::new(
                                egui::RichText::new("Reset All")
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(220, 80, 80)),
                            )
                            .fill(theme::SURFACE_2)
                            .stroke(Stroke::new(1.0, theme::BORDER));
                            if ui
                                .add(reset_all_btn)
                                .on_hover_text("Delete all artwork for this game")
                                .clicked()
                            {
                                reset_all = true;
                            }

                            let reset_kind_label =
                                format!("Reset {}", kind_label(self.active_kind));
                            let reset_kind_btn = egui::Button::new(
                                egui::RichText::new(reset_kind_label)
                                    .size(12.0)
                                    .color(egui::Color32::from_rgb(220, 80, 80)),
                            )
                            .fill(theme::SURFACE_2)
                            .stroke(Stroke::new(1.0, theme::BORDER));
                            if ui
                                .add(reset_kind_btn)
                                .on_hover_text("Delete this artwork type for this game")
                                .clicked()
                            {
                                reset_kind = true;
                            }
                        }
                    });
                });
            });

        // Handle kind change after borrowing config
        if let Some(kind) = kind_changed {
            self.active_kind = kind;
            let sgdb_id = if self.is_fetch_selection {
                self.resolved_sgdb_id
            } else if !self.search_results.is_empty() {
                Some(self.search_results[self.selected_sgdb_idx].id)
            } else {
                None
            };
            let needs_fetch = self
                .loaded_for
                .map(|(_, k)| k != kind)
                .unwrap_or(sgdb_id.is_some());
            if let (true, Some(id)) = (needs_fetch, sgdb_id) {
                if !config.steamgriddb.api_key.is_empty() {
                    let opts = Self::make_opts(config, 0);
                    self.trigger_image_fetch(config.steamgriddb.api_key.clone(), id, kind, opts);
                }
            }
        }

        // ── Central: gallery ──────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::SURFACE_1))
            .show_inside(ui, |ui| {
                self.show_gallery_area(ui, config, &mut clicked_url);
            });

        // Handle reset artwork
        if reset_kind || reset_all {
            let appid = self.selected_app_id();
            let gdir = find_steam_dir().and_then(|steam_dir| {
                let uid = config
                    .steam
                    .user_id
                    .parse::<u64>()
                    .ok()
                    .or_else(|| find_user_ids(&steam_dir).into_iter().next())?;
                Some(grid_dir(&steam_dir, uid))
            });
            if let Some(ref gdir) = gdir {
                if reset_all {
                    delete_all_artwork(appid, gdir);
                    tracing::info!("All artwork reset for appid {appid}.");
                    self.toast = Some(ToastNotification {
                        message: "All artwork deleted.".to_string(),
                        file_path: None,
                        expires_at: Instant::now() + Duration::from_secs(4),
                    });
                } else {
                    let removed = delete_artwork(appid, self.active_kind, gdir);
                    if removed {
                        tracing::info!(
                            "{} artwork reset for appid {appid}.",
                            kind_label(self.active_kind)
                        );
                        self.toast = Some(ToastNotification {
                            message: format!("{} artwork deleted.", kind_label(self.active_kind)),
                            file_path: None,
                            expires_at: Instant::now() + Duration::from_secs(4),
                        });
                    }
                }
            }
        }

        // Handle prev/next page navigation
        let nav_page: Option<usize> = if load_next {
            Some(self.current_page + 1)
        } else if load_prev {
            Some(self.current_page.saturating_sub(1))
        } else {
            None
        };
        if let Some(page) = nav_page {
            let sgdb_id = if self.is_fetch_selection {
                self.resolved_sgdb_id
            } else if !self.search_results.is_empty() {
                Some(self.search_results[self.selected_sgdb_idx].id)
            } else {
                None
            };
            if let Some(id) = sgdb_id {
                let opts = Self::make_opts(config, page);
                self.trigger_image_fetch(
                    config.steamgriddb.api_key.clone(),
                    id,
                    self.active_kind,
                    opts,
                );
            }
        }

        // Handle image clicks
        if let Some(url) = clicked_url {
            let appid = self.selected_app_id();
            let kind = self.active_kind;
            let gdir = find_steam_dir().and_then(|steam_dir| {
                let uid = config
                    .steam
                    .user_id
                    .parse::<u64>()
                    .ok()
                    .or_else(|| find_user_ids(&steam_dir).into_iter().next())?;
                Some(grid_dir(&steam_dir, uid))
            });
            if let Some(gdir) = gdir {
                self.trigger_apply(url, appid, kind, gdir);
            }
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
                        egui::RichText::new("Searching SteamGridDB...")
                            .color(theme::TEXT_2)
                            .size(13.0),
                    );
                });
            });
            return;
        }
        if self.fetching {
            ui.centered_and_justified(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("Loading images...")
                            .color(theme::TEXT_2)
                            .size(13.0),
                    );
                });
            });
            return;
        }
        let has_selection = self.selected_app_id.is_some() || self.is_fetch_selection;
        if !has_selection {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Select a game to browse artwork.")
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
                let hover = format!("{}x{}  {:?}", e.meta.width, e.meta.height, e.meta.mime);
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
                                            egui::RichText::new("!").color(theme::ERROR),
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
        if config.shortcuts_changed {
            self.shortcuts_loaded = false;
            config.shortcuts_changed = false;
        }
        if !self.shortcuts_loaded {
            self.load_shortcuts(config);
        }
        if !self.steam_games_loaded {
            self.load_steam_games();
        }

        self.check_fetch_debounce(config);
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
