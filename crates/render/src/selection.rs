//! Selection state carried from the UI into the scene pass (Phase 2).
//!
//! The UI owns the selection (set by clicking an Outliner row) and passes it,
//! plus the solo flag, a highlight color, and the flash fade, into the
//! [`SceneCallback`] each frame (invariant 2). The renderer builds the selected-
//! triangle index buffer (an isolate/solo draw list, reused as the highlight
//! flash's fill source) on demand and frees it on deselect (invariant 3).
//!
//! [`SceneCallback`]: crate::SceneCallback

/// What the user has selected in the Outliner, driving the viewport highlight and
/// the solo (isolate) filter. A [`Node`] selection covers the node's own mesh and
/// every descendant node's mesh; a [`Material`] selection covers every triangle of
/// that material slot.
///
/// [`Node`]: Selection::Node
/// [`Material`]: Selection::Material
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Selection {
    /// Nothing selected — no highlight, solo is a no-op.
    #[default]
    None,
    /// A scene-graph node (index into [`ModelData::nodes`]); selects its subtree.
    ///
    /// [`ModelData::nodes`]: review_model::ModelData::nodes
    Node(usize),
    /// A material slot (index into [`ModelData::materials`]).
    ///
    /// [`ModelData::materials`]: review_model::ModelData::materials
    Material(usize),
}

impl Selection {
    /// Whether anything is selected.
    pub fn is_active(self) -> bool {
        !matches!(self, Selection::None)
    }
}

/// The selection view carried into the scene callback each frame: what is
/// selected, whether to isolate it (solo), the highlight color the UI sources
/// from its theme (gamma-space RGB), and the flash fade.
///
/// The viewport highlight is a flat color *fill* over the selected geometry that
/// flashes on selection and fades out: `fade` runs 1→0 over the flash (driven by
/// `app`'s redraw loop, invariant 6), modulating the fill alpha. At `fade == 0`
/// the selection is still active (the Inspector stays populated, solo still
/// isolates) — only the flash overlay is gone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionView {
    pub selection: Selection,
    pub solo: bool,
    pub highlight_color: [f32; 4],
    /// Flash fade factor, 1 at the start of a selection flash down to 0 when it
    /// finishes. Multiplies the highlight fill's alpha each frame.
    pub fade: f32,
}

impl Default for SelectionView {
    fn default() -> Self {
        Self {
            selection: Selection::None,
            solo: false,
            highlight_color: [1.0, 1.0, 1.0, 1.0],
            fade: 0.0,
        }
    }
}
