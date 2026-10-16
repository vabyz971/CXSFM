//! Live game values (phase 3), tick side.
//!
//! RULE EXCEPTION (user-approved 2026-10-16, see AGENTS.md): the root
//! instance (e.g. `UITachometer`) is KEPT between reads instead of
//! being re-found each time — re-finding means `FindObjectsOfType`
//! (managed call + allocation) at read rate, the profile that crashed
//! the old gauge after ~1 min. Bounds on the exception:
//! - re-found every [`REFIND_TICKS`] (~2 s) anyway, and at most that
//!   often while missing (menus cost one call per 2 s, not per read);
//! - before EVERY read: root header readable + `m_CachedPtr` alive;
//!   every reference hop checked readable (+ alive for Unity objects);
//! - reads are raw field copies (`il2cpp_field_get_value`): no getter,
//!   no managed code, no allocation. Worst case on a recycled address:
//!   wrong values for < 2 s, never a write, never an unmapped read.

use super::bind::is_unity_object;
use super::model::{SourceDef, Values, decode_number, primitive_size, te};
use crate::il2cpp::{self, Il2cppApi};
use crate::unity::UnityCache;
use std::collections::HashMap;

/// Root re-find cadence (ticks, ~2 s at 60 Hz).
pub(crate) const REFIND_TICKS: u64 = 120;
/// Class lookup retry cadence when a root class is unknown (~10 s).
const CLASS_RETRY_TICKS: u64 = 600;

type Ptr = *mut std::ffi::c_void;

#[derive(Default)]
struct RootSlot {
    /// Class pointer (process-stable metadata), 0 = not found yet.
    klass: usize,
    class_tried: u64,
    /// Live instance address (the bounded exception), 0 = none.
    inst: usize,
    found: u64,
    tried: u64,
}

/// Resolved instance field (class metadata, process-stable).
#[derive(Clone, Copy)]
struct FieldSlot {
    field: usize,
    te: i32,
    size: usize,
    reference: bool,
}

/// Tick-only reader state (behind a mutex for `Sync`, never contended).
#[derive(Default)]
pub(crate) struct LiveReader {
    roots: HashMap<String, RootSlot>,
    fields: HashMap<(usize, String), Option<FieldSlot>>,
    unity_class: HashMap<usize, bool>,
}

/// What the control window shows about live reading.
#[derive(Clone, Default)]
pub(crate) struct LiveStatus {
    /// `(root, instance found)`.
    pub(crate) roots: Vec<(String, bool)>,
    /// `(source, reason)` for sources that produced no value.
    pub(crate) errors: Vec<(String, String)>,
}

impl LiveReader {
    /// Exact short-name class lookup across loaded images (cached).
    unsafe fn find_root_class(api: &Il2cppApi, name: &str) -> Option<Ptr> {
        // SAFETY: attached tick thread; metadata enumeration only.
        unsafe {
            let domain = il2cpp::domain_checked(api)?;
            for img in il2cpp::all_images(api, domain) {
                let n = il2cpp::image_class_count(api, img).unwrap_or(0).min(200_000);
                for i in 0..n {
                    let Some(k) = il2cpp::image_class(api, img, i) else {
                        continue;
                    };
                    if il2cpp::class_name(api, k).as_deref() == Some(name) {
                        return Some(k);
                    }
                }
            }
        }
        None
    }

    /// Keep/refresh the root instance under the bounded-exception rules.
    unsafe fn root(&mut self, api: &Il2cppApi, cache: &UnityCache, name: &str, tick: u64) -> Option<Ptr> {
        let slot = self.roots.entry(name.to_string()).or_default();
        // SAFETY: attached tick thread; see module docs for the rules.
        unsafe {
            if slot.klass == 0 {
                if slot.class_tried != 0 && tick.saturating_sub(slot.class_tried) < CLASS_RETRY_TICKS {
                    return None;
                }
                slot.class_tried = tick.max(1);
                slot.klass = Self::find_root_class(api, name).map(|k| k as usize).unwrap_or(0);
                if slot.klass == 0 {
                    return None;
                }
            }
            let due = slot.inst == 0 || tick.saturating_sub(slot.found) >= REFIND_TICKS;
            if due && (slot.tried == 0 || tick.saturating_sub(slot.tried) >= REFIND_TICKS) {
                slot.tried = tick.max(1);
                let objs = crate::unity::find_objects_of_class(api, cache, slot.klass as Ptr);
                let alive: Vec<Ptr> = objs
                    .into_iter()
                    .filter(|&o| crate::unity::object_alive(api, cache, o) == Some(true))
                    .collect();
                // Prefer the instance on an active branch (a garage
                // preview may hold a second, hidden tachometer).
                let pick = alive
                    .iter()
                    .copied()
                    .find(|&o| crate::unity::component_active(api, cache, o) == Some(true))
                    .or_else(|| alive.first().copied());
                slot.inst = pick.map(|p| p as usize).unwrap_or(0);
                slot.found = tick;
            }
            if slot.inst == 0 {
                return None;
            }
            let p = slot.inst as Ptr;
            if !crate::memory::ptr_readable(slot.inst, 16) || crate::unity::object_alive(api, cache, p) != Some(true) {
                // Gone: next re-find is still rate-limited by `tried`.
                slot.inst = 0;
                return None;
            }
            Some(p)
        }
    }

    unsafe fn field(&mut self, api: &Il2cppApi, cls: Ptr, name: &str) -> Option<FieldSlot> {
        let key = (cls as usize, name.to_string());
        if let Some(cached) = self.fields.get(&key) {
            return *cached;
        }
        // SAFETY: caller contract; metadata reads only.
        let slot = unsafe {
            il2cpp::find_field(api, cls, name)
                .and_then(|f| il2cpp::field_info(api, f))
                .filter(|d| !d.is_static)
                .and_then(|d| {
                    let vt_generic = d.type_enum == te::GENERICINST
                        && il2cpp::class_is_valuetype(api, d.type_class) == Some(true);
                    let reference = matches!(d.type_enum, te::CLASS | te::OBJECT)
                        || (d.type_enum == te::GENERICINST && !vt_generic);
                    let size = if reference {
                        Some(8)
                    } else if d.type_enum == te::VALUETYPE {
                        il2cpp::class_value_size(api, d.type_class)
                    } else {
                        primitive_size(d.type_enum)
                    }?;
                    (size <= 8).then_some(FieldSlot { field: d.field as usize, te: d.type_enum, size, reference })
                })
        };
        self.fields.insert(key, slot);
        slot
    }

    unsafe fn is_unity(&mut self, api: &Il2cppApi, cls: Ptr) -> bool {
        // SAFETY: caller contract.
        *self.unity_class.entry(cls as usize).or_insert_with(|| unsafe { is_unity_object(api, cls) })
    }

    /// One source: hop the reference chain, read the value field.
    unsafe fn read_source(
        &mut self,
        api: &Il2cppApi,
        cache: &UnityCache,
        root: Ptr,
        s: &SourceDef,
    ) -> Result<f32, String> {
        let mut obj = root;
        // SAFETY: every hop checked readable (+ alive when Unity).
        unsafe {
            for seg in &s.refs {
                let cls = il2cpp::object_class(api, obj).ok_or_else(|| format!("objet illisible avant `{seg}`"))?;
                let f = self
                    .field(api, cls, seg)
                    .filter(|f| f.reference)
                    .ok_or_else(|| format!("`{seg}` : pas une référence lisible"))?;
                let mut buf = [0u8; 8];
                if !il2cpp::field_get_raw(api, obj, f.field as Ptr, &mut buf) {
                    return Err(format!("lecture de `{seg}` impossible"));
                }
                let next = usize::from_le_bytes(buf) as Ptr;
                if next.is_null() {
                    return Err(format!("`{seg}` est null"));
                }
                let next_cls = il2cpp::object_class(api, next).ok_or_else(|| format!("`{seg}` illisible"))?;
                if self.is_unity(api, next_cls) && crate::unity::object_alive(api, cache, next) != Some(true) {
                    return Err(format!("`{seg}` : objet Unity détruit"));
                }
                obj = next;
            }
            let cls = il2cpp::object_class(api, obj).ok_or("objet illisible")?;
            let f = self
                .field(api, cls, &s.field)
                .filter(|f| !f.reference)
                .ok_or_else(|| format!("`{}` : champ numérique introuvable", s.field))?;
            let mut buf = [0u8; 8];
            if !il2cpp::field_get_raw(api, obj, f.field as Ptr, &mut buf) {
                return Err(format!("lecture de `{}` impossible", s.field));
            }
            let raw = decode_number(f.te, &buf[..f.size]).ok_or_else(|| format!("`{}` : type non numérique", s.field))?;
            Ok(raw * s.scale + s.offset)
        }
    }

    /// Read every source once (tick thread). Missing roots/sources are
    /// simply absent from the returned values.
    ///
    /// # Safety
    /// Attached tick thread, never during teardown (the tick skips mod
    /// updates then).
    pub(crate) unsafe fn read_all(
        &mut self,
        api: &Il2cppApi,
        cache: &UnityCache,
        sources: &[SourceDef],
        tick: u64,
    ) -> (Values, LiveStatus) {
        let mut values = Values::default();
        let mut status = LiveStatus::default();
        let mut live_roots: Vec<(String, Option<Ptr>)> = Vec::new();
        // SAFETY: forwarded caller contract.
        unsafe {
            for s in sources {
                if !live_roots.iter().any(|(n, _)| n == &s.root) {
                    let r = self.root(api, cache, &s.root, tick);
                    status.roots.push((s.root.clone(), r.is_some()));
                    live_roots.push((s.root.clone(), r));
                }
                let Some(root) = live_roots.iter().find(|(n, _)| n == &s.root).and_then(|(_, r)| *r) else {
                    continue;
                };
                match self.read_source(api, cache, root, s) {
                    Ok(v) => values.set(&s.name, v),
                    Err(e) => status.errors.push((s.name.clone(), e)),
                }
            }
        }
        (values, status)
    }
}
