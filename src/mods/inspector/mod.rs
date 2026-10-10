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
//! the game (see log).

pub mod i18n;

use crate::mods::api::Mod;
use crate::mods::tool::ToolChrome;
use crate::mods::common::lock;
use crate::unity::UnityCache;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Refresh cadence on the tick thread (~0.5 Hz at 60 Hz).
const REFRESH_EVERY_TICKS: u64 = 120;
/// Hard caps: a scene walk must never read the world unbounded.
/// Children load on demand (node open in the UI), so roots stay
/// shallow and cheap — no more 3000-node truncated walks.
const MAX_ROOTS: usize = 512;
const MAX_CHILDREN: usize = 128;
/// Transform sweep cap (merge pass for DontDestroyOnLoad objects).
const MAX_SWEEP: usize = 2048;
/// Parent-climb cap while resolving a sweep hit to its root.
const MAX_CLIMB: usize = 64;
/// DontDestroyOnLoad node cap (manual refresh only).
const DDL_MAX_NODES: usize = 512;
/// Child-load budget per tick drain (on-demand expansion).
const LOAD_BUDGET: usize = 1000;
/// Flat search results cap.
const MAX_MATCHES: usize = 200;

/// Readable state colors on the dark panel.
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x8F, 0xD0, 0x8A);
const ORANGE: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xA0, 0x40);
const RED: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x70, 0x60);
const DIM: egui::Color32 = egui::Color32::from_rgb(0x8A, 0x8A, 0x92);
const BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xEC, 0xEC, 0xF0);

/// One scene object, owned data (no live handles escape the walk).
/// Children load on demand: `children` fills when the node opens in
/// the UI (`children_loaded`), `tr` re-probes the transform then.
#[derive(Clone)]
struct SceneNode {
    name: String,
    id: i32,
    active: bool,
    pos: Option<[f32; 3]>,
    /// Transform address for lazy child loads (0 = synthetic node,
    /// children preloaded).
    tr: usize,
    /// Child count read at walk time (expander shows before load).
    child_count: usize,
    children_loaded: bool,
    children: Vec<SceneNode>,
}

/// One finished walk.
struct Snapshot {
    roots: Vec<SceneNode>,
    nodes: usize,
    truncated: bool,
    /// Sweep diagnostics: transforms seen / skipped-known / added.
    sweep_seen: usize,
    sweep_skipped: usize,
    sweep_added: usize,
    /// DontDestroyOnLoad roots (manual refresh only).
    ddl_added: usize,
}

/// Scene inspector: snapshot on tick, tree on render.
pub struct InspectorMod {
    enabled: AtomicBool,
    unity: Mutex<Option<UnityCache>>,
    tick: AtomicU64,
    /// Set by the Refresh button (UI thread), consumed by the tick.
    refresh_wanted: AtomicBool,
    /// Index paths whose children the UI requested (node opened).
    /// Drained by the tick (Unity reads), then cleared.
    want_children: Mutex<Vec<Vec<usize>>>,
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
            want_children: Mutex::new(Vec::new()),
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

    /// Read one GameObject as a childless node (depth-first walks are
    /// gone: children load on demand when the node opens in the UI).
    /// Stores the transform address + child count for later expansion.
    ///
    /// # Safety
    /// Attached thread, live handles consumed immediately, no handle
    /// stored past the walk except the transform address (re-probed
    /// alive before every later use).
    unsafe fn walk_shallow(
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
        go: *mut std::ffi::c_void,
    ) -> Option<SceneNode> {
        // SAFETY: live object, immediate reads, nothing stored but data.
        unsafe {
            let name = crate::unity::object_name(api, cache, go);
            let id = crate::unity::get_instance_id(api, cache, go).unwrap_or(-1);
            let active = crate::unity::go_active(api, cache, go).unwrap_or(false);
            let (tr, pos, child_count) = match crate::unity::go_transform(api, cache, go) {
                Some(t) if !t.is_null() => {
                    let pos = crate::unity::tr_position(api, cache, t);
                    let n = crate::unity::tr_child_count(api, cache, t).min(MAX_CHILDREN);
                    (t as usize, pos, n)
                }
                _ => (0, None, 0),
            };
            Some(SceneNode {
                name,
                id,
                active,
                pos,
                tr,
                child_count,
                children_loaded: false,
                children: Vec::new(),
            })
        }
    }

    /// Synthetic (non-game) node: scene roots, sweep/DDL groups.
    fn group_node(name: String, id: i32, children: Vec<SceneNode>) -> SceneNode {
        SceneNode {
            name,
            id,
            active: true,
            pos: None,
            tr: 0,
            child_count: children.len(),
            children_loaded: true,
            children,
        }
    }

    /// Scene walk → snapshot (tick thread, throttled by caller).
    /// Roots stay shallow: children load on demand when a node opens
    /// in the UI (`want_children`). `manual` (Refresh button/enable)
    /// additionally runs the DontDestroyOnLoad walk + per-scene log;
    /// the periodic pass skips both (each walk stutters).
    fn refresh(&self, manual: bool) {
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
            if manual {
                for (i, (name, roots)) in scenes.iter().enumerate() {
                    crate::log_line(&format!(
                        "inspector: scene[{i}] '{name}' roots={}",
                        roots.len()
                    ));
                }
            }
            let mut truncated = false;
            let mut total = 0usize;
            let mut nodes = Vec::new();
            for (name, roots) in scenes {
                let mut children = Vec::new();
                for go in roots.into_iter().take(MAX_ROOTS) {
                    if let Some(n) = Self::walk_shallow(api, &cache, go) {
                        total += 1;
                        children.push(n);
                    }
                }
                nodes.push(Self::group_node(format!("Scene: {name}"), -2, children));
            }
            // Sweep: every live Transform climbed to its root, merged by
            // root instance id. This catches DontDestroyOnLoad objects
            // (managers, often UI) living in no enumerated scene, plus
            // anything the root walk missed. Already-walked ids
            // (scene tree + earlier sweep hits) are skipped — never
            // duplicated, never replaced. Shallow leaves: children
            // load when opened.
            let mut known = HashSet::new();
            for scene_node in &nodes {
                for root in &scene_node.children {
                    collect_ids(root, &mut known);
                }
            }
            let mut extra = Vec::new();
            let mut sweep_seen = 0usize;
            let mut sweep_skipped = 0usize;
            for tr in crate::unity::all_transforms(api, &cache, MAX_SWEEP) {
                sweep_seen += 1;
                let mut top = tr;
                for _ in 0..MAX_CLIMB {
                    match crate::unity::tr_parent(api, &cache, top) {
                        Some(p) => top = p,
                        None => break,
                    }
                }
                let root_go = match crate::unity::component_gameobject(api, &cache, top) {
                    Some(g) => g,
                    None => continue,
                };
                let rid = crate::unity::get_instance_id(api, &cache, root_go).unwrap_or(-1);
                if rid != -1 && !known.insert(rid) {
                    sweep_skipped += 1;
                    continue;
                }
                if let Some(n) = Self::walk_shallow(api, &cache, root_go) {
                    total += 1;
                    collect_ids(&n, &mut known);
                    extra.push(n);
                }
            }
            if !extra.is_empty() {
                nodes.push(Self::group_node(
                    String::from("Unparented (sweep)"),
                    -3,
                    extra,
                ));
            }
            // DontDestroyOnLoad (manual refresh only): the
            // `FindObjectsOfTypeAll` walk reaches what SceneManager
            // cannot — parentless transforms in a valid scene group
            // under an explicit node. Capped; never periodic.
            let mut ddl_added = 0usize;
            if manual {
                let mut ddl = Vec::new();
                for tr in crate::unity::find_all_objects_of_class(api, &cache, cache.tr_klass)
                {
                    if ddl.len() >= DDL_MAX_NODES {
                        truncated = true;
                        break;
                    }
                    if tr.is_null() {
                        continue;
                    }
                    match crate::unity::object_alive(api, &cache, tr) {
                        Some(true) => {}
                        _ => continue,
                    }
                    // Parentless only: parented objects already hang
                    // under some walked root.
                    match crate::unity::tr_parent(api, &cache, tr) {
                        Some(p) if !p.is_null() => continue,
                        _ => {}
                    }
                    match crate::unity::transform_scene_name(api, &cache, tr) {
                        Some(name) if name == "DontDestroyOnLoad" => {}
                        _ => continue,
                    }
                    let go = match crate::unity::component_gameobject(api, &cache, tr) {
                        Some(g) => g,
                        None => continue,
                    };
                    if let Some(n) = Self::walk_shallow(api, &cache, go) {
                        total += 1;
                        ddl_added += 1;
                        ddl.push(n);
                    }
                }
                if !ddl.is_empty() {
                    nodes.push(Self::group_node(
                        String::from("Scene: DontDestroyOnLoad"),
                        -4,
                        ddl,
                    ));
                }
            }
            let sweep_added = nodes
                .iter()
                .find(|n| n.name == "Unparented (sweep)")
                .map(|n| n.children.len())
                .unwrap_or(0);
            Snapshot {
                roots: nodes,
                nodes: total,
                truncated,
                sweep_seen,
                sweep_skipped,
                sweep_added,
                ddl_added,
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
            "inspector: walk done ({} nodes, {} scenes{} [{}] sweep seen={} skipped={} added={} ddl={})",
            snap.nodes,
            snap.roots.len(),
            if snap.truncated { ", truncated" } else { "" },
            scene_names.join(", "),
            snap.sweep_seen,
            snap.sweep_skipped,
            snap.sweep_added,
            snap.ddl_added
        ));
        *self.snapshot.lock().unwrap() = Some(snap);
    }

    /// Load one level of children for UI-opened nodes (tick thread).
    /// Paths come from `want_children` (brief lock, taken); every
    /// transform re-probes alive before its reads, grafts go back
    /// under a brief lock. Budgeted per drain.
    fn load_children(&self, paths: Vec<Vec<usize>>) {
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
        let mut spent = 0usize;
        // SAFETY: attached tick thread; handles consumed immediately,
        // grafted as owned data only.
        unsafe {
            for path in paths {
                if spent >= LOAD_BUDGET {
                    break;
                }
                // Snapshot the transform (brief lock, no Unity under it).
                let tr = {
                    let snap = self.snapshot.lock().unwrap();
                    let roots = match snap.as_ref() {
                        Some(s) => &s.roots,
                        None => continue,
                    };
                    match node_at(roots, &path) {
                        Some(n) if !n.children_loaded && n.tr != 0 => n.tr as *mut std::ffi::c_void,
                        _ => continue,
                    }
                };
                match crate::unity::object_alive(api, &cache, tr) {
                    Some(true) => {}
                    _ => continue,
                }
                let n = crate::unity::tr_child_count(api, &cache, tr).min(MAX_CHILDREN);
                let mut children = Vec::new();
                for i in 0..n {
                    if spent >= LOAD_BUDGET {
                        break;
                    }
                    let child_tr = match crate::unity::tr_child(api, &cache, tr, i as i32) {
                        Some(c) => c,
                        None => continue,
                    };
                    // Transform IS a Component: back to its GameObject.
                    let child_go =
                        match crate::unity::component_gameobject(api, &cache, child_tr) {
                            Some(g) => g,
                            None => continue,
                        };
                    if let Some(node) = Self::walk_shallow(api, &cache, child_go) {
                        spent += 1;
                        children.push(node);
                    }
                }
                // Graft (brief lock, no Unity under it).
                let mut snap = self.snapshot.lock().unwrap();
                if let Some(roots) = snap.as_mut().map(|s| &mut s.roots) {
                    if let Some(node) = node_at_mut(roots, &path) {
                        node.children = children;
                        node.children_loaded = true;
                    }
                }
            }
        }
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

/// Mutable twin (child grafting on lazy load).
fn node_at_mut<'a>(roots: &'a mut [SceneNode], path: &[usize]) -> Option<&'a mut SceneNode> {
    let (head, tail) = path.split_first()?;
    let node = roots.get_mut(*head)?;
    if tail.is_empty() {
        Some(node)
    } else {
        node_at_mut(&mut node.children, tail)
    }
}

/// Collect every node id in a subtree (sweep dedupe set).
fn collect_ids(node: &SceneNode, set: &mut HashSet<i32>) {
    set.insert(node.id);
    for child in &node.children {
        collect_ids(child, set);
    }
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
            if ui.small_button(if is_open { "▾" } else { "▸" }).clicked() {
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
        // Manual refresh (button/enable) also runs the
        // DontDestroyOnLoad walk; the periodic pass skips it.
        let manual = self.refresh_wanted.swap(false, Ordering::SeqCst);
        if manual || tick % REFRESH_EVERY_TICKS == 0 {
            self.refresh(manual);
        }
        // Drain on-demand child loads (UI-opened nodes, Unity here).
        let want = std::mem::take(&mut *lock(&self.want_children));
        if !want.is_empty() {
            self.load_children(want);
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
