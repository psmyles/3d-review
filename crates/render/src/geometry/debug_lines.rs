//! The 3D debug line views: the wireframe traced over original polygons, the
//! bounding box, the face/vertex normal lines and the UV-seam edges — each able
//! to skip an Outliner-hidden mesh via the per-triangle node info.

use std::borrow::Cow;
use std::collections::HashMap;

use glam::{Vec2, Vec3};
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

    for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
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

/// How far apart two UVs must sit (squared, in UV units) before the edge between
/// them reads as a discontinuity. The two sides of a *non*-seam edge are one source
/// UV element copied into two corners, so they normally agree bit for bit; this only
/// absorbs the last-place noise of a file that stored them separately.
const UV_SEAM_EPSILON: f32 = 1e-12;

/// One use of one mesh edge by one face, keyed by the pair of **logical** (DCC)
/// vertices it connects.
///
/// The logical pair is what makes the two sides of a seam meet at all: import splits
/// every face corner into its own render vertex, so the two faces sharing an edge
/// reference four different [`ModelData::vertices`] entries — which all trace back to
/// the same two entries of [`ModelData::corner_to_logical`]. Comparing render-vertex
/// indices would find no shared edges whatsoever.
struct EdgeUse {
    /// `min(logical) << 32 | max(logical)`, so both winding directions of one edge
    /// sort together and a single pass over the sorted list groups them.
    key: u64,
    /// The render corner this use saw the edge's *lower*-numbered logical vertex at.
    /// Ordered to match `key` so two uses of an edge compare UVs end for end
    /// whichever way each face winds around it.
    low: u32,
    /// The render corner this use saw the higher-numbered logical vertex at.
    high: u32,
}

/// One line per **UV seam**: a mesh edge across which the chosen UV set is
/// discontinuous, which is exactly where the texture is cut. Maya draws the same set
/// as "texture border edges". The line pipeline's width is fixed at 1px, so `color`
/// is the whole of what separates these from the wireframe beneath them.
///
/// An edge is a seam when the faces meeting on it disagree about the UV of either
/// shared vertex, or when only one face uses it at all — an open border, where a UV
/// shell ends just the same.
///
/// Works on an optimizer-rebuilt mesh as well as an imported one; the two differ only
/// in how a shared vertex is recognized, see [`welded_logical_ids`].
pub(crate) fn uv_seam_lines(
    model: &ModelData,
    lanes: &[[u32; 4]],
    color: [f32; 4],
    uv_channel: u32,
    hidden_nodes: &[u32],
) -> Vec<SceneVertex> {
    // Which render corners are the same point of the same surface. Import hands that
    // over for free; anything else has to be welded for it (invariant 1: both are
    // views *onto* the shared buffers, neither copies geometry).
    let logical: Cow<'_, [u32]> = if model.corner_to_logical.len() == model.vertices.len() {
        Cow::Borrowed(&model.corner_to_logical)
    } else {
        Cow::Owned(welded_logical_ids(model))
    };
    // `None` for a single-set model, which keeps its UVs in `Vertex::uv` (see
    // `ModelData::uv_channels`) — and for a channel whose array doesn't run parallel
    // to the vertices, where a lookup could only produce a wrong-but-plausible seam.
    let channel_uvs = model
        .uv_channels
        .get(uv_channel as usize)
        .map(Vec::as_slice)
        .filter(|uvs| uvs.len() == model.vertices.len());
    let uv = |corner: u32| -> Vec2 {
        match channel_uvs {
            Some(uvs) => uvs[corner as usize],
            None => model.vertices[corner as usize].uv,
        }
    };

    let mut edges = collect_edge_uses(model, &logical, hidden_nodes);
    edges.sort_unstable_by_key(|edge| edge.key);

    let mut vertices = Vec::new();
    let mut start = 0;
    while start < edges.len() {
        let key = edges[start].key;
        let mut end = start + 1;
        while end < edges.len() && edges[end].key == key {
            end += 1;
        }
        let group = &edges[start..end];
        start = end;

        // Every use is compared against the first rather than pairwise: a seam is
        // "not all the same", so a non-manifold edge with three faces on it is one as
        // soon as any of them disagrees.
        let first = &group[0];
        let (low_uv, high_uv) = (uv(first.low), uv(first.high));
        let seam = group.len() == 1
            || group[1..].iter().any(|other| {
                uv(other.low).distance_squared(low_uv) > UV_SEAM_EPSILON
                    || uv(other.high).distance_squared(high_uv) > UV_SEAM_EPSILON
            });
        if !seam {
            continue;
        }

        // Each end follows its own corner's deform lane, so a seam on a skinned mesh
        // stretches with the skin exactly as the wireframe edge under it does.
        let (a, b) = (
            model.vertices[first.low as usize].position,
            model.vertices[first.high as usize].position,
        );
        push_line_deformed(
            &mut vertices,
            a.to_array(),
            b.to_array(),
            color,
            corner_deform(lanes, first.low as usize),
            corner_deform(lanes, first.high as usize),
        );
    }

    vertices
}

/// Every edge of every *visible* face, one [`EdgeUse`] per (face, edge) pair, with
/// `map` giving each render corner the identity two faces meet on.
///
/// Walks the original polygons wherever the model carries face topology — so a
/// quad's four real edges are considered and never its triangulation diagonal, which
/// is not an edge an artist can put a seam on — and falls back to triangle edges only
/// for a mesh that arrived without it. A processed mesh takes the fallback (it is
/// pure triangles and carries no `faces`), which costs it nothing: a diagonal is an
/// interior edge whose two triangles agree about the UVs, so it is never a seam.
fn collect_edge_uses(model: &ModelData, map: &[u32], hidden_nodes: &[u32]) -> Vec<EdgeUse> {
    let mut edges = Vec::with_capacity(model.indices.len());
    let hidden = HiddenFilter::new(model, hidden_nodes);

    if model.faces.is_empty() {
        for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
            if hidden.is_hidden(triangle_index) {
                continue;
            }
            for corner in 0..3 {
                push_edge_use(
                    &mut edges,
                    map,
                    triangle[corner] as usize,
                    triangle[(corner + 1) % 3] as usize,
                );
            }
        }
        return edges;
    }

    // The same face -> node resolution the wireframe uses, so a hidden mesh's seams
    // disappear with its edges. `None` when visibility can't be resolved, in which
    // case every face contributes.
    let face_node = hidden.is_active().then(|| face_node_map(model)).flatten();
    for (face_index, face) in model.faces.iter().enumerate() {
        if let Some(nodes) = face_node.as_ref()
            && nodes
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
            push_edge_use(
                &mut edges,
                map,
                first + corner,
                first + (corner + 1) % count,
            );
        }
    }

    edges
}

/// A stand-in for [`ModelData::corner_to_logical`] on a mesh that carries none —
/// today, the Opt workspace's processed levels, whose rebuilt vertex buffer has no
/// source vertices left to map back to. Two corners get the same id when they sit at
/// the **same position on the same object**, which is the identity a UV seam is
/// actually about.
///
/// This is exact rather than tolerant, and that is not a simplification:
/// `optimize` never synthesizes a position — every operation either drops vertices or
/// copies whole ones through a remap — so a processed vertex's position is bit for bit
/// one the source mesh had, and the corners that came from one source vertex still
/// match to the bit. Welding within a tolerance instead would risk merging genuinely
/// separate surfaces that pass close by.
///
/// The owning node is part of the key because a weld is otherwise happy to join two
/// objects that touch, which would hide the border each of them really has there. It
/// comes from the per-triangle node tags; a mesh without them welds as one object,
/// the most a position alone can say.
///
/// Cost is one hash pass over the vertices, and it lands where it is cheapest: a
/// processed mesh has been through `index_mesh`, so it has far fewer vertices than
/// the corner-split mesh import produces. An imported mesh never pays it at all.
fn welded_logical_ids(model: &ModelData) -> Vec<u32> {
    let triangle_count = model.indices.len() / 3;
    let node_tags =
        (model.triangles.node.len() == triangle_count).then_some(model.triangles.node.as_slice());

    let mut ids = vec![u32::MAX; model.vertices.len()];
    let mut welded: HashMap<(u32, [u32; 3]), u32> = HashMap::new();
    for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        let node = node_tags.map_or(0, |nodes| nodes[triangle_index]);
        for &corner in triangle {
            let corner = corner as usize;
            // Skips an out-of-range index and an already-assigned corner in one test.
            // A vertex no triangle references keeps `u32::MAX`, and is never reached
            // by the edge walk either.
            if ids.get(corner).copied() != Some(u32::MAX) {
                continue;
            }
            let Some(vertex) = model.vertices.get(corner) else {
                continue;
            };
            let key = (
                node,
                [
                    position_bits(vertex.position.x),
                    position_bits(vertex.position.y),
                    position_bits(vertex.position.z),
                ],
            );
            let next = welded.len() as u32;
            ids[corner] = *welded.entry(key).or_insert(next);
        }
    }
    ids
}

/// One position component as a hash key. `-0.0` folds onto `0.0` and every NaN onto
/// one canonical NaN, because the comparison is bitwise: without that, two vertices
/// at the same visible point could miss each other over a sign bit.
fn position_bits(value: f32) -> u32 {
    if value == 0.0 {
        0.0f32.to_bits()
    } else if value.is_nan() {
        f32::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

/// Record one use of the edge between render corners `a` and `b`, on the canonical
/// (lower logical vertex first) form. Skips a corner outside the map, and an edge
/// whose ends share a logical vertex — degenerate either way, and neither can carry
/// a seam.
fn push_edge_use(edges: &mut Vec<EdgeUse>, map: &[u32], a: usize, b: usize) {
    let (Some(&logical_a), Some(&logical_b)) = (map.get(a), map.get(b)) else {
        return;
    };
    if logical_a == logical_b {
        return;
    }
    let (key, low, high) = if logical_a < logical_b {
        (
            (u64::from(logical_a) << 32) | u64::from(logical_b),
            a as u32,
            b as u32,
        )
    } else {
        (
            (u64::from(logical_b) << 32) | u64::from(logical_a),
            b as u32,
            a as u32,
        )
    };
    edges.push(EdgeUse { key, low, high });
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
    for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
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

    /// Two triangles sharing one edge, corner-split the way import leaves them:
    /// corners 0,1,2 (logical 0,1,2) and 3,4,5 (logical 1,3,2), so the shared edge
    /// is logical (1,2) seen once from each side. `uvs` supplies one UV per corner.
    fn seam_pair_model(uvs: [Vec2; 6]) -> ModelData {
        use review_model::TopologyFace;
        let logical = [0u32, 1, 2, 1, 3, 2];
        let positions = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        ModelData {
            vertices: (0..6)
                .map(|index| Vertex {
                    position: positions[index],
                    normal: Vec3::Y,
                    uv: uvs[index],
                    tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                    vertex_color: Vec4::ONE,
                })
                .collect(),
            indices: (0..6).collect(),
            faces: (0..2)
                .map(|face| TopologyFace {
                    first_index: face * 3,
                    index_count: 3,
                })
                .collect(),
            triangles: TriangleData {
                to_face: vec![0, 1],
                node: vec![0, 1],
                ..Default::default()
            },
            corner_to_logical: logical.to_vec(),
            ..Default::default()
        }
    }

    /// The seam view draws an edge only where the two faces on it disagree about the
    /// UVs — plus every open border, where a UV shell ends anyway. Both sides of the
    /// shared edge are corner-split, so agreement can only be seen through
    /// `corner_to_logical`; a model without that map draws nothing at all rather
    /// than lighting up every edge.
    #[test]
    fn uv_seam_lines_follow_uv_discontinuities() {
        let color = [1.0, 1.0, 1.0, 1.0];
        // The four corners of the shared edge agree: corners 1 and 3 are logical 1,
        // corners 2 and 5 are logical 2.
        let matched = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        // Only the four open borders are seams -> 4 lines, 2 vertices each.
        let model = seam_pair_model(matched);
        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[]).len(), 8);

        // Move one side of the shared edge in UV space: it becomes a seam too.
        let mut split = matched;
        split[3] = Vec2::new(0.5, 0.5);
        let model = seam_pair_model(split);
        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[]).len(), 10);

        // Hiding a face drops its edges, exactly as the wireframe does: only the
        // remaining triangle's three edges are left, all open borders now.
        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[0]).len(), 6);
    }

    /// An *indexed* two-triangle mesh shaped like an Opt-processed level: no
    /// `corner_to_logical`, no face topology, and vertices shared wherever every
    /// attribute agrees.
    ///
    /// `duplicate` is how the second triangle meets the first along the (1,0,0) —
    /// (0,0,1) edge: `None` reuses vertices 1 and 2 outright (what indexing produces
    /// for a continuous edge), `Some(uvs)` gives it its own copies at the same
    /// positions carrying those UVs — a cut when they differ from the originals, and
    /// the shape a *second object* has when they don't, since submesh reassembly
    /// concatenates per-object vertex buffers and never shares one across nodes.
    fn processed_pair_model(duplicate: Option<[Vec2; 2]>, nodes: [u32; 2]) -> ModelData {
        let corner = |position: Vec3, uv: Vec2| Vertex {
            position,
            normal: Vec3::Y,
            uv,
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            vertex_color: Vec4::ONE,
        };
        let (a, b) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
        let mut vertices = vec![
            corner(Vec3::ZERO, Vec2::new(0.0, 0.0)),
            corner(a, Vec2::new(1.0, 0.0)),
            corner(b, Vec2::new(0.0, 1.0)),
            corner(Vec3::new(1.0, 0.0, 1.0), Vec2::new(1.0, 1.0)),
        ];
        let indices = match duplicate {
            None => vec![0, 1, 2, 1, 3, 2],
            Some([uv_a, uv_b]) => {
                vertices.push(corner(a, uv_a));
                vertices.push(corner(b, uv_b));
                vec![0, 1, 2, 4, 3, 5]
            }
        };
        ModelData {
            vertices,
            indices,
            faces: Vec::new(),
            triangles: TriangleData {
                node: nodes.to_vec(),
                ..Default::default()
            },
            corner_to_logical: Vec::new(),
            ..Default::default()
        }
    }

    /// A mesh with no corner -> logical map — an Opt-processed level — still reports
    /// its seams: corners are matched by exact position within one object instead.
    #[test]
    fn uv_seam_lines_weld_a_rebuilt_mesh_by_position() {
        let color = [1.0, 1.0, 1.0, 1.0];
        let (uv_a, uv_b) = (Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0));

        // Continuous across the shared edge -> only the four open borders.
        let model = processed_pair_model(None, [0, 0]);
        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[]).len(), 8);

        // Cut along it: separate vertices at the same positions carrying their own
        // UVs. The weld still finds the edge, and the UVs disagree -> five edges.
        let model = processed_pair_model(Some([Vec2::new(0.25, 0.75), uv_b]), [0, 0]);
        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[]).len(), 10);

        // Split vertices whose UVs *agree* are the same surface after all (a weld the
        // stack could have collapsed), so the edge is shared and only the borders show.
        let model = processed_pair_model(Some([uv_a, uv_b]), [0, 0]);
        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[]).len(), 8);

        // The identical geometry as two objects that merely touch: the node tags keep
        // them apart, so each keeps the border it really has -> six edges, not five.
        let model = processed_pair_model(Some([uv_a, uv_b]), [0, 1]);
        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[]).len(), 12);
    }

    /// The seam test reads the UV set it is pointed at: a model whose second set is
    /// cut where the first is continuous shows seams only on channel 1.
    #[test]
    fn uv_seam_lines_read_the_selected_uv_channel() {
        let color = [1.0, 1.0, 1.0, 1.0];
        let matched = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        let mut model = seam_pair_model(matched);
        // Channel 0 mirrors `Vertex::uv` (continuous across the shared edge);
        // channel 1 splits corner 3 away from corner 1.
        let mut second = matched.to_vec();
        second[3] = Vec2::new(0.5, 0.5);
        model.uv_channels = vec![matched.to_vec(), second];

        assert_eq!(uv_seam_lines(&model, &[], color, 0, &[]).len(), 8);
        assert_eq!(uv_seam_lines(&model, &[], color, 1, &[]).len(), 10);
        // An out-of-range channel falls back to `Vertex::uv` rather than panicking.
        assert_eq!(uv_seam_lines(&model, &[], color, 7, &[]).len(), 8);
    }
}
