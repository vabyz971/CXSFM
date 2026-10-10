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

pub mod i18n;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::common::lock;
use crate::mods::tool::ToolChrome;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Poll cadence (~1 Hz at 60 Hz tick).
const POLL_EVERY_TICKS: u64 = 60;
/// Ring capacity (lines).
const MAX_LINES: usize = 300;
/// Per-poll read cap (a flood must not stall the tick).
const MAX_BYTES_PER_POLL: u64 = 64 * 1024;

/// Resolve the game's `Player.log`:
///
/// `$XDG_CONFIG_HOME/unity3d/CarX Technologies/CarX Street/Player.log`,
/// else `~/.config/...`. `None` outside a standard home layout.
fn player_log_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = std::path::PathBuf::from(h);
                p.push(".config");
                p
            })
        })?;
    let mut p = base;
    p.push("unity3d");
    p.push("CarX Technologies");
    p.push("CarX Street");
    p.push("Player.log");
    Some(p)
}

/// A line is an error when it smells like one (case-insensitive).
fn is_error(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("error")
        || l.contains("exception")
        || l.contains("failed")
        || l.contains("nullreference")
        || l.contains("crash")
}

/// Game-log tool: tail on tick, filter on render.
pub struct GameLogMod {
    enabled: AtomicBool,
    tick: AtomicU64,
    path: Mutex<Option<std::path::PathBuf>>,
    offset: Mutex<u64>,
    pending: Mutex<String>,
    lines: Mutex<Vec<String>>,
    logged_path: AtomicBool,
    /// Pin + opacity chrome (shared tool pattern).
    tool: ToolChrome,
    // -- UI state (render thread only) --
    filter: String,
    errors_only: bool,
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

    /// Read new bytes (tick thread). Rotation/truncation rewinds.
    /// Slow polls (>200 ms) log a tripwire: the present thread clones
    /// under the same lock, so a slow tick visibly stalls frames.
    fn poll(&self) {
        let t0 = std::time::Instant::now();
        let path = lock(&self.path).clone();
        let path = match path {
            Some(p) => p,
            None => return,
        };
        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut offset = lock(&self.offset);
        if len < *offset {
            // Rotated (Player.log → Player-prev.log): start over.
            *offset = 0;
            lock(&self.pending).clear();
        }
        if len == *offset {
            return;
        }
        if file.seek(SeekFrom::Start(*offset)).is_err() {
            return;
        }
        let take = (len - *offset).min(MAX_BYTES_PER_POLL);
        let mut buf = vec![0u8; take as usize];
        let n = match file.read(&mut buf) {
            Ok(n) => n,
            Err(_) => return,
        };
        *offset += n as u64;
        drop(offset);
        buf.truncate(n);
        let mut pending = lock(&self.pending);
        pending.push_str(&String::from_utf8_lossy(&buf));
        // Drain complete lines, keep the tail.
        let mut lines = lock(&self.lines);
        while let Some(pos) = pending.find('\n') {
            let line: String = pending.drain(..=pos).collect();
            let line = line.trim_end_matches(['\n', '\r']).to_string();
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if lines.len() > MAX_LINES {
            let drop_n = lines.len() - MAX_LINES;
            lines.drain(..drop_n);
        }
        drop(lines);
        drop(pending);
        let ms = t0.elapsed().as_millis();
        if ms > 200 {
            crate::log_line(&format!("gamelog: slow poll ({ms} ms)"));
        }
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
            let p = player_log_path();
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
        if tick % POLL_EVERY_TICKS == 0 {
            self.poll();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        // Fixed size: log lines must never resize the window.
        // Custom header (full-width bar + opacity options).
        let mut win = egui::Window::new("Game Log")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(700.0, 500.0))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
                self.tool.enter(ui);
                self.tool.header(ui, "Game Log");
                if !self.tool.hide_controls {
                ui.horizontal(|ui| {
                    ui.label("🔍");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("filter…")
                            .desired_width(f32::INFINITY),
                    );
                    ui.checkbox(&mut self.errors_only, "errors");
                });
                } // hide_controls
                // Clone under a brief lock: the tick's poll() also
                // takes it, and holding it across the whole ScrollArea
                // layout would stall the tick (visible as skipped
                // frames, and a stalled tick holds the mod lock that
                // the present thread skips on).
                let lines = lock(&self.lines).clone();
                if lines.is_empty() {
                    ui.weak("No lines yet — play a bit, the game logs as it goes.");
                    return;
                }
                let query = self.filter.to_lowercase();
                ui.label(format!("{} lines", lines.len()));
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in lines.iter() {
                            if self.errors_only && !is_error(line) {
                                continue;
                            }
                            if !query.is_empty() && !line.to_lowercase().contains(&query) {
                                continue;
                            }
                            let text = egui::RichText::new(line).monospace().size(14.0);
                            if is_error(line) {
                                ui.label(text.color(egui::Color32::from_rgb(0xE8, 0x70, 0x60)));
                            } else if line.contains("[cxsfm") {
                                ui.label(text.color(egui::Color32::from_rgb(0x8F, 0xD0, 0x8A)));
                            } else {
                                ui.label(text.color(egui::Color32::from_rgb(0xDC, 0xDC, 0xE2)));
                            }
                        }
                    });
            });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
        }
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
