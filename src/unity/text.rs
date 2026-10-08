//! uGUI `Text` + TextMeshPro helpers (HUD text mods).
//!
//! Modern Unity games render UI text through TMP, not uGUI — both are
//! enumerated, callers pick what exists.

use super::{UnityCache, find_objects_of_class};
use crate::il2cpp::{self, Il2cppApi};

/// All live `UnityEngine.UI.Text` instances (possibly empty — modern
/// games use TextMeshPro instead; see [`find_tmp_texts`).
///
/// # Safety
/// Same contract as `super::init`, plus the array-consumed-
/// immediately rule from `il2cpp::find_objects_of_type`.
pub unsafe fn find_texts(
    api: &Il2cppApi,
    cache: &UnityCache,
    domain: *mut std::ffi::c_void,
) -> Vec<*mut std::ffi::c_void> {
    let _ = domain;
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.text_klass) }
}

/// All live `TMPro.TextMeshProUGUI` instances (possibly empty).
///
/// # Safety
/// Same contract as [`find_texts`].
pub unsafe fn find_tmp_texts(
    api: &Il2cppApi,
    cache: &UnityCache,
) -> Vec<*mut std::ffi::c_void> {
    // SAFETY: cached class handle, consumed immediately.
    unsafe { find_objects_of_class(api, cache, cache.tmp_klass) }
}

/// Read a Text's current content.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn get_text(
    api: &Il2cppApi,
    cache: &UnityCache,
    text_obj: *mut std::ffi::c_void,
) -> Option<String> {
    if cache.m_get_text.is_null() {
        return None;
    }
    // SAFETY: getter with no args; string copied out immediately.
    let s = unsafe { il2cpp::invoke(api, cache.m_get_text, text_obj, &[]) }?;
    unsafe { il2cpp::read_string(api, s) }
}

/// Overwrite a Text's content. Returns success.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn set_text(
    api: &Il2cppApi,
    cache: &UnityCache,
    text_obj: *mut std::ffi::c_void,
    content: &str,
) -> bool {
    if cache.m_set_text.is_null() {
        return false;
    }
    // SAFETY: one managed-string argument, allocated just above.
    unsafe {
        let s = match il2cpp::new_string(api, content) {
            Some(v) => v,
            None => return false,
        };
        let params = [s];
        // Void callee: `invoke_void`, never `invoke` (see its docs).
        il2cpp::invoke_void(api, cache.m_set_text, text_obj, &params)
    }
}

/// Read a TextMeshProUGUI's current content.
///
/// Same `text` property shape as uGUI `Text`, different cached method.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tmp_get_text(
    api: &Il2cppApi,
    cache: &UnityCache,
    text_obj: *mut std::ffi::c_void,
) -> Option<String> {
    if cache.m_tmp_get_text.is_null() {
        return None;
    }
    // SAFETY: getter with no args; string copied out immediately.
    let s = unsafe { il2cpp::invoke(api, cache.m_tmp_get_text, text_obj, &[]) }?;
    unsafe { il2cpp::read_string(api, s) }
}

/// Overwrite a TextMeshProUGUI's content. Returns success.
///
/// # Safety
/// Same contract as `super::init`.
pub unsafe fn tmp_set_text(
    api: &Il2cppApi,
    cache: &UnityCache,
    text_obj: *mut std::ffi::c_void,
    content: &str,
) -> bool {
    if cache.m_tmp_set_text.is_null() {
        return false;
    }
    // SAFETY: one managed-string argument, allocated just above.
    unsafe {
        let s = match il2cpp::new_string(api, content) {
            Some(v) => v,
            None => return false,
        };
        let params = [s];
        // Void callee: `invoke_void`, never `invoke` (see its docs).
        il2cpp::invoke_void(api, cache.m_tmp_set_text, text_obj, &params)
    }
}
