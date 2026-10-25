//! FPV drone flight model (pure math, no Unity — unit-tested).
//!
//! Conventions are Unity's: +Y up, +Z forward, `pitch` positive =
//! looking DOWN (rotation about +X). Quaternion numerics from `glam`
//! match Unity's for the same axis/angle composition.

use glam::{EulerRot, Quat, Vec3};

/// Pitch clamp (rad, ~89°): no flip over the poles.
const PITCH_LIMIT: f32 = 1.553;
/// Longest step integrated at once (a hitch must not teleport).
const MAX_DT: f32 = 0.1;
/// Velocity / bank smoothing rates (1/s).
const ACCEL_RATE: f32 = 6.0;
const BANK_RATE: f32 = 8.0;
/// Full-stick bank angle at `bank == 1` (rad, ~28°).
const MAX_BANK: f32 = 0.5;

pub const SPEED_RANGE: (f32, f32) = (1.0, 120.0);
pub const BOOST_RANGE: (f32, f32) = (1.0, 5.0);
pub const TURN_RANGE: (f32, f32) = (30.0, 360.0);
pub const TILT_RANGE: (f32, f32) = (0.0, 60.0);
pub const FOV_RANGE: (f32, f32) = (50.0, 140.0);

/// User-tunable flight parameters (UI → tick).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Cruise speed, m/s.
    pub speed: f32,
    /// Speed multiplier while boosting.
    pub boost: f32,
    /// Yaw/pitch rate at full stick, deg/s.
    pub turn_deg_s: f32,
    /// Fixed camera up-tilt (FPV drones film angled up), degrees.
    pub tilt_deg: f32,
    /// Roll into turns, 0 (none) … 1.
    pub bank: f32,
    pub fov: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self { speed: 20.0, boost: 2.5, turn_deg_s: 120.0, tilt_deg: 25.0, bank: 0.3, fov: 100.0 }
    }
}

impl Params {
    /// Every field clamped to its range (NaN → default).
    pub fn sanitized(&self) -> Self {
        let d = Self::default();
        let c = |v: f32, (lo, hi): (f32, f32), def: f32| if v.is_nan() { def } else { v.clamp(lo, hi) };
        Self {
            speed: c(self.speed, SPEED_RANGE, d.speed),
            boost: c(self.boost, BOOST_RANGE, d.boost),
            turn_deg_s: c(self.turn_deg_s, TURN_RANGE, d.turn_deg_s),
            tilt_deg: c(self.tilt_deg, TILT_RANGE, d.tilt_deg),
            bank: c(self.bank, (0.0, 1.0), d.bank),
            fov: c(self.fov, FOV_RANGE, d.fov),
        }
    }
}

/// Normalized controls, each −1..1 (`pitch` positive = look up).
#[derive(Clone, Copy, Default, Debug)]
pub struct Input {
    pub fwd: f32,
    pub right: f32,
    pub up: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub boost: bool,
}

/// The drone's own pose, independent of the game camera.
#[derive(Clone, Copy, Debug)]
pub struct Drone {
    pub pos: Vec3,
    vel: Vec3,
    yaw: f32,
    pitch: f32,
    roll: f32,
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

impl Drone {
    /// Start where the game camera is, looking where it looks (no roll).
    pub fn from_pose(pos: [f32; 3], rot: [f32; 4]) -> Self {
        let q = Quat::from_xyzw(rot[0], rot[1], rot[2], rot[3]);
        let q = if q.is_finite() && q.length_squared() > 1e-6 { q.normalize() } else { Quat::IDENTITY };
        let (yaw, pitch, _) = q.to_euler(EulerRot::YXZ);
        let pos = Vec3::from_array(pos);
        Self {
            pos: if pos.is_finite() { pos } else { Vec3::ZERO },
            vel: Vec3::ZERO,
            yaw,
            pitch: pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT),
            roll: 0.0,
        }
    }

    /// Advance one tick.
    pub fn step(&mut self, i: &Input, p: &Params, dt: f32) {
        let dt = finite(dt).clamp(0.0, MAX_DT);
        let p = p.sanitized();
        let rate = p.turn_deg_s.to_radians();
        self.yaw += finite(i.yaw).clamp(-1.0, 1.0) * rate * dt;
        self.yaw = self.yaw.rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch - finite(i.pitch).clamp(-1.0, 1.0) * rate * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);

        let local = Vec3::new(finite(i.right), 0.0, finite(i.fwd));
        // Heading only: pitch never bleeds into altitude (natural
        // flight: the sticks move on the ground plane, triggers climb).
        let mut dir = Quat::from_rotation_y(self.yaw) * local + Vec3::Y * finite(i.up);
        if dir.length() > 1.0 {
            dir = dir.normalize();
        }
        let mult = if i.boost { p.boost } else { 1.0 };
        let target = dir * p.speed * mult;
        self.vel += (target - self.vel) * (1.0 - (-ACCEL_RATE * dt).exp());
        self.pos += self.vel * dt;

        let roll_target = -finite(i.yaw).clamp(-1.0, 1.0) * p.bank * MAX_BANK;
        self.roll += (roll_target - self.roll) * (1.0 - (-BANK_RATE * dt).exp());
    }

    /// Camera rotation `[x, y, z, w]`: look + fixed up-tilt + bank.
    pub fn rotation(&self, p: &Params) -> [f32; 4] {
        let tilt = p.sanitized().tilt_deg.to_radians();
        let q = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch - tilt, self.roll).normalize();
        q.to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(d: &mut Drone, i: &Input, p: &Params) {
        for _ in 0..240 {
            d.step(i, p, 1.0 / 60.0);
        }
    }

    #[test]
    fn forward_follows_yaw() {
        // Unity yaw 90° = facing +X.
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut d = Drone::from_pose([0.0; 3], q.to_array());
        settle(&mut d, &Input { fwd: 1.0, ..Input::default() }, &Params::default());
        assert!(d.pos.x > 10.0 && d.pos.z.abs() < 0.5, "{:?}", d.pos);
    }

    #[test]
    fn looking_down_does_not_lose_altitude() {
        let mut d = Drone::from_pose([0.0; 3], Quat::IDENTITY.to_array());
        for _ in 0..30 {
            d.step(&Input { pitch: -1.0, ..Input::default() }, &Params::default(), 1.0 / 60.0);
        }
        settle(&mut d, &Input { fwd: 1.0, ..Input::default() }, &Params::default());
        assert!(d.pos.y.abs() < 0.01, "{:?}", d.pos);
        assert!(d.pos.z > 10.0);
    }

    #[test]
    fn up_input_climbs() {
        let mut d = Drone::from_pose([0.0; 3], Quat::IDENTITY.to_array());
        settle(&mut d, &Input { up: 1.0, ..Input::default() }, &Params::default());
        assert!(d.pos.y > 10.0);
    }

    #[test]
    fn pitch_is_clamped() {
        let mut d = Drone::from_pose([0.0; 3], Quat::IDENTITY.to_array());
        for _ in 0..600 {
            d.step(&Input { pitch: -1.0, ..Input::default() }, &Params::default(), 0.05);
        }
        assert!(d.pitch <= PITCH_LIMIT + 1e-6);
    }

    #[test]
    fn bad_numbers_never_poison_the_pose() {
        let mut d = Drone::from_pose([f32::NAN; 3], [0.0; 4]);
        d.step(&Input { fwd: f32::NAN, yaw: f32::INFINITY, ..Input::default() }, &Params::default(), f32::NAN);
        assert!(d.pos.is_finite());
        assert!(d.rotation(&Params::default()).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn long_hitch_is_capped() {
        let mut d = Drone::from_pose([0.0; 3], Quat::IDENTITY.to_array());
        d.step(&Input { fwd: 1.0, boost: true, ..Input::default() }, &Params::default(), 100.0);
        assert!(d.pos.length() < 10.0);
    }

    #[test]
    fn params_are_sanitized() {
        let p = Params { speed: 1e9, fov: f32::NAN, ..Params::default() }.sanitized();
        assert_eq!(p.speed, SPEED_RANGE.1);
        assert_eq!(p.fov, Params::default().fov);
    }
}
