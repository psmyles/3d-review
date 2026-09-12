//! Buttons and tab strips.

use crate::assets::{self, AppIcon};
use crate::theme::{self, color, font, size};

/// A button that fills the width it is given, with its label centred.
///
/// egui left-aligns a button's label as soon as `min_size` widens the button past
/// the text ("if there are no growable atoms then everything will be left-aligned")
/// — growable spacers either side are what re-centre it.
pub(crate) fn wide_button(label: &str, width: f32) -> egui::Button<'_> {
    egui::Button::new((egui::Atom::grow(), label, egui::Atom::grow()))
        .min_size(egui::vec2(width, 0.0))
}

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
        false,
    )
}

/// A standard icon toggle for a button that also exposes a right-click options
/// panel: identical to [`icon_toggle_button`] but while hovered it paints a thin
/// green line along the tile's top edge — end to end, fading from transparent at
/// both ends to opaque at the center — hinting that there are more options behind
/// it.
pub(crate) fn icon_toggle_button_with_options(
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
        true,
    )
}

/// A custom-painted icon tile button: recessed when idle, accent-filled when
/// selected, lifted on hover. The icon is tinted brighter when selected. When
/// `has_options` is set, a hover paints the green options-hint underline.
pub(crate) fn icon_tile_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon: &AppIcon,
    selected: bool,
    tooltip: &str,
    tile_size: egui::Vec2,
    has_options: bool,
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
        let image_padding = rect.width().min(rect.height()) * size::TILE_ICON_INSET_RATIO;
        let image_rect = rect.shrink2(egui::vec2(image_padding, image_padding));
        egui::Image::from_texture(texture)
            .fit_to_exact_size(image_rect.size())
            .tint(tint)
            .paint_at(ui, image_rect);
    }

    // Hint that this button has a right-click options panel: a thin green line
    // along the top edge, end to end, fading from transparent at both ends to
    // opaque at the center. egui has no gradient-stroke primitive, so it's a
    // 1px-tall two-quad mesh with per-vertex colors, painted last so it sits over
    // the fill/icon.
    if has_options && response.hovered() {
        let edge = theme::with_opacity(color::OPTIONS_HINT, 0.0);
        let center = color::OPTIONS_HINT;
        let top = rect.top();
        let bottom = top + size::OPTIONS_HINT_THICKNESS;
        let mid_x = rect.center().x;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(egui::pos2(rect.left(), top), edge); // 0
        mesh.colored_vertex(egui::pos2(rect.left(), bottom), edge); // 1
        mesh.colored_vertex(egui::pos2(mid_x, top), center); // 2
        mesh.colored_vertex(egui::pos2(mid_x, bottom), center); // 3
        mesh.colored_vertex(egui::pos2(rect.right(), top), edge); // 4
        mesh.colored_vertex(egui::pos2(rect.right(), bottom), edge); // 5
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(1, 3, 2);
        mesh.add_triangle(2, 3, 4);
        mesh.add_triangle(3, 5, 4);
        ui.painter().add(egui::Shape::mesh(mesh));
    }

    response.on_hover_text(tooltip)
}

/// A custom-painted **text** segment tile (a labelled radio cell), sized to
/// `width`: accent-filled when selected, lifted on hover, idle transparent. The
/// shared primitive behind the mode tabs (3D/UV/Tex), the Tex channel group
/// (RGB/R/G/B/A) and the Tex background group (B/W/G/C). Returns the response so
/// the caller drives selection on `clicked()`.
pub(crate) fn segment_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    label: &str,
    selected: bool,
    width: f32,
) -> egui::Response {
    let desired = egui::vec2(width, theme::px(ctx, size::MODE_SEGMENT_HEIGHT));
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    let fill = if selected {
        color::ACCENT
    } else if response.hovered() {
        color::HOVER_BG
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = if selected {
        color::TEXT_PRIMARY
    } else {
        color::TEXT_SEGMENT_IDLE
    };

    ui.painter().rect(
        rect,
        theme::px(ctx, size::TILE_CORNER_RADIUS),
        fill,
        egui::Stroke::NONE,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(theme::px(ctx, font::MODE_SEGMENT)),
        text_color,
    );
    response
}

/// A full-width tab strip: one equal-width cell per label, all sharing a single
/// baseline hairline, with the active tab marked by an accent underline and a
/// brightened label. Hovering an inactive tab lifts it with the standard hover
/// fill and a pointing-hand cursor.
///
/// The label is drawn in the ambient `TextStyle::Button` font — the same one the
/// stock `selectable_label` rows beneath the strip resolve — so the tabs and the
/// list they head always read at one size. For the same reason every measurement
/// here is in egui **points**, not DPI-scaled logical pixels: the strip has to
/// track the surrounding stock widgets, not a fixed physical size.
///
/// This is the `egui_tabs` look, painted here rather than pulled in: that crate
/// pins `egui 0.29`/`0.30` and this workspace is on 0.36, so linking it would fork a
/// second `egui` into the graph.
/// It also parks the selection in `ctx` temp data, which would duplicate the
/// `UiState` field that already owns it (invariant 2).
///
/// Returns the index of the tab clicked this frame, if any; the caller owns the
/// selection.
pub(crate) fn tab_bar(ui: &mut egui::Ui, labels: &[&str], selected: usize) -> Option<usize> {
    if labels.is_empty() {
        return None;
    }

    let font = egui::TextStyle::Button.resolve(ui.style());
    // Tall enough to clear the label with the standard tab padding above and
    // below it, and never shorter than the strip's floor.
    let height = (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * size::OUTLINER_TAB_PAD_Y)
        .max(size::OUTLINER_TAB_HEIGHT);
    let underline = size::OUTLINER_TAB_UNDERLINE;
    let (strip, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let cell_width = strip.width() / labels.len() as f32;
    // The underline is centred on the rail, so both share this baseline.
    let baseline = strip.bottom() - underline * 0.5;
    let painter = ui.painter().clone();
    let mut clicked = None;

    // One continuous rail under the whole strip, so the active underline reads as
    // a lit segment of it rather than a floating bar.
    painter.hline(
        strip.x_range(),
        baseline,
        egui::Stroke::new(size::HAIRLINE, color::DIVIDER),
    );

    for (index, label) in labels.iter().enumerate() {
        let cell = egui::Rect::from_min_size(
            egui::pos2(strip.left() + cell_width * index as f32, strip.top()),
            egui::vec2(cell_width, height),
        );
        let response = ui.interact(cell, ui.id().with(("tab", index)), egui::Sense::click());
        let active = index == selected;

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if !active {
                painter.rect_filled(cell, size::TILE_CORNER_RADIUS, color::HOVER_BG);
            }
        }
        if response.clicked() {
            clicked = Some(index);
        }

        // Centre the label in the space above the rail, not in the whole cell.
        painter.text(
            egui::pos2(cell.center().x, (cell.top() + baseline) * 0.5),
            egui::Align2::CENTER_CENTER,
            label,
            font.clone(),
            if active {
                color::TEXT_PRIMARY
            } else {
                color::TEXT_SEGMENT_IDLE
            },
        );

        if active {
            painter.hline(
                cell.x_range(),
                baseline,
                egui::Stroke::new(underline, color::ACCENT),
            );
        }
    }

    clicked
}

/// A stock "Reset all" button (default egui button styling).
pub(crate) fn reset_button(ui: &mut egui::Ui) -> egui::Response {
    ui.button("Reset all")
}
