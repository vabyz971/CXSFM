//! Drone motor sound: four rotors on FMOD Core.
//!
//! A one-second periodic rotor loop (harmonics + whine + prop noise, all
//! at integer Hz so the loop seam is click-free) is built in memory as a
//! WAV and played on four 2D channels, one per motor. Each channel gets
//! its own pitch (rotor speed), volume and pan (left/right side of the
//! frame), so a motor spinning faster than the others is audible.
//!
//! The game's FMOD core system is reached through its Studio system
//! (`FMOD_Studio_System_GetCoreSystem`). Channels go to the master
//! channel group, so the game's own volume sliders do not apply — the
//! mod has its own volume.
//!
//! `FMOD_CREATESOUNDEXINFO` is version-dependent: only `cbsize` and
//! `length` are set, and `cbsize` is probed until FMOD accepts it.

use crate::fmod_sys::{open_loaded, sym};
use std::ffi::{c_char, c_int, c_uint, c_void};

type Handle = *mut c_void;

const RATE: u32 = 44_100;
/// Loop length in samples (exactly one second: integer-Hz partials).
const LOOP_LEN: usize = RATE as usize;
/// `LOOP_NORMAL | 2D | CREATESAMPLE | OPENMEMORY`.
const SOUND_MODE: c_uint = 0x2 | 0x8 | 0x100 | 0x800;
/// Pan per motor (FL, FR, RL, RR) and a small detune so the rotors beat.
const PAN: [f32; 4] = [-0.8, 0.8, -0.8, 0.8];
const DETUNE: [f32; 4] = [0.992, 1.008, 1.004, 0.996];

fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

/// The rotor loop as a 16-bit mono WAV file.
pub fn synth_wav() -> Vec<u8> {
    let mut seed = 0x1234_5678u32;
    let tau = std::f32::consts::TAU;
    // (frequency Hz, amplitude, phase): blade-pass harmonics of 200 Hz,
    // a high whine, and prop noise as many random-phase partials.
    let mut partials: Vec<(f32, f32, f32)> = Vec::new();
    for h in 1..=14 {
        partials.push((200.0 * h as f32, 1.0 / (h as f32).powf(0.9), lcg(&mut seed) * tau));
    }
    partials.push((3_100.0, 0.10, lcg(&mut seed) * tau));
    partials.push((6_200.0, 0.05, lcg(&mut seed) * tau));
    for _ in 0..90 {
        let f = 700.0 + lcg(&mut seed) * 7_500.0;
        partials.push((f.round(), 0.035, lcg(&mut seed) * tau));
    }
    let mut pcm = vec![0f32; LOOP_LEN];
    for (f, a, ph) in &partials {
        for (i, s) in pcm.iter_mut().enumerate() {
            *s += a * (tau * f * i as f32 / RATE as f32 + ph).sin();
        }
    }
    let peak = pcm.iter().fold(1e-6f32, |m, v| m.max(v.abs()));
    let data_len = (LOOP_LEN * 2) as u32;
    let mut wav = Vec::with_capacity(44 + LOOP_LEN * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        wav.extend_from_slice(&((s / peak * 0.8 * 32767.0) as i16).to_le_bytes());
    }
    wav
}

/// Rotor speed (0…1) → (channel pitch, volume factor) before master.
pub fn rotor_voice(rpm: f32, motor: usize) -> (f32, f32) {
    let r = if rpm.is_finite() { rpm.clamp(0.0, 1.0) } else { 0.0 };
    ((0.45 + 2.0 * r) * DETUNE[motor % 4], 0.3 * (0.35 + 0.65 * r))
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
    sound: usize,
    chans: [usize; 4],
    wav: Vec<u8>,
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

    /// Create the loop sound, probing `cbsize` until FMOD accepts it.
    fn make_sound(&mut self) -> bool {
        if self.sound != 0 {
            return true;
        }
        let Some(f) = self.fns.as_ref() else { return false };
        if self.wav.is_empty() {
            self.wav = synth_wav();
        }
        for size in (64u32..=512).step_by(4) {
            let mut info = [0u32; 256];
            info[0] = size;
            info[1] = self.wav.len() as u32;
            let mut sound: Handle = std::ptr::null_mut();
            // SAFETY: memory file stays alive in `self.wav`; `info` is a
            // zeroed 1 KiB block larger than any FMOD version's struct.
            let r = unsafe {
                (f.create)(h(self.core), self.wav.as_ptr() as *const c_char, SOUND_MODE, info.as_mut_ptr() as *mut c_void, &mut sound)
            };
            if r == 0 && !sound.is_null() {
                self.sound = sound as usize;
                crate::log_line(&format!("fpv_drone: rotor sound created (exinfo size {size})"));
                return true;
            }
        }
        crate::log_line("fpv_drone: could not create the rotor sound (no exinfo size accepted), sound off");
        self.failed = true;
        false
    }

    /// Drive the four rotors. `false` = the Studio handle is no longer
    /// good (caller should look it up again).
    pub(crate) fn update(&mut self, studio: usize, motors: &[f32; 4], master: f32) -> bool {
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
        if !self.make_sound() {
            return true;
        }
        let Some(f) = self.fns.as_ref() else { return true };
        let master = if master.is_finite() { master.clamp(0.0, 1.5) } else { 0.0 };
        for i in 0..4 {
            let (pitch, vol) = rotor_voice(motors[i], i);
            // SAFETY: handles validated by FMOD on every call.
            unsafe {
                if self.chans[i] == 0 {
                    let mut ch: Handle = std::ptr::null_mut();
                    if (f.play)(h(self.core), h(self.sound), std::ptr::null_mut(), 1, &mut ch) != 0 || ch.is_null() {
                        continue;
                    }
                    (f.pan)(ch, PAN[i]);
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

    /// Silence: stop the channels and free the loop sound.
    pub(crate) fn stop(&mut self) {
        if let Some(f) = self.fns.as_ref() {
            for c in self.chans.iter_mut().filter(|c| **c != 0) {
                // SAFETY: FMOD validates the channel handle.
                unsafe { (f.stop)(h(*c)) };
                *c = 0;
            }
            if self.sound != 0 {
                // SAFETY: sound created by us, released once.
                unsafe { (f.release)(h(self.sound)) };
            }
        }
        self.sound = 0;
        self.chans = [0; 4];
        self.core = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_is_well_formed() {
        let w = synth_wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(w.len(), 44 + LOOP_LEN * 2);
        let riff = u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, w.len());
    }

    #[test]
    fn loop_seam_is_click_free() {
        let w = synth_wav();
        let s = |i: usize| i16::from_le_bytes([w[44 + 2 * i], w[45 + 2 * i]]) as i32;
        let max_step = (1..LOOP_LEN).map(|i| (s(i) - s(i - 1)).abs()).max().unwrap();
        let seam = (s(0) - s(LOOP_LEN - 1)).abs();
        assert!(seam <= max_step, "seam {seam} vs max step {max_step}");
    }

    #[test]
    fn voice_follows_rotor_speed() {
        let (p_lo, v_lo) = rotor_voice(0.2, 0);
        let (p_hi, v_hi) = rotor_voice(0.9, 0);
        assert!(p_hi > p_lo && v_hi > v_lo);
        assert!(rotor_voice(f32::NAN, 1).0.is_finite());
    }
}
