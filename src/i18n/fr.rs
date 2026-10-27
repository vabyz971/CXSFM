//! French strings (missing keys fall back to English).

/// Translate `key` to French.
pub fn strings(key: &str) -> &'static str {
    match key {
        "help" => "AIDE",
        "back" => "RETOUR",
        "footer_locked" => "LES CONTRÔLES DU JEU SONT BLOQUÉS QUAND LE MENU EST OUVERT",
        "mod_fpv" => "Caméra FPV",
        "tile_settings" => "Paramètres",
        "tile_about" => "À propos",
        "tile_quit" => "Quitter",
        "parked" => "en pause",
        "settings_language" => "Langue",
        "settings_auto" => "Auto (jeu)",
        "settings_detected" => "Détectée",
        "settings_keys" => "Touches",
        "settings_autoload" => "Charger les saves à l'arrivée",
        "settings_autoload_hint" => "Les valeurs Speedo/Map s'appliquent quelques secondes après l'arrivée en jeu.",
        "keys_f8" => "F8 — menu + capture souris",
        "keys_note" => "Les mods restent actifs quand le menu se ferme.",
        _ => super::en::strings(key),
    }
}
