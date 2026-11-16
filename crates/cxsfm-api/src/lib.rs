//! CXSFM mod API — the bottom crate every other one builds on.
//!
//! - [`Mod`] (the trait a mod implements), tile types and icons;
//! - the mod registry ([`registry`]: per-mod panic quarantine) behind
//!   the process-wide [`manager`] ([`register_mod`], [`get_mod_manager`]);
//! - framework settings and per-mod files ([`config`]), translations
//!   ([`i18n`]) and the framework log ([`log_line`]);
//! - the Steam Runtime `fmod` link shim ([`shim`]), kept here so every
//!   binary of the workspace (the `.so` and each crate's tests) links it.

pub mod common;
pub mod config;
pub mod i18n;
pub mod log;
pub mod manager;
pub mod mod_trait;
pub mod registry;

// Link-time symbol shim, no public API (see shim.rs). Must stay
// compiled in every build: referenced via `-Wl,--wrap=fmod`.
mod shim;

pub use common::lock;
pub use log::{log_line, log_path};
pub use manager::{ModManager, draw_ui_all, get_mod_manager, register_mod, update_all_mods};
pub use mod_trait::{Mod, ModTile, TileIcon};
