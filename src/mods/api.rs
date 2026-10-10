//! Mod plugin API: the only surface a mod may touch.
//!
//! Each mod lives in its own folder under `src/mods/` and is compiled
//! into the framework `.so` (no external files, nothing extra
//! injected). It talks to the framework only through this module; the
//! loader (`super::loader`) calls these hooks with per-mod panic
//! isolation, so one crashing mod can never take the menu down.
//!
//! ## Writing a mod
//!
//! ```
//! use cxsfm::mods::api::{Mod, TileIcon};
//!
//! struct MyMod;
//!
//! impl Mod for MyMod {
//!     fn name(&self) -> &'static str {
//!         "My Custom Mod"
//!     }
//!
//!     fn on_update(&mut self, delta_time: f32) {
//!         // Game logic — runs at ~60 Hz while enabled.
//!     }
//! }
//! ```

/// Vector icon drawn on a mod tile.
///
/// Owned here (not by UI code) so mods declare their icon without
/// depending on the menu. The menu renders each variant with the
/// `Painter` (no fonts, no assets).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileIcon {
    Camera,
    Scout,
    EyeOff,
    Tag,
    Globe,
    Info,
    /// Scene-tree browser.
    Tree,
    /// Sliders (video/quality settings).
    Sliders,
    /// Log lines (game log viewer).
    Log,
    /// Two cars (player collision toggle).
    Cars,
    /// Gamepad (free camera drive).
    Gamepad,
    /// Gauge dial (custom speedometer).
    Gauge,
}

impl Default for TileIcon {
    /// Neutral fallback (used when a mod's icon getter panics).
    fn default() -> Self {
        TileIcon::Info
    }
}

/// Core trait that all mods must implement.
///
/// Implementors receive callbacks for game update ticks and UI
/// rendering. `name()` identifies the mod (must be unique across all
/// registered mods — the loader rejects duplicates).
///
/// ## Threading
///
/// `on_update` runs on the framework tick thread, `on_draw_ui` on the
/// render (present) thread. Implementations must be thread-safe when
/// sharing state between the two (the trait requires `Send + Sync`).
///
/// ## Panics
///
/// A panicking hook does NOT crash the framework: the loader catches
/// it, quarantines the mod (auto-disabled) and logs. Keep hooks
/// infallible anyway — a quarantined mod stays off until re-enabled.
pub trait Mod: Send + Sync {
    /// Human-readable, unique mod name (also the default menu id).
    fn name(&self) -> &'static str;

    /// Stable namespaced id for i18n keys (`{id}:key`).
    ///
    /// Defaults to `name()`; override when the display name may change
    /// while translations must stay put.
    #[inline]
    fn mod_id(&self) -> &'static str {
        self.name()
    }

    /// Called each game tick with seconds since the last update.
    ///
    /// Only called while the mod is enabled. Keep it fast (the tick
    /// runs at ~60 Hz) and never block: resolve Unity handles at low
    /// cadence and cache classes, not objects.
    fn on_update(&mut self, delta_time: f32);

    /// Optional: render UI elements.
    ///
    /// Only called while the mod is enabled. Return early when there
    /// is nothing to draw this frame.
    #[inline]
    fn on_draw_ui(&mut self, _ctx: &egui::Context) {
        // Default implementation does nothing.
    }

    /// Optional: called once on the off→on transition.
    #[inline]
    fn on_enable(&mut self) {
        // Default implementation does nothing.
    }

    /// Optional: called once on the on→off transition.
    ///
    /// Restore everything touched (flags, texts, FOV) so disabling is
    /// a full undo. Also called automatically before unregistration.
    #[inline]
    fn on_disable(&mut self) {
        // Default implementation does nothing.
    }

    /// i18n key for the menu tile label.
    ///
    /// Defaults to the technical `name()`; override with a stable key
    /// whose strings live in the mod's own `i18n` module (see
    /// `inspector` for the pattern).
    #[inline]
    fn menu_label_key(&self) -> &'static str {
        self.name()
    }

    /// One-line mod description for the About page, in `lang_code`
    /// (`en`, `fr`, … — see `Lang::code`).
    ///
    /// Defaults to empty (no description line); override with the
    /// mod's own `i18n::describe` so the text follows the menu
    /// language without touching framework files.
    #[inline]
    fn describe_in(&self, _lang_code: &str) -> &'static str {
        ""
    }

    /// Vector icon drawn on the menu tile.
    #[inline]
    fn menu_icon(&self) -> TileIcon {
        TileIcon::Info
    }

    /// Whether this mod's window stays visible when the menu closes.
    ///
    /// Tool mods expose a pin toggle (see `super::tool::ToolChrome`);
    /// the loader only draws pinned mods then. Pinned windows are
    /// view-only (capture follows the menu), so they can never freeze
    /// game input. Defaults to off (window lives and dies with the menu).
    #[inline]
    fn is_pinned(&self) -> bool {
        false
    }
}

/// One menu tile's worth of mod state (owned snapshot, no lock held).
pub struct ModTile {
    /// Technical name (for enable/disable by name).
    pub name: String,
    /// Current on/off state.
    pub enabled: bool,
    /// i18n key for the tile label.
    pub label_key: &'static str,
    /// Vector icon for the tile.
    pub icon: TileIcon,
}
