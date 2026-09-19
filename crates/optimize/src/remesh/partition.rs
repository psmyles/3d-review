//! Which seed each input vertex belongs to.
//!
//! A multi-source shortest path over the vertex graph, with every seed at
//! distance zero: each vertex ends up labelled with the seed it can be reached
//! from most cheaply. The regions that come out are the Voronoi cells of the
//! seeds in the size field's own metric, and each one collapses to a single
//! output vertex later.
//!
//! ## The metric is the size field, not distance
//!
//! An edge costs `length / h`, where `h` is the face size asked for along it. So
//! a region is "one face wide" wherever it is, and the same seed count covers a
//! finely detailed area with small regions and a flat area with large ones —
//! which is how the budget moves to where the shape turns.
//!
//! ## Why Bellman-Ford and not Dijkstra
//!
//! Dijkstra is the faster algorithm and the wrong one here. It is inherently
//! sequential — a priority queue with a single frontier — and its answer depends
//! on the order equal-cost vertices leave the queue. What is wanted instead is a
//! *fixed point*: sweep every vertex, take the best offer from its neighbours,
//! repeat until nothing changes. That is
//!
//! * **parallel**, because a sweep reads the previous pass and writes the next,
//!   so vertices never see each other's half-finished work
//!   ([`parallel::sweep`]);
//! * **deterministic**, because the fixed point does not depend on visiting
//!   order — and where two seeds are exactly equidistant, the lower-numbered one
//!   wins by rule rather than by whichever arrived first;
//! * **interruptible**, because a sweep is a natural place to stop.
//!
//! It costs more passes than Dijkstra would, and buys an answer that is the same
//! on every machine.
//!
//! ## Walls
//!
//! Crossing a feature edge costs [`WALL`] times more. Without it a region
//! happily straddles a crease — taking half its vertices from each side of a
//! ninety-degree fold — and the single vertex it collapses to lands in mid-air
//! off the corner. It also keeps a region on one side of a thin strip rather
//! than wrapping it, which is what stops a leaf from being pinched shut.

use crate::cancel::{CancelToken, cancelled};
use crate::parallel;

use super::features::Features;
use super::seeds::Seeds;
use super::surface::Surface;

/// What crossing a feature edge is multiplied by. Large enough that a region
/// prefers to stop at a crease, small enough that a region may still cross one
/// when the alternative is leaving vertices unreached.
const WALL: f32 = 4.0;

/// A ceiling on the sweeps, so a pathological mesh cannot spin. Reaching it
/// leaves the partition slightly coarse rather than wrong: every vertex still
/// has the best label found so far, and the pass below fills any that have none.
const MAX_SWEEPS: usize = 512;

/// No seed reached this vertex.
pub(crate) const NO_REGION: u32 = u32::MAX;

/// The surface divided into one region per seed.
#[derive(Debug, Default)]
pub(crate) struct Partition {
    /// Per vertex: the seed it belongs to, or [`NO_REGION`].
    pub(crate) label: Vec<u32>,
    /// Per vertex: the cost of reaching it from that seed.
    pub(crate) distance: Vec<f32>,
    /// Region-major CSR start offsets, `regions + 1` long.
    starts: Vec<u32>,
    /// Vertices of each region, ascending within a region.
    members: Vec<u32>,
    pub(crate) regions: usize,
}

impl Partition {
    /// The vertices of one region, ascending.
    ///
    /// Ascending matters: the centroid of a region is summed over this slice,
    /// and a float sum in a different order is a different number.
    pub(crate) fn members_of(&self, region: u32) -> &[u32] {
        let start = self.starts[region as usize] as usize;
        let end = self.starts[region as usize + 1] as usize;
        &self.members[start..end]
    }

    /// Regions that reached no vertex at all. A seed whose neighbours all went
    /// to someone else is one of these, and it simply produces no output vertex.
    pub(crate) fn empty_regions(&self) -> usize {
        (0..self.regions as u32)
            .filter(|&region| self.members_of(region).is_empty())
            .count()
    }
}

/// One vertex's standing claim: which seed holds it, and at what cost.
///
/// The two travel together because a sweep decides them together — evaluating
/// the neighbourhood once and writing a pair is half the work of evaluating it
/// once per output array.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Claim {
    label: u32,
    distance: f32,
}

impl Claim {
    const UNCLAIMED: Claim = Claim {
        label: NO_REGION,
        distance: f32::INFINITY,
    };
}

/// Grow every seed outward until the whole surface is claimed.
pub(crate) fn build(
    surface: Surface<'_>,
    features: &Features,
    seeds: &Seeds,
    threads: usize,
    cancel: Option<&CancelToken>,
) -> Partition {
    let _z = crate::prof::zone!("Remesh Partition");
    let count = surface.vertex_count();
    let mut partition = Partition {
        label: vec![NO_REGION; count],
        distance: vec![f32::INFINITY; count],
        starts: Vec::new(),
        members: Vec::new(),
        regions: seeds.len(),
    };
    if count == 0 || seeds.is_empty() {
        partition.starts = vec![0; partition.regions + 1];
        return partition;
    }

    let mut claims = vec![Claim::UNCLAIMED; count];
    for (region, &vertex) in seeds.vertices.iter().enumerate() {
        claims[vertex as usize] = Claim {
            label: region as u32,
            distance: 0.0,
        };
    }

    let mut next = claims.clone();
    for _ in 0..MAX_SWEEPS {
        parallel::sweep(&mut next, threads, |base, chunk| {
            for (offset, slot) in chunk.iter_mut().enumerate() {
                let vertex = (base + offset) as u32;
                *slot = best_offer(surface, features, &claims, vertex);
            }
        });
        // Compared in vertex order rather than reduced across the chunks: it is
        // a single pass over a flat array, and "did anything move" decides how
        // many more sweeps run, so it must not depend on the schedule either.
        let moved = next
            .iter()
            .zip(&claims)
            .any(|(fresh, old)| fresh.label != old.label);
        std::mem::swap(&mut claims, &mut next);
        if !moved || cancelled(cancel) {
            break;
        }
    }

    for (vertex, claim) in claims.iter().enumerate() {
        partition.label[vertex] = claim.label;
        partition.distance[vertex] = claim.distance;
    }

    // Anything the sweeps never reached — an isolated vertex, or a component
    // whose seeds were all consumed — takes its lowest labelled neighbour, so
    // no vertex is left out of the collapse.
    fill_unreached(surface, &mut partition);
    index_regions(&mut partition);
    partition
}

/// The cheapest way to reach `vertex`: its own standing claim, or a neighbour's
/// plus the edge between them.
fn best_offer(surface: Surface<'_>, features: &Features, claims: &[Claim], vertex: u32) -> Claim {
    let mut best = claims[vertex as usize];
    // A seed never gives up its own vertex.
    if best.distance == 0.0 {
        return best;
    }

    for &face in surface.topology.faces_of(vertex) {
        for &other in surface.face(face) {
            let their = claims[other as usize];
            if other == vertex || their.label == NO_REGION {
                continue;
            }
            let candidate = their.distance + edge_cost(surface, features, vertex, other);
            // Strictly closer wins; exactly as close is settled on the lower
            // region number, which is what makes an equidistant vertex land in
            // the same place on every machine.
            let closer = candidate < best.distance;
            let tie = candidate == best.distance && their.label < best.label;
            if closer || tie {
                best = Claim {
                    label: their.label,
                    distance: candidate,
                };
            }
        }
    }
    best
}

/// What one edge costs to cross: its length in units of the face size asked for
/// along it, times [`WALL`] if it is a feature.
fn edge_cost(surface: Surface<'_>, features: &Features, a: u32, b: u32) -> f32 {
    let length = surface.distance(a, b) as f32;
    let size = (surface.sizes[a as usize] + surface.sizes[b as usize]) * 0.5;
    let base = if size > 0.0 { length / size } else { length };
    // Both ends on a feature means the edge runs *along* one, which is free; one
    // end means it crosses off it, which is what the wall is for.
    let crossing = features.on_feature[a as usize] != features.on_feature[b as usize];
    if crossing { base * WALL } else { base }
}

/// Give every unreached vertex its lowest labelled neighbour.
fn fill_unreached(surface: Surface<'_>, partition: &mut Partition) {
    let topology = surface.topology;
    let mut pending: Vec<u32> = (0..topology.vertex_count as u32)
        .filter(|&vertex| {
            partition.label[vertex as usize] == NO_REGION && topology.is_referenced(vertex)
        })
        .collect();
    // Bounded by the number of unreached vertices: each round either labels at
    // least one or there is nothing more to reach.
    while !pending.is_empty() {
        let mut labelled = false;
        let mut still: Vec<u32> = Vec::new();
        for &vertex in &pending {
            let mut best = NO_REGION;
            for &face in topology.faces_of(vertex) {
                for &other in surface.face(face) {
                    let label = partition.label[other as usize];
                    if other != vertex && label != NO_REGION {
                        best = best.min(label);
                    }
                }
            }
            if best == NO_REGION {
                still.push(vertex);
            } else {
                partition.label[vertex as usize] = best;
                labelled = true;
            }
        }
        if !labelled {
            break;
        }
        pending = still;
    }
}

/// Counting sort of the vertices by region, which gives each region's members
/// ascending for free.
fn index_regions(partition: &mut Partition) {
    let regions = partition.regions;
    partition.starts = vec![0u32; regions + 1];
    for &label in &partition.label {
        if label != NO_REGION {
            partition.starts[label as usize + 1] += 1;
        }
    }
    for region in 0..regions {
        partition.starts[region + 1] += partition.starts[region];
    }
    partition.members = vec![0u32; partition.starts[regions] as usize];
    let mut cursor = partition.starts[..regions].to_vec();
    for (vertex, &label) in partition.label.iter().enumerate() {
        if label != NO_REGION {
            let slot = &mut cursor[label as usize];
            partition.members[*slot as usize] = vertex as u32;
            *slot += 1;
        }
    }
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
        seeds: Seeds,
        sizes: Vec<f32>,
        areas: Vec<f32>,
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
    }

    fn setup(side: u32, size: f32, faces: u32) -> Setup {
        let (positions, indices) = grid(side);
        let topology = Topology::build(&indices, positions.len() / 3, 1, None);
        let sizes = vec![size; topology.vertex_count];
        let areas = size_field::dual_areas(&positions, &indices, &topology, 1);
        let surface = Surface {
            positions: &positions,
            indices: &indices,
            topology: &topology,
            sizes: &sizes,
            areas: &areas,
        };
        let features = features::build(surface, true, None, None);
        let seeds = seeds::place(surface, &features, faces, None);
        Setup {
            positions,
            indices,
            topology,
            features,
            seeds,
            sizes,
            areas,
        }
    }

    fn partition_of(setup: &Setup, threads: usize) -> Partition {
        build(
            setup.surface(),
            &setup.features,
            &setup.seeds,
            threads,
            None,
        )
    }

    #[test]
    fn every_vertex_ends_up_in_a_region() {
        let setup = setup(20, 0.1, 200);

        let partition = partition_of(&setup, 1);

        for vertex in 0..setup.topology.vertex_count as u32 {
            if setup.topology.is_referenced(vertex) {
                assert_ne!(
                    partition.label[vertex as usize], NO_REGION,
                    "vertex {vertex} was never claimed"
                );
            }
        }
    }

    #[test]
    fn a_seed_keeps_its_own_vertex() {
        let setup = setup(20, 0.1, 200);

        let partition = partition_of(&setup, 1);

        for (region, &vertex) in setup.seeds.vertices.iter().enumerate() {
            assert_eq!(
                partition.label[vertex as usize], region as u32,
                "seed {region} lost its own vertex"
            );
            assert_eq!(partition.distance[vertex as usize], 0.0);
        }
    }

    #[test]
    fn the_regions_index_holds_every_labelled_vertex_ascending() {
        let setup = setup(16, 0.12, 150);

        let partition = partition_of(&setup, 1);

        let mut seen = 0;
        for region in 0..partition.regions as u32 {
            let members = partition.members_of(region);
            assert!(
                members.windows(2).all(|pair| pair[0] < pair[1]),
                "region {region} is not ascending"
            );
            for &vertex in members {
                assert_eq!(partition.label[vertex as usize], region);
            }
            seen += members.len();
        }
        let labelled = partition
            .label
            .iter()
            .filter(|&&label| label != NO_REGION)
            .count();
        assert_eq!(seen, labelled, "every labelled vertex is indexed once");
    }

    /// The property the whole design rests on.
    #[test]
    fn the_partition_is_the_same_at_every_thread_count() {
        let setup = setup(40, 0.05, 600);

        let one = partition_of(&setup, 1);
        let two = partition_of(&setup, 2);
        let many = partition_of(&setup, 8);

        assert_eq!(one.label, two.label);
        assert_eq!(one.label, many.label);
        assert_eq!(one.members, many.members);
        assert_eq!(one.starts, many.starts);
    }

    #[test]
    fn two_runs_agree() {
        let setup = setup(24, 0.08, 300);

        assert_eq!(partition_of(&setup, 4).label, partition_of(&setup, 4).label);
    }

    /// Regions should be about as big as the size field asked for, rather than
    /// one region swallowing the surface.
    #[test]
    fn the_regions_are_of_a_sensible_size() {
        let setup = setup(30, 0.08, 400);

        let partition = partition_of(&setup, 1);

        let filled: Vec<usize> = (0..partition.regions as u32)
            .map(|region| partition.members_of(region).len())
            .filter(|&size| size > 0)
            .collect();
        assert!(!filled.is_empty());
        let largest = filled.iter().copied().max().unwrap_or(0);
        let total: usize = filled.iter().sum();
        assert!(
            largest < total / 4,
            "one region took {largest} of {total} vertices"
        );
    }

    #[test]
    fn an_empty_seed_set_partitions_nothing() {
        let (positions, indices) = grid(4);
        let topology = Topology::build(&indices, positions.len() / 3, 1, None);
        let features = features::Features::default();
        let sizes = [0.1f32; 16];
        let areas = size_field::dual_areas(&positions, &indices, &topology, 1);
        let partition = build(
            Surface {
                positions: &positions,
                indices: &indices,
                topology: &topology,
                sizes: &sizes,
                areas: &areas,
            },
            &features,
            &Seeds::default(),
            1,
            None,
        );

        assert_eq!(partition.regions, 0);
        assert!(partition.label.iter().all(|&label| label == NO_REGION));
    }
}
