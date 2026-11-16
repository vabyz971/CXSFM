//! Repaint bursts after a layout switch (menu page, full-screen screen).

use std::sync::atomic::{AtomicU8, Ordering};

/// Frames still to repaint after a layout switch (page / screen).
static REPAINT_BURST: AtomicU8 = AtomicU8::new(0);

/// Ask for a few extra repaints. egui lays a new page out over more
/// than one pass, and the overlay only re-runs on input or on a
/// repaint request: without this the first frames after a switch stay
/// half-drawn until the mouse moves.
pub fn kick() {
    REPAINT_BURST.store(4, Ordering::SeqCst);
}

/// Spend one burst frame (call once per pass, from the menu).
pub fn pump(ctx: &egui::Context) {
    if REPAINT_BURST.load(Ordering::SeqCst) > 0 {
        REPAINT_BURST.fetch_sub(1, Ordering::SeqCst);
        ctx.request_repaint();
    }
}
