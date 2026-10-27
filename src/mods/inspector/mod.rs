//! Scene inspector tool: browse what's live in the scene.
//!
//! Walks the active scene's root `GameObject`s via `SceneManager`
//! (inactive branches included — `FindObjectsOfType` would skip
//! them), then `Transform` children down. Each node shows name,
//! instance id, active flag and world position — the exact handles
//! future mods need (object names for `search_target`-style mods,
//! ids for log correlation).
//!
//! Walks run ONLY on explicit Refresh (button / arming) — never
//! periodically, so the open tree, selection and scroll offset survive
//! while the user browses. UI only reads the snapshot. Strictly
//! read-only: an explicit activate/deactivate toggle was tried and
//! parked — `SetActive` fires game lifecycle callbacks from a foreign
//! thread and crashed the game (see log).
//!
//! Selecting a node lists its components (`GetComponents`); a
//! component opens as a field table (raw field reads), reference
//! fields and arrays can be followed, and primitive / vector / color
//! fields edited (raw stores) — see `detail`.
//!
//! Layout mirrors `speedometer`/`map`: `model` (data + pure helpers),
//! `bind` + `detail` (tick-side Unity), `ui` (render-side window).

pub mod i18n;

mod bind;
mod detail;
pub(crate) mod model;
mod ui;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Mod version (shown on the phone; bump with its features).
pub const VERSION: &str = "1.1.0";

/// Scene inspector: snapshot on tick, tree on render.
pub struct InspectorMod {
    pub(crate) enabled: AtomicBool,
    pub(crate) unity: Mutex<Option<UnityCache>>,
    /// Set by the Refresh button (UI thread), consumed by the tick.
    pub(crate) refresh_wanted: AtomicBool,
    /// Index paths whose children the UI requested (node opened).
    /// Drained by the tick (Unity reads), then cleared.
    pub(crate) want_children: Mutex<Vec<Vec<usize>>>,
    pub(crate) snapshot: Mutex<Option<model::Snapshot>>,
    /// Pin + opacity chrome (shared tool pattern).
    pub(crate) tool: ToolChrome,
    // -- UI state (render thread only, via `on_draw_ui(&mut)`) --
    pub(crate) search: String,
    pub(crate) open: HashSet<Vec<usize>>,
    pub(crate) selected: Option<Vec<usize>>,
    /// Components / field view of the selection (tick → UI).
    pub(crate) detail: Mutex<Option<model::Detail>>,
    /// UI → tick requests (drained every tick).
    pub(crate) detail_req: Mutex<Vec<model::Req>>,
    pub(crate) comp_api: Mutex<Option<detail::CompApi>>,
    pub(crate) det_ui: ui::DetUi,
}

impl InspectorMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            refresh_wanted: AtomicBool::new(false),
            want_children: Mutex::new(Vec::new()),
            snapshot: Mutex::new(None),
            tool: ToolChrome::new(),
            search: String::new(),
            open: HashSet::new(),
            selected: None,
            detail: Mutex::new(None),
            detail_req: Mutex::new(Vec::new()),
            comp_api: Mutex::new(None),
            det_ui: ui::DetUi::default(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
}

impl Default for InspectorMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for InspectorMod {
    fn name(&self) -> &'static str {
        "Scene Inspector"
    }

    fn mod_id(&self) -> &'static str {
        "inspector"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_inspector"
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Tree
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
        // Manual refresh ONLY (Refresh button / arming): the old
        // periodic walk replaced the snapshot under the open tree every
        // ~2 s, yanking scroll + selection back to the top. A snapshot
        // now lives until the user asks for a new one.
        if self.refresh_wanted.swap(false, Ordering::SeqCst) {
            self.refresh(true);
        }
        // Drain on-demand child loads (UI-opened nodes, Unity here).
        let want = std::mem::take(&mut *lock(&self.want_children));
        if !want.is_empty() {
            self.load_children(want);
        }
        // Component / field requests (selection, edits, live view).
        let reqs = std::mem::take(&mut *lock(&self.detail_req));
        for r in reqs {
            self.detail_tick(r);
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        self.draw_window(ctx);
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        // Walk on the very next tick so the window isn't empty.
        self.refresh_wanted.store(true, Ordering::SeqCst);
        crate::log_line("inspector: armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        crate::log_line("inspector: off (field edits are not undone)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(InspectorMod::new()));
}
