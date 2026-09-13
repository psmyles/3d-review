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

/// What the left mouse button does in the 3D viewport.
///
/// An explicit mode rather than a modifier, because the two jobs want the same
/// gesture: reviewing a model is mostly camera work, and a viewer that selected
/// something every time a drag ended would fight the user. In
/// [`ViewportTool::Select`] a *click* (a press and release that does not move)
/// picks, while a drag still orbits exactly as it always did — so the camera is
/// never taken away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ViewportTool {
    /// Camera only: the viewer's long-standing behaviour.
    #[default]
    View,
    /// Clicking picks what is under the pointer, and hovering previews it.
    Select,
}

impl ViewportTool {
    /// The other tool — what the toolbar button and `Q` switch to.
    pub fn toggled(self) -> Self {
        match self {
            Self::View => Self::Select,
            Self::Select => Self::View,
        }
    }
}

/// What the pointer is over in the viewport, resolved by `app`'s pick each time
/// the pointer moves and read back by the renderer as a highlight (data flows
/// app→UI, invariant 2).
///
/// A bone and a mesh part are never both hovered: the skeleton overlay decides
/// which of the two is pickable at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverTarget {
    /// A mesh or group node (an index into `ModelData::nodes`).
    Node(usize),
    /// A bone node, while the skeleton overlay is up.
    Bone(usize),
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
