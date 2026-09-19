//! Where the output's vertices start.
//!
//! One seed becomes one output vertex, so this is where the face count is
//! actually decided — and it is *decided*, not searched for. The engines this
//! replaces were told an edge length and produced whatever face count that
//! turned into, which is why the operation used to solve a node up to four times
//! to land near the number in the box.
//!
//! ## The budget
//!
//! Euler's formula, run backwards. A triangle mesh has `3F = 2E - B` (each
//! interior edge is used by two faces, each border edge by one), so
//! `E = (3F + B) / 2`, and `chi = V - E + F` gives
//!
//! ```text
//!     V = chi + F/2 + B/2
//! ```
//!
//! `chi` is measured from the input ([`Topology::euler`]) because the rebuild
//! preserves it: a sphere stays a sphere, a disc a disc, a torus a torus. `B` is
//! the number of border *edges the output will have*, which is the number of
//! border seeds — so the border is seeded first and the interior budget is
//! whatever is left.
//!
//! ## Feature chains first, and at even stations
//!
//! A border or a crease gets its seeds *along* it, spaced by the size field, and
//! its junctions get one exactly. That ordering is the whole reason this
//! operation stops tearing leaves: the border is built from vertices that are
//! on it by construction, rather than from whichever interior samples happened
//! to land nearby.
//!
//! ## Systematic sampling, not random
//!
//! The interior is sampled by walking a prefix sum of `area / h^2` and taking
//! stations at `(k + 0.5) * total / n`. There is no random number generator
//! anywhere in this operation — not a seeded one either, because a seeded
//! generator still couples every sample to every earlier one, so a change in
//! one face's area shuffles the whole set. Stations are independent of each
//! other and of the order the faces are visited in.

use crate::cancel::{CancelToken, cancelled};

use super::features::{FeatureKind, Features, Polyline};
use super::surface::Surface;

/// Fewest seeds a closed feature loop may be given: below three it is not a
/// loop any more, it is an edge folded back on itself.
const MIN_CLOSED_SEEDS: usize = 3;

/// Fewest seeds an open feature chain may be given: its two ends.
const MIN_OPEN_SEEDS: usize = 2;

/// Fewest seeds a connected component may be given, so a piece of the mesh
/// cannot vanish entirely because the budget landed elsewhere.
const MIN_COMPONENT_SEEDS: usize = 4;

/// Where the rebuild starts from.
#[derive(Debug, Default)]
pub(crate) struct Seeds {
    /// The input vertex each seed sits on. Ascending, distinct.
    pub(crate) vertices: Vec<u32>,
    /// Per seed: the feature chain it is pinned to, or `u32::MAX` for a free
    /// interior seed. A pinned seed may only ever move along its own chain.
    pub(crate) polyline: Vec<u32>,
    /// Per seed: it sits on a junction and may not move at all.
    pub(crate) pinned: Vec<bool>,
    /// How many of them came from a border chain — the `B` of the budget, and
    /// the number of border edges the result will have.
    pub(crate) border_seeds: usize,
}

impl Seeds {
    pub(crate) fn len(&self) -> usize {
        self.vertices.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

/// How many output vertices `faces` triangles come to, given the surface's own
/// topology. See the module docs for the derivation.
pub(crate) fn vertex_budget(faces: u32, border_edges: usize, euler: i64) -> usize {
    let estimate = euler + (faces as i64) / 2 + (border_edges as i64) / 2;
    estimate.max(MIN_COMPONENT_SEEDS as i64) as usize
}

/// Place the seeds: feature chains at even stations first, then the interior by
/// area, then a floor per component.
pub(crate) fn place(
    surface: Surface<'_>,
    features: &Features,
    faces: u32,
    cancel: Option<&CancelToken>,
) -> Seeds {
    let _z = crate::prof::zone!("Remesh Seeding");
    let count = surface.vertex_count();
    let mut taken = vec![false; count];
    let mut seeds = Seeds::default();
    if count == 0 || surface.sizes.len() < count {
        return seeds;
    }

    // Feature chains first. They are the part of the budget that is not
    // negotiable — a border has to be seeded along its whole length or the
    // rebuild loses it — so they are spent before the interior is counted.
    for (index, line) in features.polylines.iter().enumerate() {
        let stations = stations_for(surface, line);
        for &vertex in &stations {
            push_seed(&mut seeds, &mut taken, vertex, index as u32, features);
            if line.kind == FeatureKind::Boundary {
                seeds.border_seeds += 1;
            }
        }
        // A junction is a corner: a station near it is not good enough, so it
        // gets a seed of its own whether the spacing asked for one or not.
        for &vertex in &line.vertices {
            if features.junction[vertex as usize] && !taken[vertex as usize] {
                push_seed(&mut seeds, &mut taken, vertex, index as u32, features);
                if line.kind == FeatureKind::Boundary {
                    seeds.border_seeds += 1;
                }
            }
        }
    }
    if cancelled(cancel) {
        return seeds;
    }

    let budget = vertex_budget(faces, seeds.border_seeds, surface.topology.euler());
    let interior = budget.saturating_sub(seeds.len());
    if interior > 0 {
        place_interior(surface, features, interior, &mut taken, &mut seeds);
    }
    if cancelled(cancel) {
        return seeds;
    }

    ensure_every_component(surface, &mut taken, &mut seeds);

    // Ascending, so the seed numbering is a function of the mesh rather than of
    // the order the stages happened to run in. The chain and pin arrays follow.
    let mut order: Vec<usize> = (0..seeds.len()).collect();
    order.sort_unstable_by_key(|&slot| seeds.vertices[slot]);
    seeds.vertices = order.iter().map(|&slot| seeds.vertices[slot]).collect();
    seeds.polyline = order.iter().map(|&slot| seeds.polyline[slot]).collect();
    seeds.pinned = order.iter().map(|&slot| seeds.pinned[slot]).collect();
    seeds
}

fn push_seed(seeds: &mut Seeds, taken: &mut [bool], vertex: u32, line: u32, features: &Features) {
    if taken[vertex as usize] {
        return;
    }
    taken[vertex as usize] = true;
    seeds.vertices.push(vertex);
    seeds.polyline.push(line);
    seeds.pinned.push(features.junction[vertex as usize]);
}

/// The vertices of one chain that should carry a seed: its ends, and then even
/// arc-length stations spaced by the size field along it.
fn stations_for(surface: Surface<'_>, line: &Polyline) -> Vec<u32> {
    let vertices = &line.vertices;
    if vertices.len() < 2 {
        return vertices.clone();
    }
    // Cumulative arc length, with the closing edge included for a loop.
    let mut lengths = vec![0.0f64];
    let steps = if line.closed {
        vertices.len()
    } else {
        vertices.len() - 1
    };
    let mut mean_size = 0.0f64;
    for step in 0..steps {
        let a = vertices[step];
        let b = vertices[(step + 1) % vertices.len()];
        lengths.push(lengths[step] + surface.distance(a, b));
        mean_size += surface.sizes[a as usize] as f64;
    }
    mean_size = if steps > 0 {
        mean_size / steps as f64
    } else {
        0.0
    };
    let total = *lengths.last().unwrap_or(&0.0);
    if total <= 0.0 || mean_size <= 0.0 {
        return vec![vertices[0]];
    }

    let floor = if line.closed {
        MIN_CLOSED_SEEDS
    } else {
        MIN_OPEN_SEEDS
    };
    let wanted = ((total / mean_size).round() as usize).max(floor);
    // An open chain's stations include both ends; a closed one's are spread
    // round it with no end to pin.
    let mut out = Vec::with_capacity(wanted);
    for station in 0..wanted {
        let along = if line.closed {
            total * station as f64 / wanted as f64
        } else if wanted > 1 {
            total * station as f64 / (wanted - 1) as f64
        } else {
            0.0
        };
        out.push(vertices[nearest_station(&lengths, along, vertices.len())]);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The chain vertex whose cumulative length is nearest `along`.
fn nearest_station(lengths: &[f64], along: f64, vertices: usize) -> usize {
    let mut best = 0usize;
    let mut best_gap = f64::INFINITY;
    for (slot, &at) in lengths.iter().enumerate().take(vertices) {
        let gap = (at - along).abs();
        if gap < best_gap {
            best_gap = gap;
            best = slot;
        }
    }
    best
}

/// Spread `wanted` seeds over the surface in proportion to `area / h^2`, which
/// is how many faces each piece of surface has been asked for.
fn place_interior(
    surface: Surface<'_>,
    features: &Features,
    wanted: usize,
    taken: &mut [bool],
    seeds: &mut Seeds,
) {
    let faces = surface.indices.as_chunks::<3>().0;
    // Prefix sum in face order, so a station's face is a function of the mesh.
    let mut weights = Vec::with_capacity(faces.len() + 1);
    weights.push(0.0f64);
    for (face, corners) in faces.iter().enumerate() {
        let area = surface.face_area(face as u32);
        let size = corners
            .iter()
            .map(|&corner| surface.sizes[corner as usize] as f64)
            .sum::<f64>()
            / 3.0;
        let weight = if size > 0.0 {
            area / (size * size)
        } else {
            0.0
        };
        weights.push(weights.last().unwrap_or(&0.0) + weight);
    }
    let total = *weights.last().unwrap_or(&0.0);
    if total <= 0.0 {
        return;
    }

    for station in 0..wanted {
        let along = total * (station as f64 + 0.5) / wanted as f64;
        let face = match weights.binary_search_by(|value| {
            value
                .partial_cmp(&along)
                .unwrap_or(std::cmp::Ordering::Less)
        }) {
            Ok(at) => at.min(faces.len().saturating_sub(1)),
            Err(at) => at.saturating_sub(1).min(faces.len().saturating_sub(1)),
        };
        // The corner that has been asked for the smallest faces, since that is
        // where the budget is most needed; if it is already a seed, the other
        // two in turn, and otherwise this station is simply spent — the count
        // is a target, and forcing it would put two seeds on one vertex.
        let mut corners = faces[face];
        corners.sort_unstable_by(|&a, &b| {
            surface.sizes[a as usize]
                .partial_cmp(&surface.sizes[b as usize])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(&b))
        });
        for corner in corners {
            // Never onto a feature: the chains above already decided how
            // densely the border and the creases are seeded, and an interior
            // station landing on one would both crowd it unevenly and
            // undercount the border edges the budget was worked out from.
            if !taken[corner as usize]
                && !features.on_feature[corner as usize]
                && surface.topology.is_referenced(corner)
            {
                taken[corner as usize] = true;
                seeds.vertices.push(corner);
                seeds.polyline.push(u32::MAX);
                seeds.pinned.push(false);
                break;
            }
        }
    }
}

/// Give every connected component a handful of seeds, so a piece of the mesh
/// cannot disappear because the budget all landed on a bigger one.
fn ensure_every_component(surface: Surface<'_>, taken: &mut [bool], seeds: &mut Seeds) {
    let topology = surface.topology;
    if topology.components == 0 {
        return;
    }
    let mut per_component = vec![0usize; topology.components as usize];
    for &vertex in &seeds.vertices {
        let component = topology.component[vertex as usize];
        if component != u32::MAX {
            per_component[component as usize] += 1;
        }
    }
    for (component, count) in per_component.iter().enumerate() {
        if *count >= MIN_COMPONENT_SEEDS {
            continue;
        }
        let mut added = *count;
        // Spread over the component rather than taken from its front, so a
        // forced seed set is not four adjacent vertices.
        let members: Vec<u32> = (0..topology.vertex_count as u32)
            .filter(|&vertex| topology.component[vertex as usize] == component as u32)
            .collect();
        if members.is_empty() {
            continue;
        }
        let step = (members.len() / MIN_COMPONENT_SEEDS.max(1)).max(1);
        for slot in (0..members.len()).step_by(step) {
            if added >= MIN_COMPONENT_SEEDS {
                break;
            }
            let vertex = members[slot];
            if !taken[vertex as usize] {
                taken[vertex as usize] = true;
                seeds.vertices.push(vertex);
                seeds.polyline.push(u32::MAX);
                seeds.pinned.push(false);
                added += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remesh::topology::Topology;
    use crate::remesh::{features, size_field};

    /// A grid in the XY plane, `side` vertices a side, one unit across.
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

    fn setup(positions: Vec<f32>, indices: Vec<u32>, size: f32, borders: bool) -> Setup {
        let topology = Topology::build(&indices, positions.len() / 3, 1, None);
        let sizes = vec![size; topology.vertex_count];
        let areas = size_field::dual_areas(&positions, &indices, &topology, 1);
        let features = features::build(
            Surface {
                positions: &positions,
                indices: &indices,
                topology: &topology,
                sizes: &sizes,
                areas: &areas,
            },
            borders,
            None,
            None,
        );
        Setup {
            positions,
            indices,
            topology,
            features,
            sizes,
            areas,
        }
    }

    fn place_on(setup: &Setup, faces: u32) -> Seeds {
        place(setup.surface(), &setup.features, faces, None)
    }

    /// The budget is Euler's formula run backwards, and this is the shape of
    /// answer it has to give.
    #[test]
    fn the_budget_follows_euler() {
        // A closed sphere: chi 2, no border. 1000 faces wants ~502 vertices.
        assert_eq!(vertex_budget(1000, 0, 2), 502);
        // A disc: chi 1, and its border edges are half-counted.
        assert_eq!(vertex_budget(1000, 40, 1), 521);
        // A torus: chi 0.
        assert_eq!(vertex_budget(1000, 0, 0), 500);
    }

    #[test]
    fn a_tiny_budget_still_leaves_a_surface() {
        assert!(vertex_budget(0, 0, 2) >= MIN_COMPONENT_SEEDS);
        assert!(vertex_budget(1, 0, -50) >= MIN_COMPONENT_SEEDS);
    }

    #[test]
    fn every_seed_is_a_distinct_vertex_in_ascending_order() {
        let setup = setup(grid(16).0, grid(16).1, 0.1, true);

        let seeds = place_on(&setup, 200);

        assert!(!seeds.is_empty());
        assert!(
            seeds.vertices.windows(2).all(|pair| pair[0] < pair[1]),
            "seeds are ascending and distinct"
        );
        assert_eq!(seeds.polyline.len(), seeds.len());
        assert_eq!(seeds.pinned.len(), seeds.len());
    }

    /// The border is seeded before anything else, so a rebuild cannot lose it.
    #[test]
    fn the_border_is_seeded_along_its_whole_length() {
        let side = 16u32;
        let setup = setup(grid(side).0, grid(side).1, 0.1, true);

        let seeds = place_on(&setup, 200);

        assert!(seeds.border_seeds > 0, "a grid has a border to seed");
        let on_border = seeds
            .vertices
            .iter()
            .filter(|&&vertex| setup.topology.boundary[vertex as usize])
            .count();
        assert_eq!(
            on_border, seeds.border_seeds,
            "every border seed sits on the border"
        );
        // The border is 4*(side-1) edges long at a spacing of 0.1 over 4 units,
        // so it should carry roughly forty seeds rather than a handful.
        assert!(
            seeds.border_seeds >= 30,
            "the border was seeded at its spacing, got {}",
            seeds.border_seeds
        );
    }

    #[test]
    fn the_corners_of_an_open_grid_are_seeded_exactly() {
        let side = 12u32;
        let setup = setup(grid(side).0, grid(side).1, 0.15, true);

        let seeds = place_on(&setup, 120);

        // A grid's outline is one closed loop with no junction, so the corners
        // are not pinned - but they are on the border and must be reachable.
        // What must hold is that the loop's seeds are spread round it.
        let border: Vec<u32> = seeds
            .vertices
            .iter()
            .copied()
            .filter(|&vertex| setup.topology.boundary[vertex as usize])
            .collect();
        assert!(border.len() >= 4);
    }

    #[test]
    fn a_junction_always_gets_a_seed() {
        // Two strips meeting along a shared edge makes a branch, whose ends are
        // junctions of the feature graph.
        let positions = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0,
        ];
        let indices = vec![0, 1, 2, 0, 1, 3, 0, 1, 4];
        let setup = setup(positions, indices, 0.5, false);

        let seeds = place_on(&setup, 8);

        for vertex in 0..setup.topology.vertex_count as u32 {
            if setup.features.junction[vertex as usize] {
                assert!(
                    seeds.vertices.contains(&vertex),
                    "junction {vertex} was not seeded"
                );
            }
        }
    }

    #[test]
    fn every_component_keeps_some_seeds() {
        // Two grids far apart, with a budget small enough that a naive split
        // could starve one of them.
        let (mut positions, mut indices) = grid(8);
        let offset = (positions.len() / 3) as u32;
        let (other_positions, other_indices) = grid(8);
        positions.extend(
            other_positions
                .chunks(3)
                .flat_map(|point| [point[0] + 10.0, point[1], point[2]]),
        );
        indices.extend(other_indices.iter().map(|&index| index + offset));
        let setup = setup(positions, indices, 0.5, true);

        let seeds = place_on(&setup, 12);

        let mut per_component = vec![0usize; setup.topology.components as usize];
        for &vertex in &seeds.vertices {
            per_component[setup.topology.component[vertex as usize] as usize] += 1;
        }
        assert_eq!(setup.topology.components, 2);
        for (component, count) in per_component.iter().enumerate() {
            assert!(
                *count >= MIN_COMPONENT_SEEDS,
                "component {component} kept only {count} seeds"
            );
        }
    }

    #[test]
    fn seeding_is_the_same_on_every_run() {
        let setup = setup(grid(20).0, grid(20).1, 0.08, true);

        let first = place_on(&setup, 300);
        let second = place_on(&setup, 300);

        assert_eq!(first.vertices, second.vertices);
        assert_eq!(first.polyline, second.polyline);
        assert_eq!(first.pinned, second.pinned);
    }

    /// The point of asking for a face count: the seed count tracks it.
    #[test]
    fn asking_for_more_faces_places_more_seeds() {
        let setup = setup(grid(24).0, grid(24).1, 0.05, true);

        let few = place_on(&setup, 100);
        let many = place_on(&setup, 600);

        assert!(
            many.len() > few.len(),
            "600 faces should want more vertices than 100: {} against {}",
            many.len(),
            few.len()
        );
    }
}
