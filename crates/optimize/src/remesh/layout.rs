//! Two layout rewrites: making the engine's output order-independent, and
//! expanding an indexed piece back into the corner-run layout a `ModelData`
//! carrying real polygons has to be in.
//!
//! ## Canonicalization
//!
//! The retopologizer's vertex and face *order* is a function of how its work was
//! scheduled, even when the geometry is not: a parallel extraction appends
//! vertices as threads reach them. That is invisible on screen and fatal to
//! reproducibility — a preset that produces one file today and a byte-different
//! one tomorrow cannot be diffed, and the export round-trip test could not pin
//! anything. [`canonicalize`] rewrites the soup into an order derived from the
//! geometry alone, after which two runs of the same input are byte-identical.
//!
//! ## Corner runs
//!
//! `ModelData::faces` describes **contiguous runs in the vertex array**: face
//! `f` owns `vertices[first_index .. first_index + index_count]`, in winding
//! order (see `demo_cube_model`). Every consumer reads it that way — the
//! wireframe walks the run to draw a quad's four edges, the stats panel counts a
//! run once instead of counting its triangles, the export writes it as one
//! polygon. An indexed mesh is not in that layout, so a level that carries
//! polygons has to be expanded into it; [`corner_run`] is that expansion, and
//! `process::assemble` is what runs it.

use std::collections::HashMap;

use glam::{Vec2, Vec3, Vec4};
use review_model::{TopologyFace, Vertex};

use crate::submesh::{NO_FACE, Submesh};

use super::RemeshOutput;

/// Rewrite `output` into an order that depends only on its geometry.
///
/// Each face is rotated to lead with its lowest-positioned corner, the faces are
/// sorted by the resulting corner sequence, and the vertices are renumbered in
/// order of first use. Winding is preserved throughout — a rotation is not a
/// reversal — so the orientation guard in [`super::project`] still sees the face
/// the engine built.
pub(super) fn canonicalize(output: &mut RemeshOutput) {
    let face_count = output.face_count();
    if face_count == 0 {
        return;
    }

    // Each face first rotated to lead with its lowest-positioned corner, then
    // the faces sorted by the resulting sequence. Both steps read positions
    // only, so neither depends on how the extraction was scheduled — and the
    // rotation has to come *first*, because the vertex renumbering below is by
    // order of first use and would otherwise inherit the emitted corner order.
    //
    // The key is the rotated sequence rather than a sorted multiset, so two
    // faces over the same corners in opposite windings stay distinct.
    let rotated: Vec<Vec<u32>> = (0..face_count)
        .map(|face| {
            let mut corners = output.face(face).to_vec();
            if let Some(lead) = lead_corner(output, &corners) {
                corners.rotate_left(lead);
            }
            corners
        })
        .collect();
    let keys: Vec<Vec<[u32; 3]>> = rotated
        .iter()
        .map(|corners| {
            corners
                .iter()
                .map(|&corner| position_key(output.positions[corner as usize]))
                .collect()
        })
        .collect();
    let mut order: Vec<usize> = (0..face_count).collect();
    order.sort_by(|&a, &b| keys[a].cmp(&keys[b]));

    let mut new_of_old: Vec<u32> = vec![u32::MAX; output.positions.len()];
    let mut positions: Vec<Vec3> = Vec::with_capacity(output.positions.len());
    let mut corners: Vec<u32> = Vec::with_capacity(output.corners.len());
    let mut face_offsets: Vec<u32> = Vec::with_capacity(face_count + 1);
    face_offsets.push(0);

    for &face in &order {
        for &corner in &rotated[face] {
            let slot = &mut new_of_old[corner as usize];
            if *slot == u32::MAX {
                *slot = positions.len() as u32;
                positions.push(output.positions[corner as usize]);
            }
            corners.push(*slot);
        }
        face_offsets.push(corners.len() as u32);
    }

    output.positions = positions;
    output.corners = corners;
    output.face_offsets = face_offsets;
}

/// Which corner a face is rotated to lead with.
///
/// The lowest-positioned one, which is a function of the geometry and so the
/// same however the face was emitted — except on a **quad**, where the choice
/// is between the two corners of one diagonal only.
///
/// A quad carries more than an outline: which of its two diagonals it is split
/// on is part of what it says, and [`super::fan`] reads that off the corner
/// order. Rotating by an odd number of places swaps the two, which would hand
/// the viewport and the export a different surface from the one the merge
/// checked — so the rotation is restricted to the even ones, and the tie
/// between them is broken the same way as for any other face.
fn lead_corner(output: &RemeshOutput, corners: &[u32]) -> Option<usize> {
    let key = |corner: usize| position_key(output.positions[corners[corner] as usize]);
    if corners.len() == 4 {
        return Some(if key(2) < key(0) { 2 } else { 0 });
    }
    (0..corners.len()).min_by_key(|&corner| key(corner))
}

fn position_key(position: Vec3) -> [u32; 3] {
    [
        position.x.to_bits(),
        position.y.to_bits(),
        position.z.to_bits(),
    ]
}

/// One piece expanded into the corner-run layout, ready to be appended to a
/// level's buffers.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CornerRun {
    pub vertices: Vec<Vertex>,
    /// Extra UV sets, parallel to `vertices`.
    pub uv_channels: Vec<Vec<Vec2>>,
    /// Vertex-color sets beyond the first, parallel to `vertices`.
    pub color_channels: Vec<Vec<Vec4>>,
    /// Parallel to `vertices`; empty when the piece carried no creases.
    pub vertex_crease: Vec<f32>,
    /// Parallel to `vertices`; empty when the piece knew no source corners.
    pub source_corner: Vec<u32>,
    /// Per run vertex, the **indexed** piece vertex it was expanded from.
    ///
    /// This is what keeps the export writing one FBX control point per real
    /// vertex instead of one per corner: the run is a render-side layout, and a
    /// quad soup is not what the file should hold. It is also what a deform row
    /// is gathered through, for the rare level whose neighbour node is skinned.
    pub control_point: Vec<u32>,
    /// Triangles over the run's own vertices.
    pub indices: Vec<u32>,
    /// One entry per polygon, each naming a contiguous run of `vertices`.
    pub faces: Vec<TopologyFace>,
    /// Per triangle of `indices`, the face it belongs to — an index into
    /// `faces`, never [`NO_FACE`], since a triangle with no carried face gets
    /// one of its own here.
    pub to_face: Vec<u32>,
    /// Per *piece* vertex, the first run slot it was emitted into, or
    /// [`u32::MAX`] for one no face reached. The inverse of `control_point`,
    /// picking one representative — which is what an authored edge needs, since
    /// an edge joins two real vertices and the run has several corners for each.
    pub first_of_vertex: Vec<u32>,
}

impl CornerRun {
    /// A run slot standing for piece vertex `vertex`, falling back to slot 0 for
    /// one no face reached (an edge naming it is then dropped downstream, which
    /// is what an edge between unused vertices deserves).
    pub fn first_slot(&self, vertex: u32) -> u32 {
        match self.first_of_vertex.get(vertex as usize) {
            Some(&slot) if slot != u32::MAX => slot,
            _ => 0,
        }
    }
}

/// Expand one piece into contiguous per-face corner runs.
///
/// A piece with a polygon carry emits one run per carried face and re-expresses
/// its triangles over that run. A piece with no carry — or a triangle belonging
/// to no face — emits a three-corner run of its own, which is what makes the
/// layout uniform across a level where only some pieces were rebuilt
/// (`TriangleData::to_face` is all-or-nothing: a level cannot carry faces for
/// one node and not another).
pub(crate) fn corner_run(piece: &Submesh) -> CornerRun {
    let mut run = CornerRun {
        uv_channels: vec![Vec::new(); piece.uv_channels.len()],
        color_channels: vec![Vec::new(); piece.color_channels.len()],
        first_of_vertex: vec![u32::MAX; piece.vertices.len()],
        ..CornerRun::default()
    };

    // Emit a piece vertex into the run, returning its run index. Called once per
    // *corner*, so one piece vertex can land in the run several times — which is
    // the whole point of the layout.
    let emit = |run: &mut CornerRun, vertex: u32| -> u32 {
        let slot = run.vertices.len() as u32;
        run.vertices.push(
            piece
                .vertices
                .get(vertex as usize)
                .copied()
                .unwrap_or_default(),
        );
        for (channel, destination) in run.uv_channels.iter_mut().enumerate() {
            destination.push(
                piece
                    .uv_channels
                    .get(channel)
                    .and_then(|values| values.get(vertex as usize))
                    .copied()
                    .unwrap_or(Vec2::ZERO),
            );
        }
        for (channel, destination) in run.color_channels.iter_mut().enumerate() {
            destination.push(
                piece
                    .color_channels
                    .get(channel)
                    .and_then(|values| values.get(vertex as usize))
                    .copied()
                    .unwrap_or(Vec4::ONE),
            );
        }
        if !piece.vertex_crease.is_empty() {
            run.vertex_crease.push(
                piece
                    .vertex_crease
                    .get(vertex as usize)
                    .copied()
                    .unwrap_or(0.0),
            );
        }
        if !piece.source_corner.is_empty() {
            run.source_corner.push(
                piece
                    .source_corner
                    .get(vertex as usize)
                    .copied()
                    .unwrap_or(u32::MAX),
            );
        }
        run.control_point.push(vertex);
        if let Some(first) = run.first_of_vertex.get_mut(vertex as usize)
            && *first == u32::MAX
        {
            *first = slot;
        }
        slot
    };

    // Per carried face: its run's base, and where each of its corners landed, so
    // a triangle of that face can be re-expressed without a search per corner.
    let mut face_slots: Vec<HashMap<u32, u32>> = Vec::new();
    if let Some(carry) = &piece.polygons {
        for face in 0..carry.face_count() {
            let corners = carry.face(face);
            let first = run.vertices.len() as u32;
            let mut slots: HashMap<u32, u32> = HashMap::with_capacity(corners.len());
            for &corner in corners {
                let slot = emit(&mut run, corner);
                // A face that names one vertex twice keeps the first slot; the
                // triangle that uses it then draws the same corner, which is
                // what it already did in the indexed mesh.
                slots.entry(corner).or_insert(slot);
            }
            run.faces.push(TopologyFace {
                first_index: first,
                index_count: corners.len() as u32,
            });
            face_slots.push(slots);
        }
    }

    for (triangle, corners) in piece.indices.as_chunks::<3>().0.iter().enumerate() {
        let face = piece
            .polygons
            .as_ref()
            .and_then(|carry| carry.triangle_face.get(triangle).copied())
            .unwrap_or(NO_FACE);
        let mapped = (face != NO_FACE)
            .then(|| face_slots.get(face as usize))
            .flatten()
            .and_then(|slots| {
                Some([
                    *slots.get(&corners[0])?,
                    *slots.get(&corners[1])?,
                    *slots.get(&corners[2])?,
                ])
            });
        match mapped {
            Some(indices) => {
                run.indices.extend_from_slice(&indices);
                run.to_face.push(face);
            }
            // No face, or a triangle whose corners its face does not name (which
            // a consistent carry never produces). Either way it becomes its own
            // three-corner polygon rather than being dropped.
            None => {
                let first = run.vertices.len() as u32;
                for &corner in corners {
                    let slot = emit(&mut run, corner);
                    run.indices.push(slot);
                }
                run.to_face.push(run.faces.len() as u32);
                run.faces.push(TopologyFace {
                    first_index: first,
                    index_count: 3,
                });
            }
        }
    }

    run
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use review_model::TriangleData;

    use super::*;
    use crate::submesh::{NO_MATERIAL, PolygonCarry};

    /// Two quads sharing an edge, as a rebuilt piece: four faces' worth of
    /// corners over six shared vertices.
    fn two_quads() -> Submesh {
        let vertex = |x: f32, y: f32| Vertex {
            position: Vec3::new(x, y, 0.0),
            ..Vertex::default()
        };
        Submesh {
            node: 0,
            material: NO_MATERIAL,
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(1.0, 0.0),
                vertex(1.0, 1.0),
                vertex(0.0, 1.0),
                vertex(2.0, 0.0),
                vertex(2.0, 1.0),
            ],
            uv_channels: Vec::new(),
            color_channels: Vec::new(),
            vertex_crease: Vec::new(),
            indices: vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2],
            polygons: Some(PolygonCarry {
                face_offsets: vec![0, 4, 8],
                corners: vec![0, 1, 2, 3, 1, 4, 5, 2],
                source_face: vec![u32::MAX, u32::MAX],
                triangle_face: vec![0, 0, 1, 1],
                rebuilt: true,
                ..PolygonCarry::default()
            }),
            skin: Default::default(),
            extra_skins: Vec::new(),
            dq_weight: Default::default(),
            morph: Default::default(),
            source_corner: Vec::new(),
        }
    }

    #[test]
    fn corner_run_of_a_rebuilt_quad_carry_validates() {
        let piece = two_quads();
        let run = corner_run(&piece);

        assert_eq!(run.vertices.len(), 8, "two quads, four corners each");
        assert_eq!(run.faces.len(), 2);
        assert_eq!(run.control_point, vec![0, 1, 2, 3, 1, 4, 5, 2]);
        // Contiguous runs in face order, exactly as import lays a mesh out.
        assert_eq!(run.faces[0].first_index, 0);
        assert_eq!(run.faces[0].index_count, 4);
        assert_eq!(run.faces[1].first_index, 4);
        assert_eq!(run.faces[1].index_count, 4);
        assert_eq!(run.to_face, vec![0, 0, 1, 1]);
        assert_eq!(run.indices.len(), 12);

        // Every run triangle addresses the run, and every triangle's corners sit
        // inside its own face's run.
        for (triangle, corners) in run.indices.as_chunks::<3>().0.iter().enumerate() {
            let face = run.faces[run.to_face[triangle] as usize];
            for &corner in corners {
                assert!((corner as usize) < run.vertices.len());
                assert!(corner >= face.first_index);
                assert!(corner < face.first_index + face.index_count);
            }
        }

        let tags = TriangleData {
            to_face: run.to_face.clone(),
            material: Vec::new(),
            node: Vec::new(),
        };
        assert!(
            tags.validate(run.indices.len() / 3, run.faces.len(), 0, 0)
                .is_ok()
        );
    }

    #[test]
    fn a_piece_with_no_carry_runs_every_triangle_as_its_own_face() {
        let mut piece = two_quads();
        piece.polygons = None;

        let run = corner_run(&piece);

        assert_eq!(run.faces.len(), 4, "four triangles, four faces");
        assert_eq!(run.vertices.len(), 12);
        assert_eq!(run.to_face, vec![0, 1, 2, 3]);
        assert_eq!(run.indices, (0..12).collect::<Vec<u32>>());
    }

    #[test]
    fn canonicalize_is_order_independent() {
        let positions = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
        ];
        let mut first = RemeshOutput {
            positions: positions.clone(),
            face_offsets: vec![0, 4, 8],
            corners: vec![0, 1, 2, 3, 1, 4, 5, 2],
        };
        // The same two quads: faces emitted in the other order, each rotated,
        // and the vertices numbered differently.
        let shuffled = vec![
            positions[1],
            positions[4],
            positions[5],
            positions[2],
            positions[3],
            positions[0],
        ];
        let mut second = RemeshOutput {
            positions: shuffled,
            face_offsets: vec![0, 4, 8],
            corners: vec![3, 4, 5, 0, 2, 3, 0, 1],
        };

        canonicalize(&mut first);
        canonicalize(&mut second);

        assert_eq!(first.positions, second.positions);
        assert_eq!(first.corners, second.corners);
        assert_eq!(first.face_offsets, second.face_offsets);
    }
}
