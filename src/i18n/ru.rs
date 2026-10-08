//! Russian strings (missing keys fall back to English).

/// Translate `key` to Russian.
pub fn strings(key: &str) -> &'static str {
    match key {
        "help" => "ПОМОЩЬ",
        "back" => "НАЗАД",
        "footer_locked" => "УПРАВЛЕНИЕ ЗАБЛОКИРОВАНО, ПОКА ОТКРЫТО МЕНЮ",
        "mod_fpv" => "FPV-камера",
        "tile_settings" => "Настройки",
        "tile_about" => "О проекте",
        "parked" => "пауза",
        "settings_language" => "Язык",
        "settings_auto" => "Авто (игра)",
        "settings_detected" => "Определён",
        "settings_keys" => "Клавиши",
        "keys_f8" => "F8 — меню + захват мыши",
        "keys_note" => "Моды остаются включены при закрытии меню.",
        _ => super::en::strings(key),
    }
}
