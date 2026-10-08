//! Menu pages: main tile grid, settings and about.
//!
//! Every page is constrained to the grid block width with wrapping
//! labels so long translations never widen the window; the block is
//! centered (see `chrome::centered_block`).

use super::chrome::{centered_block, forward_drag, nav_button};
use super::layout::{GridLayout, grid_layout};
use super::theme::{BODY_SIZE, GRID_GAP, SMALL_SIZE};
use super::tiles::tile_at;
use crate::menu::{Page, set_page};
use crate::mod_api::get_mod_manager;
use crate::mods::api::TileIcon;

/// Main tile grid: one tile per mod, then Settings + About.
///
/// Responsive: up to 4 columns from the available width, extras wrap
/// below; past 8 mods the tiles shrink. Tiles sit at explicit
/// `x = c * (tile+12)` / `y = r * (tile+12)` — no `Grid`, no flow
/// layout, so nothing justifies or stretches the gaps.
pub fn page_main(ui: &mut egui::Ui, layout: &GridLayout) {
    let mgr = get_mod_manager();
    let tiles = mgr.tile_info();
    let mut go_settings = false;
    let mut dirty = false;

    struct Item {
        icon: TileIcon,
        label: String,
        active: bool,
        kind: u8, // 0 = mod toggle, 1 = settings, 2 = about
    }
    let mut items: Vec<Item> = Vec::new();
    for t in &tiles {
        items.push(Item {
            icon: t.icon,
            label: crate::i18n::t(t.label_key).to_string(),
            active: t.enabled,
            kind: 0,
        });
    }
    items.push(Item {
        icon: TileIcon::Globe,
        label: crate::i18n::t("tile_settings").to_string(),
        active: false,
        kind: 1,
    });
    items.push(Item {
        icon: TileIcon::Info,
        label: crate::i18n::t("tile_about").to_string(),
        active: false,
        kind: 2,
    });
    // Keep a parallel list of mod names for toggles (settings/about
    // entries have none).
    let names: Vec<Option<String>> = tiles
        .iter()
        .map(|t| Some(t.name.clone()))
        .chain(std::iter::repeat(None).take(2))
        .collect();

    let cols = layout.cols;
    let tile = layout.tile;
    let rows = items.len().div_ceil(cols);
    let grid_h = rows as f32 * tile.y + (rows as f32 - 1.0).max(0.0) * GRID_GAP;
    centered_block(ui, layout.block_w, |ui| {
        let (block, _) =
            ui.allocate_exact_size(egui::Vec2::new(layout.block_w, grid_h), egui::Sense::hover());
        for (idx, item) in items.iter().enumerate() {
            let r = idx / cols;
            let c = idx % cols;
            let min = block.min
                + egui::Vec2::new(
                    c as f32 * (tile.x + GRID_GAP),
                    r as f32 * (tile.y + GRID_GAP),
                );
            let rect = egui::Rect::from_min_size(min, tile);
            let id = ui.make_persistent_id(format!("cxsfm_tile_{idx}"));
            let resp = tile_at(ui, rect, id, item.icon, &item.label, item.active, layout.label);
            // Drag from anywhere: a drag started on a tile moves the
            // window; a clean click activates it.
            forward_drag(&resp);
            if resp.clicked() {
                match item.kind {
                    1 => go_settings = true,
                    2 => set_page(Page::About),
                    _ => {
                        if let Some(name) = &names[idx] {
                            mgr.set_mod_enabled(name, !item.active);
                            dirty = true;
                        }
                    }
                }
            }
        }
    });
    if go_settings {
        set_page(Page::Settings);
    }
    if dirty {
        ui.ctx().request_repaint();
    }
}

/// Compute the grid for this `ui`'s available width (mods + 2 implicit
/// Settings/About tiles).
pub fn content_layout(ui: &egui::Ui) -> GridLayout {
    let avail = ui.available_width();
    let mods = get_mod_manager().mod_count();
    grid_layout(avail, mods)
}

/// Settings page: language pin + keybind reminder.
pub fn page_settings(ui: &mut egui::Ui, w: f32) {
    centered_block(ui, w, |ui| {
        ui.allocate_ui_with_layout(
            egui::Vec2::new(w, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
            if nav_button(ui, crate::i18n::t("back")).clicked() {
                set_page(Page::Main);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(crate::i18n::t("settings_language")).size(BODY_SIZE));
                let current = crate::i18n::override_lang();
                let shown = match current {
                    Some(l) => format!("{} ({})", l.name(), l.code()),
                    None => crate::i18n::t("settings_auto").to_string(),
                };
                egui::ComboBox::from_id_salt("cxsfm_lang")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(current.is_none(), crate::i18n::t("settings_auto"))
                            .clicked()
                        {
                            crate::i18n::set_override(None);
                        }
                        for l in crate::i18n::Lang::ALL {
                            if ui
                                .selectable_label(
                                    current == Some(*l),
                                    format!("{} ({})", l.name(), l.code()),
                                )
                                .clicked()
                            {
                                crate::i18n::set_override(Some(*l));
                            }
                        }
                    });
            });
            let detected = match crate::i18n::detected() {
                Some(l) => format!("{} ({})", l.name(), l.code()),
                None => "—".to_string(),
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{}: {}",
                        crate::i18n::t("settings_detected"),
                        detected
                    ))
                    .size(SMALL_SIZE),
                )
                .wrap()
                .sense(egui::Sense::hover()),
            );
            ui.separator();
            ui.label(egui::RichText::new(crate::i18n::t("settings_keys")).size(BODY_SIZE));
            for key in ["keys_f8", "keys_note"] {
                ui.add(
                    egui::Label::new(egui::RichText::new(crate::i18n::t(key)).size(SMALL_SIZE))
                        .wrap()
                        .sense(egui::Sense::hover()),
                );
            }
            },
        );
    });
}

/// About page: version, stats, HELP content.
pub fn page_about(ui: &mut egui::Ui, w: f32) {
    centered_block(ui, w, |ui| {
        ui.allocate_ui_with_layout(
            egui::Vec2::new(w, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
            if nav_button(ui, crate::i18n::t("back")).clicked() {
                set_page(Page::Main);
            }
            ui.separator();
            let mgr = get_mod_manager();
            ui.label(
                egui::RichText::new(format!("CXSFM {}", env!("CARGO_PKG_VERSION"))).size(BODY_SIZE),
            );
            for line in [
                format!("Platform: {}", std::env::consts::OS),
                format!("Uptime: {} s", mgr.uptime_secs()),
                format!(
                    "Mods: {} enabled / {} total",
                    mgr.enabled_count(),
                    mgr.mod_count()
                ),
                format!("Log: {}", crate::log_path().display()),
            ] {
                ui.add(
                    egui::Label::new(egui::RichText::new(line).size(SMALL_SIZE))
                        .wrap()
                        .sense(egui::Sense::hover()),
                );
            }
            ui.separator();
            ui.label(egui::RichText::new(crate::i18n::t("settings_keys")).size(BODY_SIZE));
            for key in ["keys_f8", "keys_note"] {
                ui.add(
                    egui::Label::new(egui::RichText::new(crate::i18n::t(key)).size(SMALL_SIZE))
                        .wrap()
                        .sense(egui::Sense::hover()),
                );
            }
            ui.separator();
            // One line per mod: name + own-language description + state.
            // Descriptions come from each mod's `i18n` module, so adding
            // a mod never touches framework translation files.
            let code = crate::i18n::current().code();
            for (name, on, desc) in mgr.describe_all(code) {
                let line = if desc.is_empty() {
                    format!("• {name} ({})", if on { "on" } else { "off" })
                } else {
                    format!("• {name} — {desc} ({})", if on { "on" } else { "off" })
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(line).size(SMALL_SIZE))
                        .wrap()
                        .sense(egui::Sense::hover()),
                );
            }
            },
        );
    });
}
