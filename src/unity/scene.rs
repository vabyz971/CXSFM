//! Scene walk: root `GameObject`s across all loaded scenes.
//!
//! Games stack additive scenes (menu + track + cars + HUD); walking
//! only the active one hides most of the game. `Scene` is a value
//! type: instance calls run on the unboxed payload, never on the box.

use super::{UnityCache, read_object_array};
use crate::il2cpp::{self, Il2cppApi};

/// Root `GameObject`s of the active scene (includes inactive roots —
/// the reason this beats `FindObjectsOfType` for inspection).
///
/// # Safety
/// Same contract as `super::init`, plus the array-consumed-immediately
/// rule.
pub unsafe fn scene_root_objects(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    if cache.m_scene_get_active.is_null()
        || cache.m_scene_get_roots.is_null()
        || api.object_unbox.is_none()
    {
        return Vec::new();
    }
    // SAFETY: static getter → boxed Scene → unbox → instance call on
    // the value payload → GameObject[] consumed immediately.
    unsafe {
        let boxed = match il2cpp::invoke(api, cache.m_scene_get_active, std::ptr::null_mut(), &[]) {
            Some(b) => b,
            None => return Vec::new(),
        };
        roots_of_boxed_scene(api, cache, boxed)
    }
}

/// Roots of one boxed `Scene` value.
///
/// # Safety
/// `boxed` must be a live boxed Scene; the array is consumed
/// immediately.
unsafe fn roots_of_boxed_scene(
    api: &Il2cppApi,
    cache: &UnityCache,
    boxed: *mut std::ffi::c_void,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: unbox → instance call on the value payload.
    unsafe {
        let scene_value = match api.object_unbox {
            Some(unbox) => unbox(boxed),
            None => return Vec::new(),
        };
        if scene_value.is_null() {
            return Vec::new();
        }
        let arr = match il2cpp::invoke(api, cache.m_scene_get_roots, scene_value, &[]) {
            Some(a) => a,
            None => return Vec::new(),
        };
        read_object_array(api, arr, 1024)
    }
}

/// Name of one boxed `Scene` value (`Scene.name`).
///
/// # Safety
/// `boxed` must be a live boxed Scene; the string is copied out.
unsafe fn name_of_boxed_scene(
    api: &Il2cppApi,
    cache: &UnityCache,
    boxed: *mut std::ffi::c_void,
) -> Option<String> {
    if cache.m_scene_get_name.is_null() {
        return None;
    }
    // SAFETY: unbox → instance getter on the value payload.
    unsafe {
        let scene_value = api.object_unbox?(boxed);
        if scene_value.is_null() {
            return None;
        }
        let s = il2cpp::invoke(api, cache.m_scene_get_name, scene_value, &[])?;
        il2cpp::read_string(api, s)
    }
}

/// All loaded scenes as `(name, roots)`. Capped at 32 scenes.
///
/// # Safety
/// Same contract as `super::init`, plus the array-consumed-immediately
/// rule per scene.
pub unsafe fn loaded_scenes(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<(String, Vec<*mut std::ffi::c_void>)> {
    if cache.m_scene_get_count.is_null()
        || cache.m_scene_get_at.is_null()
        || api.object_unbox.is_none()
    {
        return Vec::new();
    }
    // SAFETY: static count → boxed Scene per index → name + roots.
    unsafe {
        let count = super::get_i32(api, cache.m_scene_get_count, std::ptr::null_mut())
            .unwrap_or(0)
            .clamp(0, 32) as i32;
        let mut out = Vec::new();
        for i in 0..count {
            let params = [&i as *const i32 as *mut std::ffi::c_void];
            let boxed = match il2cpp::invoke(api, cache.m_scene_get_at, std::ptr::null_mut(), &params)
            {
                Some(b) => b,
                None => continue,
            };
            let name =
                name_of_boxed_scene(api, cache, boxed).unwrap_or_else(|| format!("scene {i}"));
            let roots = roots_of_boxed_scene(api, cache, boxed);
            out.push((name, roots));
        }
        out
    }
}
