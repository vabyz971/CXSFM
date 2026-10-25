//! FPV drone mod: a free-flying camera on the gamepad, cosmetic only.
//!
//! On enable the game's camera drivers (Cinemachine brain + camera
//! controllers) are switched off once and `Camera.main` is flown from
//! the tick: left stick move (ground plane), right stick look, RT/LT up/down, A boost.
//! Disable puts every driver flag and the FOV back as found.
//!
//! KNOWN LIMIT: the game still reads the same pad, so the car reacts to
//! the sticks — enter photo mode first, then enable the mod.
//! Writes come from the tick thread (not the render loop), so a little
//! judder is possible (see docs/field-notes.md, FPV mod).
//!
//! Layout: `model` (flight math, tests), `bind` (tick side), `ui`.

pub mod i18n;

mod bind;
pub(crate) mod model;
mod ui;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use model::Params;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct FpvDroneMod {
    enabled: AtomicBool,
    /// UI → tick.
    pub(crate) shared: Mutex<Params>,
    /// Tick → UI.
    pub(crate) note: Mutex<String>,
    rt: Mutex<bind::Rt>,
    pub(crate) tool: ToolChrome,
    /// Render-side edit buffer.
    pub(crate) edit: Params,
}

impl FpvDroneMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            shared: Mutex::new(Params::default()),
            note: Mutex::new(String::new()),
            rt: Mutex::new(bind::Rt::default()),
            tool: ToolChrome::new(),
            edit: Params::default(),
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
        // Bind-then-use: params copied before the runtime lock.
        let p = *lock(&self.shared);
        lock(&self.rt).fly(&p, delta_time, &self.note);
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        self.draw_window(ctx);
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        crate::log_line("fpv_drone: armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        lock(&self.rt).stop();
        lock(&self.note).clear();
        crate::log_line("fpv_drone: off (camera drivers and FOV restored)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(FpvDroneMod::new()));
}
