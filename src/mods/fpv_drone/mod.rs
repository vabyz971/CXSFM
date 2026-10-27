//! FPV drone mod: a physically simulated quadcopter camera on the
//! gamepad, cosmetic only.
//!
//! A flight model (thrust, gravity, drag, wind, Angle and Acro attitude
//! modes) drives `Camera.main`. On enable the game's camera drivers
//! (Cinemachine brain + camera controllers) are switched off once; the
//! drone starts DISARMED at the camera's pose. Disable puts every driver
//! flag and the FOV back as found.
//!
//! Default pad (Xbox): left stick = throttle (Y) + roll (X), right
//! stick = pitch (Y) + yaw (X), A = arm, Y = reset, X = Angle/Acro.
//! Every control can be re-bound from the window ("Bind"), so any pad
//! layout works.
//!
//! KNOWN LIMIT: the game still reads the same pad, so the car reacts to
//! the sticks — enter photo mode first, then enable the mod. Writes
//! come from the tick thread, so a little judder is possible (see
//! docs/field-notes.md, FPV mod).
//!
//! Layout: `model` (flight math, tests), `bind` (tick side), `ui`.

pub mod i18n;

mod bind;
pub(crate) mod model;
mod sound;
mod ui;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use model::{Action, Bindings, Params};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Everything the user edits (UI ⇄ tick, one brief lock each way).
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Shared {
    pub params: Params,
    pub bindings: Bindings,
    /// Control currently waiting for the next pad movement.
    pub learn: Option<Action>,
    /// Also switch off the game's `CameraController` at takeover
    /// (default off; applies at the next takeover).
    pub cut_controller: bool,
}

/// Tick → UI.
#[derive(Clone, Default)]
pub(crate) struct Telemetry {
    pub note: String,
    pub armed: bool,
    pub speed: f32,
    pub alt: f32,
    pub throttle: f32,
    pub impact: f32,
    pub collide: &'static str,
    /// throttle, yaw, pitch, roll after the bindings.
    pub sticks: [f32; 4],
    /// Rotor speeds 0…1: FL, FR, RL, RR.
    pub motors: [f32; 4],
}

/// Mod version (shown on the phone; bump with its features).
pub const VERSION: &str = "0.5.0";

pub struct FpvDroneMod {
    enabled: AtomicBool,
    pub(crate) shared: Mutex<Shared>,
    pub(crate) tel: Mutex<Telemetry>,
    rt: Mutex<bind::Rt>,
    pub(crate) tool: ToolChrome,
}

impl FpvDroneMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            shared: Mutex::new(Shared {
                params: Params::default(),
                bindings: Bindings::default(),
                learn: None,
                cut_controller: false,
            }),
            tel: Mutex::new(Telemetry::default()),
            rt: Mutex::new(bind::Rt::default()),
            tool: ToolChrome::new(),
        }
    }
}

impl Default for FpvDroneMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for FpvDroneMod {
    fn name(&self) -> &'static str {
        "FPV Drone"
    }

    fn mod_id(&self) -> &'static str {
        "fpv_drone"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_fpv_drone"
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Gamepad
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn is_pinned(&self) -> bool {
        self.tool.pin
    }

    fn on_update(&mut self, delta_time: f32) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        lock(&self.rt).fly(&self.shared, delta_time, &self.tel);
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        self.draw_window(ctx);
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        crate::log_line("fpv_drone: armed (mod on; drone starts disarmed)");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        lock(&self.rt).stop();
        lock(&self.shared).learn = None;
        *lock(&self.tel) = Telemetry::default();
        crate::log_line("fpv_drone: off (camera drivers and FOV restored)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(FpvDroneMod::new()));
}
