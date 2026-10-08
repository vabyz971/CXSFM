//! Menu UI kit: theme, tiles, window chrome and pages.
//!
//! `menu.rs` keeps only navigation state (`Page`) and window
//! orchestration; every visual lives here. Invariants (fixed `399px`
//! width, `12px` gaps, `10px` rounding, drag-anywhere) are owned by
//! these modules, not by callers.

pub mod chrome;
pub mod layout;
pub mod pages;
pub mod theme;
pub mod tiles;
