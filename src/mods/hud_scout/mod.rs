//! HUD Scout example mod: read-only inventory of the game's UI texts.
//!
//! Copy this folder to start a new mod: it shows the minimal shape —
//! `struct` + `Mod` impl + `register()` + own `i18n` — with ZERO
//! writes. When armed it enumerates every live `UnityEngine.UI.Text`
//! and `TMPro.TextMeshProUGUI` once, logs `name + content` per item,
//! then idles. The log inventory is the shopping list for rewrite
//! mods. Nothing to restore on disable — nothing was ever touched.
//!
//! Disarmed at registration like every mod; armed from the menu tile.

pub mod i18n;

use super::common::{MAX_ITEMS, RESOLVE_EVERY_TICKS, preview};
use crate::mods::api::{Mod, TileIcon};
use crate::unity::UnityCache;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// HUD Scout — one-shot, read-only UI inventory.
///
/// Cosmetic-adjacent and anti-cheat-safe by construction: pure reads,
/// no game state altered, no traffic generated.
pub struct HudScoutMod {
    /// Whether the mod is currently armed (runs once per arming).
    enabled: AtomicBool,
    /// Cached Unity classes/methods (`None` until first resolve).
    unity: Mutex<Option<UnityCache>>,
    /// Set after a completed inventory; re-arming clears it.
    done: AtomicBool,
    /// Tick counter throttling resolve attempts.
    tick: AtomicU64,
}

impl HudScoutMod {
    /// Create a new disarmed scout (arming runs the one-shot).
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            done: AtomicBool::new(false),
            tick: AtomicU64::new(0),
        }
    }

    /// Check if the scout is currently armed.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Ensure the Unity cache exists; resolve once, reuse afterwards.
    /// Returns false outside Unity (or before scripting is up).
    fn ensure_unity(&self) -> bool {
        if self.unity.lock().unwrap().is_some() {
            return true;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return false,
        };
        // SAFETY: called on attached framework threads with the
        // published API; every step below is null-checked.
        let cache = unsafe {
            let domain = match crate::il2cpp::domain_checked(api) {
                Some(d) => d,
                None => return false,
            };
            match crate::unity::init(api, domain) {
                Some(c) => c,
                None => return false,
            }
        };
        *self.unity.lock().unwrap() = Some(cache);
        true
    }

    /// One inventory run: enumerate texts, log each, park via `done`.
    fn scout_once(&self) {
        if !self.ensure_unity() {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        // Bind first so the guard drops at the statement boundary (see
        // the self-deadlock note in `poll_framework_input_edge`).
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        // SAFETY: attached threads; cached classes; array consumed
        // immediately per `find_objects_of_type`'s contract.
        unsafe {
            // Legacy uGUI texts (often zero on modern builds) AND
            // TextMeshPro (where current games render their UI).
            let legacy = crate::unity::find_texts(api, &cache, std::ptr::null_mut());
            let tmp = crate::unity::find_tmp_texts(api, &cache);
            crate::log_line(&format!(
                "hud: scout found {} legacy Texts + {} TMP texts",
                legacy.len(),
                tmp.len()
            ));
            Self::log_items(api, &cache, "Text", &legacy, false);
            Self::log_items(api, &cache, "TMP", &tmp, true);
            crate::log_line("hud: scout complete — mod idle, no writes performed");
        }
        self.done.store(true, Ordering::SeqCst);
    }

    /// Log one inventory section. `tmp` selects the TextMeshPro getters;
    /// legacy uGUI otherwise. Read-only: names + content previews.
    ///
    /// # Safety
    /// Same contract as `scout_once` (attached thread, live objects).
    unsafe fn log_items(
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
        tag: &str,
        items: &[*mut std::ffi::c_void],
        tmp: bool,
    ) {
        // SAFETY: caller guarantees attached thread + live objects.
        unsafe {
            for (i, obj) in items.iter().take(MAX_ITEMS).enumerate() {
                let name = if cache.m_get_name.is_null() {
                    String::from("<unnamed>")
                } else {
                    match crate::il2cpp::invoke(api, cache.m_get_name, *obj, &[]) {
                        Some(s) => crate::il2cpp::read_string(api, s)
                            .unwrap_or_else(|| String::from("<unreadable>")),
                        None => String::from("<unnamed>"),
                    }
                };
                let content = if tmp {
                    crate::unity::tmp_get_text(api, cache, *obj)
                } else {
                    crate::unity::get_text(api, cache, *obj)
                }
                .map(|t| preview(&t))
                .unwrap_or_else(|| String::from("<unreadable>"));
                crate::log_line(&format!(
                    "hud: {tag}[{i}] name=\"{name}\" text=\"{content}\""
                ));
            }
            if items.len() > MAX_ITEMS {
                crate::log_line(&format!(
                    "hud: ... and {} more {tag} (capped at {})",
                    items.len() - MAX_ITEMS,
                    MAX_ITEMS
                ));
            }
        }
    }
}

impl Default for HudScoutMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for HudScoutMod {
    fn name(&self) -> &'static str {
        "HUD Scout"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_scout"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Scout
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.is_enabled() || self.done.load(Ordering::SeqCst) {
            return;
        }
        // Throttle resolve attempts; the inventory itself runs once.
        if self.tick.fetch_add(1, Ordering::SeqCst) % RESOLVE_EVERY_TICKS != 0 {
            return;
        }
        self.scout_once();
    }

    // No `on_draw_ui`: the menu tile is the whole UI (single window).
    // Status detail lives in the log; the tile accent shows on/off.

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        // Re-arm: toggling re-runs the inventory (new scene = new texts).
        self.done.store(false, Ordering::SeqCst);
        crate::log_line("hud: scout armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        crate::log_line("hud: scout disarmed (nothing to restore — read-only)");
    }
}

/// Register the scout disarmed: like every mod it boots quiet and the
/// user opts in from the menu tile (which re-arms the one-shot
/// inventory for the current scene). No UI, no log spam before an
/// explicit action.
pub fn register() {
    crate::mod_api::register_mod(Box::new(HudScoutMod::new()));
}
