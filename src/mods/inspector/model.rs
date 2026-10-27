//! Inspector data model: walk caps, snapshot types, pure tree helpers,
//! and the component/field view (rows, navigation path, edit encoding).
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

// --- Component / field view -------------------------------------------------

/// Field rows / array elements shown per object (caps the reads).
pub(crate) const MAX_ROWS: usize = 256;
/// Methods listed per class.
pub(crate) const MAX_METHODS: usize = 200;
/// Components listed per GameObject.
pub(crate) const MAX_COMPONENTS: usize = 64;

/// One step from a component to the object being viewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Seg {
    /// Follow a reference field.
    Field(String),
    /// Follow element `i` of a reference array.
    Index(usize),
}

impl Seg {
    pub(crate) fn label(&self) -> String {
        match self {
            Seg::Field(f) => f.clone(),
            Seg::Index(i) => format!("[{i}]"),
        }
    }
}

/// How a row can be edited (current value inside).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Edit {
    /// Read-only (references, strings, unknown structs).
    None,
    Bool(bool),
    /// Integer of `size` bytes (signed or not), enums included.
    Int { v: i64, size: usize, signed: bool },
    /// `float` (`size` 4) or `double` (`size` 8).
    Float { v: f64, size: usize },
    /// Packed floats: Vector2/3/4, Quaternion, Color.
    Floats(Vec<f32>),
}

impl Edit {
    /// Payload to store back into the field (`None` = read-only).
    pub(crate) fn encode(&self) -> Option<Vec<u8>> {
        Some(match self {
            Edit::None => return None,
            Edit::Bool(b) => vec![*b as u8],
            Edit::Int { v, size, .. } => v.to_le_bytes().get(..*size)?.to_vec(),
            Edit::Float { v, size: 4 } => (*v as f32).to_le_bytes().to_vec(),
            Edit::Float { v, size: 8 } => v.to_le_bytes().to_vec(),
            Edit::Float { .. } => return None,
            Edit::Floats(f) => f.iter().flat_map(|x| x.to_le_bytes()).collect(),
        })
    }
}

/// Integer payload of `size` bytes, sign-extended when `signed`.
pub(crate) fn decode_int(b: &[u8], size: usize, signed: bool) -> Option<i64> {
    let b = b.get(..size)?;
    let mut buf = [0u8; 8];
    buf[..size].copy_from_slice(b);
    let v = i64::from_le_bytes(buf);
    if signed && size < 8 {
        let shift = 64 - size * 8;
        Some((v << shift) >> shift)
    } else {
        Some(v)
    }
}

/// Packed floats of a known Unity struct (`None` = not one).
pub(crate) fn float_count(class_name: &str) -> Option<usize> {
    match class_name {
        "Vector2" => Some(2),
        "Vector3" => Some(3),
        "Vector4" | "Quaternion" | "Color" => Some(4),
        _ => None,
    }
}

/// One displayed field / array element.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) type_name: String,
    /// Display text (read-only rows, and the reference target).
    pub(crate) value: String,
    pub(crate) edit: Edit,
    /// Non-null reference: step into it.
    pub(crate) follow: Option<Seg>,
}

/// One component of the selected GameObject.
#[derive(Clone, Debug)]
pub(crate) struct CompSummary {
    pub(crate) class: String,
    pub(crate) namespace: String,
    /// `Behaviour.enabled` (`None` = not a Behaviour).
    pub(crate) enabled: Option<bool>,
}

/// The object being viewed: component `comp`, then `chain`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ObjView {
    pub(crate) comp: usize,
    pub(crate) chain: Vec<Seg>,
    /// `Namespace.Class` of the object at the end of the chain.
    pub(crate) class: String,
    pub(crate) rows: Vec<Row>,
    /// `name(argc)` of the class methods (own class first, then parents).
    pub(crate) methods: Vec<String>,
    pub(crate) note: Option<String>,
}

/// Details of the selected GameObject (tick → UI).
#[derive(Clone, Debug, Default)]
pub(crate) struct Detail {
    /// Instance id of the GameObject these components belong to.
    pub(crate) id: i32,
    pub(crate) comps: Vec<CompSummary>,
    pub(crate) view: Option<ObjView>,
    pub(crate) error: Option<String>,
    /// Bumped on every reload (UI resets its edit buffers).
    pub(crate) generation: u64,
}

/// Where a request points: the node's transform (re-probed alive,
/// identity checked by instance id), a component, a path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    pub(crate) tr: usize,
    pub(crate) id: i32,
    pub(crate) comp: Option<usize>,
    pub(crate) chain: Vec<Seg>,
}

/// UI → tick request.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Req {
    /// (Re)read components, and the view when `target.comp` is set.
    Load(Target),
    /// Store `bytes` into `field` of the viewed object, then reload.
    Write { target: Target, field: String, bytes: Vec<u8> },
    /// `Behaviour.enabled` of component `target.comp`, then reload.
    Enable { target: Target, on: bool },
}

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn ints_round_trip_with_sign() {
        assert_eq!(decode_int(&[0xff], 1, true), Some(-1));
        assert_eq!(decode_int(&[0xff], 1, false), Some(255));
        assert_eq!(decode_int(&(-5i32).to_le_bytes(), 4, true), Some(-5));
        let e = Edit::Int { v: -5, size: 4, signed: true };
        assert_eq!(e.encode(), Some((-5i32).to_le_bytes().to_vec()));
    }

    #[test]
    fn floats_and_bools_encode_to_field_size() {
        assert_eq!(Edit::Float { v: 1.5, size: 4 }.encode(), Some(1.5f32.to_le_bytes().to_vec()));
        assert_eq!(Edit::Float { v: 1.5, size: 8 }.encode().map(|b| b.len()), Some(8));
        assert_eq!(Edit::Bool(true).encode(), Some(vec![1]));
        assert_eq!(Edit::Floats(vec![1.0, 2.0, 3.0]).encode().map(|b| b.len()), Some(12));
        assert_eq!(Edit::None.encode(), None);
        assert_eq!(float_count("Color"), Some(4));
    }
}
