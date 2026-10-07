//! HUD Scout mod: read-only inventory of the game's UI texts (uGUI +
//! TextMeshPro).
//!
//! # Why read-only
//!
//! Rewriting HUD text is only sane once you know which texts exist and
//! which are static (game rewrites dynamic readouts every frame — the
//! same fight as FPV's FOV, target statics). This mod performs ZERO
//! writes: when armed it enumerates every live `UnityEngine.UI.Text`
//! and `TMPro.TextMeshProUGUI` once, logs `name + content` per item,
//! then idles. The log inventory is the shopping list for a later
//! HUD-rewrite mod. Nothing to restore on disable — nothing was ever
//! touched.
//!
//! Armed at registration and re-armed by the O key (see tick thread):
//! press O in each scene/menu to inventory that scene's texts.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use egui;
use crate::mod_api::Mod;
use crate::unity::UnityCache;

/// Resolve attempts are throttled: every Nth tick (~1 Hz at 60 Hz).
const RESOLVE_EVERY_TICKS: u64 = 60;
/// Max texts inventoried per run (a broken count must not flood the log).
const MAX_ITEMS: usize = 64;
/// Content preview length (chars, newlines flattened — one log line each).
const PREVIEW_CHARS: usize = 100;

/// Flatten to one line and cap length: log records must stay
/// single-line (see `log_line`'s atomic-record contract). Shared by the
/// scout inventory and the version-tag read-backs.
fn preview(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .take(PREVIEW_CHARS + 1)
        .collect();
    if flat.chars().count() > PREVIEW_CHARS {
        let mut s: String = flat.chars().take(PREVIEW_CHARS).collect();
        s.push('…');
        s
    } else {
        flat
    }
}

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
    /// Create a new (disarmed — registration arms it) scout.
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
            let domain = match api.domain() {
                Ok(d) => d,
                Err(_) => return false,
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

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        if !self.is_enabled() {
            return;
        }
        egui::Window::new("HUD Scout")
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 220.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("HUD Scout");
                ui.label("Read-only UI inventory (see log). Zero writes.");
                if self.done.load(Ordering::SeqCst) {
                    ui.label("Status: complete, mod idle.");
                } else {
                    ui.label("Status: waiting for Unity scripting…");
                }
            });
    }

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

/// Register the scout and arm it immediately: the one-shot inventory
/// needs no keypress and no render hook, just a resolved Unity cache.
pub fn register_hud_scout() {
    crate::mod_api::register_mod(Box::new(HudScoutMod::new()));
    crate::mod_api::get_mod_manager().enable_mod("HUD Scout");
}

/// Hide target: the proven `Net.` label (hide confirmed on screen).
const HIDE_TARGET_NAME: &str = "Text (TMP) Net";
/// Version-text target: the static per-session username label (same
/// menu scenes as the hide target; independent objects so both mods
/// stay verifiable in one O press).
const VERSION_TARGET_NAME: &str = "Nickname Text (TMP)";
/// Check cadence with a live handle (~1 Hz at 60 Hz tick).
const TAG_CHECK_EVERY_TICKS: u64 = 60;
/// Search cadence without a handle (gentle scene walk, ~0.2 Hz).
const TAG_SEARCH_EVERY_TICKS: u64 = 300;

/// HUD Hide — hides one static label by flipping its Behaviour flag,
/// restores it on disable.
///
/// # Why hiding instead of rewriting
///
/// Text rewrites ran without exception yet never showed on screen
/// (mesh-rebuild subtleties from a foreign thread, or a non-rendered
/// object). `Behaviour.enabled = false` is the surgical alternative:
/// one boolean the renderer checks every frame — no strings, no mesh
/// rebuild, instantly visible when it works, instantly restored.
///
/// # How it works
///
/// While armed, and only while armed: resolve the `Text (TMP) Net`
/// object (re-resolved whenever the handle dies — scene changes),
/// capture its original `enabled` flag once, and keep it `false`.
/// Disable restores the exact original flag.
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
            let domain = match api.domain() {
                Ok(d) => d,
                Err(_) => return false,
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
                let current =
                    unsafe { crate::unity::beh_get_enabled(api, &cache, obj) };
                match current {
                    Some(false) => {} // hidden as intended
                    Some(true) => {
                        // SAFETY: same live-object reasoning as above.
                        if unsafe { crate::unity::beh_set_enabled(api, &cache, obj, false) } {
                            // Immediate read-back: distinguishes "flag
                            // changed, display didn't" (wrong/invisible
                            // object) from "write silently failed".
                            let readback = unsafe {
                                crate::unity::beh_get_enabled(api, &cache, obj)
                            };
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
            if matches!(current, Some(c) if c != flag) {
                if crate::unity::beh_set_enabled(api, &cache, obj, flag) {
                    crate::log_line(&format!(
                        "hud: label restored ('{HIDE_TARGET_NAME}' enabled={flag})"
                    ));
                }
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

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        if !self.is_enabled() {
            return;
        }
        let state = if self.target.lock().unwrap().is_some() {
            "Status: label hidden."
        } else {
            "Status: target not in this scene."
        };
        egui::Window::new("HUD Hide")
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 260.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("HUD Hide");
                ui.label(format!("Target: {HIDE_TARGET_NAME}"));
                ui.label(state);
            });
    }

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

/// Register the hide mod disarmed: the user opts in with O (see the
/// tick thread). The framework boots quiet — no game writes before an
/// explicit action.
pub fn register_hud_hide() {
    crate::mod_api::register_mod(Box::new(HudHideMod::new()));
}

/// HUD Version Tag (v2) — stamps the framework version onto the proven
/// target label, restores it on disable.
///
/// # Why retry
///
/// The first attempt failed before a single write executed (the
/// silent-maintain bug: cache checked before resolved). The hide mod
/// since proved the full pipeline — resolve → write → pixels — from
/// the same thread. Same scene family (static username label), same
/// instrumentation (active state + immediate read-back), fixed
/// maintain. If the text still doesn't show, the read-back lines will
/// say whether it's memory or mesh.
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

    /// Desired content for the label.
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
            let domain = match api.domain() {
                Ok(d) => d,
                Err(_) => return false,
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
                // read mismatch → re-search (see `HudHideMod::maintain`
                // for the full reasoning).
                let current =
                    unsafe { crate::unity::tmp_get_text(api, &cache, obj) };
                match current {
                    Some(text) if text == want => {}
                    Some(_) => {
                        // SAFETY: same live-object reasoning as above.
                        if unsafe { crate::unity::tmp_set_text(api, &cache, obj, &want) } {
                            let readback = unsafe {
                                crate::unity::tmp_get_text(api, &cache, obj)
                            }
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
            if matches!(current, Some(c) if c != text) {
                if crate::unity::tmp_set_text(api, &cache, obj, &text) {
                    crate::log_line(&format!(
                        "hud: version tag restored ('{VERSION_TARGET_NAME}')"
                    ));
                }
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

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        if !self.is_enabled() {
            return;
        }
        let state = if self.target.lock().unwrap().is_some() {
            "Status: tag live."
        } else {
            "Status: target not in this scene."
        };
        egui::Window::new("HUD Version Tag")
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 300.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("HUD Version Tag");
                ui.label(format!("Target: {VERSION_TARGET_NAME}"));
                ui.label(state);
            });
    }

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

/// Register the version tag disarmed: the user opts in with O (see the
/// tick thread). The framework boots quiet — no game writes before an
/// explicit action.
pub fn register_hud_version() {
    crate::mod_api::register_mod(Box::new(HudVersionMod::new()));
}
