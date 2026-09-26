//! Tidying the rebuilt mesh: better-shaped triangles, more even face sizes.
//!
//! What the collapse produces is correct but lumpy. Each region became one
//! vertex, and where a region was an awkward shape its vertex lands wherever the
//! last valid collapse left it — so face sizes vary more than the size field
//! asked for, and some triangles come out thin. A field extraction does not have
//! this problem because it *places* vertices on a lattice; a collapse inherits
//! whatever the input had.
//!
//! Two passes fix most of it, and they are the two standard ones:
//!
//! * **Tangential relaxation.** Move each vertex toward the middle of its
//!   neighbours, but only *along the surface* — the component of that move along
//!   the vertex normal is dropped. Equalizing the distance to the neighbours is
//!   what evens the face sizes out; staying in the tangent plane is what stops
//!   it from shrinking the shape, which is the failure a plain Laplacian smooth
//!   has (a sphere smoothed that way steadily becomes a smaller sphere).
//! * **Valence flips.** A triangle mesh is best conditioned when interior
//!   vertices have six neighbours and border vertices four. Flipping the shared
//!   edge of two triangles moves one neighbour from each of two vertices to the
//!   other two, so a pass of flips that reduce the total deviation from those
//!   targets straightens out the worst of it.
//!
//! ## Snapping back to the source, and why this does not
//!
//! The obvious third pass is to put every vertex back on the source surface —
//! it is what a remesher that *synthesizes* a surface ends with, since the
//! surface it built is genuinely somewhere else. Measured here, twice, it makes
//! every fixture worse: over the whole output (a plant 96.8 % of its area to
//! 96.1 %, a column 86.1 % to 84.0 %, stones 99.7 % to 98.0 %) and again when
//! narrowed to only the vertices the relaxation had just moved (the plant
//! 89.0 % to 87.3 %, the column 85.9 % to 83.3 %).
//!
//! The reason is that this rebuild synthesizes nothing. Its vertices are source
//! vertices, or the optimum of a quadric built from the source's own planes —
//! and on a convex patch that optimum sits a little *outside* the surface,
//! where the tangent planes meet, which is where a coarse mesh has to be if it
//! is to keep the area of the fine one. The nearest point on the source is by
//! definition not outside it, so snapping trades a circumscribing approximation
//! for an inscribing one, every time.
//!
//! ## What is never touched
//!
//! Border vertices do not move and border edges are never flipped. The outline
//! is the thing the whole rebuild is most careful about, and a relaxation that
//! slid vertices along it would undo the even spacing the seeding gave it. A
//! border vertex is recognised from the *output's* own topology — an edge with
//! one face.
//!
//! `pinned` carries what the output's topology *cannot* say. A rim — where a
//! thin shell folds back on itself — is an ordinary interior edge here, two
//! faces like any other, and yet it is exactly where the silhouette is. The
//! collapse knows (it was a feature vertex in the source) and says so.

use crate::cancel::{CancelToken, cancelled};

use super::RemeshOutput;
use super::geom;
use super::topology::Topology;

/// How far toward the neighbourhood centre a vertex moves in one pass. Half is
/// the usual choice: the whole way overshoots and oscillates.
const RELAXATION: f64 = 0.5;

/// How much the faces around a vertex must agree on a normal before the
/// relaxation will trust one and move it.
///
/// About a hundred degrees of spread across the fan, which reads as "the surface
/// is locally flat enough here to slide along". Chosen by measurement: the
/// plant keeps 89.0 % of its area with no guard at all, 90.6 % at a half,
/// **96.6 % here** and 97.6 % at 0.95 — but by 0.95 it is refusing so many
/// vertices that the evenness the pass exists for starts going with it (the
/// edge spread 5.60x here against 5.94x there, where not relaxing at all is
/// 7.22x). See [`vertex_normal`].
const NORMAL_AGREEMENT: f64 = 0.85;

/// Neighbours an interior vertex is best off with. Six is what a regular
/// triangulation of a plane gives.
const INTERIOR_VALENCE: i32 = 6;

/// And on a border, where a vertex only has surface on one side.
const BORDER_VALENCE: i32 = 4;

/// Tidy `output` in place, `rounds` times over.
///
/// One round is a flip pass and then a relaxation pass — in that order, because
/// flipping changes who a vertex's neighbours are and the relaxation should see
/// the result.
pub(crate) fn run(
    output: &mut RemeshOutput,
    pinned: &[bool],
    rounds: u32,
    cancel: Option<&CancelToken>,
) {
    if rounds == 0 || output.positions.is_empty() {
        return;
    }
    let _z = crate::prof::zone!("Remesh Cleanup");

    for _ in 0..rounds {
        if cancelled(cancel) {
            return;
        }
        // Rebuilt each round: both passes change the connectivity, and this
        // mesh is the *output*, so indexing it is cheap next to the input.
        let topology = Topology::build(&output.corners, output.positions.len(), 1, cancel);
        flip_pass(output, &topology);

        let topology = Topology::build(&output.corners, output.positions.len(), 1, cancel);
        relax_pass(output, &topology, pinned);
    }
}

/// Whether `vertex` is one this module may not move: an outline of the output's
/// own, or one the collapse marked as carrying the source's shape.
fn is_held(topology: &Topology, pinned: &[bool], vertex: u32) -> bool {
    topology.boundary[vertex as usize]
        || topology.nonmanifold[vertex as usize]
        || pinned.get(vertex as usize).copied().unwrap_or(false)
}

/// Flip the shared edge of two triangles wherever it brings the four vertices
/// closer to their target valence.
fn flip_pass(output: &mut RemeshOutput, topology: &Topology) {
    let count = output.positions.len();
    let mut valence: Vec<i32> = vec![0; count];
    let mut scratch = Vec::new();
    for vertex in 0..count as u32 {
        topology.edges_at(&output.corners, vertex, &mut scratch);
        valence[vertex as usize] = scratch.len() as i32;
    }

    // In vertex order, and each edge considered from its lower end, so the pass
    // is a function of the mesh rather than of a traversal.
    for vertex in 0..count as u32 {
        topology.edges_at(&output.corners, vertex, &mut scratch);
        for edge in &scratch {
            if edge.other < vertex || edge.uses != 2 {
                continue;
            }
            let [first, second] = edge.faces;
            let Some(quad) = Quad::read(output, first, second, vertex, edge.other) else {
                continue;
            };
            let (left, right) = (quad.forward_apex, quad.backward_apex);
            // The flip replaces the edge (vertex, other) with (left, right), so
            // the two ends lose a neighbour and the two opposites gain one.
            let before = deviation(&valence, topology, vertex)
                + deviation(&valence, topology, edge.other)
                + deviation(&valence, topology, left)
                + deviation(&valence, topology, right);
            let after = deviation_if(&valence, topology, vertex, -1)
                + deviation_if(&valence, topology, edge.other, -1)
                + deviation_if(&valence, topology, left, 1)
                + deviation_if(&valence, topology, right, 1);
            if after >= before {
                continue;
            }
            if !flip_is_safe(output, topology, &quad) {
                continue;
            }
            let [forward, backward] = quad.flipped();
            write_face(output, quad.forward, forward);
            write_face(output, quad.backward, backward);
            valence[vertex as usize] -= 1;
            valence[edge.other as usize] -= 1;
            valence[left as usize] += 1;
            valence[right as usize] += 1;
            // The cached edge list for this vertex is stale now; the next
            // vertex re-reads it, and this one is done either way.
            break;
        }
    }
}

/// The two triangles either side of an edge, read in the order they are
/// actually wound.
///
/// **Which way each face runs has to be read, never assumed.** The two faces
/// sharing an edge of an oriented surface traverse it in opposite directions:
/// one runs `a -> b`, the other `b -> a`. Which is which is a property of the
/// mesh, and the rewritten pair keeps the surface's winding only if it is taken
/// from there. Deciding it by convention instead turns both triangles inside
/// out on every flip whenever the convention is the wrong way round — and an
/// inverted triangle reads as a *hole* on screen, because a surface lit from
/// behind is black.
/// Two triangles sharing an edge, read as the quad they make.
///
/// Shared with the [quad merge](super::quads), which has the same question to
/// ask and must not answer it a second way: the corner order the pair is
/// actually in is a property of the mesh, and assuming it rather than reading it
/// is what turned triangles inside out here before.
pub(super) struct Quad {
    /// The face traversing `a -> b`, and the corner of it opposite that edge.
    forward: u32,
    forward_apex: u32,
    /// The face traversing `b -> a`, and its opposite corner.
    backward: u32,
    backward_apex: u32,
    a: u32,
    b: u32,
}

impl Quad {
    /// Read the pair, or `None` if the two do not form an orientable quad.
    pub(super) fn read(
        output: &RemeshOutput,
        first: u32,
        second: u32,
        a: u32,
        b: u32,
    ) -> Option<Self> {
        let (first_apex, first_forward) = apex_and_direction(output, first, a, b)?;
        let (second_apex, second_forward) = apex_and_direction(output, second, a, b)?;
        // Both running the same way means the two faces disagree about which
        // side of the surface they are on. A flip cannot repair that, and
        // rewriting the pair would only spread it.
        if first_forward == second_forward || first_apex == second_apex {
            return None;
        }
        let (forward, forward_apex, backward, backward_apex) = if first_forward {
            (first, first_apex, second, second_apex)
        } else {
            (second, second_apex, first, first_apex)
        };
        Some(Self {
            forward,
            forward_apex,
            backward,
            backward_apex,
            a,
            b,
        })
    }

    /// The pair as it stands.
    fn faces(&self) -> [[u32; 3]; 2] {
        [
            [self.a, self.b, self.forward_apex],
            [self.b, self.a, self.backward_apex],
        ]
    }

    /// The pair as one quad, wound to match.
    ///
    /// The shared edge is the `0..2` diagonal, so the two triangles this stands
    /// for are `0,1,2` and `0,2,3` — the pair that exists now, in the order
    /// it exists in. Anything that splits the quad back on that diagonal gets
    /// the surface it started with.
    pub(super) fn merged(&self) -> [u32; 4] {
        [self.a, self.backward_apex, self.b, self.forward_apex]
    }

    /// The pair split across the other diagonal, wound to match.
    ///
    /// The quad runs `a -> backward_apex -> b -> forward_apex` all the way
    /// round, so splitting it the other way gives these two, each traversed the
    /// same way as the boundary it sits on.
    fn flipped(&self) -> [[u32; 3]; 2] {
        [
            [self.a, self.backward_apex, self.forward_apex],
            [self.backward_apex, self.b, self.forward_apex],
        ]
    }
}

/// A face's corner opposite the edge `a..b`, and whether it traverses `a -> b`.
fn apex_and_direction(output: &RemeshOutput, face: u32, a: u32, b: u32) -> Option<(u32, bool)> {
    let corners = face_of(output, face);
    let at = corners.iter().position(|&corner| corner == a)?;
    if corners[(at + 1) % 3] == b {
        Some((corners[(at + 2) % 3], true))
    } else if corners[(at + 2) % 3] == b {
        Some((corners[(at + 1) % 3], false))
    } else {
        None
    }
}

/// Whether flipping is legal: it must not duplicate an existing edge, and it
/// must not turn either new face inside out.
fn flip_is_safe(output: &RemeshOutput, topology: &Topology, quad: &Quad) -> bool {
    // An edge between the two opposite corners already existing would make the
    // flip produce a second copy of it, which no surface can carry.
    let mut edges = Vec::new();
    topology.edges_at(&output.corners, quad.forward_apex, &mut edges);
    if edges.iter().any(|edge| edge.other == quad.backward_apex) {
        return false;
    }

    // A fold is the silhouette of a thin shell, and flipping across one swaps a
    // pair of triangles that lie on opposite sides of the surface for a pair
    // that span it — a fin through the middle of a leaf.
    let faces = quad.faces().map(|corners| normal_of(output, corners));
    if let [Some(one), Some(other)] = faces
        && one[0] * other[0] + one[1] * other[1] + one[2] * other[2] <= 0.0
    {
        return false;
    }

    // Both sides are read from the mesh now, which is what makes this guard
    // mean anything: a fold shows up as the new normal opposing the old one.
    let before = quad.faces().map(|corners| normal_of(output, corners));
    let after = quad.flipped().map(|corners| normal_of(output, corners));
    for (before, after) in before.iter().zip(&after) {
        match (before, after) {
            (Some(before), Some(after)) => {
                if before[0] * after[0] + before[1] * after[1] + before[2] * after[2] <= 0.0 {
                    return false;
                }
            }
            // A new face with no area is a sliver, which is what this pass is
            // meant to remove rather than create.
            (_, None) => return false,
            (None, _) => {}
        }
    }
    true
}

/// Move every interior vertex toward the middle of its neighbours, along the
/// surface.
fn relax_pass(output: &mut RemeshOutput, topology: &Topology, pinned: &[bool]) {
    let count = output.positions.len();
    let mut moved: Vec<[f64; 3]> = output
        .positions
        .iter()
        .map(|point| [point.x as f64, point.y as f64, point.z as f64])
        .collect();
    let original = moved.clone();
    let mut scratch = Vec::new();

    for vertex in 0..count as u32 {
        // A border vertex holds the outline; the seeding already spaced it. A
        // pinned one holds a rim or a crease, which the output's topology has no
        // way of recognising.
        if is_held(topology, pinned, vertex) {
            continue;
        }
        topology.edges_at(&output.corners, vertex, &mut scratch);
        if scratch.is_empty() {
            continue;
        }
        let here = original[vertex as usize];
        let mut centre = [0.0f64; 3];
        for edge in &scratch {
            let there = original[edge.other as usize];
            for (axis, value) in centre.iter_mut().enumerate() {
                *value += there[axis];
            }
        }
        let weight = scratch.len() as f64;
        let mut step = [0.0f64; 3];
        for axis in 0..3 {
            step[axis] = (centre[axis] / weight - here[axis]) * RELAXATION;
        }

        // Only along the surface: the part of the move that runs along the
        // vertex normal is what would shrink the shape.
        //
        // Which makes the whole pass only as good as that normal. Where the
        // faces around a vertex do not agree on one — the narrow end of a leaf,
        // where the sheet above and the sheet below are both in the same fan and
        // point opposite ways — the area-weighted sum very nearly cancels and
        // what is left is rounding error. Subtracting *that* from the step
        // leaves most of the true normal component in, and the vertex is walked
        // straight through the shell. Refusing to move is the honest answer, and
        // it is worth ten per cent of a thin object's surface area.
        let Some((normal, agreement)) = vertex_normal(output, topology, vertex) else {
            continue;
        };
        if agreement < NORMAL_AGREEMENT {
            continue;
        }
        let along = step[0] * normal[0] + step[1] * normal[1] + step[2] * normal[2];
        let mut candidate = here;
        for axis in 0..3 {
            candidate[axis] = here[axis] + step[axis] - along * normal[axis];
        }

        // The tangent plane is only an estimate of the surface, and on a coarse
        // patch of a tightly curved one it is a poor enough estimate to carry a
        // vertex past its own neighbours, folding the faces around it over so
        // they light from behind. Taking the move only when the neighbourhood
        // does not fold *more* than it already did is what keeps this pass from
        // making the mesh worse — while still letting it undo a fold the
        // collapse left, which is most of what it achieves here.
        if fold_count(output, topology, vertex, &original, Some(candidate))
            > fold_count(output, topology, vertex, &original, None)
        {
            continue;
        }
        moved[vertex as usize] = candidate;
    }

    for (slot, point) in output.positions.iter_mut().zip(&moved) {
        *slot = glam::Vec3::new(point[0] as f32, point[1] as f32, point[2] as f32);
    }
}

/// How many pairs of faces around `vertex` fold back past a right angle, with
/// the vertex either where it is (`candidate` of `None`) or moved.
///
/// A fold is a disagreement between two faces sharing an edge, which is why it
/// cannot be judged by comparing one face against its own earlier self: every
/// face agrees with where it just was.
fn fold_count(
    output: &RemeshOutput,
    topology: &Topology,
    vertex: u32,
    positions: &[[f64; 3]],
    candidate: Option<[f64; 3]>,
) -> usize {
    let at = |corner: u32| -> [f64; 3] {
        match candidate {
            Some(point) if corner == vertex => point,
            _ => positions[corner as usize],
        }
    };
    let faces = topology.faces_of(vertex);
    let mut folded = 0;
    for (index, &face) in faces.iter().enumerate() {
        let corners = face_of(output, face);
        let normal = geom::normalized(cross_at(&at, corners));
        for &other in &faces[index + 1..] {
            let others = face_of(output, other);
            // Only faces sharing an edge say anything about a fold; two faces
            // meeting at this vertex alone may legitimately face anywhere.
            if others.iter().filter(|c| corners.contains(c)).count() != 2 {
                continue;
            }
            let against = geom::normalized(cross_at(&at, others));
            match (normal, against) {
                (Some(normal), Some(against)) => {
                    if normal[0] * against[0] + normal[1] * against[1] + normal[2] * against[2]
                        < 0.0
                    {
                        folded += 1;
                    }
                }
                // A face with no area has no opinion, and making one is itself
                // a fault: count it so the move is refused.
                _ => folded += 1,
            }
        }
    }
    folded
}

/// The unnormalized normal of `corners`, reading each position through `point`.
fn cross_at(point: &dyn Fn(u32) -> [f64; 3], corners: [u32; 3]) -> [f64; 3] {
    geom::triangle_cross(point(corners[0]), point(corners[1]), point(corners[2]))
}

/// The area-weighted normal of the faces around a vertex, and how much those
/// faces agree on it.
///
/// Agreement is the length of the area-weighted sum over the total area: one
/// where the fan is flat, zero where it folds back on itself so exactly that the
/// two halves cancel. It is what says whether the direction is worth anything.
fn vertex_normal(
    output: &RemeshOutput,
    topology: &Topology,
    vertex: u32,
) -> Option<([f64; 3], f64)> {
    let mut sum = [0.0f64; 3];
    let mut total = 0.0f64;
    for &face in topology.faces_of(vertex) {
        let corners = face_of(output, face);
        let Some(normal) = normal_of(output, corners) else {
            continue;
        };
        let area = area_of(output, corners);
        total += area;
        for (axis, value) in sum.iter_mut().enumerate() {
            *value += normal[axis] * area;
        }
    }
    let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    if length <= 0.0 || total <= 0.0 {
        return None;
    }
    Some((
        [sum[0] / length, sum[1] / length, sum[2] / length],
        length / total,
    ))
}

fn deviation(valence: &[i32], topology: &Topology, vertex: u32) -> i32 {
    deviation_if(valence, topology, vertex, 0)
}

fn deviation_if(valence: &[i32], topology: &Topology, vertex: u32, change: i32) -> i32 {
    let target = if topology.boundary[vertex as usize] {
        BORDER_VALENCE
    } else {
        INTERIOR_VALENCE
    };
    (valence[vertex as usize] + change - target).abs()
}

fn face_of(output: &RemeshOutput, face: u32) -> [u32; 3] {
    let base = face as usize * 3;
    [
        output.corners[base],
        output.corners[base + 1],
        output.corners[base + 2],
    ]
}

fn write_face(output: &mut RemeshOutput, face: u32, corners: [u32; 3]) {
    let base = face as usize * 3;
    output.corners[base..base + 3].copy_from_slice(&corners);
}

fn point_of(output: &RemeshOutput, vertex: u32) -> [f64; 3] {
    let point = output.positions[vertex as usize];
    [point.x as f64, point.y as f64, point.z as f64]
}

fn cross_of(output: &RemeshOutput, corners: [u32; 3]) -> [f64; 3] {
    geom::triangle_cross(
        point_of(output, corners[0]),
        point_of(output, corners[1]),
        point_of(output, corners[2]),
    )
}

fn normal_of(output: &RemeshOutput, corners: [u32; 3]) -> Option<[f64; 3]> {
    geom::normalized(cross_of(output, corners))
}

fn area_of(output: &RemeshOutput, corners: [u32; 3]) -> f64 {
    geom::length(cross_of(output, corners)) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    /// Two triangles over a square, sharing the diagonal `0..2`, both wound the
    /// same way round the square.
    ///
    /// ```text
    ///   3 ----- 2
    ///   | \     |
    ///   |   \   |
    ///   |     \ |
    ///   0 ----- 1
    /// ```
    fn square(first: [u32; 3], second: [u32; 3]) -> RemeshOutput {
        RemeshOutput {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            face_offsets: vec![0, 3, 6],
            corners: [first, second].concat(),
        }
    }

    /// Every face of a rebuilt mesh has to end up on the same side of the
    /// surface, and a flip is the one pass that rewrites faces wholesale.
    ///
    /// Both windings of the same pair are checked, because the fault this pins
    /// was assuming one of them: the pass read neither face's corner order, so
    /// on the mesh wound the other way it turned both triangles over. That
    /// shows on screen as a black patch, since a surface lit from behind is
    /// dark, and it is what a rebuilt plant came back covered in.
    #[test]
    fn a_flip_keeps_the_winding_it_found() {
        // The same two triangles, wound each way round.
        for (first, second) in [([0, 1, 2], [0, 2, 3]), ([0, 2, 1], [0, 3, 2])] {
            let output = square(first, second);
            let before = normal_of(&output, first).expect("the face has area");

            let quad = Quad::read(&output, 0, 1, 0, 2).expect("the two share edge 0..2");
            let mut flipped = output;
            let [forward, backward] = quad.flipped();
            write_face(&mut flipped, quad.forward, forward);
            write_face(&mut flipped, quad.backward, backward);

            // The new diagonal is the other one, 1..3.
            let corners: Vec<[u32; 3]> = (0..2).map(|face| face_of(&flipped, face)).collect();
            for corners in &corners {
                assert!(
                    corners.contains(&1) && corners.contains(&3),
                    "the flip did not move to the other diagonal: {corners:?}"
                );
                let after = normal_of(&flipped, *corners).expect("the face has area");
                let dot = before[0] * after[0] + before[1] * after[1] + before[2] * after[2];
                assert!(
                    dot > 0.0,
                    "a flip of {first:?}/{second:?} turned {corners:?} over"
                );
            }
        }
    }

    /// Two faces that disagree about which way round they go are not a quad to
    /// be flipped, and rewriting them would only spread the disagreement.
    #[test]
    fn a_pair_that_disagrees_is_left_alone() {
        let output = square([0, 1, 2], [0, 3, 2]);
        assert!(Quad::read(&output, 0, 1, 0, 2).is_none());
    }

    /// The relaxation is allowed to undo a fold, which is most of what it
    /// achieves, but never to add one.
    #[test]
    fn relaxing_never_folds_a_neighbourhood_further() {
        // A hexagonal fan: a centre vertex ringed by six, flat.
        let mut positions = vec![Vec3::ZERO];
        let mut corners = Vec::new();
        for step in 0..6u32 {
            let angle = step as f32 / 6.0 * std::f32::consts::TAU;
            positions.push(Vec3::new(angle.cos(), angle.sin(), 0.0));
        }
        for step in 0..6u32 {
            corners.extend_from_slice(&[0, 1 + step, 1 + (step + 1) % 6]);
        }
        // Pull the centre far off the plane, so relaxing has real work to do.
        positions[0] = Vec3::new(0.35, 0.2, 2.0);
        let mut output = RemeshOutput {
            positions,
            face_offsets: (0..=6).map(|face| face * 3).collect(),
            corners,
        };

        let folds_before = {
            let topology = Topology::build(&output.corners, output.positions.len(), 1, None);
            let points: Vec<[f64; 3]> = output
                .positions
                .iter()
                .map(|p| [p.x as f64, p.y as f64, p.z as f64])
                .collect();
            fold_count(&output, &topology, 0, &points, None)
        };

        run(&mut output, &[], 4, None);

        let topology = Topology::build(&output.corners, output.positions.len(), 1, None);
        let points: Vec<[f64; 3]> = output
            .positions
            .iter()
            .map(|p| [p.x as f64, p.y as f64, p.z as f64])
            .collect();
        let folds_after = fold_count(&output, &topology, 0, &points, None);
        assert!(
            folds_after <= folds_before,
            "relaxing added folds: {folds_before} -> {folds_after}"
        );
    }
}
