//! Video / quality knobs: `QualitySettings`, `Screen`, `RenderSettings`.
//!
//! Pure static getters/setters — the same invoke shapes as FOV/text.
//! Missing classes degrade the video tool, never the cache.

use super::UnityCache;
use crate::il2cpp::{self, Il2cppApi};

/// `QualitySettings.GetQualityLevel()`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn quality_level(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { super::get_i32(api, cache.m_qs_get_level, std::ptr::null_mut()) }
}

/// `QualitySettings.SetQualityLevel(int)`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_quality_level(api: &Il2cppApi, cache: &UnityCache, level: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { super::set_i32(api, cache.m_qs_set_level, std::ptr::null_mut(), level) }
}

/// `QualitySettings.names` (level names, capped at 16).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn quality_names(api: &Il2cppApi, cache: &UnityCache) -> Vec<String> {
    if cache.m_qs_get_names.is_null() {
        return Vec::new();
    }
    // SAFETY: cached static getter; string[] consumed immediately.
    unsafe {
        let arr = match il2cpp::invoke(api, cache.m_qs_get_names, std::ptr::null_mut(), &[]) {
            Some(a) => a,
            None => return Vec::new(),
        };
        super::read_object_array(api, arr, 16)
            .into_iter()
            .filter_map(|s| il2cpp::read_string(api, s))
            .collect()
    }
}

/// `QualitySettings.get_vSyncCount/set_vSyncCount`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn vsync_count(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { super::get_i32(api, cache.m_qs_get_vsync, std::ptr::null_mut()) }
}

/// `QualitySettings.get_vSyncCount/set_vSyncCount`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_vsync_count(api: &Il2cppApi, cache: &UnityCache, v: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { super::set_i32(api, cache.m_qs_set_vsync, std::ptr::null_mut(), v) }
}

/// `QualitySettings.get_antiAliasing/set_antiAliasing` (0/2/4/8).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn antialiasing(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { super::get_i32(api, cache.m_qs_get_aa, std::ptr::null_mut()) }
}

/// `QualitySettings.get_antiAliasing/set_antiAliasing` (0/2/4/8).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_antialiasing(api: &Il2cppApi, cache: &UnityCache, v: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { super::set_i32(api, cache.m_qs_set_aa, std::ptr::null_mut(), v) }
}

/// `QualitySettings.get_shadows/set_shadows` (ShadowQuality as int).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn shadows(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { super::get_i32(api, cache.m_qs_get_shadows, std::ptr::null_mut()) }
}

/// `QualitySettings.get_shadows/set_shadows` (ShadowQuality as int).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_shadows(api: &Il2cppApi, cache: &UnityCache, v: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { super::set_i32(api, cache.m_qs_set_shadows, std::ptr::null_mut(), v) }
}

/// `Screen.currentResolution` as `(width, height, refresh_hz)`.
///
/// `Resolution` is 3 packed ints (width, height, refreshRate).
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn screen_resolution(api: &Il2cppApi, cache: &UnityCache) -> Option<(i32, i32, i32)> {
    if cache.m_screen_get_resolution.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: cached static getter; boxed Resolution unboxed below.
    unsafe {
        let boxed = il2cpp::invoke(api, cache.m_screen_get_resolution, std::ptr::null_mut(), &[])?;
        let p = api.object_unbox.unwrap()(boxed) as *const i32;
        if p.is_null() {
            return None;
        }
        Some((
            std::ptr::read_unaligned(p),
            std::ptr::read_unaligned(p.add(1)),
            std::ptr::read_unaligned(p.add(2)),
        ))
    }
}

/// `Screen.SetResolution(w, h, fullscreen)`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn screen_set_resolution(
    api: &Il2cppApi,
    cache: &UnityCache,
    w: i32,
    h: i32,
    fullscreen: bool,
) -> bool {
    if cache.m_screen_set_resolution.is_null() {
        return false;
    }
    let mut fs: u8 = if fullscreen { 1 } else { 0 };
    let params = [
        &w as *const i32 as *mut std::ffi::c_void,
        &h as *const i32 as *mut std::ffi::c_void,
        &mut fs as *mut u8 as *mut std::ffi::c_void,
    ];
    // SAFETY: (int, int, bool) by address; void callee.
    unsafe { il2cpp::invoke_void(api, cache.m_screen_set_resolution, std::ptr::null_mut(), &params) }
}

/// `Screen.get_fullScreen/set_fullScreen`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn screen_fullscreen(api: &Il2cppApi, cache: &UnityCache) -> Option<bool> {
    // SAFETY: cached static getter.
    unsafe { super::get_bool(api, cache.m_screen_get_fullscreen, std::ptr::null_mut()) }
}

/// `Screen.get_fullScreen/set_fullScreen`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_screen_fullscreen(api: &Il2cppApi, cache: &UnityCache, v: bool) -> bool {
    if cache.m_screen_set_fullscreen.is_null() {
        return false;
    }
    let mut flag: u8 = if v { 1 } else { 0 };
    let params = [&mut flag as *mut u8 as *mut std::ffi::c_void];
    // SAFETY: single bool by address; void callee.
    unsafe { il2cpp::invoke_void(api, cache.m_screen_set_fullscreen, std::ptr::null_mut(), &params) }
}

/// `RenderSettings.get_fog/set_fog`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn render_fog(api: &Il2cppApi, cache: &UnityCache) -> Option<bool> {
    // SAFETY: cached static getter.
    unsafe { super::get_bool(api, cache.m_rs_get_fog, std::ptr::null_mut()) }
}

/// `RenderSettings.get_fog/set_fog`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_render_fog(api: &Il2cppApi, cache: &UnityCache, v: bool) -> bool {
    if cache.m_rs_set_fog.is_null() {
        return false;
    }
    let mut flag: u8 = if v { 1 } else { 0 };
    let params = [&mut flag as *mut u8 as *mut std::ffi::c_void];
    // SAFETY: single bool by address; void callee.
    unsafe { il2cpp::invoke_void(api, cache.m_rs_set_fog, std::ptr::null_mut(), &params) }
}

/// `RenderSettings.get_fogDensity/set_fogDensity`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn render_fog_density(api: &Il2cppApi, cache: &UnityCache) -> Option<f32> {
    // SAFETY: cached static getter.
    unsafe { super::get_f32(api, cache.m_rs_get_fog_density, std::ptr::null_mut()) }
}

/// `RenderSettings.get_fogDensity/set_fogDensity`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_render_fog_density(api: &Il2cppApi, cache: &UnityCache, v: f32) -> bool {
    // SAFETY: cached static setter, float by address.
    unsafe { super::set_f32(api, cache.m_rs_set_fog_density, std::ptr::null_mut(), v) }
}

/// `RenderSettings.get_ambientIntensity/set_ambientIntensity`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn render_ambient(api: &Il2cppApi, cache: &UnityCache) -> Option<f32> {
    // SAFETY: cached static getter.
    unsafe { super::get_f32(api, cache.m_rs_get_ambient, std::ptr::null_mut()) }
}

/// `RenderSettings.get_ambientIntensity/set_ambientIntensity`.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_render_ambient(api: &Il2cppApi, cache: &UnityCache, v: f32) -> bool {
    // SAFETY: cached static setter, float by address.
    unsafe { super::set_f32(api, cache.m_rs_set_ambient, std::ptr::null_mut(), v) }
}
