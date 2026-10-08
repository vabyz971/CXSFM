//! Video tool: reuse the game's own settings functions.
//!
//! Instead of going through the game's HUD, this calls the engine
//! methods directly (`QualitySettings`, `Screen`, `RenderSettings` —
//! the same invoke shapes as FOV/text, proven). Every knob the game
//! exposes but its menu hides becomes settable; disabling the mod
//! restores the exact values captured on enable.
//!
//! Writes run on the tick thread (attached); the UI only edits a
//! `wanted` copy. Resolution applies on its Apply button (no thrash);
//! other knobs apply on change with read-back logging.

pub mod i18n;

use crate::mods::api::Mod;
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Refresh cadence for the live readout (~1 Hz at 60 Hz).
const POLL_EVERY_TICKS: u64 = 60;

/// Desired values; `None` = unsupported on this build (getter missing).
#[derive(Clone, Default)]
struct Wanted {
    level: Option<i32>,
    vsync: Option<i32>,
    aa: Option<i32>,
    shadows: Option<i32>,
    fog: Option<bool>,
    fog_density: Option<f32>,
    ambient: Option<f32>,
    res_w: i32,
    res_h: i32,
    fullscreen: bool,
    apply_res: bool,
}

/// Captured originals for restore-on-disable.
#[derive(Clone, Default)]
struct Originals {
    level: Option<i32>,
    vsync: Option<i32>,
    aa: Option<i32>,
    shadows: Option<i32>,
    fog: Option<bool>,
    fog_density: Option<f32>,
    ambient: Option<f32>,
}

/// Video tool: capture on tick, apply diffs on tick, restore on disable.
pub struct VideoMod {
    enabled: AtomicBool,
    unity: Mutex<Option<UnityCache>>,
    tick: AtomicU64,
    wanted: Mutex<Wanted>,
    dirty: AtomicBool,
    captured: AtomicBool,
    originals: Mutex<Option<Originals>>,
    level_names: Mutex<Vec<String>>,
    live_summary: Mutex<String>,
    /// Pin + opacity chrome (shared tool pattern).
    tool: ToolChrome,
}

impl VideoMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            tick: AtomicU64::new(0),
            wanted: Mutex::new(Wanted::default()),
            dirty: AtomicBool::new(false),
            captured: AtomicBool::new(false),
            originals: Mutex::new(None),
            level_names: Mutex::new(Vec::new()),
            live_summary: Mutex::new(String::from("—")),
            tool: ToolChrome::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn ensure_unity(&self) -> bool {
        if self.unity.lock().unwrap().is_some() {
            return true;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return false,
        };
        // SAFETY: attached framework threads, null-checked chain.
        let cache = unsafe {
            let domain = match crate::il2cpp::domain_checked(api) {
                Some(d) => d,
                None => return false,
            };
            match crate::unity::init(api, domain) {
                Some(c) => c,
                None => return false,
            }
        };
        *self.unity.lock().unwrap() = Some(cache);
        true
    }

    /// Read everything live (tick thread). Missing getters → `None`
    /// (knob hidden in the UI).
    fn read_live(&self) -> Option<Wanted> {
        if !self.ensure_unity() {
            return None;
        }
        let api = crate::il2cpp_api()?;
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = cached?;
        // SAFETY: attached tick thread; reads only.
        unsafe {
            let res = crate::unity::screen_resolution(api, &cache);
            let w = Wanted {
                level: crate::unity::quality_level(api, &cache),
                vsync: crate::unity::vsync_count(api, &cache),
                aa: crate::unity::antialiasing(api, &cache),
                shadows: crate::unity::shadows(api, &cache),
                fog: crate::unity::render_fog(api, &cache),
                fog_density: crate::unity::render_fog_density(api, &cache),
                ambient: crate::unity::render_ambient(api, &cache),
                res_w: res.map(|r| r.0).unwrap_or(0),
                res_h: res.map(|r| r.1).unwrap_or(0),
                fullscreen: crate::unity::screen_fullscreen(api, &cache).unwrap_or(true),
                apply_res: false,
            };
            let _ = res;
            Some(w)
        }
    }

    /// First tick after enable: capture originals, seed `wanted` with
    /// live values so sliders start at the current state.
    fn capture(&self) {
        let live = match self.read_live() {
            Some(l) => l,
            None => return,
        };
        *self.originals.lock().unwrap() = Some(Originals {
            level: live.level,
            vsync: live.vsync,
            aa: live.aa,
            shadows: live.shadows,
            fog: live.fog,
            fog_density: live.fog_density,
            ambient: live.ambient,
        });
        *self.wanted.lock().unwrap() = live;
        self.captured.store(true, Ordering::SeqCst);
        crate::log_line("video: originals captured");
    }

    /// Apply `wanted` diffs (tick thread). Each set is read back and
    /// logged — a silent setter shows up immediately.
    fn apply(&self) {
        if !self.ensure_unity() {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        let w = self.wanted.lock().unwrap().clone();
        // SAFETY: attached tick thread; setters + read-backs.
        unsafe {
            if let Some(v) = w.level {
                if crate::unity::set_quality_level(api, &cache, v) {
                    crate::log_line(&format!(
                        "video: quality -> {} (readback {:?})",
                        v,
                        crate::unity::quality_level(api, &cache)
                    ));
                }
            }
            if let Some(v) = w.vsync {
                if crate::unity::set_vsync_count(api, &cache, v) {
                    crate::log_line(&format!(
                        "video: vsync -> {} (readback {:?})",
                        v,
                        crate::unity::vsync_count(api, &cache)
                    ));
                }
            }
            if let Some(v) = w.aa {
                if crate::unity::set_antialiasing(api, &cache, v) {
                    crate::log_line(&format!(
                        "video: aa -> {} (readback {:?})",
                        v,
                        crate::unity::antialiasing(api, &cache)
                    ));
                }
            }
            if let Some(v) = w.shadows {
                if crate::unity::set_shadows(api, &cache, v) {
                    crate::log_line(&format!(
                        "video: shadows -> {} (readback {:?})",
                        v,
                        crate::unity::shadows(api, &cache)
                    ));
                }
            }
            if let Some(v) = w.fog {
                if crate::unity::set_render_fog(api, &cache, v) {
                    crate::log_line(&format!(
                        "video: fog -> {} (readback {:?})",
                        v,
                        crate::unity::render_fog(api, &cache)
                    ));
                }
            }
            if let Some(v) = w.fog_density {
                if crate::unity::set_render_fog_density(api, &cache, v) {
                    crate::log_line(&format!(
                        "video: fog density -> {:.3} (readback {:?})",
                        v,
                        crate::unity::render_fog_density(api, &cache)
                    ));
                }
            }
            if let Some(v) = w.ambient {
                if crate::unity::set_render_ambient(api, &cache, v) {
                    crate::log_line(&format!(
                        "video: ambient -> {:.2} (readback {:?})",
                        v,
                        crate::unity::render_ambient(api, &cache)
                    ));
                }
            }
            if w.apply_res && w.res_w > 0 && w.res_h > 0 {
                if crate::unity::screen_set_resolution(api, &cache, w.res_w, w.res_h, w.fullscreen)
                {
                    crate::log_line(&format!(
                        "video: resolution -> {}x{} fs={} (readback {:?})",
                        w.res_w,
                        w.res_h,
                        w.fullscreen,
                        crate::unity::screen_resolution(api, &cache)
                    ));
                }
                self.wanted.lock().unwrap().apply_res = false;
            }
        }
    }

    /// Restore captured originals (disable path — same thread contract
    /// as the HUD mods' restore).
    fn restore(&self) {
        if !self.ensure_unity() {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        let orig = self.originals.lock().unwrap().clone().unwrap_or_default();
        // SAFETY: same contract as `apply`.
        unsafe {
            if let Some(v) = orig.level {
                crate::unity::set_quality_level(api, &cache, v);
            }
            if let Some(v) = orig.vsync {
                crate::unity::set_vsync_count(api, &cache, v);
            }
            if let Some(v) = orig.aa {
                crate::unity::set_antialiasing(api, &cache, v);
            }
            if let Some(v) = orig.shadows {
                crate::unity::set_shadows(api, &cache, v);
            }
            if let Some(v) = orig.fog {
                crate::unity::set_render_fog(api, &cache, v);
            }
            if let Some(v) = orig.fog_density {
                crate::unity::set_render_fog_density(api, &cache, v);
            }
            if let Some(v) = orig.ambient {
                crate::unity::set_render_ambient(api, &cache, v);
            }
        }
        crate::log_line("video: originals restored");
    }

    /// Refresh names + one-line live summary for the UI.
    fn poll_live(&self) {
        if !self.ensure_unity() {
            return;
        }
        let api = match crate::il2cpp_api() {
            Some(a) => a,
            None => return,
        };
        let cached = self.unity.lock().unwrap().as_ref().copied();
        let cache = match cached {
            Some(c) => c,
            None => return,
        };
        // SAFETY: attached tick thread; reads only.
        unsafe {
            *self.level_names.lock().unwrap() = crate::unity::quality_names(api, &cache);
            let res = crate::unity::screen_resolution(api, &cache);
            *self.live_summary.lock().unwrap() = match res {
                Some((w, h, hz)) => format!("{w}x{h}@{hz}"),
                None => String::from("unknown"),
            };
        }
    }
}

impl Default for VideoMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for VideoMod {
    fn name(&self) -> &'static str {
        "Video Settings"
    }

    fn mod_id(&self) -> &'static str {
        "video"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_video"
    }

    fn menu_icon(&self) -> crate::mods::api::TileIcon {
        crate::mods::api::TileIcon::Sliders
    }

    fn describe_in(&self, lang_code: &str) -> &'static str {
        self::i18n::describe(lang_code)
    }

    fn is_pinned(&self) -> bool {
        self.tool.pin
    }

    fn on_update(&mut self, _delta_time: f32) {
        if !self.is_enabled() {
            return;
        }
        if !self.captured.load(Ordering::SeqCst) {
            self.capture();
            self.poll_live();
            return;
        }
        if self.dirty.swap(false, Ordering::SeqCst) {
            self.apply();
        }
        let tick = self.tick.fetch_add(1, Ordering::SeqCst);
        if tick % POLL_EVERY_TICKS == 0 {
            self.poll_live();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        let win = egui::Window::new("Video Settings")
            .default_size(egui::Vec2::new(380.0, 480.0))
            .frame(self.tool.frame(ctx))
            .show(ctx, |ui| {
                self.tool.enter(ui);
                self.tool.pin_toggle(ui);
                let mut w = self.wanted.lock().unwrap();
                let names = self.level_names.lock().unwrap().clone();
                let summary = self.live_summary.lock().unwrap().clone();
                ui.label(format!("live: {summary}"));
                ui.separator();
                ui.heading("Quality");
                if let Some(level) = w.level.as_mut() {
                    let max = names.len().saturating_sub(1).max(1) as i32;
                    let name = names.get(*level as usize).cloned().unwrap_or_default();
                    if ui
                        .add(egui::Slider::new(level, 0..=max).text(format!("level {name}")))
                        .changed()
                    {
                        self.dirty.store(true, Ordering::SeqCst);
                    }
                } else {
                    ui.weak("quality level n/a");
                }
                if let Some(v) = w.vsync.as_mut() {
                    if ui.add(egui::Slider::new(v, 0..=4).text("vsync")).changed() {
                        self.dirty.store(true, Ordering::SeqCst);
                    }
                }
                if w.aa.is_some() {
                    let mut sel = w.aa.unwrap_or(0);
                    egui::ComboBox::from_label("anti-aliasing")
                        .selected_text(format!("{sel}x"))
                        .show_ui(ui, |ui| {
                            for opt in [0, 2, 4, 8] {
                                if ui.selectable_label(sel == opt, format!("{opt}x")).clicked() {
                                    sel = opt;
                                }
                            }
                        });
                    if sel != w.aa.unwrap_or(0) {
                        w.aa = Some(sel);
                        self.dirty.store(true, Ordering::SeqCst);
                    }
                }
                if let Some(v) = w.shadows.as_mut() {
                    if ui.add(egui::Slider::new(v, 0..=2).text("shadows")).changed() {
                        self.dirty.store(true, Ordering::SeqCst);
                    }
                }
                ui.separator();
                ui.heading("Screen");
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::DragValue::new(&mut w.res_w).speed(8.0).prefix("w "))
                        .changed()
                    {
                        // Applied on the Apply button below.
                    }
                    ui.add(egui::DragValue::new(&mut w.res_h).speed(8.0).prefix("h "));
                    if ui.checkbox(&mut w.fullscreen, "fullscreen").changed() {
                        // Applied together with the resolution.
                    }
                });
                if ui.button("Apply resolution").clicked() {
                    w.apply_res = true;
                    self.dirty.store(true, Ordering::SeqCst);
                }
                ui.separator();
                ui.heading("Atmosphere");
                if let Some(v) = w.fog.as_mut() {
                    if ui.checkbox(v, "fog").changed() {
                        self.dirty.store(true, Ordering::SeqCst);
                    }
                }
                if let Some(v) = w.fog_density.as_mut() {
                    if ui.add(egui::Slider::new(v, 0.0..=0.2).text("fog density")).changed() {
                        self.dirty.store(true, Ordering::SeqCst);
                    }
                }
                if let Some(v) = w.ambient.as_mut() {
                    if ui.add(egui::Slider::new(v, 0.0..=3.0).text("ambient")).changed() {
                        self.dirty.store(true, Ordering::SeqCst);
                    }
                }
            });
        if let Some(r) = win {
            self.tool.context_menu(&r.response);
        }
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        // Captured on the next tick (attached thread), not here.
        self.captured.store(false, Ordering::SeqCst);
        crate::log_line("video: armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore();
        self.captured.store(false, Ordering::SeqCst);
        crate::log_line("video: off (originals restored)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(VideoMod::new()));
}
