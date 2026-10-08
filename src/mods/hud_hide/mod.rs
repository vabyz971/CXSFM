//! HUD Hide example mod: masquer un élément de HUD.
//!
//! Copy this folder to start a write-capable mod: it shows the full
//! surgical pattern — resolve by exact object name, capture the
//! original `Behaviour.enabled` flag BEFORE writing, keep it `false`
//! while armed, restore the exact flag on disable.
//!
//! # Why hiding instead of rewriting
//!
//! Text rewrites ran without exception yet never showed on screen
//! (mesh-rebuild subtleties from a foreign thread, or a non-rendered
//! object). `Behaviour.enabled = false` is the surgical alternative:
//! one boolean the renderer checks every frame — no strings, no mesh
//! rebuild, instantly visible when it works, instantly restored.

pub mod i18n;

use super::common::{TAG_CHECK_EVERY_TICKS, TAG_SEARCH_EVERY_TICKS};
use crate::mods::api::{Mod, TileIcon};
use crate::unity::UnityCache;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Hide target: the proven `Net.` label (hide confirmed on screen).
const HIDE_TARGET_NAME: &str = "Text (TMP) Net";

/// HUD Hide — hides one static label by flipping its Behaviour flag,
/// restores it on disable.
///
/// While armed, and only while armed: resolve the target object
/// (re-resolved whenever the handle dies — scene changes), capture
/// its original `enabled` flag once, and keep it `false`. Disable
/// restores the exact original flag.
pub struct HudHideMod {
    /// Whether hiding is currently active.
    enabled: AtomicBool,
    /// Cached Unity classes/methods (`None` until first resolve).
    unity: Mutex<Option<UnityCache>>,
    /// Live target handle as an address (`usize`: `Send + Sync` without
    /// wrapper types; cast back at use). Cleared on scene change.
    target: Mutex<Option<usize>>,
    /// Original `enabled` flag, captured once per arming (restore
    /// source). `None` = nothing captured → nothing is ever written.
    was_enabled: Mutex<Option<bool>>,
    /// Tick counter driving both cadences.
    tick: AtomicU64,
    /// Set on arming: the next `on_update` maintains immediately instead
    /// of waiting for the cadence (users toggle faster than 5 s — the
    /// search must not miss the arming window).
    pending: AtomicBool,
    /// Whether "target missing" was already logged this arming.
    missing_logged: AtomicBool,
}

impl HudHideMod {
    /// Create a new (disarmed) hide mod.
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            target: Mutex::new(None),
            was_enabled: Mutex::new(None),
            tick: AtomicU64::new(0),
            pending: AtomicBool::new(false),
            missing_logged: AtomicBool::new(false),
        }
    }

    /// Check if hiding is currently armed.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Ensure the Unity cache exists; resolve once, reuse afterwards.
    fn ensure_unity(&self) -> bool {
        if self.unity.lock().unwrap().is_some() {
            return true;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return false,
        };
        // SAFETY: attached framework threads, null-checked chain.
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

    /// Find the target by exact object name; capture its original
    /// `enabled` flag. Returns the live handle address, or `None`
    /// (missing/unreadable — never write blind).
    fn search_target(&self) -> Option<usize> {
        if !self.ensure_unity() {
            return None;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return None,
        };
        // Bind first so the guard drops at the statement boundary (see
        // the self-deadlock note in `poll_framework_input_edge`).
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return None,
        };
        if cache.tmp_klass.is_null() || cache.m_get_name.is_null() {
            return None;
        }
        // SAFETY: attached thread; objects consumed immediately.
        unsafe {
            for obj in crate::unity::find_tmp_texts(api, &cache) {
                let name = match crate::il2cpp::invoke(api, cache.m_get_name, obj, &[]) {
                    Some(s) => match crate::il2cpp::read_string(api, s) {
                        Some(n) => n,
                        None => continue,
                    },
                    None => continue,
                };
                if name != HIDE_TARGET_NAME {
                    continue;
                }
                // Exact hit: capture the original flag BEFORE writing
                // anything. Skip targets whose flag is unreadable.
                // (Already inside the surrounding unsafe block.)
                match crate::unity::beh_get_enabled(api, &cache, obj) {
                    Some(flag) => {
                        // Live object by construction (just enumerated).
                        let active = crate::unity::component_active(api, &cache, obj);
                        crate::log_line(&format!(
                            "hud: hide target resolved (enabled={flag}, active={})",
                            match active {
                                Some(true) => "yes",
                                Some(false) => "NO — disabled branch, hiding is a no-op visually",
                                None => "unknown",
                            }
                        ));
                        *self.was_enabled.lock().unwrap() = Some(flag);
                        return Some(obj as usize);
                    }
                    None => return None,
                }
            }
            None
        }
    }

    /// Maintain hiding: search when handle-less, verify when held.
    ///
    /// Resolves the cache FIRST: an empty cache must trigger a resolve,
    /// not a silent return (that hole froze all tag activity with zero
    /// log lines — the cache only filled inside `search_target`, which
    /// this function returned before reaching).
    fn maintain(&self) {
        if !self.ensure_unity() {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        let addr = *self.target.lock().unwrap();
        match addr {
            None => {
                if !self.missing_logged.swap(true, Ordering::SeqCst) {
                    crate::log_line(&format!(
                        "hud: hide target '{HIDE_TARGET_NAME}' not present in this scene (retrying)"
                    ));
                }
                if let Some(a) = self.search_target() {
                    *self.target.lock().unwrap() = Some(a);
                    self.missing_logged.store(false, Ordering::SeqCst);
                }
            }
            Some(a) => {
                let obj = a as *mut std::ffi::c_void;
                let was = self.was_enabled.lock().unwrap().clone();
                let _was = match was {
                    Some(w) => w,
                    None => {
                        // No captured original: drop the handle rather
                        // than write unrestorably.
                        *self.target.lock().unwrap() = None;
                        return;
                    }
                };
                // SAFETY: attached thread; handle re-validated below by
                // the read (dead objects read as garbage → mismatch →
                // re-search; never a write to a stale pointer... see
                // note below).
                //
                // NOTE: a destroyed Unity object is NOT freed memory
                // (the handle stays mapped); the worst case is a failed
                // read or a no-op write, both null-checked. Scene
                // changes surface as flag mismatch → re-search.
                let current = unsafe { crate::unity::beh_get_enabled(api, &cache, obj) };
                match current {
                    Some(false) => {} // hidden as intended
                    Some(true) => {
                        // SAFETY: same live-object reasoning as above.
                        if unsafe { crate::unity::beh_set_enabled(api, &cache, obj, false) } {
                            // Immediate read-back: distinguishes "flag
                            // changed, display didn't" (wrong/invisible
                            // object) from "write silently failed".
                            let readback =
                                unsafe { crate::unity::beh_get_enabled(api, &cache, obj) };
                            crate::log_line(&format!(
                                "hud: label hidden ('{HIDE_TARGET_NAME}' readback={})",
                                match readback {
                                    Some(v) => v.to_string(),
                                    None => String::from("<unreadable>"),
                                }
                            ));
                        }
                    }
                    None => {
                        // Unreadable: assume scene change, re-search.
                        *self.target.lock().unwrap() = None;
                    }
                }
            }
        }
    }

    /// Restore the captured flag through a live handle, if any.
    fn restore(&self) {
        let (addr, was) = (
            *self.target.lock().unwrap(),
            self.was_enabled.lock().unwrap().clone(),
        );
        let (api, cache, obj, flag) = match (
            crate::il2cpp_api(),
            self.unity.lock().unwrap().as_ref().copied(),
            addr,
            was,
        ) {
            (Some(api), Some(cache), Some(a), Some(f)) => {
                (api, cache, a as *mut std::ffi::c_void, f)
            }
            _ => return,
        };
        // SAFETY: same live-object reasoning as `maintain`.
        unsafe {
            let current = crate::unity::beh_get_enabled(api, &cache, obj);
            if matches!(current, Some(c) if c != flag)
                && crate::unity::beh_set_enabled(api, &cache, obj, flag)
            {
                crate::log_line(&format!(
                    "hud: label restored ('{HIDE_TARGET_NAME}' enabled={flag})"
                ));
            }
        }
    }
}

impl Default for HudHideMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for HudHideMod {
    fn name(&self) -> &'static str {
        "HUD Hide"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_hide"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::EyeOff
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.is_enabled() {
            return;
        }
        // Arming runs maintain on the very next tick; afterwards the two
        // cadences take over (gentle search while handle-less, 1 Hz
        // verify while held).
        if self.pending.swap(false, Ordering::SeqCst) {
            self.maintain();
            return;
        }
        let tick = self.tick.fetch_add(1, Ordering::SeqCst);
        let has_handle = self.target.lock().unwrap().is_some();
        let due = if has_handle {
            tick % TAG_CHECK_EVERY_TICKS == 0
        } else {
            tick % TAG_SEARCH_EVERY_TICKS == 0
        };
        if !due {
            return;
        }
        self.maintain();
    }

    // No `on_draw_ui`: the menu tile is the whole UI (single window).

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        // Fresh arming: drop stale handles/flags (new scene may hold a
        // different object under the same name) and maintain on the
        // next tick — no waiting for the search cadence.
        *self.target.lock().unwrap() = None;
        *self.was_enabled.lock().unwrap() = None;
        self.missing_logged.store(false, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        crate::log_line("hud: hide armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore();
        *self.target.lock().unwrap() = None;
        *self.was_enabled.lock().unwrap() = None;
        crate::log_line("hud: hide off");
    }
}

/// Register the hide mod disarmed: the framework boots quiet — no game
/// writes before an explicit user action on the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(HudHideMod::new()));
}
