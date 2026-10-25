//! Shared FMOD plumbing: find the already-loaded `libfmodstudio.so`,
//! resolve symbols, and locate the game's `FMOD.Studio.System` handle.
//!
//! Used by the mods that talk to FMOD (`engine_audio`, `fpv_drone`).
//! Nothing here loads or unloads a library: the game owns FMOD.

use crate::unity::UnityCache;
use std::ffi::{CStr, CString, c_void};

/// Already-loaded `libfmodstudio` (never loads it): by soname first,
/// then by the full path found in `/proc/self/maps`.
pub fn open_loaded() -> Option<*mut c_void> {
    // SAFETY: NUL-terminated literal; NOLOAD only looks up.
    let h = unsafe { libc::dlopen(c"libfmodstudio.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_NOLOAD) };
    if !h.is_null() {
        return Some(h);
    }
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    let line = maps.lines().find(|l| l.contains("libfmodstudio.so"))?;
    let path = &line[line.find('/')?..];
    let c = CString::new(path).ok()?;
    // SAFETY: NUL-terminated path of a module that is mapped right now.
    let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_NOLOAD) };
    if h.is_null() { None } else { Some(h) }
}

/// Resolve `name` as `T` (a `fn` pointer type); logs a missing symbol.
///
/// # Safety
/// `T` must be the real C signature of the exported symbol.
pub unsafe fn sym<T: Copy>(lib: *mut c_void, name: &CStr) -> Option<T> {
    // SAFETY: valid module handle + NUL-terminated name.
    let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if p.is_null() {
        crate::log_line(&format!("fmod: missing symbol {}", name.to_string_lossy()));
        return None;
    }
    // SAFETY: fn pointers and data pointers have the same size on every
    // supported target; the caller vouches for the signature.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
}

/// `FMODUnity.RuntimeManager` singleton → its `FMOD.Studio.System`
/// field (value type wrapping one `IntPtr`). Field found by TYPE, not
/// by name (private names move between integration versions). Every
/// candidate handle is checked with `valid` (FMOD validates handles
/// itself, a stale one only errors). `logged_fields` limits the
/// "field list" diagnostic to once.
pub fn discover_studio_system(
    unity: &mut Option<UnityCache>,
    valid: &dyn Fn(usize) -> bool,
    logged_fields: &mut bool,
) -> Option<usize> {
    let api = crate::il2cpp_api()?;
    // SAFETY: attached tick thread; every step null-checked and the
    // object is alive-checked before its fields are read.
    unsafe {
        let domain = crate::il2cpp::domain_checked(api)?;
        if unity.is_none() {
            *unity = crate::unity::init(api, domain);
        }
        let cache = unity.as_ref()?;
        let klass = crate::il2cpp::find_class(api, domain, None, "FMODUnity", "RuntimeManager")?;
        let get_instance = crate::il2cpp::get_method(api, klass, "get_Instance", 0)?;
        let obj = crate::il2cpp::invoke(api, get_instance, std::ptr::null_mut(), &[])?;
        if crate::unity::object_alive(api, cache, obj) != Some(true) {
            return None;
        }
        let fields = crate::il2cpp::class_fields(api, klass);
        let mut found = None;
        for f in fields.iter().filter(|f| !f.is_static && !f.type_class.is_null()) {
            let name = crate::il2cpp::class_name(api, f.type_class).unwrap_or_default();
            let ns = crate::il2cpp::class_namespace(api, f.type_class).unwrap_or_default();
            if name == "System" && ns == "FMOD.Studio" {
                found = Some((f.field, f.type_class));
                break;
            }
        }
        let Some((field, tclass)) = found else {
            if !*logged_fields {
                *logged_fields = true;
                let list: Vec<String> = fields
                    .iter()
                    .map(|f| {
                        let t = crate::il2cpp::class_name(api, f.type_class).unwrap_or_default();
                        format!("{}:{}", f.name, t)
                    })
                    .collect();
                crate::log_line(&format!(
                    "fmod: no FMOD.Studio.System field on RuntimeManager; fields = {}",
                    list.join(", ")
                ));
            }
            return None;
        };
        // Struct payload copied into a roomy buffer; only a struct that
        // is exactly one pointer wide is a bare handle.
        let size = crate::il2cpp::class_value_size(api, tclass)?;
        if size != std::mem::size_of::<usize>() {
            crate::log_line(&format!("fmod: FMOD.Studio.System is {size} bytes, expected 8"));
            return None;
        }
        let mut buf = [0u8; 64];
        if !crate::il2cpp::field_get_raw(api, obj, field, &mut buf) {
            return None;
        }
        let handle = usize::from_ne_bytes(buf[..8].try_into().ok()?);
        if handle == 0 || !valid(handle) {
            return None;
        }
        crate::log_line(&format!("fmod: studio system bound ({handle:#x})"));
        Some(handle)
    }
}
