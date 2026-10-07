//! Unity-facing helpers built on the [`crate::il2cpp`] bridge.
//!
//! # What this is
//!
//! Thin, fallible wrappers around UnityEngine classes, mirroring proven
//! community-mod practice: resolve classes once (`UnityEngine.CoreModule`
//! et al.), cache `MethodInfo*`, invoke sparingly. Everything returns
//! `Option`/bool — a missing class or a mid-load scene degrades to
//! "try again later", never a crash.
//!
//! # Threading
//!
//! Call only from threads attached to IL2CPP (the framework's init and
//! tick threads already are). Object handles go stale across scene
//! changes: re-resolve objects often (cheap single invokes), cache only
//! classes and methods (stable for the process lifetime).

use crate::il2cpp::{self, Il2cppApi};

/// Cached Unity classes + methods. Classes are process-stable and safe
/// to keep; object handles are NOT (re-resolve per use).
#[derive(Debug, Clone, Copy)]
pub struct UnityCache {
    /// `UnityEngine.Camera` class.
    pub cam_klass: *mut std::ffi::c_void,
    /// `Camera.get_main()` (static, 0 args).
    pub m_cam_main: *mut std::ffi::c_void,
    /// `Camera.get_fieldOfView()` (0 args, boxed float).
    pub m_get_fov: *mut std::ffi::c_void,
    /// `Camera.set_fieldOfView(float)` (1 arg).
    pub m_set_fov: *mut std::ffi::c_void,
    /// `UnityEngine.UI.Text` class (optional: absent on builds stripping UI).
    pub text_klass: *mut std::ffi::c_void,
    /// `Object.FindObjectsOfType(Type)` (static, 1 arg).
    pub m_find_objects: *mut std::ffi::c_void,
    /// `Object.get_name()` (0 args).
    pub m_get_name: *mut std::ffi::c_void,
    /// `Text.get_text()` (0 args).
    pub m_get_text: *mut std::ffi::c_void,
    /// `Text.set_text(string)` (1 arg).
    pub m_set_text: *mut std::ffi::c_void,
    /// `TMPro.TextMeshProUGUI` class (optional — null when the game uses
    /// legacy uGUI text or no text at all; see `find_tmp_texts`).
    pub tmp_klass: *mut std::ffi::c_void,
    /// `TextMeshProUGUI.get_text()` (0 args, optional).
    pub m_tmp_get_text: *mut std::ffi::c_void,
    /// `TextMeshProUGUI.set_text(string)` (1 arg, optional).
    pub m_tmp_set_text: *mut std::ffi::c_void,
    /// `Component.get_gameObject()` (optional — visibility diagnosis).
    pub m_comp_get_gameobject: *mut std::ffi::c_void,
    /// `GameObject.get_activeInHierarchy()` (optional — same).
    pub m_go_get_active: *mut std::ffi::c_void,
    /// `Behaviour.set_enabled(bool)` (optional — hide/show writes).
    pub m_beh_set_enabled: *mut std::ffi::c_void,
    /// `Behaviour.get_enabled()` (optional — read-back).
    pub m_beh_get_enabled: *mut std::ffi::c_void,
    /// `Input.GetKey(KeyCode)` (static, 1 arg; optional — null when the
    /// Input class can't be resolved, in which case `key_held` is false).
    pub m_get_key: *mut std::ffi::c_void,
}

/// `KeyCode.O` as Unity defines it (letters follow ASCII: `A = 97 … Z = 122`).
pub const KEYCODE_O: i32 = 111;
/// `KeyCode.F9` (`F1 = 282`, so `F9 = 290`) — UI capture toggle.
pub const KEYCODE_F9: i32 = 290;
/// `KeyCode.F8` (`F8 = 289`) — combined UI + capture toggle.
pub const KEYCODE_F8: i32 = 289;

// SAFETY: raw handles are just addresses; synchronized by callers.
unsafe impl Send for UnityCache {}
unsafe impl Sync for UnityCache {}

/// Resolve Unity classes + methods. Needs domain + attached thread.
/// Missing optional pieces (UI `Text`) zero out instead of failing;
/// missing camera support fails the whole cache (nothing to drive).
///
/// # Safety
/// Same contract as `il2cpp::find_class`.
pub unsafe fn init(
    api: &Il2cppApi,
    domain: *mut std::ffi::c_void,
) -> Option<UnityCache> {
    // SAFETY: all calls below go through validated helpers on an
    // attached thread with a live domain.
    unsafe {
        let obj_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Object")?;
        let cam_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Camera")?;
        let m_find_objects = il2cpp::get_method(api, obj_klass, "FindObjectsOfType", 1)?;
        let m_get_name = il2cpp::get_method(api, obj_klass, "get_name", 0);
        let m_cam_main = il2cpp::get_method(api, cam_klass, "get_main", 0)?;
        let m_get_fov = il2cpp::get_method(api, cam_klass, "get_fieldOfView", 0);
        let m_set_fov = il2cpp::get_method(api, cam_klass, "set_fieldOfView", 1)?;

        // UI text is optional: present in full builds, stripped in some.
        let text_klass = il2cpp::find_class(api, domain, Some("UnityEngine.UI"), "UnityEngine.UI", "Text").unwrap_or(std::ptr::null_mut());
        let (m_get_text, m_set_text) = if text_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, text_klass, "get_text", 0),
                il2cpp::get_method(api, text_klass, "set_text", 1),
            )
        };

        // Input polling is optional: the SDL path (see `hotkey`) stays as
        // fallback, so a missing Input class degrades instead of failing.
        let m_get_key = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Input")
            .and_then(|input_klass| il2cpp::get_method(api, input_klass, "GetKey", 1))
            .unwrap_or(std::ptr::null_mut());

        // TextMeshPro is optional and resolved by full image walk (no
        // assembly-name guess: the TMP package assembly name varies).
        // Modern Unity games (this one included — zero legacy Texts
        // in-game) render UI text through TMP, not uGUI.
        let tmp_klass = il2cpp::find_class(api, domain, None, "TMPro", "TextMeshProUGUI")
            .unwrap_or(std::ptr::null_mut());
        let (m_tmp_get_text, m_tmp_set_text) = if tmp_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, tmp_klass, "get_text", 0),
                il2cpp::get_method(api, tmp_klass, "set_text", 1),
            )
        };

        // Component/GameObject state: optional visibility diagnosis
        // (an enumerated object may live on a disabled branch — writing
        // to it then changes memory nobody renders).
        let comp_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Component")
            .unwrap_or(std::ptr::null_mut());
        let go_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "GameObject")
            .unwrap_or(std::ptr::null_mut());
        let m_comp_get_gameobject = if comp_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, comp_klass, "get_gameObject", 0)
        };
        let m_go_get_active = if go_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, go_klass, "get_activeInHierarchy", 0)
        };
        // Behaviour on/off: the surgical hide switch (component-level,
        // no strings, no mesh rebuild — the renderer checks the flag).
        let beh_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Behaviour")
            .unwrap_or(std::ptr::null_mut());
        let (m_beh_set_enabled, m_beh_get_enabled) = if beh_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, beh_klass, "set_enabled", 1),
                il2cpp::get_method(api, beh_klass, "get_enabled", 0),
            )
        };

        Some(UnityCache {
            cam_klass,
            m_cam_main,
            m_get_fov: m_get_fov.unwrap_or(std::ptr::null_mut()),
            m_set_fov,
            text_klass,
            m_find_objects,
            m_get_name: m_get_name.unwrap_or(std::ptr::null_mut()),
            m_get_text: m_get_text.unwrap_or(std::ptr::null_mut()),
            m_set_text: m_set_text.unwrap_or(std::ptr::null_mut()),
            tmp_klass,
            m_tmp_get_text: m_tmp_get_text.unwrap_or(std::ptr::null_mut()),
            m_tmp_set_text: m_tmp_set_text.unwrap_or(std::ptr::null_mut()),
            m_comp_get_gameobject: m_comp_get_gameobject.unwrap_or(std::ptr::null_mut()),
            m_go_get_active: m_go_get_active.unwrap_or(std::ptr::null_mut()),
            m_beh_set_enabled: m_beh_set_enabled.unwrap_or(std::ptr::null_mut()),
            m_beh_get_enabled: m_beh_get_enabled.unwrap_or(std::ptr::null_mut()),
            m_get_key,
        })
    }
}

/// Whether a Unity `KeyCode` is currently held (`Input.GetKey` level).
///
/// Level — not `GetKeyDown` — on purpose: the Down flag lives a single
/// 16 ms frame and our poll could straddle it; the level plus our own
/// edge detector (`hotkey::poll_unity_edge`) never misses a human press.
/// Returns false when the Input method is unavailable.
///
/// # Safety
/// Same contract as `il2cpp::find_class`.
pub unsafe fn key_held(
    api: &Il2cppApi,
    cache: &UnityCache,
    keycode: i32,
) -> bool {
    if cache.m_get_key.is_null() {
        return false;
    }
    // SAFETY: cached static, one int value-type argument passed by
    // address; boxed Boolean unboxed below.
    unsafe {
        let mut key = keycode;
        let params = [&mut key as *mut i32 as *mut std::ffi::c_void];
        let boxed = match il2cpp::invoke(api, cache.m_get_key, std::ptr::null_mut(), &params) {
            Some(b) => b,
            None => return false,
        };
        match api.object_unbox {
            Some(unbox) => {
                let p = unbox(boxed) as *const u8;
                !p.is_null() && std::ptr::read_unaligned(p) != 0
            }
            None => false,
        }
    }
}

/// Current main camera (`Camera.main`), or `None` (no camera yet — menu,
/// loading screen — or API missing).
///
/// # Safety
/// Same contract as `il2cpp::find_class`.
pub unsafe fn main_camera(api: &Il2cppApi, cache: &UnityCache) -> Option<*mut std::ffi::c_void> {
    // SAFETY: cached static getter, 0 args, null receiver.
    let cam = unsafe { il2cpp::invoke(api, cache.m_cam_main, std::ptr::null_mut(), &[]) };
    match cam {
        Some(c) if !c.is_null() => Some(c),
        _ => None,
    }
}

/// Read a camera's field of view in degrees.
///
/// # Safety
/// Same contract as `il2cpp::find_class`.
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
/// Same contract as `il2cpp::find_class`.
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

/// All live instances of one class via `Object.FindObjectsOfType`.
///
/// Shared core behind [`find_texts`] and [`find_tmp_texts`]: the only
/// difference is which cached class we enumerate.
///
/// # Safety
/// Same contract as `il2cpp::find_class`, plus the array-consumed-
/// immediately rule from `il2cpp::find_objects_of_type`.
unsafe fn find_objects_of_class(
    api: &Il2cppApi,
    cache: &UnityCache,
    klass: *mut std::ffi::c_void,
) -> Vec<*mut std::ffi::c_void> {
    if klass.is_null() {
        return Vec::new();
    }
    // SAFETY: full chain null-checked step by step.
    unsafe {
        let type_obj = match il2cpp::type_object_for_class(api, klass) {
            Some(t) => t,
            None => return Vec::new(),
        };
        let (items, n) =
            match il2cpp::find_objects_of_type(api, cache.m_find_objects, type_obj) {
                Some(v) => v,
                None => return Vec::new(),
            };
        if items.is_null() || n == 0 {
            return Vec::new();
        }
        // Cap the walk: a broken count must not read the world.
        let n = (n as usize).min(4096);
        let mut out = Vec::with_capacity(n.min(64));
        for i in 0..n {
            let o = *items.add(i);
            if !o.is_null() {
                out.push(o);
            }
        }
        out
    }
}

/// All live `UnityEngine.UI.Text` instances (possibly empty — modern
/// games use TextMeshPro instead; see [`find_tmp_texts`).
///
/// # Safety
/// Same contract as `il2cpp::find_class`, plus the array-consumed-
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
/// Same contract as `il2cpp::find_class`.
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
/// Same contract as `il2cpp::find_class`.
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
/// Same contract as `il2cpp::find_class`.
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
/// Same contract as `il2cpp::find_class`.
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

/// Whether a component's GameObject is active in the hierarchy.
///
/// An enumerated component may sit on a disabled branch: writes to it
/// change memory nobody renders. Returns `None` when undeterminable.
///
/// # Safety
/// Same contract as `il2cpp::find_class`.
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
/// Same contract as `il2cpp::find_class`.
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
/// Same contract as `il2cpp::find_class`.
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
