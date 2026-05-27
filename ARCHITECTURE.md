# VaporForge — Architecture & Reference

Rust application for local Steam library management. Runs primarily as a background service with scheduled tasks. Some features (artwork selection) require a desktop GUI.

---

## Running the Application

```
vaporforge    # starts the GUI and background scheduler together
```

The background scheduler and the GUI always run together in a single process. The scheduler runs on tokio background threads; the GUI (and later the tray icon) runs on the main thread as required by winit.

---

## Module Map

```
src/
  main.rs              entry point — CLI mode dispatch, runtime setup
  config.rs            AppConfig struct, TOML load/save, platform config path
  autoadd.rs           end-to-end auto-add: scan → compare → append → write
  steam/
    mod.rs
    paths.rs           Steam install detection, user ID enumeration, derived paths
    shortcuts.rs       shortcuts.vdf read/write (ShortcutOwned)
    appid.rs           AppID calculation (crc32 + high bit)
  scanner/
    mod.rs
    detector.rs        recursive .exe scanner; largest-binary-per-folder heuristic
  griddb/
    mod.rs
    client.rs          GridDbClient wrapping steamgriddb_api; ImageKind enum
    artwork.rs         artwork_filename(), apply_artwork(), write_artwork_bytes()
  scheduler/
    mod.rs             Task type, run_tasks(), async run() loop
  ui/
    mod.rs             VaporForgeApp (eframe::App), View trait, tab bar, API key warning
    artwork_picker.rs  ArtworkPickerView — full artwork browser UI
```

**Architecture principle:** each feature is a self-contained module under `src/`. Adding new features wires into `main.rs` and `scheduler` only.

---

## Key Design Decisions

### GUI / Runtime split
`eframe` (winit) requires the main thread for its event loop. A multi-threaded tokio runtime is built with `Builder::new_multi_thread()` and entered on the main thread via `rt.enter()`. The scheduler is spawned as a background tokio task. The GUI then runs on the main thread, and can dispatch async work via `Handle::current()`.

### Extensible scheduler
`scheduler::Task` is `Arc<dyn Fn(&AppConfig) -> Result + Send + Sync>`. New tasks are pushed to a `Vec<Task>` in `main.rs` before calling `scheduler::run`; the loop itself never changes.

### Extensible UI tabs
`ui::View` trait (`name()` + `show(ui, config)`). Adding a tab = implement the trait, push one `Box::new(...)` into `VaporForgeApp::new`. No other code changes.

### Async in the UI
`ArtworkPickerView` uses `std::sync::mpsc` channels + `Handle::current().spawn(...)` to offload network calls. The `update()` loop polls channels via `try_recv()` each frame — no blocking, no unsafe.

---

## Crate Reference

| Purpose | Crate | Notes |
|---|---|---|
| shortcuts.vdf read/write | `steam_shortcuts_util 1.x` | Provides `parse_shortcuts`, `shortcuts_to_bytes`, `ShortcutOwned` |
| SteamGridDB client | `steamgriddb_api 0.3` | Read-only; wraps to `GridDbClient` with `eyre` errors |
| Steam path detection | `steamlocate 2.x` | `steamlocate::locate()` → `SteamDir`; `.path()` gives install root |
| Desktop GUI | `egui 0.29` + `eframe 0.29` | Immediate-mode; `eframe::App::update` called each frame |
| Image rendering | `egui_extras 0.29` (`all_loaders`) | `egui_extras::install_image_loaders(ctx)` enables URL thumbnails |
| Async runtime | `tokio 1` (full) | Multi-threaded; background threads only in GUI mode |
| HTTP client | `reqwest 0.12` (`rustls-tls`) | Used directly for artwork download |
| Config | `serde` + `toml 0.8` | `toml::to_string_pretty` / `toml::from_str` |
| Directory scan | `walkdir 2` | `filter_entry` prunes blocklisted dirs before recursing |
| Error handling | `eyre 0.6` + `color-eyre 0.6` | `eyre::eyre!()` at boundaries; `?` everywhere else |
| AppID hashing | `crc32fast 1` | `hash(bytes) \| 0x80000000` — matches Steam's algorithm |
| Platform dirs | `dirs 5` | `dirs::config_dir()` → `%APPDATA%` on Windows |
| Logging | `log 0.4` | Facade only; no subscriber wired yet (Task 13) |

---

## Steam File Paths (Windows)

```
Steam install:      C:\Program Files (x86)\Steam\
shortcuts.vdf:      {steam}\userdata\{userid}\config\shortcuts.vdf
Grid artwork dir:   {steam}\userdata\{userid}\config\grid\
```

Derived by `steam::paths::{shortcuts_path, grid_dir}`.

---

## AppID Formula for Non-Steam Games

```rust
fn calculate_app_id(exe_path: &str, app_name: &str) -> u32 {
    let input = format!("{}{}", exe_path, app_name);
    crc32fast::hash(input.as_bytes()) | 0x80000000
}
```

`exe_path` must use the quoted Steam format: `"\"C:\\Games\\game.exe\""`.  
Matches Steam's own algorithm; verified against BoilR and steam_shortcuts_util.

---

## Artwork Filename Convention

For a non-Steam game with `appid = 1234567890`:

| Kind | Filename |
|---|---|
| Grid (horizontal) | `1234567890.png` |
| Portrait (vertical) | `1234567890p.png` |
| Hero (banner) | `1234567890_hero.png` |
| Logo | `1234567890_logo.png` |
| Icon | `1234567890_icon.png` |

Extension is preserved from the source URL (`png`, `jpg`, `jpeg`, `webp`); falls back to `png`.

---

## SteamGridDB API

- Base URL: `https://www.steamgriddb.com/api/v2`
- Auth: `Authorization: Bearer {api_key}`
- Key obtained from: `https://www.steamgriddb.com/profile/preferences/api`
- Grid and Portrait images share the `/grids` endpoint; `ImageKind::Portrait` maps to `QueryType::Grid(None)` — the UI distinguishes by aspect ratio.

---

## Config File

Location: `%APPDATA%\VaporForge\config.toml` (Windows) / `~/.config/VaporForge/config.toml`

```toml
[steam]
user_id = ""           # override auto-detected user; blank = auto

[scanner]
scan_dirs = []
blocklist = ["Redist", "DirectX", "vcredist", "_CommonRedist"]

[scheduler]
scan_interval_minutes = 60

[steamgriddb]
api_key = ""
```

Created with defaults on first run.

---

## Data Flow: Auto-Add

```
scheduler::run()
  └─ autoadd::auto_add(config)
       ├─ steam::paths::find_steam_dir()
       ├─ steam::paths::find_user_ids()
       ├─ steam::shortcuts::read_shortcuts(vdf_path)  → Vec<ShortcutOwned>
       ├─ build known AppID set
       ├─ scanner::detector::scan_dirs(scan_dirs, blocklist)  → Vec<DetectedGame>
       ├─ for each new game: append ShortcutOwned
       └─ steam::shortcuts::write_shortcuts(vdf_path, &shortcuts)
          NOTE: Steam must be restarted to apply changes
```

## Data Flow: Artwork Picker

```
ArtworkPickerView::show()
  ├─ [once] read_shortcuts → game list in left panel
  ├─ [on game select] GridDbClient::search_game(name) → search_results (async)
  ├─ [on result / kind tab change] GridDbClient::get_images(sgdb_id, kind) → images (async)
  └─ [on thumbnail click] reqwest::get(url) → write_artwork_bytes(appid, kind, url, grid_dir)
```
