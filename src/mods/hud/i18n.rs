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

/// Value-source mode lines.
pub fn mode_live(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Mode : live (valeurs lues dans le jeu)",
        _ => "Mode: live (values read from the game)",
    }
}
pub fn mode_demo(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Mode : démo (valeurs factices)",
        _ => "Mode: demo (synthetic values)",
    }
}

/// Live root state. Placeholder: `{root}`.
pub fn root_found(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{root} : trouvé",
        _ => "{root}: found",
    }
}
pub fn root_missing(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{root} : introuvable (hors course ? recherche toutes les 2 s)",
        _ => "{root}: not found (not driving? retried every 2 s)",
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

/// Tabs.
pub fn tab_hud(_lang_code: &str) -> &'static str {
    "HUD"
}
pub fn tab_explore(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Explorateur",
        _ => "Explorer",
    }
}

/// Explorer search box.
pub fn search_hint(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "nom de classe (ex. Tach, Rpm, Engine)",
        _ => "class name (e.g. Tach, Rpm, Engine)",
    }
}
pub fn search_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Chercher",
        _ => "Search",
    }
}

/// Explorer usage line.
pub fn explore_hint(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Lecture seule. Clique une classe pour lire ses champs, → pour suivre une référence, Relire après avoir roulé. Survole un champ pour son chemin de liaison.",
        _ => "Read-only. Click a class to read its fields, → to follow a reference, Reread after driving. Hover a field for its binding path.",
    }
}

/// Search results line. Placeholders: `{n} {q}`.
pub fn results_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "{n} classes pour « {q} »",
        _ => "{n} classes for \"{q}\"",
    }
}

pub fn back_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "← Résultats",
        _ => "← Results",
    }
}
pub fn reread_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Relire",
        _ => "Reread",
    }
}
pub fn follow_hint(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Suivre cette référence",
        _ => "Follow this reference",
    }
}
