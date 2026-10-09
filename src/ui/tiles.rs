//! Tile widgets: vector icons + explicit-rect clickable tiles.
//!
//! Tiles are placed at caller-computed rects (not layout flow) so gaps
//! are exactly [`crate::ui::theme::GRID_GAP`] whatever the window does.

use super::theme::{ACCENT, ICON_COLOR, ROUNDING, TEXT_DIM, TILE_ACTIVE_BG, TILE_BG, TILE_HOVER};
use crate::mods::api::TileIcon;

/// Draw a vector icon centered at `c` with half-size `s`.
///
/// Pure `Painter` strokes (no font glyphs, no assets) so icons render
/// identically on every setup.
pub fn draw_icon(painter: &egui::Painter, c: egui::Pos2, s: f32, icon: TileIcon) {
    let col = ICON_COLOR;
    let w = egui::Stroke::new(2.5, col);
    let thin = egui::Stroke::new(2.0, col);
    match icon {
        TileIcon::Camera => {
            // Body.
            let body = egui::Rect::from_center_size(
                c + egui::Vec2::new(0.0, s * 0.12),
                egui::Vec2::new(s * 1.5, s * 0.95),
            );
            painter.rect_stroke(body, 4.0, w);
            // Viewfinder bump.
            let bump = egui::Rect::from_center_size(
                c + egui::Vec2::new(-s * 0.42, -s * 0.48),
                egui::Vec2::new(s * 0.4, s * 0.25),
            );
            painter.rect_stroke(bump, 2.0, thin);
            // Lens.
            painter.circle_stroke(c + egui::Vec2::new(0.0, s * 0.12), s * 0.3, w);
        }
        TileIcon::Scout => {
            // Magnifier glass.
            let g = c + egui::Vec2::new(-s * 0.18, -s * 0.18);
            painter.circle_stroke(g, s * 0.42, w);
            // Handle.
            painter.line_segment(
                [
                    g + egui::Vec2::new(s * 0.3, s * 0.3),
                    c + egui::Vec2::new(s * 0.62, s * 0.62),
                ],
                w,
            );
        }
        TileIcon::EyeOff => {
            // Iris.
            painter.circle_filled(c, s * 0.14, col);
            // Lash hints.
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.5, -s * 0.42),
                    c + egui::Vec2::new(s * 0.5, -s * 0.42),
                ],
                thin,
            );
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.5, s * 0.42),
                    c + egui::Vec2::new(s * 0.5, s * 0.42),
                ],
                thin,
            );
            // Slash.
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.62, s * 0.62),
                    c + egui::Vec2::new(s * 0.62, -s * 0.62),
                ],
                w,
            );
        }
        TileIcon::Tag => {
            // Diamond tag.
            let p0 = c + egui::Vec2::new(-s * 0.1, -s * 0.55);
            let p1 = c + egui::Vec2::new(s * 0.45, 0.0);
            let p2 = c + egui::Vec2::new(-s * 0.1, s * 0.55);
            let p3 = c + egui::Vec2::new(-s * 0.55, 0.0);
            for (a, b) in [(p0, p1), (p1, p2), (p2, p3), (p3, p0)] {
                painter.line_segment([a, b], w);
            }
            // Hole.
            painter.circle_stroke(c + egui::Vec2::new(-s * 0.14, 0.0), s * 0.09, thin);
        }
        TileIcon::Globe => {
            painter.circle_stroke(c, s * 0.5, w);
            // Equator + meridian.
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.5, 0.0),
                    c + egui::Vec2::new(s * 0.5, 0.0),
                ],
                thin,
            );
            painter.line_segment(
                [
                    c + egui::Vec2::new(0.0, -s * 0.5),
                    c + egui::Vec2::new(0.0, s * 0.5),
                ],
                thin,
            );
        }
        TileIcon::Info => {
            painter.circle_stroke(c, s * 0.5, w);
            // Stem of the "i".
            painter.line_segment(
                [
                    c + egui::Vec2::new(0.0, -s * 0.05),
                    c + egui::Vec2::new(0.0, s * 0.35),
                ],
                w,
            );
            // Dot.
            painter.circle_filled(c + egui::Vec2::new(0.0, -s * 0.25), 2.5, col);
        }
        TileIcon::Tree => {
            // Trunk + three branches ending in dots (scene hierarchy).
            let x0 = c.x - s * 0.4;
            painter.line_segment(
                [
                    egui::pos2(x0, c.y - s * 0.5),
                    egui::pos2(x0, c.y + s * 0.5),
                ],
                w,
            );
            for (i, dy) in [-0.32, 0.0, 0.32].iter().enumerate() {
                let y = c.y + s * dy;
                let x1 = x0 + s * (0.35 + 0.15 * (i as f32));
                painter.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], thin);
                painter.circle_filled(egui::pos2(x1 + s * 0.12, y), 2.5, col);
            }
        }
        TileIcon::Sliders => {
            // Three rails with knobs (settings).
            for (i, dy) in [-0.35, 0.0, 0.35].iter().enumerate() {
                let y = c.y + s * dy;
                painter.line_segment(
                    [
                        egui::pos2(c.x - s * 0.5, y),
                        egui::pos2(c.x + s * 0.5, y),
                    ],
                    thin,
                );
                let kx = c.x + s * [-0.2, 0.15, -0.05][i];
                painter.circle_filled(egui::pos2(kx, y), s * 0.12, col);
            }
        }
        TileIcon::Log => {
            // Document lines with a folded corner.
            let wdt = s * 0.7;
            let hgt = s * 0.9;
            let r = egui::Rect::from_center_size(c, egui::Vec2::new(wdt, hgt));
            painter.rect_stroke(r, 2.0, w);
            for (i, dy) in [-0.22, 0.02, 0.26].iter().enumerate() {
                let y = c.y + s * dy;
                let x1 = c.x + s * (0.22 - 0.06 * (i as f32));
                painter.line_segment(
                    [egui::pos2(c.x - s * 0.22, y), egui::pos2(x1, y)],
                    thin,
                );
            }
        }
        TileIcon::Gauge => {
            // Semicircular dial: tick marks, arc segments, needle + hub.
            let r = s * 0.48;
            let n_ticks = 7;
            for i in 0..n_ticks {
                let a = std::f32::consts::PI * (1.0 - i as f32 / (n_ticks - 1) as f32);
                let (outer, inner) = (r, r - s * 0.1);
                painter.line_segment(
                    [
                        egui::pos2(c.x + outer * a.cos(), c.y - outer * a.sin()),
                        egui::pos2(c.x + inner * a.cos(), c.y - inner * a.sin()),
                    ],
                    thin,
                );
            }
            let n_arc = 16;
            let mut prev = egui::pos2(c.x + r, c.y);
            for i in 1..=n_arc {
                let a = std::f32::consts::PI * (1.0 - i as f32 / n_arc as f32);
                let p = egui::pos2(c.x + r * a.cos(), c.y - r * a.sin());
                painter.line_segment([prev, p], w);
                prev = p;
            }
            let na = std::f32::consts::PI * 0.72;
            painter.line_segment(
                [c, egui::pos2(c.x + r * 0.8 * na.cos(), c.y - r * 0.8 * na.sin())],
                w,
            );
            painter.circle_filled(c, 2.5, col);
        }
    }
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
        // outer edge == fill edge, so the visible corner radius is
        // exactly 10px like every inactive tile.
        let inner = rect.shrink(1.0);
        painter.rect_stroke(inner, ROUNDING - 1.0, egui::Stroke::new(2.0, ACCENT));
    }

    // Icon up top, label zone below sized for two small lines.
    // Ratios of the 125x110 reference tile, scaled to any rect.
    let icon_c = rect.center() + egui::Vec2::new(0.0, -rect.height() * 0.145);
    draw_icon(&painter, icon_c, rect.width() * 0.144, icon);
    let label_rect = egui::Rect::from_min_max(
        egui::Pos2::new(rect.min.x + 4.0, rect.max.y - rect.height() * 0.4),
        egui::Pos2::new(rect.max.x - 4.0, rect.max.y - 4.0),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(label_rect), |ui| {
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
