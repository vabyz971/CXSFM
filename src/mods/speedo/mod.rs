//! Speedometer objects, Unity-Inspector style.
//!
//! No polling, no hooks, no per-tick Unity traffic (that crashed the
//! game after ~1 min — isolation test: 5 min clean with the mod off).
//! Instead the window lists the GameObjects composing the game's
//! speedometer the way Unity's Inspector shows components:
//! `GameObject` (name + active), `Transform` (position / rotation /
//! scale), `Image` (color) or `TextMeshProUGUI` (text preview, color,
//! font size). Editing a field queues a one-shot write, applied once
//! on the tick thread; the game then owns the values again.
//!
//! Rules (proven in game, do not improvise):
//! - Unity calls only (a) on arming, (b) on Refresh / Restore clicks,
//!   (c) to drain an explicit write queue. NEVER per tick, NEVER an
//!   enumeration: targets bind via `GameObject.Find` + `GetComponent`
//!   (direct lookups, no scene walk — repeated enumeration from the
//!   tick crashed another mod).
//! - Every handle alive-checked (`m_CachedPtr`) before every invoke;
//!   intermediates too (a scene churn between two calls segfaults).
//! - Originals (scale, position, color, font size) recorded on first
//!   discovery per address, written back on disable / Restore.
//! - No Mutex held across a Unity call; the present thread only
//!   swaps/clones small data and pushes write requests.
//! - All mutexes go through `common::lock` (poison-recovering): a
//!   quarantined panic must never wedge the overlay.

pub mod i18n;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Retry cadence (ticks, >= 5 s at 60 Hz) for a failed cache init
/// (game loading). Forced on arming / Refresh / Restore / disable.
/// Missing *targets* never auto-retry (no periodic binding by design —
/// that traffic crashed the game); the status line shows n/m and the
/// user presses Refresh.
const RESOLVE_EVERY_TICKS: u64 = 300;

/// Target groups (checkboxes; Gauge + Speed on by default). The gear
/// readout is only bound when its group is checked.
const GROUP_GAUGE: u8 = 0;
const GROUP_SPEED: u8 = 1;
const GROUP_GEAR: u8 = 2;
const GROUP_NITRO: u8 = 3;

/// Group checkbox state lookup (pure, testable).
fn group_enabled(id: u8, gauge: bool, speed: bool, gear: bool, nitro: bool) -> bool {
    match id {
        GROUP_GAUGE => gauge,
        GROUP_SPEED => speed,
        GROUP_GEAR => gear,
        GROUP_NITRO => nitro,
        _ => false,
    }
}

/// Write-request opcodes (plain `u8`, `Copy` across threads).
const OP_SCALE: u8 = 0;
const OP_POS: u8 = 1;
const OP_COLOR: u8 = 2;
const OP_FONT: u8 = 3;
const OP_ROT: u8 = 4;
const OP_CGALPHA: u8 = 5;

/// Component flavour of a wanted object.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CompKind {
    Image,
    Tmp,
}

impl CompKind {
    fn label(self) -> &'static str {
        match self {
            CompKind::Image => "Image",
            CompKind::Tmp => "TextMeshProUGUI",
        }
    }
}

/// One wanted speedometer object: bound by direct `GameObject.Find`
/// (bare name, then `Speedometer/`-prefixed path) + `GetComponent` of
/// its flavour — no enumeration. The "name contains speed" fallback
/// is gone: it required a scene walk, nothing else can do it.
#[derive(Clone, Copy)]
struct Want {
    group: u8,
    owner: &'static str,
    comp: CompKind,
}

/// The GameObjects composing the stock speedometer. Names confirmed
/// against the shipped bundle (`uihud_assets_hudspeedometer`:
/// `HUDCarDashboard` prefab, `SpeedometerAtlas`, `ImgPointer` needle,
/// `speedometer_bg`, `nitro_bar_new`, TMP texts, Orbitron/Digital
/// fonts, `UITach`/`AngleRpm`/`TurboValue` driver scripts — the game
/// keeps owning values, we only restyle).
const WANTS: &[Want] = &[
    Want { group: GROUP_GAUGE, owner: "Background", comp: CompKind::Image },
    Want { group: GROUP_GAUGE, owner: "speedometer_bg", comp: CompKind::Image },
    Want { group: GROUP_GAUGE, owner: "Tachometer", comp: CompKind::Image },
    Want { group: GROUP_GAUGE, owner: "Arrow", comp: CompKind::Image },
    Want { group: GROUP_GAUGE, owner: "ImgPointer", comp: CompKind::Image },
    Want { group: GROUP_SPEED, owner: "Text (TMP) Speed", comp: CompKind::Tmp },
    Want { group: GROUP_GEAR, owner: "Text (TMP) Gear", comp: CompKind::Tmp },
    Want { group: GROUP_NITRO, owner: "Nitro", comp: CompKind::Image },
    Want { group: GROUP_NITRO, owner: "nitro_bar_new", comp: CompKind::Image },
    Want { group: GROUP_NITRO, owner: "TextN2O", comp: CompKind::Tmp },
];

/// `GameObject.Find` candidates for an owner: bare name first (root
/// object), then the known `Speedometer/` parent path (HUD children).
/// `Find` only sees active objects; a missing target simply stays out
/// of this discovery (status n/m, manual Refresh — never auto-retry).
fn find_candidates(owner: &str) -> [String; 2] {
    [owner.to_string(), format!("Speedometer/{owner}")]
}

/// Linear `[r, g, b, a]` (0..1, Unity `Color`) → egui color.
/// Round-trips through linear `Rgba`: `Color32` itself is gamma-space
/// premultiplied, so naive u8 scaling loses precision for a < 1.
fn f32_to_color(c: [f32; 4]) -> egui::Color32 {
    egui::Rgba::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]).into()
}

/// egui color → linear `[r, g, b, a]` (0..1).
fn color_to_f32(c: egui::Color32) -> [f32; 4] {
    egui::Rgba::from(c).to_rgba_unmultiplied()
}

/// Overlay drag (egui px, y down-positive) → world offset, using the
/// per-object calibration (`ex`, `ey` = screen px per world unit
/// measured at discovery with the same camera). A zero factor means a
/// locked axis (drag along it does nothing — the marker snaps back).
/// Pure and unit-tested; the signs fall out of the calibration, no
/// canvas knowledge required.
fn screen_drag_to_world(drag: [f32; 2], ex: f32, ey: f32) -> [f32; 3] {
    let wx = if ex.abs() > 1e-6 { drag[0] / ex } else { 0.0 };
    let wy = if ey.abs() > 1e-6 { -drag[1] / ey } else { 0.0 };
    [wx, wy, 0.0]
}

/// Published snapshot of one object (tick writes on discovery, render
/// clones; swapped under a brief lock, never held across Unity calls).
/// `pos`/`euler` are LOCAL (what persists on UI objects — the Canvas
/// layout discards world-space writes); the marker projection uses a
/// separate world read.
#[derive(Clone)]
struct ObjSnap {
    owner: String,
    comp: CompKind,
    active: bool,
    tr_addr: usize,
    comp_addr: usize,
    pos: [f32; 3],
    euler: [f32; 3],
    scale: [f32; 3],
    color: [f32; 4],
    opacity: f32,
    /// CanvasGroup address when present (opacity path), else `None`
    /// (color-alpha fallback).
    cg: Option<usize>,
    font_size: Option<f32>,
    text: String,
    /// Raw `WorldToScreenPoint` anchor (px, y bottom-up, z = depth;
    /// `None` = behind camera or unprojectable → no marker).
    screen: Option<[f32; 3]>,
    /// Calibration: screen px per world unit on X / Y, measured with
    /// the same camera (0 = locked axis).
    ex: f32,
    ey: f32,
}

/// First-recorded originals per address pair (restore source).
/// `owner`/`comp` are the re-resolve keys: raw addresses go stale on
/// scene change (menu → city rebuilds the HUD), so restore re-finds by
/// name and writes to the *live* object, never the stored pointer.
struct OrigRec {
    owner: String,
    kind: CompKind,
    #[allow(dead_code)]
    tr: usize,
    #[allow(dead_code)]
    comp: usize,
    pos: [f32; 3],
    euler: [f32; 3],
    scale: [f32; 3],
    color: [f32; 4],
    /// CanvasGroup address + alpha when present (restored separately;
    /// the color write covers the no-CanvasGroup case).
    #[allow(dead_code)]
    cg: Option<usize>,
    alpha: Option<f32>,
    font_size: Option<f32>,
}

/// One queued one-shot write (render pushes, tick drains).
/// `owner`/`kind` re-resolve the live target at drain time; `addr` is
/// only a hint kept for logging.
struct WriteReq {
    owner: String,
    kind: CompKind,
    #[allow(dead_code)]
    addr: usize,
    op: u8,
    v: [f32; 4],
}

/// Render-side mirror of [`ObjSnap`] with editable buffers. Lives only
/// on the present thread (`&mut` in `on_draw_ui`); rebuilt whenever
/// the snapshot version bumps. Edits mutate these buffers only —
/// nothing reaches Unity until the Appliquer batch — the snapshot
/// itself is never written by the UI.
struct UiObj {
    owner: String,
    comp: CompKind,
    active: bool,
    tr_addr: usize,
    comp_addr: usize,
    pos: [f32; 3],
    euler: [f32; 3],
    scale: [f32; 3],
    color: egui::Color32,
    /// Opacity 0..1: CanvasGroup alpha when present, else color alpha
    /// (merged into the color write on Apply).
    opacity: f32,
    cg: Option<usize>,
    size: Option<f32>,
    text: String,
    screen: Option<[f32; 3]>,
    ex: f32,
    ey: f32,
    /// Scale-zero hide switch (Appliquer writes zero / buffer scale).
    /// Initialized from the snapshot (a zero scale reads as hidden).
    visible: bool,
    /// Discovery-time position (marker anchor math).
    pos0: [f32; 3],
    /// Last marker window position (screen px, egui coords); marker
    /// drag deltas convert to world offsets into `pos` above.
    last_rect: Option<egui::Pos2>,
}

/// Scale epsilon: below this a scale counts as hidden (zero).
const SCALE_EPS: f32 = 1e-6;

/// True when every component is near zero (scale-hidden).
fn is_zero_scale(s: [f32; 3]) -> bool {
    s[0].abs() < SCALE_EPS && s[1].abs() < SCALE_EPS && s[2].abs() < SCALE_EPS
}

pub struct SpeedoMod {
    enabled: AtomicBool,
    unity: Mutex<Option<UnityCache>>,
    tick: AtomicU64,
    /// Discover on next tick (arming, Refresh button).
    pending: AtomicBool,
    /// Restore originals on next tick (Restore button, no disable).
    restore_now: AtomicBool,
    /// Last tick a (re-)init was attempted; 0 = never.
    last_init: AtomicU64,
    /// Group checkboxes (UI writes, tick reads; gear binds only when on).
    gauge: AtomicBool,
    speed: AtomicBool,
    gear: AtomicBool,
    nitro: AtomicBool,
    /// Published snapshots + version (render clones).
    snaps: Mutex<Vec<ObjSnap>>,
    snap_ver: AtomicU64,
    /// First-recorded originals (tick only).
    origs: Mutex<Vec<OrigRec>>,
    /// Staged Appliquer batch (render swaps in, tick drains once —
    /// every edit lands in a SINGLE Unity pass, one log line).
    pending_batch: Mutex<Option<Vec<WriteReq>>>,
    /// Drain the staged batch on next tick (Appliquer button).
    apply_now: AtomicBool,
    /// Discovery status for the status line.
    status_ok: AtomicU64,
    status_total: AtomicU64,
    /// Render-side mirror (present thread only).
    ui: Vec<UiObj>,
    ui_ver: u64,
    /// Pin + opacity chrome (shared tool pattern).
    tool: ToolChrome,
}

// SAFETY: all shared state is atomics or mutex-guarded plain data;
// Unity handles are `usize`, never dereferenced.
unsafe impl Send for SpeedoMod {}
unsafe impl Sync for SpeedoMod {}

impl SpeedoMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            tick: AtomicU64::new(0),
            pending: AtomicBool::new(false),
            restore_now: AtomicBool::new(false),
            last_init: AtomicU64::new(0),
            gauge: AtomicBool::new(true),
            speed: AtomicBool::new(true),
            gear: AtomicBool::new(false),
            nitro: AtomicBool::new(false),
            snaps: Mutex::new(Vec::new()),
            snap_ver: AtomicU64::new(0),
            origs: Mutex::new(Vec::new()),
            pending_batch: Mutex::new(None),
            apply_now: AtomicBool::new(false),
            status_ok: AtomicU64::new(0),
            status_total: AtomicU64::new(0),
            ui: Vec::new(),
            ui_ver: 0,
            tool: ToolChrome::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Group checkbox read for a target (tick side).
    fn group_on(&self, id: u8) -> bool {
        group_enabled(
            id,
            self.gauge.load(Ordering::SeqCst),
            self.speed.load(Ordering::SeqCst),
            self.gear.load(Ordering::SeqCst),
            self.nitro.load(Ordering::SeqCst),
        )
    }

    /// Resolve the method/field cache once, then serve from memory.
    /// The hot path (cache present) performs zero Unity calls. A
    /// missing cache attempts resolution at most every
    /// `RESOLVE_EVERY_TICKS` unless `force`.
    fn ensure_unity(&self, tick: u64, force: bool) -> bool {
        if lock(&self.unity).is_some() {
            return true;
        }
        if !force {
            let last = self.last_init.load(Ordering::SeqCst);
            if last != 0 && tick.saturating_sub(last) < RESOLVE_EVERY_TICKS {
                return false;
            }
        }
        self.last_init.store(tick.saturating_add(1), Ordering::SeqCst);
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
        *lock(&self.unity) = Some(cache);
        true
    }

    /// One-shot discovery (tick thread, explicit trigger only):
    /// each checked target binds via `GameObject.Find` + `GetComponent`
    /// — direct lookups, zero enumeration — then one snapshot read per
    /// match, first-seen originals recorded, snapshot published. No
    /// periodic work — the game owns the values afterwards. A missing
    /// target stays missing until the next manual Refresh (status n/m).
    fn discover(&self) {
        let tick = self.tick.load(Ordering::SeqCst);
        if !self.ensure_unity(tick, true) {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = lock(&self.unity).as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        // Checked wants only (gear binds solely when its group is on).
        let wants: Vec<Want> = WANTS
            .iter()
            .copied()
            .filter(|w| self.group_on(w.group))
            .collect();
        let t0 = std::time::Instant::now();
        // Bind: Find → alive → GetComponent → alive. One `Find` chain
        // per candidate, no walk; a dead intermediate unbinds silently.
        // SAFETY: attached tick thread; every handle alive-checked
        // before any further invoke, consumed immediately.
        let found: Vec<(Want, usize, usize)> = unsafe {
            let mut out = Vec::new();
            for want in wants.iter().copied() {
                let klass = match want.comp {
                    CompKind::Image => cache.image_klass,
                    CompKind::Tmp => cache.tmp_klass,
                };
                if klass.is_null() {
                    continue;
                }
                for name in find_candidates(want.owner) {
                    let go = match crate::unity::go_find(api, &cache, &name) {
                        Some(g) => g,
                        None => continue,
                    };
                    if go.is_null() {
                        continue;
                    }
                    match crate::unity::object_alive(api, &cache, go) {
                        Some(true) => {}
                        _ => continue,
                    }
                    let comp = match crate::unity::go_get_component(api, &cache, go, klass) {
                        Some(c) => c,
                        None => continue,
                    };
                    if comp.is_null() {
                        continue;
                    }
                    match crate::unity::object_alive(api, &cache, comp) {
                        Some(true) => {}
                        _ => continue,
                    }
                    out.push((want, comp as usize, go as usize));
                    break;
                }
            }
            out
        };
        // Projection camera (one lookup): overlay markers + the
        // px/world calibration below run on the same camera, so signs
        // and scale fall out measured — no canvas knowledge required.
        // SAFETY: same contract; alive-probed like any handle.
        let cam = unsafe {
            match crate::unity::main_camera(api, &cache) {
                Some(c) if !c.is_null() => {
                    match crate::unity::object_alive(api, &cache, c) {
                        Some(true) => Some(c),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        // Read one snapshot per match. Originals accumulate locally
        // and merge under a brief lock AFTER the Unity calls — no
        // Mutex is ever held across an invoke (a slow call would
        // stall the present thread into skipping this window).
        // SAFETY: same contract; transform derived from the live owner.
        let mut snaps = Vec::new();
        let mut fresh_origs = Vec::new();
        // Addresses already recorded (merge skips them). The guard
        // ends here (last use) — well before any Unity call below.
        let known = lock(&self.origs);
        let mut seen: Vec<(usize, usize)> = known.iter().map(|r| (r.tr, r.comp)).collect();
        drop(known);
            unsafe {
                for (want, comp, go_addr) in found {
                    let comp_p = comp as *mut std::ffi::c_void;
                    // Owner game object comes from the bind step above;
                    // re-probed here (a churn between bind and read
                    // frees it), then its transform.
                    let go = go_addr as *mut std::ffi::c_void;
                    match crate::unity::object_alive(api, &cache, go) {
                        Some(true) => {}
                        _ => continue,
                    }
                    let tr = match crate::unity::go_transform(api, &cache, go) {
                        Some(t) => t,
                        None => continue,
                    };
                if tr.is_null() {
                    continue;
                }
                match crate::unity::object_alive(api, &cache, tr) {
                    Some(true) => {}
                    _ => continue,
                }
                let active = crate::unity::go_active(api, &cache, go).unwrap_or(true);
                let scale = crate::unity::tr_get_scale(api, &cache, tr).unwrap_or([1.0, 1.0, 1.0]);
                // LOCAL transforms: what the Canvas keeps (world-space
                // writes get discarded on every layout rebuild).
                let pos = crate::unity::tr_get_local_position(api, &cache, tr)
                    .unwrap_or([0.0, 0.0, 0.0]);
                let euler = crate::unity::tr_get_local_euler(api, &cache, tr)
                    .unwrap_or([0.0, 0.0, 0.0]);
                // World position: projection + calibration only, never
                // displayed or written.
                let world = crate::unity::tr_position(api, &cache, tr).unwrap_or(pos);
                let color = crate::unity::graphic_get_color(api, &cache, comp_p)
                    .unwrap_or([1.0, 1.0, 1.0, 1.0]);
                // Opacity: CanvasGroup alpha when present (whole
                // subtree, miniHUD proven), else the color alpha.
                // `go_get_component` already degrades on a null klass.
                let cg_raw = crate::unity::go_get_component(api, &cache, go, cache.canvasgroup_klass);
                let cg_live = match cg_raw {
                    Some(c) if !c.is_null() => {
                        match crate::unity::object_alive(api, &cache, c) {
                            Some(true) => Some(c),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let (cg, opacity) = match cg_live {
                    Some(c) => {
                        let a = crate::unity::canvasgroup_get_alpha(api, &cache, c)
                            .unwrap_or(color[3]);
                        (Some(c as usize), a)
                    }
                    None => (None, color[3]),
                };
                let (font_size, text) = if want.comp == CompKind::Tmp {
                    let size = crate::unity::tmp_get_font_size(api, &cache, comp_p);
                    let txt = crate::unity::tmp_get_text(api, &cache, comp_p).unwrap_or_default();
                    (size, txt)
                } else {
                    (None, String::new())
                };
                // Marker projection + calibration (same camera): anchor
                // S0 plus one world unit along X and Y, from the WORLD
                // position (projection space — never written).
                // `screen` stays `None` behind the camera or without
                // projection API.
                let (screen, ex, ey) = match cam {
                    Some(c) => {
                        let s0 = crate::unity::world_to_screen(api, &cache, c, world);
                        let sx = match s0 {
                            Some(_) => crate::unity::world_to_screen(
                                api, &cache, c, [world[0] + 1.0, world[1], world[2]],
                            ),
                            None => None,
                        };
                        let sy = match s0 {
                            Some(_) => crate::unity::world_to_screen(
                                api, &cache, c, [world[0], world[1] + 1.0, world[2]],
                            ),
                            None => None,
                        };
                        match (s0, sx, sy) {
                            (Some(a), Some(b), Some(d)) if a[2] > 0.0 => {
                                (Some(a), b[0] - a[0], d[1] - a[1])
                            }
                            _ => (None, 0.0, 0.0),
                        }
                    }
                    None => (None, 0.0, 0.0),
                };
                let tr_addr = tr as usize;
                if !seen.iter().any(|&(t, c)| t == tr_addr && c == comp) {
                    seen.push((tr_addr, comp));
                    fresh_origs.push(OrigRec {
                        owner: want.owner.to_string(),
                        kind: want.comp,
                        tr: tr_addr,
                        comp,
                        pos,
                        euler,
                        scale,
                        color,
                        cg,
                        alpha: cg.map(|_| opacity),
                        font_size,
                    });
                }
                if screen.is_some() {
                    let s = screen.unwrap();
                    crate::log_line(&format!(
                        "speedo: '{}' projeté à ({:.0}, {:.0})",
                        want.owner, s[0], s[1]
                    ));
                }
                snaps.push(ObjSnap {
                    owner: want.owner.to_string(),
                    comp: want.comp,
                    active,
                    tr_addr,
                    comp_addr: comp,
                    pos,
                    euler,
                    scale,
                    color,
                    opacity,
                    cg,
                    font_size,
                    text,
                    screen,
                    ex,
                    ey,
                });
            }
        }
        // Merge first-seen originals + swap the snapshot, both under
        // brief locks with zero Unity calls in between.
        lock(&self.origs).extend(fresh_origs);
        let n = snaps.len();
        let total = wants.len();
        *lock(&self.snaps) = snaps;
        self.snap_ver.fetch_add(1, Ordering::SeqCst);
        self.status_ok.store(n as u64, Ordering::SeqCst);
        self.status_total.store(total as u64, Ordering::SeqCst);
        let ms = t0.elapsed().as_millis();
        crate::log_line(&format!("speedo: objets {n}/{total} ({ms} ms)"));
    }

    /// Fresh re-resolve of a named HUD target (tick thread only).
    ///
    /// Stored `usize` handles go stale on scene change (menu → city
    /// rebuilds the HUD, GC frees old wrappers). Re-find by name and
    /// return the *live* `(transform, component, canvas_group?)`.
    /// `None` = target gone — caller skips, never touches stale memory.
    unsafe fn resolve_live(
        api: &crate::il2cpp::Il2cppApi,
        cache: &UnityCache,
        owner: &str,
        kind: CompKind,
    ) -> Option<(
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        Option<*mut std::ffi::c_void>,
    )> {
        // SAFETY: attached tick thread; every handle alive-checked
        // before return, consumed immediately by the caller.
        unsafe {
            let klass = match kind {
                CompKind::Image => cache.image_klass,
                CompKind::Tmp => cache.tmp_klass,
            };
            if klass.is_null() {
                return None;
            }
            for name in find_candidates(owner) {
                let go = crate::unity::go_find(api, cache, &name)?;
                if go.is_null() || crate::unity::object_alive(api, cache, go) != Some(true) {
                    continue;
                }
                let comp = crate::unity::go_get_component(api, cache, go, klass)?;
                if comp.is_null() || crate::unity::object_alive(api, cache, comp) != Some(true) {
                    continue;
                }
                let tr = crate::unity::go_transform(api, cache, go)?;
                if tr.is_null() || crate::unity::object_alive(api, cache, tr) != Some(true) {
                    continue;
                }
                let cg = match crate::unity::go_get_component(api, cache, go, cache.canvasgroup_klass) {
                    Some(c) if !c.is_null() && crate::unity::object_alive(api, cache, c) == Some(true) => Some(c),
                    _ => None,
                };
                return Some((tr, comp, cg));
            }
            None
        }
    }

    /// Drain the staged Appliquer batch (tick thread): every staged
    /// edit lands in ONE Unity pass, then a fresh discovery
    /// re-projects the markers (same explicit trigger, still no
    /// polling). Each address alive-checked right before its invoke;
    /// two log lines per Apply at most.
    fn drain_batch(&self) {
        let reqs = lock(&self.pending_batch).take();
        let reqs = match reqs {
            Some(r) if !r.is_empty() => r,
            _ => return,
        };
        let tick = self.tick.load(Ordering::SeqCst);
        if !self.ensure_unity(tick, false) {
            *lock(&self.pending_batch) = Some(reqs);
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = lock(&self.unity).as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        let total = reqs.len();
        let mut ok = 0usize;
        let mut skipped_stale = 0usize;
        // SAFETY: attached tick thread. Stale addresses are NEVER
        // dereferenced: each request re-resolves its named target first
        // (menu → city rebuilds HUD objects, GC frees old wrappers —
        // touching the stored pointer is the delayed SIGSEGV). The
        // stored `addr` is only a log hint.
        unsafe {
            // Batch re-resolve per owner (one Find per object, not per op).
            let mut live: std::collections::HashMap<String, (usize, usize, Option<usize>)> =
                std::collections::HashMap::new();
            for r in &reqs {
                let entry = match live.get(&r.owner) {
                    Some(e) => *e,
                    None => match Self::resolve_live(api, &cache, &r.owner, r.kind) {
                        Some((tr, comp, cg)) => {
                            let e = (tr as usize, comp as usize, cg.map(|p| p as usize));
                            live.insert(r.owner.clone(), e);
                            e
                        }
                        None => {
                            skipped_stale += 1;
                            continue;
                        }
                    },
                };
                let obj = match r.op {
                    OP_SCALE | OP_POS | OP_ROT => entry.0 as *mut std::ffi::c_void,
                    OP_COLOR | OP_FONT => entry.1 as *mut std::ffi::c_void,
                    OP_CGALPHA => match entry.2 {
                        Some(a) => a as *mut std::ffi::c_void,
                        None => continue,
                    },
                    _ => continue,
                };
                // Fresh handle, still probe (churn between resolve+write).
                match crate::unity::object_alive(api, &cache, obj) {
                    Some(true) => {}
                    _ => {
                        skipped_stale += 1;
                        continue;
                    }
                }
                let wrote = match r.op {
                    OP_SCALE => crate::unity::tr_set_scale(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_POS => crate::unity::tr_set_local_position(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_COLOR => crate::unity::graphic_set_color(api, &cache, obj, r.v),
                    OP_FONT => crate::unity::tmp_set_font_size(api, &cache, obj, r.v[0]),
                    OP_ROT => crate::unity::tr_set_local_euler(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_CGALPHA => crate::unity::canvasgroup_set_alpha(api, &cache, obj, r.v[0]),
                    _ => false,
                };
                if wrote {
                    ok += 1;
                }
            }
        }
        crate::log_line(&format!(
            "speedo: application {ok}/{total} en 1 passe (stale ignorés: {skipped_stale})"
        ));
        // Re-project markers on the just-written state (same pass).
        self.discover();
    }

    /// Write back first-recorded originals (disable / Restore paths,
    /// tick thread, one shot). Dead objects are skipped; the records
    /// are consumed so the next discovery starts fresh.
    fn restore_all(&self) {
        let tick = self.tick.load(Ordering::SeqCst);
        if !self.ensure_unity(tick, true) {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = lock(&self.unity).as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        // Take the records (brief lock, no Unity under it).
        let recs = std::mem::take(&mut *lock(&self.origs));
        if recs.is_empty() {
            return;
        }
        let mut ok = 0usize;
        let total = recs.len();
        let mut gone = 0usize;
        // SAFETY: attached tick thread. Same stale-handle rule as
        // `drain_batch`: re-resolve each record by owner name, write to
        // the live object. Never touch `r.tr / r.comp` directly.
        unsafe {
            for r in &recs {
                let (tr, comp, cg_live) = match Self::resolve_live(api, &cache, &r.owner, r.kind) {
                    Some(v) => v,
                    None => {
                        gone += 1;
                        continue;
                    }
                };
                let mut restored = false;
                restored |= crate::unity::tr_set_scale(api, &cache, tr, r.scale);
                restored |= crate::unity::tr_set_local_position(api, &cache, tr, r.pos);
                restored |= crate::unity::tr_set_local_euler(api, &cache, tr, r.euler);
                restored |= crate::unity::graphic_set_color(api, &cache, comp, r.color);
                if let Some(size) = r.font_size {
                    restored |= crate::unity::tmp_set_font_size(api, &cache, comp, size);
                }
                // CanvasGroup original on the live CG object (if any).
                if let (Some(cg), Some(a)) = (cg_live, r.alpha) {
                    restored |= crate::unity::canvasgroup_set_alpha(api, &cache, cg, a);
                }
                if restored {
                    ok += 1;
                }
            }
        }
        crate::log_line(&format!("speedo: restaurés {ok}/{total} (disparus: {gone})"));
        // Fresh UI next arming (scene may have changed meanwhile).
        *lock(&self.snaps) = Vec::new();
        self.snap_ver.fetch_add(1, Ordering::SeqCst);
        self.status_ok.store(0, Ordering::SeqCst);
        self.status_total.store(WANTS.len() as u64, Ordering::SeqCst);
    }

    /// Rebuild the render-side mirror when the snapshot version bumped
    /// (discovery / restore). Brief clone, lock released before draw.
    /// Returns true on resync (markers re-anchor this frame).
    fn sync_ui(&mut self) -> bool {
        let ver = self.snap_ver.load(Ordering::SeqCst);
        if ver == self.ui_ver {
            return false;
        }
        self.ui_ver = ver;
        let snaps = lock(&self.snaps).clone();
        self.ui = snaps
            .into_iter()
            .map(|s| UiObj {
                owner: s.owner,
                comp: s.comp,
                active: s.active,
                tr_addr: s.tr_addr,
                comp_addr: s.comp_addr,
                pos: s.pos,
                pos0: s.pos,
                euler: s.euler,
                scale: s.scale,
                color: f32_to_color(s.color),
                opacity: s.opacity,
                cg: s.cg,
                size: s.font_size,
                text: s.text,
                screen: s.screen,
                ex: s.ex,
                ey: s.ey,
                visible: !is_zero_scale(s.scale),
                last_rect: None,
            })
            .collect();
        true
    }

    /// One `Position / Rotation / Scale` trio row: three X/Y/Z values.
    /// Display-only when `editable` is false; otherwise `DragValue`s
    /// mutate the render-side buffer ONLY — nothing reaches Unity
    /// until the Appliquer batch. Returns true when a value changed
    /// (markers re-anchor on value-driven frames).
    fn xyz_row(ui: &mut egui::Ui, name: &str, vals: &mut [f32; 3], editable: bool) -> bool {
        let mut edited = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).weak().monospace());
            for (axis, v) in vals.iter_mut().enumerate() {
                let tag = ["X", "Y", "Z"][axis];
                ui.label(egui::RichText::new(tag).weak().small());
                if editable {
                    if ui
                        .add(egui::DragValue::new(v).speed(0.01).max_decimals(3))
                        .changed()
                    {
                        edited = true;
                    }
                } else {
                    ui.monospace(format!("{v:.2}"));
                }
            }
        });
        edited
    }

    /// Build the Appliquer batch from every render-side buffer: one
    /// `WriteReq` per aspect of every object, drained in a single
    /// Unity pass. Marker drag offsets already live in `pos`
    /// (converted at drag time). Opacity rides the CanvasGroup when
    /// present, else merges into the color alpha.
    fn build_batch(ui: &[UiObj]) -> Vec<WriteReq> {
        let mut reqs = Vec::new();
        for o in ui {
            // Hidden = scale-zero (object stays active: texts keep
            // updating, restore writes the recorded original back).
            let sc = if o.visible { o.scale } else { [0.0, 0.0, 0.0] };
            let mk = |addr: usize, op: u8, v: [f32; 4]| WriteReq {
                owner: o.owner.clone(),
                kind: o.comp,
                addr,
                op,
                v,
            };
            reqs.push(mk(o.tr_addr, OP_SCALE, [sc[0], sc[1], sc[2], 0.0]));
            reqs.push(mk(o.tr_addr, OP_POS, [o.pos[0], o.pos[1], o.pos[2], 0.0]));
            reqs.push(mk(o.tr_addr, OP_ROT, [o.euler[0], o.euler[1], o.euler[2], 0.0]));
            let mut col = color_to_f32(o.color);
            match o.cg {
                Some(cg) => {
                    reqs.push(mk(cg, OP_CGALPHA, [o.opacity, 0.0, 0.0, 0.0]));
                }
                None => {
                    col[3] = o.opacity.clamp(0.0, 1.0);
                }
            }
            reqs.push(mk(o.comp_addr, OP_COLOR, col));
            if let Some(size) = o.size {
                reqs.push(mk(o.comp_addr, OP_FONT, [size, 0.0, 0.0, 0.0]));
            }
        }
        reqs
    }
}

impl Default for SpeedoMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for SpeedoMod {
    fn name(&self) -> &'static str {
        "Speedometer"
    }

    fn mod_id(&self) -> &'static str {
        "speedo"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_speedo"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Gauge
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
        let _tick = self.tick.fetch_add(1, Ordering::SeqCst);
        // Explicit triggers only: discovery, restore, one batched
        // apply. An idle tick performs zero Unity calls (no polling —
        // that crashed the game), so the present thread never skips us.
        if self.pending.swap(false, Ordering::SeqCst) {
            self.discover();
        }
        if self.restore_now.swap(false, Ordering::SeqCst) {
            self.restore_all();
        }
        if self.apply_now.swap(false, Ordering::SeqCst) {
            self.drain_batch();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        let code = crate::i18n::current().code();
        let just_synced = self.sync_ui();
        // True when a panel edit moved values this frame: markers
        // re-anchor on value-driven frames, stay free-draggable else.
        let mut panel_edited = false;
        let win = egui::Window::new("Speedometer")
            .resizable(true)
            .frame(self.tool.frame(ctx))
            .show(ctx, |ui| {
                self.tool.enter(ui);
                ui.horizontal(|ui| {
                    self.tool.pin_toggle(ui);
                    if ui.button(self::i18n::refresh_label(code)).clicked() {
                        // Re-discover (scene changed, objects rebound).
                        self.pending.store(true, Ordering::SeqCst);
                    }
                    if ui
                        .button(self::i18n::apply_label(code))
                        .on_hover_text(self::i18n::apply_hint(code))
                        .clicked()
                    {
                        // Stage every buffer and apply in ONE Unity pass.
                        let batch = Self::build_batch(&self.ui);
                        *lock(&self.pending_batch) = Some(batch);
                        self.apply_now.store(true, Ordering::SeqCst);
                    }
                    if ui.button(self::i18n::restore_label(code)).clicked() {
                        // Write back originals without disabling.
                        self.restore_now.store(true, Ordering::SeqCst);
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.weak(format!(
                                "{} {}/{}",
                                self::i18n::objects_label(code),
                                self.status_ok.load(Ordering::SeqCst),
                                self.status_total.load(Ordering::SeqCst),
                            ));
                        },
                    );
                });
                // Discovery groups: unchecked targets are never bound
                // (the gear readout binds solely when Gear is on).
                // Takes effect on the next Refresh / arming.
                ui.horizontal(|ui| {
                    let mut gauge = self.gauge.load(Ordering::SeqCst);
                    if ui.checkbox(&mut gauge, "Gauge").changed() {
                        self.gauge.store(gauge, Ordering::SeqCst);
                    }
                    let mut speed = self.speed.load(Ordering::SeqCst);
                    if ui.checkbox(&mut speed, "Speed").changed() {
                        self.speed.store(speed, Ordering::SeqCst);
                    }
                    let mut gear = self.gear.load(Ordering::SeqCst);
                    if ui.checkbox(&mut gear, "Gear").changed() {
                        self.gear.store(gear, Ordering::SeqCst);
                    }
                    let mut nitro = self.nitro.load(Ordering::SeqCst);
                    if ui.checkbox(&mut nitro, "Nitro").changed() {
                        self.nitro.store(nitro, Ordering::SeqCst);
                    }
                });
                ui.separator();
                if self.ui.is_empty() {
                    ui.weak(self::i18n::hint_label(code));
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for obj in self.ui.iter_mut() {
                            let header = format!("{}  [{}]", obj.owner, obj.comp.label());
                            egui::CollapsingHeader::new(header)
                                .default_open(true)
                                .show(ui, |ui| {
                                    // — GameObject —
                                    egui::Frame::group(ui.style()).show(ui, |ui| {
                                        ui.monospace("GameObject");
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Nom").weak());
                                            ui.monospace(&obj.owner);
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Actif").weak());
                                            ui.monospace(if obj.active { "●" } else { "○" });
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Visible").weak());
                                            // Scale-zero hide (staged —
                                            // Appliquer writes zero or
                                            // the buffer scale).
                                            if ui.checkbox(&mut obj.visible, "").changed() {
                                                panel_edited = true;
                                            }
                                        });
                                    });
                                    // — Transform —
                                    egui::Frame::group(ui.style()).show(ui, |ui| {
                                        ui.monospace("Transform");
                                        // Buffers only — Appliquer sends.
                                        // `edited` is captured by the
                                        // outer closure (no `self` borrow
                                        // inside the rows themselves).
                                        panel_edited |=
                                            Self::xyz_row(ui, "Position", &mut obj.pos, true);
                                        panel_edited |=
                                            Self::xyz_row(ui, "Rotation", &mut obj.euler, true);
                                        panel_edited |=
                                            Self::xyz_row(ui, "Échelle", &mut obj.scale, true);
                                        // — Image / TextMeshProUGUI —
                                        ui.monospace(obj.comp.label());
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Couleur").weak());
                                            if ui.color_edit_button_srgba(&mut obj.color).changed() {
                                                panel_edited = true;
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Opacité").weak());
                                            // Dedicated buffer: CanvasGroup
                                            // path when present, merged
                                            // into color alpha on Apply
                                            // otherwise. Initialized from
                                            // whichever source was read.
                                            let mut pct = obj.opacity * 100.0;
                                            if ui
                                                .add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%"))
                                                .changed()
                                            {
                                                panel_edited = true;
                                                obj.opacity = (pct / 100.0).clamp(0.0, 1.0);
                                            }
                                        });
                                        if obj.comp == CompKind::Tmp {
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("Texte").weak());
                                                let preview: String =
                                                    obj.text.chars().take(40).collect();
                                                ui.monospace(format!("“{preview}”"));
                                            });
                                            ui.horizontal(|ui| {
                                                ui.label(egui::RichText::new("Taille").weak());
                                                match &mut obj.size {
                                                    Some(size) => {
                                                        if ui
                                                            .add(
                                                                egui::DragValue::new(size)
                                                                    .speed(0.5)
                                                                    .range(4.0..=300.0),
                                                            )
                                                            .changed()
                                                        {
                                                            panel_edited = true;
                                                        }
                                                    }
                                                    None => {
                                                        ui.monospace("n/a");
                                                    }
                                                }
                                            });
                                        }
                                    });
                                });
                        }
                    });
            });
        if let Some(r) = win {
            self.tool.context_menu(&r.response);
        }
        // — Overlay markers at the projected GameObject positions —
        // Each marker is a small draggable window anchored where the
        // game object projects on screen (one-shot projection at
        // discovery). Dragging a marker converts the pixel delta back
        // to a world offset (per-object calibration) straight into
        // the Position buffer — Appliquer then moves the real object
        // in the single batch. When panel values move instead, markers
        // re-anchor on those value-driven frames (`driven`); free
        // drags never fight.
        let scr_h = ctx.content_rect().height();
        for obj in self.ui.iter_mut() {
            let (sx_raw, sy_raw) = match obj.screen {
                Some(s) if s[2] > 0.0 => (s[0], s[1]),
                _ => continue,
            };
            // Derived anchor: projection + (buffer − discovery) mapped
            // back to pixels (egui y grows down, hence the flip).
            let px = sx_raw + (obj.pos[0] - obj.pos0[0]) * obj.ex;
            let py = scr_h - (sy_raw + (obj.pos[1] - obj.pos0[1]) * obj.ey);
            let driven = just_synced || panel_edited;
            let id = egui::Id::new(format!("speedo::marker::{}", obj.owner));
            let mut w = egui::Window::new(format!("◎ {}", obj.owner))
                .id(id)
                .resizable(false)
                .collapsible(false)
                .default_pos(egui::pos2(px, py));
            if driven {
                w = w.current_pos(egui::pos2(px, py));
            }
            let mr = w.show(ctx, |ui| {
                ui.monospace(format!("{px:.0},{py:.0}"));
            });
            if let Some(r) = mr {
                let cur = r.response.rect.min;
                match obj.last_rect {
                    // Free drags only: value-driven frames already
                    // placed the window, converting again would
                    // double-apply.
                    Some(prev) if !driven => {
                        let d = [cur.x - prev.x, cur.y - prev.y];
                        if d[0] != 0.0 || d[1] != 0.0 {
                            let off = screen_drag_to_world(d, obj.ex, obj.ey);
                            obj.pos[0] += off[0];
                            obj.pos[1] += off[1];
                        }
                    }
                    _ => {}
                }
                obj.last_rect = Some(cur);
            }
        }
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        crate::log_line("speedo: inspection des objets (one-shot)");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore_all();
        *lock(&self.pending_batch) = None;
        crate::log_line("speedo: off (originaux restaurés)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(SpeedoMod::new()));
}

#[cfg(test)]
mod tests {
    use super::{WANTS, color_to_f32, f32_to_color, group_enabled, screen_drag_to_world};

    #[test]
    fn group_flags() {
        assert!(group_enabled(0, true, false, false, false));
        assert!(!group_enabled(0, false, true, true, true));
        assert!(group_enabled(1, false, true, false, false));
        assert!(group_enabled(2, false, false, true, false));
        assert!(group_enabled(3, false, false, false, true));
        assert!(!group_enabled(9, true, true, true, true));
    }

    #[test]
    fn table_covers_gauge_and_texts() {
        let owners: Vec<&str> = WANTS.iter().map(|w| w.owner).collect();
        for need in [
            "Background",
            "speedometer_bg",
            "Tachometer",
            "Arrow",
            "ImgPointer",
            "Text (TMP) Speed",
            "Text (TMP) Gear",
            "Nitro",
            "nitro_bar_new",
            "TextN2O",
        ] {
            assert!(owners.contains(&need), "missing {need}");
        }
    }

    #[test]
    fn drag_to_world_mapping() {
        // 100 px per world unit on both axes (W2S, y up).
        let w = screen_drag_to_world([50.0, -20.0], 100.0, 100.0);
        assert!((w[0] - 0.5).abs() < 1e-6);
        // egui drag down (-20 dy) = up on screen = +0.2 world.
        assert!((w[1] - 0.2).abs() < 1e-6);
        assert_eq!(w[2], 0.0);
        // Locked axis (zero factor) never moves.
        let w = screen_drag_to_world([50.0, 50.0], 0.0, 0.0);
        assert_eq!(w, [0.0, 0.0, 0.0]);
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
