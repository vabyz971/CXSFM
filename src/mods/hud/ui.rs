//! HUD rendering (present thread) + control window.
//!
//! Pure egui: no Unity call, no lock held beyond cloning the published
//! `Arc<HudConfig>`. HUDs paint on a background layer (under every
//! window, never interactive), so they stay on screen with the menu
//! closed (the mod reports itself pinned) without touching input.

use super::HudMod;
use super::model::{
    Align, ExploreReq, Hud, HudConfig, Mode, Values, Widget, arc_point, arc_points, bar_fraction,
    apply_derived, binding_path, demo_values, format_template, rotate_about, segment_height, tick_label,
    tick_layout, ticks_range, with_main_color,
};
use crate::mods::common::lock;
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

/// `active` = names of the `[states]` entries showing this frame.
fn draw_hud(ctx: &egui::Context, idx: usize, h: &Hud, v: &Values, active: &[&str]) {
    let screen = ctx.content_rect();
    let rect = hud_rect(screen, h);
    let mut p = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("cxsfm_hud", idx)),
    ));
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
                    Align::Left => egui::Align2::LEFT_TOP,
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
        }
    }
}

impl HudMod {
    pub(crate) fn draw(&mut self, ctx: &egui::Context) {
        let cfg: Option<std::sync::Arc<HudConfig>> = lock(&self.config).clone();
        let mut values = match cfg.as_deref().map(|c| c.mode) {
            Some(Mode::Demo) => demo_values(ctx.input(|i| i.time)),
            _ => lock(&self.live).clone(),
        };
        // Derived sources (remaps) on top of whatever was read.
        if let Some(c) = cfg.as_deref() {
            if !values.is_empty() {
                apply_derived(&mut values, &c.derived);
            }
        }
        if let Some(cfg) = &cfg {
            let t = ctx.input(|i| i.time);
            let active: Vec<&str> = cfg
                .states
                .iter()
                .filter(|s| s.active(&values, t))
                .map(|s| s.name.as_str())
                .collect();
            let mut any = false;
            for (i, h) in cfg.huds.iter().enumerate().filter(|(_, h)| h.enabled) {
                any = true;
                if h.hide_when_missing && values.is_empty() {
                    continue;
                }
                draw_hud(ctx, i, h, &values, &active);
            }
            // Live values need frames even with the menu closed: the
            // overlay re-runs egui when the requested delay expires.
            if any {
                ctx.request_repaint_after(std::time::Duration::from_millis(
                    1000 / u64::from(cfg.fps.max(1)),
                ));
            }
        }
        if crate::hotkey::ui_visible() {
            self.draw_window(ctx, cfg.as_deref(), &values);
        }
    }

    fn draw_window(&mut self, ctx: &egui::Context, cfg: Option<&HudConfig>, values: &Values) {
        let code = crate::i18n::current().code();
        let mut win = egui::Window::new(super::i18n::tile_label(code))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::vec2(WIN_W, WIN_H))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
            self.tool.enter(ui);
            self.tool.header(ui, super::i18n::tile_label(code));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab_explore, false, super::i18n::tab_hud(code));
                ui.selectable_value(&mut self.tab_explore, true, super::i18n::tab_explore(code));
            });
            ui.separator();
            if self.tab_explore {
                self.explore_tab(ui, code);
                return;
            }
            let status = lock(&self.status).clone();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(super::i18n::file_label(code)).weak());
                ui.monospace(status.path.as_deref().unwrap_or("—"));
            });
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
                    let (txt, col) = if *found {
                        (super::i18n::root_found(code), ui.visuals().text_color())
                    } else {
                        (super::i18n::root_missing(code), ui.visuals().warn_fg_color)
                    };
                    ui.label(egui::RichText::new(txt.replace("{root}", root)).color(col));
                }
                for (name, err) in live.errors.iter().take(8) {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("{name} : {err}")).small().color(ui.visuals().warn_fg_color),
                        )
                        .wrap(),
                    );
                }
            }
            // Current values, one compact line (what the HUDs see).
            let line: Vec<String> = values.0.iter().map(|(n, v)| format!("{n}={v:.2}")).collect();
            ui.add(egui::Label::new(egui::RichText::new(line.join("  ")).monospace().small()).wrap());
            if ui.button(super::i18n::reload_label(code)).clicked() {
                self.reload_now.store(true, Ordering::SeqCst);
            }
            if let Some(e) = &status.error {
                ui.label(
                    egui::RichText::new(super::i18n::error_label(code))
                        .color(ui.visuals().error_fg_color),
                );
                ui.add(egui::Label::new(egui::RichText::new(e).small().monospace()).wrap());
            }
            if !status.warnings.is_empty() {
                ui.label(
                    egui::RichText::new(super::i18n::warnings_label(code))
                        .color(ui.visuals().warn_fg_color),
                );
                for w in status.warnings.iter().take(12) {
                    ui.add(egui::Label::new(egui::RichText::new(w).small()).wrap());
                }
            }
        });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
        }
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
        ui.add(egui::Label::new(egui::RichText::new(super::i18n::explore_hint(code)).small()).wrap());
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
                    ui.add(egui::Label::new(egui::RichText::new(e).small().color(ui.visuals().warn_fg_color)).wrap());
                }
                let root_short = o.root.rsplit('.').next().unwrap_or(&o.root).to_string();
                egui::ScrollArea::vertical().max_height(LIST_H).auto_shrink([false, false]).show(ui, |ui| {
                    for row in &o.fields {
                        ui.horizontal(|ui| {
                            if row.follow {
                                if ui.small_button("→").on_hover_text(super::i18n::follow_hint(code)).clicked() {
                                    let mut path = o.path.clone();
                                    path.push(row.name.clone());
                                    req = Some(ExploreReq::Open { klass: o.klass, root: o.root.clone(), path, instance: o.instance });
                                }
                            } else {
                                ui.add_space(22.0);
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
                egui::ScrollArea::vertical().max_height(LIST_H).auto_shrink([false, false]).show(ui, |ui| {
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
    use crate::mods::hud::model::{DEFAULT_CONFIG, parse_config};

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
    #[test]
    fn explorer_stays_within_window() {
        use super::super::model::{FieldRow, OpenView};
        let mut m = HudMod::new();
        m.tab_explore = true;
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
        let cluster = section_block("## 5 bis.");
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
            draw_hud(&ctx, i, h, &v, &[]);
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
                draw_hud(&ctx, i, h, &demo_values(t as f64 * 3.3), &["rouge"]);
            }
            let mut out = ctx.end_pass();
            out.textures_delta.clear();
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            assert!(!prims.is_empty());
        }
    }
}
