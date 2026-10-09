//! Review-comment pins over a view: a numbered circle wherever a listed thread
//! points, placed this frame by `app` (which resolves each anchor against the
//! current pose, or onto the UV layout) and projected here through the view's
//! own camera.
//!
//! In the 3D view a pin behind the model is drawn faded rather than hidden, so a
//! reviewer always knows a comment is there; one about frames the animation isn't
//! on is fainter still. Hovering shows the comment's opening; a click selects it
//! without moving the camera — going to it is the Comments tab's click, or the
//! Inspector's button.

use glam::{Mat4, Vec3};
use review_model::{ModelData, SceneBvh};
use review_render::{OrbitCamera, UvCamera};

use crate::dimensions;
use crate::state::{CommentPin, UiState, ViewProjectionMode};
use crate::theme::{color, font, size};

/// What pins are projected through: the view's projection of the space `app`
/// placed them in, the rect its picture covers, the rect pins are kept inside
/// (the free viewport), and — for a 3D view — what can hide them.
pub(crate) struct PinView<'a> {
    pub(crate) view_projection: Mat4,
    pub(crate) image: egui::Rect,
    pub(crate) clamp: egui::Rect,
    pub(crate) occlusion: Option<PinOcclusion<'a>>,
}

/// The 3D view's occlusion test for pins.
pub(crate) struct PinOcclusion<'a> {
    camera: OrbitCamera,
    orthographic: bool,
    model: &'a ModelData,
    /// Occlusion structure over `model`; `None` draws every pin at full strength.
    bvh: Option<&'a SceneBvh>,
}

impl<'a> PinView<'a> {
    /// A 3D view through `camera`, whose pins `bvh` can hide behind `model`.
    pub(crate) fn scene(
        state: &UiState,
        camera: OrbitCamera,
        image: egui::Rect,
        clamp: egui::Rect,
        model: &'a ModelData,
        bvh: Option<&'a SceneBvh>,
    ) -> Self {
        Self {
            view_projection: camera.view_projection(state.projection_mode.into()),
            image,
            clamp,
            occlusion: Some(PinOcclusion {
                camera,
                orthographic: matches!(state.projection_mode, ViewProjectionMode::Orthographic),
                model,
                bvh,
            }),
        }
    }

    /// The UV layout through `camera`, whose pins `app` placed at `(u, v, 0)`.
    /// Nothing hides a pin there.
    pub(crate) fn uv(camera: UvCamera, image: egui::Rect, clamp: egui::Rect) -> Self {
        Self {
            view_projection: camera.view_projection(),
            image,
            clamp,
            occlusion: None,
        }
    }

    /// Where `world` lands on screen, if it is in the picture.
    pub(crate) fn project(&self, world: Vec3) -> Option<egui::Pos2> {
        dimensions::project(self.view_projection, world, self.image)
    }

    /// Whether the model hides the pin at `world` from the camera.
    fn occluded(&self, world: Vec3, on_surface: bool, hidden: &[u32]) -> bool {
        let Some(occlusion) = &self.occlusion else {
            return false;
        };
        let Some(bvh) = occlusion.bvh else {
            return false;
        };
        let scene_size = occlusion
            .model
            .bounds
            .map_or(1.0, |bounds| bounds.size().length())
            .max(1.0e-3);
        let origin = if occlusion.orthographic {
            world - occlusion.camera.forward_dir() * scene_size * 2.0
        } else {
            occlusion.camera.eye_position()
        };
        let target = if on_surface {
            lifted(world, origin, scene_size)
        } else {
            world
        };
        bvh.segment_occluded(occlusion.model, origin, target, hidden)
    }
}

/// How far a surface pin's occlusion ray stops short of it, as a fraction of the
/// scene's size — so the surface the pin sits on can't count as hiding it.
const SURFACE_LIFT: f32 = 1.0e-3;

/// Draw every listed pin, and select the thread whose pin is clicked.
pub(crate) fn draw_comment_pins(ctx: &egui::Context, state: &mut UiState, view: &PinView<'_>) {
    if !state.comments.show_pins || state.comments.pins.is_empty() {
        return;
    }
    let hidden: Vec<u32> = state.hidden_meshes.iter().map(|&i| i as u32).collect();
    let selected = state
        .comment_inspected()
        .then_some(state.comments.selected)
        .flatten();

    // The selected pin is drawn last, so it is never under a neighbour.
    let mut pins: Vec<CommentPin> = state
        .comments
        .pins
        .iter()
        .copied()
        .filter(|pin| state.comments.listed(pin.thread))
        .collect();
    pins.sort_by_key(|pin| selected == Some(pin.thread));

    let mut clicked = None;
    egui::Area::new(egui::Id::new("comment_pins"))
        .order(egui::Order::Background)
        .fixed_pos(view.clamp.min)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_clip_rect(view.clamp);
            for pin in pins {
                let Some(center) = view.project(pin.world) else {
                    continue;
                };
                if !view.clamp.contains(center) {
                    continue;
                }
                let occluded = view.occluded(pin.world, pin.on_surface, &hidden);
                let mut opacity: f32 = 1.0;
                if occluded {
                    opacity = opacity.min(color::COMMENT_PIN_OCCLUDED_OPACITY);
                }
                if !pin.in_range {
                    opacity = opacity.min(color::COMMENT_PIN_GHOST_OPACITY);
                }
                let is_selected = selected == Some(pin.thread);
                let response = paint_pin(ui, pin.thread, center, opacity, is_selected, state);
                if response.clicked() {
                    clicked = Some(pin.thread);
                }
            }
        });
    if let Some(thread) = clicked {
        state.select_comment(thread, false);
    }
}

/// One pin: a filled circle in its status colour with the thread's number, a
/// ring when selected, and the comment's opening on hover.
fn paint_pin(
    ui: &mut egui::Ui,
    thread: usize,
    center: egui::Pos2,
    opacity: f32,
    selected: bool,
    state: &UiState,
) -> egui::Response {
    let radius = size::COMMENT_PIN_RADIUS;
    let rect = egui::Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0));
    let response = ui.interact(
        rect,
        egui::Id::new(("comment_pin", thread)),
        egui::Sense::click(),
    );
    let Some(entry) = state.comments.threads.get(thread) else {
        return response;
    };
    let fill = match entry.thread.status {
        review_annotate::thread::Status::Open => color::COMMENT_PIN_OPEN,
        review_annotate::thread::Status::Resolved => color::COMMENT_PIN_RESOLVED,
    };
    let painter = ui.painter();
    if selected {
        painter.circle_stroke(
            center,
            radius + size::COMMENT_PIN_RING_GAP,
            egui::Stroke::new(
                size::COMMENT_PIN_STROKE,
                color::COMMENT_PIN_SELECTED.gamma_multiply(opacity),
            ),
        );
    }
    painter.circle(
        center,
        radius,
        fill.gamma_multiply(opacity),
        egui::Stroke::new(
            size::COMMENT_PIN_STROKE,
            color::COMMENT_PIN_OUTLINE.gamma_multiply(opacity),
        ),
    );
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        (thread + 1).to_string(),
        egui::FontId::proportional(font::COMMENT_PIN),
        color::COMMENT_PIN_TEXT.gamma_multiply(opacity),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let title = entry.thread.title().to_owned();
    let author = entry.thread.author().to_owned();
    response.on_hover_ui(|ui| {
        ui.label(egui::RichText::new(author).strong());
        ui.add(egui::Label::new(title).wrap());
    })
}

/// Where a surface pin's occlusion ray ends: just short of the pin, toward the
/// ray's origin.
fn lifted(world: Vec3, origin: Vec3, scene_size: f32) -> Vec3 {
    world + (origin - world).normalize_or_zero() * scene_size * SURFACE_LIFT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A surface pin's occlusion ray stops just short of the pin, toward the eye.
    #[test]
    fn a_surface_pin_is_lifted_toward_the_eye() {
        let lifted = lifted(Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0), 2.0);
        assert!(lifted.z > 0.0 && lifted.z < 0.01);
        assert_eq!((lifted.x, lifted.y), (0.0, 0.0));
    }
}
