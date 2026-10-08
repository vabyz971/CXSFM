//! Spanish strings (missing keys fall back to English).

/// Translate `key` to Spanish.
pub fn strings(key: &str) -> &'static str {
    match key {
        "help" => "AYUDA",
        "back" => "ATRÁS",
        "footer_locked" => "LOS CONTROLES ESTÁN BLOQUEADOS MIENTRAS EL MENÚ ESTÁ ABIERTO",
        "mod_fpv" => "Cámara FPV",
        "tile_settings" => "Ajustes",
        "tile_about" => "Acerca de",
        "parked" => "en pausa",
        "settings_language" => "Idioma",
        "settings_auto" => "Auto (juego)",
        "settings_detected" => "Detectado",
        "settings_keys" => "Teclas",
        "keys_f8" => "F8 — menú + capturar ratón",
        "keys_note" => "Los mods siguen activos al cerrar el menú.",
        _ => super::en::strings(key),
    }
}
