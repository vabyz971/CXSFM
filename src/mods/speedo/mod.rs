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

/// The GameObjects composing the stock speedometer.
const WANTS: &[Want] = &[
    Want { group: GROUP_GAUGE, owner: "Background", comp: CompKind::Image },
    Want { group: GROUP_GAUGE, owner: "Tachometer", comp: CompKind::Image },
    Want { group: GROUP_GAUGE, owner: "Arrow", comp: CompKind::Image },
    Want { group: GROUP_SPEED, owner: "Text (TMP) Speed", comp: CompKind::Tmp },
    Want { group: GROUP_GEAR, owner: "Text (TMP) Gear", comp: CompKind::Tmp },
    Want { group: GROUP_NITRO, owner: "Nitro", comp: CompKind::Image },
];

/// `GameObject.Find` candidates for an owner: bare name first (root
/// object), then the known `Speedometer/` parent path (HUD children).
/// `Find` only sees active objects; a missing target simply stays out
/// of this discovery (status n/m, manual Refresh — never auto-retry).
fn find_candidates(owner: &str) -> [String; 2] {
    [owner.to_string(), format!("Speedometer/{owner}")]
}

/// Quaternion (x, y, z, w) → XYZ Euler degrees, display only.
/// Standard conversion; order noted because Unity shows ZXY — close
/// enough for a read-only readout, never fed back into a write.
fn quat_to_euler(q: [f32; 4]) -> [f32; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let sinr = 2.0 * (w * x + y * z);
    let cosr = 1.0 - 2.0 * (x * x + y * y);
    let sinp = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
    let siny = 2.0 * (w * z + x * y);
    let cosy = 1.0 - 2.0 * (y * y + z * z);
    let r2d = 180.0 / std::f32::consts::PI;
    [
        sinr.atan2(cosr) * r2d,
        sinp.asin() * r2d,
        siny.atan2(cosy) * r2d,
    ]
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

/// Published snapshot of one object (tick writes on discovery, render
/// clones; swapped under a brief lock, never held across Unity calls).
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
    font_size: Option<f32>,
    text: String,
}

/// First-recorded originals per address pair (restore source).
struct OrigRec {
    tr: usize,
    comp: usize,
    pos: [f32; 3],
    scale: [f32; 3],
    color: [f32; 4],
    font_size: Option<f32>,
}

/// One queued one-shot write (render pushes, tick drains).
struct WriteReq {
    addr: usize,
    op: u8,
    v: [f32; 4],
}

/// Render-side mirror of [`ObjSnap`] with editable buffers. Lives only
/// on the present thread (`&mut` in `on_draw_ui`); rebuilt whenever
/// the snapshot version bumps. Edits mutate these buffers and queue a
/// [`WriteReq`] — the snapshot itself is never written by the UI.
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
    size: Option<f32>,
    text: String,
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
    /// Queued one-shot writes (render pushes, tick drains).
    writes: Mutex<Vec<WriteReq>>,
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
            writes: Mutex::new(Vec::new()),
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
                let pos = crate::unity::tr_position(api, &cache, tr).unwrap_or([0.0, 0.0, 0.0]);
                let quat = crate::unity::tr_get_rotation(api, &cache, tr).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let color = crate::unity::graphic_get_color(api, &cache, comp_p)
                    .unwrap_or([1.0, 1.0, 1.0, 1.0]);
                let (font_size, text) = if want.comp == CompKind::Tmp {
                    let size = crate::unity::tmp_get_font_size(api, &cache, comp_p);
                    let txt = crate::unity::tmp_get_text(api, &cache, comp_p).unwrap_or_default();
                    (size, txt)
                } else {
                    (None, String::new())
                };
                let tr_addr = tr as usize;
                if !seen.iter().any(|&(t, c)| t == tr_addr && c == comp) {
                    seen.push((tr_addr, comp));
                    fresh_origs.push(OrigRec {
                        tr: tr_addr,
                        comp,
                        pos,
                        scale,
                        color,
                        font_size,
                    });
                }
                snaps.push(ObjSnap {
                    owner: want.owner.to_string(),
                    comp: want.comp,
                    active,
                    tr_addr,
                    comp_addr: comp,
                    pos,
                    euler: quat_to_euler(quat),
                    scale,
                    color,
                    font_size,
                    text,
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

    /// Drain queued one-shot writes (tick thread, only when the UI
    /// queued edits). Each address alive-checked right before its
    /// invoke; at most one log line per drain.
    fn drain_writes(&self) {
        let reqs = std::mem::take(&mut *lock(&self.writes));
        if reqs.is_empty() {
            return;
        }
        let tick = self.tick.load(Ordering::SeqCst);
        if !self.ensure_unity(tick, false) {
            lock(&self.writes).splice(0..0, reqs);
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
        // SAFETY: attached tick thread; per-request alive probe.
        unsafe {
            for r in &reqs {
                let obj = r.addr as *mut std::ffi::c_void;
                match crate::unity::object_alive(api, &cache, obj) {
                    Some(true) => {}
                    _ => continue,
                }
                let wrote = match r.op {
                    OP_SCALE => crate::unity::tr_set_scale(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_POS => crate::unity::tr_set_position(api, &cache, obj, [r.v[0], r.v[1], r.v[2]]),
                    OP_COLOR => crate::unity::graphic_set_color(api, &cache, obj, r.v),
                    OP_FONT => crate::unity::tmp_set_font_size(api, &cache, obj, r.v[0]),
                    _ => false,
                };
                if wrote {
                    ok += 1;
                }
            }
        }
        crate::log_line(&format!("speedo: écritures {ok}/{total}"));
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
        // SAFETY: attached tick thread; per-handle alive probes.
        unsafe {
            for r in &recs {
                let tr = r.tr as *mut std::ffi::c_void;
                let comp = r.comp as *mut std::ffi::c_void;
                let mut restored = false;
                if crate::unity::object_alive(api, &cache, tr) == Some(true) {
                    restored |= crate::unity::tr_set_scale(api, &cache, tr, r.scale);
                    restored |= crate::unity::tr_set_position(api, &cache, tr, r.pos);
                }
                if crate::unity::object_alive(api, &cache, comp) == Some(true) {
                    restored |= crate::unity::graphic_set_color(api, &cache, comp, r.color);
                    if let Some(size) = r.font_size {
                        restored |= crate::unity::tmp_set_font_size(api, &cache, comp, size);
                    }
                }
                if restored {
                    ok += 1;
                }
            }
        }
        crate::log_line(&format!("speedo: restaurés {ok}/{total}"));
        // Fresh UI next arming (scene may have changed meanwhile).
        *lock(&self.snaps) = Vec::new();
        self.snap_ver.fetch_add(1, Ordering::SeqCst);
        self.status_ok.store(0, Ordering::SeqCst);
        self.status_total.store(WANTS.len() as u64, Ordering::SeqCst);
    }

    /// Rebuild the render-side mirror when the snapshot version bumped
    /// (discovery / restore). Brief clone, lock released before draw.
    fn sync_ui(&mut self) {
        let ver = self.snap_ver.load(Ordering::SeqCst);
        if ver == self.ui_ver {
            return;
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
                euler: s.euler,
                scale: s.scale,
                color: f32_to_color(s.color),
                size: s.font_size,
                text: s.text,
            })
            .collect();
    }

    /// One `Position / Rotation / Scale` trio row: three X/Y/Z
    /// `DragValue`s. Rotation is display-only (the game drives it);
    /// position/scale edits queue one-shot writes. Associated function
    /// (no `&self`) so the caller keeps its `&mut` UI mirror while
    /// pushing to the queue: disjoint field borrows.
    fn xyz_row(
        queue: &Mutex<Vec<WriteReq>>,
        ui: &mut egui::Ui,
        name: &str,
        vals: &mut [f32; 3],
        editable: bool,
        addr: usize,
        op: u8,
    ) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).weak().monospace());
            // Borrow discipline: the `iter_mut` borrow lives for the
            // whole loop, so the queued write goes out AFTER it ends.
            let mut edited = false;
            for (axis, v) in vals.iter_mut().enumerate() {
                let tag = ["X", "Y", "Z"][axis];
                ui.label(egui::RichText::new(tag).weak().small());
                if editable {
                    let resp = ui.add(
                        egui::DragValue::new(v).speed(0.01).max_decimals(3),
                    );
                    if resp.changed() {
                        edited = true;
                    }
                } else {
                    ui.monospace(format!("{v:.2}"));
                }
            }
            if edited {
                lock(queue).push(WriteReq {
                    addr,
                    op,
                    v: [vals[0], vals[1], vals[2], 0.0],
                });
            }
        });
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
        // Explicit triggers only: discovery, restore, queued edits.
        // An idle tick performs zero Unity calls (no polling — that
        // crashed the game), so the present thread never skips us.
        if self.pending.swap(false, Ordering::SeqCst) {
            self.discover();
        }
        if self.restore_now.swap(false, Ordering::SeqCst) {
            self.restore_all();
        }
        if !lock(&self.writes).is_empty() {
            self.drain_writes();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        let code = crate::i18n::current().code();
        self.sync_ui();
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
                                    });
                                    // — Transform —
                                    egui::Frame::group(ui.style()).show(ui, |ui| {
                                        ui.monospace("Transform");
                                        // Split borrows: rows need `&mut`
                                        // fields while `queue` needs
                                        // `&self` — copy addresses out.
                                        let tr = obj.tr_addr;
                                        let comp = obj.comp_addr;
                                        let queue = &self.writes;
                                        Self::xyz_row(queue, ui, "Position", &mut obj.pos, true, tr, OP_POS);
                                        Self::xyz_row(queue, ui, "Rotation", &mut obj.euler, false, tr, OP_POS);
                                        Self::xyz_row(queue, ui, "Échelle", &mut obj.scale, true, tr, OP_SCALE);
                                        // — Image / TextMeshProUGUI —
                                        ui.monospace(obj.comp.label());
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new("Couleur").weak());
                                            if ui.color_edit_button_srgba(&mut obj.color).changed() {
                                                lock(queue).push(WriteReq {
                                                    addr: comp,
                                                    op: OP_COLOR,
                                                    v: color_to_f32(obj.color),
                                                });
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
                                                        let resp = ui.add(
                                                            egui::DragValue::new(size)
                                                                .speed(0.5)
                                                                .range(4.0..=300.0),
                                                        );
                                                        if resp.changed() {
                                                            lock(queue).push(WriteReq {
                                                                addr: comp,
                                                                op: OP_FONT,
                                                                v: [*size, 0.0, 0.0, 0.0],
                                                            });
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
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        crate::log_line("speedo: inspection des objets (one-shot)");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore_all();
        *lock(&self.writes) = Vec::new();
        crate::log_line("speedo: off (originaux restaurés)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(SpeedoMod::new()));
}

#[cfg(test)]
mod tests {
    use super::{WANTS, color_to_f32, f32_to_color, group_enabled, quat_to_euler};

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
            "Tachometer",
            "Arrow",
            "Text (TMP) Speed",
            "Text (TMP) Gear",
            "Nitro",
        ] {
            assert!(owners.contains(&need), "missing {need}");
        }
    }

    #[test]
    fn quat_identity_is_zero_euler() {
        let e = quat_to_euler([0.0, 0.0, 0.0, 1.0]);
        assert!(e[0].abs() < 1e-4 && e[1].abs() < 1e-4 && e[2].abs() < 1e-4);
    }

    #[test]
    fn quat_z90_is_yaw_90() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let e = quat_to_euler([0.0, 0.0, s, s]);
        assert!(e[0].abs() < 0.05 && e[1].abs() < 0.05 && (e[2] - 90.0).abs() < 0.05);
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
