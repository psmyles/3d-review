//! Collision meshes: the parts whose names match the Wireframe panel's collider
//! names, drawn as their edges, shaded like any part, or not at all.
//!
//! Which nodes match is worked out once per model and list of names, not per
//! frame: a scene of thousands of parts would otherwise test every name against
//! every pattern on every frame.

use review_audit::glob::GlobSet;
use review_ui::ColliderDisplay;

use crate::App;

/// The collision meshes of one model under one list of names.
#[derive(Debug, Default)]
pub(crate) struct ColliderCache {
    key: Option<(u64, Vec<String>)>,
    nodes: Vec<u32>,
}

impl App {
    /// The model's collision meshes: the nodes whose names match the collider
    /// names (sorted). None while the names hold a pattern that does not parse.
    fn collider_nodes(&mut self) -> &[u32] {
        let names = &self.ui.wireframe.collider_names;
        let fresh = self
            .colliders
            .key
            .as_ref()
            .is_some_and(|(revision, baked)| *revision == self.scene_revision && baked == names);
        if !fresh {
            self.colliders.nodes = match GlobSet::new(names) {
                Ok(set) if !names.is_empty() => self
                    .scene_model
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| set.matches(&node.name))
                    .map(|(index, _)| index as u32)
                    .collect(),
                _ => Vec::new(),
            };
            self.colliders.key = Some((self.scene_revision, names.clone()));
        }
        &self.colliders.nodes
    }

    /// The mesh nodes whose surfaces are not drawn or picked (sorted,
    /// deduplicated): the ones hidden in the Outliner, and the collision meshes
    /// unless they are shown shaded.
    pub(crate) fn surface_hidden_nodes(&mut self) -> Vec<u32> {
        let mut hidden = self.ui.hidden_mesh_nodes();
        if self.ui.wireframe.colliders != ColliderDisplay::Shaded {
            hidden.extend_from_slice(self.collider_nodes());
            hidden.sort_unstable();
            hidden.dedup();
        }
        hidden
    }

    /// The collision meshes drawn as their edges (sorted): all of them while
    /// they are shown as wireframe, but the ones the Outliner hides.
    pub(crate) fn wire_mesh_nodes(&mut self) -> Vec<u32> {
        if self.ui.wireframe.colliders != ColliderDisplay::Wireframe {
            return Vec::new();
        }
        let hidden = self.ui.hidden_mesh_nodes();
        self.collider_nodes()
            .iter()
            .copied()
            .filter(|node| hidden.binary_search(node).is_err())
            .collect()
    }
}
