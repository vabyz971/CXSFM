//! Minimap objects, Unity-Inspector style (same interface as speedo).
//!
//! No polling, no hooks, no per-tick Unity traffic. Instead the window
//! lists the GameObjects composing the game's minimap the way Unity's
//! Inspector shows them: a section tree (`Map`, fuel/tires roots as
//! found…) with per-object cards — `GameObject` (name + active
//! toggle), `Transform` (position / rotation / scale), and for graphic
//! leaves `Image` (color) or `TextMeshProUGUI` (text preview, color,
//! font size).
//!
//! Roots and vocabulary come from wyr-menu v0.2 (`minhud.so` strings):
//! `UIHUDMinimap` → `Map/...` (`ImgFrame`, `Mask/Base/RoadImage`,
//! `PlayerIconContainer`, `NorthIcon`…), hide via `SetActive`, opacity
//! via `CanvasGroup`. See `docs/field-notes.md` (WYR-MENU section).
//!
//! Edits apply in REALTIME: touching a field stages a write request
//! for that object, drained on the very next tick in ONE Unity pass
//! (re-resolved by map path first — stale handles are never touched).
//! Refresh re-discovers, Restore writes first-seen originals back
//! (transforms, colors, active states).
//!
//! Layout mirrors `speedo`: `bind` (tick-side Unity), `ui`
//! (render-side window); data types are shared from
//! [`crate::mods::speedo::model`]. Same proven rules (see speedo):
//! explicit triggers only, alive-checks everywhere, originals on
//! disable / Restore, no Mutex across Unity calls.

pub mod i18n;

mod bind;
mod ui;

/// This mod's key inside save files (`[map."path"]`).
pub(crate) const MOD_ID: &str = "map";

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use crate::mods::speedo::model::{ObjSnap, OrigRec, UiObj, WriteReq};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct MinimapMod {
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
    /// Write the staged save text to disk on next tick (Save button).
    pub(crate) save_now: AtomicBool,
    /// Save text + object count, staged by the render thread (tick
    /// performs the file IO — never on the present thread).
    pub(crate) save_data: Mutex<Option<(String, usize)>>,
    /// Load and apply the save file on next tick (Load button).
    pub(crate) load_now: AtomicBool,
    /// Re-run discovery after the next drain (load-then-refresh so
    /// buffers mirror the just-written game state).
    pub(crate) discover_after_drain: AtomicBool,
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
unsafe impl Send for MinimapMod {}
unsafe impl Sync for MinimapMod {}

impl MinimapMod {
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
            save_now: AtomicBool::new(false),
            save_data: Mutex::new(None),
            load_now: AtomicBool::new(false),
            discover_after_drain: AtomicBool::new(false),
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

impl Default for MinimapMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for MinimapMod {
    fn name(&self) -> &'static str {
        "Minimap"
    }

    fn mod_id(&self) -> &'static str {
        "map"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_map"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Globe
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
        if self.save_now.swap(false, Ordering::SeqCst) {
            // File IO stays on the tick: the render thread only staged
            // the text (see draw_window Save button).
            if let Some((text, n)) = lock(&self.save_data).take() {
                match crate::config::saves_path() {
                    Some(path) => match crate::config::save_toml(&path, &text) {
                        Ok(()) => crate::log_line(&format!(
                            "map: sauvegardé {n} objets → {}",
                            path.display()
                        )),
                        Err(e) => crate::log_line(&format!("map: sauvegarde impossible ({e})")),
                    },
                    None => crate::log_line("map: sauvegarde impossible (HOME introuvable)"),
                }
            }
        }
        if self.load_now.swap(false, Ordering::SeqCst) {
            self.load_saved();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        self.draw_window(ctx);
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        self.pending.store(true, Ordering::SeqCst);
        crate::log_line("map: inspection des objets (one-shot)");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore_all();
        *lock(&self.pending_batch) = None;
        crate::log_line("map: off (originaux restaurés)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(MinimapMod::new()));
}
