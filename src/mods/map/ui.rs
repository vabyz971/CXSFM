//! Speedo render-side UI: snapshot mirror, params cards, window.
//!
//! Present thread only (`&mut` access). Edits mutate the buffers here
//! and stage one-shot write requests; the tick drains them
//! ([`super::MinimapMod::drain_batch`]). Nothing touches Unity directly.

use crate::mods::speedo::model::{
    CompKind, SavedObj, UiObj, diff_object, display_name, color_to_f32, f32_to_color, is_zero_scale,
    write_reqs_for,
};
use super::MinimapMod;
use crate::mods::common::lock;
use crate::ui::theme::{STATE_WARN as OFF_ORANGE, TEXT_BRIGHT as BRIGHT, TEXT_DIM as DIM};
use std::sync::atomic::Ordering;

impl MinimapMod {
    /// Rebuild the render-side mirror when the snapshot version bumped
    /// (discovery / restore). Brief clone, lock released before draw.
    fn sync_ui(&mut self) {
        let ver = self.snap_ver.load(Ordering::SeqCst);
        if ver == self.ui_ver {
            return;
        }
        self.ui_ver = ver;
        let snaps = lock(&self.snaps).clone();
        self.ui = snaps
            .into_iter()
            .map(|s| UiObj {
                owner: s.owner,
                comp: s.comp,
                active: s.active,
                tr_addr: s.tr_addr,
                comp_addr: s.comp_addr,
                pos: s.pos,
                euler: s.euler,
                scale: s.scale,
                color: s.color.map(f32_to_color).unwrap_or(egui::Color32::WHITE),
                opacity: s.opacity.unwrap_or(1.0),
                cg: s.cg,
                size: s.font_size,
                text: s.text,
                visible: !is_zero_scale(s.scale),
            })
            .collect();
    }

    /// One `Position / Rotation / Scale` trio row: three X/Y/Z values.
    /// Display-only when `editable` is false; otherwise `DragValue`s
    /// mutate the render-side buffer ONLY — nothing reaches Unity
    /// until the Appliquer batch. Returns true when a value changed
    /// (markers re-anchor on value-driven frames).
    /// One transform group (`Position`…) as three full-width rows
    /// (`X`/`Y`/`Z`), each with a label column and a fixed-width value
    /// field. Vertical on purpose (horizontal triples overflowed narrow
    /// panes), and fixed widths only: `available_width()` can report
    /// infinite during egui's measure phase, which used to blow the
    /// window up to 3000px. Returns true when a value moved.
    fn axis_rows(ui: &mut egui::Ui, name: &str, vals: &mut [f32; 3]) -> bool {
        let mut edited = false;
        for (axis, v) in vals.iter_mut().enumerate() {
            let tag = ["X", "Y", "Z"][axis];
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("{name}.{tag}"))
                        .weak()
                        .monospace(),
                );
                let h = ui.spacing().interact_size.y;
                if ui
                    .add_sized(
                        [96.0, h],
                        egui::DragValue::new(v).speed(0.01).max_decimals(3),
                    )
                    .changed()
                {
                    edited = true;
                }
            });
        }
        edited
    }

    /// Stage one object's buffers as write requests (render pushes,
    /// tick drains once — every edit lands in a SINGLE Unity pass).
    /// Single code path with save/load (see `write_reqs_for`).
    fn stage_object(&self, o: &UiObj) {
        let s = SavedObj {
            pos: Some(o.pos),
            euler: Some(o.euler),
            scale: Some(o.scale),
            color: Some(color_to_f32(o.color)),
            opacity: Some(o.opacity),
            active: Some(o.active),
            visible: Some(o.visible),
            size: o.size,
        };
        let reqs = write_reqs_for(&o.owner, o.comp, o.cg.is_some(), &s, Some(color_to_f32(o.color)));
        if reqs.is_empty() {
            return;
        }
        let mut slot = lock(&self.pending_batch);
        match slot.as_mut() {
            Some(v) => v.extend(reqs),
            None => *slot = Some(reqs),
        }
        self.apply_now.store(true, Ordering::SeqCst);
    }
}

impl MinimapMod {
    /// One object's full parameter card: GameObject state (with a REAL
    /// active toggle — `SetActive`, same alive-checked path as every
    /// write), LOCAL transform buffers, and — only with a graphic
    /// component — color + opacity + TMP text/size.
    /// Buffers mutate live; returns true when anything moved (the
    /// caller stages the object for the next tick drain = realtime).
    fn draw_obj_params(ui: &mut egui::Ui, obj: &mut UiObj, code: &str) -> bool {
        let mut edited = false;
        // — GameObject card: plain rows (a Grid + wrapping label
        // measured unbounded here and blew the window to 3000px — see
        // layout_stays_within_window).
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.monospace(super::i18n::card_gameobject(code));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(super::i18n::card_name(code)).weak());
                // Leaf name; full dashboard path below (paths are long,
                // the row must stay narrow — see layout test).
                let leaf = obj.owner.split('/').next_back().unwrap_or(&obj.owner);
                ui.monospace(leaf);
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(super::i18n::card_path(code)).weak());
                ui.monospace(display_name(&obj.owner));
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(super::i18n::card_active(code)).weak());
                // Real SetActive toggle (hides the whole subtree).
                // Staged like every edit — restore writes it back.
                if ui.checkbox(&mut obj.active, "").changed() {
                    edited = true;
                }
            });
            // Inactive (by the game or by us): re-enabling fires the
            // game's lifecycle callbacks from our thread — crashed the
            // game in tests (see OP_ACTIVE). Warn before the click.
            if !obj.active {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(super::i18n::active_warning(code))
                            .small()
                            .color(ui.visuals().warn_fg_color),
                    )
                    .wrap(),
                );
            }
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(super::i18n::card_visible(code)).weak());
                // Scale-zero hide (staged; object stays active so texts
                // keep updating, restore writes the original back).
                if ui.checkbox(&mut obj.visible, "").changed() {
                    edited = true;
                }
            });
        });
        // — Transform card: one full-width row per axis.
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.monospace(super::i18n::card_transform(code));
            edited |= Self::axis_rows(ui, super::i18n::axis_position(code), &mut obj.pos);
            edited |= Self::axis_rows(ui, super::i18n::axis_rotation(code), &mut obj.euler);
            edited |= Self::axis_rows(ui, super::i18n::axis_scale(code), &mut obj.scale);
        });
        // — Image / TextMeshProUGUI / RawImage card: only with a
        // component. Pure containers skip to the opacity card below.
        if obj.comp.is_some() {
            let comp_label = obj.comp.map(|c| c.label()).unwrap_or("?");
            egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.monospace(comp_label);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(super::i18n::card_color(code)).weak());
                if ui.color_edit_button_srgba(&mut obj.color).changed() {
                    edited = true;
                }
            });
            // TMP draws through a text material: some elements ignore
            // the vertex color no matter what we write.
            if obj.comp == Some(CompKind::Tmp) {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(super::i18n::color_hint(code)).small(),
                    )
                    .wrap(),
                );
            }
            if obj.comp == Some(CompKind::Tmp) {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(super::i18n::card_text(code)).weak());
                    let preview: String =
                        obj.text.chars().take(40).collect();
                    ui.monospace(format!("“{preview}”"));
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(super::i18n::card_size(code)).weak());
                    match &mut obj.size {
                        Some(size) => {
                            if ui
                                .add(
                                    egui::DragValue::new(size)
                                        .speed(0.5)
                                        .range(4.0..=300.0),
                                )
                                .changed()
                            {
                                edited = true;
                            }
                        }
                        None => {
                            ui.monospace("n/a");
                        }
                    }
                });
            }
        });
        }
        // — Opacité card: ALWAYS shown (containers included). Own group
        // first, else the nearest ancestor group (bounded climb at
        // drain), else a clear skip in the log — never a dead end
        // without explanation.
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.monospace(super::i18n::card_opacity(code));
            ui.horizontal(|ui| {
                let mut pct = obj.opacity * 100.0;
                if ui
                    .add_sized(
                        [160.0, ui.spacing().interact_size.y],
                        egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%"),
                    )
                    .changed()
                {
                    obj.opacity = (pct / 100.0).clamp(0.0, 1.0);
                    edited = true;
                }
            });
        });
        edited
    }}

impl MinimapMod {
    /// Main window: header, tree, params. Edits stage write requests
    /// as they happen (drained next tick = realtime); Refresh
    /// re-discovers, Restore writes originals back.
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {        let code = crate::i18n::current().code();
        self.sync_ui();
        // Drop a selection whose object vanished (scene change):
        // params must never edit a ghost row.
        if let Some(sel) = self.selected.clone() {
            if !self.ui.iter().any(|o| o.owner == sel) {
                self.selected = None;
            }
        }
        let mut win = egui::Window::new(super::i18n::tile_label(code))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(760.0, 540.0))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
                self.tool.enter(ui);
                self.tool.header(ui, super::i18n::tile_label(code));
                if !self.tool.hide_controls {
                ui.horizontal(|ui| {
                    if ui.button(super::i18n::refresh_label(code)).clicked() {
                        // Re-discover (scene changed, objects rebound).
                        self.pending.store(true, Ordering::SeqCst);
                    }
                    if ui.button(super::i18n::restore_label(code)).clicked() {
                        // Write back originals without disabling.
                        self.restore_now.store(true, Ordering::SeqCst);
                    }
                    if ui.button(super::i18n::save_label(code)).clicked() {
                        // Diff buffers vs first-seen originals (brief
                        // data lock — no Unity, no IO here); the tick
                        // writes the TOML file (see on_update).
                        let origs = lock(&self.origs);
                        let mut entries = Vec::new();
                        for o in &self.ui {
                            let orig = origs.iter().find(|r| r.owner == o.owner);
                            if let Some(saved) = diff_object(o, orig) {
                                entries.push((o.owner.clone(), saved));
                            }
                        }
                        drop(origs);
                        // Entries only — the tick merges them into the
                        // shared file (other mods preserved) + writes.
                        *lock(&self.save_data) = Some(entries);
                        self.save_now.store(true, Ordering::SeqCst);
                    }
                    if ui.button(super::i18n::load_label(code)).clicked() {
                        // Tick reads + parses the file, then stages one
                        // batched pass + a re-discover (see load_saved).
                        self.load_now.store(true, Ordering::SeqCst);
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.weak(format!(
                                "{} {}",
                                self.ui.len(),
                                super::i18n::objects_label(code),
                            ));
                        },
                    );
                });
                } // hide_controls
                ui.separator();
                if self.ui.is_empty() {
                    ui.weak(super::i18n::hint_label(code));
                } else {
                    // LEFT: section tree (mirrors the inspector).
                    // RIGHT: selected object's component params.
                    // (Snapshot first: selection + draw must not share
                    // the `ui` borrow.)
                    let rows: Vec<(String, bool)> = self
                        .ui
                        .iter()
                        .map(|o| (o.owner.clone(), o.active))
                        .collect();
                    // Group by section (first path segment),
                    // first-appearance order (stable across refreshes).
                    let mut sections: Vec<(String, Vec<usize>)> = Vec::new();
                    for (i, (owner, _)) in rows.iter().enumerate() {
                        let sec = owner.split('/').next().unwrap_or("").to_string();
                        match sections.iter_mut().find(|(s, _)| *s == sec) {
                            Some((_, v)) => v.push(i),
                            None => sections.push((sec, vec![i])),
                        }
                    }
                    // Two explicit columns (NOT bare `horizontal_top`): a
                    // ScrollArea inherits its parent layout, so inside a
                    // horizontal row both panes flowed sideways and blew
                    // the window to 3000px. `allocate_ui_with_layout`
                    // forces `top_down` + fixed widths instead.
                    let pane_h = ui.available_height().max(200.0);
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::Vec2::new(230.0, pane_h),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("map_list")
                            .show(ui, |ui| {
                                for (sec, idxs) in &sections {
                                    let sec_name =
                                        if sec.is_empty() { "?" } else { sec };
                                    let open = !self.collapsed.contains(sec);
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing =
                                            egui::Vec2::new(4.0, 0.0);
                                        if crate::ui::chrome::expander(ui, open)
                                            .clicked()
                                        {
                                            if open {
                                                self.collapsed.insert(sec.clone());
                                            } else {
                                                self.collapsed.remove(sec);
                                            }
                                        }
                                        let header = format!(
                                            "{} ({})",
                                            sec_name,
                                            idxs.len()
                                        );
                                        if ui
                                            .selectable_label(
                                                false,
                                                egui::RichText::new(header)
                                                    .color(BRIGHT),
                                            )
                                            .clicked()
                                        {
                                            // Header click toggles too:
                                            // sections aren't selectable.
                                            if open {
                                                self.collapsed.insert(sec.clone());
                                            } else {
                                                self.collapsed.remove(sec);
                                            }
                                        }
                                    });
                                    if open {
                                        ui.indent(
                                            format!("map_sec_{sec}"),
                                            |ui| {
                                                for &i in idxs {
                                                    let (owner, active) = &rows[i];
                                                    let leaf = owner
                                                        .split('/')
                                                        .next_back()
                                                        .unwrap_or(owner);
                                                    // Truncated leaf: rows
                                                    // never wrap, so a long
                                                    // name must not widen
                                                    // the pane (see layout
                                                    // test); the full path
                                                    // shows in the params.
                                                    let short: String = leaf
                                                        .chars()
                                                        .take(24)
                                                        .collect();
                                                    let short = if short.len()
                                                        < leaf.len()
                                                    {
                                                        format!("{short}…")
                                                    } else {
                                                        short
                                                    };
                                                    let text = if *active {
                                                        egui::RichText::new(short)
                                                            .color(BRIGHT)
                                                    } else {
                                                        egui::RichText::new(short)
                                                            .color(DIM)
                                                            .strikethrough()
                                                    };
                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut()
                                                            .item_spacing =
                                                            egui::Vec2::new(
                                                                4.0, 0.0,
                                                            );
                                                        if ui
                                                            .selectable_label(
                                                                self.selected
                                                                    .as_deref()
                                                                    == Some(
                                                                        owner.as_str(),
                                                                    ),
                                                                text,
                                                            )
                                                            .clicked()
                                                        {
                                                            self.selected = Some(
                                                                owner.clone(),
                                                            );
                                                        }
                                                        if !active {
                                                            ui.label(
                                                                egui::RichText::new(
                                                                    "OFF",
                                                                )
                                                                .small()
                                                                .strong()
                                                                .color(OFF_ORANGE),
                                                            );
                                                        }
                                                    });
                                                }
                                            },
                                        );
                                    }
                                }
                            });
                            },
                        );
                        ui.separator();
                        let rest_w = (ui.available_width() - 8.0).max(300.0);
                        ui.allocate_ui_with_layout(
                            egui::Vec2::new(rest_w, pane_h),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("map_params")
                            .show(ui, |ui| {
                                ui.set_min_width(320.0);
                                let mut edited_owner: Option<String> = None;
                                match self.selected.clone() {
                                    Some(sel) => {
                                        match self.ui.iter_mut().find(|o| o.owner == sel) {
                                            Some(obj) => {
                                                if Self::draw_obj_params(ui, obj, code) {
                                                    edited_owner =
                                                        Some(obj.owner.clone());
                                                }
                                            }
                                            None => {
                                                ui.weak(super::i18n::hint_label(code));
                                            }
                                        }
                                    }
                                    None => {
                                        ui.weak(super::i18n::select_hint(code));
                                    }
                                }
                                // Realtime: stage the touched object for
                                // the next tick drain (no Apply button).
                                if let Some(owner) = edited_owner {
                                    if let Some(obj) =
                                        self.ui.iter().find(|o| o.owner == owner)
                                    {
                                        self.stage_object(obj);
                                    }
                                }
                            });
                            },
                        );
                    });
                }
            });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mesh_max_x;
    use super::super::MinimapMod;
    use crate::mods::speedo::model::{CompKind, UiObj};
    use crate::mods::api::Mod;


    /// Layout regression: the window is fixed 760px wide, so no painted
    /// primitive may extend past it (catches unbounded rows that used
    /// to stretch the window across the whole screen).
    #[test]
    fn layout_fixed_window_sanity() {
        let ctx = egui::Context::default();
        // Same contract as production ctx creation (overlay state +
        // headless check): tool windows use Material Symbols, and an
        // unbound font family panics at tessellation.
        egui_material_icons::initialize(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1920.0, 1080.0),
            )),
            ..Default::default()
        };
        let mut max_x = 0.0f32;
        for _ in 0..3 {
            ctx.begin_pass(input.clone());
            egui::Window::new("sanity")
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .fixed_size(egui::Vec2::new(760.0, 540.0))
                .show(&ctx, |ui| {
                    ui.label("hello");
                });
            let mut out = ctx.end_pass();
            out.textures_delta.clear();
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            max_x = mesh_max_x(&prims);
        }
        assert!(max_x <= 800.0, "even a bare fixed window blows up?!");
    }

    /// Layout regression: the window is fixed 760px wide, so no painted
    /// primitive may extend past it (catches unbounded rows that used
    /// to stretch the window across the whole screen).
    #[test]
    fn layout_stays_within_window() {
        use super::super::MinimapMod;
        use crate::mods::api::Mod;
        use crate::mods::speedo::model::{CompKind, UiObj};

        fn obj(owner: &str, comp: CompKind) -> UiObj {
            UiObj {
                owner: owner.into(),
                comp: Some(comp),
                active: true,
                tr_addr: 0,
                comp_addr: Some(0),
                pos: [26.0, -116.1, 0.0],
                euler: [0.0; 3],
                scale: [1.0; 3],
                color: egui::Color32::WHITE,
                opacity: 1.0,
                cg: None,
                size: if comp == CompKind::Tmp {
                    Some(40.0)
                } else {
                    None
                },
                text: "000".into(),
                visible: true,
            }
        }

        let mut m = MinimapMod::new();
        m.ui = vec![
            obj("Map/Mask/Base/RoadImage", CompKind::Image),
            obj("Map/PlayerIconContainer", CompKind::Image),
            obj(
                "Map/NorthIcon With A Very Long Name",
                CompKind::Image,
            ),
        ];
        // Inactive selection: the re-activation warning row must wrap
        // inside the window too.
        m.ui[1].active = false;
        m.selected = Some("Map/PlayerIconContainer".into());
        // Pin the window: placement itself is not under test.
        m.tool.pos = Some(egui::pos2(100.0, 100.0));

        let ctx = egui::Context::default();
        // See layout_fixed_window_sanity: material font must be bound.
        egui_material_icons::initialize(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1920.0, 1080.0),
            )),
            ..Default::default()
        };
        let mut max_x = 0.0f32;
        // Several passes: first-frame auto-size must settle.
        for _ in 0..3 {
            ctx.begin_pass(input.clone());
            m.on_draw_ui(&ctx);
            let mut out = ctx.end_pass();
            // Font atlas upload happens on the first pass: consume the
            // deltas like the renderer does (0.36 debug-asserts this).
            out.textures_delta.clear();
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            max_x = mesh_max_x(&prims);
        }
        assert!(
            max_x <= 100.0 + 760.0 + 40.0,
            "overlay overflows its 760px window: max_x={max_x}"
        );
    }
}

/// Max painted vertex x over tessellated mesh primitives.
///
/// Layer clips (`Noop`/background) may legitimately span the screen —
/// only mesh vertices are real paint.
#[cfg(test)]
fn mesh_max_x(prims: &[egui::ClippedPrimitive]) -> f32 {
    prims
        .iter()
        .filter_map(|p| match &p.primitive {
            egui::epaint::Primitive::Mesh(m) => Some(m),
            _ => None,
        })
        .flat_map(|m| m.vertices.iter().map(|v| v.pos.x))
        .fold(0.0f32, f32::max)
}

