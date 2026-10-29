//! Game-style fonts: Bebas Neue for titles / tabs / tile labels,
//! Rajdhani for body text, PT Serif for HUD labels that ask for a serif
//! face (all SIL OFL, see `assets/fonts`).
//!
//! `install` replaces `egui_material_icons::initialize` at the two
//! places that own a context (overlay swapchain, headless check): it
//! sets the font set once, then registers the icon font on top.
//! Fonts only ever grow the default set — Cyrillic and the other
//! default glyphs stay reachable as fallbacks.

use egui::{FontData, FontDefinitions, FontFamily, FontId};
use std::sync::Arc;

/// Family name of the uppercase condensed title font.
const TITLE: &str = "carx_title";
/// Family name of the serif font (Custom HUD labels).
const SERIF: &str = "carx_serif";

/// Font for titles, tabs and tile labels (condensed caps).
pub fn title(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(TITLE.into()))
}

/// Serif font (Custom HUD `label` widgets, `font = "serif"`).
pub fn serif(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SERIF.into()))
}

/// Set the game fonts, then the Material Symbols icon font.
pub fn install(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert(
        "bebas".to_owned(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/BebasNeue-Regular.ttf"))),
    );
    defs.font_data.insert(
        "rajdhani".to_owned(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Rajdhani-Medium.ttf"))),
    );
    defs.font_data.insert(
        "ptserif".to_owned(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/PTSerif-Regular.ttf"))),
    );
    let fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut body = vec!["rajdhani".to_owned()];
    body.extend(fallback.iter().cloned());
    let mut title = vec!["bebas".to_owned()];
    title.extend(fallback.iter().cloned());
    let mut serif = vec!["ptserif".to_owned()];
    serif.extend(fallback);
    defs.families.insert(FontFamily::Proportional, body);
    defs.families.insert(FontFamily::Name(TITLE.into()), title);
    defs.families.insert(FontFamily::Name(SERIF.into()), serif);
    ctx.set_fonts(defs);
    egui_material_icons::initialize(ctx);
}
