//! Mods module - isolated game mod implementations.
//!
//! Each mod lives in its own folder and is compiled into the framework
//! `.so` (no external files). Shared pieces:
//!
//! - [`api`]: the `Mod` trait, tile types and icon vocabulary — the
//!   only surface a mod may touch.
//! - [`loader`]: the isolated registry (per-mod panic quarantine, lock
//!   discipline, disable-before-remove).
//! - [`common`]: tiny shared plumbing (poison-recovering mutex lock).
pub mod api;
pub mod common;
pub mod loader;
pub mod tool;
pub mod fpv_camera;
pub mod engine_audio;
pub mod fpv_drone;
pub mod gamelog;
pub mod custom_hud;
pub mod inspector;
pub mod map;
pub mod speedometer;
