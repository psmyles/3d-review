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
//! ## What is never touched
//!
//! Border vertices do not move and border edges are never flipped. The outline
//! is the thing the whole rebuild is most careful about, and a relaxation that
//! slid vertices along it would undo the even spacing the seeding gave it. A
//! border vertex is recognised from the *output's* own topology — an edge with
//! one face — so nothing has to be carried here from earlier stages.

use crate::cancel::{CancelToken, cancelled};

use super::RemeshOutput;
use super::topology::Topology;

/// How far toward the neighbourhood centre a vertex moves in one pass. Half is
/// the usual choice: the whole way overshoots and oscillates.
const RELAXATION: f64 = 0.5;

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
pub(crate) fn run(output: &mut RemeshOutput, rounds: u32, cancel: Option<&CancelToken>) {
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
        relax_pass(output, &topology);
    }
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
            let Some((left, right)) = opposite_corners(output, first, second, vertex, edge.other)
            else {
                continue;
            };
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
            if !flip_is_safe(output, topology, vertex, edge.other, left, right, &scratch) {
                continue;
            }
            write_face(output, first, [vertex, left, right]);
            write_face(output, second, [edge.other, right, left]);
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

/// The two corners opposite a shared edge, oriented so the rewritten faces keep
/// the surface's winding.
fn opposite_corners(
    output: &RemeshOutput,
    first: u32,
    second: u32,
    a: u32,
    b: u32,
) -> Option<(u32, u32)> {
    let third = |face: u32| -> Option<u32> {
        let corners = face_of(output, face);
        corners
            .iter()
            .copied()
            .find(|&corner| corner != a && corner != b)
    };
    let left = third(first)?;
    let right = third(second)?;
    (left != right).then_some((left, right))
}

/// Whether flipping is legal: it must not duplicate an existing edge, and it
/// must not turn either new face inside out.
fn flip_is_safe(
    output: &RemeshOutput,
    topology: &Topology,
    a: u32,
    b: u32,
    left: u32,
    right: u32,
    _scratch: &[super::topology::EdgeAt],
) -> bool {
    // An edge between the two opposite corners already existing would make the
    // flip produce a second copy of it, which no surface can carry.
    let mut edges = Vec::new();
    topology.edges_at(&output.corners, left, &mut edges);
    if edges.iter().any(|edge| edge.other == right) {
        return false;
    }

    let before = [
        normal_of(output, [a, left, b]),
        normal_of(output, [b, right, a]),
    ];
    let after = [
        normal_of(output, [a, left, right]),
        normal_of(output, [b, right, left]),
    ];
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
fn relax_pass(output: &mut RemeshOutput, topology: &Topology) {
    let count = output.positions.len();
    let mut moved: Vec<[f64; 3]> = output
        .positions
        .iter()
        .map(|point| [point.x as f64, point.y as f64, point.z as f64])
        .collect();
    let original = moved.clone();
    let mut scratch = Vec::new();

    for vertex in 0..count as u32 {
        // A border vertex holds the outline; the seeding already spaced it.
        if topology.boundary[vertex as usize] || topology.nonmanifold[vertex as usize] {
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
        let Some(normal) = vertex_normal(output, topology, vertex) else {
            continue;
        };
        let along = step[0] * normal[0] + step[1] * normal[1] + step[2] * normal[2];
        for axis in 0..3 {
            moved[vertex as usize][axis] = here[axis] + step[axis] - along * normal[axis];
        }
    }

    for (slot, point) in output.positions.iter_mut().zip(&moved) {
        *slot = glam::Vec3::new(point[0] as f32, point[1] as f32, point[2] as f32);
    }
}

/// The area-weighted normal of the faces around a vertex.
fn vertex_normal(output: &RemeshOutput, topology: &Topology, vertex: u32) -> Option<[f64; 3]> {
    let mut sum = [0.0f64; 3];
    for &face in topology.faces_of(vertex) {
        let corners = face_of(output, face);
        let Some(normal) = normal_of(output, corners) else {
            continue;
        };
        let area = area_of(output, corners);
        for (axis, value) in sum.iter_mut().enumerate() {
            *value += normal[axis] * area;
        }
    }
    let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    (length > 0.0).then(|| [sum[0] / length, sum[1] / length, sum[2] / length])
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
    let (a, b, c) = (
        point_of(output, corners[0]),
        point_of(output, corners[1]),
        point_of(output, corners[2]),
    );
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

fn normal_of(output: &RemeshOutput, corners: [u32; 3]) -> Option<[f64; 3]> {
    let cross = cross_of(output, corners);
    let length = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    (length > 0.0).then(|| [cross[0] / length, cross[1] / length, cross[2] / length])
}

fn area_of(output: &RemeshOutput, corners: [u32; 3]) -> f64 {
    let cross = cross_of(output, corners);
    (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() * 0.5
}
