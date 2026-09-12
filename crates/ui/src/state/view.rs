//! Which viewport is showing, and how its camera is pointed.

use glam::{Vec2, Vec3};
use review_render::CameraProjection;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMode {
    #[default]
    ThreeD,
    Uv,
    Texture,
    /// Mesh optimization: the same 3D scene chrome, plus the operation stack and
    /// a source-vs-processed comparison viewport.
    Opt,
}

impl WorkspaceMode {
    /// True for the workspaces that draw the 3D scene and therefore share its
    /// chrome — side panels, option windows, the axis gizmo, the stats overlay,
    /// and every shading / diagnostic control. Opt is a 3D workspace with extra
    /// tooling, not a separate kind of viewport.
    pub fn is_scene(self) -> bool {
        matches!(self, WorkspaceMode::ThreeD | WorkspaceMode::Opt)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewProjectionMode {
    Perspective,
    Orthographic,
}

impl From<ViewProjectionMode> for CameraProjection {
    fn from(value: ViewProjectionMode) -> Self {
        match value {
            ViewProjectionMode::Perspective => Self::Perspective,
            ViewProjectionMode::Orthographic => Self::Orthographic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewAxis {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

impl ViewAxis {
    pub fn offset_direction(self) -> Vec3 {
        match self {
            Self::PositiveX => Vec3::X,
            Self::NegativeX => Vec3::NEG_X,
            Self::PositiveY => Vec3::Y,
            Self::NegativeY => Vec3::NEG_Y,
            Self::PositiveZ => Vec3::Z,
            Self::NegativeZ => Vec3::NEG_Z,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisGizmoAction {
    Orbit(Vec2),
    Snap(ViewAxis),
    /// Return the camera to its starting "home" view (the gizmo's reset button).
    ResetView,
}
