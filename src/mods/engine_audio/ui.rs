//! Engine-audio full-screen settings: game-style rows (pitch, volume,
//! filter) on the left, live events and parameter list on the right.
//! Edits stage in `self.edit` and reach the tick through one brief
//! store (no present-thread work beyond that).

use super::EngineAudioMod;
use super::model::{DEFAULT_FILTER, MAX_OVERRIDES, PITCH_RANGE, ParamOverride, VOLUME_RANGE};
use crate::mods::common::lock;
use crate::padnav;
use crate::ui::game::{self, Nav};
use crate::ui::screen;

const NAME: &str = "Engine Sound";

impl EngineAudioMod {
    pub(crate) fn draw_window(&mut self, ctx: &egui::Context) {
        // Only the full-screen frame (opened from the phone tile).
        if !crate::hotkey::ui_visible() || !screen::is_open(NAME) {
            return;
        }
        let status = lock(&self.status).clone();
        let owner = screen::pad_id(NAME);
        let hints: &[(&str, &str)] = if padnav::has_focus(&owner) {
            &[("A", "select"), ("X", "default")]
        } else {
            &[("L3+R3", "pad mode")]
        };
        let exit = screen::show(ctx, NAME, NAME, true, hints, |ui, rg| {
            let mut nav = Nav::begin(ui, &mut self.nav, &owner);
            screen::region(ui, rg.left, |ui| {
                game::section(ui, "Engine");
                let o = game::slider_row(ui, &mut nav, "ea_pitch", "Pitch", &mut self.edit.pitch, PITCH_RANGE, 0.05, 2, "x", "Scales the pitch of the matching engine instances (1.00 = untouched).");
                if o.reset {
                    self.edit.pitch = 1.0;
                }
                let o = game::slider_row(ui, &mut nav, "ea_volume", "Volume", &mut self.edit.volume, VOLUME_RANGE, 0.05, 2, "x", "Scales the volume of the matching engine instances (1.00 = untouched).");
                if o.reset {
                    self.edit.volume = 1.0;
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Event filter").size(20.0));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit.filter)
                            .hint_text(DEFAULT_FILTER)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            screen::region(ui, rg.right.shrink(18.0), |ui| {
                game::help_text(ui, nav.help.as_deref().unwrap_or("Select a setting to read what it does."));
                ui.add_space(8.0);
                game::dim(ui, &status.note);
                ui.add_space(10.0);
                ui.label(format!(
                    "{} banks · {} events · {} live instances",
                    status.banks, status.events, status.instances
                ));
                egui::ScrollArea::vertical().max_height(150.0).id_salt("ea_events").show(ui, |ui| {
                    for (path, n) in &status.matched {
                        let line = egui::RichText::new(format!("{n:>2}  {path}")).monospace().size(13.0);
                        ui.label(if *n > 0 { line } else { line.weak() });
                    }
                });
                ui.separator();
                ui.label(egui::RichText::new("Parameters (experimental: the game rewrites them every frame)").small());
                egui::ScrollArea::vertical().max_height(200.0).id_salt("ea_params").show(ui, |ui| {
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
            nav.finish();
        });
        if exit.power_off {
            crate::mod_api::get_mod_manager().set_mod_enabled(NAME, false);
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
