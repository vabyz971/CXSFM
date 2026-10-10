//! Window chrome: drag-anywhere handling, header and footer.
//!
//! The window runs with `title_bar(false)` (no built-in drag), so this
//! module owns movement: a beneath-widgets background handle, per-tile
//! drag forwarding, and a full-window fallback for tall frames. Child
//! widgets keep priority (they consume their own press), so clicks on
//! tiles/buttons still work.

use super::theme::{
    FOOTER_BG, FOOTER_SIZE, NAV_ROUNDING, ROUNDING, TEXT_DIM, TILE_BG, TITLE_SIZE, VERSION_SIZE,
};

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

/// Nav button (HELP / BACK): tile background with a tighter radius
/// suited to its small height (full 10px would look like a pill).
pub fn nav_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(VERSION_SIZE))
            .fill(TILE_BG)
            .corner_radius(egui::CornerRadius::same(NAV_ROUNDING as u8)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Header row, spanning the available width: version left, title
/// center, HELP right. Version and HELP share the same size; the
/// title is one step above. `on_help` runs on click (page switch).
pub fn header(ui: &mut egui::Ui, on_help: impl FnOnce()) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(env!("CARGO_PKG_VERSION"))
                .size(VERSION_SIZE)
                .color(TEXT_DIM),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if nav_button(ui, crate::i18n::t("help")).clicked() {
                on_help();
            }
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("CXSFM")
                        .size(TITLE_SIZE)
                        .strong()
                        .color(TEXT_DIM),
                );
            });
        });
    });
}

/// Run `body` in a fixed-width block centered in the available width.
///
/// The window can be resized to any width (see `menu`); centering the
/// content keeps the left/right margins symmetric instead of pushing
/// the surplus to the right.
pub fn centered_block(ui: &mut egui::Ui, w: f32, body: impl FnOnce(&mut egui::Ui)) {
    let avail = ui.available_width();
    let off = ((avail - w) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        if off > 0.0 {
            ui.add_space(off);
        }
        body(ui);
    });
}

/// Red footer bar at `w` (the grid block width): the warning wraps
/// inside instead of widening the window.
pub fn footer(ui: &mut egui::Ui, w: f32) {
    centered_block(ui, w, |ui| {
        let (rect, _) =
            ui.allocate_exact_size(egui::Vec2::new(w, 40.0), egui::Sense::hover());
        ui.painter_at(rect).rect_filled(rect, ROUNDING, FOOTER_BG);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(6.0)), |ui| {
            ui.centered_and_justified(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(crate::i18n::t("footer_locked"))
                            .size(FOOTER_SIZE)
                            .color(egui::Color32::WHITE),
                    )
                    .wrap()
                    .sense(egui::Sense::hover()),
                );
            });
        });
    });
}
