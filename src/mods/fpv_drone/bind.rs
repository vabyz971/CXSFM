//! FPV drone tick-side work: take over the camera, fly the rigid-body
//! model, collide with the world, give the camera back.
//!
//! Runs on the framework tick thread. Unity traffic is bounded:
//! - takeover = ONE enumeration (Cinemachine brains + the game's camera
//!   controllers, flags recorded then disabled), repeated only when
//!   `Camera.main` changes (scene change) — never periodic, because a
//!   periodic `FindObjectsOfType` crashed the game while driving;
//! - flight = `Camera.main` + its transform (alive-checked) + at most
//!   one `Physics.Raycast` per tick; no search, no stored object
//!   pointers across ticks except the controller flags needed for the
//!   restore (each one alive-checked before use).
//!
//! Collisions need `Physics.Raycast` to work from this thread. A managed
//! exception (main-thread-only check) disables them after a few strikes
//! and a flat fallback floor under the takeoff point takes over.

use super::sound::Audio;
use super::model::{BUTTONS, Bindings, Drone, Hit, Mode, Params, Sticks, learn_axis, learn_button, read_sticks};
use super::{Shared, Telemetry};
use crate::gamepad;
use crate::mods::common::lock;
use crate::unity::{self, UnityCache};
use glam::Vec3;
use std::sync::Mutex;

/// Retry cadence while the camera is not there yet (ticks, ~1 Hz).
const RETRY_EVERY: u64 = 60;
/// Takeover caps (a broken list must not disable the world).
const MAX_CUT: usize = 16;
/// Raycast failures in a row before collisions are given up.
const RAY_STRIKES: u32 = 5;
/// Fallback floor below the takeoff point (m): photo mode hovers about
/// eye level above the ground.
const FALLBACK_FLOOR: f32 = 1.5;
/// `Physics.DefaultRaycastLayers` and `QueryTriggerInteraction.Ignore`.
const LAYER_MASK: i32 = -5;
const QTI_IGNORE: i32 = 1;
/// Axis travel that counts as "this is the one" while learning.
const LEARN_TIMEOUT_TICKS: u64 = 60 * 15;

#[derive(Default)]
pub(crate) struct Rt {
    unity: Option<UnityCache>,
    tick: u64,
    last_try: u64,
    flying: bool,
    armed: bool,
    drone: Option<Drone>,
    /// Pose at takeover, for Reset and the fallback floor.
    start: Option<([f32; 3], [f32; 4])>,
    /// Camera we took over (address only compared, never dereferenced
    /// without `object_alive`).
    cam: usize,
    /// `(component, original enabled)` of what we switched off.
    cut: Vec<(usize, bool)>,
    base_fov: Option<f32>,
    last_fov: f32,
    /// `Physics.Raycast` (6-arg overload), resolved once.
    ray_method: usize,
    ray_tried: bool,
    ray_strikes: u32,
    prev_buttons: [bool; BUTTONS],
    /// Camera changed under us (game menu, scene switch): the game has
    /// its drivers back, we wait for the original camera (or the arm
    /// button on the new one).
    suspended: bool,
    learn_base: Option<([f32; 8], [bool; BUTTONS], u64)>,
    audio: Audio,
    /// The game's `FMOD.Studio.System` (0 = not found yet).
    studio: usize,
    studio_try: u64,
    studio_fields_logged: bool,
}

/// Camera transform of a live camera component.
///
/// # Safety
/// Attached tick thread; `cam` alive-checked by the caller.
unsafe fn cam_transform(
    api: &crate::il2cpp::Il2cppApi,
    cache: &UnityCache,
    cam: *mut std::ffi::c_void,
) -> Option<*mut std::ffi::c_void> {
    // SAFETY: cached getters on a live object.
    unsafe {
        let go = unity::component_gameobject(api, cache, cam)?;
        unity::go_transform(api, cache, go)
    }
}

/// `Physics.Raycast(origin, dir, out hit, maxDistance, mask, qti)`.
/// `None` = the call failed (exception); `Some(None)` = no hit.
///
/// # Safety
/// Attached tick thread; `method` is that 6-argument overload.
unsafe fn raycast(
    api: &crate::il2cpp::Il2cppApi,
    method: *mut std::ffi::c_void,
    origin: Vec3,
    dir: Vec3,
    max: f32,
) -> Option<Option<Hit>> {
    let unbox = api.object_unbox?;
    let (mut o, mut d) = (origin.to_array(), dir.to_array());
    // RaycastHit is 44 bytes: point@0, normal@12, faceID@24, distance@28.
    let mut hit = [0f32; 16];
    let (mut maxd, mut mask, mut qti) = (max, LAYER_MASK, QTI_IGNORE);
    let params = [
        &mut o as *mut [f32; 3] as *mut std::ffi::c_void,
        &mut d as *mut [f32; 3] as *mut std::ffi::c_void,
        hit.as_mut_ptr() as *mut std::ffi::c_void,
        &mut maxd as *mut f32 as *mut std::ffi::c_void,
        &mut mask as *mut i32 as *mut std::ffi::c_void,
        &mut qti as *mut i32 as *mut std::ffi::c_void,
    ];
    // SAFETY: argument types match the overload; boxed Boolean out.
    let ret = unsafe { crate::il2cpp::invoke(api, method, std::ptr::null_mut(), &params)? };
    // SAFETY: boxed value returned by the runtime.
    let p = unsafe { unbox(ret) } as *const u8;
    if p.is_null() {
        return None;
    }
    // SAFETY: non-null unboxed Boolean.
    if unsafe { std::ptr::read_unaligned(p) } == 0 {
        return Some(None);
    }
    let point = Vec3::new(hit[0], hit[1], hit[2]);
    let normal = Vec3::new(hit[3], hit[4], hit[5]);
    let dist = hit[7];
    let sane = point.is_finite()
        && normal.is_finite()
        && (normal.length() - 1.0).abs() < 0.05
        && dist.is_finite()
        && (0.0..=max + 0.01).contains(&dist);
    if sane { Some(Some(Hit { point, normal })) } else { Some(None) }
}

impl Rt {
    /// One tick while the mod is enabled.
    pub(crate) fn fly(&mut self, sh: &Mutex<Shared>, dt: f32, tel: &Mutex<Telemetry>) {
        self.tick = self.tick.saturating_add(1);
        let Some(api) = crate::il2cpp_api() else { return };
        if self.unity.is_none() {
            if self.last_try != 0 && self.tick.saturating_sub(self.last_try) < RETRY_EVERY {
                return;
            }
            self.last_try = self.tick;
            // SAFETY: attached tick thread, null-checked chain.
            self.unity = unsafe {
                crate::il2cpp::domain_checked(api).and_then(|d| unity::init(api, d))
            };
            if self.unity.is_none() {
                note(tel, "waiting for Unity");
                return;
            }
        }
        let Some(cache) = self.unity else { return };
        // SAFETY: attached tick thread; every object is alive-checked
        // before use and re-resolved from `Camera.main` each tick.
        unsafe {
            let cam = unity::main_camera(api, &cache).filter(|c| unity::object_alive(api, &cache, *c) == Some(true));
            let Some(cam) = cam else {
                note(tel, "waiting for a camera");
                return;
            };
            let shared = *lock(sh);
            let pad = gamepad::poll();
            if self.flying && self.cam != cam as usize {
                crate::log_line("fpv_drone: main camera changed (menu / scene?), suspended, game camera restored");
                self.release(api, &cache);
                self.suspended = true;
                self.armed = false;
            }
            if self.suspended {
                let b = shared.bindings.arm as usize;
                let arm_pressed = b < BUTTONS && pad.buttons[b] && !self.prev_buttons[b];
                self.prev_buttons = pad.buttons;
                if self.cam == cam as usize || arm_pressed {
                    // Back on the camera we left, or an explicit takeover
                    // of the new one: pose of the new camera, disarmed.
                    let keep = self.cam == cam as usize;
                    if !self.start(api, &cache, cam, &shared, keep) {
                        return;
                    }
                    self.suspended = false;
                    crate::log_line("fpv_drone: resumed");
                } else {
                    note(tel, "suspended — camera changed (menu?). Returns by itself, or press arm to fly this camera");
                    return;
                }
            } else if !self.flying {
                if self.last_try != 0 && self.tick.saturating_sub(self.last_try) < RETRY_EVERY {
                    return;
                }
                self.last_try = self.tick;
                if !self.start(api, &cache, cam, &shared, false) {
                    return;
                }
            }
            let Some(tr) = cam_transform(api, &cache, cam) else { return };
            self.handle_learn(sh, &shared, &pad);
            self.handle_buttons(sh, &shared, &pad);
            let sticks = read_sticks(&shared.bindings, &pad.axes);
            let p = shared.params.sanitized();

            if self.armed && pad.connected && shared.learn.is_none() {
                self.advance(api, &cache, &sticks, &p, dt, tel);
            }
            self.publish(tel, &shared, &pad, &sticks, &p);

            self.update_sound(&p);
            let Some(drone) = self.drone.as_ref() else { return };
            unity::tr_set_position(api, &cache, tr, drone.pos.to_array());
            unity::tr_set_rotation(api, &cache, tr, drone.rotation(&p));
            if (p.fov - self.last_fov).abs() > 0.01 && unity::set_fov(api, &cache, cam, p.fov) {
                self.last_fov = p.fov;
            }
        }
    }

    /// Four-rotor sound while armed; silence otherwise.
    fn update_sound(&mut self, p: &Params) {
        if !(p.sound && self.armed) {
            self.audio.stop();
            return;
        }
        let Some(motors) = self.drone.as_ref().map(|d| d.motors()) else { return };
        if !self.audio.load() {
            return;
        }
        if self.studio == 0 {
            if self.studio_try != 0 && self.tick.saturating_sub(self.studio_try) < 180 {
                return;
            }
            self.studio_try = self.tick;
            let audio = &self.audio;
            self.studio = crate::fmod_sys::discover_studio_system(
                &mut self.unity,
                &|hnd| audio.studio_valid(hnd),
                &mut self.studio_fields_logged,
            )
            .unwrap_or(0);
            if self.studio == 0 {
                return;
            }
        }
        if !self.audio.update(self.studio, &motors, p.sound_volume) {
            self.studio = 0;
        }
    }

    /// Physics step + collision.
    ///
    /// # Safety
    /// Attached tick thread.
    unsafe fn advance(
        &mut self,
        api: &crate::il2cpp::Il2cppApi,
        _cache: &UnityCache,
        sticks: &Sticks,
        p: &Params,
        dt: f32,
        _tel: &Mutex<Telemetry>,
    ) {
        let Some(drone) = self.drone.as_mut() else { return };
        let from = drone.pos;
        drone.step(sticks, p, dt);
        if !p.collisions {
            return;
        }
        // Real world, while Physics.Raycast works from this thread.
        if self.ray_strikes < RAY_STRIKES {
            if !self.ray_tried {
                self.ray_tried = true;
                // SAFETY: attached tick thread, null-checked chain.
                let m = unsafe {
                    crate::il2cpp::domain_checked(api)
                        .and_then(|d| crate::il2cpp::find_class(api, d, None, "UnityEngine", "Physics"))
                        .and_then(|k| crate::il2cpp::get_method(api, k, "Raycast", 6))
                };
                self.ray_method = m.map_or(0, |m| m as usize);
                crate::log_line(&format!(
                    "fpv_drone: Physics.Raycast {}",
                    if self.ray_method != 0 { "resolved" } else { "NOT found — flat fallback floor" }
                ));
                if self.ray_method == 0 {
                    self.ray_strikes = RAY_STRIKES;
                }
            }
            if self.ray_method != 0 {
                let delta = drone.pos - from;
                let len = delta.length();
                if len > 1e-5 {
                    // SAFETY: method resolved above, 6-arg overload.
                    let r = unsafe {
                        raycast(api, self.ray_method as *mut std::ffi::c_void, from, delta / len, len + p.radius)
                    };
                    match r {
                        Some(Some(hit)) => {
                            drone.contact(&hit, p);
                            self.ray_strikes = 0;
                        }
                        Some(None) => self.ray_strikes = 0,
                        None => {
                            self.ray_strikes += 1;
                            if self.ray_strikes >= RAY_STRIKES {
                                crate::log_line("fpv_drone: Physics.Raycast keeps failing off-thread, using the flat fallback floor");
                            }
                        }
                    }
                }
                return;
            }
        }
        // Fallback: flat floor under the takeoff point.
        if let Some((start, _)) = self.start {
            let floor = start[1] - FALLBACK_FLOOR;
            if drone.pos.y < floor + p.radius {
                drone.contact(&Hit { point: Vec3::new(drone.pos.x, floor, drone.pos.z), normal: Vec3::Y }, p);
            }
        }
    }

    /// Rebinding: capture the next moved axis / pressed button.
    fn handle_learn(&mut self, sh: &Mutex<Shared>, shared: &Shared, pad: &gamepad::GamepadState) {
        let Some(action) = shared.learn else {
            self.learn_base = None;
            return;
        };
        if !pad.connected {
            return;
        }
        let (base_axes, base_buttons, since) =
            *self.learn_base.get_or_insert((pad.axes, pad.buttons, self.tick));
        let found = if action.is_axis() {
            learn_axis(&base_axes, &pad.axes).map(|b| (Some(b), None))
        } else {
            learn_button(&base_buttons, &pad.buttons).map(|b| (None, Some(b)))
        };
        let timed_out = self.tick.saturating_sub(since) > LEARN_TIMEOUT_TICKS;
        if found.is_none() && !timed_out {
            return;
        }
        let mut s = lock(sh);
        if s.learn == Some(action) {
            match found {
                Some((Some(axis), _)) => {
                    if let Some(slot) = s.bindings.axis_mut(action) {
                        *slot = axis;
                    }
                    crate::log_line(&format!("fpv_drone: {action:?} bound to axis {} (invert {})", axis.idx, axis.invert));
                }
                Some((_, Some(btn))) => {
                    s.bindings.set_button(action, btn);
                    crate::log_line(&format!("fpv_drone: {action:?} bound to button {btn}"));
                }
                _ => {}
            }
            s.learn = None;
        }
        self.learn_base = None;
    }

    /// Arm / reset / mode buttons (rising edges).
    fn handle_buttons(&mut self, sh: &Mutex<Shared>, shared: &Shared, pad: &gamepad::GamepadState) {
        let b: &Bindings = &shared.bindings;
        let prev = self.prev_buttons;
        let pressed = |idx: u8| {
            let i = idx as usize;
            i < BUTTONS && pad.buttons[i] && !prev[i]
        };
        if shared.learn.is_none() && pad.connected {
            if pressed(b.arm) {
                self.armed = !self.armed;
                if self.armed {
                    if let Some(d) = self.drone.as_mut() {
                        d.arm();
                    }
                }
                crate::log_line(&format!("fpv_drone: {}", if self.armed { "armed" } else { "disarmed" }));
            }
            if pressed(b.reset) {
                self.reset_pose();
            }
            if pressed(b.mode) {
                let mut s = lock(sh);
                s.params.mode = if s.params.mode == Mode::Angle { Mode::Acro } else { Mode::Angle };
                crate::log_line(&format!("fpv_drone: mode {:?}", s.params.mode));
            }
        }
        self.prev_buttons = pad.buttons;
    }

    fn reset_pose(&mut self) {
        if let Some((pos, rot)) = self.start {
            self.drone = Some(Drone::from_pose(pos, rot));
            self.armed = false;
            crate::log_line("fpv_drone: reset to start, disarmed");
        }
    }

    fn publish(&self, tel: &Mutex<Telemetry>, shared: &Shared, pad: &gamepad::GamepadState, s: &Sticks, p: &Params) {
        if self.tick % 3 != 0 {
            return;
        }
        let Some(d) = self.drone.as_ref() else { return };
        let start_y = self.start.map_or(0.0, |(pos, _)| pos[1]);
        let collide = if !p.collisions {
            "off"
        } else if self.ray_strikes < RAY_STRIKES && self.ray_method != 0 {
            "world"
        } else if self.ray_tried || self.ray_strikes >= RAY_STRIKES {
            "flat floor"
        } else {
            "world (untested)"
        };
        let note = if !pad.connected {
            "no gamepad detected"
        } else if shared.learn.is_some() {
            "binding… move the control"
        } else if self.armed {
            "armed"
        } else {
            "disarmed — press the arm button"
        };
        *lock(tel) = Telemetry {
            note: note.to_string(),
            armed: self.armed,
            speed: d.vel.length(),
            alt: d.pos.y - start_y,
            throttle: d.thrust_fraction(p),
            motors: d.motors(),
            impact: d.impact,
            collide,
            sticks: [s.throttle, s.yaw, s.pitch, s.roll],
        };
    }

    /// Record the pose, switch the game's camera drivers off.
    ///
    /// # Safety
    /// Attached tick thread; `cam` alive-checked.
    unsafe fn start(
        &mut self,
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
        cam: *mut std::ffi::c_void,
        shared: &Shared,
        keep_pose: bool,
    ) -> bool {
        // SAFETY: caller contract; all reads/writes alive-checked.
        unsafe {
            let Some(tr) = cam_transform(api, cache, cam) else { return false };
            let (Some(pos), Some(rot)) = (unity::tr_position(api, cache, tr), unity::tr_get_rotation(api, cache, tr)) else {
                return false;
            };
            if !keep_pose || self.drone.is_none() {
                self.start = Some((pos, rot));
                self.drone = Some(Drone::from_pose(pos, rot));
            }
            self.armed = false;
            self.base_fov = unity::get_fov(api, cache, cam);
            self.last_fov = f32::NAN;
            // Controllers of a previous camera are gone with it.
            self.cut.clear();
            // Only the Cinemachine brain writes the camera pose; the
            // game's own CameraController is left alone unless asked
            // (switching it off is suspected of breaking LOD / the menu).
            let mut objs = unity::cm_brain_list(api, cache);
            if shared.cut_controller {
                objs.extend(unity::cam_controller_list(api, cache));
            }
            let mut names = Vec::new();
            for o in objs.into_iter().take(MAX_CUT) {
                if unity::object_alive(api, cache, o) != Some(true) {
                    continue;
                }
                if unity::beh_get_enabled(api, cache, o) == Some(true) && unity::beh_set_enabled(api, cache, o, false) {
                    self.cut.push((o as usize, true));
                    let cls = crate::il2cpp::object_class(api, o)
                        .and_then(|k| crate::il2cpp::class_name(api, k))
                        .unwrap_or_default();
                    names.push(cls);
                }
            }
            self.cam = cam as usize;
            self.flying = true;
            crate::log_line(&format!(
                "fpv_drone: takeover, {} camera drivers off [{}] (disarmed)",
                self.cut.len(),
                names.join(", ")
            ));
            true
        }
    }

    /// Give the camera back: drivers on again, FOV as found.
    fn release(&mut self, api: &crate::il2cpp::Il2cppApi, cache: &UnityCache) {
        // SAFETY: attached tick thread; each object alive-checked.
        unsafe {
            for &(o, was_on) in &self.cut {
                let o = o as *mut std::ffi::c_void;
                if unity::object_alive(api, cache, o) == Some(true) {
                    unity::beh_set_enabled(api, cache, o, was_on);
                }
            }
            // FOV only goes back on the camera it was taken from.
            if let (Some(fov), Some(cam)) = (self.base_fov, unity::main_camera(api, cache)) {
                if cam as usize == self.cam && unity::object_alive(api, cache, cam) == Some(true) {
                    unity::set_fov(api, cache, cam, fov);
                }
            }
        }
        self.cut.clear();
        self.flying = false;
        self.audio.stop();
    }

    /// Disable path: restore everything and forget the flight.
    pub(crate) fn stop(&mut self) {
        if self.flying {
            if let (Some(api), Some(cache)) = (crate::il2cpp_api(), self.unity) {
                self.release(api, &cache);
            }
        }
        self.audio.stop();
        self.flying = false;
        self.suspended = false;
        self.armed = false;
        self.drone = None;
        self.start = None;
        self.cut.clear();
        self.cam = 0;
        self.base_fov = None;
        self.last_try = 0;
        self.learn_base = None;
        self.prev_buttons = [false; BUTTONS];
    }
}

fn note(tel: &Mutex<Telemetry>, text: &str) {
    lock(tel).note = text.to_string();
}
