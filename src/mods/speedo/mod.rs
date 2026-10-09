//! Custom speedometer: scale-zero hides the game's gauges, TMP reads
//! feed ours.
//!
//! Hiding technique ("scale zéro", minimal-hud pattern): setting
//! `Transform.localScale` to (0,0,0) hides a whole subtree while the
//! object stays ACTIVE, so game scripts keep updating texts —
//! indispensable, since our readout parses those same texts. Never
//! `SetActive(false)` (it would stop the scripts we read from).
//!
//! Groups (checkboxes in the window, Gauge + Speed on by default):
//! - Compteur : Speedometer/Background, Speedometer/Tachometer,
//!   Speedometer/Arrow (child paths under the "Speedometer" root).
//! - Vitesse  : GameObject "Text (TMP) Speed".
//! - Rapport  : GameObject "Text (TMP) Gear".
//! - Nitro    : Speedometer/Nitro, fallback "Nitro", plus "TextN2O".
//!
//! Rules (proven in game, do not improvise):
//! - Resolution only (a) on arming, (b) when a pointer died, (c) on
//!   the Refresh button, (d) every N>=5 s while a checked target is
//!   missing. NEVER a Find* per tick, NEVER FindObjectsOfType here.
//! - Original scale read BEFORE writing; a zero original is never
//!   recorded (fallback (1,1,1)).
//! - Maintain (~1 Hz) re-hides only on transition (game restored the
//!   scale); unchecking a group restores its originals on the tick.
//! - Value reads (~15 Hz) on pinned TMP handles, alive-checked;
//!   gear + speed published via atomics, render only.
//! - No Mutex held across a Unity call: copy out, drop the guard,
//!   then call. No match on a live guard that re-locks (bind-then-match).

pub mod i18n;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

/// Resolve cadence (ticks, >= 5 s at 60 Hz) while a checked hide
/// target is still missing.
const RESOLVE_EVERY_TICKS: u64 = 300;
/// Maintain cadence (~1 Hz): re-hide on transition, restore unchecked.
const MAINTAIN_EVERY_TICKS: u64 = 60;
/// Value read cadence (~15 Hz): pinned TMP handles, no enumeration.
const READ_EVERY_TICKS: u64 = 4;

/// Hide groups (checkbox ids).
const GROUP_GAUGE: u8 = 0;
const GROUP_SPEED: u8 = 1;
const GROUP_GEAR: u8 = 2;
const GROUP_NITRO: u8 = 3;

/// Zero scale (hidden) and its epsilon comparison.
const ZERO_SCALE: [f32; 3] = [0.0, 0.0, 0.0];
/// Fallback original when the live scale already reads zero.
const FALLBACK_SCALE: [f32; 3] = [1.0, 1.0, 1.0];
/// Scale epsilon: below this a scale counts as zero / equal.
const SCALE_EPS: f32 = 1e-6;

/// True when every component is near zero.
fn is_zero_scale(s: [f32; 3]) -> bool {
    s[0].abs() < SCALE_EPS && s[1].abs() < SCALE_EPS && s[2].abs() < SCALE_EPS
}

/// Group checkbox state lookup (pure, testable).
fn group_enabled(id: u8, gauge: bool, speed: bool, gear: bool, nitro: bool) -> bool {
    match id {
        GROUP_GAUGE => gauge,
        GROUP_SPEED => speed,
        GROUP_GEAR => gear,
        GROUP_NITRO => nitro,
        _ => false,
    }
}

/// Parse the leading numeric run of a HUD text (`"123"`, `"123 km/h"`,
/// `" 87.5 "`). Returns the value; unit is detected separately.
fn parse_speed(text: &str) -> Option<f32> {
    let mut num = String::new();
    let mut seen_digit = false;
    let mut seen_sep = false;
    for c in text.trim().chars() {
        if c.is_ascii_digit() {
            num.push(c);
            seen_digit = true;
        } else if (c == '.' || c == ',') && seen_digit && !seen_sep {
            num.push('.');
            seen_sep = true;
        } else if seen_digit {
            break;
        }
    }
    if !seen_digit {
        return None;
    }
    num.parse::<f32>().ok()
}

/// Parse a gear readout: digits → value, `N` → 0 (neutral), `R` → -1
/// (reverse). Case-insensitive, leading whitespace tolerated. Anything
/// else (empty included) → `None`, keep the last valid value.
fn parse_gear(text: &str) -> Option<i32> {
    let mut chars = text.trim().chars();
    let first = chars.next()?.to_ascii_uppercase();
    if first == 'N' {
        return Some(0);
    }
    if first == 'R' {
        return Some(-1);
    }
    if !first.is_ascii_digit() {
        return None;
    }
    let mut num = String::from(first);
    for c in chars {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            break;
        }
    }
    num.parse::<i32>().ok()
}

/// One hideable target: group id, root object name, optional child path
/// under the root's transform. All `Copy` (no allocation, no lock).
#[derive(Clone, Copy)]
struct HideTarget {
    group: u8,
    root: &'static str,
    child: Option<&'static str>,
}

/// Hide table: (group label, root, optional child).
const HIDE_TARGETS: &[HideTarget] = &[
    HideTarget { group: GROUP_GAUGE, root: "Speedometer", child: Some("Background") },
    HideTarget { group: GROUP_GAUGE, root: "Speedometer", child: Some("Tachometer") },
    HideTarget { group: GROUP_GAUGE, root: "Speedometer", child: Some("Arrow") },
    HideTarget { group: GROUP_SPEED, root: "Text (TMP) Speed", child: None },
    HideTarget { group: GROUP_GEAR, root: "Text (TMP) Gear", child: None },
    HideTarget { group: GROUP_NITRO, root: "Speedometer", child: Some("Nitro") },
    HideTarget { group: GROUP_NITRO, root: "Nitro", child: None },
    HideTarget { group: GROUP_NITRO, root: "TextN2O", child: None },
];

/// Runtime slot per table entry: bound address + recorded original.
struct HideSlot {
    target: HideTarget,
    addr: Option<usize>,
    orig: Option<[f32; 3]>,
    missing_logged: bool,
}

impl HideSlot {
    fn fresh(target: HideTarget) -> Self {
        Self { target, addr: None, orig: None, missing_logged: false }
    }

    fn path(&self) -> String {
        match self.target.child {
            Some(c) => format!("{}/{}", self.target.root, c),
            None => self.target.root.to_string(),
        }
    }
}

/// Custom speedometer: hide on tick (scale-zero), values on tick,
/// big digits on render.
pub struct SpeedoMod {
    enabled: AtomicBool,
    unity: Mutex<Option<UnityCache>>,
    tick: AtomicU64,
    /// Hide slots (one per table entry).
    slots: Mutex<Vec<HideSlot>>,
    /// Group checkboxes (UI writes, tick reads).
    gauge: AtomicBool,
    speed: AtomicBool,
    gear: AtomicBool,
    nitro: AtomicBool,
    /// Immediate resolve on next tick (arming, Refresh button).
    pending: AtomicBool,
    /// Last tick a resolve pass ran (missing-target throttle).
    last_resolve: AtomicU64,
    /// One-shot "N/M found" summary per arming.
    armed_logged: AtomicBool,
    /// Pinned TMP handles for value reads (no enumeration per read).
    speed_target: Mutex<Option<usize>>,
    gear_target: Mutex<Option<usize>>,
    /// Published readout (render reads atomics only).
    speed_bits: AtomicU32,
    gear_val: AtomicI32,
    valid: AtomicBool,
    /// Debug display (raw text + unit + source, UI reads).
    raw: Mutex<String>,
    unit: Mutex<String>,
    source: Mutex<String>,
    /// Render-side smoothing state (only touched in `on_draw_ui`).
    disp: f32,
    /// Pin + opacity chrome (shared tool pattern).
    tool: ToolChrome,
}

impl SpeedoMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            tick: AtomicU64::new(0),
            slots: Mutex::new(HIDE_TARGETS.iter().map(|t| HideSlot::fresh(*t)).collect()),
            gauge: AtomicBool::new(true),
            speed: AtomicBool::new(true),
            gear: AtomicBool::new(false),
            nitro: AtomicBool::new(false),
            pending: AtomicBool::new(false),
            last_resolve: AtomicU64::new(0),
            armed_logged: AtomicBool::new(false),
            speed_target: Mutex::new(None),
            gear_target: Mutex::new(None),
            speed_bits: AtomicU32::new(0.0f32.to_bits()),
            gear_val: AtomicI32::new(0),
            valid: AtomicBool::new(false),
            raw: Mutex::new(String::new()),
            unit: Mutex::new(String::from("km/h")),
            source: Mutex::new(String::new()),
            disp: 0.0,
            tool: ToolChrome::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn group_on(&self, id: u8) -> bool {
        group_enabled(
            id,
            self.gauge.load(Ordering::SeqCst),
            self.speed.load(Ordering::SeqCst),
            self.gear.load(Ordering::SeqCst),
            self.nitro.load(Ordering::SeqCst),
        )
    }

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

    /// Bind value sources (tick thread, throttled by caller): one TMP
    /// enumeration, exact-name match for speed + gear (fallback:
    /// name containing "speed" for the speed source only). No hiding
    /// here — hiding is the slots' job (`resolve`). Pins raw pointers;
    /// every use is alive-checked first.
    fn bind_values(&self) {
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
        let t0 = std::time::Instant::now();
        // SAFETY: attached tick thread; enumeration consumed now.
        unsafe {
            for obj in crate::unity::find_tmp_texts(api, &cache) {
                // Dead-object check BEFORE any touch: a null
                // m_CachedPtr means reads AND writes crash.
                // SAFETY: m_CachedPtr probe copies one pointer.
                match crate::unity::object_alive(api, &cache, obj) {
                    Some(true) => {}
                    _ => continue,
                }
                let go = match crate::unity::component_gameobject(api, &cache, obj) {
                    Some(g) => g,
                    None => continue,
                };
                let name = crate::unity::object_name(api, &cache, go);
                if name == "Text (TMP) Speed" {
                    *self.speed_target.lock().unwrap() = Some(obj as usize);
                    let mut source = self.source.lock().unwrap();
                    if *source != name {
                        crate::log_line(&format!("speedo: speed source '{name}'"));
                        *source = name;
                    }
                } else if name == "Text (TMP) Gear" {
                    *self.gear_target.lock().unwrap() = Some(obj as usize);
                } else if name.to_lowercase().contains("speed")
                    && self.speed_target.lock().unwrap().is_none()
                {
                    // Tolerant fallback for renamed builds.
                    *self.speed_target.lock().unwrap() = Some(obj as usize);
                    let mut source = self.source.lock().unwrap();
                    if *source != name {
                        crate::log_line(&format!("speedo: speed source '{name}' (fallback)"));
                        *source = name;
                    }
                }
            }
        }
        // Slow-pass tripwire (diagnostic, not spam).
        let ms = t0.elapsed().as_millis();
        if ms > 500 {
            crate::log_line(&format!("speedo: slow bind pass ({ms} ms)"));
        }
    }

    /// Locate one slot's transform, no writes: `GameObject.Find(root)`,
    /// `Transform` of it, then `Transform.Find(child)` or itself.
    /// Returns the transform address, or `None` (missing API, missing
    /// object, dead wrapper — never throws, never writes).
    ///
    /// # Safety
    /// Attached tick thread; handles consumed immediately.
    unsafe fn locate(
        api: &crate::il2cpp::Il2cppApi,
        cache: &crate::unity::UnityCache,
        target: HideTarget,
    ) -> Option<usize> {
        // SAFETY: attached tick thread; each step null-checked, the
        // wrapper alive-checked before use.
        unsafe {
            let go = crate::unity::go_find(api, cache, target.root)?;
            if go.is_null() {
                return None;
            }
            let root_tr = crate::unity::go_transform(api, cache, go)?;
            let tr = match target.child {
                Some(c) => crate::unity::tr_find(api, cache, root_tr, c)?,
                None => root_tr,
            };
            if tr.is_null() {
                return None;
            }
            match crate::unity::object_alive(api, cache, tr) {
                Some(true) => Some(tr as usize),
                _ => None,
            }
        }
    }

    /// Resolve pass (tick thread): bind every checked-but-unresolved
    /// slot, record its original scale (never a zero one), hide it.
    /// At most one Find chain per slot per pass; missing targets are
    /// only retried on the resolve cadence. Logs one summary per arming.
    fn resolve(&self) {
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
        // Snapshot the work list, then drop the guard: no Mutex held
        // across any Unity call below.
        let work: Vec<(usize, HideTarget)> = {
            let slots = self.slots.lock().unwrap();
            slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.addr.is_none() && self.group_on(s.target.group))
                .map(|(i, s)| (i, s.target))
                .collect()
        };
        if work.is_empty() {
            return;
        }
        let t0 = std::time::Instant::now();
        let mut found = 0usize;
        let mut missing: Vec<String> = Vec::new();
        // SAFETY: attached tick thread; each located handle consumed now.
        unsafe {
            for (idx, target) in work {
                let path = match target.child {
                    Some(c) => format!("{}/{}", target.root, c),
                    None => target.root.to_string(),
                };
                let tr = match Self::locate(api, &cache, target) {
                    Some(t) => t as *mut std::ffi::c_void,
                    None => {
                        missing.push(path);
                        continue;
                    }
                };
                let live = match crate::unity::tr_get_scale(api, &cache, tr) {
                    Some(s) => s,
                    None => {
                        missing.push(path);
                        continue;
                    }
                };
                let orig = if is_zero_scale(live) { FALLBACK_SCALE } else { live };
                let mut wrote = false;
                if !is_zero_scale(live)
                    && crate::unity::tr_set_scale(api, &cache, tr, ZERO_SCALE)
                {
                    let back = crate::unity::tr_get_scale(api, &cache, tr);
                    crate::log_line(&format!(
                        "speedo: '{path}' hidden (readback={back:?})"
                    ));
                    wrote = true;
                }
                // Brief merge: plain data only, no Unity calls.
                {
                    let mut slots = self.slots.lock().unwrap();
                    if let Some(slot) = slots.get_mut(idx) {
                        // Re-check the group: the user may have unchecked
                        // mid-pass; never hide for a deselected group.
                        if self.group_on(slot.target.group) {
                            slot.addr = Some(tr as usize);
                            slot.orig = Some(orig);
                            if wrote {
                                found += 1;
                            }
                        }
                    }
                }
            }
        }
        // One-shot summary per arming (checked targets only).
        if !self.armed_logged.swap(true, Ordering::SeqCst) {
            let total: usize = {
                let slots = self.slots.lock().unwrap();
                slots.iter().filter(|s| self.group_on(s.target.group)).count()
            };
            // Count bound slots (resolved now or earlier this arming).
            let bound: usize = {
                let slots = self.slots.lock().unwrap();
                slots
                    .iter()
                    .filter(|s| self.group_on(s.target.group) && s.addr.is_some())
                    .count()
            };
            let _ = found;
            let mut msg = format!("speedo: {bound}/{total} cibles trouvées");
            if !missing.is_empty() {
                msg.push_str(&format!(" (manquantes : {})", missing.join(", ")));
            }
            crate::log_line(&msg);
        }
        // Slow-pass tripwire (diagnostic, not spam).
        let ms = t0.elapsed().as_millis();
        if ms > 500 {
            crate::log_line(&format!("speedo: slow resolve pass ({ms} ms)"));
        }
    }

    /// Maintain pass (~1 Hz, tick thread): for resolved slots, re-hide
    /// only on transition (game restored the scale), or restore the
    /// original when the group was unchecked. No blind rewrites.
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
        // Snapshot (index, wanted-hidden?) then drop the guard.
        let work: Vec<(usize, bool)> = {
            let slots = self.slots.lock().unwrap();
            slots
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    if s.addr.is_none() || s.orig.is_none() {
                        return None;
                    }
                    Some((i, self.group_on(s.target.group)))
                })
                .collect()
        };
        // SAFETY: attached tick thread; handles re-validated per touch.
        unsafe {
            for (idx, want_hidden) in work {
                // Re-read addr+orig under a brief lock (may have been
                // cleared by restore/disable meanwhile).
                let (addr, orig) = {
                    let slots = self.slots.lock().unwrap();
                    match slots.get(idx) {
                        Some(s) => match (s.addr, s.orig) {
                            (Some(a), Some(o)) => (a, o),
                            _ => continue,
                        },
                        None => continue,
                    }
                };
                let tr = addr as *mut std::ffi::c_void;
                // SAFETY: m_CachedPtr probe copies one pointer.
                match crate::unity::object_alive(api, &cache, tr) {
                    Some(true) => {}
                    _ => {
                        // Dead: unbind, discovery rebinds next pass.
                        let mut slots = self.slots.lock().unwrap();
                        if let Some(s) = slots.get_mut(idx) {
                            s.addr = None;
                        }
                        continue;
                    }
                }
                let path = {
                    let slots = self.slots.lock().unwrap();
                    match slots.get(idx) {
                        Some(s) => s.path(),
                        None => continue,
                    }
                };
                let current = match crate::unity::tr_get_scale(api, &cache, tr) {
                    Some(s) => s,
                    None => continue,
                };
                if want_hidden {
                    if !is_zero_scale(current)
                        && crate::unity::tr_set_scale(api, &cache, tr, ZERO_SCALE)
                    {
                        crate::log_line(&format!(
                            "speedo: '{path}' restored by game — hiding again"
                        ));
                    }
                } else if is_zero_scale(current)
                    && crate::unity::tr_set_scale(api, &cache, tr, orig)
                {
                    crate::log_line(&format!("speedo: '{path}' restored (group off)"));
                }
            }
        }
    }

    /// Restore all recorded originals (disable path): one fresh locate
    /// per slot, write-back only where our zero is still in place.
    /// Dead or missing objects are skipped silently.
    fn restore_all(&self) {
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
        // Snapshot (target, orig) pairs, then drop the guard.
        let work: Vec<(HideTarget, [f32; 3])> = {
            let slots = self.slots.lock().unwrap();
            slots
                .iter()
                .filter_map(|s| s.orig.map(|o| (s.target, o)))
                .collect()
        };
        if work.is_empty() {
            return;
        }
        let t0 = std::time::Instant::now();
        // SAFETY: attached tick thread (loader transitions never run on
        // the UI thread); each located handle consumed now.
        unsafe {
            for (target, orig) in work {
                let path = match target.child {
                    Some(c) => format!("{}/{}", target.root, c),
                    None => target.root.to_string(),
                };
                let tr = match Self::locate(api, &cache, target) {
                    Some(t) => t as *mut std::ffi::c_void,
                    None => continue,
                };
                let current = match crate::unity::tr_get_scale(api, &cache, tr) {
                    Some(s) => s,
                    None => continue,
                };
                if is_zero_scale(current)
                    && crate::unity::tr_set_scale(api, &cache, tr, orig)
                {
                    crate::log_line(&format!("speedo: '{path}' restored"));
                }
            }
        }
        let ms = t0.elapsed().as_millis();
        if ms > 500 {
            crate::log_line(&format!("speedo: slow restore pass ({ms} ms)"));
        }
        // Fresh start next arming (scene may have changed meanwhile).
        let mut slots = self.slots.lock().unwrap();
        for s in slots.iter_mut() {
            s.addr = None;
            s.orig = None;
            s.missing_logged = false;
        }
    }

    /// Fast value reads (tick thread, ~15 Hz): alive-checked reads on
    /// pinned TMP handles, published via atomics (`valid` gates the
    /// render). No enumeration here. A dead target unbinds; the bind
    /// pass rebinds next cycle.
    fn read_values(&self) {
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
        let speed_addr = *self.speed_target.lock().unwrap();
        let gear_addr = *self.gear_target.lock().unwrap();
        let mut speed_ok = false;
        // SAFETY: m_CachedPtr probes; reads only on proven-live wrappers.
        unsafe {
            if let Some(a) = speed_addr {
                let obj = a as *mut std::ffi::c_void;
                match crate::unity::object_alive(api, &cache, obj) {
                    Some(true) => {}
                    _ => {
                        *self.speed_target.lock().unwrap() = None;
                    }
                }
            }
            if let Some(a) = *self.speed_target.lock().unwrap() {
                let obj = a as *mut std::ffi::c_void;
                if let Some(text) = crate::unity::tmp_get_text(api, &cache, obj) {
                    if let Some(value) = parse_speed(&text) {
                        self.speed_bits.store(value.to_bits(), Ordering::SeqCst);
                        *self.raw.lock().unwrap() = text.clone();
                        let lower = text.to_lowercase();
                        *self.unit.lock().unwrap() = if lower.contains("mph") {
                            String::from("mph")
                        } else {
                            String::from("km/h")
                        };
                        speed_ok = true;
                    }
                } else {
                    *self.speed_target.lock().unwrap() = None;
                }
            }
            if let Some(a) = gear_addr {
                let obj = a as *mut std::ffi::c_void;
                match crate::unity::object_alive(api, &cache, obj) {
                    Some(true) => {}
                    _ => {
                        *self.gear_target.lock().unwrap() = None;
                    }
                }
            }
            if let Some(a) = *self.gear_target.lock().unwrap() {
                let obj = a as *mut std::ffi::c_void;
                if let Some(text) = crate::unity::tmp_get_text(api, &cache, obj) {
                    if let Some(g) = parse_gear(&text) {
                        self.gear_val.store(g, Ordering::SeqCst);
                    }
                } else {
                    *self.gear_target.lock().unwrap() = None;
                }
            }
        }
        self.valid.store(speed_ok, Ordering::SeqCst);
    }

    /// Restore exactly the components this mod turned off (disable
    /// path): re-enumerate live TMP texts, revive ours, skip the dead.
    /// (Legacy beh-flag restore, kept while any pre-scale-zero hide
    /// from an older build may still be in effect.)
    fn restore(&self) {
        // No beh-flag hides are created anymore (scale-zero replaced
        // them); originals live in the slots, restored by restore_all.
        self.restore_all();
    }
}

impl Default for SpeedoMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for SpeedoMod {
    fn name(&self) -> &'static str {
        "Speedometer"
    }

    fn mod_id(&self) -> &'static str {
        "speedo"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_speedo"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Gauge
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn is_pinned(&self) -> bool {
        self.tool.pin
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.is_enabled() {
            return;
        }
        let tick = self.tick.fetch_add(1, Ordering::SeqCst);
        // Arming (or Refresh) forces an immediate resolve + bind.
        if self.pending.swap(false, Ordering::SeqCst) {
            self.last_resolve.store(tick, Ordering::SeqCst);
            self.resolve();
            self.bind_values();
        }
        // Slow re-resolve while a checked target is missing.
        let missing = {
            let slots = self.slots.lock().unwrap();
            slots.iter().any(|s| {
                s.addr.is_none() && self.group_on(s.target.group)
            })
        };
        if missing && tick.saturating_sub(self.last_resolve.load(Ordering::SeqCst)) >= RESOLVE_EVERY_TICKS {
            self.last_resolve.store(tick, Ordering::SeqCst);
            self.resolve();
        }
        // Maintain (~1 Hz): re-hide on transition, restore unchecked.
        if tick % MAINTAIN_EVERY_TICKS == 0 {
            self.maintain();
        }
        // Value reads (~15 Hz) on pinned handles, published atomically.
        if tick % READ_EVERY_TICKS == 0 {
            self.read_values();
        }
        // Rebind value sources when lost (throttled like resolve).
        let unbound = self.speed_target.lock().unwrap().is_none()
            || self.gear_target.lock().unwrap().is_none();
        if unbound && tick.saturating_sub(self.last_resolve.load(Ordering::SeqCst)) >= RESOLVE_EVERY_TICKS {
            self.last_resolve.store(tick, Ordering::SeqCst);
            self.bind_values();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        // Proportional window: a fixed pixel box would dwarf small
        // screens or shrink on 4K; derive everything from height.
        let h = (ctx.screen_rect().height() * 0.42).clamp(260.0, 560.0);
        let w = h * 1.15;
        let win = egui::Window::new("Speedometer")
            .resizable(false)
            .fixed_size(egui::Vec2::new(w, h))
            .frame(self.tool.frame(ctx))
            .show(ctx, |ui| {
                self.tool.enter(ui);
                ui.horizontal(|ui| {
                    self.tool.pin_toggle(ui);
                    if ui.button("Refresh").clicked() {
                        // Force re-resolution on the tick (never Unity
                        // calls on the present thread).
                        {
                            let mut slots = self.slots.lock().unwrap();
                            for s in slots.iter_mut() {
                                s.addr = None;
                                s.missing_logged = false;
                            }
                        }
                        *self.speed_target.lock().unwrap() = None;
                        *self.gear_target.lock().unwrap() = None;
                        self.pending.store(true, Ordering::SeqCst);
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let mut gauge = self.gauge.load(Ordering::SeqCst);
                    if ui.checkbox(&mut gauge, "Gauge").changed() {
                        self.gauge.store(gauge, Ordering::SeqCst);
                    }
                    let mut speed = self.speed.load(Ordering::SeqCst);
                    if ui.checkbox(&mut speed, "Speed").changed() {
                        self.speed.store(speed, Ordering::SeqCst);
                    }
                    let mut gear = self.gear.load(Ordering::SeqCst);
                    if ui.checkbox(&mut gear, "Gear").changed() {
                        self.gear.store(gear, Ordering::SeqCst);
                    }
                    let mut nitro = self.nitro.load(Ordering::SeqCst);
                    if ui.checkbox(&mut nitro, "Nitro").changed() {
                        self.nitro.store(nitro, Ordering::SeqCst);
                    }
                });
                ui.separator();
                if self.valid.load(Ordering::SeqCst) {
                    // Exponential smoothing on the render clock so the
                    // ~15 Hz tick readout glides instead of stepping.
                    let dt = ctx.input(|i| i.stable_dt).min(0.1);
                    let target = f32::from_bits(self.speed_bits.load(Ordering::SeqCst));
                    let k = 1.0 - (-8.0f32 * dt).exp();
                    self.disp += (target - self.disp) * k;
                    let gear = self.gear_val.load(Ordering::SeqCst);
                    let gear_txt = match gear {
                        -1 => String::from("R"),
                        0 => String::from("N"),
                        v => v.to_string(),
                    };
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{:.0}", self.disp))
                                .size(h * 0.24)
                                .strong(),
                        );
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(self.unit.lock().unwrap().clone())
                                    .size(h * 0.075)
                                    .weak(),
                            );
                            ui.label(
                                egui::RichText::new(gear_txt)
                                    .size(h * 0.075)
                                    .strong(),
                            );
                        });
                        ui.separator();
                        ui.weak(format!("game: {}", self.raw.lock().unwrap().clone()));
                    });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.weak("en attente du jeu… (menu, garage, changement de scène)");
                    });
                }
            });
        if let Some(r) = win {
            self.tool.context_menu(&r.response);
        }
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        self.armed_logged.store(false, Ordering::SeqCst);
        crate::log_line("speedo: armed (scale-zero hide per group)");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore();
        self.restore_all();
        *self.speed_target.lock().unwrap() = None;
        *self.gear_target.lock().unwrap() = None;
        self.valid.store(false, Ordering::SeqCst);
        crate::log_line("speedo: off (originals restored)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(SpeedoMod::new()));
}

#[cfg(test)]
mod tests {
    use super::{group_enabled, is_zero_scale, parse_gear, parse_speed};

    #[test]
    fn parses_plain_integer() {
        assert_eq!(parse_speed("123"), Some(123.0));
    }

    #[test]
    fn parses_with_unit_suffix() {
        assert_eq!(parse_speed("87 km/h"), Some(87.0));
    }

    #[test]
    fn parses_mph_suffix() {
        assert_eq!(parse_speed("45 mph"), Some(45.0));
    }

    #[test]
    fn parses_decimal_dot_and_comma() {
        assert_eq!(parse_speed(" 92.5 "), Some(92.5));
        assert_eq!(parse_speed("92,5"), Some(92.5));
        assert_eq!(parse_speed("12,5"), Some(12.5));
    }

    #[test]
    fn rejects_non_numeric() {
        assert_eq!(parse_speed(""), None);
        assert_eq!(parse_speed("km/h"), None);
        assert_eq!(parse_speed("--"), None);
    }

    #[test]
    fn parses_gears() {
        assert_eq!(parse_gear("1"), Some(1));
        assert_eq!(parse_gear(" 3 "), Some(3));
        assert_eq!(parse_gear("N"), Some(0));
        assert_eq!(parse_gear("n"), Some(0));
        assert_eq!(parse_gear("R"), Some(-1));
        assert_eq!(parse_gear("r"), Some(-1));
    }

    #[test]
    fn rejects_bad_gears() {
        assert_eq!(parse_gear(""), None);
        assert_eq!(parse_gear("D"), None);
        assert_eq!(parse_gear("--"), None);
    }

    #[test]
    fn zero_scale_checks() {
        assert!(is_zero_scale([0.0, 0.0, 0.0]));
        assert!(is_zero_scale([1e-7, 0.0, -1e-7]));
        assert!(!is_zero_scale([1.0, 1.0, 1.0]));
        assert!(!is_zero_scale([0.0, 1.0, 0.0]));
    }

    #[test]
    fn group_flags() {
        assert!(group_enabled(0, true, false, false, false));
        assert!(!group_enabled(0, false, true, true, true));
        assert!(group_enabled(1, false, true, false, false));
        assert!(group_enabled(2, false, false, true, false));
        assert!(group_enabled(3, false, false, false, true));
        assert!(!group_enabled(9, true, true, true, true));
    }
}
