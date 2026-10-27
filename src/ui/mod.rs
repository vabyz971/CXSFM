//! Menu UI kit: theme, fonts, phone, window chrome and pages.
//!
//! `menu.rs` keeps only navigation state (`Page`) and window
//! orchestration; every visual lives here. Invariants (phone geometry,
//! drag-anywhere) are owned by these modules, not by callers.

pub mod chrome;
pub mod fonts;
pub mod game;
pub mod paramfile;
pub mod pages;
pub mod phone;
pub mod screen;
pub mod theme;
pub mod tiles;
