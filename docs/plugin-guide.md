# Plugin guide — writing a CXSFM mod

A mod = **one folder** under `src/mods/`, compiled into the framework
`.so` (no external file injected). Examples to copy:

| Folder | Demonstrated operation |
|---|---|
| `inspector/` | tool: scene tree (roots + hierarchy + position, throttled snapshot) |
| `speedo/` | HUD restyle: edits the game's speedometer objects in realtime (Restore button) |
| `map/` | same pattern for the minimap (shares `speedo::model`) |
| `gamelog/` | tool: the game's live `Player.log` (filter + errors, read-only) |
| `hud/` | pure egui overlay: HUDs described in `hud.toml` (hot-reloaded), live values read from game fields |
| `fpv_drone/` | free FPV drone camera on the gamepad (one-shot camera takeover, restore on disable) |
| `engine_audio/` | FMOD Studio audio: pitch/volume multipliers on the engine events (C API via `dlsym`, restore on disable) |

## 1. Copy a folder

```bash
cp -r src/mods/inspector src/mods/my_mod
```

Uniform layout (every mod follows it):

| File | Contents |
|---|---|
| `mod.rs` | struct + `new` + `impl Mod` (delegates to `bind`/`ui`) + `register()` |
| `model.rs` | data + pure helpers (+ tests) — no Unity call |
| `bind.rs` | tick-side Unity (`ensure`, walk, drain, restore) |
| `ui.rs` | present-side window (`draw_window`, staging) |
| `i18n.rs` | 7 languages, English fallback |

## 2. Implement the `Mod` trait (`mod.rs`)

```rust
use crate::mods::api::{Mod, TileIcon};

pub struct MyMod { /* atomics + Mutex, never a shared &mut */ }

impl Mod for MyMod {
    fn name(&self) -> &'static str { "My Mod" }        // unique!
    fn menu_label_key(&self) -> &'static str { "mod_my" } // stable key
    fn menu_icon(&self) -> TileIcon { TileIcon::Tag }
    fn describe_in(&self, lang: &str) -> &'static str {
        self::i18n::describe(lang)                     // About page
    }
    fn on_update(&mut self, dt: f32) { /* ~60 Hz, fast, never blocking */ }
    fn on_enable(&mut self) { /* arm: drop handles, pending = true */ }
    fn on_disable(&mut self) { /* RESTORE the exact game state */ }
}
```

Unity rules (proven in game, do not improvise):

- calling thread already attached (tick + render); `domain_checked`
  before use;
- classes/methods cached (`UnityCache`), **objects never cached** —
  handles as `usize`, re-resolved on every use, validated by a read;
- cadences: search ~0.2 Hz without a handle, check ~1 Hz with one,
  `pending` to act on the very next tick after arming;
- **never a blind write**: capture the original → write → read back →
  restore on `on_disable`;
- locks: bind the snapshot (`let cached = ...`) before the `match`
  (self-deadlock otherwise).

## 3. Mod strings (`i18n.rs`, 7 languages, English fallback)

```rust
pub fn tile_label(lang_code: &str) -> &'static str { match lang_code {
    "fr" => "Mon Mod", /* de es it pt ru */ _ => "My Mod",
}}
pub fn describe(lang_code: &str) -> &'static str { /* same */ }
```

Then **one line** in `src/i18n/mod.rs::mod_tile`:

```rust
"mod_my" => Some(crate::mods::my_mod::i18n::tile_label(code)),
```

The menu only knows the stable key; every displayed letter comes from
the mod's folder. Framework files (`en.rs`, `fr.rs`, …) are never
touched for a mod.

## 4. Register (2 lines)

`src/mods.rs`: `pub mod my_mod;`
`src/lib.rs` (registrations): `mods::my_mod::register();`

```rust
pub fn register() {
    crate::mod_api::register_mod(Box::new(MyMod::new())); // boots disarmed
}
```

## 5. Isolation — what is guaranteed

Every hook runs under `catch_unwind`: a panic → the mod is quarantined
(auto-disabled, logged), menu + other mods stay alive. The registry
lock is never held while mod code runs; poisoned mutexes are
recovered. A mod cannot spoof the menu (egui context not shared with
the menu window).
