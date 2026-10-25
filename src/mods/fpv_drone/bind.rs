//! FPV drone tick-side work: take over the camera, fly, give it back.
//!
//! Runs on the framework tick thread. Unity traffic is bounded:
//! - takeover = ONE enumeration (Cinemachine brains + the game's camera
//!   controllers, flags recorded then disabled), repeated only when
//!   `Camera.main` changes (scene change) — never periodic, because a
//!   periodic `FindObjectsOfType` crashed the game while driving;
//! - flight = `Camera.main` + its transform, alive-checked, ~5 invokes
//!   per tick; no search, no stored object pointers across ticks
//!   except the controller flags needed for the restore (each one
//!   alive-checked before use).

use super::model::{Drone, Input, Params};
use crate::gamepad;
use crate::mods::common::lock;
use crate::unity::{self, UnityCache};
use std::sync::Mutex;

/// Retry cadence while the camera is not there yet (ticks, ~1 Hz).
const RETRY_EVERY: u64 = 60;
/// Takeover caps (a broken list must not disable the world).
const MAX_CUT: usize = 16;
/// Gamepad button (Xbox layout): A boosts. Height = triggers.
const BTN_BOOST: usize = 0;
/// A trigger counts only after it was seen released (some pads report
/// 0 instead of "rest" until first moved — a phantom half-press).
const TRIGGER_REST: f32 = 0.1;

#[derive(Default)]
pub(crate) struct Rt {
    unity: Option<UnityCache>,
    tick: u64,
    last_try: u64,
    flying: bool,
    drone: Option<Drone>,
    /// Camera we took over (address only compared, never dereferenced
    /// without `object_alive`).
    cam: usize,
    /// `(component, original enabled)` of what we switched off.
    cut: Vec<(usize, bool)>,
    base_fov: Option<f32>,
    last_fov: f32,
    lt_ready: bool,
    rt_ready: bool,
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

impl Rt {
    /// Stick/trigger state → drone controls (RT climbs, LT descends).
    fn read_input(&mut self) -> Option<Input> {
        let g = gamepad::poll();
        if !g.connected {
            return None;
        }
        self.lt_ready |= g.lt < TRIGGER_REST;
        self.rt_ready |= g.rt < TRIGGER_REST;
        let lt = if self.lt_ready { g.lt } else { 0.0 };
        let rt = if self.rt_ready { g.rt } else { 0.0 };
        // SDL stick Y is negative when pushed up/forward.
        Some(Input { fwd: -g.ly, right: g.lx, up: rt - lt, yaw: g.rx, pitch: -g.ry, boost: g.buttons[BTN_BOOST] })
    }

    /// One tick while the mod is enabled.
    pub(crate) fn fly(&mut self, p: &Params, dt: f32, note: &Mutex<String>) {
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
                *lock(note) = "waiting for Unity".into();
                return;
            }
        }
        let Some(cache) = self.unity else { return };
        // SAFETY: attached tick thread; every object is alive-checked
        // before use and re-resolved from `Camera.main` each tick.
        unsafe {
            let cam = unity::main_camera(api, &cache).filter(|c| unity::object_alive(api, &cache, *c) == Some(true));
            let Some(cam) = cam else {
                *lock(note) = "waiting for a camera".into();
                return;
            };
            if !self.flying || self.cam != cam as usize {
                if self.flying {
                    crate::log_line("fpv_drone: main camera changed, taking over again");
                }
                if self.flying && self.tick.saturating_sub(self.last_try) < RETRY_EVERY {
                    return;
                }
                self.last_try = self.tick;
                if !self.start(api, &cache, cam) {
                    return;
                }
            }
            let Some(tr) = cam_transform(api, &cache, cam) else { return };
            let input = self.read_input();
            let Some(drone) = self.drone.as_mut() else { return };
            match input {
                Some(i) => {
                    drone.step(&i, p, dt);
                    *lock(note) = "flying".into();
                }
                None => *lock(note) = "flying — no gamepad detected (holding position)".into(),
            }
            let pos = drone.pos.to_array();
            let rot = drone.rotation(p);
            unity::tr_set_position(api, &cache, tr, pos);
            unity::tr_set_rotation(api, &cache, tr, rot);
            let fov = p.sanitized().fov;
            if (fov - self.last_fov).abs() > 0.01 && unity::set_fov(api, &cache, cam, fov) {
                self.last_fov = fov;
            }
        }
    }

    /// Record the pose, switch the game's camera drivers off.
    ///
    /// # Safety
    /// Attached tick thread; `cam` alive-checked.
    unsafe fn start(&mut self, api: &crate::il2cpp::Il2cppApi, cache: &UnityCache, cam: *mut std::ffi::c_void) -> bool {
        // SAFETY: caller contract; all reads/writes alive-checked.
        unsafe {
            let Some(tr) = cam_transform(api, cache, cam) else { return false };
            let (Some(pos), Some(rot)) = (unity::tr_position(api, cache, tr), unity::tr_get_rotation(api, cache, tr)) else {
                return false;
            };
            self.drone = Some(Drone::from_pose(pos, rot));
            if !self.flying {
                self.base_fov = unity::get_fov(api, cache, cam);
            }
            self.last_fov = f32::NAN;
            // Controllers of a previous camera are gone with it.
            self.cut.clear();
            let mut objs = unity::cm_brain_list(api, cache);
            objs.extend(unity::cam_controller_list(api, cache));
            for o in objs.into_iter().take(MAX_CUT) {
                if unity::object_alive(api, cache, o) != Some(true) {
                    continue;
                }
                if unity::beh_get_enabled(api, cache, o) == Some(true) && unity::beh_set_enabled(api, cache, o, false) {
                    self.cut.push((o as usize, true));
                }
            }
            self.cam = cam as usize;
            self.flying = true;
            crate::log_line(&format!("fpv_drone: takeover, {} camera drivers off", self.cut.len()));
            true
        }
    }

    /// Give the camera back: drivers on again, FOV as found.
    pub(crate) fn stop(&mut self) {
        if self.flying {
            if let (Some(api), Some(cache)) = (crate::il2cpp_api(), self.unity) {
                // SAFETY: attached tick thread; each object alive-checked.
                unsafe {
                    for &(o, was_on) in &self.cut {
                        let o = o as *mut std::ffi::c_void;
                        if unity::object_alive(api, &cache, o) == Some(true) {
                            unity::beh_set_enabled(api, &cache, o, was_on);
                        }
                    }
                    if let (Some(fov), Some(cam)) = (self.base_fov, unity::main_camera(api, &cache)) {
                        if unity::object_alive(api, &cache, cam) == Some(true) {
                            unity::set_fov(api, &cache, cam, fov);
                        }
                    }
                }
            }
        }
        self.flying = false;
        self.drone = None;
        self.cut.clear();
        self.cam = 0;
        self.base_fov = None;
        self.last_try = 0;
    }
}
