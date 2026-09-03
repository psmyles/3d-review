//! Selection + per-mesh visibility geometry: the masked, per-material reordered
//! draw lists the solo/highlight views and the hidden-mesh filter share. All
//! build over fresh index buffers that reuse the steady-state mesh vertex buffer
//! (invariant 1).

use std::collections::HashMap;

use review_model::ModelData;

use crate::material::MaterialDrawRange;
use crate::selection::Selection;

use super::hidden::HiddenFilter;

/// The renderer-side geometry a selection needs: the selected triangles reordered
/// into a per-material draw list over a fresh index buffer that shares the mesh
/// vertex buffer. Used both by the solo (isolate) view to draw only the selection
/// and by the highlight flash, which redraws the same triangles as a flat color
/// fill on top of the mesh. `None` when nothing is selected or the model carries
/// no usable per-triangle node/material info. An empty draw list (a selection that
/// resolves to no triangles, e.g. an empty group node) still returns `Some` so the
/// caller stops rebuilding (invariant 3).
pub(crate) fn selection_geometry(
    model: &ModelData,
    selection: Selection,
    hidden_nodes: &[u32],
    tri_key: Option<&[u32]>,
) -> Option<(Vec<u32>, Vec<MaterialDrawRange>)> {
    let mut mask = selected_triangle_mask(model, selection)?;
    // A hidden mesh isn't drawn, so its triangles must drop out of both the solo
    // list and the highlight flash — otherwise the flash floats in empty space
    // where the mesh would be. Clear the selected triangles owned by a hidden node.
    let hidden = HiddenFilter::new(model, hidden_nodes);
    if hidden.is_active() {
        for (triangle, selected) in mask.iter_mut().enumerate() {
            *selected &= !hidden.is_hidden(triangle);
        }
    }
    Some(selection_mesh(model, &mask, tri_key))
}

/// The renderer-side geometry for per-mesh visibility: every triangle whose owning
/// node is *not* in `hidden_nodes`, reordered into a per-material draw list over a
/// fresh index buffer that shares the mesh vertex buffer (the same shape as
/// [`selection_geometry`], built once when the hidden set changes). `hidden_nodes`
/// are mesh-node indices the Outliner has unchecked. Returns `None` when nothing is
/// hidden (the full mesh is drawn) or the model carries no per-triangle node info
/// (so visibility can't be resolved — draw everything). When *every* mesh is hidden
/// the draw list is empty (`Some` with no ranges), so the caller draws nothing.
pub(crate) fn visible_geometry(
    model: &ModelData,
    hidden_nodes: &[u32],
    tri_key: Option<&[u32]>,
) -> Option<(Vec<u32>, Vec<MaterialDrawRange>)> {
    let hidden = HiddenFilter::new(model, hidden_nodes);
    if !hidden.is_active() {
        return None;
    }
    let mask: Vec<bool> = (0..model.indices.len() / 3)
        .map(|triangle| !hidden.is_hidden(triangle))
        .collect();
    Some(selection_mesh(model, &mask, tri_key))
}

/// A per-triangle boolean mask of the triangles `selection` covers, or `None` when
/// nothing is selected (or the model lacks the parallel arrays the selection
/// needs). A node selection includes the node's subtree; a material selection
/// every triangle of that slot.
pub(crate) fn selected_triangle_mask(model: &ModelData, selection: Selection) -> Option<Vec<bool>> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return None;
    }
    match selection {
        Selection::None => None,
        Selection::Material(slot) => {
            if model.triangles.material.len() != triangle_count {
                return None;
            }
            let slot = slot as u32;
            Some(
                model
                    .triangles
                    .material
                    .iter()
                    .map(|&m| m == slot)
                    .collect(),
            )
        }
        Selection::Node(node) => {
            if model.triangles.node.len() != triangle_count {
                return None;
            }
            let subtree = model.node_subtree_mask(node);
            Some(
                model
                    .triangles
                    .node
                    .iter()
                    .map(|&n| subtree.get(n as usize).copied().unwrap_or(false))
                    .collect(),
            )
        }
    }
}

/// Reorder the masked triangles' indices grouped by `tri_key` (the Unique
/// mesh-part index) when given, else by material slot — one [`MaterialDrawRange`]
/// per group, in first-seen order — over a fresh index buffer that shares the
/// steady-state mesh vertex buffer. Used by the solo view to draw only the
/// selection while still feeding each group's uniform; the grouping must match the
/// main mesh's so a range's `material` field indexes the same effective table.
fn selection_mesh(
    model: &ModelData,
    mask: &[bool],
    tri_key: Option<&[u32]>,
) -> (Vec<u32>, Vec<MaterialDrawRange>) {
    let triangle_count = model.indices.len() / 3;
    // Same grouping precedence as `material_draw_ranges`: explicit key, else the
    // per-triangle material slot, else a single group (slot 0).
    let key = tri_key
        .filter(|key| key.len() == triangle_count)
        .or_else(|| {
            (model.triangles.material.len() == triangle_count)
                .then_some(model.triangles.material.as_slice())
        });
    let mut order: Vec<u32> = Vec::new();
    let mut groups: HashMap<u32, Vec<usize>> = HashMap::new();
    for triangle in 0..triangle_count {
        if !mask.get(triangle).copied().unwrap_or(false) {
            continue;
        }
        let slot = key.map(|key| key[triangle]).unwrap_or(0);
        groups
            .entry(slot)
            .or_insert_with(|| {
                order.push(slot);
                Vec::new()
            })
            .push(triangle);
    }

    let mut indices = Vec::new();
    let mut ranges = Vec::with_capacity(order.len());
    for slot in order {
        let triangles = &groups[&slot];
        let first_index = indices.len() as u32;
        for &triangle in triangles {
            let base = triangle * 3;
            indices.extend_from_slice(&model.indices[base..base + 3]);
        }
        ranges.push(MaterialDrawRange {
            material: slot,
            first_index,
            index_count: (triangles.len() * 3) as u32,
        });
    }
    (indices, ranges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3, Vec4};
    use review_model::{TriangleData, Vertex};

    fn corner(index: usize) -> Vertex {
        Vertex {
            position: Vec3::new(index as f32, 0.0, 0.0),
            normal: Vec3::Y,
            uv: Vec2::ZERO,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            vertex_color: Vec4::ONE,
        }
    }

    /// A material selection isolates exactly that slot's triangles into a single
    /// draw range (the solo list), and outlines them.
    #[test]
    fn selection_geometry_isolates_material() {
        let tri_material = vec![0u32, 0, 2, 1, 2];
        let triangle_count = tri_material.len();
        let vertices: Vec<Vertex> = (0..triangle_count * 3).map(corner).collect();
        let indices: Vec<u32> = (0..(triangle_count * 3) as u32).collect();
        let model = ModelData {
            vertices,
            indices,
            triangles: TriangleData {
                material: tri_material,
                ..Default::default()
            },
            ..Default::default()
        };

        let (_, ranges) = selection_geometry(&model, Selection::Material(2), &[], None).unwrap();
        // Material 2 owns triangles {2, 4}: one range, six indices.
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].material, 2);
        assert_eq!(ranges[0].index_count, 6);
        assert_eq!(ranges[0].first_index, 0);

        // Nothing selected -> no geometry.
        assert!(selection_geometry(&model, Selection::None, &[], None).is_none());
    }

    /// A node selection covers the node's subtree: selecting a parent isolates the
    /// parent's and the child's triangles; selecting the child isolates only its.
    #[test]
    fn selection_geometry_covers_node_subtree() {
        use review_model::SceneNode;
        // 3 triangles: triangles 0,1 belong to node 0 (root), triangle 2 to node 1
        // (a child of node 0). One material throughout.
        let tri_node = vec![0u32, 0, 1];
        let triangle_count = tri_node.len();
        let vertices: Vec<Vertex> = (0..triangle_count * 3).map(corner).collect();
        let indices: Vec<u32> = (0..(triangle_count * 3) as u32).collect();
        let nodes = vec![
            SceneNode {
                name: "root".into(),
                parent: None,
                mesh_part: Some(0),
                source_vertex_count: 0,
                transform: glam::Mat4::IDENTITY,
                rest_local: Default::default(),
                kind: review_model::NodeKind::Mesh,
                bone: None,
            },
            SceneNode {
                name: "child".into(),
                parent: Some(0),
                mesh_part: Some(1),
                source_vertex_count: 0,
                transform: glam::Mat4::IDENTITY,
                rest_local: Default::default(),
                kind: review_model::NodeKind::Mesh,
                bone: None,
            },
        ];
        let model = ModelData {
            vertices,
            indices,
            triangles: TriangleData {
                node: tri_node,
                ..Default::default()
            },
            nodes,
            ..Default::default()
        };

        // Root subtree = both nodes -> all three triangles (9 indices).
        let (root_indices, _) = selection_geometry(&model, Selection::Node(0), &[], None).unwrap();
        assert_eq!(root_indices.len(), 9);
        // Child only -> just triangle 2 (3 indices).
        let (child_indices, _) = selection_geometry(&model, Selection::Node(1), &[], None).unwrap();
        assert_eq!(child_indices.len(), 3);
        // Hiding the child drops its triangle from the parent's selection list.
        let (root_visible, _) = selection_geometry(&model, Selection::Node(0), &[1], None).unwrap();
        assert_eq!(root_visible.len(), 6);
    }

    /// Per-mesh visibility drops the hidden node's triangles and keeps the rest;
    /// an empty hidden set returns `None` (draw the full mesh) and hiding every
    /// mesh yields an empty draw list (draw nothing).
    #[test]
    fn visible_geometry_drops_hidden_nodes() {
        // 3 triangles: 0,1 owned by node 0, triangle 2 by node 1. One material.
        let tri_node = vec![0u32, 0, 1];
        let triangle_count = tri_node.len();
        let vertices: Vec<Vertex> = (0..triangle_count * 3).map(corner).collect();
        let indices: Vec<u32> = (0..(triangle_count * 3) as u32).collect();
        let model = ModelData {
            vertices,
            indices,
            triangles: TriangleData {
                node: tri_node,
                ..Default::default()
            },
            ..Default::default()
        };

        // Nothing hidden -> draw the full mesh (no filtered buffer).
        assert!(visible_geometry(&model, &[], None).is_none());

        // Hide node 0 -> only node 1's single triangle (3 indices) survives.
        let (visible_indices, _) = visible_geometry(&model, &[0], None).unwrap();
        assert_eq!(visible_indices.len(), 3);

        // Hide both nodes -> empty draw list (the caller draws nothing).
        let (none_visible, ranges) = visible_geometry(&model, &[0, 1], None).unwrap();
        assert!(none_visible.is_empty());
        assert!(ranges.is_empty());
    }
}
