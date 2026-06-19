//! Reusable, theme-driven UI primitives shared by the toolbar, option panels and
//! stats overlay. None of these own state — they paint and report interactions.

use crate::assets::{self, AppIcon};
use crate::theme::{self, color, font, size};

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

/// Paint a fixed-width label cell (left column of the control table) and return
/// the width remaining for the control cell. Advances the cursor past the label
/// column + gap so the caller can drop the control straight after it.
pub(crate) fn table_label_cell(ui: &mut egui::Ui, label: &str) -> f32 {
    let control_w = (ui.available_width() - size::PANEL_LABEL_COL_W - size::PANEL_COL_GAP).max(0.0);
    let (label_rect, _) = ui.allocate_exact_size(
        egui::vec2(size::PANEL_LABEL_COL_W, size::PANEL_ROW_H),
        egui::Sense::hover(),
    );
    ui.painter().text(
        egui::pos2(label_rect.left(), label_rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(font::PANEL_LABEL),
        color::TEXT_LABEL,
    );
    ui.add_space(size::PANEL_COL_GAP);
    control_w
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

/// A label + slider row that also shows the live value in an editable numeric
/// field at the right of the row. The slider gives up a fixed slice of the
/// control column to the field; both bind the same value, so dragging the slider
/// updates the field and clicking/typing/dragging the field updates the slider.
/// `decimals` fixes the field's displayed precision (use 0 for integers) and
/// `speed` is the field's drag step.
///
/// This is the standard slider for every option panel: a rail plus an inline,
/// click-to-edit, drag-to-scrub value field. New panels should use it rather than
/// a bare slider so all sliders behave identically.
pub(crate) fn labeled_slider_with_value<Num: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Num,
    range: std::ops::RangeInclusive<Num>,
    decimals: usize,
    speed: f32,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        // Center every control on the row's vertical axis so the slider rail, the
        // value field and the label all line up regardless of their natural heights.
        ui.set_min_height(size::PANEL_ROW_H);
        let control_w = table_label_cell(ui, label);
        let slider_w = (control_w - size::PANEL_VALUE_FIELD_W - size::PANEL_COL_GAP).max(0.0);
        ui.spacing_mut().slider_width = slider_w;
        ui.add(
            egui::Slider::new(value, range.clone())
                .show_value(false)
                .clamping(egui::SliderClamping::Always),
        );
        ui.add_space(size::PANEL_COL_GAP);
        // Shrink the field's vertical padding so it sits exactly at row height
        // (the global button padding would otherwise make it taller than the row,
        // growing the row and breaking the inter-row spacing).
        ui.spacing_mut().interact_size.y = size::PANEL_ROW_H;
        ui.spacing_mut().button_padding.y = size::PANEL_COMBO_BUTTON_PAD_Y;
        // Tighten the field's inner horizontal padding so the digits sit close to
        // the box edges and the widest readout fits without clipping.
        ui.spacing_mut().button_padding.x = size::PANEL_VALUE_FIELD_PAD_X;
        // Render the field's digits in the monospace face (this is the last widget
        // in the row, so the font override doesn't bleed into other controls).
        ui.style_mut().override_font_id = Some(egui::FontId::monospace(font::PANEL_VALUE));
        ui.add_sized(
            egui::vec2(size::PANEL_VALUE_FIELD_W, size::PANEL_ROW_H),
            egui::DragValue::new(value)
                .range(range)
                .speed(speed)
                .min_decimals(decimals)
                .max_decimals(decimals)
                .clamp_existing_to_range(true),
        );
    });
}

/// A label + checkbox row using the shared two-column table layout. The checkbox
/// (no inline text — the label cell carries it) sits left-aligned in the control
/// column so it lines up with the other panel controls.
pub(crate) fn labeled_checkbox(ui: &mut egui::Ui, label: &str, value: &mut bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let _control_w = table_label_cell(ui, label);
        // Keep the box the same height as other control rows so the row aligns.
        ui.spacing_mut().interact_size.y = size::PANEL_ROW_H;
        ui.add(egui::Checkbox::new(value, ""));
    });
}

/// A row of clickable color swatches; clicking one writes it into `selected`.
pub(crate) fn color_swatch_row(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut egui::Color32,
    colors: &[egui::Color32],
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let control_w = table_label_cell(ui, label);
        // Compact thumbnails with tight, even gaps, left-aligned in the column;
        // capped so they stay small rather than filling the whole column.
        let gaps = size::PANEL_SWATCH_GAP * (colors.len() as f32 - 1.0);
        let swatch = ((control_w - gaps) / colors.len() as f32).clamp(0.0, size::PANEL_SWATCH_SIZE);
        for (i, swatch_color) in colors.iter().enumerate() {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(swatch, swatch), egui::Sense::click());
            if response.clicked() {
                *selected = *swatch_color;
            }
            let stroke = if selected == swatch_color {
                egui::Stroke::new(size::SELECTION_STROKE_WIDTH, color::SELECTION_STROKE)
            } else {
                egui::Stroke::new(size::HAIRLINE, color::SWATCH_BORDER)
            };
            ui.painter().rect(
                rect,
                size::SWATCH_CORNER_RADIUS,
                *swatch_color,
                stroke,
                egui::StrokeKind::Outside,
            );
            if i + 1 < colors.len() {
                ui.add_space(size::PANEL_SWATCH_GAP);
            }
        }
    });
}

/// A full-width "Reset all" button with the label painted dead-center.
pub(crate) fn wide_reset_button(ui: &mut egui::Ui) -> egui::Response {
    // egui's `Button` left-aligns text within an over-wide rect (it honors the
    // parent layout's horizontal align), so paint the label ourselves to keep it
    // centered.
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, size::PANEL_BUTTON_HEIGHT),
        egui::Sense::click(),
    );
    let fill = if response.hovered() {
        color::BUTTON_HOVER_BG
    } else {
        color::BUTTON_BG
    };
    ui.painter().rect(
        rect,
        size::BUTTON_CORNER_RADIUS,
        fill,
        egui::Stroke::new(size::HAIRLINE, color::BUTTON_BORDER),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Reset all",
        egui::FontId::proportional(font::PANEL_BUTTON),
        color::TEXT_BUTTON,
    );
    response
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
