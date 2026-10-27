//! Full-screen "Settings" frame for mods that opt in (see [`declare`]):
//! dimmed backdrop, big title top-left, tab strip, a rows panel on the
//! left and a help panel on the right, key-hint footer — the layout of
//! the game's own settings screens.
//!
//! A screen is opened from the phone tile ([`open`]); only one is open
//! at a time and the phone hides while it is. BACK (footer chip or pad
//! B) closes it and returns to the phone; the mod itself stays enabled
//! (its power button, top right, turns it off).

use super::fonts;
use super::theme::{ACCENT, TEXT_DIM};
use crate::padnav;
use egui::{Align2, Color32, Pos2, Rect, Sense, Ui, Vec2};
use std::sync::Mutex;

/// Mods that live in a full-screen frame (names, as registered).
static APPS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
/// Name of the open screen, if any.
static OPEN: Mutex<Option<String>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Declare that mod `name` draws a full-screen frame (call from its
/// `register()`): its phone tile opens the screen instead of toggling.
pub fn declare(name: &'static str) {
    let mut apps = lock(&APPS);
    if !apps.contains(&name) {
        apps.push(name);
    }
}

pub fn is_app(name: &str) -> bool {
    lock(&APPS).iter().any(|n| *n == name)
}

pub fn open(name: &str) {
    *lock(&OPEN) = Some(name.to_string());
    crate::menu::kick();
}

pub fn close() {
    if lock(&OPEN).take().is_some() {
        crate::menu::kick();
    }
}

pub fn is_open(name: &str) -> bool {
    lock(&OPEN).as_deref() == Some(name)
}

pub fn any_open() -> bool {
    lock(&OPEN).is_some()
}

/// Pad-focus id of a mod window: its name, snake-cased.
pub fn pad_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

/// Where the body draws.
pub struct Regions {
    pub tabs: Rect,
    pub left: Rect,
    pub right: Rect,
}

/// What the user asked for this frame.
#[derive(Default, Clone, Copy)]
pub struct Exit {
    /// Power button: turn the mod off (screen closes).
    pub power_off: bool,
}

/// Run `body` inside `rect` as its own child `Ui`.
pub fn region(ui: &mut Ui, rect: Rect, body: impl FnOnce(&mut Ui)) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), body);
}

/// Footer chip (white key box + label); returns its rect.
fn chip(ui: &Ui, at: Pos2, key: &str, label: &str) -> Rect {
    let p = ui.painter();
    let kg = p.layout_no_wrap(key.to_string(), fonts::title(22.0), Color32::BLACK);
    let lg = p.layout_no_wrap(label.to_uppercase(), fonts::title(22.0), Color32::WHITE);
    let (kw, lw, h) = (kg.size().x + 14.0, lg.size().x, 34.0);
    let r = Rect::from_min_size(at, Vec2::new(kw + lw + 24.0, h));
    p.rect_filled(r, 2.0, Color32::from_black_alpha(150));
    let kr = Rect::from_min_size(at + Vec2::new(5.0, 5.0), Vec2::new(kw, h - 10.0));
    p.rect_filled(kr, 2.0, Color32::WHITE);
    p.galley(Pos2::new(kr.center().x - kg.size().x / 2.0, kr.center().y - kg.size().y / 2.0), kg, Color32::BLACK);
    p.galley(Pos2::new(kr.right() + 8.0, r.center().y - lg.size().y / 2.0), lg, Color32::WHITE);
    r
}

/// Draw the screen of mod `name` and run `body` for its panels.
///
/// `hints` are the extra footer chips (after BACK).
pub fn show(
    ctx: &egui::Context,
    name: &str,
    title: &str,
    powered: bool,
    hints: &[(&str, &str)],
    body: impl FnOnce(&mut Ui, &Regions),
) -> Exit {
    let r = ctx.content_rect();
    let (w, h) = (r.width(), r.height());
    let mut exit = Exit::default();
    let owner = pad_id(name);
    let mut back = padnav::has_focus(&owner) && padnav::events() & padnav::BACK != 0;
    egui::Area::new(egui::Id::new(("cxsfm_screen", name)))
        .order(egui::Order::Foreground)
        .fixed_pos(r.min)
        .show(ctx, |ui| {
            // Swallow every pointer event so nothing leaks to the game
            // windows below.
            ui.allocate_rect(r, Sense::click_and_drag());
            let p = ui.painter().clone();
            p.rect_filled(r, 0.0, Color32::from_black_alpha(205));
            let left = r.left() + w * 0.05;
            let tg = p.layout_no_wrap(title.to_uppercase(), fonts::title(54.0), Color32::WHITE);
            let title_right = r.left() + w * 0.026 + tg.size().x;
            p.galley(Pos2::new(r.left() + w * 0.026, r.top() + h * 0.07 - tg.size().y / 2.0), tg, Color32::WHITE);
            p.text(
                Pos2::new(r.right() - w * 0.026, r.bottom() - h * 0.035),
                Align2::RIGHT_CENTER,
                format!("CXSFM v{}", env!("CARGO_PKG_VERSION")),
                egui::FontId::proportional(14.0),
                TEXT_DIM,
            );
            // Power button (left, right after the title): ON / OFF of the mod.
            let pb = Rect::from_min_size(Pos2::new(title_right + 28.0, r.top() + h * 0.07 - 19.0), Vec2::new(150.0, 38.0));
            let pr = ui.interact(pb, ui.make_persistent_id(("cxsfm_screen_power", name)), Sense::click());
            p.rect_filled(pb, 3.0, if pr.hovered() { Color32::from_rgb(0x44, 0x44, 0x4A) } else { Color32::from_rgb(0x2A, 0x2A, 0x2F) });
            p.text(pb.center(), Align2::CENTER_CENTER, if powered { "POWER · ON" } else { "POWER · OFF" }, fonts::title(22.0), if powered { ACCENT } else { Color32::WHITE });
            if pr.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                exit.power_off = true;
            }
            // Footer chips: BACK (clickable) then the caller's hints.
            let y = r.bottom() - h * 0.075;
            let mut x = r.left() + w * 0.026;
            let back_r = chip(ui, Pos2::new(x, y), "ESC", "Back");
            if ui.interact(back_r, ui.make_persistent_id(("cxsfm_screen_back", name)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                back = true;
            }
            x = back_r.right() + 8.0;
            for (k, l) in hints {
                x = chip(ui, Pos2::new(x, y), k, l).right() + 8.0;
            }
            let regions = Regions {
                tabs: Rect::from_min_size(Pos2::new(left, r.top() + h * 0.17), Vec2::new(w * 0.62, 38.0)),
                left: Rect::from_min_max(Pos2::new(left, r.top() + h * 0.27), Pos2::new(r.left() + w * 0.50, r.bottom() - h * 0.12)),
                right: Rect::from_min_max(Pos2::new(r.left() + w * 0.515, r.top() + h * 0.27), Pos2::new(r.right() - w * 0.05, r.bottom() - h * 0.12)),
            };
            p.rect_filled(regions.right, 0.0, Color32::from_black_alpha(90));
            body(ui, &regions);
        });
    if back || exit.power_off {
        close();
        padnav::set_focus("menu");
    }
    exit
}
