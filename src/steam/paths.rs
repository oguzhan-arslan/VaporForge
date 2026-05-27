use std::fs;
use std::path::{Path, PathBuf};

pub fn find_steam_dir() -> Option<PathBuf> {
    steamlocate::locate().ok().map(|d| d.path().to_owned())
}

pub fn find_user_ids(steam_dir: &Path) -> Vec<u64> {
    let userdata = steam_dir.join("userdata");
    let Ok(entries) = fs::read_dir(&userdata) else {
        return vec![];
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_string_lossy().parse::<u64>().ok())
        .collect()
}

pub fn shortcuts_path(steam_dir: &Path, user_id: u64) -> PathBuf {
    steam_dir
        .join("userdata")
        .join(user_id.to_string())
        .join("config")
        .join("shortcuts.vdf")
}

pub fn grid_dir(steam_dir: &Path, user_id: u64) -> PathBuf {
    steam_dir
        .join("userdata")
        .join(user_id.to_string())
        .join("config")
        .join("grid")
}
