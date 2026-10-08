//! CXSFM - CarX Street Framework Mod
//!
//! Copyright (c) 2026 CXSFM Team
//!
//! License: MIT
//!
//! Project: CarX Street Framework Mod (CXSFM)
//!
//! This framework enables injection of a graphical interface into CarX Street games,
//! using egui for UI rendering and glam for graphics composition. It respects game
//! anti-cheat policies by focusing solely on cosmetic/UI modifications and camera control.
//!
//! # Features
//!
//! - **Cross-platform**: Compiles to `.dll` (Windows), `.so` (Linux), and `.dylib` (macOS)
//! - **Modern Rust**: Uses `std::sync::LazyLock` instead of legacy lazy_static
//! - **Safe Memory Access**: Encapsulates unsafe operations in `memory.rs` with careful bounds checking
//! - **EGUI Integration**: Provides a headless UI layer compatible with egui >= 0.29
//! - **Mod Management**: Trait-based architecture for modular mod development
//!
//! # Installation
//!
//! ```bash
//! # Build for Windows
//! cargo build --target x86_64-pc-windows-msvc --release
//!
//! # Build for Linux
//! cargo build --target x86_64-unknown-linux-gnu --release
//!
//! # Build for macOS
//! cargo build --target x86_64-apple-darwin --release
//! ```
//!
//! # Injection Guide
//!
//! 1. **Compile** the framework to obtain `libcxsfm.so` / `libcxsfm.dll` / `libcxsfm.dylib`
//! 2. **Inject** the shared library into your game process using a hook mechanism (e.g., DLL injection on Windows, ptrace on Linux, or dynamic loading on macOS)
//! 3. **Initialize** the framework in your mod's entry point
//! 4. **Access** the provided traits (`Mod`, `ModManager`) to register your UI and camera modules
//!
//! # Architecture
//!
//! - `src/lib.rs`: Main entry point with `#[ctor]` macro for initialization
//! - `src/memory.rs`: Memory scanning and manipulation (AOB scanning, process memory introspection)
//! - `src/mod_api.rs`: Core trait definitions and manager for mod plugins
//! - `src/mods/fpv_camera.rs`: Example implementation for FPV camera mod
//!
//! # Contributing
//!
//! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on developing new mods.
//!
//! # License
//!
//! MIT - See LICENSE file for details.
//!
pub mod memory;
pub mod mod_api;
pub mod mods;
pub mod il2cpp;
pub mod hotkey;
pub mod render;
pub mod unity;
pub mod i18n;
pub mod menu;
pub mod ui;
/// Implicit Vulkan layer (Linux): loader-driven present routing.
/// Only exists where Vulkan does; see `layer.rs`.
#[cfg(target_os = "linux")]
pub mod layer;

// Private: link-time symbol shims, no public API (see shim.rs).
// Must stay compiled in release builds: referenced via --wrap.
mod shim;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// Delay after library load before probing game readiness.
/// Unity prints its `UnityMemory` config lines during this phase; touching
/// game memory earlier is the classic LD_PRELOAD segfault cause.
const INIT_DELAY: Duration = Duration::from_secs(10);
/// How long `wait_for_game_ready` polls before giving up.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// Interval between readiness probes.
const READY_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Minimum readable regions before the stability check applies.
/// Calibrated empirically: a plain shell maps ~57 readable regions,
/// python ~51, while a Unity game maps hundreds during startup.
/// The absolute value only filters noise — the real signal is the
/// count *stabilizing* (engine finished mapping), see below.
#[cfg(target_os = "linux")]
const MIN_READY_REGIONS: usize = 40;
/// Consecutive stable polls required to declare the game ready.
#[cfg(target_os = "linux")]
const STABLE_POLLS_REQUIRED: u32 = 2;

/// Set to `true` once background initialization finished (success or not).
static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Resolved IL2CPP API of the host game, if it provides one.
///
/// Populated once during [`deferred_init`]; mods query it via
/// [`il2cpp_api`] and must handle `None` (non-Unity process, or scripting
/// not up yet). `Il2cppApi` is shareable: it only holds function pointers.
static IL2CPP: std::sync::OnceLock<il2cpp::Il2cppApi> = std::sync::OnceLock::new();

/// Access the host game's IL2CPP API, if resolved.
///
/// Returns `None` outside Unity IL2CPP processes. Mods calling into IL2CPP
/// must first ensure their thread is attached (the init and tick threads
/// already are; see [`il2cpp`] module docs for the rule).
#[inline]
pub fn il2cpp_api() -> Option<&'static il2cpp::Il2cppApi> {
    IL2CPP.get()
}

/// Returns `true` once the background init thread has finished.
///
/// Mods and external tooling can poll this instead of guessing timing.
#[inline]
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::SeqCst)
}

/// Library entry point, executed automatically by the dynamic loader
/// (`LD_PRELOAD` on Linux, DLL injection on Windows,
/// `DYLD_INSERT_LIBRARIES` on macOS) — *before* the game's `main()` runs.
///
/// # Load-time safety
/// This executes while the game is only partially initialized, so it must
/// **never** touch game memory, do I/O, or block. It only spawns a detached
/// background thread; all real work (including logging) happens later in
/// [`deferred_init`], after the engine has started.
#[ctor::ctor]
fn cxsym_init() {
    // Intentionally ignore spawn failure: without the thread there is
    // simply no framework, but the game must keep running.
    let _ = std::thread::Builder::new()
        .name("cxsfm-init".into())
        .spawn(deferred_init);
}

/// Background initialization, runs on its own thread seconds after load.
fn deferred_init() {
    // Give the engine time to map its modules and init its allocators.
    // Override with `CXSF_DELAY_SECS=N` (e.g. `=2` when injecting into an
    // already-running game via the watchdog — the engine is up already).
    // Invalid values fall back to the default; never panics.
    let delay = std::env::var("CXSF_DELAY_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(INIT_DELAY);
    log_line("init thread started, waiting for engine...");
    std::thread::sleep(delay);

    if wait_for_game_ready() {
        log_line("game ready, resolving IL2CPP");
        match il2cpp::resolve() {
            Ok(api) => {
                // SAFETY: fresh domain handle on our own init thread;
                // attach before any further IL2CPP use (see il2cpp docs).
                unsafe {
                    match api.domain() {
                        Ok(domain) => {
                            let _ = api.attach_current_thread(domain);
                            match api.assembly_count(domain) {
                                Ok(n) => log_line(&format!(
                                    "il2cpp: domain + attach ok, {} assemblies",
                                    n
                                )),
                                Err(e) => log_line(&format!(
                                    "il2cpp: assemblies query failed: {}",
                                    e
                                )),
                            }
                            // Publish for mods; silently keeps the old
                            // value if somehow already set (never happens).
                            let _ = IL2CPP.set(api);
                            // Game language for the menu (icall; logs raw
                            // value so the SystemLanguage mapping stays
                            // verifiable). Thread is attached already.
                            let lang = unity::system_language_raw(IL2CPP.get().unwrap());
                            i18n::note_detected(lang);
                        }
                        Err(e) => log_line(&format!(
                            "il2cpp: scripting domain not up yet: {} (mods still registered)",
                            e
                        )),
                    }
                }
            }
            Err(e) => log_line(&format!("il2cpp: unavailable ({})", e)),
        }
        log_line("installing render hook");
        // A panic anywhere in the hook install must never take the game
        // down with it (faults are already contained by sigguard; this
        // covers logic panics like poisoned mutexes or arithmetic).
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(render::install)) {
            Ok(Ok(backend)) => log_line(&format!("render hook installed: {:?}", backend)),
            Ok(Err(e)) => log_line(&format!("render hook unavailable: {} (UI stays headless)", e)),
            Err(_) => log_line("render: install panicked (caught), continuing headless"),
        }
        log_line("registering mods");
        // Register bundled example mods here. Third-party mods hook in
        // the same way via `mod_api::register_mod`.
        // NOTE: FPV is NOT registered (user-deactivated while HUD work
        // goes on). The parked module stays compiled in `mods::fpv_camera`
        // for the render-hook future.
        mods::hud_scout::register();
        mods::hud_hide::register();
        mods::hud_text::register();
        mods::inspector::register();
        mods::gamelog::register();
        // NOTE: video is PARKED — some setters crash the game on write
        // (reads were fine). The module stays compiled in
        // `mods::video` until the faulting setter is isolated.
        // mods::video::register();
        log_line("mods registered, starting tick thread");
        spawn_tick_thread();
        headless_ui_check();
    } else {
        log_line("game NOT ready after timeout, mods NOT registered");
    }

    INITIALIZED.store(true, Ordering::SeqCst);
    log_line("init finished");
}

/// Poll until the game process looks fully started, or timeout.
///
/// On Linux this watches `/proc/self/maps`: a Unity build maps hundreds
/// of regions while starting, so the engine is considered up once the
/// readable-region count is substantial *and stable* across consecutive
/// polls. On other platforms we fall back to the initial sleep only.
fn wait_for_game_ready() -> bool {
    #[cfg(target_os = "linux")]
    {
        let deadline = std::time::Instant::now() + READY_TIMEOUT;
        let mut last_count = 0usize;
        let mut stable_polls = 0u32;
        while std::time::Instant::now() < deadline {
            match memory::enumerate_memory_regions() {
                Ok(regions) => {
                    let readable = regions.iter().filter(|r| r.readable).count();
                    if readable >= MIN_READY_REGIONS && readable == last_count {
                        stable_polls += 1;
                        if stable_polls >= STABLE_POLLS_REQUIRED {
                            return true;
                        }
                    } else {
                        // Still mapping (or below noise floor): reset.
                        stable_polls = 0;
                    }
                    last_count = readable;
                }
                // `/proc/self/maps` unreadable (hardened kernel, anti-cheat):
                // stop polling and let mods decide at call time.
                Err(_) => return false,
            }
            std::thread::sleep(READY_POLL_INTERVAL);
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Resolve the framework log file path, in priority order:
/// 1. `$CXSF_LOG` when set (explicit user choice, e.g. Steam launch options).
/// 2. `cxsfm.log` next to our own loaded `.so` — proven container-visible
///    because the loader just mapped it from there (unlike the container's
///    private `/tmp`, which swallows logs invisibly).
/// 3. `/tmp/cxsfm.log` as a last resort.
///
/// The result is cached: the path cannot usefully change mid-process.
pub fn log_path() -> std::path::PathBuf {
    static CACHED: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    CACHED
        .get_or_init(|| {
            if let Ok(p) = std::env::var("CXSF_LOG") {
                if !p.is_empty() {
                    return std::path::PathBuf::from(p);
                }
            }
            if let Some(dir) = own_module_dir() {
                let candidate = dir.join("cxsfm.log");
                // Validate once: actually open for append, so a weird
                // mount (read-only, noexec quirks) falls through below.
                if std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&candidate)
                    .is_ok()
                {
                    return candidate;
                }
            }
            std::path::PathBuf::from("/tmp/cxsfm.log")
        })
        .clone()
}

/// Directory containing our own loaded module, via address-to-module
/// lookup on a known function address. Returns `None` when the lookup
/// fails (non-dynamic loading, exotic loaders).
#[cfg(unix)]
fn own_module_dir() -> Option<std::path::PathBuf> {
    // SAFETY: `dladdr` with a valid in-module function address and a
    // valid out-pointer; read-only query, no side effects.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        let addr = is_initialized as usize as *const libc::c_void;
        if libc::dladdr(addr, &mut info) == 0 || info.dli_fname.is_null() {
            return None;
        }
        let path = std::ffi::CStr::from_ptr(info.dli_fname)
            .to_string_lossy()
            .into_owned();
        std::path::PathBuf::from(path).parent().map(|p| p.to_path_buf())
    }
}

/// Windows twin: `GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS)`
/// on a known function address, then `GetModuleFileNameW`.
///
/// Syntax-checked on Linux; first type-check happens on a Windows build.
#[cfg(windows)]
fn own_module_dir() -> Option<std::path::PathBuf> {
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleFileNameW, GetModuleHandleExW,
    };
    // SAFETY: valid in-module address; stack buffer with explicit length;
    // both return values checked before use.
    unsafe {
        let addr = is_initialized as usize as *const std::ffi::c_void;
        let mut hmod: isize = 0;
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            addr as *const u16,
            &mut hmod,
        ) == 0
        {
            return None;
        }
        let mut buf = [0u16; 260];
        let len = GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::PathBuf::from(path).parent().map(|p| p.to_path_buf())
    }
}

/// Append one line to the framework log file (see [`log_path`]).
///
/// Every line carries a seconds-since-load timestamp (`[+83.2s]`) so key
/// presses, scene loads and restarts can be correlated after the fact —
/// essential now that two sessions may share one log file. Lines are also
/// mirrored to stderr so they show up in Steam's own logs. This is the
/// out-of-game verification channel: until the graphics hook renders
/// windows on screen, the log proves the library loaded, the init thread
/// ran, and which mods registered. All I/O failures are silently
/// ignored — logging must never break the game.
///
/// Never call this from the `#[ctor]` itself (see its safety notes).
fn log_line(msg: &str) {
    use std::io::Write as _;
    /// Process-load instant: the zero of every log timestamp.
    static T0: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    // Single `write_all` (not `writeln!`, which may split into several
    // syscalls): with O_APPEND one syscall = one atomic record, so lines
    // from concurrent writers can't interleave mid-line.
    let line = format!("[cxsfm +{:6.1}s] {}\n", T0.elapsed().as_secs_f32(), msg);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = f.write_all(line.as_bytes());
    }
    eprint!("{}", line);
}

/// Framework-level key edge via Unity's `Input.GetKey`.
///
/// The SDL path (`hotkey::poll_toggle_edge`) proved deaf inside the game
/// process, so the tick ORs both sources. This one owns a small dedicated
/// cache — framework input must work even with zero mods enabled — and
/// logs its resolve outcome exactly once for in-game diagnosis.
///
/// Returns false outside Unity (no API yet).
fn poll_framework_input_edge(keycode: i32) -> bool {
    static CACHE: LazyLock<Mutex<Option<unity::UnityCache>>> =
        LazyLock::new(|| Mutex::new(None));
    static LOGGED: AtomicBool = AtomicBool::new(false);
    static FAILED_LOGGED: AtomicBool = AtomicBool::new(false);
    let api = match IL2CPP.get() {
        Some(a) => a,
        None => return false,
    };
    // NOTE: bind the snapshot to a variable FIRST so the `MutexGuard`
    // temporary drops at the statement boundary. Matching directly on
    // `CACHE.lock()...` extends the guard through the whole `match`, and
    // the `None` arm below re-locks the same mutex — a deterministic
    // self-deadlock that froze the tick thread on its very first input
    // poll in-game (found via the thread's futex address in /proc).
    let cached: Option<unity::UnityCache> = CACHE.lock().unwrap().as_ref().copied();
    let cache = match cached {
        Some(c) => c,
        None => {
            // SAFETY: tick thread is IL2CPP-attached (see
            // `spawn_tick_thread`); resolve-once, reuse afterwards.
            // `domain_checked` parks IL2CPP use if the runtime is
            // dying (teardown) instead of crashing in its corpse.
            let resolved = unsafe {
                crate::il2cpp::domain_checked(api)
                    .and_then(|domain| unity::init(api, domain))
            };
            match resolved {
                Some(c) => {
                    *CACHE.lock().unwrap() = Some(c);
                    c
                }
                None => {
                    // Resolve keeps retrying every poll; log once so a
                    // permanently failing resolve is visible, not silent.
                    // SDL/X11 states ride along — they don't need Unity.
                    if !FAILED_LOGGED.swap(true, Ordering::SeqCst) {
                        log_line(&format!(
                            "input: unity cache resolve failed, sdl {}, x11 {} (retrying)",
                            if hotkey::sdl_available() { "live" } else { "dead" },
                            if hotkey::x11_available() { "live" } else { "dead" },
                        ));
                    }
                    return false;
                }
            }
        }
    };
    if !LOGGED.swap(true, Ordering::SeqCst) {
        log_line(&format!(
            "input: unity cache resolved (GetKey {}), sdl {}, x11 {}",
            if cache.m_get_key.is_null() {
                "unavailable — SDL/X11 fallback only"
            } else {
                "ready"
            },
            if hotkey::sdl_available() { "live" } else { "dead" },
            if hotkey::x11_available() { "live" } else { "dead" },
        ));
    }
    // SAFETY: same attached thread + cached methods as above.
    unsafe { hotkey::poll_unity_edge(api, &cache, keycode) == Some(true) }
}

/// Whether the game is tearing down.
///
/// Cross-platform shim: only Linux tracks Vulkan object lifetimes yet.
/// Elsewhere the tick keeps polling (Windows/macOS teardown hardening
/// is future work alongside their render hooks).
#[cfg(target_os = "linux")]
#[inline]
fn game_shutting_down() -> bool {
    crate::layer::shutting_down()
}
#[cfg(not(target_os = "linux"))]
#[inline]
fn game_shutting_down() -> bool {
    false
}

/// Spawn the ~60 Hz tick thread driving `on_update` on all enabled mods.
///
/// Detached on purpose: it lives as long as the game process. `dt` is
/// clamped so a hitch (alt-tab, loading screen) can't destabilize mods.
/// Also attaches this thread to IL2CPP (mods may call into it from
/// `on_update`) and polls toggles at ~10 Hz from three independent
/// sources: SDL (`hotkey`), Unity `Input` (this module) and the X server
/// itself (`hotkey` X11 path — the Wayland-session-proof one). O drives
/// the HUD mods, F9 the UI input capture, F8 both at once.
fn spawn_tick_thread() {
    let _ = std::thread::Builder::new()
        .name("cxsfm-tick".into())
        .spawn(|| {
            if let Some(api) = IL2CPP.get() {
                // SAFETY: domain re-fetched on this thread; attach is
                // idempotent per thread, failure just skips IL2CPP use.
                unsafe {
                    if let Ok(domain) = api.domain() {
                        let _ = api.attach_current_thread(domain);
                    }
                }
            }
            let mut last = std::time::Instant::now();
            let mut tick: u64 = 0;
            // Pre-start the X11 event thread (grab window ready before
            // the first F8/F9 press — no activation delay).
            hotkey::ensure_capture_thread();
            loop {
                std::thread::sleep(Duration::from_millis(16));
                let dt = last.elapsed().as_secs_f32().min(0.1);
                last = std::time::Instant::now();
                tick += 1;
                // During teardown (swapchains/devices/instances all
                // gone) Unity kills its scripting domain: ANY IL2CPP
                // call can segfault instead of erroring (caught live
                // via Player.log). SDL/X11/log paths stay alive —
                // only IL2CPP-touching work stops below.
                let down = game_shutting_down();
                // Render heartbeat: proves presents keep routing through
                // us long after init (a stuck count with a live game
                // means present routing died silently).
                if tick % 900 == 0 {
                    log_line(&format!(
                        "render: {} presents ({} overlay, {} plain)",
                        render::frame_count(),
                        render::overlay_drew(),
                        render::overlay_skipped()
                    ));
                }
                if tick % 6 == 0 {
                    // F8 is the ONLY hotkey: UI visibility + input
                    // capture, aligned in one direction (shown/captured
                    // or hidden/released) across three independent input
                    // sources (SDL, Unity Input, X11 — first edge wins).
                    // It deliberately does NOT touch mods: menu
                    // visibility and mod effects are independent —
                    // closing the menu hides windows but running mods
                    // keep working (their state persists). Mods toggle
                    // from the manager window while the UI is open.
                    let sdl_combo = hotkey::poll_scancode_edge(hotkey::COMBO_SCANCODE_F8)
                        == Some(true);
                    // No Unity polling during teardown (see above).
                    let unity_combo = if down {
                        false
                    } else {
                        poll_framework_input_edge(crate::unity::KEYCODE_F8)
                    };
                    let x11_combo =
                        hotkey::poll_x11_key_edge(hotkey::X11_KEYCODE_F8) == Some(true);
                    if sdl_combo || unity_combo || x11_combo {
                        let src = if unity_combo {
                            "unity"
                        } else if sdl_combo {
                            "sdl"
                        } else {
                            "x11"
                        };
                        let target = !hotkey::ui_visible();
                        if hotkey::ui_visible() != target {
                            hotkey::toggle_ui();
                        }
                        hotkey::set_captured(target);
                        log_line(&format!(
                            "hotkey F8 ({src}): UI + capture {} (mods untouched)",
                            if target { "ON" } else { "OFF" }
                        ));
                    }
                    // Pinned tool windows stay VISIBLE with the menu
                    // closed, but input stays with the game: capture
                    // follows the menu only. A pinned window that kept
                    // capture froze all game input (indistinguishable
                    // from a crash), so pinned == view-only until the
                    // menu reopens. See `loader::draw_ui_all`.
                    let want_capture = hotkey::ui_visible();
                    if hotkey::is_captured() != want_capture {
                        hotkey::set_captured(want_capture);
                    }
                }
                // Mod updates touch IL2CPP: skip them entirely during
                // teardown (see above). The tick itself (heartbeat,
                // SDL/X11 polls) keeps running.
                if !down {
                    mod_api::update_all_mods(dt);
                }
            }
        });
}

/// Run every UI code path once without a renderer.
///
/// Builds a throwaway egui frame, draws all enabled mods plus the status
/// and manager windows, and discards the output. This validates the UI
/// code (a panic here is caught and logged instead of killing the game)
/// and proves the whole draw pipeline works before the graphics hook
/// exists to show it on screen.
fn headless_ui_check() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        mod_api::draw_ui_all(&ctx, true);
        mod_api::draw_manager_ui(&ctx);
        mod_api::draw_status_ui(&ctx);
        let _ = ctx.end_pass();
    }));
    log_line(&format!(
        "headless UI check: {}",
        if result.is_ok() { "ok" } else { "PANIC" }
    ));
}