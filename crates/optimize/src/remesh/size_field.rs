//! How big a face should be, per vertex.
//!
//! The rule is `h` proportional to `k` to the power of minus the strength, where
//! `k` is how tightly the surface turns here and `h` is the face size to ask
//! for. The two ends of that slider are both named rules rather than arbitrary
//! settings. At 0.5, `h ~ 1/sqrt(k)`: a chord of length `h` across a surface of
//! curvature `k` misses it by about `k * h^2 / 8`, so the square root is what
//! spends the same *chordal error* everywhere. At 1.0, `h ~ 1/k`: every face
//! turns through the same *angle*, which is the rule a hand retopology follows
//! and is markedly more aggressive — a tube of half the radius gets faces of
//! half the size rather than of seven tenths.
//!
//! Everything else here is the guards that stop that from being useless in
//! practice — a floor under "flat", a dilation so a one-ring-wide fillet
//! survives, and a floor under how fine a face may be asked to be.
//!
//! ## A port, deliberately line for line
//!
//! This began as `remesh_density.cpp`, the density pass of the C++ bridge
//! Remesh used to be built on, and it keeps that file's constants,
//! its pass counts, its arithmetic in `f64` and — in [`for_each_neighbour`] —
//! even the *order* its neighbour lists were built in, because a float sum is
//! not associative and the port was checked against the original to the last
//! bit where it could be. The C++ is gone; "the reference" below means it. The
//! constants are the measured part of this operation: each was set against a
//! real asset, and the reasons are kept with them below.
//!
//! The one deliberate difference is the **reductions**, which fold per
//! [`parallel::CHUNK`] and merge in chunk order rather than summing straight
//! down the vertex array. That is what makes the answer independent of the
//! thread count, and it moves the totals by about one part in 1e15.
//!
//! ## Everything here is a Jacobi sweep
//!
//! Every pass reads one buffer and writes another, so a vertex never sees a
//! value from its own pass. That is what lets [`parallel::sweep`] split it
//! across cores without the schedule reaching the result, and what lets the
//! whole thing be abandoned between passes when the user has moved on.

use crate::cancel::{CancelToken, cancelled};
use crate::parallel;

use super::geom;
use super::topology::Topology;

/// A surface counts as flat once its radius of curvature passes this many times
/// the object's own diagonal. Expressed against the object rather than against a
/// length, because it has to mean the same thing whether the mesh is in meters
/// or normalized into a unit box.
const FLAT_RADII: f64 = 100.0;

/// How far the normals are averaged before curvature is read off them, as a
/// multiple of the target face size.
///
/// This is the whole difference between a field that helps and one that makes
/// the result worse. Curvature taken across one triangle answers "how bumpy is
/// this surface", and a sculpted or scanned asset is bumpy *everywhere* at that
/// scale — so a flat slab reads as detailed, the field asks for small faces over
/// all of it, and what comes back is worse than the even layout it replaced.
/// Measured on a sculpted pedestal: the slab's median face went from 12mm to 7mm
/// and the grid broke up into a triangle mess.
///
/// Averaging the normals first asks the question that actually matters: does the
/// surface *turn* over the distance one face spans. Bumps finer than a face
/// cancel — no face size can represent them, so nothing is lost by ignoring them
/// — while a fillet or a rim, which turns through most of a right angle over its
/// width, survives untouched.
const NORMAL_RADII: f64 = 1.0;

/// A ceiling on the normal averaging, in passes. Only reached when the input is
/// far finer than the target face size, where the cost of the exact answer is
/// out of proportion to a field that is then clamped and smoothed anyway.
const MAX_NORMAL_PASSES: i32 = 256;

/// Passes that spread the *dense* requirement outward before anything is
/// averaged. A fillet one ring wide is exactly the feature this is for, and
/// smoothing it against its flat neighbours is what would lose it.
const DILATE_PASSES: i32 = 2;

/// Laplacian passes over the dilated field, to take the last of the noise out. A
/// scale field that is not smooth is one the position solve fights rather than
/// follows.
const SMOOTH_PASSES: i32 = 6;

/// How far either side of the object's own average face size the field may reach
/// at full strength, as a ratio — so the widest the whole field can span is its
/// square. Past that the layout starts to come apart. Tightening it is not a
/// free quality win, it just moves along the same axis the strength slider
/// already runs: measured on a driftwood branch at full strength, dropping this
/// to 2 took the face-size spread from 6.1x down to 4.2x.
const MAX_RANGE: f64 = 4.0;

/// How much finer than the triangles under it a face may be asked to be — or
/// rather, how much it may not. The rebuild works from the input mesh; it cannot
/// place a vertex where the input has no surface, so a field that asks for it
/// does not get those faces, it just loses the ones the flat side gave up. This
/// is the single biggest reason a strong field can come back with fewer faces
/// than a uniform one.
const SUPPORT: f64 = 1.2;

/// Passes of the budget normalization. The floor above takes area out of the
/// budget sum, so it is iterated rather than solved; it is monotone, so stopping
/// early only leaves the field slightly coarse.
const NORMALIZE_PASSES: i32 = 8;

/// Where the normalization stops: a correction this close to 1 has converged.
const NORMALIZE_EPSILON: f64 = 1.0e-4;

/// How far from 1 a multiplier has to be for the field to count as varying.
const VARIES_EPSILON: f64 = 1.0e-3;

/// The surface a field is wanted over, and the index of it.
///
/// Bundled rather than passed as five parameters for the reason the C++ this
/// was ported from bundled its own: the list only grows, and every caller
/// passes the same five things together.
#[derive(Clone, Copy)]
pub(crate) struct FieldInput<'a> {
    /// Three per vertex.
    pub(crate) positions: &'a [f32],
    /// Three per vertex, unit length.
    pub(crate) normals: &'a [f32],
    /// One per vertex — the dual area, so the budget follows surface rather
    /// than triangle count. `None` weighs every vertex the same, which biases
    /// the budget toward whichever part of the surface is finely triangulated.
    pub(crate) areas: Option<&'a [f32]>,
    /// Three per triangle.
    pub(crate) indices: &'a [u32],
    pub(crate) topology: &'a Topology,
}

/// One spacing multiplier per vertex: the factor the uniform target edge length
/// is scaled by there. Below 1 is denser than the uniform result, above 1 is
/// coarser, and the field is normalized so the *face count* comes out where it
/// would have — the budget is redistributed, not raised.
///
/// `strength` runs 0 (no field at all) to 1 (as far as the layout will take it).
/// `target_edge` is the uniform spacing derived from the requested face count,
/// in the same units as `positions`.
///
/// `None` means there is nothing to do — `strength` at zero, an empty mesh, a
/// surface with no area, or a field that came out flat. A caller that gets
/// `None` uses one size everywhere, which is the same answer by a shorter road.
pub(crate) fn build(
    input: &FieldInput<'_>,
    strength: f32,
    target_edge: f32,
    threads: usize,
    cancel: Option<&CancelToken>,
) -> Option<Vec<f32>> {
    let _z = crate::prof::zone!("Remesh Size Field");
    let &FieldInput {
        positions,
        normals,
        areas,
        indices,
        topology,
    } = input;
    let count = topology.vertex_count;
    // Spelled out rather than left to a negated comparison, which reads as the
    // opposite of what it does: a NaN must fail these, and `x <= 0.0` alone
    // lets one through.
    if strength.is_nan()
        || strength <= 0.0
        || count == 0
        || indices.len() < 3
        || target_edge.is_nan()
        || target_edge <= 0.0
        || positions.len() < count * 3
        || normals.len() < count * 3
    {
        return None;
    }

    let diagonal = diagonal_of(positions, count);
    if diagonal.is_nan() || diagonal <= 0.0 {
        return None;
    }
    let flat = 1.0 / (FLAT_RADII * diagonal);

    let position_of = |vertex: u32| -> [f64; 3] {
        let base = vertex as usize * 3;
        [
            positions[base] as f64,
            positions[base + 1] as f64,
            positions[base + 2] as f64,
        ]
    };
    let weight_of = |vertex: usize| -> f64 {
        let area = areas.map_or(1.0, |areas| areas[vertex] as f64);
        if area > 0.0 { area } else { 0.0 }
    };

    // Every vertex's incident edge lengths, which two later steps want: the
    // averaging radius is set against them, and so is the floor on how fine a
    // face may be asked to be.
    let mut span = vec![0.0f64; count];
    let mut degree = vec![0u32; count];
    parallel::sweep(&mut span, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let vertex = (base + offset) as u32;
            let here = position_of(vertex);
            let mut total = 0.0;
            for_each_neighbour(topology, indices, vertex, |other| {
                let length = distance(here, position_of(other));
                if length > 0.0 {
                    total += length;
                }
            });
            *slot = total;
        }
    });
    parallel::sweep(&mut degree, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let vertex = (base + offset) as u32;
            let here = position_of(vertex);
            let mut count = 0u32;
            for_each_neighbour(topology, indices, vertex, |other| {
                if distance(here, position_of(other)) > 0.0 {
                    count += 1;
                }
            });
            *slot = count;
        }
    });

    // Each (triangle, corner) length lands in exactly two spans, so half their
    // sum is the total the mean edge is taken over. The divisor is the corner
    // count whether or not a pair was skipped, exactly as the reference does.
    let total_length: f64 = merge(parallel::sweep_reduce(count, threads, |range| {
        range.map(|vertex| span[vertex]).sum::<f64>()
    })) * 0.5;
    let triangles = indices.len() / 3;
    let mean_edge = if total_length > 0.0 {
        total_length / (3 * triangles) as f64
    } else {
        0.0
    };
    if mean_edge.is_nan() || mean_edge <= 0.0 {
        return None;
    }
    if cancelled(cancel) {
        return None;
    }

    // Average the normals out to the radius a face will span (see
    // NORMAL_RADII). One Laplacian pass diffuses about half an edge length, so
    // reaching a radius takes its square in passes.
    let radius = NORMAL_RADII * target_edge as f64;
    let passes =
        MAX_NORMAL_PASSES.min((4.0 * radius * radius / (mean_edge * mean_edge)).ceil() as i32);
    // One element per *vertex*, never per float. The sweep hands out fixed
    // chunks of elements, and CHUNK is not a multiple of three: sweeping a flat
    // float buffer put chunk k's first vertex `base % 3` floats away from where
    // its local writes landed, which scrambled every normal past vertex 21845.
    let mut smoothed: Vec<[f64; 3]> = (0..count)
        .map(|vertex| {
            let at = vertex * 3;
            [
                normals[at] as f64,
                normals[at + 1] as f64,
                normals[at + 2] as f64,
            ]
        })
        .collect();
    {
        let mut next = vec![[0.0f64; 3]; smoothed.len()];
        for _ in 0..passes {
            parallel::sweep(&mut next, threads, |base, chunk| {
                for (offset, slot) in chunk.iter_mut().enumerate() {
                    let vertex = (base + offset) as u32;
                    let own = smoothed[vertex as usize];
                    let mut sum = own;
                    for_each_neighbour(topology, indices, vertex, |other| {
                        let theirs = smoothed[other as usize];
                        for (value, add) in sum.iter_mut().zip(theirs) {
                            *value += add;
                        }
                    });
                    // Renormalized rather than averaged: what is wanted is the
                    // direction the surface faces around here, and two opposed
                    // normals a fold apart must not average to a shorter vector
                    // that then reads as low curvature.
                    let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
                    *slot = if length > 0.0 {
                        sum.map(|value| value / length)
                    } else {
                        own
                    };
                }
            });
            std::mem::swap(&mut smoothed, &mut next);
            if cancelled(cancel) {
                return None;
            }
        }
    }

    // Curvature, as the angle the surface turns through per unit length — and
    // specifically the *largest* such rate over the incident edges, not their
    // average.
    //
    // That is the difference between measuring the tightest direction and
    // measuring a blend of every direction, and on the shapes this tool is
    // pointed at it is most of the signal. A branch is a tube: it turns through
    // `1/radius` around its girth and through nothing at all along its length,
    // so averaging the two reports a fraction of the curvature that actually
    // constrains the face size. Measured on a driftwood branch: trunk to tip,
    // the averaged rate spans 3.2x and the largest spans 3.7x, and at the same
    // strength that turns a 3.4x spread of output face sizes into 4.2x. A face
    // has to be small enough to follow the tightest direction; the others are
    // free.
    //
    // A maximum needs no ordering, so unlike the sums this one is exactly the
    // reference's answer however it is split up.
    let mut spacing = vec![0.0f64; count];
    parallel::sweep(&mut spacing, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let vertex = (base + offset) as u32;
            let here = position_of(vertex);
            let own = smoothed[vertex as usize];
            let mut peak = 0.0f64;
            for_each_neighbour(topology, indices, vertex, |other| {
                let there = position_of(other);
                let theirs = smoothed[other as usize];
                let dot = own[0] * theirs[0] + own[1] * theirs[1] + own[2] * theirs[2];
                let length = distance(here, there);
                if length > 0.0 {
                    let angle = dot.clamp(-1.0, 1.0).acos();
                    peak = peak.max(angle / length);
                }
            });
            // In logs throughout: the smoothing, the averaging and the strength
            // blend are all multiplicative on a spacing, and a scale field is a
            // ratio rather than a difference. The additive constant drops out at
            // the centring step, so only the shape of the field matters here —
            // which is also why the exponent is applied later, as a multiply.
            *slot = -(peak + flat).ln();
        }
    });
    drop(smoothed);
    if cancelled(cancel) {
        return None;
    }

    let mut scratch = vec![0.0f64; count];
    for _ in 0..DILATE_PASSES {
        parallel::sweep(&mut scratch, threads, |base, chunk| {
            for (offset, slot) in chunk.iter_mut().enumerate() {
                let vertex = (base + offset) as u32;
                let mut smallest = spacing[vertex as usize];
                for_each_neighbour(topology, indices, vertex, |other| {
                    smallest = smallest.min(spacing[other as usize]);
                });
                *slot = smallest;
            }
        });
        std::mem::swap(&mut spacing, &mut scratch);
    }
    for _ in 0..SMOOTH_PASSES {
        parallel::sweep(&mut scratch, threads, |base, chunk| {
            for (offset, slot) in chunk.iter_mut().enumerate() {
                let vertex = (base + offset) as u32;
                let mut sum = spacing[vertex as usize];
                let mut weight = 1.0f64;
                for_each_neighbour(topology, indices, vertex, |other| {
                    sum += spacing[other as usize];
                    weight += 1.0;
                });
                *slot = sum / weight;
            }
        });
        std::mem::swap(&mut spacing, &mut scratch);
    }
    drop(scratch);
    if cancelled(cancel) {
        return None;
    }

    let total_area = merge(parallel::sweep_reduce(count, threads, |range| {
        range.map(weight_of).sum::<f64>()
    }));
    if total_area.is_nan() || total_area <= 0.0 {
        return None;
    }
    let mean = merge(parallel::sweep_reduce(count, threads, |range| {
        range
            .map(|vertex| weight_of(vertex) * spacing[vertex])
            .sum::<f64>()
    })) / total_area;

    let limit = MAX_RANGE.ln();
    let mut field = vec![1.0f64; count];
    parallel::sweep(&mut field, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let centred = (spacing[base + offset] - mean) * strength as f64;
            *slot = centred.clamp(-limit, limit).exp();
        }
    });
    drop(spacing);

    // No face finer than the triangles under it (see SUPPORT).
    let mut floor_at = vec![0.0f64; count];
    parallel::sweep(&mut floor_at, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let vertex = base + offset;
            let edge = if degree[vertex] > 0 {
                span[vertex] / degree[vertex] as f64
            } else {
                0.0
            };
            *slot = edge * SUPPORT / target_edge as f64;
        }
    });

    for _ in 0..NORMALIZE_PASSES {
        parallel::sweep(&mut field, threads, |base, chunk| {
            for (offset, slot) in chunk.iter_mut().enumerate() {
                *slot = slot.max(floor_at[base + offset]);
            }
        });
        // A face covers `(target_edge * field)^2` of surface, so holding the
        // count means holding the area-weighted mean of `1 / field^2` at one.
        let density = merge(parallel::sweep_reduce(count, threads, |range| {
            range
                .map(|vertex| weight_of(vertex) / (field[vertex] * field[vertex]))
                .sum::<f64>()
        }));
        let correction = (density / total_area).sqrt();
        if !correction.is_finite()
            || correction <= 0.0
            || (correction - 1.0).abs() < NORMALIZE_EPSILON
        {
            break;
        }
        parallel::sweep(&mut field, threads, |_, chunk| {
            for slot in chunk.iter_mut() {
                *slot *= correction;
            }
        });
        if cancelled(cancel) {
            return None;
        }
    }

    let mut out = vec![1.0f32; count];
    let mut varies = false;
    for (vertex, &value) in field.iter().enumerate() {
        if !value.is_finite() || value <= 0.0 {
            return None;
        }
        out[vertex] = value as f32;
        if (value - 1.0).abs() > VARIES_EPSILON {
            varies = true;
        }
    }
    varies.then_some(out)
}

/// Visit `vertex`'s neighbours, with the multiset *and the order* the reference
/// implementation's adjacency array held.
///
/// It built one entry per (triangle, corner) pair in corner order, which for a
/// vertex works out to the two other corners of each of its incident faces —
/// but which of the two comes first depends on where the vertex sits in the
/// face, because the pair naming it as the *second* endpoint is visited one
/// iteration earlier. A neighbour reached from two triangles is simply weighted
/// twice; every use here is a sum, a mean or a maximum, so that is deliberate
/// and not a bug to be deduplicated away.
fn for_each_neighbour(
    topology: &Topology,
    indices: &[u32],
    vertex: u32,
    mut visit: impl FnMut(u32),
) {
    for &face in topology.faces_of(vertex) {
        let corners = &indices[face as usize * 3..face as usize * 3 + 3];
        let Some(at) = corners.iter().position(|&corner| corner == vertex) else {
            continue;
        };
        let next = corners[(at + 1) % 3];
        let previous = corners[(at + 2) % 3];
        if at == 0 {
            visit(next);
            visit(previous);
        } else {
            visit(previous);
            visit(next);
        }
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (x * x + y * y + z * z).sqrt()
}

/// Fold a sweep's per-chunk partials, in chunk order.
fn merge(partials: Vec<f64>) -> f64 {
    partials.iter().sum()
}

fn diagonal_of(positions: &[f32], count: usize) -> f64 {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for vertex in 0..count {
        for axis in 0..3 {
            let value = positions[vertex * 3 + axis] as f64;
            low[axis] = low[axis].min(value);
            high[axis] = high[axis].max(value);
        }
    }
    let mut sum = 0.0;
    for axis in 0..3 {
        let extent = high[axis] - low[axis];
        sum += extent * extent;
    }
    sum.sqrt()
}

/// Area-weighted vertex normals over the proxy, unit length.
///
/// The reference was handed the old engines' own smoothed or creased normals;
/// there is no engine now, so they are computed here from the same surface.
/// Area weighting rather than plain averaging for the reason
/// `ModelData::generate_normals` uses it: the unnormalized cross product *is*
/// twice the area, so a sliver contributes in proportion to the surface it
/// actually covers.
pub(crate) fn vertex_normals(
    positions: &[f32],
    indices: &[u32],
    topology: &Topology,
    threads: usize,
) -> Vec<f32> {
    let count = topology.vertex_count;
    // One element per vertex, for the reason `build`'s smoothing gives: a sweep
    // over the flat float buffer misplaces every chunk whose start is not a
    // multiple of three. Flattened once at the end for the callers.
    let mut normals = vec![[0.0f32; 3]; count];
    let position_of = |vertex: u32| -> [f64; 3] {
        let base = vertex as usize * 3;
        [
            positions[base] as f64,
            positions[base + 1] as f64,
            positions[base + 2] as f64,
        ]
    };
    parallel::sweep(&mut normals, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let vertex = (base + offset) as u32;
            let mut sum = [0.0f64; 3];
            // Gathered from the vertex's own faces rather than scattered from
            // each face, so the sum is per vertex and the sweep needs no
            // synchronization.
            for &face in topology.faces_of(vertex) {
                let corners = &indices[face as usize * 3..face as usize * 3 + 3];
                let cross = geom::triangle_cross(
                    position_of(corners[0]),
                    position_of(corners[1]),
                    position_of(corners[2]),
                );
                sum = geom::add(sum, cross);
            }
            // A vertex whose faces cancel, or has none, keeps a unit normal
            // rather than a zero one: every use below divides by its length.
            let unit = geom::normalized(sum).unwrap_or([0.0, 1.0, 0.0]);
            *slot = unit.map(|value| value as f32);
        }
    });
    normals.into_iter().flatten().collect()
}

/// The dual area of each vertex: a third of each incident face's, which is what
/// makes the budget follow surface rather than follow triangle count.
pub(crate) fn dual_areas(
    positions: &[f32],
    indices: &[u32],
    topology: &Topology,
    threads: usize,
) -> Vec<f32> {
    let count = topology.vertex_count;
    let mut areas = vec![0.0f32; count];
    let position_of = |vertex: u32| -> [f64; 3] {
        let base = vertex as usize * 3;
        [
            positions[base] as f64,
            positions[base + 1] as f64,
            positions[base + 2] as f64,
        ]
    };
    parallel::sweep(&mut areas, threads, |base, chunk| {
        for (offset, slot) in chunk.iter_mut().enumerate() {
            let vertex = (base + offset) as u32;
            let mut total = 0.0f64;
            for &face in topology.faces_of(vertex) {
                let corners = &indices[face as usize * 3..face as usize * 3 + 3];
                total += geom::triangle_area(
                    position_of(corners[0]),
                    position_of(corners[1]),
                    position_of(corners[2]),
                );
            }
            *slot = (total / 3.0) as f32;
        }
    });
    areas
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid in the XY plane, `side` vertices on a side, one unit across.
    fn flat_grid(side: u32) -> (Vec<f32>, Vec<u32>) {
        let step = 1.0 / (side - 1) as f32;
        let mut positions = Vec::new();
        for row in 0..side {
            for column in 0..side {
                positions.extend_from_slice(&[column as f32 * step, row as f32 * step, 0.0]);
            }
        }
        let mut indices = Vec::new();
        for row in 0..side - 1 {
            for column in 0..side - 1 {
                let at = row * side + column;
                indices.extend_from_slice(&[at, at + 1, at + side]);
                indices.extend_from_slice(&[at + 1, at + side + 1, at + side]);
            }
        }
        (positions, indices)
    }

    /// A cone of `around` x `along` vertices, open at both ends, whose radius
    /// runs from `wide` down to `narrow`.
    ///
    /// The fixture of choice here because its curvature *varies*: a surface
    /// turns through `1/radius` around its girth, so the narrow end curves four
    /// times as hard as the wide one and the field has something to find. A
    /// plain tube is uniform, and correctly produces no field at all.
    fn cone(around: u32, along: u32, wide: f32, narrow: f32) -> (Vec<f32>, Vec<u32>) {
        graded_cone(around, along, wide, narrow, 1.0)
    }

    /// [`cone`], with its rings bunched toward the wide end when `grade` is
    /// above 1 — so the narrow, tightly curved end is also the *coarsely*
    /// tessellated one.
    ///
    /// That combination is what the support floor exists for and what a
    /// uniformly tessellated fixture cannot produce: on a plain cone the edge
    /// lengths shrink alongside the curvature, so the floor and the field move
    /// together and the floor never binds.
    fn graded_cone(
        around: u32,
        along: u32,
        wide: f32,
        narrow: f32,
        grade: f32,
    ) -> (Vec<f32>, Vec<u32>) {
        let mut positions = Vec::new();
        for step in 0..along {
            let t = (step as f32 / (along - 1) as f32).powf(grade);
            let radius = wide + (narrow - wide) * t;
            for turn in 0..around {
                let angle = turn as f32 / around as f32 * std::f32::consts::TAU;
                positions.extend_from_slice(&[radius * angle.cos(), radius * angle.sin(), t * 2.0]);
            }
        }
        let mut indices = Vec::new();
        for step in 0..along - 1 {
            for turn in 0..around {
                let next_turn = (turn + 1) % around;
                let a = step * around + turn;
                let b = step * around + next_turn;
                let c = (step + 1) * around + turn;
                let d = (step + 1) * around + next_turn;
                indices.extend_from_slice(&[a, b, c]);
                indices.extend_from_slice(&[b, d, c]);
            }
        }
        (positions, indices)
    }

    fn field_of(
        positions: &[f32],
        indices: &[u32],
        strength: f32,
        target_edge: f32,
        threads: usize,
    ) -> Option<Vec<f32>> {
        let count = positions.len() / 3;
        let topology = Topology::build(indices, count, threads, None);
        let normals = vertex_normals(positions, indices, &topology, threads);
        let areas = dual_areas(positions, indices, &topology, threads);
        build(
            &FieldInput {
                positions,
                normals: &normals,
                areas: Some(&areas),
                indices,
                topology: &topology,
            },
            strength,
            target_edge,
            threads,
            None,
        )
    }

    #[test]
    fn a_flat_surface_asks_for_one_size_everywhere() {
        let (positions, indices) = flat_grid(24);

        let field = field_of(&positions, &indices, 1.0, 0.1, 1);

        assert!(
            field.is_none(),
            "a plane has no curvature to vary the size by, so there is no field to build"
        );
    }

    #[test]
    fn strength_zero_builds_no_field_at_all() {
        let (positions, indices) = cone(32, 12, 0.6, 0.15);

        assert!(
            field_of(&positions, &indices, 0.0, 0.1, 1).is_none(),
            "at zero the caller takes its uniform path rather than a field of ones"
        );
    }

    /// The budget rule: a face covers `(target_edge * field)^2`, so holding the
    /// count means holding the area-weighted mean of `1 / field^2` at one.
    #[test]
    fn the_field_holds_the_face_count_it_was_given() {
        let (positions, indices) = cone(48, 32, 0.6, 0.15);
        let count = positions.len() / 3;
        let topology = Topology::build(&indices, count, 1, None);
        let areas = dual_areas(&positions, &indices, &topology, 1);

        let field = field_of(&positions, &indices, 1.0, 0.08, 1).expect("a cone varies");

        let mut density = 0.0f64;
        let mut total = 0.0f64;
        for vertex in 0..count {
            let weight = areas[vertex] as f64;
            total += weight;
            density += weight / (field[vertex] as f64).powi(2);
        }
        let mean = density / total;
        assert!(
            (mean - 1.0).abs() < 1.0e-3,
            "the budget is redistributed, not raised: mean 1/field^2 was {mean}"
        );
    }

    /// The whole point of the operation: the budget moves to where the shape
    /// turns. On a cone the narrow end curves hardest, so it must be asked for
    /// the smaller faces.
    #[test]
    fn the_tightly_curved_end_is_asked_for_smaller_faces() {
        let (around, along) = (48u32, 32u32);
        let (positions, indices) = cone(around, along, 0.6, 0.15);

        let field = field_of(&positions, &indices, 1.0, 0.08, 1).expect("a cone varies");

        let ring_mean = |step: u32| -> f64 {
            let start = (step * around) as usize;
            let slice = &field[start..start + around as usize];
            slice.iter().map(|&value| value as f64).sum::<f64>() / around as f64
        };
        let wide = ring_mean(1);
        let narrow = ring_mean(along - 2);
        assert!(
            narrow < wide * 0.75,
            "the narrow end should ask for markedly smaller faces: {narrow} against {wide}"
        );
    }

    /// The average incident edge length per vertex, which the support floor is
    /// expressed against.
    fn mean_edges(positions: &[f32], indices: &[u32], topology: &Topology) -> Vec<f64> {
        (0..topology.vertex_count as u32)
            .map(|vertex| {
                let base = vertex as usize * 3;
                let here = [
                    positions[base] as f64,
                    positions[base + 1] as f64,
                    positions[base + 2] as f64,
                ];
                let mut span = 0.0;
                let mut degree = 0u32;
                for_each_neighbour(topology, indices, vertex, |other| {
                    let base = other as usize * 3;
                    let there = [
                        positions[base] as f64,
                        positions[base + 1] as f64,
                        positions[base + 2] as f64,
                    ];
                    let length = distance(here, there);
                    if length > 0.0 {
                        span += length;
                        degree += 1;
                    }
                });
                if degree > 0 {
                    span / degree as f64
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// Where the floor test puts its target edge, as a multiple of the mesh's
    /// own mean edge. Above the support ratio so the budget can afford the
    /// floor, and close enough to it that the floor still binds somewhere.
    const PROBE_MULTIPLIER: f64 = 1.5;

    /// No face finer than the triangles under it: the rebuild cannot place a
    /// vertex where the input has no surface, so a field that asked for one
    /// would only lose the faces the flat side gave up.
    #[test]
    fn the_floor_keeps_a_face_from_being_finer_than_its_triangles() {
        let (positions, indices) = graded_cone(64, 24, 0.9, 0.08, 2.5);
        let count = positions.len() / 3;
        let topology = Topology::build(&indices, count, 1, None);
        let edges = mean_edges(&positions, &indices, &topology);

        // Coarse enough that the budget can afford the floor everywhere, which
        // is the regime the floor is written for — see the test below for what
        // happens when it cannot.
        let mean_edge = edges.iter().sum::<f64>() / count as f64;
        let target_edge = (mean_edge * PROBE_MULTIPLIER) as f32;

        let field = field_of(&positions, &indices, 1.0, target_edge, 1).expect("a cone varies");

        // Not exactly the floor: the normalization alternates clamping the
        // field up to it and scaling the whole field back to the budget, and it
        // stops after a fixed number of passes rather than on agreement - so
        // when the two are in tension the last pass leaves the field a fraction
        // of a percent under. What would fail here is a port slip, which moves
        // the field by orders of magnitude, not by a rounding of the budget.
        const RESIDUAL: f64 = 0.99;
        let mut clamped = 0;
        for vertex in 0..count {
            let floor = edges[vertex] * SUPPORT / target_edge as f64;
            assert!(
                field[vertex] as f64 >= floor * RESIDUAL,
                "vertex {vertex} was asked for {} against a floor of {floor}",
                field[vertex]
            );
            if (field[vertex] as f64) <= floor * 1.01 {
                clamped += 1;
            }
        }
        assert!(
            clamped > 0,
            "the floor never bit, so this proves nothing - pick a finer target"
        );
    }

    /// The floor and the budget can contradict each other, and when they do the
    /// budget wins.
    ///
    /// Asking for faces far finer than the input's own triangles puts every
    /// vertex on its floor, which is a count the surface cannot deliver; the
    /// normalization then scales the whole field back down to the budget it was
    /// given. Worth pinning because it looks like the floor failing, and it is
    /// really the two rules meeting head on.
    #[test]
    fn a_target_finer_than_the_mesh_itself_falls_back_to_the_budget() {
        let (positions, indices) = cone(64, 24, 0.6, 0.15);

        let field = field_of(&positions, &indices, 1.0, 0.001, 1).expect("a cone varies");

        let high = field.iter().cloned().fold(0.0f32, f32::max);
        assert!(
            high < 10.0,
            "the field stays near its budget rather than climbing to an impossible floor, got {high}"
        );
    }

    #[test]
    fn the_field_is_the_same_at_every_thread_count() {
        let (positions, indices) = cone(64, 48, 0.6, 0.15);

        let one = field_of(&positions, &indices, 1.0, 0.05, 1).expect("a cone varies");
        let many = field_of(&positions, &indices, 1.0, 0.05, 8).expect("a cone varies");

        assert_eq!(one.len(), many.len());
        for (vertex, (a, b)) in one.iter().zip(&many).enumerate() {
            assert!(
                (a - b).abs() <= a.abs() * 1.0e-6,
                "vertex {vertex}: {a} at one thread, {b} at eight"
            );
        }
    }

    /// Enough vertices that a sweep over their floats spans three chunks (past
    /// vertex 43690), so both misaligned offsets (`base % 3` of 1 and 2) occur -
    /// every chunk boundary a float-wise sweep could misplace is crossed.
    const PAST_ONE_CHUNK: u32 = 220;

    /// The area-weighted normal of each vertex, one vertex at a time, with no
    /// sweep: the reference the parallel version has to reproduce exactly.
    fn serial_normals(positions: &[f32], indices: &[u32], topology: &Topology) -> Vec<f32> {
        let position_of = |vertex: u32| {
            let at = vertex as usize * 3;
            [
                positions[at] as f64,
                positions[at + 1] as f64,
                positions[at + 2] as f64,
            ]
        };
        let mut out = Vec::with_capacity(topology.vertex_count * 3);
        for vertex in 0..topology.vertex_count as u32 {
            let mut sum = [0.0f64; 3];
            for &face in topology.faces_of(vertex) {
                let corners = &indices[face as usize * 3..face as usize * 3 + 3];
                let (a, b, c) = (
                    position_of(corners[0]),
                    position_of(corners[1]),
                    position_of(corners[2]),
                );
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                sum[0] += u[1] * v[2] - u[2] * v[1];
                sum[1] += u[2] * v[0] - u[0] * v[2];
                sum[2] += u[0] * v[1] - u[1] * v[0];
            }
            let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
            let unit = if length > 0.0 {
                [sum[0] / length, sum[1] / length, sum[2] / length]
            } else {
                [0.0, 1.0, 0.0]
            };
            out.extend(unit.map(|value| value as f32));
        }
        out
    }

    /// The normals past the first sweep chunk land on their own vertex. The sweep
    /// used to run over the flat float buffer, and `CHUNK` is not a multiple of
    /// three, so two chunks in three wrote each vertex's normal one or two floats
    /// off - identically at any thread count, which is why the determinism test
    /// never saw it.
    #[test]
    fn normals_past_the_first_chunk_belong_to_their_own_vertex() {
        let (positions, indices) = cone(PAST_ONE_CHUNK, PAST_ONE_CHUNK, 0.6, 0.15);
        let count = positions.len() / 3;
        assert!(
            count * 3 > 2 * parallel::CHUNK,
            "the fixture crosses two chunks"
        );
        let topology = Topology::build(&indices, count, 1, None);
        let reference = serial_normals(&positions, &indices, &topology);

        for threads in [1, 8] {
            let normals = vertex_normals(&positions, &indices, &topology, threads);
            assert_eq!(normals.len(), reference.len());
            if let Some(slot) = (0..normals.len()).find(|&slot| normals[slot] != reference[slot]) {
                panic!(
                    "vertex {} axis {} at {threads} threads: {} against {}",
                    slot / 3,
                    slot % 3,
                    normals[slot],
                    reference[slot]
                );
            }
        }
    }

    /// A plane has no curvature anywhere, however many vertices it has. With the
    /// normals scrambled past the first chunk it read as creased all over.
    #[test]
    fn a_large_flat_surface_still_asks_for_one_size_everywhere() {
        let (positions, indices) = flat_grid(PAST_ONE_CHUNK);
        assert!(positions.len() > 2 * parallel::CHUNK);

        assert!(field_of(&positions, &indices, 1.0, 0.02, 8).is_none());
    }

    /// Bit for bit, across more than one chunk of vertices: the claim the fixed
    /// chunk exists to keep, on a fixture large enough for the parallel path to
    /// actually split the work.
    #[test]
    fn the_field_is_bit_identical_at_every_thread_count_past_one_chunk() {
        let (positions, indices) = cone(300, 240, 0.6, 0.15);
        assert!(positions.len() / 3 > parallel::CHUNK);

        let one = field_of(&positions, &indices, 1.0, 0.02, 1).expect("a cone varies");
        let many = field_of(&positions, &indices, 1.0, 0.02, 8).expect("a cone varies");

        assert!(one == many, "the field changed with the thread count");
    }

    #[test]
    fn an_empty_mesh_builds_nothing() {
        let topology = Topology::build(&[], 0, 1, None);
        let input = FieldInput {
            positions: &[],
            normals: &[],
            areas: None,
            indices: &[],
            topology: &topology,
        };
        assert!(build(&input, 1.0, 0.1, 1, None).is_none());
    }
}
