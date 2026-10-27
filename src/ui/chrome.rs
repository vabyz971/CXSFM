//! Window chrome: drag-anywhere handling and shared small widgets.
//!
//! The window runs with `title_bar(false)` (no built-in drag), so this
//! module owns movement: a beneath-widgets background handle, per-tile
//! drag forwarding, and a full-window fallback for tall frames. Child
//! widgets keep priority (they consume their own press), so clicks on
//! tiles/buttons still work.

use super::theme::{ACCENT, ICON_COLOR};

/// Window position (custom drag). `None` = engine default until the
/// first drag.
static WIN_POS: std::sync::Mutex<Option<egui::Pos2>> = std::sync::Mutex::new(None);
/// Last measured window rect (anchors drags without a first-frame jump).
static WIN_RECT: std::sync::Mutex<Option<egui::Rect>> = std::sync::Mutex::new(None);

/// Lock a mutex, recovering from poisoning (a prior panic while
/// locked must never wedge the menu).
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Move the pinned window by a drag delta.
pub fn shift_window(delta: egui::Vec2) {
    if delta == egui::Vec2::ZERO {
        return;
    }
    let mut pos = lock(&WIN_POS);
    let anchor = (*pos).or_else(|| lock(&WIN_RECT).map(|r| r.min));
    if let Some(cur) = anchor {
        *pos = Some(cur + delta);
    }
}

/// Pinned position after the first drag (`None` = engine default).
pub fn pinned_pos() -> Option<egui::Pos2> {
    *lock(&WIN_POS)
}

/// Record the measured window rect (next frame's drag anchor).
pub fn note_window_rect(rect: egui::Rect) {
    *lock(&WIN_RECT) = Some(rect);
}

/// Background drag handle, created FIRST (beneath every widget):
/// dragging from ANY empty spot — header, gaps, footer margins —
/// moves the window.
pub fn background_drag(ui: &mut egui::Ui) {
    let bg_rect = ui.available_rect_before_wrap();
    let bg = ui
        .interact(
            bg_rect,
            ui.make_persistent_id("cxsfm_bg_drag"),
            egui::Sense::drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab);
    if bg.dragged() {
        let delta = bg.drag_delta();
        // Anchor without a first-frame jump: prefer the pinned pos,
        // else the last measured rect, else cursor minus press offset.
        let mut pos = lock(&WIN_POS);
        let anchor = (*pos).or_else(|| lock(&WIN_RECT).map(|r| r.min));
        let cur = anchor.or_else(|| bg.interact_pointer_pos().map(|p| p - delta));
        if let Some(cur) = cur {
            *pos = Some(cur + delta);
        }
    }
}

/// Forward a tile's drag to the window (drag started ON a widget).
/// Also used for the full-window fallback response.
pub fn forward_drag(resp: &egui::Response) {
    if resp.dragged() {
        shift_window(resp.drag_delta());
    }
}

/// Tree expander chevron: painted triangle (right = closed, down =
/// open), zero font dependency — text glyphs like `▾`/`▸` render as
/// tofu boxes when the active font lacks them, which is exactly what
/// happened in-game. Same look everywhere (inspector, speedo).
pub fn expander(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::click());
    let c = rect.center();
    let s = 4.5;
    let (p0, p1, p2) = if open {
        (
            c + egui::Vec2::new(-s, -s * 0.6),
            c + egui::Vec2::new(s, -s * 0.6),
            c + egui::Vec2::new(0.0, s * 0.8),
        )
    } else {
        (
            c + egui::Vec2::new(-s * 0.6, -s),
            c + egui::Vec2::new(s * 0.8, 0.0),
            c + egui::Vec2::new(-s * 0.6, s),
        )
    };
    let col = if resp.hovered() { ACCENT } else { ICON_COLOR };
    ui.painter().add(egui::Shape::convex_polygon(
        vec![p0, p1, p2],
        col,
        egui::Stroke::NONE,
    ));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
