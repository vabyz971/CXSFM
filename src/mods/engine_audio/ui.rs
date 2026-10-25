//! Engine-audio render-side window: sliders, filter, live events and
//! the parameter list. Edits stage in `self.edit` and reach the tick
//! through one brief store (no present-thread work beyond that).

use super::EngineAudioMod;
use super::model::{DEFAULT_FILTER, MAX_OVERRIDES, PITCH_RANGE, ParamOverride, VOLUME_RANGE};
use crate::mods::common::lock;

impl EngineAudioMod {
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        let status = lock(&self.status).clone();
        let mut win = egui::Window::new("Engine Sound")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::Vec2::new(430.0, 520.0))
            .frame(self.tool.frame(ctx));
        if let Some(p) = self.tool.pos {
            win = win.current_pos(p);
        }
        let win = win.show(ctx, |ui| {
            self.tool.enter(ui);
            self.tool.header(ui, "Engine Sound");
            if self.tool.hide_controls {
                return;
            }
            ui.label(egui::RichText::new(&status.note).weak());
            ui.add_space(4.0);
            ui.add(egui::Slider::new(&mut self.edit.pitch, PITCH_RANGE.0..=PITCH_RANGE.1).text("pitch ×"));
            ui.add(egui::Slider::new(&mut self.edit.volume, VOLUME_RANGE.0..=VOLUME_RANGE.1).text("volume ×"));
            ui.horizontal(|ui| {
                ui.label("filter");
                ui.add(
                    egui::TextEdit::singleline(&mut self.edit.filter)
                        .hint_text(DEFAULT_FILTER)
                        .desired_width(f32::INFINITY),
                );
            });
            if ui.button("Reset").clicked() {
                self.edit.pitch = 1.0;
                self.edit.volume = 1.0;
            }
            ui.separator();
            ui.label(format!(
                "{} banks · {} events · {} live instances",
                status.banks, status.events, status.instances
            ));
            egui::ScrollArea::vertical().max_height(110.0).id_salt("ea_events").show(ui, |ui| {
                for (path, n) in &status.matched {
                    let line = egui::RichText::new(format!("{n:>2}  {path}")).monospace().size(13.0);
                    ui.label(if *n > 0 { line } else { line.weak() });
                }
            });
            ui.separator();
            ui.label(egui::RichText::new("Parameters (experimental: the game rewrites them every frame)").small());
            egui::ScrollArea::vertical().max_height(150.0).id_salt("ea_params").show(ui, |ui| {
                for p in &status.params {
                    let idx = self.edit.overrides.iter().position(|o| o.name == p.name);
                    ui.horizontal(|ui| {
                        let mut on = idx.is_some_and(|i| self.edit.overrides[i].on);
                        if ui.checkbox(&mut on, "").changed() {
                            match idx {
                                Some(i) => self.edit.overrides[i].on = on,
                                None if self.edit.overrides.len() < MAX_OVERRIDES => {
                                    self.edit.overrides.push(ParamOverride { name: p.name.clone(), value: p.value, on })
                                }
                                None => {}
                            }
                        }
                        ui.label(egui::RichText::new(&p.name).monospace().size(13.0));
                        ui.weak(format!("{:.2} → {:.2}", p.value, p.final_value));
                    });
                    if let Some(i) = idx {
                        let (lo, hi) = if p.max > p.min { (p.min, p.max) } else { (p.min, p.min + 1.0) };
                        ui.add(egui::Slider::new(&mut self.edit.overrides[i].value, lo..=hi));
                    }
                }
            });
        });
        if let Some(r) = win {
            self.tool.pos = Some(r.response.rect.min);
            self.tool.context_menu(&r.response);
        }
        self.push_if_changed();
    }

    fn push_if_changed(&mut self) {
        let mut shared = lock(&self.shared);
        if *shared != self.edit {
            *shared = self.edit.clone();
        }
    }
}
