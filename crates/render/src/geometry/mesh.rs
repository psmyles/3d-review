//! The shaded mesh build: per-channel scene vertices plus the per-material index
//! reorder (one [`MaterialDrawRange`] per material, vertices left untouched).

use std::collections::HashMap;

use review_model::ModelData;

use crate::material::MaterialDrawRange;
use crate::scene::SceneVertex;

/// The shaded mesh vertices + reordered indices for `uv_channel`, with one
/// [`MaterialDrawRange`] per material (the renderer draws one range at a time,
/// feeding each material's parameters from the group-3 uniform). Base color /
/// smoothness are no longer baked per vertex (Phase 1); only the DCC vertex-color
/// attribute rides along, for the vertex-color debug view.
pub(crate) fn model_mesh(
    model: &ModelData,
    uv_channel: u32,
) -> (Vec<SceneVertex>, Vec<u32>, Vec<MaterialDrawRange>) {
    let channel = uv_channel as usize;
    let vertices = model
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| SceneVertex {
            position: vertex.position.to_array(),
            normal: vertex.normal.to_array(),
            uv: model.uv_for_channel(index, channel).to_array(),
            tangent: vertex.tangent.to_array(),
            vertex_color: vertex.vertex_color.to_array(),
        })
        .collect();
    let (indices, ranges) = material_draw_ranges(model);
    (vertices, indices, ranges)
}

/// Reorder the mesh index buffer so each material's triangles are contiguous, and
/// return the per-material draw ranges. Vertices are left untouched — only index
/// order changes (invariant 1: [`ModelData`]'s buffers stay authoritative). Groups
/// in first-seen material order; falls back to a single range over the original
/// index order when the model carries no usable per-triangle material info. The
/// number of ranges matches [`ModelData::material_draw_count`].
fn material_draw_ranges(model: &ModelData) -> (Vec<u32>, Vec<MaterialDrawRange>) {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return (model.indices.clone(), Vec::new());
    }
    if model.triangles.material.len() != triangle_count {
        let ranges = vec![MaterialDrawRange {
            material: 0,
            first_index: 0,
            index_count: model.indices.len() as u32,
        }];
        return (model.indices.clone(), ranges);
    }

    let mut order: Vec<u32> = Vec::new();
    let mut groups: HashMap<u32, Vec<usize>> = HashMap::new();
    for (triangle, &slot) in model.triangles.material.iter().enumerate() {
        groups
            .entry(slot)
            .or_insert_with(|| {
                order.push(slot);
                Vec::new()
            })
            .push(triangle);
    }

    let mut indices = Vec::with_capacity(model.indices.len());
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

    /// `model_mesh` reorders indices into one contiguous range per material, in
    /// first-seen order, and the range count matches `material_draw_count`.
    #[test]
    fn model_mesh_groups_indices_by_material() {
        // 5 triangles (15 corner vertices); materials per triangle below.
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

        let (verts, reordered, ranges) = model_mesh(&model, 0);

        assert_eq!(verts.len(), triangle_count * 3);
        assert_eq!(ranges.len(), model.material_draw_count());
        // First-seen material order: 0, then 2, then 1.
        let order: Vec<u32> = ranges.iter().map(|range| range.material).collect();
        assert_eq!(order, vec![0, 2, 1]);
        // Material 0 owns triangles {0,1} (6 indices), 2 owns {2,4} (6), 1 owns {3} (3).
        assert_eq!(ranges[0].index_count, 6);
        assert_eq!(ranges[1].index_count, 6);
        assert_eq!(ranges[2].index_count, 3);
        // Ranges tile the whole (reordered) index buffer back-to-back.
        let mut next = 0;
        for range in &ranges {
            assert_eq!(range.first_index, next);
            next += range.index_count;
        }
        assert_eq!(next as usize, reordered.len());
        // Material 0's range is triangles 0 and 1 in their original index order.
        assert_eq!(&reordered[0..6], &[0, 1, 2, 3, 4, 5]);
    }

    /// Without per-triangle material info the mesh draws as a single range over
    /// the original index order (invariant 1: geometry untouched).
    #[test]
    fn model_mesh_single_range_without_tri_material() {
        let vertices: Vec<Vertex> = (0..3).map(corner).collect();
        let model = ModelData {
            vertices,
            indices: vec![0, 1, 2],
            ..Default::default()
        };

        let (_, reordered, ranges) = model_mesh(&model, 0);

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges.len(), model.material_draw_count());
        assert_eq!(ranges[0].material, 0);
        assert_eq!(reordered, vec![0, 1, 2]);
    }
}
