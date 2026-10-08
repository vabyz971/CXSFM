//! Isolated mod registry: owns every mod, contains every failure.
//!
//! Design rules:
//!
//! - The registry lock is NEVER held across mod code. Entry handles
//!   are cloned under a short lock; user hooks run with only the
//!   entry lock held (or none at all for pure reads).
//! - Every hook (`on_update`, `on_draw_ui`, `on_enable`,
//!   `on_disable`, `menu_label_key`, `menu_icon`) runs inside
//!   `catch_unwind`. A panic quarantines that mod (auto-disabled,
//!   logged) — the menu and the other mods keep running.
//! - Poisoned mutexes (a panic while locked) are recovered via
//!   `into_inner`, never `unwrap`-crashed.
//! - `unregister` runs `on_disable` first when the mod was enabled,
//!   so removal always restores game state.

use super::api::{Mod, ModTile, TileIcon};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

/// Lock a mutex, recovering from poisoning (a previous hook panic).
///
/// Poison means a prior holder panicked mid-critical-section; the data
/// itself is a plain struct, so resuming with it is safe here.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One registered plugin plus its runtime state.
struct Entry {
    /// Technical name, snapshotted at registration (calling user code
    /// for it on every frame would be another panic surface).
    name: String,
    inner: Box<dyn Mod>,
    enabled: bool,
    /// A quarantined mod stays registered and visible but can never
    /// run again until explicitly re-enabled (which clears the flag).
    quarantined: bool,
    /// Last good tile presentation (used when the getters panic).
    cached_label: &'static str,
    cached_icon: TileIcon,
}

/// Runs `f`, catching a mod panic. Returns the value (or `Default`)
/// plus whether the mod must be quarantined.
fn guard<R: Default>(mod_name: &str, what: &str, f: impl FnOnce() -> R) -> (R, bool) {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => (r, false),
        Err(_) => {
            crate::log_line(&format!(
                "mods: '{mod_name}' panicked in {what} — quarantined (auto-disabled)"
            ));
            (R::default(), true)
        }
    }
}

/// Owns every mod. All public methods are thread-safe.
pub struct ModRegistry {
    entries: Mutex<Vec<Arc<Mutex<Entry>>>>,
    loaded_at: SystemTime,
}

impl ModRegistry {
    /// Empty registry, creation time noted for the About page.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            loaded_at: SystemTime::now(),
        }
    }

    /// Short-lived clones of the entry handles (registry lock dropped
    /// before any mod code runs).
    fn snapshot(&self) -> Vec<Arc<Mutex<Entry>>> {
        lock(&self.entries).clone()
    }

    /// Register a new mod (starts disabled: the framework boots quiet
    /// and the user opts in).
    ///
    /// Duplicate names are rejected (logged) — names address mods for
    /// toggling, so they must be unique.
    pub fn register(&self, mod_obj: Box<dyn Mod>) {
        // `guard` returns (value, must_quarantine): a clean call yields
        // `bad == false` with the real name.
        let (name, bad) = guard("?", "name", || mod_obj.name().to_string());
        if bad || name.is_empty() {
            crate::log_line("mods: refused registration (name panicked or empty)");
            return;
        }
        let mut entries = lock(&self.entries);
        if entries.iter().any(|e| lock(e).name == name) {
            crate::log_line(&format!("mods: duplicate '{name}' refused"));
            return;
        }
        // NOTE: `menu_label_key`/`menu_icon` defaults cannot panic, but
        // an override could — validate now, cache the good values.
        let (label, label_bad) = guard(&name, "menu_label_key", || mod_obj.menu_label_key());
        let (icon, icon_bad) = guard(&name, "menu_icon", || mod_obj.menu_icon());
        if label_bad || icon_bad {
            crate::log_line(&format!("mods: '{name}' panicked during registration"));
            return;
        }
        entries.push(Arc::new(Mutex::new(Entry {
            name: name.clone(),
            inner: mod_obj,
            enabled: false,
            quarantined: false,
            cached_label: label,
            cached_icon: icon,
        })));
        crate::log_line(&format!(
            "mods: registered '{name}' ({} total)",
            entries.len()
        ));
    }

    /// Unregister by name. Runs `on_disable` first when enabled, so
    /// removal always restores game state. Returns true if found.
    pub fn unregister(&self, name: &str) -> bool {
        let taken = {
            let mut entries = lock(&self.entries);
            match entries.iter().position(|e| lock(e).name == name) {
                Some(i) => entries.remove(i),
                None => return false,
            }
        };
        let mut e = lock(&taken);
        if e.enabled {
            let name = e.name.clone();
            let ((), bad) = guard(&name, "on_disable", || e.inner.on_disable());
            let _ = bad;
            let _ = bad;
            e.enabled = false;
        }
        true
    }

    /// Owned `(name, enabled, description)` snapshot for the About
    /// page. Descriptions come from each mod's own `i18n` module via
    /// `describe_in` (guarded like every other hook).
    pub fn describe_all(&self, lang_code: &str) -> Vec<(String, bool, &'static str)> {
        self.snapshot()
            .iter()
            .map(|handle| {
                let mut e = lock(handle);
                let name = e.name.clone();
                let (desc, bad) = guard(&name, "describe_in", || e.inner.describe_in(lang_code));
                if bad {
                    e.quarantined = true;
                    e.enabled = false;
                }
                (name, e.enabled, desc)
            })
            .collect()
    }

    /// Snapshot of `(name, enabled)` for every registered mod.
    pub fn mod_list(&self) -> Vec<(String, bool)> {
        self.snapshot()
            .iter()
            .map(|e| {
                let e = lock(e);
                (e.name.clone(), e.enabled)
            })
            .collect()
    }

    /// Owned per-mod tile snapshot for the menu grid (draws lock-free).
    pub fn tile_info(&self) -> Vec<ModTile> {
        self.snapshot()
            .iter()
            .map(|handle| {
                let mut e = lock(handle);
                if !e.quarantined {
                    let name = e.name.clone();
                    let (label, bad_label) =
                        guard(&name, "menu_label_key", || e.inner.menu_label_key());
                    let (icon, bad_icon) = guard(&name, "menu_icon", || e.inner.menu_icon());
                    if bad_label || bad_icon {
                        e.quarantined = true;
                        e.enabled = false;
                    } else {
                        e.cached_label = label;
                        e.cached_icon = icon;
                    }
                }
                ModTile {
                    name: e.name.clone(),
                    enabled: e.enabled,
                    label_key: e.cached_label,
                    icon: e.cached_icon,
                }
            })
            .collect()
    }

    /// Set a mod's on/off state by name. Runs the transition hook
    /// (`on_enable`/`on_disable`) only on a real transition, guarded.
    /// Re-enabling clears a quarantine. Returns true if found.
    pub fn set_enabled(&self, name: &str, enabled: bool) -> bool {
        let handle = self.snapshot().into_iter().find(|e| lock(e).name == name);
        let handle = match handle {
            Some(h) => h,
            None => return false,
        };
        let mut e = lock(&handle);
        if e.enabled == enabled && !e.quarantined {
            return true;
        }
        e.quarantined = false;
        let what = if enabled { "on_enable" } else { "on_disable" };
        let mod_name = e.name.clone();
        let ((), bad) = guard(&mod_name, what, || {
            if enabled {
                e.inner.on_enable();
            } else {
                e.inner.on_disable();
            }
        });
        if bad {
            e.quarantined = true;
            e.enabled = false;
        } else {
            e.enabled = enabled;
        }
        true
    }

    /// Enable a mod by name (transition hook only on off→on).
    pub fn enable(&self, name: &str) -> bool {
        self.set_enabled(name, true)
    }

    /// Disable a mod by name (transition hook only on on→off).
    pub fn disable(&self, name: &str) -> bool {
        self.set_enabled(name, false)
    }

    /// Query a mod's on/off state by name.
    pub fn is_enabled(&self, name: &str) -> Option<bool> {
        self.snapshot()
            .iter()
            .find(|e| lock(e).name == name)
            .map(|e| lock(e).enabled)
    }

    /// Call `on_update` on all enabled mods (tick thread).
    pub fn update_all(&self, delta_time: f32) {
        for handle in self.snapshot() {
            let mut e = lock(&handle);
            if !e.enabled || e.quarantined {
                continue;
            }
            let mod_name = e.name.clone();
            let ((), bad) = guard(&mod_name, "on_update", || {
                e.inner.on_update(delta_time);
            });
            if bad {
                e.quarantined = true;
                e.enabled = false;
            }
        }
    }

    /// Call `on_draw_ui` on all enabled mods (render thread).
    pub fn draw_ui_all(&self, ctx: &egui::Context) {
        for handle in self.snapshot() {
            let mut e = lock(&handle);
            if !e.enabled || e.quarantined {
                continue;
            }
            let mod_name = e.name.clone();
            let ((), bad) = guard(&mod_name, "on_draw_ui", || {
                e.inner.on_draw_ui(ctx);
            });
            if bad {
                e.quarantined = true;
                e.enabled = false;
            }
        }
    }

    /// Seconds since the registry was created (About page).
    pub fn uptime_secs(&self) -> u64 {
        self.loaded_at.elapsed().map(|d| d.as_secs()).unwrap_or(0)
    }

    /// Number of registered mods.
    pub fn mod_count(&self) -> usize {
        lock(&self.entries).len()
    }

    /// Number of currently enabled mods.
    pub fn enabled_count(&self) -> usize {
        lock(&self.entries)
            .iter()
            .filter(|e| {
                let e = lock(e);
                e.enabled && !e.quarantined
            })
            .count()
    }
}

impl Default for ModRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::api::Mod;
    use super::ModRegistry;

    struct DummyMod {
        panic_on_update: bool,
    }

    impl Mod for DummyMod {
        fn name(&self) -> &'static str {
            "Dummy"
        }

        fn on_update(&mut self, _dt: f32) {
            if self.panic_on_update {
                panic!("boom");
            }
        }
    }

    /// Registering a well-behaved mod must stick (regression: an
    /// inverted guard condition once refused every registration).
    #[test]
    fn register_keeps_healthy_mod() {
        let reg = ModRegistry::new();
        reg.register(Box::new(DummyMod {
            panic_on_update: false,
        }));
        assert_eq!(reg.mod_count(), 1);
        assert_eq!(reg.tile_info().len(), 1);
        assert!(reg.set_enabled("Dummy", true));
        assert_eq!(reg.enabled_count(), 1);
    }

    /// A panicking hook quarantines only that mod (auto-disabled,
    /// others unaffected).
    #[test]
    fn panic_quarantines_single_mod() {
        let reg = ModRegistry::new();
        reg.register(Box::new(DummyMod {
            panic_on_update: true,
        }));
        assert!(reg.set_enabled("Dummy", true));
        reg.update_all(0.016);
        assert_eq!(reg.enabled_count(), 0);
        assert_eq!(reg.mod_count(), 1);
    }

    /// Duplicate names are refused.
    #[test]
    fn duplicate_name_refused() {
        let reg = ModRegistry::new();
        reg.register(Box::new(DummyMod {
            panic_on_update: false,
        }));
        reg.register(Box::new(DummyMod {
            panic_on_update: false,
        }));
        assert_eq!(reg.mod_count(), 1);
    }
}
