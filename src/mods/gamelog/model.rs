//! Gamelog data model: poll caps, path resolution, error sniffing.
//!
//! No Unity calls here (no game interaction at all — plain file IO
//! lives in `bind.rs`, everything drawn lives in `ui.rs`).

/// Poll cadence (~1 Hz at 60 Hz tick).
pub(crate) const POLL_EVERY_TICKS: u64 = 60;
/// Ring capacity (lines).
pub(crate) const MAX_LINES: usize = 300;
/// Per-poll read cap (a flood must not stall the tick).
pub(crate) const MAX_BYTES_PER_POLL: u64 = 64 * 1024;

/// Resolve the game's `Player.log`:
///
/// `$XDG_CONFIG_HOME/unity3d/CarX Technologies/CarX Street/Player.log`,
/// else `~/.config/...`. `None` outside a standard home layout.
pub(crate) fn player_log_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = std::path::PathBuf::from(h);
                p.push(".config");
                p
            })
        })?;
    let mut p = base;
    p.push("unity3d");
    p.push("CarX Technologies");
    p.push("CarX Street");
    p.push("Player.log");
    Some(p)
}

/// A state-transition line: the engine logs every game-state switch
/// as `[GameStateManager]: <Name> (...) to queue!` (menu, city,
/// garage — names beyond MainMenuState are unobserved, so any name
/// counts). Drives save auto-apply (see `super::note_game_state_line`).
pub(crate) fn is_state_line(line: &str) -> bool {
    line.contains("[GameStateManager]:")
}

/// A line is an error when it smells like one (case-insensitive).
pub(crate) fn is_error(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("error")
        || l.contains("exception")
        || l.contains("failed")
        || l.contains("nullreference")
        || l.contains("crash")
}
#[cfg(test)]
mod tests {
    use super::{is_error, is_state_line};

    #[test]
    fn error_sniffing() {
        assert!(is_error("NullReferenceException: foo"));
        assert!(is_error("ERROR: something failed"));
        assert!(is_error("Game crashed: out of memory"));
        assert!(!is_error("[cxsfm + 1.0s] render: first present intercepted"));
        assert!(!is_error("Unloading 1455 unused Assets to reduce memory usage."));
    }

    #[test]
    fn state_line_sniffing() {
        assert!(is_state_line(
            "[GameStateManager]: MainMenuState (CarX.Street.GameModes.States.MainMenuState) to queue!"
        ));
        assert!(!is_state_line("render: first present intercepted, hook is live"));
        assert!(!is_state_line(""));
    }
}
