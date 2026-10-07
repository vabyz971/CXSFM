# CarX Street Framework Mod (CXSFM)

> Open-source modding framework for CarX Street - Built with Rust (Edition 2024)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust Edition](https://img.shields.io/badge/Rust-2024-orange.svg)](https://doc.rust-lang.org/edition-guide/editions/2024.html)
[![egui](https://img.shields.io/badge/egui->=0.36-FF0000)](https://github.com/emilk/egui)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-blue)](https://github.com/CXSFM/cxsfm)

## What is CXSFM?

CXSFM is a **framework for creating in-game mods** for CarX Street. It provides:

- 🎨 **Cosmetic UI Overlays** - Inject egui-based interfaces into the game
- 📷 **Camera Manipulation** - Modify camera perspective (FPV, drone, etc.)
- 🛡️ **Anti-Cheat Compliant** - Focuses on visual modifications only
- 🔌 **Modular Architecture** - Easy-to-implement trait-based mod system
- 🚀 **Cross-Platform** - Supports Windows (.dll), Linux (.so), macOS (.dylib)

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

- Rust >= 1.75 (Edition 2024)
- CMake / Ninja (for egui renderer)
- C++ build tools

### Building

```bash
# Clone the repository
git clone https://github.com/CXSFM/cxsfm.git
cd cxsfm

# Build for Windows (x86_64)
cargo build --target x86_64-pc-windows-msvc --release

# Build for Linux
cargo build --target x86_64-unknown-linux-gnu --release

# Build for macOS
cargo build --target x86_64-apple-darwin --release
```

### Output Files

After building, you'll find:
- `target/x86_64-pc-windows-msvc/release/cxsfm.dll` (Windows)
- `target/x86_64-unknown-linux-gnu/release/libcxsfm.so` (Linux)
- `target/x86_64-apple-darwin/release/libcxsfm.dylib` (macOS)

## Injection Guide

### Windows (DLL Injection)

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
Check `CXSF_LOG` (default `/tmp/cxsfm.log`) for the init sequence.

### Linux — LD_PRELOAD (alternative)

1. Set `LD_PRELOAD` to the shared library:
   ```bash
   LD_PRELOAD=./libcxsfm.so ./CarXStreet
   ```
   Or via Steam launch options:
   ```
   env LD_PRELOAD="/abs/path/libcxsfm.so" CXSF_LOG="$HOME/cxsfm.log" %command%
   ```

### macOS (dynamic loading)

1. Use `DYLD_INSERT_LIBRARIES`:
   ```bash
   DYLD_INSERT_LIBRARIES=./libcxsfm.dylib ./CarXStreet
   ```

## Framework API

### Creating a Mod

Implement the `Mod` trait:

```rust
use cxsfm::mod_api::{Mod, ModManager};
use egui;

struct MyMod;

impl Mod for MyMod {
    fn name(&self) -> &'static str {
        "My Mod"
    }
    
    fn on_update(&mut self, delta_time: f32) {
        // Game logic update
    }
    
    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        egui::Window::new("My Mod").show(ctx, |ui| {
            ui.label("Hello from CXSFM!");
        });
    }
}
```

### Registering a Mod

```rust
use cxsfm::mod_api::register_mod;

fn register_mods() {
    register_mod(Box::new(MyMod));
}
```

## Project Structure

```
cxsfm/
├── Cargo.toml           # Project configuration
├── README.md            # This file
├── CONTRIBUTING.md      # Contribution guidelines
├── OBJECTIVES.md        # Roadmap
├── docs/
│   ├── README.md        # Documentation overview
│   └── architecture.md  # System architecture
└── src/
    ├── lib.rs          # Main entry point
    ├── memory.rs       # Memory scanning & operations
    ├── mod_api.rs      # Mod trait & manager
    └── mods/
        └── fpv_camera.rs  # Example FPV camera mod
```

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| egui | >=0.36 | In-game UI rendering |
| glam | >=0.29 | 3D math vectors/matrices |
| ctor | 0.2 | Library constructor (auto-init) |
| libc | 0.2 | Cross-platform C library bindings |
| windows-sys | 0.59 | Windows API bindings |

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