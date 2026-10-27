//! Mod menu window: navigation state + orchestration.
//!
//! One window replaces the old manager/status pair: the in-game phone
//! (see `ui::phone`) with one icon tile per registered mod, then
//! Settings and About. All strings go through `i18n`.
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
    if PAGE.swap(p.encode(), Ordering::SeqCst) != p.encode() {
        kick();
    }
}

/// Frames still to repaint after a layout switch (page / screen).
static REPAINT_BURST: AtomicU8 = AtomicU8::new(0);

/// Ask for a few extra repaints. egui lays a new page out over more
/// than one pass, and the overlay only re-runs on input or on a
/// repaint request: without this the first frames after a switch stay
/// half-drawn until the mouse moves.
pub(crate) fn kick() {
    REPAINT_BURST.store(4, Ordering::SeqCst);
}

fn pump(ctx: &egui::Context) {
    if REPAINT_BURST.load(Ordering::SeqCst) > 0 {
        REPAINT_BURST.fetch_sub(1, Ordering::SeqCst);
        ctx.request_repaint();
    }
}

fn page() -> Page {
    Page::decode(PAGE.load(Ordering::SeqCst))
}

/// Draw the whole menu (called from the render hook when visible).
pub fn draw_menu_ui(ctx: &egui::Context) {
    draw_menu_inner(ctx);
    pump(ctx);
}

fn draw_menu_inner(ctx: &egui::Context) {
    if !crate::hotkey::ui_visible() {
        return;
    }
    // Dark menu palette, scoped to this window. Text is never
    // selectable (labels would otherwise eat drags/clicks and start
    // selection marquees over the tiles). egui 0.36 themes styles per
    // Theme: mutate the active one — once per theme switch, not every
    // frame (style mutation is wasteful in immediate mode).
    static STYLED: AtomicU8 = AtomicU8::new(u8::MAX);
    let theme = ctx.theme();
    let code = match theme {
        egui::Theme::Dark => 0,
        egui::Theme::Light => 1,
    };
    if STYLED.swap(code, Ordering::SeqCst) != code {
        ctx.style_mut_of(theme, |style| {
            // Game overlay is always dark, whatever the ambient theme is.
            style.visuals.dark_mode = true;
            style.visuals.window_fill = WIN_BG;
            style.visuals.window_corner_radius = ROUNDING.into();
            style.interaction.selectable_labels = false;
        });
    }

    // The phone: fixed size scaled to the screen height, drag from
    // anywhere (background handle + tile forwarding). Frameless — the
    // bezel is painted by `ui::phone`.
    // A full-screen mod screen replaces the phone (and its key hints,
    // the screen has its own footer) until BACK.
    if crate::ui::screen::any_open() {
        return;
    }
    let sc = crate::ui::phone::scale(ctx);
    let mut win = egui::Window::new("cxsfm_menu")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(egui::Frame::NONE)
        .fixed_size(crate::ui::phone::size(sc))
        .default_pos(egui::pos2(56.0 * sc, 48.0 * sc));
    // Pinned position after the first drag.
    if let Some(p) = chrome::pinned_pos() {
        win = win.current_pos(p);
    }
    let inner = win.show(ctx, |ui| {
        chrome::background_drag(ui);
        let (phone, _) = ui.allocate_exact_size(crate::ui::phone::size(sc), egui::Sense::hover());
        crate::ui::phone::draw(ui, phone, sc, page());
    });
    if let Some(r) = inner {
        chrome::forward_drag(&r.response);
        chrome::note_window_rect(r.response.rect);
    }
    // Game-style key hints, bottom-left of the screen.
    egui::Area::new(egui::Id::new("cxsfm_key_hints"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(24.0, -24.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                crate::ui::game::key_hint(ui, "F8", "close");
                if crate::padnav::active() {
                    crate::ui::game::key_hint(ui, "A", "select");
                    crate::ui::game::key_hint(ui, "X", "open window");
                    crate::ui::game::key_hint(ui, "B", "back");
                    crate::ui::game::key_hint(ui, "L3+R3", "leave");
                }
            });
        });
}
