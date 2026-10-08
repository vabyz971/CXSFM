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
- TEXTURE USE-AFTER-FREE (invisible text, working shapes/clicks):
  on atlas resize egui emits free+set for the SAME TextureId in one
  delta, and `set_textures` already destroys/replaces same-id
  occupants internally — so a deferred `free_textures` for a re-set
  id destroys the NEW texture. Fix: run egui first, drop pending
  frees for ids in the current set, then free, then upload. Rule: a
  pending free is obsolete iff its id is (re-)set. (Trigger here:
  non-ASCII titles like the em-dash force the first atlas growth.)
- Heartbeat: tick logs `render: N presents intercepted so far` every
  ~15 s (routing-liveness proof); `CreateDevice entry` logs every
  device creation (a bypassed second device would otherwise be
  invisible).
- TEARDOWN CRASH (sigsegv in tick `poll_framework_input_edge`,
  caught via Player.log stack): Unity kills its scripting domain
  while our tick keeps polling IL2CPP → Unity's crash handler hangs
  on black screen (Steam kill required). Fix layers: (1)
  `layer::shutting_down()` — true when object kinds that existed are
  all gone (swaps/devices/instances + had_* flags); tick skips ALL
  IL2CPP touch (framework input Unity path + `update_all_mods`) while
  true, SDL/X11/log keep running. (2) `il2cpp::domain_checked` —
  permanent IL2CPP stop after 3 consecutive `domain()` failures.
  Rule: teardown is detected via object lifetimes, never probed via
  the dying runtime itself.

## Overlay input: X11 capture + event pump (2026-10-07)

- Showing UI is not interacting: while the game owns input, clicks
  also drive the game. **F8** now toggles UI capture (F9/O retired,
  see hotkey section): `XGrabPointer` + `XGrabKeyboard` to our own invisible 1x1
  InputOnly window (mapped once by a dedicated event thread on its
  OWN display connection; `XInitThreads` once; `XSetErrorHandler`
  no-op installed — Xlib's default handler would `exit(1)` the game
  on any protocol error).
- Event thread blocks in `XNextEvent`, translates to `egui::Event`
  (buttons/wheel/motion/keys + `XLookupString` text, modifiers from
  the event state mask), pushes to a queue drained per frame. Keycode
  table is positional (same physical keys QWERTY/AZERTY); Shift/Ctrl/
  Alt/Caps produce no Key events (modifiers only). No key-repeat v1.
- Release pushes `PointerGone` (no stuck hover). Overlay paints a
  software cursor (halo + dot) in the foreground while captured —
  the game hides/confines the OS cursor.
- Edge states are per (source, key) (`edge()` map): O + F9 share no
  state across SDL/Unity/X11 paths.
- F8 (SDL 65, Unity 289, X11 74) is the ONLY hotkey: UI visibility
  + capture, both directions. O and F9 bindings removed (helpers
  stay, unused). F8 never touches mods (see decoupling below).
- UI/effects decoupled: F8 flips visibility + capture only, never
  mods (manager checkboxes own them; effects persist with menu
  closed). Overlay gate = mods-enabled OR ui-visible (gating on
  mods alone blanked empty menus — caught live: 21k presents, 0
  overlay attempts). `run_ui` gates `draw_ui_all` on `ui_visible`
  (mod windows hide with the menu, `on_update` effects continue).
- Heartbeat reports `presents (overlay, plain)` counters: a high
  plain/(overlay+plain) ratio with visible UI quantifies flicker
  (fallback rate) instead of guessing.
- CAUGHT: the re-targeted `VkPresentInfoKHR` local lost its
  `#[repr(C)]` during an edit — Rust layout is unspecified without
  it and the driver would read garbage. Always re-verify ABI markers
  after touching FFI structs.

## Code review pass (2026-10-07, user-reported fan noise + missing text)

## Kino-style menu + i18n auto-detect (2026-10-07)

## Flicker, second root cause: single buffer replayed on all images (2026-10-07)

- The replay submitted ONE command buffer for every present — but a
  recording embeds its target framebuffer/image. Presents cycling to
  another image drew the UI onto the wrong image: UI visible only on
  presents reusing the recorded index (~1/N frames = flicker).
- Fix: one command buffer + one recorded flag PER swapchain image;
  replay submits `cmd[image_index]`; content changes invalidate all
  images (each re-records on its next present, converging in one
  cycle). Single fence still serializes every submit.
- Menu polish same day: square corners everywhere (active stroke uses
  the same rect as the fill — identical corners), tiles 125x110
  (-2%), tile labels 18pt + footer 16pt + pages 18/15pt (labels wrap
  two lines), custom topbar drag (title_bar(false) kills the built-in
  one: drag strip + WIN_POS/WIN_RECT + Window::current_pos).
- Regression pass (flicker gone, French auto-detect confirmed on the
  screenshot): Grid justified columns across the whole window width
  (egui Grid takes all available width — hence the gaps) AND the long
  footer line stretched the window. Replaced with `horizontal_wrapped`
  flow (tiles butt together, reflow on resize — the responsive pattern
  from egui's own layouts); window is now resizable with a 3-column
  default size. Label widgets default to hover sense BUT
  `selectable_labels` (on by default) upgrades them to click-and-drag:
  that ate tile clicks and started selections — menu style now sets
  `selectable_labels = false` plus explicit hover-sense labels.
  Header strip is the full visible 44px bar (whole topbar drags).
  Tile label zone resized for two 18pt lines.

- YES, the game language is readable: no PlayerPrefs key exists, but
  Unity logs `System Language: xx` at startup — it comes from
  `Application.systemLanguage`. Read live via
  `il2cpp_resolve_icall("UnityEngine.Application::get_systemLanguage")`
  (new 14th bridge symbol; raw icall, no invoke/unbox). Values follow
  the engine enum (En=10, Fr=14, De=15, It=20, Pt=27, Ru=29, Es=33).
- Chain: game → `LANG`/`LC_ALL` → English; Settings tile pins a
  language (override). Raw value is logged (`i18n: game language
  raw=…`) so the mapping stays verifiable.
- Caveat: this machine is `LANG=fr_FR` yet the game logged
  `System Language: en` (Steam runtime locale?) — expect English
  auto-detect here, pin Français in Settings.
- Menu: single window (header version/title/HELP, 3-col tile grid,
  red lock footer with wrapping text), vector Painter icons per mod
  (no font/asset dependency), active tile = green accent border.
- Per-mod windows removed (tile IS the control); status detail stays
  in the log. `Mod` gains `menu_label_key`/`menu_icon` (defaults keep
  third-party mods working); manager snapshots tiles lock-free.
- 7 languages, English fallback per key (partial translations never
  blank the UI).

## Full overlay victory (2026-10-07, screenshot: docs/overlay-proof.png)

- Text + flicker fixed together: `default_fonts` (glyphs exist) and
  record/replay without `ONE_TIME_SUBMIT` (steady frames).
- On screen simultaneously: framework status + manager windows, HUD
  Scout/Hide/Version windows (all draggable, checkboxes working),
  software cursor, AND the game's own HUD showing
  `VABYZ971 | CXSFM v0.1.0` (version tag through the game's renderer).
- F8 = UI + capture only; mods persist via manager checkboxes.

- NO DEFAULT FONTS: `egui = { default-features = false }` drops
  `default_fonts` → zero glyphs tessellated, shapes fine (white UV).
  The census hinted it (94 textured verts total — starved font).
  Fix: `features = ["default_fonts"]` (no version change). Visible
  proof in the binary: .so 2.5 MB → 4 MB (embedded TTFs).
- REPLAY vs ONE_TIME_SUBMIT: recording with ONE_TIME_SUBMIT then
  re-submitting is undefined (blank/garbage frames). Fresh records
  (dragging = dirty every frame) looked fine, replays flickered —
  exactly the reported symptom. Fix: empty usage flags + explicit
  reset before re-record (fence still serializes).

- Record/replay (flicker fix): the naive gate (skip submit when
  idle) caused FLICKER — skipped frames present WITHOUT ui because
  the game redraws every frame. Now: egui re-runs only when dirty
  (events, repaint due, never recorded, enabled-set changed); the
  last recorded command buffer is REPLAYED every present
  (fence-cycled). No flicker, near-zero idle CPU (no egui run, no
  tessellation on replay). UI-hide invalidates the recording (no
  stale frames). `pixels_per_point` flows tessellate→cmd_draw.
- Teardown waits bounded: `drop_swapchain`/`drop_device` fence-wait
  with 1 s timeout instead of `device_wait_idle` (an unbounded device
  wait can hang quit behind stuck queue work); destroy anyway on
  timeout (leak beats hang).
- Mesh census upgraded: first NON-EMPTY frame with glyph UV bbox
  (sane [0,1] = sampling/atlas suspect; insane = transform bug;
  zero textured verts = tessellation).
- Shader review (`egui-ash-renderer` frag/vert): `oColor * tex`
  with `(ONE, ONE_MINUS_SRC_ALPHA)` blend + premultiplied font texels
  is correct by construction; solids sample the same atlas at
  WHITE_UV, so working shapes prove texture/descriptor/pipeline —
  narrowing text failure to atlas CONTENT or UVs.
- Fan-noise assessment: overlay adds one fullscreen LOAD+STORE pass +
  per-frame CPU wakeups; the dominant load remains the game's own
  (uncapped menus). Repaint gating removes the idle cost; remaining
  noise under interaction is expected overlay cost.
- INSTANT OVERFLOW ABORT (menu-open freeze/crash, caught via
  Player.log stack: `Instant::add` inside `draw_frame` ← present):
  `repaint_delay` is `Duration::MAX` for static UI and plain
  `Instant + delay` panics → abort through foreign frames → SIGABRT.
  Fix: `checked_add` saturating to +1h (events bypass the gate
  anyway). RULE: no unbounded arithmetic on render/present paths —
  panics there can't unwind (abort), so every op must be
  checked/saturating by construction.
- Open text hypotheses if census shows glyph verts present: font
  sampler/descriptor staleness across atlas resizes (beyond the fixed
  use-after-free), or partial-update (`delta.pos`) path in the
  renderer.
- Boot-quiet rule (2026-10-07): `UI_VISIBLE` defaults to false AND
  every mod registers disarmed (scout included — no more auto-arm).
  Nothing on screen or in the log until O/F8. Mod windows use
  `default_pos` (initial stagger) instead of `anchor` so all are
  draggable by title bar; positions persist in ctx memory.

## In-game test protocol

1. Rebuild, `./tools/cxsfm-setup.sh`, empty the log, launch.
2. Expect init block → `input: ...` line with no keypress.
3. Press F8 → `hotkey F8 (...)` + mod lines. Report the `(src)`.

## Transitions de mods sur le thread de tick

- Symptôme : freeze à la **désactivation** du mod camera (image
  figée, son vivant) — `CameraMod::on_disable()` → `restore()` fait
  3 `FindObjectsOfType` + écritures Unity. Et si le tick est bloqué
  dans `on_update` pendant un churn de scène, le thread UI attend le
  verrou d'entrée partagé et gèle aussi (processus ensuite tué comme
  "crashé" : Player.log fini en `SIGINT`+`SIGTERM`, pas de segfault).
- Cause : `set_enabled` exécutait `on_enable`/`on_disable` de façon
  synchrone, sous verrou, depuis le thread UI/present.
- Règle : `on_enable`/`on_disable` **ne s'exécutent jamais sur le
  thread UI**. `set_enabled` n'enregistre que l'intention (`desired`,
  verrou d'état, retour immédiat) ; le tick (`update_all`) applique
  les transitions en attente avant `on_update` — une désactivation
  exécute donc `restore()` là où Unity est appelable. Entrées à
  verrous séparés état/code : le present ne touche que l'état (+ cache
  avec `try_lock`, stale servi sinon) et ne peut plus se garer
  derrière un mod enlisé dans Unity. Pendant le teardown le tick
  saute `update_all` : les transitions en attente ne s'appliquent
  pas — voulu (jamais d'IL2CPP pendant que Unity démonte son domaine).
- `apply()` sans snapshot : `apply()`/`do_boost()` réutilisent leur
  propre énumération pour rafraîchir les lignes UI (offset relu après
  écriture) au lieu de relancer `snapshot()` (~5 énumérations
  juste après avoir écrit). Durées loggées
  (`camera: apply/boost/snapshot/restore took X ms, N enumerations`).
