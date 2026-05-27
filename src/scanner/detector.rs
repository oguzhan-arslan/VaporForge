use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct DetectedGame {
    pub name: String,
    pub exe_path: PathBuf,
    pub install_dir: PathBuf,
}

/// Scans `roots` for game folders.
///
/// Each scan root is expected to contain one subdirectory per game. The
/// function walks each immediate subdirectory, picks the largest `.exe` found
/// anywhere within it, and emits one `DetectedGame` per subdirectory.
/// Subdirectories (at any depth) whose names appear in `blocklist` are skipped.
pub fn scan_dirs(roots: &[String], blocklist: &[String]) -> Vec<DetectedGame> {
    // game_root → (best_exe, best_size)
    let mut best: HashMap<PathBuf, (PathBuf, u64)> = HashMap::new();

    for root in roots {
        let root_path = Path::new(root);
        let walker = walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_blocked(e.file_name().to_string_lossy().as_ref(), blocklist));

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false) {
                continue;
            }

            // Key on the immediate subdirectory of the scan root, not the
            // exe's parent. This ensures one entry per game folder regardless
            // of how deeply nested the exe is.
            let game_root = match game_root_for(path, root_path) {
                Some(r) => r,
                None => continue, // exe is directly inside the scan root — skip
            };

            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let record = best.entry(game_root).or_insert_with(|| (path.to_path_buf(), 0));
            if size > record.1 {
                *record = (path.to_path_buf(), size);
            }
        }
    }

    best.into_iter()
        .filter_map(|(game_root, (exe_path, _))| {
            let name = game_root.file_name()?.to_string_lossy().into_owned();
            Some(DetectedGame { name, exe_path, install_dir: game_root })
        })
        .collect()
}

/// Returns the immediate subdirectory of `scan_root` that contains `exe_path`,
/// or `None` if the exe sits directly inside `scan_root` (no game subfolder).
fn game_root_for(exe_path: &Path, scan_root: &Path) -> Option<PathBuf> {
    let relative = exe_path.strip_prefix(scan_root).ok()?;
    let mut components = relative.components();
    let first = components.next()?;
    // Require at least one more component so we know `first` is a directory,
    // not the exe file itself living directly in the scan root.
    components.next()?;
    Some(scan_root.join(first))
}

fn is_blocked(name: &str, blocklist: &[String]) -> bool {
    let lower = name.to_lowercase();
    blocklist.iter().any(|b| lower.contains(&b.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_exe(path: &Path, size: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; size]).unwrap();
    }

    #[test]
    fn detects_largest_exe_per_game_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Celeste: two exes — launcher (small) and main binary (large)
        make_exe(&root.join("Celeste").join("CelesteRunner.exe"), 1_000);
        make_exe(&root.join("Celeste").join("Celeste.exe"), 50_000);

        // Hades: one exe
        make_exe(&root.join("Hades").join("Hades.exe"), 30_000);

        let games = scan_dirs(&[root.to_string_lossy().into_owned()], &[]);
        assert_eq!(games.len(), 2);

        let celeste = games.iter().find(|g| g.name == "Celeste").unwrap();
        assert_eq!(celeste.exe_path.file_name().unwrap(), "Celeste.exe");
        assert_eq!(celeste.install_dir, root.join("Celeste"));

        let hades = games.iter().find(|g| g.name == "Hades").unwrap();
        assert_eq!(hades.exe_path.file_name().unwrap(), "Hades.exe");
    }

    #[test]
    fn nested_exe_maps_to_game_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Exe is nested in a bin/ subdirectory — install_dir must still be the game root
        make_exe(&root.join("DeepGame").join("bin").join("DeepGame.exe"), 40_000);

        let games = scan_dirs(&[root.to_string_lossy().into_owned()], &[]);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "DeepGame");
        assert_eq!(games[0].install_dir, root.join("DeepGame"));
        assert_eq!(games[0].exe_path.file_name().unwrap(), "DeepGame.exe");
    }

    #[test]
    fn exe_directly_in_scan_root_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // This exe is directly in the scan root (no game subfolder) — should be ignored
        make_exe(&root.join("stray.exe"), 10_000);
        // A proper game nearby
        make_exe(&root.join("ProperGame").join("ProperGame.exe"), 20_000);

        let games = scan_dirs(&[root.to_string_lossy().into_owned()], &[]);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "ProperGame");
    }

    #[test]
    fn blocklist_skips_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        make_exe(&root.join("MyGame").join("MyGame.exe"), 10_000);
        make_exe(&root.join("MyGame").join("Redist").join("vc_redist.exe"), 5_000);

        let games = scan_dirs(
            &[root.to_string_lossy().into_owned()],
            &["Redist".to_string()],
        );

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "MyGame");
    }

    #[test]
    fn empty_roots_returns_empty() {
        assert!(scan_dirs(&[], &[]).is_empty());
    }
}
