//! Gamelog render-side UI: filter + error-highlighted tail.
//!
//! Present thread only (`&mut` access). Clones the tick's lines under
//! a brief lock; the tick's poll also takes it, so the whole
//! ScrollArea layout must never hold it.

use super::model::is_error;
use super::GameLogMod;
use crate::mods::common::lock;

impl GameLogMod {
    /// Main window: filter + tail (render thread only).
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        // Fixed size: log lines must never resize the window.
        // Custom header (full-width bar + opacity options).
        let mut win = egui::Window::new("Game Log")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(700.0, 500.0))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
                self.tool.enter(ui);
                self.tool.header(ui, "Game Log");
                if !self.tool.hide_controls {
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("filter…")
                            .desired_width(f32::INFINITY),
                    );
                    ui.checkbox(&mut self.errors_only, "errors");
                });
                } // hide_controls
                // Clone under a brief lock: the tick's poll() also
                // takes it, and holding it across the whole ScrollArea
                // layout would stall the tick (visible as skipped
                // frames, and a stalled tick holds the mod lock that
                // the present thread skips on).
                let lines = lock(&self.lines).clone();
                if lines.is_empty() {
                    ui.weak("No lines yet — play a bit, the game logs as it goes.");
                    return;
                }
                let query = self.filter.to_lowercase();
                ui.label(format!("{} lines", lines.len()));
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in lines.iter() {
                            if self.errors_only && !is_error(line) {
                                continue;
                            }
                            if !query.is_empty() && !line.to_lowercase().contains(&query) {
                                continue;
                            }
                            let text = egui::RichText::new(line).monospace().size(14.0);
                            if is_error(line) {
                                ui.label(text.color(crate::ui::theme::STATE_ERR));
                            } else if line.contains("[cxsfm") {
                                ui.label(text.color(crate::ui::theme::STATE_OK));
                            } else {
                                ui.label(text.color(crate::ui::theme::LOG_TEXT));
                            }
                        }
                    });
            });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
        }
    }
}
