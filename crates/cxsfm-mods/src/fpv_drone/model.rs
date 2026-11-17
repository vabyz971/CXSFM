//! FPV drone flight model (pure math, no Unity — unit-tested).
//!
//! A quadcopter as a rigid body, integrated at ~500 Hz:
//! - four motors with their own spool-up / spool-down lag, thrust ∝ rpm²;
//! - a flight controller like a real one: rate loop + stick feedforward
//!   (Acro), with an angle loop on top (Angle), airmode mixer (torque
//!   kept when a motor saturates or idles);
//! - attitude from the motors' torques (arm × thrust difference, prop
//!   drag for yaw) with inertia and gyroscopic coupling;
//! - aerodynamics: drag per body axis, rotor drag (the H-force that slows
//!   a levelled quad), inflow (thrust fades with speed through the props:
//!   top / climb speed), translational lift, vortex ring + prop wash
//!   (thrust loss and shake when sinking into the own downwash), ground
//!   effect, battery sag on punch-outs, tumble on hard crashes;
//! - three airframes (3" whoop, 5" freestyle, 7" long range).
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
    /// Motor sound (4 synthesized rotors) and its volume.
    pub sound: bool,
    pub sound_volume: f32,
    /// Airframe preset (inertia, motors, drag, sound).
    pub frame: Frame,
    /// Battery voltage sag at full punch-out (fraction of thrust lost).
    pub battery_sag: f32,
    /// Prop-wash shake multiplier (0 = clean air).
    pub propwash: f32,
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
            sound: true,
            sound_volume: 0.6,
            frame: Frame::Freestyle,
            battery_sag: 0.12,
            propwash: 1.0,
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
pub const SOUND_RANGE: (f32, f32) = (0.0, 1.5);
pub const SAG_RANGE: (f32, f32) = (0.0, 0.3);
pub const PROPWASH_RANGE: (f32, f32) = (0.0, 2.0);

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
            sound: self.sound,
            sound_volume: c(self.sound_volume, SOUND_RANGE, d.sound_volume),
            frame: self.frame,
            battery_sag: c(self.battery_sag, SAG_RANGE, d.battery_sag),
            propwash: c(self.propwash, PROPWASH_RANGE, d.propwash),
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

/// 0 below `a`, 1 above `b`, smooth in between.
fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Deterministic turbulence, −1…1 per axis, 6–25 Hz content (the
/// shake of a quad in its own prop wash).
fn wash_noise(t: f32) -> Vec3 {
    let tau = std::f32::consts::TAU;
    let s = |f: f32, ph: f32| (tau * f * t + ph).sin();
    Vec3::new(
        (s(7.3, 0.0) + 0.6 * s(13.1, 1.0) + 0.4 * s(23.7, 2.0)) / 2.0,
        (s(5.9, 0.4) + 0.5 * s(17.3, 2.2)) / 1.5,
        (s(8.1, 1.7) + 0.6 * s(11.9, 0.3) + 0.4 * s(21.1, 2.9)) / 2.0,
    )
}

/// Simulation step (s): motors, rate loop and aerodynamics need
/// ~500 Hz to stay stable; a 60 Hz tick runs ~8 of them.
const SUBSTEP: f32 = 1.0 / 500.0;
/// Rate loop: P gain on roll/pitch and yaw (1/s) and stick feedforward.
const KP_RATE: f32 = 30.0;
const KP_YAW: f32 = 15.0;
const FEEDFORWARD: f32 = 1.0;
/// Angle mode: attitude error → rate (1/s), rate cap (rad/s).
const ANGLE_GAIN: f32 = 8.0;
const ANGLE_MAX_RATE: f32 = 7.0;
/// Angular velocity cap (rad/s) and aerodynamic rotation damping (1/s).
const MAX_OMEGA: f32 = 40.0;
const ROT_DAMP: f32 = 0.5;
/// Always-on linear air drag (1/s), keeps very low thrust stable.
const DRAG_FLOOR: f32 = 0.05;
/// Motor layout, X frame: (x, z) sign of FL, FR, RL, RR, and the sign
/// of the yaw torque each prop's drag puts on the body (props out).
const MOTOR_XZ: [(f32, f32); 4] = [(-1.0, 1.0), (1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
const MOTOR_SPIN: [f32; 4] = [1.0, -1.0, -1.0, 1.0];

/// What the airframe is: everything the presets change. Inertias are
/// per unit of mass (the model works in accelerations).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSpec {
    /// Centre-to-motor distance, m.
    pub arm: f32,
    /// Motor time constants spooling up / down, s (props brake slower).
    pub tau_up: f32,
    pub tau_down: f32,
    /// Prop drag torque per unit of thrust, m (yaw authority).
    pub yaw_k: f32,
    /// Body drag per mass, 1/m, along body x (side), y (top), z (front).
    pub body_drag: [f32; 3],
    /// Rotor drag ("H-force", blade flapping) per m/s of in-plane air
    /// speed, at hover thrust.
    pub rotor_drag: f32,
    /// Air speed through the props at full rpm, m/s: thrust fades as
    /// the inflow approaches it (top and climb speed limit).
    pub pitch_speed: f32,
    /// Idle rotor speed (fraction of full rpm, airmode floor).
    pub idle: f32,
    /// Prop-wash shake at full turbulence, rad/s².
    pub wash: f32,
    /// Sound: shaft frequency at idle and full rpm (Hz), blades per
    /// prop, motor pole pairs.
    pub shaft_hz: (f32, f32),
    pub blades: u32,
    pub poles: u32,
}

impl FrameSpec {
    /// Roll / pitch inertia per mass, m² (a quad ≈ 0.42 arm²).
    fn inertia(&self) -> Vec3 {
        let a2 = self.arm * self.arm;
        Vec3::new(0.42 * a2, 0.75 * a2, 0.42 * a2)
    }

    /// Shaft frequency (Hz) of a rotor at speed `w` (0…1).
    pub fn shaft(&self, w: f32) -> f32 {
        let k = ((finite(w) - self.idle) / (1.0 - self.idle)).clamp(0.0, 1.0);
        self.shaft_hz.0 + (self.shaft_hz.1 - self.shaft_hz.0) * k
    }
}

/// Airframe preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Frame {
    /// 3" ducted cinewhoop: light, draggy, buzzy 4-blade props.
    Whoop,
    /// 5" freestyle: the reference quad.
    #[default]
    Freestyle,
    /// 7" long range: heavier, smoother, slower motors, 2-blade props.
    LongRange,
}

impl Frame {
    pub const ALL: [Frame; 3] = [Frame::Whoop, Frame::Freestyle, Frame::LongRange];

    pub fn spec(self) -> FrameSpec {
        match self {
            Frame::Whoop => FrameSpec {
                arm: 0.065,
                tau_up: 0.02,
                tau_down: 0.035,
                yaw_k: 0.010,
                body_drag: [0.030, 0.050, 0.025],
                rotor_drag: 0.45,
                pitch_speed: 38.0,
                idle: 0.06,
                wash: 45.0,
                shaft_hz: (130.0, 620.0),
                blades: 4,
                poles: 6,
            },
            Frame::Freestyle => FrameSpec {
                arm: 0.115,
                tau_up: 0.03,
                tau_down: 0.05,
                yaw_k: 0.012,
                body_drag: [0.010, 0.018, 0.008],
                rotor_drag: 0.25,
                pitch_speed: 60.0,
                idle: 0.05,
                wash: 30.0,
                shaft_hz: (70.0, 480.0),
                blades: 3,
                poles: 7,
            },
            Frame::LongRange => FrameSpec {
                arm: 0.155,
                tau_up: 0.05,
                tau_down: 0.08,
                yaw_k: 0.014,
                body_drag: [0.008, 0.016, 0.006],
                rotor_drag: 0.20,
                pitch_speed: 62.0,
                idle: 0.05,
                wash: 20.0,
                shaft_hz: (45.0, 300.0),
                blades: 2,
                poles: 7,
            },
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(1)
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL.get(i).copied().unwrap_or_default()
    }
}

/// What the sound needs from one step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Acoustics {
    /// Rotor speeds 0…1 (FL, FR, RL, RR).
    pub motors: [f32; 4],
    /// Mean rotor load (mean of rpm², 0…1).
    pub load: f32,
    /// Speed through the air, m/s.
    pub airspeed: f32,
    /// Dirty air 0…1 (vortex ring / prop wash).
    pub turbulence: f32,
}

/// The drone's own rigid-body state, independent of the game camera.
#[derive(Clone, Copy, Debug)]
pub struct Drone {
    pub pos: Vec3,
    pub vel: Vec3,
    q: Quat,
    /// Body angular velocity (rad/s): x = pitch (nose down +), y = yaw
    /// (right +), z = roll (right wing up +).
    omega: Vec3,
    /// Previous rate target (stick feedforward).
    last_target: Vec3,
    /// Total thrust acceleration (m/s²), as produced last step.
    thrust: f32,
    time: f32,
    /// Last impact speed (m/s), for the UI.
    pub impact: f32,
    /// Rotor speed 0…1 per motor: front-left, front-right, rear-left,
    /// rear-right (X frame).
    motor: [f32; 4],
    /// Height above the ground (m) when known, for ground effect.
    ground: Option<f32>,
    turbulence: f32,
    airspeed: f32,
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
            last_target: Vec3::ZERO,
            thrust: REF_G,
            time: 0.0,
            impact: 0.0,
            motor: [0.5; 4],
            ground: None,
            turbulence: 0.0,
            airspeed: 0.0,
        }
    }

    /// Rotor speeds 0…1 (FL, FR, RL, RR) for the sound and the UI.
    pub fn motors(&self) -> [f32; 4] {
        self.motor
    }

    /// Everything the rotor sound follows.
    pub fn acoustics(&self) -> Acoustics {
        let load = self.motor.iter().map(|w| w * w).sum::<f32>() / 4.0;
        Acoustics { motors: self.motor, load, airspeed: self.airspeed, turbulence: self.turbulence }
    }

    /// Height above the ground, m (`None` = unknown / far).
    pub fn set_ground(&mut self, h: Option<f32>) {
        self.ground = h.filter(|h| h.is_finite() && *h >= 0.0);
    }

    /// Hover thrust, no motion (called when arming).
    pub fn arm(&mut self) {
        self.vel = Vec3::ZERO;
        self.omega = Vec3::ZERO;
        self.last_target = Vec3::ZERO;
        self.thrust = REF_G;
        self.motor = [0.5; 4];
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

    /// Rate the pilot asks for (body rad/s): sticks (Acro) or the
    /// angle loop (Angle: stick tilt, current heading).
    fn rate_target(&self, s: &Sticks, p: &Params) -> Vec3 {
        let yaw_rate = p.yaw_rate.to_radians() * s.yaw;
        match p.mode {
            Mode::Acro => Vec3::new(rate_curve(s.pitch, p.rate, p.expo), yaw_rate, -rate_curve(s.roll, p.rate, p.expo)),
            Mode::Angle => {
                let (yaw, _, _) = self.q.to_euler(EulerRot::YXZ);
                let max = p.max_angle.to_radians();
                let want = Quat::from_euler(EulerRot::YXZ, yaw, s.pitch * max, -s.roll * max);
                let mut err = self.q.inverse() * want;
                if err.w < 0.0 {
                    err = -err;
                }
                let e = err.to_scaled_axis() * ANGLE_GAIN;
                let xz = Vec3::new(e.x, 0.0, e.z);
                let xz = if xz.length() > ANGLE_MAX_RATE { xz.normalize() * ANGLE_MAX_RATE } else { xz };
                Vec3::new(xz.x, yaw_rate, xz.z)
            }
        }
    }

    /// Advance one tick (split into `SUBSTEP`s).
    pub fn step(&mut self, s: &Sticks, p: &Params, dt: f32) {
        let dt = finite(dt).clamp(0.0, MAX_DT);
        if dt <= 0.0 {
            return;
        }
        let p = p.sanitized();
        let f = p.frame.spec();
        let s = Sticks {
            throttle: finite(s.throttle).clamp(0.0, 1.0),
            yaw: finite(s.yaw).clamp(-1.0, 1.0),
            pitch: finite(s.pitch).clamp(-1.0, 1.0),
            roll: finite(s.roll).clamp(-1.0, 1.0),
        };
        self.impact = (self.impact - 4.0 * dt).max(0.0);
        let n = ((dt / SUBSTEP).ceil() as usize).clamp(1, 64);
        let h = dt / n as f32;
        for _ in 0..n {
            self.substep(&s, &p, &f, h);
        }
        if !(self.pos.is_finite() && self.vel.is_finite() && self.q.is_finite() && self.omega.is_finite()) {
            self.vel = Vec3::ZERO;
            self.omega = Vec3::ZERO;
            self.last_target = Vec3::ZERO;
            self.q = Quat::IDENTITY;
            self.motor = [0.5; 4];
            if !self.pos.is_finite() {
                self.pos = Vec3::ZERO;
            }
        }
    }

    fn substep(&mut self, s: &Sticks, p: &Params, f: &FrameSpec, h: f32) {
        self.time += h;
        let inertia = f.inertia();
        let d = f.arm * std::f32::consts::FRAC_1_SQRT_2;

        // 1. Flight controller: rate loop + stick feedforward → torque.
        let target = self.rate_target(s, p);
        let ff = (target - self.last_target) / h * FEEDFORWARD;
        self.last_target = target;
        let err = target - self.omega;
        let alpha_cmd = Vec3::new(KP_RATE * err.x, KP_YAW * err.y, KP_RATE * err.z) + ff;
        let torque = alpha_cmd * inertia;

        // 2. Mixer (thrust per motor, m/s² of the whole drone), airmode
        // desaturation: roll/pitch first, then as much yaw as still
        // fits (yaw is the weak axis, like Betaflight's yaw limit), the
        // collective shifted to fit.
        let tpm = p.twr * REF_G / 4.0;
        let collective = Self::thrust_g(s.throttle, p) * REF_G / 4.0;
        let mut rp = [0f32; 4];
        let mut yw = [0f32; 4];
        for (i, (x, z)) in MOTOR_XZ.iter().enumerate() {
            rp[i] = (-z * torque.x + x * torque.z) / (4.0 * d);
            yw[i] = MOTOR_SPIN[i] * torque.y / (4.0 * f.yaw_k);
        }
        let (lo, hi) = (tpm * f.idle * f.idle, tpm);
        let room = hi - lo;
        let span = |v: &[f32; 4]| {
            let mn = v.iter().copied().fold(f32::MAX, f32::min);
            let mx = v.iter().copied().fold(f32::MIN, f32::max);
            (mn, mx)
        };
        let (mn, mx) = span(&rp);
        if mx - mn > room {
            let k = room / (mx - mn);
            rp.iter_mut().for_each(|v| *v *= k);
        }
        let mixed = |k: f32| std::array::from_fn::<f32, 4, _>(|i| rp[i] + k * yw[i]);
        let fits = |k: f32| {
            let (a, b) = span(&mixed(k));
            b - a <= room + 1e-4
        };
        let mut k = 1.0;
        if !fits(1.0) {
            let (mut a, mut b) = (0.0f32, 1.0f32);
            for _ in 0..10 {
                let m = 0.5 * (a + b);
                if fits(m) { a = m } else { b = m }
            }
            k = a;
        }
        let delta = mixed(k);
        let (mn, mx) = span(&delta);
        let base = collective.max(lo - mn).min(hi - mx);
        for (i, w) in self.motor.iter_mut().enumerate() {
            let cmd = ((base + delta[i]).max(0.0) / tpm).sqrt().clamp(f.idle, 1.0);
            let tau = if cmd > *w { f.tau_up } else { f.tau_down };
            *w += (cmd - *w) * (1.0 - (-h / tau).exp());
        }

        // 3. Rotor aerodynamics → what the props really give.
        let v_rel = self.vel - wind_at(p, self.time);
        self.airspeed = v_rel.length();
        let vb = self.q.inverse() * v_rel;
        let v_ax = vb.y; // + = climbing through the air (inflow)
        let v_pl = (vb.x * vb.x + vb.z * vb.z).sqrt();
        let mean_w = self.motor.iter().sum::<f32>() / 4.0;
        let inflow = (1.0 - v_ax / (f.pitch_speed * mean_w.max(0.15))).clamp(0.2, 1.2);
        let lift = 1.0 + 0.08 * (1.0 - (-v_pl / 6.0).exp());
        // Vortex ring: sinking into your own downwash at low speed —
        // only while the props really push air (near hover or more).
        let pushing = smooth(0.2, 0.45, mean_w);
        let vrs = smooth(2.0, 6.0, -v_ax) * (1.0 - smooth(3.0, 9.0, v_pl)) * pushing;
        let wash = smooth(1.0, 5.0, -v_ax) * pushing;
        self.turbulence = (vrs + 0.5 * wash).min(1.0);
        let up_y = (self.q * Vec3::Y).y.max(0.0);
        let ground = match self.ground {
            Some(gh) => {
                let r = f.arm * 1.15 / (4.0 * gh.max(f.arm * 0.7));
                1.0 + (1.0 / (1.0 - r * r) - 1.0) * up_y
            }
            None => 1.0,
        };
        // Battery sag: only what is pulled above hover.
        let load = self.motor.iter().map(|w| w * w * w).sum::<f32>() / 4.0;
        let hover_load = (1.0 / p.twr).powf(1.5);
        let sag = 1.0 - p.battery_sag * ((load - hover_load) / (1.0 - hover_load).max(1e-3)).clamp(0.0, 1.0);
        let eff = inflow * lift * ground * sag * (1.0 - 0.3 * vrs);
        let thrust: [f32; 4] = std::array::from_fn(|i| tpm * self.motor[i] * self.motor[i] * eff);

        // 4. Rotation: torques of the real thrusts, gyroscopic coupling,
        // air damping, prop-wash shake.
        let mut tq = Vec3::ZERO;
        for (i, (x, z)) in MOTOR_XZ.iter().enumerate() {
            tq.x += -z * d * thrust[i];
            tq.z += x * d * thrust[i];
            tq.y += MOTOR_SPIN[i] * f.yaw_k * thrust[i];
        }
        let mut alpha = (tq - self.omega.cross(inertia * self.omega)) / inertia;
        alpha -= self.omega * ROT_DAMP;
        alpha += wash_noise(self.time) * Vec3::new(1.0, 0.1, 1.0) * (p.propwash * f.wash * self.turbulence);
        self.omega += alpha * h;
        if self.omega.length() > MAX_OMEGA {
            self.omega = self.omega.normalize() * MAX_OMEGA;
        }
        self.q = (self.q * Quat::from_scaled_axis(self.omega * h)).normalize();

        // 5. Translation: thrust along body up, body + rotor drag.
        let total: f32 = thrust.iter().sum();
        self.thrust = total;
        let c = f.body_drag;
        let body = -Vec3::new(c[0] * vb.x * vb.x.abs(), c[1] * vb.y * vb.y.abs(), c[2] * vb.z * vb.z.abs());
        let rotor = -Vec3::new(vb.x, 0.0, vb.z) * (f.rotor_drag * total / REF_G);
        let drag = (body + rotor - vb * DRAG_FLOOR) * p.drag;
        let acc = self.q * (Vec3::Y * total + drag) - Vec3::Y * p.gravity;
        self.vel += acc * h;
        if self.vel.length() > MAX_SPEED {
            self.vel = self.vel.normalize() * MAX_SPEED;
        }
        self.pos += self.vel * h;
    }

    /// Resolve a contact: sit on the surface, bounce, scrub tangential
    /// speed; a hard hit sets the drone tumbling. Returns the impact
    /// speed (m/s) along the normal.
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
        if impact > 3.0 {
            // An arm catches the surface: spin about the axis across
            // the travel, in the body frame.
            let axis = n.cross(tangent.try_normalize().unwrap_or(Vec3::X)).try_normalize().unwrap_or(Vec3::X);
            let kick = (self.q.inverse() * axis) * (impact * 0.8).min(25.0);
            self.omega += kick + Vec3::new(0.0, 0.15 * impact, 0.0);
        }
        self.impact = self.impact.max(impact);
        impact
    }

    /// Camera rotation `[x, y, z, w]`: body attitude + fixed up-tilt.
    pub fn rotation(&self, p: &Params) -> [f32; 4] {
        let tilt = p.sanitized().tilt_deg.to_radians();
        (self.q * Quat::from_rotation_x(-tilt)).normalize().to_array()
    }

    /// Thrust produced, as a 0…1 fraction of the motors' maximum.
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
        // Clean air: this checks the controller, not the prop wash.
        let p = Params { mode: Mode::Acro, expo: 0.0, rate: 360.0, propwash: 0.0, ..Params::default() };
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
    fn pitching_forward_spins_the_rear_rotors_faster() {
        let mut d = level();
        run(&mut d, &Sticks { pitch: 1.0, ..HOVER }, &Params::default(), 0.3);
        let m = d.motors();
        assert!(m[2] > m[0] && m[3] > m[1], "{m:?}");
    }

    #[test]
    fn banking_right_spins_the_left_rotors_faster() {
        let mut d = level();
        run(&mut d, &Sticks { roll: 1.0, ..HOVER }, &Params::default(), 0.3);
        let m = d.motors();
        assert!(m[0] > m[1] && m[2] > m[3], "{m:?}");
    }

    #[test]
    fn rotors_speed_up_with_throttle() {
        let mut lo = level();
        let mut hi = level();
        run(&mut lo, &Sticks { throttle: 0.4, ..HOVER }, &Params::default(), 1.0);
        run(&mut hi, &Sticks { throttle: 1.0, ..HOVER }, &Params::default(), 1.0);
        assert!(hi.motors()[0] > lo.motors()[0] + 0.2);
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

    fn rolled(d: &Drone) -> f32 {
        d.q.to_euler(EulerRot::YXZ).2
    }

    #[test]
    fn motors_spool_with_lag_and_brake_slower() {
        let p = Params::default();
        let mut d = level();
        d.step(&Sticks { throttle: 1.0, ..HOVER }, &p, 0.03);
        let up = d.motors()[0];
        assert!(up > 0.75 && up < 0.95, "about one time constant: {up}");
        run(&mut d, &Sticks { throttle: 1.0, ..HOVER }, &p, 0.5);
        let mut down = d;
        down.step(&Sticks { throttle: 0.0, ..HOVER }, &p, 0.03);
        // Same time, fraction of the gap covered: up (0.5 → 1) vs down
        // (1 → idle).
        let rise = (up - 0.5) / 0.5;
        let idle = Params::default().frame.spec().idle;
        let drop = (1.0 - down.motors()[0]) / (1.0 - idle);
        assert!(drop > 0.2 && drop < rise, "spool-down slower: {drop} vs {rise}");
    }

    #[test]
    fn rotor_drag_slows_a_levelled_quad() {
        let mut d = level();
        d.vel = Vec3::new(10.0, 0.0, 0.0);
        run(&mut d, &HOVER, &Params::default(), 2.0);
        assert!(d.vel.x < 6.0 && d.vel.x > 0.0, "{:?}", d.vel);
        let mut free = level();
        free.vel = Vec3::new(10.0, 0.0, 0.0);
        run(&mut free, &HOVER, &Params { drag: 0.0, ..Params::default() }, 2.0);
        assert!(free.vel.x > 9.5, "{:?}", free.vel);
    }

    #[test]
    fn inflow_caps_the_climb_rate() {
        let mut d = level();
        let full = Sticks { throttle: 1.0, ..HOVER };
        run(&mut d, &full, &Params::default(), 6.0);
        let v6 = d.vel.y;
        run(&mut d, &full, &Params::default(), 2.0);
        // Ballistic 3 g would be +59 m/s by now: the props run out of pitch.
        assert!(v6 > 10.0 && v6 < 40.0, "{v6}");
        assert!((d.vel.y - v6).abs() < 2.0, "terminal climb: {v6} → {}", d.vel.y);
    }

    /// Sinking at hover throttle into the own downwash: thrust loss and
    /// shake (none with propwash off).
    #[test]
    fn vortex_ring_shakes_and_sinks() {
        let sink = |propwash: f32| {
            let p = Params { propwash, ..Params::default() };
            let mut d = level();
            d.vel = Vec3::new(0.0, -5.0, 0.0);
            let mut worst = 0.0f32;
            let mut turb = 0.0f32;
            for _ in 0..60 {
                d.step(&HOVER, &p, STEP);
                worst = worst.max(d.q.angle_between(Quat::IDENTITY));
                turb = turb.max(d.acoustics().turbulence);
            }
            (worst, turb, d.vel.y)
        };
        let (shake, turb, vy) = sink(1.0);
        let (calm, _, _) = sink(0.0);
        assert!(turb > 0.3, "{turb}");
        assert!(shake > 0.005 && calm < shake / 5.0, "shake {shake} vs calm {calm}");
        assert!(vy < -3.0, "thrust lost: still sinking at {vy}");
    }

    #[test]
    fn ground_effect_lifts_near_the_floor() {
        let p = Params::default();
        let mut d = level();
        for _ in 0..60 {
            d.set_ground(Some(0.05 + d.pos.y.max(0.0)));
            d.step(&HOVER, &p, STEP);
        }
        assert!(d.pos.y > 0.05, "{:?}", d.pos);
        let mut far = level();
        far.set_ground(Some(50.0));
        run(&mut far, &HOVER, &p, 1.0);
        assert!(far.pos.y.abs() < 0.02, "{:?}", far.pos);
    }

    #[test]
    fn battery_sags_on_punch_outs_only() {
        let climb = |sag: f32| {
            let mut d = level();
            run(&mut d, &Sticks { throttle: 1.0, ..HOVER }, &Params { battery_sag: sag, ..Params::default() }, 1.0);
            d.pos.y
        };
        assert!(climb(0.3) < climb(0.0) * 0.9);
        let mut d = level();
        run(&mut d, &HOVER, &Params { battery_sag: 0.3, ..Params::default() }, 2.0);
        assert!(d.pos.y.abs() < 0.05, "hover unaffected: {:?}", d.pos);
    }

    #[test]
    fn airmode_keeps_control_at_zero_throttle() {
        let p = Params { mode: Mode::Acro, expo: 0.0, ..Params::default() };
        let mut d = level();
        run(&mut d, &Sticks { throttle: 0.0, roll: 1.0, ..HOVER }, &p, 0.3);
        assert!(rolled(&d) < -0.5, "{}", rolled(&d));
    }

    #[test]
    fn yaw_is_the_weak_axis() {
        let p = Params { mode: Mode::Acro, expo: 0.0, rate: 600.0, yaw_rate: 500.0, ..Params::default() };
        let mut r = level();
        run(&mut r, &Sticks { roll: 1.0, ..HOVER }, &p, 0.1);
        let mut y = level();
        run(&mut y, &Sticks { yaw: 1.0, ..HOVER }, &p, 0.1);
        let yawed = y.q.to_euler(EulerRot::YXZ).0;
        assert!(yawed > 0.05 && yawed < rolled(&r).abs() * 0.8, "yaw {yawed} vs roll {}", rolled(&r));
    }

    #[test]
    fn hard_crash_tumbles() {
        let mut d = level();
        d.vel = Vec3::new(6.0, -8.0, 0.0);
        d.contact(&Hit { point: Vec3::ZERO, normal: Vec3::Y }, &Params::default());
        assert!(d.omega.length() > 3.0, "{:?}", d.omega);
        let mut soft = level();
        soft.vel = Vec3::new(1.0, -2.0, 0.0);
        soft.contact(&Hit { point: Vec3::ZERO, normal: Vec3::Y }, &Params::default());
        assert!(soft.omega.length() < 1e-6);
    }

    #[test]
    fn small_frames_react_faster() {
        let flip = |frame: Frame| {
            let p = Params { mode: Mode::Acro, expo: 0.0, rate: 800.0, frame, ..Params::default() };
            let mut d = level();
            run(&mut d, &Sticks { roll: 1.0, ..HOVER }, &p, 0.06);
            -rolled(&d)
        };
        let (w, f, l) = (flip(Frame::Whoop), flip(Frame::Freestyle), flip(Frame::LongRange));
        assert!(w > f && f > l, "whoop {w} > 5\" {f} > 7\" {l}");
    }

    #[test]
    fn flight_is_deterministic() {
        let fly = || {
            let p = Params { gust: 3.0, wind_speed: 4.0, ..Params::default() };
            let mut d = level();
            run(&mut d, &Sticks { throttle: 0.7, pitch: 0.6, roll: -0.3, yaw: 0.2 }, &p, 2.0);
            (d.pos, d.q)
        };
        assert_eq!(fly(), fly());
    }

    #[test]
    fn frame_presets_round_trip_and_stay_sane() {
        for (i, f) in Frame::ALL.iter().enumerate() {
            assert_eq!(Frame::from_index(f.index()), *f);
            assert_eq!(f.index(), i);
            let s = f.spec();
            assert!(s.tau_down > s.tau_up && s.idle > 0.0 && s.shaft_hz.1 > s.shaft_hz.0);
            assert!((s.shaft(1.0) - s.shaft_hz.1).abs() < 1e-3 && (s.shaft(0.0) - s.shaft_hz.0).abs() < 1e-3);
            // Every airframe hovers at centre stick.
            let mut d = level();
            run(&mut d, &HOVER, &Params { frame: *f, ..Params::default() }, 2.0);
            assert!(d.pos.length() < 0.05, "{f:?} {:?}", d.pos);
        }
    }
}
