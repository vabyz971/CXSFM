//! Engine-sound mod: pitch / volume (and experimental parameter
//! overrides) on the FMOD engine events, cosmetic audio only.
//!
//! The game plays its engines through FMOD Studio (Unity integration).
//! We reach the Studio system through `FMODUnity.RuntimeManager`, list
//! the loaded events, and scale the *user* pitch/volume of the live
//! instances whose path matches the filter (default `engine`). The
//! game keeps owning rpm/load parameters; originals are recorded and
//! restored on neutral / disable.
//!
//! Layout mirrors every other mod: `fmod` (C API), `model` (pure),
//! `bind` (tick side), `ui` (render side).

pub mod i18n;

mod bind;
mod fmod;
pub(crate) mod model;
mod ui;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use model::{Settings, Status};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Mod version (shown on the phone; bump with its features).
pub const VERSION: &str = "0.2.0";

pub struct EngineAudioMod {
    enabled: AtomicBool,
    /// UI → tick (brief clone/store, never held across work).
    pub(crate) shared: Mutex<Settings>,
    /// Tick → UI.
    pub(crate) status: Mutex<Status>,
    rt: Mutex<bind::Runtime>,
    /// Pad focus of the rows panel.
    pub(crate) nav: crate::ui::game::NavState,
    /// Render-side edit buffer (pushed to `shared` on change).
    pub(crate) edit: Settings,
}

impl EngineAudioMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            shared: Mutex::new(Settings::default()),
            status: Mutex::new(Status::default()),
            rt: Mutex::new(bind::Runtime::default()),
            nav: Default::default(),
            edit: Settings::default(),
        }
    }
}

impl Default for EngineAudioMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for EngineAudioMod {
    fn name(&self) -> &'static str {
        "Engine Sound"
    }

    fn mod_id(&self) -> &'static str {
        "engine_audio"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_engine_audio"
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Sound
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        // Bind-then-use: settings cloned before the runtime lock.
        let s = lock(&self.shared).clone();
        lock(&self.rt).tick(&s, &self.status);
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        self.draw_window(ctx);
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        crate::log_line("engine_audio: armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        lock(&self.rt).restore();
        *lock(&self.status) = Status::default();
        crate::ui::screen::close();
        crate::log_line("engine_audio: off (pitch/volume restored)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::ui::screen::declare("Engine Sound");
    crate::mod_api::register_mod(Box::new(EngineAudioMod::new()));
}
