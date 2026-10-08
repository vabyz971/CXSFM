//! English strings: the canonical key set and the fallback target.
//!
//! Every other language file falls back here per key (`_ => super::en`).

/// Translate `key` to English (`"[?]"` for unknown keys — never blank).
pub fn strings(key: &str) -> &'static str {
    match key {
        "help" => "HELP",
        "back" => "BACK",
        "footer_locked" => "GAME CONTROLS ARE LOCKED WHILE THE MOD MENU IS UP",
        "mod_fpv" => "FPV Camera",
        "tile_settings" => "Settings",
        "tile_about" => "About",
        "parked" => "parked",
        "settings_language" => "Language",
        "settings_auto" => "Auto (game)",
        "settings_detected" => "Detected",
        "settings_keys" => "Keys",
        "keys_f8" => "F8 — menu + mouse capture",
        "keys_note" => "Mods stay on when the menu closes.",
        _ => "[?]",
    }
}
