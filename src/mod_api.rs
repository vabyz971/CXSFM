//! Mod API module for CarX Street Framework Mod
//!
//! Defines the core trait for implementing mods and manages the mod lifecycle.
//!
//! The framework uses a trait-based approach where each mod implements the `Mod` trait.
//! The ModManager handles registration, initialization, and drawing of mods.

use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;
use egui;

/// Core trait that all mods must implement
///
/// Implementors of this trait receive callbacks for game update ticks and UI rendering.
/// Mods should implement `name()` to identify themselves and `on_update()` to respond
/// to game logic changes. The `on_draw_ui()` method is optional and should only be
/// implemented if the mod needs to render UI elements.
///
/// ## Safety
///
/// All methods receive safe wrappers around external resources. The `ctx` parameter
/// in `on_draw_ui` is a thread-safe egui context that can be used to create UI
/// elements without unsafe code.
///
/// ## Threading
///
/// The `on_update` method is called from a dedicated framework thread, not the game
/// main thread. Mod developers must ensure their implementation is thread-safe when
/// accessing shared state.
///
/// ## Example
/// ```
/// use cxsfm::mod_api::{Mod, ModManager};
/// use egui;
///
/// struct MyMod;
///
/// impl Mod for MyMod {
///     fn name(&self) -> &'static str {
///         "My Custom Mod"
///     }
///     
///     fn on_update(&mut self, delta_time: f32) {
///         // Game logic update - runs at fixed interval
///     }
///     
///     fn on_draw_ui(&mut self, ctx: &egui::Context) {
///         egui::Window::new("My Mod")
///             .show(ctx, |ui| {
///                 ui.label("Hello from CXSFM!");
///             });
///     }
/// }
/// ```
pub trait Mod: Send + Sync {
    /// Returns a human-readable name for the mod
    ///
    /// Used for debugging, logging, and the mod manager UI
    fn name(&self) -> &'static str;

    /// Called each game tick with the time elapsed since last update
    ///
    /// ~delta_time: Time in seconds since the last call to on_update
    ///
    /// Implement game logic modifications here. This is called from the framework's
    /// update thread, which runs independently of the game's main thread.
    /// Only called while the mod is enabled (see `ModManager::enable_mod`).
    fn on_update(&mut self, delta_time: f32);

    /// Optional: Called to render UI elements
    ///
    /// ~ctx: The egui context for this frame
    ///
    /// Mods that need to display UI should implement this method. The context is
    /// thread-safe and can be used to create windows, buttons, sliders, etc.
    /// Return early if no UI is needed for this frame.
    /// Only called while the mod is enabled.
    #[inline]
    fn on_draw_ui(&mut self, _ctx: &egui::Context) {
        // Default implementation does nothing
    }

    /// Optional: Called when the mod is enabled
    ///
    /// Implement initialization logic here (e.g., loading resources, setting up hooks)
    #[inline]
    fn on_enable(&mut self) {
        // Default implementation does nothing
    }

    /// Optional: Called when the mod is disabled
    ///
    /// Implement cleanup logic here (e.g., unloading resources, restoring original code)
    #[inline]
    fn on_disable(&mut self) {
        // Default implementation does nothing
    }
}

/// One registered plugin plus its on/off state.
///
/// The manager owns every mod; the `enabled` flag decides whether
/// `on_update` / `on_draw_ui` are invoked. Toggling the flag always
/// goes through the `on_enable` / `on_disable` hooks so mods can
/// set up and tear down cleanly.
struct ModEntry {
    inner: Box<dyn Mod>,
    enabled: bool,
}

/// Manages the collection of registered mods and their lifecycle
///
/// The ModManager is responsible for:
/// - Registering new mods
/// - Calling update/draw methods on enabled mods
/// - Managing mod enable/disable state
/// - Providing thread-safe access to the mod collection
/// - Rendering the built-in status and manager windows
///
/// ## Thread Safety
///
/// All public methods are thread-safe. Internal synchronization uses a `Mutex` to
/// protect the mod vector. Mod callbacks are invoked with the lock held for
/// simplicity, so mods should avoid performing long operations in their callbacks.
pub struct ModManager {
    /// Thread-safe collection of registered mods with their on/off state
    mods: Mutex<Vec<ModEntry>>,
    /// Whether the manager is initialized
    initialized: bool,
    /// When the manager singleton was first created (for uptime display)
    loaded_at: SystemTime,
}

impl ModManager {
    /// Create a new mod manager
    ///
    /// Returns a new instance ready for mod registration
    pub fn new() -> Self {
        Self {
            mods: Mutex::new(Vec::new()),
            initialized: false,
            loaded_at: SystemTime::now(),
        }
    }

    /// Register a new mod with the manager
    ///
    /// ~mod: Boxed trait object implementing the `Mod` trait
    ///
    /// Takes ownership of the mod and stores it internally. New mods start
    /// **disabled**: the framework boots quiet and the user opts in (O,
    /// manager UI). This guarantees a freshly injected framework never
    /// alters the game before an explicit user action.
    pub fn register_mod(&self, mod_obj: Box<dyn Mod>) {
        let mut mods = self.mods.lock().unwrap();
        mods.push(ModEntry {
            inner: mod_obj,
            enabled: false,
        });
    }

    /// Unregister a mod by name
    ///
    /// ~name: The name of the mod to remove (as returned by `Mod::name()`)
    ///
    /// Returns true if a mod with the given name was found and removed.
    /// Note: `on_disable` is intentionally *not* called here; disable the
    /// mod first if it needs teardown.
    pub fn unregister_mod(&self, name: &str) -> bool {
        let mut mods = self.mods.lock().unwrap();
        let len_before = mods.len();

        mods.retain(|entry| entry.inner.name() != name);

        mods.len() != len_before
    }

    /// Snapshot of `(name, enabled)` for every registered mod.
    ///
    /// Returns owned data (no lock held) so status UIs and external
    /// tooling can read it without borrow issues.
    pub fn mod_list(&self) -> Vec<(String, bool)> {
        self.mods
            .lock()
            .unwrap()
            .iter()
            .map(|entry| (entry.inner.name().to_string(), entry.enabled))
            .collect()
    }

    /// Call the update method on all *enabled* mods
    ///
    /// ~delta_time: Time in seconds since last update
    ///
    /// This should be called regularly (e.g., every frame) to drive mod logic
    pub fn update_all(&self, delta_time: f32) {
        let mut mods = self.mods.lock().unwrap();
        for entry in mods.iter_mut().filter(|e| e.enabled) {
            entry.inner.on_update(delta_time);
        }
    }

    /// Call the draw UI method on all *enabled* mods
    ///
    /// ~ctx: The egui context for this frame
    ///
    /// This should be called once per frame after egui has begun the frame
    pub fn draw_ui_all(&self, ctx: &egui::Context) {
        let mut mods = self.mods.lock().unwrap();
        for entry in mods.iter_mut().filter(|e| e.enabled) {
            entry.inner.on_draw_ui(ctx);
        }
    }

    /// Enable a mod by name
    ///
    /// ~name: The name of the mod to enable
    ///
    /// Runs `on_enable` only on the off→on transition. Returns true if a
    /// mod with that name exists (even if it was already enabled).
    pub fn enable_mod(&self, name: &str) -> bool {
        let mut mods = self.mods.lock().unwrap();
        match mods.iter_mut().find(|e| e.inner.name() == name) {
            Some(entry) => {
                if !entry.enabled {
                    entry.inner.on_enable();
                    entry.enabled = true;
                }
                true
            }
            None => false,
        }
    }

    /// Disable a mod by name
    ///
    /// ~name: The name of the mod to disable
    ///
    /// Runs `on_disable` only on the on→off transition, so a disabled mod
    /// immediately stops receiving `on_update` / `on_draw_ui`. Returns true
    /// if a mod with that name exists (even if already disabled).
    pub fn disable_mod(&self, name: &str) -> bool {
        let mut mods = self.mods.lock().unwrap();
        match mods.iter_mut().find(|e| e.inner.name() == name) {
            Some(entry) => {
                if entry.enabled {
                    entry.inner.on_disable();
                    entry.enabled = false;
                }
                true
            }
            None => false,
        }
    }

    /// Framework status window: proves the framework loaded correctly.
    ///
    /// Shows version, platform, init state, uptime and mod counts, plus
    /// the path of the log file. Render this from the game's render hook;
    /// until the graphics hook exists it is exercised headless at startup
    /// (see `headless_ui_check` in `lib.rs`).
    pub fn draw_status_ui(&self, ctx: &egui::Context) {
        // Framework-level visibility toggle (O by default, see `hotkey`).
        if !crate::hotkey::ui_visible() {
            return;
        }
        let uptime = self
            .loaded_at
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let total = self.mod_count();
        let enabled = self.enabled_count();

        egui::Window::new("CXSFM — Framework Status")
            .collapsible(true)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Version: {}", env!("CARGO_PKG_VERSION")));
                ui.label(format!("Platform: {}", std::env::consts::OS));
                ui.label(format!(
                    "Initialized: {}",
                    if crate::is_initialized() { "yes" } else { "no" }
                ));
                ui.label(format!("Uptime: {} s", uptime));
                ui.label(format!("Mods: {} enabled / {} total", enabled, total));
                ui.label(format!("Log file: {}", crate::log_path().display()));
            });
    }

    /// Mod manager window: checkbox list to enable/disable mods live.
    ///
    /// Toggling a checkbox immediately calls the mod's `on_enable` /
    /// `on_disable` hook and flips its state, so `update_all` /
    /// `draw_ui_all` pick it up on the next tick.
    pub fn draw_manager_ui(&self, ctx: &egui::Context) {
        // Framework-level visibility toggle (O by default, see `hotkey`).
        if !crate::hotkey::ui_visible() {
            return;
        }
        let mut mods = self.mods.lock().unwrap();

        egui::Window::new("CXSFM — Mods")
            .collapsible(true)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Loaded modules");
                ui.separator();

                if mods.is_empty() {
                    ui.label("No mods registered.");
                }

                for entry in mods.iter_mut() {
                    // `name()` returns &'static str: no borrow conflict
                    // with the mutable entries iterator.
                    let mut on = entry.enabled;
                    if ui.checkbox(&mut on, entry.inner.name()).changed()
                        && on != entry.enabled
                    {
                        if on {
                            entry.inner.on_enable();
                        } else {
                            entry.inner.on_disable();
                        }
                        entry.enabled = on;
                    }
                }
            });
    }

    /// Check if the manager has been initialized
    #[inline]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Query a mod's on/off state by name.
    ///
    /// Returns `Some(true/false)` when a mod with that name exists,
    /// `None` otherwise. Used by hotkey handling to flip mods.
    #[inline]
    pub fn is_mod_enabled(&self, name: &str) -> Option<bool> {
        self.mods
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.inner.name() == name)
            .map(|e| e.enabled)
    }

    /// Mark the manager as initialized
    #[inline]
    pub fn set_initialized(&mut self) {
        self.initialized = true;
    }

    /// Get the number of registered mods
    #[inline]
    pub fn mod_count(&self) -> usize {
        self.mods.lock().unwrap().len()
    }

    /// Get the number of currently enabled mods
    #[inline]
    pub fn enabled_count(&self) -> usize {
        self.mods
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.enabled)
            .count()
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
///
/// Provides access to the singleton mod manager instance
#[inline]
pub fn get_mod_manager() -> &'static ModManager {
    &MOD_MANAGER
}

/// Convenience function to register a mod with the global manager
///
/// ~mod: Boxed trait object implementing the `Mod` trait
///
/// Equivalent to calling `get_mod_manager().register_mod(mod_obj)`
#[inline]
pub fn register_mod(mod_obj: Box<dyn Mod>) {
    get_mod_manager().register_mod(mod_obj)
}

/// Convenience function to update all registered mods
///
/// ~delta_time: Time in seconds since last update
///
/// Equivalent to calling `get_mod_manager().update_all(delta_time)`
#[inline]
pub fn update_all_mods(delta_time: f32) {
    get_mod_manager().update_all(delta_time)
}

/// Convenience function to draw UI for all registered mods
///
/// ~ctx: The egui context for this frame
///
/// Equivalent to calling `get_mod_manager().draw_ui_all(ctx)`
#[inline]
pub fn draw_ui_all(ctx: &egui::Context) {
    get_mod_manager().draw_ui_all(ctx)
}

/// Convenience function to draw the framework status window
///
/// ~ctx: The egui context for this frame
///
/// Equivalent to calling `get_mod_manager().draw_status_ui(ctx)`
#[inline]
pub fn draw_status_ui(ctx: &egui::Context) {
    get_mod_manager().draw_status_ui(ctx)
}

/// Convenience function to draw the mod manager window
///
/// ~ctx: The egui context for this frame
///
/// Equivalent to calling `get_mod_manager().draw_manager_ui(ctx)`
#[inline]
pub fn draw_manager_ui(ctx: &egui::Context) {
    get_mod_manager().draw_manager_ui(ctx)
}
