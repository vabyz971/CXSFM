//! Gamelog render-side UI: filter + error-highlighted tail.
//!
//! Present thread only (`&mut` access). Clones the tick's lines under
//! a brief lock; the tick's poll also takes it, so the whole
//! ScrollArea layout must never hold it.

use super::model::is_error;
use super::GameLogMod;
use crate::mods::common::lock;
use crate::ui::kit;

impl GameLogMod {
    /// Main window: filter + tail (render thread only).
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        // Fixed size: log lines must never resize the window.
        self.tool.show(ctx, "Game Log", egui::Vec2::new(700.0, 500.0), |ui, controls| {
                if controls {
                    ui.horizontal(|ui| {
                        kit::search(ui, &mut self.filter, "filter…");
                        ui.checkbox(&mut self.errors_only, "errors");
                    });
                }
                // Clone under a brief lock: the tick's poll() also
                // takes it, and holding it across the whole ScrollArea
                // layout would stall the tick (visible as skipped
                // frames, and a stalled tick holds the mod lock that
                // the present thread skips on).
                let lines = lock(&self.lines).clone();
                if lines.is_empty() {
                    kit::hint(ui, "No lines yet — play a bit, the game logs as it goes.");
                    return;
                }
                let query = self.filter.to_lowercase();
                kit::hint(ui, &format!("{} lines", lines.len()));
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
    }
}
