//! Process-wide mod manager (the singleton behind [`register_mod`]).
//!
//! The real pieces live in [`crate::mod_trait`] (trait + tile types)
//! and [`crate::registry`] (isolated registry). Mods implement [`Mod`]
//! (see its docs for the tutorial) and register via [`register_mod`];
//! the menu (in the `cxsfm` crate) reads [`ModManager::tile_info`].

use std::sync::LazyLock;

use crate::mod_trait::{Mod, ModTile};
use crate::registry::ModRegistry;

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
    ///
    /// Records the intent and returns immediately: the transition hook
    /// runs on the tick thread (see `ModRegistry::update_all`), never
    /// on the calling thread. Returns true when a mod with that name
    /// exists.
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

    /// Draw the body of phone app `name` (see `ModRegistry::draw_app`).
    pub fn draw_app(&self, name: &str, ui: &mut egui::Ui, tab: usize) -> Option<String> {
        self.registry.draw_app(name, ui, tab)
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
