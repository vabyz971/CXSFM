//! FPV drone window: flight, physics, camera, controls, telemetry.

use super::model::{
    Action, DRAG_RANGE, FOV_RANGE, GRAVITY_RANGE, GUST_RANGE, MAX_ANGLE_RANGE, Mode, Params, RADIUS_RANGE, RATE_RANGE,
    TILT_RANGE, TWR_RANGE, WIND_RANGE, YAW_RATE_RANGE,
};
use super::{FpvDroneMod, Shared};
use crate::mods::common::lock;

const AXIS_NAMES: [&str; 8] = ["0 · LX", "1 · LY", "2 · LT", "3 · RX", "4 · RY", "5 · RT", "6", "7"];
const BUTTON_NAMES: [&str; 16] =
    ["A", "B", "X", "Y", "LB", "RB", "Back", "Start", "L3", "R3", "10", "11", "12", "13", "14", "15"];

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

fn slider(ui: &mut egui::Ui, v: &mut f32, r: (f32, f32), text: &str) {
    ui.add(egui::Slider::new(v, r.0..=r.1).text(text));
}

fn flight_and_physics(ui: &mut egui::Ui, p: &mut Params) {
    egui::ComboBox::from_label("mode")
        .selected_text(if p.mode == Mode::Angle { "Angle (self-level)" } else { "Acro (rates)" })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut p.mode, Mode::Angle, "Angle (self-level)");
            ui.selectable_value(&mut p.mode, Mode::Acro, "Acro (rates)");
        });
    if p.mode == Mode::Angle {
        slider(ui, &mut p.max_angle, MAX_ANGLE_RANGE, "max tilt °");
    } else {
        slider(ui, &mut p.rate, RATE_RANGE, "roll/pitch rate °/s");
        slider(ui, &mut p.expo, (0.0, 1.0), "expo");
    }
    slider(ui, &mut p.yaw_rate, YAW_RATE_RANGE, "yaw rate °/s");
    slider(ui, &mut p.twr, TWR_RANGE, "thrust / weight");
    ui.checkbox(&mut p.throttle_mid, "hover at stick centre (spring pad)");
    ui.separator();
    slider(ui, &mut p.gravity, GRAVITY_RANGE, "gravity m/s²");
    slider(ui, &mut p.drag, DRAG_RANGE, "air drag ×");
    slider(ui, &mut p.wind_speed, WIND_RANGE, "wind m/s");
    slider(ui, &mut p.wind_dir, (0.0, 360.0), "wind blows toward °");
    slider(ui, &mut p.gust, GUST_RANGE, "gusts m/s");
    ui.separator();
    ui.checkbox(&mut p.collisions, "collisions");
    slider(ui, &mut p.radius, RADIUS_RANGE, "radius m");
    slider(ui, &mut p.bounce, (0.0, 0.9), "bounce");
    ui.separator();
    slider(ui, &mut p.tilt_deg, TILT_RANGE, "camera tilt °");
    slider(ui, &mut p.fov, FOV_RANGE, "FOV °");
    if ui.button("Reset settings").clicked() {
        *p = Params::default();
    }
}

fn controls(ui: &mut egui::Ui, s: &mut Shared, sticks: [f32; 4]) {
    ui.label(egui::RichText::new("Bind, then move the control in its positive direction (throttle up, yaw/roll right, pitch forward) or press the button.").small().weak());
    for a in Action::ALL {
        ui.horizontal(|ui| {
            ui.label(action_label(a));
            let learning = s.learn == Some(a);
            let text = if learning {
                "move / press…".to_string()
            } else if let Some(b) = s.bindings.axis(a) {
                format!("axis {}", AXIS_NAMES[(b.idx as usize).min(7)])
            } else {
                let i = s.bindings.button(a).unwrap_or(0) as usize;
                format!("button {}", BUTTON_NAMES[i.min(15)])
            };
            if ui.button(text).clicked() {
                s.learn = if learning { None } else { Some(a) };
            }
            if let Some(b) = s.bindings.axis_mut(a) {
                ui.checkbox(&mut b.invert, "invert");
                let v = match a {
                    Action::Throttle => sticks[0],
                    Action::Yaw => (sticks[1] + 1.0) * 0.5,
                    Action::Pitch => (sticks[2] + 1.0) * 0.5,
                    _ => (sticks[3] + 1.0) * 0.5,
                };
                ui.add(egui::ProgressBar::new(v.clamp(0.0, 1.0)).desired_width(70.0));
            }
        });
    }
    if ui.button("Default controls").clicked() {
        s.bindings = Default::default();
        s.learn = None;
    }
}

impl FpvDroneMod {
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        let tel = lock(&self.tel).clone();
        let orig = *lock(&self.shared);
        let mut s = orig;
        let mut win = egui::Window::new("FPV Drone")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(430.0, 600.0))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
            self.tool.enter(ui);
            self.tool.header(ui, "FPV Drone");
            if self.tool.hide_controls {
                return;
            }
            let note = if tel.note.is_empty() { "idle" } else { &tel.note };
            let color = if tel.armed { crate::ui::theme::STATE_OK } else { egui::Color32::WHITE };
            ui.label(egui::RichText::new(note).strong().color(color));
            if let Some(a) = s.learn {
                ui.label(egui::RichText::new(format!("Bind: {}", a.hint())).color(crate::ui::theme::STATE_ERR));
            }
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
                .size(13.0),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(480.0).auto_shrink([false, false]).show(ui, |ui| {
                egui::CollapsingHeader::new("Flight & physics").default_open(true).show(ui, |ui| {
                    flight_and_physics(ui, &mut s.params);
                });
                egui::CollapsingHeader::new("Controls").default_open(true).show(ui, |ui| {
                    controls(ui, &mut s, tel.sticks);
                });
            });
            ui.label(egui::RichText::new("The car also reads the pad: use photo mode.").small().weak());
        });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
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
    }
}
