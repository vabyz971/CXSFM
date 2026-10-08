//! hud_scout strings, owned by the mod.
//!
//! The menu only knows the stable key (`mod_scout`); every letter on
//! screen comes from here. New language = one arm per function.

/// Tile label for `lang_code` (`en`, `fr`, … — unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Inspecteur HUD",
        "de" => "HUD-Inspektor",
        "es" => "Inspector HUD",
        "it" => "Ispettore HUD",
        "pt" => "Inspetor HUD",
        "ru" => "Инспектор HUD",
        _ => "HUD Scout",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Inventaire read-only des textes HUD (log uniquement).",
        "de" => "Schreibgeschütztes HUD-Textinventar (nur Log).",
        "es" => "Inventario de textos del HUD (solo registro).",
        "it" => "Inventario dei testi HUD (solo log).",
        "pt" => "Inventário dos textos do HUD (só registo).",
        "ru" => "Инвентаризация текстов HUD (только лог).",
        _ => "Read-only HUD text inventory (log only).",
    }
}
