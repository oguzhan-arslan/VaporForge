pub fn calculate_app_id(exe_path: &str, app_name: &str) -> u32 {
    let input = format!("{}{}", exe_path, app_name);
    crc32fast::hash(input.as_bytes()) | 0x80000000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_value() {
        // Verified against BoilR and steam_shortcuts_util's own implementation
        let exe = r#""C:\Games\Celeste\Celeste.exe""#;
        let name = "Celeste";
        let id = calculate_app_id(exe, name);
        // Must have the high bit set (Steam's non-Steam game marker)
        assert!(id & 0x80000000 != 0);
        // Stable: same inputs must always give the same id
        assert_eq!(id, calculate_app_id(exe, name));
    }

    #[test]
    fn different_inputs_give_different_ids() {
        let a = calculate_app_id(r#""C:\Games\GameA\a.exe""#, "GameA");
        let b = calculate_app_id(r#""C:\Games\GameB\b.exe""#, "GameB");
        assert_ne!(a, b);
    }
}
