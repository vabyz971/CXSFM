//! Camera tool: drive the game's Cinemachine rig instead of fighting it.
//!
//! The game films through `CinemachineVirtualCamera` + Brain (see log:
//! `RearRaceCamera`, `UpdateVirtualCameras`) — Cinemachine **v2** API.
//!
//! > **ONE-SHOT, TICK-THREAD ONLY** (lessons from Minimal HUD + a
//! > proven present-thread deadlock): repeated `FindObjectsOfType` +
//! > writes from a background thread SIGSEGVs, and ANY Unity call on
//! > the present thread freezes the image (sound alive). So: sliders
//! > edit a recipe, Apply/Go queue a single pass consumed by the tick
//! > thread, reads are user-timed (Refresh). Never enumerate or write
//! > in `on_draw_ui`. Enable/disable transitions also run on the tick
//! > (loader), so `restore()` can touch Unity. `apply()`/`do_boost()`
//! > never re-snapshot: they reuse their own enumeration to refresh
//! > the UI rows.

pub mod i18n;

use crate::mods::api::{Mod, TileIcon};
use crate::mods::tool::ToolChrome;
use crate::unity::UnityCache;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// No periodic snapshot: enumeration runs on Refresh / enable only.
/// A background heartbeat enumerating Cinemachine objects while views
/// churn (spawn/destroy) races Unity — the freeze killed as "crash"
/// came from the tick holding entry locks mid-stall, fixed structurally
/// in the loader, but there is no reason to enumerate blindly anyway.
/// Priority given to a preset-activated vcam (restored on disable).
const PRESET_PRIORITY: i32 = 100;

/// Original body values (restore source, keyed by component name).
#[derive(Clone, Copy)]
struct BodyOrig {
    offset: [f32; 3],
    damp: [f32; 3],
}

/// One live body component for the UI.
#[derive(Clone)]
struct BodyInfo {
    name: String,
    kind: &'static str,
    enabled: bool,
    offset: [f32; 3],
}

/// One vcam for the UI.
#[derive(Clone)]
struct VcamInfo {
    name: String,
    enabled: bool,
    priority: i32,
    follow: String,
}

/// Camera tool: snapshot on tick, sliders on render.
pub struct CameraMod {
    enabled: AtomicBool,
    unity: Mutex<Option<UnityCache>>,
    /// One-shot write request (Apply button / preset).
    apply_once: AtomicBool,
    /// Queued vcam name for a priority boost (Go button). Consumed by
    /// the tick thread — Unity calls must never run on the present
    /// thread (proven deadlock: image frozen, sound alive).
    pending_boost: Mutex<Option<String>>,
    /// Snapshot request (Refresh button / enable). Reads are user-timed
    /// so enumeration never races a view swap.
    refresh_wanted: AtomicBool,
    // Wanted framing (UI thread writes, tick thread applies).
    distance: Mutex<f32>,
    height: Mutex<f32>,
    smooth: Mutex<f32>,
    // Captured originals (restore on disable).
    bodies: Mutex<HashMap<String, BodyOrig>>,
    priorities: Mutex<HashMap<String, i32>>,
    // Readout snapshot (tick writes, UI reads).
    active_vcam: Mutex<String>,
    follow_name: Mutex<String>,
    main_fov: Mutex<Option<f32>>,
    vcams: Mutex<Vec<VcamInfo>>,
    bodies_info: Mutex<Vec<BodyInfo>>,
    /// Pin + opacity chrome (shared tool pattern).
    tool: ToolChrome,
}

impl CameraMod {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            unity: Mutex::new(None),
            apply_once: AtomicBool::new(false),
            pending_boost: Mutex::new(None),
            refresh_wanted: AtomicBool::new(true),
            distance: Mutex::new(1.0),
            height: Mutex::new(0.0),
            smooth: Mutex::new(1.0),
            bodies: Mutex::new(HashMap::new()),
            priorities: Mutex::new(HashMap::new()),
            active_vcam: Mutex::new(String::from("—")),
            follow_name: Mutex::new(String::from("—")),
            main_fov: Mutex::new(None),
            vcams: Mutex::new(Vec::new()),
            bodies_info: Mutex::new(Vec::new()),
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

    /// Read one body's offset + damping through cached FieldInfo.
    ///
    /// # Safety
    /// Attached tick thread; live component; fields class-validated.
    unsafe fn read_body(
        api: &crate::il2cpp::Il2cppApi,
        obj: *mut std::ffi::c_void,
        offset_field: *mut std::ffi::c_void,
        damp_fields: [*mut std::ffi::c_void; 3],
    ) -> Option<([f32; 3], [f32; 3])> {
        // SAFETY: field payloads copied by the runtime (12 / 4 bytes).
        unsafe {
            let offset = crate::il2cpp::field_get_vec3(api, obj, offset_field)?;
            let mut damp = [0f32; 3];
            for (i, f) in damp_fields.iter().enumerate() {
                damp[i] = crate::il2cpp::field_get_f32(api, obj, *f).unwrap_or(0.0);
            }
            Some((offset, damp))
        }
    }

    /// Snapshot discovery (tick thread): brains, vcams, bodies.
    /// Collects into locals, then merges under brief locks — never a
    /// MutexGuard alive across a Unity call.
    fn snapshot(&self) {
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
        let t0 = std::time::Instant::now();
        let mut enums = 0u32;
        // Locals first: active name, vcam rows, follow, fov, body rows
        // plus seed originals. Locks only below, for the merge.
        let active: String;
        let mut vcams = Vec::new();
        let mut follow = String::from("—");
        let fov: Option<f32>;
        let mut infos = Vec::new();
        let mut seeds: Vec<(String, BodyOrig)> = Vec::new();
        // SAFETY: attached tick thread; enumerations consumed now.
        unsafe {
            active = crate::unity::cm_active_vcam_name(api, &cache)
                .unwrap_or_else(|| String::from("—"));
            enums += 1;
            for v in crate::unity::cm_vcam_list(api, &cache).iter().take(32) {
                let name = crate::unity::object_name(api, &cache, *v);
                let f = crate::unity::cm_vcam_follow_name(api, &cache, *v)
                    .unwrap_or_else(|| String::from("—"));
                if name == active {
                    follow = f.clone();
                }
                // Behaviour state via the shared flag reader (vcams are Behaviours).
                let on = crate::unity::beh_get_enabled(api, &cache, *v).unwrap_or(false);
                let prio = crate::unity::cm_vcam_priority(api, &cache, *v).unwrap_or(0);
                vcams.push(VcamInfo {
                    name,
                    enabled: on,
                    priority: prio,
                    follow: f,
                });
            }
            enums += 1;
            // Main-camera FOV readout (display only — the Brain owns it).
            fov = crate::unity::main_camera(api, &cache)
                .and_then(|cam| crate::unity::get_fov(api, &cache, cam));
            // Bodies: transposers + orbitals, enabled ones drive framing.
            // Rows feed the UI; seeds merge into restore originals below.
            for (list, kind, off, damp) in [
                (
                    crate::unity::cm_transposer_list(api, &cache),
                    "Transposer",
                    cache.m_trans_offset,
                    cache.m_trans_damp,
                ),
                (
                    crate::unity::cm_orbital_list(api, &cache),
                    "Orbital",
                    cache.m_orbital_offset,
                    cache.m_orbital_damp,
                ),
            ] {
                enums += 1;
                for obj in list.into_iter().take(16) {
                    let name = crate::unity::object_name(api, &cache, obj);
                    let on = crate::unity::beh_get_enabled(api, &cache, obj).unwrap_or(false);
                    if let Some((offset, d)) = Self::read_body(api, obj, off, damp) {
                        seeds.push((name.clone(), BodyOrig { offset, damp: d }));
                        infos.push(BodyInfo {
                            name,
                            kind,
                            enabled: on,
                            offset,
                        });
                    }
                }
            }
        }
        // Brief merge: no Unity calls under these guards.
        *self.active_vcam.lock().unwrap() = active;
        *self.vcams.lock().unwrap() = vcams;
        *self.follow_name.lock().unwrap() = follow;
        *self.main_fov.lock().unwrap() = fov;
        {
            let mut known = self.bodies.lock().unwrap();
            for (name, orig) in seeds {
                known.entry(name).or_insert(orig);
            }
        }
        *self.bodies_info.lock().unwrap() = infos;
        crate::log_line(&format!(
            "camera: snapshot took {} ms, {} enumerations",
            t0.elapsed().as_millis(),
            enums
        ));
    }

    /// Apply wanted framing to every enabled body (tick, on change).
    /// Single pass, no re-snapshot: UI rows refresh from the read-backs
    /// below (one enumeration per body kind).
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
        let dist = *self.distance.lock().unwrap();
        let h = *self.height.lock().unwrap();
        let smooth = *self.smooth.lock().unwrap();
        let known = self.bodies.lock().unwrap().clone();
        let t0 = std::time::Instant::now();
        let mut enums = 0u32;
        let mut infos = Vec::new();
        // SAFETY: attached tick thread; field writes + read-backs.
        unsafe {
            for (list, kind, off, damp) in [
                (
                    crate::unity::cm_transposer_list(api, &cache),
                    "Transposer",
                    cache.m_trans_offset,
                    cache.m_trans_damp,
                ),
                (
                    crate::unity::cm_orbital_list(api, &cache),
                    "Orbital",
                    cache.m_orbital_offset,
                    cache.m_orbital_damp,
                ),
            ] {
                enums += 1;
                for obj in list.into_iter().take(16) {
                    let name = crate::unity::object_name(api, &cache, obj);
                    let on = crate::unity::beh_get_enabled(api, &cache, obj).unwrap_or(false);
                    // Display row always shows the live offset when
                    // readable; only enabled + known bodies get written.
                    let live = Self::read_body(api, obj, off, damp).map(|(o, _)| o);
                    let base = known.get(&name).copied();
                    match (on, base, live) {
                        (true, Some(base), _) => {
                            // Distance scales the captured vector, height shifts Y.
                            let want = [
                                base.offset[0] * dist,
                                base.offset[1] + h,
                                base.offset[2] * dist,
                            ];
                            let ok_off = crate::il2cpp::field_set_vec3(api, obj, off, want);
                            let back_off = crate::il2cpp::field_get_vec3(api, obj, off);
                            let mut ok_damp = true;
                            for f in damp {
                                ok_damp &= crate::il2cpp::field_set_f32(api, obj, f, smooth);
                            }
                            crate::log_line(&format!(
                                "camera: '{name}' offset -> [{:.1}, {:.1}, {:.1}] (ok={ok_off} readback={back_off:?}) damp -> {smooth:.1} (ok={ok_damp})",
                                want[0], want[1], want[2]
                            ));
                            infos.push(BodyInfo {
                                name,
                                kind,
                                enabled: true,
                                offset: back_off.unwrap_or(want),
                            });
                        }
                        (_, _, Some(offset)) => {
                            infos.push(BodyInfo {
                                name,
                                kind,
                                enabled: on,
                                offset,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
        *self.bodies_info.lock().unwrap() = infos;
        crate::log_line(&format!(
            "camera: apply took {} ms, {} enumerations",
            t0.elapsed().as_millis(),
            enums
        ));
    }

    /// Queue a priority boost (UI/present thread): stores the name only,
    /// no Unity calls here — the tick thread consumes it below.
    fn activate_vcam(&self, name: &str) {
        *self.pending_boost.lock().unwrap() = Some(name.to_string());
        crate::log_line(&format!("camera: boost queued for '{name}' (tick applies)"));
    }

    /// Boost one vcam's priority (tick thread, one-shot); originals
    /// restored on disable. Refreshes the matching UI row from the
    /// read-back instead of re-snapshotting.
    fn do_boost(&self, name: &str) {
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
        let t0 = std::time::Instant::now();
        // SAFETY: attached tick thread; single enumeration + write,
        // then stop (one-shot — never a loop, never on present).
        unsafe {
            for v in crate::unity::cm_vcam_list(api, &cache).iter().take(32) {
                if crate::unity::object_name(api, &cache, *v) != name {
                    continue;
                }
                let was = crate::unity::cm_vcam_priority(api, &cache, *v).unwrap_or(10);
                self.priorities.lock().unwrap().entry(name.to_string()).or_insert(was);
                if crate::unity::cm_set_vcam_priority(api, &cache, *v, PRESET_PRIORITY) {
                    let back = crate::unity::cm_vcam_priority(api, &cache, *v);
                    crate::log_line(&format!(
                        "camera: '{name}' priority {was} -> {PRESET_PRIORITY} (readback={back:?})"
                    ));
                    let mut vcams = self.vcams.lock().unwrap();
                    if let Some(row) = vcams.iter_mut().find(|r| r.name == name) {
                        row.priority = back.unwrap_or(PRESET_PRIORITY);
                    }
                }
            }
        }
        crate::log_line(&format!(
            "camera: boost took {} ms, 1 enumeration",
            t0.elapsed().as_millis()
        ));
    }

    /// Restore offsets, damping and priorities (disable path).
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
        let bodies = std::mem::take(&mut *self.bodies.lock().unwrap());
        let prios = std::mem::take(&mut *self.priorities.lock().unwrap());
        let t0 = std::time::Instant::now();
        let mut enums = 0u32;
        // SAFETY: same contract as `apply`.
        unsafe {
            for (list, off, damp) in [
                (
                    crate::unity::cm_transposer_list(api, &cache),
                    cache.m_trans_offset,
                    cache.m_trans_damp,
                ),
                (
                    crate::unity::cm_orbital_list(api, &cache),
                    cache.m_orbital_offset,
                    cache.m_orbital_damp,
                ),
            ] {
                enums += 1;
                for obj in list.into_iter().take(16) {
                    let name = crate::unity::object_name(api, &cache, obj);
                    if let Some(orig) = bodies.get(&name) {
                        crate::il2cpp::field_set_vec3(api, obj, off, orig.offset);
                        for (f, v) in damp.iter().zip(orig.damp.iter()) {
                            crate::il2cpp::field_set_f32(api, obj, *f, *v);
                        }
                    }
                }
            }
            enums += 1;
            for v in crate::unity::cm_vcam_list(api, &cache).iter().take(32) {
                let name = crate::unity::object_name(api, &cache, *v);
                if let Some(was) = prios.get(&name) {
                    crate::unity::cm_set_vcam_priority(api, &cache, *v, *was);
                }
            }
        }
        // Reset sliders to neutral for the next arming.
        *self.distance.lock().unwrap() = 1.0;
        *self.height.lock().unwrap() = 0.0;
        crate::log_line(&format!(
            "camera: originals restored (took {} ms, {} enumerations)",
            t0.elapsed().as_millis(),
            enums
        ));
    }
}

impl Default for CameraMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Mod for CameraMod {
    fn name(&self) -> &'static str {
        "Camera Rig"
    }

    fn mod_id(&self) -> &'static str {
        "camera"
    }

    fn menu_label_key(&self) -> &'static str {
        "mod_camera"
    }

    fn menu_icon(&self) -> TileIcon {
        TileIcon::Camera
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
        if self.apply_once.swap(false, Ordering::SeqCst) {
            self.apply();
        }
        if let Some(name) = self.pending_boost.lock().unwrap().take() {
            self.do_boost(&name);
        }
        // Reads are user-timed (Refresh) — see module docs.
        if self.refresh_wanted.swap(false, Ordering::SeqCst) {
            self.snapshot();
        }
    }

    fn on_draw_ui(&mut self, ctx: &egui::Context) {
        let win = egui::Window::new("Camera Rig")
            .default_size(egui::Vec2::new(400.0, 520.0))
            .frame(self.tool.frame(ctx))
            .show(ctx, |ui| {
                self.tool.enter(ui);
                ui.horizontal(|ui| {
                    self.tool.pin_toggle(ui);
                    if ui.button("Refresh").clicked() {
                        // User-timed enumeration (scene calm, no view swap).
                        self.refresh_wanted.store(true, Ordering::SeqCst);
                    }
                });
                let active = self.active_vcam.lock().unwrap().clone();
                let follow = self.follow_name.lock().unwrap().clone();
                let fov = *self.main_fov.lock().unwrap();
                ui.label(format!("live: {active}"));
                ui.label(format!("follow: {follow}"));
                ui.label(match fov {
                    Some(v) => format!("main fov: {v:.0}° (Brain-owned, readout only)"),
                    None => String::from("main fov: —"),
                });
                ui.separator();
                ui.heading("Framing (sliders edit the recipe — nothing writes yet)");
                {
                    let mut d = self.distance.lock().unwrap();
                    ui.add(egui::Slider::new(&mut *d, 0.2..=2.5).text("distance"));
                }
                {
                    let mut h = self.height.lock().unwrap();
                    ui.add(egui::Slider::new(&mut *h, -6.0..=8.0).text("height"));
                }
                {
                    let mut s = self.smooth.lock().unwrap();
                    ui.add(egui::Slider::new(&mut *s, 0.0..=5.0).text("smooth"));
                }
                if ui.button("Apply framing (one-shot write)").clicked() {
                    self.apply_once.store(true, Ordering::SeqCst);
                }
                ui.horizontal(|ui| {
                    for (label, d, h) in [
                        ("Chase", 1.0, 0.0),
                        ("Near", 0.6, -1.0),
                        ("Far", 1.7, 1.5),
                        ("Top", 0.5, 8.0),
                    ] {
                        if ui.button(label).clicked() {
                            *self.distance.lock().unwrap() = d;
                            *self.height.lock().unwrap() = h;
                            self.apply_once.store(true, Ordering::SeqCst);
                        }
                    }
                });
                ui.separator();
                ui.heading("Bodies");
                for b in self.bodies_info.lock().unwrap().clone() {
                    ui.label(format!(
                        "{} [{}] {} off [{:.1}, {:.1}, {:.1}]",
                        if b.enabled { "●" } else { "○" },
                        b.kind,
                        b.name,
                        b.offset[0],
                        b.offset[1],
                        b.offset[2]
                    ));
                }
                ui.separator();
                ui.heading("Vcams (Go = priority boost via tick, logged)");
                for v in self.vcams.lock().unwrap().clone() {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{} {} (prio {}, follow {})",
                            if v.enabled { "●" } else { "○" },
                            v.name,
                            v.priority,
                            v.follow
                        ));
                        if ui.small_button("Go").clicked() {
                            self.activate_vcam(&v.name);
                        }
                    });
                }
            });
        if let Some(r) = win {
            self.tool.context_menu(&r.response);
        }
    }

    fn on_enable(&mut self) {
        self.enabled.store(true, Ordering::SeqCst);
        // Fresh snapshot on the next tick: the scene may have changed
        // while off, and restore originals must seed from live values.
        self.refresh_wanted.store(true, Ordering::SeqCst);
        crate::log_line("camera: armed");
    }

    fn on_disable(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.restore();
        crate::log_line("camera: off (originals restored)");
    }
}

/// Register disarmed: boots quiet, user opts in from the menu tile.
pub fn register() {
    crate::mod_api::register_mod(Box::new(CameraMod::new()));
}
