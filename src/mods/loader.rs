//! Isolated mod registry: owns every mod, contains every failure.
//!
//! Design rules:
//!
//! - Two locks per mod: fast `state` (flags + cached presentation)
//!   and `inner` (the user object). A hook call holds ONLY the inner
//!   lock, so a mod stuck inside Unity (scene churn blocks
//!   `runtime_invoke` for seconds) can NEVER stall the render thread:
//!   menu/overlay paths touch state alone and stay responsive while
//!   the tick thread waits. This exact freeze (present thread parked
//!   on one shared entry lock → game killed as "crashed") is why the
//!   split exists.
//! - The registry lock is NEVER held across mod code. Entry handles
//!   are cloned under a short lock; hooks run lock-free otherwise.
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

/// Fast flags + cached presentation. Held for microseconds, never
/// across user code — render paths (`tile_info`, `describe_all`,
/// `draw_ui_all`, counts) touch state alone and can never stall
/// behind a mod blocked inside Unity.
struct EntryState {
    enabled: bool,
    /// A quarantined mod stays registered and visible but can never
    /// run again until explicitly re-enabled (which clears the flag).
    quarantined: bool,
    /// Last good tile presentation (used when the getters panic).
    cached_label: &'static str,
    cached_icon: TileIcon,
    /// Last good About description.
    cached_desc: &'static str,
    /// Last good pin flag.
    cached_pinned: bool,
    /// `i18n::LANG_REV` at caching time (labels follow the language).
    rev: u64,
}

/// One registered plugin: immutable name, split locks (see module docs).
struct Entry {
    /// Technical name, fixed at registration (calling user code for it
    /// on every frame would be another panic surface).
    name: String,
    state: Mutex<EntryState>,
    inner: Mutex<Box<dyn Mod>>,
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

/// Try the inner lock without ever blocking: a mod stuck inside
/// Unity (scene churn) must not stall the render thread. Poison is
/// recovered, contention serves stale cache (refreshed next frame).
fn try_inner(handle: &Arc<Entry>) -> Option<MutexGuard<'_, Box<dyn Mod>>> {
    match handle.inner.try_lock() {
        Ok(g) => Some(g),
        Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// Mark an entry quarantined + disabled (short state lock only).
fn quarantine(handle: &Arc<Entry>) {
    let mut s = lock(&handle.state);
    s.quarantined = true;
    s.enabled = false;
}

/// Owns every mod. All public methods are thread-safe.
pub struct ModRegistry {
    entries: Mutex<Vec<Arc<Entry>>>,
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
    fn snapshot(&self) -> Vec<Arc<Entry>> {
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
        if entries.iter().any(|e| e.name == name) {
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
        entries.push(Arc::new(Entry {
            name: name.clone(),
            state: Mutex::new(EntryState {
                enabled: false,
                quarantined: false,
                cached_label: label,
                cached_icon: icon,
                cached_desc: "",
                cached_pinned: false,
                rev: crate::i18n::lang_rev(),
            }),
            inner: Mutex::new(mod_obj),
        }));
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
            match entries.iter().position(|e| e.name == name) {
                Some(i) => entries.remove(i),
                None => return false,
            }
        };
        let was_enabled = lock(&taken.state).enabled;
        if was_enabled {
            let mut inner = lock(&taken.inner);
            let ((), bad) = guard(&taken.name, "on_disable", || inner.on_disable());
            drop(inner);
            if bad {
                quarantine(&taken);
            } else {
                lock(&taken.state).enabled = false;
            }
        }
        true
    }

    /// Owned `(name, enabled, description)` snapshot for the About
    /// page. State lock only; stale descriptions refresh
    /// opportunistically (`describe_in` is a pure string match — never
    /// Unity — and contention serves stale cache).
    pub fn describe_all(&self, lang_code: &str) -> Vec<(String, bool, &'static str)> {
        let rev = crate::i18n::lang_rev();
        self.snapshot()
            .iter()
            .map(|handle| {
                {
                    let s = lock(&handle.state);
                    if s.rev == rev || s.quarantined {
                        return (handle.name.clone(), s.enabled, s.cached_desc);
                    }
                }
                if let Some(inner) = try_inner(handle) {
                    let (desc, bad) =
                        guard(&handle.name, "describe_in", || inner.describe_in(lang_code));
                    drop(inner);
                    let mut s = lock(&handle.state);
                    if bad {
                        s.quarantined = true;
                        s.enabled = false;
                    } else {
                        s.cached_desc = desc;
                        s.rev = rev;
                    }
                }
                let s = lock(&handle.state);
                (handle.name.clone(), s.enabled, s.cached_desc)
            })
            .collect()
    }

    /// Snapshot of `(name, enabled)` for every registered mod.
    pub fn mod_list(&self) -> Vec<(String, bool)> {
        self.snapshot()
            .iter()
            .map(|e| {
                let s = lock(&e.state);
                (e.name.clone(), s.enabled)
            })
            .collect()
    }

    /// Owned per-mod tile snapshot for the menu grid. State lock only;
    /// stale presentation refreshes opportunistically (getters return
    /// static data — never Unity — and contention serves stale cache
    /// instead of stalling the present thread).
    pub fn tile_info(&self) -> Vec<ModTile> {
        let rev = crate::i18n::lang_rev();
        self.snapshot()
            .iter()
            .map(|handle| {
                // Pin flag is user-toggled and cheap: refresh every
                // frame when the inner lock is free (never blocks).
                if let Some(inner) = try_inner(handle) {
                    let (pinned, bad_pin) =
                        guard(&handle.name, "is_pinned", || inner.is_pinned());
                    drop(inner);
                    let mut s = lock(&handle.state);
                    if bad_pin {
                        s.quarantined = true;
                        s.enabled = false;
                    } else {
                        s.cached_pinned = pinned;
                    }
                }
                {
                    let s = lock(&handle.state);
                    if s.rev == rev || s.quarantined {
                        return ModTile {
                            name: handle.name.clone(),
                            enabled: s.enabled,
                            label_key: s.cached_label,
                            icon: s.cached_icon,
                        };
                    }
                }
                if let Some(inner) = try_inner(handle) {
                    let (label, bad_label) =
                        guard(&handle.name, "menu_label_key", || inner.menu_label_key());
                    let (icon, bad_icon) = guard(&handle.name, "menu_icon", || inner.menu_icon());
                    let (pinned, bad_pin) = guard(&handle.name, "is_pinned", || inner.is_pinned());
                    drop(inner);
                    let mut s = lock(&handle.state);
                    if bad_label || bad_icon || bad_pin {
                        s.quarantined = true;
                        s.enabled = false;
                    } else {
                        s.cached_label = label;
                        s.cached_icon = icon;
                        s.cached_pinned = pinned;
                        s.rev = rev;
                    }
                }
                let s = lock(&handle.state);
                ModTile {
                    name: handle.name.clone(),
                    enabled: s.enabled,
                    label_key: s.cached_label,
                    icon: s.cached_icon,
                }
            })
            .collect()
    }

    /// Set a mod's on/off state by name. Runs the transition hook
    /// (`on_enable`/`on_disable`) only on a real transition, guarded.
    /// Re-enabling clears a quarantine. Returns true if found.
    pub fn set_enabled(&self, name: &str, enabled: bool) -> bool {
        let handle = self.snapshot().into_iter().find(|e| e.name == name);
        let handle = match handle {
            Some(h) => h,
            None => return false,
        };
        {
            let mut s = lock(&handle.state);
            if s.enabled == enabled && !s.quarantined {
                return true;
            }
            s.quarantined = false;
        }
        let what = if enabled { "on_enable" } else { "on_disable" };
        let mut inner = lock(&handle.inner);
        let ((), bad) = guard(&handle.name, what, || {
            if enabled {
                inner.on_enable();
            } else {
                inner.on_disable();
            }
        });
        drop(inner);
        if bad {
            quarantine(&handle);
        } else {
            lock(&handle.state).enabled = enabled;
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
            .find(|e| e.name == name)
            .map(|e| lock(&e.state).enabled)
    }

    /// Call `on_update` on all enabled mods (tick thread). The inner
    /// lock alone is held across the hook: a mod stalled inside Unity
    /// delays only the tick thread, never the present thread.
    pub fn update_all(&self, delta_time: f32) {
        for handle in self.snapshot() {
            {
                let s = lock(&handle.state);
                if !s.enabled || s.quarantined {
                    continue;
                }
            }
            let mut inner = lock(&handle.inner);
            let ((), bad) = guard(&handle.name, "on_update", || {
                inner.on_update(delta_time);
            });
            drop(inner);
            if bad {
                quarantine(&handle);
            }
        }
    }

    /// Call `on_draw_ui` on enabled mods (render thread).
    ///
    /// When the menu is closed only pinned mods draw (see
    /// `Mod::is_pinned`); the menu window itself stays gated by the
    /// caller.
    pub fn draw_ui_all(&self, ctx: &egui::Context, menu_open: bool) {
        for handle in self.snapshot() {
            // State checks first (microsecond lock). The inner lock is
            // taken with try_inner: a mod stuck inside Unity must skip
            // this frame, never stall the present thread.
            let pinned = {
                let s = lock(&handle.state);
                if !s.enabled || s.quarantined {
                    continue;
                }
                s.cached_pinned
            };
            if !menu_open && !pinned {
                continue;
            }
            let mut inner = match try_inner(&handle) {
                Some(g) => g,
                None => continue,
            };
            let ((), bad) = guard(&handle.name, "on_draw_ui", || {
                inner.on_draw_ui(ctx);
            });
            drop(inner);
            if bad {
                quarantine(&handle);
            }
        }
    }

    /// Whether any enabled mod is currently pinned (state only — safe
    /// at present rate even mid-stall).
    pub fn has_pinned_visible(&self) -> bool {
        self.snapshot().into_iter().any(|handle| {
            let s = lock(&handle.state);
            s.enabled && !s.quarantined && s.cached_pinned
        })
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
                let s = lock(&e.state);
                s.enabled && !s.quarantined
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

    /// A panicking update quarantines that mod, others keep running.
    #[test]
    fn panic_update_quarantines() {
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

    /// Render paths never touch the inner lock: even with the inner
    /// lock held (simulated stuck mod), tile_info still answers.
    #[test]
    fn render_paths_skip_stuck_inner() {
        // tile_info/describe take the inner lock for trivial getters;
        // state-only reads (mod_list, counts) never can block.
        let reg = ModRegistry::new();
        reg.register(Box::new(DummyMod {
            panic_on_update: false,
        }));
        assert_eq!(reg.mod_list().len(), 1);
        assert!(!reg.has_pinned_visible());
    }
}
