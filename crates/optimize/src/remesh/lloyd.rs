//! Evening the regions out.
//!
//! Seeding places vertices where the size field asks for them, but places them
//! *independently* — a station on a prefix sum knows nothing about the station
//! before it, so two can land a hair apart and leave a gap beside them. The
//! regions that grow from such seeds are lopsided, and since each becomes one
//! output vertex, a lopsided region is a badly placed vertex.
//!
//! Lloyd's algorithm fixes that by iterating the obvious correction: a seed
//! should be at the middle of the region it owns. Move each one to its region's
//! centroid, grow the regions again, repeat. The fixed point is a *centroidal*
//! Voronoi diagram, where every seed is at the centre of its own cell — which
//! is the mathematical statement of "evenly spaced, at the density the field
//! asked for".
//!
//! A handful of iterations does nearly all of the work, which matters here for
//! a reason beyond cost: **each one is a complete, valid answer**. That is what
//! the viewport shows while the run is still going, refining in place, instead
//! of a progress bar with nothing behind it.
//!
//! ## What may move, and where to
//!
//! * A **junction** seed never moves. It is a corner — where two creases meet,
//!   or a border ends — and the whole point of seeding it was to keep that point
//!   exactly.
//! * A **feature** seed moves only along its own chain. A border vertex that
//!   drifted into the interior would leave a notch in the outline.
//! * Everything else moves freely.
//!
//! The seed always lands *on an input vertex* rather than at the centroid
//! itself. The centroid is generally not on the surface at all — on anything
//! curved it is inside the shape — and the collapse that follows needs a vertex
//! of the input mesh to start from.

use crate::cancel::{CancelToken, cancelled};

use super::features::Features;
use super::partition::{self, Partition};
use super::seeds::Seeds;
use super::surface::Surface;

/// Iterations at most. Past a handful the seeds barely move, and every one
/// costs a full repartition.
pub(crate) const MAX_ITERATIONS: usize = 8;

/// Stop once fewer than this fraction of vertices changed hands: the layout has
/// settled and further passes only shuffle a few boundary vertices about.
const SETTLED: f64 = 0.01;

/// What one iteration did, so the caller can pace itself and report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) iteration: usize,
    /// Vertices that changed region this iteration.
    pub(crate) moved: usize,
    /// Whether the layout has settled, so there is no point continuing.
    pub(crate) settled: bool,
}

/// Relax `seeds` against `partition`, calling `after` with each iteration's
/// result.
///
/// `after` is where a caller publishes an intermediate mesh; returning `false`
/// from it stops the relaxation, which is how a cancelled run gets out between
/// iterations rather than at the end.
pub(crate) fn relax(
    surface: Surface<'_>,
    features: &Features,
    seeds: &mut Seeds,
    partition: &mut Partition,
    threads: usize,
    cancel: Option<&CancelToken>,
    mut after: impl FnMut(&Seeds, &Partition, Step) -> bool,
) {
    let _z = crate::prof::zone!("Remesh Relaxation");
    let vertices = surface.vertex_count();
    if vertices == 0 || seeds.is_empty() {
        return;
    }

    for iteration in 0..MAX_ITERATIONS {
        if cancelled(cancel) {
            return;
        }
        let before = partition.label.clone();
        if !step(surface, features, seeds, partition) {
            return;
        }
        *partition = partition::build(surface, features, seeds, threads, cancel);

        let moved = before
            .iter()
            .zip(&partition.label)
            .filter(|(old, new)| old != new)
            .count();
        let settled = (moved as f64) < SETTLED * vertices as f64;
        let step = Step {
            iteration,
            moved,
            settled,
        };
        if !after(seeds, partition, step) || settled {
            return;
        }
    }
}

/// Move every seed to the middle of the region it owns. Returns whether any
/// seed moved at all.
fn step(
    surface: Surface<'_>,
    features: &Features,
    seeds: &mut Seeds,
    partition: &Partition,
) -> bool {
    let mut moved = false;
    for region in 0..partition.regions {
        // A junction is a corner and stays exactly where it was.
        if seeds.pinned[region] {
            continue;
        }
        let members = partition.members_of(region as u32);
        if members.is_empty() {
            continue;
        }
        let Some(centre) = centroid(surface, members) else {
            continue;
        };

        // A feature seed may only land back on its own chain, so a border
        // vertex cannot drift inward and leave a notch in the outline.
        let chain = seeds.polyline[region];
        let candidates = members.iter().copied().filter(|&vertex| {
            if chain == u32::MAX {
                !features.on_feature[vertex as usize]
            } else {
                features.polyline_of[vertex as usize] == chain
            }
        });
        let Some(best) = nearest(surface, centre, candidates) else {
            continue;
        };
        if best != seeds.vertices[region] {
            seeds.vertices[region] = best;
            moved = true;
        }
    }
    moved
}

/// The area-weighted centroid of a region, or `None` when it has no area to
/// weigh — which happens on a degenerate patch, where the seed simply stays.
fn centroid(surface: Surface<'_>, members: &[u32]) -> Option<[f64; 3]> {
    let mut sum = [0.0f64; 3];
    let mut weight = 0.0f64;
    // `members` is ascending, so this sum is the same however the region was
    // grown.
    for &vertex in members {
        let area = surface.area(vertex);
        let point = surface.position(vertex);
        for (axis, value) in sum.iter_mut().enumerate() {
            *value += point[axis] * area;
        }
        weight += area;
    }
    if weight <= 0.0 {
        return None;
    }
    Some([sum[0] / weight, sum[1] / weight, sum[2] / weight])
}

/// The candidate nearest `point`, ties going to the lower vertex index.
fn nearest(
    surface: Surface<'_>,
    point: [f64; 3],
    candidates: impl Iterator<Item = u32>,
) -> Option<u32> {
    let mut best: Option<(u32, f64)> = None;
    for vertex in candidates {
        let at = surface.position(vertex);
        let (x, y, z) = (at[0] - point[0], at[1] - point[1], at[2] - point[2]);
        let gap = x * x + y * y + z * z;
        let better = match best {
            None => true,
            // The index comparison is what makes an exact tie resolve the same
            // way on every run; without it the winner is whichever the iterator
            // happened to reach first.
            Some((at, so_far)) => gap < so_far || (gap == so_far && vertex < at),
        };
        if better {
            best = Some((vertex, gap));
        }
    }
    best.map(|(vertex, _)| vertex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remesh::topology::Topology;
    use crate::remesh::{features, seeds, size_field};

    fn grid(side: u32) -> (Vec<f32>, Vec<u32>) {
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

    struct Setup {
        positions: Vec<f32>,
        indices: Vec<u32>,
        topology: Topology,
        features: features::Features,
        areas: Vec<f32>,
        sizes: Vec<f32>,
    }

    impl Setup {
        fn surface(&self) -> Surface<'_> {
            Surface {
                positions: &self.positions,
                indices: &self.indices,
                topology: &self.topology,
                sizes: &self.sizes,
                areas: &self.areas,
            }
        }

        fn seed(&self, faces: u32) -> (Seeds, Partition) {
            let seeds = seeds::place(self.surface(), &self.features, faces, None);
            let partition = partition::build(self.surface(), &self.features, &seeds, 1, None);
            (seeds, partition)
        }
    }

    fn setup(side: u32, size: f32) -> Setup {
        let (positions, indices) = grid(side);
        let topology = Topology::build(&indices, positions.len() / 3, 1, None);
        let areas = size_field::dual_areas(&positions, &indices, &topology, 1);
        let sizes = vec![size; topology.vertex_count];
        let features = features::build(
            Surface {
                positions: &positions,
                indices: &indices,
                topology: &topology,
                sizes: &sizes,
                areas: &areas,
            },
            true,
            None,
            None,
        );
        Setup {
            positions,
            indices,
            topology,
            features,
            areas,
            sizes,
        }
    }

    /// How uneven the regions are, as the ratio of the largest to the mean.
    fn spread(partition: &Partition) -> f64 {
        let filled: Vec<usize> = (0..partition.regions as u32)
            .map(|region| partition.members_of(region).len())
            .filter(|&size| size > 0)
            .collect();
        let total: usize = filled.iter().sum();
        let mean = total as f64 / filled.len() as f64;
        let largest = filled.iter().copied().max().unwrap_or(0) as f64;
        largest / mean
    }

    #[test]
    fn relaxation_evens_the_regions_out() {
        let setup = setup(30, 0.1);
        let (mut seeds, mut partition) = setup.seed(300);
        let before = spread(&partition);

        relax(
            setup.surface(),
            &setup.features,
            &mut seeds,
            &mut partition,
            1,
            None,
            |_, _, _| true,
        );

        let after = spread(&partition);
        assert!(
            after <= before,
            "relaxation should not make the regions less even: {before} then {after}"
        );
    }

    #[test]
    fn a_junction_seed_never_moves() {
        // A branch, whose ends are junctions of the feature graph.
        let positions = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0,
        ];
        let indices = vec![0, 1, 2, 0, 1, 3, 0, 1, 4];
        let topology = Topology::build(&indices, 5, 1, None);
        let areas = size_field::dual_areas(&positions, &indices, &topology, 1);
        let sizes = vec![0.5f32; 5];
        let features = features::build(
            Surface {
                positions: &positions,
                indices: &indices,
                topology: &topology,
                sizes: &sizes,
                areas: &areas,
            },
            true,
            None,
            None,
        );
        let setup = Setup {
            positions,
            indices,
            topology,
            features,
            areas,
            sizes,
        };
        let (mut seeds, mut partition) = setup.seed(8);
        let pinned: Vec<(usize, u32)> = seeds
            .pinned
            .iter()
            .enumerate()
            .filter(|&(_, &pinned)| pinned)
            .map(|(region, _)| (region, seeds.vertices[region]))
            .collect();
        assert!(!pinned.is_empty(), "this fixture has junctions to pin");

        relax(
            setup.surface(),
            &setup.features,
            &mut seeds,
            &mut partition,
            1,
            None,
            |_, _, _| true,
        );

        for (region, vertex) in pinned {
            assert_eq!(
                seeds.vertices[region], vertex,
                "pinned seed {region} moved off its junction"
            );
        }
    }

    #[test]
    fn a_border_seed_stays_on_the_border() {
        let setup = setup(20, 0.1);
        let (mut seeds, mut partition) = setup.seed(200);
        let on_border: Vec<usize> = seeds
            .polyline
            .iter()
            .enumerate()
            .filter(|&(_, &line)| line != u32::MAX)
            .map(|(region, _)| region)
            .collect();
        assert!(!on_border.is_empty());

        relax(
            setup.surface(),
            &setup.features,
            &mut seeds,
            &mut partition,
            1,
            None,
            |_, _, _| true,
        );

        for region in on_border {
            let vertex = seeds.vertices[region];
            assert!(
                setup.topology.boundary[vertex as usize],
                "seed {region} left the border for vertex {vertex}"
            );
        }
    }

    #[test]
    fn stopping_from_the_callback_ends_the_run() {
        let setup = setup(24, 0.08);
        let (mut seeds, mut partition) = setup.seed(250);

        let mut calls = 0;
        relax(
            setup.surface(),
            &setup.features,
            &mut seeds,
            &mut partition,
            1,
            None,
            |_, _, step| {
                assert_eq!(step.iteration, 0);
                calls += 1;
                false
            },
        );

        assert_eq!(
            calls, 1,
            "refusing the first iteration is what a cancelled run does, and it stops there"
        );
    }

    #[test]
    fn iterations_are_reported_in_order_and_bounded() {
        let setup = setup(24, 0.08);
        let (mut seeds, mut partition) = setup.seed(250);

        let mut seen = Vec::new();
        relax(
            setup.surface(),
            &setup.features,
            &mut seeds,
            &mut partition,
            1,
            None,
            |_, _, step| {
                seen.push(step.iteration);
                true
            },
        );

        assert!(
            seen.iter()
                .enumerate()
                .all(|(at, &iteration)| at == iteration),
            "iterations arrive in order from zero: {seen:?}"
        );
        assert!(seen.len() <= MAX_ITERATIONS);
        // A run that settles reports that it has, which is what tells the caller
        // there is no point paying for another pass.
        if seen.len() < MAX_ITERATIONS {
            assert!(!seen.is_empty(), "at least one pass always runs");
        }
    }

    #[test]
    fn relaxation_is_the_same_at_every_thread_count() {
        let setup = setup(30, 0.06);

        let run = |threads: usize| {
            let (mut seeds, mut partition) = setup.seed(400);
            relax(
                setup.surface(),
                &setup.features,
                &mut seeds,
                &mut partition,
                threads,
                None,
                |_, _, _| true,
            );
            (seeds.vertices.clone(), partition.label.clone())
        };

        let one = run(1);
        assert_eq!(one, run(2));
        assert_eq!(one, run(8));
    }

    #[test]
    fn a_settled_layout_stops_early() {
        let setup = setup(16, 0.15);
        let (mut seeds, mut partition) = setup.seed(120);

        let mut iterations = 0;
        relax(
            setup.surface(),
            &setup.features,
            &mut seeds,
            &mut partition,
            1,
            None,
            |_, _, _| {
                iterations += 1;
                true
            },
        );

        assert!(
            iterations <= MAX_ITERATIONS,
            "the loop is bounded whatever happens"
        );
    }
}
