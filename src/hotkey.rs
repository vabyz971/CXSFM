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
//! Default keys (positional codes everywhere — same physical key on
//! every layout):
//!
//! * **O** (SDL scancode 18, Unity `KeyCode.O` = 111, X11 keycode 32):
//!   framework menu toggle — plain letter, never swallowed by overlays
//!   or window managers.
//! * **F9** (SDL scancode 66, Unity `KeyCode.F9` = 290, X11 keycode 75):
//!   UI capture toggle (grab input for the overlay, see below).
//!
//! # UI capture (X11 grab + event thread)
//!
//! Showing the UI is not interacting with it: while the game owns the
//! input, every click would also drive the game. Toggling capture
//! (`set_captured(true)`) grabs pointer + keyboard to our own invisible
//! input window, so the game receives NOTHING and every event reaches
//! egui instead. A dedicated thread pumps `XNextEvent` into
//! [`drain_input_events`]; the overlay consumes the queue each frame
//! and paints a software cursor (the game hides/confines the OS one).

#[cfg(unix)]
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::collections::HashMap;

/// SDL scancode for O — the framework menu toggle. Positional HID-based
/// codes (`A = 4 … Z = 29`), so `O = 18` on every layout.
pub const TOGGLE_SCANCODE_O: i32 = 18;
/// SDL scancode for F9 — the UI capture toggle (`F1 = 58`, so `F9 = 66`).
pub const CAPTURE_SCANCODE_F9: i32 = 66;
/// SDL scancode for F8 — the combined UI + capture toggle (`F8 = 65`).
pub const COMBO_SCANCODE_F8: i32 = 65;

/// C signature of `SDL_GetKeyboardState`: returns a process-lifetime
/// key array and writes its length to the out-pointer.
pub type GetStateFn = unsafe extern "C" fn(*mut libc::c_int) -> *const u8;

/// Whether framework windows (status + manager) should draw.
///
/// Mods keep their own visibility; this flag only gates the two built-in
/// windows in `mod_api`. Defaults to HIDDEN: the framework boots
/// quiet and the user opts in with O/F8 (nothing on screen until an
/// explicit action — same rule as mods, which register disabled).
static UI_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Last observed key levels, per (source, code), for edge detection.
///
/// One static per key (the old design) breaks as soon as two keys share
/// a source (O + F9): levels would overwrite each other and edges get
/// lost. A tiny map costs nothing at ~10 Hz poll rates.
static EDGE_STATES: OnceLock<Mutex<HashMap<(u8, u32), bool>>> = OnceLock::new();

/// Edge detector shared by all input sources: returns true on a fresh
/// press (rising edge) of `(source, code)`.
fn edge(source: u8, code: u32, down: bool) -> bool {
    let mut states = EDGE_STATES.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    let was = states.get(&(source, code)).copied().unwrap_or(false);
    states.insert((source, code), down);
    down && !was
}

/// Input sources for [`edge`].
const SRC_SDL: u8 = 1;
const SRC_UNITY: u8 = 2;
const SRC_X11: u8 = 3;

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

/// Poll the O toggle key once; returns `Some(true)` on a fresh press
/// (rising edge), `Some(false)` otherwise, `None` when SDL input is
/// unavailable in this process.
pub fn poll_toggle_edge() -> Option<bool> {
    poll_scancode_edge(TOGGLE_SCANCODE_O)
}

/// Poll the F9 capture key the same way (see [`poll_toggle_edge`]).
pub fn poll_capture_edge() -> Option<bool> {
    poll_scancode_edge(CAPTURE_SCANCODE_F9)
}

/// Whether the SDL path is usable at all in this process (resolver hit).
///
/// Diagnostic only: lets the log state plainly which input sources are
/// alive (`sdl live/dead`, alongside the Unity line in `lib.rs`), so a
/// dead key can be attributed instead of guessed at.
pub fn sdl_available() -> bool {
    resolve_get_state().is_some()
}

/// Poll **Unity's** `Input.GetKey` level with our own edge detector.
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
    keycode: i32,
) -> Option<bool> {
    if cache.m_get_key.is_null() {
        return None;
    }
    // SAFETY: cached static getter; level read, no side effects.
    let down = unsafe { crate::unity::key_held(api, cache, keycode) };
    Some(edge(SRC_UNITY, keycode as u32, down))
}

/// X11 keycode for the O toggle — positional codes, so 32 is the physical
/// key right of I on both QWERTY and AZERTY.
pub const X11_KEYCODE_O: u32 = 32;
/// X11 keycode for F9 — the UI capture toggle.
pub const X11_KEYCODE_F9: u32 = 75;
/// X11 keycode for F8 — the combined UI + capture toggle.
pub const X11_KEYCODE_F8: u32 = 74;

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

/// Poll a key straight from the X server.
///
/// Server-wide bitmap, focus-independent: immune to SDL pump state,
/// `SDL_VIDEODRIVER` mismatches and Unity thread affinity alike. Under
/// XWayland this reflects the compositor-forwarded keyboard state, which
/// is exactly what the game itself consumes.
pub fn poll_x11_edge() -> Option<bool> {
    poll_x11_key_edge(X11_KEYCODE_O)
}

/// Same for an arbitrary X11 keycode (F9 capture toggle).
pub fn poll_x11_key_edge(keycode: u32) -> Option<bool> {
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
    let byte = (keycode / 8) as usize;
    if byte >= keys.len() {
        return None;
    }
    let down = (keys[byte] as u8) & (1u8 << (keycode % 8)) != 0;
    Some(edge(SRC_X11, keycode, down))
}

/// Rising-edge detector for an arbitrary SDL scancode.
///
/// Cheap enough for a ~10 Hz poll: one cached function call, edge state
/// in a tiny map (see [`edge`]).
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
    Some(edge(SRC_SDL, scancode as u32, down))
}

// --- UI capture: X11 grab + event pump ------------------------------------------
// X event types we pump.
const X_KEY_PRESS: i32 = 2;
const X_KEY_RELEASE: i32 = 3;
const X_BUTTON_PRESS: i32 = 4;
const X_BUTTON_RELEASE: i32 = 5;
const X_MOTION: i32 = 6;
// Event masks for the grab.
const X_BUTTON_PRESS_MASK: u32 = 1 << 2;
const X_BUTTON_RELEASE_MASK: u32 = 1 << 3;
const X_POINTER_MOTION_MASK: u32 = 1 << 6;
// Grab modes + results.
const X_GRAB_MODE_ASYNC: i32 = 1;
const X_GRAB_SUCCESS: i32 = 0;
// X modifier bits (state field). Mod1 is conventionally Alt; Super
// has no egui key (modifiers only) and is ignored.
const X_SHIFT_MASK: u32 = 1;
const X_CONTROL_MASK: u32 = 1 << 2;
const X_MOD1_MASK: u32 = 1 << 3;

/// Extra libX11 entry points for capture (resolved alongside the base
/// pair; all are stable ABI since X11R6).
type XDefaultRootWindowFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> u64;
type XCreateWindowFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    u64,
    i32,
    i32,
    u32,
    u32,
    u32,
    i32,
    i32,
    *mut std::ffi::c_void,
    u64,
    *mut std::ffi::c_void,
) -> u64;
type XMapWindowFn = unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> i32;
type XGrabPointerFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    u64,
    i32,
    u32,
    i32,
    i32,
    u64,
    u64,
    u64,
) -> i32;
type XGrabKeyboardFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    u64,
    i32,
    i32,
    i32,
    u64,
) -> i32;
type XUngrabPointerFn = unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> i32;
type XUngrabKeyboardFn = unsafe extern "C" fn(*mut std::ffi::c_void, u64) -> i32;
type XNextEventFn =
    unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32;
type XLookupStringFn = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *mut libc::c_char,
    i32,
    *mut u64,
    *mut std::ffi::c_void,
) -> i32;
type XInitThreadsFn = unsafe extern "C" fn() -> i32;
/// `XErrorHandler XSetErrorHandler(int (*handler)(Display*, XErrorEvent*))`.
type XSetErrorHandlerFn = unsafe extern "C" fn(
    Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32>,
) -> Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> i32>;

/// Swallow X protocol errors instead of killing the process.
///
/// Xlib's default error handler prints and calls `exit(1)` — inside an
/// injected game library that means instant death for a mismatched
/// visual, a raced window id, anything. Installed on our own display
/// connections; the game's own X usage is untouched.
unsafe extern "C" fn ignore_xerror(
    _disp: *mut std::ffi::c_void,
    _event: *mut std::ffi::c_void,
) -> i32 {
    0
}

/// Resolved capture functions (or `None`: no X11 capture here).
struct CaptureFns {
    open: XOpenDisplayFn,
    root: XDefaultRootWindowFn,
    create: XCreateWindowFn,
    map: XMapWindowFn,
    grab_ptr: XGrabPointerFn,
    grab_kbd: XGrabKeyboardFn,
    ungrab_ptr: XUngrabPointerFn,
    ungrab_kbd: XUngrabKeyboardFn,
    next_event: XNextEventFn,
    lookup: XLookupStringFn,
    set_handler: XSetErrorHandlerFn,
}

static CAPTURE_FNS: OnceLock<Option<CaptureFns>> = OnceLock::new();
/// Event-thread display + grab window (owned by the event thread only).
static CAPTURE_STARTED: AtomicBool = AtomicBool::new(false);
/// Whether input is currently grabbed for the overlay.
static CAPTURED: AtomicBool = AtomicBool::new(false);
/// egui events pumped by the event thread, drained per frame.
static EVENT_QUEUE: OnceLock<Mutex<Vec<egui::Event>>> = OnceLock::new();
/// Last known pointer position in root/screen pixels.
static CURSOR_POS: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

/// Resolve the capture entry points (implies `XInitThreads`, once —
/// the tick and event threads both touch Xlib on their own displays).
fn resolve_capture() -> Option<&'static CaptureFns> {
    CAPTURE_FNS
        .get_or_init(|| {
            // SAFETY: dlsym/dlopen with valid names; null-checked.
            unsafe {
                macro_rules! sym {
                    ($handle:expr, $name:expr, $ty:ty) => {{
                        let n = CString::new($name).ok()?;
                        let p = libc::dlsym($handle, n.as_ptr());
                        if p.is_null() {
                            return None;
                        }
                        Some(std::mem::transmute::<*mut libc::c_void, $ty>(p))
                    }};
                }
                let mut h = libc::dlopen(
                    CString::new("libX11.so.6").ok()?.as_ptr(),
                    libc::RTLD_LAZY | libc::RTLD_NOLOAD,
                );
                if h.is_null() {
                    // The game links X11 (Unity native input) — usually
                    // resolvable globally without opening anything.
                    h = libc::RTLD_DEFAULT as *mut std::ffi::c_void;
                }
                // XInitThreads once: the tick and event threads both
                // touch Xlib on their own displays.
                {
                    let n = CString::new("XInitThreads").ok()?;
                    let p = libc::dlsym(h, n.as_ptr());
                    if !p.is_null() {
                        let f = std::mem::transmute::<
                            *mut libc::c_void,
                            XInitThreadsFn,
                        >(p);
                        f();
                    }
                }
                {
                    let n = CString::new("XInitThreads").ok()?;
                    let p = libc::dlsym(h, n.as_ptr());
                    if !p.is_null() {
                        let f = std::mem::transmute::<
                            *mut libc::c_void,
                            XInitThreadsFn,
                        >(p);
                        f();
                    }
                }
                Some(CaptureFns {
                    open: sym!(h, "XOpenDisplay", XOpenDisplayFn)?,
                    root: sym!(h, "XDefaultRootWindow", XDefaultRootWindowFn)?,
                    create: sym!(h, "XCreateWindow", XCreateWindowFn)?,
                    map: sym!(h, "XMapWindow", XMapWindowFn)?,
                    grab_ptr: sym!(h, "XGrabPointer", XGrabPointerFn)?,
                    grab_kbd: sym!(h, "XGrabKeyboard", XGrabKeyboardFn)?,
                    ungrab_ptr: sym!(h, "XUngrabPointer", XUngrabPointerFn)?,
                    ungrab_kbd: sym!(h, "XUngrabKeyboard", XUngrabKeyboardFn)?,
                    next_event: sym!(h, "XNextEvent", XNextEventFn)?,
                    lookup: sym!(h, "XLookupString", XLookupStringFn)?,
                    set_handler: sym!(h, "XSetErrorHandler", XSetErrorHandlerFn)?,
                })
            }
        })
        .as_ref()
}

/// Whether UI capture is available (all entry points resolved and a
/// display opens). Diagnostic: reported in the framework log.
pub fn capture_available() -> bool {
    let fns = match resolve_capture() {
        Some(f) => f,
        None => return false,
    };
    // SAFETY: NULL display name = `$DISPLAY`; closed immediately.
    // Note: XCloseDisplay omitted on purpose for a probe (leaks one
    // connection per process lifetime at most — the event thread opens
    // its own long-lived one below anyway).
    unsafe { (fns.open)(std::ptr::null()) }.is_null() == false
}

/// Whether input is currently grabbed for the overlay.
#[inline]
pub fn is_captured() -> bool {
    CAPTURED.load(Ordering::SeqCst)
}

/// Last known pointer position (root pixels), for the software cursor.
pub fn cursor_pos() -> (f32, f32) {
    *CURSOR_POS.lock().unwrap()
}

/// Drain events pumped since the last frame (overlay consumes these).
pub fn drain_input_events() -> Vec<egui::Event> {
    std::mem::take(
        &mut EVENT_QUEUE
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap(),
    )
}

/// X11 keycode -> egui logical key (positional; modifiers and unknowns
/// yield `None` — text still flows via `XLookupString`).
fn x11_key(keycode: u32) -> Option<egui::Key> {
    use egui::Key as K;
    Some(match keycode {
        9 => K::Escape,
        10 => K::Num1,
        11 => K::Num2,
        12 => K::Num3,
        13 => K::Num4,
        14 => K::Num5,
        15 => K::Num6,
        16 => K::Num7,
        17 => K::Num8,
        18 => K::Num9,
        19 => K::Num0,
        20 => K::Minus,
        21 => K::Equals,
        22 => K::Backspace,
        23 => K::Tab,
        24 => K::Q,
        25 => K::W,
        26 => K::E,
        27 => K::R,
        28 => K::T,
        29 => K::Y,
        30 => K::U,
        31 => K::I,
        32 => K::O,
        33 => K::P,
        34 => K::OpenBracket,
        35 => K::CloseBracket,
        36 => K::Enter,
        38 => K::A,
        39 => K::S,
        40 => K::D,
        41 => K::F,
        42 => K::G,
        43 => K::H,
        44 => K::J,
        45 => K::K,
        46 => K::L,
        47 => K::Semicolon,
        48 => K::Quote,
        49 => K::Backtick,
        51 => K::Backslash,
        52 => K::Z,
        53 => K::X,
        54 => K::C,
        55 => K::V,
        56 => K::B,
        57 => K::N,
        58 => K::M,
        59 => K::Comma,
        60 => K::Period,
        61 => K::Slash,
        65 => K::Space,
        67 => K::F1,
        68 => K::F2,
        69 => K::F3,
        70 => K::F4,
        71 => K::F5,
        72 => K::F6,
        73 => K::F7,
        74 => K::F8,
        75 => K::F9,
        76 => K::F10,
        79 => K::Num7,
        80 => K::Num8,
        81 => K::Num9,
        82 => K::Minus,
        83 => K::Num4,
        84 => K::Num5,
        85 => K::Num6,
        86 => K::Plus,
        87 => K::Num1,
        88 => K::Num2,
        89 => K::Num3,
        90 => K::Num0,
        91 => K::Period,
        95 => K::F11,
        96 => K::F12,
        104 => K::Enter,
        106 => K::Plus,
        110 => K::Home,
        111 => K::ArrowUp,
        112 => K::PageUp,
        113 => K::ArrowLeft,
        114 => K::ArrowRight,
        115 => K::End,
        116 => K::ArrowDown,
        117 => K::PageDown,
        118 => K::Insert,
        119 => K::Delete,
        _ => return None,
    })
}

/// X event `state` bitmask -> egui modifiers.
fn x11_modifiers(state: u32) -> egui::Modifiers {
    let ctrl = state & X_CONTROL_MASK != 0;
    egui::Modifiers {
        alt: state & X_MOD1_MASK != 0,
        ctrl,
        shift: state & X_SHIFT_MASK != 0,
        mac_cmd: false,
        command: ctrl,
    }
}

/// Translate one raw X event (192-byte buffer) into egui events.
///
/// Layout (LP64): `type@0 i32, x_root@72 i32, y_root@76 i32,
/// state@80 u32, detail(keycode/button)@84 u32`. `XLookupString`
/// reads the embedded display/keycode/state straight from the buffer.
fn translate_xevent(buf: &[u8; 192], fns: &CaptureFns) -> Vec<egui::Event> {
    let ty = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let x = i32::from_ne_bytes([buf[72], buf[73], buf[74], buf[75]]) as f32;
    let y = i32::from_ne_bytes([buf[76], buf[77], buf[78], buf[79]]) as f32;
    let state = u32::from_ne_bytes([buf[80], buf[81], buf[82], buf[83]]);
    let detail = u32::from_ne_bytes([buf[84], buf[85], buf[86], buf[87]]);
    let pos = egui::pos2(x, y);
    let mods = x11_modifiers(state);
    match ty {
        X_MOTION => {
            *CURSOR_POS.lock().unwrap() = (x, y);
            vec![egui::Event::PointerMoved(pos)]
        }
        X_BUTTON_PRESS | X_BUTTON_RELEASE => {
            let pressed = ty == X_BUTTON_PRESS;
            *CURSOR_POS.lock().unwrap() = (x, y);
            match detail {
                // Wheel (no position in the event itself; hover known).
                4 => vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, 1.0),
                    modifiers: mods,
                }],
                5 => vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, -1.0),
                    modifiers: mods,
                }],
                6 => vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(-1.0, 0.0),
                    modifiers: mods,
                }],
                7 => vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(1.0, 0.0),
                    modifiers: mods,
                }],
                b => {
                    let button = match b {
                        1 => egui::PointerButton::Primary,
                        2 => egui::PointerButton::Middle,
                        3 => egui::PointerButton::Secondary,
                        8 => egui::PointerButton::Extra1,
                        9 => egui::PointerButton::Extra2,
                        _ => return Vec::new(),
                    };
                    vec![egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: mods,
                    }]
                }
            }
        }
        X_KEY_PRESS | X_KEY_RELEASE => {
            let pressed = ty == X_KEY_PRESS;
            let mut out = Vec::with_capacity(2);
            if let Some(key) = x11_key(detail) {
                out.push(egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers: mods,
                });
            }
            // Printable text (layout-correct via the server). Skip
            // control keys that already have Key events with meaning.
            if pressed && !matches!(detail, 9 | 22 | 23 | 36 | 119) {
                let mut text = [0 as libc::c_char; 32];
                let mut keysym: u64 = 0;
                // SAFETY: real grabbed KeyPress event; bounded buffer.
                let n = unsafe {
                    (fns.lookup)(
                        buf.as_ptr() as *mut std::ffi::c_void,
                        text.as_mut_ptr(),
                        text.len() as i32,
                        &mut keysym as *mut u64,
                        std::ptr::null_mut(),
                    )
                };
                if n > 0 {
                    let s = String::from_utf8_lossy(unsafe {
                        std::slice::from_raw_parts(text.as_ptr() as *const u8, n as usize)
                    })
                    .into_owned();
                    if !s.is_empty()
                        && s.chars().all(|c| !c.is_control() || c == '\t')
                        && detail != 23
                    {
                        out.push(egui::Event::Text(s));
                    }
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Event pump thread: blocks in `XNextEvent` on its OWN display
/// connection (never shared with the tick thread's polling display)
/// and appends translated events to [`EVENT_QUEUE`].
fn spawn_event_thread() {
    std::thread::Builder::new()
        .name("cxsfm-xinput".into())
        .spawn(|| {
            let fns = match resolve_capture() {
                Some(f) => f,
                None => return,
            };
            // SAFETY: NULL display name = `$DISPLAY`.
            let disp = unsafe { (fns.open)(std::ptr::null()) };
            if disp.is_null() {
                return;
            }
            // SAFETY: our own connection; swallows protocol errors that
            // would otherwise exit(1) the game process.
            unsafe {
                (fns.set_handler)(Some(ignore_xerror));
            }
            // SAFETY: valid display; 1x1 invisible InputOnly child of
            // root (class 2). Never shown, never receives anything
            // except grabbed events.
            let (_root, win) = unsafe {
                let root = (fns.root)(disp);
                let win = (fns.create)(
                    disp,
                    root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    0,
                    2,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                );
                (root, win)
            };
            if win == 0 {
                return;
            }
            // SAFETY: map our (invisible) window so grabs accept it.
            unsafe {
                (fns.map)(disp, win);
            }
            // Stash for grab/ungrab below.
            GRAB_CTX.get_or_init(|| Mutex::new(GrabCtx { disp, win }));
            let mut buf = [0u8; 192];
            loop {
                // SAFETY: valid display + 192-byte XEvent buffer.
                unsafe {
                    (fns.next_event)(disp, buf.as_mut_ptr() as *mut std::ffi::c_void);
                }
                // Only translate while captured (stray events from the
                // grab window's own mapping are ignored otherwise).
                if !is_captured() {
                    continue;
                }
                let events = translate_xevent(&buf, fns);
                if !events.is_empty() {
                    EVENT_QUEUE
                        .get_or_init(|| Mutex::new(Vec::new()))
                        .lock()
                        .unwrap()
                        .extend(events);
                }
            }
        })
        .ok();
}

/// Grab target created by the event thread.
struct GrabCtx {
    disp: *mut std::ffi::c_void,
    win: u64,
}
// SAFETY: written once before any grab; read from the tick thread.
unsafe impl Send for GrabCtx {}
unsafe impl Sync for GrabCtx {}
static GRAB_CTX: OnceLock<Mutex<GrabCtx>> = OnceLock::new();

/// Start the X11 event thread once (idempotent).
///
/// Called at tick-thread startup so the grab window already exists
/// when the user first toggles capture — otherwise the first F8/F9
/// pays the full spawn + X round-trips delay (up to ~0.5 s). Also
/// called from [`set_captured`] as a fallback.
pub fn ensure_capture_thread() {
    if CAPTURE_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // Only worth spawning when X11 resolves at all.
        if resolve_capture().is_some() {
            spawn_event_thread();
        } else {
            CAPTURE_STARTED.store(false, Ordering::SeqCst);
        }
    }
}

/// Grab (`on = true`) or release (`false`) all input for the overlay.
///
/// Grabbing routes pointer + keyboard to our invisible window: the game
/// receives NOTHING while captured. Releasing pushes `PointerGone` so
/// egui drops hover state instead of sticking it. Idempotent: already
/// in the requested state returns after a state check (no duplicate
/// grabs or log lines).
pub fn set_captured(on: bool) {
    if is_captured() == on {
        return;
    }
    ensure_capture_thread();
    let fns = match resolve_capture() {
        Some(f) => f,
        None => {
            crate::log_line("input: capture unavailable (no X11 capture path)");
            return;
        }
    };
    // Event thread pre-started above; wait (briefly) for it to
    // publish the grab target.
    let mut waited = 0;
    while GRAB_CTX.get().is_none() && waited < 50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        waited += 1;
    }
    let ctx = match GRAB_CTX.get() {
        Some(c) => c,
        None => {
            crate::log_line("input: capture unavailable (no grab target)");
            return;
        }
    };
    let guard = ctx.lock().unwrap();
    // SAFETY: validated display + window from the event thread.
    unsafe {
        if on {
            let ps = (fns.grab_ptr)(
                guard.disp,
                guard.win,
                0,
                X_BUTTON_PRESS_MASK | X_BUTTON_RELEASE_MASK | X_POINTER_MOTION_MASK,
                X_GRAB_MODE_ASYNC,
                X_GRAB_MODE_ASYNC,
                0,
                0,
                0,
            );
            let ks = (fns.grab_kbd)(guard.disp, guard.win, 0, X_GRAB_MODE_ASYNC, X_GRAB_MODE_ASYNC, 0);
            if ps == X_GRAB_SUCCESS && ks == X_GRAB_SUCCESS {
                CAPTURED.store(true, Ordering::SeqCst);
                crate::log_line("input: UI capture ON (game input held)");
            } else {
                crate::log_line(&format!(
                    "input: grab failed (pointer={ps} keyboard={ks}) — overlay stays passive"
                ));
            }
        } else {
            (fns.ungrab_ptr)(guard.disp, 0);
            (fns.ungrab_kbd)(guard.disp, 0);
            CAPTURED.store(false, Ordering::SeqCst);
            EVENT_QUEUE
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .unwrap()
                .push(egui::Event::PointerGone);
            crate::log_line("input: UI capture OFF (game input released)");
        }
    }
}
