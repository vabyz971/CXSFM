//! Custom HUDs, minimalist GT style, laid out in `hud.toml`.
//!
//! `$XDG_CONFIG_HOME/cxsfm/hud.toml` (else `~/.config/cxsfm/`) holds
//! any number of `[[hud]]` blocks (see `model::DEFAULT_CONFIG`, written
//! on first enable when the file is missing). The tick re-reads it when
//! its fingerprint changes (~1 s), so edits show up in game without a
//! restart; a broken file keeps the last good layout on screen.
//!
//! PHASE 1: values come from `model::demo_values` (rendering + config
//! only, zero Unity traffic). Live game values are the next phase —
//! the getter-polling path crashed the game before (see field-notes,
//! speedometer REDESIGN), so they will come from plain field reads.
//!
//! Layout: `model` (config, values, pure helpers + tests), `ui`
//! (present-thread rendering + control window). No `bind`: nothing
//! touches Unity yet. Disabling only hides the HUDs (nothing in the
//! game was changed).

pub mod i18n;

pub(crate) mod model;
mod ui;

/// Log tag.
pub(crate) const MOD_ID: &str = "hud";

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use model::HudConfig;
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
}

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
        }
    }

    /// Tick thread: (re)load `hud.toml` when forced or changed. Writes
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
        "mod_hud"
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
        self.enabled.store(false, Ordering::SeqCst);
        crate::log_line("hud: off");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(HudMod::new()));
}
