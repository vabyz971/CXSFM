//! IL2CPP bridge for Unity games (CarX Street runs on IL2CPP).
//!
//! # Approach (proven by community Unity mods)
//!
//! The game exposes the standard IL2CPP embedding API (`il2cpp_domain_get`,
//! `il2cpp_thread_attach`, …) from its already-loaded `GameAssembly.so`.
//! We resolve those exports at runtime — first via an explicit `dlopen` of
//! the path found in `/proc/self/maps`, falling back to a bare
//! `"GameAssembly.so"` lookup and finally to the global scope
//! (`RTLD_DEFAULT`, which works when Unity loaded the assembly globally).
//!
//! # Threading rule (read this before touching IL2CPP from a mod)
//!
//! IL2CPP must see every calling thread via [`Il2cppApi::attach_thread`]
//! first. Calling convention APIs (e.g. object enumeration) from a thread
//! that was never attached — or hammering them in a tight loop — is a
//! known SIGSEGV source. Attach once per thread, then call sparingly.
//!
//! # Safety
//!
//! Raw function pointers cross the FFI boundary here; every public entry
//! point validates nulls and reports [`Il2cppError`] instead of panicking.

use std::ffi::CStr;
use std::ffi::CString;

/// `void* il2cpp_domain_get(void)`
pub type DomainGetFn = unsafe extern "C" fn() -> *mut std::ffi::c_void;
/// `void* il2cpp_thread_attach(void* domain)`
pub type ThreadAttachFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void;
/// `const void** il2cpp_domain_get_assemblies(void* domain, size_t* size)`
pub type DomainGetAssembliesFn =
    unsafe extern "C" fn(*mut std::ffi::c_void, *mut usize) -> *const *const std::ffi::c_void;
/// `void* il2cpp_class_from_name(void* image, const char* ns, const char* name)`
pub type ClassFromNameFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *const libc::c_char,
    *const libc::c_char,
) -> *mut std::ffi::c_void;
/// `void* il2cpp_runtime_invoke(void* method, void* obj, void** params, void** exc)`
pub type RuntimeInvokeFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    *mut *mut std::ffi::c_void,
    *mut *mut std::ffi::c_void,
) -> *mut std::ffi::c_void;
/// `void* il2cpp_domain_assembly_open(void* domain, const char* name)`
pub type DomainAssemblyOpenFn =
    unsafe extern "C" fn(*mut std::ffi::c_void, *const libc::c_char) -> *mut std::ffi::c_void;
/// `void* il2cpp_assembly_get_image(void* assembly)`
pub type AssemblyGetImageFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void;
/// `const char* il2cpp_image_get_name(void* image)`
pub type ImageGetNameFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *const libc::c_char;
/// `void* il2cpp_class_get_method_from_name(void* klass, const char* name, int args)`
pub type ClassGetMethodFromNameFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *const libc::c_char,
    i32,
) -> *mut std::ffi::c_void;
/// `const char* il2cpp_method_get_name(void* method)` (diagnostics only)
pub type MethodGetNameFn =
    unsafe extern "C" fn(*mut std::ffi::c_void) -> *const libc::c_char;
/// `void* il2cpp_class_get_type(void* klass)`
pub type ClassGetTypeFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void;
/// `void* il2cpp_type_get_object(void* type)`
pub type TypeGetObjectFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void;
/// `uint32_t il2cpp_array_length(void* arr)`
pub type ArrayLengthFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> u32;
/// `uint32_t il2cpp_array_object_header_size(void)`
pub type ArrayHeaderSizeFn = unsafe extern "C" fn() -> u32;
/// `void* il2cpp_object_unbox(void* obj)` (unbox value-type results)
pub type ObjectUnboxFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut std::ffi::c_void;
/// `int32_t il2cpp_string_length(void* str)` (UTF-16 code units)
pub type StringLengthFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;
/// `uint16_t* il2cpp_string_chars(void* str)` (no copy, process-lifetime)
pub type StringCharsFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *mut u16;
/// `void* il2cpp_string_new(const char* str)` (UTF-8 in, managed string out)
pub type StringNewFn = unsafe extern "C" fn(*const libc::c_char) -> *mut std::ffi::c_void;
/// `void* il2cpp_resolve_icall(const char* name)` (raw icall pointer out)
///
/// Used for `extern` Unity getters with no managed `MethodInfo`, e.g.
/// `UnityEngine.Application::get_systemLanguage` (returns the
/// `SystemLanguage` int directly — no invoke/unbox needed).
pub type ResolveIcallFn = unsafe extern "C" fn(*const libc::c_char) -> *mut std::ffi::c_void;

/// Failure modes of IL2CPP resolution and probing.
#[derive(Debug)]
pub enum Il2cppError {
    /// No `GameAssembly.so` mapping and no global `il2cpp_domain_get`.
    /// Expected outside the game (smoke tests) — not fatal.
    NotPresent,
    /// The core trio (`domain_get`) is missing; the API is unusable.
    MissingCore(&'static str),
    /// A call returned null where it must not (e.g. null domain).
    NullResult(&'static str),
    /// A string (path/symbol) contained an interior NUL byte.
    BadString(&'static str),
}

impl std::fmt::Display for Il2cppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Il2cppError::NotPresent => write!(f, "no IL2CPP runtime in this process"),
            Il2cppError::MissingCore(name) => {
                write!(f, "required IL2CPP export missing: {}", name)
            }
            Il2cppError::NullResult(what) => write!(f, "IL2CPP returned null: {}", what),
            Il2cppError::BadString(what) => {
                write!(f, "invalid string for IL2CPP call: {}", what)
            }
        }
    }
}

impl std::error::Error for Il2cppError {}

/// Resolved IL2CPP embedding API.
///
/// `domain_get` / `thread_attach` / `domain_get_assemblies` are guaranteed
/// present (resolution fails otherwise). The remaining entries are
/// best-effort (`None` when the game build omits them) for future mods.
pub struct Il2cppApi {
    /// `il2cpp_domain_get` — always present after [`resolve`].
    pub domain_get: DomainGetFn,
    /// `il2cpp_thread_attach` — always present after [`resolve`].
    pub thread_attach: ThreadAttachFn,
    /// `il2cpp_domain_get_assemblies` — always present after [`resolve`].
    pub domain_get_assemblies: DomainGetAssembliesFn,
    /// `il2cpp_class_from_name` — best effort.
    pub class_from_name: Option<ClassFromNameFn>,
    /// `il2cpp_runtime_invoke` — best effort.
    pub runtime_invoke: Option<RuntimeInvokeFn>,
    /// `il2cpp_domain_assembly_open` — best effort.
    pub domain_assembly_open: Option<DomainAssemblyOpenFn>,
    /// `il2cpp_assembly_get_image` — best effort.
    pub assembly_get_image: Option<AssemblyGetImageFn>,
    /// `il2cpp_image_get_name` — best effort (diagnostics).
    pub image_get_name: Option<ImageGetNameFn>,
    /// `il2cpp_class_get_method_from_name` — best effort.
    pub class_get_method_from_name: Option<ClassGetMethodFromNameFn>,
    /// `il2cpp_method_get_name` — best effort (diagnostics).
    pub method_get_name: Option<MethodGetNameFn>,
    /// `il2cpp_class_get_type` — best effort.
    pub class_get_type: Option<ClassGetTypeFn>,
    /// `il2cpp_type_get_object` — best effort.
    pub type_get_object: Option<TypeGetObjectFn>,
    /// `il2cpp_array_length` — best effort.
    pub array_length: Option<ArrayLengthFn>,
    /// `il2cpp_array_object_header_size` — best effort.
    pub array_object_header_size: Option<ArrayHeaderSizeFn>,
    /// `il2cpp_object_unbox` — best effort.
    pub object_unbox: Option<ObjectUnboxFn>,
    /// `il2cpp_string_length` — best effort.
    pub string_length: Option<StringLengthFn>,
    /// `il2cpp_string_chars` — best effort.
    pub string_chars: Option<StringCharsFn>,
    /// `il2cpp_string_new` — best effort.
    pub string_new: Option<StringNewFn>,
    /// `il2cpp_resolve_icall` — best effort.
    pub resolve_icall: Option<ResolveIcallFn>,
}

impl Il2cppApi {
    /// Attach the *current* thread to the IL2CPP domain.
    ///
    /// Must be called once on every thread before it touches any other
    /// IL2CPP function. Returns the opaque thread handle (or null on
    /// failure — check before proceeding).
    ///
    /// # Safety
    /// Calls into the game's runtime; the domain pointer must come from
    /// [`Il2cppApi::domain`] on the same process.
    pub unsafe fn attach_current_thread(&self, domain: *mut std::ffi::c_void) -> *mut std::ffi::c_void {
        // SAFETY: caller guarantees a live domain from this same API.
        unsafe { (self.thread_attach)(domain) }
    }

    /// Fetch the current IL2CPP domain, or [`Il2cppError::NullResult`].
    ///
    /// # Safety
    /// Calls into the game's runtime. Safe to call on any thread, but the
    /// thread should have been attached first for subsequent use.
    pub unsafe fn domain(&self) -> Result<*mut std::ffi::c_void, Il2cppError> {
        // SAFETY: `domain_get` is a validated non-null export.
        let d = unsafe { (self.domain_get)() };
        if d.is_null() {
            return Err(Il2cppError::NullResult("domain"));
        }
        Ok(d)
    }

    /// Count assemblies in the domain (smoke proof the API is alive).
    ///
    /// # Safety
    /// Same contract as [`Il2cppApi::domain`].
    pub unsafe fn assembly_count(        &self,
        domain: *mut std::ffi::c_void,
    ) -> Result<usize, Il2cppError> {
        let mut n: usize = 0;
        // SAFETY: `domain` validated by the caller; `&mut n` is a valid
        // out-pointer living for the whole call.
        let list = unsafe { (self.domain_get_assemblies)(domain, &mut n as *mut usize) };
        if list.is_null() && n == 0 {
            return Err(Il2cppError::NullResult("assemblies"));
        }
        Ok(n)
    }
}

/// Consecutive `domain()` failures before the runtime is declared dead.
const DOMAIN_DEATH_STRIKES: u32 = 3;
/// Consecutive failure count (any success resets it).
static DOMAIN_FAILURES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Set once the runtime is declared dead: all further IL2CPP use stops
/// for the rest of the process (see below).
static DOMAIN_DEAD: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Fetch the domain, declaring the runtime dead after repeated failure.
///
/// A healthy runtime never fails `domain_get`; a dying one (Unity
/// teardown) fails persistently — and heavier calls on a half-torn
/// runtime segfault instead of erroring (caught live: tick thread
/// inside `poll_framework_input_edge` during game exit). After
/// [`DOMAIN_DEATH_STRIKES`] consecutive failures this returns `None`
/// forever: mods park, input falls back to SDL/X11, the process exits
/// cleanly instead of crashing in Unity's corpse.
///
/// # Safety
/// Same contract as [`Il2cppApi::domain`].
pub unsafe fn domain_checked(api: &Il2cppApi) -> Option<*mut std::ffi::c_void> {
    use std::sync::atomic::Ordering;
    if DOMAIN_DEAD.load(Ordering::SeqCst) {
        return None;
    }
    // SAFETY: same contract, enforced by the caller.
    match unsafe { api.domain() } {
        Ok(d) => {
            DOMAIN_FAILURES.store(0, Ordering::SeqCst);
            Some(d)
        }
        Err(_) => {
            let strikes = DOMAIN_FAILURES.fetch_add(1, Ordering::SeqCst) + 1;
            if strikes >= DOMAIN_DEATH_STRIKES
                && !DOMAIN_DEAD.swap(true, Ordering::SeqCst)
            {
                crate::log_line("il2cpp: runtime unresponsive, parking IL2CPP use");
            }
            None
        }
    }
}

/// Look up `symbol`, first in the explicit `GameAssembly` handle (when we
/// have one), then in the global process scope (Unix only — Windows has
/// no `RTLD_DEFAULT` equivalent).
///
/// # Safety
/// `dlsym` on a live handle; the returned pointer is only as valid as the
/// exporting library, which outlives the game process by definition.
#[cfg(unix)]
unsafe fn lookup(
    game_assembly: *mut libc::c_void,
    symbol: &CStr,
) -> Option<*mut libc::c_void> {
    // SAFETY: both handles (when non-null) are live `dlopen` results.
    unsafe {
        if !game_assembly.is_null() {
            let p = libc::dlsym(game_assembly, symbol.as_ptr());
            if !p.is_null() {
                return Some(p);
            }
        }
        let p = libc::dlsym(libc::RTLD_DEFAULT, symbol.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(p)
        }
    }
}

/// Windows twin of [`lookup`]: `GetProcAddress` on the module handle.
/// No global-scope fallback exists here — `acquire_handle` already covers
/// both already-loaded and load-by-name cases.
///
/// Syntax-checked on Linux; first type-check happens on a Windows build.
#[cfg(windows)]
unsafe fn lookup(
    game_assembly: *mut std::ffi::c_void,
    symbol: &CStr,
) -> Option<*mut std::ffi::c_void> {
    use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
    if game_assembly.is_null() {
        return None;
    }
    // SAFETY: live module handle + valid NUL-terminated symbol name.
    // `GetProcAddress` wants narrow (ANSI) names: `CStr` already is.
    unsafe {
        match GetProcAddress(
            game_assembly as isize,
            symbol.as_ptr() as *const u8,
        ) {
            Some(f) => Some(f as *mut std::ffi::c_void),
            None => None,
        }
    }
}

/// Acquire a handle on the game's already-loaded assembly (Unix):
/// exact path from our own maps first (`RTLD_NOLOAD` loads no second
/// copy), then a bare-name lookup.
#[cfg(unix)]
fn acquire_handle() -> *mut std::ffi::c_void {
    let mut handle: *mut libc::c_void = std::ptr::null_mut();
    if let Some(path) = crate::memory::find_module_path("GameAssembly.so") {
        let path_str = path.to_string_lossy().into_owned();
        if let Ok(c_path) = CString::new(path_str) {
            // SAFETY: valid NUL-terminated path; flags are constants.
            unsafe {
                let h =
                    libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_NOLOAD);
                if !h.is_null() {
                    handle = h;
                } else {
                    // Mapped but not dlopen-able by path (weird namespaces):
                    // fall through to the name-based lookup below.
                    let _ = libc::dlerror();
                }
            }
        }
    }
    if handle.is_null() {
        if let Ok(name) = CString::new("GameAssembly.so") {
            // SAFETY: static string, valid flags.
            unsafe {
                let h = libc::dlopen(name.as_ptr(), libc::RTLD_NOW);
                if !h.is_null() {
                    handle = h;
                }
            }
        }
    }
    handle as *mut std::ffi::c_void
}

/// Acquire a handle on the game's already-loaded assembly (Windows):
/// `GetModuleHandleW` (already loaded — the normal case) then
/// `LoadLibraryW` by bare name (resolved via the process DLL search
/// order, which includes the game directory).
///
/// Syntax-checked on Linux; first type-check happens on a Windows build.
#[cfg(windows)]
fn acquire_handle() -> *mut std::ffi::c_void {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    let name: Vec<u16> = "GameAssembly.dll\0".encode_utf16().collect();
    // SAFETY: valid NUL-terminated UTF-16; null handles checked.
    unsafe {
        let h = GetModuleHandleW(name.as_ptr());
        if h != 0 {
            return h as *mut std::ffi::c_void;
        }
        LoadLibraryW(name.as_ptr()) as *mut std::ffi::c_void
    }
}

/// Resolve the IL2CPP embedding API of the host game.
///
/// Never panics; outside a Unity IL2CPP process this returns
/// [`Il2cppError::NotPresent`] (callers treat that as "skip game hooks").
pub fn resolve() -> Result<Il2cppApi, Il2cppError> {
    // 1) Explicit handle on the already-loaded game assembly
    //    (platform-specific, see `acquire_handle`).
    let handle = acquire_handle();

    // 2) Fast negative path: nothing found means no IL2CPP runtime lives
    //    here at all (plain smoke-test process, wrong game, …). Report
    //    cleanly so callers can skip game hooks without treating it as
    //    an error. Unix additionally probes the global scope, which may
    //    hold the symbols even when no explicit handle was acquired.
    #[cfg(unix)]
    if handle.is_null() {
        if let Ok(probe) = CString::new("il2cpp_domain_get") {
            // SAFETY: static NUL-terminated name; read-only query.
            let found = unsafe { !libc::dlsym(libc::RTLD_DEFAULT, probe.as_ptr()).is_null() };
            if !found {
                return Err(Il2cppError::NotPresent);
            }
        }
    }
    #[cfg(windows)]
    if handle.is_null() {
        return Err(Il2cppError::NotPresent);
    }

    // 3) Core trio — all three or nothing.
    //
    // SAFETY: `lookup` is null-checked; `transmute` targets match the
    // documented IL2CPP signatures exactly.
    unsafe fn required<T>(handle: *mut libc::c_void, symbol: &CStr, name: &'static str) -> Result<T, Il2cppError>
    where
        T: Copy,
    {
        let p = unsafe { lookup(handle, symbol) }.ok_or(Il2cppError::MissingCore(name))?;
        Ok(unsafe { std::mem::transmute_copy::<*mut libc::c_void, T>(&p) })
    }
    // NUL-terminated literals: `c"..."` cannot fail.
    let domain_get: DomainGetFn = unsafe { required(handle, c"il2cpp_domain_get", "il2cpp_domain_get")? };
    let thread_attach: ThreadAttachFn =
        unsafe { required(handle, c"il2cpp_thread_attach", "il2cpp_thread_attach")? };
    let domain_get_assemblies: DomainGetAssembliesFn =
        unsafe { required(handle, c"il2cpp_domain_get_assemblies", "il2cpp_domain_get_assemblies")? };

    // 4) Best-effort extras for mods. One macro keeps the thirteen
    //    lookups uniform: null-checked, transmuted, never panicking.
    macro_rules! sym_opt {
        ($name:expr, $ty:ty) => {
            match CString::new($name) {
                Ok(s) => unsafe { lookup(handle, &s) }
                    .map(|p| unsafe { std::mem::transmute::<*mut libc::c_void, $ty>(p) }),
                Err(_) => None,
            }
        };
    }
    let class_from_name: Option<ClassFromNameFn> = sym_opt!("il2cpp_class_from_name", ClassFromNameFn);
    let runtime_invoke: Option<RuntimeInvokeFn> = sym_opt!("il2cpp_runtime_invoke", RuntimeInvokeFn);
    let domain_assembly_open: Option<DomainAssemblyOpenFn> =
        sym_opt!("il2cpp_domain_assembly_open", DomainAssemblyOpenFn);
    let assembly_get_image: Option<AssemblyGetImageFn> =
        sym_opt!("il2cpp_assembly_get_image", AssemblyGetImageFn);
    let image_get_name: Option<ImageGetNameFn> = sym_opt!("il2cpp_image_get_name", ImageGetNameFn);
    let class_get_method_from_name: Option<ClassGetMethodFromNameFn> =
        sym_opt!("il2cpp_class_get_method_from_name", ClassGetMethodFromNameFn);
    let method_get_name: Option<MethodGetNameFn> = sym_opt!("il2cpp_method_get_name", MethodGetNameFn);
    let class_get_type: Option<ClassGetTypeFn> = sym_opt!("il2cpp_class_get_type", ClassGetTypeFn);
    let type_get_object: Option<TypeGetObjectFn> = sym_opt!("il2cpp_type_get_object", TypeGetObjectFn);
    let array_length: Option<ArrayLengthFn> = sym_opt!("il2cpp_array_length", ArrayLengthFn);
    let array_object_header_size: Option<ArrayHeaderSizeFn> =
        sym_opt!("il2cpp_array_object_header_size", ArrayHeaderSizeFn);
    let object_unbox: Option<ObjectUnboxFn> = sym_opt!("il2cpp_object_unbox", ObjectUnboxFn);
    let string_length: Option<StringLengthFn> = sym_opt!("il2cpp_string_length", StringLengthFn);
    let string_chars: Option<StringCharsFn> = sym_opt!("il2cpp_string_chars", StringCharsFn);
    let string_new: Option<StringNewFn> = sym_opt!("il2cpp_string_new", StringNewFn);
    let resolve_icall: Option<ResolveIcallFn> = sym_opt!("il2cpp_resolve_icall", ResolveIcallFn);

    Ok(Il2cppApi {
        domain_get,
        thread_attach,
        domain_get_assemblies,
        class_from_name,
        runtime_invoke,
        domain_assembly_open,
        assembly_get_image,
        image_get_name,
        class_get_method_from_name,
        method_get_name,
        class_get_type,
        type_get_object,
        array_length,
        array_object_header_size,
        object_unbox,
        string_length,
        string_chars,
        string_new,
        resolve_icall,
    })
}

// --- High-level helpers (mirror community Unity-mod practice) -----------------

/// Find a class: try the named assembly first, then walk every loaded
/// image. Returns the `Il2CppClass*` or `None`.
///
/// # Safety
/// Calls into the game's runtime on the caller's thread (must be
/// attached for subsequent use of the result).
pub unsafe fn find_class(
    api: &Il2cppApi,
    domain: *mut std::ffi::c_void,
    assembly: Option<&str>,
    ns: &str,
    name: &str,
) -> Option<*mut std::ffi::c_void> {
    let (Some(class_from_name), Some(domain_assembly_open), Some(assembly_get_image)) =
        (api.class_from_name, api.domain_assembly_open, api.assembly_get_image)
    else {
        return None;
    };
    let (Ok(ns_c), Ok(name_c)) = (CString::new(ns), CString::new(name)) else {
        return None;
    };
    // Fast path: named assembly.
    if let Some(asm_name) = assembly {
        if let Ok(asm_c) = CString::new(asm_name) {
            // SAFETY: validated API + NUL-terminated inputs.
            unsafe {
                let asmbl = domain_assembly_open(domain, asm_c.as_ptr());
                if !asmbl.is_null() {
                    let img = assembly_get_image(asmbl);
                    if !img.is_null() {
                        let klass = class_from_name(img, ns_c.as_ptr(), name_c.as_ptr());
                        if !klass.is_null() {
                            return Some(klass);
                        }
                    }
                }
            }
        }
    }
    // Slow path: walk every loaded assembly image.
    let mut n: usize = 0;
    // SAFETY: `domain` came from this same API; `&mut n` outlives the call.
    let asms = unsafe { (api.domain_get_assemblies)(domain, &mut n as *mut usize) };
    if asms.is_null() {
        return None;
    }
    for i in 0..n {
        // SAFETY: array bounds are the runtime-reported count.
        let asmbl = unsafe { *asms.add(i) } as *mut std::ffi::c_void;
        if asmbl.is_null() {
            continue;
        }
        // SAFETY: live assembly handle from the domain enumeration.
        let img = unsafe { assembly_get_image(asmbl) };
        if img.is_null() {
            continue;
        }
        // SAFETY: live image + NUL-terminated inputs.
        let klass =
            unsafe { class_from_name(img, ns_c.as_ptr(), name_c.as_ptr()) };
        if !klass.is_null() {
            return Some(klass);
        }
    }
    None
}

/// Fetch a method by name + argument count. Returns the `MethodInfo*`.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn get_method(
    api: &Il2cppApi,
    klass: *mut std::ffi::c_void,
    name: &str,
    argc: i32,
) -> Option<*mut std::ffi::c_void> {
    let class_get_method_from_name = api.class_get_method_from_name?;
    let name_c = CString::new(name).ok()?;
    // SAFETY: validated API + live class + NUL-terminated name.
    let m = unsafe { class_get_method_from_name(klass, name_c.as_ptr(), argc) };
    if m.is_null() {
        None
    } else {
        Some(m)
    }
}

/// Raw invoke core: returns `(return_value, exception)` so value- and
/// void-returning callees can apply their own success rule.
///
/// # Safety
/// Same contract as [`find_class`]. `params` holds one raw pointer per
/// declared argument.
unsafe fn invoke_raw(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
    params: &[*mut std::ffi::c_void],
) -> Option<(*mut std::ffi::c_void, *mut std::ffi::c_void)> {
    let runtime_invoke = api.runtime_invoke?;
    if method.is_null() {
        return None;
    }
    let parg = if params.is_empty() {
        std::ptr::null_mut()
    } else {
        params.as_ptr() as *mut *mut std::ffi::c_void
    };
    let mut exc: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: validated API + live method; params match the callee.
    let ret = unsafe { runtime_invoke(method, obj, parg, &mut exc as *mut *mut std::ffi::c_void) };
    Some((ret, exc))
}

/// Invoke a method; returns the result object (or boxed value-type) —
/// `None` on missing API, null method, or a managed exception (which is
/// swallowed the way community mods do: the game keeps running).
///
/// Only for methods that **return a value**. Void setters must use
/// [`invoke_void`]: the runtime legitimately returns NULL for them and
/// that is success, not failure.
///
/// # Safety
/// Same contract as [`find_class`]. `params` holds one raw pointer per
/// declared argument (value types passed by address, e.g. `&fov`).
pub unsafe fn invoke(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
    params: &[*mut std::ffi::c_void],
) -> Option<*mut std::ffi::c_void> {
    // SAFETY: same contract, enforced by the caller of `invoke`.
    let (ret, exc) = unsafe { invoke_raw(api, method, obj, params) }?;
    if !exc.is_null() || ret.is_null() {
        return None;
    }
    Some(ret)
}

/// Invoke a `void` method; returns success.
///
/// The runtime returns NULL for void callees even on success, so only a
/// managed exception counts as failure. Using [`invoke`] here would
/// report every setter call as failed while it actually executed — a
/// silent-failure trap this split exists to prevent.
///
/// # Safety
/// Same contract as [`invoke`].
pub unsafe fn invoke_void(
    api: &Il2cppApi,
    method: *mut std::ffi::c_void,
    obj: *mut std::ffi::c_void,
    params: &[*mut std::ffi::c_void],
) -> bool {
    // SAFETY: same contract, enforced by the caller of `invoke_void`.
    match unsafe { invoke_raw(api, method, obj, params) } {
        Some((_, exc)) => exc.is_null(),
        None => false,
    }
}

/// `System.Type` object for a class (needed by `FindObjectsOfType`).
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn type_object_for_class(
    api: &Il2cppApi,
    klass: *mut std::ffi::c_void,
) -> Option<*mut std::ffi::c_void> {
    // SAFETY: validated API + live class, chained null-checked.
    unsafe {
        let t = (api.class_get_type?)(klass);
        if t.is_null() {
            return None;
        }
        let o = (api.type_get_object?)(t);
        if o.is_null() {
            None
        } else {
            Some(o)
        }
    }
}

/// Live instances of a class via `Object.FindObjectsOfType`.
///
/// Returns `(items_pointer, count)`; items are `Il2CppObject*` laid out
/// back-to-back after the array header. Use sparingly (scene walk) and
/// never in a tight loop — see the threading rule in the module docs.
///
/// # Safety
/// Same contract as [`find_class`], plus: the returned array is only
/// valid until the next GC-moving call; consume it immediately.
pub unsafe fn find_objects_of_type(
    api: &Il2cppApi,
    find_method: *mut std::ffi::c_void,
    type_obj: *mut std::ffi::c_void,
) -> Option<(*mut *mut std::ffi::c_void, u32)> {
    let (array_length, array_object_header_size) =
        (api.array_length?, api.array_object_header_size?);
    let params = [type_obj];
    // SAFETY: `find_method` is the bound `FindObjectsOfType(Type)`.
    let arr = unsafe { invoke(api, find_method, std::ptr::null_mut(), &params) }?;
    // SAFETY: `arr` is a live managed array just returned to us.
    let (n, hdr) = unsafe { (array_length(arr), array_object_header_size()) };
    if arr.is_null() {
        return None;
    }
    Some((unsafe { arr.add(hdr as usize) } as *mut *mut std::ffi::c_void, n))
}

/// Allocate a managed string from UTF-8.
///
/// # Safety
/// Same contract as [`find_class`].
pub unsafe fn new_string(
    api: &Il2cppApi,
    text: &str,
) -> Option<*mut std::ffi::c_void> {
    let string_new = api.string_new?;
    let c = CString::new(text).ok()?;
    // SAFETY: validated API + NUL-terminated input.
    let s = unsafe { string_new(c.as_ptr()) };
    if s.is_null() {
        None
    } else {
        Some(s)
    }
}

/// Read a managed string as lossy UTF-8 (reads UTF-16 code units).
///
/// # Safety
/// Same contract as [`find_class`]; the chars pointer is only valid
/// while the string is alive — copy out immediately (we do).
pub unsafe fn read_string(
    api: &Il2cppApi,
    s: *mut std::ffi::c_void,
) -> Option<String> {
    let (string_length, string_chars) = (api.string_length?, api.string_chars?);
    if s.is_null() {
        return None;
    }
    // SAFETY: live string; length bounds the chars read below.
    let (n, chars) = unsafe { (string_length(s), string_chars(s)) };
    if chars.is_null() || n <= 0 {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(chars, n as usize) };
    Some(String::from_utf16_lossy(slice))
}
