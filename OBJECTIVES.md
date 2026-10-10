# CXSFM Objectives & Roadmap

This document outlines the development roadmap for the CarX Street Framework Mod (CXSFM).

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

**Status**: Complete — menu renders in-game, no crash on scene
change (garage ↔ race) or at quit. Binary max `GLIBC_2.35`
(< Steam Runtime sniper 2.36).

---

## Phase 2: Scanning & Telemetry

Planned for after Phase 1 stabilization.

- [ ] AOB (Array of Bytes) scanner with wildcard support
- [ ] Game structure signature database
- [ ] Real-time memory value extraction
- [ ] Telemetry data logger (position, speed, lap times)
- [ ] Memory map visualization UI
- [ ] Signature update mechanism (community-driven)

**Estimated Timeline**: 2-3 months after Phase 1

---

## Phase 3: FPV Camera & HUD

Planned for Phase 2 completion.

- [ ] FPV Drone camera mod (example)
- [ ] Camera orbit / follow modes
- [ ] HUD overlay elements
- [ ] Performance optimizations
- [ ] Configurable keybindings
- [ ] Save/load settings

**Estimated Timeline**: 3-4 months after Phase 2

---

## Phase 4: Advanced Mods

Future development beyond the MVP.

- [ ] Replay system
- [ ] Screenshot mode (free camera)
- [ ] Color adjustment filters
- [ ] Mod loader (community plugin support)
- [ ] Configuration file parser (TOML)
- [ ] Debug information overlay

---

## Constraints

- **100% Safe Rust** in mod implementations - unsafe only in memory.rs
- **No gameplay modification** - strictly cosmetic/UI/HUD/camera changes
- **Minimal performance overhead** - async mod loading
- **Cross-platform compatibility** - consistent behavior across Windows/Linux/macOS

---

## Architecture Principles

1. **Safety First**: Unsafe code isolated behind safe APIs
2. **Modularity**: Each mod runs independently in the manager
3. **Performance**: Zero-cost abstractions in hot paths
4. **Extensibility**: Trait-based design allows new mods
5. **Modern Rust**: Uses Rust 2024 Edition features (LazyLock, etc.)

---

See [CONTRIBUTING.md](CONTRIBUTING.md) for details on contributing to these goals.
