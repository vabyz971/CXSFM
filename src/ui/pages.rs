//! Menu pages shown inside the phone: settings and about (the main
//! tile grid lives in `phone`).
//!
//! Both pages use the game's rows (`ui::game`), so the pad drives them
//! like every other screen: up/down moves, left/right changes a value,
//! A acts, B goes back. Each page returns the help line of the focused
//! row; the phone shows it in its bottom strip.

use super::game::{self, Nav, NavState};
use super::theme::TEXT_DIM;
use crate::menu::{Page, set_page};
use crate::mod_api::get_mod_manager;
use crate::padnav;
use egui::{RichText, Ui};
use std::sync::{LazyLock, Mutex};

/// Pad-focus owner shared with the phone grid.
const OWNER: &str = "menu";

static SETTINGS_NAV: LazyLock<Mutex<NavState>> = LazyLock::new(|| Mutex::new(NavState::default()));
static ABOUT_NAV: LazyLock<Mutex<NavState>> = LazyLock::new(|| Mutex::new(NavState::default()));

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Wrapping dim line (never widens the phone).
fn note(ui: &mut Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text.into()).size(16.0).color(TEXT_DIM)).wrap().sense(egui::Sense::hover()));
}

/// Settings page: language, auto-load, keys. Returns the focused row's
/// `(title, help)`.
pub fn page_settings(ui: &mut Ui) -> Option<(String, String)> {
    let mut st = lock(&SETTINGS_NAV);
    let mut nav = Nav::begin(ui, &mut st, OWNER);
    if nav.window_events() & padnav::BACK != 0 {
        set_page(Page::Main);
    }
    let t = crate::i18n::t;
    let detected = match crate::i18n::detected() {
        Some(l) => format!("{} ({})", l.name(), l.code()),
        None => "—".to_string(),
    };
    let lang_help = format!("{}: {}", t("settings_detected"), detected);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        game::section(ui, t("settings_language"));
        // Option 0 = follow the game, then every shipped language.
        let mut names: Vec<String> = vec![t("settings_auto").to_string()];
        names.extend(crate::i18n::Lang::ALL.iter().map(|l| format!("{} ({})", l.name(), l.code())));
        let opts: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut cur = crate::i18n::override_lang()
            .and_then(|l| crate::i18n::Lang::ALL.iter().position(|x| *x == l))
            .map_or(0, |i| i + 1);
        let out = game::choice_row(ui, &mut nav, "cx_lang", t("settings_language"), &mut cur, &opts, &lang_help);
        if out.changed {
            crate::i18n::set_override(cur.checked_sub(1).and_then(|i| crate::i18n::Lang::ALL.get(i).copied()));
        }
        if out.reset {
            crate::i18n::set_override(None);
        }

        game::section(ui, "Saves");
        let mut auto = crate::config::auto_load();
        let out = game::toggle_row(ui, &mut nav, "cx_autoload", t("settings_autoload"), &mut auto, t("settings_autoload_hint"));
        if out.changed {
            crate::config::set_auto_load(auto);
            crate::config::mark_settings_dirty();
        }

        game::section(ui, t("settings_keys"));
        note(ui, t("keys_f8"));
        note(ui, t("keys_note"));
    });
    let help = nav.help.clone().map(|h| (t("tile_settings").to_string(), h));
    nav.finish();
    help
}

/// About page: version, stats, key reminder and one line per mod.
pub fn page_about(ui: &mut Ui) -> Option<(String, String)> {
    let mut st = lock(&ABOUT_NAV);
    let mut nav = Nav::begin(ui, &mut st, OWNER);
    if nav.window_events() & padnav::BACK != 0 {
        set_page(Page::Main);
    }
    let mgr = get_mod_manager();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let out = game::button_row(ui, &mut nav, "cx_about_back", crate::i18n::t("back"), "", "A  BACK", "Back to the mod list.");
        if out.accept {
            set_page(Page::Main);
        }
        game::section(ui, format!("CXSFM {}", env!("CARGO_PKG_VERSION")).as_str());
        for line in [
            format!("Platform: {}", std::env::consts::OS),
            format!("Uptime: {} s", mgr.uptime_secs()),
            format!("Mods: {} enabled / {} total", mgr.enabled_count(), mgr.mod_count()),
            format!("Log: {}", crate::log_path().display()),
        ] {
            note(ui, line);
        }
        game::section(ui, crate::i18n::t("settings_keys"));
        note(ui, crate::i18n::t("keys_f8"));
        note(ui, crate::i18n::t("keys_note"));
        game::section(ui, "Mods");
        // Descriptions come from each mod's own `i18n` module, so adding
        // a mod never touches framework translation files.
        let code = crate::i18n::current().code();
        for (name, on, desc) in mgr.describe_all(code) {
            let line = if desc.is_empty() {
                format!("• {name} ({})", if on { "on" } else { "off" })
            } else {
                format!("• {name} — {desc} ({})", if on { "on" } else { "off" })
            };
            note(ui, line);
        }
    });
    let help = nav.help.clone().map(|h| (crate::i18n::t("tile_about").to_string(), h));
    nav.finish();
    help
}
