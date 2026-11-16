//! CXSFM bundled mods. Each one lives in its own folder (`mod.rs` =
//! the `Mod` impl, `bind.rs` = tick/Unity side, `model.rs` = pure data,
//! `ui.rs` = present side, `i18n.rs` = its strings) and is compiled into
//! the framework `.so`.
//!
//! Mods build on `cxsfm_api` (the `Mod` trait, registry, config, i18n),
//! `cxsfm_sys` (IL2CPP / Unity / FMOD), `cxsfm_input` and `cxsfm_ui`
//! (shared widgets — see `cxsfm_ui::kit`).

pub mod custom_hud;
pub mod engine_audio;
pub mod fpv_camera;
pub mod fpv_drone;
pub mod gamelog;
pub mod inspector;
pub mod map;
pub mod speedometer;

/// Register every bundled mod (all boot disarmed).
///
/// FPV Camera is NOT registered (parked until per-frame application is
/// possible); its module stays compiled for the render-hook future.
pub fn register_all() {
    inspector::register();
    gamelog::register();
    speedometer::register();
    map::register();
    custom_hud::register();
    engine_audio::register();
    fpv_drone::register();
}
