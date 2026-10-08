//! Portuguese strings (missing keys fall back to English).

/// Translate `key` to Portuguese.
pub fn strings(key: &str) -> &'static str {
    match key {
        "help" => "AJUDA",
        "back" => "VOLTAR",
        "footer_locked" => "OS CONTROLOS ESTÃO BLOQUEADOS ENQUANTO O MENU ESTIVER ABERTO",
        "mod_fpv" => "Câmara FPV",
        "tile_settings" => "Configurações",
        "tile_about" => "Sobre",
        "parked" => "pausado",
        "settings_language" => "Idioma",
        "settings_auto" => "Auto (jogo)",
        "settings_detected" => "Detetado",
        "settings_keys" => "Teclas",
        "keys_f8" => "F8 — menu + capturar rato",
        "keys_note" => "Os mods continuam ativos ao fechar o menu.",
        _ => super::en::strings(key),
    }
}
