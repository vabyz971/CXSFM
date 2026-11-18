//! CXSFM UI kit: everything a mod draws with, so one edit restyles
//! every mod.
//!
//! - [`theme`] / [`fonts`]: CarX look (black panels, yellow accent,
//!   3px corners, Bebas Neue + Rajdhani);
//! - [`tool`] + [`kit`]: tool windows (`ToolChrome::show`) and the
//!   widgets drawn inside them;
//! - [`game`] + [`app`]: game-style rows with pad navigation and the
//!   phone apps (new-generation mods drawn inside the phone, with tabs);
//! - [`chrome`], [`tiles`], [`paramfile`], [`repaint`]: shared plumbing.
//!
//! The phone menu itself (pages, tile grid) lives in the `cxsfm` crate.

pub mod app;
pub mod chrome;
pub mod fonts;
pub mod game;
pub mod kit;
pub mod paramfile;
pub mod repaint;
pub mod theme;
pub mod tiles;
pub mod tool;
