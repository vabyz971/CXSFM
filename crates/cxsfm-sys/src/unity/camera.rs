//! Main camera + Cinemachine v2: discovery, follow info, priority.
//!
//! The game films through `CinemachineVirtualCamera` + Brain (see log:
//! `RearRaceCamera`, `UpdateVirtualCameras`). Offsets and damping are
//! raw fields — reads/writes go through the cached `FieldInfo` with
//! `il2cpp::field_get/set_*` (see the camera mod).

use super::{UnityCache, find_objects_of_class};
use crate::il2cpp::{self, Il2cppApi};

/// Current main camera (`Camera.main`), or `None` (no camera yet — menu,
/// loading screen — or API missing).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn main_camera(api: &Il2cppApi, cache: &UnityCache) -> Option<*mut std::ffi::c_void> {
    // SAFETY: cached static getter, 0 args, null receiver.
    let cam = unsafe { il2cpp::invoke(api, cache.m_cam_main, std::ptr::null_mut(), &[]) };
    match cam {
        Some(c) if !c.is_null() => Some(c),
        _ => None,
    }
}

/// `Camera.WorldToScreenPoint(world)` → `[x, y, depth]` (pixels,
/// y bottom-up, depth = distance in front of the camera plane;
/// negative = behind). Struct-by-address argument, boxed Vector3 out.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn world_to_screen(
    api: &Il2cppApi,
    cache: &UnityCache,
    cam: *mut std::ffi::c_void,
    world: [f32; 3],
) -> Option<[f32; 3]> {
    if cache.m_cam_world_to_screen.is_null() || cam.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: cached method, one Vector3 by address; boxed Vector3 out.
    unsafe {
        let mut v = world;
        let params = [&mut v as *mut [f32; 3] as *mut std::ffi::c_void];
        let boxed = il2cpp::invoke(api, cache.m_cam_world_to_screen, cam, &params)?;
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

/// Read a camera's field of view in degrees.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn get_fov(
    api: &Il2cppApi,
    cache: &UnityCache,
    cam: *mut std::ffi::c_void,
) -> Option<f32> {
    if cache.m_get_fov.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: getter with no args; boxed float unboxed below.
    let boxed = unsafe { il2cpp::invoke(api, cache.m_get_fov, cam, &[]) }?;
    let unbox = api.object_unbox.unwrap();
    // SAFETY: `unbox` on a boxed Single yields its 4 payload bytes.
    let p = unsafe { unbox(boxed) } as *const f32;
    if p.is_null() {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(p) })
}

/// Set a camera's field of view in degrees. Returns success.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_fov(
    api: &Il2cppApi,
    cache: &UnityCache,
    cam: *mut std::ffi::c_void,
    fov: f32,
) -> bool {
    if cache.m_set_fov.is_null() {
        return false;
    }
    let params = [&fov as *const f32 as *mut std::ffi::c_void];
    // SAFETY: single float argument passed by address, as the ABI wants.
    // Void callee: `invoke_void`, never `invoke` (see its docs).
    unsafe { il2cpp::invoke_void(api, cache.m_set_fov, cam, &params) }
}

/// All live `CinemachineBrain` instances (usually one, on MainCamera).
///
/// # Safety
/// Same contract as `super::init`, plus the array-consumed-immediately
/// rule.
pub unsafe fn cm_brain_list(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.cm_brain_klass) }
}

/// All live game `CameraController` instances (freecam cutoff: disable
/// to stop the game driving the camera, restore on disable).
///
/// # Safety
/// Same contract as [`cm_brain_list`].
pub unsafe fn cam_controller_list(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.cam_controller_klass) }
}

/// All live `CinemachineVirtualCamera` instances (derived game vcams
/// like `RearRaceCamera` included).
///
/// # Safety
/// Same contract as [`cm_brain_list`].
pub unsafe fn cm_vcam_list(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.cm_vcam_klass) }
}

/// All live `CinemachineTransposer` body components.
///
/// # Safety
/// Same contract as [`cm_brain_list`].
pub unsafe fn cm_transposer_list(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.cm_trans_klass) }
}

/// All live `CinemachineOrbitalTransposer` body components.
///
/// # Safety
/// Same contract as [`cm_brain_list`].
pub unsafe fn cm_orbital_list(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.cm_orbital_klass) }
}

/// All live `CinemachineComposer` aim components.
///
/// # Safety
/// Same contract as [`cm_brain_list`].
pub unsafe fn cm_composer_list(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.cm_composer_klass) }
}

/// Name of the Brain's currently live virtual camera (`None` when the
/// Brain, the getter, or the vcam is missing).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn cm_active_vcam_name(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Option<String> {
    if cache.m_brain_get_active_vcam.is_null() {
        return None;
    }
    // SAFETY: first brain, 0-arg getter, name copied out.
    unsafe {
        let brains = cm_brain_list(api, cache);
        let brain = *brains.first()?;
        let vcam = il2cpp::invoke(api, cache.m_brain_get_active_vcam, brain, &[])?;
        if vcam.is_null() {
            return None;
        }
        Some(super::object_name(api, cache, vcam))
    }
}

/// Follow target's GameObject name for one vcam (`None` when unknown).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn cm_vcam_follow_name(
    api: &Il2cppApi,
    cache: &UnityCache,
    vcam: *mut std::ffi::c_void,
) -> Option<String> {
    if cache.m_vcam_get_follow.is_null() || vcam.is_null() {
        return None;
    }
    // SAFETY: Follow Transform → its GameObject → name, all immediate.
    unsafe {
        let follow = il2cpp::invoke(api, cache.m_vcam_get_follow, vcam, &[])?;
        if follow.is_null() {
            return None;
        }
        let go = super::component_gameobject(api, cache, follow)?;
        Some(super::object_name(api, cache, go))
    }
}

/// LookAt target's GameObject name for one vcam (`None` when unknown).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn cm_vcam_lookat_name(
    api: &Il2cppApi,
    cache: &UnityCache,
    vcam: *mut std::ffi::c_void,
) -> Option<String> {
    if cache.m_vcam_get_lookat.is_null() || vcam.is_null() {
        return None;
    }
    // SAFETY: LookAt Transform → its GameObject → name, all immediate.
    unsafe {
        let lookat = il2cpp::invoke(api, cache.m_vcam_get_lookat, vcam, &[])?;
        if lookat.is_null() {
            return None;
        }
        let go = super::component_gameobject(api, cache, lookat)?;
        Some(super::object_name(api, cache, go))
    }
}

/// A vcam's `Priority` (the Brain blends to the highest).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn cm_vcam_priority(
    api: &Il2cppApi,
    cache: &UnityCache,
    vcam: *mut std::ffi::c_void,
) -> Option<i32> {
    // SAFETY: cached getter on a live vcam.
    unsafe { super::get_i32(api, cache.m_vcam_get_priority, vcam) }
}

/// Set a vcam's `Priority` (preset switching = boost one, restore on
/// disable). Returns success.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn cm_set_vcam_priority(
    api: &Il2cppApi,
    cache: &UnityCache,
    vcam: *mut std::ffi::c_void,
    priority: i32,
) -> bool {
    // SAFETY: cached setter, int by address.
    unsafe { super::set_i32(api, cache.m_vcam_set_priority, vcam, priority) }
}
