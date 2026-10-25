//! Engine-audio tick-side work: find the FMOD system, catalogue the
//! events, drive the live engine instances.
//!
//! Runs on the framework tick thread. Only FMOD Studio calls (handles
//! validated by the library itself) plus ONE bounded IL2CPP read to
//! obtain the system handle — never `FindObjectsOfType`, never a write
//! to a Unity object. Pitch/volume are *multipliers on the instance's
//! user values*: originals are recorded before the first write and put
//! back on neutral / disable.

use super::fmod::{Fmod, Handle};
use super::model::{ParamRow, Settings, Status, matches_filter};
use crate::mods::common::lock;
use crate::unity::UnityCache;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Retry cadences (ticks at ~60 Hz).
const LOAD_EVERY: u64 = 120;
const DISCOVER_EVERY: u64 = 180;
const VALID_EVERY: u64 = 60;
const CATALOG_EVERY: u64 = 300;
const POLL_EVERY: u64 = 6;
const STATUS_EVERY: u64 = 30;
/// Safety cap on catalogued events (a corrupt bank list must not eat RAM).
const MAX_CATALOG: usize = 50_000;

/// Per-instance bookkeeping: what the game had, what we last wrote.
struct Touched {
    orig_pitch: f32,
    orig_volume: f32,
    applied: (f32, f32),
}

#[derive(Default)]
pub(crate) struct Runtime {
    fmod: Option<Fmod>,
    unity: Option<UnityCache>,
    /// `Studio.System` handle (0 = not found). A plain integer: FMOD
    /// validates it on every call, a stale one only returns an error.
    sys: usize,
    tick: u64,
    last_load: u64,
    last_discover: u64,
    last_catalog: u64,
    /// `(EventDescription, lowercase path)` of every loaded event.
    catalog: Vec<(usize, String)>,
    bank_count: usize,
    /// Catalog indexes matching `matched_for`.
    matched: Vec<usize>,
    matched_for: Option<String>,
    /// Live `(instance, catalog index)` from the last poll.
    live: Vec<(usize, usize)>,
    touched: HashMap<usize, Touched>,
    logged_fields: bool,
}

fn h(v: usize) -> Handle {
    v as Handle
}

impl Runtime {
    /// One tick (~60 Hz). Cheap when idle; FMOD traffic is bounded to
    /// the catalog (rare) and the matched events (10 Hz).
    pub(crate) fn tick(&mut self, s: &Settings, status: &Mutex<Status>) {
        self.tick = self.tick.saturating_add(1);
        if self.fmod.is_none() {
            if self.due(self.last_load, LOAD_EVERY) {
                self.last_load = self.tick;
                self.fmod = Fmod::load();
                if self.fmod.is_some() {
                    crate::log_line("engine_audio: libfmodstudio bound");
                }
            }
            if self.fmod.is_none() {
                self.publish(status, "waiting for libfmodstudio.so");
                return;
            }
        }
        if self.sys == 0 {
            if self.due(self.last_discover, DISCOVER_EVERY) {
                self.last_discover = self.tick;
                self.sys = self.discover_system().unwrap_or(0);
            }
            if self.sys == 0 {
                self.publish(status, "FMOD system not found yet (see log)");
                return;
            }
        } else if self.tick % VALID_EVERY == 0 && !self.sys_valid() {
            crate::log_line("engine_audio: FMOD system gone, dropping state");
            self.reset();
            return;
        }
        let empty_retry = self.catalog.is_empty() && self.due(self.last_catalog, CATALOG_EVERY / 2);
        if empty_retry || self.tick.saturating_sub(self.last_catalog) >= CATALOG_EVERY {
            self.last_catalog = self.tick;
            self.refresh_catalog();
        }
        if self.tick % POLL_EVERY == 0 {
            self.poll_and_apply(s);
        }
        self.apply_overrides(s);
        if self.tick % STATUS_EVERY == 0 {
            self.publish_full(status);
        }
    }

    fn due(&self, last: u64, every: u64) -> bool {
        last == 0 || self.tick.saturating_sub(last) >= every
    }

    fn sys_valid(&self) -> bool {
        self.fmod.as_ref().is_some_and(|f| f.system_valid(h(self.sys)))
    }

    fn reset(&mut self) {
        self.sys = 0;
        self.catalog.clear();
        self.bank_count = 0;
        self.matched.clear();
        self.matched_for = None;
        self.live.clear();
        self.touched.clear();
    }

    /// `FMODUnity.RuntimeManager` singleton → its `FMOD.Studio.System`
    /// field (value type wrapping one `IntPtr`). Field found by TYPE,
    /// not by name (private names move between integration versions).
    fn discover_system(&mut self) -> Option<usize> {
        let api = crate::il2cpp_api()?;
        // SAFETY: attached tick thread; every step null-checked and the
        // object is alive-checked before its fields are read.
        unsafe {
            let domain = crate::il2cpp::domain_checked(api)?;
            if self.unity.is_none() {
                self.unity = crate::unity::init(api, domain);
            }
            let cache = self.unity.as_ref()?;
            let klass = crate::il2cpp::find_class(api, domain, None, "FMODUnity", "RuntimeManager")?;
            let get_instance = crate::il2cpp::get_method(api, klass, "get_Instance", 0)?;
            let obj = crate::il2cpp::invoke(api, get_instance, std::ptr::null_mut(), &[])?;
            if crate::unity::object_alive(api, cache, obj) != Some(true) {
                return None;
            }
            let fields = crate::il2cpp::class_fields(api, klass);
            let mut found = None;
            for f in fields.iter().filter(|f| !f.is_static && !f.type_class.is_null()) {
                let name = crate::il2cpp::class_name(api, f.type_class).unwrap_or_default();
                let ns = crate::il2cpp::class_namespace(api, f.type_class).unwrap_or_default();
                if name == "System" && ns == "FMOD.Studio" {
                    found = Some((f.field, f.type_class));
                    break;
                }
            }
            let Some((field, tclass)) = found else {
                if !self.logged_fields {
                    self.logged_fields = true;
                    let list: Vec<String> = fields
                        .iter()
                        .map(|f| {
                            let t = crate::il2cpp::class_name(api, f.type_class).unwrap_or_default();
                            format!("{}:{}", f.name, t)
                        })
                        .collect();
                    crate::log_line(&format!(
                        "engine_audio: no FMOD.Studio.System field on RuntimeManager; fields = {}",
                        list.join(", ")
                    ));
                }
                return None;
            };
            // Struct payload copied into a roomy buffer; only a struct
            // that is exactly one pointer wide is a bare handle.
            let size = crate::il2cpp::class_value_size(api, tclass)?;
            if size != std::mem::size_of::<usize>() {
                crate::log_line(&format!("engine_audio: FMOD.Studio.System is {size} bytes, expected 8"));
                return None;
            }
            let mut buf = [0u8; 64];
            if !crate::il2cpp::field_get_raw(api, obj, field, &mut buf) {
                return None;
            }
            let handle = usize::from_ne_bytes(buf[..8].try_into().ok()?);
            if handle == 0 || !self.fmod.as_ref()?.system_valid(h(handle)) {
                return None;
            }
            crate::log_line(&format!("engine_audio: FMOD studio system bound ({handle:#x})"));
            Some(handle)
        }
    }

    /// (Re)build the event catalog when the bank set changed or it is empty.
    fn refresh_catalog(&mut self) {
        let Some(f) = self.fmod.as_ref() else { return };
        let banks = f.banks(h(self.sys));
        if banks.len() == self.bank_count && !self.catalog.is_empty() {
            return;
        }
        let mut cat = Vec::new();
        'banks: for b in &banks {
            for d in f.bank_events(*b) {
                if cat.len() >= MAX_CATALOG {
                    break 'banks;
                }
                if let Some(p) = f.event_path(d) {
                    cat.push((d as usize, p.to_lowercase()));
                }
            }
        }
        crate::log_line(&format!("engine_audio: catalog {} events in {} banks", cat.len(), banks.len()));
        // An empty result (strings bank not loaded yet) is retried.
        self.bank_count = if cat.is_empty() { 0 } else { banks.len() };
        self.catalog = cat;
        self.matched_for = None;
    }

    fn ensure_matched(&mut self, filter: &str) {
        if self.matched_for.as_deref() == Some(filter) {
            return;
        }
        self.matched = (0..self.catalog.len())
            .filter(|&i| matches_filter(&self.catalog[i].1, filter))
            .collect();
        self.matched_for = Some(filter.to_string());
        crate::log_line(&format!("engine_audio: filter \"{}\" matches {} events", filter, self.matched.len()));
    }

    fn poll_and_apply(&mut self, s: &Settings) {
        self.ensure_matched(&s.filter);
        let Some(f) = self.fmod.as_ref() else { return };
        let mut live = Vec::new();
        for &ci in &self.matched {
            let desc = h(self.catalog[ci].0);
            if f.instance_count(desc) == 0 {
                continue;
            }
            for inst in f.instances(desc) {
                live.push((inst as usize, ci));
            }
        }
        let alive: HashSet<usize> = live.iter().map(|&(i, _)| i).collect();
        // Gone instances need no restore (their handle is dead).
        self.touched.retain(|k, _| alive.contains(k));
        self.live = live;
        let (tp, tv) = s.targets();
        if s.is_neutral() {
            self.restore();
            return;
        }
        let Some(f) = self.fmod.as_ref() else { return };
        for &(inst, _) in &self.live {
            let t = self.touched.entry(inst).or_insert_with(|| Touched {
                orig_pitch: f.pitch(h(inst)).unwrap_or(1.0),
                orig_volume: f.volume(h(inst)).unwrap_or(1.0),
                applied: (f32::NAN, f32::NAN),
            });
            let want = (t.orig_pitch * tp, t.orig_volume * tv);
            if t.applied != want {
                let ok = f.set_pitch(h(inst), want.0) & f.set_volume(h(inst), want.1);
                if ok {
                    t.applied = want;
                }
            }
        }
    }

    /// Experimental: pin game parameters on every live instance.
    fn apply_overrides(&self, s: &Settings) {
        let Some(f) = self.fmod.as_ref() else { return };
        for o in s.overrides.iter().filter(|o| o.on && o.value.is_finite()) {
            for &(inst, _) in &self.live {
                f.set_param(h(inst), &o.name, o.value);
            }
        }
    }

    /// Put every touched instance back to the values the game had.
    pub(crate) fn restore(&mut self) {
        if let Some(f) = self.fmod.as_ref() {
            for (inst, t) in self.touched.iter() {
                f.set_pitch(h(*inst), t.orig_pitch);
                f.set_volume(h(*inst), t.orig_volume);
            }
        }
        self.touched.clear();
    }

    fn publish(&self, status: &Mutex<Status>, note: &str) {
        if self.tick % STATUS_EVERY != 0 {
            return;
        }
        let mut st = lock(status);
        *st = Status { note: note.to_string(), ..Status::default() };
    }

    fn publish_full(&self, status: &Mutex<Status>) {
        let Some(f) = self.fmod.as_ref() else { return };
        let mut matched = Vec::new();
        for &ci in self.matched.iter().take(64) {
            let n = f.instance_count(h(self.catalog[ci].0));
            matched.push((self.catalog[ci].1.clone(), n));
        }
        matched.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let params = match self.live.first() {
            Some(&(inst, ci)) => f
                .params(h(self.catalog[ci].0))
                .into_iter()
                .map(|p| {
                    let (value, final_value) = f.param(h(inst), &p.name).unwrap_or((p.default, p.default));
                    ParamRow { name: p.name, min: p.min, max: p.max, value, final_value }
                })
                .collect(),
            None => Vec::new(),
        };
        let note = if self.catalog.is_empty() {
            "no events yet (banks loading?)"
        } else if self.matched.is_empty() {
            "no event matches the filter"
        } else if self.live.is_empty() {
            "no live engine event (get in a car)"
        } else {
            "driving live engine events"
        };
        *lock(status) = Status {
            note: note.to_string(),
            banks: self.bank_count,
            events: self.catalog.len(),
            matched,
            instances: self.live.len(),
            params,
        };
    }
}
