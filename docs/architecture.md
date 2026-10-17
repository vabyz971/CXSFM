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

- `unsafe` is fenced in the FFI/bridge modules (`memory.rs`,
  `il2cpp.rs`, `layer.rs`, `unity/`, `hotkey.rs`, `render/`) behind
  safe APIs with documented `SAFETY` contracts — mods contain none
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

## Refactor: isolated mods, split i18n/UI (2026-10)

A mod is one folder under `src/mods/`, compiled into the framework
`.so` (no external files). See `docs/plugin-guide.md`.

```
mods/
  api.rs        Mod trait + ModTile + TileIcon (only surface a mod touches)
  loader.rs     isolated registry: catch_unwind per hook, quarantine +
                auto-disable on panic, lock never held across mod code,
                unregister disables first, poison recovered
  inspector/    tool: scene tree via SceneManager roots + Transform walk
                (names, ids, active, position; throttled snapshot)
  speedo/       tool: custom speedometer (hides game speed texts, restore)
  fpv_camera.rs parked (render-hook future)
mod_api.rs      façade: ModManager delegates to the loader (paths stable)
i18n/           mod.rs (detection + dispatch) + one file per language;
                mod tile keys (mod_scout/…) delegate to each mod's own
                i18n module — framework files never change for a mod
ui/             theme.rs (palette, 399px, 12px gaps, 10px rounding)
                tiles.rs (draw_icon + tile_at)
                chrome.rs (drag-anywhere, header, footer, nav buttons)
                pages.rs (main / settings / about, About lists mod
                descriptions via loader.describe_all)
menu.rs         Page state + window orchestration only
```

Rules: capture originals before writing, read-back after, restore on
disable; Unity objects as re-resolved `usize` handles; UI width
locked, labels wrapped and hover-only.
## Main-thread tick (MethodInfo.methodPointer swap, 2026-10)

Mods called Unity from the `cxsfm-tick` thread (≠ Unity's main
thread): residual races (freezes, SIGSEGV). `minhud` runs every frame
through hooks on game methods (hence on the main thread).
`src/mainthread.rs` (phase 1: infra + probes + API, no mod migrated)
does the same by swapping the `MethodInfo.methodPointer` pointer
(offset 0): a pure data write, no code patching (see
`src/render/vk.rs` for the safety style).

- Candidates (priority): DOTweenComponent::Update, RtmpManager::Update,
  EventSystem::Update, CanvasScaler::Update,
  CinemachineBrain::ManualUpdate (delegate probe), CarBehaviour::Update.
  All in the After phase (original first).
- Discovery (logged once, ≤60 lines): Assembly-CSharp classes whose
  name contains hud/speed/tacho/gauge/camera/drive/car/tween/rtmp/race
  with a 0-arg Update/LateUpdate, plus
  RearRaceCamera::PostPipelineStageCallback and
  CamerasManager::ForceUpdate when present.
- Install (tick, never present): original checked executable inside
  GameAssembly.so (`/proc/self/maps`), stored before an atomic 8-byte
  swap; ≤6 hooks; retry every ~3 s (max 20).
- `hook<const I>` hooks use the 5-arg invoker ABI (the pointer is the
  invoker, not the raw method); panics contained by `catch_unwind`
  (callbacks cut, original always relayed).
- Dedup by `Time.get_frameCount` (fallback: first instance seen,
  re-lock after 1 s); one callback pass per frame.
- Selection: first slot with ≥20 calls/s on the main thread for 3 s
  (locked); otherwise `is_active() == false` and mods keep working as
  before (no behaviour change).
- API: `is_active()`, `post(label, f)` (256-entry queue),
  `every_frame(label, f)` (max 8), `stats()`. 1 ms/frame budget,
  surplus deferred.
- Shutdown: when `game_shutting_down()`, relay only; pointers never
  restored (inert).
- Scenes: `get_sceneCount`/`GetSceneCount` + `GetSceneAt` (first one
  found, logged once); `DontDestroyOnLoad` through
  `Resources.FindObjectsOfTypeAll<Transform>` on manual refresh only;
  children loaded when a node is opened (on-demand expansion).
