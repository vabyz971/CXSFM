//! Unity main-thread execution for mods (PHASE 1: infrastructure +
//! diagnostic probes + API; no mod migrates here yet).
//!
//! # Why
//! Every Unity call our mods make today leaves from the `cxsfm-tick`
//! thread, which is NOT Unity's main thread — a standing race with the
//! engine (freezes, SIGSEGV; see `docs/field-notes.md`). Community mods
//! (minhud) run their code every frame inside game-method hooks, i.e.
//! on the main thread. This module gives our mods the same: run
//! callbacks ONCE per frame, on the main thread.
//!
//! # Approach A: MethodInfo.methodPointer swap (tried first)
//! Unity message methods (`Update`, `LateUpdate`) are invoked by the
//! engine through `MethodInfo.methodPointer` (offset 0 of the
//! `MethodInfo` struct). Overwriting that word with our function
//! redirects the calls with a pure DATA write (same idea as the
//! `jmp [rip+off]` case in `src/render/vk.rs`) — no code patching.
//! HYPOTHESIS UNDER TEST, verified by the probes: if every slot stays
//! at 0 calls/s, the engine does not invoke through this path on this
//! build; that is information, not an error, and approach B (code hook
//! with iced-x86 relocation) is out of scope for this task.
//!
//! # ABI note (deliberate deviation from the first sketch)
//! `methodPointer` is the INVOKER (`InvokerMethod`), not the raw
//! method: `ret = f(method_ptr, method_info, this, params, ret)`. The
//! hook below implements exactly that 5-argument shape and relays the
//! return value; a 2-argument `(this, mi)` hook would corrupt the
//! register contract on x86-64 System V.
//!
//! # Threading
//! - Install runs on the tick thread (never present), throttled.
//! - Hooks run wherever the engine calls them (expected: main).
//! - The dispatcher never holds a Mutex while a callback runs, never
//!   blocks the main thread (`try_lock` everywhere on that path; a
//!   missed frame just skips), and never lets a panic cross FFI.
//! - During teardown the dispatcher relays to the original and runs
//!   nothing. Hook pointers are never restored at runtime (inert).

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Invoker ABI: what `MethodInfo.methodPointer` really points at.
type InvokerFn = unsafe extern "C" fn(
    *mut c_void,
    *const c_void,
    *mut c_void,
    *mut *mut c_void,
    *mut c_void,
) -> *mut c_void;

/// Max hooked methods (one slot each, own original pointer).
const MAX_SLOTS: usize = 6;
/// Max repeated callbacks.
const MAX_EVERY: usize = 8;
/// `post()` queue bound (oldest kept; push fails past it, never grows).
const POST_CAP: usize = 256;
/// Callback budget per frame (ms); the rest defers to next frame.
const FRAME_BUDGET_MS: u128 = 1;
/// Active-slot election: rate threshold…
const ELECT_HZ: u32 = 20;
/// …sustained over this many consecutive 1 s windows…
const ELECT_WINDOWS: u32 = 3;
/// Install attempts: at most this many, ~3 s apart.
const INSTALL_TRIES: u32 = 20;
const INSTALL_EVERY_MS: u64 = 3000;
/// Probe log cadence.
const PROBE_EVERY_MS: u64 = 5000;
/// Over-budget log throttle.
const BUDGET_LOG_EVERY_MS: u64 = 10_000;

/// One hook candidate, priority order (first qualifying slot wins and
/// latches). All run AFTER the original.
struct Candidate {
    label: &'static str,
    namespaces: &'static [&'static str],
    class: &'static str,
    method: &'static str,
}

/// Priority table (amended): engine-driven per-frame methods first,
/// game-specific before generic UGUI, probes last.
const CANDIDATES: &[Candidate] = &[
    Candidate {
        label: "DOTween",
        namespaces: &["DG.Tweening.Core"],
        class: "DOTweenComponent",
        method: "Update",
    },
    Candidate {
        label: "RtmpManager",
        namespaces: &["CarX.Street.RTMP"],
        class: "RtmpManager",
        method: "Update",
    },
    Candidate {
        label: "EventSystem",
        namespaces: &["UnityEngine.EventSystems"],
        class: "EventSystem",
        method: "Update",
    },
    Candidate {
        label: "CanvasScaler",
        namespaces: &["UnityEngine.UI"],
        class: "CanvasScaler",
        method: "Update",
    },
    // PROBE: ManualUpdate may run via delegate, not the engine —
    // calls==0 despite a live class is the tell (see probe log).
    Candidate {
        label: "CinemachineBrain.ManualUpdate",
        namespaces: &["Cinemachine", "Unity.Cinemachine"],
        class: "CinemachineBrain",
        method: "ManualUpdate",
    },
    // One instance per car: the per-frame dedup exists for this one.
    Candidate {
        label: "CarBehaviour",
        namespaces: &["Framework.Views.Cars"],
        class: "CarBehaviour",
        method: "Update",
    },
];

/// Discovery keywords (case-insensitive, class short name).
const DISCOVER_KEYWORDS: &[&str] = &[
    "hud", "speed", "tacho", "gauge", "camera", "drive", "car", "tween", "rtmp", "race",
];
/// Max discovery log lines (one-shot).
const DISCOVER_MAX_LINES: usize = 60;

/// Per-slot live state. Atomics only on the hook path; nothing here
/// blocks.
struct Slot {
    label: &'static str,
    /// `MethodInfo*` (0 = not installed).
    mi: AtomicUsize,
    /// Original `methodPointer` (0 = none).
    orig: AtomicUsize,
    installed: AtomicBool,
    /// Class resolved (skip re-resolution on retry).
    class_found: AtomicBool,
    /// Engine-vs-delegate note logged once.
    noted_silent: AtomicBool,
    /// Total hook calls (all instances, all frames).
    calls: AtomicU64,
    /// Current 1 s window: calls + window start (ms clock).
    window_calls: AtomicU64,
    window_start_ms: AtomicU64,
    /// Last closed window rate (calls/s, set by `poll`).
    last_rate: AtomicU32,
    /// Consecutive qualifying windows (election).
    good_windows: AtomicU32,
    /// Last call ran on the main thread (gettid == getpid).
    main_thread: AtomicBool,
    /// Cumulative callback time (µs, all frames).
    cb_micros: AtomicU64,
    /// Current window callback time (µs, reset with the window).
    cb_window_micros: AtomicU64,
    /// Last closed window callback time (ms, set by `poll`).
    last_cb_ms: AtomicU64,
    /// Frame dedup: last `Time.get_frameCount` seen (-1 = none).
    last_frame: AtomicI32,
    /// Fallback instance lock when the frame clock is missing.
    first_this: AtomicUsize,
    /// Adoption time of `first_this` (ms clock; 1 s re-lock).
    first_this_ms: AtomicU64,
    /// Last hook call (ms clock) for the 1 s fallback re-lock.
    last_call_ms: AtomicU64,
}

impl Slot {
    const fn new(label: &'static str) -> Self {
        Self {
            label,
            mi: AtomicUsize::new(0),
            orig: AtomicUsize::new(0),
            installed: AtomicBool::new(false),
            class_found: AtomicBool::new(false),
            noted_silent: AtomicBool::new(false),
            calls: AtomicU64::new(0),
            window_calls: AtomicU64::new(0),
            window_start_ms: AtomicU64::new(0),
            last_rate: AtomicU32::new(0),
            good_windows: AtomicU32::new(0),
            main_thread: AtomicBool::new(false),
            cb_micros: AtomicU64::new(0),
            cb_window_micros: AtomicU64::new(0),
            last_cb_ms: AtomicU64::new(0),
            last_frame: AtomicI32::new(-1),
            first_this: AtomicUsize::new(0),
            first_this_ms: AtomicU64::new(0),
            last_call_ms: AtomicU64::new(0),
        }
    }
}

static SLOTS: [Slot; MAX_SLOTS] = [
    Slot::new("DOTween"),
    Slot::new("RtmpManager"),
    Slot::new("EventSystem"),
    Slot::new("CanvasScaler"),
    Slot::new("CinemachineBrain.ManualUpdate"),
    Slot::new("CarBehaviour"),
];

/// Latched active slot (-1 = none). Only it runs callbacks.
static ACTIVE: AtomicI32 = AtomicI32::new(-1);
/// Panic kill-switch: set on the first callback panic, never cleared.
static CALLBACKS_OFF: AtomicBool = AtomicBool::new(false);
/// Cached `Time.get_frameCount` MethodInfo (0 = unresolved).
static FRAMECOUNT_MI: AtomicUsize = AtomicUsize::new(0);
/// Install/discovery/probe pacing (ms clock).
static INSTALL_TRIES_DONE: AtomicU32 = AtomicU32::new(0);
static LAST_TRY_MS: AtomicU64 = AtomicU64::new(0);
static LAST_PROBE_MS: AtomicU64 = AtomicU64::new(0);
static DISCOVERY_DONE: AtomicBool = AtomicBool::new(false);
static BUDGET_LOG_MS: AtomicU64 = AtomicU64::new(0);

/// Repeated callbacks: plain `fn` pointers (Copy — the guard never
/// survives into execution).
type CallbackEntry = Option<(fn(), &'static str)>;

struct Registry {
    fns: [CallbackEntry; MAX_EVERY],
    len: usize,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    fns: [None; MAX_EVERY],
    len: 0,
});

/// One-shot posted work.
struct PostItem {
    label: &'static str,
    f: Box<dyn FnOnce() + Send>,
}

static QUEUE: Mutex<Vec<PostItem>> = Mutex::new(Vec::new());

/// Process monotonic clock (ms). `OnceLock` read is an atomic load —
/// cheap enough for the hook hot path.
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn now_ms() -> u64 {
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// Whether the calling thread is the process main thread.
fn on_main_thread() -> bool {
    // SAFETY: trivial getters, no state.
    unsafe { libc::gettid() == libc::getpid() }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested; no Unity, no threads)
// ---------------------------------------------------------------------------

/// Frame dedup: run once per `Time.get_frameCount` value.
/// Pure state machine over the last seen frame.
fn should_run(last_frame: &mut i32, frame: i32) -> bool {
    if *last_frame == frame {
        false
    } else {
        *last_frame = frame;
        true
    }
}

/// Bounded push: oldest kept, `false` past capacity (never grows).
fn try_push<T>(v: &mut Vec<T>, item: T, cap: usize) -> bool {
    if v.len() >= cap {
        false
    } else {
        v.push(item);
        true
    }
}

/// Election step over one closed 1 s window: bump consecutive-good
/// counters, return the first slot latched at threshold (if any).
/// `rates`: (calls/s, on_main_thread) per slot, priority order.
fn elect(rates: &[(u32, bool)], goods: &mut [u32]) -> Option<usize> {
    for (i, (rate, main)) in rates.iter().enumerate() {
        if *rate >= ELECT_HZ && *main {
            goods[i] += 1;
        } else {
            goods[i] = 0;
        }
        if goods[i] >= ELECT_WINDOWS {
            return Some(i);
        }
    }
    None
}

/// Parse one `/proc/self/maps` line into (start, end, executable,
/// has_gameassembly_path). Pure — the executable-range check below is
/// unit-tested through this.
fn parse_maps_line(line: &str) -> Option<(usize, usize, bool, bool)> {
    let mut parts = line.split_whitespace();
    let range = parts.next()?;
    let perms = parts.next()?;
    let perm_bytes = perms.as_bytes();
    if perm_bytes.len() < 3 {
        return None;
    }
    let dash = range.find('-')?;
    let start = usize::from_str_radix(&range[..dash], 16).ok()?;
    let end = usize::from_str_radix(&range[dash + 1..], 16).ok()?;
    let executable = perm_bytes[0] == b'r' && perm_bytes[2] == b'x';
    let is_gameassembly = line.contains("GameAssembly");
    Some((start, end, executable, is_gameassembly))
}

/// Executable `GameAssembly.so` ranges (parsed live; installs are
/// rare so no caching). Empty = refuse everything.
fn game_executable_ranges() -> Vec<(usize, usize)> {
    let text = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some((s, e, x, ga)) = parse_maps_line(line) {
            if x && ga {
                out.push((s, e));
            }
        }
    }
    out
}

fn in_ranges(ranges: &[(usize, usize)], addr: usize) -> bool {
    ranges.iter().any(|&(s, e)| addr >= s && addr < e)
}

/// Hook trampoline address per slot index.
fn hook_addr(i: usize) -> usize {
    match i {
        0 => hook::<0> as usize,
        1 => hook::<1> as usize,
        2 => hook::<2> as usize,
        3 => hook::<3> as usize,
        4 => hook::<4> as usize,
        5 => hook::<5> as usize,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Public API (mods; phase 2 target)
// ---------------------------------------------------------------------------

/// True once a main-thread slot latched (callbacks actually run).
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst) >= 0
}

/// Queue one-shot work for the next main-thread frame. Bounded (256);
/// `false` when inactive, full, or contended (never blocks).
pub fn post(label: &'static str, f: impl FnOnce() + Send + 'static) -> bool {
    if !is_active() {
        return false;
    }
    let mut q = match QUEUE.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    try_push(&mut *q, PostItem { label, f: Box::new(f) }, POST_CAP)
}

/// Register a repeated per-frame callback (max 8). `false` when full
/// or contended (never blocks).
pub fn every_frame(label: &'static str, f: fn()) -> bool {
    let mut reg = match REGISTRY.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if reg.len >= MAX_EVERY {
        return false;
    }
    let at = reg.len;
    reg.fns[at] = Some((f, label));
    reg.len = at + 1;
    true
}

/// Per-slot diagnostic snapshot (future UI).
pub struct SlotStats {
    pub label: &'static str,
    pub installed: bool,
    pub active: bool,
    pub calls_total: u64,
    pub calls_per_sec: u32,
    pub main_thread: bool,
    pub cb_ms_total: u64,
}

/// Snapshot all slots (atomics only — safe at any rate).
pub fn stats() -> Vec<SlotStats> {
    let active = ACTIVE.load(Ordering::SeqCst);
    SLOTS
        .iter()
        .enumerate()
        .map(|(i, s)| SlotStats {
            label: s.label,
            installed: s.installed.load(Ordering::SeqCst),
            active: active == i as i32,
            calls_total: s.calls.load(Ordering::SeqCst),
            calls_per_sec: s.last_rate.load(Ordering::SeqCst),
            main_thread: s.main_thread.load(Ordering::SeqCst),
            cb_ms_total: s.cb_micros.load(Ordering::SeqCst) / 1000,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Dispatcher (runs on the hooked thread — expected: main)
// ---------------------------------------------------------------------------

/// Drain posted + repeated callbacks under a 1 ms/frame budget.
/// Locks are taken to SNAPSHOT work only, never held across a
/// callback; on contention or overrun the frame is skipped, never
/// stalled. `slot` accounts callback time for the probes.
fn dispatch(slot: &Slot) {
    if CALLBACKS_OFF.load(Ordering::SeqCst) || crate::game_shutting_down() {
        return;
    }
    let t0 = std::time::Instant::now();
    let over = || t0.elapsed().as_millis() > FRAME_BUDGET_MS;
    // Posted one-shots first.
    let items = match QUEUE.try_lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => Vec::new(),
    };
    let mut rest: Vec<PostItem> = Vec::new();
    let mut it = items.into_iter();
    for item in it.by_ref() {
        if over() {
            rest.push(item);
            break;
        }
        (item.f)();
    }
    rest.extend(it);
    if !rest.is_empty() {
        // Return the leftovers; on contention they drop (logged with
        // labels so a starving callback is identifiable).
        let labels: Vec<&str> = rest.iter().map(|i| i.label).collect();
        match QUEUE.try_lock() {
            Ok(mut q) => {
                let mut back = std::mem::take(&mut *q);
                back.splice(0..0, rest);
                *q = back;
            }
            Err(_) => {
                crate::log_line(&format!(
                    "mainthread: dropped {} deferred callbacks [{}] (queue contended)",
                    labels.len(),
                    labels.join(",")
                ));
            }
        }
        let now = now_ms();
        if now.saturating_sub(BUDGET_LOG_MS.load(Ordering::SeqCst)) >= BUDGET_LOG_EVERY_MS {
            BUDGET_LOG_MS.store(now, Ordering::SeqCst);
            crate::log_line("mainthread: frame budget overrun, remainder deferred");
        }
    }
    // Repeated callbacks (snapshotted fn pointers + length in one
    // guard — never held across execution).
    let (fns, len): ([CallbackEntry; MAX_EVERY], usize) = match REGISTRY.try_lock() {
        Ok(reg) => {
            let mut out: [CallbackEntry; MAX_EVERY] = [None; MAX_EVERY];
            for (i, slot) in reg.fns.iter().enumerate().take(reg.len) {
                if let Some(f) = slot {
                    out[i] = Some(*f);
                }
            }
            (out, reg.len)
        }
        Err(_) => ([None; MAX_EVERY], 0),
    };
    for (f, _label) in fns.iter().flatten().take(len) {
        if over() {
            let now = now_ms();
            if now.saturating_sub(BUDGET_LOG_MS.load(Ordering::SeqCst)) >= BUDGET_LOG_EVERY_MS {
                BUDGET_LOG_MS.store(now, Ordering::SeqCst);
                crate::log_line("mainthread: frame budget overrun, remainder deferred");
            }
            break;
        }
        f();
    }
    slot.cb_micros
        .fetch_add(t0.elapsed().as_micros() as u64, Ordering::SeqCst);
    slot.cb_window_micros
        .fetch_add(t0.elapsed().as_micros() as u64, Ordering::SeqCst);
}

/// Resolve + cache `Time.get_frameCount` (best effort, once found).
fn frame_count(api: &crate::il2cpp::Il2cppApi) -> Option<i32> {
    let mut mi = FRAMECOUNT_MI.load(Ordering::SeqCst);
    if mi == 0 {
        // SAFETY: attached thread (hook) or tick; resolution only.
        let klass = unsafe {
            crate::il2cpp::find_class(api, api.domain().ok()?, None, "UnityEngine", "Time")
        }?;
        // SAFETY: live class; pure metadata.
        mi = unsafe { crate::il2cpp::get_method(api, klass, "get_frameCount", 0) }? as usize;
        FRAMECOUNT_MI.store(mi, Ordering::SeqCst);
    }
    if mi == 0 {
        return None;
    }
    // SAFETY: main thread; static getter, no args; boxed int out.
    unsafe {
        let boxed = crate::il2cpp::invoke(
            api,
            mi as *mut c_void,
            std::ptr::null_mut(),
            &[],
        )?;
        let p = api.object_unbox?(boxed) as *const i32;
        if p.is_null() {
            return None;
        }
        Some(std::ptr::read_unaligned(p))
    }
}

/// The hook: After-phase relay + probes + gated dispatcher.
/// One monomorphization per slot so each keeps its own original.
///
/// # Safety
/// Called by the engine as an invoker (5-arg ABI — see module docs).
/// `mi`/`this`/`params`/`ret` are engine-owned for this call only.
/// Never panics: the dispatcher runs under `catch_unwind` and a panic
/// disables callbacks while still relaying to the original.
unsafe extern "C" fn hook<const I: usize>(
    method_ptr: *mut c_void,
    mi: *const c_void,
    this: *mut c_void,
    params: *mut *mut c_void,
    ret: *mut c_void,
) -> *mut c_void {
    let slot = &SLOTS[I];
    // 1. Relay to the original FIRST (After phase).
    let out = {
        let orig = slot.orig.load(Ordering::SeqCst);
        if orig == 0 {
            ret
        } else {
            // SAFETY: `orig` was verified executable at install; the
            // call honors the invoker ABI with engine-owned args.
            let f: InvokerFn = unsafe { std::mem::transmute::<usize, InvokerFn>(orig) };
            unsafe { f(method_ptr, mi, this, params, ret) }
        }
    };
    // 2. Probes (cheap atomics only).
    slot.calls.fetch_add(1, Ordering::SeqCst);
    slot.window_calls.fetch_add(1, Ordering::SeqCst);
    let now = now_ms();
    slot.last_call_ms.store(now, Ordering::SeqCst);
    slot.main_thread.store(on_main_thread(), Ordering::SeqCst);
    // 3. Only the latched slot runs callbacks.
    if ACTIVE.load(Ordering::SeqCst) != I as i32 || CALLBACKS_OFF.load(Ordering::SeqCst) {
        return out;
    }
    // 4. One pass per frame: frame clock, else first-instance lock
    //    with 1 s re-lock (multi-instance classes like CarBehaviour).
    // Direct call (no closure): this `unsafe fn` needs no block.
    let api_now = crate::il2cpp_api();
    let frame = match api_now {
        Some(api) => frame_count(api),
        None => None,
    };
    let mut last = slot.last_frame.load(Ordering::SeqCst);
    let run = match frame {
        Some(f) => should_run(&mut last, f),
        None => {
            let prev = slot.first_this.load(Ordering::SeqCst);
            let addr = this as usize;
            if prev == 0 || prev == addr {
                if prev == 0 {
                    slot.first_this.store(addr, Ordering::SeqCst);
                    slot.first_this_ms.store(now, Ordering::SeqCst);
                }
                true
            } else if now.saturating_sub(slot.first_this_ms.load(Ordering::SeqCst)) > 1000 {
                slot.first_this.store(addr, Ordering::SeqCst);
                slot.first_this_ms.store(now, Ordering::SeqCst);
                true
            } else {
                false
            }
        }
    };
    slot.last_frame.store(last, Ordering::SeqCst);
    if !run {
        return out;
    }
    // 5. Dispatcher, panic-quarantined (never across FFI).
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(slot);
    }));
    if r.is_err() {
        CALLBACKS_OFF.store(true, Ordering::SeqCst);
        crate::log_line(&format!(
            "mainthread: {}: callback panic, callbacks disabled (original still relayed)",
            slot.label
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Install + discovery + probes (tick thread)
// ---------------------------------------------------------------------------

/// Resolve one candidate class across its namespaces (first hit).
///
/// # Safety
/// Attached tick thread; resolution only.
unsafe fn resolve_candidate_class(
    api: &crate::il2cpp::Il2cppApi,
    domain: *mut c_void,
    cand: &Candidate,
) -> Option<*mut c_void> {
    for ns in cand.namespaces {
        // SAFETY: attached tick thread; `find_class` null-checks.
        if let Some(k) = unsafe { crate::il2cpp::find_class(api, domain, None, ns, cand.class) } {
            if !k.is_null() {
                return Some(k);
            }
        }
    }
    None
}

/// Install pending hooks (tick thread, throttled by `poll`).
fn try_install(api: &crate::il2cpp::Il2cppApi, domain: *mut c_void) {
    let ranges = game_executable_ranges();
    if ranges.is_empty() {
        crate::log_line("mainthread: install refused (no executable GameAssembly ranges)");
        return;
    }
    for (i, cand) in CANDIDATES.iter().enumerate() {
        let slot = &SLOTS[i];
        if slot.installed.load(Ordering::SeqCst) {
            continue;
        }
        // SAFETY: attached tick thread; resolution only.
        let klass = unsafe { resolve_candidate_class(api, domain, cand) };
        let klass = match klass {
            Some(k) => {
                slot.class_found.store(true, Ordering::SeqCst);
                k
            }
            None => continue, // class not loaded yet — retry next round
        };
        // SAFETY: live class; 0-arg message method.
        let mi = unsafe { crate::il2cpp::get_method(api, klass, cand.method, 0) };
        let mi = match mi {
            Some(m) if !m.is_null() => m as usize,
            _ => {
                crate::log_line(&format!(
                    "mainthread: {}: class found, {}(0) missing",
                    cand.label, cand.method
                ));
                continue;
            }
        };
        // Original pointer (first word of MethodInfo), must live in
        // executable GameAssembly pages — else refuse + log.
        // SAFETY: `mi` is a live MethodInfo; aligned 8-byte read while
        // the engine only reads (no writer yet).
        let orig = unsafe { std::ptr::read_volatile(mi as *const usize) };
        if orig == 0 || !in_ranges(&ranges, orig) {
            crate::log_line(&format!(
                "mainthread: {}: install refused (orig=0x{orig:x} outside executable GameAssembly)",
                cand.label
            ));
            continue;
        }
        slot.orig.store(orig, Ordering::SeqCst);
        slot.mi.store(mi, Ordering::SeqCst);
        // Atomic aligned 8-byte swap (x86-64: no tear); the engine may
        // be calling through the old pointer concurrently — a single
        // aligned store is the smallest possible race window, and the
        // old target stays valid regardless.
        // SAFETY: `mi` points at writable MethodInfo data (il2cpp heap,
        // RW); 8-byte aligned by construction.
        unsafe {
            (*(mi as *const AtomicUsize)).store(hook_addr(i), Ordering::SeqCst);
        }
        slot.installed.store(true, Ordering::SeqCst);
        crate::log_line(&format!(
            "mainthread: {} installed (orig=0x{orig:x}, mi=0x{mi:x})",
            cand.label
        ));
    }
}

/// One-shot updater discovery in Assembly-CSharp (log only, ≤60
/// lines): classes matching updater keywords with a 0-arg Update /
/// LateUpdate, plus the two named CarX callbacks when present.
///
/// # Safety
/// Attached tick thread; metadata reads only, capped.
unsafe fn discover(api: &crate::il2cpp::Il2cppApi, domain: *mut c_void) {
    let image = match unsafe { crate::il2cpp::find_image(api, domain, "Assembly-CSharp") } {
        Some(i) => i,
        None => {
            crate::log_line("mainthread: discovery skipped (no Assembly-CSharp image)");
            return;
        }
    };
    // SAFETY: live image; capped walk.
    let total = unsafe { crate::il2cpp::image_class_count(api, image) }.unwrap_or(0);
    let mut lines = 0usize;
    let cap = total.min(50_000);
    for idx in 0..cap {
        if lines >= DISCOVER_MAX_LINES {
            crate::log_line("mainthread: discovery capped at 60 lines");
            break;
        }
        // SAFETY: index under the counted total.
        let klass = match unsafe { crate::il2cpp::image_class(api, image, idx) } {
            Some(k) => k,
            None => continue,
        };
        // SAFETY: live class; names are process-lifetime.
        let short = match unsafe { crate::il2cpp::class_name(api, klass) } {
            Some(n) => n,
            None => continue,
        };
        let lower = short.to_lowercase();
        let keyword = DISCOVER_KEYWORDS.iter().any(|k| lower.contains(k));
        let named = short == "RearRaceCamera" || short == "CamerasManager";
        if !keyword && !named {
            continue;
        }
        // SAFETY: live class; capped method walk.
        for m in unsafe { crate::il2cpp::class_methods(api, klass) } {
            // SAFETY: live MethodInfo; pure metadata.
            let name = unsafe { crate::il2cpp::method_name(api, m) }.unwrap_or_default();
            let argc = unsafe { crate::il2cpp::method_param_count(api, m) }.unwrap_or(99);
            if named {
                if (short == "RearRaceCamera" && name == "PostPipelineStageCallback")
                    || (short == "CamerasManager" && name == "ForceUpdate")
                {
                    crate::log_line(&format!(
                        "mainthread: found {short}::{name} ({argc} args)"
                    ));
                    lines += 1;
                }
                continue;
            }
            if (name == "Update" || name == "LateUpdate") && argc == 0 {
                crate::log_line(&format!("mainthread: updater candidate {short}::{name}"));
                lines += 1;
                break;
            }
        }
    }
    crate::log_line(&format!(
        "mainthread: discovery done ({total} classes scanned, {lines} lines)"
    ));
}

/// Probe tick: per-slot 5 s lines, window rollover + active election.
/// Tick thread; atomics + log only.
fn probe_and_select() {
    let now = now_ms();
    let mut rates: [(u32, bool); MAX_SLOTS] = [(0, false); MAX_SLOTS];
    let mut goods = [0u32; MAX_SLOTS];
    for (i, slot) in SLOTS.iter().enumerate() {
        // Rollover check first (uses the slot's own window clock).
        let start = slot.window_start_ms.load(Ordering::SeqCst);
        if now.saturating_sub(start) >= 1000 {
            let n = slot.window_calls.swap(0, Ordering::SeqCst) as u32;
            let cb_us = slot.cb_window_micros.swap(0, Ordering::SeqCst);
            slot.window_start_ms.store(now, Ordering::SeqCst);
            slot.last_rate.store(n, Ordering::SeqCst);
            slot.last_cb_ms.store(cb_us / 1000, Ordering::SeqCst);
        }
        // Preserve election progress across polls.
        goods[i] = slot.good_windows.load(Ordering::SeqCst);
        rates[i] = (
            slot.last_rate.load(Ordering::SeqCst),
            slot.main_thread.load(Ordering::SeqCst),
        );
    }
    if ACTIVE.load(Ordering::SeqCst) < 0 {
        if let Some(i) = elect(&rates, &mut goods) {
            for (s, g) in SLOTS.iter().zip(goods.iter()) {
                s.good_windows.store(*g, Ordering::SeqCst);
            }
            ACTIVE.store(i as i32, Ordering::SeqCst);
            crate::log_line(&format!(
                "mainthread: active slot latched: {} (>=20/s on main x3s)",
                SLOTS[i].label
            ));
        } else {
            for (s, g) in SLOTS.iter().zip(goods.iter()) {
                s.good_windows.store(*g, Ordering::SeqCst);
            }
        }
    }
    for slot in SLOTS.iter() {
        if !slot.installed.load(Ordering::SeqCst) {
            continue;
        }
        let thread = if slot.main_thread.load(Ordering::SeqCst) {
            "main"
        } else {
            "other"
        };
        crate::log_line(&format!(
            "mainthread: {} calls={}/s thread={thread} cb_ms={}",
            slot.label,
            slot.last_rate.load(Ordering::SeqCst),
            slot.last_cb_ms.load(Ordering::SeqCst),
        ));
        // Engine-vs-delegate note: installed for a while, class
        // resolved, yet zero engine calls — likely delegate-driven.
        if slot.calls.load(Ordering::SeqCst) == 0
            && !slot.noted_silent.load(Ordering::SeqCst)
            && INSTALL_TRIES_DONE.load(Ordering::SeqCst) >= 3
        {
            slot.noted_silent.store(true, Ordering::SeqCst);
            crate::log_line(&format!(
                "mainthread: {} src=delegate? (installed, class live, 0 engine calls)",
                slot.label
            ));
        }
    }
}

/// Tick entry: discovery once, install retries (~3 s, ≤20), probes
/// (~5 s). Cheap atomics when idle; never blocks.
pub fn poll() {
    let api = match crate::il2cpp_api() {
        Some(a) => a,
        None => return,
    };
    // SAFETY: attached tick thread; liveness probe only.
    let domain = match unsafe { crate::il2cpp::domain_checked(api) } {
        Some(d) => d,
        None => return,
    };
    let now = now_ms();
    if !DISCOVERY_DONE.swap(true, Ordering::SeqCst) {
        // SAFETY: attached tick thread; metadata reads, capped.
        unsafe { discover(api, domain) };
    }
    if INSTALL_TRIES_DONE.load(Ordering::SeqCst) < INSTALL_TRIES
        && now.saturating_sub(LAST_TRY_MS.load(Ordering::SeqCst)) >= INSTALL_EVERY_MS
    {
        LAST_TRY_MS.store(now, Ordering::SeqCst);
        INSTALL_TRIES_DONE.fetch_add(1, Ordering::SeqCst);
        try_install(api, domain);
    }
    if now.saturating_sub(LAST_PROBE_MS.load(Ordering::SeqCst)) >= PROBE_EVERY_MS {
        LAST_PROBE_MS.store(now, Ordering::SeqCst);
        probe_and_select();
    }
}

#[cfg(test)]
mod tests {
    use super::{DISCOVER_KEYWORDS, POST_CAP, elect, parse_maps_line, should_run, try_push};

    #[test]
    fn candidates_priority_table() {
        // Amended order: DOTween first, no LateUpdate anywhere.
        assert_eq!(super::CANDIDATES.len(), 6);
        assert_eq!(super::CANDIDATES[0].class, "DOTweenComponent");
        assert_eq!(super::CANDIDATES[0].method, "Update");
        assert_eq!(super::CANDIDATES[1].class, "RtmpManager");
        assert_eq!(super::CANDIDATES[4].method, "ManualUpdate");
        assert_eq!(super::CANDIDATES[5].class, "CarBehaviour");
        for c in super::CANDIDATES {
            assert_ne!(c.method, "LateUpdate");
            assert!(!c.namespaces.is_empty());
            assert!(!c.class.is_empty());
        }
        // Slots mirror the table 1:1 (hook index == candidate index).
        for (s, c) in super::SLOTS.iter().zip(super::CANDIDATES.iter()) {
            assert_eq!(s.label, c.label);
        }
        for k in ["hud", "speed", "tacho", "gauge", "camera", "drive", "car", "tween", "rtmp", "race"] {
            assert!(DISCOVER_KEYWORDS.contains(&k));
        }
    }

    #[test]
    fn frame_dedup_runs_once() {
        let mut last = -1;
        assert!(should_run(&mut last, 100));
        assert!(!should_run(&mut last, 100));
        assert!(!should_run(&mut last, 100));
        assert!(should_run(&mut last, 101));
        assert_eq!(last, 101);
    }

    #[test]
    fn bounded_queue_never_grows() {
        let mut v: Vec<u32> = Vec::new();
        for i in 0..POST_CAP as u32 {
            assert!(try_push(&mut v, i, POST_CAP));
        }
        assert!(!try_push(&mut v, 999, POST_CAP));
        assert_eq!(v.len(), POST_CAP);
    }

    #[test]
    fn election_latches_first_qualifier() {
        // Slot 1 qualifies 3 windows straight; slot 0 never does.
        let mut goods = [0u32; 6];
        let mut won = None;
        for _ in 0..3 {
            won = elect(&[(5, true), (25, true), (0, false), (0, false), (0, false), (0, false)], &mut goods);
        }
        assert_eq!(won, Some(1));
        // Non-main rate resets the streak.
        let mut goods = [0u32; 6];
        elect(&[(25, false), (0, false), (0, false), (0, false), (0, false), (0, false)], &mut goods);
        elect(&[(25, true), (0, false), (0, false), (0, false), (0, false), (0, false)], &mut goods);
        assert_eq!(goods[0], 1);
    }

    #[test]
    fn maps_line_parsing() {
        let line = "7f3a9c000000-7f3a9c021000 r-xp 00000000 00:00 0  /memfd:GameAssembly.so (deleted)";
        let (s, e, x, ga) = parse_maps_line(line).unwrap();
        assert_eq!((s, e), (0x7f3a9c000000, 0x7f3a9c021000));
        assert!(x && ga);
        let line2 = "7fff12345000-7fff12346000 rw-p 00000000 00:00 0  [stack]";
        let (_, _, x2, ga2) = parse_maps_line(line2).unwrap();
        assert!(!x2 && !ga2);
    }
}
