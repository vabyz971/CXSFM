//! Mod API façade: stable import surface for mods and UI code.
//!
//! The real pieces live in [`crate::mods::api`] (trait + tile types)
//! and [`crate::mods::loader`] (isolated registry). This module keeps
//! the historical `mod_api` paths working and hosts the singleton.
//!
//! Mods implement [`Mod`] (see its docs for the tutorial) and register
//! via [`register_mod`]; the menu reads [`ModManager::tile_info`].

use std::sync::LazyLock;

pub use crate::mods::api::{Mod, ModTile, TileIcon};
use crate::mods::loader::ModRegistry;

/// Manages the collection of registered mods and their lifecycle.
///
/// Thin wrapper over [`ModRegistry`]: every hook call is panic-isolated
/// per mod (a crashing mod is quarantined, never fatal), the registry
/// lock is never held across mod code, and `unregister` disables first.
///
/// ## Thread Safety
///
/// All public methods are thread-safe and lock-free from the caller's
/// point of view; internal poisoning is recovered, not propagated.
pub struct ModManager {
    registry: ModRegistry,
    /// Whether the manager is initialized
    initialized: bool,
}

impl ModManager {
    /// Create a new mod manager.
    pub fn new() -> Self {
        Self {
            registry: ModRegistry::new(),
            initialized: false,
        }
    }

    /// Register a new mod (starts disabled — the framework boots quiet
    /// and the user opts in; duplicate names are refused).
    pub fn register_mod(&self, mod_obj: Box<dyn Mod>) {
        self.registry.register(mod_obj);
    }

    /// Unregister a mod by name (disables first, so game state is
    /// restored). Returns true if a mod with that name was removed.
    pub fn unregister_mod(&self, name: &str) -> bool {
        self.registry.unregister(name)
    }

    /// Snapshot of `(name, enabled)` for every registered mod (no lock held).
    pub fn mod_list(&self) -> Vec<(String, bool)> {
        self.registry.mod_list()
    }

    /// Owned per-mod tile snapshot for the menu grid (draws lock-free).
    pub fn tile_info(&self) -> Vec<ModTile> {
        self.registry.tile_info()
    }

    /// Owned `(name, enabled, description)` snapshot for the About
    /// page (`lang_code` like `"fr"` — see `i18n::Lang::code`).
    pub fn describe_all(&self, lang_code: &str) -> Vec<(String, bool, &'static str)> {
        self.registry.describe_all(lang_code)
    }

    /// Set a mod's on/off state by name (menu tile toggle path).
    /// Returns true when a mod with that name exists.
    pub fn set_mod_enabled(&self, name: &str, enabled: bool) -> bool {
        self.registry.set_enabled(name, enabled)
    }

    /// Seconds since the manager was created (About page).
    pub fn uptime_secs(&self) -> u64 {
        self.registry.uptime_secs()
    }

    /// Call the update method on all *enabled* mods.
    pub fn update_all(&self, delta_time: f32) {
        self.registry.update_all(delta_time);
    }

    /// Call the draw UI method on *enabled* mods (pinned ones draw
    /// even when `menu_open` is false).
    pub fn draw_ui_all(&self, ctx: &egui::Context, menu_open: bool) {
        self.registry.draw_ui_all(ctx, menu_open);
    }

    /// Whether any enabled mod is currently pinned.
    pub fn has_pinned_visible(&self) -> bool {
        self.registry.has_pinned_visible()
    }

    /// Enable a mod by name. Returns true if it exists (even if already on).
    pub fn enable_mod(&self, name: &str) -> bool {
        self.registry.enable(name)
    }

    /// Disable a mod by name. Returns true if it exists (even if already off).
    pub fn disable_mod(&self, name: &str) -> bool {
        self.registry.disable(name)
    }

    /// Framework status window: folded into the menu's About page.
    ///
    /// Kept for API compatibility; the single menu window (`draw_menu_ui`
    /// via `draw_manager_ui`) now carries version, uptime and log path
    /// (see `crate::ui::pages::page_about`).
    pub fn draw_status_ui(&self, _ctx: &egui::Context) {
        // Intentionally empty: see `crate::menu::page_about`.
    }

    /// Mod menu window: Kino-style tile grid (see `crate::menu`).
    pub fn draw_manager_ui(&self, ctx: &egui::Context) {
        crate::menu::draw_menu_ui(ctx);
    }

    /// Check if the manager has been initialized
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Query a mod's on/off state by name.
    #[inline]
    pub fn is_mod_enabled(&self, name: &str) -> Option<bool> {
        self.registry.is_enabled(name)
    }

    /// Mark the manager as initialized
    #[inline]
    pub fn set_initialized(&mut self) {
        self.initialized = true;
    }

    /// Get the number of registered mods
    #[inline]
    pub fn mod_count(&self) -> usize {
        self.registry.mod_count()
    }

    /// Get the number of currently enabled mods
    #[inline]
    pub fn enabled_count(&self) -> usize {
        self.registry.enabled_count()
    }
}

impl Default for ModManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global mod manager instance using modern `std::sync::LazyLock`
static MOD_MANAGER: LazyLock<ModManager> = LazyLock::new(ModManager::new);

/// Get a reference to the global mod manager
#[inline]
pub fn get_mod_manager() -> &'static ModManager {
    &MOD_MANAGER
}

/// Convenience function to register a mod with the global manager
#[inline]
pub fn register_mod(mod_obj: Box<dyn Mod>) {
    get_mod_manager().register_mod(mod_obj)
}

/// Convenience function to update all registered mods
#[inline]
pub fn update_all_mods(delta_time: f32) {
    get_mod_manager().update_all(delta_time)
}

/// Convenience function to draw UI for enabled mods (pinned ones
/// draw even when `menu_open` is false)
#[inline]
pub fn draw_ui_all(ctx: &egui::Context, menu_open: bool) {
    get_mod_manager().draw_ui_all(ctx, menu_open)
}

/// Convenience function to draw the framework status window
#[inline]
pub fn draw_status_ui(ctx: &egui::Context) {
    get_mod_manager().draw_status_ui(ctx)
}

/// Convenience function to draw the mod manager window
#[inline]
pub fn draw_manager_ui(ctx: &egui::Context) {
    get_mod_manager().draw_manager_ui(ctx)
}
