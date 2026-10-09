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

## Refactor: isolated mods, split i18n/UI (2026-10)

A mod is one folder under `src/mods/`, compiled into the framework
`.so` (no external files). See `docs/plugin-guide.md`.

```
mods/
  api.rs        Mod trait + ModTile + TileIcon (only surface a mod touches)
  loader.rs     isolated registry: catch_unwind per hook, quarantine +
                auto-disable on panic, lock never held across mod code,
                unregister disables first, poison recovered
  common.rs     shared cadences (1 Hz verify / 0.2 Hz search) + preview()
  hud_scout/    example: read-only HUD text inventory (mod.rs + i18n.rs)
  hud_hide/     example: hide one element, restore on disable
  hud_text/     example: replace (or blank) one text, restore on disable
  inspector/    tool: scene tree via SceneManager roots + Transform walk
                (names, ids, active, position; throttled snapshot)
  camera/       PARKED (writes): live Cinemachine discovery, sliders
                simulate recipes into the log without touching the game
  video/        PARKED: reads fine, some setters crash the game —
                re-enable once the faulting setter is isolated
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
## Tick main-thread (échange de MethodInfo.methodPointer, 2026-10)

Les mods appelaient Unity depuis le thread `cxsfm-tick` (≠ main
thread Unity) : courses résiduelles (freezes, SIGSEGV). `minhud`
tourne chaque frame via des hooks sur des méthodes du jeu (donc sur
le main thread). `src/mainthread.rs` (phase 1 : infra + sondes + API,
aucun mod migré) fait pareil par échange du pointeur
`MethodInfo.methodPointer` (offset 0) : écriture de donnée pure, sans
patch de code (cf. `src/render/vk.rs` pour le style de sûreté).

- Candidats (priorité) : DOTweenComponent::Update, RtmpManager::Update,
  EventSystem::Update, CanvasScaler::Update,
  CinemachineBrain::ManualUpdate (sonde delegate), CarBehaviour::Update.
  Tous en phase After (original d'abord).
- Découverte (log, 1 fois, ≤60 lignes) : classes d'Assembly-CSharp
  contenant hud/speed/tacho/gauge/camera/drive/car/tween/rtmp/race
  avec Update/LateUpdate 0-arg + RearRaceCamera::PostPipelineStageCallback
  et CamerasManager::ForceUpdate si présentes.
- Installation (tick, jamais present) : original vérifié exécutable
  dans GameAssembly.so (`/proc/self/maps`), stocké avant échange
  atomique 8 octets ; ≤6 hooks ; réessai ~3 s (max 20).
- Hooks `hook<const I>` au format ABI invoker à 5 args (le pointeur
  est l'invoker, pas la méthode brute) ; panique contenue par
  `catch_unwind` (callbacks coupés, original toujours relayé).
- Déduplication par `Time.get_frameCount` (repli : première instance
  vue, re-lock après 1 s) ; un seul passage de callbacks par frame.
- Sélection : premier slot ≥20 appels/s sur le main thread pendant
  3 s (verrouillé) ; sinon `is_active() == false` et les mods
  continuent comme avant (aucun changement de comportement).
- API : `is_active()`, `post(label, f)` (file 256), `every_frame(label,
  f)` (max 8), `stats()`. Budget 1 ms/frame, surplus reporté.
- Arrêt : si `game_shutting_down()`, relais seul ; pointeurs jamais
  restaurés (inertes).
- Scènes : `get_sceneCount`/`GetSceneCount` + `GetSceneAt` (premier
  trouvé, loggé une fois) ; `DontDestroyOnLoad` via
  `Resources.FindObjectsOfTypeAll<Transform>` au refresh manuel seul ;
  enfants chargés à l'ouverture du nœud (dépliage à la demande).
