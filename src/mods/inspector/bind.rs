//! Inspector tick-side Unity work: resolve, walks, child loads.
//!
//! Runs on the framework tick thread with an attached IL2CPP domain.
//! Every handle is alive-checked before any invoke; stored transform
//! addresses are re-probed before every later use.

use super::model::{
    DDL_MAX_NODES, LOAD_BUDGET, MAX_CHILDREN, MAX_CLIMB, MAX_ROOTS, MAX_SWEEP, SceneNode,
    Snapshot, collect_ids, group_node, node_at, node_at_mut,
};
use std::collections::HashSet;
use super::InspectorMod;
use crate::unity::UnityCache;

impl InspectorMod {
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

    /// Scene walk → snapshot (tick thread, throttled by caller).
    /// Roots stay shallow: children load on demand when a node opens
    /// in the UI (`want_children`). `manual` (Refresh button/enable)
    /// additionally runs the DontDestroyOnLoad walk + per-scene log;
    /// the periodic pass skips both (each walk stutters).
    pub(crate) fn refresh(&self, manual: bool) {
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
                nodes.push(group_node(format!("Scene: {name}"), -2, children));
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
                nodes.push(group_node(
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
                    nodes.push(group_node(
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
    pub(crate) fn load_children(&self, paths: Vec<Vec<usize>>) {
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
