//! The Convex Hull operation: an object replaced by its convex hull, built from
//! at most a set number of vertices — what a physics engine takes as a convex
//! collision mesh.
//!
//! The hull is grown the way Quickhull grows it, one point at a time, always
//! taking the point furthest outside the hull so far. Stopping once it has the
//! vertices it is allowed gives the hull of the points that matter most: every
//! point left out is within the last step's distance of it. That is an inner
//! hull, as a physics engine's own vertex limit makes one, rather than an outer
//! one with planes pushed out to cover everything.

use std::collections::HashMap;

use glam::{DVec3, Vec3};
use review_model::ModelData;
use review_model::glob::GlobSet;

use crate::Warnings;
use crate::notice::OptWarning;
use crate::remesh::{self, nodes};
use crate::stack::{HULL_VERTICES_MIN, HullParams, NormalParams, OpInstance, OpKind, OptStack};
use crate::submesh::Submesh;

/// A hull: its vertices, and its triangles over them, wound outward.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Hull {
    pub(crate) positions: Vec<Vec3>,
    pub(crate) triangles: Vec<[u32; 3]>,
}

struct Face {
    corners: [usize; 3],
    normal: DVec3,
    offset: f64,
    /// Points outside this face, each with its distance above it.
    outside: Vec<(usize, f64)>,
    alive: bool,
}

impl Face {
    fn new(points: &[DVec3], corners: [usize; 3]) -> Self {
        let [a, b, c] = corners.map(|corner| points[corner]);
        let normal = (b - a).cross(c - a).normalize_or_zero();
        Self {
            corners,
            normal,
            offset: normal.dot(a),
            outside: Vec::new(),
            alive: true,
        }
    }

    fn distance(&self, point: DVec3) -> f64 {
        self.normal.dot(point) - self.offset
    }

    fn furthest(&self) -> Option<(usize, f64)> {
        self.outside
            .iter()
            .copied()
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
    }
}

/// The convex hull of `points`, with at most `max_vertices` vertices (at least
/// four). `None` when the points do not span a volume: all on one plane, one
/// line, or one point.
pub(crate) fn convex_hull(points: &[Vec3], max_vertices: usize) -> Option<Hull> {
    let max_vertices = max_vertices.max(HULL_VERTICES_MIN as usize);
    // Distinct points, in first-seen order, so the result is a function of the
    // input alone.
    let mut seen = HashMap::new();
    let points: Vec<DVec3> = points
        .iter()
        .filter(|point| point.is_finite())
        .filter(|point| {
            seen.insert(point.to_array().map(f32::to_bits), ())
                .is_none()
        })
        .map(|point| point.as_dvec3())
        .collect();
    if points.len() < 4 {
        return None;
    }
    let (min, max) = points.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(min, max), &point| (min.min(point), max.max(point)),
    );
    let epsilon = (max - min).length() * 1.0e-9;
    if epsilon <= 0.0 {
        return None;
    }

    let simplex = initial_simplex(&points, epsilon)?;
    let mut faces: Vec<Face> = Vec::new();
    let centroid = simplex.iter().map(|&k| points[k]).sum::<DVec3>() / 4.0;
    for [a, b, c] in [[0, 1, 2], [0, 3, 1], [0, 2, 3], [1, 3, 2]] {
        let mut corners = [simplex[a], simplex[b], simplex[c]];
        let face = Face::new(&points, corners);
        // Wound so the normal points away from the inside.
        if face.distance(centroid) > 0.0 {
            corners.swap(1, 2);
        }
        faces.push(Face::new(&points, corners));
    }
    let mut used = simplex.to_vec();
    let candidates: Vec<usize> = (0..points.len())
        .filter(|point| !simplex.contains(point))
        .collect();
    assign(&points, &mut faces, 0..4, &candidates, epsilon);

    while used.len() < max_vertices {
        // The point furthest outside the hull so far.
        let Some((eye, _)) = faces
            .iter()
            .filter(|face| face.alive)
            .filter_map(Face::furthest)
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        else {
            break;
        };
        let point = points[eye];
        let visible: Vec<usize> = (0..faces.len())
            .filter(|&face| faces[face].alive && faces[face].distance(point) > epsilon)
            .collect();
        // Every directed edge of the faces still standing, to find which edges
        // of the visible region border a face that stays.
        let mut edge_face: HashMap<(usize, usize), usize> = HashMap::new();
        for (index, face) in faces.iter().enumerate().filter(|(_, face)| face.alive) {
            let [a, b, c] = face.corners;
            for edge in [(a, b), (b, c), (c, a)] {
                edge_face.insert(edge, index);
            }
        }
        let mut horizon = Vec::new();
        for &face in &visible {
            let [a, b, c] = faces[face].corners;
            for (from, to) in [(a, b), (b, c), (c, a)] {
                let across = edge_face.get(&(to, from)).copied();
                if across.is_some_and(|across| !visible.contains(&across)) {
                    horizon.push((from, to));
                }
            }
        }
        if horizon.is_empty() {
            // Only rounding can get here; the point is as good as inside.
            for &face in &visible {
                faces[face].outside.retain(|&(point, _)| point != eye);
            }
            continue;
        }
        let mut orphans = Vec::new();
        for &face in &visible {
            faces[face].alive = false;
            orphans.extend(
                std::mem::take(&mut faces[face].outside)
                    .into_iter()
                    .map(|(point, _)| point)
                    .filter(|&point| point != eye),
            );
        }
        let first_new = faces.len();
        for (from, to) in horizon {
            faces.push(Face::new(&points, [from, to, eye]));
        }
        used.push(eye);
        let new_faces = first_new..faces.len();
        assign(&points, &mut faces, new_faces, &orphans, epsilon);
    }

    // Number the hull's vertices in the order they joined it.
    let mut index_of = HashMap::new();
    for (index, &point) in used.iter().enumerate() {
        index_of.insert(point, index as u32);
    }
    Some(Hull {
        positions: used.iter().map(|&point| points[point].as_vec3()).collect(),
        triangles: faces
            .iter()
            .filter(|face| face.alive)
            .map(|face| face.corners.map(|corner| index_of[&corner]))
            .collect(),
    })
}

/// Four points spanning a volume: the two furthest apart of the six extremes,
/// the point furthest from the line through them, and the point furthest from
/// the plane through those three.
fn initial_simplex(points: &[DVec3], epsilon: f64) -> Option<[usize; 4]> {
    let mut extremes = [0usize; 6];
    for (index, point) in points.iter().enumerate() {
        for axis in 0..3 {
            if point[axis] < points[extremes[axis * 2]][axis] {
                extremes[axis * 2] = index;
            }
            if point[axis] > points[extremes[axis * 2 + 1]][axis] {
                extremes[axis * 2 + 1] = index;
            }
        }
    }
    let mut pair = (extremes[0], extremes[1]);
    let mut longest = -1.0;
    for &a in &extremes {
        for &b in &extremes {
            let length = points[a].distance_squared(points[b]);
            if length > longest {
                longest = length;
                pair = (a, b);
            }
        }
    }
    let (a, b) = pair;
    let direction = (points[b] - points[a]).normalize_or_zero();
    let off_line = |index: usize| {
        let offset = points[index] - points[a];
        (offset - direction * offset.dot(direction)).length()
    };
    let c = (0..points.len()).max_by(|&x, &y| off_line(x).total_cmp(&off_line(y)))?;
    if off_line(c) <= epsilon {
        return None;
    }
    let normal = (points[b] - points[a])
        .cross(points[c] - points[a])
        .normalize_or_zero();
    let off_plane = |index: usize| normal.dot(points[index] - points[a]).abs();
    let d = (0..points.len()).max_by(|&x, &y| off_plane(x).total_cmp(&off_plane(y)))?;
    if off_plane(d) <= epsilon {
        return None;
    }
    Some([a, b, c, d])
}

/// Give each of `candidates` to the face among `faces[range]` it is furthest
/// outside of; a point outside none of them is inside the hull for good.
fn assign(
    points: &[DVec3],
    faces: &mut [Face],
    range: std::ops::Range<usize>,
    candidates: &[usize],
    epsilon: f64,
) {
    for &point in candidates {
        let best = range
            .clone()
            .map(|face| (face, faces[face].distance(points[point])))
            .filter(|&(_, distance)| distance > epsilon)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((face, distance)) = best {
            faces[face].outside.push((point, distance));
        }
    }
}

/// Replace every object `op` selects with its convex hull, its attributes
/// projected from the object's own surface as Shrinkwrap's are.
pub(crate) fn hull_submeshes(
    submeshes: &mut Vec<Submesh>,
    op: &OpInstance,
    stack: &OptStack,
    model: &ModelData,
    warnings: &mut Warnings,
    run: &crate::process::RunContext<'_>,
) {
    let _z = crate::prof::zone!("Convex Hull");
    let OpKind::ConvexHull(global) = &op.kind else {
        return;
    };
    let mut replaced: Vec<(u32, Vec<Submesh>)> = Vec::new();
    for (node, pieces) in nodes::pieces_by_node(submeshes) {
        if run.cancelled() {
            return;
        }
        if crate::process::is_excluded(stack, node) {
            continue;
        }
        let params = match crate::process::resolve_op(stack, op, node) {
            OpKind::ConvexHull(params) => params,
            _ => global,
        };
        let name = remesh::node_name(model, node);
        let Ok(selection) = GlobSet::new(&params.objects) else {
            warnings.push(OptWarning::HullBadPattern);
            return;
        };
        if !selection.matches(&name) {
            continue;
        }
        if pieces.iter().any(|piece| piece.has_rows()) {
            warnings.push(OptWarning::HullKeptDeforming { object: name });
            continue;
        }
        if let Some(built) = hull_node(&pieces, params, &name, warnings, run) {
            replaced.push((node, built));
        }
    }
    for (node, built) in replaced {
        // In place of the node's own pieces, where the first of them stood, so
        // the level's piece order stays the stack's.
        let at = submeshes
            .iter()
            .position(|piece| piece.node == node)
            .unwrap_or(submeshes.len());
        submeshes.retain(|piece| piece.node != node);
        let at = at.min(submeshes.len());
        submeshes.splice(at..at, built);
    }
}

/// One object's hull, or `None` (with a warning) when it has none.
fn hull_node(
    pieces: &[&Submesh],
    params: &HullParams,
    name: &str,
    warnings: &mut Warnings,
    run: &crate::process::RunContext<'_>,
) -> Option<Vec<Submesh>> {
    let positions: Vec<Vec3> = pieces
        .iter()
        .flat_map(|piece| {
            piece
                .indices
                .iter()
                .map(|&index| piece.vertices[index as usize].position)
        })
        .collect();
    let Some(hull) = convex_hull(&positions, params.max_vertices as usize) else {
        warnings.push(OptWarning::HullFlat {
            object: name.to_owned(),
        });
        return None;
    };
    let output = remesh::RemeshOutput {
        positions: hull.positions,
        face_offsets: (0..=hull.triangles.len() as u32)
            .map(|face| face * 3)
            .collect(),
        corners: hull.triangles.iter().flatten().copied().collect(),
    };
    let token = run.token();
    let source = remesh::ProjectionSource::build(pieces, token)?;
    // Flat: a hull's faces meet at edges, never across a curve.
    let normals = NormalParams {
        crease_angle: 1.0,
        ..NormalParams::default()
    };
    match output
        .validate()
        .and_then(|()| remesh::build_pieces_from(&output, &source, pieces, Some(normals), 1, token))
    {
        Ok(built) if !built.is_empty() => Some(built),
        Ok(_) => None,
        Err(crate::OptError::Cancelled) => None,
        Err(error) => {
            warnings.push(OptWarning::HullFailed {
                object: name.to_owned(),
                detail: error.to_string(),
            });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(hull: &Hull) -> f64 {
        hull.triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.map(|k| hull.positions[k as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// Every directed edge appears once and its reverse once: a closed,
    /// consistently wound surface.
    fn closed(hull: &Hull) -> bool {
        let mut edges = HashMap::new();
        for &[a, b, c] in &hull.triangles {
            for edge in [(a, b), (b, c), (c, a)] {
                *edges.entry(edge).or_insert(0) += 1;
            }
        }
        edges
            .iter()
            .all(|(&(a, b), &count)| count == 1 && edges.get(&(b, a)) == Some(&1))
    }

    fn sphere(rings: usize, segments: usize) -> Vec<Vec3> {
        let mut points = Vec::new();
        for ring in 0..=rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for segment in 0..segments {
                let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
                points.push(Vec3::new(
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ));
            }
        }
        points
    }

    #[test]
    fn a_cube_with_points_inside_is_its_eight_corners() {
        let mut points: Vec<Vec3> = (0..8)
            .map(|k| Vec3::new((k & 1) as f32, ((k >> 1) & 1) as f32, ((k >> 2) & 1) as f32))
            .collect();
        points.extend([
            Vec3::splat(0.5),
            Vec3::new(0.25, 0.5, 0.75),
            Vec3::new(0.5, 0.5, 1.0),
        ]);
        let hull = convex_hull(&points, 32).unwrap();
        assert_eq!(hull.positions.len(), 8);
        assert!(closed(&hull));
        assert!((volume(&hull) - 1.0).abs() < 1e-6, "{}", volume(&hull));
    }

    #[test]
    fn a_limit_keeps_the_points_that_matter_most() {
        let points = sphere(16, 32);
        let full = convex_hull(&points, 255).unwrap();
        let limited = convex_hull(&points, 32).unwrap();
        assert_eq!(limited.positions.len(), 32);
        assert!(closed(&limited));
        // An inner hull, and not far inside: 32 points of a unit sphere hold
        // most of its volume.
        let (full, limited) = (volume(&full), volume(&limited));
        assert!(
            limited < full && limited > 0.75 * full,
            "{limited} of {full}"
        );
        // Every vertex is one of the input points.
        assert!(
            convex_hull(&points, 32)
                .unwrap()
                .positions
                .iter()
                .all(|vertex| points.contains(vertex))
        );
    }

    #[test]
    fn points_on_a_plane_have_no_hull() {
        let flat: Vec<Vec3> = (0..20)
            .map(|k| Vec3::new(k as f32, (k * k % 7) as f32, 0.0))
            .collect();
        assert!(convex_hull(&flat, 32).is_none());
        assert!(convex_hull(&[Vec3::ZERO; 10], 32).is_none());
    }

    #[test]
    fn the_same_points_in_another_order_give_the_same_hull_volume() {
        let points = sphere(8, 12);
        let mut reversed = points.clone();
        reversed.reverse();
        let (a, b) = (
            convex_hull(&points, 255).unwrap(),
            convex_hull(&reversed, 255).unwrap(),
        );
        assert_eq!(a.positions.len(), b.positions.len());
        assert!((volume(&a) - volume(&b)).abs() < 1e-6);
    }
}
