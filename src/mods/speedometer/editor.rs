//! UI-element editor window shared by the speedometer and the minimap:
//! section tree on the left, the selected object's parameter cards on
//! the right, Refresh / Restore / Save / Load on top.
//!
//! Present thread only. The editor owns the render-side state (mirror,
//! selection, collapsed sections, window chrome) and talks to the mod's
//! tick through [`Links`] — the same atomics and mutexes as before:
//! edits stage write requests, buttons raise flags, nothing here
//! touches Unity. Shared labels come from the speedometer's i18n (the
//! minimap's were identical); each mod passes its own [`Texts`].

use super::i18n as t;
use super::model::{
    CompKind, ObjSnap, OrigRec, SavedObj, UiObj, WriteReq, color_to_f32, diff_object, display_name,
    f32_to_color, is_zero_scale, write_reqs_for,
};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use crate::ui::kit::{self, Pane};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Fixed window size (rows truncate inside it — see the layout test).
const WIN: egui::Vec2 = egui::Vec2::new(760.0, 540.0);
/// Tree column width.
const TREE_W: f32 = 230.0;
/// Leaf names longer than this are cut in the tree (full path in the cards).
const LEAF_CHARS: usize = 24;

/// Render-side state (present thread only).
#[derive(Default)]
pub(crate) struct Editor {
    /// Mirror of the tick's snapshot, edited in place.
    pub ui: Vec<UiObj>,
    ui_ver: u64,
    /// Owner path selected in the tree.
    pub selected: Option<String>,
    /// Collapsed section names (absent = open).
    collapsed: HashSet<String>,
    /// Pin + opacity chrome (shared tool pattern).
    pub tool: ToolChrome,
}

/// The mod's tick-side state the editor reads and signals.
pub(crate) struct Links<'a> {
    pub snaps: &'a Mutex<Vec<ObjSnap>>,
    pub snap_ver: &'a AtomicU64,
    pub origs: &'a Mutex<Vec<OrigRec>>,
    pub pending_batch: &'a Mutex<Option<Vec<WriteReq>>>,
    pub apply_now: &'a AtomicBool,
    /// Re-discover (Refresh).
    pub pending: &'a AtomicBool,
    pub restore_now: &'a AtomicBool,
    pub save_data: &'a Mutex<Option<Vec<(String, SavedObj)>>>,
    pub save_now: &'a AtomicBool,
    pub load_now: &'a AtomicBool,
}

/// Per-mod strings.
pub(crate) struct Texts {
    pub title: &'static str,
    /// Shown while nothing was discovered yet.
    pub waiting: &'static str,
}

impl Editor {
    /// The whole window.
    pub fn draw(&mut self, ctx: &egui::Context, l: &Links, txt: &Texts) {
        let code = crate::i18n::current().code();
        self.sync(l);
        // A selection whose object vanished (scene change) must never
        // edit a ghost row.
        if let Some(sel) = &self.selected
            && !self.ui.iter().any(|o| &o.owner == sel)
        {
            self.selected = None;
        }
        let mut tool = std::mem::take(&mut self.tool);
        tool.show(ctx, txt.title, WIN, |ui, controls| {
            if controls {
                self.toolbar(ui, l, code);
            }
            ui.separator();
            if self.ui.is_empty() {
                kit::hint(ui, txt.waiting);
                return;
            }
            let mut edited: Option<String> = None;
            kit::two_panes(ui, "elements", TREE_W, |ui, pane| {
                if pane == Pane::Left {
                    self.tree(ui);
                    return;
                }
                let Some(sel) = self.selected.clone() else {
                    kit::hint(ui, t::select_hint(code));
                    return;
                };
                match self.ui.iter_mut().find(|o| o.owner == sel) {
                    Some(obj) => {
                        if params(ui, obj, code) {
                            edited = Some(sel);
                        }
                    }
                    None => kit::hint(ui, txt.waiting),
                }
            });
            // Realtime: stage the touched object for the next tick drain.
            if let Some(owner) = edited
                && let Some(obj) = self.ui.iter().find(|o| o.owner == owner)
            {
                stage(l, obj);
            }
        });
        self.tool = tool;
    }

    /// Rebuild the mirror when the snapshot version bumped (discovery /
    /// restore). Brief clone, lock released before drawing.
    fn sync(&mut self, l: &Links) {
        let ver = l.snap_ver.load(Ordering::SeqCst);
        if ver == self.ui_ver {
            return;
        }
        self.ui_ver = ver;
        let snaps = lock(l.snaps).clone();
        self.ui = snaps
            .into_iter()
            .map(|s| UiObj {
                owner: s.owner,
                comp: s.comp,
                active: s.active,
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

    /// Refresh / Restore / Save / Load + object count.
    fn toolbar(&self, ui: &mut egui::Ui, l: &Links, code: &str) {
        ui.horizontal(|ui| {
            if ui.button(t::refresh_label(code)).clicked() {
                l.pending.store(true, Ordering::SeqCst);
            }
            if ui.button(t::restore_label(code)).clicked() {
                l.restore_now.store(true, Ordering::SeqCst);
            }
            if ui.button(t::save_label(code)).clicked() {
                // Diff buffers vs first-seen originals (brief data lock,
                // no IO here): the tick merges + writes the TOML file.
                let origs = lock(l.origs);
                let entries: Vec<(String, SavedObj)> = self
                    .ui
                    .iter()
                    .filter_map(|o| {
                        let orig = origs.iter().find(|r| r.owner == o.owner);
                        diff_object(o, orig).map(|s| (o.owner.clone(), s))
                    })
                    .collect();
                drop(origs);
                *lock(l.save_data) = Some(entries);
                l.save_now.store(true, Ordering::SeqCst);
            }
            if ui.button(t::load_label(code)).clicked() {
                l.load_now.store(true, Ordering::SeqCst);
            }
            kit::right_hint(ui, &format!("{} {}", self.ui.len(), t::objects_label(code)));
        });
    }

    /// Section tree (first path segment, first-appearance order).
    fn tree(&mut self, ui: &mut egui::Ui) {
        let rows: Vec<(String, bool)> = self.ui.iter().map(|o| (o.owner.clone(), o.active)).collect();
        for (sec, idxs) in sections(&rows) {
            let open = !self.collapsed.contains(&sec);
            let name = if sec.is_empty() { "?" } else { sec.as_str() };
            if kit::tree_group(ui, open, &format!("{name} ({})", idxs.len())) {
                if open {
                    self.collapsed.insert(sec.clone());
                } else {
                    self.collapsed.remove(&sec);
                }
            }
            if !open {
                continue;
            }
            ui.indent(("elements_sec", &sec), |ui| {
                for i in idxs {
                    let (owner, active) = &rows[i];
                    let leaf = owner.rsplit('/').next().unwrap_or(owner);
                    let sel = self.selected.as_deref() == Some(owner.as_str());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = egui::Vec2::new(4.0, 0.0);
                        if kit::tree_item(ui, sel, &kit::short(leaf, LEAF_CHARS), *active).clicked() {
                            self.selected = Some(owner.clone());
                        }
                    });
                }
            });
        }
    }
}

/// Group row indices by first path segment, first-appearance order
/// (stable across refreshes).
fn sections(rows: &[(String, bool)]) -> Vec<(String, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, (owner, _)) in rows.iter().enumerate() {
        let sec = owner.split('/').next().unwrap_or("");
        match out.iter_mut().find(|(s, _)| s == sec) {
            Some((_, v)) => v.push(i),
            None => out.push((sec.to_string(), vec![i])),
        }
    }
    out
}

/// Stage one object's buffers as write requests (render pushes, the
/// tick drains once — every edit lands in a single Unity pass). Same
/// code path as save/load (`write_reqs_for`).
fn stage(l: &Links, o: &UiObj) {
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
    let mut slot = lock(l.pending_batch);
    match slot.as_mut() {
        Some(v) => v.extend(reqs),
        None => *slot = Some(reqs),
    }
    l.apply_now.store(true, Ordering::SeqCst);
}

/// One object's parameter cards: GameObject state (real `SetActive`
/// toggle, scale-zero hide), local transform, then — only with a
/// graphic component — color and TMP text/size, and opacity always.
/// Buffers mutate live; returns true when anything moved.
fn params(ui: &mut egui::Ui, obj: &mut UiObj, code: &str) -> bool {
    let mut edited = false;
    kit::card(ui, t::card_gameobject(code), |ui| {
        // Leaf name, full path below (paths are long, rows stay narrow).
        kit::kv(ui, t::card_name(code), obj.owner.rsplit('/').next().unwrap_or(&obj.owner));
        kit::kv(ui, t::card_path(code), &display_name(&obj.owner));
        edited |= kit::field(ui, t::card_active(code), |ui| ui.checkbox(&mut obj.active, "").changed());
        // Re-enabling fires the game's lifecycle callbacks from our
        // thread — crashed the game in tests (see OP_ACTIVE).
        if !obj.active {
            kit::warn(ui, t::active_warning(code));
        }
        edited |= kit::field(ui, t::card_visible(code), |ui| ui.checkbox(&mut obj.visible, "").changed());
    });
    kit::card(ui, t::card_transform(code), |ui| {
        edited |= kit::xyz(ui, t::axis_position(code), &mut obj.pos);
        edited |= kit::xyz(ui, t::axis_rotation(code), &mut obj.euler);
        edited |= kit::xyz(ui, t::axis_scale(code), &mut obj.scale);
    });
    if let Some(comp) = obj.comp {
        kit::card(ui, comp.label(), |ui| {
            edited |= kit::field(ui, t::card_color(code), |ui| ui.color_edit_button_srgba(&mut obj.color).changed());
            if comp == CompKind::Tmp {
                // TMP draws through a text material: some elements
                // ignore the vertex color no matter what we write.
                kit::note(ui, t::color_hint(code));
                let preview: String = obj.text.chars().take(40).collect();
                kit::kv(ui, t::card_text(code), &format!("“{preview}”"));
                edited |= kit::field(ui, t::card_size(code), |ui| match &mut obj.size {
                    Some(size) => ui.add(egui::DragValue::new(size).speed(0.5).range(4.0..=300.0)).changed(),
                    None => {
                        ui.monospace("n/a");
                        false
                    }
                });
            }
        });
    }
    // Opacity: always shown (containers included) — own group first,
    // else the nearest ancestor group at drain, else a logged skip.
    kit::card(ui, t::card_opacity(code), |ui| {
        edited |= kit::percent(ui, &mut obj.opacity);
    });
    edited
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Static links for tests (nothing is drained).
    fn with_links(f: impl FnOnce(&Links)) {
        let snaps = Mutex::new(Vec::new());
        let snap_ver = AtomicU64::new(0);
        let origs = Mutex::new(Vec::new());
        let pending_batch = Mutex::new(None);
        let save_data = Mutex::new(None);
        let flags: [AtomicBool; 5] = Default::default();
        f(&Links {
            snaps: &snaps,
            snap_ver: &snap_ver,
            origs: &origs,
            pending_batch: &pending_batch,
            apply_now: &flags[0],
            pending: &flags[1],
            restore_now: &flags[2],
            save_data: &save_data,
            save_now: &flags[3],
            load_now: &flags[4],
        });
    }

    fn obj(owner: &str, comp: CompKind) -> UiObj {
        UiObj {
            owner: owner.into(),
            comp: Some(comp),
            active: true,
            comp_addr: Some(0),
            pos: [26.0, -116.1, 0.0],
            euler: [0.0; 3],
            scale: [1.0; 3],
            color: egui::Color32::WHITE,
            opacity: 1.0,
            cg: None,
            size: if comp == CompKind::Tmp { Some(40.0) } else { None },
            text: "000".into(),
            visible: true,
        }
    }

    /// Max painted vertex x over tessellated meshes (layer clips may
    /// legitimately span the screen — only mesh vertices are paint).
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

    #[test]
    fn sections_keep_first_appearance_order() {
        let rows: Vec<(String, bool)> =
            ["B/x", "A/y", "B/z", "solo"].iter().map(|s| (s.to_string(), true)).collect();
        let s = sections(&rows);
        assert_eq!(s, vec![("B".into(), vec![0, 2]), ("A".into(), vec![1]), ("solo".into(), vec![3])]);
    }

    /// Layout regression: the window is fixed 760px wide, so no painted
    /// primitive may extend past it (catches unbounded rows that used
    /// to stretch the window across the whole screen).
    #[test]
    fn layout_stays_within_window() {
        let mut ed = Editor {
            ui: vec![
                obj("Speedometer/Background/ImgBg", CompKind::Image),
                obj("Speed/Text (TMP) Speed", CompKind::Tmp),
                obj("Speed/Text (TMP) Gear With A Very Long Name", CompKind::Tmp),
                obj("Minimap/NorthIcon With A Very Long Name", CompKind::Image),
            ],
            ..Default::default()
        };
        // Inactive selection: the re-activation warning must wrap too.
        ed.ui[1].active = false;
        ed.selected = Some("Speed/Text (TMP) Speed".into());
        // Pin the window: placement itself is not under test.
        ed.tool.pos = Some(egui::pos2(100.0, 100.0));
        let txt = Texts { title: "test", waiting: "waiting" };

        let ctx = egui::Context::default();
        // Tool windows use Material Symbols: an unbound font family
        // panics at tessellation.
        egui_material_icons::initialize(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ..Default::default()
        };
        let mut max_x = 0.0f32;
        with_links(|l| {
            // Several passes: first-frame auto-size must settle.
            for _ in 0..3 {
                ctx.begin_pass(input.clone());
                ed.draw(&ctx, l, &txt);
                let mut out = ctx.end_pass();
                // Consume the font atlas upload like the renderer does.
                out.textures_delta.clear();
                let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
                max_x = mesh_max_x(&prims);
            }
        });
        assert!(max_x > 100.0, "nothing painted?");
        assert!(max_x <= 100.0 + WIN.x + 40.0, "editor overflows its window: max_x={max_x}");
    }
}
