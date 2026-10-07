//! Windows backend: intercept `IDXGISwapChain::Present`.
//!
//! # Why one hook covers DirectX 11 *and* 12
//!
//! Both renderers present through DXGI — the final frame always crosses
//! `IDXGISwapChain::Present` (vtable slot 8: IUnknown 0-2, IDXGIObject
//! 3-6, IDXGIDeviceSubObject 7, then Present). There is deliberately no
//! separate D3D11-vs-D3D12 path; hooking Present catches whichever
//! renderer the game uses.
//!
//! # Staged plan (this file: stage 1)
//!
//! Stage 1 (here): resolve `D3D11CreateDeviceAndSwapChain`, build a throwaway
//! device + swapchain, read `Present`'s address from vtable slot 8, tear
//! everything down, and log the address. This already proves end-to-end
//! reachability of the present path on a real Windows test run.
//!
//! Stage 2 (Phase 1b): patch the call path itself. The vtable belongs to
//! the *game's* swapchain object (which we cannot enumerate), so the
//! production mechanism will be an inline hook on the resolved `Present`
//! address (trampoline via `VirtualAlloc`, prologue analysis via the
//! `iced-x86` crate — no dependency added until a Windows machine can
//! test it). Until then [`DxgiHook::install`] reports `Unsupported` with
//! the located address attached, and the game runs untouched.
//!
//! # FFI note
//!
//! `windows-sys` 0.59 ships no Dxgi/Direct3D11 bindings, so the handful of
//! types below are hand-declared `#[repr(C)]` against the MSDN layouts
//! (stable since Windows 7; field order double-checked against the docs).
//! Only `LoadLibraryW` / `GetProcAddress` / `GetDesktopWindow` come from
//! `windows-sys`. This keeps the dependency surface unchanged.
//!
//! # Status
//!
//! Compiled for Windows only; on Linux/macOS this module does not exist.
//! The code below is syntax-checked on Linux but NOT type-checked there —
//! first real validation happens on a Windows build machine.

use super::{RenderBackend, RenderError};
use windows_sys::Win32::Foundation::{HRESULT, TRUE};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

// --- Hand-declared DirectX types (MSDN layouts, stable since Win7) ----------

// D3D_DRIVER_TYPE_HARDWARE
const D3D_DRIVER_TYPE_HARDWARE: i32 = 1;
// D3D11_SDK_VERSION
const D3D11_SDK_VERSION: u32 = 7;
// D3D_FEATURE_LEVEL_11_0
const D3D_FEATURE_LEVEL_11_0: u32 = 0xb000;
// DXGI_FORMAT_R8G8B8A8_UNORM
const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
// DXGI_USAGE_RENDER_TARGET_OUTPUT
const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
// DXGI_SWAP_EFFECT_DISCARD
const DXGI_SWAP_EFFECT_DISCARD: u32 = 0;

#[repr(C)]
struct DxgiRational {
    numerator: u32,
    denominator: u32,
}

#[repr(C)]
struct DxgiModeDesc {
    width: u32,
    height: u32,
    refresh_rate: DxgiRational,
    format: u32,
    scanline_ordering: u32,
    scaling: u32,
}

#[repr(C)]
struct DxgiSampleDesc {
    count: u32,
    quality: u32,
}

#[repr(C)]
struct DxgiSwapDesc {
    buffer_desc: DxgiModeDesc,
    sample_desc: DxgiSampleDesc,
    buffer_usage: u32,
    buffer_count: u32,
    output_window: isize,
    windowed: i32,
    swap_effect: u32,
    flags: u32,
}

/// `IDXGISwapChain::Present` lives at vtable slot 8 (see module docs).
const PRESENT_VTABLE_INDEX: usize = 8;

/// A resolved-but-not-yet-patched Present address (stage 1 proof).
static PRESENT_ADDRESS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Windows DXGI present hook (stage 1: locate + log; stage 2: patch).
pub struct DxgiHook;

impl super::RenderHook for DxgiHook {
    fn backend(&self) -> RenderBackend {
        RenderBackend::Dxgi
    }

    fn install(&self) -> Result<(), RenderError> {
        // Stage 1: prove we can reach Present's address.
        let addr = locate_present()?;
        PRESENT_ADDRESS.store(addr, std::sync::atomic::Ordering::SeqCst);
        crate::log_line(&format!(
            "render: IDXGISwapChain::Present located at {:#x} (patch pending, Phase 1b)",
            addr
        ));
        // Stage 2 not implemented yet: report honestly, touch nothing.
        Err(RenderError::Unsupported(
            "Present vtable/inline patch is Phase 1b (address located, see log)".into(),
        ))
    }

    fn uninstall(&self) -> Result<(), RenderError> {
        PRESENT_ADDRESS.store(0, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

/// Signature of `D3D11CreateDeviceAndSwapChain` (d3d11.dll).
#[allow(non_snake_case)]
type D3D11CreateDeviceAndSwapChainFn = unsafe extern "system" fn(
    pAdapter: *const std::ffi::c_void,
    DriverType: i32,
    Software: isize,
    Flags: u32,
    pFeatureLevels: *const u32,
    FeatureLevels: u32,
    SDKVersion: u32,
    pSwapChainDesc: *const DxgiSwapDesc,
    ppSwapChain: *mut *mut std::ffi::c_void,
    ppDevice: *mut *mut std::ffi::c_void,
    pFeatureLevel: *mut u32,
    ppImmediateContext: *mut *mut std::ffi::c_void,
) -> HRESULT;

/// `IUnknown::Release` as found in any COM vtable slot 1.
type ReleaseFn = unsafe extern "system" fn(*mut std::ffi::c_void) -> u32;

/// Build a throwaway D3D11 device + swapchain, read `Present` from
/// vtable slot 8, release everything. Returns the function address.
///
/// Uses the desktop window with a tiny windowed swapchain: created and
/// destroyed within milliseconds, no visible artifact.
fn locate_present() -> Result<usize, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    // SAFETY: straightforward Win32 calls; all out-pointers are valid
    // stack slots living for the whole call; strings are NUL-terminated
    // UTF-16 built just below.
    unsafe {
        let lib_name: Vec<u16> = "d3d11.dll\0".encode_utf16().collect();
        let d3d11 = LoadLibraryW(lib_name.as_ptr());
        if d3d11 == 0 {
            return Err(fail("LoadLibraryW(d3d11.dll) failed".into()));
        }
        let proc_name = c"D3D11CreateDeviceAndSwapChain";
        let create_raw = GetProcAddress(d3d11, proc_name.as_ptr() as *const u8);
        let create: D3D11CreateDeviceAndSwapChainFn = match create_raw {
            Some(f) => std::mem::transmute(f),
            None => {
                return Err(fail(
                    "D3D11CreateDeviceAndSwapChain not exported".into(),
                ))
            }
        };

        let desc = DxgiSwapDesc {
            buffer_desc: DxgiModeDesc {
                width: 8,
                height: 8,
                refresh_rate: DxgiRational {
                    numerator: 60,
                    denominator: 1,
                },
                format: DXGI_FORMAT_R8G8B8A8_UNORM,
                scanline_ordering: 0,
                scaling: 0,
            },
            sample_desc: DxgiSampleDesc {
                count: 1,
                quality: 0,
            },
            buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            buffer_count: 1,
            output_window: GetDesktopWindow(),
            windowed: TRUE,
            swap_effect: DXGI_SWAP_EFFECT_DISCARD,
            flags: 0,
        };
        let feature_levels = [D3D_FEATURE_LEVEL_11_0];
        let mut swapchain: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut device: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut feature_level: u32 = 0;
        let mut context: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr = create(
            std::ptr::null(),
            D3D_DRIVER_TYPE_HARDWARE,
            0,
            0,
            feature_levels.as_ptr(),
            1,
            D3D11_SDK_VERSION,
            &desc,
            &mut swapchain,
            &mut device,
            &mut feature_level,
            &mut context,
        );
        if hr < 0 || swapchain.is_null() {
            return Err(fail(format!(
                "dummy swapchain creation failed (HRESULT {:#x})",
                hr as u32
            )));
        }
        // vtable[8] == Present (see module docs for the slot math).
        let vtbl = *(swapchain as *const *const usize);
        let present = *vtbl.add(PRESENT_VTABLE_INDEX);
        // Release everything through IUnknown::Release (slot 1).
        let release: ReleaseFn = std::mem::transmute(*vtbl.add(1));
        release(swapchain);
        if !device.is_null() {
            let dev_vtbl = *(device as *const *const usize);
            let dev_release: ReleaseFn = std::mem::transmute(*dev_vtbl.add(1));
            dev_release(device);
        }
        if present == 0 {
            return Err(fail("Present vtable slot is null".into()));
        }
        Ok(present)
    }
}
