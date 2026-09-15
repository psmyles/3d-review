//! Panel form rows: a label in one column, a stock egui control in the other.
//!
//! Numeric value boxes render in monospace and everything else proportional,
//! which is the one styling exception the chrome keeps.

use crate::theme::{color, size};
use crate::widgets::{Tip, tip};

/// Wrap a panel's label+control rows in a stock striped two-column
/// `egui::Grid` — the same primitive (and default styling) egui's demo widget
/// gallery uses: alternating faint row backgrounds (egui's stock
/// `faint_bg_color`), a label column auto-sized to its widest entry, the control
/// column filling the rest, all at egui's default grid spacing, row height and
/// font. The row helpers below (`labeled_slider_with_value`,
/// `labeled_color_button`, `labeled_checkbox`, `labeled_color32`,
/// `labeled_combo`, `value_row`) each emit exactly one grid row, so a panel body
/// just calls them inside this closure. Full-width controls (the reset button)
/// go *after* the grid, not inside it.
pub(crate) fn panel_grid(ui: &mut egui::Ui, salt: &str, contents: impl FnOnce(&mut egui::Ui)) {
    let row_gap = ui.spacing().item_spacing.y;
    egui::Grid::new(("panel_grid", salt))
        .num_columns(2)
        .min_col_width(size::PANEL_LABEL_COL_WIDTH)
        .spacing(egui::vec2(size::PANEL_GRID_COL_GAP, row_gap))
        .striped(true)
        .show(ui, contents);
}

/// The left (label) cell of a panel-grid row: a plain default-styled label pinned
/// to the fixed [`size::PANEL_LABEL_COL_WIDTH`] (left-aligned; an over-long label
/// truncates rather than widening the column, so every panel keeps the same
/// label/control split), carrying the row's explanation and a link to the manual.
/// Leaves the cursor in the control column for the caller to drop the row's
/// widget and then call `ui.end_row()`.
///
/// The label *is* the tooltip's title, taken from the same [`Tip`], so the two
/// cannot drift apart the way two separate arguments would — and every row is
/// obliged to say what it is for, rather than only what it is called.
pub(crate) fn grid_label_tip(ui: &mut egui::Ui, row: Tip) {
    let response = ui
        .scope(|ui| {
            ui.set_width(size::PANEL_LABEL_COL_WIDTH);
            ui.add(egui::Label::new(row.title()).truncate())
        })
        .inner;
    tip(response, row);
}

/// The right (control) cell of a panel-grid row: a cell spanning **all the
/// remaining width** of the row (the window minus the fixed label column) whose
/// contents are **right-aligned**, so the control's right edge sits flush with
/// the panel's right edge — no matter how wide egui sizes the window — and any
/// slack in a row (a short swatch run, a narrow value) falls *between* the label
/// and the control. Controls that already fill the column (slider rail, combo)
/// span it edge to edge regardless.
pub(crate) fn grid_control<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        add,
    )
    .inner
}

/// A dropdown sized to one control row: the closed button matches `PANEL_ROW_H`
/// (so combo rows are the same height as slider / text-field rows), and the
/// opened popup uses short option rows with the active option distinguished by a
/// brighter text color instead of a low-contrast highlight fill.
pub(crate) fn compact_combo(
    ui: &mut egui::Ui,
    id_salt: &str,
    control_w: f32,
    selected_text: impl Into<egui::WidgetText>,
    contents: impl FnOnce(&mut egui::Ui),
) {
    // Shrink the closed button to row height.
    ui.spacing_mut().interact_size.y = size::PANEL_ROW_H;
    ui.spacing_mut().button_padding.y = size::PANEL_COMBO_BUTTON_PAD_Y;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .width(control_w)
        .height(size::PANEL_COMBO_POPUP_MAX_H)
        .show_ui(ui, |ui| {
            style_combo_popup(ui);
            contents(ui);
        });
}

/// Applies the shared combo-popup look: compact option rows, the blue selection
/// fill dropped (zero-width stroke paints no border) with the brighter color
/// carried through as the selected text color, and the unselected options
/// dimmed so the active one stands out by contrast alone. Call this at the top
/// of any `ComboBox::show_ui` closure so toolbar and panel dropdowns read the
/// same.
pub(crate) fn style_combo_popup(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = size::PANEL_COMBO_OPTION_GAP;
    ui.spacing_mut().interact_size.y = size::PANEL_COMBO_OPTION_H;
    ui.spacing_mut().button_padding.y = size::PANEL_COMBO_OPTION_PAD_Y;
    ui.visuals_mut().selection.bg_fill = egui::Color32::TRANSPARENT;
    ui.visuals_mut().selection.stroke = egui::Stroke::new(0.0_f32, color::TEXT_COMBO_SELECTED);
    ui.visuals_mut().widgets.inactive.fg_stroke.color = color::TEXT_COMBO_DIM;
}

/// A label + slider grid row, styled like the egui demo's slider: a stock
/// [`egui::Slider`] with its built-in inline value (drag-to-scrub,
/// double-click-to-type), at egui's default rail width and row height. `decimals`
/// caps the value's displayed precision (use 0 for an integer readout).
///
/// This is the standard slider for every option panel — emit it inside a
/// [`panel_grid`] closure. Returns `true` when the value changed this frame, so a
/// caller that emits an edit intent (e.g. the Inspector) can react; panels that
/// write straight into their own state can ignore it.
pub(crate) fn labeled_slider_with_value<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: Tip,
    value: &mut Num,
    range: std::ops::RangeInclusive<Num>,
    decimals: usize,
) -> bool {
    grid_label_tip(ui, label);
    // The slider's own inline readout auto-sizes to its digit count, so it
    // reflows as the number changes. Instead, hide it (`show_value(false)`) and
    // append a separate **fixed-width** `DragValue` box: the rail fills the row
    // up to a constant value-box width on the right, which never moves.
    let changed = ui
        .scope(|ui| {
            let value_w = size::PANEL_SLIDER_VALUE_W;
            let gap = ui.spacing().item_spacing.x;
            ui.spacing_mut().slider_width = (ui.available_width() - value_w - gap).max(0.0);
            let mut response = ui.add(
                egui::Slider::new(value, range.clone())
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            let box_height = ui.spacing().interact_size.y;
            response |= value_box(ui, egui::vec2(value_w, box_height), |ui| {
                ui.add(
                    egui::DragValue::new(value)
                        .range(range)
                        .max_decimals(decimals),
                )
            });
            response.changed()
        })
        .inner;
    ui.end_row();
    changed
}

/// Draw a value box in a cell of exactly `size` that its contents can never
/// widen: the cell's space is allocated up front and the box goes into a child
/// `Ui` built over that rect and **clipped to it**, so a box that measures wider
/// than the cell is cut off at the cell's edge instead of reporting its width
/// back to the row.
///
/// The cell's layout is justified (the box fills the cell, so its width never
/// reflows with the digit count) but left-aligned (`main_align = Min`) — egui's
/// `DragValue` positions its text via the current layout, so this left-aligns
/// both the displayed value and the edit-mode caret. (`ui.add_sized` would centre
/// it, and would not bound the width.)
///
/// Bounding it is the point: an `egui::DragValue` has no width of its own. Idle
/// it is a `Button` with `TextWrapMode::Extend`; focused it is a `TextEdit` with
/// `clip_text(false)`. Both size themselves to their text, and a plain
/// `allocate_ui_with_layout` cell does not stop them — it advances its parent by
/// the *child's* `min_rect`, so an overflowing box widened the grid's control
/// column, and with it the auto-sized option window and the Inspector panel:
/// typing a long number grew the whole window around it, and (`egui::Resize`
/// never shrinking back) left it grown.
fn value_box<R>(ui: &mut egui::Ui, size: egui::Vec2, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let clip = ui.clip_rect();
    let (_id, rect) = ui.allocate_space(size);
    let layout = egui::Layout::centered_and_justified(egui::Direction::LeftToRight)
        .with_main_align(egui::Align::Min);
    let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
    cell.set_clip_rect(rect.intersect(clip));
    add(&mut cell)
}

/// A label + color-button grid row (the stock egui color-edit button). Returns the
/// button's [`egui::Response`] so the caller can react to an edit.
pub(crate) fn labeled_color_button(
    ui: &mut egui::Ui,
    label: Tip,
    rgb: &mut [f32; 3],
) -> egui::Response {
    grid_label_tip(ui, label);
    let response = grid_control(ui, |ui| ui.color_edit_button_rgb(rgb));
    ui.end_row();
    response
}

/// A label + checkbox grid row. The checkbox carries no inline text — the label
/// cell is its label.
pub(crate) fn labeled_checkbox(ui: &mut egui::Ui, label: Tip, value: &mut bool) {
    grid_label_tip(ui, label);
    grid_control(ui, |ui| ui.checkbox(value, ""));
    ui.end_row();
}

/// A label + dropdown grid row: a stock [`egui::ComboBox`] with egui's default
/// styling (the toolbar's compact-combo styling is *not* applied here).
pub(crate) fn labeled_combo(
    ui: &mut egui::Ui,
    label: Tip,
    id_salt: &str,
    selected_text: impl Into<egui::WidgetText>,
    contents: impl FnOnce(&mut egui::Ui),
) {
    grid_label_tip(ui, label);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .width(ui.available_width())
        .height(size::LABELED_COMBO_POPUP_MAX_H)
        .show_ui(ui, contents);
    ui.end_row();
}

#[cfg(test)]
mod tests {
    //! The value cell is a hard boundary. Every numeric readout in the chrome —
    //! every option panel's, every Inspector row's — sits in one, and a row that
    //! can be widened by what is typed into it widens the auto-sized window
    //! around it too.

    use super::*;

    /// One layout pass with the chrome's real fonts and style installed: the
    /// measurement is only as good as the face it is taken in, and egui's own
    /// test helper loads no fonts at all (every string would measure zero).
    fn run(add: impl FnOnce(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        crate::theme::init_style(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut add = Some(add);
        let output = ctx.run_ui(input, |ui| add.take().expect("one pass")(ui));
        // No renderer here, so the font-atlas deltas are deliberately dropped.
        output.drop_without_applying_deltas();
    }

    /// A value far too long for the box — what a user typing digits into it
    /// produces before the range clamps them back on commit.
    #[test]
    fn a_long_value_cannot_widen_the_row_it_sits_in() {
        run(|ui| {
            let cell = egui::vec2(size::PANEL_SLIDER_VALUE_W, ui.spacing().interact_size.y);
            let mut value = 16_000_000_000_000.0_f64;
            let response = value_box(ui, cell, |ui| {
                ui.add(
                    egui::DragValue::new(&mut value)
                        .range(0.0..=1e15)
                        .max_decimals(0),
                )
            });

            // The box really did measure wider than its cell: without this the
            // assertion below would pass on a value that simply fit, and stop
            // saying anything.
            assert!(
                response.rect.width() > cell.x,
                "the test value fits the box ({:.0}pt in a {:.0}pt cell), so it \
                 no longer exercises an overflow",
                response.rect.width(),
                cell.x,
            );
            assert!(
                ui.min_rect().width() <= cell.x + 0.5,
                "the value box widened its row to {:.0}pt from the {:.0}pt cell \
                 it was given - an option window auto-sizes to this, and \
                 `egui::Resize` never gives the width back",
                ui.min_rect().width(),
                cell.x,
            );
        });
    }
}
