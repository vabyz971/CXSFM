//! Gamepad navigation for the overlay ("mod mode").
//!
//! The pad thread polls the pad, turns it into menu events and parks
//! them in an atomic mask; the render thread takes them once per frame
//! ([`begin_frame`]) and every window reads the same frame snapshot
//! ([`events`]). Only the focused window acts on them ([`has_focus`]).
//!
//! Mod mode is entered/left with **L3 + R3** (press both sticks). It
//! opens the overlay, then: D-pad / left stick navigate (with key
//! repeat), A = select, B = back, X = reset the row to its default,
//! Y = alternate action (advanced toggle), LB / RB = previous / next tab.
//!
//! The game still reads the same pad (a limit of the overlay, same as
//! the drone mod): use mod mode in a menu, the phone or photo mode.

use crate::gamepad::{self, GamepadState};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const UP: u32 = 1 << 0;
pub const DOWN: u32 = 1 << 1;
pub const LEFT: u32 = 1 << 2;
pub const RIGHT: u32 = 1 << 3;
pub const ACCEPT: u32 = 1 << 4;
pub const BACK: u32 = 1 << 5;
pub const DEFAULT: u32 = 1 << 6;
pub const ALT: u32 = 1 << 7;
pub const TAB_PREV: u32 = 1 << 8;
pub const TAB_NEXT: u32 = 1 << 9;

/// Buttons (Xbox layout): A, B, X, Y, LB, RB.
const BTN_A: usize = 0;
const BTN_B: usize = 1;
const BTN_X: usize = 2;
const BTN_Y: usize = 3;
const BTN_LB: usize = 4;
const BTN_RB: usize = 5;
/// Stick clicks. Raw SDL joystick indices differ by driver: Windows /
/// SDL game-controller order puts L3/R3 at 8/9, the Linux xpad driver
/// (Xbox-mode 8BitDo, Xbox pads) at 9/10 with the Guide button at 8.
/// Either pair counts as the combo.
const STICK_PAIRS: [(usize, usize); 2] = [(9, 10), (8, 9)];
/// Left-stick travel that counts as a D-pad press.
const STICK_THRESHOLD: f32 = 0.6;
/// Key repeat: delay before the first repeat, then period (s).
const REPEAT_DELAY: f32 = 0.4;
const REPEAT_PERIOD: f32 = 0.12;

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The pad opened the overlay (so the pad closes it again).
static OPENED_BY_PAD: AtomicBool = AtomicBool::new(false);
/// Events produced by the tick, not yet seen by a frame.
static PENDING: AtomicU32 = AtomicU32::new(0);
/// Events of the frame being drawn.
static FRAME: AtomicU32 = AtomicU32::new(0);
static FOCUS: Mutex<String> = Mutex::new(String::new());

/// Is mod mode on?
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Leave mod mode (closes the overlay if the pad opened it).
pub fn deactivate() {
    if ACTIVE.swap(false, Ordering::SeqCst) {
        if OPENED_BY_PAD.swap(false, Ordering::SeqCst) && crate::hotkey::ui_visible() {
            crate::hotkey::toggle_ui();
        }
        crate::log_line("padnav: mod mode OFF");
    }
}

fn activate() {
    if !crate::hotkey::ui_visible() {
        crate::hotkey::toggle_ui();
        OPENED_BY_PAD.store(true, Ordering::SeqCst);
    }
    set_focus("menu");
    PENDING.store(0, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    crate::log_line("padnav: mod mode ON (L3+R3 to leave)");
}

/// Which window owns the pad (`"menu"` or a mod id).
pub fn focus() -> String {
    FOCUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn set_focus(name: &str) {
    *FOCUS.lock().unwrap_or_else(|e| e.into_inner()) = name.to_string();
}

/// Does `name` own the pad right now?
pub fn has_focus(name: &str) -> bool {
    active() && *FOCUS.lock().unwrap_or_else(|e| e.into_inner()) == name
}

/// Render thread, once per frame, before any window: freeze this
/// frame's events.
pub fn begin_frame() {
    let ev = if active() { PENDING.swap(0, Ordering::SeqCst) } else { 0 };
    FRAME.store(ev, Ordering::SeqCst);
}

/// Events waiting for the next frame (lets the overlay repaint: pad
/// input is not an egui event, so a static UI would never wake up).
pub fn has_pending() -> bool {
    active() && PENDING.load(Ordering::SeqCst) != 0
}

/// This frame's events (valid after [`begin_frame`]).
pub fn events() -> u32 {
    FRAME.load(Ordering::SeqCst)
}

/// Directions held → press events with key repeat. Pure, unit-tested.
#[derive(Default)]
pub struct Repeater {
    held: u32,
    timer: f32,
    repeating: bool,
}

impl Repeater {
    /// `mask` = directions currently held; returns the directions that
    /// fire this step (new presses, then repeats while held).
    pub fn step(&mut self, mask: u32, dt: f32) -> u32 {
        let fresh = mask & !self.held;
        if mask == 0 {
            *self = Self::default();
            return 0;
        }
        if fresh != 0 {
            self.held = mask;
            self.timer = 0.0;
            self.repeating = false;
            return fresh;
        }
        self.held = mask;
        self.timer += dt.max(0.0);
        let limit = if self.repeating { REPEAT_PERIOD } else { REPEAT_DELAY };
        if self.timer >= limit {
            self.timer = 0.0;
            self.repeating = true;
            return mask;
        }
        0
    }
}

/// D-pad (hat or buttons) + left stick → direction mask.
fn directions(g: &GamepadState) -> u32 {
    let mut m = 0;
    if g.hat & 1 != 0 || g.axes[1] < -STICK_THRESHOLD {
        m |= UP;
    }
    if g.hat & 4 != 0 || g.axes[1] > STICK_THRESHOLD {
        m |= DOWN;
    }
    if g.hat & 8 != 0 || g.axes[0] < -STICK_THRESHOLD {
        m |= LEFT;
    }
    if g.hat & 2 != 0 || g.axes[0] > STICK_THRESHOLD {
        m |= RIGHT;
    }
    m
}

/// Tick-side state.
#[derive(Default)]
struct Tick {
    prev_buttons: [bool; 16],
    combo_held: bool,
    /// Raw-button diagnostics already logged (capped).
    logged: u8,
    rep: Repeater,
}

static TICK: Mutex<Option<Tick>> = Mutex::new(None);

/// Start the pad thread (once). The pad used to be polled from the mod
/// tick, which also runs IL2CPP work: a slow mod update delayed every
/// menu move. Own thread, ~120 Hz, IL2CPP-free.
pub fn spawn_thread() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("cxsfm-pad".into()).spawn(|| {
        let mut last = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(8));
            let dt = last.elapsed().as_secs_f32().min(0.1);
            last = std::time::Instant::now();
            tick(dt);
        }
    });
}

/// Poll the pad (pad thread, every iteration). Cheap and IL2CPP-free.
fn tick(dt: f32) {
    // The overlay was closed some other way (F8): leave mod mode.
    if active() && !crate::hotkey::ui_visible() {
        ACTIVE.store(false, Ordering::SeqCst);
        OPENED_BY_PAD.store(false, Ordering::SeqCst);
    }
    let g = gamepad::poll();
    if !g.connected {
        return;
    }
    let mut guard = TICK.lock().unwrap_or_else(|e| e.into_inner());
    let st = guard.get_or_insert_with(Tick::default);
    let combo = STICK_PAIRS.iter().any(|&(a, b)| g.buttons[a] && g.buttons[b]);
    let combo_edge = combo && !st.combo_held;
    st.combo_held = combo;
    // Diagnose odd pad layouts: log the raw indices when two or more
    // buttons go down together (capped, so it never floods the log).
    let held: Vec<usize> = (0..16).filter(|&i| g.buttons[i]).collect();
    if held.len() >= 2 && st.prev_buttons.iter().filter(|b| **b).count() < 2 && st.logged < 12 {
        st.logged += 1;
        crate::log_line(&format!("padnav: buttons held together (raw SDL indices): {held:?}"));
    }
    let pressed = |i: usize| g.buttons[i] && !st.prev_buttons[i];
    let mut ev = 0;
    if active() && !combo {
        ev |= st.rep.step(directions(&g), dt);
        for (btn, bit) in [
            (BTN_A, ACCEPT),
            (BTN_B, BACK),
            (BTN_X, DEFAULT),
            (BTN_Y, ALT),
            (BTN_LB, TAB_PREV),
            (BTN_RB, TAB_NEXT),
        ] {
            if pressed(btn) {
                ev |= bit;
            }
        }
    }
    st.prev_buttons = g.buttons;
    drop(guard);
    if combo_edge {
        if active() {
            deactivate();
        } else {
            activate();
        }
    } else if ev != 0 {
        PENDING.fetch_or(ev, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_fires_once_then_repeats_after_the_delay() {
        let mut r = Repeater::default();
        assert_eq!(r.step(DOWN, 0.016), DOWN);
        assert_eq!(r.step(DOWN, 0.016), 0);
        let mut fired = 0;
        for _ in 0..40 {
            if r.step(DOWN, 0.016) != 0 {
                fired += 1;
            }
        }
        // 0.64 s held after the press: delay 0.4 s, then ~every 0.12 s.
        assert!((2..=3).contains(&fired), "{fired}");
    }

    #[test]
    fn release_resets_the_repeat() {
        let mut r = Repeater::default();
        r.step(UP, 0.016);
        r.step(0, 0.016);
        assert_eq!(r.step(UP, 0.016), UP);
    }

    #[test]
    fn a_second_direction_fires_immediately() {
        let mut r = Repeater::default();
        r.step(DOWN, 0.016);
        assert_eq!(r.step(DOWN | RIGHT, 0.016), RIGHT);
    }

    #[test]
    fn stick_and_hat_both_navigate() {
        let mut g = GamepadState::default();
        g.axes[1] = -1.0;
        assert_eq!(directions(&g), UP);
        g = GamepadState::default();
        g.hat = 2 | 4;
        assert_eq!(directions(&g), RIGHT | DOWN);
    }
}
