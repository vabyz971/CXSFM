//! Symbol shims for Steam Runtime compatibility.
//!
//! # Why this exists
//!
//! Modern glibc re-versions some `libm` symbols when their implementation
//! changes. Our egui dependency tree performs float math (`emath`/`epaint`),
//! which pulls in e.g. `fmod`, and a current toolchain stamps it as
//! `fmod@GLIBC_2.43`. The Steam Linux Runtime (sniper = glibc 2.36) does
//! not provide that version, so `LD_PRELOAD` fails and the game never
//! starts — with no chance to recover at runtime.
//!
//! # How it works
//!
//! The final link passes `-Wl,--wrap=fmod` (see `.cargo/config.toml`,
//! Linux GNU only): every `fmod` reference *inside this crate* is
//! statically redirected to [`__wrap_fmod`] below. The output `.so`
//! therefore carries **no** versioned `fmod` requirement at all.
//!
//! At runtime the wrapper forwards to the host libc's real `fmod`,
//! resolved once via `dlsym(RTLD_NEXT, ...)`. If resolution ever fails,
//! a small self-contained fallback computes the remainder with plain
//! arithmetic (no `libm` calls, so no recursion).
//!
//! # No game interposition
//!
//! Only references *within our own objects* are wrapped (a link-time
//! rewrite, not `LD_PRELOAD` symbol preemption). We deliberately do
//! **not** define a global `fmod`, so the game's own math calls are
//! untouched — critical for a mod framework that must not alter physics.
//!
//! Adding a new wrap (if a future dependency drags in another re-versioned
//! symbol) means: one more `--wrap=` flag plus one more forwarder here.

use std::ffi::CStr;
use std::sync::OnceLock;

/// C ABI signature of the host `fmod`.
type FmodFn = unsafe extern "C" fn(f64, f64) -> f64;

/// The host libc's `fmod`, resolved once on first use.
static REAL_FMOD: OnceLock<FmodFn> = OnceLock::new();

/// Replacement for our crate's internal `fmod` references.
///
/// Linked via `-C link-arg=-Wl,--wrap=fmod`. `#[unsafe(no_mangle)]` keeps
/// the exact symbol name the linker rewrite expects (edition 2024 syntax).
#[unsafe(no_mangle)]
pub extern "C" fn __wrap_fmod(x: f64, y: f64) -> f64 {
    let real = *REAL_FMOD.get_or_init(|| {
        // SAFETY: `dlsym` with a valid handle and a NUL-terminated name;
        // the result is null-checked before use.
        unsafe {
            // `CStr::as_ptr` borrows a `'static` literal: always valid.
            let name: &CStr = c"fmod";
            let sym = libc::dlsym(libc::RTLD_NEXT, name.as_ptr());
            if sym.is_null() {
                return naive_fmod as FmodFn;
            }
            std::mem::transmute::<*mut libc::c_void, FmodFn>(sym)
        }
    });
    // SAFETY: `real` is either the host libm `fmod` or the fallback
    // below, both honoring this exact signature.
    unsafe { real(x, y) }
}

/// Last-resort remainder without any `libm` call.
///
/// Declared `extern "C"` so the function item coerces directly to [`FmodFn`].
/// Only used if `dlsym` fails (practically impossible on glibc, but this
/// keeps the wrapper total). Division is a hardware instruction, so there
/// is no recursion back into a wrapped symbol.
extern "C" fn naive_fmod(x: f64, y: f64) -> f64 {
    if y == 0.0 || !x.is_finite() || !y.is_finite() {
        return f64::NAN;
    }
    x - (x / y).trunc() * y
}
