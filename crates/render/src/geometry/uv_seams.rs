//! The UV seam view: every edge across which the chosen UV set is cut.
//!
//! A seam is a mesh edge whose faces disagree about the UVs, so the whole
//! question is which render corners are the same point. An imported mesh answers
//! from `corner_to_logical`; an Opt-processed level has no such map, so it welds
//! by (owning node, exact position bits) instead — exact is correct, not a
//! shortcut, because `optimize` never synthesizes a position.

use std::borrow::Cow;
use std::collections::HashMap;

use glam::Vec2;
use review_model::ModelData;

use crate::scene::SceneVertex;

use super::deform::corner_deform;
use super::hidden::HiddenFilter;
use super::vertex::push_line_deformed;

use super::wireframe::face_node_map;

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

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3, Vec4};
    use review_model::{TriangleData, Vertex};

    use super::*;

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
