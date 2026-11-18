# AGENTS.md — CXSFM (CarX Street Framework Mod)

> Rust (Edition 2024) mod framework for CarX Street: implicit Vulkan
> layer, egui 0.36 overlay, IL2CPP bridge. Cosmetic/interface/camera
> mods only — never gameplay, economy or physics.

## Commands

```bash
nix develop --command cargo build --release  # build (rustc 1.98 via flake)
nix develop --command bash -c 'cargo test --workspace --lib'
./tools/cxsfm-setup.sh                       # stage the layer after every rebuild
```

## Architecture (Cargo workspace, `crates/`, deps point down)

- `cxsfm-api` — `Mod` trait, registry with panic quarantine, manager
  (`register_mod`), `config` (settings, paths, atomic IO), `i18n`
  (+ `register_tile_label`), `log_line`, `fmod` link shim.
- `cxsfm-sys` — FFI: `il2cpp` (`resolve`, `invoke`/`invoke_void`,
  `domain_checked`, cached classes/methods, objects re-resolved on use),
  `unity/` (`object_alive` via `m_CachedPtr`, camera, scene, text/TMP,
  transforms, `UnityCache`), `memory`, `fmod`, `mainthread`
  (`methodPointer` swap, phase 1: probes, no mod migrated),
  `il2cpp_api()` + `shutting_down()` (probe set by the layer).
- `cxsfm-input` — `hotkey` (F8 = UI visibility + capture, SDL + X11;
  Unity path cut: it segfaulted at teardown), `gamepad`, `padnav`.
- `cxsfm-ui` — CarX theme (black panels, yellow accent `#F5C518`, 3px
  corners, Material Symbols icons), `tool::ToolChrome` + `kit` (every
  tool window), `game` + `app` (phone apps: `Mod::app_tabs` +
  `on_draw_app`, tabs under the app title). No `unsafe`.
- `cxsfm-mods` — bundled mods (`inspector`, `gamelog`, `speedometer`
  + shared `editor`/`saves`, `map`, `custom_hud`, `engine_audio`,
  `fpv_drone`, `fpv_camera` parked), `register_all()`.
- `cxsfm` (cdylib) — `lib.rs`: `#[ctor]` → deferred init thread →
  ~60 Hz tick (`on_update`), never IL2CPP during teardown; `layer.rs`:
  implicit Vulkan layer (negotiation, link nodes, `advance_link`);
  `render/overlay.rs`: egui into the swapchain images at present
  (`loadOp=LOAD`, 1 frame in flight, per-image replay);
  `render/vk.rs`: LEGACY GOT/inline patching (do not extend without
  agreement); `menu/`: phone menu + Settings/About.

## Rules proven in game (docs/field-notes.md — READ before changing)

- **bind-then-match** on locks: never a direct `match` on a guard if an
  arm may re-lock (tick self-deadlock, incident 2026-10-06).
- **`invoke_void` for setters**: the runtime returns NULL on success —
  `invoke` on them = silent failure although the write happened.
- **`object_alive` before ANY Unity access** (read as well as write);
  stored pointers forbidden — re-resolve by path/name on every use.
  Only exception (user agreement 2026-10-16): `custom_hud/live.rs` keeps the
  root instance (e.g. `UITachometer`) between two reads — readable
  memory + `m_CachedPtr` + same class checked before EVERY read, reads
  = raw field copies (no getter, no write). `FindObjectsOfType` only
  once the instance is gone, ≤ once / 5 s: a periodic (2 s) re-search
  crashed the game while driving (the game creates/destroys objects
  continuously). Do not extend this pattern to writes.
- **No Unity call from the present thread**; the tick skips all IL2CPP
  during teardown (domain killed before Vulkan).
- **No unbounded arithmetic on the render path** (`checked`/saturating
  — a panic in `extern "C"` = abort).
- **`#[repr(C)]` on FFI structs** (e.g. the local `VkPresentInfoKHR`).
- Textures: a free is stale if its id is (re-)set; `set_texture` /
  `free_texture` per item; never `vkQueueWaitIdle`, bounded timeouts.
- Vulkan handles tracked until destruction (instances/devices/swapchains).

## Constraints

- Final binary: symbols **max `GLIBC_2.35`** (Steam Runtime sniper =
  2.36) — check after every toolchain bump:
  `objdump -T target/release/libcxsfm.so | grep -oP "GLIBC_[0-9.]+"`.
- **Never** any gameplay/economy/physics/network change.
- Mods **disarmed at boot**, restore on disable — except speedometer/map:
  their edits stay on disable (user request), back to originals through
  the Restore button; with auto-load (Settings opt-in), their saves are
  re-applied even with the tile closed (`Mod::on_background_update`).
- One change = one commit, clear message, a single subject.
- No new dependency without agreement (pure Rust only).
- `cargo test` proves nothing in game: state what remains to validate.

## In-game test

1. Rebuild + `./tools/cxsfm-setup.sh`, empty `~/.cache/cxsfm/cxsfm.log`.
2. Launch (Steam options: `VK_ADD_LAYER_PATH="$HOME/.cache/cxsfm"
   VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay %command%`).
3. Expected: `negotiated` → `render hook installed: Vulkan` →
   `first present intercepted` → an `input: ...` line without a key press.
4. F8 → `hotkey F8 (...)`; scenarios: race, garage↔city, clean quit.
5. After quit: `grep -a -c SIGSEGV Player.log` must stay silent about cxsfm.
