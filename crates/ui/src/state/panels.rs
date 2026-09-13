//! One state struct per option tool, and the defaults a reset returns to.
//!
//! Each of these pairs with a file in `panels/`, and carries the *uncommitted*
//! slider values that tool is editing; `sync_debug_state` funnels the committed
//! ones into the `SceneDebugOptions` the renderer reads.

use review_render::{CheckerTexture, VertexColorMode};

use crate::theme;

/// Default checker repeats across the 0..1 UV range for a fresh / reset panel.
pub(crate) const DEFAULT_CHECKER_TILING: u32 = 4;

/// Default length for the face/vertex normal-line views, as a fraction of the
/// model's largest bounding extent (see `debug_normal_length` in `review-render`).
pub(crate) const DEFAULT_NORMAL_LENGTH: f32 = 0.03;

/// Default multiplier for the skeleton overlay's bone size.
pub(crate) const DEFAULT_SKELETON_SCALE: f32 = 1.0;

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

/// Editable state backing the Wireframe options panel. The renderer bakes the
/// chosen color into the final wireframe overlay line buffer via
/// [`SceneDebugOptions`].
#[derive(Debug, Clone)]
pub struct WireframePanelState {
    pub color: egui::Color32,
}

impl Default for WireframePanelState {
    fn default() -> Self {
        Self {
            color: theme::color::WIREFRAME_DEFAULT,
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
