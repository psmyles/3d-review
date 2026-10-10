//! Which imported scene node each FBX `Model` object became.
//!
//! The viewer's scene graph comes from ufbx, which does not keep the file's
//! object ids, and does not keep the file's order either: it lists nodes by depth,
//! every parent before any child (measured on the rigged fixture, where the file
//! interleaves a skeleton with the meshes it drives). It also adds nodes the file
//! never had — the scene root, and the helper nodes its inherit-mode handling
//! inserts.
//!
//! What it does keep is the hierarchy, each node's name, and the file's order
//! among a parent's children. So the mapping walks both trees from the root, and
//! pairs each parent's children by name — the *k*-th child named `x` on one side
//! with the *k*-th child named `x` on the other — skipping the importer's
//! synthetic nodes as if their children hung from the nearest real ancestor. A
//! node with no counterpart maps to nothing, and the caller treats a comment on
//! it as unattached rather than guessing.

use std::collections::HashMap;

use crate::fbx::ModelObject;

/// One node of the importer's scene graph, as the mapping needs it.
#[derive(Clone, Copy, Debug)]
pub struct ImportedNode<'a> {
    pub name: &'a str,
    /// Index of the parent node, `None` for a root.
    pub parent: Option<usize>,
    /// `false` for a node the importer made up (the scene root, a helper).
    pub real: bool,
}

/// For each imported node, the index into `models` of the `Model` it was
/// imported from: `None` for a synthetic node, and for one with no counterpart.
pub fn map_nodes(models: &[ModelObject], nodes: &[ImportedNode<'_>]) -> Vec<Option<usize>> {
    let mut mapped = vec![None; nodes.len()];

    // The imported tree with its synthetic nodes elided: each real node under
    // its nearest real ancestor, in the importer's order.
    let mut imported_children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if node.real {
            imported_children
                .entry(real_ancestor(nodes, index))
                .or_default()
                .push(index);
        }
    }

    // The file's tree, in file order.
    let by_id: HashMap<i64, usize> = models
        .iter()
        .enumerate()
        .map(|(index, model)| (model.id, index))
        .collect();
    let mut file_children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for (index, model) in models.iter().enumerate() {
        let parent = model.parent.and_then(|id| by_id.get(&id).copied());
        file_children.entry(parent).or_default().push(index);
    }

    let mut queue: Vec<(Option<usize>, Option<usize>)> = vec![(None, None)];
    while let Some((imported_parent, file_parent)) = queue.pop() {
        let (Some(imported), Some(file)) = (
            imported_children.get(&imported_parent),
            file_children.get(&file_parent),
        ) else {
            continue;
        };
        // How many of each name have been paired under this parent so far.
        let mut taken: HashMap<&str, usize> = HashMap::new();
        for &node in imported {
            let name = nodes[node].name;
            let occurrence = taken.entry(name).or_default();
            let counterpart = file
                .iter()
                .filter(|&&model| models[model].name == name)
                .nth(*occurrence)
                .copied();
            *occurrence += 1;
            if let Some(model) = counterpart {
                mapped[node] = Some(model);
                queue.push((Some(node), Some(model)));
            }
        }
    }
    mapped
}

/// The nearest real ancestor of `index`, guarding against a malformed cycle.
fn real_ancestor(nodes: &[ImportedNode<'_>], index: usize) -> Option<usize> {
    let mut current = nodes[index].parent;
    for _ in 0..=nodes.len() {
        match current {
            Some(parent) if parent < nodes.len() && !nodes[parent].real => {
                current = nodes[parent].parent;
            }
            Some(parent) if parent < nodes.len() => return Some(parent),
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: i64, name: &str, parent: Option<i64>) -> ModelObject {
        ModelObject {
            id,
            name: name.to_owned(),
            class: "Null".to_owned(),
            parent,
        }
    }

    fn node(name: &str, parent: Option<usize>, real: bool) -> ImportedNode<'_> {
        ImportedNode { name, parent, real }
    }

    #[test]
    fn a_reordered_hierarchy_maps_by_parent_and_name() {
        // File order interleaves the two branches; the importer lists by depth
        // under a synthetic root.
        let models = [
            model(10, "Hips", None),
            model(11, "Spine", Some(10)),
            model(20, "Body", None),
            model(12, "Head", Some(11)),
        ];
        let nodes = [
            node("", None, false),
            node("Hips", Some(0), true),
            node("Body", Some(0), true),
            node("Spine", Some(1), true),
            node("Head", Some(3), true),
        ];
        assert_eq!(
            map_nodes(&models, &nodes),
            [None, Some(0), Some(2), Some(1), Some(3)]
        );
    }

    #[test]
    fn same_named_siblings_pair_in_order_and_helpers_are_transparent() {
        let models = [
            model(1, "Root", None),
            model(2, "Leaf", Some(1)),
            model(3, "Leaf", Some(1)),
        ];
        // A scale helper between Root and the second Leaf.
        let nodes = [
            node("Root", None, true),
            node("Leaf", Some(0), true),
            node("helper", Some(0), false),
            node("Leaf", Some(2), true),
        ];
        assert_eq!(
            map_nodes(&models, &nodes),
            [Some(0), Some(1), None, Some(2)]
        );
    }

    #[test]
    fn a_node_without_a_counterpart_maps_to_nothing() {
        let models = [model(1, "A", None)];
        let nodes = [node("B", None, true)];
        assert_eq!(map_nodes(&models, &nodes), [None]);
    }
}
