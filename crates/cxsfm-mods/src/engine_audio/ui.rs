//! Engine-audio phone app: game-style rows (pitch, volume, filter),
//! live events and the parameter list, one tab each. Edits stage in
//! `self.edit` and reach the tick through one brief store (no
//! present-thread work beyond that).

use super::EngineAudioMod;
use super::model::{DEFAULT_FILTER, MAX_OVERRIDES, PITCH_RANGE, ParamOverride, VOLUME_RANGE};
use cxsfm_api::common::lock;
use cxsfm_ui::app::PAD_OWNER;
use cxsfm_ui::game::{self, Nav};

/// Phone tabs (see `Mod::app_tabs`).
pub(crate) const TABS: &[&str] = &["Engine", "Events", "Parameters"];
const TAB_ENGINE: usize = 0;
const TAB_EVENTS: usize = 1;

impl EngineAudioMod {
    /// Body of tab `tab` inside the phone; returns the focused row's help.
    pub(crate) fn draw_app(&mut self, ui: &mut egui::Ui, tab: usize) -> Option<String> {
        let status = lock(&self.status).clone();
        let mut nav = Nav::begin(ui, &mut self.nav, PAD_OWNER);
        game::dim(ui, &status.note);
        ui.add_space(6.0);
        match tab {
            TAB_ENGINE => {
                game::section(ui, "Engine");
                let o = game::slider_row(ui, &mut nav, "ea_pitch", "Pitch", &mut self.edit.pitch, PITCH_RANGE, 0.05, 2, "x", "Scales the pitch of the matching engine instances (1.00 = untouched).");
                if o.reset {
                    self.edit.pitch = 1.0;
                }
                let o = game::slider_row(ui, &mut nav, "ea_volume", "Volume", &mut self.edit.volume, VOLUME_RANGE, 0.05, 2, "x", "Scales the volume of the matching engine instances (1.00 = untouched).");
                if o.reset {
                    self.edit.volume = 1.0;
                }
                game::section(ui, "Event filter");
                ui.add(
                    egui::TextEdit::singleline(&mut self.edit.filter)
                        .hint_text(DEFAULT_FILTER)
                        .desired_width(f32::INFINITY),
                );
                game::dim(ui, "Only FMOD events whose path contains this text are driven.");
            }
            TAB_EVENTS => {
                game::section(ui, "Live events");
                ui.label(format!("{} banks · {} events · {} live instances", status.banks, status.events, status.instances));
                ui.add_space(4.0);
                for (path, n) in &status.matched {
                    let line = egui::RichText::new(format!("{n:>2}  {path}")).monospace().size(13.0);
                    ui.label(if *n > 0 { line } else { line.weak() });
                }
            }
            _ => {
                game::section(ui, "Parameters");
                game::warning(ui, "EXPERIMENTAL — the game rewrites them every frame");
                ui.add_space(6.0);
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
                if status.params.is_empty() {
                    game::dim(ui, "No live engine instance yet: drive, then come back.");
                }
            }
        }
        let help = nav.help.clone();
        nav.finish();
        self.push_if_changed();
        help
    }

    fn push_if_changed(&mut self) {
        let mut shared = lock(&self.shared);
        if *shared != self.edit {
            *shared = self.edit.clone();
        }
    }
}
