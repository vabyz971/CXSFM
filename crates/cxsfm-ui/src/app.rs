//! Phone apps (new-generation mods): a mod that lists tabs
//! (`Mod::app_tabs`) opens INSIDE the phone instead of in its own
//! window or a full-screen frame.
//!
//! Layout, drawn by the phone: banner (app title, power, BACK) → tab
//! strip ([`tab_strip`]) → the mod's body (`Mod::on_draw_app`, one tab
//! at a time, scrollable) → bottom strip with the focused row's help.
//! Only one app is open at a time; BACK (banner or pad B) returns to the
//! grid and leaves the mod running, power turns it off.
//!
//! The pad belongs to the phone while an app is open: app rows use
//! [`PAD_OWNER`] with `game::Nav`, LB / RB switch tabs.

use super::fonts;
use super::theme::ACCENT;
use cxsfm_input::padnav;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Ui, Vec2};
use std::sync::Mutex;

/// Pad-focus owner of the phone (grid, pages and apps).
pub const PAD_OWNER: &str = "menu";

/// Open app: `(mod name, tab)`.
static OPEN: Mutex<Option<(String, usize)>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Open app `name` (keeps its tab when it is already the open one).
pub fn open(name: &str) {
    let mut o = lock(&OPEN);
    if o.as_ref().is_none_or(|(n, _)| n != name) {
        *o = Some((name.to_string(), 0));
    }
    drop(o);
    crate::repaint::kick();
}

/// Back to the grid (no-op when nothing is open).
pub fn close() {
    if lock(&OPEN).take().is_some() {
        crate::repaint::kick();
    }
}

/// Close only if `name` is the open app (a mod turning itself off).
pub fn close_if(name: &str) {
    let mut o = lock(&OPEN);
    if o.as_ref().is_some_and(|(n, _)| n == name) {
        *o = None;
        drop(o);
        crate::repaint::kick();
    }
}

/// The open app and its tab.
pub fn current() -> Option<(String, usize)> {
    lock(&OPEN).clone()
}

pub fn is_open(name: &str) -> bool {
    lock(&OPEN).as_ref().is_some_and(|(n, _)| n == name)
}

fn set_tab(name: &str, tab: usize) {
    let mut o = lock(&OPEN);
    if let Some((n, t)) = o.as_mut()
        && n == name
        && *t != tab
    {
        *t = tab;
        drop(o);
        crate::repaint::kick();
    }
}

/// Pad-focus id of a classic mod window: its name, snake-cased.
pub fn pad_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

/// Tab step from this frame's pad events (LB / RB), clamped.
fn step_tab(cur: usize, n: usize, ev: u32) -> usize {
    let mut t = cur.min(n.saturating_sub(1));
    if ev & padnav::TAB_PREV != 0 {
        t = t.saturating_sub(1);
    }
    if ev & padnav::TAB_NEXT != 0 && t + 1 < n {
        t += 1;
    }
    t
}

/// Tab strip of app `name` under the banner, `s` = phone scale. Click
/// or LB / RB switch; the strip scrolls sideways when the labels do not
/// fit. Returns the current tab.
pub fn tab_strip(ui: &mut Ui, name: &str, cur: usize, tabs: &[&str], s: f32) -> usize {
    let ev = if padnav::has_focus(PAD_OWNER) { padnav::events() } else { 0 };
    let mut tab = step_tab(cur, tabs.len(), ev);
    let h = 30.0 * s;
    let font = fonts::title(19.0 * s);
    egui::ScrollArea::horizontal()
        .id_salt(("cxsfm_app_tabs", name))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0 * s;
                for (i, label) in tabs.iter().enumerate() {
                    let on = i == tab;
                    let g = ui.painter().layout_no_wrap(label.to_uppercase(), font.clone(), Color32::WHITE);
                    let w = g.size().x + 22.0 * s;
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    let bg = if on {
                        Color32::WHITE
                    } else if resp.hovered() {
                        Color32::from_rgb(0x3C, 0x3C, 0x44)
                    } else {
                        Color32::from_rgb(0x22, 0x22, 0x27)
                    };
                    let r = CornerRadius::same((4.0 * s).clamp(0.0, 255.0) as u8);
                    ui.painter().rect_filled(rect, r, bg);
                    if on {
                        let bar = Rect::from_min_size(rect.left_bottom() - Vec2::new(0.0, 3.0 * s), Vec2::new(rect.width(), 3.0 * s));
                        ui.painter().rect_filled(bar, 0.0, ACCENT);
                    }
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        label.to_uppercase(),
                        font.clone(),
                        if on { Color32::BLACK } else { Color32::WHITE },
                    );
                    if resp.clicked() {
                        tab = i;
                    }
                    if on && tab != cur {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
        });
    set_tab(name, tab);
    tab
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_step_clamps() {
        assert_eq!(step_tab(0, 4, padnav::TAB_NEXT), 1);
        assert_eq!(step_tab(3, 4, padnav::TAB_NEXT), 3);
        assert_eq!(step_tab(0, 4, padnav::TAB_PREV), 0);
        assert_eq!(step_tab(9, 4, 0), 3);
        assert_eq!(step_tab(0, 0, padnav::TAB_NEXT), 0);
    }

    #[test]
    fn open_keeps_tab_of_same_app() {
        open("A");
        set_tab("A", 2);
        open("A");
        assert_eq!(current(), Some(("A".to_string(), 2)));
        close_if("B");
        assert!(is_open("A"));
        open("B");
        assert_eq!(current(), Some(("B".to_string(), 0)));
        close();
        assert_eq!(current(), None);
    }
}
