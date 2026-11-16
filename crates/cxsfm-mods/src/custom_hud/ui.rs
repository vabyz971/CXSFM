//! HUD rendering (present thread) + control window.
//!
//! Pure egui: no Unity call, no lock held beyond cloning the published
//! `Arc<HudConfig>`. HUDs paint on a background layer (under every
//! window, never interactive), so they stay on screen with the menu
//! closed (the mod reports itself pinned) without touching input.

use super::HudMod;
use super::model::{
    Align, ExploreReq, FontKind, Hud, HudConfig, Mode, ShapeKind, Values, Widget, shape_outline, shape_text, arc_point, arc_points, bar_fraction,
    apply_derived, binding_path, demo_values, format_template, rotate_about, segment_height, tick_label,
    tick_layout, ticks_range, with_main_color,
};
use super::editor::PanelOut;
use super::{TAB_EDIT, TAB_EXPLORE};
use cxsfm_api::common::lock;
use cxsfm_ui::kit;
use std::sync::atomic::Ordering;

/// Screen-space box of a HUD (anchor point + offset, scaled size).
pub(crate) fn hud_rect(screen: egui::Rect, h: &Hud) -> egui::Rect {
    let (fx, fy) = h.anchor.factors();
    let size = egui::vec2(h.size[0] * h.scale, h.size[1] * h.scale);
    let at = egui::pos2(
        screen.min.x + fx * screen.width() + h.offset[0],
        screen.min.y + fy * screen.height() + h.offset[1],
    );
    egui::Rect::from_min_size(egui::pos2(at.x - fx * size.x, at.y - fy * size.y), size)
}

/// One arc segment `a0 → a1` (degrees, 0 = top, clockwise): a ring of
/// width `th` (anti-aliased polyline), or a filled pie/disc. Pies use a
/// triangle fan so spans past 180° stay correct (a convex polygon
/// would not).
fn draw_arc(p: &egui::Painter, c: egui::Pos2, r: f32, th: f32, a0: f32, a1: f32, filled: bool, col: egui::Color32) {
    let span = (a1 - a0).abs();
    if col.a() == 0 || !(span > 0.01) {
        return;
    }
    let full = span >= 359.9;
    if filled && full {
        p.circle_filled(c, r, col);
        return;
    }
    if !filled && full {
        p.circle_stroke(c, r, egui::Stroke::new(th, col));
        return;
    }
    let pts: Vec<egui::Pos2> = arc_points([c.x, c.y], r, a0, a1)
        .into_iter()
        .map(|q| egui::pos2(q[0], q[1]))
        .collect();
    if pts.len() < 2 {
        return;
    }
    if filled {
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(c, col);
        for q in &pts {
            mesh.colored_vertex(*q, col);
        }
        for i in 1..pts.len() as u32 {
            mesh.add_triangle(0, i, i + 1);
        }
        p.add(egui::Shape::mesh(mesh));
    } else {
        p.add(egui::Shape::line(pts, egui::Stroke::new(th, col)));
    }
}

fn pt(q: [f32; 2]) -> egui::Pos2 {
    egui::pos2(q[0], q[1])
}

/// Filled rect turned `deg` clockwise around `pivot` (axis-aligned
/// fast path when unrotated).
fn fill_rotated(p: &egui::Painter, r: egui::Rect, pivot: egui::Pos2, deg: f32, col: egui::Color32) {
    if col.a() == 0 || r.width() <= 0.0 || r.height() <= 0.0 {
        return;
    }
    if deg == 0.0 {
        p.rect_filled(r, 0.0, col);
        return;
    }
    let corners: Vec<egui::Pos2> = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
        .into_iter()
        .map(|q| pt(rotate_about([q.x, q.y], [pivot.x, pivot.y], deg)))
        .collect();
    p.add(egui::Shape::convex_polygon(corners, col, egui::Stroke::NONE));
}

/// Text as a raw mesh (font texture), sheared by `skew` degrees for an
/// italic look (bottom edge fixed, top leaning right), turned `deg`
/// around `anchor_local` (galley space) and placed at `top_left +
/// shift`. Mirrors epaint's own text tessellation (row meshes carry
/// texel UVs: normalise by the font atlas size).
#[allow(clippy::too_many_arguments)]
fn text_mesh(
    galley: &egui::Galley,
    top_left: egui::Pos2,
    anchor_local: egui::Vec2,
    deg: f32,
    skew: f32,
    color: egui::Color32,
    tex: [usize; 2],
    shift: egui::Vec2,
) -> egui::Mesh {
    let k = skew.to_radians().tan();
    let h = galley.size().y;
    let norm = egui::vec2(1.0 / tex[0].max(1) as f32, 1.0 / tex[1].max(1) as f32);
    let mut mesh = egui::Mesh::default();
    for placed in &galley.rows {
        let src = &placed.row.visuals.mesh;
        let base = mesh.vertices.len() as u32;
        mesh.indices.extend(src.indices.iter().map(|i| i + base));
        for v in &src.vertices {
            let mut lp = placed.pos.to_vec2() + v.pos.to_vec2();
            lp.x += (h - lp.y) * k;
            let r = rotate_about([lp.x, lp.y], [anchor_local.x, anchor_local.y], deg);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: top_left + egui::vec2(r[0], r[1]) + shift,
                uv: egui::pos2(v.uv.x * norm.x, v.uv.y * norm.y),
                color,
            });
        }
    }
    mesh
}

/// Background layer of HUD `idx` (under every window, never interactive).
fn hud_painter(ctx: &egui::Context, idx: usize) -> egui::Painter {
    ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new(("cxsfm_hud", idx))))
}

/// Paint HUD `h` with `p` (its own background layer, or the editor's
/// interactive layer). `active` = names of the `[states]` entries
/// showing this frame.
pub(crate) fn draw_hud(ctx: &egui::Context, p: &egui::Painter, h: &Hud, v: &Values, active: &[&str]) {
    let screen = ctx.content_rect();
    let rect = hud_rect(screen, h);
    let mut p = p.clone();
    p.set_opacity(h.opacity);
    if h.background.a() > 0 {
        p.rect_filled(rect, 3.0, h.background);
    }
    let s = h.scale;
    let at = |pos: [f32; 2]| rect.min + egui::vec2(pos[0] * s, pos[1] * s);
    for pw in &h.widgets {
        // Shared colour: the widget's alert state is showing → its main
        // colour switches for this frame.
        let alerted;
        let w = match &pw.alert {
            Some(a) if active.contains(&a.state.as_str()) => {
                alerted = with_main_color(&pw.w, a.color);
                &alerted
            }
            _ => &pw.w,
        };
        match w {
            Widget::Bar {
                source,
                from_right,
                rotation,
                opacity,
                taper,
                taper_curve,
                min,
                max,
                redline,
                pos,
                size,
                segments,
                color,
                redline_color,
                track_color,
            } => {
                let pivot = at(*pos);
                let r = egui::Rect::from_min_size(pivot, egui::vec2(size[0] * s, size[1] * s));
                let k = |col: egui::Color32| col.gamma_multiply(*opacity);
                let fill_r = |rect: egui::Rect, col: egui::Color32| fill_rotated(&p, rect, pivot, *rotation, k(col));
                // Source or bound unavailable: skip this widget.
                let (Some(value), Some(min), Some(max)) = (v.get(source), min.eval(v), max.eval(v)) else {
                    continue;
                };
                let frac = bar_fraction(value, min, max);
                let redline = redline.as_ref().and_then(|b| b.eval(v));
                let red_frac = redline.map(|rl| bar_fraction(rl, min, max));
                let hot = redline.is_some_and(|rl| value >= rl);
                let fill = if hot { *redline_color } else { *color };
                if *segments == 0 {
                    fill_r(r, *track_color);
                    // Redline zone hinted on the track (dim, always on).
                    if let Some(rf) = red_frac {
                        let z = egui::Rect::from_min_max(
                            egui::pos2(r.min.x + r.width() * rf, r.min.y),
                            r.max,
                        );
                        fill_r(z, redline_color.gamma_multiply(0.35));
                    }
                    let x = r.min.x + r.width() * frac;
                    let f = if *from_right {
                        egui::Rect::from_min_max(egui::pos2(x, r.min.y), r.max)
                    } else {
                        egui::Rect::from_min_max(r.min, egui::pos2(x, r.max.y))
                    };
                    fill_r(f, fill);
                } else {
                    let n = *segments as f32;
                    let gap = 2.0 * s;
                    let seg_w = ((r.width() - gap * (n - 1.0)) / n).max(1.0);
                    for i in 0..*segments {
                        let x = r.min.x + i as f32 * (seg_w + gap);
                        // Ramp: bottom-aligned, height per taper curve.
                        let hf = taper.map(|t| segment_height(i, *segments, t, *taper_curve)).unwrap_or(1.0);
                        let sr = egui::Rect::from_min_max(
                            egui::pos2(x, r.max.y - r.height() * hf),
                            egui::pos2(x + seg_w, r.max.y),
                        );
                        let center = (i as f32 + 0.5) / n;
                        let in_red = red_frac.is_some_and(|rf| center >= rf);
                        let lit = if *from_right { center >= frac } else { center <= frac };
                        let c = if lit {
                            if in_red { *redline_color } else { fill }
                        } else if in_red {
                            redline_color.gamma_multiply(0.35)
                        } else {
                            *track_color
                        };
                        fill_r(sr, c);
                    }
                }
            }
            Widget::Arc {
                source,
                min,
                max,
                redline,
                from_end,
                pos,
                radius,
                thickness,
                start,
                end,
                rotation,
                filled,
                color,
                track_color,
                redline_color,
                opacity,
            } => {
                let c = at(*pos);
                let (r, th) = (radius * s, thickness * s);
                let (a0, a1) = (start + rotation, end + rotation);
                let k = |col: egui::Color32| col.gamma_multiply(*opacity);
                let Some(src) = source else {
                    draw_arc(&p, c, r, th, a0, a1, *filled, k(*color));
                    continue;
                };
                let (Some(value), Some(min), Some(max)) = (v.get(src), min.eval(v), max.eval(v)) else {
                    continue;
                };
                let frac = bar_fraction(value, min, max);
                let redline = redline.as_ref().and_then(|b| b.eval(v));
                let hot = redline.is_some_and(|rl| value >= rl);
                draw_arc(&p, c, r, th, a0, a1, *filled, k(*track_color));
                if let Some(rf) = redline.map(|rl| bar_fraction(rl, min, max)) {
                    let ar = a0 + (a1 - a0) * rf;
                    draw_arc(&p, c, r, th, ar, a1, *filled, k(redline_color.gamma_multiply(0.35)));
                }
                let av = a0 + (a1 - a0) * frac;
                let fill = k(if hot { *redline_color } else { *color });
                if *from_end {
                    draw_arc(&p, c, r, th, av, a1, *filled, fill);
                } else {
                    draw_arc(&p, c, r, th, a0, av, *filled, fill);
                }
            }
            Widget::Needle {
                source,
                min,
                max,
                redline,
                pos,
                start,
                end,
                rotation,
                length,
                inner,
                tail,
                width,
                color,
                redline_color,
                opacity,
            } => {
                let (Some(value), Some(min), Some(max)) = (v.get(source), min.eval(v), max.eval(v)) else {
                    continue;
                };
                let c = at(*pos);
                let cc = [c.x, c.y];
                let (a0, a1) = (start + rotation, end + rotation);
                let angle = a0 + (a1 - a0) * bar_fraction(value, min, max);
                let hot = redline.as_ref().and_then(|b| b.eval(v)).is_some_and(|rl| value >= rl);
                let col = (if hot { *redline_color } else { *color }).gamma_multiply(*opacity);
                let stroke = egui::Stroke::new(width * s, col);
                p.line_segment([pt(arc_point(cc, inner * s, angle)), pt(arc_point(cc, length * s, angle))], stroke);
                if *tail > 0.0 {
                    p.line_segment([c, pt(arc_point(cc, tail * s, angle + 180.0))], stroke);
                }
            }
            Widget::Ticks {
                pos,
                radius,
                start,
                end,
                rotation,
                count,
                minor,
                major_length,
                minor_length,
                major_width,
                minor_width,
                color,
                minor_color,
                labels,
                label_min,
                label_max,
                label_margin,
                label_decimals,
                label_radius,
                label_size,
                label_color,
                opacity,
            } => {
                // Auto range (label_max from a source + margin, count
                // following it) or fixed; missing source = skip.
                let Some((count, label_hi)) = ticks_range(*count, *label_min, label_max, *label_margin, v) else {
                    continue;
                };
                let c = at(*pos);
                let cc = [c.x, c.y];
                let k = |col: egui::Color32| col.gamma_multiply(*opacity);
                let label_r = label_radius.unwrap_or(radius - major_length - label_size * 0.9);
                for t in tick_layout(start + rotation, end + rotation, count, *minor) {
                    let (len, w, col) = match t.major {
                        Some(_) => (*major_length, *major_width, *color),
                        None => (*minor_length, *minor_width, *minor_color),
                    };
                    let outer = pt(arc_point(cc, radius * s, t.angle));
                    let inner = pt(arc_point(cc, (radius - len).max(0.0) * s, t.angle));
                    p.line_segment([inner, outer], egui::Stroke::new(w * s, k(col)));
                    if let (true, Some(i)) = (*labels, t.major) {
                        p.text(
                            pt(arc_point(cc, label_r * s, t.angle)),
                            egui::Align2::CENTER_CENTER,
                            tick_label(i, count, *label_min, label_hi, *label_decimals),
                            egui::FontId::proportional(label_size * s),
                            k(*label_color),
                        );
                    }
                }
            }
            Widget::Text { text, pos, size, align, color, mono, rotation, opacity, skew, glow, glow_color } => {
                let anchor = match align {
                    Align::Left | Align::Justify => egui::Align2::LEFT_TOP,
                    Align::Center => egui::Align2::CENTER_TOP,
                    Align::Right => egui::Align2::RIGHT_TOP,
                };
                let font = if *mono {
                    egui::FontId::monospace(size * s)
                } else {
                    egui::FontId::proportional(size * s)
                };
                let col = color.gamma_multiply(*opacity);
                let galley = p.layout_no_wrap(format_template(text, v), font, col);
                // Unrotated placement by alignment, then turn around the
                // alignment point (= `pos`).
                let top_left = anchor.anchor_size(at(*pos), galley.size()).min;
                if *skew == 0.0 && *glow <= 0.0 {
                    p.add(
                        egui::epaint::TextShape::new(top_left, galley, col)
                            .with_angle_and_anchor(rotation.to_radians(), anchor),
                    );
                    continue;
                }
                // Italic / halo: draw the glyph mesh ourselves.
                let tex = ctx.fonts(|f| f.font_image_size());
                let anchor_local = anchor
                    .pos_in_rect(&egui::Rect::from_min_size(egui::Pos2::ZERO, galley.size()))
                    .to_vec2();
                let mk = |c: egui::Color32, shift: egui::Vec2| {
                    egui::Shape::mesh(text_mesh(&galley, top_left, anchor_local, *rotation, *skew, c, tex, shift))
                };
                if *glow > 0.0 {
                    // Halo: two rings of 8 faint copies around the text.
                    let gc = glow_color.unwrap_or(color.gamma_multiply(0.12)).gamma_multiply(*opacity);
                    for ring in [0.5f32, 1.0] {
                        for step in 0..8 {
                            let o = arc_point([0.0, 0.0], glow * s * ring, step as f32 * 45.0);
                            p.add(mk(gc, egui::vec2(o[0], o[1])));
                        }
                    }
                }
                p.add(mk(col, egui::Vec2::ZERO));
            }
            Widget::Label {
                text,
                trim,
                font,
                bold,
                size,
                align,
                color,
                pos,
                rotation,
                opacity,
                skew,
                letter_spacing,
                line_height,
                max_width,
                outline,
                outline_color,
                glow,
                glow_color,
                background,
                padding,
                corner,
            } => {
                let shown = shape_text(&format_template(text, v), trim);
                if shown.is_empty() {
                    continue;
                }
                let fid = match font {
                    FontKind::Sans => egui::FontId::proportional(size * s),
                    FontKind::Mono => egui::FontId::monospace(size * s),
                    FontKind::Serif => cxsfm_ui::fonts::serif(size * s),
                    FontKind::Title => cxsfm_ui::fonts::title(size * s),
                };
                let col = color.gamma_multiply(*opacity);
                let mut job = egui::text::LayoutJob::single_section(
                    shown,
                    egui::TextFormat {
                        font_id: fid,
                        color: col,
                        extra_letter_spacing: letter_spacing * s,
                        line_height: line_height.map(|h| h * s),
                        ..Default::default()
                    },
                );
                if *max_width > 0.0 {
                    job.wrap.max_width = max_width * s;
                }
                job.halign = match align {
                    Align::Left | Align::Justify => egui::Align::LEFT,
                    Align::Center => egui::Align::Center,
                    Align::Right => egui::Align::RIGHT,
                };
                // Both edges flush: only meaningful on wrapped lines.
                job.justify = *align == Align::Justify && *max_width > 0.0;
                let galley = p.layout_job(job);
                let anchor = match align {
                    Align::Left | Align::Justify => egui::Align2::LEFT_TOP,
                    Align::Center => egui::Align2::CENTER_TOP,
                    Align::Right => egui::Align2::RIGHT_TOP,
                };
                // Rows of centred/right jobs sit left of x = 0: work in
                // the galley's own coordinates (`raw`), placed so its
                // box lands on the alignment point, turned around it.
                let raw = galley.rect;
                let top_left = anchor.anchor_size(at(*pos), raw.size()).min;
                let origin = top_left - raw.min.to_vec2();
                let pivot = raw.min.to_vec2()
                    + anchor.pos_in_rect(&egui::Rect::from_min_size(egui::Pos2::ZERO, raw.size())).to_vec2();
                let k = |c: egui::Color32| c.gamma_multiply(*opacity);
                if let Some(bg) = background {
                    let pad = egui::vec2(padding[0] * s, padding[1] * s);
                    let r = raw.expand2(pad);
                    if *rotation == 0.0 {
                        p.rect_filled(r.translate(origin.to_vec2()), corner * s, k(*bg));
                    } else {
                        let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
                            .map(|q| {
                                let t = rotate_about([q.x, q.y], [pivot.x, pivot.y], *rotation);
                                origin + egui::vec2(t[0], t[1])
                            })
                            .to_vec();
                        p.add(egui::Shape::convex_polygon(pts, k(*bg), egui::Stroke::NONE));
                    }
                }
                let tex = ctx.fonts(|f| f.font_image_size());
                let mk = |c: egui::Color32, shift: egui::Vec2| {
                    egui::Shape::mesh(text_mesh(&galley, origin, pivot, *rotation, *skew, c, tex, shift))
                };
                // Faux bold: the glyphs again, nudged right.
                let bold_px = if *bold { (size * s * 0.045).max(1.0) } else { 0.0 };
                let strokes: Vec<f32> = if *bold { vec![0.0, bold_px * 0.5, bold_px] } else { vec![0.0] };
                let ring = |p: &egui::Painter, r: f32, c: egui::Color32| {
                    for step in 0..8 {
                        let o = arc_point([0.0, 0.0], r, step as f32 * 45.0);
                        for dx in &strokes {
                            p.add(mk(c, egui::vec2(o[0] + dx, o[1])));
                        }
                    }
                };
                if *glow > 0.0 {
                    let gc = k(glow_color.unwrap_or(color.gamma_multiply(0.12)));
                    ring(&p, glow * s * 0.5, gc);
                    ring(&p, glow * s, gc);
                }
                if *outline > 0.0 {
                    ring(&p, outline * s, k(*outline_color));
                }
                for dx in &strokes {
                    p.add(mk(col, egui::vec2(*dx, 0.0)));
                }
            }
            Widget::Shape { kind, pos, size, rotation, corner, fill, stroke, stroke_color, opacity } => {
                let k = |c: egui::Color32| c.gamma_multiply(*opacity);
                let a = at(*pos);
                let d = egui::vec2(size[0] * s, size[1] * s);
                let st = if *stroke > 0.0 {
                    egui::Stroke::new(stroke * s, k(*stroke_color))
                } else {
                    egui::Stroke::NONE
                };
                let fill = fill.map(k).unwrap_or(egui::Color32::TRANSPARENT);
                match kind {
                    ShapeKind::Line => {
                        let b = pt(rotate_about([a.x + d.x, a.y + d.y], [a.x, a.y], *rotation));
                        p.line_segment([a, b], st);
                    }
                    ShapeKind::Rect if *rotation == 0.0 => {
                        let r = egui::Rect::from_two_pos(a, a + d);
                        p.rect(r, corner * s, fill, st, egui::StrokeKind::Inside);
                    }
                    _ => {
                        let pts: Vec<egui::Pos2> =
                            shape_outline(*kind, [a.x, a.y], [a.x + d.x, a.y + d.y], corner * s, *rotation)
                                .into_iter()
                                .map(pt)
                                .collect();
                        if pts.len() >= 3 {
                            p.add(egui::Shape::convex_polygon(pts, fill, st));
                        }
                    }
                }
            }
        }
    }
}

impl HudMod {
    pub(crate) fn draw(&mut self, ctx: &egui::Context) {
        let cfg: Option<std::sync::Arc<HudConfig>> = lock(&self.config).clone();
        let t = ctx.input(|i| i.time);
        // Editing = menu open on the Editor tab: the HUDs become
        // interactive and show the working copy.
        let editing = cxsfm_input::hotkey::ui_visible() && self.tab == TAB_EDIT;
        if editing {
            self.editor.sync(&cfg);
        }
        let mut values = match cfg.as_deref().map(|c| c.mode) {
            Some(Mode::Demo) => demo_values(t),
            _ => lock(&self.live).clone(),
        };
        // Laying out in the garage: no live value yet → demo values so
        // every widget shows (and can be grabbed).
        if editing && values.is_empty() {
            values = demo_values(t);
        }
        // Unsaved edits stay visible with the menu closed too.
        let shown: Option<&HudConfig> =
            if editing || !self.editor.edits.is_empty() { self.editor.work.as_ref() } else { cfg.as_deref() };
        if let Some(c) = shown
            && !values.is_empty()
        {
            apply_derived(&mut values, &c.derived);
        }
        if editing {
            self.editor.overlay(ctx, &values, t);
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        } else if let Some(cfg) = shown {
            let active: Vec<&str> = cfg.states.iter().filter(|s| s.active(&values, t)).map(|s| s.name.as_str()).collect();
            let mut any = false;
            for (i, h) in cfg.huds.iter().enumerate().filter(|(_, h)| h.enabled) {
                any = true;
                if h.hide_when_missing && values.is_empty() {
                    continue;
                }
                draw_hud(ctx, &hud_painter(ctx, i), h, &values, &active);
            }
            // Live values need frames even with the menu closed: the
            // overlay re-runs egui when the requested delay expires.
            if any {
                ctx.request_repaint_after(std::time::Duration::from_millis(1000 / u64::from(cfg.fps.max(1))));
            }
        }
        if cxsfm_input::hotkey::ui_visible() {
            let cfg = cfg.clone();
            self.draw_window(ctx, cfg.as_deref(), &values);
        }
    }

    fn draw_window(&mut self, ctx: &egui::Context, cfg: Option<&HudConfig>, values: &Values) {
        let code = cxsfm_api::i18n::current().code();
        // The tabs need the whole mod: take the chrome out.
        let mut tool = std::mem::take(&mut self.tool);
        tool.show(ctx, super::i18n::tile_label(code), egui::vec2(WIN_W, WIN_H), |ui, _controls| {
            kit::tabs(
                ui,
                &mut self.tab,
                &[super::i18n::tab_hud(code), super::i18n::ed(code, "tab_edit"), super::i18n::tab_explore(code)],
            );
            if self.tab == TAB_EXPLORE {
                self.explore_tab(ui, code);
                return;
            }
            if self.tab == TAB_EDIT {
                let status = lock(&self.save_status).clone();
                if let PanelOut::Save(edits) = self.editor.panel(ui, values, code, status.as_deref()) {
                    lock(&self.save_req).get_or_insert_with(Vec::new).extend(edits);
                }
                return;
            }
            let status = lock(&self.status).clone();
            kit::kv(ui, super::i18n::file_label(code), status.path.as_deref().unwrap_or("—"));
            match cfg {
                Some(c) => {
                    let on = c.huds.iter().filter(|h| h.enabled).count();
                    ui.label(
                        super::i18n::huds_label(code)
                            .replace("{on}", &on.to_string())
                            .replace("{total}", &c.huds.len().to_string()),
                    );
                }
                None => {
                    ui.label(super::i18n::no_config(code));
                }
            }
            let demo = cfg.is_some_and(|c| c.mode == Mode::Demo);
            ui.label(if demo { super::i18n::mode_demo(code) } else { super::i18n::mode_live(code) });
            if !demo {
                let live = lock(&self.live_status).clone();
                for (root, found) in &live.roots {
                    if *found {
                        ui.label(super::i18n::root_found(code).replace("{root}", root));
                    } else {
                        kit::warn(ui, &super::i18n::root_missing(code).replace("{root}", root));
                    }
                }
                for (name, err) in live.errors.iter().take(8) {
                    kit::warn(ui, &format!("{name} : {err}"));
                }
            }
            // Current values, one compact line (what the HUDs see).
            let line: Vec<String> = values
                .nums
                .iter()
                .map(|(n, v)| format!("{n}={v:.2}"))
                .chain(values.texts.iter().map(|(n, t)| format!("{n}=\"{t}\"")))
                .collect();
            ui.add(egui::Label::new(egui::RichText::new(line.join("  ")).monospace().small()).wrap());
            if ui.button(super::i18n::reload_label(code)).clicked() {
                self.reload_now.store(true, Ordering::SeqCst);
            }
            if let Some(e) = &status.error {
                kit::error(ui, super::i18n::error_label(code));
                ui.add(egui::Label::new(egui::RichText::new(e).small().monospace()).wrap());
            }
            if !status.warnings.is_empty() {
                kit::warn(ui, super::i18n::warnings_label(code));
                for w in status.warnings.iter().take(12) {
                    kit::note(ui, w);
                }
            }
        });
        self.tool = tool;
    }

    /// Explorer tab: search classes, open one, read its fields, step
    /// into reference fields. Every click stages ONE request for the
    /// tick; nothing here touches Unity.
    fn explore_tab(&mut self, ui: &mut egui::Ui, code: &str) {
        let view = lock(&self.explore).clone();
        let pending = lock(&self.explore_req).is_some();
        let mut req: Option<ExploreReq> = None;
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .desired_width(260.0)
                    .hint_text(super::i18n::search_hint(code)),
            );
            let go = ui
                .add_enabled(self.query.trim().len() >= 2, egui::Button::new(super::i18n::search_label(code)))
                .clicked()
                || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go && self.query.trim().len() >= 2 {
                req = Some(ExploreReq::Search(self.query.trim().to_string()));
            }
            if pending {
                ui.spinner();
            }
        });
        kit::note(ui, super::i18n::explore_hint(code));
        self.new_var_row(ui, code);
        let mut add_var: Option<(String, String)> = None;
        match &view.open {
            Some(o) => {
                ui.horizontal(|ui| {
                    if ui.button(super::i18n::back_label(code)).clicked() {
                        lock(&self.explore).open = None;
                    }
                    if !o.path.is_empty() && ui.button("↑").clicked() {
                        let mut path = o.path.clone();
                        path.pop();
                        req = Some(ExploreReq::Open { klass: o.klass, root: o.root.clone(), path, instance: o.instance });
                    }
                    if ui.button(super::i18n::reread_label(code)).clicked() {
                        req = Some(ExploreReq::Open {
                            klass: o.klass,
                            root: o.root.clone(),
                            path: o.path.clone(),
                            instance: o.instance,
                        });
                    }
                    if o.instances > 1 {
                        if ui.add_enabled(o.instance > 0, egui::Button::new("◀")).clicked() {
                            req = Some(ExploreReq::Open {
                                klass: o.klass,
                                root: o.root.clone(),
                                path: o.path.clone(),
                                instance: o.instance - 1,
                            });
                        }
                        ui.monospace(format!("{}/{}", o.instance + 1, o.instances));
                        if ui.add_enabled(o.instance + 1 < o.instances, egui::Button::new("▶")).clicked() {
                            req = Some(ExploreReq::Open {
                                klass: o.klass,
                                root: o.root.clone(),
                                path: o.path.clone(),
                                instance: o.instance + 1,
                            });
                        }
                    }
                });
                let mut crumb = o.root.rsplit('.').next().unwrap_or(&o.root).to_string();
                for seg in &o.path {
                    crumb.push_str(" › ");
                    crumb.push_str(seg);
                }
                if !o.at_class.is_empty() {
                    crumb.push_str(&format!("  ({})", o.at_class));
                }
                ui.add(egui::Label::new(egui::RichText::new(crumb).monospace().strong()).truncate());
                if let Some(e) = &o.error {
                    kit::warn(ui, e);
                }
                let root_short = o.root.rsplit('.').next().unwrap_or(&o.root).to_string();
                kit::list(ui, "hud_explore_fields", LIST_H, |ui| {
                    for row in &o.fields {
                        ui.horizontal(|ui| {
                            if row.follow {
                                if ui.small_button("→").on_hover_text(super::i18n::follow_hint(code)).clicked() {
                                    let mut path = o.path.clone();
                                    path.push(row.name.clone());
                                    req = Some(ExploreReq::Open { klass: o.klass, root: o.root.clone(), path, instance: o.instance });
                                }
                            } else if ui
                                .small_button(egui_material_icons::icons::ICON_ADD.rich_text())
                                .on_hover_text(super::i18n::ed(code, "use_var"))
                                .clicked()
                            {
                                add_var = Some((row.name.clone(), binding_path(&root_short, &o.path, &row.name)));
                            }
                            let line = format!("{}: {} = {}", row.name, row.type_name, row.value);
                            ui.add(egui::Label::new(egui::RichText::new(line).monospace()).truncate())
                                .on_hover_text(binding_path(&root_short, &o.path, &row.name));
                        });
                    }
                });
            }
            None => {
                if let Some(q) = &view.searched {
                    ui.label(
                        super::i18n::results_label(code)
                            .replace("{n}", &view.hits.len().to_string())
                            .replace("{q}", q)
                            + if view.truncated { " (+)" } else { "" },
                    );
                }
                kit::list(ui, "hud_explore_hits", LIST_H, |ui| {
                    for h in &view.hits {
                        let text = format!("{}   [{}]", h.full, h.image);
                        let r = ui.add(
                            egui::Label::new(egui::RichText::new(text).monospace())
                                .truncate()
                                .sense(egui::Sense::click()),
                        );
                        if r.clicked() {
                            req = Some(ExploreReq::Open { klass: h.klass, root: h.full.clone(), path: Vec::new(), instance: 0 });
                        }
                    }
                });
            }
        }
        if let Some(r) = req {
            *lock(&self.explore_req) = Some(r);
        }
        if let Some((field, path)) = add_var {
            let name = super::edit::suggest_var_name(&field, &self.taken_names());
            self.new_var = Some((name, path));
        }
    }

    /// Source names already in use (file + editor copy).
    fn taken_names(&self) -> Vec<String> {
        let published = lock(&self.config).clone();
        let mut v: Vec<String> = published
            .iter()
            .flat_map(|c| c.sources.iter().map(|s| s.name.clone()).chain(c.derived.iter().map(|d| d.name.clone())))
            .collect();
        if let Some(w) = &self.editor.work {
            v.extend(w.sources.iter().map(|s| s.name.clone()));
            v.extend(w.derived.iter().map(|d| d.name.clone()));
        }
        v.push("speed_mph".into());
        v
    }

    /// "Use as variable": name the field, then write it into
    /// `[sources]` right away (through the tick) — the editor's pickers
    /// list it at once, the live reader once the file is reloaded.
    fn new_var_row(&mut self, ui: &mut egui::Ui, code: &str) {
        let Some((mut name, path)) = self.new_var.take() else {
            return;
        };
        let taken = self.taken_names();
        let ok = super::edit::valid_var_name(&name) && !taken.contains(&name);
        let mut keep = true;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(super::i18n::ed(code, "var_name"));
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(160.0));
                if ui.add_enabled(ok, egui::Button::new(super::i18n::ed(code, "var_add"))).clicked() {
                    lock(&self.save_req)
                        .get_or_insert_with(Vec::new)
                        .push(super::edit::Edit::AddSource { name: name.clone(), path: path.clone() });
                    self.editor.note_source(&name, &path);
                    *lock(&self.save_status) = Some(super::i18n::ed(code, "var_added").replace("{name}", &name));
                    keep = false;
                }
                if ui.button(egui_material_icons::icons::ICON_CANCEL.rich_text()).clicked() {
                    keep = false;
                }
            });
            ui.add(egui::Label::new(egui::RichText::new(&path).monospace().small()).truncate());
            if !ok {
                kit::warn(ui, super::i18n::ed(code, "var_bad"));
            }
        });
        if keep {
            self.new_var = Some((name, path));
        }
    }
}

/// Fixed control-window size (explorer rows truncate inside it).
const WIN_W: f32 = 620.0;
const WIN_H: f32 = 520.0;
/// Explorer list height.
const LIST_H: f32 = 340.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_hud::model::{DEFAULT_CONFIG, parse_config};

    #[test]
    fn example_huds_sit_inside_1080p() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0));
        let (cfg, _) = parse_config(DEFAULT_CONFIG).unwrap();
        for h in &cfg.huds {
            let r = hud_rect(screen, h);
            assert!(screen.contains_rect(r), "{} at {r:?}", h.name);
        }
        let r = hud_rect(screen, &cfg.huds[0]);
        assert_eq!(r.max, egui::pos2(1920.0 - 48.0, 1080.0 - 48.0));
    }

    /// Explorer with long names must stay inside the fixed window.
    /// Editor tab: HUD props, then each widget type's props (and a
    /// multi-selection) must stay inside the fixed window.
    #[test]
    fn editor_panel_stays_within_window() {
        use super::super::model::parse_config;
        let text = r##"
version = 2
mode = "demo"
[sources]
rpm = "UITachometer/m_rpm"
[states]
rouge = { source = "rpm", above = 0.9 }
[[hud]]
name = "All widgets with a deliberately long HUD name"
size = [400.0, 200.0]
  [[hud.widget]]
  type = "bar"
  source = "rpm"
  redline = 0.8
  [[hud.widget]]
  type = "arc"
  source = "rpm"
  [[hud.widget]]
  type = "needle"
  source = "rpm"
  [[hud.widget]]
  type = "ticks"
  [[hud.widget]]
  type = "text"
  text = "{rpm} a long template with many words in it"
  [[hud.widget]]
  type = "label"
  text = "{rpm}"
  align = "justify"
  [[hud.widget]]
  type = "rect"
  [[hud.widget]]
  type = "ellipse"
  fill = "none"
  stroke = 2
  [[hud.widget]]
  type = "line"
  alert = "rouge"
"##;
        let (cfg, w) = parse_config(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let n = cfg.huds[0].widgets.len();
        assert_eq!(n, 9);
        let mut m = HudMod::new();
        m.tab = TAB_EDIT;
        m.tool.pos = Some(egui::pos2(100.0, 100.0));
        m.editor.work = Some(cfg);
        let ctx = egui::Context::default();
        egui_material_icons::initialize(&ctx);
        cxsfm_ui::fonts::install(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        let values = demo_values(1.0);
        let mut selections: Vec<Vec<usize>> = vec![vec![]];
        selections.extend((0..n).map(|i| vec![i]));
        selections.push(vec![0, 4, 6]);
        for sel in selections {
            m.editor.sel = sel.clone();
            let mut max_x = 0.0f32;
            for _ in 0..3 {
                ctx.begin_pass(input.clone());
                m.draw_window(&ctx, None, &values);
                let mut out = ctx.end_pass();
                out.textures_delta.clear();
                max_x = 0.0;
                for p in ctx.tessellate(out.shapes, out.pixels_per_point) {
                    if let egui::epaint::Primitive::Mesh(mesh) = &p.primitive {
                        for v in &mesh.vertices {
                            max_x = max_x.max(v.pos.x);
                        }
                    }
                }
            }
            assert!(max_x <= 100.0 + WIN_W + 40.0, "editor panel overflows with selection {sel:?}: max_x={max_x}");
        }
        assert!(m.editor.edits.is_empty(), "drawing alone must not record edits");
    }

    /// Every shape (rotated, outlined, filled, line) paints without
    /// panicking and inside its HUD.
    #[test]
    fn shapes_render() {
        use super::super::model::parse_config;
        let text = r##"
[[hud]]
anchor = "top_left"
size = [300.0, 200.0]
  [[hud.widget]]
  type = "rect"
  corner = 8
  rotation = 30
  stroke = 2
  [[hud.widget]]
  type = "rect"
  pos = [50, 50]
  [[hud.widget]]
  type = "ellipse"
  pos = [100, 20]
  size = [80, 40]
  rotation = 45
  [[hud.widget]]
  type = "line"
  pos = [10, 150]
  size = [200, 0]
  rotation = -10
"##;
        let (cfg, _) = parse_config(text).unwrap();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        ctx.begin_pass(input);
        draw_hud(&ctx, &hud_painter(&ctx, 0), &cfg.huds[0], &Values::default(), &[]);
        let mut out = ctx.end_pass();
        out.textures_delta.clear();
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let verts: Vec<egui::Pos2> = prims
            .iter()
            .filter_map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => Some(m),
                _ => None,
            })
            .flat_map(|m| m.vertices.iter().map(|v| v.pos))
            .collect();
        assert!(verts.len() > 100, "shapes painted");
        assert!(verts.iter().all(|v| v.x < 320.0 && v.y < 220.0), "inside the HUD");
    }

    #[test]
    fn explorer_stays_within_window() {
        use super::super::model::{FieldRow, OpenView};
        let mut m = HudMod::new();
        m.tab = TAB_EXPLORE;
        m.tool.pos = Some(egui::pos2(100.0, 100.0));
        lock(&m.explore).open = Some(OpenView {
            root: "CarX.Street.Very.Long.Namespace.UITachometerControllerWithALongName".into(),
            path: vec!["m_carControllerReferenceWithAVeryLongName".into()],
            instances: 3,
            at_class: "CarControllerWithAnEvenLongerNameForTesting".into(),
            error: Some("x ".repeat(200)),
            fields: (0..40)
                .map(|i| FieldRow {
                    name: format!("m_field_with_a_really_long_name_number_{i}"),
                    type_name: "System.Collections.Generic.Dictionary".into(),
                    value: "(1.000, 2.000, 3.000, 4.000) ".repeat(6),
                    follow: i % 2 == 0,
                })
                .collect(),
            ..OpenView::default()
        });
        let ctx = egui::Context::default();
        egui_material_icons::initialize(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        let mut max_x = 0.0f32;
        for _ in 0..3 {
            ctx.begin_pass(input.clone());
            m.draw_window(&ctx, None, &Values::default());
            let mut out = ctx.end_pass();
            out.textures_delta.clear();
            max_x = 0.0;
            for p in ctx.tessellate(out.shapes, out.pixels_per_point) {
                if let egui::epaint::Primitive::Mesh(mesh) = &p.primitive {
                    for v in &mesh.vertices {
                        max_x = max_x.max(v.pos.x);
                    }
                }
            }
        }
        assert!(max_x <= 100.0 + WIN_W + 40.0, "explorer overflows: max_x={max_x}");
    }

    /// The README's `[sources]` + full dial example must parse with
    /// zero warnings and render (keeps the doc honest).
    #[test]
    fn readme_example_parses_and_renders() {
        let readme = include_str!("README.md");
        let blocks: Vec<&str> = readme
            .split("```toml")
            .skip(1)
            .filter_map(|b| b.split("```").next())
            .collect();
        let sources = blocks.iter().find(|b| b.contains("rpm_max")).expect("sources block");
        // Examples are found by SECTION (§5 / §5 bis), not by HUD name:
        // renaming an example must not break the test.
        let section_block = |heading: &str| -> &str {
            let start = readme.find(heading).unwrap_or_else(|| panic!("README section {heading}"));
            let body = &readme[start + heading.len()..];
            let body = &body[..body.find("\n## ").unwrap_or(body.len())];
            body.split("```toml").nth(1).and_then(|b| b.split("```").next()).expect("toml block in section")
        };
        let dial = section_block("## 5. ");
        let states = blocks.iter().find(|b| b.contains("[states]")).expect("states block");
        let cluster = section_block("## 5b.");
        let text = format!("version = 2\nmode = \"demo\"\n{sources}\n{states}\n{dial}\n{cluster}");
        let (cfg, w) = parse_config(&text).expect("README example must parse");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.huds.len(), 2);
        assert_eq!(cfg.huds[0].widgets.len(), 11);
        assert_eq!(cfg.huds[1].widgets.len(), 6);
        assert_eq!(cfg.states.len(), 5);
        assert_eq!(cfg.derived.len(), 2);
        let ctx = egui::Context::default();
        egui_material_icons::initialize(&ctx);
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        });
        let mut v = demo_values(2.0);
        apply_derived(&mut v, &cfg.derived);
        assert!(v.get("rpm_tr").is_some());
        for (i, h) in cfg.huds.iter().enumerate() {
            draw_hud(&ctx, &hud_painter(&ctx, i), h, &v, &[]);
        }
        let mut out = ctx.end_pass();
        out.textures_delta.clear();
        assert!(!ctx.tessellate(out.shapes, out.pixels_per_point).is_empty());
    }

    /// Render smoke test: every widget kind tessellates without panic
    /// (same font setup as production).
    #[test]
    fn example_renders() {
        let (mut cfg, _) = parse_config(DEFAULT_CONFIG).unwrap();
        // Every arc flavour: static ring, bound ring (from end), pie
        // past 180°, full disc.
        let (arcs, _) = parse_config(
            "mode = \"demo\"\n[[hud]]\n\
             [[hud.widget]]\ntype = \"arc\"\n\
             [[hud.widget]]\ntype = \"arc\"\nsource = \"rpm\"\nmin = \"rpm_min\"\nmax = \"rpm_max\"\nredline = \"redline\"\nfill = \"end\"\n\
             [[hud.widget]]\ntype = \"arc\"\nsource = \"rpm\"\nmin = \"rpm_min\"\nmax = \"rpm_max\"\nfilled = true\nstart = -150\nend = 150\n\
             [[hud.widget]]\ntype = \"arc\"\nfilled = true\nstart = 0\nend = 360\nopacity = 0.3\n\
             [[hud.widget]]\ntype = \"needle\"\nsource = \"rpm\"\nmin = \"rpm_min\"\nmax = \"rpm_max\"\nredline = \"redline\"\ntail = 12\nalert = \"rouge\"\n\
             [[hud.widget]]\ntype = \"ticks\"\nlabel_max = 8\nrotation = 15\nalert = \"rouge\"\n\
             [[hud.widget]]\ntype = \"ticks\"\ncount = \"auto\"\nlabel_max = \"max_gear\"\nlabel_margin = 2\n\
             [[hud.widget]]\ntype = \"bar\"\nsource = \"rpm\"\nrotation = 30\nsegments = 5\n\
             [[hud.widget]]\ntype = \"text\"\ntext = \"{speed}\"\nalign = \"right\"\nrotation = -12\n\
             [[hud.widget]]\ntype = \"text\"\ntext = \"130\"\nalign = \"center\"\nskew = 12\nglow = 6\nrotation = 5\n\
             [[hud.widget]]\ntype = \"bar\"\nsource = \"rpm\"\nsegments = 24\ntaper = [0.3, 1.0]\ntaper_curve = 2.5",
        )
        .unwrap();
        cfg.huds.extend(arcs.huds);
        let ctx = egui::Context::default();
        egui_material_icons::initialize(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        for t in 0..3 {
            ctx.begin_pass(input.clone());
            for (i, h) in cfg.huds.iter().enumerate() {
                draw_hud(&ctx, &hud_painter(&ctx, i), h, &demo_values(t as f64 * 3.3), &["rouge"]);
            }
            let mut out = ctx.end_pass();
            out.textures_delta.clear();
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            assert!(!prims.is_empty());
        }
    }

    /// Every `label` option draws, with the real fonts, at any angle.
    #[test]
    fn label_draws_with_every_option() {
        use crate::custom_hud::model::parse_config;
        let toml = r##"
            mode = "demo"
            [[hud]]
            name = "labels"
            anchor = "center"
            size = [400.0, 200.0]
              [[hud.widget]]
              type = "label"
              text = "<b>{track}</b> · {speed} km/h"
              font = "serif"
              bold = true
              outline = 2
              glow = 4
              background = "#000000A0"
              padding = [6, 3]
              corner = 4
              rotation = 15
              skew = 10
              align = "center"
              letter_spacing = 2
              max_width = 120
              pos = [200.0, 50.0]
              [[hud.widget]]
              type = "label"
              source = "track"
              font = "mono"
              max_chars = 4
              keep = "end"
              align = "right"
              [[hud.widget]]
              type = "label"
              source = "missing"
              font = "title"
        "##;
        let (cfg, w) = parse_config(toml).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let mut v = Values::default();
        v.set("speed", 123.0);
        v.set_text("track", "Downtown Tokyo".into());
        let ctx = egui::Context::default();
        cxsfm_ui::fonts::install(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        let mut verts = 0;
        for _ in 0..2 {
            ctx.begin_pass(input.clone());
            draw_hud(&ctx, &hud_painter(&ctx, 0), &cfg.huds[0], &v, &[]);
            let mut out = ctx.end_pass();
            out.textures_delta.clear();
            verts = ctx
                .tessellate(out.shapes, out.pixels_per_point)
                .iter()
                .map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                    _ => 0,
                })
                .sum();
        }
        assert!(verts > 100, "{verts}");
    }
}
