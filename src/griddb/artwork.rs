use std::path::{Path, PathBuf};

use crate::griddb::client::ImageKind;

/// Derives the grid artwork filename for `appid` and `kind`.
///
/// The extension is extracted from the URL path (query strings are stripped).
/// Falls back to `png` if the URL has no recognised image extension.
pub fn artwork_filename(appid: u32, kind: ImageKind, url: &str) -> String {
    let path_part = url.split('?').next().unwrap_or(url);
    let ext = path_part
        .rsplit('.')
        .next()
        .map(str::to_lowercase)
        .filter(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg" | "webp"))
        .unwrap_or_else(|| "png".to_string());

    format!("{}{}.{}", appid, kind.filename_suffix(), ext)
}

/// Downloads `url` and writes the artwork to `grid_dir` using the correct filename.
///
/// Creates `grid_dir` if it does not exist.
pub async fn apply_artwork(
    appid: u32,
    kind: ImageKind,
    url: &str,
    grid_dir: &Path,
) -> eyre::Result<PathBuf> {
    let filename = artwork_filename(appid, kind, url);
    let dest = grid_dir.join(&filename);

    let bytes = reqwest::get(url).await?.bytes().await?;
    std::fs::create_dir_all(grid_dir)?;
    std::fs::write(&dest, &bytes)?;

    tracing::info!("artwork saved: {}", dest.display());
    Ok(dest)
}

/// Writes pre-downloaded bytes to `grid_dir` — used internally and in tests.
pub fn write_artwork_bytes(
    appid: u32,
    kind: ImageKind,
    url: &str,
    grid_dir: &Path,
    bytes: &[u8],
) -> eyre::Result<PathBuf> {
    let filename = artwork_filename(appid, kind, url);
    let dest = grid_dir.join(&filename);
    std::fs::create_dir_all(grid_dir)?;
    std::fs::write(&dest, bytes)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn filename_wide_cover() {
        assert_eq!(artwork_filename(1234567890, ImageKind::WideCover, "https://cdn.example.com/img.png"), "1234567890.png");
    }

    #[test]
    fn filename_cover() {
        assert_eq!(artwork_filename(1234567890, ImageKind::Cover, "https://cdn.example.com/img.jpg"), "1234567890p.jpg");
    }

    #[test]
    fn filename_background() {
        assert_eq!(artwork_filename(1234567890, ImageKind::Background, "https://cdn.example.com/img.webp"), "1234567890_hero.webp");
    }

    #[test]
    fn filename_logo() {
        assert_eq!(artwork_filename(1234567890, ImageKind::Logo, "https://cdn.example.com/img.png"), "1234567890_logo.png");
    }

    #[test]
    fn filename_icon() {
        assert_eq!(artwork_filename(1234567890, ImageKind::Icon, "https://cdn.example.com/img.png"), "1234567890_icon.png");
    }

    #[test]
    fn filename_strips_query_string() {
        let url = "https://cdn.example.com/img.jpg?v=2&token=abc";
        assert_eq!(artwork_filename(42, ImageKind::WideCover, url), "42.jpg");
    }

    #[test]
    fn filename_unknown_ext_falls_back_to_png() {
        assert_eq!(artwork_filename(1, ImageKind::Background, "https://cdn.example.com/img.bmp"), "1_hero.png");
    }

    #[test]
    fn filename_ext_is_lowercased() {
        assert_eq!(artwork_filename(1, ImageKind::WideCover, "https://cdn.example.com/img.PNG"), "1.png");
    }

    #[test]
    fn write_artwork_bytes_creates_file() {
        let tmp = tempdir().unwrap();
        let grid_dir = tmp.path().join("grid");

        let dest = write_artwork_bytes(
            999,
            ImageKind::Background,
            "https://cdn.example.com/hero.png",
            &grid_dir,
            b"fake image bytes",
        )
        .unwrap();

        assert!(dest.exists());
        assert_eq!(dest.file_name().unwrap(), "999_hero.png");
        assert_eq!(std::fs::read(&dest).unwrap(), b"fake image bytes");
    }

    #[test]
    fn write_artwork_bytes_creates_grid_dir_if_missing() {
        let tmp = tempdir().unwrap();
        let grid_dir = tmp.path().join("a").join("b").join("grid");

        assert!(!grid_dir.exists());
        write_artwork_bytes(1, ImageKind::Logo, "https://x.com/l.png", &grid_dir, b"x").unwrap();
        assert!(grid_dir.exists());
    }
}
