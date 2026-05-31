use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub steam: SteamConfig,
    pub scanner: ScannerConfig,
    pub steamgriddb: SteamGridDbConfig,
    /// Set to true by NonSteamManager after writing shortcuts; cleared by ArtworkPickerView.
    #[serde(skip, default)]
    pub shortcuts_changed: bool,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SteamConfig {
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    pub scan_dirs: Vec<String>,
    pub blocklist: Vec<String>,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            scan_dirs: vec![],
            blocklist: vec![
                "Redist".to_string(),
                "DirectX".to_string(),
                "vcredist".to_string(),
                "_CommonRedist".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbScaleConfig {
    #[serde(default = "default_thumb_scale")]
    pub cover: f32,
    #[serde(default = "default_thumb_scale")]
    pub wide_cover: f32,
    #[serde(default = "default_thumb_scale")]
    pub background: f32,
    #[serde(default = "default_thumb_scale")]
    pub logo: f32,
    #[serde(default = "default_thumb_scale")]
    pub icon: f32,
}

fn default_thumb_scale() -> f32 {
    2.0
}

impl Default for ThumbScaleConfig {
    fn default() -> Self {
        Self {
            cover: 1.3,
            wide_cover: 1.4,
            background: 1.2,
            logo: 1.6,
            icon: 1.7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamGridDbConfig {
    pub api_key: String,
    #[serde(default = "default_auto_artwork")]
    pub auto_artwork: bool,
    /// Images per page (10–50). SteamGridDB default is 50; we default to 25.
    #[serde(default = "default_page_size")]
    pub page_size: u8,
    #[serde(default)]
    pub show_nsfw: bool,
    #[serde(default)]
    pub show_humor: bool,
    #[serde(default)]
    pub show_epilepsy: bool,
    #[serde(default)]
    pub thumb_scales: ThumbScaleConfig,
}

fn default_auto_artwork() -> bool {
    true
}
fn default_page_size() -> u8 {
    25
}

impl Default for SteamGridDbConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            auto_artwork: true,
            page_size: 25,
            show_nsfw: false,
            show_humor: false,
            show_epilepsy: false,
            thumb_scales: ThumbScaleConfig::default(),
        }
    }
}

/// Returns `%APPDATA%\VaporForge\config.toml` on Windows,
/// `~/.config/VaporForge/config.toml` on other platforms.
pub fn config_path() -> eyre::Result<PathBuf> {
    let base =
        dirs::config_dir().ok_or_else(|| eyre::eyre!("Could not determine config directory"))?;
    Ok(base.join("VaporForge").join("config.toml"))
}

/// Loads config from the platform config path.
/// Creates and writes the default config if the file does not exist.
pub fn load() -> eyre::Result<AppConfig> {
    load_from(&config_path()?)
}

pub fn load_from(path: &std::path::Path) -> eyre::Result<AppConfig> {
    if !path.exists() {
        let default = AppConfig::default();
        save_to(path, &default)?;
        return Ok(default);
    }
    let text = fs::read_to_string(path)?;
    let config: AppConfig = toml::from_str(&text)?;
    Ok(config)
}

/// Saves config to the platform config path.
pub fn save(config: &AppConfig) -> eyre::Result<()> {
    save_to(&config_path()?, config)
}

pub fn save_to(path: &std::path::Path, config: &AppConfig) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(config)?;
    fs::write(path, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_default_when_missing() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        assert!(!path.exists());
        let config = load_from(&path).unwrap();
        assert!(path.exists(), "default config should have been written");
        assert!(config.steam.user_id.is_empty());
    }

    #[test]
    fn roundtrip_save_load() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let mut original = AppConfig::default();
        original.steamgriddb.api_key = "test-key-123".to_string();
        original.scanner.scan_dirs = vec!["C:\\Games".to_string()];

        save_to(&path, &original).unwrap();
        let loaded = load_from(&path).unwrap();

        assert_eq!(loaded.steamgriddb.api_key, "test-key-123");
        assert_eq!(loaded.scanner.scan_dirs, vec!["C:\\Games"]);
    }

    #[test]
    fn default_blocklist_is_populated() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let config = load_from(&path).unwrap();
        assert!(config.scanner.blocklist.contains(&"Redist".to_string()));
        assert!(config.scanner.blocklist.contains(&"DirectX".to_string()));
    }
}
