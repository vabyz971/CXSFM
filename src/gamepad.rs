//! Gamepad input straight from the Linux kernel (evdev), independent of
//! the game's SDL.
//!
//! Why not the game's SDL2: driving it from our threads
//! (`SDL_JoystickUpdate`) corrupted the game's heap (double free, abort
//! a few seconds into FPV Drone use), and without that call the state
//! never moves (the game does not pump joysticks itself). evdev gives
//! every reader its own event stream, so we share nothing with SDL.
//!
//! The first `/dev/input/event*` node with gamepad buttons and axes is
//! opened read-only, non-blocking (readable through the `uaccess` ACL,
//! like SDL does). Buttons and axes are numbered in SDL's Linux order
//! (key codes from `BTN_JOYSTICK` upward, then `BTN_MISC`; absolute
//! axes ascending, hats excluded) and axes are scaled to SDL's
//! -32768..32767, so pad layouts and saved bindings are unchanged
//! (xpad: A=0 B=1 X=2 Y=3 LB=4 RB=5, L3=9 R3=10; LX=0 LY=1 LT=2 RX=3
//! RY=4 RT=5). A lost device is closed and searched again every 2 s.

use std::sync::Mutex;
use std::time::{Duration, Instant};

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_DROPPED: u16 = 3;
const BTN_MISC: u16 = 0x100;
const BTN_JOYSTICK: u16 = 0x120;
const BTN_SOUTH: u16 = 0x130;
const KEY_CNT: usize = 0x300;
const ABS_CNT: usize = 0x40;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;
const ABS_HAT3Y: u16 = 0x17;
const MAX_NODES: usize = 64;
const RESCAN: Duration = Duration::from_secs(2);

/// `struct input_event` (64-bit: `timeval` = 2 × i64).
#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    sec: i64,
    usec: i64,
    kind: u16,
    code: u16,
    value: i32,
}

/// `struct input_absinfo`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// `_IOC(_IOC_READ, 'E', nr, size)`.
const fn eioc(nr: u32, size: u32) -> u64 {
    ((2u64) << 30) | ((size as u64) << 16) | ((b'E' as u64) << 8) | nr as u64
}
fn eviocgbit(ev: u32, len: u32) -> u64 {
    eioc(0x20 + ev, len)
}
fn eviocgabs(abs: u32) -> u64 {
    eioc(0x40 + abs, std::mem::size_of::<AbsInfo>() as u32)
}
const EVIOCGKEY_LEN: u32 = (KEY_CNT / 8) as u32;
const EVIOCGNAME_LEN: u32 = 256;

fn bit(buf: &[u8], n: usize) -> bool {
    buf.get(n / 8).is_some_and(|b| b & (1 << (n % 8)) != 0)
}

/// One opened pad.
struct Dev {
    fd: libc::c_int,
    /// Button index → key code (SDL order).
    keys: Vec<u16>,
    /// Axis index → (abs code, min, max) (SDL order).
    axes: Vec<(u16, i32, i32)>,
    buttons: [bool; 16],
    raw: [i16; 8],
    hat_x: i32,
    hat_y: i32,
}

impl Drop for Dev {
    fn drop(&mut self) {
        // SAFETY: fd opened by us, closed once.
        unsafe { libc::close(self.fd) };
    }
}

struct Slot {
    dev: Option<Dev>,
    next_scan: Option<Instant>,
}

static SLOT: Mutex<Slot> = Mutex::new(Slot { dev: None, next_scan: None });

/// SDL's mapping of an absolute value to -32768..=32767.
fn scale(v: i32, min: i32, max: i32) -> i16 {
    if max <= min {
        return 0;
    }
    let t = (v.clamp(min, max) as i64 - min as i64) * 65535 / (max as i64 - min as i64) - 32768;
    t.clamp(-32768, 32767) as i16
}

impl Dev {
    /// Open `path` if it is a gamepad (buttons + sticks).
    fn open(path: &str) -> Option<(Dev, String)> {
        let c = std::ffi::CString::new(path).ok()?;
        // SAFETY: valid NUL-terminated path; fd checked.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let mut key_bits = [0u8; KEY_CNT / 8];
        let mut abs_bits = [0u8; ABS_CNT / 8];
        // SAFETY: buffers sized exactly as the request lengths.
        let ok = unsafe {
            libc::ioctl(fd, eviocgbit(EV_KEY as u32, key_bits.len() as u32) as _, key_bits.as_mut_ptr()) >= 0
                && libc::ioctl(fd, eviocgbit(EV_ABS as u32, abs_bits.len() as u32) as _, abs_bits.as_mut_ptr()) >= 0
        };
        // A gamepad has the south face button and at least two axes.
        if !ok || !bit(&key_bits, BTN_SOUTH as usize) || !bit(&abs_bits, 0) || !bit(&abs_bits, 1) {
            // SAFETY: fd opened above.
            unsafe { libc::close(fd) };
            return None;
        }
        let keys: Vec<u16> = (BTN_JOYSTICK as usize..KEY_CNT)
            .chain(BTN_MISC as usize..BTN_JOYSTICK as usize)
            .filter(|&k| bit(&key_bits, k))
            .map(|k| k as u16)
            .collect();
        let mut axes = Vec::new();
        for a in 0..ABS_CNT as u16 {
            if (ABS_HAT0X..=ABS_HAT3Y).contains(&a) || !bit(&abs_bits, a as usize) {
                continue;
            }
            let mut info = AbsInfo::default();
            // SAFETY: `info` matches `struct input_absinfo`.
            if unsafe { libc::ioctl(fd, eviocgabs(a as u32) as _, &mut info as *mut AbsInfo) } >= 0 {
                axes.push((a, info.minimum, info.maximum));
            }
        }
        let mut name = [0u8; EVIOCGNAME_LEN as usize];
        // SAFETY: buffer sized as the request length.
        unsafe { libc::ioctl(fd, eioc(0x06, EVIOCGNAME_LEN) as _, name.as_mut_ptr()) };
        let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
        let name = String::from_utf8_lossy(&name[..end]).into_owned();
        let mut dev = Dev { fd, keys, axes, buttons: [false; 16], raw: [0; 8], hat_x: 0, hat_y: 0 };
        dev.resync();
        Some((dev, name))
    }

    /// Re-read the whole state (open, and after `SYN_DROPPED`).
    fn resync(&mut self) {
        let mut key_state = [0u8; KEY_CNT / 8];
        // SAFETY: buffer sized as the request length.
        if unsafe { libc::ioctl(self.fd, eioc(0x18, EVIOCGKEY_LEN) as _, key_state.as_mut_ptr()) } >= 0 {
            for (i, code) in self.keys.iter().enumerate().take(16) {
                self.buttons[i] = bit(&key_state, *code as usize);
            }
        }
        for (i, &(code, min, max)) in self.axes.iter().enumerate().take(8) {
            let mut info = AbsInfo::default();
            // SAFETY: `info` matches `struct input_absinfo`.
            if unsafe { libc::ioctl(self.fd, eviocgabs(code as u32) as _, &mut info as *mut AbsInfo) } >= 0 {
                self.raw[i] = scale(info.value, min, max);
            }
        }
        for (code, slot) in [(ABS_HAT0X, 0), (ABS_HAT0Y, 1)] {
            let mut info = AbsInfo::default();
            // SAFETY: as above.
            if unsafe { libc::ioctl(self.fd, eviocgabs(code as u32) as _, &mut info as *mut AbsInfo) } >= 0 {
                if slot == 0 {
                    self.hat_x = info.value.signum();
                } else {
                    self.hat_y = info.value.signum();
                }
            }
        }
    }

    /// Drain pending events. `false` = device gone.
    fn drain(&mut self) -> bool {
        let mut buf = [InputEvent { sec: 0, usec: 0, kind: 0, code: 0, value: 0 }; 64];
        loop {
            let size = std::mem::size_of_val(&buf);
            // SAFETY: reading into a properly sized repr(C) array.
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, size) };
            if n < 0 {
                let err = std::io::Error::last_os_error().raw_os_error();
                return matches!(err, Some(libc::EAGAIN) | Some(libc::EINTR));
            }
            if n == 0 {
                return false;
            }
            let count = n as usize / std::mem::size_of::<InputEvent>();
            for ev in &buf[..count] {
                match ev.kind {
                    EV_KEY => {
                        if let Some(i) = self.keys.iter().position(|k| *k == ev.code)
                            && i < 16
                        {
                            self.buttons[i] = ev.value != 0;
                        }
                    }
                    EV_ABS if ev.code == ABS_HAT0X => self.hat_x = ev.value.signum(),
                    EV_ABS if ev.code == ABS_HAT0Y => self.hat_y = ev.value.signum(),
                    EV_ABS => {
                        if let Some(i) = self.axes.iter().position(|a| a.0 == ev.code)
                            && i < 8
                        {
                            let (_, min, max) = self.axes[i];
                            self.raw[i] = scale(ev.value, min, max);
                        }
                    }
                    EV_SYN if ev.code == SYN_DROPPED => self.resync(),
                    _ => {}
                }
            }
            if count < buf.len() {
                return true;
            }
        }
    }

    fn state(&self) -> GamepadState {
        let mut hat = 0u8;
        if self.hat_y < 0 {
            hat |= 1;
        }
        if self.hat_x > 0 {
            hat |= 2;
        }
        if self.hat_y > 0 {
            hat |= 4;
        }
        if self.hat_x < 0 {
            hat |= 8;
        }
        if hat == 0 {
            // D-pad reported as buttons (SDL order 11–14).
            for (i, b) in [(11usize, 1u8), (12, 4), (13, 8), (14, 2)] {
                if self.buttons[i] {
                    hat |= b;
                }
            }
        }
        GamepadState {
            connected: true,
            lx: norm(self.raw[0]),
            ly: norm(self.raw[1]),
            rx: norm(self.raw[3]),
            ry: norm(self.raw[4]),
            lt: trigger(self.raw[2]),
            rt: trigger(self.raw[5]),
            axes: std::array::from_fn(|i| norm(self.raw[i])),
            hat,
            buttons: self.buttons,
        }
    }
}

/// First gamepad among `/dev/input/event*`.
fn scan() -> Option<(Dev, String)> {
    (0..MAX_NODES).find_map(|i| Dev::open(&format!("/dev/input/event{i}")))
}

/// Deadzone-applied, normalized axis (-1.0..=1.0).
fn norm(raw: i16) -> f32 {
    const DEAD: f32 = 0.12;
    let v = (raw as f32 / 32767.0).clamp(-1.0, 1.0);
    if v.abs() < DEAD {
        0.0
    } else {
        (v - DEAD.copysign(v)) / (1.0 - DEAD)
    }
}

/// Trigger axis (-32768 released … 32767 pressed) → 0.0..=1.0.
fn trigger(raw: i16) -> f32 {
    let v = ((raw as f32 + 32768.0) / 65535.0).clamp(0.0, 1.0);
    if v < 0.05 { 0.0 } else { v }
}

/// Snapshot of the pad (all zeros/false when absent).
#[derive(Clone, Copy, Default, Debug)]
pub struct GamepadState {
    pub connected: bool,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    /// Triggers 0.0 (released) … 1.0 (fully pressed). Xbox layout:
    /// axes 2 (left) and 5 (right).
    pub lt: f32,
    pub rt: f32,
    /// Every axis by SDL index 0..8, -1.0..=1.0 with the stick
    /// deadzone applied (a released trigger reads -1.0). Lets mods
    /// remap controls for pads whose layout is not Xbox's.
    pub axes: [f32; 8],
    /// D-pad as a hat bitmask (1 up, 2 right, 4 down, 8 left); falls
    /// back to buttons 11–14 (SDL game-controller order) when the pad
    /// has no hat.
    pub hat: u8,
    /// Buttons 0..16 (Xbox: 0=A, 1=B, 2=X, 3=Y, 4=LB, 5=RB).
    pub buttons: [bool; 16],
}

/// Poll the pad (pad thread and mod tick; serialized by the lock).
/// Opens lazily, re-scans every 2 s while none is found.
pub fn poll() -> GamepadState {
    let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    if slot.dev.is_none() {
        let now = Instant::now();
        if slot.next_scan.is_some_and(|t| now < t) {
            return GamepadState::default();
        }
        slot.next_scan = Some(now + RESCAN);
        if let Some((dev, name)) = scan() {
            crate::log_line(&format!(
                "gamepad: opened {name} (evdev, {} buttons, {} axes)",
                dev.keys.len(),
                dev.axes.len()
            ));
            slot.dev = Some(dev);
        }
    }
    let gone = match slot.dev.as_mut() {
        Some(d) => !d.drain(),
        None => return GamepadState::default(),
    };
    if gone {
        slot.dev = None;
        crate::log_line("gamepad: device lost, searching again");
        return GamepadState::default();
    }
    slot.dev.as_ref().map_or_else(GamepadState::default, Dev::state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_matches_sdl_range() {
        assert_eq!(scale(-32768, -32768, 32767), -32768);
        assert_eq!(scale(32767, -32768, 32767), 32767);
        assert_eq!(scale(0, 0, 255), -32768);
        assert_eq!(scale(255, 0, 255), 32767);
        assert_eq!(scale(5, 5, 5), 0);
    }

    #[test]
    fn ioctl_numbers_match_linux() {
        // EVIOCGBIT(EV_KEY, 96) and EVIOCGABS(ABS_X) from <linux/input.h>.
        assert_eq!(eviocgbit(1, 96), 0x8060_4521);
        assert_eq!(eviocgabs(0), 0x8018_4540);
        assert_eq!(std::mem::size_of::<InputEvent>(), 24);
    }

    /// Needs a real pad: `cargo test --lib live_pad -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_pad() {
        let g = poll();
        assert!(g.connected, "no gamepad found under /dev/input");
        println!("{g:?}");
    }
}
