//! Italian strings (missing keys fall back to English).

/// Translate `key` to Italian.
pub fn strings(key: &str) -> &'static str {
    match key {
        "help" => "AIUTO",
        "back" => "INDIETRO",
        "footer_locked" => "I CONTROLLI SONO BLOCCATI MENTRE IL MENU È APERTO",
        "mod_fpv" => "Fotocamera FPV",
        "tile_settings" => "Impostazioni",
        "tile_about" => "Info",
        "parked" => "in pausa",
        "settings_language" => "Lingua",
        "settings_auto" => "Auto (gioco)",
        "settings_detected" => "Rilevata",
        "settings_keys" => "Tasti",
        "keys_f8" => "F8 — menu + cattura mouse",
        "keys_note" => "Le mod restano attive alla chiusura del menu.",
        _ => super::en::strings(key),
    }
}
