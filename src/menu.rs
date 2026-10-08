//! Kino-style mod menu: tile grid + lock footer.
//!
//! One window replaces the old manager/status pair: a header (version,
//! title, HELP), a grid of icon tiles (one per registered mod, then
//! Settings and About), and a red footer warning that game controls
//! are locked while the menu is up. All strings go through `i18n`.
//!
//! Icons are drawn with the egui `Painter` (no font glyphs, no assets)
//! so they render identically on every setup.

use crate::i18n;
use crate::mod_api::get_mod_manager;
use std::sync::atomic::{AtomicU8, Ordering};

/// Vector icon drawn on a mod tile.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TileIcon {
    Camera,
    Scout,
    EyeOff,
    Tag,
    Globe,
    Info,
}

/// Menu page (single window, swaps content).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Settings,
    About,
}

impl Page {
    fn encode(self) -> u8 {
        match self {
            Page::Main => 0,
            Page::Settings => 1,
            Page::About => 2,
        }
    }

    fn decode(v: u8) -> Page {
        match v {
            1 => Page::Settings,
            2 => Page::About,
            _ => Page::Main,
        }
    }
}

static PAGE: AtomicU8 = AtomicU8::new(0);
/// Window position (custom drag — `title_bar(false)` kills the built-in
/// drag). `None` = engine default until the first drag.
static WIN_POS: std::sync::Mutex<Option<egui::Pos2>> = std::sync::Mutex::new(None);
/// Last measured window rect (anchors drags without a first-frame jump).
static WIN_RECT: std::sync::Mutex<Option<egui::Rect>> = std::sync::Mutex::new(None);

/// Move the pinned window by a drag delta. Shared by the background
/// handle and the tiles so the menu drags from anywhere.
fn shift_window(delta: egui::Vec2) {
    if delta == egui::Vec2::ZERO {
        return;
    }
    let mut pos = WIN_POS.lock().unwrap();
    let anchor = (*pos).or_else(|| WIN_RECT.lock().unwrap().map(|r| r.min));
    if let Some(cur) = anchor {
        *pos = Some(cur + delta);
    }
}

fn set_page(p: Page) {
    PAGE.store(p.encode(), Ordering::SeqCst);
}

fn page() -> Page {
    Page::decode(PAGE.load(Ordering::SeqCst))
}

// Palette (dark translucent like the reference).
const WIN_BG: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x3E);
const TILE_BG: egui::Color32 = egui::Color32::from_rgb(0x2B, 0x2B, 0x2F);
const TILE_HOVER: egui::Color32 = egui::Color32::from_rgb(0x3F, 0x3F, 0x46);
const TILE_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(0x33, 0x42, 0x38);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x7B, 0xC7, 0x8A);
const ICON_COLOR: egui::Color32 = egui::Color32::from_rgb(0xCC, 0xCC, 0xD2);
const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0xAA, 0xAA, 0xB2);
const FOOTER_BG: egui::Color32 = egui::Color32::from_rgb(0xA8, 0x1C, 0x2C);

const TILE_SIZE: egui::Vec2 = egui::Vec2::new(125.0, 110.0);
/// Rounded corners: 10px everywhere (tiles, window, footer, outline).
const ROUNDING: f32 = 10.0;
/// Tile grid geometry: 3 columns, fixed gaps.
const GRID_COLS: usize = 3;
const GRID_GAP: f32 = 12.0;
/// Fixed content width: 3x125 + 2x12 = 399. Every page (header, grid,
/// footer, settings, about) is constrained to it so the window can
/// never stretch and gaps stay exact.
const CONTENT_W: f32 = GRID_COLS as f32 * 125.0 + (GRID_COLS as f32 - 1.0) * GRID_GAP;
/// Type scale (single scale everywhere): title 17, body/labels 14-15,
/// footer warning 12.5. Version uses the same size as HELP.
const TITLE_SIZE: f32 = 17.0;
const VERSION_SIZE: f32 = 14.0;
const TILE_LABEL_SIZE: f32 = 14.0;
const FOOTER_SIZE: f32 = 12.5;
const BODY_SIZE: f32 = 14.0;
const SMALL_SIZE: f32 = 12.5;

/// Draw a vector icon centered at `c` with half-size `s`.
fn draw_icon(painter: &egui::Painter, c: egui::Pos2, s: f32, icon: TileIcon) {
    let col = ICON_COLOR;
    let w = egui::Stroke::new(2.5, col);
    let thin = egui::Stroke::new(2.0, col);
    match icon {
        TileIcon::Camera => {
            // Body.
            let body = egui::Rect::from_center_size(
                c + egui::Vec2::new(0.0, s * 0.12),
                egui::Vec2::new(s * 1.5, s * 0.95),
            );
            painter.rect_stroke(body, 4.0, w);
            // Viewfinder bump.
            let bump = egui::Rect::from_center_size(
                c + egui::Vec2::new(-s * 0.42, -s * 0.48),
                egui::Vec2::new(s * 0.4, s * 0.25),
            );
            painter.rect_stroke(bump, 2.0, thin);
            // Lens.
            painter.circle_stroke(c + egui::Vec2::new(0.0, s * 0.12), s * 0.3, w);
        }
        TileIcon::Scout => {
            // Magnifier glass.
            let g = c + egui::Vec2::new(-s * 0.18, -s * 0.18);
            painter.circle_stroke(g, s * 0.42, w);
            // Handle.
            painter.line_segment(
                [
                    g + egui::Vec2::new(s * 0.3, s * 0.3),
                    c + egui::Vec2::new(s * 0.62, s * 0.62),
                ],
                w,
            );
        }
        TileIcon::EyeOff => {
            // Iris.
            painter.circle_filled(c, s * 0.14, col);
            // Lash hints.
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.5, -s * 0.42),
                    c + egui::Vec2::new(s * 0.5, -s * 0.42),
                ],
                thin,
            );
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.5, s * 0.42),
                    c + egui::Vec2::new(s * 0.5, s * 0.42),
                ],
                thin,
            );
            // Slash.
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.62, s * 0.62),
                    c + egui::Vec2::new(s * 0.62, -s * 0.62),
                ],
                w,
            );
        }
        TileIcon::Tag => {
            // Diamond tag.
            let p0 = c + egui::Vec2::new(-s * 0.1, -s * 0.55);
            let p1 = c + egui::Vec2::new(s * 0.45, 0.0);
            let p2 = c + egui::Vec2::new(-s * 0.1, s * 0.55);
            let p3 = c + egui::Vec2::new(-s * 0.55, 0.0);
            for (a, b) in [(p0, p1), (p1, p2), (p2, p3), (p3, p0)] {
                painter.line_segment([a, b], w);
            }
            // Hole.
            painter.circle_stroke(c + egui::Vec2::new(-s * 0.14, 0.0), s * 0.09, thin);
        }
        TileIcon::Globe => {
            painter.circle_stroke(c, s * 0.5, w);
            // Equator + meridian.
            painter.line_segment(
                [
                    c + egui::Vec2::new(-s * 0.5, 0.0),
                    c + egui::Vec2::new(s * 0.5, 0.0),
                ],
                thin,
            );
            painter.line_segment(
                [
                    c + egui::Vec2::new(0.0, -s * 0.5),
                    c + egui::Vec2::new(0.0, s * 0.5),
                ],
                thin,
            );
        }
        TileIcon::Info => {
            painter.circle_stroke(c, s * 0.5, w);
            // Stem of the "i".
            painter.line_segment(
                [
                    c + egui::Vec2::new(0.0, -s * 0.05),
                    c + egui::Vec2::new(0.0, s * 0.35),
                ],
                w,
            );
            // Dot.
            painter.circle_filled(c + egui::Vec2::new(0.0, -s * 0.25), 2.5, col);
        }
    }
}

/// One clickable tile at an explicit `rect`: icon up top, label
/// underneath. Explicit placement (not layout flow) so gaps are exactly
/// `GRID_GAP` whatever the window does. Returns the raw response
/// (`click_and_drag`: a press-drag moves the window, a clean
/// press-release clicks).
fn tile_at(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    icon: TileIcon,
    label: &str,
    active: bool,
) -> egui::Response {
    let resp = ui
        .interact(rect, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(rect);

    let bg = if resp.hovered() {
        TILE_HOVER
    } else if active {
        TILE_ACTIVE_BG
    } else {
        TILE_BG
    };
    painter.rect_filled(rect, ROUNDING, bg);
    if active {
        // Outline INSIDE the fill (shrink by half the 2px stroke):
        // outer edge == fill edge, so the visible corner radius is
        // exactly 10px like every inactive tile. The old centered
        // stroke stuck 1px outside and read as mismatched corners.
        let inner = rect.shrink(1.0);
        painter.rect_stroke(inner, ROUNDING - 1.0, egui::Stroke::new(2.0, ACCENT));
    }

    // Icon up top, label zone below sized for two small lines.
    let icon_c = rect.center() + egui::Vec2::new(0.0, -16.0);
    draw_icon(&painter, icon_c, 18.0, icon);
    let label_rect = egui::Rect::from_min_max(
        egui::Pos2::new(rect.min.x + 4.0, rect.max.y - 44.0),
        egui::Pos2::new(rect.max.x - 4.0, rect.max.y - 4.0),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(label_rect), |ui| {
        ui.centered_and_justified(|ui| {
            // Hover-only: clicks fall through to the tile below, and
            // the text can never start a selection drag.
            ui.add(
                egui::Label::new(egui::RichText::new(label).size(TILE_LABEL_SIZE).color(
                    if active {
                        egui::Color32::WHITE
                    } else {
                        TEXT_DIM
                    },
                ))
                .wrap()
                .sense(egui::Sense::hover()),
            );
        });
    });
    resp
}

/// Red footer bar, fixed to the content width so it can never widen
/// the window: the warning always wraps inside the same 399px.
fn footer(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(CONTENT_W, 40.0), egui::Sense::hover());
    ui.painter_at(rect).rect_filled(rect, ROUNDING, FOOTER_BG);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect.shrink(6.0)), |ui| {
        ui.centered_and_justified(|ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(i18n::t("footer_locked"))
                        .size(FOOTER_SIZE)
                        .color(egui::Color32::WHITE),
                )
                .wrap()
                .sense(egui::Sense::hover()),
            );
        });
    });
}

/// Header row, fixed to the content width: version left, title center,
/// HELP right. Version and HELP share the same 14px size; the title
/// is 17px (was `heading`, ~2x bigger, hence the mismatch).
fn header(ui: &mut egui::Ui) {
    ui.allocate_ui_with_layout(
        egui::Vec2::new(CONTENT_W, 0.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(env!("CARGO_PKG_VERSION"))
                        .size(VERSION_SIZE)
                        .color(TEXT_DIM),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(i18n::t("help")).size(VERSION_SIZE),
                            )
                            .fill(TILE_BG)
                            .rounding(egui::Rounding::same(ROUNDING)),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        set_page(Page::About);
                    }
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new("CXSFM")
                                .size(TITLE_SIZE)
                                .strong()
                                .color(TEXT_DIM),
                        );
                    });
                });
            });
        },
    );
}

/// Main tile grid: one tile per mod, then Settings + About.
///
/// Fixed layout (not responsive by design): tiles are placed at
/// explicit `x = c * (125+12)` / `y = r * (110+12)` inside one
/// `399 x H` block. No `Grid`, no flow layout — nothing can justify
/// or stretch the gaps, and the block's fixed size is what locks the
/// window width.
fn page_main(ui: &mut egui::Ui) {
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
            label: i18n::t(t.label_key).to_string(),
            active: t.enabled,
            kind: 0,
        });
    }
    items.push(Item {
        icon: TileIcon::Globe,
        label: i18n::t("tile_settings").to_string(),
        active: false,
        kind: 1,
    });
    items.push(Item {
        icon: TileIcon::Info,
        label: i18n::t("tile_about").to_string(),
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

    let rows = items.len().div_ceil(GRID_COLS);
    let grid_h = rows as f32 * TILE_SIZE.y + (rows as f32 - 1.0).max(0.0) * GRID_GAP;
    let (block, _) =
        ui.allocate_exact_size(egui::Vec2::new(CONTENT_W, grid_h), egui::Sense::hover());
    for (idx, item) in items.iter().enumerate() {
        let r = idx / GRID_COLS;
        let c = idx % GRID_COLS;
        let min = block.min
            + egui::Vec2::new(
                c as f32 * (TILE_SIZE.x + GRID_GAP),
                r as f32 * (TILE_SIZE.y + GRID_GAP),
            );
        let rect = egui::Rect::from_min_size(min, TILE_SIZE);
        let id = ui.make_persistent_id(format!("cxsfm_tile_{idx}"));
        let resp = tile_at(ui, rect, id, item.icon, &item.label, item.active);
        // Drag from anywhere: a drag started on a tile moves the
        // window; a clean click activates it.
        if resp.dragged() {
            shift_window(resp.drag_delta());
        }
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
    if go_settings {
        set_page(Page::Settings);
    }
    if dirty {
        ui.ctx().request_repaint();
    }
}

/// Settings page: language pin + keybind reminder. Constrained to the
/// content width with wrapping labels so long languages never widen
/// the window.
fn page_settings(ui: &mut egui::Ui) {
    ui.allocate_ui_with_layout(
        egui::Vec2::new(CONTENT_W, 0.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(i18n::t("back")).size(VERSION_SIZE))
                        .fill(TILE_BG)
                        .rounding(egui::Rounding::same(ROUNDING)),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                set_page(Page::Main);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(i18n::t("settings_language")).size(BODY_SIZE));
                let current = i18n::override_lang();
                let shown = match current {
                    Some(l) => format!("{} ({})", l.name(), l.code()),
                    None => i18n::t("settings_auto").to_string(),
                };
                egui::ComboBox::from_id_salt("cxsfm_lang")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(current.is_none(), i18n::t("settings_auto"))
                            .clicked()
                        {
                            i18n::set_override(None);
                        }
                        for l in i18n::Lang::ALL {
                            if ui
                                .selectable_label(
                                    current == Some(*l),
                                    format!("{} ({})", l.name(), l.code()),
                                )
                                .clicked()
                            {
                                i18n::set_override(Some(*l));
                            }
                        }
                    });
            });
            let detected = match i18n::detected() {
                Some(l) => format!("{} ({})", l.name(), l.code()),
                None => "—".to_string(),
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{}: {}", i18n::t("settings_detected"), detected))
                        .size(SMALL_SIZE),
                )
                .wrap()
                .sense(egui::Sense::hover()),
            );
            ui.separator();
            ui.label(egui::RichText::new(i18n::t("settings_keys")).size(BODY_SIZE));
            for key in ["keys_f8", "keys_note"] {
                ui.add(
                    egui::Label::new(egui::RichText::new(i18n::t(key)).size(SMALL_SIZE))
                        .wrap()
                        .sense(egui::Sense::hover()),
                );
            }
        },
    );
}

/// About page: version, stats, HELP content. Same width lock + wrap.
fn page_about(ui: &mut egui::Ui) {
    ui.allocate_ui_with_layout(
        egui::Vec2::new(CONTENT_W, 0.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(i18n::t("back")).size(VERSION_SIZE))
                        .fill(TILE_BG)
                        .rounding(egui::Rounding::same(ROUNDING)),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
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
            ui.label(egui::RichText::new(i18n::t("settings_keys")).size(BODY_SIZE));
            for key in ["keys_f8", "keys_note"] {
                ui.add(
                    egui::Label::new(egui::RichText::new(i18n::t(key)).size(SMALL_SIZE))
                        .wrap()
                        .sense(egui::Sense::hover()),
                );
            }
        },
    );
}

/// Draw the whole menu (called from the render hook when visible).
pub fn draw_menu_ui(ctx: &egui::Context) {
    if !crate::hotkey::ui_visible() {
        return;
    }
    // Dark menu palette, scoped to this window. Text is never
    // selectable (labels would otherwise eat drags/clicks and start
    // selection marquees over the tiles).
    let mut style = (*ctx.style()).clone();
    style.visuals.window_fill = WIN_BG;
    style.visuals.window_rounding = ROUNDING.into();
    style.interaction.selectable_labels = false;
    ctx.set_style(style);

    // Fixed width (no resize by design), height follows the tallest
    // page: CONTENT_W (399) + frame/margins. Every child is locked to
    // CONTENT_W so nothing can stretch the window. Rounding 10px.
    let mut win = egui::Window::new("cxsfm_menu")
        .title_bar(false)
        .resizable(false)
        .min_size(egui::Vec2::new(CONTENT_W + 32.0, 0.0))
        .max_size(egui::Vec2::new(CONTENT_W + 32.0, 2000.0))
        .collapsible(false)
        .default_pos(egui::pos2(160.0, 120.0));
    // Pinned position after the first drag (bind-then-use: copy
    // out of the lock, never hold it while showing the window).
    let pinned = *WIN_POS.lock().unwrap();
    if let Some(p) = pinned {
        win = win.current_pos(p);
    }
    let inner = win.show(ctx, |ui| {
        // Background drag handle FIRST (beneath every widget, so
        // tiles/buttons keep their clicks): dragging from ANY empty
        // spot — header, gaps, footer margins — moves the window.
        // Drags started ON tiles are forwarded in `page_main`, so
        // together the menu drags from anywhere.
        let bg_rect = ui.available_rect_before_wrap();
        let bg = ui
            .interact(
                bg_rect,
                ui.make_persistent_id("cxsfm_bg_drag"),
                egui::Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Grab);
        if bg.dragged() {
            let delta = bg.drag_delta();
            // Anchor without a first-frame jump: prefer the pinned
            // pos, else the last measured rect, else cursor minus
            // press offset.
            let mut pos = WIN_POS.lock().unwrap();
            let anchor = (*pos).or_else(|| WIN_RECT.lock().unwrap().map(|r| r.min));
            let cur = anchor.or_else(|| bg.interact_pointer_pos().map(|p| p - delta));
            if let Some(cur) = cur {
                *pos = Some(cur + delta);
            }
        }
        header(ui);
        ui.separator();
        match page() {
            Page::Main => page_main(ui),
            Page::Settings => page_settings(ui),
            Page::About => page_about(ui),
        }
        ui.separator();
        footer(ui);
    });
    // Measure for the next frame's drag anchor + full-window drag
    // fallback (point 4): the pre-content `bg` handle covers header /
    // gaps / footer margins, tiles forward their own drags, and the
    // window response below covers any remaining background — even on
    // tall windows where the pre-wrap rect no longer spans the frame.
    // Child widgets keep priority (they consume their own press), so
    // clicks on tiles/buttons still work.
    if let Some(r) = inner {
        if r.response.dragged() {
            shift_window(r.response.drag_delta());
        }
        *WIN_RECT.lock().unwrap() = Some(r.response.rect);
    }
}
