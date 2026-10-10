//! Shared tool-window chrome: pin + opacity, one pattern for every
//! tool window (inspector, game log, video …).
//!
//! - **pin**: the window stays visible when the menu closes (view-only:
//!   input capture follows the menu, so a pinned window never freezes
//!   game input — reopen the menu to interact with it).
//! - **background**: per-window fill alpha (menu style untouched).
//! - **content**: whole-content opacity, scoped to the window's `Ui`.
//!
//! Both live in a right-click menu on the window (title bar included)
//! plus a pin checkbox in the content top bar.

/// Pin + opacity state, one per tool mod.
pub struct ToolChrome {
    /// Keep visible when the menu closes.
    pub pin: bool,
    /// Window background alpha (40..=255).
    pub bg: u8,
    /// Whole-content opacity (0.25..=1.0).
    pub opacity: f32,
}

impl ToolChrome {
    pub fn new() -> Self {
        Self {
            pin: false,
            bg: 235,
            opacity: 1.0,
        }
    }

    /// Per-window frame with background alpha (menu style untouched).
    pub fn frame(&self, ctx: &egui::Context) -> egui::Frame {
        let mut f = egui::Frame::window(&ctx.style_of(ctx.theme()));
        let c = f.fill;
        f.fill = egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), self.bg);
        f
    }

    /// Apply content opacity (call first inside the window closure).
    pub fn enter(&self, ui: &mut egui::Ui) {
        ui.set_opacity(self.opacity);
    }

    /// Pin checkbox for the content top bar.
    pub fn pin_toggle(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.pin, "pin");
    }

    /// Right-click menu on the window (title bar or background):
    /// pin + background alpha + content opacity.
    pub fn context_menu(&mut self, resp: &egui::Response) {
        resp.context_menu(|ui| {
            ui.checkbox(&mut self.pin, "keep visible (menu closed)");
            ui.add(egui::Slider::new(&mut self.bg, 40..=255).text("background"));
            ui.add(egui::Slider::new(&mut self.opacity, 0.25..=1.0).text("content"));
        });
    }
}

impl Default for ToolChrome {
    fn default() -> Self {
        Self::new()
    }
}
