//! Inspector data model: walk caps, snapshot types, pure tree helpers.
//!
//! No Unity calls here — everything touching the game lives in
//! `bind.rs`, everything drawn lives in `ui.rs`.

use std::collections::HashSet;

/// Hard caps: a scene walk must never read the world unbounded.
/// Children load on demand (node open in the UI), so roots stay
/// shallow and cheap — no more 3000-node truncated walks.
pub(crate) const MAX_ROOTS: usize = 512;
pub(crate) const MAX_CHILDREN: usize = 128;
/// Transform sweep cap (merge pass for DontDestroyOnLoad objects).
pub(crate) const MAX_SWEEP: usize = 2048;
/// Parent-climb cap while resolving a sweep hit to its root.
pub(crate) const MAX_CLIMB: usize = 64;
/// DontDestroyOnLoad node cap (manual refresh only).
pub(crate) const DDL_MAX_NODES: usize = 512;
/// Child-load budget per tick drain (on-demand expansion).
pub(crate) const LOAD_BUDGET: usize = 1000;
/// Flat search results cap.
pub(crate) const MAX_MATCHES: usize = 200;

/// One scene object, owned data (no live handles escape the walk).
/// Children load on demand: `children` fills when the node opens in
/// the UI (`children_loaded`), `tr` re-probes the transform then.
#[derive(Clone)]
pub(crate) struct SceneNode {
    pub(crate) name: String,
    pub(crate) id: i32,
    pub(crate) active: bool,
    pub(crate) pos: Option<[f32; 3]>,
    /// Transform address for lazy child loads (0 = synthetic node,
    /// children preloaded).
    pub(crate) tr: usize,
    /// Child count read at walk time (expander shows before load).
    pub(crate) child_count: usize,
    pub(crate) children_loaded: bool,
    pub(crate) children: Vec<SceneNode>,
}

/// One finished walk.
pub(crate) struct Snapshot {
    pub(crate) roots: Vec<SceneNode>,
    pub(crate) nodes: usize,
    pub(crate) truncated: bool,
    /// Sweep diagnostics: transforms seen / skipped-known / added.
    pub(crate) sweep_seen: usize,
    pub(crate) sweep_skipped: usize,
    pub(crate) sweep_added: usize,
    /// DontDestroyOnLoad roots (manual refresh only).
    pub(crate) ddl_added: usize,
}

    /// Synthetic (non-game) node: scene roots, sweep/DDL groups.
    pub(crate) fn group_node(name: String, id: i32, children: Vec<SceneNode>) -> SceneNode {
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

/// Find a node by index path.
pub(crate) fn node_at<'a>(roots: &'a [SceneNode], path: &[usize]) -> Option<&'a SceneNode> {
    let mut level = roots;
    let mut node = None;
    for &i in path {
        node = level.get(i);
        level = &node?.children;
    }
    node
}

/// Mutable twin (child grafting on lazy load).
pub(crate) fn node_at_mut<'a>(roots: &'a mut [SceneNode], path: &[usize]) -> Option<&'a mut SceneNode> {
    let (head, tail) = path.split_first()?;
    let node = roots.get_mut(*head)?;
    if tail.is_empty() {
        Some(node)
    } else {
        node_at_mut(&mut node.children, tail)
    }
}

/// Collect every node id in a subtree (sweep dedupe set).
pub(crate) fn collect_ids(node: &SceneNode, set: &mut HashSet<i32>) {
    set.insert(node.id);
    for child in &node.children {
        collect_ids(child, set);
    }
}

/// Collect `path string` for nodes whose name contains `query`.
pub(crate) fn collect_matches(
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
