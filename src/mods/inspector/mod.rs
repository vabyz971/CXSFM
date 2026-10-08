//! Scene inspector tool: browse what's live in the scene.
//!
//! Walks the active scene's root `GameObject`s via `SceneManager`
//! (inactive branches included — `FindObjectsOfType` would skip
//! them), then `Transform` children down. Each node shows name,
//! instance id, active flag and world position — the exact handles
//! future mods need (object names for `search_target`-style mods,
//! ids for log correlation).
//!
//! Heavy walk runs on the tick thread at ~0.5 Hz (or on Refresh); the
//! UI only reads the snapshot. Strictly read-only: an explicit
//! activate/deactivate toggle was tried and parked — `SetActive`
//! fires game lifecycle callbacks from a foreign thread and crashed
//! the game (see log). Use hud_hide's `Behaviour.enabled` pattern for
//! hiding single components instead.

pub mod i18n;

use crate::mods::api::Mod;
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Refresh cadence on the tick thread (~0.5 Hz at 60 Hz).
const REFRESH_EVERY_TICKS: u64 = 120;
/// Hard caps: a scene walk must never read the world unbounded.
const MAX_NODES: usize = 3000;
const MAX_ROOTS: usize = 512;
const MAX_CHILDREN: usize = 128;
const MAX_DEPTH: usize = 24;
/// Flat search results cap.
const MAX_MATCHES: usize = 200;

/// Readable state colors on the dark panel.
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x8F, 0xD0, 0x8A);
const ORANGE: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xA0, 0x40);
const RED: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x70, 0x60);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x8A, 0x8A, 0x92);
const BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xEC, 0xEC, 0xF0);

/// One scene object, owned data (no live handles escape the walk).
#[derive(Clone)]
struct SceneNode {
    name: String,
    id: i32,
    active: bool,
    pos: Option<[f32; 3]>,
    children: Vec<SceneNode>,
}

/// One finished walk.
struct Snapshot {
    roots: Vec<SceneNode>,
    nodes: usize,
    truncated: bool,
}

/// Scene inspector: snapshot on tick, tree on render.
pub struct InspectorMod {
    enabled: AtomicBool,
    unity: Mutex<Option<UnityCache>>,
    tick: AtomicU64,
    /// Set by the Refresh button (UI thread), consumed by the tick.
    refresh_wanted: AtomicBool,
    snapshot: Mutex<Option<Snapshot>>,
    /// Pin + opacity chrome (shared tool pattern).
    tool: ToolChrome,
    // -- UI state (render thread only, via `on_draw_ui(&mut)`) --
    search: String,
    open: HashSet<Vec<usize>>,
    selected: Option<Vec<usize>>,
}

impl InspectorMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            tick: AtomicU64::new(0),
            refresh_wanted: AtomicBool::new(false),
            snapshot: Mutex::new(None),
            tool: ToolChrome::new(),
            search: String::new(),
            open: HashSet::new(),
            selected: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn ensure_unity(&self) -> bool {
        if self.unity.lock().unwrap().is_some() {
            return true;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return false,
        };
        // SAFETY: attached framework threads, null-checked chain.
        let cache = unsafe {
            let domain = match crate::il2cpp::domain_checked(api) {
                Some(d) => d,
                None => return false,
            };
            match crate::unity::init(api, domain) {
                Some(c) => c,
                None => return false,
            }
        };
        *self.unity.lock().unwrap() = Some(cache);
        true
    }

    /// Walk one GameObject + Transform subtree (depth-first, budgeted).
    ///
    /// # Safety
    /// Attached thread, live handles consumed immediately, no handle
    /// stored past the walk.
    unsafe fn walk_go(
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
        go: *mut std::ffi::c_void,
        depth: usize,
        budget: &mut usize,
        truncated: &mut bool,
    ) -> Option<SceneNode> {
        if *budget == 0 {
            *truncated = true;
            return None;
        }
        *budget -= 1;
        // SAFETY: live object, immediate reads, nothing stored.
        unsafe {
            let name = crate::unity::object_name(api, cache, go);
            let id = crate::unity::get_instance_id(api, cache, go).unwrap_or(-1);
            let active = crate::unity::go_active(api, cache, go).unwrap_or(false);
            let tr = crate::unity::go_transform(api, cache, go);
            let pos = tr.and_then(|t| crate::unity::tr_position(api, cache, t));
            let mut children = Vec::new();
            if depth < MAX_DEPTH {
                if let Some(t) = tr {
                    let n = crate::unity::tr_child_count(api, cache, t).min(MAX_CHILDREN);
                    for i in 0..n {
                        let child_tr = match crate::unity::tr_child(api, cache, t, i as i32) {
                            Some(c) => c,
                            None => continue,
                        };
                        // Transform IS a Component: back to its GameObject.
                        let child_go =
                            match crate::unity::component_gameobject(api, cache, child_tr) {
                                Some(g) => g,
                                None => continue,
                            };
                        if let Some(node) =
                            Self::walk_go(api, cache, child_go, depth + 1, budget, truncated)
                        {
                            children.push(node);
                        }
                        if *budget == 0 {
                            *truncated = true;
                            break;
                        }
                    }
                }
            } else {
                *truncated = true;
            }
            Some(SceneNode {
                name,
                id,
                active,
                pos,
                children,
            })
        }
    }

    /// Full scene walk → snapshot (tick thread, throttled by caller).
    fn refresh(&self) {
        if !self.ensure_unity() {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        // SAFETY: attached tick thread; roots consumed immediately.
        // All loaded scenes (menu + track + cars + HUD stack up —
        // the active scene alone hides most of the game). Each scene
        // becomes a synthetic root node so paths stay plain indices.
        // Fallback: if the multi-scene walk comes back empty (API
        // gap on this build), use the active scene like before.
        let snap = unsafe {
            let mut scenes = crate::unity::loaded_scenes(api, &cache);
            if scenes.iter().all(|(_, r)| r.is_empty()) {
                let active = crate::unity::scene_root_objects(api, &cache);
                if !active.is_empty() {
                    crate::log_line(
                        "inspector: multi-scene empty, falling back to active scene",
                    );
                    scenes = vec![(String::from("active (fallback)"), active)];
                }
            }
            let mut budget = MAX_NODES;
            let mut truncated = false;
            let mut nodes = Vec::new();
            for (name, roots) in scenes {
                let mut children = Vec::new();
                for go in roots.into_iter().take(MAX_ROOTS) {
                    if budget == 0 {
                        truncated = true;
                        break;
                    }
                    if let Some(n) =
                        Self::walk_go(api, &cache, go, 1, &mut budget, &mut truncated)
                    {
                        children.push(n);
                    }
                }
                nodes.push(SceneNode {
                    name: format!("Scene: {name}"),
                    id: -2,
                    active: true,
                    pos: None,
                    children,
                });
                if budget == 0 {
                    truncated = true;
                    break;
                }
            }
            let total = MAX_NODES - budget;
            Snapshot {
                roots: nodes,
                nodes: total,
                truncated,
            }
        };
        let scene_names: Vec<String> = snap
            .roots
            .iter()
            .map(|n| {
                n.name
                    .strip_prefix("Scene: ")
                    .unwrap_or(&n.name)
                    .to_string()
            })
            .collect();
        crate::log_line(&format!(
            "inspector: walk done ({} nodes, {} scenes{} [{}])",
            snap.nodes,
            snap.roots.len(),
            if snap.truncated { ", truncated" } else { "" },
            scene_names.join(", ")
        ));
        *self.snapshot.lock().unwrap() = Some(snap);
    }
}

impl Default for InspectorMod {
    fn default() -> Self {
        Self::new()
    }
}

/// Find a node by index path.
fn node_at<'a>(roots: &'a [SceneNode], path: &[usize]) -> Option<&'a SceneNode> {
    let mut level = roots;
    let mut node = None;
    for &i in path {
        node = level.get(i);
        level = &node?.children;
    }
    node
}

/// Collect `path string` for nodes whose name contains `query`.
fn collect_matches(
    nodes: &[SceneNode],
    prefix: &mut String,
    query: &str,
    out: &mut Vec<(Vec<usize>, String)>,
    path: &mut Vec<usize>,
) {
    if out.len() >= MAX_MATCHES {
        return;
    }
    for (i, n) in nodes.iter().enumerate() {
        path.push(i);
        let full = if prefix.is_empty() {
            n.name.clone()
        } else {
            format!("{prefix}/{}", n.name)
        };
        if n.name.to_lowercase().contains(query) {
            out.push((path.clone(), full.clone()));
            if out.len() >= MAX_MATCHES {
                path.pop();
                return;
            }
        }
        let mut child_prefix = full;
        collect_matches(&n.children, &mut child_prefix, query, out, path);
        path.pop();
        if out.len() >= MAX_MATCHES {
            return;
        }
    }
}

/// One tree row (+ recursion when open).
fn draw_node(
    ui: &mut egui::Ui,
    node: &SceneNode,
    path: &mut Vec<usize>,
    open: &mut HashSet<Vec<usize>>,
    selected: &mut Option<Vec<usize>>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::new(4.0, 0.0);
        if node.children.is_empty() {
            ui.label("•");
        } else {
            let is_open = open.contains(path);
            if ui.small_button(if is_open { "▾" } else { "▸" }).clicked() {
                if is_open {
                    open.remove(path);
                } else {
                    open.insert(path.clone());
                }
            }
        }
        let label = if node.children.is_empty() {
            node.name.clone()
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
                draw_node(ui, child, path, open, selected);
                path.pop();
            }
        });
    }
}

impl Mod for InspectorMod {
    fn name(&self) -> &'static str {
        "Scene Inspector"
    }

    fn mod_id(&self) -> &'static str {
        "inspector"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_inspector"
    }

    fn menu_icon(&self) -> crate::mods::api::TileIcon {
        crate::mods::api::TileIcon::Tree
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn is_pinned(&self) -> bool {
        self.tool.pin
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.is_enabled() {
            return;
        }
        let tick = self.tick.fetch_add(1, Ordering::SeqCst);
        if self.refresh_wanted.swap(false, Ordering::SeqCst) || tick % REFRESH_EVERY_TICKS == 0 {
            self.refresh();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        // Fixed size: no auto-resize at all. Panes scroll inside;
        // the divider still adjusts the tree/details split.
        // Right-click the window for pin + opacity.
        let win = egui::Window::new("Scene Inspector")
            .resizable(false)
            .fixed_size(egui::Vec2::new(700.0, 560.0))
            .frame(self.tool.frame(ctx))
            .show(ctx, |ui| {
                self.tool.enter(ui);
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
                    self.tool.pin_toggle(ui);
                });
                let snap = self.snapshot.lock().unwrap();
                match snap.as_ref() {
                    None => {
                        ui.label("No data yet — open the menu once so the tick walks the scene.");
                    }
                    Some(s) => {
                        ui.label(format!(
                            "{} nodes / {} scenes{}",
                            s.nodes,
                            s.roots.len(),
                            if s.truncated { " (truncated)" } else { "" }
                        ));
                        ui.separator();
                        // Left: tree. Fixed strip with its own scrollbars
                        // (divider draggable).
                        egui::SidePanel::left("inspector_tree")
                            .resizable(true)
                            .default_width(270.0)
                            .width_range(180.0..=450.0)
                            .show_inside(ui, |ui| {
                                egui::ScrollArea::both()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let query = self.search.to_lowercase();
                                        if query.is_empty() {
                                            let mut path = Vec::new();
                                            for (i, root) in s.roots.iter().enumerate() {
                                                path.push(i);
                                                draw_node(
                                                    ui,
                                                    root,
                                                    &mut path,
                                                    &mut self.open,
                                                    &mut self.selected,
                                                );
                                                path.pop();
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
            self.tool.context_menu(&r.response);
        }
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        // Walk on the very next tick so the window isn't empty.
        self.refresh_wanted.store(true, Ordering::SeqCst);
        crate::log_line("inspector: armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        crate::log_line("inspector: off (read-only — nothing to restore)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(InspectorMod::new()));
}
