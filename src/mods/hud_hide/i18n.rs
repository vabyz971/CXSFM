//! hud_hide strings, owned by the mod.
//!
//! The menu only knows the stable key (`mod_hide`); every letter on
//! screen comes from here. New language = one arm per function.

/// Tile label for `lang_code` (`en`, `fr`, … — unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Masquer HUD",
        "de" => "HUD ausblenden",
        "es" => "Ocultar HUD",
        "it" => "Nascondi HUD",
        "pt" => "Ocultar HUD",
        "ru" => "Скрыть HUD",
        _ => "HUD Hide",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Masque un élément HUD (restaure à la désactivation).",
        "de" => "Blendet ein HUD-Element aus (stellt beim Deaktivieren wieder her).",
        "es" => "Oculta un elemento del HUD (restaura al desactivar).",
        "it" => "Nasconde un elemento HUD (ripristina alla disattivazione).",
        "pt" => "Oculta um elemento do HUD (repõe ao desativar).",
        "ru" => "Скрывает элемент HUD (восстанавливает при отключении).",
        _ => "Hides one HUD element (restores on disable).",
    }
}
