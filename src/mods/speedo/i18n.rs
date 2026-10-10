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
        "fr" => "Inspecte et restyle le compteur du jeu, façon Inspector (one-shot, restaure).",
        "de" => "Inspiziert und stylt den Spieltacho, Inspector-Stil (One-Shot, stellt wieder her).",
        "es" => "Inspecciona y personaliza el velocímetro del juego, estilo Inspector (único, restaura).",
        "it" => "Ispeziona e restyle il tachimetro del gioco, stile Inspector (one-shot, ripristina).",
        "pt" => "Inspeciona e personaliza o velocímetro do jogo, estilo Inspector (único, repõe).",
        "ru" => "Показывает и меняет спидометр игры в стиле Inspector (разово, восстанавливает).",
        _ => "Inspects and restyles the game speedometer, Inspector-style (one-shot, restores).",
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

/// Apply-staged-edits button (single Unity pass).
pub fn apply_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Appliquer",
        "de" => "Anwenden",
        "es" => "Aplicar",
        "it" => "Applica",
        "pt" => "Aplicar",
        "ru" => "Применить",
        _ => "Apply",
    }
}

/// Apply button tooltip.
pub fn apply_hint(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Écrit tout d'un coup (1 passe Unity)",
        "de" => "Alles auf einmal schreiben (1 Unity-Durchlauf)",
        "es" => "Escribe todo de una vez (1 pasada Unity)",
        "it" => "Scrive tutto in una volta (1 passata Unity)",
        "pt" => "Escreve tudo de uma vez (1 passagem Unity)",
        "ru" => "Записать всё за раз (1 проход Unity)",
        _ => "Write everything at once (1 Unity pass)",
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
pub fn hint_label(lang_code: &str) -> &'static str {    match lang_code {
        "fr" => "En attente de découverte… (menu, garage, changement de scène : Actualiser)",
        "de" => "Warte auf Erkennung… (Menü, Garage, Szenenwechsel: Aktualisieren)",
        "es" => "Esperando detección… (menú, garaje, cambio de escena: Actualizar)",
        "it" => "In attesa di rilevamento… (menu, garage, cambio scena: Aggiorna)",
        "pt" => "A aguardar deteção… (menu, garagem, mudança de cena: Atualizar)",
        "ru" => "Ожидание поиска… (меню, гараж, смена сцены: Обновить)",
        _ => "Waiting for discovery… (menu, garage, scene change: Refresh)",
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
