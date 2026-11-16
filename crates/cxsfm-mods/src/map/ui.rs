//! Minimap render-side UI: the speedometer's shared element editor
//! ([`crate::speedometer::editor`]) wired to this mod's tick
//! state.
//!
//! Present thread only. Edits stage one-shot write requests; the tick
//! drains them ([`super::MinimapMod::drain_batch`]). Nothing touches
//! Unity directly.

use super::MinimapMod;
use crate::speedometer::editor::{Links, Texts};

impl MinimapMod {
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        let code = cxsfm_api::i18n::current().code();
        let links = Links {
            snaps: &self.snaps,
            snap_ver: &self.snap_ver,
            origs: &self.origs,
            pending_batch: &self.pending_batch,
            apply_now: &self.apply_now,
            pending: &self.pending,
            restore_now: &self.restore_now,
            save_data: &self.save_data,
            save_now: &self.save_now,
            load_now: &self.load_now,
        };
        let txt = Texts { title: super::i18n::tile_label(code), waiting: super::i18n::hint_label(code) };
        self.editor.draw(ctx, &links, &txt);
    }
}
