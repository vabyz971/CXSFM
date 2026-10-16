//! hud strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "HUD perso",
        "de" => "Eigenes HUD",
        "es" => "HUD propio",
        "it" => "HUD personale",
        "pt" => "HUD próprio",
        "ru" => "Свой HUD",
        _ => "Custom HUD",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "HUD personnalisés définis dans hud.toml (rechargé à chaud), façon GT minimaliste.",
        "de" => "Eigene HUDs aus hud.toml (Live-Reload), minimalistischer GT-Stil.",
        "es" => "HUD personalizados definidos en hud.toml (recarga en caliente), estilo GT minimalista.",
        "it" => "HUD personalizzati definiti in hud.toml (ricarica a caldo), stile GT minimalista.",
        "pt" => "HUD personalizados definidos em hud.toml (recarga a quente), estilo GT minimalista.",
        "ru" => "Свои HUD из hud.toml (горячая перезагрузка), минималистичный стиль GT.",
        _ => "Custom HUDs defined in hud.toml (hot-reloaded), minimalist GT style.",
    }
}

/// Reload button.
pub fn reload_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Recharger",
        _ => "Reload",
    }
}

/// Config file path row label.
pub fn file_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Fichier",
        _ => "File",
    }
}

/// HUD count row. Placeholders: `{on} {total}`.
pub fn huds_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "HUD actifs : {on}/{total}",
        _ => "Active HUDs: {on}/{total}",
    }
}

/// Phase-1 value source notice.
pub fn demo_notice(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Valeurs de démo (lecture du jeu : phase suivante).",
        _ => "Demo values (game reading: next phase).",
    }
}

/// No layout loaded yet.
pub fn no_config(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Aucune config chargée.",
        _ => "No config loaded.",
    }
}

/// Parse error header (last good layout stays on screen).
pub fn error_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Erreur (dernière config valide conservée) :",
        _ => "Error (last valid config kept):",
    }
}

/// Warnings header.
pub fn warnings_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Avertissements :",
        _ => "Warnings:",
    }
}

/// Tick-side log templates (`{tag}` `{path}` `{n}` `{w}` `{e}`).
pub fn log_example_written(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: config d'exemple créée : {path}",
        _ => "{tag}: example config written: {path}",
    }
}
pub fn log_loaded(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: {n} HUD chargés ({w} avertissements)",
        _ => "{tag}: {n} HUDs loaded ({w} warnings)",
    }
}
pub fn log_error(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: hud.toml invalide ({e}), config précédente conservée",
        _ => "{tag}: invalid hud.toml ({e}), previous config kept",
    }
}
pub fn log_no_home(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{tag}: aucun dossier de config (HOME/XDG_CONFIG_HOME absents)",
        _ => "{tag}: no config directory (HOME/XDG_CONFIG_HOME missing)",
    }
}
