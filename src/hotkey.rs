//! Hotkey handling, three independent sources (first edge wins).
//!
//! 1. The game's own SDL2 (`SDL_GetKeyboardState`) — zero new input
//!    plumbing, but dead when the game never pumps SDL keyboard events
//!    (Unity reads its keyboard natively; SDL may serve the gamepad only).
//! 2. Unity's `Input.GetKey` via IL2CPP — the engine's own input state,
//!    but callable only off the main thread (technically unsupported).
//! 3. The X server itself (`XQueryKeymap`) — bypasses SDL, the video
//!    driver and Unity entirely; works under XWayland (which is how
//!    native X11 Unity games run on Wayland compositors like niri).
//!
//! Every poll returns `Some(true)` on a fresh press (rising edge),
//! `Some(false)` otherwise, `None` when its source is unavailable here.
//! Until the graphics hook renders on screen, toggles are echoed to the
//! log file so the input path stays verifiable.
//!
//! Default key: **O** — positional codes everywhere (SDL scancode 18,
//! Unity `KeyCode.O` = 111, X11 keycode 32), plain letter, never
//! swallowed by overlays or window managers.

#[cfg(unix)]
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// SDL scancode for O — the framework menu toggle. Positional HID-based
/// codes (`A = 4 … Z = 29`), so `O = 18` on every layout.
pub const TOGGLE_SCANCODE_O: i32 = 18;

/// C signature of `SDL_GetKeyboardState`: returns a process-lifetime
/// key array and writes its length to the out-pointer.
pub type GetStateFn = unsafe extern "C" fn(*mut libc::c_int) -> *const u8;

/// Whether framework windows (status + manager) should draw.
///
/// Mods keep their own visibility; this flag only gates the two built-in
/// windows in `mod_api`. Defaults to visible.
static UI_VISIBLE: AtomicBool = AtomicBool::new(true);

/// Last observed level of the toggle key, for edge detection.
static WAS_DOWN: AtomicBool = AtomicBool::new(false);

/// Same, for the Unity-`Input` path below (independent source, own edge).
static WAS_DOWN_UNITY: AtomicBool = AtomicBool::new(false);

/// Same, for the X11 path below.
static WAS_DOWN_X11: AtomicBool = AtomicBool::new(false);

/// Cached resolver outcome: `None` means SDL input is unavailable here
/// (headless smoke tests, non-SDL games) and every poll short-circuits.
static GET_STATE: OnceLock<Option<GetStateFn>> = OnceLock::new();

/// Current framework-UI visibility. See [`TOGGLE_SCANCODE_O`].
#[inline]
pub fn ui_visible() -> bool {
    UI_VISIBLE.load(Ordering::SeqCst)
}

/// Flip framework-UI visibility, returning the new state.
#[inline]
pub fn toggle_ui() -> bool {
    !UI_VISIBLE.fetch_xor(true, Ordering::SeqCst)
}

/// Resolve `SDL_GetKeyboardState` once: global scope first, then an
/// explicit handle on the SDL2 runtime — `RTLD_NOLOAD` first so we never
/// pull a second SDL copy into the game, exactly like community mods do.
/// Resolve `SDL_GetKeyboardState` once (cached): global scope first,
/// then an explicit handle on the SDL2 runtime — `RTLD_NOLOAD` first so
/// we never pull a second SDL copy into the game.
fn resolve_get_state() -> Option<GetStateFn> {
    *GET_STATE.get_or_init(platform_resolve)
}

/// Unix resolver: `dlsym` + `dlopen` (see [`resolve_get_state`]).
#[cfg(unix)]
fn platform_resolve() -> Option<GetStateFn> {
    // SAFETY: `dlsym`/`dlopen` with valid NUL-terminated names;
    // every returned pointer is null-checked before transmuting.
    unsafe {
        let name = CString::new("SDL_GetKeyboardState").ok()?;
        let p = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr());
        if !p.is_null() {
            return Some(std::mem::transmute::<*mut libc::c_void, GetStateFn>(p));
        }
        let lib = CString::new("libSDL2-2.0.so.0").ok()?;
        let mut h = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY | libc::RTLD_NOLOAD);
        if h.is_null() {
            h = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY);
        }
        if h.is_null() {
            return None;
        }
        let p = libc::dlsym(h, name.as_ptr());
        if p.is_null() {
            return None;
        }
        Some(std::mem::transmute::<*mut libc::c_void, GetStateFn>(p))
    }
}

/// Windows resolver: `GetModuleHandleW` on `SDL2.dll` (Unity links it;
/// already loaded in-game), falling back to `LoadLibraryW` by bare name.
/// `GetProcAddress` wants narrow names, module names are wide — hence
/// the two encodings below.
///
/// Syntax-checked on Linux; first type-check happens on a Windows build.
#[cfg(windows)]
fn platform_resolve() -> Option<GetStateFn> {
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleW, GetProcAddress, LoadLibraryW,
    };
    let module: Vec<u16> = "SDL2.dll\0".encode_utf16().collect();
    // SAFETY: valid NUL-terminated names on both sides; every handle and
    // pointer is null-checked. `extern "system"` and `extern "C"` share
    // the calling convention on x86_64 Windows, so the transmute below
    // preserves call semantics.
    unsafe {
        let mut h = GetModuleHandleW(module.as_ptr());
        if h == 0 {
            h = LoadLibraryW(module.as_ptr());
        }
        if h == 0 {
            return None;
        }
        match GetProcAddress(h, c"SDL_GetKeyboardState".as_ptr() as *const u8) {
            Some(f) => Some(std::mem::transmute(f)),
            None => None,
        }
    }
}

/// Poll the toggle key once; returns `Some(true)` on a fresh press
/// (rising edge), `Some(false)` otherwise, `None` when SDL input is
/// unavailable in this process.
pub fn poll_toggle_edge() -> Option<bool> {
    poll_scancode_edge(TOGGLE_SCANCODE_O)
}

/// Whether the SDL path is usable at all in this process (resolver hit).
///
/// Diagnostic only: lets the log state plainly which input sources are
/// alive (`sdl live/dead`, alongside the Unity line in `lib.rs`), so a
/// dead key can be attributed instead of guessed at.
pub fn sdl_available() -> bool {
    resolve_get_state().is_some()
}

/// Poll **Unity's** `Input.GetKey(O)` level with our own edge detector.
///
/// Primary in-game path: it reads the input state the engine itself
/// maintains, so it works regardless of SDL availability or window
/// focus quirks. Returns `None` outside Unity (no API/cache yet), so
/// callers can OR it with the SDL path.
///
/// # Safety
/// Same contract as `crate::il2cpp::find_class`: attached thread only.
pub unsafe fn poll_unity_edge(
    api: &crate::il2cpp::Il2cppApi,
    cache: &crate::unity::UnityCache,
) -> Option<bool> {
    if cache.m_get_key.is_null() {
        return None;
    }
    // SAFETY: cached static getter; level read, no side effects.
    let down = unsafe { crate::unity::key_held(api, cache, crate::unity::KEYCODE_O) };
    let was = WAS_DOWN_UNITY.swap(down, Ordering::SeqCst);
    Some(down && !was)
}

/// X11 keycode for the O toggle — positional codes, so 32 is the physical
/// key right of I on both QWERTY and AZERTY.
pub const X11_KEYCODE_O: u32 = 32;

/// `Display* XOpenDisplay(const char*)` — NULL opens `$DISPLAY`.
type XOpenDisplayFn = unsafe extern "C" fn(*const libc::c_char) -> *mut std::ffi::c_void;
/// `int XQueryKeymap(Display*, char[32])` — server-wide key bitmap.
type XQueryKeymapFn =
    unsafe extern "C" fn(*mut std::ffi::c_void, *mut libc::c_char) -> libc::c_int;

/// Our own X connection. Opened once, used from the tick thread only.
struct DisplayHandle(*mut std::ffi::c_void);
// SAFETY: single-threaded use (tick thread); never shared.
unsafe impl Send for DisplayHandle {}
unsafe impl Sync for DisplayHandle {}

/// Cached X11 entry points (`None` = no libX11: pure-Wayland session
/// without XWayland; every poll short-circuits).
static X11_FNS: OnceLock<Option<(XOpenDisplayFn, XQueryKeymapFn)>> = OnceLock::new();
/// Cached connection (null sentinel = open failed, never retried).
static X11_DISPLAY: OnceLock<DisplayHandle> = OnceLock::new();

/// Resolve libX11 once: the game's own symbols first (Unity links X11
/// for its native keyboard/mouse), then an explicit `libX11.so.6`.
fn resolve_x11() -> Option<(XOpenDisplayFn, XQueryKeymapFn)> {
    *X11_FNS.get_or_init(|| {
        // SAFETY: `dlsym`/`dlopen` with valid NUL-terminated names;
        // every returned pointer is null-checked before transmuting.
        unsafe {
            let open_name = CString::new("XOpenDisplay").ok()?;
            let query_name = CString::new("XQueryKeymap").ok()?;
            let mut open_p = libc::dlsym(libc::RTLD_DEFAULT, open_name.as_ptr());
            let mut query_p = libc::dlsym(libc::RTLD_DEFAULT, query_name.as_ptr());
            if open_p.is_null() || query_p.is_null() {
                let lib = CString::new("libX11.so.6").ok()?;
                let h = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY);
                if h.is_null() {
                    return None;
                }
                open_p = libc::dlsym(h, open_name.as_ptr());
                query_p = libc::dlsym(h, query_name.as_ptr());
            }
            if open_p.is_null() || query_p.is_null() {
                return None;
            }
            Some((
                std::mem::transmute::<*mut libc::c_void, XOpenDisplayFn>(open_p),
                std::mem::transmute::<*mut libc::c_void, XQueryKeymapFn>(query_p),
            ))
        }
    })
}

/// Our X connection, opened once on `$DISPLAY` (the game's own, i.e. the
/// XWayland socket for native X11 games on Wayland sessions).
fn x11_display(open: XOpenDisplayFn) -> *mut std::ffi::c_void {
    X11_DISPLAY
        .get_or_init(|| {
            // SAFETY: NULL display name = `$DISPLAY`; null-checked below.
            DisplayHandle(unsafe { open(std::ptr::null()) })
        })
        .0
}

/// Whether the X11 path is usable (libX11 present + display opened).
///
/// Diagnostic only: reported once in the log next to the SDL/Unity
/// states, so a dead key can be attributed instead of guessed at.
pub fn x11_available() -> bool {
    let (open, _) = match resolve_x11() {
        Some(f) => f,
        None => return false,
    };
    !x11_display(open).is_null()
}

/// Poll the O key straight from the X server.
///
/// Server-wide bitmap, focus-independent: immune to SDL pump state,
/// `SDL_VIDEODRIVER` mismatches and Unity thread affinity alike. Under
/// XWayland this reflects the compositor-forwarded keyboard state, which
/// is exactly what the game itself consumes.
pub fn poll_x11_edge() -> Option<bool> {
    let (open, query) = resolve_x11()?;
    let disp = x11_display(open);
    if disp.is_null() {
        return None;
    }
    let mut keys = [0 as libc::c_char; 32];
    // SAFETY: `keys` is exactly the 32 bytes `XQueryKeymap` writes.
    unsafe {
        query(disp, keys.as_mut_ptr());
    }
    let down = (keys[(X11_KEYCODE_O / 8) as usize] as u8) & (1u8 << (X11_KEYCODE_O % 8)) != 0;
    let was = WAS_DOWN_X11.swap(down, Ordering::SeqCst);
    Some(down && !was)
}

/// Rising-edge detector for an arbitrary SDL scancode.
///
/// Cheap enough for a ~10 Hz poll: one cached function call, no locks
/// beyond the atomic edge state.
pub fn poll_scancode_edge(scancode: i32) -> Option<bool> {
    if scancode <= 0 {
        return None;
    }
    let get = resolve_get_state()?;
    // SAFETY: `get` is the validated SDL export; the returned array is
    // owned by SDL and stays valid for the process lifetime.
    let (state, n) = unsafe {
        let mut n: libc::c_int = 0;
        let state = get(&mut n as *mut libc::c_int);
        (state, n)
    };
    if state.is_null() || scancode >= n {
        return None;
    }
    // SAFETY: bounds checked against SDL's own count just above.
    let down = unsafe { *state.offset(scancode as isize) } != 0;
    let was = WAS_DOWN.swap(down, Ordering::SeqCst);
    Some(down && !was)
}
