//! Speedometer objects, Unity-Inspector style.
//!
//! No polling, no hooks, no per-tick Unity traffic (that crashed the
//! game after ~1 min — isolation test: 5 min clean with the mod off).
//! Instead the window lists the GameObjects composing the game's
//! dashboard the way Unity's Inspector shows them: a section tree
//! (`Turbo`, `Speedometer`, `Nitro`…) with per-object cards —
//! `GameObject` (name + active toggle), `Transform` (position /
//! rotation / scale), and for graphic leaves `Image` (color) or
//! `TextMeshProUGUI` (text preview, color, font size).
//!
//! Edits apply in REALTIME: touching a field stages a write request
//! for that object, drained on the very next tick in ONE Unity pass
//! (re-resolved by dashboard path first — stale handles are never
//! touched). Refresh re-discovers, Restore writes first-seen
//! originals back (transforms, colors, active states).
//!
//! Layout: `model` (data + pure helpers), `bind` (tick-side Unity),
//! `ui` (render-side window). Rules (proven in game, do not improvise):
//! - Unity calls only (a) on arming, (b) on Refresh / Restore clicks,
//!   (c) to drain an explicit write queue. NEVER per tick. Discovery
//!   is one explicit bounded walk of `UIHUDCarDashboard` (root `Find`
//!   + `Transform` children — the ONLY way to see INACTIVE branches
//!   like `Numbers`/`ElectroBooster`, which `GameObject.Find` never
//!   returns). Repeated/periodic enumeration from the tick crashed
//!   another mod — this walk runs on demand alone.
//! - Every handle alive-checked (`m_CachedPtr`) before every invoke;
//!   intermediates too (a scene churn between two calls segfaults).
//! - Originals recorded on first discovery per address, written back
//!   on disable / Restore.
//! - No Mutex held across a Unity call; the present thread only
//!   swaps/clones small data and pushes write requests.
//! - All mutexes go through `common::lock` (poison-recovering): a
//!   quarantined panic must never wedge the overlay.

pub mod i18n;

mod bind;
pub(crate) mod model;
mod ui;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use model::{ObjSnap, OrigRec, UiObj, WriteReq};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct SpeedoMod {
    pub(crate) enabled: AtomicBool,
    pub(crate) unity: Mutex<Option<UnityCache>>,
    pub(crate) tick: AtomicU64,
    /// Discover on next tick (arming, Refresh button).
    pub(crate) pending: AtomicBool,
    /// Restore originals on next tick (Restore button, no disable).
    pub(crate) restore_now: AtomicBool,
    /// Last tick a (re-)init was attempted; 0 = never.
    pub(crate) last_init: AtomicU64,
    /// Published snapshots + version (render clones).
    pub(crate) snaps: Mutex<Vec<ObjSnap>>,
    pub(crate) snap_ver: AtomicU64,
    /// First-recorded originals (tick only).
    pub(crate) origs: Mutex<Vec<OrigRec>>,
    /// Staged batch (render extends, tick takes once — every edit
    /// lands in a SINGLE Unity pass).
    pub(crate) pending_batch: Mutex<Option<Vec<WriteReq>>>,
    /// Drain the staged batch on next tick (set by every staged edit).
    pub(crate) apply_now: AtomicBool,
    /// Discovery status for the status line.
    pub(crate) status_ok: AtomicU64,
    pub(crate) status_total: AtomicU64,
    /// Render-side mirror (present thread only).
    pub(crate) ui: Vec<UiObj>,
    pub(crate) ui_ver: u64,
    /// Owner path selected in the tree (present thread only).
    pub(crate) selected: Option<String>,
    /// Collapsed section names (present thread only; absent = open).
    pub(crate) collapsed: HashSet<String>,
    /// Pin + opacity chrome (shared tool pattern).
    pub(crate) tool: ToolChrome,
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
            snaps: Mutex::new(Vec::new()),
            snap_ver: AtomicU64::new(0),
            origs: Mutex::new(Vec::new()),
            pending_batch: Mutex::new(None),
            apply_now: AtomicBool::new(false),
            status_ok: AtomicU64::new(0),
            status_total: AtomicU64::new(0),
            ui: Vec::new(),
            ui_ver: 0,
            selected: None,
            collapsed: HashSet::new(),
            tool: ToolChrome::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
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
        // Explicit triggers only: discovery, restore, staged-batch
        // drain. An idle tick performs zero Unity calls (no polling —
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
        self.draw_window(ctx);
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
