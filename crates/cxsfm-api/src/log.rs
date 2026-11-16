//! Framework log file (`cxsfm.log`): one timestamped line per call.

use std::sync::LazyLock;

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
            if let Ok(p) = std::env::var("CXSF_LOG")
                && !p.is_empty()
            {
                return std::path::PathBuf::from(p);
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
/// lookup on a known function address (every crate links into the one
/// `.so`). Returns `None` when the lookup fails (non-dynamic loading,
/// exotic loaders).
#[cfg(unix)]
fn own_module_dir() -> Option<std::path::PathBuf> {
    // SAFETY: `dladdr` with a valid in-module function address and a
    // valid out-pointer; read-only query, no side effects.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        let addr = log_path as *const () as usize as *const libc::c_void;
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
        let addr = log_path as usize as *const std::ffi::c_void;
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
/// mirrored to stderr so they show up in Steam's own logs. All I/O
/// failures are silently ignored — logging must never break the game.
///
/// Never call this from the `#[ctor]` itself (see its safety notes).
pub fn log_line(msg: &str) {
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
