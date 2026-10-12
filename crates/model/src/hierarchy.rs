//! The authored hierarchy, as opposed to the one import hands over.
//!
//! ufbx adds nodes of its own: a root holding the file's unit scale, and scale
//! helpers for non-standard inherit modes. Anything that reports on the scene
//! the artist built — the exporter, the audit — has to see past them.

use crate::ModelData;
use crate::extras::{SourceExtras, Synthetic};

/// Whether node `index` is one the importer made up rather than the file
/// authored: the synthetic file root and the scale helpers ufbx inserts. Falls
/// back to a structural test (a parentless, unnamed, mesh-less node) when no
/// source-property capture is available.
pub fn is_synthetic(model: &ModelData, extras: Option<&SourceExtras>, index: usize) -> bool {
    match extras
        .and_then(|extras| extras.nodes.get(index))
        .map(|node| node.synthetic)
    {
        Some(Synthetic::Root | Synthetic::ScaleHelper) => true,
        Some(_) => false,
        None => model.nodes.get(index).is_some_and(|node| {
            node.parent.is_none() && node.name.is_empty() && node.mesh_part.is_none()
        }),
    }
}

/// The nearest authored ancestor of `index`, skipping synthetic nodes.
pub fn authored_parent(
    model: &ModelData,
    extras: Option<&SourceExtras>,
    index: usize,
) -> Option<usize> {
    let mut cursor = model.nodes.get(index)?.parent;
    let mut steps = 0;
    while let Some(parent) = cursor {
        if !is_synthetic(model, extras, parent) {
            return Some(parent);
        }
        cursor = model.nodes.get(parent)?.parent;
        steps += 1;
        if steps > model.nodes.len() {
            return None;
        }
    }
    None
}

/// Every authored node with no authored parent — the file's own top level.
pub fn authored_roots(model: &ModelData, extras: Option<&SourceExtras>) -> Vec<usize> {
    (0..model.nodes.len())
        .filter(|&index| {
            !is_synthetic(model, extras, index) && authored_parent(model, extras, index).is_none()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SceneNode;

    fn node(name: &str, parent: Option<usize>) -> SceneNode {
        SceneNode {
            name: name.to_owned(),
            parent,
            ..Default::default()
        }
    }

    #[test]
    fn the_unnamed_root_is_seen_through() {
        let model = ModelData {
            nodes: vec![
                node("", None),
                node("Body", Some(0)),
                node("Arm", Some(1)),
                node("Prop", Some(0)),
            ],
            ..Default::default()
        };
        assert!(is_synthetic(&model, None, 0));
        assert_eq!(authored_parent(&model, None, 1), None);
        assert_eq!(authored_parent(&model, None, 2), Some(1));
        assert_eq!(authored_roots(&model, None), vec![1, 3]);
    }
}
