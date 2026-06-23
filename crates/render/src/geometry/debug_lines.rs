//! The 3D debug line views: the wireframe traced over original polygons, the
//! bounding box, and the face/vertex normal lines — each able to skip an
//! Outliner-hidden mesh via the per-triangle node info.

use std::collections::HashSet;

use glam::Vec3;
use review_model::{Bounds, ModelData};

use crate::scene::SceneVertex;

use super::vertex::{debug_normal_length, push_line};

/// Wireframe line segments tracing each *original* polygon's edges (quads stay
/// quads, n-gons stay n-gons) in the given color — not the triangulated
/// diagonals, which a viewer must not show as real edges (Maya/Blender don't).
///
/// The import bridge lays out each face's corners as a contiguous run of
/// vertices, so a `TopologyFace` is the closed loop over the `index_count`
/// vertices starting at `first_index`. Falls back to triangle edges only if a
/// model somehow arrives without face topology.
pub(crate) fn wireframe_lines(
    model: &ModelData,
    color: [f32; 4],
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);

    if model.faces.is_empty() {
        return triangulated_wireframe_lines(model, color, hidden_nodes);
    }

    // Map each face to its owning scene-graph node so faces of an Outliner-hidden
    // mesh are skipped — `None` when the model carries no per-triangle node info,
    // in which case visibility can't be resolved and every face is drawn.
    let face_node = (!hidden_nodes.is_empty())
        .then(|| face_node_map(model))
        .flatten();
    let hidden: HashSet<u32> = hidden_nodes.iter().copied().collect();

    for (face_index, face) in model.faces.iter().enumerate() {
        if let Some(map) = face_node.as_ref() {
            if map
                .get(face_index)
                .is_some_and(|node| hidden.contains(node))
            {
                continue;
            }
        }
        let count = face.index_count as usize;
        if count < 2 {
            continue;
        }
        let first = face.first_index as usize;

        for corner in 0..count {
            let a = first + corner;
            let b = first + (corner + 1) % count;
            let (Some(start), Some(end)) = (
                model.vertices.get(a).map(|vertex| vertex.position),
                model.vertices.get(b).map(|vertex| vertex.position),
            ) else {
                continue;
            };
            push_line(&mut vertices, start.to_array(), end.to_array(), color);
        }
    }

    vertices
}

/// A per-face owning scene-graph node index, parallel to [`ModelData::faces`].
/// Built from the per-triangle node info (`tri_node`) projected through
/// `tri_to_face`: each triangle stamps its node onto its face. `None` when the
/// model carries no per-triangle node/face info, so face visibility can't be
/// resolved (the caller then draws every face).
fn face_node_map(model: &ModelData) -> Option<Vec<u32>> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0
        || model.tri_node.len() != triangle_count
        || model.tri_to_face.len() != triangle_count
        || model.faces.is_empty()
    {
        return None;
    }
    let mut map = vec![u32::MAX; model.faces.len()];
    for triangle in 0..triangle_count {
        let face = model.tri_to_face[triangle] as usize;
        if let Some(slot) = map.get_mut(face) {
            *slot = model.tri_node[triangle];
        }
    }
    Some(map)
}

/// Triangle-edge fallback for [`wireframe_lines`] when face topology is absent.
/// Skips triangles owned by an Outliner-hidden node (via `tri_node`); an empty
/// `hidden_nodes` (or a model without per-triangle node info) draws every edge.
fn triangulated_wireframe_lines(
    model: &ModelData,
    color: [f32; 4],
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);
    let triangle_count = model.indices.len() / 3;
    let hidden: HashSet<u32> = hidden_nodes.iter().copied().collect();
    let resolve_nodes = !hidden.is_empty() && model.tri_node.len() == triangle_count;

    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
        if resolve_nodes && hidden.contains(&model.tri_node[triangle_index]) {
            continue;
        }
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

        push_line(&mut vertices, a.to_array(), b.to_array(), color);
        push_line(&mut vertices, b.to_array(), c.to_array(), color);
        push_line(&mut vertices, c.to_array(), a.to_array(), color);
    }

    vertices
}

/// The 12 edges of an axis-aligned bounding box, in the given color. The caller
/// passes whichever box it wants drawn — the whole model's bounds, or just its
/// currently-visible geometry (see [`ModelData::visible_bounds`]).
pub(crate) fn bounding_box_lines(bounds: Bounds, color: [f32; 4]) -> Vec<SceneVertex> {
    let (min, max) = (bounds.min, bounds.max);
    let corners = [
        [min.x, min.y, min.z],
        [max.x, min.y, min.z],
        [max.x, max.y, min.z],
        [min.x, max.y, min.z],
        [min.x, min.y, max.z],
        [max.x, min.y, max.z],
        [max.x, max.y, max.z],
        [min.x, max.y, max.z],
    ];
    // Pairs of corner indices: the bottom (z=min) loop, the top (z=max) loop,
    // then the four verticals connecting them.
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];

    let mut vertices = Vec::with_capacity(EDGES.len() * 2);
    for (a, b) in EDGES {
        push_line(&mut vertices, corners[a], corners[b], color);
    }
    vertices
}

/// One line per original face, from the face centroid along its averaged normal.
/// `length_scale` is relative to the model's largest extent; `color` is baked in.
pub(crate) fn face_normal_lines(
    model: &ModelData,
    length_scale: f32,
    color: [f32; 4],
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return Vec::new();
    }

    let face_count = model
        .tri_to_face
        .iter()
        .copied()
        .max()
        .map(|max_face| max_face as usize + 1)
        .unwrap_or(triangle_count);
    let mut accum_centers = vec![Vec3::ZERO; face_count];
    let mut accum_normals = vec![Vec3::ZERO; face_count];
    let mut counts = vec![0_u32; face_count];
    let normal_length = debug_normal_length(model, length_scale);

    // Skip triangles owned by an Outliner-hidden node so a hidden mesh's faces
    // contribute no normal lines; with no hidden set (or no per-triangle node
    // info) every triangle counts.
    let hidden: HashSet<u32> = hidden_nodes.iter().copied().collect();
    let resolve_nodes = !hidden.is_empty() && model.tri_node.len() == triangle_count;

    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
        if resolve_nodes && hidden.contains(&model.tri_node[triangle_index]) {
            continue;
        }
        let face_index = model
            .tri_to_face
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

        if let (Some(center_accum), Some(normal_accum), Some(count)) = (
            accum_centers.get_mut(face_index),
            accum_normals.get_mut(face_index),
            counts.get_mut(face_index),
        ) {
            *center_accum += (a + b + c) / 3.0;
            *normal_accum += normal;
            *count += 1;
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
        push_line(&mut vertices, center.to_array(), end.to_array(), color);
    }

    vertices
}

/// One line per vertex, from the vertex along its normal. `length_scale` is
/// relative to the model's largest extent; `color` is baked in.
pub(crate) fn vertex_normal_lines(
    model: &ModelData,
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
        if let Some(mask) = visible.as_ref() {
            if !mask.get(index).copied().unwrap_or(false) {
                continue;
            }
        }
        if vertex.normal.length_squared() <= f32::EPSILON {
            continue;
        }

        let start = vertex.position;
        let end = start + vertex.normal.normalize() * normal_length;
        push_line(&mut vertices, start.to_array(), end.to_array(), color);
    }

    vertices
}

/// A per-vertex visibility mask: `true` for every vertex referenced by a triangle
/// whose owning node is *not* hidden. `None` when nothing is hidden or the model
/// carries no per-triangle node info (so visibility can't be resolved and every
/// vertex is drawn).
fn visible_vertex_mask(model: &ModelData, hidden_nodes: &[u32]) -> Option<Vec<bool>> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 || hidden_nodes.is_empty() || model.tri_node.len() != triangle_count {
        return None;
    }
    let hidden: HashSet<u32> = hidden_nodes.iter().copied().collect();
    let mut mask = vec![false; model.vertices.len()];
    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
        if hidden.contains(&model.tri_node[triangle_index]) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec4};
    use review_model::Vertex;

    fn corner(index: usize) -> Vertex {
        Vertex {
            position: Vec3::new(index as f32, 0.0, 0.0),
            normal: Vec3::Y,
            uv: Vec2::ZERO,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            vertex_color: Vec4::ONE,
        }
    }

    /// The wireframe overlay drops the edges of an Outliner-hidden mesh: hiding a
    /// node removes exactly its faces' line segments, while an empty hidden set
    /// keeps every edge.
    #[test]
    fn wireframe_lines_drops_hidden_nodes() {
        // 3 triangular faces: faces 0,1 owned by node 0, face 2 by node 1.
        let tri_node = vec![0u32, 0, 1];
        let tri_to_face = vec![0u32, 1, 2];
        let triangle_count = tri_node.len();
        let vertices: Vec<Vertex> = (0..triangle_count * 3).map(corner).collect();
        let indices: Vec<u32> = (0..(triangle_count * 3) as u32).collect();
        use review_model::TopologyFace;
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
            tri_to_face,
            tri_node,
            ..Default::default()
        };
        let color = [1.0, 1.0, 1.0, 1.0];

        // Nothing hidden -> all 3 faces, 3 edges each, 2 verts per edge = 18.
        assert_eq!(wireframe_lines(&model, color, &[]).len(), 18);
        // Hide node 0 -> only face 2 survives (6 verts).
        assert_eq!(wireframe_lines(&model, color, &[0]).len(), 6);
        // Hide both nodes -> no edges at all.
        assert!(wireframe_lines(&model, color, &[0, 1]).is_empty());
    }

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
            tri_to_face,
            tri_node,
            ..Default::default()
        };
        let color = [1.0, 1.0, 1.0, 1.0];

        // Face normals: one line (2 verts) per face -> 6 with all visible, 2 when
        // node 0's two faces are hidden.
        assert_eq!(face_normal_lines(&model, 0.1, color, &[]).len(), 6);
        assert_eq!(face_normal_lines(&model, 0.1, color, &[0]).len(), 2);

        // Vertex normals: one line (2 verts) per vertex -> 18 visible, 6 when node
        // 0 is hidden (only face 2's three corners remain).
        assert_eq!(vertex_normal_lines(&model, 0.1, color, &[]).len(), 18);
        assert_eq!(vertex_normal_lines(&model, 0.1, color, &[0]).len(), 6);
    }
}
