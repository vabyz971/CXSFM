//! Tile icons: Material Symbols glyphs painted at caller-computed
//! rects (the phone grid in `phone` draws the tiles themselves).

use crate::mods::api::TileIcon;

/// Map a tile to its Material Symbols glyph (filled style).
fn material_icon(icon: TileIcon) -> egui_material_icons::MaterialIcon {
    use egui_material_icons::icons::*;
    match icon {
        TileIcon::Camera => ICON_PHOTO_CAMERA,
        TileIcon::Scout => ICON_SEARCH,
        TileIcon::EyeOff => ICON_VISIBILITY_OFF,
        TileIcon::Tag => ICON_LABEL,
        TileIcon::Globe => ICON_PUBLIC,
        TileIcon::Info => ICON_INFO,
        TileIcon::Tree => ICON_ACCOUNT_TREE,
        TileIcon::Sliders => ICON_TUNE,
        TileIcon::Log => ICON_DESCRIPTION,
        TileIcon::Cars => ICON_DIRECTIONS_CAR,
        TileIcon::Gamepad => ICON_SPORTS_ESPORTS,
        TileIcon::Gauge => ICON_SPEED,
        TileIcon::Sound => ICON_VOLUME_UP,
        TileIcon::Power => ICON_POWER_SETTINGS_NEW,
    }
}

/// Draw an icon glyph centered at `c` with half-size `s`.
///
/// Material Symbols font (registered once via
/// `egui_material_icons::initialize`), painted as text so icons land
/// at caller-computed rects exactly like the old strokes did.
pub fn draw_icon(painter: &egui::Painter, c: egui::Pos2, s: f32, icon: TileIcon, col: egui::Color32) {
    let mi = material_icon(icon);
    let font = egui::FontId::new(s * 1.7, mi.font_family());
    painter.text(c, egui::Align2::CENTER_CENTER, mi.codepoint, font, col);
}
