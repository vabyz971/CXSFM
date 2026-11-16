//! Layout editor (present thread only): select, drag, align and edit
//! the HUD widgets right on the game screen, add primitive shapes,
//! bind values. Pure geometry and file edits live in [`super::edit`].
//!
//! The editor works on a copy of the published layout: every change
//! shows at once and is recorded as an [`Edit`]; **Save** hands the log
//! to the tick (file IO never happens here), which writes only the
//! touched keys into `custom_hud.toml` — comments stay. The copy
//! re-syncs from the file whenever nothing is pending.
//!
//! On screen (menu open, Editor tab): click a widget to select it
//! (Shift = add/remove), drag to move (magnetic: HUD box edges and
//! centre, other widgets, grid), drag an empty spot of a HUD to move the
//! whole HUD. Keys: arrows nudge (Shift ×10), Delete, Ctrl+Z, Ctrl+D.

use super::edit::{self, AlignOp, Edit, Val};
use super::i18n::ed;
use super::model::{
    Align, Alert, Anchor, Bound, FontKind, Hud, HudConfig, Placed, ShapeKind, SourceDef, Values, Widget,
    format_template, rotate_about, shape_text,
};
use super::ui::{draw_hud, hud_rect};
use cxsfm_ui::kit::{self, Pane};
use cxsfm_ui::theme::{ACCENT, TEXT_DIM};
use egui::{Color32, Pos2, Rect, Vec2};
use egui_material_icons::icons as ic;
use std::sync::Arc;

/// Undo depth (layout snapshots are small).
const UNDO_MAX: usize = 100;
/// Magnet reach, in screen pixels.
const SNAP_PX: f32 = 6.0;

#[derive(Clone)]
enum DragKind {
    /// Selected widgets: original positions, their union box and the
    /// snap lines of everything else (HUD units).
    Widgets { orig: Vec<(usize, [f32; 2])>, bbox: Rect, xs: Vec<f32>, ys: Vec<f32> },
    /// Whole HUD box: original offset and screen box.
    Hud { offset: [f32; 2], rect: Rect },
}

#[derive(Clone)]
struct Drag {
    start: Pos2,
    scale: f32,
    kind: DragKind,
}

/// One "add" button: icon, i18n key, enabled, keys of the new widget.
type AddItem = (egui_material_icons::MaterialIcon, &'static str, bool, Vec<(String, Val)>);

/// What the panel asks the mod to do.
pub(crate) enum PanelOut {
    None,
    Save(Vec<Edit>),
}

pub(crate) struct Editor {
    /// Working copy (None until the first published layout).
    pub(crate) work: Option<HudConfig>,
    /// Published layout the copy was taken from.
    base: Option<Arc<HudConfig>>,
    /// Pending changes (Save sends them).
    pub(crate) edits: Vec<Edit>,
    undo: Vec<(HudConfig, usize)>,
    /// Edited HUD (position in `work.huds`).
    pub(crate) hud: usize,
    /// Selected widgets (file indices, `Placed::index`).
    pub(crate) sel: Vec<usize>,
    drag: Option<Drag>,
    /// Last property key changed (one undo step per key run).
    last_key: Option<String>,
    pub(crate) magnet: bool,
    pub(crate) grid: bool,
    pub(crate) grid_step: f32,
    /// Snap guides hit by the current drag (HUD units).
    guides: (Option<f32>, Option<f32>),
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            work: None,
            base: None,
            edits: Vec::new(),
            undo: Vec::new(),
            hud: 0,
            sel: Vec::new(),
            drag: None,
            last_key: None,
            magnet: true,
            grid: false,
            grid_step: 8.0,
            guides: (None, None),
        }
    }
}

// --- Geometry --------------------------------------------------------------

fn p2(q: [f32; 2]) -> Pos2 {
    egui::pos2(q[0], q[1])
}

/// Bounding box of `r` turned `deg` around `pivot`.
fn rotated(r: Rect, pivot: [f32; 2], deg: f32) -> Rect {
    if deg == 0.0 {
        return r;
    }
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|q| p2(rotate_about([q.x, q.y], pivot, deg)));
    Rect::from_points(&pts)
}

fn circle(pos: [f32; 2], r: f32) -> Rect {
    Rect::from_center_size(p2(pos), Vec2::splat(2.0 * r.max(2.0)))
}

/// Box of a text of `size` placed by `align` on `pos`.
fn text_box(pos: [f32; 2], size: Vec2, align: Align) -> Rect {
    let x = match align {
        Align::Left | Align::Justify => pos[0],
        Align::Center => pos[0] - size.x * 0.5,
        Align::Right => pos[0] - size.x,
    };
    Rect::from_min_size(egui::pos2(x, pos[1]), size)
}

fn label_font(font: FontKind, size: f32) -> egui::FontId {
    match font {
        FontKind::Sans => egui::FontId::proportional(size),
        FontKind::Mono => egui::FontId::monospace(size),
        FontKind::Serif => cxsfm_ui::fonts::serif(size),
        FontKind::Title => cxsfm_ui::fonts::title(size),
    }
}

/// Box of a widget in HUD units (rotation included). `p` only lays
/// text out (any painter of the context).
pub(crate) fn widget_bounds(p: &egui::Painter, w: &Widget, v: &Values) -> Rect {
    let r = match w {
        Widget::Bar { pos, size, rotation, .. } => {
            rotated(Rect::from_two_pos(p2(*pos), egui::pos2(pos[0] + size[0], pos[1] + size[1])), *pos, *rotation)
        }
        Widget::Arc { pos, radius, thickness, filled, .. } => {
            circle(*pos, if *filled { *radius } else { radius + thickness * 0.5 })
        }
        Widget::Needle { pos, length, tail, .. } => circle(*pos, length.max(*tail)),
        Widget::Ticks { pos, radius, .. } => circle(*pos, *radius),
        Widget::Text { text, pos, size, align, mono, rotation, .. } => {
            let shown = format_template(text, v);
            let shown = if shown.is_empty() { text.clone() } else { shown };
            let font = if *mono { egui::FontId::monospace(*size) } else { egui::FontId::proportional(*size) };
            let g = p.layout_no_wrap(shown, font, Color32::WHITE);
            rotated(text_box(*pos, g.size(), *align), *pos, *rotation)
        }
        Widget::Label { text, trim, font, size, align, pos, rotation, max_width, .. } => {
            let shown = shape_text(&format_template(text, v), trim);
            let shown = if shown.is_empty() { text.clone() } else { shown };
            let wrap = if *max_width > 0.0 { *max_width } else { f32::INFINITY };
            let g = p.layout(shown, label_font(*font, *size), Color32::WHITE, wrap);
            rotated(text_box(*pos, g.rect.size(), *align), *pos, *rotation)
        }
        Widget::Shape { kind: ShapeKind::Line, pos, size, rotation, stroke, .. } => {
            let end = rotate_about([pos[0] + size[0], pos[1] + size[1]], *pos, *rotation);
            Rect::from_two_pos(p2(*pos), p2(end)).expand((stroke * 0.5).max(3.0))
        }
        Widget::Shape { pos, size, rotation, .. } => {
            let r = Rect::from_two_pos(p2(*pos), egui::pos2(pos[0] + size[0], pos[1] + size[1]));
            rotated(r, [r.center().x, r.center().y], *rotation)
        }
    };
    // Never a zero-size target (empty text, flat bar).
    let grow = Vec2::new((6.0 - r.width()).max(0.0) * 0.5, (6.0 - r.height()).max(0.0) * 0.5);
    r.expand2(grow)
}

/// HUD-unit box → screen box.
fn to_screen(hud: Rect, scale: f32, r: Rect) -> Rect {
    Rect::from_min_max(hud.min + r.min.to_vec2() * scale, hud.min + r.max.to_vec2() * scale)
}

fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Short type name shown in the widget list.
fn kind_name(w: &Widget) -> &'static str {
    match w {
        Widget::Bar { .. } => "bar",
        Widget::Arc { .. } => "arc",
        Widget::Needle { .. } => "needle",
        Widget::Ticks { .. } => "ticks",
        Widget::Text { .. } => "text",
        Widget::Label { .. } => "label",
        Widget::Shape { kind, .. } => kind.key(),
    }
}

/// One-line hint next to the type (bound value or text).
fn kind_hint(w: &Widget) -> String {
    match w {
        Widget::Bar { source, .. } | Widget::Needle { source, .. } => source.clone(),
        Widget::Arc { source, .. } => source.clone().unwrap_or_default(),
        Widget::Text { text, .. } | Widget::Label { text, .. } => text.clone(),
        _ => String::new(),
    }
}

// --- State -----------------------------------------------------------------

impl Editor {
    /// Take the published layout when nothing is pending (first open,
    /// after Save, after a hand edit of the file).
    pub(crate) fn sync(&mut self, published: &Option<Arc<HudConfig>>) {
        if !self.edits.is_empty() {
            return;
        }
        let same = match (&self.base, published) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same && self.work.is_some() {
            return;
        }
        self.base = published.clone();
        self.work = published.as_deref().cloned();
        self.undo.clear();
        self.drag = None;
        let n = self.work.as_ref().map_or(0, |w| w.huds.len());
        self.hud = self.hud.min(n.saturating_sub(1));
        let alive: Vec<usize> = self.cur().map(|h| h.widgets.iter().map(|p| p.index).collect()).unwrap_or_default();
        self.sel.retain(|i| alive.contains(i));
    }

    fn cur(&self) -> Option<&Hud> {
        self.work.as_ref()?.huds.get(self.hud)
    }

    /// Undo floor for coalescing (see `edit::push_edit`).
    fn floor(&self) -> usize {
        self.undo.last().map_or(0, |u| u.1)
    }

    fn push_undo(&mut self, snapshot: HudConfig, edits_len: usize) {
        self.undo.push((snapshot, edits_len));
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
    }

    fn record(&mut self, e: Edit) {
        let floor = self.floor();
        edit::push_edit(&mut self.edits, floor, e);
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn undo(&mut self) {
        if let Some((cfg, n)) = self.undo.pop() {
            self.work = Some(cfg);
            self.edits.truncate(n);
            self.drag = None;
            self.last_key = None;
            let alive: Vec<usize> = self.cur().map(|h| h.widgets.iter().map(|p| p.index).collect()).unwrap_or_default();
            self.sel.retain(|i| alive.contains(i));
        }
    }

    /// Drop pending changes, back to the file.
    pub(crate) fn revert(&mut self) {
        self.edits.clear();
        self.undo.clear();
        self.base = None;
        self.work = None;
        self.drag = None;
    }

    /// Snapshot then run `f` on the working copy (one undo step).
    fn step(&mut self, f: impl FnOnce(&mut Self)) {
        if let Some(w) = self.work.clone() {
            let n = self.edits.len();
            self.push_undo(w, n);
            self.last_key = None;
            f(self);
        }
    }

    /// Add the bound variable to the working copy (Explorer button; the
    /// file write goes through the tick right away).
    pub(crate) fn note_source(&mut self, name: &str, path: &str) {
        if let Some(w) = self.work.as_mut()
            && !w.sources.iter().any(|s| s.name == name)
            && let Some(d) = SourceDef::from_path(name, path, 1.0, 0.0)
        {
            w.sources.push(d);
        }
    }

    /// Move widgets (file indices) by HUD-unit deltas, recorded.
    fn move_by(&mut self, moves: &[(usize, Vec2)]) {
        let hud = self.hud;
        let Some(h) = self.work.as_mut().and_then(|w| w.huds.get_mut(hud)) else { return };
        let hidx = h.index;
        let mut out = Vec::new();
        for (i, d) in moves {
            if let Some(pw) = h.widgets.iter_mut().find(|p| p.index == *i) {
                let pos = edit::pos_mut(&mut pw.w);
                *pos = [round1(pos[0] + d.x), round1(pos[1] + d.y)];
                out.push(Edit::Widget { hud: hidx, widget: *i, key: "pos".into(), val: Val::Pair(*pos) });
            }
        }
        for e in out {
            self.record(e);
        }
    }

    fn selected_bounds(&self, p: &egui::Painter, v: &Values) -> Vec<(usize, Rect)> {
        let Some(h) = self.cur() else { return Vec::new() };
        h.widgets
            .iter()
            .filter(|pw| self.sel.contains(&pw.index))
            .map(|pw| (pw.index, widget_bounds(p, &pw.w, v)))
            .collect()
    }

    fn align_op(&mut self, p: &egui::Painter, v: &Values, op: AlignOp) {
        let b = self.selected_bounds(p, v);
        let Some(h) = self.cur() else { return };
        if b.is_empty() {
            return;
        }
        let reference = if b.len() == 1 {
            Rect::from_min_size(Pos2::ZERO, egui::vec2(h.size[0], h.size[1]))
        } else {
            b.iter().skip(1).fold(b[0].1, |a, (_, r)| a.union(*r))
        };
        let rects: Vec<Rect> = b.iter().map(|(_, r)| *r).collect();
        let d = edit::align(&rects, reference, op);
        let moves: Vec<(usize, Vec2)> = b.iter().map(|(i, _)| *i).zip(d).collect();
        self.step(|e| e.move_by(&moves));
    }

    fn distribute_op(&mut self, p: &egui::Painter, v: &Values, horizontal: bool) {
        let b = self.selected_bounds(p, v);
        let rects: Vec<Rect> = b.iter().map(|(_, r)| *r).collect();
        let d = edit::distribute(&rects, horizontal);
        let moves: Vec<(usize, Vec2)> = b.iter().map(|(i, _)| *i).zip(d).collect();
        self.step(|e| e.move_by(&moves));
    }

    fn add_widget(&mut self, fields: Vec<(String, Val)>) {
        let Some(w) = edit::widget_from_fields(&fields) else { return };
        let hud = self.hud;
        self.step(|e| {
            let Some(h) = e.work.as_mut().and_then(|c| c.huds.get_mut(hud)) else { return };
            let index = h.slots;
            h.slots += 1;
            h.widgets.push(Placed { w, alert: None, index });
            let hidx = h.index;
            e.record(Edit::AddWidget { hud: hidx, fields });
            e.sel = vec![index];
        });
    }

    fn duplicate(&mut self) {
        let hud = self.hud;
        let sel = self.sel.clone();
        if sel.is_empty() {
            return;
        }
        self.step(|e| {
            let Some(h) = e.work.as_mut().and_then(|c| c.huds.get_mut(hud)) else { return };
            let hidx = h.index;
            let mut new_sel = Vec::new();
            let mut out = Vec::new();
            for i in sel {
                let Some(src) = h.widgets.iter().find(|p| p.index == i).cloned() else { continue };
                let mut copy = src;
                let pos = edit::pos_mut(&mut copy.w);
                *pos = [pos[0] + 10.0, pos[1] + 10.0];
                copy.index = h.slots;
                h.slots += 1;
                new_sel.push(copy.index);
                h.widgets.push(copy);
                out.push(Edit::Duplicate { hud: hidx, widget: i, shift: [10.0, 10.0] });
            }
            for x in out {
                e.record(x);
            }
            e.sel = new_sel;
        });
    }

    fn delete(&mut self) {
        let hud = self.hud;
        let mut sel = self.sel.clone();
        if sel.is_empty() {
            return;
        }
        // Highest first: lower indices stay valid in file and copy.
        sel.sort_unstable_by(|a, b| b.cmp(a));
        self.step(|e| {
            let Some(h) = e.work.as_mut().and_then(|c| c.huds.get_mut(hud)) else { return };
            let hidx = h.index;
            let mut out = Vec::new();
            for i in sel {
                edit::delete_in_copy(&mut h.widgets, &mut h.slots, i);
                out.push(Edit::DeleteWidget { hud: hidx, widget: i });
            }
            for x in out {
                e.record(x);
            }
            e.sel.clear();
        });
    }

    /// Save: hand the log over (the copy stays as is until the file
    /// comes back through the reload).
    fn take_save(&mut self) -> Option<Vec<Edit>> {
        if self.edits.is_empty() {
            return None;
        }
        self.undo.clear();
        self.last_key = None;
        Some(std::mem::take(&mut self.edits))
    }
}

// --- On-screen interaction ----------------------------------------------------

/// What was pressed this frame on the HUDs.
struct Press {
    hud: usize,
    widget: Option<usize>,
    drag: bool,
}

impl Editor {
    /// Draw every enabled HUD (plus the edited one) in an interactive
    /// layer and handle selection, dragging and keys. `t` = time for
    /// blinking states.
    pub(crate) fn overlay(&mut self, ctx: &egui::Context, v: &Values, t: f64) {
        let Some(work) = self.work.take() else { return };
        let active: Vec<String> = work.states.iter().filter(|s| s.active(v, t)).map(|s| s.name.clone()).collect();
        let active: Vec<&str> = active.iter().map(String::as_str).collect();
        let screen = ctx.content_rect();
        let (pointer, origin, shift, down) = ctx.input(|i| {
            (i.pointer.interact_pos(), i.pointer.press_origin(), i.modifiers.shift, i.pointer.primary_down())
        });
        let mut press: Option<Press> = None;
        for (hi, h) in work.huds.iter().enumerate() {
            if !h.enabled && hi != self.hud {
                continue;
            }
            let rect = hud_rect(screen, h);
            let s = h.scale;
            let edited = hi == self.hud;
            egui::Area::new(egui::Id::new(("cxsfm_hud_edit", hi)))
                .order(egui::Order::Background)
                .fixed_pos(rect.min)
                .interactable(true)
                .show(ctx, |ui| {
                    let p = ui.painter().clone().with_clip_rect(screen);
                    let bg = ui.interact(rect, ui.id().with("bg"), egui::Sense::click_and_drag());
                    draw_hud(ctx, &p, h, v, &active);
                    let mut hit: Option<(usize, bool)> = None;
                    let mut boxes = Vec::new();
                    for pw in &h.widgets {
                        let sb = to_screen(rect, s, widget_bounds(&p, &pw.w, v));
                        let r = ui.interact(sb.expand(3.0), ui.id().with(("w", pw.index)), egui::Sense::click_and_drag());
                        if r.drag_started() || r.clicked() {
                            hit = Some((pw.index, r.drag_started()));
                        }
                        if r.hovered() {
                            ctx.set_cursor_icon(egui::CursorIcon::Move);
                        }
                        boxes.push((pw.index, sb));
                    }
                    if let Some((w, drag)) = hit {
                        press = Some(Press { hud: hi, widget: Some(w), drag });
                    } else if bg.drag_started() || bg.clicked() {
                        press = Some(Press { hud: hi, widget: None, drag: bg.drag_started() });
                    }
                    // Chrome: HUD box, widget boxes, selection, guides, grid.
                    let box_col = if edited { ACCENT.gamma_multiply(0.8) } else { Color32::from_white_alpha(60) };
                    p.extend(egui::Shape::dashed_line(
                        &[rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()],
                        egui::Stroke::new(1.0, box_col),
                        6.0,
                        4.0,
                    ));
                    if !edited {
                        return;
                    }
                    if self.grid {
                        let step = self.grid_step * s;
                        if step >= 6.0 {
                            let nx = (rect.width() / step) as usize;
                            let ny = (rect.height() / step) as usize;
                            if nx.saturating_mul(ny) <= 6000 {
                                for gx in 0..=nx {
                                    for gy in 0..=ny {
                                        let q = rect.min + egui::vec2(gx as f32 * step, gy as f32 * step);
                                        p.circle_filled(q, 0.8, Color32::from_white_alpha(70));
                                    }
                                }
                            }
                        }
                    }
                    for (i, sb) in &boxes {
                        let on = self.sel.contains(i);
                        let st = if on {
                            egui::Stroke::new(1.5, ACCENT)
                        } else {
                            egui::Stroke::new(1.0, Color32::from_white_alpha(45))
                        };
                        p.rect_stroke(*sb, 1.0, st, egui::StrokeKind::Outside);
                    }
                    let gs = egui::Stroke::new(1.0, ACCENT);
                    if let Some(x) = self.guides.0 {
                        let sx = rect.min.x + x * s;
                        p.line_segment([egui::pos2(sx, rect.top() - 30.0), egui::pos2(sx, rect.bottom() + 30.0)], gs);
                    }
                    if let Some(y) = self.guides.1 {
                        let sy = rect.min.y + y * s;
                        p.line_segment([egui::pos2(rect.left() - 30.0, sy), egui::pos2(rect.right() + 30.0, sy)], gs);
                    }
                });
        }
        self.work = Some(work);
        let measure = ctx.layer_painter(egui::LayerId::background());
        if let Some(pr) = press {
            if pr.hud != self.hud {
                self.hud = pr.hud;
                self.sel.clear();
            }
            match pr.widget {
                Some(i) => {
                    if shift && !pr.drag {
                        match self.sel.iter().position(|x| *x == i) {
                            Some(k) => {
                                self.sel.remove(k);
                            }
                            None => self.sel.push(i),
                        }
                    } else if !self.sel.contains(&i) {
                        if !shift {
                            self.sel.clear();
                        }
                        self.sel.push(i);
                    }
                    if pr.drag {
                        self.begin_widget_drag(&measure, v, origin.or(pointer));
                    }
                }
                None => {
                    if !shift {
                        self.sel.clear();
                    }
                    if pr.drag {
                        self.begin_hud_drag(screen, origin.or(pointer));
                    }
                }
            }
        }
        match (self.drag.clone(), pointer) {
            (Some(d), Some(at)) if down => self.update_drag(&d, at, screen),
            (Some(_), _) => {
                self.drag = None;
                self.guides = (None, None);
            }
            _ => {}
        }
        self.keys(ctx, &measure, v);
    }

    fn begin_widget_drag(&mut self, p: &egui::Painter, v: &Values, pointer: Option<Pos2>) {
        let (Some(start), Some(h)) = (pointer, self.cur()) else { return };
        let mut orig = Vec::new();
        let mut bbox: Option<Rect> = None;
        let (mut xs, mut ys) = {
            let (x, y) = edit::rect_lines(Rect::from_min_size(Pos2::ZERO, egui::vec2(h.size[0], h.size[1])));
            (x.to_vec(), y.to_vec())
        };
        for pw in &h.widgets {
            let b = widget_bounds(p, &pw.w, v);
            if self.sel.contains(&pw.index) {
                let mut w = pw.w.clone();
                orig.push((pw.index, *edit::pos_mut(&mut w)));
                bbox = Some(bbox.map_or(b, |a| a.union(b)));
            } else {
                let (x, y) = edit::rect_lines(b);
                xs.extend(x);
                ys.extend(y);
            }
        }
        let (Some(bbox), scale) = (bbox, h.scale) else { return };
        if let Some(w) = self.work.clone() {
            let n = self.edits.len();
            self.push_undo(w, n);
        }
        self.last_key = None;
        self.drag = Some(Drag { start, scale, kind: DragKind::Widgets { orig, bbox, xs, ys } });
    }

    fn begin_hud_drag(&mut self, screen: Rect, pointer: Option<Pos2>) {
        let (Some(start), Some(h)) = (pointer, self.cur()) else { return };
        let kind = DragKind::Hud { offset: h.offset, rect: hud_rect(screen, h) };
        let scale = h.scale;
        if let Some(w) = self.work.clone() {
            let n = self.edits.len();
            self.push_undo(w, n);
        }
        self.last_key = None;
        self.drag = Some(Drag { start, scale, kind });
    }

    fn update_drag(&mut self, d: &Drag, at: Pos2, screen: Rect) {
        let raw = at - d.start;
        match &d.kind {
            DragKind::Widgets { orig, bbox, xs, ys } => {
                let s = d.scale.max(0.01);
                let mut delta = raw / s;
                self.guides = (None, None);
                if self.magnet || self.grid {
                    let (mut cx, mut cy) = if self.magnet { (xs.clone(), ys.clone()) } else { (Vec::new(), Vec::new()) };
                    if self.grid {
                        let (gx, gy) = edit::grid_lines(bbox.translate(delta), self.grid_step);
                        cx.extend(gx);
                        cy.extend(gy);
                    }
                    let sn = edit::snap(*bbox, delta, &cx, &cy, SNAP_PX / s);
                    delta = sn.delta;
                    self.guides = (sn.guide_x, sn.guide_y);
                }
                // Absolute from the drag start (no drift), as moves.
                let Some(h) = self.cur() else { return };
                let moves: Vec<(usize, Vec2)> = orig
                    .iter()
                    .filter_map(|(i, o)| {
                        let pw = h.widgets.iter().find(|p| p.index == *i)?;
                        let mut w = pw.w.clone();
                        let now = *edit::pos_mut(&mut w);
                        Some((*i, egui::vec2(o[0] + delta.x - now[0], o[1] + delta.y - now[1])))
                    })
                    .collect();
                self.move_by(&moves);
            }
            DragKind::Hud { offset, rect } => {
                let mut delta = raw;
                self.guides = (None, None);
                if self.magnet {
                    let (xs, ys) = edit::rect_lines(screen);
                    delta = edit::snap(*rect, delta, &xs, &ys, SNAP_PX * 1.5).delta;
                }
                let hud = self.hud;
                let Some(h) = self.work.as_mut().and_then(|w| w.huds.get_mut(hud)) else { return };
                h.offset = [round1(offset[0] + delta.x), round1(offset[1] + delta.y)];
                let e = Edit::Hud { hud: h.index, key: "offset".into(), val: Val::Pair(h.offset) };
                self.record(e);
            }
        }
    }

    fn keys(&mut self, ctx: &egui::Context, p: &egui::Painter, v: &Values) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (cmd, shift, left, right, up, down, del, z, dup) = ctx.input(|i| {
            (
                i.modifiers.command,
                i.modifiers.shift,
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::Z),
                i.key_pressed(egui::Key::D),
            )
        });
        if cmd && z {
            self.undo();
            return;
        }
        if cmd && dup {
            self.duplicate();
            return;
        }
        if del {
            self.delete();
            return;
        }
        let step = if shift { 10.0 } else { 1.0 };
        let d = egui::vec2(
            if right { step } else if left { -step } else { 0.0 },
            if down { step } else if up { -step } else { 0.0 },
        );
        if d != Vec2::ZERO && !self.sel.is_empty() {
            let moves: Vec<(usize, Vec2)> = self.selected_bounds(p, v).iter().map(|(i, _)| (*i, d)).collect();
            self.step(|e| e.move_by(&moves));
        }
    }
}

// --- Panel (control window, Editor tab) ------------------------------------------

/// Property rows of one target (a widget, or the HUD itself): every
/// change mutates the working copy and records one edit.
struct Props<'a> {
    edits: &'a mut Vec<Edit>,
    floor: usize,
    hud: usize,
    widget: Option<usize>,
    names: &'a [String],
    values: &'a Values,
    code: &'a str,
    /// Key of the current run of changes (coalesces into one entry).
    last_key: Option<String>,
    /// Key changed this frame.
    changed: Option<String>,
}

impl Props<'_> {
    fn put(&mut self, key: &str, val: Val) {
        let e = match self.widget {
            Some(w) => Edit::Widget { hud: self.hud, widget: w, key: key.into(), val },
            None => Edit::Hud { hud: self.hud, key: key.into(), val },
        };
        // A new run starts a new entry (the undo step taken for it
        // must not see an older entry rewritten).
        let floor = if self.last_key.as_deref() == Some(key) { self.floor } else { self.edits.len() };
        edit::push_edit(self.edits, floor, e);
        self.changed = Some(key.to_string());
    }

    fn label(ui: &mut egui::Ui, text: &str) {
        ui.add_sized([108.0, 18.0], egui::Label::new(egui::RichText::new(text).color(TEXT_DIM)).truncate());
    }

    fn num(&mut self, ui: &mut egui::Ui, label: &str, key: &str, v: &mut f32, speed: f32, range: std::ops::RangeInclusive<f32>) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            if ui.add(egui::DragValue::new(v).speed(speed).range(range).max_decimals(2)).changed() {
                self.put(key, Val::Num(*v));
            }
        });
    }

    fn count(&mut self, ui: &mut egui::Ui, label: &str, key: &str, v: &mut u32, range: std::ops::RangeInclusive<u32>) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            if ui.add(egui::DragValue::new(v).range(range)).changed() {
                self.put(key, Val::Num(*v as f32));
            }
        });
    }

    fn pair(&mut self, ui: &mut egui::Ui, label: &str, key: &str, v: &mut [f32; 2], speed: f32) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            let a = ui.add(egui::DragValue::new(&mut v[0]).speed(speed).max_decimals(1)).changed();
            let b = ui.add(egui::DragValue::new(&mut v[1]).speed(speed).max_decimals(1)).changed();
            if a || b {
                self.put(key, Val::Pair(*v));
            }
        });
    }

    fn color(&mut self, ui: &mut egui::Ui, label: &str, key: &str, c: &mut Color32) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            if ui.color_edit_button_srgba(c).changed() {
                self.put(key, Val::Color(*c));
            }
        });
    }

    /// Optional colour: checkbox + picker; `off` = what "none" writes.
    fn opt_color(&mut self, ui: &mut egui::Ui, label: &str, key: &str, c: &mut Option<Color32>, off: Val, on_default: Color32) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            let mut on = c.is_some();
            if ui.checkbox(&mut on, "").changed() {
                *c = on.then_some(c.unwrap_or(on_default));
                match c {
                    Some(col) => self.put(key, Val::Color(*col)),
                    None => self.put(key, off.clone()),
                }
            }
            if let Some(col) = c
                && ui.color_edit_button_srgba(col).changed()
            {
                self.put(key, Val::Color(*col));
            }
        });
    }

    fn toggle(&mut self, ui: &mut egui::Ui, label: &str, key: &str, b: &mut bool) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            if ui.checkbox(b, "").changed() {
                self.put(key, Val::Bool(*b));
            }
        });
    }

    fn choice<T: Copy + PartialEq>(&mut self, ui: &mut egui::Ui, label: &str, key: &str, cur: &mut T, opts: &[(T, &str, &str)]) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            let shown = opts.iter().find(|o| o.0 == *cur).map_or("?", |o| o.2);
            let mut picked = None;
            egui::ComboBox::from_id_salt((key, self.widget, "choice")).selected_text(shown).show_ui(ui, |ui| {
                for (v, k, l) in opts {
                    if ui.selectable_label(*v == *cur, *l).clicked() {
                        picked = Some((*v, *k));
                    }
                }
            });
            if let Some((v, k)) = picked
                && v != *cur
            {
                *cur = v;
                self.put(key, Val::Str(k.to_string()));
            }
        });
    }

    /// Variable picker entries: `name  (value)`.
    fn var_label(&self, name: &str) -> String {
        match (self.values.get(name), self.values.text(name)) {
            (Some(x), _) => format!("{name}  ({x:.2})"),
            (None, Some(t)) => format!("{name}  \"{}\"", t.chars().take(16).collect::<String>()),
            _ => name.to_string(),
        }
    }

    /// Pick a variable name; `none` adds a "none" entry (`None` result).
    fn var_combo(&self, ui: &mut egui::Ui, id: (&str, &str), cur: Option<&str>, none: bool) -> Option<Option<String>> {
        let mut out = None;
        egui::ComboBox::from_id_salt((id.0, id.1, self.widget))
            .selected_text(cur.unwrap_or(ed(self.code, "none")))
            .width(150.0)
            .show_ui(ui, |ui| {
                if none && ui.selectable_label(cur.is_none(), ed(self.code, "none")).clicked() {
                    out = Some(None);
                }
                for n in self.names {
                    if ui.selectable_label(cur == Some(n.as_str()), self.var_label(n)).clicked() {
                        out = Some(Some(n.clone()));
                    }
                }
            });
        out
    }

    fn source(&mut self, ui: &mut egui::Ui, label: &str, key: &str, s: &mut String) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            if let Some(Some(n)) = self.var_combo(ui, (key, "src"), Some(s.as_str()), false) {
                *s = n;
                self.put(key, Val::Str(s.clone()));
            }
        });
    }

    fn opt_source(&mut self, ui: &mut egui::Ui, label: &str, key: &str, s: &mut Option<String>) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            if let Some(n) = self.var_combo(ui, (key, "osrc"), s.as_deref(), true) {
                *s = n;
                match s {
                    Some(n) => self.put(key, Val::Str(n.clone())),
                    None => self.put(key, Val::Remove),
                }
            }
        });
    }

    /// Number or variable (bounds, red zone). `optional`: "none" too.
    fn bound_row(&mut self, ui: &mut egui::Ui, label: &str, key: &str, b: &mut Option<Bound>, optional: bool) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            let number = ed(self.code, "number");
            let cur = match b {
                None => ed(self.code, "none").to_string(),
                Some(Bound::Num(_)) => number.to_string(),
                Some(Bound::Src(s)) => s.clone(),
            };
            let mut picked: Option<Option<Bound>> = None;
            egui::ComboBox::from_id_salt((key, "bound", self.widget)).selected_text(cur).width(130.0).show_ui(ui, |ui| {
                if optional && ui.selectable_label(b.is_none(), ed(self.code, "none")).clicked() {
                    picked = Some(None);
                }
                if ui.selectable_label(matches!(b, Some(Bound::Num(_))), number).clicked() {
                    picked = Some(Some(Bound::Num(0.0)));
                }
                for n in self.names {
                    if ui.selectable_label(matches!(b, Some(Bound::Src(s)) if s == n), self.var_label(n)).clicked() {
                        picked = Some(Some(Bound::Src(n.clone())));
                    }
                }
            });
            if let Some(nb) = picked {
                *b = nb;
                match b {
                    None => self.put(key, Val::Remove),
                    Some(Bound::Num(x)) => self.put(key, Val::Num(*x)),
                    Some(Bound::Src(s)) => self.put(key, Val::Str(s.clone())),
                }
            }
            if let Some(Bound::Num(x)) = b
                && ui.add(egui::DragValue::new(x).speed(0.01).max_decimals(3)).changed()
            {
                self.put(key, Val::Num(*x));
            }
        });
    }

    fn bound(&mut self, ui: &mut egui::Ui, label: &str, key: &str, b: &mut Bound) {
        let mut o = Some(b.clone());
        self.bound_row(ui, label, key, &mut o, false);
        if let Some(nb) = o {
            *b = nb;
        }
    }

    /// Template text + "insert a variable" picker (`{name}`).
    fn template(&mut self, ui: &mut egui::Ui, label: &str, key: &str, s: &mut String) {
        ui.horizontal(|ui| {
            Self::label(ui, label);
            if ui.add(egui::TextEdit::singleline(s).desired_width(170.0)).changed() {
                self.put(key, Val::Str(s.clone()));
            }
        });
        ui.horizontal(|ui| {
            Self::label(ui, "");
            let mut add = None;
            egui::ComboBox::from_id_salt((key, "insert", self.widget))
                .selected_text(ed(self.code, "insert_var"))
                .width(150.0)
                .show_ui(ui, |ui| {
                    for n in self.names {
                        if ui.selectable_label(false, self.var_label(n)).clicked() {
                            add = Some(n.clone());
                        }
                    }
                });
            if let Some(n) = add {
                s.push_str(&format!("{{{n}}}"));
                self.put(key, Val::Str(s.clone()));
            }
        });
    }
}

const ANCHORS: [(Anchor, &str, &str); 9] = [
    (Anchor::TopLeft, "top_left", "top_left"),
    (Anchor::Top, "top", "top"),
    (Anchor::TopRight, "top_right", "top_right"),
    (Anchor::Left, "left", "left"),
    (Anchor::Center, "center", "center"),
    (Anchor::Right, "right", "right"),
    (Anchor::BottomLeft, "bottom_left", "bottom_left"),
    (Anchor::Bottom, "bottom", "bottom"),
    (Anchor::BottomRight, "bottom_right", "bottom_right"),
];

fn hud_props(pr: &mut Props, ui: &mut egui::Ui, h: &mut Hud) {
    let c = pr.code;
    ui.horizontal(|ui| {
        Props::label(ui, ed(c, "name"));
        if ui.add(egui::TextEdit::singleline(&mut h.name).desired_width(170.0)).changed() {
            pr.put("name", Val::Str(h.name.clone()));
        }
    });
    pr.toggle(ui, ed(c, "enabled"), "enabled", &mut h.enabled);
    pr.choice(ui, ed(c, "anchor"), "anchor", &mut h.anchor, &ANCHORS);
    pr.pair(ui, ed(c, "offset"), "offset", &mut h.offset, 1.0);
    pr.pair(ui, ed(c, "size"), "size", &mut h.size, 1.0);
    pr.num(ui, ed(c, "scale"), "scale", &mut h.scale, 0.01, 0.1..=10.0);
    pr.num(ui, ed(c, "opacity"), "opacity", &mut h.opacity, 0.01, 0.0..=1.0);
    pr.color(ui, ed(c, "background"), "background", &mut h.background);
    pr.toggle(ui, ed(c, "hide_missing"), "hide_when_missing", &mut h.hide_when_missing);
}

const ALIGNS3: [(Align, &str, &str); 3] =
    [(Align::Left, "left", "left"), (Align::Center, "center", "center"), (Align::Right, "right", "right")];
const ALIGNS4: [(Align, &str, &str); 4] = [
    (Align::Left, "left", "left"),
    (Align::Center, "center", "center"),
    (Align::Right, "right", "right"),
    (Align::Justify, "justify", "justify"),
];
const FONTS: [(FontKind, &str, &str); 4] = [
    (FontKind::Sans, "sans", "sans"),
    (FontKind::Mono, "mono", "mono"),
    (FontKind::Serif, "serif", "serif"),
    (FontKind::Title, "title", "title"),
];

fn widget_props(pr: &mut Props, ui: &mut egui::Ui, pw: &mut Placed, states: &[String]) {
    let c = pr.code;
    kit::hint(ui, kind_name(&pw.w));
    pr.pair(ui, ed(c, "pos"), "pos", edit::pos_mut(&mut pw.w), 0.5);
    match &mut pw.w {
        Widget::Bar { source, from_right, rotation, opacity, min, max, redline, size, segments, color, redline_color, track_color, .. } => {
            pr.source(ui, ed(c, "value"), "source", source);
            pr.bound(ui, ed(c, "min"), "min", min);
            pr.bound(ui, ed(c, "max"), "max", max);
            pr.bound_row(ui, ed(c, "redline"), "redline", redline, true);
            pr.pair(ui, ed(c, "size"), "size", size, 0.5);
            pr.count(ui, ed(c, "segments"), "segments", segments, 0..=200);
            let mut fill = *from_right;
            pr.choice(ui, ed(c, "fill"), "fill", &mut fill, &[(false, "left", "left"), (true, "right", "right")]);
            *from_right = fill;
            pr.num(ui, ed(c, "rotation"), "rotation", rotation, 0.5, -360.0..=360.0);
            pr.num(ui, ed(c, "opacity"), "opacity", opacity, 0.01, 0.0..=1.0);
            pr.color(ui, ed(c, "color"), "color", color);
            pr.color(ui, ed(c, "track"), "track_color", track_color);
            pr.color(ui, ed(c, "redline_color"), "redline_color", redline_color);
        }
        Widget::Arc { source, min, max, redline, from_end, radius, thickness, start, end, rotation, filled, color, track_color, redline_color, opacity, .. } => {
            pr.opt_source(ui, ed(c, "value"), "source", source);
            if source.is_some() {
                pr.bound(ui, ed(c, "min"), "min", min);
                pr.bound(ui, ed(c, "max"), "max", max);
                pr.bound_row(ui, ed(c, "redline"), "redline", redline, true);
                let mut fe = *from_end;
                pr.choice(ui, ed(c, "fill"), "fill", &mut fe, &[(false, "start", "start"), (true, "end", "end")]);
                *from_end = fe;
            }
            pr.num(ui, ed(c, "radius"), "radius", radius, 0.5, 1.0..=2000.0);
            pr.num(ui, ed(c, "thickness"), "thickness", thickness, 0.2, 0.5..=1000.0);
            pr.num(ui, ed(c, "start"), "start", start, 1.0, -720.0..=720.0);
            pr.num(ui, ed(c, "end"), "end", end, 1.0, -720.0..=720.0);
            pr.num(ui, ed(c, "rotation"), "rotation", rotation, 0.5, -360.0..=360.0);
            pr.toggle(ui, ed(c, "filled"), "filled", filled);
            pr.num(ui, ed(c, "opacity"), "opacity", opacity, 0.01, 0.0..=1.0);
            pr.color(ui, ed(c, "color"), "color", color);
            pr.color(ui, ed(c, "track"), "track_color", track_color);
            pr.color(ui, ed(c, "redline_color"), "redline_color", redline_color);
        }
        Widget::Needle { source, min, max, redline, start, end, rotation, length, inner, tail, width, color, redline_color, opacity, .. } => {
            pr.source(ui, ed(c, "value"), "source", source);
            pr.bound(ui, ed(c, "min"), "min", min);
            pr.bound(ui, ed(c, "max"), "max", max);
            pr.bound_row(ui, ed(c, "redline"), "redline", redline, true);
            pr.num(ui, ed(c, "length"), "length", length, 0.5, 0.0..=2000.0);
            pr.num(ui, ed(c, "inner"), "inner", inner, 0.5, 0.0..=2000.0);
            pr.num(ui, ed(c, "tail"), "tail", tail, 0.5, 0.0..=2000.0);
            pr.num(ui, ed(c, "width"), "width", width, 0.1, 0.5..=200.0);
            pr.num(ui, ed(c, "start"), "start", start, 1.0, -720.0..=720.0);
            pr.num(ui, ed(c, "end"), "end", end, 1.0, -720.0..=720.0);
            pr.num(ui, ed(c, "rotation"), "rotation", rotation, 0.5, -360.0..=360.0);
            pr.num(ui, ed(c, "opacity"), "opacity", opacity, 0.01, 0.0..=1.0);
            pr.color(ui, ed(c, "color"), "color", color);
            pr.color(ui, ed(c, "redline_color"), "redline_color", redline_color);
        }
        Widget::Ticks { radius, start, end, rotation, count, minor, major_length, minor_length, color, minor_color, labels, label_min, label_max, label_size, label_color, opacity, .. } => {
            pr.num(ui, ed(c, "radius"), "radius", radius, 0.5, 1.0..=2000.0);
            pr.num(ui, ed(c, "start"), "start", start, 1.0, -720.0..=720.0);
            pr.num(ui, ed(c, "end"), "end", end, 1.0, -720.0..=720.0);
            pr.num(ui, ed(c, "rotation"), "rotation", rotation, 0.5, -360.0..=360.0);
            ui.horizontal(|ui| {
                Props::label(ui, ed(c, "count"));
                let mut auto = count.is_none();
                if ui.checkbox(&mut auto, "auto").changed() {
                    *count = if auto { None } else { Some(8) };
                    match count {
                        None => pr.put("count", Val::Str("auto".into())),
                        Some(n) => pr.put("count", Val::Num(*n as f32)),
                    }
                }
                if let Some(n) = count
                    && ui.add(egui::DragValue::new(n).range(1..=100)).changed()
                {
                    pr.put("count", Val::Num(*n as f32));
                }
            });
            pr.count(ui, ed(c, "minor"), "minor", minor, 1..=20);
            pr.num(ui, ed(c, "major_length"), "major_length", major_length, 0.5, 0.0..=500.0);
            pr.num(ui, ed(c, "minor_length"), "minor_length", minor_length, 0.5, 0.0..=500.0);
            pr.toggle(ui, ed(c, "labels"), "labels", labels);
            if *labels {
                pr.num(ui, ed(c, "label_min"), "label_min", label_min, 0.1, -1.0e6..=1.0e6);
                pr.bound(ui, ed(c, "label_max"), "label_max", label_max);
                pr.num(ui, ed(c, "label_size"), "label_size", label_size, 0.2, 4.0..=300.0);
                pr.color(ui, ed(c, "label_color"), "label_color", label_color);
            }
            pr.num(ui, ed(c, "opacity"), "opacity", opacity, 0.01, 0.0..=1.0);
            pr.color(ui, ed(c, "color"), "color", color);
            pr.color(ui, ed(c, "minor_color"), "minor_color", minor_color);
        }
        Widget::Text { text, size, align, color, mono, rotation, opacity, skew, glow, .. } => {
            pr.template(ui, ed(c, "text"), "text", text);
            pr.num(ui, ed(c, "font_size"), "size", size, 0.2, 4.0..=300.0);
            pr.choice(ui, ed(c, "align"), "align", align, &ALIGNS3);
            pr.toggle(ui, "mono", "mono", mono);
            pr.num(ui, ed(c, "rotation"), "rotation", rotation, 0.5, -360.0..=360.0);
            pr.num(ui, ed(c, "skew"), "skew", skew, 0.2, -45.0..=45.0);
            pr.num(ui, ed(c, "glow"), "glow", glow, 0.1, 0.0..=40.0);
            pr.num(ui, ed(c, "opacity"), "opacity", opacity, 0.01, 0.0..=1.0);
            pr.color(ui, ed(c, "color"), "color", color);
        }
        Widget::Label { text, font, bold, size, align, color, rotation, opacity, letter_spacing, max_width, outline, outline_color, glow, background, .. } => {
            pr.template(ui, ed(c, "text"), "text", text);
            pr.choice(ui, ed(c, "font"), "font", font, &FONTS);
            pr.num(ui, ed(c, "font_size"), "size", size, 0.2, 4.0..=300.0);
            pr.choice(ui, ed(c, "align"), "align", align, &ALIGNS4);
            pr.num(ui, ed(c, "max_width"), "max_width", max_width, 1.0, 0.0..=10_000.0);
            if *align == Align::Justify && *max_width <= 0.0 {
                kit::note(ui, ed(c, "justify_hint"));
            }
            pr.toggle(ui, ed(c, "bold"), "bold", bold);
            pr.num(ui, ed(c, "letter_spacing"), "letter_spacing", letter_spacing, 0.1, -20.0..=100.0);
            pr.num(ui, ed(c, "outline"), "outline", outline, 0.1, 0.0..=20.0);
            if *outline > 0.0 {
                pr.color(ui, ed(c, "outline_color"), "outline_color", outline_color);
            }
            pr.num(ui, ed(c, "glow"), "glow", glow, 0.1, 0.0..=40.0);
            pr.opt_color(ui, ed(c, "background"), "background", background, Val::Remove, Color32::from_black_alpha(160));
            pr.num(ui, ed(c, "rotation"), "rotation", rotation, 0.5, -360.0..=360.0);
            pr.num(ui, ed(c, "opacity"), "opacity", opacity, 0.01, 0.0..=1.0);
            pr.color(ui, ed(c, "color"), "color", color);
        }
        Widget::Shape { kind, size, rotation, corner, fill, stroke, stroke_color, opacity, .. } => {
            let line = *kind == ShapeKind::Line;
            pr.pair(ui, ed(c, if line { "vector" } else { "size" }), "size", size, 0.5);
            if *kind == ShapeKind::Rect {
                pr.num(ui, ed(c, "corner"), "corner", corner, 0.2, 0.0..=1000.0);
            }
            if !line {
                pr.opt_color(ui, ed(c, "fill_color"), "fill", fill, Val::Str("none".into()), Color32::WHITE);
            }
            pr.num(ui, ed(c, if line { "thickness" } else { "stroke" }), "stroke", stroke, 0.1, 0.0..=200.0);
            pr.color(ui, ed(c, "stroke_color"), "stroke_color", stroke_color);
            pr.num(ui, ed(c, "rotation"), "rotation", rotation, 0.5, -360.0..=360.0);
            pr.num(ui, ed(c, "opacity"), "opacity", opacity, 0.01, 0.0..=1.0);
        }
    }
    // Shared alert colour (all widget types).
    if !states.is_empty() {
        ui.separator();
        ui.horizontal(|ui| {
            Props::label(ui, ed(c, "alert"));
            let cur = pw.alert.as_ref().map(|a| a.state.clone());
            let mut picked: Option<Option<String>> = None;
            egui::ComboBox::from_id_salt(("alert", pw.index)).selected_text(cur.clone().unwrap_or_else(|| ed(c, "none").into())).show_ui(ui, |ui| {
                if ui.selectable_label(cur.is_none(), ed(c, "none")).clicked() {
                    picked = Some(None);
                }
                for s in states {
                    if ui.selectable_label(cur.as_deref() == Some(s), s).clicked() {
                        picked = Some(Some(s.clone()));
                    }
                }
            });
            if let Some(p) = picked {
                match p {
                    None => {
                        pw.alert = None;
                        pr.put("alert", Val::Remove);
                    }
                    Some(s) => {
                        let color = pw.alert.as_ref().map_or(Color32::from_rgb(0xE0, 0x30, 0x30), |a| a.color);
                        pw.alert = Some(Alert { state: s.clone(), color });
                        pr.put("alert", Val::Str(s));
                    }
                }
            }
        });
        if let Some(a) = pw.alert.as_mut() {
            pr.color(ui, ed(c, "alert_color"), "alert_color", &mut a.color);
        }
    }
}

impl Editor {
    /// Control-window tab. `values` = what the HUDs show this frame.
    pub(crate) fn panel(&mut self, ui: &mut egui::Ui, values: &Values, code: &str, save_status: Option<&str>) -> PanelOut {
        let mut out = PanelOut::None;
        let Some(work) = self.work.as_ref() else {
            kit::hint(ui, ed(code, "no_layout"));
            return out;
        };
        if work.huds.is_empty() {
            kit::hint(ui, ed(code, "no_hud"));
            return out;
        }
        let measure = ui.painter().clone();
        let names = edit::variable_names(work, values);
        let states: Vec<String> = work.states.iter().map(|s| s.name.clone()).collect();
        let hud_names: Vec<String> = work.huds.iter().map(|h| h.name.clone()).collect();
        let shapes_ok = !names.is_empty();
        let sel_n = self.sel.len();
        // Row 1: HUD + save / undo / revert.
        ui.horizontal(|ui| {
            let mut hud = self.hud;
            egui::ComboBox::from_id_salt("hud_edit_pick")
                .selected_text(hud_names.get(hud).cloned().unwrap_or_default())
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (i, n) in hud_names.iter().enumerate() {
                        ui.selectable_value(&mut hud, i, n);
                    }
                });
            if hud != self.hud {
                self.hud = hud;
                self.sel.clear();
            }
            let pending = self.edits.len();
            let save = format!("{} ({pending})", ed(code, "save"));
            if ui.add_enabled(pending > 0, egui::Button::new(save)).clicked()
                && let Some(e) = self.take_save()
            {
                out = PanelOut::Save(e);
            }
            if ui.add_enabled(self.can_undo(), egui::Button::new(ic::ICON_UNDO.rich_text())).on_hover_text(ed(code, "undo")).clicked() {
                self.undo();
            }
            if ui.add_enabled(pending > 0, egui::Button::new(ic::ICON_CANCEL.rich_text())).on_hover_text(ed(code, "revert")).clicked() {
                self.revert();
            }
        });
        // Row 2: add shapes / widgets.
        ui.horizontal(|ui| {
            kit::hint(ui, ed(code, "add"));
            let Some(h) = self.cur() else { return };
            let c = [h.size[0] * 0.5, h.size[1] * 0.5];
            let s = |x: &str| Val::Str(x.to_string());
            let first = names.first().cloned().unwrap_or_default();
            let pick = |want: &str| if names.iter().any(|n| n == want) { want.to_string() } else { first.clone() };
            let items: [AddItem; 6] = [
                (ic::ICON_CROP_SQUARE, "add_rect", true, vec![
                    ("type".into(), s("rect")), ("pos".into(), Val::Pair([c[0] - 40.0, c[1] - 20.0])),
                    ("size".into(), Val::Pair([80.0, 40.0])), ("corner".into(), Val::Num(4.0)), ("fill".into(), s("#FFFFFF40")),
                ]),
                (ic::ICON_RADIO_BUTTON_UNCHECKED, "add_ellipse", true, vec![
                    ("type".into(), s("ellipse")), ("pos".into(), Val::Pair([c[0] - 30.0, c[1] - 30.0])),
                    ("size".into(), Val::Pair([60.0, 60.0])), ("fill".into(), s("#FFFFFF40")),
                ]),
                (ic::ICON_HORIZONTAL_RULE, "add_line", true, vec![
                    ("type".into(), s("line")), ("pos".into(), Val::Pair([c[0] - 50.0, c[1]])),
                    ("size".into(), Val::Pair([100.0, 0.0])), ("stroke".into(), Val::Num(2.0)),
                ]),
                (ic::ICON_TEXT_FIELDS, "add_text", true, vec![
                    ("type".into(), s("label")), ("text".into(), s(&if shapes_ok { format!("{{{}}}", pick("speed")) } else { "Text".into() })),
                    ("pos".into(), Val::Pair(c)), ("size".into(), Val::Num(24.0)), ("align".into(), s("center")),
                ]),
                (ic::ICON_DONUT_LARGE, "add_arc", true, vec![
                    ("type".into(), s("arc")), ("pos".into(), Val::Pair(c)), ("radius".into(), Val::Num(40.0)),
                    ("thickness".into(), Val::Num(6.0)),
                ]),
                (ic::ICON_LINEAR_SCALE, "add_bar", shapes_ok, vec![
                    ("type".into(), s("bar")), ("source".into(), s(&pick("rpm"))),
                    ("pos".into(), Val::Pair([c[0] - 80.0, c[1] - 4.0])), ("size".into(), Val::Pair([160.0, 8.0])),
                ]),
            ];
            let mut add = None;
            for (icon, key, ok, fields) in items {
                if ui.add_enabled(ok, egui::Button::new(icon.rich_text())).on_hover_text(ed(code, key)).clicked() {
                    add = Some(fields);
                }
            }
            if let Some(f) = add {
                self.add_widget(f);
            }
        });
        // Row 3: align / distribute / duplicate / delete, magnet + grid.
        ui.horizontal(|ui| {
            let one = sel_n >= 1;
            let ops = [
                (ic::ICON_ALIGN_HORIZONTAL_LEFT, "al_left", AlignOp::Left),
                (ic::ICON_ALIGN_HORIZONTAL_CENTER, "al_hcenter", AlignOp::HCenter),
                (ic::ICON_ALIGN_HORIZONTAL_RIGHT, "al_right", AlignOp::Right),
                (ic::ICON_ALIGN_VERTICAL_TOP, "al_top", AlignOp::Top),
                (ic::ICON_ALIGN_VERTICAL_CENTER, "al_vcenter", AlignOp::VCenter),
                (ic::ICON_ALIGN_VERTICAL_BOTTOM, "al_bottom", AlignOp::Bottom),
            ];
            for (icon, key, op) in ops {
                if ui.add_enabled(one, egui::Button::new(icon.rich_text())).on_hover_text(ed(code, key)).clicked() {
                    self.align_op(&measure, values, op);
                }
            }
            if ui.add_enabled(sel_n >= 3, egui::Button::new(ic::ICON_HORIZONTAL_DISTRIBUTE.rich_text())).on_hover_text(ed(code, "dist_h")).clicked() {
                self.distribute_op(&measure, values, true);
            }
            if ui.add_enabled(sel_n >= 3, egui::Button::new(ic::ICON_VERTICAL_DISTRIBUTE.rich_text())).on_hover_text(ed(code, "dist_v")).clicked() {
                self.distribute_op(&measure, values, false);
            }
            if ui.add_enabled(one, egui::Button::new(ic::ICON_CONTENT_COPY.rich_text())).on_hover_text(ed(code, "duplicate")).clicked() {
                self.duplicate();
            }
            if ui.add_enabled(one, egui::Button::new(ic::ICON_DELETE.rich_text())).on_hover_text(ed(code, "delete")).clicked() {
                self.delete();
            }
            ui.separator();
            ui.checkbox(&mut self.magnet, ed(code, "magnet"));
            ui.checkbox(&mut self.grid, ic::ICON_GRID_ON.rich_text()).on_hover_text(ed(code, "grid"));
            if self.grid {
                ui.add(egui::DragValue::new(&mut self.grid_step).range(2.0..=100.0).speed(0.2));
            }
        });
        match save_status {
            Some(s) => kit::note(ui, s),
            None => kit::note(ui, ed(code, "help")),
        }
        ui.separator();
        // Panes: widget list | properties.
        let floor = self.floor();
        let snapshot = self.work.clone();
        let edits_before = self.edits.len();
        let mut changed: Option<String> = None;
        let mut pick: Option<(usize, bool)> = None;
        let hud = self.hud;
        let sel = self.sel.clone();
        let last_key = self.last_key.clone();
        let shift = ui.input(|i| i.modifiers.shift);
        let edits = &mut self.edits;
        let Some(h) = self.work.as_mut().and_then(|w| w.huds.get_mut(hud)) else { return out };
        kit::two_panes(ui, "hud_editor", 170.0, |ui, pane| {
            if pane == Pane::Left {
                let on = sel.is_empty();
                if ui.selectable_label(on, egui::RichText::new(format!("HUD · {}", h.name)).strong()).clicked() {
                    pick = Some((usize::MAX, false));
                }
                for pw in &h.widgets {
                    let hint = kit::short(&kind_hint(&pw.w), 14);
                    let text = format!("{}  {}", kind_name(&pw.w), hint);
                    if ui.selectable_label(sel.contains(&pw.index), text).clicked() {
                        pick = Some((pw.index, shift));
                    }
                }
                return;
            }
            let hidx = h.index;
            let mut pr = Props { edits, floor, hud: hidx, widget: None, names: &names, values, code, last_key: last_key.clone(), changed: None };
            match sel.as_slice() {
                [] => hud_props(&mut pr, ui, h),
                [one] => match h.widgets.iter_mut().find(|p| p.index == *one) {
                    Some(pw) => {
                        pr.widget = Some(pw.index);
                        widget_props(&mut pr, ui, pw, &states);
                    }
                    None => kit::hint(ui, ed(code, "select")),
                },
                many => kit::hint(ui, &ed(code, "multi").replace("{n}", &many.len().to_string())),
            }
            if pr.changed.is_some() {
                changed = pr.changed;
            }
        });
        if let Some((i, add)) = pick {
            if i == usize::MAX {
                self.sel.clear();
            } else if add {
                match self.sel.iter().position(|x| *x == i) {
                    Some(k) => {
                        self.sel.remove(k);
                    }
                    None => self.sel.push(i),
                }
            } else {
                self.sel = vec![i];
            }
        }
        // One undo step per run of changes to the same key.
        match changed {
            Some(k) => {
                if self.last_key.as_deref() != Some(k.as_str())
                    && let Some(s) = snapshot
                {
                    self.push_undo(s, edits_before);
                }
                self.last_key = Some(k);
            }
            None => {
                if !ui.input(|i| i.pointer.any_down()) {
                    self.last_key = None;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_hud::model::parse_config;

    /// One HUD at the top-left corner, scale 2: a text and a rect.
    const CFG: &str = r##"
version = 2
mode = "demo"
[[hud]]
name = "T"
anchor = "top_left"
offset = [100.0, 100.0]
size = [200.0, 100.0]
scale = 2.0
  [[hud.widget]]
  type = "rect"
  pos = [20.0, 20.0]
  size = [40.0, 20.0]
  [[hud.widget]]
  type = "ellipse"
  pos = [120.0, 50.0]
  size = [20.0, 20.0]
"##;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        egui_material_icons::initialize(&ctx);
        ctx
    }

    fn frame(ctx: &egui::Context, ed: &mut Editor, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            events,
            ..Default::default()
        };
        ctx.begin_pass(input);
        ed.overlay(ctx, &Values::default(), 0.0);
        let mut out = ctx.end_pass();
        out.textures_delta.clear();
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// Press on the rect (screen = 100 + 2 × local), move, release.
    fn drag(ed: &mut Editor, by: Vec2) {
        let ctx = ctx();
        let at = egui::pos2(100.0 + 2.0 * 40.0, 100.0 + 2.0 * 30.0);
        for _ in 0..3 {
            frame(&ctx, ed, vec![]);
        }
        frame(&ctx, ed, vec![egui::Event::PointerMoved(at)]);
        frame(&ctx, ed, vec![button(at, true)]);
        for k in 1..=4 {
            frame(&ctx, ed, vec![egui::Event::PointerMoved(at + by * (k as f32 / 4.0))]);
        }
        frame(&ctx, ed, vec![button(at + by, false)]);
        frame(&ctx, ed, vec![]);
    }

    fn editor() -> Editor {
        let (cfg, w) = parse_config(CFG).unwrap();
        assert!(w.is_empty(), "{w:?}");
        Editor { work: Some(cfg), ..Editor::default() }
    }

    fn rect_pos(ed: &Editor) -> [f32; 2] {
        let mut w = ed.work.as_ref().unwrap().huds[0].widgets[0].w.clone();
        *edit::pos_mut(&mut w)
    }

    #[test]
    fn drag_moves_selects_and_records() {
        let mut ed = editor();
        ed.magnet = false;
        drag(&mut ed, egui::vec2(60.0, 20.0));
        assert_eq!(ed.sel, vec![0]);
        // 60 px on screen at scale 2 = 30 HUD units.
        assert_eq!(rect_pos(&ed), [50.0, 30.0]);
        assert_eq!(
            ed.edits,
            vec![Edit::Widget { hud: 0, widget: 0, key: "pos".into(), val: Val::Pair([50.0, 30.0]) }]
        );
        assert!(ed.can_undo());
        ed.undo();
        assert_eq!(rect_pos(&ed), [20.0, 20.0]);
        assert!(ed.edits.is_empty());
    }

    #[test]
    fn magnet_snaps_on_the_hud_centre() {
        let mut ed = editor();
        // Rect centre x = 40; the box centre is 100: move by ~58 units
        // (116 px) — the magnet pulls the last 2 units.
        drag(&mut ed, egui::vec2(116.0, 0.0));
        assert_eq!(rect_pos(&ed), [80.0, 20.0]);
        assert!(ed.drag.is_none());
        assert_eq!(ed.guides, (None, None), "guides cleared on release");
    }

    #[test]
    fn add_duplicate_delete_keep_file_indices() {
        let mut ed = editor();
        ed.add_widget(vec![("type".into(), Val::Str("line".into())), ("pos".into(), Val::Pair([1.0, 1.0]))]);
        assert_eq!(ed.sel, vec![2]);
        ed.duplicate();
        assert_eq!(ed.sel, vec![3]);
        ed.sel = vec![0, 2];
        ed.delete();
        let h = &ed.work.as_ref().unwrap().huds[0];
        let idx: Vec<usize> = h.widgets.iter().map(|p| p.index).collect();
        assert_eq!(idx, vec![0, 1]);
        assert_eq!(h.slots, 2);
        // Same log on the file gives the same layout.
        let (text, w) = edit::apply_edits(CFG, &ed.edits).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let (back, _) = parse_config(&text).unwrap();
        let kinds = |c: &HudConfig| c.huds[0].widgets.iter().map(|p| kind_name(&p.w)).collect::<Vec<_>>();
        assert_eq!(kinds(&back), kinds(ed.work.as_ref().unwrap()));
        assert_eq!(kinds(&back), vec!["ellipse", "line"]);
    }
}
