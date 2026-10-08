//! German strings (missing keys fall back to English).

/// Translate `key` to German.
pub fn strings(key: &str) -> &'static str {
    match key {
        "help" => "HILFE",
        "back" => "ZURÜCK",
        "footer_locked" => "SPIELSTEUERUNG IST GESPERRT, SOLANGE DAS MENÜ OFFEN IST",
        "mod_fpv" => "FPV-Kamera",
        "tile_settings" => "Einstellungen",
        "tile_about" => "Über",
        "parked" => "pausiert",
        "settings_language" => "Sprache",
        "settings_auto" => "Auto (Spiel)",
        "settings_detected" => "Erkannt",
        "settings_keys" => "Tasten",
        "keys_f8" => "F8 — Menü + Maus erfassen",
        "keys_note" => "Mods bleiben aktiv, wenn das Menü schließt.",
        _ => super::en::strings(key),
    }
}
