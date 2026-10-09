//! GameObject / Transform / Behaviour helpers: identity, hierarchy
//! walk, visibility and the surgical `enabled` switch.
//!
//! Object handles go stale across scene changes: re-resolve often,
//! cache only classes/methods (in [`UnityCache`]).

use super::UnityCache;
use crate::il2cpp::{self, Il2cppApi};

/// A component's `GameObject` (every `Transform` is one — the way back
/// from a child transform to its object while walking).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn component_gameobject(
    api: &Il2cppApi,
    cache: &UnityCache,
    component: *mut std::ffi::c_void,
) -> Option<*mut std::ffi::c_void> {
    if cache.m_comp_get_gameobject.is_null() || component.is_null() {
        return None;
    }
    // SAFETY: cached getter; null (destroyed) → None.
    match unsafe { il2cpp::invoke(api, cache.m_comp_get_gameobject, component, &[]) } {
        Some(g) if !g.is_null() => Some(g),
        _ => None,
    }
}

/// Whether a component's GameObject is active in the hierarchy.
///
/// An enumerated component may sit on a disabled branch: writes to it
/// change memory nobody renders. Returns `None` when undeterminable.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn component_active(
    api: &Il2cppApi,
    cache: &UnityCache,
    component: *mut std::ffi::c_void,
) -> Option<bool> {
    if cache.m_comp_get_gameobject.is_null() || cache.m_go_get_active.is_null() {
        return None;
    }
    // SAFETY: cached getters, no args; boxed Boolean unboxed below.
    unsafe {
        let go = il2cpp::invoke(api, cache.m_comp_get_gameobject, component, &[])?;
        if go.is_null() {
            return None;
        }
        let boxed = il2cpp::invoke(api, cache.m_go_get_active, go, &[])?;
        match api.object_unbox {
            Some(unbox) => {
                let p = unbox(boxed) as *const u8;
                if p.is_null() {
                    None
                } else {
                    Some(std::ptr::read_unaligned(p) != 0)
                }
            }
            None => None,
        }
    }
}

/// Read a Behaviour's `enabled` flag.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn beh_get_enabled(
    api: &Il2cppApi,
    cache: &UnityCache,
    component: *mut std::ffi::c_void,
) -> Option<bool> {
    if cache.m_beh_get_enabled.is_null() {
        return None;
    }
    // SAFETY: cached getter, no args; boxed Boolean unboxed below.
    unsafe {
        let boxed = il2cpp::invoke(api, cache.m_beh_get_enabled, component, &[])?;
        match api.object_unbox {
            Some(unbox) => {
                let p = unbox(boxed) as *const u8;
                if p.is_null() {
                    None
                } else {
                    Some(std::ptr::read_unaligned(p) != 0)
                }
            }
            None => None,
        }
    }
}

/// Set a Behaviour's `enabled` flag. `false` hides a renderer component
/// (the renderer checks the flag every frame — no mesh rebuild, no
/// strings involved). Returns success.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn beh_set_enabled(
    api: &Il2cppApi,
    cache: &UnityCache,
    component: *mut std::ffi::c_void,
    enabled: bool,
) -> bool {
    if cache.m_beh_set_enabled.is_null() {
        return false;
    }
    let mut flag: u8 = if enabled { 1 } else { 0 };
    let params = [&mut flag as *mut u8 as *mut std::ffi::c_void];
    // SAFETY: single bool argument passed by address, as the ABI wants.
    // Void callee: `invoke_void`, never `invoke` (see its docs).
    unsafe { il2cpp::invoke_void(api, cache.m_beh_set_enabled, component, &params) }
}

/// `Object.get_name()` copied out (`<unnamed>` when unreadable —
/// names feed the inspector tree, where `None` would drop the node).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn object_name(
    api: &Il2cppApi,
    cache: &UnityCache,
    obj: *mut std::ffi::c_void,
) -> String {
    if cache.m_get_name.is_null() || obj.is_null() {
        return String::from("<unnamed>");
    }
    // SAFETY: cached getter; string copied out immediately.
    unsafe {
        match il2cpp::invoke(api, cache.m_get_name, obj, &[]) {
            Some(s) => il2cpp::read_string(api, s).unwrap_or_else(|| String::from("<unreadable>")),
            None => String::from("<unnamed>"),
        }
    }
}

/// `Object.GetInstanceID()` — stable per-object id for the inspector.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn get_instance_id(
    api: &Il2cppApi,
    cache: &UnityCache,
    obj: *mut std::ffi::c_void,
) -> Option<i32> {
    // SAFETY: cached getter on a live object.
    unsafe { super::get_i32(api, cache.m_get_instance_id, obj) }
}

/// Whether the native object behind a wrapper is still alive.
///
/// Reads `Object.m_CachedPtr`: null means the native object is gone
/// (scene/view churn destroyed it) and ANY further read or write on
/// the wrapper crashes — proven on camera writes seconds after a view
/// change. Call before every write; on `false`, force a re-search and
/// bind the replacement instead of touching the dead pointer.
/// `None` (field missing) is treated as alive: without the probe there
/// is nothing to check, same as before.
///
/// # Safety
/// Same contract as `super::init`; `field_get_value` only copies one
/// pointer-sized payload into a caller buffer.
pub unsafe fn object_alive(
    api: &Il2cppApi,
    cache: &UnityCache,
    obj: *mut std::ffi::c_void,
) -> Option<bool> {
    let get_value = api.field_get_value?;
    if cache.m_cached_ptr.is_null() || obj.is_null() {
        return None;
    }
    // SAFETY: IntPtr-sized caller buffer, field-sized copy by the runtime.
    unsafe {
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        get_value(
            obj,
            cache.m_cached_ptr,
            &mut ptr as *mut *mut std::ffi::c_void as *mut std::ffi::c_void,
        );
        Some(!ptr.is_null())
    }
}

/// `GameObject.get_activeInHierarchy()`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn go_active(
    api: &Il2cppApi,
    cache: &UnityCache,
    go: *mut std::ffi::c_void,
) -> Option<bool> {
    // SAFETY: cached getter on a live object.
    unsafe { super::get_bool(api, cache.m_go_get_active, go) }
}

/// `GameObject.get_transform()`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn go_transform(
    api: &Il2cppApi,
    cache: &UnityCache,
    go: *mut std::ffi::c_void,
) -> Option<*mut std::ffi::c_void> {
    if cache.m_go_get_transform.is_null() || go.is_null() {
        return None;
    }
    // SAFETY: cached getter on a live object.
    match unsafe { il2cpp::invoke(api, cache.m_go_get_transform, go, &[]) } {
        Some(t) if !t.is_null() => Some(t),
        _ => None,
    }
}

/// `GameObject.SetActive(bool)`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn go_set_active(
    api: &Il2cppApi,
    cache: &UnityCache,
    go: *mut std::ffi::c_void,
    active: bool,
) -> bool {
    if cache.m_go_set_active.is_null() || go.is_null() {
        return false;
    }
    let mut flag: u8 = if active { 1 } else { 0 };
    let params = [&mut flag as *mut u8 as *mut std::ffi::c_void];
    // SAFETY: single bool by address; void callee.
    unsafe { il2cpp::invoke_void(api, cache.m_go_set_active, go, &params) }
}

/// `Transform.get_childCount()` (0 when unknown — the walk just stops).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tr_child_count(
    api: &Il2cppApi,
    cache: &UnityCache,
    tr: *mut std::ffi::c_void,
) -> usize {
    // SAFETY: cached getter on a live transform.
    unsafe { super::get_i32(api, cache.m_tr_get_child_count, tr).unwrap_or(0).max(0) as usize }
}

/// `Transform.GetChild(i)`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tr_child(
    api: &Il2cppApi,
    cache: &UnityCache,
    tr: *mut std::ffi::c_void,
    index: i32,
) -> Option<*mut std::ffi::c_void> {
    if cache.m_tr_get_child.is_null() || tr.is_null() {
        return None;
    }
    let params = [&index as *const i32 as *mut std::ffi::c_void];
    // SAFETY: cached method, int by address; null (bad index) → None.
    match unsafe { il2cpp::invoke(api, cache.m_tr_get_child, tr, &params) } {
        Some(t) if !t.is_null() => Some(t),
        _ => None,
    }
}

/// `Transform.get_position()` as `[x, y, z]`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tr_position(
    api: &Il2cppApi,
    cache: &UnityCache,
    tr: *mut std::ffi::c_void,
) -> Option<[f32; 3]> {
    if cache.m_tr_get_position.is_null() || tr.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: cached getter; boxed Vector3 is 3 packed floats.
    unsafe {
        let boxed = il2cpp::invoke(api, cache.m_tr_get_position, tr, &[])?;
        let p = api.object_unbox.unwrap()(boxed) as *const f32;
        if p.is_null() {
            return None;
        }
        Some([
            std::ptr::read_unaligned(p),
            std::ptr::read_unaligned(p.add(1)),
            std::ptr::read_unaligned(p.add(2)),
        ])
    }
}

/// `GameObject.Find(name)` (static): root lookup by exact scene path
/// name. Returns `None` when missing — never throws.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn go_find(
    api: &Il2cppApi,
    cache: &UnityCache,
    name: &str,
) -> Option<*mut std::ffi::c_void> {
    if cache.m_go_find.is_null() {
        return None;
    }
    let s = unsafe { il2cpp::new_string(api, name) }?;
    let params = [s];
    // SAFETY: cached static, one managed-string argument.
    match unsafe { il2cpp::invoke(api, cache.m_go_find, std::ptr::null_mut(), &params) } {
        Some(g) if !g.is_null() => Some(g),
        _ => None,
    }
}

/// `Transform.Find(child)` path lookup under a transform.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tr_find(
    api: &Il2cppApi,
    cache: &UnityCache,
    tr: *mut std::ffi::c_void,
    child: &str,
) -> Option<*mut std::ffi::c_void> {
    if cache.m_tr_find.is_null() || tr.is_null() {
        return None;
    }
    let s = unsafe { il2cpp::new_string(api, child) }?;
    let params = [s];
    // SAFETY: cached method, one managed-string argument; null
    // (missing child) → None.
    match unsafe { il2cpp::invoke(api, cache.m_tr_find, tr, &params) } {
        Some(t) if !t.is_null() => Some(t),
        _ => None,
    }
}

/// `Transform.get_localScale()` as `[x, y, z]`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tr_get_scale(
    api: &Il2cppApi,
    cache: &UnityCache,
    tr: *mut std::ffi::c_void,
) -> Option<[f32; 3]> {
    if cache.m_tr_get_local_scale.is_null() || tr.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: cached getter; boxed Vector3 is 3 packed floats.
    unsafe {
        let boxed = il2cpp::invoke(api, cache.m_tr_get_local_scale, tr, &[])?;
        let p = api.object_unbox.unwrap()(boxed) as *const f32;
        if p.is_null() {
            return None;
        }
        Some([
            std::ptr::read_unaligned(p),
            std::ptr::read_unaligned(p.add(1)),
            std::ptr::read_unaligned(p.add(2)),
        ])
    }
}

/// `Transform.set_localScale(Vector3)`, passed struct-by-address
/// (safe on pinned handles).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tr_set_scale(
    api: &Il2cppApi,
    cache: &UnityCache,
    tr: *mut std::ffi::c_void,
    scale: [f32; 3],
) -> bool {
    if cache.m_tr_set_local_scale.is_null() || tr.is_null() {
        return false;
    }
    // SAFETY: single Vector3 argument passed by address, as the ABI wants.
    // Void callee: `invoke_void`, never `invoke` (see its docs).
    unsafe {
        let mut v = scale;
        let params = [&mut v as *mut [f32; 3] as *mut std::ffi::c_void];
        il2cpp::invoke_void(api, cache.m_tr_set_local_scale, tr, &params)
    }
}
