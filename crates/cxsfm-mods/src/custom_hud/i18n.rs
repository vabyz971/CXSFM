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
        "fr" => "HUD personnalisés définis dans custom_hud.toml (rechargé à chaud), façon GT minimaliste.",
        "de" => "Eigene HUDs aus custom_hud.toml (Live-Reload), minimalistischer GT-Stil.",
        "es" => "HUD personalizados definidos en custom_hud.toml (recarga en caliente), estilo GT minimalista.",
        "it" => "HUD personalizzati definiti in custom_hud.toml (ricarica a caldo), stile GT minimalista.",
        "pt" => "HUD personalizados definidos em custom_hud.toml (recarga a quente), estilo GT minimalista.",
        "ru" => "Свои HUD из custom_hud.toml (горячая перезагрузка), минималистичный стиль GT.",
        _ => "Custom HUDs defined in custom_hud.toml (hot-reloaded), minimalist GT style.",
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
        "fr" => "{root} : introuvable (hors course ? nouvelle recherche toutes les 5 s)",
        _ => "{root}: not found (not driving? retried every 5 s)",
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
        "fr" => "{tag}: custom_hud.toml invalide ({e}), config précédente conservée",
        _ => "{tag}: invalid custom_hud.toml ({e}), previous config kept",
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

/// Layout editor strings (`key` → French / English; other languages
/// fall back to English). Unknown keys echo back.
pub fn ed(lang_code: &str, key: &str) -> &'static str {
    let (fr, en) = match key {
        "tab_edit" => ("Éditeur", "Editor"),
        "widget_name" => ("Nom", "Name"),
        "widget_name_hint" => ("ex. Compte-tours", "e.g. Rev counter"),
        "reads" => ("Lit :", "Reads:"),
        "filter_widgets" => ("chercher (nom, type, variable)", "search (name, type, variable)"),
        "bound_only" => ("liés à une variable", "bound to a variable"),
        "no_match" => ("Aucun élément.", "No element."),
        "none" => ("aucune", "none"),
        "number" => ("nombre", "number"),
        "insert_var" => ("+ variable", "+ variable"),
        "name" => ("Nom", "Name"),
        "enabled" => ("Actif", "Enabled"),
        "anchor" => ("Ancrage", "Anchor"),
        "offset" => ("Décalage", "Offset"),
        "size" => ("Taille", "Size"),
        "vector" => ("Vecteur", "Vector"),
        "scale" => ("Échelle", "Scale"),
        "opacity" => ("Opacité", "Opacity"),
        "background" => ("Fond", "Background"),
        "hide_missing" => ("Cacher hors course", "Hide when no value"),
        "pos" => ("Position", "Position"),
        "value" => ("Variable", "Variable"),
        "min" => ("Min", "Min"),
        "max" => ("Max", "Max"),
        "redline" => ("Zone rouge", "Red zone"),
        "segments" => ("Segments", "Segments"),
        "fill" => ("Remplissage", "Fill"),
        "rotation" => ("Rotation", "Rotation"),
        "color" => ("Couleur", "Colour"),
        "track" => ("Piste", "Track"),
        "redline_color" => ("Couleur zone rouge", "Red zone colour"),
        "radius" => ("Rayon", "Radius"),
        "thickness" => ("Épaisseur", "Thickness"),
        "start" => ("Début (°)", "Start (°)"),
        "end" => ("Fin (°)", "End (°)"),
        "filled" => ("Plein", "Filled"),
        "length" => ("Longueur", "Length"),
        "inner" => ("Départ", "Inner"),
        "tail" => ("Queue", "Tail"),
        "width" => ("Largeur", "Width"),
        "count" => ("Graduations", "Count"),
        "minor" => ("Sous-graduations", "Minor"),
        "major_length" => ("Long. majeure", "Major length"),
        "minor_length" => ("Long. mineure", "Minor length"),
        "labels" => ("Chiffres", "Labels"),
        "label_min" => ("Chiffre min", "Label min"),
        "label_max" => ("Chiffre max", "Label max"),
        "label_size" => ("Taille chiffres", "Label size"),
        "label_color" => ("Couleur chiffres", "Label colour"),
        "minor_color" => ("Couleur mineure", "Minor colour"),
        "text" => ("Texte", "Text"),
        "font_size" => ("Taille police", "Font size"),
        "align" => ("Alignement", "Alignment"),
        "skew" => ("Italique", "Skew"),
        "glow" => ("Halo", "Glow"),
        "font" => ("Police", "Font"),
        "max_width" => ("Largeur max", "Max width"),
        "justify_hint" => ("Justifié : donne une largeur max (le texte passe à la ligne).", "Justify needs a max width (the text wraps)."),
        "bold" => ("Gras", "Bold"),
        "letter_spacing" => ("Espacement", "Letter spacing"),
        "outline" => ("Contour", "Outline"),
        "outline_color" => ("Couleur contour", "Outline colour"),
        "corner" => ("Coins", "Corners"),
        "fill_color" => ("Remplissage", "Fill"),
        "stroke" => ("Contour", "Stroke"),
        "stroke_color" => ("Couleur trait", "Stroke colour"),
        "alert" => ("Alerte", "Alert"),
        "alert_color" => ("Couleur alerte", "Alert colour"),
        "no_layout" => ("Aucune mise en page chargée (voir l'onglet HUD).", "No layout loaded (see the HUD tab)."),
        "no_hud" => ("Le fichier n'a aucun [[hud]].", "The file has no [[hud]]."),
        "save" => ("Enregistrer", "Save"),
        "undo" => ("Annuler (Ctrl+Z)", "Undo (Ctrl+Z)"),
        "revert" => ("Abandonner les changements", "Discard changes"),
        "add" => ("Ajouter", "Add"),
        "add_rect" => ("Rectangle", "Rectangle"),
        "add_ellipse" => ("Cercle / ellipse", "Circle / ellipse"),
        "add_line" => ("Ligne", "Line"),
        "add_text" => ("Texte / nombre", "Text / number"),
        "add_arc" => ("Cercle de jauge (arc)", "Gauge circle (arc)"),
        "add_bar" => ("Barre", "Bar"),
        "al_left" => ("Aligner à gauche", "Align left"),
        "al_hcenter" => ("Centrer horizontalement", "Centre horizontally"),
        "al_right" => ("Aligner à droite", "Align right"),
        "al_top" => ("Aligner en haut", "Align top"),
        "al_vcenter" => ("Centrer verticalement", "Centre vertically"),
        "al_bottom" => ("Aligner en bas", "Align bottom"),
        "dist_h" => ("Répartir horizontalement (3+)", "Distribute horizontally (3+)"),
        "dist_v" => ("Répartir verticalement (3+)", "Distribute vertically (3+)"),
        "duplicate" => ("Dupliquer (Ctrl+D)", "Duplicate (Ctrl+D)"),
        "delete" => ("Supprimer (Suppr)", "Delete (Del)"),
        "magnet" => ("Magnétique", "Magnetic"),
        "grid" => ("Grille", "Grid"),
        "help" => (
            "Clique un widget à l'écran (Maj = plusieurs), glisse pour déplacer ; glisse le fond d'un HUD pour le déplacer. Flèches = 1 px (Maj ×10). Un seul widget s'aligne sur la boîte du HUD.",
            "Click a widget on screen (Shift = several), drag to move; drag a HUD's background to move it. Arrows = 1 px (Shift ×10). A single widget aligns on the HUD box.",
        ),
        "select" => ("Sélectionne un widget.", "Select a widget."),
        "multi" => ("{n} widgets sélectionnés : aligne / répartis-les avec les boutons.", "{n} widgets selected: align / distribute them with the buttons."),
        "use_var" => ("Utiliser comme variable", "Use as variable"),
        "var_name" => ("Nom de la variable", "Variable name"),
        "var_add" => ("Ajouter", "Add"),
        "var_bad" => ("Nom invalide ou déjà pris (lettres, chiffres, _).", "Invalid or taken name (letters, digits, _)."),
        "saved" => ("Enregistré : {n} changement(s) écrit(s) dans le fichier.", "Saved: {n} change(s) written to the file."),
        "saved_skipped" => ("Enregistré, {w} changement(s) ignoré(s) (fichier modifié entre-temps ?).", "Saved, {w} change(s) skipped (file changed meanwhile?)."),
        "save_error" => ("Échec de l'enregistrement : {e}", "Save failed: {e}"),
        "var_added" => ("Variable `{name}` ajoutée à [sources].", "Variable `{name}` added to [sources]."),
        _ => return "?",
    };
    if lang_code == "fr" { fr } else { en }
}
