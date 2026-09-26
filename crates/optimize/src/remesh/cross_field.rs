//! A direction field over the surface, for the rebuild to line its edges up
//! with.
//!
//! Everything before this stage knows *how big* a face should be here and
//! nothing about which way it should point. That is why a rebuilt surface comes
//! out even in density and unstructured in layout, and it costs more than
//! tidiness: a vertex lattice with no common direction is locally bumpy, and a
//! bump is a crease the source did not have. Measured on a rock pillar at a
//! ratio of 0.6, a rebuild that keeps sharp edges reproduces **112 %** of the
//! source's crease length - not eighty-eight per cent of it with some missed,
//! but all of it plus an eighth again of creases that are not in the model at
//! all. Those are what read as broken shading.
//!
//! ## What the field is
//!
//! One direction per vertex, lying in that vertex's tangent plane - but standing
//! for *four* directions, at right angles to each other. A surface has no way to
//! tell a quad's "along" from its "across", so a field that distinguished them
//! could not be made continuous: walk once around a cone and the direction comes
//! back rotated. Treating `d`, `n x d`, `-d` and `-n x d` as the same value is
//! what lets one exist, and it is the standard construction - a 4-RoSy field, or
//! a *cross* field.
//!
//! It is also exactly what a quad topology needs, which is what it is for: a
//! quad's two edge directions *are* the cross, and laying triangles along one is
//! the same problem solved less completely.
//!
//! ## How it is built
//!
//! Smoothed, from constraints, the same shape as [`super::size_field`]:
//!
//! 1. Every vertex gets a tangent frame from its normal.
//! 2. Vertices on a feature are **pinned** to the direction that feature runs
//!    in. A crease or an open border is a direction the surface itself has
//!    already committed to, and the rest of the field is grown from those.
//! 3. Everything else is smoothed toward the average of its neighbours, in the
//!    4-RoSy sense: each neighbour's direction is turned into the vertex's own
//!    tangent plane and then rotated by whichever multiple of ninety degrees
//!    brings it closest before it is added in.
//!
//! Jacobi passes over a ping-pong buffer, so the answer does not depend on the
//! order vertices are visited in, cancellable between passes, and parallel over
//! a fixed chunk exactly as every other sweep here is.
//!
//! ## Why it does not help a triangle mesh, and is spent on quads
//!
//! It was wired into the existing stages four different ways before this was
//! understood, and every one of them measured neutral or worse:
//!
//! * biasing which of a region's edges the collapse merges first - no change,
//!   because the output's edges run between the *survivors* of neighbouring
//!   regions, and which vertex survives is not decided there;
//! * measuring the partition's distances in the field's own frame, so regions
//!   come out square - no change, because a centroid is a centroid: spreading
//!   seeds out evenly settles them into a hexagonal packing whatever metric the
//!   distances were taken in;
//! * pulling the seeds themselves onto a grid during the relaxation - worse,
//!   because a seed is snapped back to the nearest *input* vertex afterwards,
//!   and at half density the input is barely finer than the output, so the
//!   target is rounded away before it can act;
//! * pulling the finished vertices onto a grid, where positions are finally
//!   real numbers - worse again, and this one gives the reason for all of it.
//!
//! **A cross describes a square lattice and a triangle mesh wants a hexagonal
//! one.** An interior vertex of a good triangulation has six neighbours; a
//! square grid offers four directions. Snapping six neighbours onto four makes
//! two pairs collide, and what comes out is a worse target than the plain
//! neighbourhood centre - measured, a plant's badly-shaped tenth fell from
//! 0.508 to 0.425.
//!
//! So the field is right and the place to spend it is the quad merge
//! ([`super::quads`]), where the output *is* four-valent and the lattice it
//! describes is the one being built. A triangle mesh would want the six-fold
//! field instead, which is the same construction with six rotations in place of
//! four.
//!
//! ## What it does not do
//!
//! Nothing about singularities. A cross field on a closed surface must have
//! points where the four directions cannot be combed flat - eight of them on a
//! sphere - and a quad *extraction* has to know where they are, because they are
//! where the quad grid's irregular vertices go. Smoothing puts them somewhere
//! sensible on its own without ever naming them, which is enough for the merge
//! to line neighbouring quads up and is not enough to extract a quad layout.
//! The rebuild does not attempt one: it pairs finished triangles rather than
//! laying out a quad grid.

// The one consumer is `super::quads`. `solve` builds the field only when
// quads were asked for, and only just before the merge, because it costs about
// a third of a rebuild's time and nothing earlier can spend it.

use crate::cancel::{CancelToken, cancelled};
use crate::parallel;

use super::features::Features;
use super::surface::Surface;

/// Passes of smoothing over the field.
///
/// A Jacobi pass propagates one ring, so this is how far a feature's direction
/// reaches into a flat region that has no opinion of its own. Thirty-two is
/// enough to cross the middle of an ordinary object and cheap next to the
/// normal averaging the size field already does.
const SMOOTH_PASSES: u32 = 32;

/// Stop early when no direction moved by more than this, in radians of the
/// *cross* (so a quarter turn is the most anything can move).
const SETTLED: f32 = 1.0e-3;

/// A direction per vertex, each standing for the four at right angles to it.
#[derive(Debug, Default)]
pub(crate) struct CrossField {
    /// Three floats per vertex: a unit vector in that vertex's tangent plane,
    /// or zero where the vertex has no usable normal.
    directions: Vec<f32>,
}

impl CrossField {
    /// The representative direction at `vertex`, or `None` where the field has
    /// nothing to say - an unreferenced vertex, or one whose normal is
    /// degenerate.
    pub(crate) fn at(&self, vertex: u32) -> Option<[f32; 3]> {
        let base = vertex as usize * 3;
        let slice = self.directions.get(base..base + 3)?;
        let direction = [slice[0], slice[1], slice[2]];
        (direction[0] != 0.0 || direction[1] != 0.0 || direction[2] != 0.0).then_some(direction)
    }
}

/// Build the field over `surface`.
///
/// `normals` is three floats per vertex, as [`super::size_field::vertex_normals`]
/// produces.
pub(crate) fn build(
    surface: Surface<'_>,
    normals: &[f32],
    features: &Features,
    threads: usize,
    cancel: Option<&CancelToken>,
) -> CrossField {
    let _z = crate::prof::zone!("Remesh Cross Field");
    let count = surface.vertex_count();
    if count == 0 || normals.len() < count * 3 {
        return CrossField::default();
    }

    let mut pinned = vec![false; count];
    let mut directions = vec![0.0f32; count * 3];
    seed(surface, normals, features, &mut directions, &mut pinned);
    if cancelled(cancel) {
        return CrossField { directions };
    }

    let mut next = directions.clone();
    for _ in 0..SMOOTH_PASSES {
        if cancelled(cancel) {
            break;
        }
        let moved = smooth(surface, normals, &pinned, &directions, &mut next, threads);
        std::mem::swap(&mut directions, &mut next);
        if moved < SETTLED {
            break;
        }
    }
    CrossField { directions }
}

/// Pin every feature vertex to the direction its feature runs in, and give
/// everything else a starting guess.
fn seed(
    surface: Surface<'_>,
    normals: &[f32],
    features: &Features,
    directions: &mut [f32],
    pinned: &mut [bool],
) {
    for line in &features.polylines {
        let count = line.vertices.len();
        if count < 2 {
            continue;
        }
        for (at, &vertex) in line.vertices.iter().enumerate() {
            // One adjacent edge, not the chord across the vertex.
            //
            // A cross does not distinguish a quarter turn, so where a chain
            // turns a right angle - the corner of a panel, the tip of a leaf -
            // its two edges stand for the *same* cross and either will do. The
            // chord between them does not: it bisects the corner, which is
            // forty-five degrees from both, and forty-five degrees is as wrong
            // as a cross can be.
            let (from, to) = if at + 1 < count {
                (vertex, line.vertices[at + 1])
            } else if line.closed {
                (vertex, line.vertices[0])
            } else {
                (line.vertices[at - 1], vertex)
            };
            let a = surface.position(from);
            let b = surface.position(to);
            let along = [
                (b[0] - a[0]) as f32,
                (b[1] - a[1]) as f32,
                (b[2] - a[2]) as f32,
            ];
            let Some(normal) = unit(normal_at(normals, vertex)) else {
                continue;
            };
            // A chain that runs across the surface rather than along it - two
            // sheets of a rim meeting - would otherwise pin a direction with no
            // tangential part at all.
            let Some(tangential) = unit(in_plane(along, normal)) else {
                continue;
            };
            write(directions, vertex, tangential);
            pinned[vertex as usize] = true;
        }
    }

    // Everything else starts from a fixed world direction pushed into the
    // tangent plane. Which one hardly matters - the smoothing overwrites it
    // wherever a feature can reach - but it has to be the same on every run, so
    // it is chosen by the normal rather than at random.
    for vertex in 0..directions.len() as u32 / 3 {
        if pinned[vertex as usize] {
            continue;
        }
        let Some(normal) = unit(normal_at(normals, vertex)) else {
            continue;
        };
        let axis = if normal[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        if let Some(start) = unit(in_plane(axis, normal)) {
            write(directions, vertex, start);
        }
    }
}

/// One Jacobi pass. Returns how far the furthest direction moved.
fn smooth(
    surface: Surface<'_>,
    normals: &[f32],
    pinned: &[bool],
    from: &[f32],
    into: &mut [f32],
    threads: usize,
) -> f32 {
    let indices = surface.indices;
    let topology = surface.topology;
    parallel::sweep(into, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let at = base + offset;
            let vertex = (at / 3) as u32;
            let axis = at % 3;
            // Each of the three floats of a vertex recomputes the same answer.
            // Wasteful by a factor of three and worth it: the sweep hands out
            // flat slices, so this is what keeps a vertex's three components in
            // one chunk and the pass independent of how the work was split.
            let mixed = mix(normals, topology, indices, pinned, from, vertex);
            *slot = mixed[axis];
        }
    });

    let mut moved = 0.0f32;
    for vertex in 0..(into.len() / 3) as u32 {
        let (before, after) = (read(from, vertex), read(into, vertex));
        let Some(normal) = unit(normal_at(normals, vertex)) else {
            continue;
        };
        // How far it turned, as the cross sees it: a quarter turn is no turn.
        let aligned = nearest_rotation(after, before, normal);
        let dot = dot(aligned, before).clamp(-1.0, 1.0);
        moved = moved.max(dot.acos());
    }
    moved
}

/// The new direction at one vertex: its neighbours, each turned into this
/// vertex's plane and rotated to agree, averaged.
fn mix(
    normals: &[f32],
    topology: &super::topology::Topology,
    indices: &[u32],
    pinned: &[bool],
    from: &[f32],
    vertex: u32,
) -> [f32; 3] {
    let here = read(from, vertex);
    if pinned[vertex as usize] {
        return here;
    }
    let Some(normal) = unit(normal_at(normals, vertex)) else {
        return here;
    };

    let mut sum = [0.0f32; 3];
    let mut seen = 0u32;
    for &face in topology.faces_of(vertex) {
        let corners = &indices[face as usize * 3..face as usize * 3 + 3];
        let Some(at) = corners.iter().position(|&corner| corner == vertex) else {
            continue;
        };
        // Both other corners, in the order `size_field` walks them, so two
        // stages that visit a neighbourhood agree about what order it is in.
        let pair = if at == 0 {
            [corners[1], corners[2]]
        } else {
            [corners[(at + 2) % 3], corners[(at + 1) % 3]]
        };
        for other in pair {
            let Some(theirs) = unit(in_plane(read(from, other), normal)) else {
                continue;
            };
            let aligned = nearest_rotation(theirs, here, normal);
            for axis in 0..3 {
                sum[axis] += aligned[axis];
            }
            seen += 1;
        }
    }
    if seen == 0 {
        return here;
    }
    // A neighbourhood whose crosses cancel exactly - a singularity sitting on
    // this vertex - has no average, and the honest answer is not to move.
    unit(in_plane(sum, normal)).unwrap_or(here)
}

/// Whichever of the four directions `candidate` stands for is closest to
/// `target`, in the plane of `normal`.
fn nearest_rotation(candidate: [f32; 3], target: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let across = cross(normal, candidate);
    let mut best = candidate;
    let mut best_dot = dot(candidate, target);
    for option in [
        across,
        [-candidate[0], -candidate[1], -candidate[2]],
        [-across[0], -across[1], -across[2]],
    ] {
        let score = dot(option, target);
        // Strictly greater, so a tie keeps the earlier option and the answer
        // does not depend on the order the four were listed in.
        if score > best_dot {
            best_dot = score;
            best = option;
        }
    }
    best
}

fn normal_at(normals: &[f32], vertex: u32) -> [f32; 3] {
    let base = vertex as usize * 3;
    match normals.get(base..base + 3) {
        Some(slice) => [slice[0], slice[1], slice[2]],
        None => [0.0; 3],
    }
}

fn read(directions: &[f32], vertex: u32) -> [f32; 3] {
    let base = vertex as usize * 3;
    match directions.get(base..base + 3) {
        Some(slice) => [slice[0], slice[1], slice[2]],
        None => [0.0; 3],
    }
}

fn write(directions: &mut [f32], vertex: u32, value: [f32; 3]) {
    let base = vertex as usize * 3;
    if let Some(slice) = directions.get_mut(base..base + 3) {
        slice.copy_from_slice(&value);
    }
}

/// `vector` with the part along `normal` removed.
fn in_plane(vector: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let along = dot(vector, normal);
    [
        vector[0] - along * normal[0],
        vector[1] - along * normal[1],
        vector[2] - along * normal[2],
    ]
}

fn unit(vector: [f32; 3]) -> Option<[f32; 3]> {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    (length > 1.0e-12).then(|| [vector[0] / length, vector[1] / length, vector[2] / length])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remesh::size_field;
    use crate::remesh::topology::Topology;

    /// A flat grid in the xy plane, `steps` quads on a side.
    fn grid(steps: u32) -> (Vec<f32>, Vec<u32>) {
        let mut positions = Vec::new();
        for row in 0..=steps {
            for column in 0..=steps {
                positions.extend_from_slice(&[column as f32, row as f32, 0.0]);
            }
        }
        let width = steps + 1;
        let mut indices = Vec::new();
        for row in 0..steps {
            for column in 0..steps {
                let at = row * width + column;
                indices.extend_from_slice(&[at, at + 1, at + width]);
                indices.extend_from_slice(&[at + 1, at + width + 1, at + width]);
            }
        }
        (positions, indices)
    }

    /// The field over a mesh, with borders as features unless `borders` is off.
    fn field_of(positions: &[f32], indices: &[u32], borders: bool, threads: usize) -> CrossField {
        let topology = Topology::build(indices, positions.len() / 3, 1, None);
        let sizes = vec![1.0f32; topology.vertex_count];
        let areas = size_field::dual_areas(positions, indices, &topology, 1);
        let surface = Surface {
            positions,
            indices,
            topology: &topology,
            sizes: &sizes,
            areas: &areas,
        };
        let normals = size_field::vertex_normals(positions, indices, &topology, 1);
        let features = crate::remesh::features::build(surface, borders, None, None);
        build(surface, &normals, &features, threads, None)
    }

    /// How far two directions are from agreeing, as the cross sees it: zero when
    /// they are parallel *or* at right angles, a maximum at forty-five degrees.
    fn disagreement(a: [f32; 3], b: [f32; 3], normal: [f32; 3]) -> f32 {
        let aligned = nearest_rotation(a, b, normal);
        dot(aligned, b).clamp(-1.0, 1.0).acos()
    }

    /// On a flat sheet every cross agrees with every other one.
    ///
    /// The field has nothing to resolve here - one plane, one set of
    /// directions - so anything but agreement would be the smoothing failing to
    /// converge rather than the surface being hard.
    #[test]
    fn a_flat_sheet_combs_flat() {
        let (positions, indices) = grid(8);

        let field = field_of(&positions, &indices, true, 1);

        let normal = [0.0f32, 0.0, 1.0];
        let first = field.at(0).expect("the field reaches the first vertex");
        for vertex in 0..(positions.len() / 3) as u32 {
            let Some(direction) = field.at(vertex) else {
                continue;
            };
            assert!(
                disagreement(direction, first, normal) < 0.05,
                "vertex {vertex} disagrees with the rest of the sheet by {:.1} degrees",
                disagreement(direction, first, normal).to_degrees()
            );
        }
    }

    /// Every direction stays in its own vertex's tangent plane.
    ///
    /// A direction with a component along the normal is not a direction *on the
    /// surface*, and everything downstream reads it as one.
    #[test]
    fn every_direction_lies_on_the_surface() {
        // A cylinder: curved, so the tangent planes genuinely differ.
        let (rings, segments) = (6u32, 16u32);
        let mut positions = Vec::new();
        for ring in 0..=rings {
            for segment in 0..segments {
                let angle = segment as f32 / segments as f32 * std::f32::consts::TAU;
                positions.extend_from_slice(&[angle.cos(), ring as f32 * 0.5, angle.sin()]);
            }
        }
        let mut indices = Vec::new();
        for ring in 0..rings {
            for segment in 0..segments {
                let next = (segment + 1) % segments;
                let a = ring * segments + segment;
                let b = ring * segments + next;
                let c = (ring + 1) * segments + segment;
                let d = (ring + 1) * segments + next;
                indices.extend_from_slice(&[a, c, b]);
                indices.extend_from_slice(&[b, c, d]);
            }
        }

        let topology = Topology::build(&indices, positions.len() / 3, 1, None);
        let normals = size_field::vertex_normals(&positions, &indices, &topology, 1);
        let field = field_of(&positions, &indices, true, 1);

        for vertex in 0..(positions.len() / 3) as u32 {
            let (Some(direction), Some(normal)) =
                (field.at(vertex), unit(normal_at(&normals, vertex)))
            else {
                continue;
            };
            assert!(
                dot(direction, normal).abs() < 1.0e-3,
                "the direction at {vertex} leans out of the surface by {}",
                dot(direction, normal)
            );
        }
    }

    /// A border runs along the field, not across it.
    ///
    /// This is the constraint the whole field is grown from: an open edge is a
    /// direction the surface has already committed to, and a rebuild that lays
    /// its faces across one gives the ragged fringe this is all meant to avoid.
    #[test]
    fn the_field_runs_along_a_border() {
        let (positions, indices) = grid(8);
        let field = field_of(&positions, &indices, true, 1);

        // The bottom row of the grid, which is a border running along +x.
        let normal = [0.0f32, 0.0, 1.0];
        for column in 0..9u32 {
            let direction = field.at(column).expect("a border vertex has a direction");
            let along = [1.0f32, 0.0, 0.0];
            assert!(
                disagreement(direction, along, normal) < 0.05,
                "the border vertex {column} points {:.1} degrees off the border",
                disagreement(direction, along, normal).to_degrees()
            );
        }
    }

    /// The field is the same however many threads built it.
    #[test]
    fn the_field_is_the_same_at_every_thread_count() {
        let (positions, indices) = grid(12);

        let one = field_of(&positions, &indices, true, 1);
        for threads in [2usize, 3, 8] {
            let many = field_of(&positions, &indices, true, threads);
            assert_eq!(
                one.directions, many.directions,
                "the field differs at {threads} threads"
            );
        }
    }

    /// A cross stands for four directions, so a quarter turn is not a change.
    #[test]
    fn a_quarter_turn_is_not_a_turn() {
        let normal = [0.0f32, 0.0, 1.0];
        let target = [1.0f32, 0.0, 0.0];
        for turned in [[0.0f32, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0]] {
            let aligned = nearest_rotation(turned, target, normal);
            assert!(
                dot(aligned, target) > 0.999,
                "{turned:?} did not come back as {target:?}"
            );
        }
    }
}
