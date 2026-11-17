//! Drone sound on FMOD Core: four rotors plus two noise layers.
//!
//! What a quad really sounds like, rebuilt from its parts:
//! - **rotor tone** (one channel per motor, panned left/right): blade-
//!   passing frequency (shaft × blades) and its harmonics, a weak shaft
//!   harmonic (prop imbalance) and the motor's electrical whine (shaft ×
//!   pole pairs). Written as a 1 s loop at a 100 Hz shaft and pitched by
//!   each motor's real rpm (airframe range), so every partial follows
//!   the motor exactly and rotors at different speeds beat;
//! - **prop noise** (broadband turbulence): level follows the rotor
//!   *load*, not the pitch, and rises in dirty air;
//! - **airflow** (low rumble on the frame): follows the speed through
//!   the air.
//!
//! In dirty air (vortex ring, prop wash) the rotor volumes flutter.
//!
//! Loops are built in memory as WAV files: tones with integer-Hz
//! partials from a sine table (exact, seamless), noise as filtered
//! white noise cross-faded into a seamless loop. The game's FMOD core
//! system is reached through its Studio system
//! (`FMOD_Studio_System_GetCoreSystem`); channels go to the master
//! channel group (the mod has its own volume).
//!
//! `FMOD_CREATESOUNDEXINFO` is version-dependent: only `cbsize` and
//! `length` are set, and `cbsize` is probed until FMOD accepts it.

use super::model::{Acoustics, FrameSpec};
use cxsfm_sys::fmod::{open_loaded, sym};
use std::ffi::{c_char, c_int, c_uint, c_void};

type Handle = *mut c_void;

const RATE: u32 = 44_100;
/// Loop length in samples (exactly one second: integer-Hz partials).
const LOOP_LEN: usize = RATE as usize;
/// Shaft frequency the rotor loop is written at (pitch 1.0).
const LOOP_SHAFT: f32 = 100.0;
/// Highest partial allowed at the top pitch (stays clear of Nyquist).
const MAX_PARTIAL_HZ: f32 = 16_000.0;
/// `LOOP_NORMAL | 2D | CREATESAMPLE | OPENMEMORY`.
const SOUND_MODE: c_uint = 0x2 | 0x8 | 0x100 | 0x800;
/// Pan per motor (FL, FR, RL, RR) and a tiny static detune (no two
/// motors are identical).
const PAN: [f32; 4] = [-0.6, 0.6, -0.6, 0.6];
const DETUNE: [f32; 4] = [0.997, 1.003, 1.002, 0.998];
/// Channels: 4 rotors, prop noise, airflow.
const NOISE: usize = 4;
const AIR: usize = 5;
const CHANNELS: usize = 6;

fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

/// Rotor spectrum for an airframe: `(harmonic k of the loop shaft,
/// amplitude)`. Partials are cut so that the top pitch keeps them
/// under `MAX_PARTIAL_HZ`.
pub fn rotor_partials(f: &FrameSpec) -> Vec<(u32, f32)> {
    let top = (f.shaft_hz.1 / LOOP_SHAFT).max(1.0);
    let kmax = (MAX_PARTIAL_HZ / (LOOP_SHAFT * top)).floor().max(1.0) as u32;
    let (b, poles) = (f.blades.max(1), f.poles.max(1));
    let mut seed = 0x51ed_270bu32 ^ (b * 977 + poles);
    let mut out = Vec::new();
    for k in 1..=kmax {
        let mut a = 0.0;
        if k % b == 0 {
            // Blade passing and its harmonics: the drone "tone".
            a += 1.0 / ((k / b) as f32).powf(0.85);
        }
        if k == 1 {
            a += 0.12; // shaft: slight prop imbalance
        } else if k % b != 0 {
            a += 0.03 * lcg(&mut seed); // blade-to-blade differences
        }
        if k % poles == 0 {
            a += 0.07 / (k / poles) as f32; // motor whine (commutation)
        }
        if a > 0.0 {
            out.push((k, a));
        }
    }
    out
}

/// 16-bit mono WAV of `pcm` (normalised to 80 % full scale).
fn wav(pcm: &[f32]) -> Vec<u8> {
    let peak = pcm.iter().fold(1e-6f32, |m, v| m.max(v.abs()));
    let data_len = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&((s / peak * 0.8 * 32767.0) as i16).to_le_bytes());
    }
    w
}

/// The rotor tone loop of an airframe (integer-Hz partials, exact sine
/// table lookups: the loop seam is click-free).
pub fn rotor_wav(f: &FrameSpec) -> Vec<u8> {
    let tau = std::f32::consts::TAU;
    let table: Vec<f32> = (0..LOOP_LEN).map(|i| (tau * i as f32 / LOOP_LEN as f32).sin()).collect();
    let mut seed = 0x1234_5678u32;
    let mut pcm = vec![0f32; LOOP_LEN];
    for (k, a) in rotor_partials(f) {
        let hz = (k as f32 * LOOP_SHAFT) as u64;
        let phase = (lcg(&mut seed) * LOOP_LEN as f32) as u64;
        for (i, s) in pcm.iter_mut().enumerate() {
            *s += a * table[((hz * i as u64 + phase) % LOOP_LEN as u64) as usize];
        }
    }
    wav(&pcm)
}

/// Band-limited noise loop: white noise through `lo`/`hi` one-pole
/// filters (each applied `order` times), made seamless by cross-fading
/// its tail into its head.
pub fn noise_wav(lo: f32, hi: f32, order: usize, seed: u32) -> Vec<u8> {
    let fade = LOOP_LEN / 5;
    let n = LOOP_LEN + fade;
    let mut st = seed;
    let k = |fc: f32| 1.0 - (-std::f32::consts::TAU * fc / RATE as f32).exp();
    let (kl, kh) = (k(hi), k(lo));
    let mut lp = vec![0f32; order];
    let mut hp = vec![0f32; order];
    let mut raw = Vec::with_capacity(n);
    for _ in 0..n {
        let mut x = lcg(&mut st) * 2.0 - 1.0;
        for j in 0..order {
            lp[j] += kl * (x - lp[j]);
            x = lp[j];
            hp[j] += kh * (x - hp[j]);
            x -= hp[j];
        }
        raw.push(x);
    }
    let mut pcm: Vec<f32> = raw[..LOOP_LEN].to_vec();
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        pcm[i] = raw[LOOP_LEN + i] * (1.0 - t) + raw[i] * t;
    }
    wav(&pcm)
}

/// Rotor speed (0…1) → (channel pitch, volume factor) before master.
pub fn rotor_voice(f: &FrameSpec, rpm: f32, motor: usize) -> (f32, f32) {
    let r = if rpm.is_finite() { rpm.clamp(0.0, 1.0) } else { 0.0 };
    (f.shaft(r) / LOOP_SHAFT * DETUNE[motor % 4], 0.16 * (0.12 + 0.88 * r.powf(1.8)))
}

/// (pitch, volume) of the prop-noise layer: louder with the load,
/// louder still in dirty air.
pub fn noise_voice(ac: &Acoustics) -> (f32, f32) {
    let load = if ac.load.is_finite() { ac.load.clamp(0.0, 1.0) } else { 0.0 };
    let turb = if ac.turbulence.is_finite() { ac.turbulence.clamp(0.0, 1.0) } else { 0.0 };
    let mean = ac.motors.iter().map(|w| if w.is_finite() { w.clamp(0.0, 1.0) } else { 0.0 }).sum::<f32>() / 4.0;
    (0.8 + 0.4 * mean, (0.05 + 0.3 * load) * (1.0 + 1.2 * turb))
}

/// (pitch, volume) of the airflow layer: rises with the air speed.
pub fn air_voice(ac: &Acoustics) -> (f32, f32) {
    let v = if ac.airspeed.is_finite() { ac.airspeed.max(0.0) } else { 0.0 };
    let k = (v / 35.0).min(1.0);
    (0.7 + 0.6 * k, 0.4 * k * k)
}

type FnIsValid = unsafe extern "C" fn(Handle) -> c_int;
type FnGetCore = unsafe extern "C" fn(Handle, *mut Handle) -> c_int;
type FnCreate = unsafe extern "C" fn(Handle, *const c_char, c_uint, *mut c_void, *mut Handle) -> c_int;
type FnPlay = unsafe extern "C" fn(Handle, Handle, Handle, c_int, *mut Handle) -> c_int;
type FnSetF = unsafe extern "C" fn(Handle, f32) -> c_int;
type FnSetI = unsafe extern "C" fn(Handle, c_int) -> c_int;
type FnHandle = unsafe extern "C" fn(Handle) -> c_int;

struct Fns {
    studio_valid: FnIsValid,
    get_core: FnGetCore,
    create: FnCreate,
    play: FnPlay,
    volume: FnSetF,
    pitch: FnSetF,
    pan: FnSetF,
    paused: FnSetI,
    stop: FnHandle,
    release: FnHandle,
}

#[derive(Default)]
pub(crate) struct Audio {
    fns: Option<Fns>,
    tried: bool,
    failed: bool,
    core: usize,
    /// `FMOD_CREATESOUNDEXINFO::cbsize` FMOD accepted (0 = not probed).
    exinfo: u32,
    /// Loops: rotor tone, prop noise, airflow (0 = not created), and
    /// their WAV bytes (kept alive with the sound).
    sounds: [usize; 3],
    wavs: [Vec<u8>; 3],
    /// Airframe the rotor loop was built for.
    rotor_for: Option<FrameSpec>,
    chans: [usize; CHANNELS],
    flutter: u32,
}

fn h(v: usize) -> Handle {
    v as Handle
}

impl Audio {
    pub(crate) fn load(&mut self) -> bool {
        if self.fns.is_some() {
            return true;
        }
        if self.tried {
            return false;
        }
        self.tried = true;
        let Some(lib) = open_loaded() else {
            self.failed = true;
            return false;
        };
        // SAFETY: documented FMOD 2.x C signatures, NUL-terminated names.
        let f = unsafe {
            (|| {
                Some(Fns {
                    studio_valid: sym(lib, c"FMOD_Studio_System_IsValid")?,
                    get_core: sym(lib, c"FMOD_Studio_System_GetCoreSystem")?,
                    create: sym(lib, c"FMOD_System_CreateSound")?,
                    play: sym(lib, c"FMOD_System_PlaySound")?,
                    volume: sym(lib, c"FMOD_Channel_SetVolume")?,
                    pitch: sym(lib, c"FMOD_Channel_SetPitch")?,
                    pan: sym(lib, c"FMOD_Channel_SetPan")?,
                    paused: sym(lib, c"FMOD_Channel_SetPaused")?,
                    stop: sym(lib, c"FMOD_Channel_Stop")?,
                    release: sym(lib, c"FMOD_Sound_Release")?,
                })
            })()
        };
        self.failed = f.is_none();
        self.fns = f;
        self.fns.is_some()
    }

    /// Is `studio` still a live Studio system? (Call `load` first.)
    pub(crate) fn studio_valid(&self, studio: usize) -> bool {
        // SAFETY: FMOD validates the handle itself.
        studio != 0 && self.fns.as_ref().is_some_and(|f| unsafe { (f.studio_valid)(h(studio)) } != 0)
    }

    /// Create a looping sound from WAV bytes, probing `cbsize` once.
    fn create(&mut self, bytes: &[u8]) -> Option<usize> {
        let f = self.fns.as_ref()?;
        let sizes: Vec<u32> = if self.exinfo != 0 { vec![self.exinfo] } else { (64u32..=512).step_by(4).collect() };
        for size in sizes {
            let mut info = [0u32; 256];
            info[0] = size;
            info[1] = bytes.len() as u32;
            let mut sound: Handle = std::ptr::null_mut();
            // SAFETY: memory file stays alive in `self.wavs`; `info` is
            // a zeroed 1 KiB block larger than any FMOD version's struct.
            let r = unsafe {
                (f.create)(h(self.core), bytes.as_ptr() as *const c_char, SOUND_MODE, info.as_mut_ptr() as *mut c_void, &mut sound)
            };
            if r == 0 && !sound.is_null() {
                if self.exinfo == 0 {
                    cxsfm_api::log_line(&format!("fpv_drone: rotor sound created (exinfo size {size})"));
                }
                self.exinfo = size;
                return Some(sound as usize);
            }
        }
        cxsfm_api::log_line("fpv_drone: could not create the drone sounds (no exinfo size accepted), sound off");
        self.failed = true;
        None
    }

    /// Stop channels `range` and release loop `slot`.
    fn drop_loop(&mut self, slot: usize, range: std::ops::Range<usize>) {
        if let Some(f) = self.fns.as_ref() {
            for c in self.chans[range].iter_mut().filter(|c| **c != 0) {
                // SAFETY: FMOD validates the channel handle.
                unsafe { (f.stop)(h(*c)) };
                *c = 0;
            }
            if self.sounds[slot] != 0 {
                // SAFETY: sound created by us, released once.
                unsafe { (f.release)(h(self.sounds[slot])) };
            }
        }
        self.sounds[slot] = 0;
    }

    /// Build what is missing: the rotor loop (again when the airframe
    /// changed), the two noise loops.
    fn make_sounds(&mut self, spec: &FrameSpec) -> bool {
        if self.sounds[0] != 0 && self.rotor_for.as_ref() != Some(spec) {
            self.drop_loop(0, 0..4);
        }
        for slot in 0..3 {
            if self.sounds[slot] != 0 {
                continue;
            }
            let bytes = match slot {
                0 => rotor_wav(spec),
                1 => noise_wav(350.0, 7_000.0, 2, 0x0bad_5eed),
                _ => noise_wav(40.0, 900.0, 2, 0x5ca1_ab1e),
            };
            self.wavs[slot] = bytes;
            let bytes = std::mem::take(&mut self.wavs[slot]);
            let made = self.create(&bytes);
            self.wavs[slot] = bytes;
            match made {
                Some(sn) => self.sounds[slot] = sn,
                None => return false,
            }
            if slot == 0 {
                self.rotor_for = Some(*spec);
            }
        }
        true
    }

    /// Drive the rotors and noise layers. `false` = the Studio handle
    /// is no longer good (caller should look it up again).
    pub(crate) fn update(&mut self, studio: usize, ac: &Acoustics, spec: &FrameSpec, master: f32) -> bool {
        if self.failed || !self.load() {
            return true;
        }
        if self.core == 0 {
            let Some(f) = self.fns.as_ref() else { return true };
            let mut core: Handle = std::ptr::null_mut();
            // SAFETY: live out param; FMOD validates the studio handle.
            if unsafe { (f.get_core)(h(studio), &mut core) } != 0 || core.is_null() {
                return false;
            }
            self.core = core as usize;
        }
        if !self.make_sounds(spec) {
            return true;
        }
        let master = if master.is_finite() { master.clamp(0.0, 1.5) } else { 0.0 };
        let turb = if ac.turbulence.is_finite() { ac.turbulence.clamp(0.0, 1.0) } else { 0.0 };
        let mut voices = [(0usize, 1.0f32, 0.0f32, 0.0f32); CHANNELS];
        for (i, v) in voices.iter_mut().enumerate().take(4) {
            let (pitch, vol) = rotor_voice(spec, ac.motors[i], i);
            // Dirty air: each rotor's level chops on its own.
            let chop = 1.0 - 0.45 * turb * lcg(&mut self.flutter);
            *v = (0, pitch, vol * chop, PAN[i]);
        }
        let (np, nv) = noise_voice(ac);
        voices[NOISE] = (1, np, nv, 0.0);
        let (ap, av) = air_voice(ac);
        voices[AIR] = (2, ap, av, 0.0);
        let Some(f) = self.fns.as_ref() else { return true };
        for (i, (slot, pitch, vol, pan)) in voices.into_iter().enumerate() {
            // SAFETY: handles validated by FMOD on every call.
            unsafe {
                if self.chans[i] == 0 {
                    let mut ch: Handle = std::ptr::null_mut();
                    if (f.play)(h(self.core), h(self.sounds[slot]), std::ptr::null_mut(), 1, &mut ch) != 0 || ch.is_null() {
                        continue;
                    }
                    (f.pan)(ch, pan);
                    (f.volume)(ch, 0.0);
                    (f.paused)(ch, 0);
                    self.chans[i] = ch as usize;
                }
                let ch = h(self.chans[i]);
                let ok = (f.volume)(ch, vol * master) == 0 && (f.pitch)(ch, pitch) == 0;
                if !ok {
                    // Channel stolen / stopped: play it again next tick.
                    self.chans[i] = 0;
                }
            }
        }
        true
    }

    /// Silence: stop the channels and free the loops.
    pub(crate) fn stop(&mut self) {
        self.drop_loop(0, 0..4);
        self.drop_loop(1, NOISE..NOISE + 1);
        self.drop_loop(2, AIR..AIR + 1);
        self.chans = [0; CHANNELS];
        self.rotor_for = None;
        self.core = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fpv_drone::model::Frame;

    fn sample(w: &[u8], i: usize) -> i32 {
        i16::from_le_bytes([w[44 + 2 * i], w[45 + 2 * i]]) as i32
    }

    fn well_formed(w: &[u8]) {
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(w.len(), 44 + LOOP_LEN * 2);
        let riff = u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, w.len());
    }

    /// The jump from the last sample back to the first is no bigger
    /// than the loop's own sample-to-sample steps.
    fn seamless(w: &[u8]) {
        let max_step = (1..LOOP_LEN).map(|i| (sample(w, i) - sample(w, i - 1)).abs()).max().unwrap();
        let seam = (sample(w, 0) - sample(w, LOOP_LEN - 1)).abs();
        assert!(seam <= max_step, "seam {seam} vs max step {max_step}");
    }

    #[test]
    fn loops_are_well_formed_and_seamless() {
        for f in Frame::ALL {
            let w = rotor_wav(&f.spec());
            well_formed(&w);
            seamless(&w);
        }
        for w in [noise_wav(350.0, 7_000.0, 2, 1), noise_wav(40.0, 900.0, 2, 2)] {
            well_formed(&w);
            seamless(&w);
        }
    }

    #[test]
    fn rotor_spectrum_is_blade_pass_and_never_aliases() {
        for f in Frame::ALL {
            let s = f.spec();
            let parts = rotor_partials(&s);
            let (k, _) = parts.iter().copied().fold((0, 0.0f32), |b, p| if p.1 > b.1 { p } else { b });
            assert_eq!(k, s.blades, "{f:?}: the blade-pass line dominates");
            let top = s.shaft_hz.1 / LOOP_SHAFT;
            let highest = parts.iter().map(|p| p.0).max().unwrap() as f32 * LOOP_SHAFT * top;
            assert!(highest <= MAX_PARTIAL_HZ + 1.0, "{f:?} reaches {highest} Hz");
            assert!(parts.iter().any(|p| p.0 % s.poles == 0 && p.0 % s.blades != 0), "{f:?}: motor whine present");
        }
    }

    #[test]
    fn voices_follow_the_drone() {
        let s = Frame::Freestyle.spec();
        let (p_lo, v_lo) = rotor_voice(&s, 0.2, 0);
        let (p_hi, v_hi) = rotor_voice(&s, 0.9, 0);
        assert!(p_hi > p_lo && v_hi > v_lo);
        // Hover-ish blade pass of a 5" lands in the hundreds of Hz.
        let bpf = s.shaft(0.5) * s.blades as f32;
        assert!(bpf > 300.0 && bpf < 1_500.0, "{bpf}");
        assert!(rotor_voice(&s, f32::NAN, 1).0.is_finite());
        let calm = Acoustics { motors: [0.5; 4], load: 0.25, airspeed: 0.0, turbulence: 0.0 };
        let dirty = Acoustics { turbulence: 1.0, ..calm };
        assert!(noise_voice(&dirty).1 > noise_voice(&calm).1);
        assert_eq!(air_voice(&calm).1, 0.0);
        assert!(air_voice(&Acoustics { airspeed: 30.0, ..calm }).1 > 0.2);
        let nan = Acoustics { motors: [f32::NAN; 4], load: f32::NAN, airspeed: f32::NAN, turbulence: f32::NAN };
        assert!(noise_voice(&nan).0.is_finite() && noise_voice(&nan).1.is_finite() && air_voice(&nan).1.is_finite());
    }
}
