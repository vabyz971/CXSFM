//! Phone-style main menu: the CarX Street in-game phone, with one
//! rounded colored tile per mod.
//!
//! Layout (all sizes scale with the screen height, see [`scale`]):
//! rounded bezel → status bar (version, logo, mods on/total) → page
//! banner (title + HELP/BACK) → scrollable icon grid → "now playing"
//! strip (description of the hovered/focused tile) → home bar.
//! Tiles sit at explicit rects (no layout flow) so gaps stay exact.
//! Enabled mods are lit in their own color, disabled ones are gray —
//! the game's gray/colored phone tiles.

use super::chrome::forward_drag;
use super::fonts;
use super::theme::{ACCENT, KEY_FG, TEXT_DIM};
use super::tiles::draw_icon;
use crate::menu::{Page, set_page};
use crate::mod_api::get_mod_manager;
use crate::mods::api::TileIcon;
use crate::padnav;
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::sync::atomic::{AtomicUsize, Ordering};

const PHONE_W: f32 = 430.0;
const PHONE_H: f32 = 800.0;
const BEZEL: f32 = 10.0;
const COLS: usize = 4;
const GAP: f32 = 12.0;
const SIDE: f32 = 14.0;
const LABEL_H: f32 = 34.0;
const BODY_BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x14);
const STRIP_BG: Color32 = Color32::from_rgb(0x0B, 0x0B, 0x0C);
const TILE_OFF: Color32 = Color32::from_rgb(0x2B, 0x2B, 0x30);
const GLYPH_OFF: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x92);

/// Pad focus on the grid (item index).
static GRID_FOCUS: AtomicUsize = AtomicUsize::new(0);

/// UI scale from the screen height (1080p = 1.0).
pub fn scale(ctx: &egui::Context) -> f32 {
    let h = ctx.content_rect().height();
    if h.is_finite() && h > 0.0 { (h / 1080.0).clamp(0.7, 1.3) } else { 1.0 }
}

/// Window size at scale `s`.
pub fn size(s: f32) -> Vec2 {
    Vec2::new(PHONE_W * s, PHONE_H * s)
}

/// Tile color of a mod icon (lit state).
fn tile_color(icon: TileIcon) -> Color32 {
    let (r, g, b) = match icon {
        TileIcon::Scout | TileIcon::Tree => (0x2E, 0x9E, 0x5B),
        TileIcon::Sound | TileIcon::Sliders | TileIcon::Info => (0x1F, 0x6F, 0xE0),
        TileIcon::Gauge | TileIcon::Cars | TileIcon::Tag => (0xE8, 0x6A, 0x10),
        TileIcon::Gamepad => (0x7A, 0x4B, 0xD6),
        TileIcon::Power => (0xB0, 0x2E, 0x2E),
        TileIcon::Camera | TileIcon::EyeOff | TileIcon::Log | TileIcon::Globe => (0x4A, 0x4A, 0x52),
    };
    Color32::from_rgb(r, g, b)
}

/// Move the grid focus for one frame's pad events.
fn move_focus(mut f: usize, ev: u32, cols: usize, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    if ev & padnav::LEFT != 0 {
        f = f.saturating_sub(1);
    }
    if ev & padnav::RIGHT != 0 {
        f = (f + 1).min(n - 1);
    }
    if ev & padnav::UP != 0 && f >= cols {
        f -= cols;
    }
    if ev & padnav::DOWN != 0 && f + cols < n {
        f += cols;
    }
    f.min(n - 1)
}

fn rounding(r: f32) -> CornerRadius {
    CornerRadius::same(r.clamp(0.0, 255.0) as u8)
}

/// Draw the whole phone into `phone` (already allocated).
pub(crate) fn draw(ui: &mut Ui, phone: Rect, s: f32, page: Page) {
    let p = ui.painter().clone();
    p.rect_filled(phone, rounding(46.0 * s), Color32::from_rgb(0x08, 0x08, 0x09));
    p.rect_stroke(phone, rounding(46.0 * s), Stroke::new(3.0 * s, Color32::from_rgb(0x3A, 0x3A, 0x40)), StrokeKind::Inside);
    let screen = phone.shrink(BEZEL * s);
    p.rect_filled(screen, rounding(36.0 * s), BODY_BG);

    let mgr = get_mod_manager();
    // Status bar.
    let sb = Rect::from_min_size(screen.min + Vec2::new(18.0 * s, 12.0 * s), Vec2::new(screen.width() - 36.0 * s, 24.0 * s));
    p.text(sb.left_center(), Align2::LEFT_CENTER, format!("v{}", env!("CARGO_PKG_VERSION")), FontId::proportional(13.0 * s), TEXT_DIM);
    p.text(sb.center(), Align2::CENTER_CENTER, "CXSFM", fonts::title(26.0 * s), Color32::WHITE);
    let batt = Rect::from_center_size(Pos2::new(sb.right() - 12.0 * s, sb.center().y), Vec2::new(22.0 * s, 11.0 * s));
    p.rect_stroke(batt, rounding(3.0 * s), Stroke::new(1.3 * s, Color32::WHITE), StrokeKind::Inside);
    let frac = if mgr.mod_count() > 0 { mgr.enabled_count() as f32 / mgr.mod_count() as f32 } else { 0.0 };
    let mut fill = batt.shrink(2.5 * s);
    fill.set_width((fill.width() * frac.clamp(0.0, 1.0)).max(0.0));
    p.rect_filled(fill, 1.0, ACCENT);
    p.text(
        Pos2::new(batt.left() - 6.0 * s, sb.center().y),
        Align2::RIGHT_CENTER,
        format!("{}/{}", mgr.enabled_count(), mgr.mod_count()),
        FontId::proportional(13.0 * s),
        Color32::WHITE,
    );

    // Banner: yellow bar + page title, HELP / BACK on the right.
    let bn = Rect::from_min_size(
        Pos2::new(screen.left() + 14.0 * s, sb.bottom() + 10.0 * s),
        Vec2::new(screen.width() - 28.0 * s, 46.0 * s),
    );
    p.rect_filled(bn, rounding(6.0 * s), STRIP_BG);
    p.rect_filled(Rect::from_min_size(bn.min, Vec2::new(5.0 * s, bn.height())), rounding(2.0 * s), ACCENT);
    let title = match page {
        Page::Main => "MODS",
        Page::Settings => crate::i18n::t("tile_settings"),
        Page::About => crate::i18n::t("tile_about"),
    };
    p.text(bn.left_center() + Vec2::new(18.0 * s, 0.0), Align2::LEFT_CENTER, title.to_uppercase(), fonts::title(30.0 * s), Color32::WHITE);
    let (btn_label, btn_target) = match page {
        Page::Main => (crate::i18n::t("help"), Page::About),
        _ => (crate::i18n::t("back"), Page::Main),
    };
    let btn = Rect::from_center_size(
        Pos2::new(bn.right() - 44.0 * s, bn.center().y),
        Vec2::new(74.0 * s, 28.0 * s),
    );
    let br = ui.interact(btn, ui.make_persistent_id("cxsfm_phone_btn"), Sense::click());
    p.rect_filled(btn, rounding(4.0 * s), if br.hovered() { Color32::from_rgb(0x3C, 0x3C, 0x44) } else { Color32::from_rgb(0x28, 0x28, 0x2D) });
    p.text(btn.center(), Align2::CENTER_CENTER, btn_label.to_uppercase(), fonts::title(18.0 * s), Color32::WHITE);
    if br.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        set_page(btn_target);
    }

    // Home bar + now-playing strip.
    let home = Rect::from_center_size(Pos2::new(screen.center().x, screen.bottom() - 12.0 * s), Vec2::new(120.0 * s, 5.0 * s));
    p.rect_filled(home, rounding(3.0 * s), Color32::from_rgb(0x5A, 0x5A, 0x62));
    let strip = Rect::from_min_max(
        Pos2::new(screen.left() + 8.0 * s, home.top() - 14.0 * s - 80.0 * s),
        Pos2::new(screen.right() - 8.0 * s, home.top() - 14.0 * s),
    );
    let area = Rect::from_min_max(
        Pos2::new(screen.left() + SIDE * s, bn.bottom() + 12.0 * s),
        Pos2::new(screen.right() - SIDE * s, strip.top() - 10.0 * s),
    );

    let mut now: Option<(String, String)> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        now = match page {
            Page::Main => grid(ui, s),
            Page::Settings => super::pages::page_settings(ui),
            Page::About => super::pages::page_about(ui),
        };
    });
    strip_now_playing(ui, strip, s, now);
}

/// Bottom strip: description of the hovered tile, else the lock notice.
fn strip_now_playing(ui: &Ui, strip: Rect, s: f32, now: Option<(String, String)>) {
    let p = ui.painter();
    p.rect_filled(strip, rounding(10.0 * s), STRIP_BG);
    let thumb = Rect::from_min_size(strip.min + Vec2::new(12.0 * s, 12.0 * s), Vec2::splat(strip.height() - 24.0 * s));
    p.rect_filled(thumb, rounding(6.0 * s), Color32::from_rgb(0x1E, 0x1E, 0x22));
    p.text(thumb.center(), Align2::CENTER_CENTER, "CX", fonts::title(24.0 * s), ACCENT);
    let (title, sub) = now.unwrap_or_else(|| ("CXSFM".to_string(), crate::i18n::t("footer_locked").to_string()));
    let tx = thumb.right() + 14.0 * s;
    let w = strip.right() - tx - 12.0 * s;
    p.text(Pos2::new(tx, strip.top() + 12.0 * s), Align2::LEFT_TOP, title.to_uppercase(), fonts::title(30.0 * s), Color32::WHITE);
    let g = p.layout(sub, FontId::proportional(12.5 * s), TEXT_DIM, w);
    p.galley(Pos2::new(tx, strip.top() + 44.0 * s), g, TEXT_DIM);
}

/// Icon grid; returns `(name, description)` of the hovered/focused tile.
fn grid(ui: &mut Ui, s: f32) -> Option<(String, String)> {
    let mgr = get_mod_manager();
    let tiles = mgr.tile_info();
    struct Item {
        icon: TileIcon,
        label: String,
        active: bool,
        lit: bool,
        version: &'static str,
        name: Option<String>,
        kind: u8, // 0 = mod toggle, 1 = settings, 2 = about, 3 = quit
    }
    let mut items: Vec<Item> = tiles
        .iter()
        .map(|t| Item {
            icon: t.icon,
            label: crate::i18n::t(t.label_key).to_string(),
            active: t.enabled,
            lit: t.enabled,
            version: t.version,
            name: Some(t.name.clone()),
            kind: 0,
        })
        .collect();
    items.push(Item { icon: TileIcon::Globe, label: crate::i18n::t("tile_settings").to_string(), active: false, lit: true, version: env!("CARGO_PKG_VERSION"), name: None, kind: 1 });
    items.push(Item { icon: TileIcon::Info, label: crate::i18n::t("tile_about").to_string(), active: false, lit: true, version: env!("CARGO_PKG_VERSION"), name: None, kind: 2 });

    items.push(Item { icon: TileIcon::Power, label: crate::i18n::t("tile_quit").to_string(), active: false, lit: true, version: env!("CARGO_PKG_VERSION"), name: None, kind: 3 });

    let pad = padnav::has_focus("menu");
    let ev = if pad { padnav::events() } else { 0 };
    let mut focus = GRID_FOCUS.load(Ordering::Relaxed);
    if pad {
        focus = move_focus(focus, ev, COLS, items.len());
        GRID_FOCUS.store(focus, Ordering::Relaxed);
        if ev & padnav::BACK != 0 {
            padnav::deactivate();
        }
    }
    let activate = (pad && ev & padnav::ACCEPT != 0 && focus < items.len()).then_some(focus);
    let open_window = (pad && ev & padnav::DEFAULT != 0 && focus < items.len()).then_some(focus);

    // Inner margin: the scroll area clips to its rect, and the focus
    // frame is drawn outside the tile.
    let inset = 5.0 * s;
    let w = ui.available_width() - 2.0 * inset;
    let gap = GAP * s;
    let tile = ((w - gap * (COLS as f32 - 1.0)) / COLS as f32).max(40.0);
    let pitch_y = tile + 4.0 * s + LABEL_H * s + 8.0 * s;
    let rows = items.len().div_ceil(COLS);
    let grid_h = rows as f32 * pitch_y;
    let mut hovered: Option<usize> = None;
    let mut dirty = false;
    let mut go: Option<Page> = None;
    let mut quit = false;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (block, _) = ui.allocate_exact_size(Vec2::new(w + 2.0 * inset, grid_h + 2.0 * inset), Sense::hover());
            let origin = block.min + Vec2::splat(inset);
            for (idx, item) in items.iter().enumerate() {
                let min = origin + Vec2::new((idx % COLS) as f32 * (tile + gap), (idx / COLS) as f32 * pitch_y);
                let rect = Rect::from_min_size(min, Vec2::splat(tile));
                let full = Rect::from_min_size(min, Vec2::new(tile, pitch_y - 8.0 * s));
                let resp = ui
                    .interact(full, ui.make_persistent_id(("cxsfm_phone_tile", idx)), Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                forward_drag(&resp);
                if resp.hovered() {
                    hovered = Some(idx);
                }
                let focused = pad && idx == focus;
                if focused {
                    hovered = Some(idx);
                }
                // Close button (enabled mods): created after the tile so
                // it wins the click.
                let mut close_hit = false;
                let mut close_hover = None;
                if item.kind == 0 && item.active {
                    let cb = Rect::from_center_size(rect.left_top() + Vec2::new(13.0 * s, 13.0 * s), Vec2::splat(24.0 * s));
                    let cr = ui
                        .interact(cb, ui.make_persistent_id(("cxsfm_phone_close", idx)), Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    close_hover = Some(cr.hovered());
                    if cr.hovered() {
                        hovered = Some(idx);
                    }
                    if cr.clicked()
                        && let Some(name) = &item.name
                    {
                        mgr.set_mod_enabled(name, false);
                        super::screen::close();
                        close_hit = true;
                        dirty = true;
                    }
                }
                paint_tile(ui, rect, s, item.icon, &item.label, item.lit, item.active && item.kind == 0, resp.hovered(), focused, close_hover);
                if !close_hit && (resp.clicked() || activate == Some(idx)) {
                    match item.kind {
                        1 => go = Some(Page::Settings),
                        2 => go = Some(Page::About),
                        3 => quit = true,
                        _ => {
                            if let Some(name) = &item.name
                                && super::screen::is_app(name)
                            {
                                // Full-screen mod: the tile opens its screen
                                // (enabling the mod first); power-off lives
                                // inside the screen.
                                if !item.active {
                                    mgr.set_mod_enabled(name, true);
                                }
                                super::screen::open(name);
                                if pad {
                                    padnav::set_focus(&super::screen::pad_id(name));
                                }
                                dirty = true;
                            } else if let Some(name) = &item.name {
                                mgr.set_mod_enabled(name, !item.active);
                                dirty = true;
                                // Turned on from the pad: its window takes
                                // the pad (B hands it back to the menu).
                                if pad && !item.active {
                                    padnav::set_focus(&super::screen::pad_id(name));
                                }
                            }
                        }
                    }
                }
            }
        });
    if let Some(i) = open_window
        && let Some(it) = items.get(i)
        && it.kind == 0
        && it.active
        && let Some(name) = &it.name
    {
        if super::screen::is_app(name) {
            super::screen::open(name);
        }
        padnav::set_focus(&super::screen::pad_id(name));
    }
    if let Some(pg) = go {
        set_page(pg);
    }
    if quit {
        // Close the mod menu (same as F8); mods keep running.
        padnav::deactivate();
        if crate::hotkey::ui_visible() {
            crate::hotkey::toggle_ui();
        }
    }
    if dirty {
        ui.ctx().request_repaint();
    }
    hovered.and_then(|i| items.get(i)).map(|it| {
        let desc = it
            .name
            .as_ref()
            .and_then(|n| {
                let code = crate::i18n::current().code();
                mgr.describe_all(code).into_iter().find(|(name, _, _)| name == n).map(|(_, _, d)| d.to_string())
            })
            .unwrap_or_else(|| match it.kind {
                1 => crate::i18n::t("settings_language").to_string(),
                3 => crate::i18n::t("keys_note").to_string(),
                _ => format!("CXSFM {}", env!("CARGO_PKG_VERSION")),
            });
        (it.label.clone(), format!("v{} · {}", it.version, desc))
    })
}

/// One rounded tile with glyph, ON badge and wrapped label underneath.
#[allow(clippy::too_many_arguments)]
fn paint_tile(ui: &Ui, rect: Rect, s: f32, icon: TileIcon, label: &str, lit: bool, on: bool, hover: bool, focused: bool, close: Option<bool>) {
    let p = ui.painter();
    let base = if lit { tile_color(icon) } else { TILE_OFF };
    let bg = if hover { base.lerp_to_gamma(Color32::WHITE, 0.14) } else { base };
    p.rect_filled(rect, rounding(9.0 * s), bg);
    // Soft top highlight, like the game's glossy tiles.
    let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), rect.height() * 0.45));
    p.rect_filled(top, CornerRadius { nw: (9.0 * s) as u8, ne: (9.0 * s) as u8, sw: 0, se: 0 }, Color32::from_white_alpha(10));
    draw_icon(p, rect.center(), rect.width() * 0.27, icon, if lit { Color32::WHITE } else { GLYPH_OFF });
    if on {
        let c = rect.right_top() + Vec2::new(-10.0 * s, 10.0 * s);
        p.circle_filled(c, 7.0 * s, ACCENT);
        p.circle_stroke(c, 7.0 * s, Stroke::new(1.2 * s, KEY_FG));
    }
    if let Some(h) = close {
        // Close chip, top-left: dark disc with an X.
        let c = rect.left_top() + Vec2::new(13.0 * s, 13.0 * s);
        p.circle_filled(c, 10.0 * s, if h { Color32::from_rgb(0xD0, 0x30, 0x30) } else { Color32::from_black_alpha(150) });
        let k = 3.6 * s;
        let st = Stroke::new(1.8 * s, Color32::WHITE);
        p.line_segment([c + Vec2::new(-k, -k), c + Vec2::new(k, k)], st);
        p.line_segment([c + Vec2::new(-k, k), c + Vec2::new(k, -k)], st);
    }
    if focused {
        p.rect_stroke(rect.expand(2.0 * s), rounding(11.0 * s), Stroke::new(2.5 * s, Color32::WHITE), StrokeKind::Outside);
    }
    let col = if lit { Color32::WHITE } else { Color32::from_rgb(0xA8, 0xA8, 0xB0) };
    let g = p.layout(label.to_uppercase(), fonts::title(17.0 * s), col, rect.width() + 6.0 * s);
    let pos = Pos2::new(rect.center().x - g.size().x / 2.0, rect.bottom() + 5.0 * s);
    p.galley(pos, g, col);
}

#[cfg(test)]
mod tests {
    use super::move_focus;
    use crate::padnav;

    #[test]
    fn focus_moves_in_grid_and_clamps() {
        assert_eq!(move_focus(0, padnav::RIGHT, 4, 10), 1);
        assert_eq!(move_focus(1, padnav::DOWN, 4, 10), 5);
        assert_eq!(move_focus(9, padnav::DOWN, 4, 10), 9);
        assert_eq!(move_focus(2, padnav::UP, 4, 10), 2);
        assert_eq!(move_focus(0, padnav::LEFT, 4, 10), 0);
        assert_eq!(move_focus(0, padnav::RIGHT, 4, 0), 0);
    }
}
