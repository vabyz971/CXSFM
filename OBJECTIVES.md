# CXSFM Objectives & Roadmap

This document outlines the development roadmap for the CarX Street
Framework Mod (CXSFM). Current release: **v0.7.0**. "Proven" means
validated in game on Linux (implicit Vulkan layer); see
[docs/field-notes.md](docs/field-notes.md) for the evidence log.

## Phase 1: Hooking & UI (Done, proven in-game on Linux)

- [x] Basic framework structure
- [x] Cross-platform shared library compilation (.dll / .so / .dylib)
- [x] Memory module with OS abstraction
- [x] Egui integration (0.36 + egui-ash-renderer 0.13)
- [x] Mod trait and manager (panic quarantine, lock-free snapshots)
- [x] Vulkan hooking layer (implicit layer, present routing)
- [x] Egui renderer integration with game's graphics pipeline
- [x] Input event forwarding (SDL + X11 capture, F8 toggle)
- [x] Stability pass: stale-handle re-resolve, teardown guards
- [x] Overlay can no longer freeze the game's present (dead fence →
      overlay paused) nor replay stale draw ranges after a page change
- [x] Threads detached from IL2CPP before the runtime shuts down

**Status**: Complete — menu renders in-game, no crash on scene
change (garage ↔ race). Binary max `GLIBC_2.35` (< Steam Runtime
sniper 2.36). Quit remains fragile on the game side (vanilla CarX
SIGSEGVs in its exit handlers); `tools/cxsfm-hang-dump.sh` covers hangs.

---

## Phase 2: Menu, Input & Settings (Done, proven in-game)

- [x] Phone-style main menu (per-mod tiles, version + description on
      hover, close button, Settings / About / Quit tiles)
- [x] CarX theme + game fonts (Bebas Neue, Rajdhani — SIL OFL),
      Material Symbols icons
- [x] Full-screen game-style mod screens (tabs, rows, help panel,
      ESC BACK / POWER)
- [x] Gamepad navigation ("mod mode", L3 + R3), pad read from evdev on
      its own thread
- [x] Window chrome: pin (visible with the menu closed), opacity
- [x] Save/load settings in hand-editable TOML (`~/.config/cxsfm/`:
      `settings.toml`, `saves.toml`, `fpv_drone.toml`, `custom_hud.toml`)
- [x] Auto-load of speedometer/map saves on arrival in game
      (Settings opt-in, `Mod::on_background_update`)
- [x] Configurable keybindings (FPV Drone rebindable controls)
- [x] Per-mod i18n translation files

---

## Phase 3: Bundled Mods

| Mod | Version | State |
|---|---|---|
| Scene Inspector | 1.1.0 | [x] scene tree proven; [ ] components, field values, reference navigation and edits (1.1.0) to validate in game |
| Game Log | 1.0.0 | [x] `Player.log` viewer |
| Speedometer | 0.5.4 | [x] HUD restyle, edits kept on disable, Restore button, auto-applied saves |
| Minimap | 0.2.0 | [x] same pattern as the speedometer |
| Custom HUD | 0.4.1 | [x] bars, arcs, needles, ticks, italic/glow text, live values through field paths (~16 min of driving validated); [ ] text sources + full `label` widget (0.3.0) to validate in game; [x] on-screen editor stable in game (0.4.0); [ ] shapes, justify, "use as variable", named widgets + search (0.4.1) to validate in game |
| Engine Sound | 0.2.0 | [x] FMOD engine-event pitch/volume (cosmetic audio), full-screen settings |
| FPV Drone | 0.6.0 | [x] free quadcopter camera on the pad (physics, collisions, rebindable controls, four-rotor sound); [ ] rigid-body flight model, airframes, rebuilt sound (0.6.0) to validate in game |
| FPV Camera | — | [ ] parked until per-frame application is possible (`mainthread.rs` phase 2) |

---

## Phase 4: Telemetry & Scanning

- [x] IL2CPP reflection (class search, field dumps, raw field reads,
      bounded `read_string`) — replaces signatures for most needs
- [x] Real-time value extraction (Custom HUD live reader: speed, RPM,
      gear via field paths)
- [~] AOB scanner with wildcard support — implemented in
      `cxsfm-sys` (`memory::AobScanner`), not used by any mod yet
- [ ] Telemetry data logger (position, speed, lap times) to a file
- [ ] Game structure signature database
- [ ] Signature update mechanism (community-driven)
- [ ] Memory map visualization UI

---

## Phase 5: Framework & Advanced Mods

- [x] Cargo workspace (`crates/`: api, sys, input, ui, mods, cxsfm)
- [ ] Safe mods: move the remaining `unsafe` of the mods (bind.rs,
      inspector/detail.rs, engine_audio/fmod.rs…) behind safe
      `cxsfm-sys` APIs, then `unsafe_code = "forbid"` in `cxsfm-mods`
      — one mod at a time, each re-validated in game
- [ ] Main-thread execution (`mainthread.rs`: `methodPointer` swap —
      phase 1 probes done, no mod migrated) — unblocks FPV Camera
- [ ] Camera orbit / follow modes
- [ ] Screenshot mode (free camera — partly covered by FPV Drone)
- [ ] Color adjustment filters
- [ ] Replay system
- [ ] Mod loader for external community plugins (see
      [docs/plugin-guide.md](docs/plugin-guide.md))
- [ ] Debug information overlay (FPS, frame time, framework state)
- [ ] Windows (DXGI) and macOS (Metal) paths validated in game
      (scaffolds only today)

---

## Constraints

- **Safe Rust in mods (goal)** - `unsafe` belongs in the FFI crates
  (`cxsfm-sys`, `cxsfm-input`, the `cxsfm` render path) behind safe
  APIs with documented `SAFETY` contracts; `cxsfm-ui` already forbids
  it, the mods still carry some (see Phase 5)
- **No gameplay modification** - strictly cosmetic/UI/HUD/camera/audio
  changes; never economy, physics or network
- **Minimal performance overhead** - nothing heavy on the present
  thread, no Unity call from it
- **Cross-platform layout** - only Linux is proven in game today

---

## Architecture Principles

1. **Safety First**: Unsafe code isolated behind safe APIs
2. **Modularity**: Each mod runs independently in the manager
3. **Performance**: Zero-cost abstractions in hot paths
4. **Extensibility**: Trait-based design allows new mods
5. **Modern Rust**: Uses Rust 2024 Edition features (LazyLock, etc.)

---

See [CONTRIBUTING.md](CONTRIBUTING.md) for details on contributing to these goals.
