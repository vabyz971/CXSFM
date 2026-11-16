//! fpv_drone strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Drone FPV",
        "de" => "FPV-Drohne",
        "es" => "Dron FPV",
        "it" => "Drone FPV",
        "pt" => "Drone FPV",
        "ru" => "FPV-дрон",
        _ => "FPV Drone",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Caméra drone FPV libre à la manette (réversible).",
        "de" => "Freie FPV-Drohnenkamera per Gamepad (umkehrbar).",
        "es" => "Cámara dron FPV libre con mando (reversible).",
        "it" => "Camera drone FPV libera col gamepad (reversibile).",
        "pt" => "Câmara drone FPV livre com comando (reversível).",
        "ru" => "Свободная FPV-камера дрона на геймпаде (обратимо).",
        _ => "Free FPV drone camera on the gamepad (reversible).",
    }
}
