//! The 3D debug line views: the wireframe traced over original polygons, the
//! bounding box, and the face/vertex normal lines — each able to skip an
//! Outliner-hidden mesh via the per-triangle node info.

use glam::Vec3;
use review_model::{Bounds, ModelData};

use crate::scene::SceneVertex;

use super::deform::{NO_DEFORM, corner_deform};
use super::hidden::HiddenFilter;
use super::vertex::{debug_normal_length, push_line, push_line_deformed};

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
    lanes: &[[u32; 4]],
    color: [f32; 4],
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);

    if model.faces.is_empty() {
        return triangulated_wireframe_lines(model, lanes, color, hidden_nodes);
    }

    // Map each face to its owning scene-graph node so faces of an Outliner-hidden
    // mesh are skipped — `None` when the model carries no per-triangle node info,
    // in which case visibility can't be resolved and every face is drawn.
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
            let a = first + corner;
            let b = first + (corner + 1) % count;
            let (Some(start), Some(end)) = (
                model.vertices.get(a).map(|vertex| vertex.position),
                model.vertices.get(b).map(|vertex| vertex.position),
            ) else {
                continue;
            };
            // Each end follows its own corner, so a skinned edge stretches with
            // the skin exactly as the mesh's own triangle edge does.
            push_line_deformed(
                &mut vertices,
                start.to_array(),
                end.to_array(),
                color,
                corner_deform(lanes, a),
                corner_deform(lanes, b),
            );
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

/// Triangle-edge fallback for [`wireframe_lines`] when face topology is absent.
/// Skips triangles owned by an Outliner-hidden node (via `tri_node`); an empty
/// `hidden_nodes` (or a model without per-triangle node info) draws every edge.
fn triangulated_wireframe_lines(
    model: &ModelData,
    lanes: &[[u32; 4]],
    color: [f32; 4],
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);
    let hidden = HiddenFilter::new(model, hidden_nodes);

    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
        if hidden.is_hidden(triangle_index) {
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
        let [Some(pa), Some(pb), Some(pc)] = positions else {
            continue;
        };
        let (da, db, dc) = (
            corner_deform(lanes, a),
            corner_deform(lanes, b),
            corner_deform(lanes, c),
        );

        push_line_deformed(&mut vertices, pa.to_array(), pb.to_array(), color, da, db);
        push_line_deformed(&mut vertices, pb.to_array(), pc.to_array(), color, db, dc);
        push_line_deformed(&mut vertices, pc.to_array(), pa.to_array(), color, dc, da);
    }

    vertices
}

/// Half-length of each pivot-marker axis line, as a fraction of the model's
/// largest bounding extent — so the marker reads at any model scale.
const PIVOT_LENGTH_FRACTION: f32 = 0.1;
/// Floor for the pivot half-length, in world units, so a degenerate or empty
/// model still shows a visible marker.
const PIVOT_MIN_HALF: f32 = 0.05;

/// World-space position of the model's pivot: the translation of its root scene
/// node (the first parentless node), i.e. where the object's local origin lands
/// after the importer world-bakes the geometry (invariant 1). The world origin
/// when the model carries no node hierarchy.
pub(crate) fn model_pivot(model: &ModelData) -> [f32; 3] {
    model
        .nodes
        .iter()
        .find(|node| node.parent.is_none())
        .map(|node| node.transform.w_axis.truncate().to_array())
        .unwrap_or([0.0, 0.0, 0.0])
}

/// Half-length of each pivot axis line: [`PIVOT_LENGTH_FRACTION`] of the model's
/// largest extent, floored at [`PIVOT_MIN_HALF`].
pub(crate) fn pivot_half_extent(model: &ModelData) -> f32 {
    let size = model.bounds.map(Bounds::size).unwrap_or(Vec3::splat(1.0));
    (size.max_element() * PIVOT_LENGTH_FRACTION).max(PIVOT_MIN_HALF)
}

/// A 3-axis pivot marker centered on `pivot`: three colored line segments running
/// through the point along the world axes (X red, Y green, Z blue), each extending
/// `half` in both directions, so their intersection marks the object's pivot. Drawn
/// with the depth-tested line pipeline like the other overlays, so it is occluded
/// where it passes inside the mesh.
pub(crate) fn pivot_lines(pivot: [f32; 3], half: f32) -> Vec<SceneVertex> {
    // Gamma-space axis colors (the line shader returns the vertex color directly),
    // matching the floor grid's X/Z hues with a green Y.
    const X_COLOR: [f32; 4] = [0.94, 0.23, 0.28, 1.0];
    const Y_COLOR: [f32; 4] = [0.45, 0.80, 0.30, 1.0];
    const Z_COLOR: [f32; 4] = [0.18, 0.53, 1.0, 1.0];

    let pivot = Vec3::from_array(pivot);
    let mut vertices = Vec::with_capacity(6);
    for (axis, color) in [(Vec3::X, X_COLOR), (Vec3::Y, Y_COLOR), (Vec3::Z, Z_COLOR)] {
        push_line(
            &mut vertices,
            (pivot - axis * half).to_array(),
            (pivot + axis * half).to_array(),
            color,
            NO_DEFORM,
        );
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
        push_line(&mut vertices, corners[a], corners[b], color, NO_DEFORM);
    }
    vertices
}

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

    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
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

/// A per-vertex visibility mask: `true` for every vertex referenced by a triangle
/// whose owning node is *not* hidden. `None` when nothing is hidden or the model
/// carries no per-triangle node info (so visibility can't be resolved and every
/// vertex is drawn).
fn visible_vertex_mask(model: &ModelData, hidden_nodes: &[u32]) -> Option<Vec<bool>> {
    let hidden = HiddenFilter::new(model, hidden_nodes);
    if !hidden.is_active() {
        return None;
    }
    let mut mask = vec![false; model.vertices.len()];
    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec4};
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
            triangles: TriangleData {
                to_face: tri_to_face,
                node: tri_node,
                ..Default::default()
            },
            ..Default::default()
        };
        let color = [1.0, 1.0, 1.0, 1.0];

        // Nothing hidden -> all 3 faces, 3 edges each, 2 verts per edge = 18.
        assert_eq!(wireframe_lines(&model, &[], color, &[]).len(), 18);
        // Hide node 0 -> only face 2 survives (6 verts).
        assert_eq!(wireframe_lines(&model, &[], color, &[0]).len(), 6);
        // Hide both nodes -> no edges at all.
        assert!(wireframe_lines(&model, &[], color, &[0, 1]).is_empty());
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
