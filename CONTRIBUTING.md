# Contributing to CXSFM

Thank you for your interest in contributing to the CarX Street Framework Mod (CXSFM)! 
This document outlines how to develop new mods and extend the framework.

## Getting Started

1. **Clone the repository**
   ```bash
   git clone https://github.com/CXSFM/cxsfm.git
   cd cxsfm
   ```

2. **Build the framework**
   ```bash
   cargo build --release
   ```

3. **Set up your development environment**
   - Ensure you have Rust 1.75+ installed
   - For Windows: Install Visual Studio 2019+ with C++ support
   - For Linux: Install `gcc`, `make`, and `libc-dev`
   - For macOS: Install Xcode command line tools

## Developing Mods

### Step 1: Understand the Mod API

All mods must implement the [`Mod` trait](src/mod_api.rs) which defines:

- `name()` - Human-readable identifier
- `on_update(delta_time)` - Called each game tick
- `on_draw_ui(ctx)` - Optional UI rendering
- `on_enable()` - Initialization when enabled
- `on_disable()` - Cleanup when disabled

### Step 2: Implement Your Mod

Create a new Rust file in `src/mods/` and implement the `Mod` trait.

Example structure:
```rust
use cxsfm::mod_api::{Mod, ModManager};
use egui;

struct MyMod;

impl Mod for MyMod {
    fn name(&self) -> &'static str {
        "My Mod"
    }
    
    fn on_update(&mut self, delta_time: f32) {
        // Game logic
    }
    
    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        // UI rendering
    }
}
```

### Step 3: Register Your Mod

Register with the global mod manager:
```rust
use cxsfm::mod_api::register_mod;

fn init() {
    register_mod(Box::new(MyMod));
}
```

## Best Practices

### Code Quality

- Use **modern Rust** features (lazy locks, pattern matching, enums)
- Prefer `Option` and `Result` over `unwrap()`
- Keep unsafe code isolated in `memory.rs` with clear documentation
- Add comprehensive doc comments for public APIs
- Follow the existing code style (snake_case, descriptive names)

### Security & Anti-Cheat

- **Never modify gameplay mechanics** (speed, money, inventory)
- Only manipulate **visual elements** (camera, HUD, overlays)
- Respect the game's anti-cheat by not interfering with core systems
- Test thoroughly on various hardware configurations

### Performance

- Minimize allocations in hot paths
- Use `LazyLock` for global singletons
- Cache frequently accessed data
- Avoid blocking calls in mod callbacks

## Testing

Run the framework in release mode and test mods:

```bash
# Build with debug symbols
cargo build --release

# Run the framework (Windows)
./target/Release/cxsfm.dll

# Run with your game
LD_PRELOAD=./libcxsfm.so ./CarXStreet
```

## Submitting Changes

1. Create a new branch: `git checkout -b feature/your-feature`
2. Make your changes and add tests
3. Run the test suite: `cargo test`
4. Push to the repository
5. Open a pull request

## Code Review

All contributions undergo review by the CXSFM team. Please ensure your code:
- Compiles cleanly with `cargo build`
- Passes all tests
- Adheres to the project's architectural principles
- Includes adequate documentation

## Support

- **Issues**: Report bugs or request features on GitHub
- **Questions**: Ask in the Discord server or email the maintainers
- **Documentation**: Contribute to improving the docs in `docs/`

Happy modding! 🚗💨
