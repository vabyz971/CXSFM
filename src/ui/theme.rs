//! Menu theme: palette, tile sizes and type scale.
//!
//! The grid is responsive: columns derive from the available width
//! (see `layout`), tiles shrink past 8 mods, gaps stay exactly 12px.

/// Window background.
pub const WIN_BG: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x3E);
/// Tile / nav-button background.
pub const TILE_BG: egui::Color32 = egui::Color32::from_rgb(0x2B, 0x2B, 0x2F);
/// Tile hover background.
pub const TILE_HOVER: egui::Color32 = egui::Color32::from_rgb(0x3F, 0x3F, 0x46);
/// Active (enabled mod) tile background.
pub const TILE_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(0x33, 0x42, 0x38);
/// Active outline accent.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x7B, 0xC7, 0x8A);
/// Icon strokes.
pub const ICON_COLOR: egui::Color32 = egui::Color32::from_rgb(0xCC, 0xCC, 0xD2);
/// Secondary text.
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0xAA, 0xAA, 0xB2);
/// Footer warning background.
pub const FOOTER_BG: egui::Color32 = egui::Color32::from_rgb(0xA8, 0x1C, 0x2C);

/// One tile: `125x110`.
pub const TILE: egui::Vec2 = egui::Vec2::new(125.0, 110.0);
/// Compact tile past 8 mods: `96x84` (same proportions, tighter text).
pub const TILE_COMPACT: egui::Vec2 = egui::Vec2::new(96.0, 84.0);
/// Rounded corners: 10px everywhere (tiles, window, footer, outline).
pub const ROUNDING: f32 = 10.0;
/// Nav buttons (HELP / BACK) are much shorter than tiles: same 10px
/// reads as a pill, so they use a tighter radius.
pub const NAV_ROUNDING: f32 = 6.0;
/// Grid cap: 4 columns per row, extras wrap below.
pub const MAX_COLS: usize = 4;
/// Past this many registered mods the tiles switch to compact.
pub const MODS_BEFORE_COMPACT: usize = 8;
/// Exact gap between tiles (never justified, never stretched).
pub const GRID_GAP: f32 = 12.0;

/// Title (`CXSFM`) size.
pub const TITLE_SIZE: f32 = 17.0;
/// Version + HELP/BACK buttons size (all three match).
pub const VERSION_SIZE: f32 = 14.0;
/// Tile label size.
pub const TILE_LABEL: f32 = 14.0;
/// Compact tile label size.
pub const TILE_LABEL_COMPACT: f32 = 12.0;
/// Footer warning size.
pub const FOOTER_SIZE: f32 = 12.5;
/// Section titles (settings/about).
pub const BODY_SIZE: f32 = 14.0;
/// Secondary lines (settings/about).
pub const SMALL_SIZE: f32 = 12.5;
