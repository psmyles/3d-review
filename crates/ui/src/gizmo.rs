//! The interactive axis gizmo: six axis balls projected from the live camera
//! orientation. Dragging it orbits the camera; clicking a ball snaps the view
//! down that axis. Returns the chosen [`AxisGizmoAction`] for `app` to apply.

use glam::{Vec2, Vec3};
use review_render::{CameraProjection, OrbitCamera};

use crate::assets::{self, ICON_RESET};
use review_localization::Key;

use crate::keys;
use crate::state::{AxisGizmoAction, ViewAxis};
use crate::theme::{self, color, font, size};
use crate::widgets::bold_text;

pub(crate) fn draw_axis_gizmo(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    camera: OrbitCamera,
    projection: CameraProjection,
) -> Option<AxisGizmoAction> {
    let gizmo_size = size::GIZMO_SIZE;
    let reach = size::GIZMO_REACH;
    let ball_radius = size::GIZMO_BALL_RADIUS;
    let (rect, panel_response) = ui.allocate_exact_size(
        egui::vec2(gizmo_size, gizmo_size),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter();
    let center = rect.center();
    let mut action = panel_response
        .dragged()
        .then(|| panel_response.drag_motion())
        .filter(|delta| delta.length_sq() > 0.0)
        .map(|delta| AxisGizmoAction::Orbit(Vec2::new(delta.x, delta.y) * ctx.pixels_per_point()));

    if panel_response.hovered() || panel_response.dragged() {
        painter.rect_filled(rect, size::GIZMO_CORNER_RADIUS, color::GIZMO_BG);
    }

    let mut points = axis_gizmo_points(camera, projection, center, reach);
    points.sort_by(|a, b| a.depth.total_cmp(&b.depth));

    for point in points.iter().filter(|point| point.positive) {
        // Depth fade: the ramp endpoints are theme tokens; only the per-axis
        // depth factor is runtime state.
        let fade = size::GIZMO_DEPTH_FADE_FLOOR
            + size::GIZMO_DEPTH_FADE_RANGE * ((point.depth + 1.0) * 0.5);
        let alpha = (fade.clamp(0.0, 1.0) * 255.0) as u8;
        painter.line_segment(
            [center, point.position],
            egui::Stroke::new(
                size::GIZMO_LINE_WIDTH,
                point.color.linear_multiply(alpha as f32 / 255.0),
            ),
        );
    }

    for point in points {
        let ball_rect = egui::Rect::from_center_size(
            point.position,
            egui::vec2(ball_radius * 2.0, ball_radius * 2.0),
        );
        let response = ui
            .interact(
                ball_rect.expand(size::GIZMO_BALL_HIT_EXPAND),
                ui.make_persistent_id(("axis_gizmo_ball", point.axis)),
                egui::Sense::click(),
            )
            .on_hover_ui(|ui| {
                ui.set_max_width(size::TOOLTIP_MAX_WIDTH);
                ui.label(
                    egui::RichText::new(keys::ui_gizmo::view_axis(
                        review_localization::tr(point.axis_name).into_owned(),
                    ))
                    .strong(),
                );
                ui.add_space(size::TOOLTIP_TITLE_GAP);
                ui.label(keys::ui_gizmo::VIEW_AXIS_DESCRIPTION);
            });
        let radius = if response.hovered() {
            ball_radius * size::GIZMO_BALL_HOVER_SCALE
        } else {
            ball_radius
        };

        if point.positive {
            painter.circle_filled(point.position, radius, point.color);
            bold_text(
                painter,
                point.position,
                &point.label,
                egui::FontId::proportional(font::GIZMO_LABEL),
                color::GIZMO_LABEL,
            );
        } else if projection == CameraProjection::Orthographic
            && point.depth > size::GIZMO_AXIS_ALIGNED_DEPTH
        {
            // The axis we're snapped down points straight at the viewer: render
            // it solid like a positive axis so its label stays readable and the
            // current view stays identified even without hovering.
            painter.circle_filled(point.position, radius, point.color);
            bold_text(
                painter,
                point.position,
                &point.label,
                egui::FontId::proportional(font::GIZMO_LABEL_NEG),
                color::GIZMO_LABEL,
            );
        } else if response.hovered() {
            // Hovered: crisp full-color outline plus the axis label.
            painter.circle_stroke(
                point.position,
                radius,
                egui::Stroke::new(size::GIZMO_RING_WIDTH, point.color),
            );
            bold_text(
                painter,
                point.position,
                &point.label,
                egui::FontId::proportional(font::GIZMO_LABEL_NEG),
                point.color,
            );
        } else {
            // Idle: a translucent filled disc. egui's antialiased closed-path
            // strokes over-render thin rings, so a faded `circle_stroke` reads
            // near-opaque; a `circle_filled` honours the alpha and clearly looks
            // translucent.
            painter.circle_filled(
                point.position,
                radius,
                theme::with_opacity(point.color, size::GIZMO_NEG_OPACITY),
            );
        }

        if response.clicked() {
            action = Some(AxisGizmoAction::Snap(point.axis));
        }
    }

    // Reset-view button: tucked into the gizmo's bottom-left corner and revealed
    // only while the pointer is over the gizmo (or mid-drag). Drawn last so it
    // sits above the axis balls; clicking returns the camera to its home view.
    // (`contains_pointer` rather than `hovered` so it stays lit while the pointer
    // moves onto the button itself.)
    if panel_response.contains_pointer() || panel_response.dragged() {
        let reset_size = size::GIZMO_RESET_ICON_SIZE;
        let inset = size::GIZMO_RESET_INSET;
        let reset_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + inset, rect.bottom() - inset),
            egui::vec2(reset_size, reset_size),
        );
        let response = ui
            .interact(
                reset_rect.expand(size::GIZMO_BALL_HIT_EXPAND),
                ui.make_persistent_id("axis_gizmo_reset"),
                egui::Sense::click(),
            )
            .on_hover_ui(|ui| {
                ui.set_max_width(size::TOOLTIP_MAX_WIDTH);
                ui.label(egui::RichText::from(keys::ui_gizmo::RESET_VIEW).strong());
                ui.add_space(size::TOOLTIP_TITLE_GAP);
                ui.label(keys::ui_gizmo::RESET_VIEW_DESCRIPTION);
            });
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let tint = if response.hovered() {
            color::GIZMO_RESET_HOVERED
        } else {
            color::GIZMO_RESET_IDLE
        };
        if let Some(texture) = assets::load_icon_texture(ui, &ICON_RESET) {
            egui::Image::from_texture(texture)
                .fit_to_exact_size(reset_rect.size())
                .tint(tint)
                .paint_at(ui, reset_rect);
        }
        if response.clicked() {
            action = Some(AxisGizmoAction::ResetView);
        }
    }

    action
}

#[derive(Debug, Clone)]
struct AxisGizmoPoint {
    axis: ViewAxis,
    position: egui::Pos2,
    depth: f32,
    color: egui::Color32,
    /// The ball's own painted label, resolved once per frame. The positive balls
    /// are painted `X`/`Y`/`Z` bare while the tooltip names `+X`, so the label
    /// and the tooltip's axis name are two different strings even in English.
    label: String,
    /// The axis name the tooltip reads.
    axis_name: Key,
    positive: bool,
}

fn axis_gizmo_points(
    camera: OrbitCamera,
    projection: CameraProjection,
    center: egui::Pos2,
    reach: f32,
) -> Vec<AxisGizmoPoint> {
    // The ball's own painted label and the axis it looks down. Both come from the
    // catalog: the positive balls are painted "X"/"Y"/"Z" bare, so the label and
    // the tooltip's axis name are two different strings even in English.
    const AXES: [(ViewAxis, Vec3, egui::Color32, Key, bool); 6] = [
        (
            ViewAxis::PositiveX,
            Vec3::X,
            color::GIZMO_AXIS_X,
            keys::ui_gizmo::AXIS_POS_X,
            true,
        ),
        (
            ViewAxis::NegativeX,
            Vec3::NEG_X,
            color::GIZMO_AXIS_X,
            keys::ui_gizmo::AXIS_NEG_X,
            false,
        ),
        (
            ViewAxis::PositiveY,
            Vec3::Y,
            color::GIZMO_AXIS_Y,
            keys::ui_gizmo::AXIS_POS_Y,
            true,
        ),
        (
            ViewAxis::NegativeY,
            Vec3::NEG_Y,
            color::GIZMO_AXIS_Y,
            keys::ui_gizmo::AXIS_NEG_Y,
            false,
        ),
        (
            ViewAxis::PositiveZ,
            Vec3::Z,
            color::GIZMO_AXIS_Z,
            keys::ui_gizmo::AXIS_POS_Z,
            true,
        ),
        (
            ViewAxis::NegativeZ,
            Vec3::NEG_Z,
            color::GIZMO_AXIS_Z,
            keys::ui_gizmo::AXIS_NEG_Z,
            false,
        ),
    ];

    // Distance from a virtual eye to the gizmo's center, derived from the
    // viewport camera's vertical FOV (clamped to keep the foreshortening stable
    // and the denominator strictly positive). `None` in orthographic mode, where
    // the eye is effectively at infinity and the projection stays flat.
    let perspective_eye = match projection {
        CameraProjection::Orthographic => None,
        CameraProjection::Perspective => {
            Some(reach / (camera.fov_y_radians * 0.5).clamp(0.1, 0.6).tan())
        }
    };

    AXES.into_iter()
        .map(|(axis, world, color, label, positive)| {
            let view = camera.view_space_direction(world);
            // +Z in view space points toward the viewer: tips facing the camera
            // are magnified, tips behind the center shrink.
            let scale = perspective_eye
                .map(|eye| eye / (eye - view.z * reach))
                .unwrap_or(1.0);
            AxisGizmoPoint {
                axis,
                position: egui::pos2(
                    center.x + view.x * reach * scale,
                    center.y - view.y * reach * scale,
                ),
                depth: view.z,
                color,
                // A positive ball is painted with the bare letter — the axis
                // name's `+` is for the tooltip, where it distinguishes the two
                // directions.
                label: review_localization::tr(label).trim_start_matches('+').to_owned(),
                axis_name: label,
                positive,
            }
        })
        .collect()
}
