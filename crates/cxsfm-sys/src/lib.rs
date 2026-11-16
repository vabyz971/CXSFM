//! CXSFM game bridge — the FFI crate: process memory, the IL2CPP
//! runtime of the host game, Unity helpers on top of it, FMOD and the
//! main-thread `methodPointer` hooks.
//!
//! `unsafe` lives here behind safe APIs with documented `SAFETY`
//! contracts. Rules proven in game (docs/field-notes.md): `object_alive`
//! before any Unity access, `invoke_void` for setters, objects
//! re-resolved on every use, no IL2CPP during teardown.

pub mod fmod;
pub mod il2cpp;
pub mod mainthread;
pub mod memory;
pub mod unity;

use std::sync::OnceLock;

/// Resolved IL2CPP API of the host game, if it provides one.
///
/// Published once by the framework init thread ([`publish_il2cpp`]);
/// mods query it via [`il2cpp_api`] and must handle `None` (non-Unity
/// process, or scripting not up yet). `Il2cppApi` is shareable: it only
/// holds function pointers.
static IL2CPP: OnceLock<il2cpp::Il2cppApi> = OnceLock::new();

/// Access the host game's IL2CPP API, if resolved.
///
/// Returns `None` outside Unity IL2CPP processes. Mods calling into IL2CPP
/// must first ensure their thread is attached (the init and tick threads
/// already are; see [`il2cpp`] module docs for the rule).
#[inline]
pub fn il2cpp_api() -> Option<&'static il2cpp::Il2cppApi> {
    IL2CPP.get()
}

/// Publish the resolved API (init thread, once). A second call keeps
/// the first value (never happens).
pub fn publish_il2cpp(api: il2cpp::Il2cppApi) {
    let _ = IL2CPP.set(api);
}

/// Teardown probe installed by the render backend (the Vulkan layer
/// tracks swapchains/devices/instances); `None` = never shutting down.
static SHUTDOWN_PROBE: OnceLock<fn() -> bool> = OnceLock::new();

/// Install the teardown probe (framework init, once).
pub fn set_shutdown_probe(probe: fn() -> bool) {
    let _ = SHUTDOWN_PROBE.set(probe);
}

/// Whether the game is tearing down: during teardown Unity kills its
/// scripting domain and ANY IL2CPP call can segfault.
///
/// Only Linux (Vulkan layer) installs a probe yet; elsewhere this stays
/// false (Windows/macOS teardown hardening is future work alongside
/// their render hooks).
#[inline]
pub fn shutting_down() -> bool {
    SHUTDOWN_PROBE.get().is_some_and(|p| p())
}
