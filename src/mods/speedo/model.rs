//! Speedo data model: snapshots, originals, write requests, pure helpers.
//!
//! No Unity calls here — everything touching the game lives in
//! `bind.rs`, everything drawn lives in `ui.rs`. Containers (Turbo,
//! Speedometer…) are first-class entries with `comp: None`: only
//! transform + active state apply to them.

use crate::unity::UnityCache;

/// Component flavour of a bound object (`None` = pure container:
/// transform + active state only, no color/text).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompKind {
    Image,
    /// Dynamic textures (minimap roads): sibling of `Image`, probed
    /// separately — `GetComponent(Image)` never matches it.
    Raw,
    Tmp,
}

impl CompKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            CompKind::Image => "Image",
            CompKind::Raw => "RawImage",
            CompKind::Tmp => "TextMeshProUGUI",
        }
    }

    /// Cached IL2CPP class for `GetComponent` (null = unavailable).
    pub(crate) fn klass(self, cache: &UnityCache) -> *mut std::ffi::c_void {
        match self {
            CompKind::Image => cache.image_klass,
            CompKind::Raw => cache.rawimage_klass,
            CompKind::Tmp => cache.tmp_klass,
        }
    }
}

/// Color read routed by component kind: TMP renders its own
/// `m_fontColor` (override), everything else the base `Graphic` color.
/// Falls back to the `Graphic` pair when the TMP override is missing.
///
/// # Safety
/// Attached tick thread, alive-checked `obj` (same as every helper).
pub(crate) unsafe fn color_get(
    api: &crate::il2cpp::Il2cppApi,
    cache: &UnityCache,
    obj: *mut std::ffi::c_void,
    kind: Option<CompKind>,
) -> Option<[f32; 4]> {
    // SAFETY: forwarded caller contract.
    unsafe {
        if kind == Some(CompKind::Tmp) && !cache.m_tmp_get_color.is_null() {
            crate::unity::tmp_get_color(api, cache, obj)
        } else {
            crate::unity::graphic_get_color(api, cache, obj)
        }
    }
}

/// Color write, same routing as [`color_get`].
///
/// # Safety
/// Same contract as [`color_get`].
pub(crate) unsafe fn color_set(
    api: &crate::il2cpp::Il2cppApi,
    cache: &UnityCache,
    obj: *mut std::ffi::c_void,
    kind: Option<CompKind>,
    color: [f32; 4],
) -> bool {
    // SAFETY: forwarded caller contract.
    unsafe {
        if kind == Some(CompKind::Tmp) && !cache.m_tmp_set_color.is_null() {
            crate::unity::tmp_set_color(api, cache, obj, color)
        } else {
            crate::unity::graphic_set_color(api, cache, obj, color)
        }
    }
}

/// Write-request opcodes (plain `u8`, `Copy` across threads).
pub(crate) const OP_SCALE: u8 = 0;
pub(crate) const OP_POS: u8 = 1;
pub(crate) const OP_COLOR: u8 = 2;
pub(crate) const OP_FONT: u8 = 3;
pub(crate) const OP_ROT: u8 = 4;
pub(crate) const OP_CGALPHA: u8 = 5;
/// `GameObject.SetActive` (`v[0]` = 1.0 active, 0.0 inactive).
/// CAUTION: flipping active fires game lifecycle callbacks from a
/// foreign thread — the inspector's `SetActive` toggle crashed the
/// game for exactly this reason. Alive-checked like every write, but
/// if toggles start crashing, this op goes first.
pub(crate) const OP_ACTIVE: u8 = 6;

/// Published snapshot of one object (tick writes on discovery, render
/// clones; swapped under a brief lock, never held across Unity calls).
/// `pos`/`euler` are LOCAL (what persists on UI objects — the Canvas
/// layout discards world-space writes).
#[derive(Clone)]
pub(crate) struct ObjSnap {
    pub(crate) owner: String,
    pub(crate) comp: Option<CompKind>,
    pub(crate) active: bool,
    /// Graphic component address (diagnostic hint only — drains always
    /// re-resolve; never dereferenced).
    #[allow(dead_code)]
    pub(crate) comp_addr: Option<usize>,
    pub(crate) pos: [f32; 3],
    pub(crate) euler: [f32; 3],
    pub(crate) scale: [f32; 3],
    pub(crate) color: Option<[f32; 4]>,
    pub(crate) opacity: Option<f32>,
    /// CanvasGroup address when present (opacity path), else `None`
    /// (color-alpha fallback for graphics, N/A for containers).
    pub(crate) cg: Option<usize>,
    pub(crate) font_size: Option<f32>,
    pub(crate) text: String,
}

/// First-recorded originals (restore source).
/// `owner` is the re-resolve key (slash path under the dashboard
/// root): raw addresses go stale on scene change (menu → city rebuilds
/// the HUD), so restore re-finds by path and writes to the *live*
/// object, never the stored pointer.
#[derive(Clone)]
pub(crate) struct OrigRec {
    pub(crate) owner: String,
    #[allow(dead_code)]
    pub(crate) kind: Option<CompKind>,
    #[allow(dead_code)]
    pub(crate) tr: usize,
    #[allow(dead_code)]
    pub(crate) comp: Option<usize>,
    pub(crate) pos: [f32; 3],
    pub(crate) euler: [f32; 3],
    pub(crate) scale: [f32; 3],
    pub(crate) color: Option<[f32; 4]>,
    /// CanvasGroup address + alpha when present (restored separately;
    /// the color write covers the no-CanvasGroup case).
    #[allow(dead_code)]
    pub(crate) cg: Option<usize>,
    pub(crate) alpha: Option<f32>,
    pub(crate) font_size: Option<f32>,
    pub(crate) active: bool,
}

/// One queued one-shot write (render stages, tick drains).
/// `owner` re-resolves the live target at drain time; `addr` is only
/// a log hint kept for diagnostics.
pub(crate) struct WriteReq {
    pub(crate) owner: String,
    pub(crate) kind: Option<CompKind>,
    #[allow(dead_code)]
    pub(crate) addr: usize,
    pub(crate) op: u8,
    pub(crate) v: [f32; 4],
}

/// Render-side mirror of [`ObjSnap`] with editable buffers. Lives only
/// on the present thread (`&mut` in `on_draw_ui`); rebuilt whenever
/// the snapshot version bumps. Edits mutate these buffers only —
/// nothing reaches Unity until the staged batch drains — the snapshot
/// itself is never written by the UI.
pub(crate) struct UiObj {
    pub(crate) owner: String,
    pub(crate) comp: Option<CompKind>,
    pub(crate) active: bool,
    /// Graphic component address (diagnostic hint only — drains always
    /// re-resolve; never dereferenced).
    #[allow(dead_code)]
    pub(crate) comp_addr: Option<usize>,
    pub(crate) pos: [f32; 3],
    pub(crate) euler: [f32; 3],
    pub(crate) scale: [f32; 3],
    pub(crate) color: egui::Color32,
    /// Opacity 0..1: CanvasGroup alpha when present, else color alpha
    /// (merged into the color write on drain) — hidden for pure
    /// containers without either.
    pub(crate) opacity: f32,
    pub(crate) cg: Option<usize>,
    pub(crate) size: Option<f32>,
    pub(crate) text: String,
    /// Scale-zero hide switch (drain writes zero / buffer scale).
    /// Initialized from the snapshot (a zero scale reads as hidden).
    pub(crate) visible: bool,
}

/// Short display name for a path: `Section/Leaf` (disambiguates the
/// three `ImgBg`s) or the bare name when there is no section.
pub(crate) fn display_name(path: &str) -> String {
    let mut parts = path.split('/');
    let leaf = parts.next_back().unwrap_or(path);
    match parts.next_back() {
        Some(section) => format!("{section}/{leaf}"),
        None => leaf.to_string(),
    }
}

/// Linear `[r, g, b, a]` (0..1, Unity `Color`) → egui color.
/// Round-trips through linear `Rgba`: `Color32` itself is gamma-space
/// premultiplied, so naive u8 scaling loses precision for a < 1.
pub(crate) fn f32_to_color(c: [f32; 4]) -> egui::Color32 {
    egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]).into()
}

/// egui color → linear `[r, g, b, a]` (0..1).
pub(crate) fn color_to_f32(c: egui::Color32) -> [f32; 4] {
    egui::Rgba::from(c).to_rgba_unmultiplied()
}

/// Scale epsilon: below this a scale counts as hidden (zero).
const SCALE_EPS: f32 = 1e-6;

/// True when every component is near zero (scale-hidden).
pub(crate) fn is_zero_scale(s: [f32; 3]) -> bool {
    s[0].abs() < SCALE_EPS && s[1].abs() < SCALE_EPS && s[2].abs() < SCALE_EPS
}

/// Wanted values for one object, as saved/loaded. Every field is
/// optional: `None` = untouched (game default / live value stays).
#[derive(Clone, Default)]
pub(crate) struct SavedObj {
    pub(crate) pos: Option<[f32; 3]>,
    pub(crate) euler: Option<[f32; 3]>,
    pub(crate) scale: Option<[f32; 3]>,
    pub(crate) color: Option<[f32; 4]>,
    pub(crate) opacity: Option<f32>,
    pub(crate) active: Option<bool>,
    pub(crate) visible: Option<bool>,
    pub(crate) size: Option<f32>,
}

fn same3(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| x == y)
}

/// Diff render buffers against first-seen originals: only MODIFIED
/// params become `Some` (bit-exact compare — untouched buffers are
/// bit-identical to the snapshot reads). `None` original (no
/// baseline, e.g. right after a restore consumed the records) saves
/// the full live buffer instead. Returns `None` when nothing moved.
pub(crate) fn diff_object(obj: &UiObj, orig: Option<&OrigRec>) -> Option<SavedObj> {
    let mut s = SavedObj::default();
    let mut any = false;
    match orig {
        Some(r) => {
            if !same3(obj.pos, r.pos) {
                s.pos = Some(obj.pos);
                any = true;
            }
            if !same3(obj.euler, r.euler) {
                s.euler = Some(obj.euler);
                any = true;
            }
            if !same3(obj.scale, r.scale) {
                s.scale = Some(obj.scale);
                any = true;
            }
            if let Some(c) = r.color {
                let cur = color_to_f32(obj.color);
                if cur != c {
                    s.color = Some(cur);
                    any = true;
                }
            }
            // Baseline: group alpha when present, else the color alpha.
            // No baseline (container without either) but moved off the
            // 1.0 default: still worth saving — drain climbs ancestors.
            let orig_opacity = r.alpha.or_else(|| r.color.map(|c| c[3]));
            match orig_opacity {
                Some(o) if (obj.opacity - o).abs() > f32::EPSILON => {
                    s.opacity = Some(obj.opacity);
                    any = true;
                }
                None if obj.opacity != 1.0 => {
                    s.opacity = Some(obj.opacity);
                    any = true;
                }
                _ => {}
            }
            if obj.active != r.active {
                s.active = Some(obj.active);
                any = true;
            }
            let vis_orig = !is_zero_scale(r.scale);
            if obj.visible != vis_orig {
                s.visible = Some(obj.visible);
                any = true;
            }
            if obj.size != r.font_size {
                s.size = obj.size;
                if obj.size.is_some() {
                    any = true;
                }
            }
        }
        None => {
            s.pos = Some(obj.pos);
            s.euler = Some(obj.euler);
            s.scale = Some(obj.scale);
            s.color = Some(color_to_f32(obj.color));
            s.opacity = Some(obj.opacity);
            s.active = Some(obj.active);
            s.visible = Some(obj.visible);
            s.size = obj.size;
            any = true;
        }
    }
    if any { Some(s) } else { None }
}

/// Build drain-ready write requests from wanted values. `live_color`
/// supplies the RGB base when opacity must merge into color alpha but
/// no saved color exists (load path reads it live; UI path passes its
/// buffer color). Skips what cannot be expressed (opacity without
/// color base nor group, font without TMP).
pub(crate) fn write_reqs_for(
    owner: &str,
    comp: Option<CompKind>,
    has_cg: bool,
    s: &SavedObj,
    live_color: Option<[f32; 4]>,
) -> Vec<WriteReq> {
    let mk = |op: u8, v: [f32; 4]| WriteReq {
        owner: owner.to_string(),
        kind: comp,
        addr: 0,
        op,
        v,
    };
    let mut reqs = Vec::new();
    // Effective scale: visible-off wins over any recorded scale.
    match (s.visible, s.scale) {
        (Some(false), _) => reqs.push(mk(OP_SCALE, [0.0, 0.0, 0.0, 0.0])),
        (_, Some(sc)) => reqs.push(mk(OP_SCALE, [sc[0], sc[1], sc[2], 0.0])),
        _ => {}
    }
    if let Some(p) = s.pos {
        reqs.push(mk(OP_POS, [p[0], p[1], p[2], 0.0]));
    }
    if let Some(e) = s.euler {
        reqs.push(mk(OP_ROT, [e[0], e[1], e[2], 0.0]));
    }
    if let Some(a) = s.active {
        reqs.push(mk(OP_ACTIVE, [if a { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]));
    }
    if comp.is_some() {
        let need_color =
            s.color.is_some() || (s.opacity.is_some() && !has_cg && live_color.is_some());
        if need_color {
            let mut col = s.color.or(live_color).unwrap_or([1.0, 1.0, 1.0, 1.0]);
            if !has_cg {
                col[3] = s.opacity.unwrap_or(col[3]).clamp(0.0, 1.0);
            }
            reqs.push(mk(OP_COLOR, col));
        }
        if has_cg {
            if let Some(o) = s.opacity {
                reqs.push(mk(OP_CGALPHA, [o, 0.0, 0.0, 0.0]));
            }
        }
        if comp == Some(CompKind::Tmp) {
            if let Some(size) = s.size {
                reqs.push(mk(OP_FONT, [size, 0.0, 0.0, 0.0]));
            }
        }
    } else if let Some(o) = s.opacity {
        // Container without own group: drain climbs to the nearest
        // ancestor group (or skips with a clear log line).
        reqs.push(mk(OP_CGALPHA, [o, 0.0, 0.0, 0.0]));
    }
    reqs
}

/// Ancestor-CanvasGroup climb budget for a dashboard path: climbing
/// must never escape above the walked root (`Map/Mask/Base/RoadImage`
/// = 4 segments → 3 climbs max, landing on `Map` at most).
pub(crate) fn ancestor_climb_limit(owner: &str) -> usize {
    owner.split('/').count().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{
        ancestor_climb_limit, color_to_f32, diff_object, display_name, f32_to_color,
        write_reqs_for, CompKind, OrigRec, UiObj, OP_ACTIVE, OP_CGALPHA, OP_COLOR, OP_POS,
        OP_SCALE,
    };

    #[test]
    fn climb_limit_never_escapes_root() {
        assert_eq!(ancestor_climb_limit("Map/Mask/Base/RoadImage"), 3);
        assert_eq!(ancestor_climb_limit("Map"), 0);
        assert_eq!(ancestor_climb_limit("ImgPointer"), 0);
    }

    #[test]
    fn display_name_shortens_paths() {
        assert_eq!(
            display_name("Speedometer/Background/ImgBg"),
            "Background/ImgBg"
        );
        assert_eq!(display_name("ImgPointer"), "ImgPointer");
    }

    #[test]
    fn color_roundtrip() {
        for c in [
            [1.0, 1.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
            [0.2, 0.4, 0.6, 0.8],
            [0.0, 0.0, 0.0, 0.0],
        ] {
            let back = color_to_f32(f32_to_color(c));
            for i in 0..4 {
                // Gamma u8 quantization between the two linear ends.
                assert!((back[i] - c[i]).abs() < 0.012, "{c:?} -> {back:?}");
            }
        }
    }

    fn test_obj() -> (UiObj, OrigRec) {
        let ui = UiObj {
            owner: "Map/X".to_string(),
            comp: Some(CompKind::Image),
            active: true,
            comp_addr: Some(0),
            pos: [1.0, 2.0, 3.0],
            euler: [0.0; 3],
            scale: [1.0; 3],
            color: egui::Color32::WHITE,
            opacity: 1.0,
            cg: None,
            size: None,
            text: String::new(),
            visible: true,
        };
        let rec = OrigRec {
            owner: "Map/X".to_string(),
            kind: Some(CompKind::Image),
            tr: 0,
            comp: Some(0),
            pos: [1.0, 2.0, 3.0],
            euler: [0.0; 3],
            scale: [1.0; 3],
            color: Some([1.0, 1.0, 1.0, 1.0]),
            cg: None,
            alpha: None,
            font_size: None,
            active: true,
        };
        (ui, rec)
    }

    #[test]
    fn diff_empty_when_untouched() {
        let (ui, rec) = test_obj();
        assert!(diff_object(&ui, Some(&rec)).is_none());
    }

    #[test]
    fn diff_captures_only_moved() {
        let (mut ui, rec) = test_obj();
        ui.pos[0] = 9.0;
        ui.active = false;
        let s = diff_object(&ui, Some(&rec)).expect("diff");
        assert_eq!(s.pos, Some([9.0, 2.0, 3.0]));
        assert_eq!(s.active, Some(false));
        assert!(s.euler.is_none() && s.scale.is_none() && s.color.is_none());
    }

    #[test]
    fn write_reqs_cover_all_ops() {
        let (mut ui, _) = test_obj();
        ui.active = false;
        ui.visible = false;
        let live = Some([1.0, 1.0, 1.0, 1.0]);
        let reqs = write_reqs_for("Map/X", Some(CompKind::Image), false, &diff_object(&ui, None).unwrap(), live);
        let ops: Vec<u8> = reqs.iter().map(|r| r.op).collect();
        // scale-zero (visible off) + pos + euler + active + merged color
        assert!(ops.contains(&OP_SCALE));
        assert!(ops.contains(&OP_POS));
        assert!(ops.contains(&OP_ACTIVE));
        assert!(ops.contains(&OP_COLOR));
        assert!(!ops.contains(&OP_CGALPHA));
    }

    #[test]
    fn write_reqs_prefer_group() {
        let s = super::SavedObj {
            opacity: Some(0.5),
            ..super::SavedObj::default()
        };
        // With a group: single CGALPHA, no color merge.
        let reqs = write_reqs_for("Map/X", Some(CompKind::Image), true, &s, None);
        let ops: Vec<u8> = reqs.iter().map(|r| r.op).collect();
        assert_eq!(ops, vec![OP_CGALPHA]);
        // Without group nor live base: opacity alone cannot land.
        let reqs = write_reqs_for("Map/X", Some(CompKind::Image), false, &s, None);
        assert!(reqs.is_empty());
    }

    #[test]
    fn raw_label() {
        assert_eq!(CompKind::Raw.label(), "RawImage");
    }

    #[test]
    fn container_opacity_emits_group_write() {
        // Pure container (no comp): opacity stages a CGALPHA the
        // drain resolves by ancestor climb.
        let s = super::SavedObj {
            opacity: Some(0.3),
            ..super::SavedObj::default()
        };
        let reqs = write_reqs_for("Map/Mask", None, false, &s, None);
        let ops: Vec<u8> = reqs.iter().map(|r| r.op).collect();
        assert_eq!(ops, vec![OP_CGALPHA]);
    }

    #[test]
    fn diff_saves_moved_opacity_without_baseline() {
        let (mut ui, _) = test_obj();
        ui.comp = None;
        ui.opacity = 0.4;
        let s = diff_object(&ui, None).expect("diff");
        assert_eq!(s.opacity, Some(0.4));
        // No baseline: full live buffer saved (opacity included even
        // at its 1.0 default — nothing to compare against).
        let (ui2, _) = test_obj();
        let mut ui2 = ui2;
        ui2.comp = None;
        let s = diff_object(&ui2, None).expect("diff");
        assert_eq!(s.opacity, Some(1.0));
    }
}
