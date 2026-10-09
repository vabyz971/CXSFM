//! speedo strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Compteur",
        "de" => "Tacho",
        "es" => "Velocímetro",
        "it" => "Tachimetro",
        "pt" => "Velocímetro",
        "ru" => "Спидометр",
        _ => "Speedo",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Remplace le compteur du jeu par le nôtre (restaure).",
        "de" => "Ersetzt den Spieltacho durch unseren (stellt wieder her).",
        "es" => "Reemplaza el velocímetro del juego por el nuestro (restaura).",
        "it" => "Sostituisce il tachimetro del gioco con il nostro (ripristina).",
        "pt" => "Substitui o velocímetro do jogo pelo nosso (repõe).",
        "ru" => "Заменяет спидометр игры нашим (восстанавливает).",
        _ => "Replaces the game speedometer with ours (restores).",
    }
}
