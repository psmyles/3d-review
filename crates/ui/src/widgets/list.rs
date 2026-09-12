//! List rows (the Outliner's and the Inspector's) and the text helpers they use.

use crate::theme::{self, color, size};

/// One row of a flat list (the Opt stack, the Animations tab): a full-width
/// band that selects on click, painted like an Outliner row. Returns the row's
/// own response (the click that selects it), the rect the row's content goes in,
/// and the `trailing`-wide rect at the right edge for trailing controls / text.
pub(crate) fn list_row(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: bool,
    trailing: f32,
    height: f32,
) -> (egui::Response, egui::Rect, egui::Rect) {
    let (slot, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    // Span the pane's full width, not just the space this layout was given, so
    // the band and the click target reach both edges.
    let row_rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), slot.y_range());
    let response = ui.interact(row_rect, id, egui::Sense::click());

    if selected {
        ui.painter().rect_filled(
            row_rect,
            size::OUTLINER_ROW_ROUNDING,
            color::OUTLINER_ROW_SELECTED_BG,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(row_rect, size::OUTLINER_ROW_ROUNDING, color::HOVER_BG);
    }

    let mut content = row_rect.shrink2(egui::vec2(size::OUTLINER_ROW_PAD_X, 0.0));
    let controls = egui::Rect::from_min_max(
        egui::pos2(content.right() - trailing, content.top()),
        content.max,
    );
    content.set_right(controls.left());
    (response, content, controls)
}

/// A [`list_row`]'s name, pinned to the left edge of `rect`.
///
/// `Ui::put` centres what it places (it lays the widget out
/// `centered_and_justified`), which reads as a list of headings rather than a
/// list of entries — so the name gets its own left-to-right scope instead. The
/// label is non-interactive so the click lands on the row band beneath it.
pub(crate) fn list_row_label(ui: &mut egui::Ui, rect: egui::Rect, text: egui::RichText) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add(
                egui::Label::new(text)
                    .truncate()
                    .selectable(false)
                    .sense(egui::Sense::hover()),
            );
        },
    );
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
