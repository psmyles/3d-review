//! Reusable, theme-driven UI primitives shared by the toolbar, option panels and
//! stats overlay. None of these own state — they paint and report interactions.

use crate::assets::{self, AppIcon};
use crate::theme::{self, color, size};

/// A square icon toggle sized to the standard toolbar tile.
pub(crate) fn icon_toggle_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon: &AppIcon,
    selected: bool,
    tooltip: &str,
) -> egui::Response {
    icon_tile_button(
        ui,
        ctx,
        icon,
        selected,
        tooltip,
        egui::vec2(
            theme::px(ctx, size::TOOLBAR_ICON_SIZE),
            theme::px(ctx, size::TOOLBAR_ICON_SIZE),
        ),
    )
}

/// A custom-painted icon tile button: recessed when idle, accent-filled when
/// selected, lifted on hover. The icon is tinted brighter when selected.
pub(crate) fn icon_tile_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon: &AppIcon,
    selected: bool,
    tooltip: &str,
    tile_size: egui::Vec2,
) -> egui::Response {
    let tint = if selected {
        color::TEXT_PRIMARY
    } else {
        color::ICON_IDLE
    };
    let (rect, response) = ui.allocate_exact_size(tile_size, egui::Sense::click());
    let fill = if selected {
        color::ACCENT
    } else if response.hovered() {
        color::HOVER_BG
    } else {
        color::GROUP_BG
    };

    ui.painter().rect(
        rect,
        theme::px(ctx, size::TILE_CORNER_RADIUS),
        fill,
        egui::Stroke::new(size::HAIRLINE, egui::Color32::TRANSPARENT),
        egui::StrokeKind::Inside,
    );

    if let Some(texture) = assets::load_icon_texture(ui, icon) {
        let image_padding = theme::px(ctx, size::TOOLBAR_ICON_PADDING);
        let image_rect = rect.shrink2(egui::vec2(image_padding, image_padding));
        egui::Image::from_texture(texture)
            .fit_to_exact_size(image_rect.size())
            .tint(tint)
            .paint_at(ui, image_rect);
    }

    response.on_hover_text(tooltip)
}

/// A recessed group shell that hosts a row of toolbar tiles, laid out
/// left-to-right with the standard inter-icon gap.
pub(crate) fn toolbar_group_shell(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let group_height = theme::px(ctx, size::TOOLBAR_GROUP_HEIGHT);
    let desired = egui::vec2(width, group_height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::hover());
    ui.painter().rect(
        rect,
        theme::px(ctx, size::TILE_CORNER_RADIUS),
        color::GROUP_BG,
        egui::Stroke::NONE,
        egui::StrokeKind::Outside,
    );
    let inner_padding = theme::px(ctx, size::TOOLBAR_GROUP_PADDING);
    let inner = rect.shrink2(egui::vec2(inner_padding, inner_padding));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::px(ctx, size::TOOLBAR_ICON_GAP);
            add_contents(ui);
        },
    );
    response
}

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
    egui::Grid::new(("panel_grid", salt))
        .num_columns(2)
        .striped(true)
        .show(ui, contents);
}

/// The left (label) cell of a panel-grid row: a plain default-styled label.
/// Leaves the cursor in the control column for the caller to drop the row's
/// widget and then call `ui.end_row()`.
fn grid_label(ui: &mut egui::Ui, label: &str) {
    ui.label(label);
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
    ui.visuals_mut().selection.stroke = egui::Stroke::new(0.0, color::TEXT_COMBO_SELECTED);
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
    label: &str,
    value: &mut Num,
    range: std::ops::RangeInclusive<Num>,
    decimals: usize,
) -> bool {
    grid_label(ui, label);
    let response = ui.add(
        egui::Slider::new(value, range)
            .max_decimals(decimals)
            .clamping(egui::SliderClamping::Always),
    );
    ui.end_row();
    response.changed()
}

/// A label + color-button grid row (the stock egui color-edit button). Returns the
/// button's [`egui::Response`] so the caller can react to an edit.
pub(crate) fn labeled_color_button(
    ui: &mut egui::Ui,
    label: &str,
    rgb: &mut [f32; 3],
) -> egui::Response {
    grid_label(ui, label);
    let response = ui.color_edit_button_rgb(rgb);
    ui.end_row();
    response
}

/// A label + checkbox grid row. The checkbox carries no inline text — the label
/// cell is its label.
pub(crate) fn labeled_checkbox(ui: &mut egui::Ui, label: &str, value: &mut bool) {
    grid_label(ui, label);
    ui.checkbox(value, "");
    ui.end_row();
}

/// A label + dropdown grid row: a stock [`egui::ComboBox`] with egui's default
/// styling (the toolbar's compact-combo styling is *not* applied here).
pub(crate) fn labeled_combo(
    ui: &mut egui::Ui,
    label: &str,
    id_salt: &str,
    selected_text: impl Into<egui::WidgetText>,
    contents: impl FnOnce(&mut egui::Ui),
) {
    grid_label(ui, label);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected_text)
        .show_ui(ui, contents);
    ui.end_row();
}

/// A label + read-only value grid row (Inspector node stats / stubbed slots).
pub(crate) fn value_row(ui: &mut egui::Ui, label: &str, value: &str) {
    grid_label(ui, label);
    ui.label(value);
    ui.end_row();
}

/// A label + color-button grid row over an `egui::Color32` (the stock egui
/// color-edit button / picker). Returns the button's [`egui::Response`].
pub(crate) fn labeled_color32(
    ui: &mut egui::Ui,
    label: &str,
    color: &mut egui::Color32,
) -> egui::Response {
    grid_label(ui, label);
    let response = ui.color_edit_button_srgba(color);
    ui.end_row();
    response
}

/// A stock "Reset all" button (default egui button styling).
pub(crate) fn reset_button(ui: &mut egui::Ui) -> egui::Response {
    ui.button("Reset all")
}

/// Monospace label using the bundled JetBrains Mono face. Used by the stats
/// overlay so numeric columns align on a fixed grid.
pub(crate) fn mono_label(text: &str, font_size: f32, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text)
        .size(font_size)
        .color(color)
        .family(egui::FontFamily::Monospace)
}

/// Draw centered text with a faux-bold weight in the given font. Only the
/// regular instance of each bundled variable font is registered with egui, so
/// there is no true bold face to select — the heavier stroke is approximated by
/// layering the glyphs with small sub-pixel offsets before the crisp center pass.
pub(crate) fn bold_text(
    painter: &egui::Painter,
    ctx: &egui::Context,
    pos: egui::Pos2,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
) {
    let offset = theme::px(ctx, size::GIZMO_BOLD_OFFSET);
    for delta in [
        egui::vec2(-offset, 0.0),
        egui::vec2(offset, 0.0),
        egui::vec2(0.0, -offset),
        egui::vec2(0.0, offset),
    ] {
        painter.text(
            pos + delta,
            egui::Align2::CENTER_CENTER,
            text,
            font.clone(),
            color,
        );
    }
    painter.text(pos, egui::Align2::CENTER_CENTER, text, font, color);
}
