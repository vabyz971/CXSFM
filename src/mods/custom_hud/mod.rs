//! Custom HUDs, minimalist GT style, laid out in `custom_hud.toml`.
//!
//! `$XDG_CONFIG_HOME/cxsfm/custom_hud.toml` (else `~/.config/cxsfm/`) holds
//! any number of `[[hud]]` blocks (see `model::DEFAULT_CONFIG`, written
//! on first enable when the file is missing). The tick re-reads it when
//! its fingerprint changes (~1 s), so edits show up in game without a
//! restart; a broken file keeps the last good layout on screen.
//!
//! Values: `mode = "live"` reads the `[sources]` field paths at the
//! config fps through `live` (raw field copies, bounded root-pointer
//! exception — see that module); `mode = "demo"` animates
//! `model::demo_values` (zero Unity traffic).
//!
//! PHASE 2: the "Explorer" tab finds where the game keeps those
//! values — class search (metadata only) + on-demand field dumps of a
//! live instance, following reference fields from a re-found root.
//! Read-only: nothing in the game is ever written.
//!
//! Layout: `model` (config, values, explorer decoding, pure helpers +
//! tests), `bind` (explorer, tick side, on demand only), `ui`
//! (present-thread rendering + control window). Disabling only hides
//! the HUDs (nothing in the game was changed).

pub mod i18n;

mod bind;
mod live;
pub(crate) mod model;
mod ui;

/// Log tag.
pub(crate) const MOD_ID: &str = "custom_hud";

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use live::{LiveReader, LiveStatus};
use model::{ExploreReq, ExploreView, HudConfig, Mode, Values};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Reload probe cadence (ticks, ~1 s at 60 Hz).
const RELOAD_EVERY_TICKS: u64 = 60;

/// Control-window status (tick writes, render clones).
#[derive(Clone, Default)]
pub(crate) struct Status {
    pub(crate) path: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) warnings: Vec<String>,
}

/// Mod version (shown on the phone; bump with its features).
pub const VERSION: &str = "0.2.0";

pub struct HudMod {
    pub(crate) enabled: AtomicBool,
    pub(crate) tick: AtomicU64,
    /// Re-read the file on next tick (enable, Reload button).
    pub(crate) reload_now: AtomicBool,
    /// Fingerprint of the last file read (`u64::MAX` = never).
    pub(crate) last_fp: AtomicU64,
    /// Published layout (tick swaps, render clones the `Arc`).
    pub(crate) config: Mutex<Option<Arc<HudConfig>>>,
    pub(crate) status: Mutex<Status>,
    /// Control-window chrome (present thread only).
    pub(crate) tool: ToolChrome,
    /// Explorer: Unity cache (resolved on first Open), pending request
    /// (render → tick, one at a time), published view (tick → render).
    pub(crate) unity: Mutex<Option<UnityCache>>,
    pub(crate) explore_req: Mutex<Option<ExploreReq>>,
    pub(crate) explore: Mutex<ExploreView>,
    /// Present thread only: active tab, search box text.
    pub(crate) tab_explore: bool,
    pub(crate) query: String,
    /// Live values (tick reads at the config fps, render clones).
    pub(crate) live: Mutex<Values>,
    pub(crate) live_status: Mutex<LiveStatus>,
    /// Tick-only reader state (bounded root-pointer exception).
    pub(crate) reader: Mutex<LiveReader>,
}

// SAFETY: shared state is atomics or mutex-guarded plain data; the
// Unity cache holds process-stable class/method pointers only.
unsafe impl Send for HudMod {}
unsafe impl Sync for HudMod {}

impl HudMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            tick: AtomicU64::new(0),
            reload_now: AtomicBool::new(false),
            last_fp: AtomicU64::new(u64::MAX),
            config: Mutex::new(None),
            status: Mutex::new(Status::default()),
            tool: ToolChrome::new(),
            unity: Mutex::new(None),
            explore_req: Mutex::new(None),
            explore: Mutex::new(ExploreView::default()),
            tab_explore: false,
            query: String::new(),
            live: Mutex::new(Values::default()),
            live_status: Mutex::new(LiveStatus::default()),
            reader: Mutex::new(LiveReader::default()),
        }
    }

    /// Tick thread: (re)load `custom_hud.toml` when forced or changed. Writes
    /// the example first when the file does not exist.
    fn reload(&self, force: bool) {
        let code = crate::i18n::current().code();
        let Some(path) = crate::config::hud_path() else {
            if force {
                crate::log_line(&i18n::log_no_home(code).replace("{tag}", MOD_ID));
            }
            return;
        };
        let shown = path.display().to_string();
        if !path.exists() {
            match crate::config::save_toml(&path, model::DEFAULT_CONFIG) {
                Ok(()) => crate::log_line(
                    &i18n::log_example_written(code)
                        .replace("{tag}", MOD_ID)
                        .replace("{path}", &shown),
                ),
                Err(e) => {
                    lock(&self.status).error = Some(e.to_string());
                    return;
                }
            }
        }
        let fp = crate::config::file_fingerprint(&path).unwrap_or(0);
        if !force && self.last_fp.load(Ordering::SeqCst) == fp {
            return;
        }
        self.last_fp.store(fp, Ordering::SeqCst);
        let parsed = crate::config::load_toml(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| model::parse_config(&text));
        let mut st = lock(&self.status);
        st.path = Some(shown);
        match parsed {
            Ok((cfg, warnings)) => {
                crate::log_line(
                    &i18n::log_loaded(code)
                        .replace("{tag}", MOD_ID)
                        .replace("{n}", &cfg.huds.len().to_string())
                        .replace("{w}", &warnings.len().to_string()),
                );
                st.error = None;
                st.warnings = warnings;
                drop(st);
                *lock(&self.config) = Some(Arc::new(cfg));
            }
            Err(e) => {
                crate::log_line(&i18n::log_error(code).replace("{tag}", MOD_ID).replace("{e}", &e));
                st.error = Some(e);
            }
        }
    }
}

impl HudMod {
    /// Live sources at the config fps (tick thread). Demo mode, no
    /// sources or no visible HUD = zero Unity traffic.
    fn live_tick(&self, t: u64) {
        let Some(cfg) = lock(&self.config).clone() else {
            return;
        };
        if cfg.mode != Mode::Live || cfg.sources.is_empty() || !cfg.huds.iter().any(|h| h.enabled) {
            return;
        }
        let every = (60 / u64::from(cfg.fps.max(1))).max(1);
        if t % every != 0 {
            return;
        }
        let Some(api) = crate::il2cpp_api() else {
            return;
        };
        let Some(cache) = self.explore_unity(api) else {
            return;
        };
        // SAFETY: attached tick thread; the tick never runs mod updates
        // during teardown. Reader rules: see `live` module docs.
        let (values, status) = unsafe { lock(&self.reader).read_all(api, &cache, &cfg.sources, t) };
        *lock(&self.live) = values;
        *lock(&self.live_status) = status;
    }
}

impl Default for HudMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for HudMod {
    fn name(&self) -> &'static str {
        "Custom HUD"
    }

    fn mod_id(&self) -> &'static str {
        MOD_ID
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_custom_hud"
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Gauge
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    /// Always "pinned": the HUDs must stay on screen with the menu
    /// closed (view-only — input capture follows the menu).
    fn is_pinned(&self) -> bool {
        true
    }

    fn on_update(&mut self, _delta_time: f32) {
        let t = self.tick.fetch_add(1, Ordering::SeqCst);
        let force = self.reload_now.swap(false, Ordering::SeqCst);
        if force || t % RELOAD_EVERY_TICKS == 0 {
            self.reload(force);
        }
        // Explorer: on-demand only (no request = no Unity traffic).
        self.explore_tick();
        self.live_tick(t);
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        self.draw(ctx);
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        self.reload_now.store(true, Ordering::SeqCst);
        crate::log_line("hud: on");
    }

    fn on_disable(&mut self) {
        // Nothing in the game was touched: hiding is the full undo.
        // Drop the kept root instance with the values (re-found on
        // the next enable).
        self.enabled.store(false, Ordering::SeqCst);
        *lock(&self.reader) = LiveReader::default();
        *lock(&self.live) = Values::default();
        *lock(&self.live_status) = LiveStatus::default();
        crate::log_line("hud: off");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(HudMod::new()));
}
