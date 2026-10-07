//! Linux backend: intercept `vkQueuePresentKHR`.
//!
//! # Method: GOT patching (no disassembler needed)
//!
//! Unity links the Vulkan loader dynamically, so its present calls go
//! through the PLT → GOT. We locate the game's own executable mappings,
//! parse its `PT_DYNAMIC` (`DT_JMPREL` relocations), find the GOT slot of
//! the target symbol, and swap the address (page permissions flipped with
//! `mprotect` around the write, original permissions restored after).
//!
//! Three layers, tried in order:
//!
//! * **(a) direct**: patch the exe's `vkQueuePresentKHR` slot.
//! * **(b) wrapper**: patch the exe's `vkGetDeviceProcAddr` slot with a
//!   function that answers `vkQueuePresentKHR` with our hook and forwards
//!   everything else. Catches volk-style loaders that never call present
//!   through the PLT — unless volk resolved everything before we loaded.
//! * **(c) inline hook**: when neither PLT slot exists (static volk
//!   dispatch, confirmed in CarX Street), resolve the real
//!   `vkQueuePresentKHR` from the loaded `libvulkan` and patch its
//!   prologue: a 12-byte absolute jump (`mov rax, hook; jmp rax`) with a
//!   trampoline preserving the stolen bytes. Prologue boundaries come
//!   from the `iced-x86` decoder — instructions are never split — and any
//!   IP-relative instruction in the stolen range aborts the install
//!   instead of mis-relocating it.
//!
//! # Threading
//!
//! Hooks run on the game's render thread: counter bump only (see
//! [`super::on_frame`]). All parsing, `mprotect` and logging happen on
//! our init thread during install, never per-frame.

use super::{RenderBackend, RenderError};
use crate::memory::{enumerate_maps, MemoryError};
use iced_x86::{Decoder, DecoderOptions, FlowControl, Mnemonic, OpKind};
use std::sync::atomic::{AtomicUsize, Ordering};

// --- ELF constants (x86_64) -------------------------------------------------

const PT_DYNAMIC: u32 = 2;
const DT_NULL: i64 = 0;
const DT_SYMTAB: i64 = 6;
const DT_STRTAB: i64 = 5;
const DT_SYMENT: i64 = 11;
const DT_JMPREL: i64 = 23;
const DT_PLTRELSZ: i64 = 2;
const R_X86_64_JUMP_SLOT: u32 = 7;
const RELA_SIZE: usize = 24;
const SYM_SIZE: usize = 24;

// --- Vulkan ABI stubs (opaque handles + result code only) --------------------

type VkQueue = *mut std::ffi::c_void;
type VkDevice = *mut std::ffi::c_void;
type VkResult = i32;
type PresentFn = unsafe extern "C" fn(VkQueue, *const std::ffi::c_void) -> VkResult;
type GetProcFn =
    unsafe extern "C" fn(VkDevice, *const libc::c_char) -> *const std::ffi::c_void;

// --- Hook state ---------------------------------------------------------------

/// Original `vkQueuePresentKHR`, once layer (a) or (b) captured it.
static ORIGINAL_PRESENT: AtomicUsize = AtomicUsize::new(0);
/// Original `vkGetDeviceProcAddr`, once layer (b) is installed.
static ORIGINAL_GETPROC: AtomicUsize = AtomicUsize::new(0);
/// GOT slots we patched (0 = untouched), for [`VulkanHook::uninstall`].
static PRESENT_SLOT: AtomicUsize = AtomicUsize::new(0);
static GETPROC_SLOT: AtomicUsize = AtomicUsize::new(0);
/// Original slot contents, paired with the slots above.
static PRESENT_ORIG: AtomicUsize = AtomicUsize::new(0);
static GETPROC_ORIG: AtomicUsize = AtomicUsize::new(0);
/// Layer (c) state: patched code address + stolen bytes for uninstall.
static INLINE_TARGET: AtomicUsize = AtomicUsize::new(0);
static INLINE_ORIG: AtomicUsize = AtomicUsize::new(0);
/// Inline-hook entry mode: 1 = patched a dispatch pointer, 2 = patched
/// code with a trampoline, 3 = patched scanned dispatch-table slots.
/// Selects the uninstall path.
static INLINE_MODE: AtomicUsize = AtomicUsize::new(0);
/// Stolen prologue bytes (mode 2), restored on uninstall. 64 B cap far
/// exceeds the 12–15 B we ever steal.
static INLINE_STOLEN: std::sync::Mutex<(usize, [u8; 64])> =
    std::sync::Mutex::new((0, [0u8; 64]));
/// Trampoline holding the stolen bytes + jump back (mode 2).
static INLINE_TRAMP: AtomicUsize = AtomicUsize::new(0);
/// Scanned table slots patched in mode 3: `(slot_addr, original_fn)`.
/// All originals must agree (checked at install); the hook forwards to
/// the first one.
static TABLE_SLOTS: std::sync::Mutex<Vec<(usize, usize)>> =
    std::sync::Mutex::new(Vec::new());

/// Present hook: count the frame, forward to the real function.
///
/// Runs on the game's render thread — see module docs: nothing else here.
unsafe extern "C" fn present_hook(
    queue: VkQueue,
    info: *const std::ffi::c_void,
) -> VkResult {
    super::on_frame();
    let orig: PresentFn =
        unsafe { std::mem::transmute(ORIGINAL_PRESENT.load(Ordering::SeqCst)) };
    // SAFETY: `ORIGINAL_PRESENT` is stored before any present can route
    // here (install order), and the signature matches Vulkan's ABI.
    unsafe { orig(queue, info) }
}

/// `vkGetDeviceProcAddr` wrapper (layer b): answer present queries with
/// our hook, forward everything else untouched.
///
/// Only consulted during device/driver init, never per-frame.
unsafe extern "C" fn getproc_wrapper(
    device: VkDevice,
    name: *const libc::c_char,
) -> *const std::ffi::c_void {
    let wanted = read_c_string(name as usize, 64);
    if wanted.as_deref() == Some("vkQueuePresentKHR") {
        let orig: GetProcFn =
            unsafe { std::mem::transmute(ORIGINAL_GETPROC.load(Ordering::SeqCst)) };
        // Resolve the real present through the ORIGINAL lookup (never
        // through ourselves — that would recurse).
        let real = unsafe {
            orig(
                device,
                c"vkQueuePresentKHR".as_ptr(),
            )
        };
        if !real.is_null() && ORIGINAL_PRESENT.load(Ordering::SeqCst) == 0 {
            ORIGINAL_PRESENT.store(real as usize, Ordering::SeqCst);
        }
        return present_hook as *const std::ffi::c_void;
    }
    let orig: GetProcFn =
        unsafe { std::mem::transmute(ORIGINAL_GETPROC.load(Ordering::SeqCst)) };
    // SAFETY: passthrough with the caller's exact arguments.
    unsafe { orig(device, name) }
}

// --- Checked in-process memory reads ------------------------------------------

/// Copy `[addr, addr+len)` via `/proc/self/mem`.
///
/// Two layers of protection instead of raw pointer reads:
///
/// 1. `enumerate_maps` pre-check: fast reject of wild addresses with a
///    precise error.
/// 2. `pread` on `/proc/self/mem`: the kernel's `access_remote_vm`
///    backing it returns `EIO`/short reads for pages that vanished
///    between check and read (live game `munmap` racing us) — it NEVER
///    delivers a SIGSEGV to us. This makes the whole multi-gigabyte table
///    scan crash-proof without touching signal handlers (which anti-cheat
///    runtimes tend to inspect).
fn read_checked(addr: usize, len: usize) -> Result<Vec<u8>, RenderError> {
    if len == 0 || len > (1 << 20) {
        return Err(RenderError::HookFailed(format!(
            "refusing wild read of {} bytes at {:#x}",
            len, addr
        )));
    }
    let end = addr
        .checked_add(len)
        .ok_or_else(|| RenderError::HookFailed("address overflow".into()))?;
    let maps = enumerate_maps()
        .map_err(|e: MemoryError| RenderError::HookFailed(e.to_string()))?;
    let covered = maps
        .iter()
        .any(|m| m.readable && m.base <= addr && end <= m.end);
    if !covered {
        return Err(RenderError::HookFailed(format!(
            "unreadable memory at {:#x}",
            addr
        )));
    }
    // SAFETY: `pread` argument discipline only — valid fd (just opened),
    // valid buffer, non-negative offset. The kernel validates the target
    // pages itself and reports failures as errors, never signals.
    unsafe {
        let fd = libc::open(
            c"/proc/self/mem".as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC,
        );
        if fd < 0 {
            return Err(RenderError::HookFailed(
                "cannot open /proc/self/mem".into(),
            ));
        }
        let mut buf = vec![0u8; len];
        let mut got = 0usize;
        while got < len {
            let n = libc::pread(
                fd,
                buf.as_mut_ptr().add(got) as *mut libc::c_void,
                len - got,
                (addr + got) as libc::off_t,
            );
            if n <= 0 {
                libc::close(fd);
                return Err(RenderError::HookFailed(format!(
                    "fault reading {:#x} (mapping vanished?)",
                    addr + got
                )));
            }
            got += n as usize;
        }
        libc::close(fd);
        Ok(buf)
    }
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn u64le(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}
fn i64le(b: &[u8]) -> i64 {
    i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Bounded NUL-terminated string read, or `None` on any anomaly.
fn read_c_string(addr: usize, max: usize) -> Option<String> {
    let bytes = read_checked(addr, max.min(256)).ok()?;
    let end = bytes.iter().position(|&b| b == 0)?;
    std::str::from_utf8(&bytes[..end]).ok().map(|s| s.to_string())
}

// --- Executable layout ----------------------------------------------------------

struct ExeInfo {
    base: usize,
    end: usize,
    bias: usize,
    dynamic: usize,
}

/// Locate the game executable's mappings and its `PT_DYNAMIC` address.
///
/// `bias` converts link-time virtual addresses to runtime ones.
fn exe_info() -> Result<ExeInfo, RenderError> {
    let fail = |m: &str| RenderError::HookFailed(m.into());
    let exe = std::env::current_exe().map_err(|e| fail(&e.to_string()))?;
    let canon = std::fs::canonicalize(&exe).unwrap_or(exe);
    let maps = enumerate_maps().map_err(|e| fail(&e.to_string()))?;
    let own: Vec<_> = maps
        .iter()
        .filter(|m| m.path.as_ref() == Some(&canon))
        .collect();
    if own.is_empty() {
        return Err(fail("own executable not found in maps"));
    }
    let base = own.iter().map(|m| m.base).min().unwrap_or(0);
    let end = own.iter().map(|m| m.end).max().unwrap_or(0);

    let eh = read_checked(base, 64)?;
    if eh[0..4] != [0x7f, b'E', b'L', b'F'] {
        return Err(fail("executable is not ELF"));
    }
    if eh[4] != 2 {
        return Err(fail("executable is not 64-bit"));
    }
    let phoff = u64le(&eh[0x20..0x28]) as usize;
    let phnum = u16le(&eh[0x38..0x3A]) as usize;
    if phnum == 0 || phnum > 64 {
        return Err(fail("implausible program header count"));
    }
    let phdrs = read_checked(base + phoff, phnum * 56)?;
    let mut load_min: Option<usize> = None;
    let mut dyn_vaddr: Option<usize> = None;
    for i in 0..phnum {
        let p = &phdrs[i * 56..(i + 1) * 56];
        let p_type = u32le(&p[0..4]);
        let p_vaddr = u64le(&p[16..24]) as usize;
        if u32le(&p[0..4]) == 1 {
            // PT_LOAD
            load_min = Some(load_min.map_or(p_vaddr, |m: usize| m.min(p_vaddr)));
        }
        if p_type == PT_DYNAMIC {
            dyn_vaddr = Some(p_vaddr);
        }
    }
    let (Some(load_min), Some(dyn_vaddr)) = (load_min, dyn_vaddr) else {
        return Err(fail("no PT_DYNAMIC in executable"));
    };
    let bias = base.saturating_sub(load_min);
    Ok(ExeInfo {
        base,
        end,
        bias,
        dynamic: bias + dyn_vaddr,
    })
}

// --- GOT slot lookup + patch -----------------------------------------------------

/// Dynamic tags we need, as `(tag, value)` pairs.
fn dynamic_tags(info: &ExeInfo) -> Result<Vec<(i64, u64)>, RenderError> {
    let mut tags = Vec::new();
    for i in 0..128 {
        let e = read_checked(info.dynamic + i * 16, 16)
            .map_err(|_| RenderError::HookFailed("dynamic array unreadable".into()))?;
        let tag = i64le(&e[0..8]);
        let val = u64le(&e[8..16]);
        if tag == DT_NULL {
            break;
        }
        tags.push((tag, val));
    }
    Ok(tags)
}

fn tag_value(tags: &[(i64, u64)], tag: i64) -> Option<u64> {
    tags.iter().find(|(t, _)| *t == tag).map(|(_, v)| *v)
}

/// Find the GOT slot of `symbol` in the game executable.
///
/// Dynamic address tags are tried both as-is and bias-adjusted (loaders
/// disagree — see `tools/inject.c` for the full story); the symbol-name
/// match itself is the validation. Returns the slot's runtime address.
fn find_got_slot(info: &ExeInfo, symbol: &str) -> Result<usize, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let tags = dynamic_tags(info)?;
    let jmprel = tag_value(&tags, DT_JMPREL)
        .ok_or_else(|| fail("no DT_JMPREL (probably statically linked PLT)".into()))?;
    let pltrelsz = tag_value(&tags, DT_PLTRELSZ).unwrap_or(0) as usize;
    let syment = tag_value(&tags, DT_SYMENT).unwrap_or(SYM_SIZE as u64) as usize;
    let nrel = pltrelsz / RELA_SIZE;
    if nrel == 0 || nrel > 1_000_000 {
        return Err(fail(format!("implausible RELA count {}", nrel)));
    }
    // Link-time vs pre-biased dynamic values: try both, first match wins.
    for bias_cand in [0usize, info.bias] {
        let symtab = tag_value(&tags, DT_SYMTAB).unwrap_or(0) as usize + bias_cand;
        let strtab = tag_value(&tags, DT_STRTAB).unwrap_or(0) as usize + bias_cand;
        let jmprel = jmprel as usize + bias_cand;
        if symtab == 0 || strtab == 0 {
            continue;
        }
        for i in 0..nrel {
            // One unreadable reloc poisons this candidate, not the search.
            let rela = match read_checked(jmprel + i * RELA_SIZE, RELA_SIZE) {
                Ok(r) => r,
                Err(_) => break,
            };
            let r_offset = u64le(&rela[0..8]) as usize;
            let r_info = u64le(&rela[8..16]);
            if (r_info & 0xffff_ffff) as u32 != R_X86_64_JUMP_SLOT {
                continue;
            }
            let sym_idx = (r_info >> 32) as usize;
            if sym_idx > 10_000_000 {
                continue;
            }
            let sym = match read_checked(symtab + sym_idx * syment, SYM_SIZE) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let st_name = u32le(&sym[0..4]) as usize;
            if st_name > 4096 {
                continue;
            }
            let name = match read_c_string(strtab + st_name, 128) {
                Some(n) => n,
                None => continue,
            };
            if name == symbol {
                // Rela records are file data (never rewritten): the slot
                // is always bias + link-time offset.
                let slot = info.bias + r_offset;
                if slot >= info.base && slot + 8 <= info.end {
                    return Ok(slot);
                }
                return Err(fail(format!("slot {:#x} outside exe mappings", slot)));
            }
        }
    }
    Err(fail(format!(
        "symbol '{}' has no PLT slot (static volk dispatch?)",
        symbol
    )))
}

/// Swap a GOT slot's address, preserving the page's original permissions.
///
/// Returns the previously installed address (for uninstall).
fn swap_got_slot(slot: usize, new_addr: usize) -> Result<usize, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    if page_size == 0 {
        return Err(fail("sysconf(_SC_PAGESIZE) failed".into()));
    }
    let page = slot & !(page_size - 1);
    // Preserve current permissions: partial RELRO leaves .got.plt
    // writable for lazy binding — restoring R-only would SIGSEGV the
    // next lazy bind of any other symbol on this page.
    let maps = enumerate_maps().map_err(|e| fail(e.to_string()))?;
    let was_writable = maps
        .iter()
        .find(|m| m.base <= slot && slot + 8 <= m.end)
        .map(|m| m.writable)
        .unwrap_or(true); // unknown: keep writable (safe direction)
    let rc = unsafe {
        libc::mprotect(
            page as *mut libc::c_void,
            page_size,
            libc::PROT_READ | libc::PROT_WRITE,
        )
    };
    if rc != 0 {
        return Err(fail(format!("mprotect RW failed at {:#x}", page)));
    }
    // SAFETY: slot validated inside exe mappings above; single word swap.
    let old = unsafe { std::ptr::read(slot as *const usize) };
    unsafe {
        std::ptr::write_volatile(slot as *mut usize, new_addr);
    }
    let restore = if was_writable {
        libc::PROT_READ | libc::PROT_WRITE
    } else {
        libc::PROT_READ
    };
    let rc = unsafe { libc::mprotect(page as *mut libc::c_void, page_size, restore) };
    if rc != 0 {
        return Err(fail(format!("mprotect restore failed at {:#x}", page)));
    }
    Ok(old)
}

// --- Public hook ------------------------------------------------------------------

// --- Layer (c): inline hook ------------------------------------------------------
//
// Used when neither PLT slot exists (static dispatch). The real function
// address comes from the loaded Vulkan loader; its prologue is analyzed
// with iced-x86 and handled by shape:
//
// * `jmp [rip+off]` (loader trampoline): patch the POINTER it jumps
//   through — a pure data write, no code touched, nothing relocated.
// * direct `jmp rel` (up to 3 hops): follow it, then re-analyze.
// * anything else: steal whole instructions until >= 12 bytes available,
//   copy them to an RX trampoline with a jump back, and plant a 12-byte
//   absolute jump (`mov rax, hook; jmp rax`). Any IP-relative operand,
//   branch, call or return inside the stolen range ABORTS the install
//   instead of mis-relocating it.
//
// Size of the planted absolute jump: `mov rax, imm64` (10 B) + `jmp rax`.
const JMP_ABS_SIZE: usize = 12;
// Hard cap on stolen bytes (backup buffer + trampoline sized accordingly).
const STOLEN_MAX: usize = 64;

/// Resolve the real `vkQueuePresentKHR` from the loaded Vulkan loader.
///
/// Explicit `libvulkan` handle first (found in our own maps, `RTLD_NOLOAD`
/// so nothing new is loaded), then the global scope as fallback.
fn resolve_present_address() -> Result<usize, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let mut handle: *mut libc::c_void = std::ptr::null_mut();
    if let Some(path) = crate::memory::find_module_path("libvulkan.so") {
        if let Ok(c_path) =
            std::ffi::CString::new(path.to_string_lossy().into_owned())
        {
            // SAFETY: valid NUL-terminated path; flags are constants.
            unsafe {
                let h =
                    libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_NOLOAD);
                if !h.is_null() {
                    handle = h;
                }
            }
        }
    }
    let sym = std::ffi::CString::new("vkQueuePresentKHR")
        .map_err(|_| fail("bad symbol string".into()))?;
    // SAFETY: live handles (or RTLD_DEFAULT); null-checked results.
    unsafe {
        if !handle.is_null() {
            let p = libc::dlsym(handle, sym.as_ptr());
            if !p.is_null() {
                return Ok(p as usize);
            }
        }
        let p = libc::dlsym(libc::RTLD_DEFAULT, sym.as_ptr());
        if p.is_null() {
            return Err(fail(
                "vkQueuePresentKHR not exported anywhere (fully static dispatch)".into(),
            ));
        }
        Ok(p as usize)
    }
}

/// Emit a 12-byte absolute jump to `dst` at `at`.
///
/// # Safety
/// Caller guarantees 12 writable bytes at `at` (see `with_writable_pages`).
unsafe fn emit_abs_jump(at: usize, dst: usize) {
    unsafe {
        let p = at as *mut u8;
        // mov rax, imm64
        *p = 0x48;
        *p.add(1) = 0xB8;
        std::ptr::write_unaligned(p.add(2) as *mut u64, dst as u64);
        // jmp rax
        *p.add(10) = 0xFF;
        *p.add(11) = 0xE0;
    }
}

/// Run `f` with `[addr, addr+len)` mapped writable, restoring each page's
/// exact prior permissions (RX for code, RW for data) afterwards.
///
/// Every failure — unreadable range, `mprotect` refusal — becomes a
/// `HookFailed` error; the game is never left half-patched on the way in
/// (restores on the way out are best-effort: nothing sensible remains to
/// do if the kernel refuses those).
fn with_writable_pages<R>(
    addr: usize,
    len: usize,
    f: impl FnOnce() -> R,
) -> Result<R, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let ps = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    if ps == 0 || !ps.is_power_of_two() {
        return Err(fail("unusable page size".into()));
    }
    let end = addr
        .checked_add(len)
        .ok_or_else(|| fail("address overflow".into()))?;
    let start = addr & !(ps - 1);
    let span_end = end
        .checked_add(ps - 1)
        .ok_or_else(|| fail("address overflow".into()))?
        & !(ps - 1);
    let maps = enumerate_maps().map_err(|e| fail(e.to_string()))?;
    let prot_of = |p: usize| -> Option<i32> {
        maps.iter().find(|m| m.base <= p && p < m.end).map(|m| {
            (if m.readable { libc::PROT_READ } else { 0 })
                | (if m.writable { libc::PROT_WRITE } else { 0 })
                | (if m.executable { libc::PROT_EXEC } else { 0 })
        })
    };
    // Flip every spanned page to RW, remembering priors.
    let mut saved: Vec<(usize, i32)> = Vec::new();
    let mut p = start;
    while p < span_end {
        let prot = prot_of(p)
            .ok_or_else(|| fail(format!("unmapped page at {:#x}", p)))?;
        if unsafe {
            libc::mprotect(
                p as *mut libc::c_void,
                ps,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        } != 0
        {
            for (rp, rprot) in &saved {
                unsafe { libc::mprotect(*rp as *mut libc::c_void, ps, *rprot) };
            }
            return Err(fail(format!("mprotect RW failed at {:#x}", p)));
        }
        saved.push((p, prot));
        p = p.checked_add(ps).ok_or_else(|| fail("address overflow".into()))?;
    }
    // The write itself targets mappings already proven stable (GOT
    // slots, loader tables, patched code — none of which the game
    // unmaps); permissions are restored on BOTH paths below, because a
    // half-patched page left writable (or a code page left
    // non-executable) would be worse than a clean abort.
    let r = f();
    for (rp, rprot) in saved {
        unsafe { libc::mprotect(rp as *mut libc::c_void, ps, rprot) };
    }
    Ok(r)
}

/// What the prologue analysis found at an address.
enum Prologue {
    /// `jmp [rip+off]`: patch the pointer it jumps through.
    PtrHook(usize),
    /// Direct jump: re-analyze at the destination.
    Jump(usize),
    /// Ordinary code: steal whole instructions here.
    Code,
}

/// Hex dump for abort diagnostics (proves exactly what the prologue
/// holds when an install refuses to proceed).
fn hex_bytes(addr: usize) -> String {
    match read_checked(addr, 32.min(32)) {
        Ok(b) => b
            .iter()
            .map(|x| format!("{:02x}", x))
            .collect::<Vec<_>>()
            .join(" "),
        Err(_) => "<unreadable>".into(),
    }
}

/// Classify the instruction at `addr` (must be readable).
///
/// Skips leading CET landing pads (`endbr64`) and NOP padding first:
/// system libraries commonly start with them, and they are pure
/// fall-through — analysis starts after them, while stealing (case
/// `Code`) still starts at the original entry to keep the 12-byte
/// plant math simple.
fn classify_prologue(addr: usize) -> Result<Prologue, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let bytes = read_checked(addr, 32)
        .map_err(|_| fail(format!("unreadable code at {:#x}", addr)))?;
    let mut off = 0usize;
    for _ in 0..4 {
        if off >= bytes.len() {
            break;
        }
        let mut dec =
            Decoder::with_ip(64, &bytes[off..], (addr + off) as u64, DecoderOptions::NONE);
        if !dec.can_decode() {
            break;
        }
        let prefix = dec.decode();
        if prefix.is_invalid() {
            break;
        }
        if prefix.mnemonic() == Mnemonic::Endbr64 || prefix.mnemonic() == Mnemonic::Nop {
            off += prefix.len();
        } else {
            break;
        }
    }
    if off >= bytes.len() {
        return Err(fail(format!("prefix-only prologue at {:#x}", addr)));
    }
    let mut decoder =
        Decoder::with_ip(64, &bytes[off..], (addr + off) as u64, DecoderOptions::NONE);
    if !decoder.can_decode() {
        return Err(fail(format!("cannot decode at {:#x}", addr)));
    }
    let instr = decoder.decode();
    if instr.is_invalid() {
        return Err(fail(format!("invalid instruction at {:#x}", addr)));
    }
    // Loader trampoline: patch the pointer, touch no code.
    if instr.flow_control() == FlowControl::UnconditionalBranch
        && instr.op_count() == 1
        && instr.op0_kind() == OpKind::Memory
        && instr.is_ip_rel_memory_operand()
    {
        return Ok(Prologue::PtrHook(instr.ip_rel_memory_address() as usize));
    }
    // Direct jump: follow one hop (bounded by the caller).
    if instr.is_jmp_short_or_near() {
        let dest = instr.near_branch_target() as usize;
        read_checked(dest, 1)
            .map_err(|_| fail(format!("jump target {:#x} unreadable", dest)))?;
        return Ok(Prologue::Jump(dest));
    }
    Ok(Prologue::Code)
}

/// Install layer (c): inline-hook the real present function at `entry`.
///
/// Follows up to 3 direct jumps, patches a dispatch pointer when the
/// prologue is a trampoline, steals code otherwise — and when the
/// prologue refuses stealing (validation trampoline with branches, as
/// seen in the wild), falls back to [`scan_loader_tables`], which finds
/// the same dispatch slots by their magic instead of by disassembly.
fn install_inline_hook(entry: usize) -> Result<&'static str, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let mut target = entry;
    for _ in 0..3 {
        match classify_prologue(target)? {
            Prologue::PtrHook(ptr) => {
                let orig = u64le(
                    &read_checked(ptr, 8)
                        .map_err(|_| fail(format!("dispatch pointer {:#x} unreadable", ptr)))?,
                ) as usize;
                if orig == 0 {
                    return Err(fail("dispatch pointer is null".into()));
                }
                with_writable_pages(ptr, 8, || unsafe {
                    std::ptr::write(ptr as *mut usize, present_hook as usize);
                })?;
                INLINE_TARGET.store(ptr, Ordering::SeqCst);
                INLINE_ORIG.store(orig, Ordering::SeqCst);
                INLINE_MODE.store(1, Ordering::SeqCst);
                ORIGINAL_PRESENT.store(orig, Ordering::SeqCst);
                return Ok("dispatch pointer");
            }
            Prologue::Jump(dest) => {
                target = dest;
            }
            Prologue::Code => {
                // Stealable prologue — but a validation trampoline may
                // still refuse below; the table scan (magic, then volk
                // density) is the last resort. `entry` is the resolved
                // real address the scan also hunts for.
                match steal_and_patch(target) {
                    Ok(how) => return Ok(how),
                    Err(_) => return install_via_table_scan(entry),
                }
            }
        }
    }
    Err(fail("jump chain too deep, refusing to hook".into()))
}

/// Steal whole prologue instructions at `target` into a trampoline and
/// plant the 12-byte absolute jump. See layer (c) docs for the abort
/// rules that keep this safe.
fn steal_and_patch(target: usize) -> Result<&'static str, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let bytes = read_checked(target, STOLEN_MAX + 16)
        .map_err(|_| fail(format!("unreadable code at {:#x}", target)))?;
    let mut decoder = Decoder::with_ip(64, &bytes, target as u64, DecoderOptions::NONE);
    let mut stolen = 0usize;
    loop {
        if !decoder.can_decode() {
            return Err(fail("prologue ends mid-instruction".into()));
        }
        let instr = decoder.decode();
        if instr.is_invalid() {
            return Err(fail("invalid instruction in prologue".into()));
        }
        // Only pure fall-through may be relocated: any branch, call,
        // return — or any RIP-relative operand, whose displacement would
        // silently break at the new address — aborts the install.
        // The hex dump names the exact blocker for the log.
        if instr.flow_control() != FlowControl::Next {
            return Err(fail(format!(
                "control flow in prologue, refusing to hook (bytes: {})",
                hex_bytes(target)
            )));
        }
        if instr.is_ip_rel_memory_operand() {
            return Err(fail(format!(
                "RIP-relative operand in prologue, refusing to hook (bytes: {})",
                hex_bytes(target)
            )));
        }
        stolen += instr.len();
        if stolen >= JMP_ABS_SIZE {
            break;
        }
        if stolen > STOLEN_MAX {
            return Err(fail("prologue unexpectedly long".into()));
        }
    }

    // Trampoline: stolen bytes + jump back. RX before anyone can call it.
    let tramp = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if tramp == libc::MAP_FAILED {
        return Err(fail("trampoline mmap failed".into()));
    }
    let tramp = tramp as usize;
    // SAFETY: freshly mmap'd RW page, bounds are ours.
    unsafe {
        std::ptr::copy_nonoverlapping(target as *const u8, tramp as *mut u8, stolen);
        emit_abs_jump(tramp + stolen, target + stolen);
        if libc::mprotect(
            (tramp & !(4096 - 1)) as *mut libc::c_void,
            4096,
            libc::PROT_READ | libc::PROT_EXEC,
        ) != 0
        {
            libc::munmap(tramp as *mut libc::c_void, 4096);
            return Err(fail("trampoline mprotect RX failed".into()));
        }
    }
    INLINE_TRAMP.store(tramp, Ordering::SeqCst);

    // Back up the bytes we are about to overwrite, then plant the jump.
    {
        let mut guard = INLINE_STOLEN.lock().unwrap();
        guard.0 = stolen;
        guard.1[..stolen].copy_from_slice(&bytes[..stolen]);
    }
    with_writable_pages(target, JMP_ABS_SIZE, || unsafe {
        emit_abs_jump(target, present_hook as usize);
    })?;
    INLINE_TARGET.store(target, Ordering::SeqCst);
    INLINE_ORIG.store(tramp, Ordering::SeqCst);
    // The hook forwards through ORIGINAL_PRESENT: for a trampoline hook
    // that IS the trampoline (stolen bytes + jump back to target+stolen).
    ORIGINAL_PRESENT.store(tramp, Ordering::SeqCst);
    INLINE_MODE.store(2, Ordering::SeqCst);
    Ok("code patch + trampoline")
}

/// Loader-dispatch magic observed in the wild (`mov rdx, <magic>;
/// cmp [rax], rdx` guarding a `jmp [rax+0x690]` trampoline).
const LOADER_MAGIC: u64 = 0x10aded040410aded;
/// Slot offset of the present pointer from the loader object base.
const PRESENT_SLOT_OFF: usize = 0x690;
/// Read chunk for the scan (matches `read_checked`'s cap).
const SCAN_CHUNK: usize = 1 << 20;
/// Overlap between chunks so an 8-byte magic straddling the boundary is
/// still seen whole. Must stay a multiple of 8 to preserve alignment.
const SCAN_OVERLAP: usize = 8;
/// Upper bound on scanned bytes; exceeding it fails closed with a clear
/// message instead of walking gigabytes.
const SCAN_BUDGET: usize = 2 << 30;

/// Raw findings of one pass over writable memory: candidate loader
/// object bases (magic) and addresses holding the known present address
/// (candidate volk-table slots). Validation + patching happen per
/// strategy afterwards.
struct ScanHits {
    magic_bases: Vec<usize>,
    present_refs: Vec<usize>,
}

/// Single pass over readable+writable non-executable mappings.
///
/// Collects both signals at once (one pass, one budget): `LOADER_MAGIC`
/// hits and qwords equal to `present_addr`. Chunked + overlapped so
/// boundary-straddling values are still seen whole; progress is logged
/// every 256 MB so a future session dying mid-scan is localizable.
fn scan_rw_memory(
    maps: &[crate::memory::MapEntry],
    present_addr: usize,
) -> Result<ScanHits, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let mut hits = ScanHits {
        magic_bases: Vec::new(),
        present_refs: Vec::new(),
    };
    let mut scanned: usize = 0;
    let mut logged_mb: usize = 0;
    for m in maps.iter().filter(|m| {
        m.readable && m.writable && !m.executable && m.end > m.base
    }) {
        let mut off = m.base;
        while off < m.end {
            if scanned > SCAN_BUDGET {
                return Err(fail("loader-table scan budget exceeded".into()));
            }
            let want = (m.end - off).min(SCAN_CHUNK);
            let chunk = match read_checked(off, want) {
                Ok(c) => c,
                Err(_) => break, // region changed under us; skip it
            };
            scanned += want;
            // Progress heartbeat: if a future session dies mid-scan,
            // the log pinpoints exactly how far it got.
            if scanned / (256 << 20) > logged_mb {
                logged_mb = scanned / (256 << 20);
                crate::log_line(&format!(
                    "render: table scan at {} MB",
                    scanned >> 20
                ));
            }
            let mut i = 0usize;
            while i + 8 <= chunk.len() {
                let v = u64le(&chunk[i..i + 8]);
                if v == LOADER_MAGIC {
                    hits.magic_bases.push(off + i);
                }
                if v == present_addr as u64 {
                    hits.present_refs.push(off + i);
                }
                i += 8;
            }
            if want < SCAN_CHUNK {
                break;
            }
            off += SCAN_CHUNK - SCAN_OVERLAP;
            // `off` stays 8-aligned: base is page-aligned, the step is a
            // multiple of 8, so every probed qword is naturally aligned.
        }
    }
    Ok(hits)
}

/// Patch validated `(slot, expected_orig)` pairs: re-verify each slot
/// still holds the expected value (TOCTOU), swap in the hook, record
/// for uninstall. Returns a static description for the log.
fn patch_slots(
    slots: &[(usize, usize)],
    what: &'static str,
) -> Result<&'static str, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let mut patched: Vec<(usize, usize)> = Vec::new();
    for (slot, expected) in slots {
        let cur = u64le(
            &read_checked(*slot, 8)
                .map_err(|_| fail(format!("slot {:#x} unreadable at patch time", slot)))?,
        ) as usize;
        if cur != *expected {
            continue; // changed under us; skip, don't corrupt
        }
        with_writable_pages(*slot, 8, || unsafe {
            std::ptr::write(*slot as *mut usize, present_hook as usize);
        })?;
        patched.push((*slot, *expected));
    }
    if patched.is_empty() {
        return Err(fail("all candidate slots changed under us".into()));
    }
    ORIGINAL_PRESENT.store(patched[0].1, Ordering::SeqCst);
    *TABLE_SLOTS.lock().unwrap() = patched;
    INLINE_MODE.store(3, Ordering::SeqCst);
    crate::log_line(&format!(
        "render: patched {} {}(s)",
        TABLE_SLOTS.lock().unwrap().len(),
        what
    ));
    Ok(what)
}

/// A volk device table is a dense run of function pointers: require at
/// least 6 of the 12 neighbors (±8..±48) to point into executable
/// mappings. Stack return addresses and stray heap pointers never look
/// like this; a real table interior always does.
fn looks_like_volk_table(
    ref_addr: usize,
    maps: &[crate::memory::MapEntry],
) -> bool {
    let mut exec = 0u32;
    for k in -6i64..=6 {
        if k == 0 {
            continue;
        }
        let a = (ref_addr as i64 + k * 8) as usize;
        if let Ok(raw) = read_checked(a, 8) {
            let v = u64le(&raw) as usize;
            if v != 0 && is_executable_addr(v, maps) {
                exec += 1;
            }
        }
    }
    exec >= 6
}

/// Install via memory scan (last resort of layer (c)).
///
/// Strategy 1 — loader magic: `[base+0x690]` of each `LOADER_MAGIC`
/// struct must point into executable mappings; all originals must agree.
/// Strategy 2 — volk density: each reference to the known present address
/// must sit inside a dense run of function pointers (see
/// [`looks_like_volk_table`]); the original is the known address itself,
/// re-verified at patch time.
fn install_via_table_scan(present_addr: usize) -> Result<&'static str, RenderError> {
    let fail = |m: String| RenderError::HookFailed(m);
    let maps = enumerate_maps().map_err(|e| fail(e.to_string()))?;
    let mut hits = scan_rw_memory(&maps, present_addr)?;
    hits.magic_bases.sort_unstable();
    hits.magic_bases.dedup();
    hits.present_refs.sort_unstable();
    hits.present_refs.dedup();

    // Strategy 1: magic-derived slots.
    let mut slots: Vec<(usize, usize)> = Vec::new();
    for base in &hits.magic_bases {
        let slot = base + PRESENT_SLOT_OFF;
        if let Ok(raw) = read_checked(slot, 8) {
            let target = u64le(&raw) as usize;
            if target != 0 && is_executable_addr(target, &maps) {
                slots.push((slot, target));
            }
        }
    }
    if !slots.is_empty() {
        let first_orig = slots[0].1;
        if slots.iter().any(|(_, o)| *o != first_orig) {
            crate::log_line("render: loader tables disagree, trying volk density");
        } else {
            return patch_slots(&slots, "loader dispatch table");
        }
    }

    // Strategy 2: volk-table density around known present references.
    let mut vslots: Vec<(usize, usize)> = Vec::new();
    for r in &hits.present_refs {
        if looks_like_volk_table(*r, &maps) {
            vslots.push((*r, present_addr));
        }
    }
    if vslots.is_empty() {
        return Err(fail(format!(
            "no hookable tables: {} magic struct(s) without valid slots, {} present reference(s) without dense tables",
            hits.magic_bases.len(),
            hits.present_refs.len()
        )));
    }
    patch_slots(&vslots, "volk dispatch table")
}

/// True when `addr` lies inside an executable mapping.
fn is_executable_addr(addr: usize, maps: &[crate::memory::MapEntry]) -> bool {
    maps
        .iter()
        .any(|m| m.executable && m.base <= addr && addr < m.end)
}

/// Linux Vulkan present hook.
pub struct VulkanHook;

impl super::RenderHook for VulkanHook {
    fn backend(&self) -> RenderBackend {
        RenderBackend::Vulkan
    }

    fn install(&self) -> Result<(), RenderError> {
        if PRESENT_SLOT.load(Ordering::SeqCst) != 0
            || GETPROC_SLOT.load(Ordering::SeqCst) != 0
            || INLINE_MODE.load(Ordering::SeqCst) != 0
        {
            return Ok(()); // idempotent
        }
        let info = exe_info()?;

        // Layer (a): direct present slot.
        match find_got_slot(&info, "vkQueuePresentKHR") {
            Ok(slot) => {
                let orig = swap_got_slot(slot, present_hook as usize)?;
                ORIGINAL_PRESENT.store(orig, Ordering::SeqCst);
                PRESENT_SLOT.store(slot, Ordering::SeqCst);
                PRESENT_ORIG.store(orig, Ordering::SeqCst);
                crate::log_line("render: hooked vkQueuePresentKHR (direct GOT)");
            }
            Err(direct_err) => {
                // Layer (b): wrap the proc-address lookup instead.
                match find_got_slot(&info, "vkGetDeviceProcAddr") {
                    Ok(slot) => {
                        let orig = swap_got_slot(slot, getproc_wrapper as usize)?;
                        ORIGINAL_GETPROC.store(orig, Ordering::SeqCst);
                        GETPROC_SLOT.store(slot, Ordering::SeqCst);
                        GETPROC_ORIG.store(orig, Ordering::SeqCst);
                        crate::log_line(
                            "render: hooked vkGetDeviceProcAddr (present wrapper)",
                        );
                    }
                    Err(wrap_err) => {
                        // Layer (c): inline hook on the real function.
                        match resolve_present_address().and_then(install_inline_hook) {
                            Ok(how) => {
                                crate::log_line(&format!(
                                    "render: hooked vkQueuePresentKHR (inline, {})",
                                    how
                                ));
                            }
                            Err(inline_err) => {
                                return Err(RenderError::HookFailed(format!(
                                    "no PLT slot for present ({}) nor for GetDeviceProcAddr ({}) nor inline ({})",
                                    direct_err, wrap_err, inline_err
                                )));
                            }
                        }
                    }
                }
            }
        }

        // Diagnostic: report interception volume once the game presents.
        // Detached; never touches the render thread.
        let _ = std::thread::Builder::new()
            .name("cxsfm-renderwatch".into())
            .spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(30));
                crate::log_line(&format!(
                    "render: {} presents intercepted so far",
                    super::frame_count()
                ));
            });
        Ok(())
    }

    fn uninstall(&self) -> Result<(), RenderError> {
        for (slot_cell, orig_cell) in [
            (&PRESENT_SLOT, &PRESENT_ORIG),
            (&GETPROC_SLOT, &GETPROC_ORIG),
        ] {
            let slot = slot_cell.load(Ordering::SeqCst);
            if slot != 0 {
                let orig = orig_cell.load(Ordering::SeqCst);
                swap_got_slot(slot, orig)?;
                slot_cell.store(0, Ordering::SeqCst);
                orig_cell.store(0, Ordering::SeqCst);
            }
        }
        match INLINE_MODE.load(Ordering::SeqCst) {
            // Mode 1: restore the dispatch pointer we overwrote.
            1 => {
                let ptr = INLINE_TARGET.load(Ordering::SeqCst);
                let orig = INLINE_ORIG.load(Ordering::SeqCst);
                with_writable_pages(ptr, 8, || unsafe {
                    std::ptr::write(ptr as *mut usize, orig);
                })?;
            }
            // Mode 2: restore the stolen prologue bytes (RX preserved
            // by the page helper), then drop the trampoline mapping.
            2 => {
                let target = INLINE_TARGET.load(Ordering::SeqCst);
                let (len, backup) = *INLINE_STOLEN.lock().unwrap();
                with_writable_pages(target, len, || unsafe {
                    std::ptr::copy_nonoverlapping(
                        backup.as_ptr(),
                        target as *mut u8,
                        len,
                    );
                })?;
                let tramp = INLINE_TRAMP.load(Ordering::SeqCst);
                if tramp != 0 {
                    unsafe { libc::munmap(tramp as *mut libc::c_void, 4096) };
                    INLINE_TRAMP.store(0, Ordering::SeqCst);
                }
            }
            // Mode 3: restore every scanned table slot we overwrote.
            3 => {
                let slots = TABLE_SLOTS.lock().unwrap();
                for (slot, orig) in slots.iter() {
                    with_writable_pages(*slot, 8, || unsafe {
                        std::ptr::write(*slot as *mut usize, *orig);
                    })?;
                }
            }
            _ => {}
        }
        INLINE_MODE.store(0, Ordering::SeqCst);
        INLINE_TARGET.store(0, Ordering::SeqCst);
        INLINE_ORIG.store(0, Ordering::SeqCst);
        TABLE_SLOTS.lock().unwrap().clear();
        Ok(())
    }
}
