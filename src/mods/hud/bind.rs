//! Field explorer, tick side (phase 2): find where the game keeps
//! speed / RPM / gear.
//!
//! Strictly on demand (Search / Open / Reread clicks), never per tick.
//! Search walks class METADATA only. Open runs ONE `FindObjectsOfType`
//! for the root class, then follows the requested chain of reference
//! fields from that fresh root and copies field payloads out with
//! `il2cpp_field_get_value` — no getter, no managed code, no managed
//! allocation. No object pointer survives the call: every reread
//! re-walks from the root (stored-pointer rule).

use super::HudMod;
use super::model::{
    ClassHit, ExploreReq, FieldRow, OpenView, decode_primitive, decode_struct, is_reference,
    primitive_name, primitive_size, te,
};
use crate::il2cpp::{self, Il2cppApi};
use crate::mods::common::lock;
use crate::unity::UnityCache;

/// Search cap (results beyond are flagged `truncated`).
const MAX_HITS: usize = 300;
/// Value-type payloads larger than this are not read (buffer bound).
const MAX_VALUE_SIZE: usize = 64;
/// Displayed string cap (chars).
const MAX_STR: usize = 60;

type Ptr = *mut std::ffi::c_void;

/// `UnityEngine.Object` (or a subclass): only those carry
/// `m_CachedPtr`, so only those get the `object_alive` probe.
unsafe fn is_unity_object(api: &Il2cppApi, mut k: Ptr) -> bool {
    // SAFETY: caller contract; metadata reads, bounded climb.
    unsafe {
        for _ in 0..64 {
            if il2cpp::class_name(api, k).as_deref() == Some("Object")
                && il2cpp::class_namespace(api, k).as_deref() == Some("UnityEngine")
            {
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

/// Engine base classes whose fields are noise for value hunting.
fn is_base_class(ns: &str, name: &str) -> bool {
    (ns == "UnityEngine" && matches!(name, "MonoBehaviour" | "Behaviour" | "Component" | "Object"))
        || (ns == "System" && name == "Object")
}

impl HudMod {
    /// Unity cache on demand (explorer only; nothing else needs it).
    fn explore_unity(&self, api: &Il2cppApi) -> Option<UnityCache> {
        if let Some(c) = lock(&self.unity).as_ref().copied() {
            return Some(c);
        }
        // SAFETY: attached tick thread; null-checked chain.
        let cache = unsafe {
            let domain = il2cpp::domain_checked(api)?;
            crate::unity::init(api, domain)?
        };
        *lock(&self.unity) = Some(cache);
        Some(cache)
    }

    /// Serve the pending explorer request, if any (tick thread).
    pub(crate) fn explore_tick(&self) {
        let Some(req) = lock(&self.explore_req).take() else {
            return;
        };
        let Some(api) = crate::il2cpp_api() else {
            return;
        };
        match req {
            ExploreReq::Search(q) => {
                // SAFETY: attached tick thread (see module docs).
                let (hits, truncated) = unsafe { Self::search(api, &q) };
                crate::log_line(&format!("hud: recherche `{q}` → {} classes", hits.len()));
                let mut v = lock(&self.explore);
                v.hits = hits;
                v.truncated = truncated;
                v.searched = Some(q);
                v.open = None;
                v.error = None;
            }
            ExploreReq::Open { klass, root, path, instance } => {
                let view = match self.explore_unity(api) {
                    // SAFETY: attached tick thread (see module docs).
                    Some(cache) => unsafe { Self::open(api, &cache, klass, root, path, instance) },
                    None => OpenView {
                        klass,
                        root,
                        path,
                        error: Some("cache Unity indisponible (jeu en chargement ?)".into()),
                        ..OpenView::default()
                    },
                };
                lock(&self.explore).open = Some(view);
            }
        }
    }

    /// Case-insensitive class-name search over every loaded image.
    unsafe fn search(api: &Il2cppApi, query: &str) -> (Vec<ClassHit>, bool) {
        let q = query.trim().to_lowercase();
        let mut hits = Vec::new();
        if q.len() < 2 {
            return (hits, false);
        }
        // SAFETY: attached thread; metadata enumeration only.
        unsafe {
            let Some(domain) = il2cpp::domain_checked(api) else {
                return (hits, false);
            };
            for img in il2cpp::all_images(api, domain) {
                let image = il2cpp::image_name(api, img).unwrap_or_default();
                let n = il2cpp::image_class_count(api, img).unwrap_or(0).min(200_000);
                for i in 0..n {
                    let Some(k) = il2cpp::image_class(api, img, i) else {
                        continue;
                    };
                    let Some(name) = il2cpp::class_name(api, k) else {
                        continue;
                    };
                    if !name.to_lowercase().contains(&q) {
                        continue;
                    }
                    if hits.len() >= MAX_HITS {
                        return (hits, true);
                    }
                    let ns = il2cpp::class_namespace(api, k).unwrap_or_default();
                    let full = if ns.is_empty() { name } else { format!("{ns}.{name}") };
                    hits.push(ClassHit { klass: k as usize, full, image: image.clone() });
                }
            }
        }
        (hits, false)
    }

    /// Resolve root instance → reference chain → field rows.
    unsafe fn open(
        api: &Il2cppApi,
        cache: &UnityCache,
        klass: usize,
        root: String,
        path: Vec<String>,
        instance: usize,
    ) -> OpenView {
        let mut v = OpenView { klass, root, path, instance, ..OpenView::default() };
        let k = klass as Ptr;
        // SAFETY: attached tick thread; every object is re-found from
        // the fresh FindObjectsOfType result and checked readable.
        unsafe {
            // Non-Unity classes make FindObjectsOfType throw: the
            // exception is swallowed by `invoke` → zero instances.
            let objs = crate::unity::find_objects_of_class(api, cache, k);
            v.instances = objs.len();
            if objs.is_empty() {
                v.at_class = v.root.clone();
                v.error = Some("aucune instance vivante (classe non-Unity, ou absente de la scène) — champs déclarés seulement".into());
                v.fields = Self::rows(api, k, None);
                return v;
            }
            v.instance = v.instance.min(objs.len() - 1);
            let mut obj = objs[v.instance];
            if crate::unity::object_alive(api, cache, obj) != Some(true) {
                v.error = Some("instance détruite entre-temps, relis".into());
                return v;
            }
            for seg in v.path.clone() {
                let Some(cls) = il2cpp::object_class(api, obj) else {
                    v.error = Some(format!("objet illisible avant `{seg}`"));
                    return v;
                };
                let Some(f) = il2cpp::find_field(api, cls, &seg) else {
                    v.error = Some(format!("champ `{seg}` introuvable"));
                    return v;
                };
                let mut buf = [0u8; 8];
                if !il2cpp::field_get_raw(api, obj, f, &mut buf) {
                    v.error = Some(format!("lecture de `{seg}` impossible"));
                    return v;
                }
                let next = usize::from_le_bytes(buf) as Ptr;
                if next.is_null() {
                    v.error = Some(format!("`{seg}` est null (pas encore assigné ?)"));
                    return v;
                }
                let Some(next_cls) = il2cpp::object_class(api, next) else {
                    v.error = Some(format!("`{seg}` pointe vers une mémoire illisible"));
                    return v;
                };
                if is_unity_object(api, next_cls) && crate::unity::object_alive(api, cache, next) != Some(true) {
                    v.error = Some(format!("`{seg}` : objet Unity détruit"));
                    return v;
                }
                obj = next;
            }
            let Some(cls) = il2cpp::object_class(api, obj) else {
                v.error = Some("objet illisible".into());
                return v;
            };
            v.at_class = il2cpp::class_name(api, cls).unwrap_or_default();
            v.fields = Self::rows(api, cls, Some(obj));
        }
        v
    }

    /// Instance fields of `cls` and its parents (engine bases skipped),
    /// most-derived first; values read from `obj` when given.
    unsafe fn rows(api: &Il2cppApi, cls: Ptr, obj: Option<Ptr>) -> Vec<FieldRow> {
        let mut out = Vec::new();
        let mut k = cls;
        // SAFETY: caller contract; bounded climb, readable-checked reads.
        unsafe {
            for _ in 0..32 {
                let ns = il2cpp::class_namespace(api, k).unwrap_or_default();
                let name = il2cpp::class_name(api, k).unwrap_or_default();
                if is_base_class(&ns, &name) {
                    break;
                }
                for f in il2cpp::class_fields(api, k) {
                    if f.is_static {
                        continue;
                    }
                    let (type_name, value, follow) = Self::describe(api, &f, obj);
                    out.push(FieldRow { name: f.name, type_name, value, follow });
                }
                match il2cpp::class_parent(api, k) {
                    Some(p) => k = p,
                    None => break,
                }
            }
        }
        out
    }

    /// `(type name, value, followable)` of one field.
    unsafe fn describe(api: &Il2cppApi, f: &il2cpp::FieldDesc, obj: Option<Ptr>) -> (String, String, bool) {
        // SAFETY: caller contract; sizes bound every copy (see field_get_raw).
        unsafe {
            let class_name = il2cpp::class_name(api, f.type_class).unwrap_or_else(|| "?".into());
            let valuetype_generic = f.type_enum == te::GENERICINST
                && il2cpp::class_is_valuetype(api, f.type_class) == Some(true);
            let reference = is_reference(f.type_enum) || (f.type_enum == te::GENERICINST && !valuetype_generic);
            let type_name = primitive_name(f.type_enum).map(str::to_string).unwrap_or(class_name.clone());
            let Some(obj) = obj else {
                return (type_name, String::new(), false);
            };
            if reference {
                let mut buf = [0u8; 8];
                if !il2cpp::field_get_raw(api, obj, f.field, &mut buf) {
                    return (type_name, "?".into(), false);
                }
                let p = usize::from_le_bytes(buf) as Ptr;
                if p.is_null() {
                    return (type_name, "null".into(), false);
                }
                if f.type_enum == te::STRING {
                    if !crate::memory::ptr_readable(p as usize, 32) {
                        return (type_name, "?".into(), false);
                    }
                    let s: String = il2cpp::read_string(api, p)
                        .unwrap_or_default()
                        .chars()
                        .take(MAX_STR)
                        .collect();
                    return (type_name, format!("\"{s}\""), false);
                }
                let target = il2cpp::object_class(api, p)
                    .and_then(|c| il2cpp::class_name(api, c))
                    .unwrap_or_else(|| "?".into());
                let arrays = matches!(f.type_enum, te::SZARRAY | te::ARRAY);
                return (type_name, format!("→ {target}"), !arrays);
            }
            let size = match primitive_size(f.type_enum) {
                Some(n) => Some(n),
                None if f.type_enum == te::VALUETYPE || valuetype_generic => {
                    il2cpp::class_value_size(api, f.type_class)
                }
                None => None,
            };
            let Some(size) = size.filter(|&n| n <= MAX_VALUE_SIZE) else {
                return (type_name, "…".into(), false);
            };
            let mut buf = [0u8; MAX_VALUE_SIZE];
            if !il2cpp::field_get_raw(api, obj, f.field, &mut buf) {
                return (type_name, "?".into(), false);
            }
            let bytes = &buf[..size];
            let value = decode_primitive(f.type_enum, bytes).unwrap_or_else(|| decode_struct(&class_name, bytes));
            (type_name, value, false)
        }
    }
}
