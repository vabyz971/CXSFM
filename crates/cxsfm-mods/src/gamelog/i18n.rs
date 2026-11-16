//! gamelog strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Log jeu",
        "de" => "Spiellog",
        "es" => "Registro",
        "it" => "Registro",
        "pt" => "Registo",
        "ru" => "Лог игры",
        _ => "Game Log",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Player.log du jeu : erreurs et contexte (lecture seule).",
        "de" => "Player.log des Spiels: Fehler und Kontext (nur Lesen).",
        "es" => "Player.log del juego: errores y contexto (lectura).",
        "it" => "Player.log del gioco: errori e contesto (lettura).",
        "pt" => "Player.log do jogo: erros e contexto (leitura).",
        "ru" => "Player.log игры: ошибки и контекст (чтение).",
        _ => "Game Player.log: errors and context (read-only).",
    }
}
