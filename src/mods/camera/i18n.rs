//! camera strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Caméra",
        "de" => "Kamera",
        "es" => "Cámara",
        "it" => "Telecamera",
        "pt" => "Câmara",
        "ru" => "Камера",
        _ => "Camera",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Cinemachine : distance, hauteur, suivi et presets (restaure).",
        "de" => "Cinemachine: Abstand, Höhe, Follow und Presets (stellt wieder her).",
        "es" => "Cinemachine: distancia, altura, seguimiento y presets (restaura).",
        "it" => "Cinemachine: distanza, altezza, follow e preset (ripristina).",
        "pt" => "Cinemachine: distância, altura, follow e presets (repõe).",
        "ru" => "Cinemachine: дистанция, высота, follow и пресеты (восстанавливает).",
        _ => "Cinemachine: distance, height, follow and presets (restores).",
    }
}
