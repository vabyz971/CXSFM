//! Per-mod parameter policy: which settings the simple view shows, and
//! how far each one may move.
//!
//! `~/.config/cxsfm/<mod>.toml`, written with defaults on first use and
//! hot-reloaded (checked every couple of seconds). The user designates
//! the editable parameters there; everything else only appears in the
//! "Advanced" view, behind a warning. Limits can only NARROW a
//! parameter's built-in safe range, never widen it, and a broken file
//! keeps the previous policy.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const CHECK_EVERY: Duration = Duration::from_secs(2);

/// Optional per-parameter overrides from the file.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Limits {
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,
}

/// The parsed file.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Parsed {
    pub show: HashSet<String>,
    pub limits: HashMap<String, Limits>,
}

/// Parse the policy text. `None` = not valid TOML (caller keeps the
/// previous policy).
pub fn parse(text: &str) -> Option<Parsed> {
    let v: toml::Value = toml::from_str(text).ok()?;
    let num = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).map(|f| f as f32);
    let mut out = Parsed::default();
    if let Some(arr) = v.get("show").and_then(|s| s.as_array()) {
        out.show = arr.iter().filter_map(|k| k.as_str().map(str::to_string)).collect();
    }
    if let Some(tbl) = v.get("limits").and_then(|l| l.as_table()) {
        for (key, t) in tbl {
            let lim = Limits {
                min: t.get("min").and_then(num).filter(|f| f.is_finite()),
                max: t.get("max").and_then(num).filter(|f| f.is_finite()),
                step: t.get("step").and_then(num).filter(|f| f.is_finite() && *f > 0.0),
            };
            out.limits.insert(key.clone(), lim);
        }
    }
    Some(out)
}

/// A file's limits intersected with the built-in range (never wider;
/// a nonsense override that empties the range is ignored).
pub fn effective_range(base: (f32, f32), lim: Limits) -> (f32, f32) {
    let lo = lim.min.map_or(base.0, |m| m.max(base.0));
    let hi = lim.max.map_or(base.1, |m| m.min(base.1));
    if lo < hi { (lo, hi) } else { base }
}

/// The commented file written on first use.
pub fn default_file(title: &str, shown: &[&str], all: &[(&str, &str)]) -> String {
    let list = shown.iter().map(|k| format!("  \"{k}\",")).collect::<Vec<_>>().join("\n");
    let doc = all.iter().map(|(k, l)| format!("#   {k:<16} {l}")).collect::<Vec<_>>().join("\n");
    format!(
        "# CXSFM - {title}: which parameters the simple view lets you change.\n\
         #\n\
         # Anything NOT listed in `show` is only reachable through the\n\
         # \"Advanced settings\" switch, behind a warning: wrong values there\n\
         # can freeze or crash the game. This file is reloaded while the\n\
         # game runs.\n\
         #\n\
         # Available parameters:\n{doc}\n\
         \n\
         show = [\n{list}\n]\n\
         \n\
         # Optional: narrow a parameter's range or change its step. A limit\n\
         # can only shrink the built-in safe range, never widen it.\n\
         # [limits.gravity]\n\
         # min = 0.0\n\
         # max = 15.0\n\
         # step = 0.5\n"
    )
}

/// Live policy of one mod.
pub struct Policy {
    file: &'static str,
    parsed: Parsed,
    fingerprint: Option<u64>,
    last_check: Option<Instant>,
}

impl Policy {
    /// Builds the policy, writing the default file if it is missing.
    pub fn new(file: &'static str, title: &str, shown: &[&str], all: &[(&str, &str)]) -> Self {
        let mut p = Self {
            file,
            parsed: Parsed { show: shown.iter().map(|s| s.to_string()).collect(), limits: HashMap::new() },
            fingerprint: None,
            last_check: None,
        };
        if let Some(path) = crate::config::mod_config_path(file) {
            if !path.exists() {
                match crate::config::save_toml(&path, &default_file(title, shown, all)) {
                    Ok(()) => crate::log_line(&format!("ui: wrote default {}", path.display())),
                    Err(e) => crate::log_line(&format!("ui: cannot write {} ({e})", path.display())),
                }
            }
        }
        p.refresh();
        p
    }

    /// Re-read the file when it changed (throttled; render thread).
    pub fn refresh(&mut self) {
        if self.last_check.is_some_and(|t| t.elapsed() < CHECK_EVERY) {
            return;
        }
        self.last_check = Some(Instant::now());
        let Some(path) = crate::config::mod_config_path(self.file) else { return };
        let fp = crate::config::file_fingerprint(&path);
        if fp == self.fingerprint {
            return;
        }
        self.fingerprint = fp;
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        match parse(&text) {
            Some(parsed) => {
                self.parsed = parsed;
                crate::log_line(&format!("ui: {} reloaded", self.file));
            }
            None => crate::log_line(&format!("ui: {} is not valid TOML, previous policy kept", self.file)),
        }
    }

    /// Is `key` visible in the simple view?
    pub fn shown(&self, key: &str) -> bool {
        self.parsed.show.contains(key)
    }

    pub fn limits(&self, key: &str) -> Limits {
        self.parsed.limits.get(key).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_show_and_limits() {
        let p = parse("show = [\"a\", \"b\"]\n[limits.a]\nmin = 1\nmax = 2.5\nstep = 0.5\n").unwrap();
        assert!(p.show.contains("a") && p.show.contains("b"));
        assert_eq!(p.limits["a"], Limits { min: Some(1.0), max: Some(2.5), step: Some(0.5) });
    }

    #[test]
    fn garbage_is_rejected_not_fatal() {
        assert!(parse("show = [").is_none());
        let p = parse("show = 3\n[limits.a]\nstep = -1\n").unwrap();
        assert!(p.show.is_empty());
        assert_eq!(p.limits["a"].step, None);
    }

    #[test]
    fn limits_only_narrow() {
        let base = (0.0, 10.0);
        assert_eq!(effective_range(base, Limits { min: Some(2.0), max: Some(5.0), step: None }), (2.0, 5.0));
        assert_eq!(effective_range(base, Limits { min: Some(-9.0), max: Some(99.0), step: None }), base);
        assert_eq!(effective_range(base, Limits { min: Some(8.0), max: Some(3.0), step: None }), base);
    }

    #[test]
    fn default_file_round_trips() {
        let text = default_file("T", &["x", "y"], &[("x", "ex"), ("y", "why")]);
        let p = parse(&text).unwrap();
        assert_eq!(p.show.len(), 2);
    }
}
