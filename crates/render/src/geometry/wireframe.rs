//! The 3D debug line views: the wireframe traced over original polygons, the
//! bounding box, the face/vertex normal lines and the UV-seam edges — each able
//! to skip an Outliner-hidden mesh via the per-triangle node info.//!
//! The pivot and bounding box are in [`super::markers`], the normal lines in
//! [`super::normal_lines`] and the seam view in [`super::uv_seams`]; what is left
//! here is the model wireframe.

use review_model::ModelData;

use super::hidden::HiddenFilter;

/// The model wireframe: each *original* polygon's edges (quads stay quads, n-gons
/// stay n-gons — not the triangulated diagonals, which a viewer must not show as
/// real edges; Maya/Blender don't), as a **`LineList` index buffer over the
/// mesh's own vertex buffer**.
///
/// The import bridge lays out each face's corners as a contiguous run of
/// vertices, so a `TopologyFace` is the closed loop over the `index_count`
/// vertices starting at `first_index`. Faces owned by an Outliner-hidden mesh are
/// skipped (their edges disappear with the mesh); a model that somehow arrives
/// without face topology falls back to triangle edges.
///
/// Indices, not a fresh vertex stream, because this is invariant 1 exactly:
/// a derived view indexes *into* the shared geometry. Two `u32` corner indices
/// per edge is 8 bytes where two 80-byte [`SceneVertex`]es were 160 — the 20×
/// that makes the wireframe cheap enough to build without a visible hitch, and
/// cheap enough to keep resident across a toggle (see
/// `DerivedViews::wireframe_index`). It also drops the two things that used to be
/// baked into every vertex: the deform lane (the mesh corner already carries its
/// own, so a skinned wireframe follows the skin for free) and the colour, which
/// moves to the `selection_color` uniform — so dragging the wireframe colour
/// slider now rebuilds nothing at all.
///
/// The returned indices address [`ModelData::vertices`] directly, which is
/// exactly what the mesh vertex buffer holds (`model_mesh` maps it 1:1);
/// out-of-range corners are dropped per edge.
pub(crate) fn wireframe_edge_indices(model: &ModelData, hidden_nodes: &[u32]) -> Vec<u32> {
    let vertex_count = model.vertices.len();
    let mut indices: Vec<u32> = Vec::with_capacity(model.indices.len() * 2);
    let mut push_edge = |a: usize, b: usize| {
        if a < vertex_count && b < vertex_count {
            indices.push(a as u32);
            indices.push(b as u32);
        }
    };

    if model.faces.is_empty() {
        // Triangle-edge fallback, mirroring `triangulated_wireframe_lines`.
        let hidden = HiddenFilter::new(model, hidden_nodes);
        for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
            if hidden.is_hidden(triangle_index) {
                continue;
            }
            let [a, b, c] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            push_edge(a, b);
            push_edge(b, c);
            push_edge(c, a);
        }
        return indices;
    }

    let hidden = HiddenFilter::new(model, hidden_nodes);
    let face_node = hidden.is_active().then(|| face_node_map(model)).flatten();
    for (face_index, face) in model.faces.iter().enumerate() {
        if let Some(map) = face_node.as_ref()
            && map
                .get(face_index)
                .is_some_and(|node| hidden.contains_node(*node))
        {
            continue;
        }
        let count = face.index_count as usize;
        if count < 2 {
            continue;
        }
        let first = face.first_index as usize;
        for corner in 0..count {
            push_edge(first + corner, first + (corner + 1) % count);
        }
    }

    indices
}

/// A per-face owning scene-graph node index, parallel to [`ModelData::faces`].
/// Built from the per-triangle node info (`tri_node`) projected through
/// `tri_to_face`: each triangle stamps its node onto its face. `None` when the
/// model carries no per-triangle node/face info, so face visibility can't be
/// resolved (the caller then draws every face).
pub(super) fn face_node_map(model: &ModelData) -> Option<Vec<u32>> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0
        || model.triangles.node.len() != triangle_count
        || model.triangles.to_face.len() != triangle_count
        || model.faces.is_empty()
    {
        return None;
    }
    let mut map = vec![u32::MAX; model.faces.len()];
    for triangle in 0..triangle_count {
        let face = model.triangles.to_face[triangle] as usize;
        if let Some(slot) = map.get_mut(face) {
            *slot = model.triangles.node[triangle];
        }
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3, Vec4};
    use review_model::{TriangleData, Vertex};

    use super::*;

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
    fn wireframe_edges_drop_hidden_nodes() {
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
            triangles: TriangleData {
                to_face: tri_to_face,
                node: tri_node,
                ..Default::default()
            },
            ..Default::default()
        };

        // Nothing hidden -> all 3 faces, 3 edges each, 2 indices per edge = 18.
        let all = wireframe_edge_indices(&model, &[]);
        assert_eq!(all.len(), 18);
        // Every index addresses a real vertex of the shared mesh buffer.
        assert!(all.iter().all(|&i| (i as usize) < model.vertices.len()));
        // Hide node 0 -> only face 2 survives (6 indices), and they are its corners.
        assert_eq!(wireframe_edge_indices(&model, &[0]), vec![6, 7, 7, 8, 8, 6]);
        // Hide both nodes -> no edges at all.
        assert!(wireframe_edge_indices(&model, &[0, 1]).is_empty());
    }
}
