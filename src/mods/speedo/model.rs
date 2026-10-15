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
    Tmp,
}

impl CompKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            CompKind::Image => "Image",
            CompKind::Tmp => "TextMeshProUGUI",
        }
    }

    /// Cached IL2CPP class for `GetComponent` (null = unavailable).
    pub(crate) fn klass(self, cache: &UnityCache) -> *mut std::ffi::c_void {
        match self {
            CompKind::Image => cache.image_klass,
            CompKind::Tmp => cache.tmp_klass,
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
    pub(crate) tr_addr: usize,
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
    pub(crate) tr_addr: usize,
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

#[cfg(test)]
mod tests {
    use super::{color_to_f32, display_name, f32_to_color};

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
}
