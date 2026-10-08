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
    /// `Object.GetInstanceID()` (optional — stable per-object id).
    pub m_get_instance_id: *mut std::ffi::c_void,
    /// `GameObject` class (optional — hierarchy + SetActive).
    pub go_klass: *mut std::ffi::c_void,
    /// `GameObject.get_transform()` (optional).
    pub m_go_get_transform: *mut std::ffi::c_void,
    /// `GameObject.SetActive(bool)` (optional).
    pub m_go_set_active: *mut std::ffi::c_void,
    /// `Transform` class (optional — hierarchy walk + position).
    pub tr_klass: *mut std::ffi::c_void,
    /// `Transform.get_childCount()` (optional).
    pub m_tr_get_child_count: *mut std::ffi::c_void,
    /// `Transform.GetChild(int)` (optional).
    pub m_tr_get_child: *mut std::ffi::c_void,
    /// `Transform.get_position()` (optional — boxed Vector3).
    pub m_tr_get_position: *mut std::ffi::c_void,
    /// `SceneManager.GetActiveScene()` (static, optional — root walk).
    pub m_scene_get_active: *mut std::ffi::c_void,
    /// `SceneManager.GetSceneCount()` (static, optional — all scenes).
    pub m_scene_get_count: *mut std::ffi::c_void,
    /// `SceneManager.GetSceneAt(int)` (static, optional — all scenes).
    pub m_scene_get_at: *mut std::ffi::c_void,
    /// `Scene.get_name()` (optional — scene labels).
    pub m_scene_get_name: *mut std::ffi::c_void,
    /// `Scene.GetRootGameObjects()` (optional — includes inactive roots).
    pub m_scene_get_roots: *mut std::ffi::c_void,
    /// `QualitySettings` class (optional — video tool).
    pub qs_klass: *mut std::ffi::c_void,
    /// `QualitySettings.GetQualityLevel()` (static, optional).
    pub m_qs_get_level: *mut std::ffi::c_void,
    /// `QualitySettings.SetQualityLevel(int)` (static, optional).
    pub m_qs_set_level: *mut std::ffi::c_void,
    /// `QualitySettings.get_names()` (static string[], optional).
    pub m_qs_get_names: *mut std::ffi::c_void,
    /// `QualitySettings.get_vSyncCount/set_vSyncCount` (static int).
    pub m_qs_get_vsync: *mut std::ffi::c_void,
    /// `QualitySettings.get_vSyncCount/set_vSyncCount` (static int).
    pub m_qs_set_vsync: *mut std::ffi::c_void,
    /// `QualitySettings.get_antiAliasing/set_antiAliasing` (static int).
    pub m_qs_get_aa: *mut std::ffi::c_void,
    /// `QualitySettings.get_antiAliasing/set_antiAliasing` (static int).
    pub m_qs_set_aa: *mut std::ffi::c_void,
    /// `QualitySettings.get_shadows/set_shadows` (static enum-as-int).
    pub m_qs_get_shadows: *mut std::ffi::c_void,
    /// `QualitySettings.get_shadows/set_shadows` (static enum-as-int).
    pub m_qs_set_shadows: *mut std::ffi::c_void,
    /// `Screen` class (optional — resolution tool).
    pub screen_klass: *mut std::ffi::c_void,
    /// `Screen.SetResolution(int,int,bool)` (static, optional — first
    /// 3-arg overload wins; the current fullscreen flag is passed back
    /// so behavior stays close whichever overload resolves).
    pub m_screen_set_resolution: *mut std::ffi::c_void,
    /// `Screen.get_currentResolution()` (static Resolution struct).
    pub m_screen_get_resolution: *mut std::ffi::c_void,
    /// `Screen.get_fullScreen/set_fullScreen` (static bool).
    pub m_screen_get_fullscreen: *mut std::ffi::c_void,
    /// `Screen.get_fullScreen/set_fullScreen` (static bool).
    pub m_screen_set_fullscreen: *mut std::ffi::c_void,
    /// `RenderSettings` class (optional — fog/ambient tool).
    pub rs_klass: *mut std::ffi::c_void,
    /// `RenderSettings.get_fog/set_fog` (static bool).
    pub m_rs_get_fog: *mut std::ffi::c_void,
    /// `RenderSettings.get_fog/set_fog` (static bool).
    pub m_rs_set_fog: *mut std::ffi::c_void,
    /// `RenderSettings.get_fogDensity/set_fogDensity` (static float).
    pub m_rs_get_fog_density: *mut std::ffi::c_void,
    /// `RenderSettings.get_fogDensity/set_fogDensity` (static float).
    pub m_rs_set_fog_density: *mut std::ffi::c_void,
    /// `RenderSettings.get_ambientIntensity/set_ambientIntensity`.
///
/// # Safety
/// Same contract as [`find_class`].
    pub m_rs_get_ambient: *mut std::ffi::c_void,
    /// `RenderSettings.get_ambientIntensity/set_ambientIntensity`.
///
/// # Safety
/// Same contract as [`find_class`].
    pub m_rs_set_ambient: *mut std::ffi::c_void,
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

        // Instance ids: Object.GetInstanceID, stable per object.
        let m_get_instance_id =
            il2cpp::get_method(api, obj_klass, "GetInstanceID", 0);

        // Hierarchy walk: GameObject roots via SceneManager (includes
        // inactive branches — FindObjectsOfType would skip them), then
        // Transform children down. Everything optional: a missing piece
        // degrades the inspector to "unavailable", never a crash.
        let scene_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine.SceneManagement", "SceneManager")
            .unwrap_or(std::ptr::null_mut());
        let m_scene_get_active = if scene_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, scene_klass, "GetActiveScene", 0)
        };
        let m_scene_get_count = if scene_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, scene_klass, "GetSceneCount", 0)
        };
        let m_scene_get_at = if scene_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, scene_klass, "GetSceneAt", 1)
        };
        // GetRootGameObjects lives on the Scene STRUCT (value type):
        // resolve against the struct class, invoke on unboxed data.
        let scene_struct = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine.SceneManagement", "Scene")
            .unwrap_or(std::ptr::null_mut());
        let m_scene_get_roots = if scene_struct.is_null() {
            None
        } else {
            il2cpp::get_method(api, scene_struct, "GetRootGameObjects", 0)
        };
        let m_scene_get_name = if scene_struct.is_null() {
            None
        } else {
            il2cpp::get_method(api, scene_struct, "get_name", 0)
        };
        let tr_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Transform")
            .unwrap_or(std::ptr::null_mut());
        let (m_tr_get_child_count, m_tr_get_child, m_tr_get_position) =
            if tr_klass.is_null() {
                (None, None, None)
            } else {
                (
                    il2cpp::get_method(api, tr_klass, "get_childCount", 0),
                    il2cpp::get_method(api, tr_klass, "GetChild", 1),
                    il2cpp::get_method(api, tr_klass, "get_position", 0),
                )
            };
        let (m_go_get_transform, m_go_set_active) = if go_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, go_klass, "get_transform", 0),
                il2cpp::get_method(api, go_klass, "SetActive", 1),
            )
        };

        // Video/quality knobs: pure static getters/setters, the same
        // invoke shapes as FOV/text (proven). Missing classes just
        // disable the video tool.
        let qs_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "QualitySettings")
            .unwrap_or(std::ptr::null_mut());
        let (m_qs_get_level, m_qs_set_level, m_qs_get_names) = if qs_klass.is_null() {
            (None, None, None)
        } else {
            (
                il2cpp::get_method(api, qs_klass, "GetQualityLevel", 0),
                il2cpp::get_method(api, qs_klass, "SetQualityLevel", 1),
                il2cpp::get_method(api, qs_klass, "get_names", 0),
            )
        };
        let (m_qs_get_vsync, m_qs_set_vsync) = if qs_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, qs_klass, "get_vSyncCount", 0),
                il2cpp::get_method(api, qs_klass, "set_vSyncCount", 1),
            )
        };
        let (m_qs_get_aa, m_qs_set_aa) = if qs_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, qs_klass, "get_antiAliasing", 0),
                il2cpp::get_method(api, qs_klass, "set_antiAliasing", 1),
            )
        };
        let (m_qs_get_shadows, m_qs_set_shadows) = if qs_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, qs_klass, "get_shadows", 0),
                il2cpp::get_method(api, qs_klass, "set_shadows", 1),
            )
        };
        let screen_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Screen")
            .unwrap_or(std::ptr::null_mut());
        let (m_screen_set_resolution, m_screen_get_resolution) = if screen_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, screen_klass, "SetResolution", 3),
                il2cpp::get_method(api, screen_klass, "get_currentResolution", 0),
            )
        };
        let (m_screen_get_fullscreen, m_screen_set_fullscreen) = if screen_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, screen_klass, "get_fullScreen", 0),
                il2cpp::get_method(api, screen_klass, "set_fullScreen", 1),
            )
        };
        let rs_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "RenderSettings")
            .unwrap_or(std::ptr::null_mut());
        let (m_rs_get_fog, m_rs_set_fog) = if rs_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, rs_klass, "get_fog", 0),
                il2cpp::get_method(api, rs_klass, "set_fog", 1),
            )
        };
        let (m_rs_get_fog_density, m_rs_set_fog_density) = if rs_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, rs_klass, "get_fogDensity", 0),
                il2cpp::get_method(api, rs_klass, "set_fogDensity", 1),
            )
        };
        let (m_rs_get_ambient, m_rs_set_ambient) = if rs_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, rs_klass, "get_ambientIntensity", 0),
                il2cpp::get_method(api, rs_klass, "set_ambientIntensity", 1),
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
            m_get_instance_id: m_get_instance_id.unwrap_or(std::ptr::null_mut()),
            go_klass,
            m_go_get_transform: m_go_get_transform.unwrap_or(std::ptr::null_mut()),
            m_go_set_active: m_go_set_active.unwrap_or(std::ptr::null_mut()),
            tr_klass,
            m_tr_get_child_count: m_tr_get_child_count.unwrap_or(std::ptr::null_mut()),
            m_tr_get_child: m_tr_get_child.unwrap_or(std::ptr::null_mut()),
            m_tr_get_position: m_tr_get_position.unwrap_or(std::ptr::null_mut()),
            m_scene_get_active: m_scene_get_active.unwrap_or(std::ptr::null_mut()),
            m_scene_get_count: m_scene_get_count.unwrap_or(std::ptr::null_mut()),
            m_scene_get_at: m_scene_get_at.unwrap_or(std::ptr::null_mut()),
            m_scene_get_name: m_scene_get_name.unwrap_or(std::ptr::null_mut()),
            m_scene_get_roots: m_scene_get_roots.unwrap_or(std::ptr::null_mut()),
            qs_klass,
            m_qs_get_level: m_qs_get_level.unwrap_or(std::ptr::null_mut()),
            m_qs_set_level: m_qs_set_level.unwrap_or(std::ptr::null_mut()),
            m_qs_get_names: m_qs_get_names.unwrap_or(std::ptr::null_mut()),
            m_qs_get_vsync: m_qs_get_vsync.unwrap_or(std::ptr::null_mut()),
            m_qs_set_vsync: m_qs_set_vsync.unwrap_or(std::ptr::null_mut()),
            m_qs_get_aa: m_qs_get_aa.unwrap_or(std::ptr::null_mut()),
            m_qs_set_aa: m_qs_set_aa.unwrap_or(std::ptr::null_mut()),
            m_qs_get_shadows: m_qs_get_shadows.unwrap_or(std::ptr::null_mut()),
            m_qs_set_shadows: m_qs_set_shadows.unwrap_or(std::ptr::null_mut()),
            screen_klass,
            m_screen_set_resolution: m_screen_set_resolution.unwrap_or(std::ptr::null_mut()),
            m_screen_get_resolution: m_screen_get_resolution.unwrap_or(std::ptr::null_mut()),
            m_screen_get_fullscreen: m_screen_get_fullscreen.unwrap_or(std::ptr::null_mut()),
            m_screen_set_fullscreen: m_screen_set_fullscreen.unwrap_or(std::ptr::null_mut()),
            rs_klass,
            m_rs_get_fog: m_rs_get_fog.unwrap_or(std::ptr::null_mut()),
            m_rs_set_fog: m_rs_set_fog.unwrap_or(std::ptr::null_mut()),
            m_rs_get_fog_density: m_rs_get_fog_density.unwrap_or(std::ptr::null_mut()),
            m_rs_set_fog_density: m_rs_set_fog_density.unwrap_or(std::ptr::null_mut()),
            m_rs_get_ambient: m_rs_get_ambient.unwrap_or(std::ptr::null_mut()),
            m_rs_set_ambient: m_rs_set_ambient.unwrap_or(std::ptr::null_mut()),
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

/// A component's `GameObject` (every `Transform` is one — the way back
/// from a child transform to its object while walking).
///
/// # Safety
/// Same contract as [`find_class`].
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

/// Raw `Application.systemLanguage` value (`SystemLanguage` int), or `None`.
///
/// Resolves the icall `UnityEngine.Application::get_systemLanguage` and
/// calls it directly — the getter is `extern`, so there is no managed
/// `MethodInfo` to invoke and no boxing/unboxing involved. Values follow
/// the Unity `SystemLanguage` order (English=10, French=14, German=15,
/// Spanish=33, Russian=29, …); the caller maps them (see `i18n`).
/// `None` when the export or the icall is missing.
///
/// # Safety
/// Same contract as [`find_class`]; call from an attached thread.
pub unsafe fn system_language_raw(api: &Il2cppApi) -> Option<i32> {
    let resolve = api.resolve_icall?;
    let name = std::ffi::CString::new("UnityEngine.Application::get_systemLanguage").ok()?;
    // SAFETY: validated export; icall getters take no arguments.
    let ptr = unsafe { resolve(name.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    type GetSystemLanguageFn = unsafe extern "C" fn() -> i32;
    let get: GetSystemLanguageFn = unsafe { std::mem::transmute(ptr) };
    // SAFETY: signature matches the engine icall (returns the enum int).
    Some(unsafe { get() })
}

// ---------------------------------------------------------------------------
// Small value-type call shapes (int / float / bool).
//
// Same invoke/unbox discipline as `get_fov`/`beh_get_enabled`, factored
// so the inspector and video tools don't re-derive the ABI each time.
// ---------------------------------------------------------------------------

/// Call a 0-arg getter returning a boxed `Int32`.
unsafe fn get_i32(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
) -> Option<i32> {
    if method.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: cached getter; boxed Int32 unboxed below.
    unsafe {
        let boxed = il2cpp::invoke(api, method, obj, &[])?;
        let p = api.object_unbox.unwrap()(boxed) as *const i32;
        if p.is_null() {
            None
        } else {
            Some(std::ptr::read_unaligned(p))
        }
    }
}

/// Call a 0-arg getter returning a boxed `Single` (float).
unsafe fn get_f32(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
) -> Option<f32> {
    if method.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: cached getter; boxed Single unboxed below.
    unsafe {
        let boxed = il2cpp::invoke(api, method, obj, &[])?;
        let p = api.object_unbox.unwrap()(boxed) as *const f32;
        if p.is_null() {
            None
        } else {
            Some(std::ptr::read_unaligned(p))
        }
    }
}

/// Call a 0-arg getter returning a boxed `Boolean`.
unsafe fn get_bool(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
) -> Option<bool> {
    if method.is_null() || api.object_unbox.is_none() {
        return None;
    }
    // SAFETY: cached getter; boxed Boolean unboxed below.
    unsafe {
        let boxed = il2cpp::invoke(api, method, obj, &[])?;
        let p = api.object_unbox.unwrap()(boxed) as *const u8;
        if p.is_null() {
            None
        } else {
            Some(std::ptr::read_unaligned(p) != 0)
        }
    }
}

/// Call a 1-arg `void` setter taking an `Int32` (passed by address).
unsafe fn set_i32(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
    v: i32,
) -> bool {
    if method.is_null() {
        return false;
    }
    let params = [&v as *const i32 as *mut std::ffi::c_void];
    // SAFETY: single int argument by address; void callee.
    unsafe { il2cpp::invoke_void(api, method, obj, &params) }
}

/// Call a 1-arg `void` setter taking a `Single` (passed by address).
unsafe fn set_f32(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
    v: f32,
) -> bool {
    if method.is_null() {
        return false;
    }
    let params = [&v as *const f32 as *mut std::ffi::c_void];
    // SAFETY: single float argument by address; void callee.
    unsafe { il2cpp::invoke_void(api, method, obj, &params) }
}

/// Read a just-returned managed object array into owned handles.
///
/// Same consume-immediately + cap discipline as `find_objects_of_type`.
unsafe fn read_object_array(
    api: &Il2cppApi,
    arr: *mut std::ffi::c_void,
    cap: usize,
) -> Vec<*mut std::ffi::c_void> {
    if arr.is_null() {
        return Vec::new();
    }
    // SAFETY: live array just returned; length bounds the walk below.
    unsafe {
        let (array_length, array_object_header_size) =
            match (api.array_length, api.array_object_header_size) {
                (Some(a), Some(h)) => (a, h),
                _ => return Vec::new(),
            };
        let n = (array_length(arr) as usize).min(cap);
        let hdr = array_object_header_size() as usize;
        let items = arr.add(hdr) as *mut *mut std::ffi::c_void;
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

// ---------------------------------------------------------------------------
// Object identity + hierarchy walk (scene inspector foundation).
// ---------------------------------------------------------------------------

/// `Object.get_name()` copied out (empty string when unreadable —
///
/// names feed the inspector tree, where `None` would drop the node).
///
/// # Safety
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
pub unsafe fn get_instance_id(
    api: &Il2cppApi,
    cache: &UnityCache,
    obj: *mut std::ffi::c_void,
) -> Option<i32> {
    // SAFETY: cached getter on a live object.
    unsafe { get_i32(api, cache.m_get_instance_id, obj) }
}

/// `GameObject.get_activeInHierarchy()`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn go_active(
    api: &Il2cppApi,
    cache: &UnityCache,
    go: *mut std::ffi::c_void,
) -> Option<bool> {
    // SAFETY: cached getter on a live object.
    unsafe { get_bool(api, cache.m_go_get_active, go) }
}

/// `GameObject.get_transform()`.
///
/// # Safety
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
pub unsafe fn tr_child_count(
    api: &Il2cppApi,
    cache: &UnityCache,
    tr: *mut std::ffi::c_void,
) -> usize {
    // SAFETY: cached getter on a live transform.
    unsafe { get_i32(api, cache.m_tr_get_child_count, tr).unwrap_or(0).max(0) as usize }
}

/// `Transform.GetChild(i)`.
///
/// # Safety
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
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

/// Root `GameObject`s of the active scene (includes inactive roots —
/// the reason this beats `FindObjectsOfType` for inspection).
///
/// # Safety
/// Same contract as [`find_class`], plus the array-consumed-immediately
/// rule. `Scene` is a value type: the instance call runs on the
/// unboxed payload, never on the box.
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

/// All loaded scenes as `(name, roots)`.
///
/// Games stack additive scenes (menu + track + cars + HUD live side by
/// side); walking only the active one hides most of the game. Capped
/// at 32 scenes.
///
/// # Safety
/// Same contract as [`find_class`], plus the array-consumed-immediately
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
        let count = get_i32(api, cache.m_scene_get_count, std::ptr::null_mut())
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

// ---------------------------------------------------------------------------
// Video / quality knobs: QualitySettings, Screen, RenderSettings.
// Static getters/setters only — same proven shapes as FOV/text.
// ---------------------------------------------------------------------------

/// `QualitySettings.GetQualityLevel()`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn quality_level(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { get_i32(api, cache.m_qs_get_level, std::ptr::null_mut()) }
}

/// `QualitySettings.SetQualityLevel(int)`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn set_quality_level(api: &Il2cppApi, cache: &UnityCache, level: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { set_i32(api, cache.m_qs_set_level, std::ptr::null_mut(), level) }
}

/// `QualitySettings.names` (level names, capped at 16).
///
/// # Safety
/// Same contract as [`find_class`].
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
        read_object_array(api, arr, 16)
            .into_iter()
            .filter_map(|s| il2cpp::read_string(api, s))
            .collect()
    }
}

/// `QualitySettings.get_vSyncCount/set_vSyncCount`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn vsync_count(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { get_i32(api, cache.m_qs_get_vsync, std::ptr::null_mut()) }
}

/// `QualitySettings.get_vSyncCount/set_vSyncCount`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn set_vsync_count(api: &Il2cppApi, cache: &UnityCache, v: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { set_i32(api, cache.m_qs_set_vsync, std::ptr::null_mut(), v) }
}

/// `QualitySettings.get_antiAliasing/set_antiAliasing` (0/2/4/8).
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn antialiasing(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { get_i32(api, cache.m_qs_get_aa, std::ptr::null_mut()) }
}

/// `QualitySettings.get_antiAliasing/set_antiAliasing` (0/2/4/8).
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn set_antialiasing(api: &Il2cppApi, cache: &UnityCache, v: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { set_i32(api, cache.m_qs_set_aa, std::ptr::null_mut(), v) }
}

/// `QualitySettings.get_shadows/set_shadows` (ShadowQuality as int).
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn shadows(api: &Il2cppApi, cache: &UnityCache) -> Option<i32> {
    // SAFETY: cached static getter.
    unsafe { get_i32(api, cache.m_qs_get_shadows, std::ptr::null_mut()) }
}

/// `QualitySettings.get_shadows/set_shadows` (ShadowQuality as int).
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn set_shadows(api: &Il2cppApi, cache: &UnityCache, v: i32) -> bool {
    // SAFETY: cached static setter, int by address.
    unsafe { set_i32(api, cache.m_qs_set_shadows, std::ptr::null_mut(), v) }
}

/// `Screen.currentResolution` as `(width, height, refresh_hz)`.
///
/// `Resolution` is 3 packed ints (width, height, refreshRate).
///
/// # Safety
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
pub unsafe fn screen_fullscreen(api: &Il2cppApi, cache: &UnityCache) -> Option<bool> {
    // SAFETY: cached static getter.
    unsafe { get_bool(api, cache.m_screen_get_fullscreen, std::ptr::null_mut()) }
}

/// `Screen.get_fullScreen/set_fullScreen`.
///
/// # Safety
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
pub unsafe fn render_fog(api: &Il2cppApi, cache: &UnityCache) -> Option<bool> {
    // SAFETY: cached static getter.
    unsafe { get_bool(api, cache.m_rs_get_fog, std::ptr::null_mut()) }
}

/// `RenderSettings.get_fog/set_fog`.
///
/// # Safety
/// Same contract as [`find_class`].
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
/// Same contract as [`find_class`].
pub unsafe fn render_fog_density(api: &Il2cppApi, cache: &UnityCache) -> Option<f32> {
    // SAFETY: cached static getter.
    unsafe { get_f32(api, cache.m_rs_get_fog_density, std::ptr::null_mut()) }
}

/// `RenderSettings.get_fogDensity/set_fogDensity`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn set_render_fog_density(api: &Il2cppApi, cache: &UnityCache, v: f32) -> bool {
    // SAFETY: cached static setter, float by address.
    unsafe { set_f32(api, cache.m_rs_set_fog_density, std::ptr::null_mut(), v) }
}

/// `RenderSettings.get_ambientIntensity/set_ambientIntensity`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn render_ambient(api: &Il2cppApi, cache: &UnityCache) -> Option<f32> {
    // SAFETY: cached static getter.
    unsafe { get_f32(api, cache.m_rs_get_ambient, std::ptr::null_mut()) }
}

/// `RenderSettings.get_ambientIntensity/set_ambientIntensity`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn set_render_ambient(api: &Il2cppApi, cache: &UnityCache, v: f32) -> bool {
    // SAFETY: cached static setter, float by address.
    unsafe { set_f32(api, cache.m_rs_set_ambient, std::ptr::null_mut(), v) }
}
