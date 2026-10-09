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

pub mod camera;
pub mod object;
pub mod scene;
pub mod text;
pub mod video;

/// First-hit method resolution over candidate spellings (managed-side
/// renames across Unity versions, e.g. `get_sceneCount` vs
/// `GetSceneCount`). Returns the pointer and the winning name
/// (`"MISSING"` when none resolves) for one-time diagnostics.
///
/// # Safety
/// Same contract as [`Il2cppApi::find_class`] callers: attached tick
/// thread, live class, resolution only.
unsafe fn get_method_any(
    api: &Il2cppApi,
    klass: *mut std::ffi::c_void,
    names: &[&'static str],
    argc: i32,
) -> (Option<*mut std::ffi::c_void>, &'static str) {
    for name in names {
        // SAFETY: attached tick thread; `get_method` null-checks.
        if let Some(m) = unsafe { il2cpp::get_method(api, klass, name, argc) } {
            return (Some(m), name);
        }
    }
    (None, "MISSING")
}

// Re-exported so `crate::unity::X` paths keep working after the split.
pub use camera::{
    cm_active_vcam_name, cm_brain_list, cm_composer_list, cm_orbital_list,
    cm_set_vcam_priority, cm_transposer_list, cm_vcam_follow_name, cm_vcam_list, cm_vcam_priority,
    get_fov, main_camera, set_fov,
};
pub use object::{
    beh_get_enabled, beh_set_enabled, component_active, component_gameobject, get_instance_id,
    go_active, go_find, go_get_component, go_set_active, go_transform, graphic_get_color,
    graphic_set_color, object_alive, object_name, tr_child, tr_child_count, tr_find,
    tr_get_rotation, tr_get_scale, tr_position, tr_set_position, tr_set_scale,
    all_transforms,
    tr_parent,
};
pub use scene::{loaded_scenes, scene_root_objects, transform_scene_name};
pub use text::{
    find_images, find_texts, find_tmp_texts, get_text, set_text, tmp_get_font_size, tmp_get_text,
    tmp_set_font_size, tmp_set_text,
};
pub use video::{
    antialiasing, quality_level, quality_names, render_ambient, render_fog, render_fog_density,
    screen_fullscreen, screen_resolution, screen_set_resolution, set_antialiasing,
    set_quality_level, set_render_ambient, set_render_fog, set_render_fog_density,
    set_screen_fullscreen, set_shadows, set_vsync_count, shadows, vsync_count,
};

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
    /// `Object.m_CachedPtr` field (optional — dead-object check: a null
    /// native pointer means the wrapper outlived its object, and any
    /// read/write on it crashes. Checked before every write).
    pub m_cached_ptr: *mut std::ffi::c_void,
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
    /// `TextMeshProUGUI.get_fontSize()` (0 args, optional).
    pub m_tmp_get_font_size: *mut std::ffi::c_void,
    /// `TextMeshProUGUI.set_fontSize(float)` (1 arg, optional).
    pub m_tmp_set_font_size: *mut std::ffi::c_void,
    /// `UnityEngine.UI.Graphic` class (optional — color live here for
    /// sprites (`Image`) and TMP texts alike).
    pub graphic_klass: *mut std::ffi::c_void,
    /// `UnityEngine.UI.Image` class (optional — sprite enumeration).
    pub image_klass: *mut std::ffi::c_void,
    /// `Graphic.get_color()` (0 args, optional — boxed Color).
    pub m_graphic_get_color: *mut std::ffi::c_void,
    /// `Graphic.set_color(Color)` (1 arg, optional).
    pub m_graphic_set_color: *mut std::ffi::c_void,
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
    /// `GameObject.Find(name)` (static, 1 string arg, optional).
    pub m_go_find: *mut std::ffi::c_void,
    /// `GameObject.GetComponent(Type)` (1 arg, optional — see
    /// `go_get_component` for the overload note).
    pub m_go_get_component: *mut std::ffi::c_void,
    /// `Transform` class (optional — hierarchy walk + position).
    pub tr_klass: *mut std::ffi::c_void,
    /// `Transform.get_childCount()` (optional).
    pub m_tr_get_child_count: *mut std::ffi::c_void,
    /// `Transform.GetChild(int)` (optional).
    pub m_tr_get_child: *mut std::ffi::c_void,
    /// `Transform.get_parent()` (optional — root climbing for the
    /// unparented sweep; null = scene root).
    pub m_tr_get_parent: *mut std::ffi::c_void,
    /// `Transform.Find(name)` (1 string arg, optional).
    pub m_tr_find: *mut std::ffi::c_void,
    /// `Transform.get_localScale()` (optional — boxed Vector3).
    pub m_tr_get_local_scale: *mut std::ffi::c_void,
    /// `Transform.set_localScale(Vector3)` (optional — scale-zero hiding).
    pub m_tr_set_local_scale: *mut std::ffi::c_void,
    /// `Transform.get_position()` (optional — boxed Vector3).
    pub m_tr_get_position: *mut std::ffi::c_void,
    /// `Transform.set_position(Vector3)` (optional — freecam drive).
    pub m_tr_set_position: *mut std::ffi::c_void,
    /// `Transform.get_rotation()` (optional — boxed Quaternion).
    pub m_tr_get_rotation: *mut std::ffi::c_void,
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
    /// `Scene` struct class (optional — validity checks on values).
    pub scene_struct_klass: *mut std::ffi::c_void,
    /// `Scene.IsValid()` (0 args, optional — DontDestroyOnLoad filter).
    pub m_scene_is_valid: *mut std::ffi::c_void,
    /// `GameObject.get_scene()` (0 args, optional — scene of an object).
    pub m_go_get_scene: *mut std::ffi::c_void,
    /// `UnityEngine.Resources` class (optional — FindObjectsOfTypeAll).
    pub resources_klass: *mut std::ffi::c_void,
    /// `Resources.FindObjectsOfTypeAll(Type)` (static, 1 arg, optional —
    /// includes inactive objects and assets; manual refreshes only).
    pub m_find_all_objects: *mut std::ffi::c_void,
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
    pub m_rs_get_ambient: *mut std::ffi::c_void,
    /// `RenderSettings.get_ambientIntensity/set_ambientIntensity`.
    pub m_rs_set_ambient: *mut std::ffi::c_void,
    // -- Cinemachine v2 (optional — camera tool). The game drives its
    // cameras through `CinemachineVirtualCamera` + Brain (see log:
    // `RearRaceCamera`, `UpdateVirtualCameras`). Offsets and damping
    // are raw FIELDS (`m_FollowOffset`, `m_XDamping`…), so the cache
    // holds `FieldInfo*` resolved once per class; reads/writes go
    // through `il2cpp::field_get/set_*` with read-back.
    /// `CinemachineBrain` class.
    pub cm_brain_klass: *mut std::ffi::c_void,
    /// `CinemachineBrain.get_ActiveVirtualCamera()`.
    pub m_brain_get_active_vcam: *mut std::ffi::c_void,
    /// `CinemachineVirtualCamera` class.
    pub cm_vcam_klass: *mut std::ffi::c_void,
    /// `CinemachineVirtualCamera.get_Follow/get_LookAt`.
    pub m_vcam_get_follow: *mut std::ffi::c_void,
    /// `CinemachineVirtualCamera.get_Follow/get_LookAt`.
    pub m_vcam_get_lookat: *mut std::ffi::c_void,
    /// `CinemachineVirtualCamera.get_Priority/set_Priority`.
    pub m_vcam_get_priority: *mut std::ffi::c_void,
    /// `CinemachineVirtualCamera.get_Priority/set_Priority`.
    pub m_vcam_set_priority: *mut std::ffi::c_void,
    /// `CinemachineTransposer` class + `m_FollowOffset` field.
    pub cm_trans_klass: *mut std::ffi::c_void,
    /// `CinemachineTransposer` class + `m_FollowOffset` field.
    pub m_trans_offset: *mut std::ffi::c_void,
    /// `m_XDamping/m_YDamping/m_ZDamping` fields (transposer).
    pub m_trans_damp: [*mut std::ffi::c_void; 3],
    /// `CinemachineOrbitalTransposer` class + `m_FollowOffset` field.
    pub cm_orbital_klass: *mut std::ffi::c_void,
    /// `CinemachineOrbitalTransposer` class + `m_FollowOffset` field.
    pub m_orbital_offset: *mut std::ffi::c_void,
    /// `m_XDamping/m_YDamping/m_ZDamping` fields (orbital).
    pub m_orbital_damp: [*mut std::ffi::c_void; 3],
    /// `CinemachineComposer` class + `m_TrackedObjectOffset` field.
    pub cm_composer_klass: *mut std::ffi::c_void,
    /// `CinemachineComposer` class + `m_TrackedObjectOffset` field.
    pub m_composer_tracked: *mut std::ffi::c_void,
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
        let (m_tmp_get_font_size, m_tmp_set_font_size) = if tmp_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, tmp_klass, "get_fontSize", 0),
                il2cpp::get_method(api, tmp_klass, "set_fontSize", 1),
            )
        };

        // UI styling is optional: `Graphic` carries `color` for sprites
        // (`Image`) and TMP texts alike; `Image` is enumerated like TMP.
        // `get_method` finds inherited members (proven: TMP `text`
        // resolves on the concrete class), so one pair covers both.
        let graphic_klass = il2cpp::find_class(api, domain, Some("UnityEngine.UI"), "UnityEngine.UI", "Graphic")
            .unwrap_or(std::ptr::null_mut());
        let (m_graphic_get_color, m_graphic_set_color) = if graphic_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, graphic_klass, "get_color", 0),
                il2cpp::get_method(api, graphic_klass, "set_color", 1),
            )
        };
        let image_klass = il2cpp::find_class(api, domain, Some("UnityEngine.UI"), "UnityEngine.UI", "Image")
            .unwrap_or(std::ptr::null_mut());

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
        let m_go_get_scene = if go_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, go_klass, "get_scene", 0)
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

        // Dead-object check: Object.m_CachedPtr, the native pointer
        // behind every wrapper. Null = the native object is gone and
        // any read/write on the wrapper crashes (proven on camera
        // writes during view churn). Resolved once like a method.
        // SAFETY: live class + static field name (outer unsafe).
        let m_cached_ptr = il2cpp::find_field(api, obj_klass, "m_CachedPtr");

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
        let (m_scene_get_count, count_name) = if scene_klass.is_null() {
            (None, "MISSING")
        } else {
            // Property is `sceneCount`: the getter is `get_sceneCount`
            // (il2cpp metadata keeps the lowercase-s spelling); try
            // the PascalCase guess too, first hit wins.
            get_method_any(api, scene_klass, &["get_sceneCount", "GetSceneCount"], 0)
        };
        let (m_scene_get_at, at_name) = if scene_klass.is_null() {
            (None, "MISSING")
        } else {
            get_method_any(api, scene_klass, &["GetSceneAt"], 1)
        };
        // One line per process: which spelling the build answered.
        static SCENE_API_LOGGED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if !SCENE_API_LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            crate::log_line(&format!(
                "unity: scene api count={count_name} at={at_name}"
            ));
        }
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
        let m_scene_is_valid = if scene_struct.is_null() {
            None
        } else {
            il2cpp::get_method(api, scene_struct, "IsValid", 0)
        };
        // `Resources.FindObjectsOfTypeAll` reaches what SceneManager
        // cannot (DontDestroyOnLoad, inactive branches). One walk per
        // MANUAL refresh — never periodic (each walk stutters).
        let resources_klass = il2cpp::find_class(api, domain, Some("UnityEngine.CoreModule"), "UnityEngine", "Resources")
            .unwrap_or(std::ptr::null_mut());
        let m_find_all_objects = if resources_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, resources_klass, "FindObjectsOfTypeAll", 1)
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
        // Position setter + rotation getter (one-shot Inspector
        // writes on pinned handles).
        let (m_tr_set_position, m_tr_get_rotation) =
            if tr_klass.is_null() {
                (None, None)
            } else {
                (
                    il2cpp::get_method(api, tr_klass, "set_position", 1),
                    il2cpp::get_method(api, tr_klass, "get_rotation", 0),
                )
            };
        let m_tr_get_parent = if tr_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, tr_klass, "get_parent", 0)
        };
        // HUD scale-zero hiding: child lookup + localScale get/set.
        // Same struct-by-address shapes as the setters above.
        let (m_tr_find, m_tr_get_local_scale, m_tr_set_local_scale) =
            if tr_klass.is_null() {
                (None, None, None)
            } else {
                (
                    il2cpp::get_method(api, tr_klass, "Find", 1),
                    il2cpp::get_method(api, tr_klass, "get_localScale", 0),
                    il2cpp::get_method(api, tr_klass, "set_localScale", 1),
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
        let m_go_find = if go_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, go_klass, "Find", 1)
        };
        let m_go_get_component = if go_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, go_klass, "GetComponent", 1)
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

        // Cinemachine v2: assembly-agnostic walk (package assembly name
        // varies — same approach as TMP). Everything optional; a
        // missing piece degrades the camera tool, never the cache.
        // SAFETY: attached thread, null-checked chain (outer unsafe).
        let cm_brain_klass = il2cpp::find_class(api, domain, None, "Cinemachine", "CinemachineBrain")
            .unwrap_or(std::ptr::null_mut());
        let m_brain_get_active_vcam = if cm_brain_klass.is_null() {
            None
        } else {
            il2cpp::get_method(api, cm_brain_klass, "get_ActiveVirtualCamera", 0)
        };
        let cm_vcam_klass = il2cpp::find_class(api, domain, None, "Cinemachine", "CinemachineVirtualCamera")
            .unwrap_or(std::ptr::null_mut());
        let (m_vcam_get_follow, m_vcam_get_lookat) = if cm_vcam_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, cm_vcam_klass, "get_Follow", 0),
                il2cpp::get_method(api, cm_vcam_klass, "get_LookAt", 0),
            )
        };
        let (m_vcam_get_priority, m_vcam_set_priority) = if cm_vcam_klass.is_null() {
            (None, None)
        } else {
            (
                il2cpp::get_method(api, cm_vcam_klass, "get_Priority", 0),
                il2cpp::get_method(api, cm_vcam_klass, "set_Priority", 1),
            )
        };
        // Offsets/damping are FIELDS: resolve FieldInfo once per class.
        let cm_trans_klass = il2cpp::find_class(api, domain, None, "Cinemachine", "CinemachineTransposer")
            .unwrap_or(std::ptr::null_mut());
        let m_trans_offset = if cm_trans_klass.is_null() {
            None
        } else {
            // SAFETY: live class + static name.
            il2cpp::find_field(api, cm_trans_klass, "m_FollowOffset")
        };
        let m_trans_damp = if cm_trans_klass.is_null() {
            [None, None, None]
        } else {
            // SAFETY: live class + static names.
            [
                il2cpp::find_field(api, cm_trans_klass, "m_XDamping"),
                il2cpp::find_field(api, cm_trans_klass, "m_YDamping"),
                il2cpp::find_field(api, cm_trans_klass, "m_ZDamping"),
            ]
        };
        let cm_orbital_klass = il2cpp::find_class(api, domain, None, "Cinemachine", "CinemachineOrbitalTransposer")
            .unwrap_or(std::ptr::null_mut());
        let m_orbital_offset = if cm_orbital_klass.is_null() {
            None
        } else {
            // SAFETY: live class + static name.
            il2cpp::find_field(api, cm_orbital_klass, "m_FollowOffset")
        };
        let m_orbital_damp = if cm_orbital_klass.is_null() {
            [None, None, None]
        } else {
            // SAFETY: live class + static names.
            [
                il2cpp::find_field(api, cm_orbital_klass, "m_XDamping"),
                il2cpp::find_field(api, cm_orbital_klass, "m_YDamping"),
                il2cpp::find_field(api, cm_orbital_klass, "m_ZDamping"),
            ]
        };
        let cm_composer_klass = il2cpp::find_class(api, domain, None, "Cinemachine", "CinemachineComposer")
            .unwrap_or(std::ptr::null_mut());
        let m_composer_tracked = if cm_composer_klass.is_null() {
            None
        } else {
            // SAFETY: live class + static name.
            il2cpp::find_field(api, cm_composer_klass, "m_TrackedObjectOffset")
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
            m_tmp_get_font_size: m_tmp_get_font_size.unwrap_or(std::ptr::null_mut()),
            m_tmp_set_font_size: m_tmp_set_font_size.unwrap_or(std::ptr::null_mut()),
            graphic_klass,
            m_graphic_get_color: m_graphic_get_color.unwrap_or(std::ptr::null_mut()),
            m_graphic_set_color: m_graphic_set_color.unwrap_or(std::ptr::null_mut()),
            image_klass,
            m_comp_get_gameobject: m_comp_get_gameobject.unwrap_or(std::ptr::null_mut()),
            m_go_get_active: m_go_get_active.unwrap_or(std::ptr::null_mut()),
            m_go_get_scene: m_go_get_scene.unwrap_or(std::ptr::null_mut()),
            m_beh_set_enabled: m_beh_set_enabled.unwrap_or(std::ptr::null_mut()),
            m_beh_get_enabled: m_beh_get_enabled.unwrap_or(std::ptr::null_mut()),
            m_get_key,
            m_get_instance_id: m_get_instance_id.unwrap_or(std::ptr::null_mut()),
            m_cached_ptr: m_cached_ptr.unwrap_or(std::ptr::null_mut()),
            go_klass,
            m_go_get_transform: m_go_get_transform.unwrap_or(std::ptr::null_mut()),
            m_go_set_active: m_go_set_active.unwrap_or(std::ptr::null_mut()),
            m_go_get_component: m_go_get_component.unwrap_or(std::ptr::null_mut()),
            m_go_find: m_go_find.unwrap_or(std::ptr::null_mut()),
            tr_klass,
            m_tr_get_child_count: m_tr_get_child_count.unwrap_or(std::ptr::null_mut()),
            m_tr_get_child: m_tr_get_child.unwrap_or(std::ptr::null_mut()),
            m_tr_get_parent: m_tr_get_parent.unwrap_or(std::ptr::null_mut()),
            m_tr_find: m_tr_find.unwrap_or(std::ptr::null_mut()),
            m_tr_get_local_scale: m_tr_get_local_scale.unwrap_or(std::ptr::null_mut()),
            m_tr_set_local_scale: m_tr_set_local_scale.unwrap_or(std::ptr::null_mut()),
            m_tr_get_position: m_tr_get_position.unwrap_or(std::ptr::null_mut()),
            m_tr_set_position: m_tr_set_position.unwrap_or(std::ptr::null_mut()),
            m_tr_get_rotation: m_tr_get_rotation.unwrap_or(std::ptr::null_mut()),
            m_scene_get_active: m_scene_get_active.unwrap_or(std::ptr::null_mut()),
            m_scene_get_count: m_scene_get_count.unwrap_or(std::ptr::null_mut()),
            m_scene_get_at: m_scene_get_at.unwrap_or(std::ptr::null_mut()),
            m_scene_get_name: m_scene_get_name.unwrap_or(std::ptr::null_mut()),
            m_scene_get_roots: m_scene_get_roots.unwrap_or(std::ptr::null_mut()),
            scene_struct_klass: scene_struct,
            m_scene_is_valid: m_scene_is_valid.unwrap_or(std::ptr::null_mut()),
            resources_klass,
            m_find_all_objects: m_find_all_objects.unwrap_or(std::ptr::null_mut()),
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
            cm_brain_klass,
            m_brain_get_active_vcam: m_brain_get_active_vcam.unwrap_or(std::ptr::null_mut()),
            cm_vcam_klass,
            m_vcam_get_follow: m_vcam_get_follow.unwrap_or(std::ptr::null_mut()),
            m_vcam_get_lookat: m_vcam_get_lookat.unwrap_or(std::ptr::null_mut()),
            m_vcam_get_priority: m_vcam_get_priority.unwrap_or(std::ptr::null_mut()),
            m_vcam_set_priority: m_vcam_set_priority.unwrap_or(std::ptr::null_mut()),
            cm_trans_klass,
            m_trans_offset: m_trans_offset.unwrap_or(std::ptr::null_mut()),
            m_trans_damp: m_trans_damp.map(|f| f.unwrap_or(std::ptr::null_mut())),
            cm_orbital_klass,
            m_orbital_offset: m_orbital_offset.unwrap_or(std::ptr::null_mut()),
            m_orbital_damp: m_orbital_damp.map(|f| f.unwrap_or(std::ptr::null_mut())),
            cm_composer_klass,
            m_composer_tracked: m_composer_tracked.unwrap_or(std::ptr::null_mut()),
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

/// All live instances of one class via `Object.FindObjectsOfType`.
///
/// Shared core behind [`find_texts`] and [`find_tmp_texts`]: the only
/// difference is which cached class we enumerate.
///
/// # Safety
/// Same contract as `il2cpp::find_class`, plus the array-consumed-
/// immediately rule from `il2cpp::find_objects_of_type`.
pub(crate) unsafe fn find_objects_of_class(
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

/// Same walk through `Resources.FindObjectsOfTypeAll`: includes
/// inactive objects (and assets) that `FindObjectsOfType` skips —
/// the only way to see DontDestroyOnLoad content. One walk per
/// MANUAL refresh, never periodic (each walk stutters the game).
///
/// # Safety
/// Same contract as [`find_objects_of_class`].
pub(crate) unsafe fn find_all_objects_of_class(
    api: &Il2cppApi,
    cache: &UnityCache,
    klass: *mut std::ffi::c_void,
) -> Vec<*mut std::ffi::c_void> {
    if klass.is_null() || cache.m_find_all_objects.is_null() {
        return Vec::new();
    }
    // SAFETY: full chain null-checked step by step.
    unsafe {
        let type_obj = match il2cpp::type_object_for_class(api, klass) {
            Some(t) => t,
            None => return Vec::new(),
        };
        let (items, n) =
            match il2cpp::find_objects_of_type(api, cache.m_find_all_objects, type_obj) {
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
pub(crate) unsafe fn get_i32(
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
pub(crate) unsafe fn get_f32(
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
pub(crate) unsafe fn get_bool(
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
pub(crate) unsafe fn set_i32(
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
pub(crate) unsafe fn set_f32(
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
pub(crate) unsafe fn read_object_array(
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
