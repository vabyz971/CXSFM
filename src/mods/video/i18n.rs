//! video strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Vidéo",
        "de" => "Video",
        "es" => "Vídeo",
        "it" => "Video",
        "pt" => "Vídeo",
        "ru" => "Видео",
        _ => "Video",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Qualité, résolution, brouillard et lumière (restaure).",
        "de" => "Qualität, Auflösung, Nebel und Licht (stellt wieder her).",
        "es" => "Calidad, resolución, niebla y luz (restaura).",
        "it" => "Qualità, risoluzione, nebbia e luce (ripristina).",
        "pt" => "Qualidade, resolução, nevoeiro e luz (repõe).",
        "ru" => "Качество, разрешение, туман и свет (восстанавливает).",
        _ => "Quality, resolution, fog and light (restores).",
    }
}
