//! In-process Vulkan overlay: draws the framework's egui UI into the
//! game's swapchain images at Present time (Linux layer path).
//!
//! # How it works
//!
//! `layer_present` calls [`draw_frame`] before forwarding the real
//! present. For each presented swapchain image we: run egui headless,
//! record UI commands (via `egui-ash-renderer`) into our own command
//! buffer wrapped in a `loadOp = LOAD` render pass (the game image is
//! preserved underneath), submit waiting on the present's own
//! semaphores, and hand a fresh semaphore back so the real present
//! waits on US instead. Binary semaphores allow a single wait each,
//! so the game's wait list is REPLACED, never duplicated.
//!
//! # Failure contract (the game never breaks for the UI)
//!
//! Every step is fallible and every failure returns `Err(())`, in
//! which case `layer_present` forwards the ORIGINAL present call
//! untouched. Unknown swapchains (created before the layer was
//! active), unknown queue families, multi-swapchain presents, and any
//! Vulkan error all degrade to "no overlay this frame".
//!
//! # Serialization (v1)
//!
//! One in-flight frame per swapchain (`in_flight_frames = 1`): each
//! present fence-waits the previous one. Simple, correct, and paced by
//! the game's own present rate. No `vkQueueWaitIdle` anywhere —
//! pipelining is preserved through the semaphore chain.
//!
//! # Threading
//!
//! Present may arrive on any thread: all state lives behind one global
//! `Mutex`. ash `Instance`/`Device` are plain function tables (Send +
//! Sync); `egui::Context` is `Arc`-backed (Send + Sync); the renderer
//! is only touched through the guard.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use ash::vk;
/// `Handle::from_raw`/`as_raw` live on the trait in ash 0.38.
use ash::vk::Handle as _;

/// One in-flight frame is enough (see module docs).
const IN_FLIGHT_FRAMES: usize = 1;

/// Per-swapchain overlay state. Self-contained (own ash objects,
/// renderer, pool, fence, semaphore) so destruction is trivial and
/// two swapchains never share mutable GPU state.
struct SwapchainState {
    /// Owning VkDevice (for device-scoped teardown).
    dev: usize,
    device: ash::Device,
    renderer: egui_ash_renderer::Renderer,
    render_pass: vk::RenderPass,
    views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
    pool: vk::CommandPool,
    /// One command buffer PER swapchain image. A recording embeds its
    /// target framebuffer/image, so replaying buffer N on image M
    /// draws the UI onto the wrong present (flicker: the UI only
    /// survived presents that reused the recorded image). Replay
    /// always submits `cmd[image_index]`.
    cmd: Vec<vk::CommandBuffer>,
    fence: vk::Fence,
    signal: vk::Semaphore,
    extent: vk::Extent2D,
    images: Vec<usize>,
    ctx: egui::Context,
    /// Frees deferred one frame: `(texture id, emission frame)`.
    /// A free is obsolete iff its id was (re-)set at the emission
    /// frame or later (`set_textures` destroys/replaces same-id
    /// occupants internally — e.g. atlas resizes — so freeing one
    /// afterwards would destroy the NEW texture).
    pending_frees: Vec<(egui::TextureId, u64)>,
    /// Last frame each texture id was uploaded (same numbering).
    last_set: std::collections::HashMap<egui::TextureId, u64>,
    /// Monotonic frame counter for the generation protocol above.
    frame: u64,
    /// Earliest time egui wants repainting (static UI sleeps instead
    /// of re-running; the last recorded command buffer is REPLAYED
    /// every present regardless — see below).
    next_repaint: Option<std::time::Instant>,
    /// Per-image recording state (parallel to `cmd`): a command buffer
    /// with UI already recorded (replayable without re-running egui).
    /// False until the first non-empty draw and after any
    /// invalidation (UI vanished or changed — other images re-record
    /// on their next present, converging within one swapchain cycle).
    recorded: Vec<bool>,
}

/// Global overlay state: swapchain handle -> renderer state, plus the
/// shared ash instance (built once from the first live VkInstance).
struct OverlayGlobals {
    states: HashMap<usize, SwapchainState>,
    ash_instance: Option<ash::Instance>,
}

static GLOBALS: LazyLock<Mutex<OverlayGlobals>> = LazyLock::new(|| {
    Mutex::new(OverlayGlobals {
        states: HashMap::new(),
        ash_instance: None,
    })
});

/// Swapchain handles whose creation already failed once: skip silently
/// afterwards (a persistent failure would otherwise log 60×/s).
static FAILED_SWAPS: LazyLock<Mutex<HashSet<usize>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Drop all overlay state for a destroyed swapchain.
///
/// Called from the layer's swapchain teardown. Renderer `Drop` frees
/// its own resources; the rest is destroyed explicitly here. Unknown
/// handles are a silent no-op.
pub fn drop_swapchain(swap: usize) {
    let mut g = GLOBALS.lock().unwrap();
    if let Some(st) = g.states.remove(&swap) {
        // SAFETY: all handles owned by this state. Fence-wait (bounded)
        // instead of device_wait_idle: only OUR submissions touch OUR
        // objects, and an unbounded device-wide wait can hang teardown
        // behind unrelated (or stuck) queue work. On timeout we destroy
        // anyway — leaking GPU objects is safer than hanging the game.
        unsafe {
            if st
                .device
                .wait_for_fences(&[st.fence], true, 1_000_000_000)
                .is_err()
            {
                crate::log_line(&format!(
                    "render/overlay: fence timeout dropping swapchain {swap:#x} (destroying anyway)"
                ));
            }
            for v in &st.views {
                st.device.destroy_image_view(*v, None);
            }
            for fb in &st.framebuffers {
                st.device.destroy_framebuffer(*fb, None);
            }
            st.device.destroy_command_pool(st.pool, None);
            st.device.destroy_fence(st.fence, None);
            st.device.destroy_semaphore(st.signal, None);
            st.device.destroy_render_pass(st.render_pass, None);
            drop(st.renderer);
            drop(st.device);
        }
        crate::log_line(&format!("render/overlay: swapchain {swap:#x} state dropped"));
    }
    FAILED_SWAPS.lock().unwrap().remove(&swap);
}

/// Drop all overlay state scoped to a destroyed VkDevice.
///
/// Called BEFORE the real destroy is forwarded (all handles still
/// live). Renderer `Drop` frees its own resources; the rest is
/// destroyed explicitly here. Unknown devices are a silent no-op.
pub fn drop_device(dev: usize) {
    let mut g = GLOBALS.lock().unwrap();
    let dead: Vec<usize> = g
        .states
        .iter()
        .filter(|(_, st)| st.dev == dev)
        .map(|(s, _)| *s)
        .collect();
    for swap in dead {
        if let Some(st) = g.states.remove(&swap) {
            // SAFETY: bounded fence wait (see `drop_swapchain`), then
            // best-effort teardown; results ignored.
            unsafe {
                let _ = st.device.wait_for_fences(&[st.fence], true, 1_000_000_000);
                for v in &st.views {
                    let _ = st.device.destroy_image_view(*v, None);
                }
                for fb in &st.framebuffers {
                    let _ = st.device.destroy_framebuffer(*fb, None);
                }
                let _ = st.device.destroy_command_pool(st.pool, None);
                let _ = st.device.destroy_fence(st.fence, None);
                let _ = st.device.destroy_semaphore(st.signal, None);
                let _ = st.device.destroy_render_pass(st.render_pass, None);
                drop(st.renderer);
                drop(st.device);
            }
            crate::log_line(&format!(
                "render/overlay: swapchain {swap:#x} state dropped with its device"
            ));
        }
        FAILED_SWAPS.lock().unwrap().remove(&swap);
    }
}

/// Whether a raw VkFormat is an sRGB target (renderer shader variant).
#[inline]
fn is_srgb(format: u32) -> bool {
    // VK_FORMAT_R8G8B8A8_SRGB = 43, VK_FORMAT_B8G8R8A8_SRGB = 50.
    format == 43 || format == 50
}

/// Build an ash `Instance` resolving through our layer chain with a
/// live VkInstance (function pointers are process-global; any live
/// instance serves for the lookup itself).
///
/// # Safety
/// The instance handle must be live at call time. Cached tables stay
/// valid afterwards (pointers don't expire with the instance).
unsafe fn make_instance(
    next: crate::layer::GetInstanceProcAddrFn,
    inst: usize,
) -> ash::Instance {
    // SAFETY: validated chain + live instance from the caller.
    unsafe {
        ash::Instance::load_with(
            |name| {
                let p = next(inst, name.as_ptr());
                p as *const std::ffi::c_void
            },
            vk::Instance::from_raw(inst as u64),
        )
    }
}

/// Build an ash `Device` resolving through one device's chain.
///
/// # Safety
/// The device handle must be live at call time (same caching note).
unsafe fn make_device(
    next: crate::layer::GetDeviceProcAddrFn,
    dev: usize,
) -> ash::Device {
    // SAFETY: validated chain + live device from the caller.
    unsafe {
        ash::Device::load_with(
            |name| {
                let p = next(dev, name.as_ptr());
                p as *const std::ffi::c_void
            },
            vk::Device::from_raw(dev as u64),
        )
    }
}

/// Create the `loadOp = LOAD` render pass preserving the game image.
///
/// One color attachment (the swapchain format, single-sampled —
/// presentable images always are), no depth: `initialLayout` and
/// `finalLayout` are both `PRESENT_SRC_KHR`; the subpass auto-moves
/// through `COLOR_ATTACHMENT_OPTIMAL` while we draw. An external
/// dependency orders the layout transition after prior color output
/// (our submit already waited the game's semaphores at that stage).
fn create_render_pass(device: &ash::Device, format: vk::Format) -> Result<vk::RenderPass, ()> {
    let attachment = vk::AttachmentDescription {
        flags: vk::AttachmentDescriptionFlags::empty(),
        format,
        samples: vk::SampleCountFlags::TYPE_1,
        load_op: vk::AttachmentLoadOp::LOAD,
        store_op: vk::AttachmentStoreOp::STORE,
        stencil_load_op: vk::AttachmentLoadOp::DONT_CARE,
        stencil_store_op: vk::AttachmentStoreOp::DONT_CARE,
        initial_layout: vk::ImageLayout::PRESENT_SRC_KHR,
        final_layout: vk::ImageLayout::PRESENT_SRC_KHR,
        ..Default::default()
    };
    let color_ref = vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    };
    let subpass = vk::SubpassDescription {
        flags: vk::SubpassDescriptionFlags::empty(),
        pipeline_bind_point: vk::PipelineBindPoint::GRAPHICS,
        input_attachment_count: 0,
        p_input_attachments: std::ptr::null(),
        color_attachment_count: 1,
        p_color_attachments: &color_ref,
        p_resolve_attachments: std::ptr::null(),
        p_depth_stencil_attachment: std::ptr::null(),
        preserve_attachment_count: 0,
        p_preserve_attachments: std::ptr::null(),
        ..Default::default()
    };
    let dependency = vk::SubpassDependency {
        src_subpass: vk::SUBPASS_EXTERNAL,
        dst_subpass: 0,
        src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        src_access_mask: vk::AccessFlags::empty(),
        dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        dependency_flags: vk::DependencyFlags::empty(),
        ..Default::default()
    };
    let create_info = vk::RenderPassCreateInfo {
        s_type: vk::StructureType::RENDER_PASS_CREATE_INFO,
        p_next: std::ptr::null(),
        flags: vk::RenderPassCreateFlags::empty(),
        attachment_count: 1,
        p_attachments: &attachment,
        subpass_count: 1,
        p_subpasses: &subpass,
        dependency_count: 1,
        p_dependencies: &dependency,
        ..Default::default()
    };
    // SAFETY: valid create-info; errors mapped to overlay fallback.
    unsafe { device.create_render_pass(&create_info, None).map_err(|_| ()) }
}

/// Create one image view per swapchain image (color aspect, single
/// mip/layer — presentable images by definition).
fn create_views(
    device: &ash::Device,
    images: &[usize],
    format: vk::Format,
) -> Result<Vec<vk::ImageView>, ()> {
    let mut views = Vec::with_capacity(images.len());
    for img in images {
        let create_info = vk::ImageViewCreateInfo {
            s_type: vk::StructureType::IMAGE_VIEW_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::ImageViewCreateFlags::empty(),
            image: vk::Image::from_raw(*img as u64),
            view_type: vk::ImageViewType::TYPE_2D,
            format,
            components: vk::ComponentMapping::default(),
            subresource_range: vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            },
            ..Default::default()
        };
        // SAFETY: live swapchain image + valid create-info.
        let view = unsafe { device.create_image_view(&create_info, None).map_err(|_| ()) }?;
        views.push(view);
    }
    Ok(views)
}

/// Create one framebuffer per view over the shared render pass.
fn create_framebuffers(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    views: &[vk::ImageView],
    extent: vk::Extent2D,
) -> Result<Vec<vk::Framebuffer>, ()> {
    let mut fbs = Vec::with_capacity(views.len());
    for view in views {
        let create_info = vk::FramebufferCreateInfo {
            s_type: vk::StructureType::FRAMEBUFFER_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::FramebufferCreateFlags::empty(),
            render_pass,
            attachment_count: 1,
            p_attachments: view,
            width: extent.width,
            height: extent.height,
            layers: 1,
            ..Default::default()
        };
        // SAFETY: compatible view + render pass + extent.
        let fb = unsafe { device.create_framebuffer(&create_info, None).map_err(|_| ()) }?;
        fbs.push(fb);
    }
    Ok(fbs)
}

/// Build full overlay state for one swapchain on first present.
///
/// Heavy one-time work (~100 ms: render pass, views, framebuffers,
/// pool, fence, semaphore, renderer + font atlas on first draw).
/// Any failure returns `Err` (caller presents untouched).
fn create_state(
    dev: usize,
    family: u32,
    info: &crate::layer::SwapchainInfo,
    instance: &ash::Instance,
) -> Result<SwapchainState, ()> {
    let next_dev = crate::layer::device_chain(dev).ok_or(())?;
    let phys = crate::layer::device_physical(dev).ok_or(())?;
    // SAFETY: all handles captured live from the loader chain.
    unsafe {
        let device = make_device(next_dev, dev);
        let format = vk::Format::from_raw(info.format as i32);
        let extent = vk::Extent2D {
            width: info.extent.0,
            height: info.extent.1,
        };
        if extent.width == 0 || extent.height == 0 {
            return Err(());
        }
        let render_pass = create_render_pass(&device, format)?;
        let views = create_views(
            &device,
            &info.images,
            format,
        )?;
        let framebuffers = create_framebuffers(&device, render_pass, &views, extent)?;
        let pool_info = vk::CommandPoolCreateInfo {
            s_type: vk::StructureType::COMMAND_POOL_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            queue_family_index: family,
            ..Default::default()
        };
        let pool = device.create_command_pool(&pool_info, None).map_err(|_| ())?;
        let alloc_info = vk::CommandBufferAllocateInfo {
            s_type: vk::StructureType::COMMAND_BUFFER_ALLOCATE_INFO,
            p_next: std::ptr::null(),
            command_pool: pool,
            level: vk::CommandBufferLevel::PRIMARY,
            command_buffer_count: info.images.len() as u32,
            ..Default::default()
        };
        let cmd = device
            .allocate_command_buffers(&alloc_info)
            .map_err(|_| ())?;
        if cmd.len() != info.images.len() {
            return Err(());
        }
        let recorded = vec![false; info.images.len()];
        let fence_info = vk::FenceCreateInfo {
            s_type: vk::StructureType::FENCE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::FenceCreateFlags::SIGNALED,
            ..Default::default()
        };
        let fence = device.create_fence(&fence_info, None).map_err(|_| ())?;
        let sem_info = vk::SemaphoreCreateInfo {
            s_type: vk::StructureType::SEMAPHORE_CREATE_INFO,
            p_next: std::ptr::null(),
            flags: vk::SemaphoreCreateFlags::empty(),
            ..Default::default()
        };
        let signal = device.create_semaphore(&sem_info, None).map_err(|_| ())?;
        let options = egui_ash_renderer::Options {
            in_flight_frames: IN_FLIGHT_FRAMES,
            srgb_framebuffer: is_srgb(info.format),
            ..Default::default()
        };
        let renderer = egui_ash_renderer::Renderer::with_default_allocator(
            &instance,
            vk::PhysicalDevice::from_raw(phys as u64),
            device.clone(),
            render_pass,
            options,
        )
        .map_err(|_| ())?;
        Ok(SwapchainState {
            dev,
            device,
            renderer,
            render_pass,
            views,
            framebuffers,
            pool,
            cmd,
            fence,
            signal,
            extent,
            images: info.images.clone(),
            ctx: egui::Context::default(),
            pending_frees: Vec::new(),
            last_set: std::collections::HashMap::new(),
            frame: 0,
            next_repaint: None,
            recorded,
        })
    }
}

/// One headless egui run: primitives to draw plus texture deltas to
/// upload/free. Same draw calls as the headless UI check, with a real
/// context. `events` come from the capture pump (empty when passive);
/// a software cursor is painted in the foreground while captured (the
/// game hides/confines the OS cursor, so egui would otherwise fly
/// blind).
struct UiOutput {
    primitives: Vec<egui::ClippedPrimitive>,
    set: Vec<(egui::TextureId, egui::epaint::ImageDelta)>,
    free: Vec<egui::TextureId>,
    /// How long egui wants to wait before the next repaint (static UI
    /// sleeps; animations/interaction repaint immediately).
    repaint_delay: std::time::Duration,
    /// Pixels per point used for tessellation (must match `cmd_draw`).
    pixels_per_point: f32,
}

/// Monotonic seconds for `RawInput::time` (animations, double-click).
static T0: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);

fn run_ui(
    ctx: &egui::Context,
    extent: vk::Extent2D,
    events: Vec<egui::Event>,
) -> UiOutput {
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(extent.width as f32, extent.height as f32),
        )),
        time: Some(T0.elapsed().as_secs_f64()),
        focused: crate::hotkey::is_captured(),
        ..Default::default()
    };
    input.events = events;
    let output = ctx.run(input, |ui_ctx| {
        crate::mod_api::draw_status_ui(ui_ctx);
        crate::mod_api::draw_manager_ui(ui_ctx);
        // Tool windows draw whenever the menu is open; pinned ones
        // stay visible after it closes (their effects keep running
        // either way — UI vs effects are independent).
        crate::mod_api::draw_ui_all(ui_ctx, crate::hotkey::ui_visible());
        // Software cursor on top while captured.
        if crate::hotkey::is_captured() {
            let (x, y) = crate::hotkey::cursor_pos();
            egui::Area::new("cxsfm_cursor".into())
                .order(egui::Order::Foreground)
                .show(ui_ctx, |ui| {
                    let p = ui.painter();
                    let c = egui::pos2(x, y);
                    p.circle_filled(c, 7.0, egui::Color32::from_white_alpha(96));
                    p.circle_filled(c, 2.5, egui::Color32::WHITE);
                });
        }
    });
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    let repaint_delay = output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|v| v.repaint_delay)
        .unwrap_or(std::time::Duration::ZERO);
    UiOutput {
        primitives,
        set: output.textures_delta.set,
        free: output.textures_delta.free,
        repaint_delay,
        pixels_per_point: output.pixels_per_point,
    }
}

/// Draw the overlay for one presented image.
///
/// Returns the semaphore the real present must wait on. Any failure
/// returns `Err` and the caller forwards the original present call.
pub fn draw_frame(
    dev: usize,
    queue: usize,
    family: u32,
    swap: usize,
    image_index: u32,
    waits: &[usize],
) -> Result<usize, ()> {
    let info = match crate::layer::swapchain_info(swap) {
        Some(i) => i,
        None => {
            // One-time per handle: presents on a swapchain whose
            // creation we never saw (missed CreateSwapchain routing).
            static UNKNOWN_SWAPS: LazyLock<Mutex<HashSet<usize>>> =
                LazyLock::new(|| Mutex::new(HashSet::new()));
            if UNKNOWN_SWAPS.lock().unwrap().insert(swap) {
                crate::log_line(&format!(
                    "render/overlay: unknown swapchain {swap:#x} (creation missed?) — forwarding untouched"
                ));
            }
            return Err(());
        }
    };
    if FAILED_SWAPS.lock().unwrap().contains(&swap) {
        return Err(());
    }
    // Get-or-create state (creation logs once; failures park the swap).
    // The shared ash Instance is built once (function tables are
    // process-global; the live instance only serves the lookup).
    let mut g = GLOBALS.lock().unwrap();
    if g.ash_instance.is_none() {
        let (next_inst, live_inst) =
            match (crate::layer::instance_chain(), crate::layer::first_instance()) {
                (Some(n), Some(i)) => (n, i),
                _ => return Err(()),
            };
        // SAFETY: live instance captured from the loader chain.
        g.ash_instance = Some(unsafe { make_instance(next_inst, live_inst) });
    }
    if !g.states.contains_key(&swap) {
        // Borrow split: instance ref + state creation (no nested locks;
        // everything here is already behind the single GLOBALS guard,
        // and layer accessors use their own separate mutex).
        let created = {
            let inst = g.ash_instance.as_ref().ok_or(())?;
            create_state(dev, family, &info, inst)
        };
        match created {
            Ok(st) => {
                crate::log_line(&format!(
                    "render/overlay: state ready for swapchain {swap:#x} ({}x{}, format {})",
                    info.extent.0, info.extent.1, info.format,
                ));
                g.states.insert(swap, st);
            }
            Err(()) => {
                crate::log_line(&format!(
                    "render/overlay: state creation failed for swapchain {swap:#x} (parking)"
                ));
                FAILED_SWAPS.lock().unwrap().insert(swap);
                return Err(());
            }
        }
    }
    let st = g.states.get_mut(&swap).ok_or(())?;
    // SAFETY: everything below runs validated handles on the present
    // queue; any Vulkan error aborts to the untouched-present fallback.
    unsafe {
        let q = vk::Queue::from_raw(queue as u64);
        // 0. Freshness: drain events (cheap) and detect UI-state
        // changes (mod toggles alter content without input events).
        // A re-run is needed when: input pending, repaint due, never
        // recorded yet, the enabled set changed, or a pinned window is
        // visible with the menu closed (10 Hz heartbeat — static UI
        // would otherwise sleep on `repaint_delay`). Otherwise the
        // last recorded command buffer is REPLAYED below — this is
        // what kills the flicker (skipped frames used to present
        // WITHOUT ui) while keeping idle cost near zero (no egui
        // run, no tessellation, no texture work — just a submit).
        let events = crate::hotkey::drain_input_events();
        static LAST_ENABLED: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(usize::MAX);
        let enabled = crate::mod_api::get_mod_manager().enabled_count();
        let enabled_changed =
            LAST_ENABLED.swap(enabled, std::sync::atomic::Ordering::SeqCst) != enabled;
        let repaint_due = match st.next_repaint {
            Some(next) => std::time::Instant::now() >= next,
            None => true,
        };
        // Pinned tool windows must stay LIVE with the menu closed:
        // static UI sleeps on `repaint_delay` (up to an hour), so
        // force a re-run at ~10 Hz while a pin is visible. Input
        // still belongs to the game (view-only) — reopen the menu
        // to interact.
        static LAST_PIN_RUN: LazyLock<Mutex<Option<std::time::Instant>>> =
            LazyLock::new(|| Mutex::new(None));
        let menu_open = crate::hotkey::ui_visible();
        let pinned_beat = !menu_open
            && crate::mod_api::get_mod_manager().has_pinned_visible()
            && {
                let mut last = LAST_PIN_RUN.lock().unwrap();
                let due = last
                    .map(|t| t.elapsed() >= std::time::Duration::from_millis(100))
                    .unwrap_or(true);
                if due {
                    *last = Some(std::time::Instant::now());
                }
                due
            };
        // Per-image dirty: an image replays only its OWN recording.
        let idx = image_index as usize;
        let img_recorded = st.recorded.get(idx).copied().unwrap_or(false);
        let content_changed = !events.is_empty() || repaint_due || enabled_changed || pinned_beat;
        let dirty = content_changed || !img_recorded;
        // A real content change invalidates EVERY image (their
        // recordings hold the old UI); each re-records on its next
        // present, converging within one swapchain cycle. A merely
        // unrecorded image re-records alone.
        if dirty && content_changed {
            for r in st.recorded.iter_mut() {
                *r = false;
            }
        }
        // Run egui only when dirty; replayed frames reuse everything.
        let ui = if dirty {
            let ui = run_ui(&st.ctx, st.extent, events);
            // Schedule the next repaint. `repaint_delay` is unbounded
            // for static UI (`Duration::MAX` = "never") — a plain
            // `Instant +` OVERFLOWS and panics (killed a session via
            // abort in the present thread!). Saturate instead; events
            // always bypass the gate regardless of this deadline.
            st.next_repaint = Some(
                std::time::Instant::now()
                    .checked_add(ui.repaint_delay)
                    .unwrap_or_else(|| {
                        std::time::Instant::now() + std::time::Duration::from_secs(3600)
                    }),
            );
            Some(ui)
        } else {
            None
        };
        // TEMPORARY text-mesh census (titles/labels missing while
        // shapes draw): first NON-EMPTY frame counts textured (glyph)
        // vs solid vertices plus the glyph UV bounding box. Solid
        // fills share the font texture at WHITE_UV; glyphs use varied
        // UVs. Zero textured verts = tessellation/culling issue; sane
        // UVs + blank text = sampling/atlas issue; insane UVs =
        // transform bug.
        static CENSUS_LOGGED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if !CENSUS_LOGGED.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(ui) = ui.as_ref() {
                if !ui.primitives.is_empty() {
                    CENSUS_LOGGED.store(true, std::sync::atomic::Ordering::SeqCst);
                    let mut prims = 0usize;
                    let mut solid_verts = 0usize;
                    let mut textured_verts = 0usize;
                    let mut umin = f32::MAX;
                    let mut umax = f32::MIN;
                    let mut vmin = f32::MAX;
                    let mut vmax = f32::MIN;
                    for prim in &ui.primitives {
                        prims += 1;
                        if let egui::epaint::Primitive::Mesh(mesh) = &prim.primitive {
                            let textured = mesh
                                .vertices
                                .iter()
                                .any(|v| v.uv != egui::epaint::WHITE_UV);
                            if textured {
                                textured_verts += mesh.vertices.len();
                                for v in &mesh.vertices {
                                    umin = umin.min(v.uv.x);
                                    umax = umax.max(v.uv.x);
                                    vmin = vmin.min(v.uv.y);
                                    vmax = vmax.max(v.uv.y);
                                }
                            } else {
                                solid_verts += mesh.vertices.len();
                            }
                        }
                    }
                    crate::log_line(&format!(
                        "render/overlay: mesh census (primitives={prims} solid_verts={solid_verts} textured_verts={textured_verts} uv=[{umin:.3}..{umax:.3}]x[{vmin:.3}..{vmax:.3}])"
                    ));
                }
            }
        }
        // `fresh` holds this frame's UI (None = replay cached commands).
        // Empty UI invalidates the recording (hidden/disabled mods must
        // not leave stale frames replaying) and presents untouched.
        let fresh: Option<UiOutput> = ui;
        match &fresh {
            Some(u) if u.primitives.is_empty() && u.set.is_empty() => {
                for r in st.recorded.iter_mut() {
                    *r = false;
                }
                return Err(());
            }
            _ => {}
        }
        st.frame += 1;
        let frame = st.frame;
        // Previous frame must be done (renderer buffers are reused
        // across frames with in_flight_frames = 1; replayed submits
        // included — the fence tracks every submit).
        st.device
            .wait_for_fences(&[st.fence], true, u64::MAX)
            .map_err(|_| ())?;
        st.device.reset_fences(&[st.fence]).map_err(|_| ())?;
        if let Some(u) = &fresh {
            // Free textures egui abandoned — EXCEPT ids (re-)set at
            // their free's emission frame or later (see field docs).
            for (id, _) in &u.set {
                st.last_set.insert(*id, frame);
            }
            // Atlas-resize witness (rare by design).
            if u.set.iter().any(|(id, _)| u.free.contains(id)) {
                crate::log_line("render/overlay: font atlas resized (free+set same id)");
            }
            let mut pending = std::mem::take(&mut st.pending_frees);
            pending.retain(|(id, emitted)| {
                st.last_set.get(id).copied().unwrap_or(0) < *emitted
            });
            if !pending.is_empty() {
                let ids: Vec<egui::TextureId> =
                    pending.iter().map(|(id, _)| *id).collect();
                let _ = st.renderer.free_textures(&ids);
            }
            // Upload new textures (font atlas lands on the first drawn
            // frame). Synchronous inside the renderer (own submit + wait).
            if !u.set.is_empty() {
                st.renderer
                    .set_textures(q, st.pool, &u.set)
                    .map_err(|_| ())?;
            }
        }
        // 4. Record fresh commands, or replay the cached buffer.
        // Replay keeps every present showing the UI (skipping the
        // submit would flicker: the game redraws its image each
        // frame) at near-zero CPU cost (no egui run, no tessellation).
        let fb = *st.framebuffers.get(idx).ok_or(())?;
        let img_raw = *st.images.get(idx).ok_or(())?;
        let img = vk::Image::from_raw(img_raw as u64);
        let cmd = *st.cmd.get(idx).ok_or(())?;
        if let Some(u) = fresh.as_ref() {
        // 4. Record: barrier into COLOR_ATTACHMENT, render pass with
        // loadOp=LOAD (game image preserved), UI draws, barrier back
        // to PRESENT_SRC for the real present.
        st.device
            .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())
            .map_err(|_| ())?;
        // Record path ONLY (replay re-submits the cached buffer
        // untouched). No ONE_TIME_SUBMIT: that flag promises a single
        // submit per recording, and re-submitting such a buffer is
        // undefined (blank/garbage frames — the flicker). Our buffer
        // is explicitly reset before every re-record instead.
        let begin_info = vk::CommandBufferBeginInfo {
            s_type: vk::StructureType::COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::CommandBufferUsageFlags::empty(),
            p_inheritance_info: std::ptr::null(),
            ..Default::default()
        };
        st.device
             .begin_command_buffer(cmd, &begin_info)
            .map_err(|_| ())?;
        let subresource = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
            ..Default::default()
        };
        let to_draw = vk::ImageMemoryBarrier {
            s_type: vk::StructureType::IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::AccessFlags::empty(),
            dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            old_layout: vk::ImageLayout::PRESENT_SRC_KHR,
            new_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image: img,
            subresource_range: subresource,
            ..Default::default()
        };
        st.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_draw],
        );
        let rp_begin = vk::RenderPassBeginInfo {
            s_type: vk::StructureType::RENDER_PASS_BEGIN_INFO,
            p_next: std::ptr::null(),
            render_pass: st.render_pass,
            framebuffer: fb,
            render_area: vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: st.extent,
            },
            clear_value_count: 0,
            p_clear_values: std::ptr::null(),
            ..Default::default()
        };
        st.device.cmd_begin_render_pass(
            cmd,
            &rp_begin,
            vk::SubpassContents::INLINE,
        );
        st.renderer
            .cmd_draw(cmd, st.extent, u.pixels_per_point, &u.primitives)
            .map_err(|_| ())?;
        st.device.cmd_end_render_pass(cmd);
        let to_present = vk::ImageMemoryBarrier {
            s_type: vk::StructureType::IMAGE_MEMORY_BARRIER,
            p_next: std::ptr::null(),
            src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            dst_access_mask: vk::AccessFlags::empty(),
            old_layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            new_layout: vk::ImageLayout::PRESENT_SRC_KHR,
            src_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: vk::QUEUE_FAMILY_IGNORED,
            image: img,
            subresource_range: subresource,
            ..Default::default()
        };
        st.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        );
        st.device.end_command_buffer(cmd).map_err(|_| ())?;
        } // end `if let Some(u)` — replay path skips recording entirely.
        // 5. Submit (freshly recorded or replayed) waiting on the
        // present's own semaphores (covers the game's prior work,
        // cross-queue included), signaling ours. The real present then
        // waits ONLY on ours (single-wait rule).
        let wait_sems: Vec<vk::Semaphore> = waits
            .iter()
            .map(|s| vk::Semaphore::from_raw(*s as u64))
            .collect();
        let wait_stages =
            vec![vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT; wait_sems.len()];
        let submit = vk::SubmitInfo {
            s_type: vk::StructureType::SUBMIT_INFO,
            p_next: std::ptr::null(),
            wait_semaphore_count: wait_sems.len() as u32,
            p_wait_semaphores: wait_sems.as_ptr(),
            p_wait_dst_stage_mask: wait_stages.as_ptr(),
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: &st.signal,
            ..Default::default()
        };
        st.device
            .queue_submit(q, &[submit], st.fence)
            .map_err(|_| ())?;
        if let Some(u) = fresh {
            st.recorded[idx] = true;
            st.pending_frees = u.free.into_iter().map(|id| (id, frame)).collect();
        }
        Ok(st.signal.as_raw() as usize)
    }
}
