//! Cross-platform render-hook layer.
//!
//! # Why this exists
//!
//! Mod UIs (`Mod::on_draw_ui`) only appear on screen once per presented
//! frame. Each OS/graphics API needs its own interception point, but the
//! framework above it stays identical:
//!
//! | Platform | Game API (CarX Street) | Hook point (this layer)        |
//! |----------|------------------------|--------------------------------|
//! | Linux    | Vulkan                 | `vkQueuePresentKHR` (GOT patch) |
//! | Windows  | DirectX 11 **and** 12  | `IDXGISwapChain::Present`      |
//! | macOS    | Metal                  | (stub — not implemented)       |
//!
//! Note for Windows: DX11 and DX12 both present through DXGI, so **one**
//! `Present` hook covers both renderers. There is deliberately no
//! separate D3D11-vs-D3D12 path.
//!
//! # Current stage
//!
//! This layer proves *interception*: hooks reroute the present call,
//! bump [`frame_count`], and forward to the original — the game renders
//! exactly as before. Actually *drawing* egui into the frame (command
//! buffers / swapchain image management) is Phase 1b and plugs into
//! [`on_frame`]; until then the headless UI check in `lib.rs` keeps the
//! draw code validated.
//!
//! # Safety model
//!
//! Hooks run on the game's render thread. [`on_frame`] therefore does the
//! absolute minimum (one atomic increment + a single one-time log line):
//! no allocation, no locking, no panics — unwinding through foreign
//! frames would abort the process.

#[cfg(target_os = "linux")]
mod vk;
/// In-process Vulkan overlay (Linux layer path): draws egui into
/// swapchain images at Present time. Fed by `layer.rs` present routing;
/// every failure degrades to an untouched present.
#[cfg(target_os = "linux")]
pub mod overlay;
#[cfg(target_os = "windows")]
mod dxgi;
#[cfg(target_os = "macos")]
mod metal;

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Which graphics API the active hook intercepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    /// Linux: `vkQueuePresentKHR`.
    Vulkan,
    /// Windows: `IDXGISwapChain::Present` (covers DirectX 11 and 12).
    Dxgi,
    /// macOS: Metal drawable present (stub — not implemented).
    Metal,
}

/// Why a render hook could not be installed.
#[derive(Debug)]
pub enum RenderError {
    /// This platform/backend has no hook implementation (yet).
    Unsupported(String),
    /// A hook exists but installation failed (symbol not found, memory
    /// protection refused, …). The string carries the precise reason and
    /// is mirrored to the framework log.
    HookFailed(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Unsupported(msg) => write!(f, "render hook unsupported: {}", msg),
            RenderError::HookFailed(msg) => write!(f, "render hook failed: {}", msg),
        }
    }
}

impl std::error::Error for RenderError {}

/// One platform's present-hook: install, and optionally remove it.
pub trait RenderHook: Send + Sync {
    /// Which backend this hook intercepts.
    fn backend(&self) -> RenderBackend;
    /// Install the hook. Idempotent: a second call is a no-op success.
    fn install(&self) -> Result<(), RenderError>;
    /// Remove the hook, restoring the original call path.
    fn uninstall(&self) -> Result<(), RenderError>;
}

/// True once any backend hook is installed in this process.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Presents intercepted since install. Proves the hook is live; doubles
/// as a cheap frame clock for future mods.
static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
/// Overlay submits performed (UI actually drawn into the frame).
static OVERLAY_DREW: AtomicU64 = AtomicU64::new(0);
/// Overlay attempts that fell back to the untouched present (per-step
/// failures, unknown swapchains, multi-presents...). A high
/// skipped/(drew+skipped) ratio with visible UI means flicker.
static OVERLAY_SKIPPED: AtomicU64 = AtomicU64::new(0);

/// Whether a render hook is currently installed.
#[inline]
pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// Presents intercepted so far (0 until the first routed frame).
#[inline]
pub fn frame_count() -> u64 {
    FRAME_COUNT.load(Ordering::SeqCst)
}

/// Overlay draws performed so far.
#[inline]
pub fn overlay_drew() -> u64 {
    OVERLAY_DREW.load(Ordering::SeqCst)
}

/// Overlay fallbacks to untouched presents so far.
#[inline]
pub fn overlay_skipped() -> u64 {
    OVERLAY_SKIPPED.load(Ordering::SeqCst)
}

/// Record one overlay draw (called on the present path).
#[inline]
pub(crate) fn note_overlay_drew() {
    OVERLAY_DREW.fetch_add(1, Ordering::SeqCst);
}

/// Record one overlay fallback (called on the present path).
#[inline]
pub(crate) fn note_overlay_skipped() {
    OVERLAY_SKIPPED.fetch_add(1, Ordering::SeqCst);
}

/// Per-present callback, invoked by every backend hook.
///
/// Runs on the game's render thread: counter only, plus one log line on
/// the very first routed frame. See the module docs for why nothing more
/// happens here yet.
pub(crate) fn on_frame() {
    let prev = FRAME_COUNT.fetch_add(1, Ordering::SeqCst);
    if prev == 0 {
        crate::log_line("render: first present intercepted, hook is live");
    }
}

/// Install the render hook for the current platform.
///
/// On success the game's present path routes through [`on_frame`] and
/// [`frame_count`] starts advancing. On failure the game is untouched —
/// callers log the reason and keep running headless.
pub fn install() -> Result<RenderBackend, RenderError> {
    #[cfg(target_os = "linux")]
    {
        // Layer path first: if the Vulkan loader negotiated with us as an
        // implicit layer (manifest + launch options), present routing is
        // already in place — patching anything would only fight it.
        if crate::layer::is_active() {
            crate::log_line("render: layer-driven present routing active");
            INSTALLED.store(true, Ordering::SeqCst);
            return Ok(RenderBackend::Vulkan);
        }
        let hook = vk::VulkanHook;
        hook.install()?;
        INSTALLED.store(true, Ordering::SeqCst);
        Ok(hook.backend())
    }
    #[cfg(target_os = "windows")]
    {
        let hook = dxgi::DxgiHook;
        hook.install()?;
        INSTALLED.store(true, Ordering::SeqCst);
        Ok(hook.backend())
    }
    #[cfg(target_os = "macos")]
    {
        let hook = metal::MetalHook;
        hook.install()?;
        INSTALLED.store(true, Ordering::SeqCst);
        Ok(hook.backend())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(RenderError::Unsupported("unknown OS".into()))
    }
}
