//! Inspector render-side UI: tree + detail panes.
//!
//! Present thread only (`&mut` access). Reads the tick's snapshot
//! under a brief lock; child expansion queues tick-side loads.

use super::model::{collect_matches, node_at, SceneNode};
use super::InspectorMod;
use crate::mods::common::lock;
use crate::ui::theme::{STATE_ERR as RED, STATE_OK as GREEN, STATE_WARN as ORANGE, TEXT_BRIGHT as BRIGHT, TEXT_DIM as DIM};
use std::collections::HashSet;
use std::sync::atomic::Ordering;

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
            .fixed_size(egui::Vec2::new(700.0, 560.0))
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
