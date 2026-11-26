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

use std::collections::HashMap;

use glam::Vec3;
use review_model::ModelData;

use crate::OptError;
use crate::Warnings;
use crate::meshopt::{self, TANGENT_COMPONENTS};
use crate::notice::OptWarning;
use crate::process::{RunContext, is_excluded, resolve_op};
use crate::stack::{NormalParams, NormalWeighting, OpInstance, OpKind, OptStack, WeldParams};
use crate::submesh::{PolygonCarry, Submesh};

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
            warnings.push(OptWarning::TangentsFailed {
                detail: error.to_string(),
            });
        }
    }
}

/// Per-corner normals for one indexed surface, from its positions alone:
/// `indices.len()` of them. `params.crease_angle` is in degrees.
pub(crate) fn corner_normals(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    params: &NormalParams,
) -> Result<Vec<Vec3>, OptError> {
    if params.weighting != NormalWeighting::AngleAndEdges {
        return Ok(weighted_corner_normals(indices, positions, params));
    }
    let flat = meshopt::generate_normals(
        indices,
        positions,
        vertex_count,
        params.crease_angle.to_radians(),
        params.smoothing,
    )?;
    Ok(flat
        .as_chunks::<3>()
        .0
        .iter()
        .map(|n| Vec3::from_array(*n).normalize_or(Vec3::Z))
        .collect())
}

/// Per-corner normals weighted as `params` says, for the weightings
/// meshoptimizer does not offer.
///
/// Corners are grouped by *position*, as meshoptimizer groups them: around each
/// point, the faces joined through shared edges that meet at less than the
/// crease angle form one fan and share one normal; a sharper edge keeps the fans
/// either side of it apart (Blender's "Keep Sharp"). Each fan's normal is its
/// faces' normals, weighted, summed and normalized.
fn weighted_corner_normals(indices: &[u32], positions: &[f32], params: &NormalParams) -> Vec<Vec3> {
    let triangles = indices.as_chunks::<3>().0;
    let position = |index: u32| {
        let at = index as usize * 3;
        Vec3::new(positions[at], positions[at + 1], positions[at + 2])
    };
    // Each vertex's position, as an id shared by every vertex there.
    let mut ids: HashMap<[u32; 3], u32> = HashMap::new();
    let vertex_count = positions.len() / 3;
    let point: Vec<u32> = (0..vertex_count as u32)
        .map(|vertex| {
            let next = ids.len() as u32;
            *ids.entry(
                position(vertex)
                    .to_array()
                    .map(|value| if value == 0.0 { 0 } else { value.to_bits() }),
            )
            .or_insert(next)
        })
        .collect();

    let normal: Vec<Vec3> = triangles
        .iter()
        .map(|corners| {
            let [a, b, c] = corners.map(position);
            (b - a).cross(c - a)
        })
        .collect();
    let unit: Vec<Vec3> = normal.iter().map(|n| n.normalize_or_zero()).collect();
    let cosine = review_model::exact::cos(params.crease_angle.clamp(0.0, 180.0).to_radians());

    // Faces sharing an edge (by points), for the fans.
    let mut edge_faces: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (face, corners) in triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (
                point[corners[k] as usize],
                point[corners[(k + 1) % 3] as usize],
            );
            edge_faces
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push(face as u32);
        }
    }
    // Per corner, a group: corners at one point whose faces are joined through
    // smooth shared edges at that point are one group (union-find over corners).
    let mut parent: Vec<u32> = (0..indices.len() as u32).collect();
    fn root(parent: &mut [u32], mut at: u32) -> u32 {
        while parent[at as usize] != at {
            parent[at as usize] = parent[parent[at as usize] as usize];
            at = parent[at as usize];
        }
        at
    }
    let corner_at = |face: usize, at: u32| -> Option<usize> {
        (0..3)
            .find(|&k| point[triangles[face][k] as usize] == at)
            .map(|k| face * 3 + k)
    };
    for (face, corners) in triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (
                point[corners[k] as usize],
                point[corners[(k + 1) % 3] as usize],
            );
            let Some(faces) = edge_faces.get(&(a.min(b), a.max(b))) else {
                continue;
            };
            for &other in faces {
                let other = other as usize;
                if other <= face || unit[face].dot(unit[other]) < cosine {
                    continue;
                }
                for at in [a, b] {
                    if let (Some(mine), Some(theirs)) = (corner_at(face, at), corner_at(other, at))
                    {
                        let (x, y) = (
                            root(&mut parent, mine as u32),
                            root(&mut parent, theirs as u32),
                        );
                        if x != y {
                            parent[x.max(y) as usize] = x.min(y);
                        }
                    }
                }
            }
        }
    }

    let mut sum = vec![Vec3::ZERO; indices.len()];
    for (face, corners) in triangles.iter().enumerate() {
        let area = 0.5 * normal[face].length();
        for k in 0..3 {
            let [here, next, previous] =
                [k, (k + 1) % 3, (k + 2) % 3].map(|at| position(corners[at]));
            let angle = review_model::exact::acos(
                (next - here)
                    .normalize_or_zero()
                    .dot((previous - here).normalize_or_zero())
                    .clamp(-1.0, 1.0),
            );
            let weight = match params.weighting {
                NormalWeighting::Area => area,
                NormalWeighting::Angle => angle,
                NormalWeighting::AreaAndAngle | NormalWeighting::AngleAndEdges => area * angle,
            };
            let group = root(&mut parent, (face * 3 + k) as u32) as usize;
            sum[group] += unit[face] * weight;
        }
    }
    (0..indices.len())
        .map(|corner| {
            let group = root(&mut parent, corner as u32) as usize;
            sum[group]
                .try_normalize()
                .unwrap_or(unit[corner / 3])
                .normalize_or(Vec3::Z)
        })
        .collect()
}

/// The Recalculate Normals operation: every eligible object's normals are
/// regenerated from its shape, with the operation's crease angle deciding which
/// edges stay hard.
///
/// Whole-object rather than per piece: an object's material pieces share one
/// surface, and generating each alone would put a hard edge along every
/// material border. So an object's pieces are concatenated, the normals are
/// generated once, and each piece then takes its share — splitting a vertex
/// where its corners landed on different sides of a crease, and merging back
/// the vertices that were only ever apart because their old normals differed.
///
/// A skinned object is recalculated (skin weights follow a split exactly). One
/// with blend shapes is skipped: its shapes carry normal *offsets* authored
/// against the old normals, which the new ones would silently contradict.
pub(crate) fn recalculate_normals_submeshes(
    submeshes: &mut [Submesh],
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    warnings: &mut Warnings,
    run: &RunContext<'_>,
) {
    let _z = crate::prof::zone!("Recalculate Normals");
    let OpKind::RecalculateNormals(global) = &op.kind else {
        return;
    };

    let mut seen = std::collections::HashSet::new();
    let nodes: Vec<u32> = submeshes
        .iter()
        .map(|piece| piece.node)
        .filter(|&node| seen.insert(node))
        .collect();

    for node in nodes {
        if run.cancelled() {
            return;
        }
        if is_excluded(stack, node) {
            continue;
        }
        let members: Vec<usize> = (0..submeshes.len())
            .filter(|&at| submeshes[at].node == node && !submeshes[at].is_empty())
            .collect();
        if members.is_empty() {
            continue;
        }
        if members.iter().any(|&at| !submeshes[at].morph.is_empty()) {
            let name = crate::remesh::node_name(model, node);
            warnings.push(OptWarning::NormalsKeptForBlendShapes {
                object: name.to_string(),
            });
            continue;
        }
        let params = match resolve_op(stack, op, node) {
            OpKind::RecalculateNormals(params) => *params,
            // An override can only ever hold the same kind as the operation it
            // overrides; fall back to the global settings if one somehow doesn't.
            _ => *global,
        };
        let pieces: Vec<&mut Submesh> = submeshes
            .iter_mut()
            .enumerate()
            .filter(|(at, _)| members.contains(at))
            .map(|(_, piece)| piece)
            .collect();
        if let Err(error) = recalculate_node(pieces, &params) {
            let name = crate::remesh::node_name(model, node);
            warnings.push(OptWarning::NormalsFailed {
                object: name.to_string(),
                detail: error.to_string(),
            });
        }
    }
}

/// One object's pieces, recalculated together.
fn recalculate_node(mut pieces: Vec<&mut Submesh>, params: &NormalParams) -> Result<(), OptError> {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut vertex_count = 0usize;
    for piece in &pieces {
        let base = u32::try_from(vertex_count).map_err(|_| OptError::SizeOverflow)?;
        positions.extend_from_slice(&piece.positions());
        indices.extend(piece.indices.iter().map(|&index| index + base));
        vertex_count += piece.vertices.len();
    }
    let normals = corner_normals(&indices, &positions, vertex_count, params)?;

    let mut first_corner = 0usize;
    for piece in &mut pieces {
        let corners = piece.indices.len();
        let assigned = piece.split_by_corner(&normals[first_corner..first_corner + corners]);
        first_corner += corners;
        for (vertex, normal) in piece.vertices.iter_mut().zip(assigned) {
            if let Some(normal) = normal {
                vertex.normal = normal;
            }
        }
        piece.normals_stale = false;
        // Vertices that were apart only because their *old* normals differed
        // now match in every attribute; an exact weld puts them back together.
        crate::ops::weld(piece, &WeldParams::default())?;
    }

    derive_edge_smoothing(&mut pieces);
    Ok(())
}

/// A vector's exact bits, as a hashable key.
fn bits(v: Vec3) -> [u32; 3] {
    v.to_array().map(f32::to_bits)
}

/// Per geometric edge (its two end positions, in a fixed order), every distinct
/// pair of end normals the faces along it use: one pair means the sides agree,
/// two or more that the edge is hard.
type EdgeSides = HashMap<([u32; 3], [u32; 3]), Vec<([u32; 3], [u32; 3])>>;

/// Rewrite the carried hard/soft edge flags to agree with the normals the
/// pieces now have, and drop the face smoothing groups, which would otherwise
/// tell a DCC to rebuild the old shading on import.
///
/// An edge is hard where the faces on either side of it disagree about the
/// normal at either end — read by *position*, because the two sides of a hard
/// edge are different vertices and may even sit in different pieces (a
/// material border). An edge only one face touches is left soft: there is no
/// second side for it to be hard against.
fn derive_edge_smoothing(pieces: &mut [&mut Submesh]) {
    let mut sides = EdgeSides::new();
    for piece in pieces.iter() {
        let Some(carry) = &piece.polygons else {
            continue;
        };
        for face in 0..carry.face_count() {
            let corners = carry.face(face);
            for k in 0..corners.len() {
                let (Some(a), Some(b)) = (
                    piece.vertices.get(corners[k] as usize),
                    piece
                        .vertices
                        .get(corners[(k + 1) % corners.len()] as usize),
                ) else {
                    continue;
                };
                // Keyed position-first-then-second in a fixed order, so both
                // faces sharing the edge file their normals under one key.
                let (first, second) = if bits(a.position) <= bits(b.position) {
                    (a, b)
                } else {
                    (b, a)
                };
                let key = (bits(first.position), bits(second.position));
                let side = (bits(first.normal), bits(second.normal));
                let entry = sides.entry(key).or_default();
                if !entry.contains(&side) {
                    entry.push(side);
                }
            }
        }
    }

    for piece in pieces.iter_mut() {
        let vertices = piece.vertices.clone();
        let Some(carry) = &mut piece.polygons else {
            continue;
        };
        apply_edge_smoothing(carry, &vertices, &sides);
    }
}

fn apply_edge_smoothing(
    carry: &mut PolygonCarry,
    vertices: &[review_model::Vertex],
    sides: &EdgeSides,
) {
    carry.face_smoothing.clear();
    carry.edge_smoothing = carry
        .edges
        .iter()
        .map(|edge| {
            let (Some(a), Some(b)) = (
                vertices.get(edge[0] as usize),
                vertices.get(edge[1] as usize),
            ) else {
                return true;
            };
            let (pa, pb) = (bits(a.position), bits(b.position));
            let key = if pa <= pb { (pa, pb) } else { (pb, pa) };
            // Soft unless the sides meeting here disagree.
            sides.get(&key).is_none_or(|sides| sides.len() < 2)
        })
        .collect();
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

    /// A flat square beside a narrow chamfer turned 20 degrees down from it,
    /// sharing the square's right edge: positions, and its four triangles.
    fn square_and_chamfer() -> (Vec<f32>, Vec<u32>) {
        let (cos, sin) = (20f32.to_radians().cos(), 20f32.to_radians().sin());
        let width = 0.05;
        let points = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0 + width * cos, 0.0, -width * sin],
            [1.0 + width * cos, 1.0, -width * sin],
        ];
        let positions = points.iter().flatten().copied().collect();
        (positions, vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2])
    }

    fn turn_from_up(normal: Vec3) -> f32 {
        normal.dot(Vec3::Z).clamp(-1.0, 1.0).acos().to_degrees()
    }

    #[test]
    fn face_area_weighting_keeps_the_flat_face_flat_at_its_edge() {
        let (positions, indices) = square_and_chamfer();
        let normals = |weighting| {
            let params = NormalParams {
                crease_angle: 60.0,
                weighting,
                ..NormalParams::default()
            };
            weighted_corner_normals(&indices, &positions, &params)
        };
        // Corner 1 is the square's corner on the shared edge.
        let area = turn_from_up(normals(NormalWeighting::Area)[1]);
        let angle = turn_from_up(normals(NormalWeighting::Angle)[1]);
        // The square outweighs the chamfer ten to one by area: it leans less
        // than 2.5 degrees where an even split would lean 10.
        assert!(
            area < 2.5,
            "the big face's normal wins by area: {area} degrees"
        );
        assert!(area < angle, "area {area} against angle {angle}");
        // Smooth across: the chamfer's corner there shares the normal.
        let shared = normals(NormalWeighting::Area);
        assert_eq!(shared[1], shared[6]);
    }

    #[test]
    fn an_edge_sharper_than_the_crease_stays_hard() {
        let (positions, indices) = square_and_chamfer();
        let params = NormalParams {
            crease_angle: 10.0,
            weighting: NormalWeighting::Area,
            ..NormalParams::default()
        };
        let normals = weighted_corner_normals(&indices, &positions, &params);
        assert!(
            turn_from_up(normals[1]) < 1.0e-3,
            "the square keeps its own"
        );
        assert!(
            (turn_from_up(normals[6]) - 20.0).abs() < 1.0e-2,
            "and the chamfer its own"
        );
    }

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
            custom: Default::default(),
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

    /// Two quads meeting at a right angle along the edge 1–2: the left one in
    /// the XY plane, the right one folded back into the plane x = 1.
    fn folded_quads() -> Submesh {
        let at = |x: f32, y: f32, z: f32| Vertex {
            position: Vec3::new(x, y, z),
            normal: Vec3::Z,
            ..Vertex::default()
        };
        let mut quads = piece(
            vec![
                at(0.0, 0.0, 0.0),
                at(1.0, 0.0, 0.0),
                at(1.0, 1.0, 0.0),
                at(0.0, 1.0, 0.0),
                at(1.0, 0.0, -1.0),
                at(1.0, 1.0, -1.0),
            ],
            vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2],
        );
        quads.polygons = Some(PolygonCarry {
            face_offsets: vec![0, 4, 8],
            corners: vec![0, 1, 2, 3, 1, 4, 5, 2],
            source_face: vec![0, 1],
            triangle_face: vec![0, 0, 1, 1],
            face_smoothing: vec![true, true],
            edges: vec![[0, 1], [1, 2], [2, 3], [3, 0], [1, 4], [4, 5], [5, 2]],
            source_edge: (0..7).collect(),
            ..PolygonCarry::default()
        });
        for i in 0..6u32 {
            quads.skin.push_row(&[(i, 1.0)]);
        }
        quads
    }

    fn recalculate(
        pieces: &mut [Submesh],
        params: NormalParams,
        stack: Option<OptStack>,
    ) -> Vec<OptWarning> {
        let mut stack = stack.unwrap_or_default();
        let id = stack.push_op(OpKind::RecalculateNormals(params));
        let op = stack.op(id).expect("just pushed").clone();
        let progress = |_: crate::process::OptProgress<'_>| {};
        let run = RunContext::new(&progress, None);
        let mut warnings = Warnings::default();
        recalculate_normals_submeshes(
            pieces,
            &op,
            &stack,
            &ModelData::default(),
            &mut warnings,
            &run,
        );
        warnings.into_vec()
    }

    #[test]
    fn a_fold_sharper_than_the_crease_splits_and_carries_the_skin() {
        let mut pieces = vec![folded_quads()];
        let warnings = recalculate(
            &mut pieces,
            NormalParams {
                crease_angle: 45.0,
                smoothing: 0.0,
                ..Default::default()
            },
            None,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let quads = &pieces[0];
        assert_eq!(quads.vertices.len(), 8, "the fold's two vertices split");
        assert_eq!(quads.skin.offsets.len(), 9, "one skin row per vertex");
        // Every normal is one face's own (the left faces +Z, the folded one
        // along X), and each fold vertex now exists once per side.
        for vertex in &quads.vertices {
            let n = vertex.normal;
            assert!(
                n.z.abs() > 0.999 || n.x.abs() > 0.999,
                "a face's own normal, not an average: {n:?}"
            );
        }
        for fold in [Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0)] {
            let sides: Vec<Vec3> = quads
                .vertices
                .iter()
                .filter(|v| v.position == fold)
                .map(|v| v.normal)
                .collect();
            assert_eq!(sides.len(), 2, "one vertex per side at {fold:?}");
            assert!(sides[0].dot(sides[1]).abs() < 1e-3, "the two sides differ");
        }
        // Skin rows follow their vertex: each vertex's row names the original
        // it descends from, found by position.
        let originals = folded_quads();
        for (at, vertex) in quads.vertices.iter().enumerate() {
            let original = originals
                .vertices
                .iter()
                .position(|v| v.position == vertex.position)
                .expect("every vertex descends from an original");
            assert_eq!(quads.skin.row(at), &[(original as u32, 1.0)]);
        }
        assert!(!quads.normals_stale);
    }

    #[test]
    fn a_fold_gentler_than_the_crease_is_smoothed_without_a_split() {
        let mut pieces = vec![folded_quads()];
        recalculate(
            &mut pieces,
            NormalParams {
                crease_angle: 120.0,
                smoothing: 0.0,
                ..Default::default()
            },
            None,
        );
        let quads = &pieces[0];
        assert_eq!(quads.vertices.len(), 6);
        let shared = quads.vertices[1].normal;
        assert!(
            shared.x.abs() > 0.1 && shared.z.abs() > 0.1,
            "averaged across the fold: {shared:?}"
        );
    }

    #[test]
    fn edge_flags_follow_the_new_normals_and_smoothing_groups_go() {
        let mut pieces = vec![folded_quads()];
        recalculate(
            &mut pieces,
            NormalParams {
                crease_angle: 45.0,
                smoothing: 0.0,
                ..Default::default()
            },
            None,
        );
        let carry = pieces[0].polygons.as_ref().expect("the carry survives");
        assert!(carry.face_smoothing.is_empty());
        assert_eq!(carry.edge_smoothing.len(), carry.edges.len());
        for (edge, &smooth) in carry.source_edge.iter().zip(&carry.edge_smoothing) {
            assert_eq!(smooth, *edge != 1, "only the fold (source edge 1) is hard");
        }

        let mut soft = vec![folded_quads()];
        recalculate(
            &mut soft,
            NormalParams {
                crease_angle: 120.0,
                smoothing: 0.0,
                ..Default::default()
            },
            None,
        );
        let carry = soft[0].polygons.as_ref().expect("the carry survives");
        assert!(carry.edge_smoothing.iter().all(|&smooth| smooth));
    }

    #[test]
    fn material_pieces_of_one_object_stay_smooth_across_their_border() {
        // The folded pair again, but each quad its own material piece: the
        // border between them must not become a hard edge on its own.
        let whole = folded_quads();
        let mut left = piece(whole.vertices[..4].to_vec(), vec![0, 1, 2, 0, 2, 3]);
        let mut right = piece(
            vec![
                whole.vertices[1],
                whole.vertices[4],
                whole.vertices[5],
                whole.vertices[2],
            ],
            vec![0, 1, 2, 0, 2, 3],
        );
        left.material = 0;
        right.material = 1;
        let mut pieces = vec![left, right];
        recalculate(
            &mut pieces,
            NormalParams {
                crease_angle: 120.0,
                smoothing: 0.0,
                ..Default::default()
            },
            None,
        );
        assert_eq!(pieces[0].vertices[1].normal, pieces[1].vertices[0].normal);
    }

    #[test]
    fn an_object_with_blend_shapes_is_skipped_with_a_warning() {
        let mut quads = folded_quads();
        for _ in 0..6 {
            quads.morph.push_row(&[]);
        }
        let before = quads.vertices.clone();
        let mut pieces = vec![quads];
        let warnings = recalculate(&mut pieces, NormalParams::default(), None);
        assert_eq!(pieces[0].vertices, before);
        assert!(
            warnings
                .iter()
                .any(|w| matches!(w, OptWarning::NormalsKeptForBlendShapes { .. })),
            "{warnings:?}"
        );
    }

    #[test]
    fn an_excluded_object_is_untouched() {
        let mut stack = OptStack::default();
        stack.node_override_mut(0).exclude = true;
        let before = folded_quads().vertices;
        let mut pieces = vec![folded_quads()];
        recalculate(&mut pieces, NormalParams::default(), Some(stack));
        assert_eq!(pieces[0].vertices, before);
    }
}
