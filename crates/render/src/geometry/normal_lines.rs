//! Face and vertex normal lines.
//!
//! Both copy their source corner's `deform` lane, so they deform with the mesh
//! rather than drawing the bind pose over a skinned model.

use glam::Vec3;
use review_model::ModelData;

use crate::scene::SceneVertex;

use super::deform::corner_deform;
use super::hidden::HiddenFilter;
use super::vertex::{debug_normal_length, push_line};

use super::hidden::visible_vertex_mask;

/// One line per original face, from the face centroid along its averaged normal.
/// `length_scale` is relative to the model's largest extent; `color` is baked in.
pub(crate) fn face_normal_lines(
    model: &ModelData,
    lanes: &[[u32; 4]],
    length_scale: f32,
    color: [f32; 4],
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return Vec::new();
    }

    // Sized by the model's own face table — never by the largest `to_face`
    // entry, which a corrupt import could inflate into a multi-gigabyte
    // allocation. A model with no face info gets one slot per triangle (the
    // per-triangle fallback the accumulation below uses).
    let face_count = if model.triangles.to_face.is_empty() {
        triangle_count
    } else {
        model.faces.len()
    };
    let mut accum_centers = vec![Vec3::ZERO; face_count];
    let mut accum_normals = vec![Vec3::ZERO; face_count];
    let mut counts = vec![0_u32; face_count];
    // A face's line rides the deform lane of its first corner: a face has no
    // influences of its own, and its first corner is as good a stand-in as any
    // (exact for rigid geometry, a close approximation across a skinned face).
    let mut first_corner = vec![u32::MAX; face_count];
    let normal_length = debug_normal_length(model, length_scale);

    // Skip triangles owned by an Outliner-hidden node so a hidden mesh's faces
    // contribute no normal lines; with no hidden set (or no per-triangle node
    // info) every triangle counts.
    let hidden = HiddenFilter::new(model, hidden_nodes);

    for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        if hidden.is_hidden(triangle_index) {
            continue;
        }
        let face_index = model
            .triangles
            .to_face
            .get(triangle_index)
            .copied()
            .unwrap_or(triangle_index as u32) as usize;
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let positions = [
            model.vertices.get(a).map(|vertex| vertex.position),
            model.vertices.get(b).map(|vertex| vertex.position),
            model.vertices.get(c).map(|vertex| vertex.position),
        ];
        let [Some(a), Some(b), Some(c)] = positions else {
            continue;
        };

        let ab = b - a;
        let ac = c - a;
        // Normalize before the degeneracy test: the raw cross product scales with
        // triangle area, so a fixed `length_squared` threshold wrongly rejects the
        // tiny (but valid) triangles of dense regions — hands, head, boots — while
        // keeping the low-poly torso. `normalize_or_zero` only yields zero for a
        // genuinely degenerate (zero-area / non-finite) triangle.
        let normal = ab.cross(ac).normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }

        if let (Some(center_accum), Some(normal_accum), Some(count), Some(first)) = (
            accum_centers.get_mut(face_index),
            accum_normals.get_mut(face_index),
            counts.get_mut(face_index),
            first_corner.get_mut(face_index),
        ) {
            *center_accum += (a + b + c) / 3.0;
            *normal_accum += normal;
            *count += 1;
            if *first == u32::MAX {
                *first = triangle[0];
            }
        }
    }

    let mut vertices = Vec::with_capacity(face_count * 2);
    for face_index in 0..face_count {
        let count = counts[face_index];
        if count == 0 {
            continue;
        }

        let center = accum_centers[face_index] / count as f32;
        let normal = accum_normals[face_index];
        if normal.length_squared() <= f32::EPSILON {
            continue;
        }

        let end = center + normal.normalize() * normal_length;
        let deform = corner_deform(lanes, first_corner[face_index] as usize);
        push_line(
            &mut vertices,
            center.to_array(),
            end.to_array(),
            color,
            deform,
        );
    }

    vertices
}

/// One line per vertex, from the vertex along its normal. `length_scale` is
/// relative to the model's largest extent; `color` is baked in.
pub(crate) fn vertex_normal_lines(
    model: &ModelData,
    lanes: &[[u32; 4]],
    length_scale: f32,
    color: [f32; 4],
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    let normal_length = debug_normal_length(model, length_scale);
    let mut vertices = Vec::with_capacity(model.vertices.len() * 2);

    // A vertex carries no node of its own, so derive a visible-vertex mask from
    // the triangles: a vertex is drawn if any non-hidden triangle references it.
    // `None` when nothing is hidden (or no per-triangle node info), drawing every
    // vertex.
    let visible = visible_vertex_mask(model, hidden_nodes);

    for (index, vertex) in model.vertices.iter().enumerate() {
        if let Some(mask) = visible.as_ref()
            && !mask.get(index).copied().unwrap_or(false)
        {
            continue;
        }
        if vertex.normal.length_squared() <= f32::EPSILON {
            continue;
        }

        let start = vertex.position;
        let end = start + vertex.normal.normalize() * normal_length;
        push_line(
            &mut vertices,
            start.to_array(),
            end.to_array(),
            color,
            corner_deform(lanes, index),
        );
    }

    vertices
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3, Vec4};
    use review_model::{TriangleData, Vertex};

    use super::*;

    /// The face- and vertex-normal overlays drop the lines of an Outliner-hidden
    /// mesh just like the wireframe, so all derived overlays stay consistent.
    #[test]
    fn normal_lines_drop_hidden_nodes() {
        use review_model::TopologyFace;
        // 3 triangular faces with distinct corner vertices: faces 0,1 on node 0,
        // face 2 on node 1. Each corner carries a unit normal so it emits a line.
        let tri_node = vec![0u32, 0, 1];
        let tri_to_face = vec![0u32, 1, 2];
        let triangle_count = tri_node.len();
        // Each triangle gets three non-collinear corners (a valid area, so the
        // face normal isn't degenerate) and a unit +Y normal per vertex.
        let vertices: Vec<Vertex> = (0..triangle_count)
            .flat_map(|t| {
                let base = t as f32;
                [
                    Vec3::new(base, 0.0, 0.0),
                    Vec3::new(base + 1.0, 0.0, 0.0),
                    Vec3::new(base, 0.0, 1.0),
                ]
            })
            .map(|position| Vertex {
                position,
                normal: Vec3::Y,
                uv: Vec2::ZERO,
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                vertex_color: Vec4::ONE,
            })
            .collect();
        let indices: Vec<u32> = (0..(triangle_count * 3) as u32).collect();
        let faces: Vec<TopologyFace> = (0..triangle_count as u32)
            .map(|f| TopologyFace {
                first_index: f * 3,
                index_count: 3,
            })
            .collect();
        let model = ModelData {
            vertices,
            indices,
            faces,
            triangles: TriangleData {
                to_face: tri_to_face,
                node: tri_node,
                ..Default::default()
            },
            ..Default::default()
        };
        let color = [1.0, 1.0, 1.0, 1.0];

        // Face normals: one line (2 verts) per face -> 6 with all visible, 2 when
        // node 0's two faces are hidden.
        assert_eq!(face_normal_lines(&model, &[], 0.1, color, &[]).len(), 6);
        assert_eq!(face_normal_lines(&model, &[], 0.1, color, &[0]).len(), 2);

        // Vertex normals: one line (2 verts) per vertex -> 18 visible, 6 when node
        // 0 is hidden (only face 2's three corners remain).
        assert_eq!(vertex_normal_lines(&model, &[], 0.1, color, &[]).len(), 18);
        assert_eq!(vertex_normal_lines(&model, &[], 0.1, color, &[0]).len(), 6);
    }
}
