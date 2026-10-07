//! FPV Camera mod: field-of-view control via IL2CPP (no render hook needed).
//!
//! > **Currently PARKED** (`PARKED = true` below): the mod performs zero
//! > game writes until the render hook allows per-frame application. The
//! > machinery (cache, resolve, apply, restore) stays compiled and ready;
//! > enabling the mod only flips framework state, provable in the log.
//!
//! # How it works when unparked (no memory patching at all)
//!
//! Each second (every 60th tick) the mod resolves `Camera.main` through
//! the cached [`crate::unity::UnityCache`] and calls
//! `set_fieldOfView` with the slider value. The game's *original* FOV is
//! read once per enable-session and restored on disable — the game is
//! left exactly as found.
//!
//! Scene changes are handled by re-resolving the camera object every
//! cycle (cheap: two invokes) while caching only classes/methods (stable
//! for the process lifetime). Everything degrades to silent no-ops
//! outside Unity (smoke tests) or before a camera exists (menus).
//!
//! # Verification without a render hook
//!
//! Press **O** in freeroam: the tick thread flips this mod on/off, the
//! perspective visibly shifts, and the log records each transition.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use egui;
use glam::{Mat4, Quat, Vec3};
use crate::mod_api::Mod;
use crate::unity::UnityCache;

/// Resolve + apply cycle: every Nth tick (~1 Hz at 60 Hz).
const RESOLVE_EVERY_TICKS: u64 = 60;
/// Fallback when the original FOV cannot be read (typical default).
const DEFAULT_BASE_FOV: f32 = 60.0;
/// Parked until the render hook exists: while true the mod performs
/// ZERO game writes (no `set_fieldOfView`, no restore — there is nothing
/// to restore). The game runs exactly as vanilla; enabling the mod only
/// flips framework state (visible in the log + UI). Flip to false once
/// per-frame application from the Present thread is possible.
const PARKED: bool = true;

/// FPV Camera mod - first-person/drone field of view via Unity API.
///
/// Purely cosmetic (camera projection only): no physics, no economy, no
/// network traffic. Anti-cheat-safe by construction.
pub struct FpvCameraMod {
    /// Whether the mod is currently active.
    enabled: AtomicBool,
    /// Camera position in world space (kept for the future render hook;
    /// the live game camera is driven through IL2CPP, not this field).
    camera_position: Vec3,
    /// Camera orientation as a quaternion (same note as above).
    camera_rotation: Quat,
    /// Target field of view in degrees (UI slider).
    fov: f32,
    /// Camera distance from target (UI slider, future use).
    distance: f32,
    /// Cached Unity classes/methods (`None` until first resolve).
    unity: Mutex<Option<UnityCache>>,
    /// Original FOV, read once per enable-session for restore.
    base_fov: Mutex<Option<f32>>,
    /// Tick counter driving the resolve cycle.
    tick: AtomicU64,
    /// Whether the current FOV was already applied + logged.
    applied: AtomicBool,
}

impl FpvCameraMod {
    /// Create a new FPV camera mod instance (starts enabled at 90° FOV so
    /// the very first session shows a visible shift vs the ~60° default).
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            camera_position: Vec3::ZERO,
            camera_rotation: Quat::IDENTITY,
            fov: 90.0,
            distance: 10.0,
            unity: Mutex::new(None),
            base_fov: Mutex::new(None),
            tick: AtomicU64::new(0),
            applied: AtomicBool::new(false),
        }
    }

    /// Set the camera position and rotation (glam-side state, used by the
    /// view/projection math below and the future render hook).
    pub fn set_camera(&mut self, position: Vec3, rotation: Quat) {
        self.camera_position = position;
        self.camera_rotation = rotation;
    }

    /// Compute the view matrix from camera position and rotation.
    ///
    /// Returns a 4x4 view matrix suitable for use with graphics APIs.
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.camera_rotation, self.camera_position)
    }

    /// Compute the projection matrix from FOV.
    ///
    /// aspect: Aspect ratio of the viewport (width / height)
    /// near: Near clipping plane
    /// far: Far clipping plane
    pub fn projection_matrix(&self, aspect: f32, near: f32, far: f32) -> Mat4 {
        Mat4::perspective_lh(self.fov.to_radians(), aspect, near, far)
    }

    /// Toggle the FPV camera on/off (shared interior-mutability core
    /// so both the UI button and programmatic callers behave identically).
    pub fn toggle(&self) {
        if self.is_enabled() {
            self.disable_inner();
        } else {
            self.enable_inner();
        }
    }

    /// Check if the FPV camera is currently enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn enable_inner(&self) {
        self.enabled.store(true, Ordering::SeqCst);
        // Force re-read of the original FOV: scenes (and their cameras)
        // change, stale values would restore the wrong framing.
        *self.base_fov.lock().unwrap() = None;
        self.applied.store(false, Ordering::SeqCst);
        if PARKED {
            crate::log_line("fpv: enabled (parked — no game writes)");
        } else {
            crate::log_line("fpv: enabled");
        }
    }

    fn disable_inner(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.applied.store(false, Ordering::SeqCst);
        self.restore();
        crate::log_line("fpv: disabled");
    }

    /// Ensure the Unity cache exists; resolve once, reuse afterwards.
    /// Returns false outside Unity (or before scripting is up).
    fn ensure_unity(&self) -> bool {
        if self.unity.lock().unwrap().is_some() {
            return true;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return false,
        };
        // SAFETY: called on attached framework threads with the
        // published API; every step below is null-checked.
        let cache = unsafe {
            let domain = match crate::il2cpp::domain_checked(api) {
                Some(d) => d,
                None => return false,
            };
            match crate::unity::init(api, domain) {
                Some(c) => c,
                None => return false,
            }
        };
        *self.unity.lock().unwrap() = Some(cache);
        crate::log_line("fpv: unity cache resolved");
        true
    }

    /// One resolve+apply cycle: fetch `Camera.main`, remember the
    /// original FOV once, apply the slider value. No-op while parked.
    fn apply_cycle(&self) {
        if PARKED {
            return;
        }        if !self.ensure_unity() {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        // Bind first so the guard drops at the statement boundary (see
        // the self-deadlock note in `poll_framework_input_edge`).
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        // SAFETY: attached threads; cached classes; fresh object handle.
        unsafe {
            let cam = match crate::unity::main_camera(api, &cache) {
                Some(c) => c,
                None => return, // no camera (menu/loading) — retry next cycle
            };
            {
                let mut base = self.base_fov.lock().unwrap();
                if base.is_none() {
                    // Remember the original exactly once per session so
                    // disable restores pixel-identical framing.
                    *base = crate::unity::get_fov(api, &cache, cam)
                        .or(Some(DEFAULT_BASE_FOV));
                }
            }
            if crate::unity::set_fov(api, &cache, cam, self.fov)
                && !self.applied.swap(true, Ordering::SeqCst)
            {
                crate::log_line(&format!("fpv: FOV applied ({:.0}°)", self.fov));
            }
        }
    }

    /// Restore the original FOV captured during this session, if any.
    /// No-op while parked (nothing was ever written).
    fn restore(&self) {
        if PARKED {
            return;
        }        let base = *self.base_fov.lock().unwrap();
        let (Some(api), Some(cache), Some(fov)) = (
            crate::il2cpp_api(),
            self.unity.lock().unwrap().as_ref().copied(),
            base,
        ) else {
            return;
        };
        // SAFETY: same contract as `apply_cycle`.
        unsafe {
            if let Some(cam) = crate::unity::main_camera(api, &cache) {
                if crate::unity::set_fov(api, &cache, cam, fov) {
                    crate::log_line(&format!("fpv: FOV restored ({:.0}°)", fov));
                }
            }
        }
    }
}

impl Default for FpvCameraMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for FpvCameraMod {
    fn name(&self) -> &'static str {
        "FPV Camera Mod"
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.is_enabled() {
            return;
        }
        // Throttle Unity traffic to ~1 Hz: two invokes per cycle is
        // nothing, but there is no reason to push harder.
        if self.tick.fetch_add(1, Ordering::SeqCst) % RESOLVE_EVERY_TICKS != 0 {
            return;
        }
        self.apply_cycle();
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        if !self.is_enabled() {
            return;
        }

        egui::Window::new("FPV Camera Controls")
            .default_pos(egui::pos2(0.0, 0.0))
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("FPV Camera Mod");
                if PARKED {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "PARKED until the render hook lands: no game writes, vanilla camera.",
                    );
                } else {
                    ui.label("Drone-like field of view (IL2CPP, no hooks)");
                }

                ui.separator();

                ui.label("Camera Settings:");
                ui.add(egui::Slider::new(&mut self.fov, 30.0..=120.0).text("FOV"));
                ui.add(
                    egui::Slider::new(&mut self.distance, 1.0..=50.0).text("Distance"),
                );

                ui.separator();

                if ui.button("Reset Camera").clicked() {
                    self.camera_position = Vec3::ZERO;
                    self.camera_rotation = Quat::IDENTITY;
                    self.restore();
                }

                if ui.button("Toggle FPV (O)").clicked() {
                    self.toggle();
                }
            });
    }

    fn on_enable(&mut self) {
        self.enable_inner();
    }

    fn on_disable(&mut self) {
        self.disable_inner();
    }
}

/// Helper function to register the FPV camera mod with the global mod manager
pub fn register_fpv_camera() {
    crate::mod_api::register_mod(Box::new(FpvCameraMod::new()));
}
