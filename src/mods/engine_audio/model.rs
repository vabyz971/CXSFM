//! Engine-audio data + pure helpers (no FMOD, no Unity — unit-tested).

/// Event-path filter used until the user changes it (comma list).
pub const DEFAULT_FILTER: &str = "engine";
pub const PITCH_RANGE: (f32, f32) = (0.25, 3.0);
pub const VOLUME_RANGE: (f32, f32) = (0.0, 3.0);
/// Overrides kept at most (the UI lists the live event's parameters).
pub const MAX_OVERRIDES: usize = 32;

/// A parameter the user pins to a fixed value.
///
/// EXPERIMENTAL: the game writes its own parameters (rpm, load…) every
/// frame; ours races with it and can only win by being written after.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamOverride {
    pub name: String,
    pub value: f32,
    pub on: bool,
}

/// What the user asked for (UI → tick, cloned under a brief lock).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Multiplier on the event's pitch (1.0 = stock).
    pub pitch: f32,
    /// Multiplier on the event's volume (1.0 = stock).
    pub volume: f32,
    /// Comma-separated, case-insensitive substrings of the event path.
    pub filter: String,
    pub overrides: Vec<ParamOverride>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { pitch: 1.0, volume: 1.0, filter: DEFAULT_FILTER.to_string(), overrides: Vec::new() }
    }
}

impl Settings {
    /// Pitch/volume at stock values.
    pub fn is_neutral(&self) -> bool {
        (self.pitch - 1.0).abs() < 1e-4 && (self.volume - 1.0).abs() < 1e-4
    }

    /// Targets clamped to safe ranges (NaN → stock).
    pub fn targets(&self) -> (f32, f32) {
        (clamp(self.pitch, PITCH_RANGE), clamp(self.volume, VOLUME_RANGE))
    }
}

fn clamp(v: f32, (lo, hi): (f32, f32)) -> f32 {
    if v.is_nan() { 1.0 } else { v.clamp(lo, hi) }
}

/// Does an event path match the comma-separated `filter`?
/// An empty filter matches nothing (never "every event in the game").
pub fn matches_filter(path: &str, filter: &str) -> bool {
    let path = path.to_lowercase();
    filter
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .any(|t| !t.is_empty() && path.contains(&t))
}

/// Live status published by the tick for the window.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// One line: what the mod is doing / why it idles.
    pub note: String,
    pub banks: usize,
    pub events: usize,
    /// `(path, live instances)` of events matching the filter.
    pub matched: Vec<(String, usize)>,
    /// Instances currently driven.
    pub instances: usize,
    /// Parameters of the first live matched instance.
    pub params: Vec<ParamRow>,
}

/// One parameter row for the window.
#[derive(Clone, Debug)]
pub struct ParamRow {
    pub name: String,
    pub min: f32,
    pub max: f32,
    /// Current value / final (modulated) value.
    pub value: f32,
    pub final_value: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_is_case_insensitive_list() {
        assert!(matches_filter("event:/Cars/Engine/V8", "engine"));
        assert!(matches_filter("event:/Cars/Turbo", "engine, turbo"));
        assert!(!matches_filter("event:/UI/Click", "engine,turbo"));
    }

    #[test]
    fn empty_filter_matches_nothing() {
        assert!(!matches_filter("event:/Cars/Engine", ""));
        assert!(!matches_filter("event:/Cars/Engine", " , "));
    }

    #[test]
    fn targets_are_clamped_and_nan_safe() {
        let s = Settings { pitch: 99.0, volume: f32::NAN, ..Settings::default() };
        assert_eq!(s.targets(), (PITCH_RANGE.1, 1.0));
        let s = Settings { pitch: 0.0, volume: -1.0, ..Settings::default() };
        assert_eq!(s.targets(), (PITCH_RANGE.0, VOLUME_RANGE.0));
    }

    #[test]
    fn neutral_detection() {
        assert!(Settings::default().is_neutral());
        assert!(!Settings { pitch: 1.2, ..Settings::default() }.is_neutral());
    }
}
