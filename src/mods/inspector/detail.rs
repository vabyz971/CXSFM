//! Components + fields of the selected GameObject, tick side.
//!
//! On demand only (selection, Reread, edit, optional 1 Hz live view).
//! Every request starts again from the node's transform: alive-probed
//! (`m_CachedPtr`), identity checked against the node's instance id,
//! `GetComponents(Component)` re-run, then the path of reference
//! fields / array indices walked again with each hop checked readable
//! (and alive for Unity objects). No object pointer survives a
//! request (stored-pointer rule).
//!
//! Reads are raw field copies (`il2cpp_field_get_value`): no getter
//! runs. Writes are raw stores (`il2cpp_field_set_value`) of a payload
//! whose size equals the field size, into the object found by that
//! fresh walk; `Behaviour.enabled` goes through its setter
//! (`invoke_void`, already used by the camera takeover).

use super::InspectorMod;
use super::model::{
    CompSummary, Detail, Edit, MAX_COMPONENTS, MAX_METHODS, MAX_ROWS, ObjView, Req, Row, Seg, Target,
    decode_int, float_count,
};
use crate::il2cpp::{self, FieldDesc, Il2cppApi};
use crate::mods::common::lock;
use crate::mods::custom_hud::model::{decode_primitive, decode_struct, is_reference, primitive_name, primitive_size, te};
use crate::unity::UnityCache;

type Ptr = *mut std::ffi::c_void;

/// Value-type payloads larger than this are shown as hex, not read in full.
const MAX_VALUE_SIZE: usize = 64;
/// Displayed string cap (chars).
const MAX_STR: usize = 80;

/// `GameObject.GetComponents(Type)` + `UnityEngine.Component`, resolved once.
#[derive(Clone, Copy)]
pub(crate) struct CompApi {
    get_components: usize,
    component: usize,
}

/// Does `k` (or a parent) match `ns.name`?
unsafe fn derives_from(api: &Il2cppApi, mut k: Ptr, ns: &str, name: &str) -> bool {
    // SAFETY: caller contract; metadata reads, bounded climb.
    unsafe {
        for _ in 0..64 {
            if il2cpp::class_name(api, k).as_deref() == Some(name) && il2cpp::class_namespace(api, k).as_deref() == Some(ns) {
                return true;
            }
            match il2cpp::class_parent(api, k) {
                Some(p) => k = p,
                None => return false,
            }
        }
    }
    false
}

/// Engine base classes: their managed fields are noise.
fn is_base(ns: &str, name: &str) -> bool {
    (ns == "UnityEngine" && matches!(name, "MonoBehaviour" | "Behaviour" | "Component" | "Object"))
        || (ns == "System" && name == "Object")
}

/// `Namespace.Class`.
unsafe fn full_name(api: &Il2cppApi, k: Ptr) -> String {
    // SAFETY: caller contract; metadata reads.
    unsafe {
        let name = il2cpp::class_name(api, k).unwrap_or_else(|| "?".into());
        match il2cpp::class_namespace(api, k) {
            Some(ns) if !ns.is_empty() => format!("{ns}.{name}"),
            _ => name,
        }
    }
}

/// Instance fields of `k` and its parents (engine bases excluded),
/// most-derived first.
unsafe fn instance_fields(api: &Il2cppApi, mut k: Ptr) -> Vec<FieldDesc> {
    let mut out = Vec::new();
    // SAFETY: caller contract; bounded climb, metadata reads.
    unsafe {
        for _ in 0..32 {
            let ns = il2cpp::class_namespace(api, k).unwrap_or_default();
            let name = il2cpp::class_name(api, k).unwrap_or_default();
            if is_base(&ns, &name) {
                break;
            }
            out.extend(il2cpp::class_fields(api, k).into_iter().filter(|f| !f.is_static));
            match il2cpp::class_parent(api, k) {
                Some(p) => k = p,
                None => break,
            }
        }
    }
    out
}

/// `name(argc)` of the methods of `k` and its parents (bases excluded).
unsafe fn methods(api: &Il2cppApi, mut k: Ptr) -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: caller contract; metadata reads.
    unsafe {
        for _ in 0..32 {
            let ns = il2cpp::class_namespace(api, k).unwrap_or_default();
            let name = il2cpp::class_name(api, k).unwrap_or_default();
            if is_base(&ns, &name) || out.len() >= MAX_METHODS {
                break;
            }
            for m in il2cpp::class_methods(api, k) {
                if out.len() >= MAX_METHODS {
                    break;
                }
                let n = il2cpp::method_name(api, m).unwrap_or_default();
                let argc = il2cpp::method_param_count(api, m).unwrap_or(0);
                out.push(format!("{name}.{n}({argc})"));
            }
            match il2cpp::class_parent(api, k) {
                Some(p) => k = p,
                None => break,
            }
        }
    }
    out
}

/// Readable managed object (and alive, when it is a Unity object).
unsafe fn usable(api: &Il2cppApi, cache: &UnityCache, obj: Ptr) -> Option<Ptr> {
    // SAFETY: caller contract; header probed readable first.
    unsafe {
        let k = il2cpp::object_class(api, obj)?;
        if derives_from(api, k, "UnityEngine", "Object") && crate::unity::object_alive(api, cache, obj) != Some(true) {
            return None;
        }
        Some(k)
    }
}

/// Elements of a managed array: `(count, header size)`.
unsafe fn array_shape(api: &Il2cppApi, arr: Ptr) -> Option<(usize, usize)> {
    let (len, hdr) = (api.array_length?, api.array_object_header_size?);
    // SAFETY: caller checked the header readable.
    unsafe { Some((len(arr) as usize, hdr() as usize)) }
}

impl InspectorMod {
    /// Resolve `GetComponents(Type)` once (the 1-argument overload
    /// taking a `System.Type`, not the generic `List<T>` one).
    fn comp_api(&self, api: &Il2cppApi, cache: &UnityCache) -> Option<CompApi> {
        if let Some(c) = *lock(&self.comp_api) {
            return Some(c);
        }
        // SAFETY: attached tick thread; metadata lookups only.
        let c = unsafe {
            let domain = il2cpp::domain_checked(api)?;
            let component = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Component")?;
            let m = il2cpp::get_method_overload(api, cache.go_klass, "GetComponents", 1, te::CLASS)?;
            CompApi { get_components: m as usize, component: component as usize }
        };
        crate::log_line("inspector: GameObject.GetComponents(Type) resolved");
        *lock(&self.comp_api) = Some(c);
        Some(c)
    }

    /// Process one UI request (tick thread).
    pub(crate) fn detail_tick(&self, req: Req) {
        if !self.ensure_unity() {
            return;
        }
        let (Some(api), Some(cache)) = (crate::il2cpp_api(), lock(&self.unity).as_ref().copied()) else {
            return;
        };
        let target = match &req {
            Req::Load(t) | Req::Write { target: t, .. } | Req::Enable { target: t, .. } => t.clone(),
        };
        let generation = lock(&self.detail).as_ref().map_or(0, |d| d.generation) + 1;
        let mut detail = Detail { id: target.id, generation, ..Detail::default() };
        // SAFETY: attached tick thread; every pointer comes from this
        // fresh walk and is probed before use.
        unsafe {
            match self.components(api, &cache, &target) {
                Err(e) => detail.error = Some(e),
                Ok(comps) => {
                    match &req {
                        Req::Enable { on, .. } => {
                            if let Some(&c) = target.comp.and_then(|i| comps.get(i))
                                && !crate::unity::beh_set_enabled(api, &cache, c, *on)
                            {
                                detail.error = Some("enabled: the setter threw".into());
                            }
                        }
                        Req::Write { field, bytes, .. } => {
                            if let Err(e) = Self::write(api, &cache, &comps, &target, field, bytes) {
                                detail.error = Some(e);
                            }
                        }
                        Req::Load(_) => {}
                    }
                    detail.comps = comps.iter().map(|&c| Self::summary(api, &cache, c)).collect();
                    if let Some(i) = target.comp {
                        detail.view = Some(Self::view(api, &cache, &comps, i, &target.chain));
                    }
                }
            }
        }
        *lock(&self.detail) = Some(detail);
    }

    /// Components of the target GameObject (fresh `GetComponents`).
    unsafe fn components(&self, api: &Il2cppApi, cache: &UnityCache, t: &Target) -> Result<Vec<Ptr>, String> {
        let ca = self.comp_api(api, cache).ok_or("GetComponents(Type) not found in this build")?;
        // SAFETY: caller contract; transform probed alive, identity
        // checked, the array consumed right away.
        unsafe {
            let tr = t.tr as Ptr;
            if t.tr == 0 || crate::unity::object_alive(api, cache, tr) != Some(true) {
                return Err("object destroyed — Refresh the tree".into());
            }
            let go = crate::unity::component_gameobject(api, cache, tr).ok_or("GameObject unreadable")?;
            if crate::unity::get_instance_id(api, cache, go) != Some(t.id) {
                return Err("tree is stale (other object at this address) — Refresh".into());
            }
            let ty = il2cpp::type_object_for_class(api, ca.component as Ptr).ok_or("typeof(Component) failed")?;
            let params = [ty];
            let arr = il2cpp::invoke(api, ca.get_components as Ptr, go, &params).ok_or("GetComponents threw")?;
            Ok(crate::unity::read_object_array(api, arr, MAX_COMPONENTS))
        }
    }

    unsafe fn summary(api: &Il2cppApi, cache: &UnityCache, c: Ptr) -> CompSummary {
        // SAFETY: caller contract; component from a fresh array.
        unsafe {
            let k = il2cpp::object_class(api, c);
            let class = k.and_then(|k| il2cpp::class_name(api, k)).unwrap_or_else(|| "?".into());
            let namespace = k.and_then(|k| il2cpp::class_namespace(api, k)).unwrap_or_default();
            // `enabled` exists on Behaviour only: never invoke it on a
            // Transform/Renderer (no type check in runtime_invoke).
            let enabled = match k {
                Some(k) if derives_from(api, k, "UnityEngine", "Behaviour") => crate::unity::beh_get_enabled(api, cache, c),
                _ => None,
            };
            CompSummary { class, namespace, enabled }
        }
    }

    /// Walk `chain` from component `comp`: the object it ends on.
    unsafe fn follow(api: &Il2cppApi, cache: &UnityCache, comps: &[Ptr], comp: usize, chain: &[Seg]) -> Result<Ptr, String> {
        // SAFETY: caller contract; each hop probed readable/alive.
        unsafe {
            let mut obj = *comps.get(comp).ok_or("component gone — Reread")?;
            usable(api, cache, obj).ok_or("component destroyed")?;
            for seg in chain {
                let k = il2cpp::object_class(api, obj).ok_or("object unreadable")?;
                let next = match seg {
                    Seg::Field(name) => {
                        let f = instance_fields(api, k)
                            .into_iter()
                            .find(|f| &f.name == name)
                            .ok_or_else(|| format!("field `{name}` not found"))?;
                        let mut buf = [0u8; 8];
                        if !il2cpp::field_get_raw(api, obj, f.field, &mut buf) {
                            return Err(format!("`{name}` unreadable"));
                        }
                        usize::from_le_bytes(buf) as Ptr
                    }
                    Seg::Index(i) => {
                        let (n, hdr) = array_shape(api, obj).ok_or("not an array")?;
                        if *i >= n {
                            return Err(format!("[{i}] out of range ({n})"));
                        }
                        let slot = (obj as usize).checked_add(hdr + i * 8).ok_or("bad address")?;
                        if !crate::memory::ptr_readable(slot, 8) {
                            return Err("array unreadable".into());
                        }
                        *(slot as *const Ptr)
                    }
                };
                if next.is_null() {
                    return Err(format!("`{}` is null", seg.label()));
                }
                usable(api, cache, next).ok_or_else(|| format!("`{}` unreadable or destroyed", seg.label()))?;
                obj = next;
            }
            Ok(obj)
        }
    }

    /// Rows of the object at the end of the chain.
    unsafe fn view(api: &Il2cppApi, cache: &UnityCache, comps: &[Ptr], comp: usize, chain: &[Seg]) -> ObjView {
        let mut v = ObjView { comp, chain: chain.to_vec(), ..ObjView::default() };
        // SAFETY: caller contract.
        unsafe {
            let obj = match Self::follow(api, cache, comps, comp, chain) {
                Ok(o) => o,
                Err(e) => {
                    v.note = Some(e);
                    return v;
                }
            };
            let Some(k) = il2cpp::object_class(api, obj) else {
                v.note = Some("object unreadable".into());
                return v;
            };
            v.class = full_name(api, k);
            if v.class.ends_with("[]") {
                v.rows = Self::array_rows(api, obj, k);
                return v;
            }
            v.methods = methods(api, k);
            let fields = instance_fields(api, k);
            if fields.is_empty() {
                v.note = Some("No managed field: a native component (its values live in the engine, behind properties).".into());
            }
            v.rows = fields.iter().take(MAX_ROWS).map(|f| Self::row(api, obj, f)).collect();
        }
        v
    }

    /// One field: type, display value, edit kind, follow step.
    unsafe fn row(api: &Il2cppApi, obj: Ptr, f: &FieldDesc) -> Row {
        // SAFETY: caller contract; every copy bounded by the field size.
        unsafe {
            let class_name = il2cpp::class_name(api, f.type_class).unwrap_or_else(|| "?".into());
            let vt_generic = f.type_enum == te::GENERICINST && il2cpp::class_is_valuetype(api, f.type_class) == Some(true);
            let reference = is_reference(f.type_enum) || (f.type_enum == te::GENERICINST && !vt_generic);
            let type_name = primitive_name(f.type_enum).map(str::to_string).unwrap_or(class_name.clone());
            let mut row = Row { name: f.name.clone(), type_name, value: String::new(), edit: Edit::None, follow: None };
            if reference {
                let mut buf = [0u8; 8];
                if !il2cpp::field_get_raw(api, obj, f.field, &mut buf) {
                    row.value = "?".into();
                    return row;
                }
                let p = usize::from_le_bytes(buf) as Ptr;
                if p.is_null() {
                    row.value = "null".into();
                } else if f.type_enum == te::STRING {
                    let s: String = if crate::memory::ptr_readable(p as usize, 32) {
                        il2cpp::read_string(api, p).unwrap_or_default().chars().take(MAX_STR).collect()
                    } else {
                        "?".into()
                    };
                    row.value = format!("\"{s}\"");
                } else {
                    let target = il2cpp::object_class(api, p).map(|c| full_name(api, c)).unwrap_or_else(|| "?".into());
                    row.value = format!("→ {target}");
                    row.follow = Some(Seg::Field(f.name.clone()));
                }
                return row;
            }
            let is_enum = f.type_enum == te::VALUETYPE && il2cpp::class_is_enum(api, f.type_class) == Some(true);
            let size = match primitive_size(f.type_enum) {
                Some(n) => Some(n),
                None if f.type_enum == te::VALUETYPE || vt_generic => il2cpp::class_value_size(api, f.type_class),
                None => None,
            };
            let Some(size) = size.filter(|&n| n <= MAX_VALUE_SIZE) else {
                row.value = "…".into();
                return row;
            };
            let mut buf = [0u8; MAX_VALUE_SIZE];
            if !il2cpp::field_get_raw(api, obj, f.field, &mut buf) {
                row.value = "?".into();
                return row;
            }
            let b = &buf[..size];
            row.value = decode_primitive(f.type_enum, b).unwrap_or_else(|| decode_struct(&class_name, b));
            row.edit = match f.type_enum {
                te::BOOLEAN => Edit::Bool(b[0] != 0),
                te::R4 => Edit::Float { v: f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64, size: 4 },
                te::R8 => Edit::Float { v: f64::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])), size: 8 },
                te::I1 | te::I2 | te::I4 | te::I8 | te::I => {
                    decode_int(b, size, true).map_or(Edit::None, |v| Edit::Int { v, size, signed: true })
                }
                te::U1 | te::U2 | te::U4 | te::U8 | te::U | te::CHAR => {
                    decode_int(b, size, false).map_or(Edit::None, |v| Edit::Int { v, size, signed: false })
                }
                _ if is_enum && size <= 8 => {
                    row.type_name = format!("enum {class_name}");
                    decode_int(b, size, true).map_or(Edit::None, |v| Edit::Int { v, size, signed: true })
                }
                _ => match float_count(&class_name) {
                    Some(n) if n * 4 == size => Edit::Floats(
                        (0..n).map(|i| f32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])).collect(),
                    ),
                    _ => Edit::None,
                },
            };
            row
        }
    }

    /// Elements of an array (references can be followed; read-only).
    unsafe fn array_rows(api: &Il2cppApi, arr: Ptr, k: Ptr) -> Vec<Row> {
        // SAFETY: caller contract; reads bounded by length and readability.
        unsafe {
            let Some((n, hdr)) = array_shape(api, arr) else { return Vec::new() };
            let elem = il2cpp::class_element_class(api, k);
            let vt = elem.and_then(|e| il2cpp::class_is_valuetype(api, e)) == Some(true);
            let ename = elem.and_then(|e| il2cpp::class_name(api, e)).unwrap_or_else(|| "?".into());
            let esize = if vt { elem.and_then(|e| il2cpp::class_value_size(api, e)).unwrap_or(0) } else { 8 };
            if esize == 0 || esize > MAX_VALUE_SIZE {
                return vec![Row { name: format!("{n} items"), type_name: ename, value: "…".into(), edit: Edit::None, follow: None }];
            }
            let mut rows = Vec::new();
            for i in 0..n.min(MAX_ROWS) {
                let Some(slot) = (arr as usize).checked_add(hdr + i * esize) else { break };
                if !crate::memory::ptr_readable(slot, esize) {
                    break;
                }
                let b = std::slice::from_raw_parts(slot as *const u8, esize);
                let (value, follow) = if vt {
                    (decode_struct(&ename, b), None)
                } else {
                    let p = usize::from_le_bytes(b[..8].try_into().unwrap_or([0; 8])) as Ptr;
                    if p.is_null() {
                        ("null".into(), None)
                    } else {
                        let t = il2cpp::object_class(api, p).map(|c| full_name(api, c)).unwrap_or_else(|| "?".into());
                        (format!("→ {t}"), Some(Seg::Index(i)))
                    }
                };
                rows.push(Row { name: format!("[{i}]"), type_name: ename.clone(), value, edit: Edit::None, follow });
            }
            if n > MAX_ROWS {
                rows.push(Row { name: format!("… {} more", n - MAX_ROWS), type_name: String::new(), value: String::new(), edit: Edit::None, follow: None });
            }
            rows
        }
    }

    /// Raw store into a field of the viewed object (size-checked).
    unsafe fn write(api: &Il2cppApi, cache: &UnityCache, comps: &[Ptr], t: &Target, field: &str, bytes: &[u8]) -> Result<(), String> {
        let comp = t.comp.ok_or("no component selected")?;
        // SAFETY: caller contract; fresh walk, size matched to the field.
        unsafe {
            let obj = Self::follow(api, cache, comps, comp, &t.chain)?;
            let k = il2cpp::object_class(api, obj).ok_or("object unreadable")?;
            let f = instance_fields(api, k).into_iter().find(|f| f.name == field).ok_or_else(|| format!("field `{field}` not found"))?;
            if is_reference(f.type_enum) {
                return Err("references are read-only".into());
            }
            let size = match primitive_size(f.type_enum) {
                Some(n) => n,
                None => il2cpp::class_value_size(api, f.type_class).ok_or("unknown field size")?,
            };
            if size != bytes.len() {
                return Err(format!("size mismatch ({} bytes for a {size}-byte field)", bytes.len()));
            }
            if !il2cpp::field_set_raw(api, obj, f.field, bytes) {
                return Err("write failed".into());
            }
            crate::log_line(&format!("inspector: wrote {} = {:02x?}", field, bytes));
        }
        Ok(())
    }
}
