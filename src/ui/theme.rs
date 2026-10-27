//! Menu theme: CarX Street visual language.
//!
//! Game DNA (from in-game captures): near-black panels, sharp corners,
//! white condensed titles, CarX yellow (#F5C518-ish) for highlights,
//! active states and key buttons, dim gray secondary text, key-hint
//! chips (white key box on black). All tokens funnel here — widgets
//! never hardcode a color or radius.
//!
//! The main menu is the phone (`phone`), which owns its own geometry.

/// Window background (game panel charcoal).
pub const WIN_BG: egui::Color32 = egui::Color32::from_rgb(0x16, 0x16, 0x18);
/// Active outline accent: CarX yellow (UPGRADE buttons, level badge).
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0xF5, 0xC5, 0x18);
/// Icon strokes.
pub const ICON_COLOR: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xE8, 0xEA);
/// Secondary text.
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x9A, 0xA2);
/// Bright text (active nodes, titles on dark).
pub const TEXT_BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xEC, 0xEC, 0xF0);
/// State colors (inspector dots, GameLog lines): readable on dark.
pub const STATE_OK: egui::Color32 = egui::Color32::from_rgb(0x8F, 0xD0, 0x8A);
pub const STATE_WARN: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xA0, 0x40);
pub const STATE_ERR: egui::Color32 = egui::Color32::from_rgb(0xE0, 0x70, 0x60);
/// Default log line text.
pub const LOG_TEXT: egui::Color32 = egui::Color32::from_rgb(0xDC, 0xDC, 0xE2);
/// Key-hint chip: white key box on black (ESCAPE/BACK style).
pub const KEY_BG: egui::Color32 = egui::Color32::WHITE;
pub const KEY_FG: egui::Color32 = egui::Color32::BLACK;

/// Sharp game corners: 3px everywhere (tiles, window, footer, outline).
pub const ROUNDING: f32 = 3.0;
/// Section titles (settings/about).
pub const BODY_SIZE: f32 = 14.0;
/// Secondary lines (settings/about).
pub const SMALL_SIZE: f32 = 12.5;
