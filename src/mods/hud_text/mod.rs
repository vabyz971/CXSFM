//! HUD Text example mod: remplacer du texte (et le masquer en le
//! vidant — même pattern, contenu vide).
//!
//! Copy this folder to start a text-rewriting mod: it stamps the
//! framework version onto a static label, repairs drift every second
//! (the game may rewrite the label), and restores the exact original
//! on disable. To BLANK a text instead of replacing it, use an empty
//! `desired()` — capture/restore/read-back are unchanged.
//!
//! Same scene family as hud_hide (static labels), same
//! instrumentation (active state + immediate read-back): if the text
//! still doesn't show, the read-back lines say whether it's memory
//! or mesh.

pub mod i18n;

use super::common::{TAG_CHECK_EVERY_TICKS, TAG_SEARCH_EVERY_TICKS, preview};
use crate::mods::api::{Mod, TileIcon};
use crate::unity::UnityCache;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Version-text target: the static per-session username label (same
/// menu scenes as the hide target; independent objects so both mods
/// stay verifiable in one pass).
const VERSION_TARGET_NAME: &str = "Nickname Text (TMP)";

/// HUD Version Tag — stamps the framework version onto the proven
/// target label, restores it on disable.
pub struct HudVersionMod {
    /// Whether the tag is currently applied/maintained.
    enabled: AtomicBool,
    /// Cached Unity classes/methods (`None` until first resolve).
    unity: Mutex<Option<UnityCache>>,
    /// Live target handle as an address (`usize`: `Send + Sync` without
    /// wrapper types; cast back at use). Cleared on scene change.
    target: Mutex<Option<usize>>,
    /// Original content, captured once per arming (restore source).
    /// `None` = nothing captured → nothing is ever written.
    original: Mutex<Option<String>>,
    /// Tick counter driving both cadences.
    tick: AtomicU64,
    /// Set on arming: the next `on_update` maintains immediately.
    pending: AtomicBool,
    /// Whether "target missing" was already logged this arming.
    missing_logged: AtomicBool,
}

impl HudVersionMod {
    /// Create a new (disarmed) version tag.
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            target: Mutex::new(None),
            original: Mutex::new(None),
            tick: AtomicU64::new(0),
            pending: AtomicBool::new(false),
            missing_logged: AtomicBool::new(false),
        }
    }

    /// Check if the tag is currently armed.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Desired content for the label (return `String::new()` here to
    /// BLANK the text instead of replacing it).
    fn desired(original: &str) -> String {
        format!("{} | CXSFM v{}", original, env!("CARGO_PKG_VERSION"))
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
    /// content. Returns the live handle address, or `None`
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
        if cache.tmp_klass.is_null() {
            return None;
        }
        // SAFETY: attached thread; objects consumed immediately.
        unsafe {
            for obj in crate::unity::find_tmp_texts(api, &cache) {
                let name_opt = if cache.m_get_name.is_null() {
                    None
                } else {
                    // SAFETY: already inside the surrounding unsafe block.
                    crate::il2cpp::invoke(api, cache.m_get_name, obj, &[])
                };
                let name = match name_opt {
                    Some(s) => match crate::il2cpp::read_string(api, s) {
                        Some(n) => n,
                        None => continue,
                    },
                    None => continue,
                };
                if name != VERSION_TARGET_NAME {
                    continue;
                }
                // Exact hit: active state + original BEFORE writing.
                // (Already inside the surrounding unsafe block.)
                let active = crate::unity::component_active(api, &cache, obj);
                match crate::unity::tmp_get_text(api, &cache, obj) {
                    Some(original) => {
                        crate::log_line(&format!(
                            "hud: version target resolved (active={}, original=\"{}\")",
                            match active {
                                Some(true) => "yes",
                                Some(false) => "NO",
                                None => "unknown",
                            },
                            preview(&original),
                        ));
                        *self.original.lock().unwrap() = Some(original);
                        return Some(obj as usize);
                    }
                    None => return None,
                }
            }
            None
        }
    }

    /// Maintain the tag: resolve the cache FIRST, then search when
    /// handle-less, verify when held.
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
                        "hud: version target '{VERSION_TARGET_NAME}' not present in this scene (retrying)"
                    ));
                }
                if let Some(a) = self.search_target() {
                    *self.target.lock().unwrap() = Some(a);
                    self.missing_logged.store(false, Ordering::SeqCst);
                }
            }
            Some(a) => {
                let obj = a as *mut std::ffi::c_void;
                let original = self.original.lock().unwrap().clone();
                let original = match original {
                    Some(o) => o,
                    None => {
                        *self.target.lock().unwrap() = None;
                        return;
                    }
                };
                let want = Self::desired(&original);
                // SAFETY: attached thread; stale handles surface as
                // read mismatch → re-search (see hud_hide's `maintain`
                // for the full reasoning).
                let current = unsafe { crate::unity::tmp_get_text(api, &cache, obj) };
                match current {
                    Some(text) if text == want => {}
                    Some(_) => {
                        // SAFETY: same live-object reasoning as above.
                        if unsafe { crate::unity::tmp_set_text(api, &cache, obj, &want) } {
                            let readback = unsafe { crate::unity::tmp_get_text(api, &cache, obj) }
                                .map(|t| preview(&t))
                                .unwrap_or_else(|| String::from("<unreadable>"));
                            crate::log_line(&format!(
                                "hud: version tag applied ('{VERSION_TARGET_NAME}' readback=\"{readback}\")"
                            ));
                        }
                    }
                    None => {
                        *self.target.lock().unwrap() = None;
                    }
                }
            }
        }
    }

    /// Restore the captured original through a live handle, if any.
    fn restore(&self) {
        let (addr, original) = (
            *self.target.lock().unwrap(),
            self.original.lock().unwrap().clone(),
        );
        let (api, cache, obj, text) = match (
            crate::il2cpp_api(),
            self.unity.lock().unwrap().as_ref().copied(),
            addr,
            original,
        ) {
            (Some(api), Some(cache), Some(a), Some(t)) => {
                (api, cache, a as *mut std::ffi::c_void, t)
            }
            _ => return,
        };
        // SAFETY: same live-object reasoning as `maintain`.
        unsafe {
            let current = crate::unity::tmp_get_text(api, &cache, obj);
            if matches!(current, Some(c) if c != text)
                && crate::unity::tmp_set_text(api, &cache, obj, &text)
            {
                crate::log_line(&format!(
                    "hud: version tag restored ('{VERSION_TARGET_NAME}')"
                ));
            }
        }
    }
}

impl Default for HudVersionMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for HudVersionMod {
    fn name(&self) -> &'static str {
        "HUD Version Tag"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_version"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Tag
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.is_enabled() {
            return;
        }
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
        *self.target.lock().unwrap() = None;
        *self.original.lock().unwrap() = None;
        self.missing_logged.store(false, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        crate::log_line("hud: version tag armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore();
        *self.target.lock().unwrap() = None;
        *self.original.lock().unwrap() = None;
        crate::log_line("hud: version tag off");
    }
}

/// Register the version tag disarmed: the framework boots quiet — no
/// game writes before an explicit user action on the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(HudVersionMod::new()));
}
