//! map strings, owned by the mod. The editor window's labels (cards,
//! buttons) are shared: they live in the speedometer's i18n.

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

/// Tick-side log templates. `{...}` placeholders are filled by the
/// caller — every wording (both languages) lives here, never inline.
pub fn log_root_missing(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: ni UIHUDMinimap ni Map trouvés (Refresh hors course ?)",
        _ => "{tag}: neither UIHUDMinimap nor Map found (Refresh outside a race?)",
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
pub fn log_load_err(lang_code: &str) -> &'static str {
    match lang_code {
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

