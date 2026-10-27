//! Inspector render-side UI: tree + detail panes.
//!
//! Present thread only (`&mut` access). Reads the tick's snapshot
//! under a brief lock; child expansion queues tick-side loads. The
//! detail pane lists the selection's components and the field table
//! of the opened one; edits and reloads are queued for the tick (no
//! Unity call here).

use super::model::{collect_matches, node_at, Detail, Edit, Req, Row, SceneNode, Target};
use super::InspectorMod;
use crate::mods::common::lock;
use crate::ui::theme::{STATE_ERR as RED, STATE_OK as GREEN, STATE_WARN as ORANGE, TEXT_BRIGHT as BRIGHT, TEXT_DIM as DIM};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

/// Detail-pane UI state (render thread only).
#[derive(Default)]
pub(crate) struct DetUi {
    /// What the pane shows (sent with every request).
    target: Option<Target>,
    /// Selection the target was built for.
    for_path: Option<Vec<usize>>,
    /// Re-read the view every second.
    live: bool,
    last_live: Option<std::time::Instant>,
}

fn push(req: &Mutex<Vec<Req>>, r: Req) {
    let mut q = lock(req);
    // Coalesce reloads: only the newest one matters.
    if matches!(r, Req::Load(_)) {
        q.retain(|x| !matches!(x, Req::Load(_)));
    }
    q.push(r);
}

/// Editable value widget; returns the payload to write when committed.
fn edit_widget(ui: &mut egui::Ui, row: &mut Row) -> Option<Vec<u8>> {
    // Commit on release / Enter, not every drag pixel (each write
    // re-walks the object on the tick).
    let done = |r: &egui::Response| r.drag_stopped() || (r.changed() && !r.dragged());
    let commit = match &mut row.edit {
        Edit::None => {
            ui.monospace(&row.value);
            false
        }
        Edit::Bool(b) => ui.checkbox(b, "").changed(),
        Edit::Int { v, size, signed } => {
            let (lo, hi) = match (*size, *signed) {
                (1, true) => (i8::MIN as i64, i8::MAX as i64),
                (1, false) => (0, u8::MAX as i64),
                (2, true) => (i16::MIN as i64, i16::MAX as i64),
                (2, false) => (0, u16::MAX as i64),
                (4, true) => (i32::MIN as i64, i32::MAX as i64),
                (4, false) => (0, u32::MAX as i64),
                _ => (i64::MIN, i64::MAX),
            };
            let r = ui.add(egui::DragValue::new(v).range(lo..=hi).speed(0.2));
            done(&r)
        }
        Edit::Float { v, .. } => {
            let r = ui.add(egui::DragValue::new(v).speed(0.01).max_decimals(4));
            done(&r)
        }
        Edit::Floats(f) if row.type_name == "Color" && f.len() == 4 => {
            let mut c = [f[0], f[1], f[2], f[3]];
            let changed = ui.color_edit_button_rgba_unmultiplied(&mut c).changed();
            let mut any = false;
            for (x, nc) in f.iter_mut().zip(c) {
                *x = nc;
            }
            for x in f.iter_mut() {
                let r = ui.add(egui::DragValue::new(x).speed(0.01).max_decimals(3));
                any |= done(&r);
            }
            changed || any
        }
        Edit::Floats(f) => {
            let mut any = false;
            for x in f.iter_mut() {
                let r = ui.add(egui::DragValue::new(x).speed(0.01).max_decimals(3));
                any |= done(&r);
            }
            any
        }
    };
    if commit { row.edit.encode() } else { None }
}

/// Components + field table of the selected node.
fn detail_pane(
    ui: &mut egui::Ui,
    node: &SceneNode,
    path: &[usize],
    detail: &Mutex<Option<Detail>>,
    req: &Mutex<Vec<Req>>,
    st: &mut DetUi,
) {
    if node.tr == 0 || node.id < 0 {
        return;
    }
    // New selection: list its components.
    if st.for_path.as_deref() != Some(path) || st.target.as_ref().is_none_or(|t| t.id != node.id) {
        let t = Target { tr: node.tr, id: node.id, comp: None, chain: Vec::new() };
        st.for_path = Some(path.to_vec());
        st.target = Some(t.clone());
        *lock(detail) = None;
        push(req, Req::Load(t));
    }
    let Some(target) = st.target.clone() else { return };
    // Live view: re-read once a second, never while a value is held.
    if st.live
        && !ui.input(|i| i.pointer.any_down())
        && st.last_live.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1))
    {
        st.last_live = Some(std::time::Instant::now());
        push(req, Req::Load(target.clone()));
    }
    if st.live {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Components").strong().color(BRIGHT));
        if ui.button("Reread").clicked() {
            push(req, Req::Load(target.clone()));
        }
        ui.checkbox(&mut st.live, "live (1 s)");
    });
    let mut guard = lock(detail);
    let Some(d) = guard.as_mut().filter(|d| d.id == node.id) else {
        ui.weak("reading…");
        return;
    };
    if let Some(e) = &d.error {
        ui.label(egui::RichText::new(e).color(RED));
    }
    let mut next: Option<Target> = None;
    for (i, c) in d.comps.iter().enumerate() {
        ui.horizontal(|ui| {
            if let Some(mut on) = c.enabled
                && ui.checkbox(&mut on, "").changed()
            {
                push(req, Req::Enable { target: Target { comp: Some(i), chain: Vec::new(), ..target.clone() }, on });
            }
            let label = if c.namespace.is_empty() { c.class.clone() } else { format!("{}  ({})", c.class, c.namespace) };
            if ui.selectable_label(target.comp == Some(i) && target.chain.is_empty(), label).clicked() {
                next = Some(Target { comp: Some(i), chain: Vec::new(), ..target.clone() });
            }
        });
    }
    if let Some(v) = d.view.as_mut().filter(|v| Some(v.comp) == target.comp) {
        ui.separator();
        // Breadcrumb: component › field › [i] …
        ui.horizontal_wrapped(|ui| {
            let comp_name = d.comps.get(v.comp).map_or("?".to_string(), |c| c.class.clone());
            if ui.link(comp_name).clicked() {
                next = Some(Target { chain: Vec::new(), ..target.clone() });
            }
            for k in 0..v.chain.len() {
                ui.label("›");
                if ui.link(v.chain[k].label()).clicked() {
                    next = Some(Target { chain: v.chain[..=k].to_vec(), ..target.clone() });
                }
            }
        });
        ui.label(egui::RichText::new(&v.class).monospace().color(DIM));
        if let Some(n) = &v.note {
            ui.label(egui::RichText::new(n).color(ORANGE));
        }
        if v.rows.iter().any(|r| r.edit != Edit::None) {
            ui.label(
                egui::RichText::new("Edits are raw field writes: the game may overwrite them, and disabling the mod does not undo them.")
                    .small()
                    .color(DIM),
            );
        }
        let mut write: Option<(String, Vec<u8>)> = None;
        egui::Grid::new("inspector_fields").striped(true).num_columns(4).show(ui, |ui| {
            for row in v.rows.iter_mut() {
                ui.label(egui::RichText::new(&row.name).color(BRIGHT));
                ui.label(egui::RichText::new(&row.type_name).color(DIM).small());
                ui.horizontal(|ui| {
                    if let Some(bytes) = edit_widget(ui, row) {
                        write = Some((row.name.clone(), bytes));
                    }
                });
                if let Some(seg) = &row.follow {
                    if ui.small_button("open").clicked() {
                        let mut chain = v.chain.clone();
                        chain.push(seg.clone());
                        next = Some(Target { chain, ..target.clone() });
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
        if let Some((field, bytes)) = write {
            push(req, Req::Write { target: Target { chain: v.chain.clone(), ..target.clone() }, field, bytes });
        }
        if !v.methods.is_empty() {
            egui::CollapsingHeader::new(format!("Methods ({})", v.methods.len())).id_salt("inspector_methods").show(ui, |ui| {
                for m in &v.methods {
                    ui.monospace(m);
                }
            });
        }
    }
    drop(guard);
    if let Some(t) = next {
        st.target = Some(t.clone());
        push(req, Req::Load(t));
    }
}

/// One tree row (+ recursion when open). Unloaded nodes with a known
/// child count request a tick-side load via `want` (drained in
/// `on_update`, never loaded here — no Unity on the render thread).
fn draw_node(
    ui: &mut egui::Ui,
    node: &SceneNode,
    path: &mut Vec<usize>,
    open: &mut HashSet<Vec<usize>>,
    selected: &mut Option<Vec<usize>>,
    want: &mut Vec<Vec<usize>>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::new(4.0, 0.0);
        let expandable = !node.children.is_empty() || (!node.children_loaded && node.child_count > 0);
        if !expandable {
            ui.label("•");
        } else {
            let is_open = open.contains(path);
            // Painted chevron (see `chrome::expander`): text glyphs
            // (`▾`/`▸`) came out as tofu boxes in-game.
            if crate::ui::chrome::expander(ui, is_open).clicked() {
                if is_open {
                    open.remove(path);
                } else {
                    open.insert(path.clone());
                }
            }
            // Lazily expand: opening an unloaded node queues one
            // level for the tick. Deduped — the queue drains fast.
            if open.contains(path) && !node.children_loaded && node.tr != 0 && !want.contains(path) {
                want.push(path.clone());
            }
        }
        let label = if node.children.is_empty() {
            if !node.children_loaded && node.child_count > 0 {
                format!("{} ({}…)", node.name, node.child_count)
            } else {
                node.name.clone()
            }
        } else {
            format!("{} ({})", node.name, node.children.len())
        };
        // Contrast: active nodes bright, inactive ones dimmed +
        // struck through with a readable orange OFF badge (the old
        // weak-gray badge was invisible on the dark panel).
        let text = if node.active {
            egui::RichText::new(label).color(BRIGHT)
        } else {
            egui::RichText::new(label).color(DIM).strikethrough()
        };
        if ui
            .selectable_label(selected.as_ref() == Some(path), text)
            .clicked()
        {
            *selected = Some(path.clone());
        }
        if !node.active {
            ui.label(egui::RichText::new("OFF").small().strong().color(ORANGE));
        }
    });
    if open.contains(path) {
        let indent_id = path
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("/");
        ui.indent(indent_id, |ui| {
            for (i, child) in node.children.iter().enumerate() {
                path.push(i);
                draw_node(ui, child, path, open, selected, want);
                path.pop();
            }
        });
    }
}

impl InspectorMod {
    /// Main window: tree + detail panes (render thread only).
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        // Fixed size: no auto-resize at all. Panes scroll inside;
        // the divider still adjusts the tree/details split.
        // Right-click the window for pin + opacity.
        let mut win = egui::Window::new("Scene Inspector")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(1100.0, 720.0))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
                self.tool.enter(ui);
                self.tool.header(ui, "Scene Inspector");
                if !self.tool.hide_controls {
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("filter by name…")
                            .desired_width(f32::INFINITY),
                    );
                    if resp.changed() {
                        self.selected = None;
                    }
                    if ui.button("Refresh").clicked() {
                        self.refresh_wanted.store(true, Ordering::SeqCst);
                    }
                });
                } // hide_controls
                let snap = self.snapshot.lock().unwrap();
                match snap.as_ref() {
                    None => {
                        ui.label("No data yet — open the menu once so the tick walks the scene.");
                    }
                    Some(s) => {
                        ui.label(format!(
                            "{} nodes / {} scenes{} / sweep +{}",
                            s.nodes,
                            s.roots.len(),
                            if s.truncated { " (truncated)" } else { "" },
                            s.sweep_added
                        ));
                        ui.separator();
                        // Left: tree. Fixed strip with its own scrollbars
                        // (divider draggable).
                        egui::Panel::left("inspector_tree")
                            .resizable(true)
                            .default_size(270.0)
                            .size_range(180.0..=450.0)
                            .show(ui, |ui| {
                                egui::ScrollArea::both()
                                    .id_salt("inspector_tree_scroll")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let query = self.search.to_lowercase();
                                        if query.is_empty() {
                                            let mut path = Vec::new();
                                            // On-demand expansion: opened
                                            // nodes queue one child level
                                            // for the tick (no Unity here).
                                            let mut want = Vec::new();
                                            for (i, root) in s.roots.iter().enumerate() {
                                                path.push(i);
                                                draw_node(
                                                    ui,
                                                    root,
                                                    &mut path,
                                                    &mut self.open,
                                                    &mut self.selected,
                                                    &mut want,
                                                );
                                                path.pop();
                                            }
                                            if !want.is_empty() {
                                                lock(&self.want_children).extend(want);
                                            }
                                        } else {
                                            let mut out = Vec::new();
                                            collect_matches(
                                                &s.roots,
                                                &mut String::new(),
                                                &query,
                                                &mut out,
                                                &mut Vec::new(),
                                            );
                                            for (p, full) in out {
                                                if ui
                                                    .selectable_label(
                                                        self.selected.as_ref() == Some(&p),
                                                        full,
                                                    )
                                                    .clicked()
                                                {
                                                    self.selected = Some(p);
                                                }
                                            }
                                        }
                                    });
                            });
                        // Right: values of the selected element.
                        egui::ScrollArea::vertical()
                            .id_salt("inspector_detail_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let path = match self.selected.clone() {
                                    Some(p) => p,
                                    None => {
                                        ui.weak("← Select a node to inspect its values.");
                                        return;
                                    }
                                };
                                let n = match node_at(&s.roots, &path) {
                                    Some(n) => n,
                                    None => {
                                        ui.weak("Selection is stale — Refresh.");
                                        return;
                                    }
                                };
                                ui.label(
                                    egui::RichText::new(&n.name).strong().size(16.0).color(BRIGHT),
                                );
                                ui.label(if n.active {
                                    egui::RichText::new("● active").color(GREEN)
                                } else {
                                    egui::RichText::new("● inactive").color(RED)
                                });
                                ui.separator();
                                let path_str = path
                                    .iter()
                                    .map(|i| i.to_string())
                                    .collect::<Vec<_>>()
                                    .join("/");
                                for (k, v) in [
                                    (
                                        "id",
                                        if n.id == -2 {
                                            String::from("— (scene)")
                                        } else {
                                            n.id.to_string()
                                        },
                                    ),
                                    (
                                        "position",
                                        match n.pos {
                                            Some(p) => format!(
                                                "({:.1}, {:.1}, {:.1})",
                                                p[0], p[1], p[2]
                                            ),
                                            None => String::from("unknown"),
                                        },
                                    ),
                                    ("children", n.children.len().to_string()),
                                    ("path", path_str),
                                ] {
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new(k).color(DIM));
                                        ui.monospace(v);
                                    });
                                }
                                detail_pane(ui, n, &path, &self.detail, &self.detail_req, &mut self.det_ui);
                                if !n.children.is_empty() {
                                    ui.separator();
                                    ui.weak("direct children (click to jump):");
                                    for (i, child) in n.children.iter().enumerate() {
                                        let mut child_path = path.clone();
                                        child_path.push(i);
                                        if ui
                                            .selectable_label(false, &child.name)
                                            .clicked()
                                        {
                                            self.open.insert(path.clone());
                                            self.selected = Some(child_path);
                                        }
                                    }
                                }
                            });
                    }
                }
            });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
        }
    }
}
