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
