//! Tile widgets: vector icons + explicit-rect clickable tiles.
//!
//! Tiles are placed at caller-computed rects (not layout flow) so gaps
//! are exactly [`crate::ui::theme::GRID_GAP`] whatever the window does.

use super::theme::{ACCENT, ICON_COLOR, ROUNDING, TEXT_DIM, TILE_ACTIVE_BG, TILE_BG, TILE_HOVER};
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

/// One clickable tile at an explicit `rect`: icon up top, label
/// underneath. Inner geometry scales with the rect so normal and
/// compact tiles share the same look. Returns the raw response
/// (`click_and_drag`: a press-drag moves the window via
/// [`crate::ui::chrome::forward_drag`], a clean press-release clicks).
pub fn tile_at(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    icon: TileIcon,
    label: &str,
    active: bool,
    label_size: f32,
) -> egui::Response {
    let resp = ui
        .interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(rect);

    let bg = if resp.hovered() {
        TILE_HOVER
    } else if active {
        TILE_ACTIVE_BG
    } else {
        TILE_BG
    };
    painter.rect_filled(rect, ROUNDING, bg);
    if active {
        // Outline INSIDE the fill (shrink by half the 2px stroke):
        // outer edge == fill edge, so the visible corner radius
        // matches inactive tiles. Yellow = CarX active state.
        let inner = rect.shrink(1.0);
        painter.rect_stroke(inner, ROUNDING - 1.0, egui::Stroke::new(2.0, ACCENT), egui::StrokeKind::Middle);
    }

    // Icon up top, label zone below sized for two small lines.
    // Ratios of the 125x110 reference tile, scaled to any rect.
    // Active tiles get yellow icons like game highlights.
    let icon_c = rect.center() + egui::Vec2::new(0.0, -rect.height() * 0.145);
    draw_icon(&painter, icon_c, rect.width() * 0.144, icon, if active { ACCENT } else { ICON_COLOR });
    let label_rect = egui::Rect::from_min_max(
        egui::Pos2::new(rect.min.x + 4.0, rect.max.y - rect.height() * 0.4),
        egui::Pos2::new(rect.max.x - 4.0, rect.max.y - 4.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(label_rect), |ui| {
        ui.centered_and_justified(|ui| {
            // Hover-only: clicks fall through to the tile below, and
            // the text can never start a selection drag.
            ui.add(
                egui::Label::new(egui::RichText::new(label).size(label_size).color(
                    if active {
                        egui::Color32::WHITE
                    } else {
                        TEXT_DIM
                    },
                ))
                .wrap()
                .sense(egui::Sense::hover()),
            );
        });
    });
    resp
}
