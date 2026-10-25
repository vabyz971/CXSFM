//! Minimal FMOD Studio C API, resolved at runtime with `dlsym`.
//!
//! The game ships `libfmodstudio.so` (Unity FMOD integration). We never
//! link it: the already-loaded module is looked up (`RTLD_NOLOAD`, so
//! nothing is ever loaded or unloaded by us) and only the handful of
//! functions the mod needs are resolved. A missing symbol makes
//! [`Fmod::load`] fail as a whole — the mod then idles.
//!
//! FMOD handles are opaque, generation-tagged ids validated by the
//! library: a stale handle yields `FMOD_ERR_INVALID_HANDLE`, not a
//! fault. Every wrapper maps any non-`FMOD_OK` result to `None`.
//! Studio is thread-safe unless the game inits it thread-unsafe
//! (to validate in game).

use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};

/// Opaque FMOD handle (system, bank, event description/instance).
pub type Handle = *mut c_void;

const FMOD_OK: c_int = 0;
/// Caps: a corrupt count must not allocate without bound.
pub const MAX_BANKS: usize = 256;
pub const MAX_EVENTS_PER_BANK: usize = 8192;
pub const MAX_INSTANCES: usize = 64;
const MAX_PARAMS: usize = 32;
const PATH_BUF: usize = 512;

/// `FMOD_STUDIO_PARAMETER_DESCRIPTION` (fmod_studio_common.h).
#[repr(C)]
struct RawParamDesc {
    name: *const c_char,
    id: [c_uint; 2],
    minimum: f32,
    maximum: f32,
    default: f32,
    kind: c_int,
    flags: c_uint,
}

/// One event parameter, as the event description declares it.
#[derive(Clone, Debug)]
pub struct ParamDesc {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

type FnHandleBool = unsafe extern "C" fn(Handle) -> c_int;
type FnList = unsafe extern "C" fn(Handle, *mut Handle, c_int, *mut c_int) -> c_int;
type FnCount = unsafe extern "C" fn(Handle, *mut c_int) -> c_int;
type FnSetF32 = unsafe extern "C" fn(Handle, f32) -> c_int;
type FnGet2F32 = unsafe extern "C" fn(Handle, *mut f32, *mut f32) -> c_int;
type FnPath = unsafe extern "C" fn(Handle, *mut c_char, c_int, *mut c_int) -> c_int;
type FnParamByIndex = unsafe extern "C" fn(Handle, c_int, *mut RawParamDesc) -> c_int;
type FnSetParam = unsafe extern "C" fn(Handle, *const c_char, f32, c_int) -> c_int;
type FnGetParam = unsafe extern "C" fn(Handle, *const c_char, *mut f32, *mut f32) -> c_int;

/// Resolved entry points (plain fn pointers: `Fmod` is `Send + Sync`).
pub struct Fmod {
    system_is_valid: FnHandleBool,
    system_bank_list: FnList,
    bank_event_list: FnList,
    desc_path: FnPath,
    desc_instance_count: FnCount,
    desc_instance_list: FnList,
    desc_param_count: FnCount,
    desc_param_by_index: FnParamByIndex,
    inst_set_pitch: FnSetF32,
    inst_set_volume: FnSetF32,
    inst_get_pitch: FnGet2F32,
    inst_get_volume: FnGet2F32,
    inst_set_param: FnSetParam,
    inst_get_param: FnGetParam,
}

/// Already-loaded `libfmodstudio` (never loads it): by soname first,
/// then by the full path found in `/proc/self/maps`.
fn open_loaded() -> Option<*mut c_void> {
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

/// Resolve `name` as `T` (a `fn` pointer type).
///
/// # Safety
/// `T` must be the real C signature of the exported symbol.
unsafe fn sym<T: Copy>(lib: *mut c_void, name: &CStr) -> Option<T> {
    // SAFETY: valid module handle + NUL-terminated name.
    let p = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if p.is_null() {
        crate::log_line(&format!("engine_audio: missing FMOD symbol {}", name.to_string_lossy()));
        return None;
    }
    // SAFETY: fn pointers and data pointers have the same size on every
    // supported target; the caller vouches for the signature.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
}

fn ok(r: c_int) -> Option<()> {
    if r == FMOD_OK { Some(()) } else { None }
}

impl Fmod {
    /// `None` while the library is not loaded yet or a symbol is gone.
    pub fn load() -> Option<Self> {
        let lib = open_loaded()?;
        // SAFETY: every signature below is the documented FMOD Studio
        // C API one (2.x); names are NUL-terminated literals.
        unsafe {
            Some(Self {
                system_is_valid: sym(lib, c"FMOD_Studio_System_IsValid")?,
                system_bank_list: sym(lib, c"FMOD_Studio_System_GetBankList")?,
                bank_event_list: sym(lib, c"FMOD_Studio_Bank_GetEventList")?,
                desc_path: sym(lib, c"FMOD_Studio_EventDescription_GetPath")?,
                desc_instance_count: sym(lib, c"FMOD_Studio_EventDescription_GetInstanceCount")?,
                desc_instance_list: sym(lib, c"FMOD_Studio_EventDescription_GetInstanceList")?,
                desc_param_count: sym(lib, c"FMOD_Studio_EventDescription_GetParameterDescriptionCount")?,
                desc_param_by_index: sym(lib, c"FMOD_Studio_EventDescription_GetParameterDescriptionByIndex")?,
                inst_set_pitch: sym(lib, c"FMOD_Studio_EventInstance_SetPitch")?,
                inst_set_volume: sym(lib, c"FMOD_Studio_EventInstance_SetVolume")?,
                inst_get_pitch: sym(lib, c"FMOD_Studio_EventInstance_GetPitch")?,
                inst_get_volume: sym(lib, c"FMOD_Studio_EventInstance_GetVolume")?,
                inst_set_param: sym(lib, c"FMOD_Studio_EventInstance_SetParameterByName")?,
                inst_get_param: sym(lib, c"FMOD_Studio_EventInstance_GetParameterByName")?,
            })
        }
    }

    pub fn system_valid(&self, sys: Handle) -> bool {
        // SAFETY: FMOD validates the handle itself.
        !sys.is_null() && unsafe { (self.system_is_valid)(sys) } != 0
    }

    /// Two-call list protocol (`capacity` slots, `count` filled).
    fn list(&self, f: FnList, owner: Handle, cap: usize) -> Vec<Handle> {
        let mut buf = vec![std::ptr::null_mut(); cap.max(1)];
        let mut n: c_int = 0;
        // SAFETY: `buf` holds `cap` slots, `n` is a live out param.
        let r = unsafe { f(owner, buf.as_mut_ptr(), cap as c_int, &mut n) };
        if ok(r).is_none() {
            return Vec::new();
        }
        buf.truncate((n.max(0) as usize).min(cap));
        buf
    }

    pub fn banks(&self, sys: Handle) -> Vec<Handle> {
        self.list(self.system_bank_list, sys, MAX_BANKS)
    }

    pub fn bank_events(&self, bank: Handle) -> Vec<Handle> {
        self.list(self.bank_event_list, bank, MAX_EVENTS_PER_BANK)
    }

    /// `event:/Cars/Engine/…` style path (needs the strings bank).
    pub fn event_path(&self, desc: Handle) -> Option<String> {
        let mut buf = [0 as c_char; PATH_BUF];
        let mut n: c_int = 0;
        // SAFETY: buffer size is passed; FMOD NUL-terminates inside it.
        ok(unsafe { (self.desc_path)(desc, buf.as_mut_ptr(), PATH_BUF as c_int, &mut n) })?;
        // SAFETY: success guarantees a NUL-terminated string in `buf`.
        Some(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
    }

    pub fn instance_count(&self, desc: Handle) -> usize {
        let mut n: c_int = 0;
        // SAFETY: live out param.
        match ok(unsafe { (self.desc_instance_count)(desc, &mut n) }) {
            Some(()) => n.max(0) as usize,
            None => 0,
        }
    }

    pub fn instances(&self, desc: Handle) -> Vec<Handle> {
        self.list(self.desc_instance_list, desc, MAX_INSTANCES)
    }

    pub fn params(&self, desc: Handle) -> Vec<ParamDesc> {
        let mut n: c_int = 0;
        // SAFETY: live out param.
        if ok(unsafe { (self.desc_param_count)(desc, &mut n) }).is_none() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 0..(n.max(0) as usize).min(MAX_PARAMS) {
            let mut raw = RawParamDesc {
                name: std::ptr::null(),
                id: [0; 2],
                minimum: 0.0,
                maximum: 0.0,
                default: 0.0,
                kind: 0,
                flags: 0,
            };
            // SAFETY: `raw` matches the C struct; `name` points into
            // FMOD-owned memory valid while the bank stays loaded.
            if ok(unsafe { (self.desc_param_by_index)(desc, i as c_int, &mut raw) }).is_none() || raw.name.is_null() {
                continue;
            }
            // SAFETY: non-null NUL-terminated FMOD string.
            let name = unsafe { CStr::from_ptr(raw.name) }.to_string_lossy().into_owned();
            out.push(ParamDesc { name, min: raw.minimum, max: raw.maximum, default: raw.default });
        }
        out
    }

    pub fn set_pitch(&self, inst: Handle, v: f32) -> bool {
        // SAFETY: handle validated by FMOD.
        ok(unsafe { (self.inst_set_pitch)(inst, v) }).is_some()
    }

    pub fn set_volume(&self, inst: Handle, v: f32) -> bool {
        // SAFETY: handle validated by FMOD.
        ok(unsafe { (self.inst_set_volume)(inst, v) }).is_some()
    }

    /// User-set pitch (not the final, parameter-driven one).
    pub fn pitch(&self, inst: Handle) -> Option<f32> {
        let (mut v, mut fin) = (0.0f32, 0.0f32);
        // SAFETY: live out params.
        ok(unsafe { (self.inst_get_pitch)(inst, &mut v, &mut fin) })?;
        Some(v)
    }

    pub fn volume(&self, inst: Handle) -> Option<f32> {
        let (mut v, mut fin) = (0.0f32, 0.0f32);
        // SAFETY: live out params.
        ok(unsafe { (self.inst_get_volume)(inst, &mut v, &mut fin) })?;
        Some(v)
    }

    pub fn set_param(&self, inst: Handle, name: &str, v: f32) -> bool {
        let Ok(c) = CString::new(name) else { return false };
        // SAFETY: NUL-terminated name; handle validated by FMOD.
        ok(unsafe { (self.inst_set_param)(inst, c.as_ptr(), v, 1) }).is_some()
    }

    /// `(value, final_value)`: final includes automation/modulation.
    pub fn param(&self, inst: Handle, name: &str) -> Option<(f32, f32)> {
        let c = CString::new(name).ok()?;
        let (mut v, mut fin) = (0.0f32, 0.0f32);
        // SAFETY: NUL-terminated name, live out params.
        ok(unsafe { (self.inst_get_param)(inst, c.as_ptr(), &mut v, &mut fin) })?;
        Some((v, fin))
    }
}
