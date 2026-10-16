//! map strings, owned by the mod (same keys as speedo).

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Carte",
        "de" => "Karte",
        "es" => "Mapa",
        "it" => "Mappa",
        "pt" => "Mapa",
        "ru" => "Карта",
        _ => "Map",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Inspecte et restyle la minimap du jeu, façon Inspector (one-shot, restaure).",
        "de" => "Inspiziert und stylt die Minikarte, Inspector-Stil (One-Shot, stellt wieder her).",
        "es" => "Inspecciona y personaliza el minimapa, estilo Inspector (único, restaura).",
        "it" => "Ispeziona e restyle la minimappa, stile Inspector (one-shot, ripristina).",
        "pt" => "Inspeciona e personaliza o minimapa, estilo Inspector (único, repõe).",
        "ru" => "Показывает и меняет миникарту в стиле Inspector (разово, восстанавливает).",
        _ => "Inspects and restyles the game minimap, Inspector-style (one-shot, restores).",
    }
}

/// Refresh (re-discover objects) button.
pub fn refresh_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Actualiser",
        "de" => "Aktualisieren",
        "es" => "Actualizar",
        "it" => "Aggiorna",
        "pt" => "Atualizar",
        "ru" => "Обновить",
        _ => "Refresh",
    }
}

/// Restore-originals button.
pub fn restore_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Restaurer",
        "de" => "Wiederherst.",
        "es" => "Restaurar",
        "it" => "Ripristina",
        "pt" => "Repôr",
        "ru" => "Вернуть",
        _ => "Restore",
    }
}

/// Save-to-file button (TOML, only modified params).
pub fn save_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Sauvegarder",
        _ => "Save",
    }
}

/// Load-from-file button (staged as one batched pass + re-discover).
pub fn load_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Charger",
        _ => "Load",
    }
}

/// Object counter prefix ("objets 5/6").
pub fn objects_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "objets",
        "de" => "Objekte",
        "es" => "objetos",
        "it" => "oggetti",
        "pt" => "objetos",
        "ru" => "объекты",
        _ => "objects",
    }
}

/// Empty-list hint (never discovered yet).
pub fn hint_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "En attente de découverte… (Refresh après changement de scène)",
        "de" => "Warte auf Erkennung… (Aktualisieren nach Szenenwechsel)",
        "es" => "Esperando detección… (Actualizar tras cambio de escena)",
        "it" => "In attesa di rilevamento… (Aggiorna dopo il cambio scena)",
        "pt" => "A aguardar deteção… (Atualizar após mudança de cena)",
        "ru" => "Ожидание поиска… (Обновить после смены сцены)",
        _ => "Waiting for discovery… (Refresh after scene change)",
    }
}

/// No-selection hint (right pane).
pub fn select_hint(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Sélectionne un objet dans la liste.",
        _ => "Select an object from the list.",
    }
}

/// TMP color caveat (text material ignores vertex color).
pub fn color_hint(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "La couleur peut être ignorée (matériau du texte).",
        _ => "Color may be ignored (text material).",
    }
}
