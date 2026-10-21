//! Color and background swatches, and the painting behind them.
//!
//! The only widgets here that draw rather than delegate to egui, which is why
//! this is the one file in the set with tests.

use review_render::ViewportBackground;

use crate::theme::{color, size};

use super::form::{grid_control, grid_label_tip};
use super::tooltip::Tip;

/// A label + read-only value grid row (Inspector node stats / stubbed slots).
pub(crate) fn value_row(ui: &mut egui::Ui, label: Tip, value: impl Into<egui::WidgetText>) {
    grid_label_tip(ui, label);
    grid_control(ui, |ui| ui.label(value));
    ui.end_row();
}

/// A label + a row of square preset-color swatch buttons (each a stock
/// `egui::Button` at the standard button height); clicking one writes it into
/// `selected`. The active swatch is outlined with the selection stroke.
///
/// Two things keep every swatch the same square in every widget state — without
/// them the row's height changes as the pointer crosses it, resizing the whole
/// option window. A stock button sizes itself as
/// `content + button_padding + frame stroke`, where the padding egui picks
/// already subtracts *its own* per-state `bg_stroke.width` (0 idle, 1 hovered)
/// so the total holds steady. Overriding the stroke breaks that compensation, so
/// the swatches carry **no content atom** (`Button::new(())`, not `""` — an empty
/// string is still a text atom one row tall): with nothing inside, `min_size`
/// alone decides the square and neither the selection stroke nor the state's
/// stroke can grow it. The corner radius is likewise pinned to the idle one, as
/// egui rounds a hovered widget more than an idle one.
pub(crate) fn color_swatch_row(
    ui: &mut egui::Ui,
    label: Tip,
    selected: &mut egui::Color32,
    colors: &[egui::Color32],
) {
    grid_label_tip(ui, label);
    grid_control(ui, |ui| {
        swatch_run(ui, colors.len(), |ui, side| {
            let corner_radius = ui.visuals().widgets.inactive.corner_radius;
            for &swatch in colors {
                let mut button = egui::Button::new(())
                    .fill(swatch)
                    .corner_radius(corner_radius)
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
    label: Tip,
    selected: &mut ViewportBackground,
) {
    grid_label_tip(ui, label);
    grid_control(ui, |ui| {
        swatch_run(ui, ViewportBackground::ALL.len(), |ui, side| {
            for preset in ViewportBackground::ALL {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                paint_background_swatch(ui, rect, preset, *selected == preset);
                if response
                    .on_hover_text(crate::labels::viewport_background(preset))
                    .clicked()
                {
                    *selected = preset;
                }
            }
        });
    });
    ui.end_row();
}

/// Lay `count` square swatches out as one left-to-right run, flush with the right
/// edge of the control cell it sits in, and hand `add` the swatch side.
///
/// The cell is a `right_to_left` layout (see [`grid_control`]), and a plain
/// `ui.horizontal` inside it inherits that direction: the first swatch landed
/// rightmost and every palette read backwards. So the run is allocated at its own
/// width — which the right-to-left cell places at its right edge — with an
/// explicit left-to-right layout inside, and the swatches go in the order given.
fn swatch_run<R>(ui: &mut egui::Ui, count: usize, add: impl FnOnce(&mut egui::Ui, f32) -> R) -> R {
    let side = ui.spacing().interact_size.y;
    let gap = size::PANEL_SWATCH_GAP;
    let width = side * count as f32 + gap * count.saturating_sub(1) as f32;
    ui.allocate_ui_with_layout(
        egui::vec2(width, side),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            add(ui, side)
        },
    )
    .inner
}

/// Paint one [`ViewportBackground`] preset swatch: a solid rounded fill, or a
/// vertical top→bottom gradient for the gradient preset, with a selection / idle
/// outline. The colors are the preset's display (sRGB) values.
pub(crate) fn paint_background_swatch(
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
pub(crate) fn rounded_rect_points(rect: egui::Rect, radius: f32) -> Vec<egui::Pos2> {
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
pub(crate) fn lerp_color32(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// A display-space (sRGB, 0..1) RGB triple to an egui `Color32` (opaque). egui's
/// `Color32` stores sRGB bytes, so the values map straight through.
pub(crate) fn srgb3_to_color32(rgb: [f32; 3]) -> egui::Color32 {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(channel(rgb[0]), channel(rgb[1]), channel(rgb[2]))
}

#[cfg(test)]
mod tests {
    use super::super::form::panel_grid;
    use super::*;

    const COLORS: [egui::Color32; 3] = [
        egui::Color32::WHITE,
        egui::Color32::GREEN,
        egui::Color32::RED,
    ];

    /// Lay out one [`color_swatch_row`] headlessly with `selected` marked and the
    /// pointer at `pointer`, and return the height the row's content took. Two
    /// passes, because a widget's state follows the *previous* pass's response —
    /// the second is the one that reflects the pointer.
    fn swatch_row_height(selected: egui::Color32, pointer: Option<egui::Pos2>) -> f32 {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                events: pointer.map(egui::Event::PointerMoved).into_iter().collect(),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let before = ui.cursor().top();
                panel_grid(ui, "swatch_test", |ui| {
                    let mut selected = selected;
                    color_swatch_row(
                        ui,
                        Tip::new(crate::keys::ui_panels::LINE_COLOR),
                        &mut selected,
                        &COLORS,
                    );
                });
                height = ui.min_rect().bottom() - before;
            });
            // Headless: nothing consumes the font-atlas upload, and epaint
            // panics on a dropped delta.
            output.textures_delta.clear();
        }
        height
    }

    /// A swatch must occupy the same square whether it is idle, hovered or
    /// selected: the row sits in a non-resizable option window, so a swatch that
    /// grows under the pointer (or once picked) resizes the whole window.
    #[test]
    fn swatch_row_height_is_state_independent() {
        let idle = swatch_row_height(egui::Color32::WHITE, None);
        assert!(idle > 0.0, "the row laid out nothing");

        // Nothing selected in the row at all — the baseline a plain button gives.
        assert_eq!(
            swatch_row_height(egui::Color32::BLUE, None),
            idle,
            "the selection stroke changed the row height"
        );

        // Sweep the pointer across the whole row: no position may change it.
        for x in (0..400).step_by(4) {
            let pointer = egui::pos2(x as f32, idle * 0.5);
            assert_eq!(
                swatch_row_height(egui::Color32::WHITE, Some(pointer)),
                idle,
                "hovering at x={x} changed the row height"
            );
        }
    }

    /// Click a [`color_swatch_row`] of [`COLORS`] at `pointer` and return the
    /// colour that click picked, if it landed on a swatch.
    fn swatch_clicked_at(pointer: egui::Pos2) -> Option<egui::Color32> {
        // Not in the palette, so any change is the click's doing.
        const NONE_PICKED: egui::Color32 = egui::Color32::BLUE;
        let ctx = egui::Context::default();
        let mut selected = NONE_PICKED;
        let button = |pressed| egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pointer)],
            vec![button(true)],
            vec![button(false)],
        ] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                panel_grid(ui, "swatch_test", |ui| {
                    color_swatch_row(
                        ui,
                        Tip::new(crate::keys::ui_panels::LINE_COLOR),
                        &mut selected,
                        &COLORS,
                    );
                });
            });
            output.textures_delta.clear();
        }
        (selected != NONE_PICKED).then_some(selected)
    }

    /// A palette reads left to right in the order it is given. The row's
    /// `ui.horizontal` used to inherit the control cell's right-to-left layout,
    /// so the first colour sat rightmost and every palette read backwards.
    #[test]
    fn a_palette_reads_left_to_right() {
        let y = swatch_row_height(egui::Color32::BLUE, None) * 0.5;
        let mut seen: Vec<egui::Color32> = Vec::new();
        for x in (0..400).step_by(2) {
            if let Some(color) = swatch_clicked_at(egui::pos2(x as f32, y))
                && seen.last() != Some(&color)
            {
                seen.push(color);
            }
        }
        assert_eq!(seen, COLORS, "the swatches, clicked from left to right");
    }
}
