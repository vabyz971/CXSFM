//! Responsive grid layout: columns from available width, compact
//! tiles past 8 mods.
//!
//! Rules (validated in game):
//!
//! - Up to 4 columns (`MAX_COLS`); extra tiles wrap to the next row.
//! - Past 8 registered mods the tiles shrink (`TILE_COMPACT`) so the
//!   whole grid stays glanceable instead of growing a third tall row.
//! - The block width is derived, never guessed: the window hugs it and
//!   callers center it, so margins stay symmetric at any size.

use super::theme::{GRID_GAP, MAX_COLS, MODS_BEFORE_COMPACT, TILE, TILE_COMPACT};

/// One computed grid arrangement.
#[derive(Clone, Copy)]
pub struct GridLayout {
    /// Columns for this width (1..=MAX_COLS).
    pub cols: usize,
    /// Tile size (normal or compact).
    pub tile: egui::Vec2,
    /// Label size matching the tile.
    pub label: f32,
    /// Exact block width: `cols * tile + (cols - 1) * gap`.
    pub block_w: f32,
}

/// Compute the grid for `avail_w` of free width and `mod_count`
/// registered mods (+2 implicit Settings/About tiles for the height).
pub fn grid_layout(avail_w: f32, mod_count: usize) -> GridLayout {
    let compact = mod_count > MODS_BEFORE_COMPACT;
    let tile = if compact { TILE_COMPACT } else { TILE };
    let label = if compact {
        super::theme::TILE_LABEL_COMPACT
    } else {
        super::theme::TILE_LABEL
    };
    let per = tile.x + GRID_GAP;
    let cols = ((avail_w + GRID_GAP) / per).floor() as usize;
    let cols = cols.clamp(1, MAX_COLS);
    GridLayout {
        cols,
        tile,
        label,
        block_w: cols as f32 * tile.x + (cols as f32 - 1.0).max(0.0) * GRID_GAP,
    }
}
