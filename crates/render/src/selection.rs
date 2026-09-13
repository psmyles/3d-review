//! Selection state carried from the UI into the scene pass.
//!
//! The UI owns the selection (set by clicking an Outliner row) and passes it,
//! plus the solo flag, a highlight color, and the flash fade, into the scene
//! renderer each frame (invariant 2). The renderer builds the selected-triangle
//! index buffer (an isolate/solo draw list, reused as the highlight flash's fill
//! source) on demand and frees it on deselect (invariant 3).

use review_model::{Bounds, ModelData};

use crate::geometry::selected_triangle_mask;

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

/// Axis-aligned bounds over exactly the geometry the selection covers — each
/// selected node's subtree, or every triangle of a material slot — matching the
/// triangles the
/// viewport highlight isolates (so the "only selection" bounding box and the
/// frame-on-selection camera wrap precisely what's highlighted). `None` when
/// nothing is selected, the model lacks the per-triangle arrays the selection
/// needs, or the selection resolves to no geometry (e.g. an empty group node).
///
/// `selected_nodes` carries the whole selected set; pass `&[]` to measure the
/// `selection` scalar alone.
pub fn selection_bounds(
    model: &ModelData,
    selection: Selection,
    selected_nodes: &[u32],
) -> Option<Bounds> {
    let mask = selected_triangle_mask(model, selection, selected_nodes)?;
    let mut bounds = Bounds::EMPTY;
    for (triangle, &included) in mask.iter().enumerate() {
        if !included {
            continue;
        }
        let base = triangle * 3;
        for &corner in &model.indices[base..base + 3] {
            if let Some(vertex) = model.vertices.get(corner as usize) {
                bounds.include_point(vertex.position);
            }
        }
    }
    (!bounds.is_empty()).then_some(bounds)
}

/// The selection view carried into the scene callback each frame: what is
/// selected, whether to isolate it (solo), and the highlight color the UI
/// sources from its theme (gamma-space RGB + the fill's opacity).
///
/// The viewport highlight is a flat color *fill* over the selected geometry, and
/// it **persists** for as long as the selection does — `highlight_color`'s own
/// alpha is the level it holds, which is what lets a user look away and still
/// see what they picked.
///
/// It is deliberately not animated. An earlier revision opened every selection
/// with a half-second flash that faded to this level; once the highlight stayed
/// up, that blink was a delay before the answer rather than a part of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionView {
    pub selection: Selection,
    pub solo: bool,
    /// Gamma-space RGB, with alpha the fill's opacity (the UI sources both from
    /// its theme).
    pub highlight_color: [f32; 4],
}

impl Default for SelectionView {
    fn default() -> Self {
        Self {
            selection: Selection::None,
            solo: false,
            highlight_color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}
