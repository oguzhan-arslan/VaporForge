pub struct SteamGame {
    pub app_id: u32,
    pub name: String,
}

/// Returns all installed Steam games, alphabetically sorted, with non-game
/// entries (Proton, runtimes, tools) filtered out.
pub fn list_installed_games() -> Vec<SteamGame> {
    let steam = match steamlocate::locate() {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let libraries = match steam.libraries() {
        Ok(l) => l,
        Err(_) => return vec![],
    };

    let mut games = Vec::new();
    for library in libraries {
        let Ok(library) = library else { continue };
        for app in library.apps() {
            let Ok(app) = app else { continue };
            if !is_game(app.app_id, app.name.as_deref()) {
                continue;
            }
            if let Some(name) = app.name {
                games.push(SteamGame { app_id: app.app_id, name });
            }
        }
    }

    games.sort_by(|a, b| a.name.cmp(&b.name));
    games
}

fn is_game(app_id: u32, name: Option<&str>) -> bool {
    let name = name.unwrap_or("");
    !name.starts_with("Proton")
        && !name.contains("Steam Linux Runtime")
        && !name.contains("Steamworks")
        && app_id > 1000
}
