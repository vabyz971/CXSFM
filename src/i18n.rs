//! Multilingual UI strings with automatic game-language detection.
//!
//! # Detection chain
//!
//! 1. Unity `Application.systemLanguage` (via the `get_systemLanguage`
//!    icall — the game itself logs `System Language: xx` from it at
//!    startup; no PlayerPrefs key exists for language).
//! 2. Process locale (`LANG` / `LC_ALL` / `LANGUAGE`).
//! 3. English.
//!
//! The Settings tile lets the user pin a language (override); `Auto`
//! follows the chain above. Adding a language = one `Lang` variant +
//! one `match` arm per `*_str` function below (English is the fallback
//! for any missing key, so partial translations never blank the UI).

use std::sync::atomic::{AtomicU8, Ordering};

/// Languages with a full menu translation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Fr,
    De,
    Es,
    It,
    Pt,
    Ru,
}

impl Lang {
    /// All selectable languages, in menu order.
    pub const ALL: &'static [Lang] = &[
        Lang::En,
        Lang::Fr,
        Lang::De,
        Lang::Es,
        Lang::It,
        Lang::Pt,
        Lang::Ru,
    ];

    /// ISO code (`en`, `fr`, …).
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::De => "de",
            Lang::Es => "es",
            Lang::It => "it",
            Lang::Pt => "pt",
            Lang::Ru => "ru",
        }
    }

    /// Native display name for the language picker.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Fr => "Français",
            Lang::De => "Deutsch",
            Lang::Es => "Español",
            Lang::It => "Italiano",
            Lang::Pt => "Português",
            Lang::Ru => "Русский",
        }
    }

    /// Map a Unity `SystemLanguage` int (see `unity::system_language_raw`).
    /// Indices follow the engine enum order (English=10, French=14,
    /// German=15, Italian=20, Portuguese=27, Russian=29, Spanish=33).
    /// Unlisted languages return `None` (caller falls through to locale).
    pub fn from_system_language(v: i32) -> Option<Lang> {
        match v {
            10 => Some(Lang::En),
            14 => Some(Lang::Fr),
            15 => Some(Lang::De),
            20 => Some(Lang::It),
            27 => Some(Lang::Pt),
            29 => Some(Lang::Ru),
            33 => Some(Lang::Es),
            _ => None,
        }
    }

    /// Map a locale-ish string (`fr`, `fr_FR.UTF-8`, …) by prefix.
    pub fn from_code(s: &str) -> Option<Lang> {
        let s = s.to_ascii_lowercase();
        for lang in Self::ALL {
            if s.starts_with(lang.code()) {
                return Some(*lang);
            }
        }
        None
    }

    fn encode(self) -> u8 {
        match self {
            Lang::En => 0,
            Lang::Fr => 1,
            Lang::De => 2,
            Lang::Es => 3,
            Lang::It => 4,
            Lang::Pt => 5,
            Lang::Ru => 6,
        }
    }

    fn decode(v: u8) -> Option<Lang> {
        match v {
            0 => Some(Lang::En),
            1 => Some(Lang::Fr),
            2 => Some(Lang::De),
            3 => Some(Lang::Es),
            4 => Some(Lang::It),
            5 => Some(Lang::Pt),
            6 => Some(Lang::Ru),
            _ => None,
        }
    }
}

/// No language stored yet.
const UNSET: u8 = 0xFF;

/// Raw `systemLanguage` mapped once the IL2CPP bridge is up.
static DETECTED: AtomicU8 = AtomicU8::new(UNSET);
/// Manual pin from the Settings tile (`UNSET` = Auto).
static OVERRIDE: AtomicU8 = AtomicU8::new(UNSET);

/// Record the game's language (call once the bridge resolves it).
/// `None` (icall missing) also sticks — no retry spam every tick.
pub fn note_detected(raw: Option<i32>) {
    let v = raw.and_then(Lang::from_system_language).map(Lang::encode);
    DETECTED.store(v.unwrap_or(UNSET), Ordering::SeqCst);
    crate::log_line(&format!(
        "i18n: game language raw={:?} -> {:?}",
        raw,
        v.and_then(Lang::decode).map(Lang::code)
    ));
}

/// Language the game reported, if any.
pub fn detected() -> Option<Lang> {
    Lang::decode(DETECTED.load(Ordering::SeqCst))
}

/// Pin a language from the Settings tile (`None` = back to Auto).
pub fn set_override(lang: Option<Lang>) {
    OVERRIDE.store(lang.map(Lang::encode).unwrap_or(UNSET), Ordering::SeqCst);
}

/// Current manual pin, if any.
pub fn override_lang() -> Option<Lang> {
    Lang::decode(OVERRIDE.load(Ordering::SeqCst))
}

/// Locale from the process environment (`LANG`, `LC_ALL`, `LANGUAGE`).
fn from_env() -> Option<Lang> {
    for var in ["LC_ALL", "LANG", "LANGUAGE"] {
        if let Ok(val) = std::env::var(var) {
            let first = val.split([':', ';']).next().unwrap_or("");
            if let Some(lang) = Lang::from_code(first.trim()) {
                return Some(lang);
            }
        }
    }
    None
}

/// Effective language: pin → game → locale → English.
pub fn current() -> Lang {
    override_lang()
        .or_else(detected)
        .or_else(from_env)
        .unwrap_or(Lang::En)
}

/// Translate `key` into the effective language.
///
/// Unknown keys echo back (never blank); any language falls back to
/// English per key, so partial translations stay usable.
pub fn t(key: &str) -> &'static str {
    match current() {
        Lang::En => en_str(key),
        Lang::Fr => fr_str(key),
        Lang::De => de_str(key),
        Lang::Es => es_str(key),
        Lang::It => it_str(key),
        Lang::Pt => pt_str(key),
        Lang::Ru => ru_str(key),
    }
}

fn en_str(key: &str) -> &'static str {
    match key {
        "help" => "HELP",
        "back" => "BACK",
        "footer_locked" => "GAME CONTROLS ARE LOCKED WHILE THE MOD MENU IS UP",
        "mod_fpv" => "FPV Camera",
        "mod_scout" => "HUD Scout",
        "mod_hide" => "HUD Hide",
        "mod_version" => "Version Tag",
        "tile_settings" => "Settings",
        "tile_about" => "About",
        "parked" => "parked",
        "settings_language" => "Language",
        "settings_auto" => "Auto (game)",
        "settings_detected" => "Detected",
        "settings_keys" => "Keys",
        "keys_f8" => "F8 — menu + mouse capture",
        "keys_note" => "Mods stay on when the menu closes.",
        _ => "[?]",
    }
}

fn fr_str(key: &str) -> &'static str {
    match key {
        "help" => "AIDE",
        "back" => "RETOUR",
        "footer_locked" => "LES CONTRÔLES DU JEU SONT BLOQUÉS QUAND LE MENU EST OUVERT",
        "mod_fpv" => "Caméra FPV",
        "mod_scout" => "Inspecteur HUD",
        "mod_hide" => "Masquer HUD",
        "mod_version" => "Tag version",
        "tile_settings" => "Paramètres",
        "tile_about" => "À propos",
        "parked" => "en pause",
        "settings_language" => "Langue",
        "settings_auto" => "Auto (jeu)",
        "settings_detected" => "Détectée",
        "settings_keys" => "Touches",
        "keys_f8" => "F8 — menu + capture souris",
        "keys_note" => "Les mods restent actifs quand le menu se ferme.",
        _ => en_str(key),
    }
}

fn de_str(key: &str) -> &'static str {
    match key {
        "help" => "HILFE",
        "back" => "ZURÜCK",
        "footer_locked" => "SPIELSTEUERUNG IST GESPERRT, SOLANGE DAS MENÜ OFFEN IST",
        "mod_fpv" => "FPV-Kamera",
        "mod_scout" => "HUD-Inspektor",
        "mod_hide" => "HUD ausblenden",
        "mod_version" => "Versions-Tag",
        "tile_settings" => "Einstellungen",
        "tile_about" => "Über",
        "parked" => "pausiert",
        "settings_language" => "Sprache",
        "settings_auto" => "Auto (Spiel)",
        "settings_detected" => "Erkannt",
        "settings_keys" => "Tasten",
        "keys_f8" => "F8 — Menü + Maus erfassen",
        "keys_note" => "Mods bleiben aktiv, wenn das Menü schließt.",
        _ => en_str(key),
    }
}

fn es_str(key: &str) -> &'static str {
    match key {
        "help" => "AYUDA",
        "back" => "ATRÁS",
        "footer_locked" => "LOS CONTROLES ESTÁN BLOQUEADOS MIENTRAS EL MENÚ ESTÁ ABIERTO",
        "mod_fpv" => "Cámara FPV",
        "mod_scout" => "Inspector HUD",
        "mod_hide" => "Ocultar HUD",
        "mod_version" => "Etiqueta versión",
        "tile_settings" => "Ajustes",
        "tile_about" => "Acerca de",
        "parked" => "en pausa",
        "settings_language" => "Idioma",
        "settings_auto" => "Auto (juego)",
        "settings_detected" => "Detectado",
        "settings_keys" => "Teclas",
        "keys_f8" => "F8 — menú + capturar ratón",
        "keys_note" => "Los mods siguen activos al cerrar el menú.",
        _ => en_str(key),
    }
}

fn it_str(key: &str) -> &'static str {
    match key {
        "help" => "AIUTO",
        "back" => "INDIETRO",
        "footer_locked" => "I CONTROLLI SONO BLOCCATI MENTRE IL MENU È APERTO",
        "mod_fpv" => "Fotocamera FPV",
        "mod_scout" => "Ispettore HUD",
        "mod_hide" => "Nascondi HUD",
        "mod_version" => "Tag versione",
        "tile_settings" => "Impostazioni",
        "tile_about" => "Info",
        "parked" => "in pausa",
        "settings_language" => "Lingua",
        "settings_auto" => "Auto (gioco)",
        "settings_detected" => "Rilevata",
        "settings_keys" => "Tasti",
        "keys_f8" => "F8 — menu + cattura mouse",
        "keys_note" => "Le mod restano attive alla chiusura del menu.",
        _ => en_str(key),
    }
}

fn pt_str(key: &str) -> &'static str {
    match key {
        "help" => "AJUDA",
        "back" => "VOLTAR",
        "footer_locked" => "OS CONTROLOS ESTÃO BLOQUEADOS ENQUANTO O MENU ESTIVER ABERTO",
        "mod_fpv" => "Câmara FPV",
        "mod_scout" => "Inspetor HUD",
        "mod_hide" => "Ocultar HUD",
        "mod_version" => "Tag de versão",
        "tile_settings" => "Configurações",
        "tile_about" => "Sobre",
        "parked" => "pausado",
        "settings_language" => "Idioma",
        "settings_auto" => "Auto (jogo)",
        "settings_detected" => "Detetado",
        "settings_keys" => "Teclas",
        "keys_f8" => "F8 — menu + capturar rato",
        "keys_note" => "Os mods continuam ativos ao fechar o menu.",
        _ => en_str(key),
    }
}

fn ru_str(key: &str) -> &'static str {
    match key {
        "help" => "ПОМОЩЬ",
        "back" => "НАЗАД",
        "footer_locked" => "УПРАВЛЕНИЕ ЗАБЛОКИРОВАНО, ПОКА ОТКРЫТО МЕНЮ",
        "mod_fpv" => "FPV-камера",
        "mod_scout" => "Инспектор HUD",
        "mod_hide" => "Скрыть HUD",
        "mod_version" => "Тег версии",
        "tile_settings" => "Настройки",
        "tile_about" => "О проекте",
        "parked" => "пауза",
        "settings_language" => "Язык",
        "settings_auto" => "Авто (игра)",
        "settings_detected" => "Определён",
        "settings_keys" => "Клавиши",
        "keys_f8" => "F8 — меню + захват мыши",
        "keys_note" => "Моды остаются включены при закрытии меню.",
        _ => en_str(key),
    }
}
