use std::fs;
use std::path::Path;

use steam_shortcuts_util::shortcut::ShortcutOwned;
use steam_shortcuts_util::{parse_shortcuts, shortcuts_to_bytes};

/// Reads shortcuts from a shortcuts.vdf file.
/// Returns owned shortcuts so the file bytes do not need to stay in scope.
pub fn read_shortcuts(path: &Path) -> eyre::Result<Vec<ShortcutOwned>> {
    let bytes = fs::read(path)?;
    let borrowed = parse_shortcuts(&bytes).map_err(|e| eyre::eyre!(e))?;
    Ok(borrowed.iter().map(|s| s.to_owned()).collect())
}

/// Writes shortcuts to a shortcuts.vdf file.
/// NOTE: Steam must be restarted for changes to take effect.
pub fn write_shortcuts(path: &Path, shortcuts: &[ShortcutOwned]) -> eyre::Result<()> {
    let borrowed: Vec<_> = shortcuts.iter().map(|s| s.borrow()).collect();
    let bytes = shortcuts_to_bytes(&borrowed);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/shortcuts.vdf");

    #[test]
    fn read_sample_vdf() {
        let shortcuts = read_shortcuts(Path::new(TESTDATA)).unwrap();
        assert!(!shortcuts.is_empty(), "expected at least one shortcut");
        // The crate's own testdata has Celeste as the first entry
        assert_eq!(shortcuts[0].app_name, "Celeste");
    }

    #[test]
    fn roundtrip_write_then_read() {
        let original = read_shortcuts(Path::new(TESTDATA)).unwrap();

        let tmp = std::env::temp_dir().join("vaporforge_test_shortcuts.vdf");
        write_shortcuts(&tmp, &original).unwrap();

        let reloaded = read_shortcuts(&tmp).unwrap();
        assert_eq!(original.len(), reloaded.len());
        for (a, b) in original.iter().zip(reloaded.iter()) {
            assert_eq!(a.app_id, b.app_id);
            assert_eq!(a.app_name, b.app_name);
            assert_eq!(a.exe, b.exe);
        }
    }
}
