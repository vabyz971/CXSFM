# CXSFM Field Notes — in-game findings

> Context-reload document. Everything below was **verified in-game**
> unless marked [hypothesis]. Empty `~/.cache/cxsfm/cxsfm.log` before
> each session — two sessions share the file otherwise.

## Session environment

- NixOS + niri (Wayland). XWayland on `:0`, `WAYLAND_DISPLAY=wayland-1`.
- Game Steam LaunchOptions:
  `SDL_VIDEODRIVER=x11 VK_ADD_LAYER_PATH="$HOME/.cache/cxsfm" VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay %command%`
- Game process env (from `/proc/<pid>/environ`): `SDL_VIDEODRIVER=x11`,
  `DISPLAY=:0`, `XDG_SESSION_TYPE=wayland`.
- The user's global `SDL_VIDEODRIVER=wayland` (niri.nix) does **not**
  leak into the game — launch options override it.

## Load path — layer ONLY

- `./tools/cxsfm-setup.sh` stages `target/release/libcxsfm.so` →
  `~/.cache/cxsfm/libcxsfm_layer.so` + `VkLayer_CXSFM.json`. The Vulkan
  loader `dlopen`s it (ctor runs) around `vkCreateInstance`.
- `tools/cxsfm-watchdog.sh` (ptrace) is the **alternative** for
  already-running games. **Never both**: double tick threads, and two
  toggles per keypress cancel out.
- No `negotiated with Vulkan loader` line → negotiation does **not**
  complete (next work item). The framework still boots via loader dlopen.
- Log lives at `~/.cache/cxsfm/cxsfm.log` (`own_module_dir` via
  `dladdr`; `CXSF_LOG` overrides). O_APPEND single-`write_all` lines
  (atomic records), `[+Xs]` timestamps, mirrored to stderr.

## Input — three paths, verdicts

Diagnostic line (zero keypresses needed):
`input: unity cache resolved (GetKey ...), sdl ..., x11 ...`

- Toggle key: **O** — SDL scancode 18, `KeyCode.O` = 111, X11 keycode
  32 (positional: same physical key on QWERTY and AZERTY).
- **Unity `Input.GetKey` (IL2CPP, tick thread): WORKS off-main-thread.**
  Proven: every O press fired via `(unity)`. Level + own edge detector
  (`GetKeyDown`'s single-frame flag could fall between 10 Hz polls).
- **SDL `SDL_GetKeyboardState`: resolves (live) but keyboard STALE.**
  The game pumps SDL for the controller only (that's what x11 is for);
  keyboard goes through Unity's native X11 input. Never won a race.
  Kept as fallback.
- **X11 `XQueryKeymap` (own `Display` connection): live**,
  focus-independent, immune to `SDL_VIDEODRIVER`. Backup, unfired so far.
- O flips the framework-UI flag + re-arms the HUD Scout; every
  transition is logged (`hotkey O (src): ...`, `hotkey O: HUD scout
  ...`, `hud: ...`). FPV is unregistered (user-deactivated); the parked
  module stays compiled for the render-hook future.

## The frozen-tick incident (self-deadlock)

- Symptom: tick thread alive in `/proc`, **0 voluntary ctxt switches
  over 300 ms**, log frozen at `init finished`, total input silence.
- Root cause: `match CACHE.lock().unwrap()...` — match-scrutinee
  temporaries (`MutexGuard`) live through the whole `match`; the `None`
  arm re-locked the same mutex → deterministic self-deadlock on the
  very first input poll, before the diagnostic line and before the X11
  path (evaluated after).
- Found via: futex uaddr in `/proc/<pid>/task/<tid>/syscall` → `nm`
  file offset → `poll_framework_input_edge::CACHE`.
- **RULE: bind-then-match** — `let snap = lock...; match snap {...}`.
  Never match directly on a lock guard when an arm can re-lock.
  Audited 2026-10-06: lib.rs was the only fatal site (fpv hardened too;
  `il2cpp_api()` is lock-free `OnceLock::get`).
- Why smoke missed it: no IL2CPP in smoke → early return before the
  mutex. **Smoke cannot cover IL2CPP-gated paths.**

## IL2CPP bridge notes

- 167 assemblies in-game; domain + attach on init AND tick threads
  (attach failure degrades, never crashes).
- `invoke` (value returns) vs `invoke_void` (**void setters**: the
  runtime returns NULL on success — NULL is success, only a managed
  exception is failure). Using `invoke` on setters = silent-failure
  trap: the set executes but reports failure.
- `find_class`: named-assembly fast path, then full image walk.
  Classes/methods cached (process-stable); objects re-resolved per use
  (go stale across scenes).
- `GetKey(KeyCode-as-int by address)` → boxed bool → `object_unbox`
  → byte. Template for other value-type returns.
- `UnityCache` also resolves `Input.GetKey`; Text pieces are optional
  (null = stripped build, degrade don't fail).

## FPV mod — PARKED

- 1 Hz `set_fieldOfView` vs the game's per-frame camera control =
  user-confirmed zoom flicker. Unwinnable without per-frame timing.
- `PARKED = true`: zero game writes; enable/disable only flips state +
  logs. `base_fov`/restore machinery intact for unpark.
- Unpark condition: Present-thread application (**needs the render
  hook**), not faster tick polling.
- `register_mod` starts mods **disabled** (framework boots quiet).

## HUD Scout mod (read-only)

- `HUD Scout` auto-arms at registration, inventories every live
  `UnityEngine.UI.Text` **and** `TMPro.TextMeshProUGUI` once (`name` +
  truncated content → log), sets `done`, then idles. **Zero writes,
  nothing to restore.**
- In-game finding: **0 legacy Texts** — CarX renders UI through TMP.
  The cache resolves TMP by full image walk (no assembly-name guess).
- O re-arms per scene/menu: press O wherever you want inventoried.
- Purpose: the log inventory tells us which texts exist so a later
  HUD-rewrite mod can target static labels (dynamic readouts are
  rewritten per frame by the game — same fight as FOV, target statics).

## HUD Hide mod (hide, don't rewrite)

- Text rewrites ran exception-free yet never showed on screen
  (mesh-rebuild subtleties from a foreign thread, or non-rendered
  objects). Switched strategy: `Behaviour.set_enabled(false)` on the
  target component — one renderer-checked boolean, no strings, no mesh
  rebuild. `HUD Hide` (O opts in) hides `Text (TMP) Net`, restores the
  exact flag on disable. Read-back (`get_enabled`) on every write.
- Display-side lesson: `applied` in the log means the setter ran
  without exception, NOT that pixels changed. Every write now logs an
  immediate read-back plus the object's `activeInHierarchy`
  (`Component.get_gameObject` + `GameObject.get_activeInHierarchy`):
  readback==want + active==NO → writing a disabled branch nobody
  renders; readback==original → silent setter no-op. Pick targets with
  active==yes.
- Maintain loop: handle-less → gentle search (~0.2 Hz, exact name
  match, original flag captured before any write, never write blind);
  handle held → 1 Hz verify, re-hide on drift.
- Handles stored as `usize` (Send+Sync without wrappers); cleared on
  disable and on unreadable reads (scene change → re-search).
- Silent-maintain incident: `maintain` checked the mod's own cache
  before resolving it → empty cache = silent return, zero log lines
  (scout worked because it ensures first). Fix: `ensure_unity()` FIRST
  in `maintain`. Rule: any resolve-then-act function must ensure before
  checking.
- Disable restores the exact flag. O drives scout + hide together:
  one press = full HUD cycle for the current scene.
- PROVEN ON SCREEN 2026-10-06: `Text (TMP) Net` disappears on O,
  reappears on re-O. Off-thread IL2CPP writes DO affect rendering —
  the bridge pipeline (resolve → write → pixels → restore) is
  validated end-to-end. Earlier text-rewrite failures are now suspect
  to have been the silent-maintain bug (writes never executed), not
  mesh-rebuild issues: retrying `set_text` is legitimate.
- Follow-up: `HUD Version Tag` v2 stamps `<original> | CXSFM v0.1.0`
  onto `Nickname Text (TMP)` (static per-session username, same menu
  scenes — independent object from the hide target so one O press
  verifies both). Same fixed maintain + active/read-back
  instrumentation.
- PROVEN ON SCREEN 2026-10-07: nickname shows `vabyz971 | CXSFM
  v0.1.0` while `Net.` hides, both restored on re-O. TEXT REWRITES
  WORK from the tick thread — the earlier failures were the
  silent-maintain bug, never mesh-rebuild issues. The whole
  read/write IL2CPP pipeline is now proven on pixels.
- Arming runs `maintain` on the very next tick (`pending` flag): the
  5 s search cadence otherwise misses fast toggle on/off presses
  entirely (seen in log: arm→off in <1 s, zero maintain lines).

## How other tools modify Unity values (research 2026-10-06)

- **BepInEx/MelonLoader**: plugins are MonoBehaviours living on the
  game's MAIN thread (`Awake`/`Update` via `AddComponent`). All Unity
  API calls run main-thread — setters "just work". (We have no
  main-thread execution without ClassInjector or a loop hook.)
- **Cheat Engine / PINCE**: dissect metadata → field offsets
  (`object base + offset`), then DIRECT memory writes or
  `il2cpp_field_get/set_value`. Thread-agnostic for plain data.
  Strings: allocate via `il2cpp_string_new`, then field write.
- **Native C++ IL2CPP mods**: `thread_attach` + `runtime_invoke`
  (our current path) + field APIs for data. Same pattern as ours.
- Consequence for the invisible-write problem: our `set_text` invoke
  is the standard native-mod call; if readback confirms memory changed
  but pixels don't, suspects are wrong/inactive object or TMP mesh
  rebuild, NOT the invoke mechanism. Fallback per research: write the
  `m_text` field directly (`il2cpp_class_get_field_from_name` +
  `il2cpp_field_set_value`) + invoke `SetAllDirty()` — new bridge
  symbols, only if the setter path stays visually dead.

## Render hook status

- Static volk dispatch: no PLT slots (`vkQueuePresentKHR`,
  `vkGetDeviceProcAddr`) → GOT/inline strategies fail gracefully, table
  scan exceeds budget. The implicit layer is THE path (validated
  headless 2026-10-07: negotiate + instance + 2× device through our
  wrappers, `vulkaninfo --summary` rc=0).

## Vulkan layer bring-up (all verified against vulkaninfo + loader source)

- `LAYER_NEGOTIATE_INTERFACE_STRUCT` is **1**, not 2 (`vk_layer.h`).
  The old `2` failed the `s_type` check → loader skipped us after
  dlopen (ctor ran, framework booted, zero present routing). The
  negotiate function now logs success AND refusal reasons.
- Link-node sTypes are **47/48** (core range, `vulkan_core.h`), NOT
  1000016000/1. Symptom of the wrong constants: chain walk sails past
  our own node (the loader's four prepended sType-47 nodes show as
  `function` 3/2/1/0 = FEATURES/DATA_CB/CREATE_DEV_CB/LINK).
- `find_link_info` must SKIP same-sType non-link nodes (`function !=
  0`), not stop at them (DATA_CALLBACK nodes come first).
- `advance_link` (`pLayerInfo = pLayerInfo->pNext`) is MANDATORY before
  forwarding CreateInstance/CreateDevice, else the terminator walks a
  stale chain → `ERROR_INITIALIZATION_FAILED`.
- `layer_get_instance_proc_addr` MUST match **both** `vkCreateInstance`
  and `vkCreateDevice`. Transparently forwarding CreateDevice hands
  the loader the terminator's function → device creation bypasses our
  wrapper → empty device map → every device lookup NULL → crash on
  first NULL dispatch call (vulkaninfo SIGSEGV at 0x0).
- `pfnGetPhysicalDeviceProcAddr`: provide a real stub returning NULL
  per name, never a NULL pointer (jump-to-zero risk).
- Tuple-slot discipline in `next_procs(li, true)` → `(inst, dev)`:
  CreateDevice lookup via **inst** (link+8), per-device map stores
  **dev** (link+16). Swapping either way breaks one side silently.
- Headless validation recipe: `VK_ADD_LAYER_PATH=~/.cache/cxsfm
  VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay CXSF_LOG=/tmp/x.log
  NODEVICE_SELECT=1 nix shell nixpkgs#vulkan-tools -c vulkaninfo
  --summary` → expect `negotiated`, `instance created`, `device
  created`, rc=0. (vulkaninfo never presents; Present path validates
  in-game via `render: first present intercepted`.)
- Manifest `api_version` 1.3.0 (silences the loader's "older than app"
  warning; we use nothing beyond 1.0).
- PROVEN IN-GAME 2026-10-07: `negotiated (interface 2 -> 2)` →
  `layer-driven present routing active` → `render hook installed:
  Vulkan` → **`render: first present intercepted, hook is live`**.
  The game creates 2 VkInstances (both negotiate). Present routing
  works under pressure-vessel with the sniper loader.

## Overlay Phase 1b (in-process egui rendering, 2026-10-07)

- Renderer: `egui-ash-renderer 0.6.0` (last line for egui 0.29 — NO
  egui bump) + `ash 0.38`, default internal allocator (no
  gpu-allocator/vk-mem needed). `ash::Instance/Device::load_with`
  resolve through our captured chains (pointers are process-global;
  any live instance/device serves the lookup).
- Interception points: `vkCreateSwapchainKHR` (capture format/extent/
  images — `imageFormat@28`, `extent@36/40`) + `vkDestroySwapchainKHR`
  (forward + drop state) + `vkQueuePresentKHR` (draw + forward).
  Queues map carries the family (V2 parses `queueFamilyIndex@20`).
- Per present (single-swapchain only; multi → untouched): fence-wait
  predecessor → run egui headless → tessellate → `set_textures` →
  record (barrier PRESENT_SRC→COLOR_ATTACHMENT, render pass
  `loadOp=LOAD` preserving the game image, `cmd_draw`, barrier back)
  → submit waiting the present's own semaphores, signaling ours →
  real present waits ONLY on ours (binary single-wait rule). Any
  failure → ORIGINAL present (game never breaks for UI).
- Serialization: `in_flight_frames=1`, fence per present, no
  `vkQueueWaitIdle` (pipelining preserved). Empty UI (no primitives,
  no deltas) and zero enabled mods skip the submit entirely.
- PROVEN ON SCREEN 2026-10-07: egui windows render in-game over the
  live frame (loadOp=LOAD preserves the game image underneath), game
  stable past the cinematic. Non-interactive v1 (empty RawInput) —
  input routing (mouse/keyboard into the overlay) is the next step.
- STALE-INSTANCE CRASH (sigsegv in `loader_gpa_instance_terminator`
  from ash table loading, caught via Unity Player.log stack): the
  overlay resolved ash functions through the FIRST VkInstance, but the
  game creates a probe instance and destroys it — resolving through
  the corpse segfaults. Fix: `live_instances` set (insert on every
  CreateInstance, remove in new `vkDestroyInstance` interception);
  ash loading uses any live member, overlay degrades when empty.
  Rule: NEVER store a Vulkan handle across calls without tracking its
  destruction (`vkDestroyDevice` interception + full state cleanup
  included for the same reason — driver handle reuse).
- Heartbeat: tick logs `render: N presents intercepted so far` every
  ~15 s (routing-liveness proof); `CreateDevice entry` logs every
  device creation (a bypassed second device would otherwise be
  invisible).

## In-game test protocol

1. Rebuild, `./tools/cxsfm-setup.sh`, empty the log, launch.
2. Expect init block → `input: ...` line with no keypress.
3. Press O → `hotkey O (...)` + mod lines. Report the `(src)`.
