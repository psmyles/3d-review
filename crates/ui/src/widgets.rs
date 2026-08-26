//! Reusable, theme-driven UI primitives shared by the toolbar, option panels and
//! stats overlay. None of these own state — they paint and report interactions.

use review_render::ViewportBackground;

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
        let image_padding = theme::px(ctx, size::TOOLBAR_ICON_PADDING);
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
/// pins `egui 0.29`/`0.30`, and this workspace is on 0.33 (the ceiling set by
/// `egui-directx11`), so linking it would fork a second `egui` into the graph.
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
    let row_gap = ui.spacing().item_spacing.y;
    egui::Grid::new(("panel_grid", salt))
        .num_columns(2)
        .min_col_width(size::PANEL_LABEL_COL_WIDTH)
        .spacing(egui::vec2(size::PANEL_GRID_COL_GAP, row_gap))
        .striped(true)
        .show(ui, contents);
}

/// The left (label) cell of a panel-grid row: a plain default-styled label
/// pinned to the fixed [`size::PANEL_LABEL_COL_WIDTH`] (left-aligned; an
/// over-long label truncates rather than widening the column, so every panel
/// keeps the same label/control split). Leaves the cursor in the control column
/// for the caller to drop the row's widget and then call `ui.end_row()`.
fn grid_label(ui: &mut egui::Ui, label: &str) {
    ui.scope(|ui| {
        ui.set_width(size::PANEL_LABEL_COL_WIDTH);
        ui.add(egui::Label::new(label).truncate());
    });
}

/// The right (control) cell of a panel-grid row: a cell spanning **all the
/// remaining width** of the row (the window minus the fixed label column) whose
/// contents are **right-aligned**, so the control's right edge sits flush with
/// the panel's right edge — no matter how wide egui sizes the window — and any
/// slack in a row (a short swatch run, a narrow value) falls *between* the label
/// and the control. Controls that already fill the column (slider rail, combo)
/// span it edge to edge regardless.
fn grid_control<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
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
            // Place the value box in a fixed-size cell whose layout is justified
            // (the box fills `value_w`, so its width never reflows) but
            // left-aligned (`main_align = Min`) — egui's `DragValue` positions
            // its text via the current layout, so this left-aligns both the
            // displayed value and the edit-mode caret. (`ui.add_sized` would
            // center it via a `centered_and_justified` layout.)
            let cell_layout = egui::Layout::centered_and_justified(egui::Direction::LeftToRight)
                .with_main_align(egui::Align::Min);
            response |= ui
                .allocate_ui_with_layout(egui::vec2(value_w, box_height), cell_layout, |ui| {
                    ui.add(
                        egui::DragValue::new(value)
                            .range(range)
                            .max_decimals(decimals),
                    )
                })
                .inner;
            response.changed()
        })
        .inner;
    ui.end_row();
    changed
}

/// A label + color-button grid row (the stock egui color-edit button). Returns the
/// button's [`egui::Response`] so the caller can react to an edit.
pub(crate) fn labeled_color_button(
    ui: &mut egui::Ui,
    label: &str,
    rgb: &mut [f32; 3],
) -> egui::Response {
    grid_label(ui, label);
    let response = grid_control(ui, |ui| ui.color_edit_button_rgb(rgb));
    ui.end_row();
    response
}

/// A label + checkbox grid row. The checkbox carries no inline text — the label
/// cell is its label.
pub(crate) fn labeled_checkbox(ui: &mut egui::Ui, label: &str, value: &mut bool) {
    grid_label(ui, label);
    grid_control(ui, |ui| ui.checkbox(value, ""));
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
        .width(ui.available_width())
        .height(size::LABELED_COMBO_POPUP_MAX_H)
        .show_ui(ui, contents);
    ui.end_row();
}

/// A label + read-only value grid row (Inspector node stats / stubbed slots).
pub(crate) fn value_row(ui: &mut egui::Ui, label: &str, value: &str) {
    grid_label(ui, label);
    grid_control(ui, |ui| ui.label(value));
    ui.end_row();
}

/// A label + a row of square preset-color swatch buttons (each a stock
/// `egui::Button` at the standard button height); clicking one writes it into
/// `selected`. The active swatch is outlined with the selection stroke.
pub(crate) fn color_swatch_row(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut egui::Color32,
    colors: &[egui::Color32],
) {
    grid_label(ui, label);
    grid_control(ui, |ui| {
        // Right-align the whole swatch run while keeping the swatches themselves
        // in left-to-right order (a plain `ui.horizontal` group, placed at the
        // right edge of the right-to-left control cell).
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = size::PANEL_SWATCH_GAP;
            let side = ui.spacing().interact_size.y;
            for &swatch in colors {
                let mut button = egui::Button::new("")
                    .fill(swatch)
                    .min_size(egui::vec2(side, side));
                if *selected == swatch {
                    button = button.stroke(egui::Stroke::new(
                        size::SELECTION_STROKE_WIDTH,
                        color::SELECTION_STROKE,
                    ));
                }
                if ui.add(button).clicked() {
                    *selected = swatch;
                }
            }
        });
    });
    ui.end_row();
}

/// A label + a row of viewport-background preset swatches: each preset painted as
/// its solid color (or a vertical gradient for [`ViewportBackground::Gradient`]),
/// the active one outlined with the selection stroke, every swatch carrying a hover
/// tooltip with its name. Clicking a swatch selects that preset. The fill colors are
/// the presets' own display values (a runtime-derived value, invariant-8 exception),
/// not theme tokens.
pub(crate) fn background_swatch_row(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut ViewportBackground,
) {
    grid_label(ui, label);
    grid_control(ui, |ui| {
        // The control cell is right-aligned (a `right_to_left` layout), and
        // `ui.horizontal` inherits that preference — so children are placed
        // right-to-left. Walk `ALL` in reverse (Gradient first → rightmost, Black
        // last → leftmost) so the run reads left-to-right on screen (Black …
        // Gradient) while staying flush to the panel's right edge.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = size::PANEL_SWATCH_GAP;
            let side = ui.spacing().interact_size.y;
            for preset in ViewportBackground::ALL.into_iter().rev() {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                paint_background_swatch(ui, rect, preset, *selected == preset);
                if response.on_hover_text(preset.label()).clicked() {
                    *selected = preset;
                }
            }
        });
    });
    ui.end_row();
}

/// Paint one [`ViewportBackground`] preset swatch: a solid rounded fill, or a
/// vertical top→bottom gradient for the gradient preset, with a selection / idle
/// outline. The colors are the preset's display (sRGB) values.
fn paint_background_swatch(
    ui: &egui::Ui,
    rect: egui::Rect,
    preset: ViewportBackground,
    selected: bool,
) {
    let (top, bottom) = preset.gradient_srgb();
    let top_color = srgb3_to_color32(top);
    let bottom_color = srgb3_to_color32(bottom);
    let stroke = if selected {
        egui::Stroke::new(size::SELECTION_STROKE_WIDTH, color::SELECTION_STROKE)
    } else {
        egui::Stroke::new(size::HAIRLINE, color::SWATCH_BORDER)
    };
    // Keep the corner radius the same as the flat swatches but never let it exceed
    // half the swatch, so the rounded-rect gradient mesh stays well-formed.
    let radius = size::SWATCH_CORNER_RADIUS.min(rect.width().min(rect.height()) * 0.5);
    let painter = ui.painter();
    if top_color == bottom_color {
        painter.rect(rect, radius, top_color, stroke, egui::StrokeKind::Inside);
    } else {
        // Vertical gradient: egui has no gradient-fill primitive, so build a
        // *rounded-rectangle* mesh as a triangle fan from the center and color each
        // boundary vertex by its vertical position. A fan reproduces a strictly
        // vertical gradient exactly (the color is affine in y, which barycentric
        // interpolation preserves), and the rounded boundary means the fill no
        // longer bleeds past the swatch's rounded outline.
        let color_at = |y: f32| {
            let t = ((y - rect.top()) / rect.height().max(f32::EPSILON)).clamp(0.0, 1.0);
            lerp_color32(top_color, bottom_color, t)
        };
        let boundary = rounded_rect_points(rect, radius);
        let mut mesh = egui::Mesh::default();
        let center = rect.center();
        mesh.colored_vertex(center, color_at(center.y)); // index 0 — fan hub
        for &p in &boundary {
            mesh.colored_vertex(p, color_at(p.y));
        }
        let n = boundary.len() as u32;
        for i in 0..n {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.rect(
            rect,
            radius,
            egui::Color32::TRANSPARENT,
            stroke,
            egui::StrokeKind::Inside,
        );
    }
}

/// The boundary points of a rounded rectangle, clockwise, with each corner sampled
/// as a quarter-circle arc — used to tessellate a rounded gradient swatch as a
/// triangle fan from the center.
fn rounded_rect_points(rect: egui::Rect, radius: f32) -> Vec<egui::Pos2> {
    // Segments per 90° corner: enough to read as round at swatch scale, cheap.
    const SEG: usize = 6;
    let r = radius.max(0.0);
    let (l, t, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    // Arc centers + start angle (degrees) for each corner, ordered to walk the
    // perimeter clockwise: top-left → top-right → bottom-right → bottom-left.
    let arcs = [
        (egui::pos2(l + r, t + r), 180.0_f32),
        (egui::pos2(right - r, t + r), 270.0),
        (egui::pos2(right - r, bottom - r), 0.0),
        (egui::pos2(l + r, bottom - r), 90.0),
    ];
    let mut pts = Vec::with_capacity(arcs.len() * (SEG + 1));
    for (c, start_deg) in arcs {
        for i in 0..=SEG {
            let deg = start_deg + 90.0 * (i as f32 / SEG as f32);
            let (sin, cos) = deg.to_radians().sin_cos();
            pts.push(egui::pos2(c.x + r * cos, c.y + r * sin));
        }
    }
    pts
}

/// Linear per-channel interpolation between two `Color32`s in their stored (sRGB
/// byte) space — `t = 0` returns `a`, `t = 1` returns `b`. Matches how egui
/// interpolates vertex colors across a mesh, so the swatch gradient reads smoothly.
fn lerp_color32(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// A display-space (sRGB, 0..1) RGB triple to an egui `Color32` (opaque). egui's
/// `Color32` stores sRGB bytes, so the values map straight through.
fn srgb3_to_color32(rgb: [f32; 3]) -> egui::Color32 {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(channel(rgb[0]), channel(rgb[1]), channel(rgb[2]))
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
