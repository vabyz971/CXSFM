//! Shared tool-window chrome: pin + opacity, one pattern for every
//! tool window (inspector, game log, speedo …).
//!
//! Windows run with `title_bar(false)` so the bar spans the full frame
//! (the native bar rendered short with custom frames). [`ToolChrome`]
//! draws the replacement header itself: title + pin + options gear,
//! draggable from anywhere on the row.
//!
//! - **pin**: the window stays visible when the menu closes (view-only:
//!   input capture follows the menu, so a pinned window never freezes
//!   game input — reopen the menu to interact with it).
//! - **background**: per-window fill alpha (menu style untouched).
//! - **content**: whole-content opacity, scoped to the window's `Ui`.
//! - **no_titlebar**: hides the header for a clean in-race overlay
//!   (a slim restore strip stays: the window never strands).
//!
//! - **no_titlebar / hide_controls**: frameless overlay and compact
//!   mode (top control rows hidden), toggled from the header or the
//!   right-click menu — the window never strands (restore strip).
//!
//! Display options live in the header's gear row (always reachable)
//! and are mirrored in the right-click menu.

/// Pin + opacity state, one per tool mod.
pub struct ToolChrome {
    /// Keep visible when the menu closes.
    pub pin: bool,
    /// Window background alpha (40..=255).
    pub bg: u8,
    /// Whole-content opacity (0.25..=1.0).
    pub opacity: f32,
    /// Hide the title bar (frameless in-race overlay).
    pub no_titlebar: bool,
    /// Hide the top control rows (buttons/search/groups).
    pub hide_controls: bool,
    /// Show the display-options row (UI-only).
    pub show_opts: bool,
    /// Manual window position (`title_bar(false)` can't drag natively).
    pub pos: Option<egui::Pos2>,
}

impl ToolChrome {
    pub fn new() -> Self {
        Self {
            pin: false,
            bg: 235,
            opacity: 1.0,
            no_titlebar: false,
            hide_controls: false,
            show_opts: false,
            pos: None,
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

    /// Custom title bar (windows run `title_bar(false)`): title left,
    /// pin + gear right, draggable from the row. Gear opens the
    /// display row (background/content opacity, hide-bar toggle).
    /// Hidden mode keeps a slim restore strip so the window can
    /// always get its bar back — and the strip drags too.
    pub fn header(&mut self, ui: &mut egui::Ui, title: &str) {
        if self.no_titlebar {
            let r = ui
                .horizontal(|ui| {
                    if ui
                        .small_button("▤")
                        .on_hover_text("afficher la barre de titre")
                        .clicked()
                    {
                        self.no_titlebar = false;
                    }
                })
                .response;
            self.drag_by(&r);
            return;
        }
        let r = ui
            .horizontal(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .strong()
                        .size(15.0)
                        .color(egui::Color32::WHITE),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .small_button("⚙")
                            .on_hover_text("affichage")
                            .clicked()
                        {
                            self.show_opts = !self.show_opts;
                        }
                        self.pin_toggle(ui);
                        // Show/hide the window's top control rows
                        // (buttons, search, groups) — compact overlay.
                        let (glyph, tip) = if self.hide_controls {
                            ("▸", "afficher les contrôles")
                        } else {
                            ("▾", "masquer les contrôles")
                        };
                        if ui.small_button(glyph).on_hover_text(tip).clicked() {
                            self.hide_controls = !self.hide_controls;
                        }
                    },
                );
            })
            .response;
        self.drag_by(&r);
        if self.show_opts {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("fond").weak().small());
                ui.add(egui::Slider::new(&mut self.bg, 40..=255));
                ui.label(egui::RichText::new("contenu").weak().small());
                ui.add(egui::Slider::new(&mut self.opacity, 0.25..=1.0));
            });
            ui.checkbox(&mut self.no_titlebar, "masquer la barre (overlay)");
        }
    }

    /// Shift the manual position by a header drag delta.
    fn drag_by(&mut self, r: &egui::Response) {
        if r.dragged() {
            let d = r.drag_delta();
            if d != egui::Vec2::ZERO {
                self.pos = Some(self.pos.unwrap_or(r.rect.min) + d);
            }
        }
    }

    /// Right-click menu on the window (title bar or background):
    /// pin + background alpha + content opacity + control rows.
    /// Also logs (once) when the popup actually opens — the in-game
    /// proof that secondary-clicks reach egui through the X11 pump.
    pub fn context_menu(&mut self, resp: &egui::Response) {
        static LOGGED: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(false);
        if resp.context_menu_opened()
            && !LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            crate::log_line("tool: context menu opened (right-click OK)");
        }
        resp.context_menu(|ui| {
            ui.checkbox(&mut self.pin, "keep visible (menu closed)");
            ui.checkbox(&mut self.hide_controls, "hide top controls");
            ui.checkbox(&mut self.no_titlebar, "hide title bar (overlay)");
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
