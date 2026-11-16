//! engine_audio strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Son moteur",
        "de" => "Motorsound",
        "es" => "Sonido motor",
        "it" => "Suono motore",
        "pt" => "Som do motor",
        "ru" => "Звук мотора",
        _ => "Engine Sound",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Pitch et volume des sons moteur via FMOD (réversible).",
        "de" => "Pitch und Lautstärke der Motorsounds über FMOD (umkehrbar).",
        "es" => "Tono y volumen del sonido del motor vía FMOD (reversible).",
        "it" => "Pitch e volume del suono motore via FMOD (reversibile).",
        "pt" => "Pitch e volume do som do motor via FMOD (reversível).",
        "ru" => "Высота и громкость звука мотора через FMOD (обратимо).",
        _ => "Engine sound pitch and volume through FMOD (reversible).",
    }
}
