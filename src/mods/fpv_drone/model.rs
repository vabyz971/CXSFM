//! FPV drone flight model (pure math, no Unity — unit-tested).
//!
//! A quadcopter as a rigid body: thrust along the body's up axis, gravity,
//! air drag against the *relative* wind, attitude controlled either by
//! target angles (Angle / self-level) or by target body rates (Acro).
//!
//! Conventions are Unity's: +Y up, +Z forward, +X right. With glam's
//! right-hand rotation formulas this gives: +X rotation = nose DOWN,
//! +Y rotation = yaw right, +Z rotation = right wing UP (so a bank to
//! the right is a NEGATIVE Z rotation).

use glam::{EulerRot, Quat, Vec3};

/// Gravity the motors are sized against (m/s²). The `gravity` slider
/// changes the world, not the motors.
pub const REF_G: f32 = 9.81;
/// Longest step integrated at once (a hitch must not teleport).
const MAX_DT: f32 = 0.1;
/// Motor spool-up lag, attitude rate lag, self-level lag (1/s).
const SPOOL_RATE: f32 = 12.0;
const RATE_LAG: f32 = 20.0;
const LEVEL_RATE: f32 = 10.0;
/// Drag per m/s (linear) and per (m/s)² (quadratic): ~40 m/s terminal
/// speed at 45° of tilt for a 5" quad. Scaled by `Params::drag`.
const DRAG_LINEAR: f32 = 0.15;
const DRAG_QUAD: f32 = 0.006;
/// Hard speed cap (m/s) against numeric blow-ups.
const MAX_SPEED: f32 = 120.0;

pub const AXES: usize = 8;
pub const BUTTONS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Sticks set the tilt angle, the drone self-levels.
    #[default]
    Angle,
    /// Sticks set rotation rates, no self-level (flips, rolls).
    Acro,
}

/// User-tunable flight parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub mode: Mode,
    /// Angle mode: tilt at full stick, degrees.
    pub max_angle: f32,
    /// Acro mode: roll/pitch rate at full stick, deg/s.
    pub rate: f32,
    /// Acro stick curve, 0 (linear) … 1 (very soft centre).
    pub expo: f32,
    /// Yaw rate at full stick, deg/s (both modes).
    pub yaw_rate: f32,
    /// Thrust-to-weight ratio at full throttle.
    pub twr: f32,
    /// Hover at stick centre (spring-centred pad) instead of 0 at the
    /// bottom of the throttle axis.
    pub throttle_mid: bool,
    /// World gravity, m/s².
    pub gravity: f32,
    /// Air drag multiplier.
    pub drag: f32,
    /// Wind speed (m/s), direction it blows TOWARD (deg, 0 = +Z, 90 = +X)
    /// and gust amplitude (m/s).
    pub wind_speed: f32,
    pub wind_dir: f32,
    pub gust: f32,
    /// Collide with the world (Physics.Raycast).
    pub collisions: bool,
    /// Drone collision radius, m.
    pub radius: f32,
    /// Bounciness 0 (dead stop) … 0.9.
    pub bounce: f32,
    /// Fixed camera up-tilt, degrees.
    pub tilt_deg: f32,
    pub fov: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            mode: Mode::Angle,
            max_angle: 45.0,
            rate: 400.0,
            expo: 0.3,
            yaw_rate: 180.0,
            twr: 4.0,
            throttle_mid: true,
            gravity: REF_G,
            drag: 1.0,
            wind_speed: 0.0,
            wind_dir: 0.0,
            gust: 0.0,
            collisions: true,
            radius: 0.15,
            bounce: 0.3,
            tilt_deg: 25.0,
            fov: 110.0,
        }
    }
}

pub const MAX_ANGLE_RANGE: (f32, f32) = (10.0, 80.0);
pub const RATE_RANGE: (f32, f32) = (90.0, 900.0);
pub const YAW_RATE_RANGE: (f32, f32) = (45.0, 500.0);
pub const TWR_RANGE: (f32, f32) = (1.5, 10.0);
pub const GRAVITY_RANGE: (f32, f32) = (0.0, 25.0);
pub const DRAG_RANGE: (f32, f32) = (0.0, 3.0);
pub const WIND_RANGE: (f32, f32) = (0.0, 25.0);
pub const GUST_RANGE: (f32, f32) = (0.0, 10.0);
pub const RADIUS_RANGE: (f32, f32) = (0.05, 1.0);
pub const TILT_RANGE: (f32, f32) = (0.0, 60.0);
pub const FOV_RANGE: (f32, f32) = (50.0, 140.0);

impl Params {
    /// Every field clamped to its range (NaN → default).
    pub fn sanitized(&self) -> Self {
        let d = Self::default();
        let c = |v: f32, (lo, hi): (f32, f32), def: f32| if v.is_nan() { def } else { v.clamp(lo, hi) };
        Self {
            mode: self.mode,
            max_angle: c(self.max_angle, MAX_ANGLE_RANGE, d.max_angle),
            rate: c(self.rate, RATE_RANGE, d.rate),
            expo: c(self.expo, (0.0, 1.0), d.expo),
            yaw_rate: c(self.yaw_rate, YAW_RATE_RANGE, d.yaw_rate),
            twr: c(self.twr, TWR_RANGE, d.twr),
            throttle_mid: self.throttle_mid,
            gravity: c(self.gravity, GRAVITY_RANGE, d.gravity),
            drag: c(self.drag, DRAG_RANGE, d.drag),
            wind_speed: c(self.wind_speed, WIND_RANGE, d.wind_speed),
            wind_dir: if self.wind_dir.is_finite() { self.wind_dir.rem_euclid(360.0) } else { 0.0 },
            gust: c(self.gust, GUST_RANGE, d.gust),
            collisions: self.collisions,
            radius: c(self.radius, RADIUS_RANGE, d.radius),
            bounce: c(self.bounce, (0.0, 0.9), d.bounce),
            tilt_deg: c(self.tilt_deg, TILT_RANGE, d.tilt_deg),
            fov: c(self.fov, FOV_RANGE, d.fov),
        }
    }
}

// ---------------------------------------------------------------- controls

/// One gamepad axis assignment (SDL index, sign flip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisBind {
    pub idx: u8,
    pub invert: bool,
}

/// Everything the user can rebind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bindings {
    pub throttle: AxisBind,
    pub yaw: AxisBind,
    pub pitch: AxisBind,
    pub roll: AxisBind,
    /// Button indexes.
    pub arm: u8,
    pub reset: u8,
    pub mode: u8,
}

impl Default for Bindings {
    /// Xbox layout: left stick = throttle (Y) + roll (X), right stick =
    /// pitch (Y) + yaw (X). SDL reports "up" as negative, hence the flips.
    fn default() -> Self {
        Self {
            throttle: AxisBind { idx: 1, invert: true },
            roll: AxisBind { idx: 0, invert: false },
            pitch: AxisBind { idx: 4, invert: true },
            yaw: AxisBind { idx: 3, invert: false },
            arm: 0,
            reset: 3,
            mode: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Throttle,
    Yaw,
    Pitch,
    Roll,
    Arm,
    Reset,
    Mode,
}

impl Action {
    pub const ALL: [Action; 7] =
        [Action::Throttle, Action::Yaw, Action::Pitch, Action::Roll, Action::Arm, Action::Reset, Action::Mode];

    pub fn is_axis(self) -> bool {
        matches!(self, Action::Throttle | Action::Yaw | Action::Pitch | Action::Roll)
    }

    /// What "positive" means, shown while learning an axis.
    pub fn hint(self) -> &'static str {
        match self {
            Action::Throttle => "throttle UP",
            Action::Yaw => "yaw RIGHT",
            Action::Pitch => "pitch FORWARD (nose down)",
            Action::Roll => "roll RIGHT",
            Action::Arm => "arm / disarm",
            Action::Reset => "reset to start",
            Action::Mode => "angle / acro",
        }
    }
}

impl Bindings {
    pub fn axis(&self, a: Action) -> Option<AxisBind> {
        match a {
            Action::Throttle => Some(self.throttle),
            Action::Yaw => Some(self.yaw),
            Action::Pitch => Some(self.pitch),
            Action::Roll => Some(self.roll),
            _ => None,
        }
    }

    pub fn axis_mut(&mut self, a: Action) -> Option<&mut AxisBind> {
        match a {
            Action::Throttle => Some(&mut self.throttle),
            Action::Yaw => Some(&mut self.yaw),
            Action::Pitch => Some(&mut self.pitch),
            Action::Roll => Some(&mut self.roll),
            _ => None,
        }
    }

    pub fn button(&self, a: Action) -> Option<u8> {
        match a {
            Action::Arm => Some(self.arm),
            Action::Reset => Some(self.reset),
            Action::Mode => Some(self.mode),
            _ => None,
        }
    }

    pub fn set_button(&mut self, a: Action, idx: u8) {
        match a {
            Action::Arm => self.arm = idx,
            Action::Reset => self.reset = idx,
            Action::Mode => self.mode = idx,
            _ => {}
        }
    }
}

/// Normalized controls. `throttle` 0…1; the rest −1…1 with pitch
/// positive = nose DOWN (forward flight) and roll positive = bank RIGHT.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sticks {
    pub throttle: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn read_axis(b: AxisBind, axes: &[f32; AXES]) -> f32 {
    let v = finite(axes.get(b.idx as usize).copied().unwrap_or(0.0)).clamp(-1.0, 1.0);
    if b.invert { -v } else { v }
}

/// Raw axes → controls through the user's bindings.
pub fn read_sticks(b: &Bindings, axes: &[f32; AXES]) -> Sticks {
    Sticks {
        throttle: (read_axis(b.throttle, axes) + 1.0) * 0.5,
        yaw: read_axis(b.yaw, axes),
        pitch: read_axis(b.pitch, axes),
        roll: read_axis(b.roll, axes),
    }
}

/// "Learn" an axis: the one that moved furthest from its resting
/// baseline (> 0.6). Pushing in the action's positive direction while
/// the raw value falls means the axis must be inverted.
pub fn learn_axis(base: &[f32; AXES], now: &[f32; AXES]) -> Option<AxisBind> {
    let mut best: Option<(usize, f32)> = None;
    for i in 0..AXES {
        let d = finite(now[i]) - finite(base[i]);
        if d.abs() > 0.6 && best.is_none_or(|(_, bd)| d.abs() > bd.abs()) {
            best = Some((i, d));
        }
    }
    best.map(|(i, d)| AxisBind { idx: i as u8, invert: d < 0.0 })
}

/// "Learn" a button: the first one newly pressed.
pub fn learn_button(prev: &[bool; BUTTONS], now: &[bool; BUTTONS]) -> Option<u8> {
    (0..BUTTONS).find(|&i| now[i] && !prev[i]).map(|i| i as u8)
}

// ------------------------------------------------------------------ flight

/// A world hit (from `Physics.Raycast`).
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
}

/// Betaflight-style stick curve: rad/s for a stick in −1…1.
fn rate_curve(x: f32, rate_deg: f32, expo: f32) -> f32 {
    let x = finite(x).clamp(-1.0, 1.0);
    rate_deg.to_radians() * ((1.0 - expo) * x + expo * x * x * x)
}

/// Wind at time `t`: steady part + smooth deterministic gusts.
pub fn wind_at(p: &Params, t: f32) -> Vec3 {
    let dir = p.wind_dir.to_radians();
    let base = Vec3::new(dir.sin(), 0.0, dir.cos()) * p.wind_speed;
    let g = Vec3::new(
        (1.3 * t).sin() * (0.37 * t + 1.0).sin(),
        0.3 * (0.9 * t + 2.0).sin(),
        (1.7 * t + 0.5).sin() * (0.53 * t).cos(),
    ) * p.gust;
    base + g
}

/// The drone's own rigid-body state, independent of the game camera.
#[derive(Clone, Copy, Debug)]
pub struct Drone {
    pub pos: Vec3,
    pub vel: Vec3,
    q: Quat,
    /// Body angular velocity (rad/s), Acro only.
    omega: Vec3,
    /// Motor thrust acceleration (m/s²), spooled.
    thrust: f32,
    time: f32,
    /// Last impact speed (m/s), for the UI.
    pub impact: f32,
}

impl Drone {
    /// Start where the game camera is, looking where it looks, level
    /// and hovering.
    pub fn from_pose(pos: [f32; 3], rot: [f32; 4]) -> Self {
        let q = Quat::from_xyzw(rot[0], rot[1], rot[2], rot[3]);
        let q = if q.is_finite() && q.length_squared() > 1e-6 { q.normalize() } else { Quat::IDENTITY };
        let (yaw, pitch, _) = q.to_euler(EulerRot::YXZ);
        let pos = Vec3::from_array(pos);
        Self {
            pos: if pos.is_finite() { pos } else { Vec3::ZERO },
            vel: Vec3::ZERO,
            q: Quat::from_euler(EulerRot::YXZ, yaw, pitch.clamp(-1.0, 1.0), 0.0),
            omega: Vec3::ZERO,
            thrust: REF_G,
            time: 0.0,
            impact: 0.0,
        }
    }

    /// Hover thrust, no motion (called when arming).
    pub fn arm(&mut self) {
        self.vel = Vec3::ZERO;
        self.omega = Vec3::ZERO;
        self.thrust = REF_G;
    }

    /// Throttle (0…1) → thrust in multiples of REF_G.
    fn thrust_g(t: f32, p: &Params) -> f32 {
        let t = finite(t).clamp(0.0, 1.0);
        if p.throttle_mid {
            if t <= 0.5 { t * 2.0 } else { 1.0 + (t - 0.5) * 2.0 * (p.twr - 1.0) }
        } else {
            t * p.twr
        }
    }

    /// Advance one step.
    pub fn step(&mut self, s: &Sticks, p: &Params, dt: f32) {
        let dt = finite(dt).clamp(0.0, MAX_DT);
        if dt <= 0.0 {
            return;
        }
        let p = p.sanitized();
        self.time += dt;
        self.impact = (self.impact - 4.0 * dt).max(0.0);

        match p.mode {
            Mode::Acro => {
                let target = Vec3::new(
                    rate_curve(s.pitch, p.rate, p.expo),
                    p.yaw_rate.to_radians() * finite(s.yaw).clamp(-1.0, 1.0),
                    -rate_curve(s.roll, p.rate, p.expo),
                );
                self.omega += (target - self.omega) * (1.0 - (-RATE_LAG * dt).exp());
                self.q = (self.q * Quat::from_scaled_axis(self.omega * dt)).normalize();
            }
            Mode::Angle => {
                let (yaw, pitch, z) = self.q.to_euler(EulerRot::YXZ);
                let k = 1.0 - (-LEVEL_RATE * dt).exp();
                let max = p.max_angle.to_radians();
                let want_pitch = finite(s.pitch).clamp(-1.0, 1.0) * max;
                let want_z = -finite(s.roll).clamp(-1.0, 1.0) * max;
                let yaw = yaw + finite(s.yaw).clamp(-1.0, 1.0) * p.yaw_rate.to_radians() * dt;
                self.q = Quat::from_euler(
                    EulerRot::YXZ,
                    yaw,
                    pitch + (want_pitch - pitch) * k,
                    z + (want_z - z) * k,
                )
                .normalize();
                self.omega = Vec3::ZERO;
            }
        }

        let target = Self::thrust_g(s.throttle, &p) * REF_G;
        self.thrust += (target - self.thrust) * (1.0 - (-SPOOL_RATE * dt).exp());
        let rel = self.vel - wind_at(&p, self.time);
        let drag = (rel * DRAG_LINEAR + rel * rel.length() * DRAG_QUAD) * p.drag;
        let acc = self.q * Vec3::new(0.0, self.thrust, 0.0) - Vec3::Y * p.gravity - drag;
        self.vel += acc * dt;
        if self.vel.length() > MAX_SPEED {
            self.vel = self.vel.normalize() * MAX_SPEED;
        }
        self.pos += self.vel * dt;
        if !(self.pos.is_finite() && self.vel.is_finite() && self.q.is_finite()) {
            self.vel = Vec3::ZERO;
            self.omega = Vec3::ZERO;
            self.q = Quat::IDENTITY;
            if !self.pos.is_finite() {
                self.pos = Vec3::ZERO;
            }
        }
    }

    /// Resolve a contact: sit on the surface, bounce, scrub tangential
    /// speed. Returns the impact speed (m/s) along the normal.
    pub fn contact(&mut self, hit: &Hit, p: &Params) -> f32 {
        let p = p.sanitized();
        let n = hit.normal.try_normalize().unwrap_or(Vec3::Y);
        if !hit.point.is_finite() {
            return 0.0;
        }
        self.pos = hit.point + n * p.radius;
        let vn = self.vel.dot(n);
        if vn >= 0.0 {
            return 0.0;
        }
        let impact = -vn;
        let tangent = self.vel - n * vn;
        // Slow impacts stick instead of jittering.
        let bounce = if impact < 1.0 { 0.0 } else { p.bounce };
        self.vel = tangent * (1.0 - (0.15 + 0.05 * impact).min(0.8)) - n * vn * bounce;
        self.omega *= 0.5;
        self.impact = self.impact.max(impact);
        impact
    }

    /// Camera rotation `[x, y, z, w]`: body attitude + fixed up-tilt.
    pub fn rotation(&self, p: &Params) -> [f32; 4] {
        let tilt = p.sanitized().tilt_deg.to_radians();
        (self.q * Quat::from_rotation_x(-tilt)).normalize().to_array()
    }

    /// Spooled throttle as a 0…1 fraction of what the bindings asked.
    pub fn thrust_fraction(&self, p: &Params) -> f32 {
        let max = p.sanitized().twr * REF_G;
        (self.thrust / max).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = 1.0 / 60.0;
    const HOVER: Sticks = Sticks { throttle: 0.5, yaw: 0.0, pitch: 0.0, roll: 0.0 };

    fn run(d: &mut Drone, s: &Sticks, p: &Params, secs: f32) {
        for _ in 0..(secs / STEP) as usize {
            d.step(s, p, STEP);
        }
    }

    fn level() -> Drone {
        Drone::from_pose([0.0; 3], Quat::IDENTITY.to_array())
    }

    #[test]
    fn centre_stick_hovers() {
        let mut d = level();
        run(&mut d, &HOVER, &Params::default(), 3.0);
        assert!(d.pos.length() < 0.05, "{:?}", d.pos);
    }

    #[test]
    fn zero_throttle_falls() {
        let mut d = level();
        run(&mut d, &Sticks { throttle: 0.0, ..HOVER }, &Params::default(), 2.0);
        assert!(d.pos.y < -10.0, "{:?}", d.pos);
    }

    #[test]
    fn no_gravity_no_fall() {
        let mut d = level();
        let p = Params { gravity: 0.0, ..Params::default() };
        run(&mut d, &Sticks { throttle: 0.0, ..HOVER }, &p, 2.0);
        // Thrust spools down to nothing; only the first instants push up.
        assert!(d.pos.y > -0.5, "{:?}", d.pos);
    }

    #[test]
    fn full_throttle_climbs() {
        let mut d = level();
        run(&mut d, &Sticks { throttle: 1.0, ..HOVER }, &Params::default(), 2.0);
        assert!(d.pos.y > 10.0);
    }

    #[test]
    fn forward_stick_flies_forward_in_angle_mode() {
        let mut d = level();
        run(&mut d, &Sticks { pitch: 1.0, ..HOVER }, &Params::default(), 2.0);
        assert!(d.pos.z > 3.0, "{:?}", d.pos);
    }

    #[test]
    fn angle_mode_self_levels() {
        let mut d = level();
        run(&mut d, &Sticks { roll: 1.0, ..HOVER }, &Params::default(), 1.0);
        run(&mut d, &HOVER, &Params::default(), 2.0);
        let (_, pitch, z) = d.q.to_euler(EulerRot::YXZ);
        assert!(pitch.abs() < 0.02 && z.abs() < 0.02);
    }

    #[test]
    fn acro_holds_attitude_and_rolls() {
        let p = Params { mode: Mode::Acro, expo: 0.0, rate: 360.0, ..Params::default() };
        let mut d = level();
        run(&mut d, &Sticks { roll: 1.0, ..HOVER }, &p, 0.25);
        let (_, _, z) = d.q.to_euler(EulerRot::YXZ);
        assert!(z < -0.5, "right bank is a negative Z rotation, got {z}");
        // The rate loop coasts briefly after release, then holds.
        run(&mut d, &HOVER, &p, 0.5);
        let before = d.q;
        run(&mut d, &HOVER, &p, 1.0);
        assert!(before.angle_between(d.q) < 0.02, "acro must not self-level");
    }

    #[test]
    fn wind_pushes_the_drone() {
        let mut d = level();
        let p = Params { wind_speed: 12.0, wind_dir: 90.0, ..Params::default() };
        run(&mut d, &HOVER, &p, 3.0);
        assert!(d.pos.x > 1.0 && d.pos.z.abs() < d.pos.x, "{:?}", d.pos);
    }

    #[test]
    fn contact_sits_on_surface_and_kills_inward_speed() {
        let mut d = level();
        d.vel = Vec3::new(2.0, -8.0, 0.0);
        let p = Params::default();
        let imp = d.contact(&Hit { point: Vec3::new(0.0, -1.0, 0.0), normal: Vec3::Y }, &p);
        assert!((imp - 8.0).abs() < 1e-4);
        assert!((d.pos.y - (-1.0 + p.radius)).abs() < 1e-5);
        assert!(d.vel.y > 0.0 && d.vel.y < 8.0 * p.bounce + 1e-3);
        assert!(d.vel.x < 2.0);
    }

    #[test]
    fn slow_contact_does_not_bounce() {
        let mut d = level();
        d.vel = Vec3::new(0.0, -0.5, 0.0);
        d.contact(&Hit { point: Vec3::ZERO, normal: Vec3::Y }, &Params::default());
        assert!(d.vel.y.abs() < 1e-6);
    }

    #[test]
    fn bad_numbers_never_poison_the_pose() {
        let mut d = Drone::from_pose([f32::NAN; 3], [0.0; 4]);
        let s = Sticks { throttle: f32::NAN, yaw: f32::INFINITY, ..Sticks::default() };
        d.step(&s, &Params { gravity: f32::NAN, ..Params::default() }, f32::NAN);
        d.step(&s, &Params::default(), STEP);
        assert!(d.pos.is_finite() && d.vel.is_finite());
        assert!(d.rotation(&Params::default()).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn long_hitch_is_capped() {
        let mut d = level();
        d.step(&Sticks { throttle: 1.0, ..HOVER }, &Params::default(), 100.0);
        assert!(d.pos.length() < 5.0);
    }

    #[test]
    fn default_bindings_map_the_xbox_pad() {
        let b = Bindings::default();
        let mut axes = [0.0; AXES];
        axes[1] = -1.0; // left stick up
        axes[4] = -1.0; // right stick forward
        axes[3] = 1.0; // right stick right
        axes[0] = 1.0; // left stick right
        let s = read_sticks(&b, &axes);
        assert_eq!((s.throttle, s.pitch, s.yaw, s.roll), (1.0, 1.0, 1.0, 1.0));
        assert!((read_sticks(&b, &[0.0; AXES]).throttle - 0.5).abs() < 1e-6);
    }

    #[test]
    fn learning_picks_the_moved_axis_and_sign() {
        let base = [0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0];
        let mut now = base;
        assert_eq!(learn_axis(&base, &now), None);
        now[4] = -0.9;
        assert_eq!(learn_axis(&base, &now), Some(AxisBind { idx: 4, invert: true }));
        now = base;
        now[5] = 1.0; // trigger pulled: +2.0 from rest
        assert_eq!(learn_axis(&base, &now), Some(AxisBind { idx: 5, invert: false }));
    }

    #[test]
    fn learning_a_button_needs_a_fresh_press() {
        let mut prev = [false; BUTTONS];
        prev[0] = true;
        let mut now = prev;
        assert_eq!(learn_button(&prev, &now), None);
        now[7] = true;
        assert_eq!(learn_button(&prev, &now), Some(7));
    }

    #[test]
    fn params_are_sanitized() {
        let p = Params { twr: 1e9, fov: f32::NAN, wind_dir: -90.0, ..Params::default() }.sanitized();
        assert_eq!(p.twr, TWR_RANGE.1);
        assert_eq!(p.fov, Params::default().fov);
        assert_eq!(p.wind_dir, 270.0);
    }
}
