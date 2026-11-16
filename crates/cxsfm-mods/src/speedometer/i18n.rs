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

/// Re-activating an inactive object can crash the game (SetActive
/// from the tick thread fires the game's lifecycle callbacks).
pub fn active_warning(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "⚠ Réactiver un élément désactivé peut faire planter le jeu.",
        _ => "⚠ Re-activating a disabled element can crash the game.",
    }
}

/// TMP color caveat (text material ignores vertex color).
pub fn color_hint(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "La couleur peut être ignorée (matériau du texte).",
        _ => "Color may be ignored (text material).",
    }
}

/// Tick-side log templates. `{...}` placeholders are filled by the
/// caller — every wording (both languages) lives here, never inline.
pub fn log_root_missing(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: ni UIHUDCarDashboard ni sections trouvées (Refresh hors course ?)",
        _ => "{tag}: neither UIHUDCarDashboard nor sections found (Refresh outside a race?)",
    }
}

/// Fallback roots bound instead of the dashboard. Placeholder: `{names}`.
pub fn log_fallback_roots(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: racines fallback [{names}]",
        _ => "{tag}: fallback roots [{names}]",
    }
}

/// Save confirmation. Placeholders: `{tag} {n} {scope} {path}`.
pub fn log_saved_ok(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: sauvegardé {n} objets ({scope}) → {path}",
        _ => "{tag}: saved {n} objects ({scope}) → {path}",
    }
}

/// `{scope}` fragment when the file is new.
pub fn log_scope_fresh(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "nouveau fichier",
        _ => "new file",
    }
}

/// `{scope}` fragment naming the preserved mods. Placeholder: `{kept}`.
pub fn log_scope_kept(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{kept} autres mods conservés",
        _ => "{kept} other mods kept",
    }
}

/// Save failure. Placeholders: `{tag} {e}`.
pub fn log_save_err(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: sauvegarde impossible ({e})",
        _ => "{tag}: save failed ({e})",
    }
}

/// No home directory. Placeholder: `{tag}`.
pub fn log_save_no_home(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: sauvegarde impossible (HOME introuvable)",
        _ => "{tag}: cannot save (HOME missing)",
    }
}

/// No home directory on load. Placeholder: `{tag}`.
pub fn log_load_no_home(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: chargement impossible (HOME introuvable)",
        _ => "{tag}: cannot load (HOME missing)",
    }
}

/// Load failure. Placeholders: `{tag} {e}`.
pub fn log_load_err(lang_code: &str) -> &'static str {    match lang_code {
        "fr" => "{tag}: chargement impossible ({e})",
        _ => "{tag}: load failed ({e})",
    }
}

/// Nothing to apply. Placeholders: `{tag} {unknown} {skipped}`.
pub fn log_load_empty(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: chargement — rien à appliquer ({unknown} inconnus, {skipped} ignorés)",
        _ => "{tag}: load — nothing to apply ({unknown} unknown, {skipped} skipped)",
    }
}

/// Load summary. Placeholders: `{tag} {staged} {unknown} {skipped}`.
pub fn log_load_ok(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: chargement {staged} objets ({unknown} inconnus, {skipped} ignorés)",
        _ => "{tag}: loaded {staged} objects ({unknown} unknown, {skipped} skipped)",
    }
}

/// Vertex-color write swallowed by the material. Placeholders: `{tag} {owner}`.
pub fn log_color_ignored(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: couleur ignorée sur {owner} (matériau custom ?)",
        _ => "{tag}: color ignored on {owner} (custom material?)",
    }
}

/// Opacity landed on an ancestor group. Placeholders: `{tag} {owner}`.
pub fn log_ancestor_served(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: opacité via groupe ancêtre sur {owner}",
        _ => "{tag}: opacity via ancestor group on {owner}",
    }
}

/// Alpha write did not stick. Placeholders: `{tag} {owner}`.
pub fn log_alpha_ignored(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: opacité non appliquée sur {owner} (jeu prioritaire ?)",
        _ => "{tag}: opacity not applied on {owner} (game overriding?)",
    }
}

/// Params-card labels (same shape for every row).
pub fn card_gameobject(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "GameObject",
        _ => "GameObject",
    }
}
pub fn card_name(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Nom",
        _ => "Name",
    }
}
pub fn card_path(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Chemin",
        _ => "Path",
    }
}
pub fn card_active(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Actif",
        _ => "Active",
    }
}
pub fn card_visible(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Visible",
        _ => "Visible",
    }
}
pub fn card_transform(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Transform",
        _ => "Transform",
    }
}
pub fn axis_position(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Position",
        _ => "Position",
    }
}
pub fn axis_rotation(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Rotation",
        _ => "Rotation",
    }
}
pub fn axis_scale(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Échelle",
        _ => "Scale",
    }
}
pub fn card_color(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Couleur",
        _ => "Color",
    }
}
pub fn card_opacity(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Opacité",
        _ => "Opacity",
    }
}
pub fn card_text(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Texte",
        _ => "Text",
    }
}
pub fn card_size(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Taille",
        _ => "Size",
    }
}
