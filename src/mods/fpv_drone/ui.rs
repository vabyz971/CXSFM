//! FPV drone window: flight sliders + control legend.

use super::FpvDroneMod;
use super::model::{BOOST_RANGE, FOV_RANGE, SPEED_RANGE, TILT_RANGE, TURN_RANGE};
use crate::mods::common::lock;

impl FpvDroneMod {
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        let note = lock(&self.note).clone();
        let mut win = egui::Window::new("FPV Drone")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(380.0, 330.0))
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
            ui.label(egui::RichText::new(if note.is_empty() { "idle" } else { &note }).weak());
            ui.add_space(4.0);
            let e = &mut self.edit;
            ui.add(egui::Slider::new(&mut e.speed, SPEED_RANGE.0..=SPEED_RANGE.1).text("speed m/s"));
            ui.add(egui::Slider::new(&mut e.boost, BOOST_RANGE.0..=BOOST_RANGE.1).text("boost ×"));
            ui.add(egui::Slider::new(&mut e.turn_deg_s, TURN_RANGE.0..=TURN_RANGE.1).text("turn °/s"));
            ui.add(egui::Slider::new(&mut e.tilt_deg, TILT_RANGE.0..=TILT_RANGE.1).text("camera tilt °"));
            ui.add(egui::Slider::new(&mut e.bank, 0.0..=1.0).text("bank"));
            ui.add(egui::Slider::new(&mut e.fov, FOV_RANGE.0..=FOV_RANGE.1).text("FOV °"));
            if ui.button("Reset").clicked() {
                *e = Default::default();
            }
            ui.separator();
            ui.label(egui::RichText::new("Left stick: move · Right stick: look").small());
            ui.label(egui::RichText::new("RT / LT: up / down · A: boost").small());
            ui.label(egui::RichText::new("The car also reads the pad: use photo mode.").small().weak());
        });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
        }
        let mut shared = lock(&self.shared);
        if *shared != self.edit {
            *shared = self.edit;
        }
    }
}
