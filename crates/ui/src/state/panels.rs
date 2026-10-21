//! One state struct per option tool, and the defaults a reset returns to.
//!
//! Each of these pairs with a file in `panels/`, and carries the values that tool
//! edits. The widgets write them in place — there is no separate commit step —
//! and `sync_debug_state` copies them each frame into the `SceneDebugOptions` the
//! renderer reads.

use review_render::{CheckerTexture, TrajectoryStyle, VertexColorMode};

use crate::theme;

/// Default checker repeats across the 0..1 UV range for a fresh / reset panel.
pub(crate) const DEFAULT_CHECKER_TILING: u32 = 4;

/// Default length for the face/vertex normal-line views, as a fraction of the
/// model's largest bounding extent (see `debug_normal_length` in `review-render`).
pub(crate) const DEFAULT_NORMAL_LENGTH: f32 = 0.03;

/// Default multiplier for the skeleton overlay's bone size.
pub(crate) const DEFAULT_SKELETON_SCALE: f32 = 1.0;

/// Default width of the model wireframe's lines, in points (the renderer scales it
/// by the display, so this is one pixel at 100% and two on a Retina screen).
pub(crate) const DEFAULT_WIREFRAME_WIDTH: f32 = 1.0;

#[derive(Debug, Clone)]
pub struct NormalPanelState {
    pub length: f32,
    pub color: egui::Color32,
}

/// Editable state backing the UV Seams options panel: the edge color, and which
/// of the model's UV sets the seam test reads. The channel is the seam view's own
/// rather than the UV checker's — which set is cut is a different question from
/// which set the checker is showing, and reading them side by side is the point.
#[derive(Debug, Clone)]
pub struct UvSeamPanelState {
    pub color: egui::Color32,
    /// 0-based index into [`UiState::uv_sets`]; clamped back to 0 when a reload
    /// leaves it past the end.
    pub uv_channel: u32,
}

impl Default for UvSeamPanelState {
    fn default() -> Self {
        Self {
            color: theme::color::UV_SEAM_DEFAULT,
            uv_channel: 0,
        }
    }
}

/// Editable state backing the Skeleton options panel. The renderer bakes both
/// values into the skeleton overlay's vertex buffers via [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct SkeletonPanelState {
    /// Multiplier on the computed bone thickness / joint-marker size.
    pub scale: f32,
    pub color: egui::Color32,
}

impl Default for SkeletonPanelState {
    fn default() -> Self {
        Self {
            scale: DEFAULT_SKELETON_SCALE,
            color: theme::color::SKELETON_DEFAULT,
        }
    }
}

/// How an animation curve's line runs between its frame dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CurveShape {
    /// A smooth curve through the dots.
    Smooth,
    /// A straight line from each dot to the next (default).
    #[default]
    Straight,
}

impl CurveShape {
    pub const ALL: [CurveShape; 2] = [CurveShape::Smooth, CurveShape::Straight];

    /// The dropdown label for this shape.
    pub fn label(self) -> review_localization::Key {
        match self {
            CurveShape::Smooth => crate::keys::ui_enums::CURVE_SHAPE_SMOOTH,
            CurveShape::Straight => crate::keys::ui_enums::CURVE_SHAPE_STRAIGHT,
        }
    }
}

/// Editable state backing the Animation Curves options panel, handed to the
/// renderer as a [`TrajectoryStyle`] with the curves themselves. The shape
/// rebuilds them; the width, opacity and dot size are uniforms and rebuild
/// nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct CurvesPanelState {
    pub shape: CurveShape,
    /// The curve's width, in points.
    pub width: f32,
    /// The curve's opacity; the dots and markers stay opaque.
    pub opacity: f32,
    /// The size of the dot on each frame, in points.
    pub dot_size: f32,
}

impl Default for CurvesPanelState {
    /// The renderer's own default style, so the two cannot drift apart.
    fn default() -> Self {
        let style = TrajectoryStyle::default();
        Self {
            shape: if style.smooth {
                CurveShape::Smooth
            } else {
                CurveShape::Straight
            },
            width: style.width,
            opacity: style.opacity,
            dot_size: style.dot_size,
        }
    }
}

impl CurvesPanelState {
    /// What the renderer draws the curves with.
    pub fn style(&self) -> TrajectoryStyle {
        TrajectoryStyle {
            smooth: self.shape == CurveShape::Smooth,
            width: self.width,
            opacity: self.opacity,
            dot_size: self.dot_size,
        }
    }
}

/// Editable state backing the Wireframe options panel. The renderer applies both
/// values as uniforms at draw time via [`SceneDebugOptions`], so neither rebuilds
/// anything.
#[derive(Debug, Clone)]
pub struct WireframePanelState {
    pub color: egui::Color32,
    /// Line width in points.
    pub width: f32,
}

impl Default for WireframePanelState {
    fn default() -> Self {
        Self {
            color: theme::color::WIREFRAME_DEFAULT,
            width: DEFAULT_WIREFRAME_WIDTH,
        }
    }
}

/// Which geometry the bounding box wraps: the whole model, or only the meshes
/// the Outliner currently shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundsScope {
    /// Wrap every mesh, regardless of selection or Outliner visibility (default).
    #[default]
    AllMeshes,
    /// Wrap only the geometry the current Outliner selection covers.
    OnlySelection,
    /// Wrap only the currently-visible meshes (Outliner-hidden meshes excluded).
    VisibleOnly,
}

impl BoundsScope {
    pub const ALL: [BoundsScope; 3] = [
        BoundsScope::AllMeshes,
        BoundsScope::OnlySelection,
        BoundsScope::VisibleOnly,
    ];

    /// The dropdown label for this scope.
    pub fn label(self) -> review_localization::Key {
        match self {
            BoundsScope::AllMeshes => crate::keys::ui_enums::SCOPE_ALL_MESHES,
            BoundsScope::OnlySelection => crate::keys::ui_enums::SCOPE_ONLY_SELECTION,
            BoundsScope::VisibleOnly => crate::keys::ui_enums::SCOPE_VISIBLE_ONLY,
        }
    }
}

/// Editable state backing the Bounding Box options panel. The renderer bakes the
/// chosen color + scope into the bounding-box line buffer via [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct BoundingBoxPanelState {
    pub color: egui::Color32,
    /// Whether the box covers all meshes or only the visible ones.
    pub scope: BoundsScope,
}

impl Default for BoundingBoxPanelState {
    fn default() -> Self {
        Self {
            color: theme::color::BOUNDING_BOX_DEFAULT,
            scope: BoundsScope::default(),
        }
    }
}

/// Editable state backing the UV Checker options panel. The renderer reads the
/// committed values via [`SceneDebugOptions`]. `tiling` is edited directly on the
/// standard slider's inline value field, so no separate text buffer is needed.
#[derive(Debug, Clone)]
pub struct UvCheckerPanelState {
    pub texture: CheckerTexture,
    pub tiling: u32,
    pub uv_channel: u32,
}

impl Default for UvCheckerPanelState {
    fn default() -> Self {
        Self {
            texture: CheckerTexture::Greyscale,
            tiling: DEFAULT_CHECKER_TILING,
            uv_channel: 0,
        }
    }
}

/// Editable state backing the Vertex Color options panel. The toggle itself
/// lives in [`SceneDebugOptions::vertex_colors`] (like the UV checker); this
/// holds only the channel-display mode, synced via [`SceneDebugOptions`].
#[derive(Debug, Clone, Default)]
pub struct VertexColorPanelState {
    pub mode: VertexColorMode,
}

/// Editable state backing the Camera options panel: the 3D cameras' field of
/// view and how fast each camera gesture moves them.
///
/// Plain values, like every panel's (invariant 2): `app` hands the field of view
/// to the renderer each frame and scales its own gestures by the speeds. View
/// state, so none of it is undone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPanelState {
    /// Vertical field of view in degrees. Perspective only: a change dollies the
    /// camera to keep the framing, which leaves the orthographic view untouched.
    pub fov_degrees: f32,
    /// Multiplier on the left-button orbit, the right-button look and the axis
    /// gizmo's drag.
    pub orbit_speed: f32,
    /// Multiplier on the wheel, the Alt + right-button drag and the trackpad
    /// pinch, in the 3D views only.
    pub zoom_speed: f32,
    /// Multiplier on the right-button flycam's base speed. The same value the
    /// wheel adjusts mid-flight ([`CameraPanelState::scale_fly_speed`]).
    pub fly_speed: f32,
}

impl Default for CameraPanelState {
    fn default() -> Self {
        Self {
            fov_degrees: review_render::DEFAULT_FOV_Y_DEGREES,
            orbit_speed: 1.0,
            zoom_speed: 1.0,
            fly_speed: 1.0,
        }
    }
}

impl CameraPanelState {
    /// Scale the fly speed by `factor`, held to the slider's range — what the
    /// wheel does while the right button is down. One value for both doors onto
    /// it, so the slider shows wherever the wheel left it.
    pub fn scale_fly_speed(&mut self, factor: f32) {
        if factor.is_finite() {
            self.fly_speed = (self.fly_speed * factor).clamp(
                super::range::CAMERA_FLY_SPEED_MIN,
                super::range::CAMERA_FLY_SPEED_MAX,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::range::{CAMERA_FLY_SPEED_MAX, CAMERA_FLY_SPEED_MIN};
    use super::*;

    #[test]
    fn the_wheel_keeps_the_fly_speed_inside_the_slider() {
        let mut camera = CameraPanelState::default();
        camera.scale_fly_speed(2.0);
        assert_eq!(camera.fly_speed, 2.0);
        for _ in 0..100 {
            camera.scale_fly_speed(1.15);
        }
        assert_eq!(camera.fly_speed, CAMERA_FLY_SPEED_MAX);
        for _ in 0..200 {
            camera.scale_fly_speed(1.0 / 1.15);
        }
        assert_eq!(camera.fly_speed, CAMERA_FLY_SPEED_MIN);
        camera.scale_fly_speed(f32::NAN);
        assert_eq!(camera.fly_speed, CAMERA_FLY_SPEED_MIN);
    }
}
