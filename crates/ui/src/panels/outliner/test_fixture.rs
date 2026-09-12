//! The scene the Outliner's tests are written against.

#![cfg(test)]

use super::*;
use review_model::{ModelStats, SceneNode};

/// A tiny scene:
///
/// ```text
/// 0 root (Empty)
/// ├── 1 mesh   (Mesh)
/// └── 2 hips   (Bone)
///     ├── 3 spine (Bone)
///     └── 4 lamp  (Light)
/// ```
pub(super) fn scene() -> ModelData {
    let node = |name: &str, parent: Option<usize>, kind: NodeKind| SceneNode {
        name: name.to_owned(),
        parent,
        mesh_part: (kind == NodeKind::Mesh).then_some(0),
        source_vertex_count: 0,
        transform: glam::Mat4::IDENTITY,
        rest_local: Default::default(),
        kind,
        bone: (kind == NodeKind::Bone).then(Default::default),
    };
    ModelData {
        nodes: vec![
            node("root", None, NodeKind::Empty),
            node("mesh", Some(0), NodeKind::Mesh),
            node("hips", Some(0), NodeKind::Bone),
            node("spine", Some(2), NodeKind::Bone),
            node("lamp", Some(2), NodeKind::Light),
        ],
        stats: ModelStats {
            gpu_vertex_count: 0,
            bone_count: 2,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Build the adjacency the way `OutlinerState::ensure_tree` does, without
/// needing a whole `UiState`.
fn adjacency(model: &ModelData) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut children = vec![Vec::new(); model.nodes.len()];
    let mut roots = Vec::new();
    for (index, node) in model.nodes.iter().enumerate() {
        match node.parent {
            Some(parent) if parent < model.nodes.len() && parent != index => {
                children[parent].push(index)
            }
            _ => roots.push(index),
        }
    }
    (children, roots)
}

pub(super) fn rows(model: &ModelData, collapsed: &[usize], hidden: &[NodeKind]) -> Vec<TreeRow> {
    let (children, roots) = adjacency(model);
    visible_tree_rows(
        model,
        &children,
        &roots,
        &collapsed.iter().copied().collect(),
        &hidden.iter().copied().collect(),
    )
}
