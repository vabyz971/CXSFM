//! Gamepad input via the game's own SDL2 (dlsym, zero new input stack).
//!
//! Opens joystick 0 lazily (`SDL_InitSubSystem(SDL_INIT_JOYSTICK)` is
//! ref-counted and safe to call when the game already inited SDL).
//! All symbols are optional: anything missing degrades to
//! `connected: false`, never a crash. Standard Xbox layout assumed
//! (LX=0, LY=1, RX=3, RY=4, LB=4, RB=5, A=0); raw values are logged
//! once on open so odd mappings stay diagnosable.

use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

/// `int SDL_NumJoysticks(void)`.
type NumJoysticksFn = unsafe extern "C" fn() -> libc::c_int;
/// `void* SDL_JoystickOpen(int)`.
type JoystickOpenFn = unsafe extern "C" fn(libc::c_int) -> *mut std::ffi::c_void;
/// `void SDL_JoystickClose(void*)`.
type JoystickCloseFn = unsafe extern "C" fn(*mut std::ffi::c_void);
/// `int16_t SDL_JoystickGetAxis(void*, int)`.
type JoystickGetAxisFn = unsafe extern "C" fn(*mut std::ffi::c_void, libc::c_int) -> i16;
/// `uint8_t SDL_JoystickGetButton(void*, int)`.
type JoystickGetButtonFn = unsafe extern "C" fn(*mut std::ffi::c_void, libc::c_int) -> u8;
/// `void SDL_JoystickUpdate(void)`.
type JoystickUpdateFn = unsafe extern "C" fn();
/// `int SDL_InitSubSystem(uint32_t)` (`SDL_INIT_JOYSTICK = 0x200`).
type InitSubSystemFn = unsafe extern "C" fn(u32) -> libc::c_int;
/// `const char* SDL_JoystickName(void*)` (diagnostics).
type JoystickNameFn = unsafe extern "C" fn(*mut std::ffi::c_void) -> *const libc::c_char;

struct GamepadApi {
    num: NumJoysticksFn,
    open: JoystickOpenFn,
    #[allow(dead_code)]
    close: JoystickCloseFn,
    axis: JoystickGetAxisFn,
    button: JoystickGetButtonFn,
    update: JoystickUpdateFn,
    name: Option<JoystickNameFn>,
}

/// Resolved once; `None` when SDL has no joystick API in scope.
static API: OnceLock<Option<GamepadApi>> = OnceLock::new();
/// Opened joystick handle (process lifetime — never closed, like the
/// game's own handle). Raw pointer in a static: guarded by the mutex
/// and only ever touched on the tick thread.
struct Handle(*mut std::ffi::c_void);
// SAFETY: single-owner pattern via the mutex below; SDL handles are
// process-global opaque cookies.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

/// Opened joystick (cached process-wide).
static HANDLE: OnceLock<Mutex<Handle>> = OnceLock::new();
/// Logged-once markers.
static LOGGED_OPEN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `dlsym` from the global scope, then `libSDL2` by soname.
unsafe fn sym(name: &str) -> Option<*mut std::ffi::c_void> {
    // SAFETY: read-only symbol query with a valid NUL-terminated name.
    unsafe {
        let cname = CString::new(name).ok()?;
        let p = libc::dlsym(libc::RTLD_DEFAULT, cname.as_ptr());
        if !p.is_null() {
            return Some(p);
        }
        let lib = CString::new("libSDL2-2.0.so.0").ok()?;
        let mut h = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY | libc::RTLD_NOLOAD);
        if h.is_null() {
            h = libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY);
        }
        if h.is_null() {
            return None;
        }
        let p = libc::dlsym(h, cname.as_ptr());
        if p.is_null() {
            None
        } else {
            Some(p)
        }
    }
}

fn api() -> Option<&'static GamepadApi> {
    API.get_or_init(|| {
        // SAFETY: each pointer is transmuted to its documented SDL
        // signature exactly; every use below is null-checked first.
        unsafe {
            let num: NumJoysticksFn = std::mem::transmute(sym("SDL_NumJoysticks")?);
            let open: JoystickOpenFn = std::mem::transmute(sym("SDL_JoystickOpen")?);
            let close: JoystickCloseFn = std::mem::transmute(sym("SDL_JoystickClose")?);
            let axis: JoystickGetAxisFn = std::mem::transmute(sym("SDL_JoystickGetAxis")?);
            let button: JoystickGetButtonFn =
                std::mem::transmute(sym("SDL_JoystickGetButton")?);
            let update: JoystickUpdateFn = std::mem::transmute(sym("SDL_JoystickUpdate")?);
            let init: InitSubSystemFn = std::mem::transmute(sym("SDL_InitSubSystem")?);
            // Joystick subsystem up (ref-counted, no-op if already on).
            if init(0x200) != 0 {
                return None;
            }
            let name: Option<JoystickNameFn> =
                sym("SDL_JoystickName").map(|p| std::mem::transmute(p));
            Some(GamepadApi {
                num,
                open,
                close,
                axis,
                button,
                update,
                name,
            })
        }
    })
    .as_ref()
}

/// Deadzone-applied, normalized axis (-1.0..=1.0).
fn norm(raw: i16) -> f32 {
    const DEAD: f32 = 0.12;
    let v = (raw as f32 / 32767.0).clamp(-1.0, 1.0);
    if v.abs() < DEAD {
        0.0
    } else {
        (v - DEAD.copysign(v)) / (1.0 - DEAD)
    }
}

/// Snapshot of joystick 0 (all zeros/false when absent).
#[derive(Clone, Copy, Default, Debug)]
pub struct GamepadState {
    pub connected: bool,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    /// Buttons 0..8 (Xbox: 0=A, 4=LB, 5=RB).
    pub buttons: [bool; 8],
}

/// Poll joystick 0 (tick thread). Opens lazily on first call.
pub fn poll() -> GamepadState {
    let api = match api() {
        Some(a) => a,
        None => return GamepadState::default(),
    };
    // SAFETY: resolved SDL signatures; handle cached process-wide.
    unsafe {
        if (api.num)() <= 0 {
            return GamepadState::default();
        }
        let slot = HANDLE.get_or_init(|| Mutex::new(Handle(std::ptr::null_mut())));
        let mut handle = slot.lock().unwrap_or_else(|e| e.into_inner());
        if handle.0.is_null() {
            handle.0 = (api.open)(0);
            if handle.0.is_null() {
                return GamepadState::default();
            }
            if !LOGGED_OPEN.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let name = api.name.and_then(|n| {
                    let p = n(handle.0);
                    if p.is_null() {
                        None
                    } else {
                        Some(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
                    }
                });
                crate::log_line(&format!(
                    "gamepad: opened joystick 0 ({})",
                    name.as_deref().unwrap_or("unnamed")
                ));
            }
        }
        let joy = handle.0;
        drop(handle);
        (api.update)();
        let mut buttons = [false; 8];
        for (i, b) in buttons.iter_mut().enumerate() {
            *b = (api.button)(joy, i as libc::c_int) != 0;
        }
        GamepadState {
            connected: true,
            lx: norm((api.axis)(joy, 0)),
            ly: norm((api.axis)(joy, 1)),
            rx: norm((api.axis)(joy, 3)),
            ry: norm((api.axis)(joy, 4)),
            buttons,
        }
    }
}
