//! The Outliner's hidden-mesh set, resolved once against a model's per-triangle
//! node info.
//!
//! Every geometry builder that can skip a hidden mesh needs the same two things: a
//! set membership test over the hidden node indices, and the guard that
//! `model.triangles.node` actually runs parallel to the triangle list. The guard is
//! the load-bearing half — it is what keeps the per-triangle lookup in range — so it
//! lives here rather than being restated at each builder.

use std::collections::HashSet;

use review_model::ModelData;

/// A resolved hidden-mesh filter: which scene-graph nodes are hidden, and the
/// per-triangle owner array to resolve a triangle through.
pub(crate) struct HiddenFilter<'a> {
    hidden: HashSet<u32>,
    /// Per-triangle owning node. `None` — meaning "hide nothing" — when the set is
    /// empty, when the model has no triangles, or when the array does not run
    /// parallel to the triangle list, in which case visibility simply can't be
    /// resolved and every triangle is drawn. Storing it only in the sound case is
    /// what makes [`Self::is_hidden`] unable to index out of range.
    triangle_node: Option<&'a [u32]>,
}

impl<'a> HiddenFilter<'a> {
    /// Resolve `hidden_nodes` against `model`'s per-triangle node info.
    pub(crate) fn new(model: &'a ModelData, hidden_nodes: &[u32]) -> Self {
        let triangle_count = model.indices.len() / 3;
        let usable = !hidden_nodes.is_empty()
            && triangle_count != 0
            && model.triangles.node.len() == triangle_count;
        Self {
            // An empty `HashSet` allocates nothing, so the common "nothing hidden"
            // case costs only the length checks above.
            hidden: if usable {
                hidden_nodes.iter().copied().collect()
            } else {
                HashSet::new()
            },
            triangle_node: usable.then_some(model.triangles.node.as_slice()),
        }
    }

    /// Whether any triangle can be hidden at all — `false` when nothing is hidden or
    /// the model can't resolve visibility, which lets a caller skip building a mask.
    pub(crate) fn is_active(&self) -> bool {
        self.triangle_node.is_some()
    }

    /// Whether triangle `index` belongs to a hidden mesh.
    pub(crate) fn is_hidden(&self, index: usize) -> bool {
        self.triangle_node
            .and_then(|nodes| nodes.get(index))
            .is_some_and(|node| self.hidden.contains(node))
    }

    /// Whether scene-graph node `node` is hidden. For the callers that resolve a
    /// node themselves — the wireframe works per *face*, through its own
    /// face-to-node map — rather than per triangle.
    pub(crate) fn contains_node(&self, node: u32) -> bool {
        self.hidden.contains(&node)
    }
}

/// A per-vertex visibility mask: `true` for every vertex referenced by a triangle
/// whose owning node is *not* hidden. `None` when nothing is hidden or the model
/// carries no per-triangle node info (so visibility can't be resolved and every
/// vertex is drawn).
pub(super) fn visible_vertex_mask(model: &ModelData, hidden_nodes: &[u32]) -> Option<Vec<bool>> {
    let hidden = HiddenFilter::new(model, hidden_nodes);
    if !hidden.is_active() {
        return None;
    }
    let mut mask = vec![false; model.vertices.len()];
    for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        if hidden.is_hidden(triangle_index) {
            continue;
        }
        for &corner in triangle {
            if let Some(slot) = mask.get_mut(corner as usize) {
                *slot = true;
            }
        }
    }
    Some(mask)
}

/// Every visible triangle's three corners, in triangle order — the whole index list,
/// borrowed, when nothing is hidden.
pub(crate) fn visible_triangle_indices<'a>(
    model: &'a ModelData,
    hidden_nodes: &[u32],
) -> std::borrow::Cow<'a, [u32]> {
    let hidden = HiddenFilter::new(model, hidden_nodes);
    if !hidden.is_active() {
        return std::borrow::Cow::Borrowed(&model.indices);
    }
    std::borrow::Cow::Owned(
        model
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .enumerate()
            .filter(|&(triangle, _)| !hidden.is_hidden(triangle))
            .flat_map(|(_, corners)| corners.iter().copied())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_model::TriangleData;

    /// A model of `triangle_count` triangles whose per-triangle node array is
    /// `nodes` — deliberately allowed to disagree in length, which is the case the
    /// guard exists for.
    fn model(triangle_count: usize, nodes: Vec<u32>) -> ModelData {
        ModelData {
            indices: (0..(triangle_count * 3) as u32).collect(),
            triangles: TriangleData {
                node: nodes,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn resolves_hidden_triangles_through_their_owning_node() {
        let model = model(3, vec![0, 1, 0]);
        let filter = HiddenFilter::new(&model, &[0]);
        assert!(filter.is_active());
        assert!(filter.contains_node(0));
        assert!(!filter.contains_node(1));
        assert!(filter.is_hidden(0));
        assert!(!filter.is_hidden(1));
        assert!(filter.is_hidden(2));
    }

    /// Nothing hidden means nothing to resolve, so the per-triangle lookup is inert
    /// and no set is built.
    #[test]
    fn an_empty_hidden_set_hides_nothing() {
        let model = model(3, vec![0, 1, 0]);
        let filter = HiddenFilter::new(&model, &[]);
        assert!(!filter.is_active());
        assert!(!filter.is_hidden(0));
        assert!(!filter.contains_node(0));
    }

    /// The guard the helper exists for: a per-triangle node array that doesn't run
    /// parallel to the triangles can't resolve visibility, so every triangle draws
    /// — and, crucially, a query past the array's end answers rather than panicking.
    #[test]
    fn a_mismatched_node_array_hides_nothing_and_stays_in_range() {
        // Three triangles, but only two node entries.
        let short = model(3, vec![0, 0]);
        let filter = HiddenFilter::new(&short, &[0]);
        assert!(!filter.is_active());
        assert!(!filter.is_hidden(0));
        assert!(!filter.is_hidden(2));

        // And a model with no per-triangle node info at all.
        let untagged = model(3, Vec::new());
        let filter = HiddenFilter::new(&untagged, &[0]);
        assert!(!filter.is_active());
        assert!(!filter.is_hidden(0));
    }

    /// An in-range filter still answers past the end of the triangle list rather
    /// than indexing out of bounds.
    #[test]
    fn a_triangle_past_the_end_is_not_hidden() {
        let model = model(2, vec![0, 0]);
        let filter = HiddenFilter::new(&model, &[0]);
        assert!(filter.is_active());
        assert!(filter.is_hidden(1));
        assert!(!filter.is_hidden(2));
    }
}
