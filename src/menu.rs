//! Mod menu window: navigation state + orchestration.
//!
//! One window replaces the old manager/status pair: a header (version,
//! title, HELP), a grid of icon tiles (one per registered mod, then
//! Settings and About), and a red footer warning that game controls
//! are locked while the menu is up. All strings go through `i18n`.
//!
//! Visuals live in [`crate::ui`] (theme, tiles, chrome, pages); this
//! module owns only the current [`Page`] and the window itself.

use crate::ui::chrome;
use crate::ui::theme::{ROUNDING, WIN_BG};
use std::sync::atomic::{AtomicU8, Ordering};

/// Icon vocabulary, owned by the mod API; re-exported here so existing
/// `menu::TileIcon` paths keep working.
pub use crate::mods::api::TileIcon;

/// Menu page (single window, swaps content).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Main,
    Settings,
    About,
}

impl Page {
    fn encode(self) -> u8 {
        match self {
            Page::Main => 0,
            Page::Settings => 1,
            Page::About => 2,
        }
    }

    fn decode(v: u8) -> Page {
        match v {
            1 => Page::Settings,
            2 => Page::About,
            _ => Page::Main,
        }
    }
}

static PAGE: AtomicU8 = AtomicU8::new(0);

pub(crate) fn set_page(p: Page) {
    PAGE.store(p.encode(), Ordering::SeqCst);
}

fn page() -> Page {
    Page::decode(PAGE.load(Ordering::SeqCst))
}

/// Draw the whole menu (called from the render hook when visible).
pub fn draw_menu_ui(ctx: &egui::Context) {
    if !crate::hotkey::ui_visible() {
        return;
    }
    // Dark menu palette, scoped to this window. Text is never
    // selectable (labels would otherwise eat drags/clicks and start
    // selection marquees over the tiles). egui 0.36 themes styles per
    // Theme: mutate the active one (was global `set_style`).
    let theme = ctx.theme();
    ctx.style_mut_of(theme, |style| {
        style.visuals.window_fill = WIN_BG;
        style.visuals.window_corner_radius = ROUNDING.into();
        style.interaction.selectable_labels = false;
    });

    // Resizable + responsive: the grid derives its columns from the
    // available width (up to 4, wrapping below; compact past 8 mods)
    // and every block is centered, so margins stay symmetric at any
    // size. Drag still works from anywhere (background handle, tiles,
    // window fallback); the corner resize handle keeps priority.
    let mut win = egui::Window::new("cxsfm_menu")
        .title_bar(false)
        .resizable(true)
        .min_size(egui::Vec2::new(300.0, 240.0))
        .collapsible(false)
        .default_pos(egui::pos2(160.0, 120.0));
    // Pinned position after the first drag.
    if let Some(p) = chrome::pinned_pos() {
        win = win.current_pos(p);
    }
    let inner = win.show(ctx, |ui| {
        chrome::background_drag(ui);
        chrome::header(ui, || set_page(Page::About));
        ui.separator();
        // One layout per frame: pages and footer share the same block
        // width so grid and bars stay aligned.
        let layout = crate::ui::pages::content_layout(ui);
        match page() {
            Page::Main => crate::ui::pages::page_main(ui, &layout),
            Page::Settings => crate::ui::pages::page_settings(ui, layout.block_w),
            Page::About => crate::ui::pages::page_about(ui, layout.block_w),
        }
        ui.separator();
        chrome::footer(ui, layout.block_w);
    });
    // Full-window drag fallback: the pre-content handle covers header
    // / gaps / footer margins, tiles forward their own drags, and the
    // window response covers any remaining background — even on tall
    // windows where the pre-wrap rect no longer spans the frame.
    if let Some(r) = inner {
        chrome::forward_drag(&r.response);
        chrome::note_window_rect(r.response.rect);
    }
}
