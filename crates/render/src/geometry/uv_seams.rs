//! The UV seam view: every edge across which the chosen UV set is cut.
//!
//! A seam is a mesh edge whose faces disagree about the UVs, so the whole
//! question is which render corners are the same point. An imported mesh answers
//! from `corner_to_logical`; an Opt-processed level has no such map, so it welds
//! by (owning node, exact position bits) instead — exact is correct, not a
//! shortcut, because `optimize` never synthesizes a position.

use glam::Vec2;
use review_model::ModelData;
use review_model::topology::{edge_uses, group_edge_uses, logical_ids};

use crate::scene::SceneVertex;

use super::deform::corner_deform;
use super::hidden::HiddenFilter;
use super::vertex::push_line_deformed;

/// How far apart two UVs must sit (squared, in UV units) before the edge between
/// them reads as a discontinuity. The two sides of a *non*-seam edge are one source
/// UV element copied into two corners, so they normally agree bit for bit; this only
/// absorbs the last-place noise of a file that stored them separately.
const UV_SEAM_EPSILON: f32 = 1e-12;

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
/// in how a shared vertex is recognized, see [`review_model::topology::welded_logical_ids`].
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
    let logical = logical_ids(model);
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

    let hidden = HiddenFilter::new(model, hidden_nodes);
    let mut edges = edge_uses(model, &logical, |node| hidden.contains_node(node));

    let mut vertices = Vec::new();
    for group in group_edge_uses(&mut edges) {
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
