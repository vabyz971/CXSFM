//! FPV drone screen in the game's settings style (full screen): tabs,
//! dark rows, description panel, key hints, pad navigation. When the
//! menu is closed and the mod is pinned, a compact telemetry window
//! stays on screen instead.
//!
//! The simple view only lists the parameters designated in
//! `fpv_drone.toml`; "Advanced settings" shows everything behind a
//! warning (see `ui::paramfile`).

use super::model::{Action, Bindings};
use super::spec::{self, CONTROLS_TAB, Kind, SPECS, TABS};
use super::{FpvDroneMod, Shared, Telemetry};
use crate::mods::common::lock;
use crate::padnav;
use crate::ui::game::{self, Nav};
use crate::ui::paramfile::{Policy, effective_range};

/// Compact in-game telemetry window width.
const WIN_W: f32 = 430.0;
const NAME: &str = "FPV Drone";

/// Render-side state of the window.
pub(crate) struct UiState {
    tab: usize,
    advanced: bool,
    /// Created on the first draw (no file IO at boot).
    policy: Option<Policy>,
}

impl UiState {
    pub(crate) fn new() -> Self {
        Self {
            tab: 0,
            advanced: false,
            policy: None,
        }
    }
}

fn action_label(a: Action) -> &'static str {
    match a {
        Action::Throttle => "Throttle",
        Action::Yaw => "Yaw",
        Action::Pitch => "Pitch",
        Action::Roll => "Roll",
        Action::Arm => "Arm / disarm",
        Action::Reset => "Reset to start",
        Action::Mode => "Angle / Acro",
    }
}

const AXIS_NAMES: [&str; 8] = ["AXIS 0 · LX", "AXIS 1 · LY", "AXIS 2 · LT", "AXIS 3 · RX", "AXIS 4 · RY", "AXIS 5 · RT", "AXIS 6", "AXIS 7"];
const BUTTON_NAMES: [&str; 16] =
    ["A", "B", "X", "Y", "LB", "RB", "BACK", "START", "L3", "R3", "10", "11", "12", "13", "14", "15"];

fn binding_text(b: &Bindings, a: Action) -> String {
    match (b.axis(a), b.button(a)) {
        (Some(x), _) => AXIS_NAMES[(x.idx as usize).min(7)].to_string(),
        (_, Some(i)) => format!("BUTTON {}", BUTTON_NAMES[(i as usize).min(15)]),
        _ => String::new(),
    }
}

/// Parameter rows of one tab.
fn param_rows(ui: &mut egui::Ui, nav: &mut Nav, st: &UiState, s: &mut Shared, tab: usize) {
    let Some(policy) = st.policy.as_ref() else { return };
    let dflt = Shared { params: Default::default(), bindings: Default::default(), learn: None, cut_controller: false };
    let mut last_section = "";
    for spec in SPECS.iter().filter(|p| p.tab == tab) {
        if !(st.advanced || policy.shown(spec.key)) {
            continue;
        }
        if spec.section != last_section {
            game::section(ui, spec.section);
            last_section = spec.section;
        }
        let id = format!("fd_{}", spec.key);
        let cur = spec::get(s, spec.key);
        let (mut out, mut new) = match spec.kind {
            Kind::Slider { min, max, step, dec, unit } => {
                let lim = policy.limits(spec.key);
                // Simple view honours the file's limits; advanced uses
                // the full built-in range.
                let (lo, hi) = if st.advanced { (min, max) } else { effective_range((min, max), lim) };
                let step = if st.advanced { step } else { lim.step.unwrap_or(step) };
                let mut v = cur.clamp(lo, hi);
                let out = game::slider_row(ui, nav, &id, spec.label, &mut v, (lo, hi), step, dec, unit, spec.help);
                (out, v)
            }
            Kind::Toggle => {
                let mut on = cur >= 0.5;
                let out = game::toggle_row(ui, nav, &id, spec.label, &mut on, spec.help);
                (out, on as u8 as f32)
            }
            Kind::Choice(opts) => {
                let mut i = cur as usize;
                let out = game::choice_row(ui, nav, &id, spec.label, &mut i, opts, spec.help);
                (out, i as f32)
            }
        };
        if out.reset {
            new = spec::get(&dflt, spec.key);
            out.changed = true;
        }
        if out.changed {
            spec::set(s, spec.key, new);
        }
    }
}

/// Control rows: axis/button bindings with learn mode.
fn control_rows(ui: &mut egui::Ui, nav: &mut Nav, s: &mut Shared, sticks: [f32; 4]) {
    game::section(ui, "Bindings");
    game::dim(
        ui,
        &format!(
            "live: throttle {:.2} · yaw {:+.2} · pitch {:+.2} · roll {:+.2}",
            sticks[0], sticks[1], sticks[2], sticks[3]
        ),
    );
    for a in Action::ALL {
        let learning = s.learn == Some(a);
        let text = if learning { "MOVE / PRESS…".to_string() } else { binding_text(&s.bindings, a) };
        let id = format!("fd_bind_{a:?}");
        let help = if a.is_axis() {
            "A: bind. Then move the stick/trigger in its positive direction (throttle up, yaw right, pitch forward, roll right)."
        } else {
            "A: bind. Then press the button you want."
        };
        let out = game::button_row(ui, nav, &id, action_label(a), &text, if learning { "CANCEL" } else { "A  BIND" }, help);
        if out.accept {
            s.learn = if learning { None } else { Some(a) };
        }
        if let Some(ax) = s.bindings.axis_mut(a) {
            let mut inv = ax.invert;
            let id = format!("fd_inv_{a:?}");
            let o = game::toggle_row(ui, nav, &id, "   Invert", &mut inv, "Flip the direction of this axis.");
            if o.changed {
                ax.invert = inv;
            }
        }
    }
    let out = game::button_row(ui, nav, "fd_bind_default", "Default controls", "", "A  RESET", "Back to the Xbox layout: left stick throttle + roll, right stick pitch + yaw.");
    if out.accept {
        s.bindings = Bindings::default();
        s.learn = None;
    }
}

impl FpvDroneMod {
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        let tel = lock(&self.tel).clone();
        let orig = *lock(&self.shared);
        let mut s = orig;
        self.ui
            .policy
            .get_or_insert_with(|| Policy::new(spec::FILE, "FPV Drone", &spec::default_shown(), &spec::catalog()))
            .refresh();
        // Live telemetry without input events.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        if !crate::hotkey::ui_visible() {
            self.draw_compact(ctx, &tel);
        } else if crate::ui::screen::is_open(NAME) {
            self.draw_screen(ctx, &tel, &mut s);
        }
        // Field-wise write-back: only what the UI changed this frame,
        // so the tick's own writes (learned binding, mode button) survive.
        let mut shared = lock(&self.shared);
        if s.params != orig.params {
            shared.params = s.params;
        }
        if s.bindings != orig.bindings {
            shared.bindings = s.bindings;
        }
        if s.learn != orig.learn {
            shared.learn = s.learn;
        }
        if s.cut_controller != orig.cut_controller {
            shared.cut_controller = s.cut_controller;
        }
    }

    /// Pinned telemetry over the game (menu closed).
    fn draw_compact(&mut self, ctx: &egui::Context, tel: &Telemetry) {
        let mut win = egui::Window::new(NAME)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(WIN_W, 0.0))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
            self.tool.enter(ui);
            self.tool.header(ui, "FPV DRONE");
            if self.tool.hide_controls {
                return;
            }
            telemetry(ui, tel);
        });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
        }
    }

    /// Full-screen settings (menu open, tile opened).
    fn draw_screen(&mut self, ctx: &egui::Context, tel: &Telemetry, s: &mut Shared) {
        let owner = crate::ui::screen::pad_id(NAME);
        let hints: &[(&str, &str)] = if padnav::has_focus(&owner) {
            &[("A", "select"), ("X", "default"), ("Y", "advanced"), ("LB/RB", "tab")]
        } else {
            &[("L3+R3", "pad mode")]
        };
        let exit = crate::ui::screen::show(ctx, NAME, "FPV Drone", true, hints, |ui, rg| {
            let mut nav = Nav::begin(ui, &mut self.nav, &owner);
            if nav.window_events() & padnav::ALT != 0 {
                self.ui.advanced = !self.ui.advanced;
            }
            let mut tab = self.ui.tab;
            crate::ui::screen::region(ui, rg.tabs, |ui| {
                game::tabs(ui, &nav, &TABS, &mut tab);
            });
            if tab != self.ui.tab {
                self.ui.tab = tab;
                nav.reset_focus();
            }
            // Rows (left).
            crate::ui::screen::region(ui, rg.left, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if tab == CONTROLS_TAB {
                        control_rows(ui, &mut nav, s, tel.sticks);
                    } else {
                        param_rows(ui, &mut nav, &self.ui, s, tab);
                    }
                    ui.add_space(8.0);
                    let mut adv = self.ui.advanced;
                    let o = game::toggle_row(
                        ui,
                        &mut nav,
                        "fd_advanced",
                        "Advanced settings",
                        &mut adv,
                        "Shows every parameter, not only the ones listed in fpv_drone.toml. Wrong values can freeze or crash the game.",
                    );
                    if o.changed {
                        self.ui.advanced = adv;
                    }
                    let mut pin = self.tool.pin;
                    let o = game::toggle_row(
                        ui,
                        &mut nav,
                        "fd_pin",
                        "Telemetry over the game",
                        &mut pin,
                        "Keeps a small telemetry window on screen when the menu is closed.",
                    );
                    if o.changed {
                        self.tool.pin = pin;
                    }
                });
            });
            // Help + telemetry (right).
            crate::ui::screen::region(ui, rg.right.shrink(18.0), |ui| {
                if let Some(a) = s.learn {
                    game::warning(ui, &format!("BIND {}: move or press the control (A on the row cancels)", a.hint().to_uppercase()));
                    ui.add_space(6.0);
                }
                if self.ui.advanced {
                    game::warning(ui, "ADVANCED — wrong values here can freeze or crash the game");
                    ui.add_space(6.0);
                }
                game::help_text(ui, nav.help.as_deref().unwrap_or("Select a setting to read what it does."));
                ui.add_space(14.0);
                game::dim(ui, "Edit fpv_drone.toml to choose which settings the simple view shows.");
                ui.add_space(14.0);
                telemetry(ui, tel);
            });
            nav.finish();
        });
        if exit.power_off {
            crate::mod_api::get_mod_manager().set_mod_enabled(NAME, false);
        }
    }
}

/// Status line, numbers and motor bars.
fn telemetry(ui: &mut egui::Ui, tel: &Telemetry) {
    let note = if tel.note.is_empty() { "idle" } else { &tel.note };
    let color = if tel.armed { crate::ui::theme::STATE_OK } else { egui::Color32::WHITE };
    ui.label(egui::RichText::new(note).strong().size(20.0).color(color));
    ui.label(
        egui::RichText::new(format!(
            "{:.1} m/s · alt {:+.1} m · thrust {:.0}% · collisions: {}{}",
            tel.speed,
            tel.alt,
            tel.throttle * 100.0,
            if tel.collide.is_empty() { "—" } else { tel.collide },
            if tel.impact > 0.5 { format!(" · impact {:.1} m/s", tel.impact) } else { String::new() }
        ))
        .monospace()
        .size(13.5),
    );
    ui.horizontal(|ui| {
        for (name, v) in ["FL", "FR", "RL", "RR"].iter().zip(tel.motors) {
            ui.add(egui::ProgressBar::new(v.clamp(0.0, 1.0)).desired_width(70.0).text(*name));
        }
    });
}
