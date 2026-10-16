//! HUD rendering (present thread) + control window.
//!
//! Pure egui: no Unity call, no lock held beyond cloning the published
//! `Arc<HudConfig>`. HUDs paint on a background layer (under every
//! window, never interactive), so they stay on screen with the menu
//! closed (the mod reports itself pinned) without touching input.

use super::HudMod;
use super::model::{
    Align, ExploreReq, Hud, HudConfig, Values, Widget, bar_fraction, binding_path, demo_values,
    format_template,
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

fn draw_hud(ctx: &egui::Context, idx: usize, h: &Hud, v: &Values) {
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
    for w in &h.widgets {
        match w {
            Widget::Bar {
                source,
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
                let r = egui::Rect::from_min_size(at(*pos), egui::vec2(size[0] * s, size[1] * s));
                let value = v.get(*source);
                let frac = bar_fraction(value, *min, *max);
                let red_frac = redline.map(|rl| bar_fraction(rl, *min, *max));
                let hot = redline.is_some_and(|rl| value >= rl);
                let fill = if hot { *redline_color } else { *color };
                if *segments == 0 {
                    p.rect_filled(r, 0.0, *track_color);
                    // Redline zone hinted on the track (dim, always on).
                    if let Some(rf) = red_frac {
                        let z = egui::Rect::from_min_max(
                            egui::pos2(r.min.x + r.width() * rf, r.min.y),
                            r.max,
                        );
                        p.rect_filled(z, 0.0, redline_color.gamma_multiply(0.35));
                    }
                    let f = egui::Rect::from_min_size(r.min, egui::vec2(r.width() * frac, r.height()));
                    p.rect_filled(f, 0.0, fill);
                } else {
                    let n = *segments as f32;
                    let gap = 2.0 * s;
                    let seg_w = ((r.width() - gap * (n - 1.0)) / n).max(1.0);
                    for i in 0..*segments {
                        let x = r.min.x + i as f32 * (seg_w + gap);
                        let sr = egui::Rect::from_min_size(egui::pos2(x, r.min.y), egui::vec2(seg_w, r.height()));
                        let center = (i as f32 + 0.5) / n;
                        let in_red = red_frac.is_some_and(|rf| center >= rf);
                        let c = if center <= frac {
                            if in_red { *redline_color } else { fill }
                        } else if in_red {
                            redline_color.gamma_multiply(0.35)
                        } else {
                            *track_color
                        };
                        p.rect_filled(sr, 0.0, c);
                    }
                }
            }
            Widget::Text { text, pos, size, align, color, mono } => {
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
                p.text(at(*pos), anchor, format_template(text, v), font, *color);
            }
        }
    }
}

impl HudMod {
    pub(crate) fn draw(&mut self, ctx: &egui::Context) {
        let cfg: Option<std::sync::Arc<HudConfig>> = lock(&self.config).clone();
        let values = demo_values(ctx.input(|i| i.time));
        if let Some(cfg) = &cfg {
            let mut any = false;
            for (i, h) in cfg.huds.iter().enumerate().filter(|(_, h)| h.enabled) {
                draw_hud(ctx, i, h, &values);
                any = true;
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
            ui.add(egui::Label::new(egui::RichText::new(super::i18n::demo_notice(code)).small()).wrap());
            ui.monospace(format!(
                "speed {:.0} km/h · rpm {:.0} · gear {}",
                values.speed_kmh,
                values.rpm,
                super::model::gear_label(values.gear)
            ));
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

    /// Render smoke test: every widget kind tessellates without panic
    /// (same font setup as production).
    #[test]
    fn example_renders() {
        let (cfg, _) = parse_config(DEFAULT_CONFIG).unwrap();
        let ctx = egui::Context::default();
        egui_material_icons::initialize(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        for t in 0..3 {
            ctx.begin_pass(input.clone());
            for (i, h) in cfg.huds.iter().enumerate() {
                draw_hud(&ctx, i, h, &demo_values(t as f64 * 3.3));
            }
            let mut out = ctx.end_pass();
            out.textures_delta.clear();
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            assert!(!prims.is_empty());
        }
    }
}
