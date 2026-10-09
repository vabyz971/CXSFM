//! Multilingual UI strings with automatic game-language detection.
//!
//! One file per language (`en.rs`, `fr.rs`, …): adding a language =
//! one new file + one line in [`Lang::ALL`] + one arm in [`t`].
//! English is the fallback for any missing key, so partial
//! translations never blank the UI.
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
//! follows the chain above.
//!
//! # Mod keys
//!
//! Framework keys are bare (`help`, `tile_settings`, …). Mods use
//! namespaced keys (`{mod_id}:{key}`); each mod owns its translations
//! next to its code (see its folder) and falls back to English like
//! every language file here.

pub mod de;
pub mod en;
pub mod es;
pub mod fr;
pub mod it;
pub mod pt;
pub mod ru;

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

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

/// Bumped on every effective-language change (pin or detection).
/// The mod registry caches per-mod presentation against this revision
/// so labels follow the language without calling into mods per frame.
static LANG_REV: AtomicU64 = AtomicU64::new(0);

/// Current language revision (see `LANG_REV`).
pub fn lang_rev() -> u64 {
    LANG_REV.load(Ordering::SeqCst)
}

/// Raw `systemLanguage` mapped once the IL2CPP bridge is up.
static DETECTED: AtomicU8 = AtomicU8::new(UNSET);
/// Manual pin from the Settings tile (`UNSET` = Auto).
static OVERRIDE: AtomicU8 = AtomicU8::new(UNSET);

/// Record the game's language (call once the bridge resolves it).
/// `None` (icall missing) also sticks — no retry spam every tick.
pub fn note_detected(raw: Option<i32>) {
    let v = raw.and_then(Lang::from_system_language).map(Lang::encode);
    DETECTED.store(v.unwrap_or(UNSET), Ordering::SeqCst);
    LANG_REV.fetch_add(1, Ordering::SeqCst);
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
    LANG_REV.fetch_add(1, Ordering::SeqCst);
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
/// Mod tile keys (`mod_scout`, `mod_hide`, `mod_version`) are owned by
/// their mod folder and resolved first; everything else goes to the
/// per-language files. Unknown keys echo back (never blank); any
/// language falls back to English per key, so partial translations
/// stay usable.
pub fn t(key: &str) -> &'static str {
    if let Some(s) = mod_tile(key) {
        return s;
    }
    match current() {
        Lang::En => en::strings(key),
        Lang::Fr => fr::strings(key),
        Lang::De => de::strings(key),
        Lang::Es => es::strings(key),
        Lang::It => it::strings(key),
        Lang::Pt => pt::strings(key),
        Lang::Ru => ru::strings(key),
    }
}

/// Mod-owned tile labels. New mod = one arm here pointing at its own
/// `i18n::tile_label` (see `docs/plugin-guide.md`).
fn mod_tile(key: &str) -> Option<&'static str> {
    let code = current().code();
    match key {
        "mod_scout" => Some(crate::mods::hud_scout::i18n::tile_label(code)),
        "mod_hide" => Some(crate::mods::hud_hide::i18n::tile_label(code)),
        "mod_version" => Some(crate::mods::hud_text::i18n::tile_label(code)),
        "mod_inspector" => Some(crate::mods::inspector::i18n::tile_label(code)),
        "mod_camera" => Some(crate::mods::camera::i18n::tile_label(code)),
        "mod_video" => Some(crate::mods::video::i18n::tile_label(code)),
        "mod_gamelog" => Some(crate::mods::gamelog::i18n::tile_label(code)),
        "mod_speedo" => Some(crate::mods::speedo::i18n::tile_label(code)),
        _ => None,
    }
}
