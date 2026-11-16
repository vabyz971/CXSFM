//! Persisted mod values (TOML save files).
//!
//! Save files live OUTSIDE the game install dir
//! (`$XDG_CONFIG_HOME/cxsfm/saves.toml`, else `~/.config/cxsfm/`),
//! are hand-editable (TOML comments survive — we never rewrite user
//! text, only whole-file replace on Save), and store values BY OBJECT
//! PATH (`[speedometer."Speedometer/Background/ImgBg"]`). Never addresses,
//! never originals (those are re-read from the game every session).
//!
//! All file IO happens on the tick thread (Save/Load buttons only
//! stage data + set flags on the render thread). Writes are atomic
//! (tmp + rename). Parsing is tolerant: unknown mods/objects/keys and
//! mistyped values are skipped + counted, never fatal.
//!
//! The save-file format itself (`[mod."path"]` tables) is the
//! speedometer's (`cxsfm_mods::speedometer::saves`); this module owns
//! the paths and atomic file IO.
//!
//! This module also hosts the session-wide auto-load switch (Settings
//! checkbox, persisted to `settings.toml` next to the saves).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Auto-apply save files a few seconds after arriving in game
/// (Settings toggle, default off). Read by speedo/map on the tick.
static AUTO_LOAD: AtomicBool = AtomicBool::new(false);
/// Set when the toggle moves (render thread); the tick persists it
/// (file IO never happens on the present thread).
static SETTINGS_DIRTY: AtomicBool = AtomicBool::new(false);

/// Whether saves auto-apply on arrival (Settings checkbox).
pub fn auto_load() -> bool {
    AUTO_LOAD.load(Ordering::SeqCst)
}

/// Set by the Settings checkbox (render thread — plain atomic; the
/// tick persists it, see `flush_settings_if_dirty`).
pub fn set_auto_load(v: bool) {
    AUTO_LOAD.store(v, Ordering::SeqCst);
}

/// Mark settings for tick-side persistence (Settings checkbox calls
/// this right after toggling; the boot path never does, so a fresh
/// read is never pointlessly rewritten).
pub fn mark_settings_dirty() {
    SETTINGS_DIRTY.store(true, Ordering::SeqCst);
}

/// Settings file next to the saves (`settings.toml`, same dir).
/// `None` when no config home exists at all.
pub fn settings_path() -> Option<PathBuf> {
    let mut dir = settings_dir()?;
    dir.push("settings.toml");
    Some(dir)
}

/// Custom HUD layouts (`custom_hud.toml`, same dir — hot-reloaded by the
/// hud mod, example written on first enable).
pub fn hud_path() -> Option<PathBuf> {
    let mut dir = settings_dir()?;
    dir.push("custom_hud.toml");
    Some(dir)
}

/// A per-mod file in the config dir (`<name>`, e.g. `fpv_drone.toml`).
pub fn mod_config_path(name: &str) -> Option<PathBuf> {
    let mut dir = settings_dir()?;
    dir.push(name);
    Some(dir)
}

fn settings_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".config");
                p
            })
        })?;
    let mut p = base;
    p.push("cxsfm");
    Some(p)
}

/// Read persisted settings at boot (init thread — file IO is fine
/// there, never on present). Missing/corrupt file = defaults (off),
/// never fatal.
pub fn load_settings() -> bool {
    let path = match settings_path() {
        Some(p) => p,
        None => return false,
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_settings(&text),
        Err(_) => false,
    }
}

/// Build settings text (pure — unit-tested).
pub fn settings_toml(auto_load: bool) -> String {
    let mut table = toml::Table::new();
    let mut general = toml::Table::new();
    general.insert("auto_load".to_string(), toml::Value::Boolean(auto_load));
    table.insert("general".to_string(), toml::Value::Table(general));
    format!(
        "# CXSFM settings. Hand-editable.\n{}",
        toml::to_string(&table).unwrap_or_default()
    )
}

/// Parse settings text (pure, tolerant — anything unreadable is off).
pub fn parse_settings(text: &str) -> bool {
    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    table
        .get("general")
        .and_then(|v| v.as_table())
        .and_then(|g| g.get("auto_load"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Persist the toggle if it moved (tick thread, best effort — a lost
/// write only costs one click next session, never game state).
pub fn flush_settings_if_dirty() {
    if !SETTINGS_DIRTY.swap(false, Ordering::SeqCst) {
        return;
    }
    let text = settings_toml(auto_load());
    let path = match settings_path() {
        Some(p) => p,
        None => return,
    };
    if let Err(e) = save_toml(&path, &text) {
        crate::log_line(&format!("config: settings save failed ({e})"));
    }
}

/// Save-file location: `$XDG_CONFIG_HOME/cxsfm/saves.toml`, falling
/// back to `~/.config/cxsfm/saves.toml` (blocked when neither `HOME`
/// nor `XDG_CONFIG_HOME` exists — tread carefully there).
pub fn saves_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".config");
                p
            })
        })?;
    let mut p = base;
    p.push("cxsfm");
    p.push("saves.toml");
    Some(p)
}

/// Atomic whole-file write: parent dirs created, tmp + rename (a torn
/// write must never leave half a save behind). The previous file, when
/// present, is copied to `<name>.toml.bak` first — a bad Save stays
/// undoable by renaming the backup back (documented in the mod READMEs).
pub fn save_toml(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let mut bak = PathBuf::from(path);
        bak.set_extension("toml.bak");
        // A failed backup must not block the save itself; the caller
        // logs it (best effort, explicit user action).
        if let Err(e) = std::fs::copy(path, &bak) {
            crate::log_line(&format!("config: backup impossible ({e})"));
        }
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read the whole save file.
pub fn load_toml(path: &std::path::Path) -> std::io::Result<String> {
    std::fs::read_to_string(path)
}

/// Cheap file fingerprint (length + mtime secs) for change detection.
/// `None` = missing/unreadable. Same-second same-size rewrites could
/// collide in theory — manual Load always works regardless.
pub fn file_fingerprint(path: &std::path::Path) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(m.len().wrapping_mul(31).wrapping_add(mtime))
}

#[cfg(test)]
mod tests {
    use super::{parse_settings, settings_toml};

    #[test]
    fn settings_roundtrip() {
        assert!(parse_settings(&settings_toml(true)));
        assert!(!parse_settings(&settings_toml(false)));
        assert!(!parse_settings("[[[broken"));
        assert!(!parse_settings("[general]\nauto_load = \"yes\""));
        assert!(!parse_settings(""));
    }
}
