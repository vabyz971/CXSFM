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
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    signal: vk::Semaphore,
    extent: vk::Extent2D,
    images: Vec<usize>,
    ctx: egui::Context,
    pending_frees: Vec<egui::TextureId>,
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
        // SAFETY: all handles owned by this state; nothing in flight
        // (every present fence-waits its predecessor).
        unsafe {
            let _ = st.device.device_wait_idle();
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
            // SAFETY: best-effort teardown (see above); results ignored.
            unsafe {
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
            command_buffer_count: 1,
            ..Default::default()
        };
        let cmd = device
            .allocate_command_buffers(&alloc_info)
            .map_err(|_| ())?
            .into_iter()
            .next()
            .ok_or(())?;
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
        })
    }
}

/// One headless egui run: primitives to draw plus texture deltas to
/// upload/free. Same draw calls as the headless UI check, with a real
/// context. Empty input (non-interactive v1): the UI is visible,
/// clicks come later.
struct UiOutput {
    primitives: Vec<egui::ClippedPrimitive>,
    set: Vec<(egui::TextureId, egui::epaint::ImageDelta)>,
    free: Vec<egui::TextureId>,
}

fn run_ui(ctx: &egui::Context, extent: vk::Extent2D) -> UiOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(extent.width as f32, extent.height as f32),
        )),
        ..Default::default()
    };
    let output = ctx.run(input, |ui_ctx| {
        crate::mod_api::draw_status_ui(ui_ctx);
        crate::mod_api::draw_manager_ui(ui_ctx);
        crate::mod_api::draw_ui_all(ui_ctx);
    });
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    UiOutput {
        primitives,
        set: output.textures_delta.set,
        free: output.textures_delta.free,
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
        // 1. Previous frame must be done (vertex buffers + frees reuse).
        st.device
            .wait_for_fences(&[st.fence], true, u64::MAX)
            .map_err(|_| ())?;
        st.device.reset_fences(&[st.fence]).map_err(|_| ())?;
        if !st.pending_frees.is_empty() {
            let frees = std::mem::take(&mut st.pending_frees);
            let _ = st.renderer.free_textures(&frees);
        }
        // 2. Run egui. Nothing to draw and nothing to upload = skip
        // the submit entirely (present goes untouched, zero cost).
        let ui = run_ui(&st.ctx, st.extent);
        if ui.primitives.is_empty() && ui.set.is_empty() {
            return Err(());
        }
        let fb = *st.framebuffers.get(image_index as usize).ok_or(())?;
        let img_raw = *st.images.get(image_index as usize).ok_or(())?;
        let img = vk::Image::from_raw(img_raw as u64);
        // 3. Upload new textures (font atlas lands on the first drawn
        // frame). Synchronous inside the renderer (own submit + wait).
        if !ui.set.is_empty() {
            st.renderer
                .set_textures(q, st.pool, &ui.set)
                .map_err(|_| ())?;
        }
        // 4. Record: barrier into COLOR_ATTACHMENT, render pass with
        // loadOp=LOAD (game image preserved), UI draws, barrier back
        // to PRESENT_SRC for the real present.
        st.device
            .reset_command_buffer(st.cmd, vk::CommandBufferResetFlags::empty())
            .map_err(|_| ())?;
        let begin_info = vk::CommandBufferBeginInfo {
            s_type: vk::StructureType::COMMAND_BUFFER_BEGIN_INFO,
            p_next: std::ptr::null(),
            flags: vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT,
            p_inheritance_info: std::ptr::null(),
            ..Default::default()
        };
        st.device
            .begin_command_buffer(st.cmd, &begin_info)
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
            st.cmd,
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
            st.cmd,
            &rp_begin,
            vk::SubpassContents::INLINE,
        );
        st.renderer
            .cmd_draw(st.cmd, st.extent, 1.0, &ui.primitives)
            .map_err(|_| ())?;
        st.device.cmd_end_render_pass(st.cmd);
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
            st.cmd,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        );
        st.device.end_command_buffer(st.cmd).map_err(|_| ())?;
        // 5. Submit waiting on the present's own semaphores (covers the
        // game's prior work, cross-queue included), signaling ours.
        // The real present then waits ONLY on ours (single-wait rule).
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
            p_command_buffers: &st.cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: &st.signal,
            ..Default::default()
        };
        st.device
            .queue_submit(q, &[submit], st.fence)
            .map_err(|_| ())?;
        st.pending_frees = ui.free;
        Ok(st.signal.as_raw() as usize)
    }
}
