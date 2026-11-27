//! Placing seams: as few as an island needs, where they show least.
//!
//! Seams come first and charts follow from them — the way an artist cuts a
//! model, and the opposite of growing charts and keeping whatever borders are
//! left between them. An island the fixed seams bound ([`super::islands`]) is
//! handled in three steps:
//!
//! 1. **Open it.** A disc flattens as it is, and so does a disc with holes (see
//!    [`super::flatten`]). A closed shell is opened by the cheapest path between
//!    its two far ends; a handle (a mug's, a ring's) by a tree-cotree cut graph
//!    whose dual tree keeps the costliest edges, pruned to the loops it must
//!    keep.
//! 2. **Relieve it.** The open end of a tube is first tried **bridged** to
//!    the outer border, which unrolls it into a strip. Then, while the
//!    flattening stretches more than the limit, a **branch** is run from where
//!    it stretches most — the cone-like vertex ([`Surface::cone`]: a cut at a
//!    saddle laps over itself) whose triangles carry the most stretch past 1,
//!    off any seam — along the cheapest path to the nearest seam or border
//!    (Gu, Gortler and Hoppe, "Geometry Images", 2002; costs as Sheffer and
//!    Hart's "Seamster", 2002, weigh visibility). A branch is a slit, not a new
//!    island. Far past the limit, up to [`BRANCHES_PER_ROUND`] go in at once,
//!    at tips well apart, and the first alone if they do not help together; a
//!    round is kept only if it brings the island nearer the limit
//!    ([`Settings::better`]).
//! 3. **Split it**, only if branching stalls, or the island is within the
//!    limit but lies over itself: in two, by the faces nearer each side of the
//!    overlap (or each of its two far ends), the border slid onto cheap edges
//!    ([`settle`]) and kept from pinching ([`unpinch`]); each half starts again
//!    from step 1. An island larger than [`PLAN_TRIANGLES`] is split before
//!    anything else, so a flattening's cost stays bounded and the halves can
//!    run side by side.
//!
//! Two faults no seam can mend are set apart face by face instead
//! ([`isolate`]): where the island is not manifold, and a few slivers that
//! collapse however they are laid flat.
//!
//! Every path keeps to the edges between faces ([`Surface::cuttable`]), so a
//! polygon is never cut; and every split assigns whole faces.

use std::collections::BinaryHeap;
use std::sync::OnceLock;

use glam::{DVec3, Vec2};

use super::charts::{Chart, CutMesh, Settings};
use super::cost::{Occluders, edge_costs};
use super::flatten::{Domain, Flat, Shape, TEST_PASSES, flatten, refine, shape};
use super::paths::{cheapest_path, farthest};
use super::surface::{NONE, Surface, root, union};
use crate::cancel::{CancelToken, cancelled};

/// An island larger than this (in triangles) is split before its seams are
/// placed: a flattening's cost grows faster than its size.
pub(super) const PLAN_TRIANGLES: usize = 20_000;

/// Rounds of branches an island is given before it is split.
const MAX_ROUNDS: usize = 16;

/// Rounds in a row that may fail to bring an island nearer the limit before
/// branching is given up on.
const MAX_FAILURES: usize = 3;

/// Branches one round may add, when the island is far past the limit.
const BRANCHES_PER_ROUND: usize = 3;

/// Past this many times the limit (see [`Settings::excess`]), a round adds
/// more than one branch.
const FAR_PAST: f64 = 1.25;

/// Tips of one round are at least this share of the square root of the
/// island's area apart.
const TIP_SPACING: f64 = 0.25;

/// A second or third tip in one round carries at least this share of the
/// stretch the first does.
const TIP_SHARE: f64 = 0.25;

/// A hole whose border is at least this share of the square root of the
/// island's area is the open end of a tube rather than a hole in a sheet, and
/// is first tried **bridged**: cut to the island's outer border along the
/// cheapest path, which unrolls a tube into a strip. An eye socket stays a
/// hole.
const BRIDGE_SHARE: f64 = 0.5;

/// The most of an island's area that may fold and still be put down to
/// slivers, set apart rather than cut round.
const SLIVER_SHARE: f64 = 0.002;

/// What happened to one island.
pub(super) enum Outcome {
    /// Flattened: its charts.
    Charts(Vec<Chart>),
    /// Split: the pieces, each to plan on its own.
    Split(Vec<Vec<u32>>),
}

/// Plans the islands of one object.
pub(super) struct Planner<'a> {
    pub mesh: &'a CutMesh<'a>,
    pub settings: Settings,
    /// The object's surface, for occlusion: built by the first island that
    /// needs it.
    pub occluders: &'a OnceLock<Occluders>,
    pub cancel: Option<&'a CancelToken>,
}

impl Planner<'_> {
    /// Plan one island (`triangles`, ascending): its charts, or the pieces it
    /// was split into, each to be planned in turn; `None` once the run is no
    /// longer wanted.
    pub(super) fn island(&self, triangles: &[u32]) -> Option<Outcome> {
        let surface = Surface::new(self.mesh, triangles);
        let pieces = surface.components();
        if pieces.len() > 1 {
            return Some(Outcome::Split(pieces));
        }
        if surface.face_count() <= 1 {
            return Some(Outcome::Charts(vec![self.single(&surface)]));
        }
        if triangles.len() > PLAN_TRIANGLES {
            return Some(self.split(&surface, None, None));
        }

        // Step 1: open it.
        let mut cut = vec![false; surface.edges.len()];
        let mut domain = surface.domain(&cut);
        let mut shape_now = shape(&domain);
        let mut costs: Option<Vec<f64>> = None;
        if !shape_now.planar() {
            if !shape_now.manifold {
                return Some(isolate(&surface, &shape_now.trouble));
            }
            let costs = costs.get_or_insert_with(|| self.costs(&surface));
            open(&surface, costs, &mut cut);
            domain = surface.domain(&cut);
            shape_now = shape(&domain);
            if !shape_now.planar() {
                return Some(self.split(&surface, Some(costs.as_slice()), None));
            }
        }
        let settings = self.settings;
        let Some(mut flat) = self.judge(&domain, &shape_now) else {
            return Some(self.split(&surface, costs.as_deref(), None));
        };

        // Step 2: relieve it.
        let mut tried = vec![false; surface.points.len()];
        let mut bridged = vec![false; surface.points.len()];
        let mut cone: Option<Vec<f64>> = None;
        let (mut rounds, mut failures) = (0, 0);
        // Where a trial within the limit lay over itself: the flap to set free
        // if branching stalls.
        let mut flap: Option<[usize; 2]> = None;
        while !settings.accepts(&flat) && rounds < MAX_ROUNDS && failures < MAX_FAILURES {
            if cancelled(self.cancel) {
                return None;
            }
            let costs = costs.get_or_insert_with(|| self.costs(&surface));
            // A tube's open end first: laid flat as a ring a tube overlaps
            // itself, and unrolled it does not.
            let mut trial = cut.clone();
            let bridging = bridge(
                &surface,
                costs,
                &domain,
                &shape_now,
                &flat,
                &mut bridged,
                &mut trial,
            );
            if !bridging && settings.excess(&flat) <= 1.0 {
                // Within the limit but lying over itself with nothing folded,
                // even fully refined: a flap (an ear, cut free at its tip) has
                // swung over the rest. No branch can draw it back, as there is
                // no stretch left to point at it, so the island comes apart
                // where it overlaps. Further past the limit, branches come
                // first: relieving the stretch often lets the flap lie down.
                if flat.overlaps && flat.folded == 0.0 {
                    return Some(self.split(&surface, Some(costs.as_slice()), flat.overlap_at));
                }
                // Within the limit but for a few slivers that collapse (a CAD
                // tessellation's needles): no seam straightens a needle, so
                // those faces go apart and the rest is planned without them.
                if flat.folded > 0.0 && flat.folded <= SLIVER_SHARE && !flat.overlaps {
                    return Some(isolate(&surface, &flat.folds));
                }
            }
            rounds += 1;

            // Then branches: every tip at once and, where that does not help
            // (two flaps cut side by side can lie over each other), the first
            // alone.
            let mut trials: Vec<Vec<bool>> = Vec::new();
            let mut tips_now: Vec<u32> = Vec::new();
            if bridging {
                trials.push(trial);
            } else {
                let count = if settings.excess(&flat) > FAR_PAST {
                    BRANCHES_PER_ROUND
                } else {
                    1
                };
                let cone = cone.get_or_insert_with(|| surface.cone());
                tips_now = tips(&surface, cone, &domain, &flat, &cut, &tried, count);
                if tips_now.is_empty() {
                    break;
                }
                for &tip in &tips_now {
                    tried[tip as usize] = true;
                    branch(&surface, costs, &mut trial, tip);
                }
                trials.push(trial);
                if tips_now.len() > 1 {
                    let mut single = cut.clone();
                    branch(&surface, costs, &mut single, tips_now[0]);
                    trials.push(single);
                }
            }
            let mut kept = None;
            for (index, trial) in trials.into_iter().enumerate() {
                let next_domain = surface.domain(&trial);
                let next_shape = shape(&next_domain);
                if !next_shape.planar() {
                    continue;
                }
                let Some(next) = self.judge(&next_domain, &next_shape) else {
                    continue;
                };
                if next.overlaps && next.folded == 0.0 && settings.excess(&next) <= 1.0 {
                    flap = next.overlap_at.or(flap);
                }
                if settings.better(&next, &flat) {
                    cut = trial;
                    domain = next_domain;
                    shape_now = next_shape;
                    flat = next;
                    kept = Some(index);
                    break;
                }
            }
            match kept {
                // The first tip alone: the others may yet be wanted.
                Some(1) => {
                    for &tip in tips_now.iter().skip(1) {
                        tried[tip as usize] = false;
                    }
                    failures = 0;
                }
                Some(_) => failures = 0,
                None => failures += 1,
            }
        }

        // Step 3: split it, if branching stalled.
        if !settings.accepts(&flat) {
            return Some(self.split(&surface, costs.as_deref(), flap));
        }
        Some(Outcome::Charts(vec![
            self.finish(&surface, &domain, &shape_now, flat),
        ]))
    }

    /// A domain flattened for judging: the test passes, and the rest of them
    /// where those leave it lying over itself with nothing folded — a flap
    /// still swinging into place reads as an overlap after a few passes, and
    /// judging it there would refuse every seam that frees one.
    fn judge(&self, domain: &Domain, shape: &Shape) -> Option<Flat> {
        let flattening = self.settings.flattening;
        let flat = flatten(domain, shape, flattening, TEST_PASSES)?;
        if !(flat.overlaps && flat.folded == 0.0) {
            return Some(flat);
        }
        let passes = super::arap::PASSES - TEST_PASSES;
        Some(match refine(domain, shape, &flat, flattening, passes) {
            Some(refined) if refined.valid || refined.folded == 0.0 => refined,
            _ => flat,
        })
    }

    /// What a seam costs along each of `surface`'s edges.
    fn costs(&self, surface: &Surface) -> Vec<f64> {
        let occluders = self
            .occluders
            .get_or_init(|| Occluders::new(self.mesh.positions, self.mesh.triangles));
        edge_costs(surface, occluders)
    }

    /// `surface` split in two, or flattened as it is when it cannot be.
    fn split(
        &self,
        surface: &Surface,
        costs: Option<&[f64]>,
        seeds: Option<[usize; 2]>,
    ) -> Outcome {
        match bisect(surface, costs, seeds) {
            Some((first, second)) => Outcome::Split(vec![first, second]),
            None => Outcome::Charts(vec![self.single(surface)]),
        }
    }

    /// A surface flattened whole with no seams placed, whatever it stretches:
    /// one polygon, which nothing may cut.
    fn single(&self, surface: &Surface) -> Chart {
        let domain = surface.domain(&vec![false; surface.edges.len()]);
        let shape = shape(&domain);
        match flatten(&domain, &shape, self.settings.flattening, TEST_PASSES) {
            Some(flat) => self.finish(surface, &domain, &shape, flat),
            // No area to flatten.
            None => Chart {
                triangles: surface.triangles.clone(),
                uvs: vec![[Vec2::ZERO; 3]; surface.triangles.len()],
            },
        }
    }

    /// The rest of the as-rigid-as-possible passes over a chart that is kept,
    /// kept where they leave it valid and no further from the limit.
    fn finish(&self, surface: &Surface, domain: &Domain, shape: &Shape, flat: Flat) -> Chart {
        let settings = self.settings;
        let passes = super::arap::PASSES - TEST_PASSES;
        let flat = match refine(domain, shape, &flat, settings.flattening, passes) {
            Some(refined)
                if (refined.valid || !flat.valid)
                    && (settings.accepts(&refined)
                        || settings.excess(&refined) <= settings.excess(&flat)) =>
            {
                refined
            }
            _ => flat,
        };
        Chart {
            triangles: surface.triangles.clone(),
            uvs: domain
                .triangles
                .iter()
                .map(|corners| corners.map(|vertex| flat.uvs[vertex as usize]))
                .collect(),
        }
    }
}

/// `surface` with the faces of the triangles `marked` set apart, each on its
/// own, and the rest planned as usual. For a fault no seam along the
/// surface's own edges can mend — where it is not manifold (an edge more than
/// two faces share, a point the border passes twice), or a sliver that
/// collapses however it is laid flat — when halving the whole island for it
/// would scatter seams far from the fault.
fn isolate(surface: &Surface, marked: &[bool]) -> Outcome {
    let mut apart: Vec<u32> = surface
        .face
        .iter()
        .zip(marked)
        .filter(|&(_, &marked)| marked)
        .map(|(&face, _)| face)
        .collect();
    apart.sort_unstable();
    apart.dedup();
    let mut rest = Vec::new();
    let mut alone: Vec<Vec<u32>> = vec![Vec::new(); apart.len()];
    for (local, &face) in surface.face.iter().enumerate() {
        match apart.binary_search(&face) {
            Ok(at) => alone[at].push(surface.triangles[local]),
            Err(_) => rest.push(surface.triangles[local]),
        }
    }
    let mut pieces = alone;
    if !rest.is_empty() {
        pieces.insert(0, rest);
    }
    Outcome::Split(pieces)
}

/// Split every island larger than [`PLAN_TRIANGLES`] until none is, pushing
/// the pieces onto `jobs`: `None` once the run is no longer wanted.
pub(super) fn split_to_size(
    mesh: &CutMesh<'_>,
    triangles: Vec<u32>,
    jobs: &mut Vec<Vec<u32>>,
    cancel: Option<&CancelToken>,
) -> Option<()> {
    let mut pending = vec![triangles];
    while let Some(triangles) = pending.pop() {
        if triangles.len() <= PLAN_TRIANGLES {
            jobs.push(triangles);
            continue;
        }
        if cancelled(cancel) {
            return None;
        }
        let surface = Surface::new(mesh, &triangles);
        let pieces = surface.components();
        if pieces.len() > 1 {
            pending.extend(pieces.into_iter().rev());
            continue;
        }
        match bisect(&surface, None, None) {
            Some((first, second)) => {
                pending.push(second);
                pending.push(first);
            }
            None => jobs.push(triangles),
        }
    }
    Some(())
}

/// Open a surface that cannot lie flat: a closed shell along the cheapest
/// path between its two far ends, then any handles along a cut graph.
fn open(surface: &Surface, costs: &[f64], cut: &mut [bool]) {
    let closed = (0..surface.edges.len() as u32).all(|edge| !surface.border(edge));
    if closed && !surface.points.is_empty() {
        let (first, _) = farthest(surface, 0);
        let (second, _) = farthest(surface, first);
        if first != second {
            let mut target = vec![false; surface.points.len()];
            target[second as usize] = true;
            let usable = |edge: u32| surface.cuttable(edge);
            if let Some(path) = cheapest_path(surface, costs, usable, &[first], &target) {
                for edge in path {
                    cut[edge as usize] = true;
                }
            }
        }
    }
    let domain = surface.domain(cut);
    let shape = shape(&domain);
    if shape.manifold && shape.components == 1 && shape.handles() > 0 {
        cut_graph(surface, costs, cut);
    }
}

/// Cut a surface with handles open: the edges a dual spanning tree does not
/// cross — the tree keeping the costliest edges, every edge inside a polygon
/// first — pruned back from their dead ends, which leaves the loops round
/// each handle and the paths joining them to the border.
fn cut_graph(surface: &Surface, costs: &[f64], cut: &mut [bool]) {
    let mut order: Vec<u32> = (0..surface.edges.len() as u32)
        .filter(|&edge| !surface.border(edge) && !cut[edge as usize])
        .collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&surface.edges[a as usize], &surface.edges[b as usize]);
        y.diagonal
            .cmp(&x.diagonal)
            .then(costs[b as usize].total_cmp(&costs[a as usize]))
            .then(a.cmp(&b))
    });
    let mut parent: Vec<u32> = (0..surface.corners.len() as u32).collect();
    let mut graph = vec![false; surface.edges.len()];
    for edge in order {
        let [first, second] = surface.edges[edge as usize].sides;
        if root(&mut parent, first) == root(&mut parent, second) {
            graph[edge as usize] = true;
        } else {
            union(&mut parent, first, second);
        }
    }
    let in_graph = |edge: u32, graph: &[bool]| {
        graph[edge as usize] || cut[edge as usize] || surface.border(edge)
    };
    let mut degree: Vec<usize> = (0..surface.points.len() as u32)
        .map(|vertex| {
            surface
                .edges_at(vertex)
                .iter()
                .filter(|&&edge| in_graph(edge, &graph))
                .count()
        })
        .collect();
    let mut ends: Vec<u32> = (0..surface.points.len() as u32)
        .filter(|&vertex| degree[vertex as usize] == 1)
        .collect();
    while let Some(vertex) = ends.pop() {
        if degree[vertex as usize] != 1 {
            continue;
        }
        let Some(&edge) = surface
            .edges_at(vertex)
            .iter()
            .find(|&&edge| graph[edge as usize])
        else {
            continue;
        };
        graph[edge as usize] = false;
        degree[vertex as usize] -= 1;
        let other = surface.across(edge, vertex);
        degree[other as usize] -= 1;
        if degree[other as usize] == 1 {
            ends.push(other);
        }
    }
    for (edge, &kept) in graph.iter().enumerate() {
        if kept {
            cut[edge] = true;
        }
    }
}

/// Per vertex of `surface`, whether a seam or border reaches it.
fn frontier(surface: &Surface, cut: &[bool]) -> Vec<bool> {
    let mut frontier = vec![false; surface.points.len()];
    for (id, edge) in surface.edges.iter().enumerate() {
        if cut[id] || edge.sides[1] == NONE {
            for vertex in edge.ends {
                frontier[vertex as usize] = true;
            }
        }
    }
    frontier
}

/// Up to `count` vertices to run branches from: those whose triangles carry
/// the most stretch past 1, off every seam and not tried before, each well
/// apart from the ones before it.
fn tips(
    surface: &Surface,
    cone: &[f64],
    domain: &Domain,
    flat: &Flat,
    cut: &[bool],
    tried: &[bool],
    count: usize,
) -> Vec<u32> {
    let frontier = frontier(surface, cut);
    let mut score = vec![0.0f64; surface.points.len()];
    for (triangle, corners) in domain.triangles.iter().enumerate() {
        let excess = flat.area[triangle] * (flat.stretch[triangle] - 1.0).max(0.0);
        for &vertex in corners {
            score[domain.origin[vertex as usize] as usize] += excess;
        }
    }
    let mut order: Vec<u32> = (0..surface.points.len() as u32)
        .filter(|&vertex| {
            !frontier[vertex as usize]
                && !tried[vertex as usize]
                && score[vertex as usize] > 0.0
                && cone[vertex as usize] > 0.0
        })
        .collect();
    order.sort_by(|&a, &b| {
        score[b as usize]
            .total_cmp(&score[a as usize])
            .then(a.cmp(&b))
    });
    let area: f64 = flat.area.iter().sum();
    let spacing = TIP_SPACING * area.sqrt();
    let mut tips: Vec<u32> = Vec::new();
    for vertex in order {
        if tips.len() >= count {
            break;
        }
        if let Some(&first) = tips.first()
            && score[vertex as usize] < TIP_SHARE * score[first as usize]
        {
            break;
        }
        let at = surface.points[vertex as usize];
        if tips
            .iter()
            .all(|&tip| f64::from(at.distance(surface.points[tip as usize])) >= spacing)
        {
            tips.push(vertex);
        }
    }
    tips
}

/// Bridge the largest hole of `domain` not bridged before whose border is long
/// enough to be a tube's open end ([`BRIDGE_SHARE`]): cut it to another border
/// along the cheapest path, marking the edges in `cut` and the hole's vertices
/// in `bridged`. Whether there was one.
fn bridge(
    surface: &Surface,
    costs: &[f64],
    domain: &Domain,
    shape: &Shape,
    flat: &Flat,
    bridged: &mut [bool],
    cut: &mut [bool],
) -> bool {
    if shape.loops.len() < 2 {
        return false;
    }
    let length = |walk: &[u32]| -> f64 {
        (0..walk.len())
            .map(|k| {
                let a = domain.points[walk[k] as usize].as_dvec3();
                let b = domain.points[walk[(k + 1) % walk.len()] as usize].as_dvec3();
                a.distance(b)
            })
            .sum()
    };
    let mut loops: Vec<(f64, usize)> = shape
        .loops
        .iter()
        .enumerate()
        .map(|(index, walk)| (length(walk), index))
        .collect();
    loops.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    // The longest is the outside; of the rest, the largest not yet bridged.
    let least = BRIDGE_SHARE * flat.area.iter().sum::<f64>().sqrt();
    let Some(&(_, hole)) = loops.iter().skip(1).find(|&&(length, index)| {
        length >= least
            && !shape.loops[index]
                .iter()
                .any(|&vertex| bridged[domain.origin[vertex as usize] as usize])
    }) else {
        return false;
    };
    let sources: Vec<u32> = shape.loops[hole]
        .iter()
        .map(|&vertex| domain.origin[vertex as usize])
        .collect();
    let mut target = vec![false; surface.points.len()];
    for (index, walk) in shape.loops.iter().enumerate() {
        if index != hole {
            for &vertex in walk {
                target[domain.origin[vertex as usize] as usize] = true;
            }
        }
    }
    for &source in &sources {
        bridged[source as usize] = true;
        target[source as usize] = false;
    }
    let usable = |edge: u32| surface.cuttable(edge) && !cut[edge as usize];
    if let Some(path) = cheapest_path(surface, costs, usable, &sources, &target) {
        for edge in path {
            cut[edge as usize] = true;
        }
    }
    true
}

/// Run a branch from `tip` along the cheapest path to the nearest seam or
/// border, marking its edges in `cut`.
fn branch(surface: &Surface, costs: &[f64], cut: &mut [bool], tip: u32) {
    let frontier = frontier(surface, cut);
    let usable = |edge: u32| surface.cuttable(edge) && !cut[edge as usize];
    if let Some(path) = cheapest_path(surface, costs, usable, &[tip], &frontier) {
        for edge in path {
            cut[edge as usize] = true;
        }
    }
}

/// `surface` in two, whole faces each, as cut mesh triangles (ascending): the
/// faces nearer one seed and the faces nearer the other — the two triangles
/// `seeds` names, or else the surface's two far ends. With `costs`, the border
/// between them is then slid onto the edges where a seam costs least
/// ([`settle`]). `None` for a surface of one face.
fn bisect(
    surface: &Surface,
    costs: Option<&[f64]>,
    seeds: Option<[usize; 2]>,
) -> Option<(Vec<u32>, Vec<u32>)> {
    // Faces as units, in order of first triangle, with their centroids.
    let mut unit_of_face: std::collections::HashMap<u32, u32> = Default::default();
    let mut unit: Vec<u32> = Vec::with_capacity(surface.face.len());
    for &face in &surface.face {
        let next = unit_of_face.len() as u32;
        unit.push(*unit_of_face.entry(face).or_insert(next));
    }
    let units = unit_of_face.len();
    if units < 2 {
        return None;
    }
    let areas = surface.areas();
    let mut weighted = vec![DVec3::ZERO; units];
    let mut total = vec![0.0f64; units];
    for (triangle, corners) in surface.corners.iter().enumerate() {
        let centre = corners
            .iter()
            .map(|&vertex| surface.points[vertex as usize].as_dvec3())
            .sum::<DVec3>()
            / 3.0;
        let at = unit[triangle] as usize;
        // A sliver still counts for something, so every unit has a centre.
        let weight = areas[triangle].max(1.0e-30);
        weighted[at] += centre * weight;
        total[at] += weight;
    }
    let centre: Vec<DVec3> = weighted
        .iter()
        .zip(&total)
        .map(|(sum, &weight)| *sum / weight)
        .collect();
    let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); units];
    for edge in &surface.edges {
        let [first, second] = edge.sides;
        if second == NONE {
            continue;
        }
        let (a, b) = (unit[first as usize], unit[second as usize]);
        if a != b {
            neighbours[a as usize].push(b);
            neighbours[b as usize].push(a);
        }
    }
    let search = |seeds: &[u32]| -> (Vec<f64>, Vec<u32>) {
        let mut distance = vec![f64::INFINITY; units];
        let mut label = vec![0u32; units];
        let mut heap = BinaryHeap::new();
        for (index, &seed) in seeds.iter().enumerate() {
            distance[seed as usize] = 0.0;
            label[seed as usize] = index as u32;
            heap.push(Near(0.0, index as u32, seed));
        }
        while let Some(Near(so_far, own, at)) = heap.pop() {
            if so_far > distance[at as usize] || own != label[at as usize] {
                continue;
            }
            for &next in &neighbours[at as usize] {
                // A seed is its own half's, even where another reaches it at
                // no distance (a face doubled back on itself has its twin's
                // centre).
                if seeds.contains(&next) {
                    continue;
                }
                let reached = so_far + centre[at as usize].distance(centre[next as usize]);
                let better = reached < distance[next as usize]
                    || (reached == distance[next as usize] && own < label[next as usize]);
                if better {
                    distance[next as usize] = reached;
                    label[next as usize] = own;
                    heap.push(Near(reached, own, next));
                }
            }
        }
        (distance, label)
    };
    let far = |distance: &[f64], from: u32| {
        (0..units as u32)
            .filter(|&at| distance[at as usize].is_finite())
            .max_by(|&a, &b| {
                distance[a as usize]
                    .total_cmp(&distance[b as usize])
                    .then(b.cmp(&a))
            })
            .unwrap_or(from)
    };
    let (first, mut second) = match seeds {
        Some([one, two]) => (unit[one], unit[two]),
        None => {
            let (distance, _) = search(&[0]);
            let first = far(&distance, 0);
            let (distance, _) = search(&[first]);
            (first, far(&distance, first))
        }
    };
    if second == first {
        second = if first == 0 { 1 } else { 0 };
    }
    let (_, mut label) = search(&[first, second]);
    let unpinched = {
        let mut cleaned = label.clone();
        if let Some(costs) = costs {
            settle(surface, costs, &unit, &mut cleaned);
        }
        unpinch(surface, &unit, &mut cleaned);
        cleaned
    };
    // Kept unless it emptied a half.
    if unpinched.contains(&0) && unpinched.contains(&1) {
        label = unpinched;
    }
    let (mut one, mut two) = (Vec::new(), Vec::new());
    for (triangle, &global) in surface.triangles.iter().enumerate() {
        if label[unit[triangle] as usize] == 0 {
            one.push(global);
        } else {
            two.push(global);
        }
    }
    (!one.is_empty() && !two.is_empty()).then_some((one, two))
}

/// Passes [`settle`] makes at most.
/// Each pass moves the border by a face at most, so this is also how far it
/// may travel: far enough to find a crease beside it, not so far that it
/// eats one half away (the border's cost only falls as a half shrinks).
const SETTLE_PASSES: usize = 4;

/// Slide the border between a split's two halves onto the edges where a seam
/// costs least: a face on the border goes to the other half wherever that
/// lowers the border's total cost — its edges to its own half cost more than
/// its edges to the other — so a corner juts in neither direction and the
/// border creeps along a crease it lies near. Local moves only: a border
/// settles where it was put, it does not travel.
fn settle(surface: &Surface, costs: &[f64], unit: &[u32], label: &mut [u32]) {
    let units = label.len();
    // Per unit, its neighbours across edges with the seam cost between them.
    let mut links: Vec<Vec<(u32, f64)>> = vec![Vec::new(); units];
    for (id, edge) in surface.edges.iter().enumerate() {
        let [first, second] = edge.sides;
        if second == NONE {
            continue;
        }
        let (a, b) = (unit[first as usize], unit[second as usize]);
        if a != b {
            links[a as usize].push((b, costs[id]));
            links[b as usize].push((a, costs[id]));
        }
    }
    let mut size = [0usize; 2];
    for &side in label.iter() {
        size[side as usize] += 1;
    }
    // Neither half below half of what it started at.
    let least = [size[0].div_ceil(2).max(1), size[1].div_ceil(2).max(1)];
    for _ in 0..SETTLE_PASSES {
        let mut moved = false;
        for at in 0..units {
            let own = label[at];
            let (mut same, mut other) = (0.0, 0.0);
            for &(next, cost) in &links[at] {
                if label[next as usize] == own {
                    same += cost;
                } else {
                    other += cost;
                }
            }
            if other > same && size[own as usize] > least[own as usize] {
                label[at] = 1 - own;
                size[own as usize] -= 1;
                size[1 - own as usize] += 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

/// Passes [`unpinch`] makes at most.
const UNPINCH_PASSES: usize = 8;

/// Move faces across a split until neither half meets itself at a point: at
/// each vertex where one half's faces fall in more than one wedge, the faces of
/// every wedge but the largest go to the other half. A half pinched at a point
/// flattens as two flaps that are free to overlap there.
fn unpinch(surface: &Surface, unit: &[u32], label: &mut [u32]) {
    let mut around: Vec<Vec<u32>> = vec![Vec::new(); surface.points.len()];
    for (triangle, corners) in surface.corners.iter().enumerate() {
        for &vertex in corners {
            if around[vertex as usize].last() != Some(&(triangle as u32)) {
                around[vertex as usize].push(triangle as u32);
            }
        }
    }
    let side = |label: &[u32], triangle: u32| label[unit[triangle as usize] as usize];
    for _ in 0..UNPINCH_PASSES {
        let mut moved = false;
        for (vertex, triangles) in around.iter().enumerate() {
            // Wedges: this vertex's triangles joined across its edges whose
            // two sides are in one half.
            let local = |triangle: u32| triangles.iter().position(|&at| at == triangle);
            let mut parent: Vec<u32> = (0..triangles.len() as u32).collect();
            for &edge in surface.edges_at(vertex as u32) {
                let [first, second] = surface.edges[edge as usize].sides;
                if second == NONE || side(label, first) != side(label, second) {
                    continue;
                }
                if let (Some(a), Some(b)) = (local(first), local(second)) {
                    union(&mut parent, a as u32, b as u32);
                }
            }
            for half in 0..2u32 {
                let mut wedges: Vec<(u32, usize)> = Vec::new();
                for (at, &triangle) in triangles.iter().enumerate() {
                    if side(label, triangle) != half {
                        continue;
                    }
                    let wedge = root(&mut parent, at as u32);
                    match wedges.iter_mut().find(|(id, _)| *id == wedge) {
                        Some((_, size)) => *size += 1,
                        None => wedges.push((wedge, 1)),
                    }
                }
                if wedges.len() < 2 {
                    continue;
                }
                let keep = wedges
                    .iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                    .map_or(0, |&(id, _)| id);
                let moves: Vec<u32> = triangles
                    .iter()
                    .enumerate()
                    .filter(|&(at, &triangle)| {
                        side(label, triangle) == half && root(&mut parent, at as u32) != keep
                    })
                    .map(|(_, &triangle)| triangle)
                    .collect();
                for triangle in moves {
                    label[unit[triangle as usize] as usize] = 1 - half;
                }
                moved = true;
                // The wedges are stale once a face has moved.
                break;
            }
        }
        if !moved {
            break;
        }
    }
}

/// A unit reached in the split's search: the distance, the seed it came from,
/// the unit. Nearest first, then lower seed, then lower unit.
struct Near(f64, u32, u32);

impl PartialEq for Near {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl Eq for Near {}
impl PartialOrd for Near {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Near {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .0
            .total_cmp(&self.0)
            .then_with(|| other.1.cmp(&self.1))
            .then_with(|| other.2.cmp(&self.2))
    }
}
