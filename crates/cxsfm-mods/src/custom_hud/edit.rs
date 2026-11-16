//! Layout editor, pure side: the edit log, its application to
//! `custom_hud.toml` (comments and key order kept, through `toml_edit`)
//! and the geometry of moving widgets (magnetic snapping, alignment,
//! spacing distribution). No egui context, no IO: unit-tested.
//!
//! The editor mutates a working copy of the layout for instant
//! preview and records one [`Edit`] per touched key; Save sends the
//! log to the tick, which applies it to the file text and writes it
//! (atomic, `.bak` kept). Only touched keys are written: hand-written
//! comments, spacing and every other key stay as they were.

use super::model::{HudConfig, Placed, Widget};
use egui::{Color32, Rect, Vec2};

/// A value written to one key.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    Num(f32),
    Pair([f32; 2]),
    Str(String),
    Bool(bool),
    Color(Color32),
    /// Delete the key (back to its default).
    Remove,
}

/// One recorded change. `hud` / `widget` are positions in the file's
/// `[[hud]]` / `[[hud.widget]]` arrays (`Hud::index`, `Placed::index`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Edit {
    Hud { hud: usize, key: String, val: Val },
    Widget { hud: usize, widget: usize, key: String, val: Val },
    /// Append a widget built from these keys.
    AddWidget { hud: usize, fields: Vec<(String, Val)> },
    /// Append a copy of `widget` moved by `shift`.
    Duplicate { hud: usize, widget: usize, shift: [f32; 2] },
    DeleteWidget { hud: usize, widget: usize },
    /// `[sources] name = "path"` (Explorer → "use as variable").
    AddSource { name: String, path: String },
}

impl Edit {
    /// Same target key (consecutive drags coalesce into one entry).
    fn same_key(&self, other: &Edit) -> bool {
        match (self, other) {
            (Edit::Hud { hud: a, key: k, .. }, Edit::Hud { hud: b, key: l, .. }) => a == b && k == l,
            (Edit::Widget { hud: a, widget: i, key: k, .. }, Edit::Widget { hud: b, widget: j, key: l, .. }) => {
                a == b && i == j && k == l
            }
            _ => false,
        }
    }
}

/// Push `e`, replacing an entry newer than `floor` (the log length of
/// the last undo snapshot — an older entry must stay as it was for undo
/// to restore it) that sets the same key: a drag of several widgets
/// keeps one entry per widget. The search stops at the first structural
/// edit (add / duplicate / delete shift what indices mean).
pub(crate) fn push_edit(log: &mut Vec<Edit>, floor: usize, e: Edit) {
    let start = floor.min(log.len());
    for old in log[start..].iter_mut().rev() {
        if matches!(old, Edit::AddWidget { .. } | Edit::Duplicate { .. } | Edit::DeleteWidget { .. } | Edit::AddSource { .. }) {
            break;
        }
        if old.same_key(&e) {
            *old = e;
            return;
        }
    }
    log.push(e);
}

/// `#RRGGBB`, `#RRGGBBAA` when not opaque (unmultiplied, like the reader).
pub(crate) fn color_hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

/// Floats written with at most 3 decimals (no `0.30000001` noise).
fn round3(v: f32) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    (f64::from(v) * 1000.0).round() / 1000.0
}

fn edit_value(v: &Val) -> Option<toml_edit::Value> {
    Some(match v {
        Val::Num(x) => round3(*x).into(),
        Val::Pair([a, b]) => {
            let mut arr = toml_edit::Array::new();
            arr.push(round3(*a));
            arr.push(round3(*b));
            arr.into()
        }
        Val::Str(s) => s.as_str().into(),
        Val::Bool(b) => (*b).into(),
        Val::Color(c) => color_hex(*c).into(),
        Val::Remove => return None,
    })
}

/// Same value as a `toml` table entry (working-copy widgets are parsed
/// from exactly what gets written).
fn plain_value(v: &Val) -> Option<toml::Value> {
    Some(match v {
        Val::Num(x) => toml::Value::Float(round3(*x)),
        Val::Pair([a, b]) => toml::Value::Array(vec![toml::Value::Float(round3(*a)), toml::Value::Float(round3(*b))]),
        Val::Str(s) => toml::Value::String(s.clone()),
        Val::Bool(b) => toml::Value::Boolean(*b),
        Val::Color(c) => toml::Value::String(color_hex(*c)),
        Val::Remove => return None,
    })
}

/// Parse a new widget from its keys (what [`Edit::AddWidget`] writes).
pub(crate) fn widget_from_fields(fields: &[(String, Val)]) -> Option<Widget> {
    let mut t = toml::Table::new();
    for (k, v) in fields {
        if let Some(v) = plain_value(v) {
            t.insert(k.clone(), v);
        }
    }
    super::model::widget_from_table(&t)
}

/// Indentation of the first key of `t` (keeps `  key = …` style).
fn key_indent(t: &toml_edit::Table) -> String {
    t.iter()
        .next()
        .and_then(|(k, _)| t.key(k))
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .map(|p| p.rsplit('\n').next().unwrap_or("").to_string())
        .unwrap_or_default()
}

/// Set (or remove) `key` in `t`, new keys indented like their siblings.
fn set_key(t: &mut toml_edit::Table, key: &str, val: &Val) {
    match edit_value(val) {
        None => {
            t.remove(key);
        }
        Some(v) => {
            let fresh = !t.contains_key(key);
            let indent = if fresh { key_indent(t) } else { String::new() };
            match t.get_mut(key).and_then(|i| i.as_value_mut()) {
                // Keep the old value's surrounding spaces / comment.
                Some(old) => {
                    let decor = old.decor().clone();
                    *old = v;
                    *old.decor_mut() = decor;
                }
                None => {
                    t.insert(key, toml_edit::Item::Value(v));
                }
            }
            if fresh
                && !indent.is_empty()
                && let Some(mut k) = t.key_mut(key)
            {
                k.leaf_decor_mut().set_prefix(indent);
            }
        }
    }
}

fn hud_table(doc: &mut toml_edit::DocumentMut, hud: usize) -> Option<&mut toml_edit::Table> {
    doc.get_mut("hud")?.as_array_of_tables_mut()?.get_mut(hud)
}

fn widgets(t: &mut toml_edit::Table) -> Option<&mut toml_edit::ArrayOfTables> {
    if !t.contains_key("widget") {
        t.insert("widget", toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
    }
    t.get_mut("widget")?.as_array_of_tables_mut()
}

/// Fresh widget table placed after its siblings, header indented like
/// them (a table without a document position is written right after
/// the previous one — i.e. inside its own `[[hud]]`).
fn new_widget_table(aot: &toml_edit::ArrayOfTables) -> toml_edit::Table {
    let mut t = toml_edit::Table::new();
    if let Some(prev) = aot.iter().last() {
        let indent = prev
            .decor()
            .prefix()
            .and_then(|p| p.as_str())
            .map(|p| p.rsplit('\n').next().unwrap_or("").to_string())
            .unwrap_or_default();
        t.decor_mut().set_prefix(format!("\n{indent}"));
    }
    t
}

/// Apply `edits` in order to the file `text`. `Err` = the file does not
/// parse any more (nothing written); `Ok` carries the new text and one
/// warning per skipped edit (target gone, e.g. the file changed by hand
/// meanwhile).
pub(crate) fn apply_edits(text: &str, edits: &[Edit]) -> Result<(String, Vec<String>), String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    let mut warnings = Vec::new();
    for e in edits {
        let ok = match e {
            Edit::Hud { hud, key, val } => hud_table(&mut doc, *hud).map(|t| set_key(t, key, val)).is_some(),
            Edit::Widget { hud, widget, key, val } => hud_table(&mut doc, *hud)
                .and_then(widgets)
                .and_then(|a| a.get_mut(*widget))
                .map(|t| set_key(t, key, val))
                .is_some(),
            Edit::AddWidget { hud, fields } => hud_table(&mut doc, *hud)
                .and_then(widgets)
                .map(|a| {
                    let mut t = new_widget_table(a);
                    for (k, v) in fields {
                        if let Some(v) = edit_value(v) {
                            t.insert(k, toml_edit::Item::Value(v));
                        }
                    }
                    let indent = a.iter().last().map(key_indent).unwrap_or_default();
                    if !indent.is_empty() {
                        let keys: Vec<String> = t.iter().map(|(k, _)| k.to_string()).collect();
                        for k in keys {
                            if let Some(mut km) = t.key_mut(&k) {
                                km.leaf_decor_mut().set_prefix(indent.clone());
                            }
                        }
                    }
                    a.push(t);
                })
                .is_some(),
            Edit::Duplicate { hud, widget, shift } => hud_table(&mut doc, *hud)
                .and_then(widgets)
                .and_then(|a| {
                    let src = a.get(*widget)?.clone();
                    let mut t = new_widget_table(a);
                    for (k, item) in src.iter() {
                        t.insert(k, item.clone());
                        if let Some(mut km) = t.key_mut(k)
                            && let Some(p) = src.key(k).and_then(|kk| kk.leaf_decor().prefix()).and_then(|p| p.as_str())
                        {
                            km.leaf_decor_mut().set_prefix(p.rsplit('\n').next().unwrap_or("").to_string());
                        }
                    }
                    let pos = t.get("pos").and_then(|i| i.as_array()).and_then(|a| {
                        let x = a.get(0)?;
                        let y = a.get(1)?;
                        let f = |v: &toml_edit::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
                        Some([f(x)? as f32, f(y)? as f32])
                    });
                    let [x, y] = pos.unwrap_or([0.0, 0.0]);
                    set_key(&mut t, "pos", &Val::Pair([x + shift[0], y + shift[1]]));
                    a.push(t);
                    Some(())
                })
                .is_some(),
            Edit::DeleteWidget { hud, widget } => hud_table(&mut doc, *hud)
                .and_then(widgets)
                .filter(|a| *widget < a.len())
                .map(|a| a.remove(*widget))
                .is_some(),
            Edit::AddSource { name, path } => {
                if !doc.contains_key("sources") {
                    doc.insert("sources", toml_edit::table());
                }
                match doc.get_mut("sources").and_then(|i| i.as_table_like_mut()) {
                    Some(t) => {
                        t.insert(name, toml_edit::value(path.as_str()));
                        true
                    }
                    None => false,
                }
            }
        };
        if !ok {
            warnings.push(format!("{e:?}: cible introuvable, ignorée"));
        }
    }
    Ok((doc.to_string(), warnings))
}

// --- Working copy -------------------------------------------------------

/// Mutable `pos` of any widget (every type has one).
pub(crate) fn pos_mut(w: &mut Widget) -> &mut [f32; 2] {
    match w {
        Widget::Bar { pos, .. }
        | Widget::Arc { pos, .. }
        | Widget::Needle { pos, .. }
        | Widget::Ticks { pos, .. }
        | Widget::Text { pos, .. }
        | Widget::Label { pos, .. }
        | Widget::Shape { pos, .. } => pos,
    }
}

/// Remove the widget at file index `index` from a working HUD (later
/// widgets shift down, like the file array).
pub(crate) fn delete_in_copy(widgets: &mut Vec<Placed>, slots: &mut usize, index: usize) {
    widgets.retain(|p| p.index != index);
    for p in widgets.iter_mut() {
        if p.index > index {
            p.index -= 1;
        }
    }
    *slots = slots.saturating_sub(1);
}

/// Every value name a widget may bind: sources, derived sources,
/// `speed_mph`, plus whatever the reader published this frame.
pub(crate) fn variable_names(cfg: &HudConfig, live: &super::model::Values) -> Vec<String> {
    let mut v: Vec<String> = cfg
        .sources
        .iter()
        .map(|s| s.name.clone())
        .chain(cfg.derived.iter().map(|d| d.name.clone()))
        .chain(std::iter::once("speed_mph".to_string()))
        .chain(live.nums.iter().map(|(n, _)| n.clone()))
        .chain(live.texts.iter().map(|(n, _)| n.clone()))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// `{name}` / `{name:N}` placeholders of a text template, in order,
/// without duplicates (`{{` / `}}` are not placeholders).
pub(crate) fn placeholders(t: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        rest = &rest[i + 1..];
        if rest.starts_with('{') {
            rest = &rest[1..];
            continue;
        }
        let Some(j) = rest.find('}') else { break };
        let name = rest[..j].split(':').next().unwrap_or("").trim();
        if valid_var_name(name) && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
        rest = &rest[j + 1..];
    }
    out
}

/// Variables a widget reads (value, bounds, red zone, placeholders),
/// in order, without duplicates. Empty = static widget.
pub(crate) fn widget_vars(w: &Widget) -> Vec<String> {
    use super::model::Bound;
    let mut v: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        if !s.is_empty() && !v.iter().any(|x| x == s) {
            v.push(s.to_string());
        }
    };
    let src = |b: &Bound| match b {
        Bound::Src(s) => Some(s.clone()),
        Bound::Num(_) => None,
    };
    match w {
        Widget::Bar { source, min, max, redline, .. } | Widget::Needle { source, min, max, redline, .. } => {
            add(source);
            for b in [Some(min), Some(max), redline.as_ref()].into_iter().flatten() {
                if let Some(s) = src(b) {
                    add(&s);
                }
            }
        }
        Widget::Arc { source, min, max, redline, .. } => {
            if let Some(s) = source {
                add(s);
                for b in [Some(min), Some(max), redline.as_ref()].into_iter().flatten() {
                    if let Some(s) = src(b) {
                        add(&s);
                    }
                }
            }
        }
        Widget::Ticks { label_max, .. } => {
            if let Some(s) = src(label_max) {
                add(&s);
            }
        }
        Widget::Text { text, .. } | Widget::Label { text, .. } => {
            for p in placeholders(text) {
                add(&p);
            }
        }
        Widget::Shape { .. } => {}
    }
    v
}

/// Editor list filter: name, type or a variable contains `q` (case
/// insensitive; empty = everything).
pub(crate) fn widget_matches(p: &Placed, kind: &str, q: &str) -> bool {
    let q = q.trim().to_lowercase();
    q.is_empty()
        || p.name.to_lowercase().contains(&q)
        || kind.contains(&q)
        || widget_vars(&p.w).iter().any(|v| v.to_lowercase().contains(&q))
}

/// Variable name for a field (`m_currentSpeed` → `current_speed`),
/// made unique against `taken` (`_2`, `_3`…).
pub(crate) fn suggest_var_name(field: &str, taken: &[String]) -> String {
    let f = field.strip_prefix("m_").unwrap_or(field).trim_start_matches('_');
    let mut base = String::new();
    let mut prev_lower = false;
    for ch in f.chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower {
                base.push('_');
            }
            base.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else if ch.is_ascii_alphanumeric() {
            base.push(ch);
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else {
            if !base.ends_with('_') && !base.is_empty() {
                base.push('_');
            }
            prev_lower = false;
        }
    }
    let base = base.trim_matches('_').to_string();
    let base = if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) { format!("v_{base}") } else { base };
    let base = base.trim_end_matches('_').to_string();
    if !taken.contains(&base) {
        return base;
    }
    (2..).map(|i| format!("{base}_{i}")).find(|n| !taken.contains(n)).unwrap_or(base)
}

/// A variable name the TOML reader and `{placeholders}` accept.
pub(crate) fn valid_var_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s.starts_with(|c: char| c.is_ascii_digit())
}

// --- Geometry: snapping, alignment, distribution -------------------------

/// Result of a magnetic snap: corrected delta + the guide lines hit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Snap {
    pub delta: Vec2,
    pub guide_x: Option<f32>,
    pub guide_y: Option<f32>,
}

/// Snap `moving` (already moved by `delta`) so one of its left /
/// centre / right edges lands on a candidate in `xs` (same for top /
/// middle / bottom with `ys`) when within `threshold`. The closest
/// match wins per axis.
pub(crate) fn snap(moving: Rect, delta: Vec2, xs: &[f32], ys: &[f32], threshold: f32) -> Snap {
    fn best(edges: [f32; 3], cands: &[f32], thr: f32) -> Option<(f32, f32)> {
        let mut out: Option<(f32, f32)> = None;
        for e in edges {
            for &c in cands {
                let d = c - e;
                if d.abs() <= thr && out.is_none_or(|(bd, _)| d.abs() < bd.abs()) {
                    out = Some((d, c));
                }
            }
        }
        out
    }
    let m = moving.translate(delta);
    let mut s = Snap { delta, ..Snap::default() };
    if let Some((d, c)) = best([m.left(), m.center().x, m.right()], xs, threshold) {
        s.delta.x += d;
        s.guide_x = Some(c);
    }
    if let Some((d, c)) = best([m.top(), m.center().y, m.bottom()], ys, threshold) {
        s.delta.y += d;
        s.guide_y = Some(c);
    }
    s
}

/// Grid lines nearest to each edge of `r` (snap candidates).
pub(crate) fn grid_lines(r: Rect, step: f32) -> (Vec<f32>, Vec<f32>) {
    if step.is_nan() || step <= 0.5 {
        return (Vec::new(), Vec::new());
    }
    let near = |v: f32| (v / step).round() * step;
    (
        vec![near(r.left()), near(r.center().x), near(r.right())],
        vec![near(r.top()), near(r.center().y), near(r.bottom())],
    )
}

/// Left / centre / right (and top / middle / bottom) lines of `r`.
pub(crate) fn rect_lines(r: Rect) -> ([f32; 3], [f32; 3]) {
    ([r.left(), r.center().x, r.right()], [r.top(), r.center().y, r.bottom()])
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AlignOp {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

/// Per-rect move aligning each rect on `reference` (the selection box,
/// or the HUD box for a single widget).
pub(crate) fn align(rects: &[Rect], reference: Rect, op: AlignOp) -> Vec<Vec2> {
    rects
        .iter()
        .map(|r| match op {
            AlignOp::Left => Vec2::new(reference.left() - r.left(), 0.0),
            AlignOp::HCenter => Vec2::new(reference.center().x - r.center().x, 0.0),
            AlignOp::Right => Vec2::new(reference.right() - r.right(), 0.0),
            AlignOp::Top => Vec2::new(0.0, reference.top() - r.top()),
            AlignOp::VCenter => Vec2::new(0.0, reference.center().y - r.center().y),
            AlignOp::Bottom => Vec2::new(0.0, reference.bottom() - r.bottom()),
        })
        .collect()
}

/// Per-rect move giving equal gaps between neighbours (sorted by
/// position along the axis; the two outermost stay put). Fewer than 3
/// rects: nothing moves.
pub(crate) fn distribute(rects: &[Rect], horizontal: bool) -> Vec<Vec2> {
    let mut out = vec![Vec2::ZERO; rects.len()];
    if rects.len() < 3 {
        return out;
    }
    let start = |r: &Rect| if horizontal { r.left() } else { r.top() };
    let size = |r: &Rect| if horizontal { r.width() } else { r.height() };
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&a, &b| start(&rects[a]).total_cmp(&start(&rects[b])));
    let first = &rects[order[0]];
    let last = &rects[order[order.len() - 1]];
    let span = (start(last) + size(last)) - start(first);
    let total: f32 = rects.iter().map(size).sum();
    let gap = (span - total) / (rects.len() - 1) as f32;
    let mut cursor = start(first);
    for &i in &order {
        let d = cursor - start(&rects[i]);
        out[i] = if horizontal { Vec2::new(d, 0.0) } else { Vec2::new(0.0, d) };
        cursor += size(&rects[i]) + gap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_hud::model::parse_config;

    const FILE: &str = r##"# top comment
version = 2

[sources]
speed = "UITachometer/m_currentSpeed"

[[hud]]
name = "A"   # trailing comment
offset = [0.0, 0.0]

  # the bar
  [[hud.widget]]
  type = "rect"
  pos = [1.0, 2.0]

  [[hud.widget]]
  type = "text"
  text = "{speed}"

[[hud]]
name = "B"

  [[hud.widget]]
  type = "line"
"##;

    #[test]
    fn edits_keep_comments_and_targets() {
        let edits = vec![
            Edit::Widget { hud: 0, widget: 0, key: "pos".into(), val: Val::Pair([10.0, 20.5]) },
            Edit::Widget { hud: 0, widget: 1, key: "size".into(), val: Val::Num(32.0) },
            Edit::Hud { hud: 1, key: "enabled".into(), val: Val::Bool(false) },
            Edit::Widget { hud: 0, widget: 0, key: "fill".into(), val: Val::Color(Color32::from_rgba_unmultiplied(255, 0, 0, 128)) },
        ];
        let (text, w) = apply_edits(FILE, &edits).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert!(text.contains("# top comment"));
        assert!(text.contains("# trailing comment"));
        assert!(text.contains("# the bar"));
        assert!(text.contains("pos = [10.0, 20.5]"));
        assert!(text.contains("  size = 32.0"), "new key indented like siblings:\n{text}");
        assert!(text.contains("fill = \"#FF000080\""));
        let (cfg, _) = parse_config(&text).unwrap();
        assert!(!cfg.huds[1].enabled);
        assert!(matches!(cfg.huds[0].widgets[1].w, Widget::Text { size, .. } if size == 32.0));
    }

    #[test]
    fn added_widget_stays_in_its_hud() {
        let fields = vec![("type".to_string(), Val::Str("ellipse".into())), ("pos".to_string(), Val::Pair([5.0, 5.0]))];
        let (text, w) = apply_edits(FILE, &[Edit::AddWidget { hud: 0, fields: fields.clone() }]).unwrap();
        assert!(w.is_empty());
        let (cfg, _) = parse_config(&text).unwrap();
        assert_eq!(cfg.huds[0].widgets.len(), 3, "{text}");
        assert_eq!(cfg.huds[1].widgets.len(), 1);
        assert!(matches!(cfg.huds[0].widgets[2].w, Widget::Shape { .. }));
        assert_eq!(cfg.huds[0].widgets[2].w, widget_from_fields(&fields).unwrap());
    }

    #[test]
    fn duplicate_and_delete() {
        let edits = vec![
            Edit::Duplicate { hud: 0, widget: 0, shift: [10.0, 10.0] },
            Edit::DeleteWidget { hud: 0, widget: 1 },
        ];
        let (text, _) = apply_edits(FILE, &edits).unwrap();
        let (cfg, _) = parse_config(&text).unwrap();
        let ws = &cfg.huds[0].widgets;
        assert_eq!(ws.len(), 2, "{text}");
        assert!(matches!(ws[0].w, Widget::Shape { pos, .. } if pos == [1.0, 2.0]));
        assert!(matches!(ws[1].w, Widget::Shape { pos, .. } if pos == [11.0, 12.0]));
        assert_eq!(cfg.huds[1].widgets.len(), 1);
    }

    #[test]
    fn source_added_and_bad_targets_warn() {
        let edits = vec![
            Edit::AddSource { name: "gear".into(), path: "UITachometer/m_gear".into() },
            Edit::Widget { hud: 7, widget: 0, key: "pos".into(), val: Val::Pair([0.0, 0.0]) },
        ];
        let (text, w) = apply_edits(FILE, &edits).unwrap();
        assert_eq!(w.len(), 1);
        let (cfg, _) = parse_config(&text).unwrap();
        assert!(cfg.sources.iter().any(|s| s.name == "gear" && s.field == "m_gear"));
        assert!(apply_edits("[[[broken", &edits).is_err());
    }

    #[test]
    fn coalesce_respects_undo_floor() {
        let set = |x| Edit::Widget { hud: 0, widget: 0, key: "pos".into(), val: Val::Pair([x, 0.0]) };
        let mut log = Vec::new();
        push_edit(&mut log, 0, set(1.0));
        push_edit(&mut log, 0, set(2.0));
        assert_eq!(log.len(), 1);
        // Snapshot taken at len 1: the next change must not rewrite #0.
        push_edit(&mut log, 1, set(3.0));
        assert_eq!(log.len(), 2);
        assert_eq!(log[0], set(2.0));
        // Two widgets dragged together: one entry each.
        let other = |x| Edit::Widget { hud: 0, widget: 1, key: "pos".into(), val: Val::Pair([x, 0.0]) };
        push_edit(&mut log, 1, other(1.0));
        push_edit(&mut log, 1, set(4.0));
        push_edit(&mut log, 1, other(5.0));
        assert_eq!(log, vec![set(2.0), set(4.0), other(5.0)]);
        // A structural edit ends the search.
        push_edit(&mut log, 1, Edit::DeleteWidget { hud: 0, widget: 3 });
        push_edit(&mut log, 1, set(6.0));
        assert_eq!(log.len(), 5);
    }

    #[test]
    fn snapping_picks_closest_edge() {
        let r = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0));
        let s = snap(r, Vec2::new(47.0, 0.0), &[50.0, 100.0], &[], 5.0);
        // left 47 → 50 (3) beats centre 52 → 50 (2)? centre is closer.
        assert_eq!(s.delta.x, 45.0);
        assert_eq!(s.guide_x, Some(50.0));
        assert_eq!(s.guide_y, None);
        let far = snap(r, Vec2::new(20.0, 0.0), &[50.0], &[], 5.0);
        assert_eq!(far.delta.x, 20.0);
        let (gx, _) = grid_lines(Rect::from_min_size(egui::pos2(12.0, 0.0), egui::vec2(6.0, 6.0)), 10.0);
        assert_eq!(gx, vec![10.0, 20.0, 20.0]);
    }

    #[test]
    fn align_and_distribute() {
        let rs = [
            Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0)),
            Rect::from_min_size(egui::pos2(50.0, 5.0), egui::vec2(20.0, 10.0)),
            Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(10.0, 10.0)),
        ];
        let bbox = rs.iter().fold(rs[0], |a, b| a.union(*b));
        let d = align(&rs, bbox, AlignOp::Right);
        assert_eq!(d[0], Vec2::new(60.0, 0.0));
        assert_eq!(d[1], Vec2::ZERO);
        let d = distribute(&rs, true);
        // span 0..70, widths 40 → gap 15: 0..10, 25..35, 50..70.
        assert_eq!(d[0], Vec2::ZERO);
        assert_eq!(d[2], Vec2::new(5.0, 0.0));
        assert_eq!(d[1], Vec2::ZERO);
        assert_eq!(distribute(&rs[..2], true), vec![Vec2::ZERO; 2]);
    }

    #[test]
    fn variable_names() {
        let taken = vec!["current_speed".to_string()];
        assert_eq!(suggest_var_name("m_currentSpeed", &taken), "current_speed_2");
        assert_eq!(suggest_var_name("m_rpmFillAmount", &[]), "rpm_fill_amount");
        assert_eq!(suggest_var_name("_gear", &[]), "gear");
        assert_eq!(suggest_var_name("3d", &[]), "v_3d");
        assert!(valid_var_name("rpm_2"));
        assert!(!valid_var_name("2rpm"));
        assert!(!valid_var_name("r pm"));
        assert!(!valid_var_name(""));
    }

    #[test]
    fn widget_names_and_variables() {
        assert_eq!(placeholders("{gear}/{max_gear}  {speed:1} {{x}} {gear} {bad name}"), vec!["gear", "max_gear", "speed"]);
        let text = r##"
[[hud]]
  [[hud.widget]]
  type = "bar"
  name = "  Compte-tours  "
  source = "rpm"
  min = "rpm_min"
  max = 1.0
  redline = "redline"
  [[hud.widget]]
  type = "label"
  text = "{speed} km/h"
  [[hud.widget]]
  type = "rect"
  name = "Fond"
"##;
        let (cfg, _) = parse_config(text).unwrap();
        let ws = &cfg.huds[0].widgets;
        assert_eq!(ws[0].name, "Compte-tours");
        assert_eq!(ws[1].name, "");
        assert_eq!(widget_vars(&ws[0].w), vec!["rpm", "rpm_min", "redline"]);
        assert_eq!(widget_vars(&ws[1].w), vec!["speed"]);
        assert!(widget_vars(&ws[2].w).is_empty());
        assert!(widget_matches(&ws[0], "bar", "COMPTE"));
        assert!(widget_matches(&ws[0], "bar", "redl"));
        assert!(widget_matches(&ws[1], "label", "speed"));
        assert!(!widget_matches(&ws[2], "rect", "speed"));
        assert!(widget_matches(&ws[2], "rect", ""));
        // The name round-trips through an edit.
        let e = Edit::Widget { hud: 0, widget: 1, key: "name".into(), val: Val::Str("Vitesse".into()) };
        let (out, _) = apply_edits(text, &[e]).unwrap();
        assert_eq!(parse_config(&out).unwrap().0.huds[0].widgets[1].name, "Vitesse");
    }

    #[test]
    fn delete_in_copy_shifts_indices() {
        let (cfg, _) = parse_config(FILE).unwrap();
        let mut ws = cfg.huds[0].widgets.clone();
        let mut slots = cfg.huds[0].slots;
        delete_in_copy(&mut ws, &mut slots, 0);
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].index, 0);
        assert_eq!(slots, 1);
    }
}
