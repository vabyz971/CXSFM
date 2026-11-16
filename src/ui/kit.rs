//! Tool-window kit: the shared widgets of every windowed mod
//! (inspector, game log, speedometer, minimap, custom HUD…).
//!
//! The window itself comes from [`crate::mods::tool::ToolChrome::show`];
//! everything drawn inside it goes through these helpers so one edit
//! here restyles every mod. Full-screen settings screens use
//! [`super::game`] instead (rows with pad navigation).
//!
//! Colors come from [`super::theme`] only. Layout rules learned the
//! hard way (see the speedometer layout tests): fixed widths, never
//! `available_width()` inside a measuring pass, long text truncated or
//! wrapped — a row must never widen its window.

use super::theme::{STATE_ERR, STATE_OK, STATE_WARN, TEXT_BRIGHT, TEXT_DIM};
use egui::{Response, RichText, Ui};

/// Titled group box (`GameObject`, `Transform`…).
pub fn card<R>(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .show(ui, |ui| {
            ui.monospace(title);
            body(ui)
        })
        .inner
}

/// One labelled row: dim label left, the widget(s) from `body` after it.
pub fn field<R>(ui: &mut Ui, label: &str, body: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(TEXT_DIM));
        body(ui)
    })
    .inner
}

/// Read-only `label  value` row (monospace value).
pub fn kv(ui: &mut Ui, label: &str, value: &str) {
    field(ui, label, |ui| {
        ui.monospace(value);
    });
}

/// `name.X / name.Y / name.Z` rows with fixed-width drag fields.
/// Vertical on purpose: horizontal triples overflowed narrow panes.
/// Returns true when a value moved.
pub fn xyz(ui: &mut Ui, name: &str, vals: &mut [f32; 3]) -> bool {
    let mut edited = false;
    for (v, tag) in vals.iter_mut().zip(["X", "Y", "Z"]) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{name}.{tag}")).color(TEXT_DIM).monospace());
            let h = ui.spacing().interact_size.y;
            edited |= ui
                .add_sized([96.0, h], egui::DragValue::new(v).speed(0.01).max_decimals(3))
                .changed();
        });
    }
    edited
}

/// Percent slider for a `0..=1` value (fixed width). True on change.
pub fn percent(ui: &mut Ui, v: &mut f32) -> bool {
    let mut pct = *v * 100.0;
    let h = ui.spacing().interact_size.y;
    let changed = ui
        .add_sized([160.0, h], egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%"))
        .changed();
    if changed {
        *v = (pct / 100.0).clamp(0.0, 1.0);
    }
    changed
}

/// Secondary line (placeholders, counts, "nothing yet").
pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(TEXT_DIM));
}

/// Small wrapped explanation line.
pub fn note(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small()).wrap());
}

/// Small wrapped warning (orange).
pub fn warn(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().color(STATE_WARN)).wrap());
}

/// Wrapped error line (red).
pub fn error(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(STATE_ERR)).wrap());
}

/// Big bright title (selected object name).
pub fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).strong().size(16.0).color(TEXT_BRIGHT));
}

/// `● on` (green) / `● off` (red) state line.
pub fn status(ui: &mut Ui, on: bool, on_text: &str, off_text: &str) {
    let (t, c) = if on { (on_text, STATE_OK) } else { (off_text, STATE_ERR) };
    ui.label(RichText::new(format!("● {t}")).color(c));
}

/// Dim text pushed to the right end of the current row.
pub fn right_hint(ui: &mut Ui, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| hint(ui, text));
}

/// Search box: icon + full-width text field.
pub fn search(ui: &mut Ui, text: &mut String, placeholder: &str) -> Response {
    ui.label(egui_material_icons::icons::ICON_SEARCH.rich_text());
    ui.add(egui::TextEdit::singleline(text).hint_text(placeholder).desired_width(f32::INFINITY))
}

/// Tab strip; returns true when the tab changed.
pub fn tabs(ui: &mut Ui, cur: &mut usize, labels: &[&str]) -> bool {
    let old = *cur;
    ui.horizontal(|ui| {
        for (i, l) in labels.iter().enumerate() {
            ui.selectable_value(cur, i, *l);
        }
    });
    ui.separator();
    *cur != old
}

/// Collapsible group header in a tree (`Section (3)`): chevron + label,
/// both toggle. Returns true when clicked.
pub fn tree_group(ui: &mut Ui, open: bool, label: &str) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::new(4.0, 0.0);
        let a = super::chrome::expander(ui, open).clicked();
        let b = ui.selectable_label(false, RichText::new(label).color(TEXT_BRIGHT)).clicked();
        a || b
    })
    .inner
}

/// Selectable tree row: bright when active, dimmed + struck through
/// with an orange `OFF` badge when not. Call inside a row (after an
/// optional expander) or alone. Returns the label's response.
pub fn tree_item(ui: &mut Ui, selected: bool, label: &str, active: bool) -> Response {
    let text = if active {
        RichText::new(label).color(TEXT_BRIGHT)
    } else {
        RichText::new(label).color(TEXT_DIM).strikethrough()
    };
    let r = ui.selectable_label(selected, text);
    if !active {
        ui.label(RichText::new("OFF").small().strong().color(STATE_WARN));
    }
    r
}

/// Which column [`two_panes`] is drawing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Left,
    Right,
}

/// Two fixed columns, each with its own vertical scroll: a list on the
/// left (`left_w` wide), details on the right. `body` runs once per
/// column (one closure, so both sides can borrow the same mod state).
/// Explicit top-down columns: a ScrollArea inside a bare horizontal
/// row flows sideways and blew windows to 3000px.
pub fn two_panes(ui: &mut Ui, id: &str, left_w: f32, mut body: impl FnMut(&mut Ui, Pane)) {
    let h = ui.available_height().max(200.0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::Vec2::new(left_w, h),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt((id, "left"))
                    .show(ui, |ui| body(ui, Pane::Left));
            },
        );
        ui.separator();
        let w = (ui.available_width() - 8.0).max(300.0);
        ui.allocate_ui_with_layout(
            egui::Vec2::new(w, h),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                egui::ScrollArea::vertical().id_salt((id, "right")).show(ui, |ui| {
                    ui.set_min_width(320.0);
                    body(ui, Pane::Right);
                });
            },
        );
    });
}

/// Fixed-height scrolling list (fills the width).
pub fn list(ui: &mut Ui, id: &str, height: f32, body: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .max_height(height)
        .auto_shrink([false, false])
        .show(ui, body);
}

/// Cap `s` to `n` chars, `…` when cut (rows never wrap).
pub fn short(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::short;

    #[test]
    fn short_cuts_on_char_boundaries() {
        assert_eq!(short("abc", 5), "abc");
        assert_eq!(short("abcdef", 3), "abc…");
        assert_eq!(short("éèàù", 2), "éè…");
        assert_eq!(short("", 0), "");
    }
}
