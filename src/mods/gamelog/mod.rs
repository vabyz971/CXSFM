//! Game-log tool: tails Unity's `Player.log` for context.
//!
//! The game logs scene loads, its own `System Language`, C# stack
//! traces and errors — everything mod work needs correlated in one
//! place. Plain file I/O, zero game interaction: this tool cannot
//! crash the game by construction.
//!
//! A reader on the tick thread (~1 Hz) follows the file offset
//! (rotation = shrink → rewind) and keeps the last lines in a ring;
//! the window filters by text and highlights errors.
//!
//! Layout mirrors every other mod: `model` (caps, path, sniffing),
//! `bind` (tick-side tail), `ui` (render-side window).

pub mod i18n;

mod bind;
pub(crate) mod model;
mod ui;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Game-state arrival tracking (drives save auto-apply in speedo/map).
/// Bumped on every DISTINCT `[GameStateManager]` transition line the
/// tail sees (menu, city, garage — repeats of the same line don't
/// bump). Generation 0 + timestamp 0 = nothing seen yet this process.
static ARRIVAL_GEN: AtomicU64 = AtomicU64::new(0);
static ARRIVAL_MS: AtomicU64 = AtomicU64::new(0);
static LAST_STATE: Mutex<String> = Mutex::new(String::new());

/// Wall-clock millis (shared tick-time base for arrival delays).
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Feed one fresh Player.log line (tick thread, from `poll`).
/// State transitions bump the arrival generation + timestamp.
pub(crate) fn note_game_state_line(line: &str) {
    if !model::is_state_line(line) {
        return;
    }
    let mut last = lock(&LAST_STATE);
    if *last != line {
        *last = line.to_string();
        drop(last);
        ARRIVAL_GEN.fetch_add(1, Ordering::SeqCst);
        ARRIVAL_MS.store(now_ms(), Ordering::SeqCst);
    }
}

/// Current arrival `(generation, millis)`. Mods serve a generation
/// once (after the settle delay) and ignore repeats.
pub(crate) fn arrival() -> (u64, u64) {
    (
        ARRIVAL_GEN.load(Ordering::SeqCst),
        ARRIVAL_MS.load(Ordering::SeqCst),
    )
}

/// Game-log tool: tail on tick, filter on render.
pub struct GameLogMod {
    pub(crate) enabled: AtomicBool,
    pub(crate) tick: AtomicU64,
    pub(crate) path: Mutex<Option<std::path::PathBuf>>,
    pub(crate) offset: Mutex<u64>,
    pub(crate) pending: Mutex<String>,
    pub(crate) lines: Mutex<Vec<String>>,
    pub(crate) logged_path: AtomicBool,
    /// Pin + opacity chrome (shared tool pattern).
    pub(crate) tool: ToolChrome,
    // -- UI state (render thread only) --
    pub(crate) filter: String,
    pub(crate) errors_only: bool,
}

impl GameLogMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            tick: AtomicU64::new(0),
            path: Mutex::new(None),
            offset: Mutex::new(0),
            pending: Mutex::new(String::new()),
            lines: Mutex::new(Vec::new()),
            logged_path: AtomicBool::new(false),
            tool: ToolChrome::new(),
            filter: String::new(),
            errors_only: false,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
}

impl Default for GameLogMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for GameLogMod {
    fn name(&self) -> &'static str {
        "Game Log"
    }

    fn mod_id(&self) -> &'static str {
        "gamelog"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_gamelog"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Log
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
        if lock(&self.path).is_none() {
            let p = model::player_log_path();
            if let Some(ref path) = p {
                // Start at the tail: history belongs to Player-prev.log.
                let off = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                *lock(&self.offset) = off;
                if !self.logged_path.swap(true, Ordering::SeqCst) {
                    crate::log_line(&format!("gamelog: watching {}", path.display()));
                }
            }
            *lock(&self.path) = p;
        }
        let tick = self.tick.fetch_add(1, Ordering::SeqCst);
        if tick % model::POLL_EVERY_TICKS == 0 {
            self.poll();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        self.draw_window(ctx);
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        crate::log_line("gamelog: armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        crate::log_line("gamelog: off (file untouched — read-only)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(GameLogMod::new()));
}
