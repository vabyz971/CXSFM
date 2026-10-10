//! Menu theme: CarX Street visual language.
//!
//! Game DNA (from in-game captures): near-black panels, sharp corners,
//! white condensed titles, CarX yellow (#F5C518-ish) for highlights,
//! active states and key buttons, dim gray secondary text, key-hint
//! chips (white key box on black). All tokens funnel here — widgets
//! never hardcode a color or radius.
//!
//! The grid is responsive: columns derive from the available width
//! (see `layout`), tiles shrink past 8 mods, gaps stay exactly 12px.

/// Window background (game panel charcoal).
pub const WIN_BG: egui::Color32 = egui::Color32::from_rgb(0x16, 0x16, 0x18);
/// Tile / nav-button background.
pub const TILE_BG: egui::Color32 = egui::Color32::from_rgb(0x23, 0x23, 0x27);
/// Tile hover background.
pub const TILE_HOVER: egui::Color32 = egui::Color32::from_rgb(0x30, 0x30, 0x36);
/// Active (enabled mod) tile background.
pub const TILE_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x26, 0x14);
/// Active outline accent: CarX yellow (UPGRADE buttons, level badge).
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0xF5, 0xC5, 0x18);
/// Icon strokes.
pub const ICON_COLOR: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xE8, 0xEA);
/// Secondary text.
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x9A, 0xA2);
/// Footer bar background (dark like the game, not red).
pub const FOOTER_BG: egui::Color32 = egui::Color32::from_rgb(0x1C, 0x1C, 0x1F);
/// Footer top edge: thin yellow rule like game section dividers.
pub const FOOTER_EDGE: egui::Color32 = ACCENT;
/// Key-hint chip: white key box on black (ESCAPE/BACK style).
pub const KEY_BG: egui::Color32 = egui::Color32::WHITE;
pub const KEY_FG: egui::Color32 = egui::Color32::BLACK;

/// One tile: `125x110`.
pub const TILE: egui::Vec2 = egui::Vec2::new(125.0, 110.0);
/// Compact tile past 8 mods: `96x84` (same proportions, tighter text).
pub const TILE_COMPACT: egui::Vec2 = egui::Vec2::new(96.0, 84.0);
/// Sharp game corners: 3px everywhere (tiles, window, footer, outline).
pub const ROUNDING: f32 = 3.0;
/// Nav buttons (HELP / BACK) match the tiles: same sharp radius.
pub const NAV_ROUNDING: f32 = 3.0;
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
