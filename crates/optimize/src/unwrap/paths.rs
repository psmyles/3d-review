//! The paths seams are run along: the cheapest chain of edges from one place
//! to another, and the far ends of a surface.
//!
//! A seam path's cost is the sum of its edges' costs ([`super::cost`]) plus a
//! charge for turning: each step that bends away from the way the path was
//! going costs that edge's own cost again times [`TURN_WEIGHT`]` × (1 − cos θ)`.
//! On a quad mesh the straight way through a four-edge vertex is the edge loop,
//! so a seam follows loops rather than stepping across them in a staircase; on
//! triangles it keeps the path from zig-zagging between equally cheap edges.
//! The search runs over directed edges (where the path came from is part of
//! where it is), with ties broken by edge index, so the path is a function of
//! the surface alone.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::surface::Surface;

/// What a full turn back costs, as a multiple of the edge turned onto (a
/// right angle costs half this).
const TURN_WEIGHT: f64 = 1.0;

/// A search state: a directed edge (`2 × edge + 1` runs from its higher end
/// to its lower) and the cost of the way there.
#[derive(Clone, Copy)]
struct State {
    cost: f64,
    directed: u32,
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for State {}
impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for State {
    /// Cheapest first, then lowest edge: the heap is a max-heap.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.directed.cmp(&self.directed))
    }
}

/// The vertex a directed edge leaves and the one it reaches.
fn ends(surface: &Surface, directed: u32) -> (u32, u32) {
    let [a, b] = surface.edges[(directed / 2) as usize].ends;
    if directed.is_multiple_of(2) {
        (a, b)
    } else {
        (b, a)
    }
}

/// The directed edge along `edge` leaving `from`.
fn leaving(surface: &Surface, edge: u32, from: u32) -> u32 {
    if surface.edges[edge as usize].ends[0] == from {
        2 * edge
    } else {
        2 * edge + 1
    }
}

/// The cheapest path from any of `sources` to a vertex `target` marks, over the
/// edges `usable` allows: its edges in order from the source, or `None` when
/// no target can be reached. A source that is a target gives an empty path.
/// The path passes through no vertex twice.
pub(super) fn cheapest_path(
    surface: &Surface,
    cost: &[f64],
    usable: impl Fn(u32) -> bool,
    sources: &[u32],
    target: &[bool],
) -> Option<Vec<u32>> {
    if sources.iter().any(|&source| target[source as usize]) {
        return Some(Vec::new());
    }
    let states = surface.edges.len() * 2;
    let mut best = vec![f64::INFINITY; states];
    let mut previous = vec![u32::MAX; states];
    let mut heap = BinaryHeap::new();
    for &source in sources {
        for &edge in surface.edges_at(source) {
            if !usable(edge) {
                continue;
            }
            let directed = leaving(surface, edge, source);
            let reached = cost[edge as usize];
            if reached < best[directed as usize] {
                best[directed as usize] = reached;
                heap.push(State {
                    cost: reached,
                    directed,
                });
            }
        }
    }
    while let Some(State {
        cost: so_far,
        directed,
    }) = heap.pop()
    {
        if so_far > best[directed as usize] {
            continue;
        }
        let (from, at) = ends(surface, directed);
        if target[at as usize] {
            return Some(walk_back(surface, &previous, directed));
        }
        let heading = (surface.points[at as usize] - surface.points[from as usize])
            .as_dvec3()
            .normalize_or_zero();
        for &edge in surface.edges_at(at) {
            if edge == directed / 2 || !usable(edge) {
                continue;
            }
            let next = surface.across(edge, at);
            let turn = (surface.points[next as usize] - surface.points[at as usize])
                .as_dvec3()
                .normalize_or_zero();
            let bend = (1.0 - heading.dot(turn)).max(0.0);
            let step = cost[edge as usize] * (1.0 + TURN_WEIGHT * 0.5 * bend);
            let onward = leaving(surface, edge, at);
            let reached = so_far + step;
            if reached < best[onward as usize] {
                best[onward as usize] = reached;
                previous[onward as usize] = directed;
                heap.push(State {
                    cost: reached,
                    directed: onward,
                });
            }
        }
    }
    None
}

/// The path ending with `last`, from its start, with any loop it made cut out.
fn walk_back(surface: &Surface, previous: &[u32], last: u32) -> Vec<u32> {
    let mut directed = vec![last];
    loop {
        let before = previous[directed[directed.len() - 1] as usize];
        if before == u32::MAX {
            break;
        }
        directed.push(before);
    }
    directed.reverse();
    // A path that came back through a vertex would cut a closed loop out of
    // the surface; keep only the way round that does not. `seen[i]` is the
    // vertex the first `i` kept edges reach.
    let mut kept: Vec<u32> = Vec::with_capacity(directed.len());
    let mut seen: Vec<u32> = vec![ends(surface, directed[0]).0];
    for &step in &directed {
        let (_, to) = ends(surface, step);
        if let Some(at) = seen.iter().position(|&vertex| vertex == to) {
            kept.truncate(at);
            seen.truncate(at + 1);
        } else {
            kept.push(step / 2);
            seen.push(to);
        }
    }
    kept
}

/// Per vertex, the length of the shortest way from `from` over the surface's
/// edges; and the farthest vertex (the lowest of equals).
pub(super) fn farthest(surface: &Surface, from: u32) -> (u32, Vec<f64>) {
    let mut distance = vec![f64::INFINITY; surface.points.len()];
    distance[from as usize] = 0.0;
    // The same ordering as the seam search, over vertices rather than
    // directed edges: `directed` holds the vertex.
    let mut heap = BinaryHeap::new();
    heap.push(State {
        cost: 0.0,
        directed: from,
    });
    while let Some(State {
        cost: so_far,
        directed: at,
    }) = heap.pop()
    {
        if so_far > distance[at as usize] {
            continue;
        }
        for &edge in surface.edges_at(at) {
            let next = surface.across(edge, at);
            let reached = so_far + surface.edges[edge as usize].length;
            if reached < distance[next as usize] {
                distance[next as usize] = reached;
                heap.push(State {
                    cost: reached,
                    directed: next,
                });
            }
        }
    }
    let far = (0..surface.points.len() as u32)
        .filter(|&vertex| distance[vertex as usize].is_finite())
        .max_by(|&a, &b| {
            distance[a as usize]
                .total_cmp(&distance[b as usize])
                .then(b.cmp(&a))
        })
        .unwrap_or(from);
    (far, distance)
}
