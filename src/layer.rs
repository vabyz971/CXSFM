//! Implicit Vulkan layer: the loader-sanctioned present hook (Linux).
//!
//! # Why a layer instead of patching
//!
//! GOT patching needs PLT slots (absent: static volk dispatch) and the
//! dispatch-table scan needs loader internals at guessable addresses.
//! A Vulkan *layer* sidesteps both: the loader itself discovers us via a
//! manifest, negotiates the interface, and routes `vkGetDeviceProcAddr`
//! queries through our wrappers. No code patching, no memory scanning, no
//! version-skewed symbol tables — this is exactly how the Steam overlay
//! and MangoHud intercept frames.
//!
//! # Activation (explicit opt-in, per game)
//!
//! 1. `tools/cxsfm-setup.sh` stages the layer `.so` + manifest JSON into
//!    `~/.cache/cxsfm/` (container-visible, game dir untouched).
//! 2. Steam launch options add:
//!    `VK_ADD_LAYER_PATH=$HOME/.cache/cxsfm VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay %command%`
//!
//! # What we intercept (minimal surface)
//!
//! * `vkCreateInstance` — capture the next-layer instance proc chain.
//! * `vkCreateDevice` — capture the per-device proc chain.
//! * `vkGetDeviceQueue[V2]` — record `queue -> device` (present only
//!   carries a queue; the real present needs its device).
//! * `vkQueuePresentKHR` — [`super::render::on_frame`] then forward.
//!
//! Everything else forwards untouched. Unknown queues fall back to the
//! last-known device (logged once); in practice every queue is created
//! after the layer loads, so the map is complete.
//!
//! # Threading
//!
//! The loader may call wrappers from any thread: one `Mutex`-guarded
//! state map, function pointers copied out before invoking (never hold
//! the lock across a real call). `present` itself only locks to resolve.

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

// --- Vulkan ABI stubs (opaque handles + codes we compare) ---------------------

type VkResult = i32;
const VK_SUCCESS: VkResult = 0;

/// `LAYER_NEGOTIATE_INTERFACE_STRUCT` per `vulkan/vk_layer.h`
/// (`VkNegotiateLayerStructType`: `UNINITIALIZED = 0`, `INTERFACE_STRUCT
/// = 1`). This was `2` — matching nothing the loader ever sends — so
/// every negotiation failed the `s_type` check and the loader skipped
/// us after dlopen (ctor ran, framework booted, but no present
/// routing). One-character class of bug, total effect.
const LAYER_NEGOTIATE_INTERFACE_STRUCT: u32 = 1;
const CURRENT_LOADER_LAYER_INTERFACE_VERSION: u32 = 2;
// VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO / _DEVICE_CREATE_INFO.
// Core-range values (NOT extension range!): 47 and 48 per
// `vulkan_core.h`. The old 1000016000/1000016001 matched nothing the
// loader ever sends — every chain lookup walked straight past our own
// link node (visible in hindsight: the loader's four prepended nodes
// all share sType 47, distinguished by `function` 3/2/1/0).
const LOADER_INSTANCE_CREATE_INFO: u32 = 47;
const LOADER_DEVICE_CREATE_INFO: u32 = 48;
// VK_LAYER_LINK_INFO
const LAYER_LINK_INFO: u32 = 0;

/// Loader proc-address signatures (public: they cross into
/// `render::overlay` for ash loading).
pub type GetInstanceProcAddrFn =
    unsafe extern "C" fn(usize, *const libc::c_char) -> *const std::ffi::c_void;
pub type GetDeviceProcAddrFn =
    unsafe extern "C" fn(usize, *const libc::c_char) -> *const std::ffi::c_void;
type CreateInstanceFn = unsafe extern "C" fn(
    *const std::ffi::c_void,
    *const std::ffi::c_void,
    *mut usize,
) -> VkResult;
type CreateDeviceFn = unsafe extern "C" fn(
    usize,
    *const std::ffi::c_void,
    *const std::ffi::c_void,
    *mut usize,
) -> VkResult;
type GetDeviceQueueFn =
    unsafe extern "C" fn(usize, u32, u32, *mut usize);
type GetDeviceQueue2Fn =
    unsafe extern "C" fn(usize, *const std::ffi::c_void, *mut usize);
type PresentFn = unsafe extern "C" fn(usize, *const std::ffi::c_void) -> VkResult;
type CreateSwapchainFn = unsafe extern "C" fn(
    usize,
    *const std::ffi::c_void,
    *const std::ffi::c_void,
    *mut usize,
) -> VkResult;
type DestroySwapchainFn =
    unsafe extern "C" fn(usize, usize, *const std::ffi::c_void);
type DestroyInstanceFn = unsafe extern "C" fn(usize, *const std::ffi::c_void);
type DestroyDeviceFn = unsafe extern "C" fn(usize, *const std::ffi::c_void);
type GetSwapchainImagesFn = unsafe extern "C" fn(
    usize,
    usize,
    *mut u32,
    *mut usize,
) -> VkResult;

/// `VkNegotiateLayerInterface` (loader ↔ layer handshake struct).
///
/// Public because it appears in the exported
/// [`vkNegotiateLoaderLayerInterfaceVersion`] signature; constructible
/// only by the loader in practice.
#[repr(C)]
pub struct NegotiateInterface {
    s_type: u32,
    p_next: *const std::ffi::c_void,
    loader_layer_interface_version: u32,
    pfn_get_instance_proc_addr: Option<GetInstanceProcAddrFn>,
    pfn_get_device_proc_addr: Option<GetDeviceProcAddrFn>,
    pfn_get_physical_device_proc_addr: Option<GetInstanceProcAddrFn>,
}

// --- Layer state -----------------------------------------------------------------

struct LayerState {
    /// Next layer's instance proc chain (captured at CreateInstance).
    next_instance_proc: Option<GetInstanceProcAddrFn>,
    /// LIVE VkInstances (inserted at every successful CreateInstance,
    /// removed at DestroyInstance). ash loading resolves through one of
    /// these — NEVER a stored-and-forgotten handle: the game creates a
    /// probe instance and destroys it, and resolving through the corpse
    /// segfaults inside the loader (seen live). Empty set = no overlay.
    live_instances: HashSet<usize>,
    /// Per-device next proc chains (captured at CreateDevice).
    devices: HashMap<usize, GetDeviceProcAddrFn>,
    /// device -> physical device (captured at CreateDevice; the overlay
    /// needs the physical device for memory properties).
    dev_phys: HashMap<usize, usize>,
    /// queue -> (device, queue family) (captured at GetDeviceQueue[V2];
    /// the family places our overlay command pool; None when the V2
    /// info was null — overlay skips such queues instead of guessing).
    queues: HashMap<usize, (usize, Option<u32>)>,
    /// Last device seen (fallback for unrecorded queues).
    last_device: Option<usize>,
    /// Swapchain handle -> captured creation state (format, extent,
    /// images) for the overlay. Filled at CreateSwapchain, dropped at
    /// DestroySwapchain.
    swaps: HashMap<usize, SwapchainInfo>,
    /// Whether each object kind was ever observed. Combined with the
    /// live maps below this detects teardown: objects that existed
    /// and are all gone mean the game is exiting (as opposed to not
    /// started yet). See [`shutting_down`].
    had_swaps: bool,
    had_devices: bool,
    had_instances: bool,
    /// Whether the unknown-queue fallback already logged once.
    fallback_logged: bool,
}

/// Swapchain creation state captured for the overlay renderer.
#[derive(Debug, Clone)]
pub struct SwapchainInfo {
    /// Owning device.
    pub device: usize,
    /// `VkFormat` as raw u32 (render pass + srgb detection).
    pub format: u32,
    /// Swapchain extent (overlay render area).
    pub extent: (u32, u32),
    /// Swapchain images (framebuffer targets).
    pub images: Vec<usize>,
}

impl LayerState {
    fn new() -> Self {
        Self {
            next_instance_proc: None,
            live_instances: HashSet::new(),
            devices: HashMap::new(),
            dev_phys: HashMap::new(),
            queues: HashMap::new(),
            last_device: None,
            swaps: HashMap::new(),
            had_swaps: false,
            had_devices: false,
            had_instances: false,
            fallback_logged: false,
        }
    }
}

static STATE: std::sync::LazyLock<Mutex<LayerState>> =
    std::sync::LazyLock::new(|| Mutex::new(LayerState::new()));
/// Set by [`vkNegotiateLoaderLayerInterfaceVersion`]: the loader loaded
/// us as a layer (as opposed to `dlopen` injection). [`crate::render`]
/// prefers this routing and skips GOT/inline patching entirely.
static LAYER_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether the game is tearing down.
///
/// True when object kinds that existed are ALL gone: swapchains AND
/// devices AND instances empty after having been observed. During play
/// at least the rendering device + instance stay alive, so this only
/// fires on the way out (or before anything started — callers combine
/// with init state as needed). While true the tick thread stops all
/// IL2CPP use: Unity kills its scripting domain during teardown and a
/// single call into the corpse segfaults (caught live via Player.log).
/// SDL/X11/log paths are unaffected (no Unity contact).
pub fn shutting_down() -> bool {
    let st = STATE.lock().unwrap();
    (st.had_swaps && st.swaps.is_empty())
        || (st.had_devices && st.devices.is_empty())
        || (st.had_instances && st.live_instances.is_empty())
}

/// True once the Vulkan loader negotiated with us as a layer.
#[inline]
pub fn is_active() -> bool {
    LAYER_ACTIVE.load(Ordering::SeqCst)
}

/// Next instance proc chain for overlay-side queries (ash loading).
pub fn instance_chain() -> Option<GetInstanceProcAddrFn> {
    STATE.lock().unwrap().next_instance_proc
}

/// First live VkInstance, if any (ash-Instance loading resolves
/// through it; function pointers are process-global).
pub fn first_instance() -> Option<usize> {
    STATE.lock().unwrap().live_instances.iter().next().copied()
}

/// Next device proc chain for one device (overlay-side queries).
pub fn device_chain(dev: usize) -> Option<GetDeviceProcAddrFn> {
    STATE.lock().unwrap().devices.get(&dev).copied()
}

/// Physical device behind a logical one (captured at CreateDevice).
pub fn device_physical(dev: usize) -> Option<usize> {
    STATE.lock().unwrap().dev_phys.get(&dev).copied()
}

/// Captured creation state for one swapchain.
pub fn swapchain_info(swap: usize) -> Option<SwapchainInfo> {
    STATE.lock().unwrap().swaps.get(&swap).cloned()
}

/// Drop overlay state for a destroyed swapchain.
pub fn remove_swapchain(swap: usize) {
    STATE.lock().unwrap().swaps.remove(&swap);
    crate::render::overlay::drop_swapchain(swap);
}

// --- Small readers -----------------------------------------------------------------

/// Bounded NUL-terminated string from game/loader memory, or `None`.
fn read_name(ptr: *const libc::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: loader-passed names are valid C strings; still bounded to
    // 128 bytes so garbage can never overrun us.
    unsafe {
        let mut len = 0usize;
        while len < 128 {
            if *ptr.add(len) == 0 {
                break;
            }
            len += 1;
        }
        if len == 128 {
            return None;
        }
        CStr::from_ptr(ptr)
            .to_str()
            .ok()
            .map(|s| s.to_string())
    }
}

/// Read a chained loader-info node: `(function, pLayerInfo)` for the
/// first LINK node matching `want_type` in the `pNext` chain (max 8 hops).
/// Layouts (both instance and device variants):
/// `{ sType u32, pNext ptr, function u32, pLayerInfo ptr }`.
///
/// NOTE: the loader inserts SEVERAL nodes with the same `sType` but
/// different `function` (e.g. `VK_LOADER_DATA_CALLBACK` for
/// SetData callbacks alongside our `VK_LAYER_LINK_INFO`). Non-link
/// nodes must be SKIPPED, not terminal: stopping at the first
/// same-type node fails every handshake on loaders that order the
/// data-callback node first (seen live: vulkaninfo + loader 1.4 never
/// got past our lookup).
fn find_link_info(
    p_next: *const std::ffi::c_void,
    want_type: u32,
) -> Option<*const std::ffi::c_void> {
    // SAFETY: loader-built chains; every hop null-checked and bounded.
    unsafe {
        let mut node = p_next as *const u8;
        for _ in 0..8 {
            if node.is_null() {
                return None;
            }
            let s_type = *(node as *const u32);
            let next = *(node.add(8) as *const *const std::ffi::c_void);
            if s_type == want_type {
                let function = *(node.add(16) as *const u32);
                if function == LAYER_LINK_INFO {
                    return Some(*(node.add(24) as *const *const std::ffi::c_void));
                }
                // Same struct, different role (data callback, ...) —
                // keep walking.
            }
            node = next as *const u8;
        }
        None
    }
}

/// Advance a consumed link node: `pLayerInfo = pLayerInfo->pNext`.
///
/// MANDATORY before forwarding `vkCreateInstance`/`vkCreateDevice` down
/// the chain. The loader threads one link node per layer; each layer
/// must pop its own node so the next callee (layer or terminator) sees
/// the REMAINDER of the chain. Forwarding the create-info untouched
/// leaves our already-consumed node in place → the terminator walks a
/// stale chain and fails the whole call (`ERROR_INITIALIZATION_FAILED`
/// with zero further explanation — seen live against vulkaninfo).
/// Both link flavors start with `pNext` at +0, so one function serves.
fn advance_link(layer_info: *const std::ffi::c_void) {
    // SAFETY: loader-built link node; the loader REQUIRES layers to
    // mutate exactly this pointer. Null-checked by callers.
    unsafe {
        let base = layer_info as *const u8 as *mut usize;
        let next = *base;
        *base = next;
    }
}

/// `pLayerInfo -> (next_proc_addr_fn)` for both link flavors:
/// instance link `{ pNext, pfnNextGetInstanceProcAddr, ... }`,
/// device link `{ pNext, pfnNextGetInstanceProcAddr, pfnNextGetDeviceProcAddr }`.
fn next_procs(
    layer_info: *const std::ffi::c_void,
    device_link: bool,
) -> (Option<GetInstanceProcAddrFn>, Option<GetDeviceProcAddrFn>) {
    // SAFETY: loader-built link node; offsets per the ABI documented above.
    // Every raw pointer is null-checked before transmuting: a null entry
    // means a broken chain, never a callable function.
    unsafe {
        if layer_info.is_null() {
            return (None, None);
        }
        let base = layer_info as *const u8;
        let inst_raw = *(base.add(8) as *const usize);
        let inst = if inst_raw == 0 {
            None
        } else {
            Some(std::mem::transmute::<usize, GetInstanceProcAddrFn>(inst_raw))
        };
        let dev = if device_link {
            let raw = *(base.add(16) as *const usize);
            if raw == 0 {
                None
            } else {
                Some(std::mem::transmute::<usize, GetDeviceProcAddrFn>(raw))
            }
        } else {
            None
        };
        (inst, dev)
    }
}

// --- Wrapped entry points -----------------------------------------------------------

unsafe extern "C" fn layer_create_instance(
    p_create_info: *const std::ffi::c_void,
    p_allocator: *const std::ffi::c_void,
    p_instance: *mut usize,
) -> VkResult {
    // pCreateInfo = { sType, pNext, ... }: chain head at +8.
    // Diagnostic: also log the create-info's own sType (must be 1 =
    // VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO) to prove we are really
    // looking at a VkInstanceCreateInfo and not a shifted struct.
    let (create_stype, chain) = if p_create_info.is_null() {
        (0u32, None)
    } else {
        // SAFETY: loader passes a valid create-info or null.
        unsafe {
            (
                *(p_create_info as *const u32),
                Some(*(p_create_info.add(8) as *const *const std::ffi::c_void)),
            )
        }
    };
    crate::log_line(&format!(
        "render/layer: CreateInstance entry (create_sType={create_stype}, info={p_create_info:p})"
    ));
    let link = chain.and_then(|c| find_link_info(c, LOADER_INSTANCE_CREATE_INFO));
    let Some(li) = link else {
        // Diagnostic dump: what did the loader actually pass? Walk the
        // chain and log every (sType, function) so a layout mismatch
        // shows up as data instead of a bare failure.
        if let Some(c) = chain {
            // SAFETY: same bounded, null-checked walk as `find_link_info`.
            unsafe {
                let mut node = c as *const u8;
                for hop in 0..8 {
                    if node.is_null() {
                        break;
                    }
                    let st = *(node as *const u32);
                    let fx = *(node.add(16) as *const u32);
                    let nx = *(node.add(8) as *const *const std::ffi::c_void);
                    crate::log_line(&format!(
                        "render/layer: chain hop {hop}: sType {st} function {fx}"
                    ));
                    node = nx as *const u8;
                }
            }
        } else {
            crate::log_line("render/layer: CreateInstance with null pNext chain");
        }
        crate::log_line("render/layer: CreateInstance without link info, failing");
        return -3; // VK_ERROR_INITIALIZATION_FAILED
    };
    let (next_inst, _) = next_procs(li, false);
    // Pop our node BEFORE forwarding (see `advance_link`).
    advance_link(li);
    let next = match next_inst {
        Some(f) => f,
        None => {
            crate::log_line("render/layer: CreateInstance with null next proc, failing");
            return -3;
        }
    };
    let real: CreateInstanceFn = unsafe {
        let p = next(0, c"vkCreateInstance".as_ptr());
        if p.is_null() {
            crate::log_line("render/layer: next vkCreateInstance lookup failed");
            return -3;
        }
        std::mem::transmute(p)
    };
    // SAFETY: forwarding the app's exact arguments to the real function.
    let r = unsafe { real(p_create_info, p_allocator, p_instance) };
    if r == VK_SUCCESS {
        let mut st = STATE.lock().unwrap();
        st.next_instance_proc = Some(next);
        if !p_instance.is_null() {
            // SAFETY: out-pointer written by the successful call above.
            // EVERY created instance is tracked (not just the first):
            // probe instances die young and must drop out of the set
            // (see DestroyInstance) instead of lingering as corpses.
            let inst = unsafe { *p_instance };
            st.live_instances.insert(inst);
            st.had_instances = true;
        }
        crate::log_line("render/layer: instance created, chain captured");
    } else {
        crate::log_line(&format!("render/layer: down-chain CreateInstance failed ({r})"));
    }
    r
}

unsafe extern "C" fn layer_create_device(
    physical: usize,
    p_create_info: *const std::ffi::c_void,
    p_allocator: *const std::ffi::c_void,
    p_device: *mut usize,
) -> VkResult {
    // Entry log (every device creation, success or not — a second
    // device bypassing us would otherwise be invisible).
    crate::log_line("render/layer: CreateDevice entry");
    let chain = if p_create_info.is_null() {
        None
    } else {
        // SAFETY: loader passes a valid create-info or null.
        unsafe { Some(*(p_create_info.add(8) as *const *const std::ffi::c_void)) }
    };
    let link = chain.and_then(|c| find_link_info(c, LOADER_DEVICE_CREATE_INFO));
    let Some(li) = link else {
        crate::log_line("render/layer: CreateDevice without link info, failing");
        return -3;
    };
    // NOTE: the CreateDevice lookup goes through pfnNextGetInstance-
    // ProcAddr (the FIRST tuple element, link+8) — never the device
    // slot. A prior revision used the second element here, which meant
    // every lookup ran against the device terminator (NULL for all
    // global queries) while the working instance chain sat one slot
    // away. The loader was innocent; the bug was ours.
    //
    // Conversely, the per-device map MUST store the device slot
    // (SECOND element, link+16): all later device-function forwarding
    // (queues, present, everything else) goes through
    // pfnNextGetDeviceProcAddr. Storing the instance chain there turns
    // every device call into a NULL dispatch (crashed vulkaninfo on
    // the first device function).
    let (next_inst, next_dev) = next_procs(li, true);
    // Pop our node BEFORE forwarding (see `advance_link`).
    advance_link(li);
    let next = match next_inst {
        Some(f) => f,
        None => {
            crate::log_line("render/layer: CreateDevice with null next proc, failing");
            return -3;
        }
    };
    let real: CreateDeviceFn = unsafe {
        let p = next(physical, c"vkCreateDevice".as_ptr());
        if p.is_null() {
            crate::log_line("render/layer: next vkCreateDevice lookup failed");
            return -3;
        }
        std::mem::transmute(p)
    };
    // SAFETY: forwarding the app's exact arguments.
    let r = unsafe { real(physical, p_create_info, p_allocator, p_device) };
    if r == VK_SUCCESS && !p_device.is_null() {
        // SAFETY: out-pointer written by the successful call above.
        let dev = unsafe { *p_device };
        match next_dev {
            Some(d) => {
                let mut st = STATE.lock().unwrap();
                st.devices.insert(dev, d);
                st.dev_phys.insert(dev, physical);
                st.last_device = Some(dev);
                st.had_devices = true;
                crate::log_line("render/layer: device created, chain captured");
            }
            None => {
                crate::log_line(
                    "render/layer: device created WITHOUT device chain (forwarding unavailable)",
                );
            }
        }
    } else if r != VK_SUCCESS {
        crate::log_line(&format!("render/layer: down-chain CreateDevice failed ({r})"));
    }
    r
}

unsafe extern "C" fn layer_get_device_queue(
    device: usize,
    family: u32,
    index: u32,
    p_queue: *mut usize,
) {
    let next = STATE.lock().unwrap().devices.get(&device).copied();
    if let Some(n) = next {
        // SAFETY: forwarding the app's exact arguments.
        unsafe {
            let real: GetDeviceQueueFn = std::mem::transmute(n(
                device,
                c"vkGetDeviceQueue".as_ptr(),
            ));
            real(device, family, index, p_queue);
        }
        if !p_queue.is_null() {
            // SAFETY: out-pointer written by the forwarded call above.
            let q = unsafe { *p_queue };
            let mut st = STATE.lock().unwrap();
            st.queues.insert(q, (device, Some(family)));
        }
    }
}

unsafe extern "C" fn layer_get_device_queue2(
    device: usize,
    p_info: *const std::ffi::c_void,
    p_queue: *mut usize,
) {
    let next = STATE.lock().unwrap().devices.get(&device).copied();
    if let Some(n) = next {
        // SAFETY: forwarding the app's exact arguments.
        unsafe {
            let real: GetDeviceQueue2Fn = std::mem::transmute(n(
                device,
                c"vkGetDeviceQueue2".as_ptr(),
            ));
            real(device, p_info, p_queue);
        }
        if !p_queue.is_null() {
            // SAFETY: out-pointer written by the forwarded call above.
            let q = unsafe { *p_queue };
            // VkDeviceQueueInfo2 = { sType@0, pNext@8, flags@16,
            // queueFamilyIndex@20, queueIndex@24 }: family at +20, or
            // unknown when info itself is null (overlay skips those).
            let family = if p_info.is_null() {
                None
            } else {
                // SAFETY: loader-built struct or null (checked).
                Some(unsafe { *(p_info.add(20) as *const u32) })
            };
            let mut st = STATE.lock().unwrap();
            st.queues.insert(q, (device, family));
        }
    }
}

/// Instance destruction: forward, then drop the handle from the live
/// set. A destroyed instance MUST NOT stay tracked: resolving ash
/// tables (or anything else) through the corpse segfaults inside the
/// loader, and the game demonstrably creates + destroys probe
/// instances around startup.
unsafe extern "C" fn layer_destroy_instance(
    instance: usize,
    p_allocator: *const std::ffi::c_void,
) {
    let next = STATE.lock().unwrap().next_instance_proc;
    if let Some(n) = next {
        // SAFETY: forwarding the app's exact arguments.
        unsafe {
            let real: DestroyInstanceFn = std::mem::transmute(n(
                instance,
                c"vkDestroyInstance".as_ptr(),
            ));
            real(instance, p_allocator);
        }
    }
    STATE.lock().unwrap().live_instances.remove(&instance);
    crate::log_line(&format!("render/layer: instance {instance:#x} destroyed"));
}

/// Device destruction: resolve the real destroy through the device's
/// own chain (device still live), drop overlay state (needs a live
/// device), drop our bookkeeping, then forward the real destroy.
/// Stale handles must never survive: driver handle reuse would
/// otherwise route a NEW device's calls through the OLD device's
/// chain.
unsafe extern "C" fn layer_destroy_device(
    device: usize,
    p_allocator: *const std::ffi::c_void,
) {
    let real: Option<DestroyDeviceFn> = STATE
        .lock()
        .unwrap()
        .devices
        .get(&device)
        .copied()
        .and_then(|n| unsafe {
            let p = n(device, c"vkDestroyDevice".as_ptr());
            if p.is_null() {
                None
            } else {
                Some(std::mem::transmute(p))
            }
        });
    // GLOBALS lock only (released on return — never nested with STATE).
    crate::render::overlay::drop_device(device);
    {
        let mut st = STATE.lock().unwrap();
        st.devices.remove(&device);
        st.dev_phys.remove(&device);
        st.queues.retain(|_, (d, _)| *d != device);
        if st.last_device == Some(device) {
            st.last_device = None;
        }
    }
    if let Some(destroy) = real {
        // SAFETY: forwarding the app's exact arguments.
        unsafe {
            destroy(device, p_allocator);
        }
    } else {
        crate::log_line(
            "render/layer: DestroyDevice for unknown device (nothing forwarded)",
        );
    }
    crate::log_line(&format!("render/layer: device {device:#x} destroyed"));
}

/// Swapchain creation: capture format/extent/images for the overlay,
/// then forward. Like CreateDevice, this runs inside the loader's
/// device chain (link node present); unlike Present it runs rarely.
///
/// `VkSwapchainCreateInfoKHR` reads (verified against headers AND
/// live values — an earlier revision read @28/@36/@40, producing the
/// tell-tale `extent=50x0` which was really `imageFormat=50`
/// (B8G8R8A8_SRGB) + `imageColorSpace=0`): `imageFormat i32@36`,
/// `imageExtent {w@44, h@48}`. Surface (u64) sits @24 after flags@16.
unsafe extern "C" fn layer_create_swapchain(
    device: usize,
    p_create_info: *const std::ffi::c_void,
    p_allocator: *const std::ffi::c_void,
    p_swapchain: *mut usize,
) -> VkResult {
    let next = STATE.lock().unwrap().devices.get(&device).copied();
    let next = match next {
        Some(n) => n,
        None => {
            crate::log_line("render/layer: CreateSwapchain for unknown device, failing");
            return -3;
        }
    };
    let real: CreateSwapchainFn = unsafe {
        let p = next(device, c"vkCreateSwapchainKHR".as_ptr());
        if p.is_null() {
            crate::log_line("render/layer: next vkCreateSwapchainKHR lookup failed");
            return -3;
        }
        std::mem::transmute(p)
    };
    // SAFETY: forwarding the app's exact arguments.
    let r = unsafe { real(device, p_create_info, p_allocator, p_swapchain) };
    if r != VK_SUCCESS || p_swapchain.is_null() {
        if r != VK_SUCCESS {
            crate::log_line(&format!("render/layer: down-chain CreateSwapchain failed ({r})"));
        }
        return r;
    }
    // SAFETY: out-pointer written by the successful call above.
    let swap = unsafe { *p_swapchain };
    // Read creation parameters (null-safe; missing info = skip capture,
    // overlay degrades to untouched presents for this swapchain).
    let (format, extent) = if p_create_info.is_null() {
        (0, (0, 0))
    } else {
        // SAFETY: loader-passed create-info, valid for this call.
        unsafe {
            (
                *(p_create_info.add(36) as *const u32),
                (
                    *(p_create_info.add(44) as *const u32),
                    *(p_create_info.add(48) as *const u32),
                ),
            )
        }
    };
    // Fetch the swapchain images now (two-call idiom through our chain).
    let images: Vec<usize> = unsafe {
        let get_images: GetSwapchainImagesFn = std::mem::transmute(next(
            device,
            c"vkGetSwapchainImagesKHR".as_ptr(),
        ));
        let mut count: u32 = 0;
        if get_images(device, swap, &mut count as *mut u32, std::ptr::null_mut()) != VK_SUCCESS
            || count == 0
            || count > 16
        {
            Vec::new()
        } else {
            let mut imgs = vec![0usize; count as usize];
            if get_images(device, swap, &mut count as *mut u32, imgs.as_mut_ptr())
                != VK_SUCCESS
            {
                Vec::new()
            } else {
                imgs.truncate(count as usize);
                imgs
            }
        }
    };
    if format == 0 || extent.0 == 0 || images.is_empty() {
        crate::log_line(&format!(
            "render/layer: swapchain {swap:#x} created but uncapturable (format={format} extent={}x{} images={})",
            extent.0,
            extent.1,
            images.len(),
        ));
        return r;
    }
    STATE.lock().unwrap().swaps.insert(
        swap,
        SwapchainInfo {
            device,
            format,
            extent,
            images,
        },
    );
    STATE.lock().unwrap().had_swaps = true;
    crate::log_line(&format!(
        "render/layer: swapchain {swap:#x} captured (format={format} extent={}x{})",
        extent.0, extent.1,
    ));
    r
}

/// Swapchain destruction: forward, then drop overlay state.
unsafe extern "C" fn layer_destroy_swapchain(
    device: usize,
    swapchain: usize,
    p_allocator: *const std::ffi::c_void,
) {
    let next = STATE.lock().unwrap().devices.get(&device).copied();
    if let Some(n) = next {
        // SAFETY: forwarding the app's exact arguments.
        unsafe {
            let real: DestroySwapchainFn = std::mem::transmute(n(
                device,
                c"vkDestroySwapchainKHR".as_ptr(),
            ));
            real(device, swapchain, p_allocator);
        }
    }
    remove_swapchain(swapchain);
    crate::log_line(&format!("render/layer: swapchain {swapchain:#x} destroyed"));
}

/// Present entry: overlay first, real present second.
///
/// Looks up `queue -> device -> next GetDeviceProcAddr`, resolves the
/// real present once per device (cached), runs [`super::render::on_frame`],
/// draws the overlay when any mod is enabled, then forwards.
///
/// Overlay policy (correctness over coverage): single-swapchain presents
/// only (`swapchainCount != 1` forwards untouched — VR/multi-view stays
/// exactly as the game made it). The overlay submit waits the present's
/// own semaphores and signals ours; the forwarded present then waits
/// ONLY on ours (binary semaphores allow a single wait each — consuming
/// the game's waits twice would be undefined). Any overlay failure at
/// any step falls back to the ORIGINAL present call: the game never
/// breaks because of the UI.
///
/// `VkPresentInfoKHR` layout (LP64, pointers 8-aligned): `{ sType@0,
/// pNext@8, waitCount@16, pWaits@24, swapCount@32, pSwaps@40,
/// pIndices@48, pResults@56 }`.
unsafe extern "C" fn layer_present(
    queue: usize,
    info: *const std::ffi::c_void,
) -> VkResult {
    // Per-device cache lives alongside STATE via a second map.
    static PRESENTS: std::sync::LazyLock<Mutex<HashMap<usize, PresentFn>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    // TEMPORARY routing proof: fires once on the very first present
    // reaching us (distinguishes "presents bypass us" from downstream
    // overlay issues). Remove after first in-game confirmation.
    static ENTERED: AtomicBool = AtomicBool::new(false);
    if !ENTERED.swap(true, Ordering::SeqCst) {
        crate::log_line(&format!(
            "render/layer: present entered (queue={queue:#x}, info={info:p})"
        ));
    }
    // Resolve device + family (family needed for the overlay pool).
    let resolved: Option<(usize, Option<u32>)> = {
        let st = STATE.lock().unwrap();
        st.queues
            .get(&queue)
            .copied()
            .or_else(|| st.last_device.map(|d| (d, None)))
    };
    let (dev, family) = match resolved {
        Some(v) => v,
        None => {
            let mut st = STATE.lock().unwrap();
            if !st.fallback_logged {
                st.fallback_logged = true;
                crate::log_line(
                    "render/layer: present from unknown queue, no device yet",
                );
            }
            return VK_SUCCESS;
        }
    };
    // NOTE: the unknown-queue fallback log lives here (outside the
    // borrow above) to keep lock scopes tiny.
    let real: Option<PresentFn> = {
        let st = STATE.lock().unwrap();
        if let Some(f) = PRESENTS.lock().unwrap().get(&dev) {
            Some(*f)
        } else if let Some(next) = st.devices.get(&dev).copied() {
            // SAFETY: validated chain + static name.
            let p = unsafe { next(dev, c"vkQueuePresentKHR".as_ptr()) };
            if p.is_null() {
                None
            } else {
                let f: PresentFn = unsafe { std::mem::transmute(p) };
                PRESENTS.lock().unwrap().insert(dev, f);
                Some(f)
            }
        } else {
            None
        }
    };
    let real = match real {
        Some(f) => f,
        None => return VK_SUCCESS,
    };
    crate::render::on_frame();
    // Overlay gate: draw when mod effects run OR framework windows are
    // shown. Gating on mods alone blanks the UI exactly when the user
    // opens an empty menu (no mods armed yet): status/manager windows
    // need ui_visible, not mod state. Skip only when truly nothing
    // could draw (idle framework costs zero GPU work).
    if crate::mod_api::get_mod_manager().enabled_count() == 0
        && !crate::hotkey::ui_visible()
    {
        // SAFETY: resolved from the live loader chain for this device.
        return unsafe { real(queue, info) };
    }
    // Parse the present struct (null-safe; anything odd → original).
    // Offsets per LP64 C alignment (pointers are 8-aligned — an
    // earlier revision forgot the padding and read garbage as
    // swapchainCount, parking every present as "multi").
    struct PresentView {
        waits: Vec<usize>,
        swapchain: usize,
        image: u32,
        multi: bool,
        results: *mut VkResult,
    }
    let view: Option<PresentView> = if info.is_null() {
        None
    } else {
        // SAFETY: loader-built present struct, valid for this call.
        unsafe {
            let swap_count = *(info.add(32) as *const u32);
            if swap_count != 1 {
                Some(PresentView {
                    waits: Vec::new(),
                    swapchain: 0,
                    image: 0,
                    multi: true,
                    results: std::ptr::null_mut(),
                })
            } else {
                let wait_count = *(info.add(16) as *const u32) as usize;
                let p_waits = *(info.add(24) as *const *const usize);
                let p_swaps = *(info.add(40) as *const *const usize);
                let p_idx = *(info.add(48) as *const *const u32);
                let p_res = *(info.add(56) as *const *mut VkResult);
                if p_swaps.is_null() || p_idx.is_null() {
                    None
                } else {
                    let waits = if wait_count == 0 || p_waits.is_null() {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(p_waits, wait_count).to_vec()
                    };
                    Some(PresentView {
                        waits,
                        swapchain: *p_swaps,
                        image: *p_idx,
                        multi: false,
                        // pResults is already an array pointer
                        // (`VkResult*`), stored as-is (count matches
                        // swapchainCount, which is 1 here).
                        results: p_res,
                    })
                }
            }
        }
    };
    let view = match view {
        Some(v) if !v.multi => v,
        _ => {
            // One-time: multi-swapchain presents stay untouched.
            static MULTI_LOGGED: AtomicBool = AtomicBool::new(false);
            if !MULTI_LOGGED.swap(true, Ordering::SeqCst) {
                crate::log_line(
                    "render/overlay: non-single present (multi or unreadable) — forwarding untouched",
                );
            }
            // SAFETY: original call, untouched.
            return unsafe { real(queue, info) };
        }
    };
    let family = match family {
        Some(f) => f,
        None => {
            // One-time: queue family unknown (queue never seen via
            // GetDeviceQueue) — cannot place a command pool.
            static NOFAMILY_LOGGED: AtomicBool = AtomicBool::new(false);
            if !NOFAMILY_LOGGED.swap(true, Ordering::SeqCst) {
                crate::log_line(&format!(
                    "render/overlay: present queue {queue:#x} has unknown family — forwarding untouched"
                ));
            }
            // SAFETY: original call, untouched.
            return unsafe { real(queue, info) };
        }
    };
    // SAFETY: overlay draws (or fails cleanly); the real present always
    // runs exactly once below, original or re-targeted.
    unsafe {
        match crate::render::overlay::draw_frame(
            dev,
            queue,
            family,
            view.swapchain,
            view.image,
            &view.waits,
        ) {
            Ok(signal) => {
                // Re-target the present at OUR semaphore (single-wait
                // rule: the game's waits were consumed by our submit).
                // NOTE: this local struct mirrors VkPresentInfoKHR with
                // correct C padding (repr(C) handles it — WITHOUT the
                // attribute Rust layout is unspecified and the driver
                // would read garbage!).
                crate::render::note_overlay_drew();
                #[repr(C)]
                struct PresentInfo {
                    s_type: u32,
                    p_next: *const std::ffi::c_void,
                    wait_count: u32,
                    p_waits: *const usize,
                    swap_count: u32,
                    p_swaps: *const usize,
                    p_indices: *const u32,
                    p_results: *mut VkResult,
                }
                let sig = [signal];
                let modified = PresentInfo {
                    s_type: *(info as *const u32),
                    p_next: *(info.add(8) as *const *const std::ffi::c_void),
                    wait_count: 1,
                    p_waits: sig.as_ptr(),
                    swap_count: 1,
                    p_swaps: &view.swapchain as *const usize,
                    p_indices: &view.image as *const u32,
                    p_results: view.results,
                };
                real(queue, &modified as *const PresentInfo as *const std::ffi::c_void)
            }
            Err(()) => {
                // One-time: overlay attempted but failed (state creation
                // logs separately; per-frame failures land here).
                static DRAWFAIL_LOGGED: AtomicBool = AtomicBool::new(false);
                if !DRAWFAIL_LOGGED.swap(true, Ordering::SeqCst) {
                    crate::log_line(
                        "render/overlay: frame draw failed — presenting untouched",
                    );
                }
                crate::render::note_overlay_skipped();
                real(queue, info)
            }
        }
    }
}

unsafe extern "C" fn layer_get_instance_proc_addr(
    instance: usize,
    name: *const libc::c_char,
) -> *const std::ffi::c_void {
    match read_name(name).as_deref() {
        // Creation points MUST be intercepted here (not transparently
        // forwarded): the loader builds its instance/device dispatch
        // from what this function returns. Forwarding "vkCreateDevice"
        // transparently hands the loader the terminator's function, so
        // device creation bypasses `layer_create_device` entirely: no
        // per-device chain, empty device map, and every device-proc
        // lookup returns NULL (crashed vulkaninfo on the first NULL
        // dispatch call). Both creation points route to our wrappers.
        Some("vkCreateInstance") => layer_create_instance as *const std::ffi::c_void,
        Some("vkCreateDevice") => layer_create_device as *const std::ffi::c_void,
        Some("vkDestroyInstance") => layer_destroy_instance as *const std::ffi::c_void,
        Some("vkDestroyDevice") => layer_destroy_device as *const std::ffi::c_void,
        _ => STATE
            .lock()
            .unwrap()
            .next_instance_proc
            // SAFETY: passthrough with the caller's exact arguments.
            .map(|next| unsafe { next(instance, name) })
            .unwrap_or(std::ptr::null()),
    }
}

unsafe extern "C" fn layer_get_device_proc_addr(
    device: usize,
    name: *const libc::c_char,
) -> *const std::ffi::c_void {
    match read_name(name).as_deref() {
        Some("vkCreateDevice") => layer_create_device as *const std::ffi::c_void,
        Some("vkDestroyDevice") => layer_destroy_device as *const std::ffi::c_void,
        Some("vkCreateSwapchainKHR") => layer_create_swapchain as *const std::ffi::c_void,
        Some("vkDestroySwapchainKHR") => layer_destroy_swapchain as *const std::ffi::c_void,
        Some("vkGetDeviceQueue") => layer_get_device_queue as *const std::ffi::c_void,
        Some("vkGetDeviceQueue2") => layer_get_device_queue2 as *const std::ffi::c_void,
        Some("vkQueuePresentKHR") => layer_present as *const std::ffi::c_void,
        _ => STATE
            .lock()
            .unwrap()
            .devices
            .get(&device)
            .copied()
            // SAFETY: passthrough with the caller's exact arguments.
            .map(|next| unsafe { next(device, name) })
            .unwrap_or(std::ptr::null()),
    }
}

/// Physical-device proc addr: valid function, intercepts nothing.
///
/// The loader may call this without a null check when building
/// physical-device dispatch tables. Handing it a NULL *pointer* (the
/// old `None`) turns into a jump-to-zero crash; handing it a real
/// function that answers NULL per name is the correct "unsupported".
unsafe extern "C" fn layer_get_physical_device_proc_addr(
    _instance: usize,
    _name: *const libc::c_char,
) -> *const std::ffi::c_void {
    std::ptr::null()
}

// --- Loader handshake -----------------------------------------------------------

/// `vkNegotiateLoaderLayerInterfaceVersion` — called by the Vulkan loader
/// when it discovers us via the manifest. Hands over our proc-address
/// entry points and marks the layer path active so `render::install`
/// skips GOT/inline patching entirely.
#[unsafe(no_mangle)]
pub extern "C" fn vkNegotiateLoaderLayerInterfaceVersion(
    p_version: *mut NegotiateInterface,
) -> VkResult {
    if p_version.is_null() {
        return -3; // VK_ERROR_INITIALIZATION_FAILED
    }
    // SAFETY: loader-owned struct, valid for this call.
    unsafe {
        let got = (*p_version).s_type;
        let want = (*p_version).loader_layer_interface_version;
        if got != LAYER_NEGOTIATE_INTERFACE_STRUCT {
            // Log the mismatch (this exact silence cost us the layer
            // for weeks: the old constant rejected every loader
            // handshake without a word).
            crate::log_line(&format!(
                "render/layer: negotiate refused (sType {got}, want interface {want})"
            ));
            return -3;
        }
        (*p_version).loader_layer_interface_version = CURRENT_LOADER_LAYER_INTERFACE_VERSION
            .min((*p_version).loader_layer_interface_version);
        (*p_version).pfn_get_instance_proc_addr = Some(layer_get_instance_proc_addr);
        (*p_version).pfn_get_device_proc_addr = Some(layer_get_device_proc_addr);
        (*p_version).pfn_get_physical_device_proc_addr =
            Some(layer_get_physical_device_proc_addr);
        crate::log_line(&format!(
            "render/layer: negotiated with Vulkan loader (interface {want} -> {}), present routing active",
            (*p_version).loader_layer_interface_version,
        ));
    }
    LAYER_ACTIVE.store(true, Ordering::SeqCst);
    VK_SUCCESS
}
