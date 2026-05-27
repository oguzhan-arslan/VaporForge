use std::collections::HashSet;
use std::path::{Path, PathBuf};

use steam_shortcuts_util::shortcut::ShortcutOwned;

use crate::config::AppConfig;
use crate::griddb::artwork::apply_artwork;
use crate::griddb::client::{GridDbClient, ImageKind};
use crate::scanner::detector::scan_dirs;
use crate::steam::appid::calculate_app_id;
use crate::steam::paths::{find_steam_dir, find_user_ids, grid_dir, shortcuts_path};
use crate::steam::shortcuts::{read_shortcuts, write_shortcuts};

pub struct AutoAddResult {
    pub added: usize,
    pub skipped: usize,
    pub artwork_applied: usize,
    pub artwork_failed: usize,
}

/// Scans configured directories, compares against existing Steam shortcuts,
/// appends any new games, writes the updated shortcuts.vdf, then optionally
/// auto-fetches Grid artwork for each newly added game.
///
/// NOTE: Steam must be restarted for shortcut changes to take effect.
pub async fn auto_add(config: &AppConfig) -> eyre::Result<AutoAddResult> {
    let steam_dir = find_steam_dir().ok_or_else(|| eyre::eyre!("Steam installation not found"))?;

    let user_ids = if config.steam.user_id.is_empty() {
        find_user_ids(&steam_dir)
    } else {
        vec![config.steam.user_id.parse::<u64>()?]
    };

    if user_ids.is_empty() {
        eyre::bail!("No Steam user IDs found under userdata/");
    }

    let user_id = user_ids[0];
    let vdf_path = shortcuts_path(&steam_dir, user_id);

    let mut shortcuts = if vdf_path.exists() {
        read_shortcuts(&vdf_path)?
    } else {
        vec![]
    };

    let known_ids: HashSet<u32> = shortcuts.iter().map(|s| s.app_id).collect();
    let detected = scan_dirs(&config.scanner.scan_dirs, &config.scanner.blocklist);

    let mut added = 0;
    let mut skipped = 0;
    let mut new_games: Vec<(String, u32)> = Vec::new();

    for game in detected {
        let exe_quoted = format!("\"{}\"", game.exe_path.display());
        let start_dir_quoted = format!("\"{}\"", game.install_dir.display());
        let app_id = calculate_app_id(&exe_quoted, &game.name);

        if known_ids.contains(&app_id) {
            skipped += 1;
            continue;
        }

        let order = shortcuts.len().to_string();
        shortcuts.push(ShortcutOwned {
            order,
            app_id,
            app_name: game.name.clone(),
            exe: exe_quoted,
            start_dir: start_dir_quoted,
            icon: String::new(),
            shortcut_path: String::new(),
            launch_options: String::new(),
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
        tracing::info!("[auto-add] added {:?} (appid {})", game.name, app_id);
        new_games.push((game.name, app_id));
        added += 1;
    }

    if added > 0 {
        write_shortcuts(&vdf_path, &shortcuts)?;
        tracing::info!("[auto-add] NOTE: restart Steam to apply changes");
    }

    tracing::info!(
        "[auto-add] scan complete — {} added, {} skipped",
        added,
        skipped
    );

    let mut artwork_applied = 0;
    let mut artwork_failed = 0;

    if config.steamgriddb.auto_artwork
        && !config.steamgriddb.api_key.is_empty()
        && !new_games.is_empty()
    {
        let client = GridDbClient::new(&config.steamgriddb.api_key);
        let art_dir = grid_dir(&steam_dir, user_id);

        for (name, app_id) in new_games {
            match fetch_grid_artwork(&client, app_id, &name, &art_dir).await {
                Ok(filename) => {
                    tracing::info!("[artwork] applied Grid for {:?} → {}", name, filename);
                    artwork_applied += 1;
                }
                Err(e) => {
                    tracing::warn!("[artwork] failed for {:?}: {e:#}", name);
                    artwork_failed += 1;
                }
            }
        }
    }

    Ok(AutoAddResult { added, skipped, artwork_applied, artwork_failed })
}

async fn fetch_grid_artwork(
    client: &GridDbClient,
    app_id: u32,
    name: &str,
    grid_dir: &Path,
) -> eyre::Result<String> {
    let results = client.search_game(name).await?;
    let first = results
        .into_iter()
        .next()
        .ok_or_else(|| eyre::eyre!("no SteamGridDB results for {name:?}"))?;

    let images = client.get_images(first.id, ImageKind::Grid).await?;
    let image = images
        .into_iter()
        .next()
        .ok_or_else(|| eyre::eyre!("no Grid images for {name:?}"))?;

    let dest = apply_artwork(app_id, ImageKind::Grid, &image.url, grid_dir).await?;
    Ok(dest.file_name().unwrap_or_default().to_string_lossy().into_owned())
}

pub fn format_result(result: &AutoAddResult) -> String {
    format!(
        "{} game(s) added, {} already present. {}",
        result.added,
        result.skipped,
        if result.added > 0 {
            "Restart Steam to apply changes."
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ScannerConfig};
    use std::fs;
    use tempfile::tempdir;

    fn make_exe(path: &Path, size: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; size]).unwrap();
    }

    fn config_for(scan_dir: &str, _vdf_path: &Path) -> AppConfig {
        let mut config = AppConfig::default();
        config.scanner = ScannerConfig {
            scan_dirs: vec![scan_dir.to_string()],
            blocklist: vec!["Redist".to_string()],
        };
        config
    }

    #[test]
    fn adds_new_games_to_empty_shortcuts() {
        let tmp = tempdir().unwrap();
        let scan_root = tmp.path().join("games");
        make_exe(&scan_root.join("Celeste").join("Celeste.exe"), 50_000);
        make_exe(&scan_root.join("Hades").join("Hades.exe"), 40_000);

        let vdf = tmp.path().join("shortcuts.vdf");

        let exe1 = format!(
            "\"{}\"",
            scan_root.join("Celeste").join("Celeste.exe").display()
        );
        let exe2 = format!(
            "\"{}\"",
            scan_root.join("Hades").join("Hades.exe").display()
        );
        let id1 = calculate_app_id(&exe1, "Celeste");
        let id2 = calculate_app_id(&exe2, "Hades");

        write_shortcuts(&vdf, &[]).unwrap();

        let mut shortcuts: Vec<ShortcutOwned> = vec![];
        let known: HashSet<u32> = HashSet::new();

        for (name, exe, install_dir) in [
            ("Celeste", exe1.clone(), scan_root.join("Celeste")),
            ("Hades", exe2.clone(), scan_root.join("Hades")),
        ] {
            let app_id = calculate_app_id(&exe, name);
            if !known.contains(&app_id) {
                shortcuts.push(ShortcutOwned {
                    order: shortcuts.len().to_string(),
                    app_id,
                    app_name: name.to_string(),
                    exe,
                    start_dir: format!("\"{}\"", install_dir.display()),
                    icon: String::new(),
                    shortcut_path: String::new(),
                    launch_options: String::new(),
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
        }

        write_shortcuts(&vdf, &shortcuts).unwrap();
        let reloaded = read_shortcuts(&vdf).unwrap();
        assert_eq!(reloaded.len(), 2);
        let ids: HashSet<u32> = reloaded.iter().map(|s| s.app_id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[test]
    fn skips_already_present_games() {
        let tmp = tempdir().unwrap();
        let scan_root = tmp.path().join("games");
        make_exe(&scan_root.join("Celeste").join("Celeste.exe"), 50_000);

        let exe = format!(
            "\"{}\"",
            scan_root.join("Celeste").join("Celeste.exe").display()
        );
        let app_id = calculate_app_id(&exe, "Celeste");

        let known: HashSet<u32> = [app_id].into();
        assert!(known.contains(&calculate_app_id(&exe, "Celeste")));
    }
}
