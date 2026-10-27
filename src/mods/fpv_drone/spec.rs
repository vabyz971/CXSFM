//! Parameter table of the drone window: what exists, in which tab,
//! with which built-in (safe) range, and whether the simple view shows
//! it by default. The user's `fpv_drone.toml` then decides what the
//! simple view really shows and may narrow the ranges (see
//! `ui::paramfile`).

use super::Shared;
use super::model::{
    DRAG_RANGE, FOV_RANGE, GRAVITY_RANGE, GUST_RANGE, MAX_ANGLE_RANGE, Mode, RADIUS_RANGE, RATE_RANGE, SOUND_RANGE,
    TILT_RANGE, TWR_RANGE, WIND_RANGE, YAW_RATE_RANGE,
};

pub const TABS: [&str; 4] = ["Flight", "World", "Camera & sound", "Controls"];
pub const CONTROLS_TAB: usize = 3;
/// Policy file (in the cxsfm config dir).
pub const FILE: &str = "fpv_drone.toml";

pub enum Kind {
    Slider { min: f32, max: f32, step: f32, dec: usize, unit: &'static str },
    Toggle,
    Choice(&'static [&'static str]),
}

pub struct Spec {
    pub key: &'static str,
    pub label: &'static str,
    pub tab: usize,
    pub section: &'static str,
    pub kind: Kind,
    pub help: &'static str,
    /// Listed in the default `show` of the policy file.
    pub shown: bool,
}

const fn slider(r: (f32, f32), step: f32, dec: usize, unit: &'static str) -> Kind {
    Kind::Slider { min: r.0, max: r.1, step, dec, unit }
}

pub const SPECS: &[Spec] = &[
    Spec { key: "mode", label: "Flight mode", tab: 0, section: "Flight model", kind: Kind::Choice(&["ANGLE", "ACRO"]), shown: true,
        help: "Angle: the drone levels itself, sticks set the tilt. Acro: sticks set rotation rates, no self-level (flips and rolls). Button X switches in flight." },
    Spec { key: "max_angle", label: "Max tilt", tab: 0, section: "Flight model", kind: slider(MAX_ANGLE_RANGE, 1.0, 0, "°"), shown: true,
        help: "Angle mode: tilt at full stick. More tilt = faster forward flight." },
    Spec { key: "rate", label: "Roll / pitch rate", tab: 0, section: "Flight model", kind: slider(RATE_RANGE, 10.0, 0, "°/s"), shown: true,
        help: "Acro mode: rotation speed at full stick." },
    Spec { key: "expo", label: "Stick expo", tab: 0, section: "Flight model", kind: slider((0.0, 1.0), 0.05, 2, ""), shown: true,
        help: "Acro mode: softens the stick around the centre for fine control." },
    Spec { key: "yaw_rate", label: "Yaw rate", tab: 0, section: "Flight model", kind: slider(YAW_RATE_RANGE, 5.0, 0, "°/s"), shown: true,
        help: "Turning speed at full yaw stick (both modes)." },
    Spec { key: "twr", label: "Thrust / weight", tab: 0, section: "Motors", kind: slider(TWR_RANGE, 0.5, 1, ""), shown: true,
        help: "Motor power relative to weight. 4 is a freestyle quad, 8+ a racing drone." },
    Spec { key: "throttle_mid", label: "Hover at stick centre", tab: 0, section: "Motors", kind: Kind::Toggle, shown: false,
        help: "ON: centred throttle stick hovers (spring pad). OFF: bottom of the axis is zero throttle (use a trigger)." },
    Spec { key: "gravity", label: "Gravity", tab: 1, section: "Physics", kind: slider(GRAVITY_RANGE, 0.1, 1, " m/s²"), shown: true,
        help: "World gravity. 9.8 is Earth." },
    Spec { key: "drag", label: "Air drag", tab: 1, section: "Physics", kind: slider(DRAG_RANGE, 0.1, 1, "×"), shown: true,
        help: "Air resistance multiplier. Higher = lower top speed, quicker stop." },
    Spec { key: "wind_speed", label: "Wind speed", tab: 1, section: "Wind", kind: slider(WIND_RANGE, 0.5, 1, " m/s"), shown: true,
        help: "Steady wind pushing the drone." },
    Spec { key: "wind_dir", label: "Wind direction", tab: 1, section: "Wind", kind: slider((0.0, 360.0), 5.0, 0, "°"), shown: true,
        help: "Direction the wind blows TOWARD (0 = world +Z, 90 = world +X)." },
    Spec { key: "gust", label: "Gusts", tab: 1, section: "Wind", kind: slider(GUST_RANGE, 0.5, 1, " m/s"), shown: true,
        help: "Random-looking variation added to the wind." },
    Spec { key: "collisions", label: "Collisions", tab: 1, section: "Collisions", kind: Kind::Toggle, shown: false,
        help: "Stops the drone against the world (Physics.Raycast from the mod thread). Off = fly through everything." },
    Spec { key: "radius", label: "Drone radius", tab: 1, section: "Collisions", kind: slider(RADIUS_RANGE, 0.05, 2, " m"), shown: false,
        help: "Size used for collisions." },
    Spec { key: "bounce", label: "Bounce", tab: 1, section: "Collisions", kind: slider((0.0, 0.9), 0.05, 2, ""), shown: true,
        help: "How much speed is kept when hitting something." },
    Spec { key: "tilt_deg", label: "Camera tilt", tab: 2, section: "Camera", kind: slider(TILT_RANGE, 1.0, 0, "°"), shown: true,
        help: "Fixed upward angle of the camera, like a real FPV drone." },
    Spec { key: "fov", label: "Field of view", tab: 2, section: "Camera", kind: slider(FOV_RANGE, 1.0, 0, "°"), shown: true,
        help: "FPV cameras are wide: 100-120° feels right." },
    Spec { key: "cut_controller", label: "Also stop game CameraController", tab: 2, section: "Camera", kind: Kind::Toggle, shown: false,
        help: "Stops the game's own camera script too (applies at the next enable). Suspected of breaking LOD and menus." },
    Spec { key: "sound", label: "Rotor sound", tab: 2, section: "Sound", kind: Kind::Toggle, shown: true,
        help: "Four synthesized rotors: each motor's speed follows the controls." },
    Spec { key: "sound_volume", label: "Drone volume", tab: 2, section: "Sound", kind: slider(SOUND_RANGE, 0.05, 2, ""), shown: true,
        help: "Volume of the rotors (independent of the game's sliders)." },
];

pub fn get(s: &Shared, key: &str) -> f32 {
    let p = &s.params;
    match key {
        "mode" => (p.mode == Mode::Acro) as u8 as f32,
        "max_angle" => p.max_angle,
        "rate" => p.rate,
        "expo" => p.expo,
        "yaw_rate" => p.yaw_rate,
        "twr" => p.twr,
        "throttle_mid" => p.throttle_mid as u8 as f32,
        "gravity" => p.gravity,
        "drag" => p.drag,
        "wind_speed" => p.wind_speed,
        "wind_dir" => p.wind_dir,
        "gust" => p.gust,
        "collisions" => p.collisions as u8 as f32,
        "radius" => p.radius,
        "bounce" => p.bounce,
        "tilt_deg" => p.tilt_deg,
        "fov" => p.fov,
        "cut_controller" => s.cut_controller as u8 as f32,
        "sound" => p.sound as u8 as f32,
        "sound_volume" => p.sound_volume,
        _ => 0.0,
    }
}

pub fn set(s: &mut Shared, key: &str, v: f32) {
    let on = v >= 0.5;
    let p = &mut s.params;
    match key {
        "mode" => p.mode = if on { Mode::Acro } else { Mode::Angle },
        "max_angle" => p.max_angle = v,
        "rate" => p.rate = v,
        "expo" => p.expo = v,
        "yaw_rate" => p.yaw_rate = v,
        "twr" => p.twr = v,
        "throttle_mid" => p.throttle_mid = on,
        "gravity" => p.gravity = v,
        "drag" => p.drag = v,
        "wind_speed" => p.wind_speed = v,
        "wind_dir" => p.wind_dir = v,
        "gust" => p.gust = v,
        "collisions" => p.collisions = on,
        "radius" => p.radius = v,
        "bounce" => p.bounce = v,
        "tilt_deg" => p.tilt_deg = v,
        "fov" => p.fov = v,
        "cut_controller" => s.cut_controller = on,
        "sound" => p.sound = on,
        "sound_volume" => p.sound_volume = v,
        _ => {}
    }
}

/// `(key, label)` of every parameter, for the policy file's comment.
pub fn catalog() -> Vec<(&'static str, &'static str)> {
    SPECS.iter().map(|s| (s.key, s.label)).collect()
}

/// Keys shown by default in the simple view.
pub fn default_shown() -> Vec<&'static str> {
    SPECS.iter().filter(|s| s.shown).map(|s| s.key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::fpv_drone::model::{Action, Bindings, Params};

    fn shared() -> Shared {
        Shared { params: Params::default(), bindings: Bindings::default(), learn: None::<Action>, cut_controller: false }
    }

    #[test]
    fn every_spec_round_trips_through_get_set() {
        let mut s = shared();
        for spec in SPECS {
            let v = get(&s, spec.key);
            set(&mut s, spec.key, v);
            assert_eq!(get(&s, spec.key), v, "{}", spec.key);
        }
    }

    #[test]
    fn keys_are_unique_and_inside_the_tabs() {
        let mut seen = std::collections::HashSet::new();
        for spec in SPECS {
            assert!(seen.insert(spec.key), "duplicate {}", spec.key);
            assert!(spec.tab < CONTROLS_TAB);
        }
    }

    #[test]
    fn slider_defaults_sit_inside_their_range() {
        let s = shared();
        for spec in SPECS {
            if let Kind::Slider { min, max, .. } = spec.kind {
                let v = get(&s, spec.key);
                assert!(v >= min && v <= max, "{} = {v} not in {min}..{max}", spec.key);
            }
        }
    }
}
