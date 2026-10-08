//! Shared mod helpers: cadences, caps and log formatting.
//!
//! Unity calls from a foreign thread must stay rare (1 Hz verify,
//! ~0.2 Hz search) and log lines single-line; these constants keep
//! every mod on the same proven rhythm.

/// Resolve attempts while armed but handle-less (~1 Hz at 60 Hz tick).
pub const RESOLVE_EVERY_TICKS: u64 = 60;
/// Check cadence with a live handle (~1 Hz at 60 Hz tick).
pub const TAG_CHECK_EVERY_TICKS: u64 = 60;
/// Search cadence without a handle (gentle scene walk, ~0.2 Hz).
pub const TAG_SEARCH_EVERY_TICKS: u64 = 300;
/// Max texts inventoried per run (a broken count must not flood the log).
pub const MAX_ITEMS: usize = 64;
/// Content preview length (chars, newlines flattened — one log line each).
pub const PREVIEW_CHARS: usize = 100;

/// Flatten to one line and cap length: log records must stay
/// single-line (see `log_line`'s atomic-record contract).
pub fn preview(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .take(PREVIEW_CHARS + 1)
        .collect();
    if flat.chars().count() > PREVIEW_CHARS {
        let mut s: String = flat.chars().take(PREVIEW_CHARS).collect();
        s.push('…');
        s
    } else {
        flat
    }
}
