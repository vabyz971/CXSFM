//! "Game style" widget kit: the CarX Street settings look (dark rows,
//! light selected row, `‹ VALUE ›` selectors, thin sliders, yellow
//! section titles, key-hint chips) with pad navigation built in.
//!
//! Immediate-mode navigation: a window keeps a [`NavState`]; each frame
//! it opens a [`Nav`], draws its rows through the widgets below (every
//! widget takes the next row index) and calls [`Nav::finish`]. Up/Down
//! from the pad move the focus, Left/Right/Accept/Default act on the
//! focused row only, and the mouse moves the focus by hovering.
//! Everything is painted (no font glyph tricks — chevrons are strokes).

use super::theme::{ACCENT, KEY_BG, KEY_FG, ROUNDING, TEXT_DIM};
use crate::padnav;
use egui::{Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

pub const ROW_H: f32 = 56.0;
const ROW_BG: Color32 = Color32::from_rgba_premultiplied(12, 12, 13, 205);
const ROW_SEL: Color32 = Color32::from_rgb(0xDA, 0xDA, 0xDC);
const ROW_SEL_TEXT: Color32 = Color32::from_rgb(0x14, 0x14, 0x16);
const SEG_OFF: Color32 = Color32::from_rgb(0x5C, 0x5C, 0x62);
const LABEL_SIZE: f32 = 24.0;
const VALUE_SIZE: f32 = 20.0;

/// Focus memory of one window.
#[derive(Default)]
pub struct NavState {
    pub focus: usize,
    count: usize,
}

/// What a row reports back to the window.
#[derive(Default, Clone, Copy)]
pub struct Out {
    pub changed: bool,
    /// X pressed on the row (reset it to its default).
    pub reset: bool,
    /// A pressed / row clicked.
    pub accept: bool,
}

/// One frame of navigation for one window.
pub struct Nav<'a> {
    st: &'a mut NavState,
    ev: u32,
    idx: usize,
    pad: bool,
    pointer_moved: bool,
    /// Description of the focused row (right-hand panel).
    pub help: Option<String>,
}

impl<'a> Nav<'a> {
    /// `owner` = the window's pad-focus name (see `padnav`).
    pub fn begin(ui: &Ui, st: &'a mut NavState, owner: &str) -> Self {
        let pad = padnav::has_focus(owner);
        let ev = if pad { padnav::events() } else { 0 };
        if pad && st.count > 0 {
            if ev & padnav::UP != 0 {
                st.focus = if st.focus == 0 { st.count - 1 } else { st.focus - 1 };
            }
            if ev & padnav::DOWN != 0 {
                st.focus = (st.focus + 1) % st.count;
            }
        }
        st.focus = st.focus.min(st.count.saturating_sub(1));
        let pointer_moved = ui.input(|i| i.pointer.delta() != Vec2::ZERO);
        Self { st, ev, idx: 0, pad, pointer_moved, help: None }
    }

    /// Close the frame: remember how many rows there were.
    pub fn finish(self) {
        self.st.count = self.idx;
    }

    /// Back to the first row (tab switch).
    pub fn reset_focus(&mut self) {
        self.st.focus = 0;
    }

    /// Is the pad driving this window?
    pub fn pad(&self) -> bool {
        self.pad
    }

    /// Frame events that belong to this window (tabs, alt…).
    pub fn window_events(&self) -> u32 {
        self.ev
    }

    /// Register a row: `(selected, events for this row)`.
    fn row(&mut self, resp: &Response, help: &str) -> (bool, u32) {
        let i = self.idx;
        self.idx += 1;
        if resp.hovered() && self.pointer_moved {
            self.st.focus = i;
        }
        let focused = self.st.focus == i;
        let selected = focused && (self.pad || resp.hovered());
        if focused && !help.is_empty() {
            self.help = Some(help.to_string());
        }
        (selected, if self.pad && focused { self.ev } else { 0 })
    }
}

fn alloc_row(ui: &mut Ui, id: &str) -> (Rect, Response) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
    let resp = ui.interact(rect, ui.make_persistent_id(id), Sense::click_and_drag());
    ui.add_space(2.0);
    (rect, resp)
}

fn paint_row(ui: &Ui, rect: Rect, selected: bool, label: &str) -> Color32 {
    let p = ui.painter();
    p.rect_filled(rect, 0.0, if selected { ROW_SEL } else { ROW_BG });
    let fg = if selected { ROW_SEL_TEXT } else { Color32::WHITE };
    p.text(
        Pos2::new(rect.left() + 20.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(LABEL_SIZE),
        fg,
    );
    fg
}

fn chevron(p: &egui::Painter, c: Pos2, dir: f32, col: Color32) {
    let s = 4.0;
    let pts = [
        Pos2::new(c.x - dir * s * 0.6, c.y - s),
        Pos2::new(c.x + dir * s * 0.6, c.y),
        Pos2::new(c.x - dir * s * 0.6, c.y + s),
    ];
    p.line_segment([pts[0], pts[1]], Stroke::new(1.6, col));
    p.line_segment([pts[1], pts[2]], Stroke::new(1.6, col));
}

fn snap(v: f32, min: f32, max: f32, step: f32) -> f32 {
    if !v.is_finite() {
        return min;
    }
    let v = v.clamp(min, max);
    if step <= 0.0 {
        return v;
    }
    (((v - min) / step).round() * step + min).clamp(min, max)
}

/// Slider row: value left of a thin track with a round knob.
#[allow(clippy::too_many_arguments)]
pub fn slider_row(
    ui: &mut Ui,
    nav: &mut Nav,
    id: &str,
    label: &str,
    v: &mut f32,
    (min, max): (f32, f32),
    step: f32,
    decimals: usize,
    unit: &str,
    help: &str,
) -> Out {
    let (rect, resp) = alloc_row(ui, id);
    let (sel, ev) = nav.row(&resp, help);
    let fg = paint_row(ui, rect, sel, label);
    let w = rect.width();
    let (x0, x1) = (rect.left() + w * 0.62, rect.left() + w * 0.94);
    let cy = rect.center().y;
    let p = ui.painter();
    p.text(
        Pos2::new(rect.left() + w * 0.575, cy),
        Align2::RIGHT_CENTER,
        format!("{:.*}{}", decimals, *v, unit),
        FontId::proportional(VALUE_SIZE),
        fg,
    );
    chevron(p, Pos2::new(rect.left() + w * 0.595, cy), -1.0, fg);
    chevron(p, Pos2::new(rect.left() + w * 0.975, cy), 1.0, fg);
    let t = if max > min { ((*v - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
    let kx = x0 + t * (x1 - x0);
    let track = if sel { ROW_SEL_TEXT } else { SEG_OFF };
    p.line_segment([Pos2::new(x0, cy), Pos2::new(x1, cy)], Stroke::new(2.0, track));
    p.line_segment([Pos2::new(x0, cy), Pos2::new(kx, cy)], Stroke::new(2.0, fg));
    p.circle(Pos2::new(kx, cy), 6.0, if sel { ROW_SEL } else { Color32::BLACK }, Stroke::new(1.8, fg));

    let mut out = Out::default();
    let old = *v;
    if (resp.clicked() || resp.dragged()) && let Some(pos) = resp.interact_pointer_pos() {
        if pos.x >= rect.left() + w * 0.58 && pos.x < rect.left() + w * 0.61 {
            *v = snap(*v - step, min, max, step);
        } else if pos.x > rect.left() + w * 0.955 {
            *v = snap(*v + step, min, max, step);
        } else if pos.x >= x0 - 8.0 && pos.x <= x1 + 8.0 {
            let k = ((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0);
            *v = snap(min + k * (max - min), min, max, step);
        }
    }
    if ev & padnav::LEFT != 0 {
        *v = snap(*v - step, min, max, step);
    }
    if ev & padnav::RIGHT != 0 {
        *v = snap(*v + step, min, max, step);
    }
    out.changed = (*v - old).abs() > f32::EPSILON;
    out.reset = ev & padnav::DEFAULT != 0;
    out
}

/// Selector row: `‹ VALUE ›` with one segment per option.
pub fn choice_row(
    ui: &mut Ui,
    nav: &mut Nav,
    id: &str,
    label: &str,
    cur: &mut usize,
    options: &[&str],
    help: &str,
) -> Out {
    choice_inner(ui, nav, id, label, cur, options, help, false)
}

/// ON / OFF row (click or A flips it, like the game's Enter).
pub fn toggle_row(ui: &mut Ui, nav: &mut Nav, id: &str, label: &str, v: &mut bool, help: &str) -> Out {
    let mut i = *v as usize;
    let out = choice_inner(ui, nav, id, label, &mut i, &["OFF", "ON"], help, true);
    *v = i == 1;
    out
}

#[allow(clippy::too_many_arguments)]
fn choice_inner(
    ui: &mut Ui,
    nav: &mut Nav,
    id: &str,
    label: &str,
    cur: &mut usize,
    options: &[&str],
    help: &str,
    toggle: bool,
) -> Out {
    let (rect, resp) = alloc_row(ui, id);
    let (sel, ev) = nav.row(&resp, help);
    let fg = paint_row(ui, rect, sel, label);
    let n = options.len().max(1);
    *cur = (*cur).min(n - 1);
    let w = rect.width();
    let (cx, cy) = (rect.left() + w * 0.78, rect.center().y);
    let p = ui.painter();
    p.text(
        Pos2::new(cx, cy - 5.0),
        Align2::CENTER_CENTER,
        options.get(*cur).copied().unwrap_or(""),
        FontId::proportional(VALUE_SIZE),
        fg,
    );
    let seg_w = (w * 0.075).min(w * 0.36 / n as f32);
    let total = seg_w * n as f32;
    for i in 0..n {
        let x = cx - total / 2.0 + i as f32 * seg_w;
        let on = i == *cur;
        p.line_segment(
            [Pos2::new(x + 2.0, cy + 11.0), Pos2::new(x + seg_w - 2.0, cy + 11.0)],
            Stroke::new(if on { 3.0 } else { 1.5 }, if on { fg } else { SEG_OFF }),
        );
    }
    if sel && !toggle {
        chevron(p, Pos2::new(rect.left() + w * 0.6, cy), -1.0, fg);
        chevron(p, Pos2::new(rect.left() + w * 0.975, cy), 1.0, fg);
    }
    let old = *cur;
    if toggle {
        if resp.clicked() || ev & padnav::ACCEPT != 0 {
            *cur = 1 - (*cur).min(1);
        }
        if ev & padnav::LEFT != 0 {
            *cur = 0;
        }
        if ev & padnav::RIGHT != 0 {
            *cur = 1;
        }
    } else {
        if resp.clicked() && let Some(pos) = resp.interact_pointer_pos() {
            if pos.x < cx {
                *cur = cur.saturating_sub(1);
            } else {
                *cur = (*cur + 1).min(n - 1);
            }
        }
        if ev & padnav::LEFT != 0 {
            *cur = cur.saturating_sub(1);
        }
        if ev & padnav::RIGHT != 0 {
            *cur = (*cur + 1).min(n - 1);
        }
    }
    Out { changed: *cur != old, reset: ev & padnav::DEFAULT != 0, accept: false }
}

/// Action row: label left, current value chip right, `A  <hint>` chip
/// when selected.
pub fn button_row(
    ui: &mut Ui,
    nav: &mut Nav,
    id: &str,
    label: &str,
    value: &str,
    hint: &str,
    help: &str,
) -> Out {
    let (rect, resp) = alloc_row(ui, id);
    let (sel, ev) = nav.row(&resp, help);
    let fg = paint_row(ui, rect, sel, label);
    let p = ui.painter();
    let w = rect.width();
    if !value.is_empty() {
        let box_r = Rect::from_min_max(
            Pos2::new(rect.left() + w * 0.5, rect.top() + 4.0),
            Pos2::new(rect.right() - if sel { 110.0 } else { 10.0 }, rect.bottom() - 4.0),
        );
        p.rect_filled(box_r, 0.0, if sel { Color32::from_rgb(0xBC, 0xBC, 0xC0) } else { Color32::from_rgb(0x2C, 0x2C, 0x30) });
        p.text(box_r.center(), Align2::CENTER_CENTER, value, FontId::proportional(VALUE_SIZE), if sel { ROW_SEL_TEXT } else { fg });
    }
    if sel {
        let chip = Rect::from_min_max(
            Pos2::new(rect.right() - 100.0, rect.top() + 8.0),
            Pos2::new(rect.right() - 8.0, rect.bottom() - 8.0),
        );
        p.rect_filled(chip, 2.0, Color32::from_rgb(0x44, 0x44, 0x48));
        p.text(chip.center(), Align2::CENTER_CENTER, hint, FontId::proportional(15.0), Color32::WHITE);
    }
    Out {
        changed: false,
        reset: ev & padnav::DEFAULT != 0,
        accept: resp.clicked() || ev & padnav::ACCEPT != 0,
    }
}

/// Yellow uppercase section title.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text.to_uppercase()).font(super::fonts::title(32.0)).color(ACCENT));
    ui.add_space(4.0);
}

/// Key chip + label (`ESC  BACK`).
pub fn key_hint(ui: &mut Ui, key: &str, text: &str) {
    ui.label(egui::RichText::new(format!(" {key} ")).font(super::fonts::title(20.0)).color(KEY_FG).background_color(KEY_BG));
    ui.label(egui::RichText::new(text.to_uppercase()).font(super::fonts::title(20.0)).color(Color32::WHITE));
    ui.add_space(10.0);
}

/// Top tabs; LB / RB (pad) or a click switch. Returns true on change.
pub fn tabs(ui: &mut Ui, nav: &Nav, labels: &[&str], cur: &mut usize) -> bool {
    let old = *cur;
    let ev = nav.window_events();
    if ev & padnav::TAB_PREV != 0 {
        *cur = cur.saturating_sub(1);
    }
    if ev & padnav::TAB_NEXT != 0 {
        *cur = (*cur + 1).min(labels.len().saturating_sub(1));
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if nav.pad() {
            ui.label(egui::RichText::new(" LB ").size(13.0).strong().color(KEY_FG).background_color(KEY_BG));
        }
        for (i, l) in labels.iter().enumerate() {
            let on = i == *cur;
            let galley_w = 30.0 + l.chars().count() as f32 * 11.0;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley_w, 38.0), Sense::click());
            ui.painter().rect_filled(rect, 0.0, if on { Color32::WHITE } else { ROW_BG });
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                l.to_uppercase(),
                super::fonts::title(22.0),
                if on { Color32::BLACK } else { Color32::WHITE },
            );
            if resp.clicked() {
                *cur = i;
            }
        }
        if nav.pad() {
            ui.label(egui::RichText::new(" RB ").size(13.0).strong().color(KEY_FG).background_color(KEY_BG));
        }
    });
    *cur != old
}

/// Warning banner (advanced mode).
pub fn warning(ui: &mut Ui, text: &str) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, ROUNDING, Color32::from_rgb(0x3A, 0x1E, 0x10));
    p.rect_stroke(rect, ROUNDING, Stroke::new(1.5, super::theme::STATE_WARN), egui::StrokeKind::Inside);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(14.0),
        super::theme::STATE_WARN,
    );
}

/// Dim helper text in the right-hand panel.
pub fn help_text(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(22.0).color(Color32::from_rgb(0xE4, 0xE4, 0xE8)));
}

/// Dim secondary line.
pub fn dim(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(16.0).color(TEXT_DIM));
}

#[cfg(test)]
mod tests {
    use super::snap;

    #[test]
    fn snap_rounds_to_step_and_clamps() {
        assert_eq!(snap(0.26, 0.0, 1.0, 0.1), 0.3);
        assert_eq!(snap(5.0, 0.0, 1.0, 0.1), 1.0);
        assert_eq!(snap(f32::NAN, 2.0, 3.0, 0.5), 2.0);
        assert_eq!(snap(0.3, 0.0, 1.0, 0.0), 0.3);
    }
}
