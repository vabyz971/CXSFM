# CXSFM Architecture

This document describes the system architecture of the CarX Street Framework Mod.

## High-Level Design

```
┌─────────────────────────────────────────────────────────────┐
│                    Game Process (CarX Street)               │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │ Game Thread │  │ Render Thread│  │ Physics Thread      │ │
│  │ (Main Loop) │  │ (egui UI)   │  │ (Physics Engine)    │ │
│  └──────┬──────┘  └──────┬──────┘  └──────────┬──────────┘ │
│         │                │                     │            │
│         └────────────────┼─────────────────────┘            │
│                          │                                  │
│  ┌───────────────────────▼───────────────────────────────┐ │
│  │              CXSFM Shared Library                     │ │
│  ├───────────────────────────────────────────────────────┤ │
│  │  [Main Thread (#[ctor])]                              │ │
│  │       │                                               │ │
│  │  ┌────▼────────────────────────────────────────┐      │ │
│  │  │           ModManager (Mutex<Vec<Box<dyn Mod>>)  │      │ │
│  │  └────┬────────────────────────────────────────┘      │ │
│  │       │                                               │ │
│  │  ┌────▼────┐            ┌────────────────────────┐   │ │
│  │  │memory.rs│            │        mods/           │   │ │
│  │  │AOB Scan │            │ fpv_camera.rs          │   │ │
│  │  │Read/Write│           │   (User mods here)     │   │ │
│  │  └─────────┘            └────────────────────────┘   │ │
│  └───────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## Module Overview

### 1. memory.rs

Provides low-level memory access with OS abstraction:

```
memory.rs (Public API)
├── Platform abstraction layer
│   ├── Windows: ReadProcessMemory / WriteProcessMemory
│   ├── Linux:   /proc/self/maps, /proc/self/mem
│   └── macOS:   mach_vm_region (requires entitlements)
├── AobScanner: Array-of-Bytes pattern matching
│   ├── Sliding window algorithm
│   ├── Wildcard support (0xCC byte)
│   └── find_all() / find_first()
├── read_value::<T>(): Safe typed memory reads
└── write_value::<T>(): Safe typed memory writes
```

### 2. mod_api.rs

Trait-based mod management system:

```
mod_api.rs
├── Mod trait (Send + Sync)
│   ├── name()
│   ├── on_update(delta_time)
│   ├── on_draw_ui(ctx)
│   ├── on_enable()
│   └── on_disable()
├── ModManager
│   ├── register_mod()
│   ├── update_all(delta_time)
│   ├── draw_ui_all(ctx)
│   └── lifecycle management
└── Global instances (LazyLock)
    ├── MOD_MANAGER
    └── get_mod_manager()
```

### 3. mods/

User-modifiable directory for game mod implementations:

```
mods/
└── fpv_camera.rs - Example mod demonstrating:
    ├── glam::Vec3 / glam::Quat usage
    ├── egui UI window creation
    ├── Mod trait implementation
    └── Mod registration
```

### 4. lib.rs

Framework entry point:

```
lib.rs
├── #[ctor] auto-initialization on library load
├── Global state management
└── Public API re-exports
```

## Data Flow

### Initialization (#[ctor])

```
Library Load
    ├── Platform detection (cfg target_os)
    ├── Memory region cache initialization
    └── Game process handle acquisition
```

### Update Loop (per frame)

```
Game Update
    ├── ModManager.update_all(delta_time)
    ├── Each mod's on_update()
    └── (Future) Camera hook integration
```

### UI Render Loop (per frame)

```
Egui Frame
    ├── ctx.begin_frame()
    ├── ModManager.draw_ui_all(ctx)
    ├── Each mod's on_draw_ui(ctx)
    └── ctx.end_frame()
```

## Thread Model

- **Main Thread**: Framework initialization (`#[ctor]`)
- **Update Thread**: Dedicated thread calling `on_update()` for all mods
- **Render Thread**: egui rendering thread calling `on_draw_ui()`

The `Mod` trait requires `Send + Sync` to allow concurrent access from multiple threads.

## Memory Safety

- All unsafe code is encapsulated in `memory.rs`
- `read_value`/`write_value` return `Option<T>`/`bool` (no panics)
- AOB scanner validates buffer bounds before pattern matching
- Platform differences are abstracted behind common interfaces

## Error Handling

- `MemoryError` enum for memory operation failures
- `Option<T>` for read failures (address not readable)
- `Result<T, MemoryError>` for operations requiring explicit error propagation

## Platform Support Matrix

| Feature               | Windows | Linux | macOS |
|----------------------|---------|-------|-------|
| AOB Scan             | ✅      | ✅    | ⚠️    |
| Memory Read/Write    | ✅      | ⚠️    | ❌    |
| Region Enumeration   | ✅      | ✅    | ⚠️    |
| eGUI Rendering       | ✅      | ✅    | ✅    |
| Shared Library       | .dll    | .so   | .dylib|

✅ = Full support | ⚠️ = Limited / requires entitlements | ❌ = Not supported