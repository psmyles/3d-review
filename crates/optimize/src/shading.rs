//! The derived shading bases — normals and tangents — of processed pieces.
//!
//! Both are computed from the geometry, so an operation that changes the shape
//! leaves them stale, and each is rebuilt here, on the [`Submesh`]es, before a
//! level is assembled. Working on the pieces rather than on the assembled model
//! is what lets a rebuild *split* a vertex: meshoptimizer answers per corner,
//! and two corners of one vertex can disagree (either side of a hard edge, either
//! side of a mirrored-UV seam). [`Submesh::split_by_corner`] turns that into
//! vertices while carrying every channel and deform row along, and everything
//! downstream — the assembled model, the corner-run layout, the export's control
//! points, the measured stats — simply sees a mesh with those vertices.
//!
//! The tangent pass must be the **last** thing to touch a level's pieces: a weld
//! never compares tangents ([`crate::ops::weld`]), so any weld after it would
//! merge the copies it made back together.

use glam::Vec3;

use crate::OptError;
use crate::Warnings;
use crate::meshopt::{self, TANGENT_COMPONENTS};
use crate::process::is_excluded;
use crate::stack::OptStack;
use crate::submesh::Submesh;

/// Finish a level's pieces: smooth the normals a normal-blind weld left stale,
/// then — when `tangents_invalidated` — rebuild every piece's tangents.
/// Excluded objects are left exactly as they came in.
pub(crate) fn finish_bases(
    submeshes: &mut [Submesh],
    stack: &OptStack,
    tangents_invalidated: bool,
    warnings: &mut Warnings,
) {
    let _z = crate::prof::zone!("Finish Shading Bases");
    for piece in submeshes.iter_mut() {
        if is_excluded(stack, piece.node) || piece.is_empty() {
            continue;
        }
        // Normals first: tangents are orthonormalized against them, so
        // rebuilding tangents from stale normals would bake the staleness into
        // both.
        if piece.normals_stale {
            review_model::smooth_vertex_normals(&mut piece.vertices, &piece.indices);
            piece.normals_stale = false;
        }
        if tangents_invalidated && let Err(error) = rebuild_tangents(piece) {
            warnings.push(&format!("Couldn't rebuild the tangents: {error}"));
        }
    }
}

/// Rebuild `piece`'s tangents with meshoptimizer's generator, splitting a
/// vertex only where its corners disagree on *handedness* — a mirrored-UV seam,
/// where one vertex genuinely needs two bases. Corners that agree share their
/// vertex and have their tangents averaged, then made exactly perpendicular to
/// the vertex normal.
///
/// A corner touching only degenerate UV triangles has no tangent of its own
/// (upstream's zero fallback): it takes its vertex's other corners' handedness
/// and contributes nothing to the average, so it never forces a split. A vertex
/// with no usable corner at all falls back to an arbitrary perpendicular with
/// `w = +1`, as [`review_model::ModelData::generate_tangents`] does, so the basis
/// is never zero.
pub(crate) fn rebuild_tangents(piece: &mut Submesh) -> Result<(), OptError> {
    let _z = crate::prof::zone!("Rebuild Tangents");
    if piece.is_empty() {
        return Ok(());
    }

    let vertex_count = piece.vertices.len();
    let positions = piece.positions();
    let mut normals = Vec::with_capacity(vertex_count * 3);
    let mut uvs = Vec::with_capacity(vertex_count * 2);
    for vertex in &piece.vertices {
        normals.extend_from_slice(&vertex.normal.normalize_or_zero().to_array());
        uvs.extend_from_slice(&vertex.uv.to_array());
    }
    let tangents =
        meshopt::generate_tangents(&piece.indices, &positions, &normals, &uvs, vertex_count)?;
    let corner = |index: usize| {
        let t = &tangents[index * TANGENT_COMPONENTS..(index + 1) * TANGENT_COMPONENTS];
        (Vec3::new(t[0], t[1], t[2]), t[3])
    };

    // Each vertex's handedness, from its first corner that has a tangent; the
    // wildcard corners adopt it.
    let mut vertex_sign: Vec<Option<bool>> = vec![None; vertex_count];
    for (index, &vertex) in piece.indices.iter().enumerate() {
        let (direction, w) = corner(index);
        if direction != Vec3::ZERO {
            vertex_sign[vertex as usize].get_or_insert(w < 0.0);
        }
    }
    let mirrored: Vec<bool> = piece
        .indices
        .iter()
        .enumerate()
        .map(|(index, &vertex)| {
            let (direction, w) = corner(index);
            if direction != Vec3::ZERO {
                w < 0.0
            } else {
                vertex_sign[vertex as usize].unwrap_or(false)
            }
        })
        .collect();

    let assigned = piece.split_by_corner(&mirrored);

    let mut accumulated = vec![Vec3::ZERO; piece.vertices.len()];
    for (index, &vertex) in piece.indices.iter().enumerate() {
        accumulated[vertex as usize] += corner(index).0;
    }
    for ((vertex, sum), mirrored) in piece.vertices.iter_mut().zip(accumulated).zip(assigned) {
        let normal = vertex.normal.normalize_or_zero();
        let projected = sum - normal * normal.dot(sum);
        let direction = if projected.length_squared() > 1e-12 {
            projected.normalize()
        } else {
            normal.any_orthonormal_vector()
        };
        let w = if mirrored == Some(true) { -1.0 } else { 1.0 };
        vertex.tangent = direction.extend(w);
    }
    Ok(())
}

#[cfg(all(test, has_meshopt))]
mod tests {
    use glam::{Vec2, Vec4};
    use review_model::{ModelData, Vertex};

    use super::*;
    use crate::submesh::{NO_MATERIAL, PolygonCarry, VertexRows};

    fn piece(vertices: Vec<Vertex>, indices: Vec<u32>) -> Submesh {
        Submesh {
            node: 0,
            material: NO_MATERIAL,
            vertices,
            uv_channels: Vec::new(),
            color_channels: Vec::new(),
            vertex_crease: Vec::new(),
            indices,
            polygons: None,
            skin: VertexRows::default(),
            extra_skins: Vec::new(),
            dq_weight: VertexRows::default(),
            morph: VertexRows::default(),
            source_corner: Vec::new(),
            normals_stale: false,
        }
    }

    fn vertex(x: f32, y: f32, u: f32, v: f32) -> Vertex {
        Vertex {
            position: Vec3::new(x, y, 0.0),
            normal: Vec3::Z,
            uv: Vec2::new(u, v),
            ..Vertex::default()
        }
    }

    /// A 2×1 strip of quads sharing the middle column, whose right half has its
    /// U mirrored about that column — the layout a symmetric model's UVs have.
    fn mirrored_strip() -> Submesh {
        piece(
            vec![
                vertex(0.0, 0.0, 0.0, 0.0),
                vertex(1.0, 0.0, 1.0, 0.0),
                vertex(1.0, 1.0, 1.0, 1.0),
                vertex(0.0, 1.0, 0.0, 1.0),
                vertex(2.0, 0.0, 0.0, 0.0),
                vertex(2.0, 1.0, 0.0, 1.0),
            ],
            vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2],
        )
    }

    #[test]
    fn a_mirror_seam_splits_exactly_the_seam_vertices() {
        let mut strip = mirrored_strip();
        rebuild_tangents(&mut strip).expect("rebuilds");
        assert_eq!(
            strip.vertices.len(),
            8,
            "the two seam vertices gained a copy"
        );
        let signs: Vec<f32> = strip.vertices.iter().map(|v| v.tangent.w).collect();
        assert!(signs.contains(&1.0) && signs.contains(&-1.0), "{signs:?}");
        for triangle in strip.indices.as_chunks::<3>().0 {
            let w = strip.vertices[triangle[0] as usize].tangent.w;
            assert!(
                triangle
                    .iter()
                    .all(|&i| strip.vertices[i as usize].tangent.w == w),
                "every triangle is one-handed after the split"
            );
        }
    }

    #[test]
    fn an_unmirrored_surface_gains_no_vertices_and_matches_the_old_convention() {
        let mut strip = mirrored_strip();
        // Un-mirror the right half.
        strip.vertices[4].uv = Vec2::new(2.0, 0.0);
        strip.vertices[5].uv = Vec2::new(2.0, 1.0);
        let mut reference = ModelData {
            vertices: strip.vertices.clone(),
            indices: strip.indices.clone(),
            ..ModelData::default()
        };
        reference.generate_tangents();

        rebuild_tangents(&mut strip).expect("rebuilds");
        assert_eq!(strip.vertices.len(), 6);
        for (ours, theirs) in strip.vertices.iter().zip(&reference.vertices) {
            assert_eq!(
                ours.tangent.w, theirs.tangent.w,
                "same handedness convention"
            );
            assert!(ours.tangent.truncate().dot(theirs.tangent.truncate()) > 0.99);
        }
    }

    #[test]
    fn rebuilt_tangents_are_unit_and_perpendicular_to_the_normal() {
        let mut strip = mirrored_strip();
        rebuild_tangents(&mut strip).expect("rebuilds");
        for vertex in &strip.vertices {
            let t = vertex.tangent.truncate();
            assert!((t.length() - 1.0).abs() < 1e-5);
            assert!(t.dot(vertex.normal).abs() < 1e-5);
        }
    }

    #[test]
    fn corners_without_uv_area_never_force_a_split() {
        // All UVs equal: every corner is a zero-tangent wildcard.
        let mut flat = mirrored_strip();
        for vertex in &mut flat.vertices {
            vertex.uv = Vec2::ZERO;
        }
        rebuild_tangents(&mut flat).expect("rebuilds");
        assert_eq!(flat.vertices.len(), 6);
        assert!(
            flat.vertices
                .iter()
                .all(|v| v.tangent.w == 1.0 && v.tangent.truncate().length() > 0.99),
            "the fallback basis is never zero"
        );
    }

    #[test]
    fn a_split_copies_every_per_vertex_row() {
        let mut strip = mirrored_strip();
        strip.color_channels = vec![(0..6).map(|i| Vec4::splat(i as f32)).collect()];
        strip.vertex_crease = (0..6).map(|i| i as f32 * 0.1).collect();
        strip.source_corner = (10..16).collect();
        for i in 0..6u32 {
            strip.skin.push_row(&[(i, 1.0)]);
        }
        rebuild_tangents(&mut strip).expect("rebuilds");

        assert_eq!(strip.color_channels[0].len(), 8);
        assert_eq!(strip.vertex_crease.len(), 8);
        assert_eq!(strip.source_corner.len(), 8);
        assert_eq!(strip.skin.offsets.len(), 9);
        // The copies are of the seam vertices 1 and 2, in corner order.
        for (copy, original) in [(6usize, 1usize), (7, 2)] {
            assert_eq!(
                strip.vertices[copy].position,
                strip.vertices[original].position
            );
            assert_eq!(strip.color_channels[0][copy], Vec4::splat(original as f32));
            assert_eq!(strip.source_corner[copy], 10 + original as u32);
            assert_eq!(strip.skin.row(copy), &[(original as u32, 1.0)]);
        }
    }

    /// Two quads sharing the edge 1–2, carried as polygons with their edges and
    /// an authored edge layer.
    fn two_carried_quads() -> Submesh {
        let mut quads = mirrored_strip();
        quads.polygons = Some(PolygonCarry {
            face_offsets: vec![0, 4, 8],
            corners: vec![0, 1, 2, 3, 1, 4, 5, 2],
            source_face: vec![0, 1],
            triangle_face: vec![0, 0, 1, 1],
            edges: vec![[0, 1], [1, 2], [2, 3], [3, 0], [1, 4], [4, 5], [5, 2]],
            source_edge: (0..7).collect(),
            edge_crease: vec![0.5; 7],
            ..PolygonCarry::default()
        });
        quads
    }

    #[test]
    fn corners_of_one_face_never_split_apart() {
        let mut quads = two_carried_quads();
        // Disagree *inside* face 0 at vertex 0 (its two triangles both use it):
        // the face's first corner wins, so no copy is made.
        let corners = [0u8, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        let values = quads.split_by_corner(&corners);
        assert_eq!(quads.vertices.len(), 6);
        assert_eq!(values[0], Some(0));
    }

    #[test]
    fn a_split_between_faces_renumbers_the_carry_and_fans_the_shared_edge() {
        let mut quads = two_carried_quads();
        // Face 0 says 0, face 1 says 1: the two shared vertices split.
        let corners = [0u8, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];
        let values = quads.split_by_corner(&corners);
        assert_eq!(quads.vertices.len(), 8);
        assert_eq!(values[6..], [Some(1), Some(1)]);

        let carry = quads.polygons.as_ref().expect("the carry survives");
        assert_eq!(carry.face(0), &[0, 1, 2, 3], "face 0 keeps the originals");
        assert_eq!(carry.face(1), &[6, 4, 5, 7], "face 1 uses the copies");
        // The shared edge is now two edges, one per side, each keeping its layers.
        let shared: Vec<usize> = (0..carry.edges.len())
            .filter(|&e| carry.source_edge[e] == 1)
            .collect();
        assert_eq!(shared.len(), 2);
        let mut pairs: Vec<[u32; 2]> = shared.iter().map(|&e| carry.edges[e]).collect();
        pairs.sort_unstable();
        assert_eq!(pairs, vec![[1, 2], [6, 7]]);
        assert_eq!(carry.edges.len(), 8);
        assert_eq!(carry.edge_crease.len(), 8);
        // Face 1's own edges follow it onto the copies.
        let edge_of =
            |source: u32| carry.edges[carry.source_edge.iter().position(|&s| s == source).unwrap()];
        assert_eq!(edge_of(4), [6, 4]);
        assert_eq!(edge_of(6), [5, 7]);
    }
}
