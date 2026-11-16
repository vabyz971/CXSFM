//! CXSFM input: the F8 hotkey and input capture (SDL + X11 — the Unity
//! `Input` path was cut, it segfaulted at teardown), the evdev gamepad
//! reader and the pad navigation thread ("mod mode", L3 + R3).

pub mod gamepad;
pub mod hotkey;
pub mod padnav;
