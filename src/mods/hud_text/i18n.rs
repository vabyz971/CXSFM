//! hud_text strings, owned by the mod.
//!
//! The menu only knows the stable key (`mod_version`); every letter on
//! screen comes from here. New language = one arm per function.

/// Tile label for `lang_code` (`en`, `fr`, … — unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Tag version",
        "de" => "Versions-Tag",
        "es" => "Etiqueta versión",
        "it" => "Tag versione",
        "pt" => "Tag de versão",
        "ru" => "Тег версии",
        _ => "Version Tag",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Remplace un texte HUD (restaure à la désactivation).",
        "de" => "Ersetzt einen HUD-Text (stellt beim Deaktivieren wieder her).",
        "es" => "Reemplaza un texto del HUD (restaura al desactivar).",
        "it" => "Sostituisce un testo HUD (ripristina alla disattivazione).",
        "pt" => "Substitui um texto do HUD (repõe ao desativar).",
        "ru" => "Заменяет текст HUD (восстанавливает при отключении).",
        _ => "Replaces one HUD text (restores on disable).",
    }
}
