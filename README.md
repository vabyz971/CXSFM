# CarX Street Framework Mod (CXSFM)

> Open-source modding framework for CarX Street - Built with Rust (Edition 2024)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust Edition](https://img.shields.io/badge/Rust-2024-orange.svg)](https://doc.rust-lang.org/edition-guide/editions/2024.html)
[![egui](https://img.shields.io/badge/egui-0.36-FF0000)](https://github.com/emilk/egui)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-blue)](https://github.com/CXSFM/cxsfm)
[![Version](https://img.shields.io/badge/version-0.4.0-informational)](Cargo.toml)

## What is CXSFM?

CXSFM is a **framework for creating in-game mods** for CarX Street. It provides:

- 🎨 **Cosmetic UI Overlays** - Inject egui-based interfaces into the game
- 📷 **Camera Manipulation** - Modify camera perspective (FPV, drone, etc.)
- 🛡️ **Visual-only by design** - Reads game state for display; never
  touches gameplay, economy, physics or the network
- 🔌 **Modular Architecture** - Easy-to-implement trait-based mod system
- 🚀 **Cross-Platform layout** - Builds as `.dll` / `.so` / `.dylib`;
  only **Linux is proven in-game** (Windows/macOS paths below are
  unvalidated scaffolds — see Injection Guide).

![CXSFM overlay in CarX Street: egui windows, software cursor, and the
HUD version tag applied to the game's own nickname label](docs/overlay-proof.png)

## Status (proven in-game, Linux)

- ✅ Implicit Vulkan layer negotiated, `vkQueuePresentKHR` routed
- ✅ egui 0.36 overlay rendered into swapchain images (draggable windows,
  checkboxes, software cursor, full text)
- ✅ X11 input capture (grab + event pump) + SDL — the UI is clickable
  (Unity `Input` path dropped: it segfaulted at teardown)
- ✅ IL2CPP bridge: camera FOV, UI text read/write, scene inspection,
  field reflection (class search, field dumps, raw field reads)
- ✅ Bundled mods: Scene Inspector (read-only), GameLog (Player.log),
  [Speedometer](src/mods/speedo/README.md) +
  [Minimap](src/mods/map/README.md) (HUD restyle; edits kept on
  disable, Restore button; saves auto-applied at launch),
  [Custom HUD](src/mods/hud/README.md) (HUDs from a hot-reloaded
  `hud.toml`: bars, arcs, needles, dial ticks, text with italic/glow;
  live game values through field paths found with its Explorer tab,
  ~16 min of driving validated without a crash)
- ✅ Stability: stale-handle re-resolve, `object_alive` readability
  guard, no crash on scene change (garage ↔ race); the overlay can no
  longer freeze the game's present (a dead fence pauses the overlay
  instead)
- ⚠️ Quit: CarX's own exit is fragile (vanilla SIGSEGVs in its exit
  handlers). CXSFM detaches its threads from IL2CPP before the runtime
  shuts down; if a quit still hangs, run `tools/cxsfm-hang-dump.sh`
  before killing the game
- ⏸️ FPV Camera mod (parked until per-frame application is possible)
- 🔑 Single hotkey: **F8** = UI visibility + input capture (mods
  persist independently, toggled from the manager window)

See [docs/field-notes.md](docs/field-notes.md) for the full verified
findings log (input paths, layer bring-up, incidents and fixes).

## Using CXSFM

- **F8** opens/closes the menu and captures input while it is open.
  Mods are tiles: enabling a tile arms the mod (all mods boot
  disarmed); windows can be pinned to stay visible with the menu
  closed.
- **Settings → auto-load**: speedo/map saves are re-applied a few
  seconds after arriving in game, even with their tiles closed.
- Files (outside the game install dir, hand-editable):

| File | Content |
|---|---|
| `~/.config/cxsfm/settings.toml` | framework settings (auto-load) |
| `~/.config/cxsfm/saves.toml` | speedo/map saved edits, by object path (`.bak` kept on save) |
| `~/.config/cxsfm/hud.toml` | Custom HUD layouts (hot-reloaded) — see [its README](src/mods/hud/README.md) |
| `~/.cache/cxsfm/cxsfm.log` | framework log (next to the staged layer; `CXSF_LOG` overrides) |

- Diagnostics: the log starts with `diag: cxsfm base=…` / `il2cpp
  base=…` (to attribute raw `Player.log` crash frames to a module);
  `tools/cxsfm-hang-dump.sh` snapshots every thread of a frozen game.

## Why CXSFM Respects CarX Street's CGU

We strictly adhere to CarX Street's Terms of Service by focusing **exclusively** on:

| Allowed (Cosmetic/UI) | Forbidden (Gameplay) |
|----------------------|---------------------|
| ✅ Camera manipulation | ❌ Money modification |
| ✅ HUD overlay | ❌ Speed modification |
| ✅ Visual effects | ❌ Inventory manipulation |
| ✅ FPV camera mod | ❌ Physics modification |
| ✅ UI elements | ❌ Game data alteration |

**No modifications are made to:**
- Player currency or economy
- Vehicle performance stats
- Game progression or unlocks
- Physics simulation
- Network protocol (anti-cheat evasion)

## Quick Start

### Prerequisites

- Nix with flakes enabled (`nix develop` provides the Rust 1.98
  toolchain — no system Rust/CMake/C++ needed, everything is pure Rust)
- Or plain Rust >= 1.95 (MSRV of egui 0.36) with network access for crates

### Building

```bash
# Nix (recommended, pinned toolchain)
nix develop --command cargo build --release

# Then stage the Vulkan layer (after every rebuild)
./tools/cxsfm-setup.sh

# Plain cargo
cargo build --release
```

Cross-compiling uses the standard target triples
(`--target x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`,
`x86_64-apple-darwin`).

### Output Files

After building, you'll find:
- `target/x86_64-pc-windows-msvc/release/cxsfm.dll` (Windows)
- `target/x86_64-unknown-linux-gnu/release/libcxsfm.so` (Linux)
- `target/x86_64-apple-darwin/release/libcxsfm.dylib` (macOS)

## Injection Guide

> Proven path: **Linux implicit Vulkan layer**. The Windows/macOS
> routes below are documented intent, **never validated in-game**
> (Windows target std isn't even installed in this dev env; DXGI is a
> scaffold, Metal is a stub).

### Windows (DLL Injection — unvalidated)

1. Use a DLL injector tool (e.g., Process Hacker, or manual `CreateRemoteThread`)
2. Target: CarXStreet.exe
3. Path: `cxsfm.dll`
4. The framework auto-initializes via `#[ctor]`

### Linux — implicit Vulkan layer (recommended)

The loader-sanctioned path (same mechanism as the Steam overlay): no
patching, no scanning, no ptrace. The loader discovers the framework via
a manifest and routes present calls through it.

```bash
# 1. Stage once (and after every rebuild)
./tools/cxsfm-setup.sh

# 2. Steam launch options (set once, kept by Steam)
VK_ADD_LAYER_PATH="$HOME/.cache/cxsfm" VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay %command%
```

Launch normally afterwards: the game loads the framework itself at
startup. Check `~/.cache/cxsfm/cxsfm.log` for
`render/layer: negotiated with Vulkan loader`.

### Linux — ptrace injection (fallback)

Injecting into the **running** game also works and needs no Steam
configuration: the engine is fully initialized, no loader-order races,
no env inheritance by helper processes.

```bash
# 1. Build the injector (resolves the target's own symbols via its
#    link_map, so injector and game may use different libcs)
make -C tools

# 2. Verify it on itself (no game needed)
./tools/cxsym-inject --self-test   # expect 3x MATCH

# 3a. Automatic: watchdog waits for the main menu in Player.log,
#     then injects (default 15 s later — click TO CITY meanwhile)
./tools/cxsfm-watchdog.sh

# 3b. Manual: game running, then (sudo needed unless
#     kernel.yama.ptrace_scope=0). NOTE: copy the .so under ~/.cache —
#     the game's container has a private /tmp, so a /tmp copy fails
#     dlopen with "No such file or directory".
PID=$(pgrep -n -f 'CarX_Street\.x86_64')
mkdir -p ~/.cache/cxsfm
cp target/release/libcxsfm.so ~/.cache/cxsfm/cxsfm-$$.so
sudo ./tools/cxsym-inject "$PID" ~/.cache/cxsfm/cxsfm-$$.so
```

Keep `tools/` **outside** the game install dir (anti-cheat scans it).
Check the log for the init sequence: `CXSF_LOG` if set, else
`cxsfm.log` next to the loaded `.so` (`~/.cache/cxsfm/` here), else
`/tmp/cxsfm.log`.

### Linux — LD_PRELOAD (alternative)

1. Set `LD_PRELOAD` to the shared library:
   ```bash
   LD_PRELOAD=./libcxsfm.so ./CarXStreet
   ```
   Or via Steam launch options:
   ```
   env LD_PRELOAD="/abs/path/libcxsfm.so" CXSF_LOG="$HOME/cxsfm.log" %command%
   ```

### macOS (dynamic loading — unvalidated, Metal stub)

1. Use `DYLD_INSERT_LIBRARIES`:
   ```bash
   DYLD_INSERT_LIBRARIES=./libcxsfm.dylib ./CarXStreet
   ```

## Framework API

Mods are compiled into the framework: one folder under `src/mods/`,
registered in `src/lib.rs`. The full walkthrough (folder layout,
i18n, Unity rules) is in [docs/plugin-guide.md](docs/plugin-guide.md).

### Creating a Mod

Implement the `Mod` trait (only `name` and `on_update` are required):

```rust
use crate::mods::api::{Mod, TileIcon};

struct MyMod;

impl Mod for MyMod {
    fn name(&self) -> &'static str { "My Mod" }          // unique
    fn menu_label_key(&self) -> &'static str { "mod_my" } // menu tile key
    fn menu_icon(&self) -> TileIcon { TileIcon::Tag }

    // Tick thread, ~60 Hz, while enabled. Unity calls allowed here.
    fn on_update(&mut self, _dt: f32) {}
    // Tick thread, while DISABLED (opt-in background work; default no-op).
    fn on_background_update(&mut self, _dt: f32) {}
    // Present thread: egui only, never Unity.
    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("My Mod").show(ctx, |ui| {
            ui.label("Hello from CXSFM!");
        });
    }
    // Transitions run on the tick thread, never on the UI thread.
    fn on_enable(&mut self) {}
    fn on_disable(&mut self) {}  // restore what you changed
    // Keep drawing with the menu closed (view-only).
    fn is_pinned(&self) -> bool { false }
}
```

Every hook runs under `catch_unwind`: a panicking mod is quarantined,
the menu and the other mods keep running.

### Registering a Mod

```rust
// src/lib.rs, next to the bundled mods (boots disarmed)
crate::mod_api::register_mod(Box::new(MyMod));
```

## Project Structure

```
cxsfm/
├── Cargo.toml           # Project configuration
├── README.md            # This file
├── CONTRIBUTING.md      # Contribution guidelines
├── OBJECTIVES.md        # Roadmap
├── flake.nix            # Pinned Nix toolchain
├── AGENTS.md            # Rules for coding agents (proven in-game constraints)
├── docs/
│   ├── README.md        # Documentation overview
│   ├── architecture.md  # System architecture
│   ├── plugin-guide.md  # Writing a mod, step by step
│   ├── field-notes.md   # Verified in-game findings log
│   └── overlay-proof.png # Screenshot: overlay in CarX Street
├── tools/
│   ├── cxsfm-setup.sh   # Stage the Vulkan layer + manifest
│   ├── cxsfm-watchdog.sh # ptrace injection for running games
│   ├── cxsfm-hang-dump.sh # Thread snapshot of a frozen game
│   ├── cxsym-inject     # ptrace injector (+ --self-test)
│   └── inject.c, Makefile # injector sources
└── src/
    ├── lib.rs          # Entry point (#[ctor], init + tick threads)
    ├── layer.rs        # Implicit Vulkan layer (present routing)
    ├── render.rs       # Render-hook dispatcher + frame counters
    ├── render/
    │   ├── vk.rs       # GOT/inline Vulkan fallback (static volk defeats it)
    │   ├── dxgi.rs     # Windows DXGI scaffold
    │   ├── metal.rs    # macOS stub
    │   └── overlay.rs  # egui-into-swapchain renderer
    ├── il2cpp.rs       # IL2CPP embedding bridge (+ field reflection)
    ├── unity/          # Unity helpers (object, camera, scene, text)
    ├── config.rs       # settings.toml / saves.toml / hud.toml paths + IO
    ├── i18n/           # Framework strings (7 languages)
    ├── menu.rs, ui/    # Tile menu + CarX theme
    ├── mainthread.rs   # methodPointer-swap infra (parked)
    ├── hotkey.rs       # SDL/X11 hotkeys + X11 input capture
    ├── memory.rs       # Maps/AOB scanning utilities
    ├── shim.rs         # glibc symbol shims (Steam Runtime)
    ├── mod_api.rs      # Mod trait + ModManager
    ├── mods.rs         # Mod registry
    └── mods/
        ├── api.rs         # Mod trait + tile types
        ├── loader.rs      # Isolated registry (quarantine on panic)
        ├── fpv_camera.rs  # FPV camera mod (parked)
        ├── inspector/     # Scene Inspector (model/bind/ui + i18n)
        ├── gamelog/       # Game Player.log viewer (model/bind/ui + i18n)
        ├── speedo/        # Speedometer restyle (model/bind/ui + i18n)
        ├── map/           # Minimap restyle (model/bind/ui + i18n)
        └── hud/           # Custom HUD (model/bind/live/ui + i18n)
```

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| egui | 0.36 (+`default_fonts`) | In-game UI rendering |
| egui-ash-renderer | 0.13 | egui → Vulkan pipeline (egui 0.36 line) |
| egui_material_icons | 0.8 | Material Symbols tile glyphs |
| toml | 0.8 | settings / saves / HUD layout files |
| ash | 0.38 | Vulkan calls through layer-resolved pointers |
| glam | 0.29 | 3D math vectors/matrices |
| ctor | 0.2 | Library constructor (auto-init) |
| libc | 0.2 | Cross-platform C library bindings |
| iced-x86 | 1 | Instruction decoding (inline-hook analysis) |
| bitflags | 2.9 | Bitflag types |
| windows-sys | 0.59 | Windows API bindings (Windows only) |
| objc | 0.2 | Objective-C runtime (macOS only) |

## Thread Safety

CXSFM uses thread-safe primitives throughout:
- `std::sync::Mutex` for mod collection
- `std::sync::LazyLock` for global instances
- `std::sync::atomic` for mod state flags

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

MIT License - See [LICENSE](LICENSE) for details.

## Disclaimer

This project is for educational purposes only. Modifying games may violate
their Terms of Service. Use at your own risk. The authors are not responsible
for any bans or account restrictions resulting from mod usage.