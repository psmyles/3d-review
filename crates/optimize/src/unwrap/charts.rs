//! The charts each island is flattened in: as few and as large as flatten
//! within the stretch limit.
//!
//! An island the seams bound is first tried whole. Where that fails — it is
//! not a disc (a closed shell, a band round a cylinder), its flattening would
//! fold or overlap itself, or it stretches more than the limit — it is cut into
//! small, nearly flat pieces ("atoms": triangles flooded out from a seed while
//! their normals stay within [`ATOM_CONE_DEGREES`] of its, at most
//! [`ATOM_TRIANGLES`] of them; one that comes out other than a disc starts as
//! its triangles instead) and grown back greedily: the two neighbouring charts
//! sharing the most border, as a share of the smaller one's, are merged first,
//! and a merge is kept only when the merged chart is a disc whose flattening is
//! valid and within the limit (Sander, Snyder, Gortler and Hoppe, "Texture
//! Mapping Progressive Meshes", 2001, grows charts the same way). Slivers
//! along a big chart's edge share most of their border with it, so they go
//! first.
//!
//! A merge is tested one of two ways. While the merged chart is small, it is
//! flattened whole: LSCM, then (for minimum stretch) a few as-rigid-as-possible
//! passes. Past [`WHOLE_TEST_TRIANGLES`], the larger chart is held as it is and
//! the smaller one is flattened onto it, its shared border pinned where the
//! larger one has it — so a test costs what the smaller chart does, and a big
//! mesh's merging grows with its size rather than its square. Two discs glued
//! along one stretch of border are a disc; with no triangle folded and the new
//! border crossing nothing, the merged flattening is one-to-one.
//!
//! Every chart that is kept is then refined the rest of the way (the remaining
//! as-rigid-as-possible passes, over the whole chart) and scaled to its own
//! area, so one UV unit is one model unit. A chart that will not flatten at all
//! falls back to its triangles, each a chart of its own.
//!
//! The stretch of a triangle is its worst stretch either way against its
//! chart's mean density: `max(σ₁, 1/σ₂)` once the chart is scaled to its area.
//! It counts both what the stretch view shows (σ₁/σ₂, an anisotropy) and what
//! the texel-density view shows (σ₁σ₂ drifting from 1). A chart's is the
//! area-weighted mean over its triangles; its worst triangle's σ₁/σ₂ is held
//! to ten times as far past 1 as the limit.
//!
//! Merge order is fixed by the chart numbering and the borders' lengths, and
//! every map is ordered, so the result is a function of the mesh alone.

use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use glam::{DVec2, DVec3, Vec2, Vec3};

use super::element::{Element, elements};
use crate::cancel::{CancelToken, cancelled};
use crate::stack::Flattening;

/// An island larger than this (in triangles) is not tried whole, and no merge
/// may make a chart larger: the final refinement's cost grows with a chart's
/// size.
const MAX_CHART_TRIANGLES: usize = 20_000;

/// A merge whose chart would be at most this many triangles is tested by
/// flattening it whole; a larger one, by flattening the smaller chart onto the
/// larger. Whole is a little better (the larger chart may give way too) and
/// costs the whole chart; measured on a 29,000-triangle creature, 100 keeps
/// within a tenth of the charts of flattening every merge whole, at a third of
/// the time.
const WHOLE_TEST_TRIANGLES: usize = 100;

/// An atom's triangles keep their normals within this many degrees of its
/// first one's.
const ATOM_CONE_DEGREES: f64 = 30.0;

/// An atom's triangles, at most.
const ATOM_TRIANGLES: usize = 64;

/// As-rigid-as-possible passes a merge is tested with; a chart that is kept
/// gets the rest of [`super::arap::PASSES`] once the merging is done.
const TEST_PASSES: usize = 4;

/// A UV triangle smaller than this share of its 3D area (once the chart is
/// scaled to its area) is collapsed, as good as folded.
const COLLAPSED: f64 = 1.0e-6;

/// How many merge tests run between two looks at the cancel token.
const CANCEL_CHECK_EVERY: usize = 32;

/// The surface the charts are cut from.
pub(super) struct CutMesh<'a> {
    /// Per cut vertex, its position.
    pub positions: &'a [Vec3],
    /// Per triangle, its three cut vertices.
    pub triangles: &'a [[u32; 3]],
    /// Per triangle, its island.
    pub island: &'a [u32],
}

/// A chart: its triangles, and per triangle its corners' UVs, in model units.
pub(super) struct Chart {
    pub triangles: Vec<u32>,
    pub uvs: Vec<[Vec2; 3]>,
}

/// What the charts are measured and flattened by.
#[derive(Clone, Copy)]
pub(super) struct Settings {
    pub flattening: Flattening,
    pub max_stretch: f32,
}

impl Settings {
    /// Whether a chart of these sums is within the limit: on average, and at
    /// its worst triangle, which may stretch ten times as far past 1 as the
    /// average.
    fn accepts(&self, stretch: Sums) -> bool {
        let limit = f64::from(self.max_stretch);
        stretch.mean() <= limit && stretch.worst <= 1.0 + 10.0 * (limit - 1.0)
    }
}

/// The charts of `mesh`, or `None` once `cancel` says the run is no longer
/// wanted.
pub(super) fn charts(
    mesh: &CutMesh<'_>,
    settings: Settings,
    cancel: Option<&CancelToken>,
) -> Option<Vec<Chart>> {
    let count = mesh.triangles.len();
    let adjacency = Adjacency::new(mesh);

    // Islands, each tried whole.
    let mut by_island: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for triangle in 0..count as u32 {
        by_island
            .entry(mesh.island[triangle as usize])
            .or_default()
            .push(triangle);
    }
    let mut done: Vec<Chart> = Vec::new();
    let mut whole = vec![false; count];
    for triangles in by_island.values() {
        if cancelled(cancel) {
            return None;
        }
        if triangles.len() > MAX_CHART_TRIANGLES {
            continue;
        }
        if let Some(flat) = flatten(mesh, triangles, settings.flattening)
            && settings.accepts(flat.sums)
        {
            for &triangle in triangles {
                whole[triangle as usize] = true;
            }
            let flat = finish(mesh, triangles, flat, settings);
            done.push(flat.into_chart(mesh, triangles.clone()));
        }
    }

    // The rest start from atoms; one that is not a disc (a ring, or two pieces
    // touching at a corner) starts as its triangles, and the merging builds it
    // back into discs.
    let atoms = grow_atoms(mesh, &adjacency, &whole);
    let mut starts: Vec<Vec<u32>> = Vec::new();
    for triangles in atoms {
        if ChartMesh::new(mesh, &triangles).is_some() {
            starts.push(triangles);
        } else {
            starts.extend(triangles.into_iter().map(|triangle| vec![triangle]));
        }
    }
    starts.sort_unstable_by_key(|triangles| triangles[0]);
    // Each triangle's chart (`u32::MAX` for one in a whole island).
    let mut owner: Vec<u32> = vec![u32::MAX; count];
    let mut states: Vec<State> = Vec::with_capacity(starts.len());
    for triangles in starts {
        for &triangle in &triangles {
            owner[triangle as usize] = states.len() as u32;
        }
        states.push(State {
            triangles,
            ..State::default()
        });
    }
    for (id, state) in states.iter_mut().enumerate() {
        let mut perimeter = 0.0;
        for &triangle in &state.triangles {
            for (k, edge) in adjacency.edges(triangle).iter().enumerate() {
                match edge {
                    Some(other) if owner[*other as usize] == id as u32 => {}
                    Some(other) if owner[*other as usize] != u32::MAX => {
                        *state.neighbours.entry(owner[*other as usize]).or_default() +=
                            adjacency.length(triangle, k);
                        perimeter += adjacency.length(triangle, k);
                    }
                    _ => perimeter += adjacency.length(triangle, k),
                }
            }
        }
        state.perimeter = perimeter;
    }

    // Merge, best pair first.
    let mut queue: BinaryHeap<Candidate> = BinaryHeap::new();
    for (id, state) in states.iter().enumerate() {
        for (&other, &shared) in &state.neighbours {
            if other > id as u32 {
                queue.push(Candidate::new(id as u32, other, shared, &states));
            }
        }
    }
    let mut tests = 0usize;
    while let Some(Candidate { a, b, .. }) = queue.pop() {
        if !states[a as usize].alive() || !states[b as usize].alive() {
            continue;
        }
        let size = states[a as usize].triangles.len() + states[b as usize].triangles.len();
        if size > MAX_CHART_TRIANGLES {
            continue;
        }
        tests += 1;
        if tests.is_multiple_of(CANCEL_CHECK_EVERY) && cancelled(cancel) {
            return None;
        }
        let Some(flat) = merge_test(mesh, &mut states, a, b, size, settings) else {
            continue;
        };
        let (first, second) = (&states[a as usize], &states[b as usize]);
        let id = states.len() as u32;
        let shared = first.neighbours.get(&b).copied().unwrap_or(0.0);
        let mut neighbours: BTreeMap<u32, f64> = BTreeMap::new();
        for (&other, &length) in first.neighbours.iter().chain(&second.neighbours) {
            if other != a && other != b {
                *neighbours.entry(other).or_default() += length;
            }
        }
        let mut triangles: Vec<u32> = first
            .triangles
            .iter()
            .chain(&second.triangles)
            .copied()
            .collect();
        triangles.sort_unstable();
        let merged = State {
            triangles,
            perimeter: first.perimeter + second.perimeter - 2.0 * shared,
            neighbours,
            flat: Some(flat),
            unflattenable: false,
            merged_into: None,
        };
        for (&other, &length) in &merged.neighbours {
            let neighbour = &mut states[other as usize].neighbours;
            neighbour.remove(&a);
            neighbour.remove(&b);
            neighbour.insert(id, length);
        }
        for gone in [a, b] {
            let gone = &mut states[gone as usize];
            gone.merged_into = Some(id);
            gone.flat = None;
            gone.neighbours.clear();
        }
        states.push(merged);
        let pushed: Vec<(u32, f64)> = states[id as usize]
            .neighbours
            .iter()
            .map(|(&other, &length)| (other, length))
            .collect();
        for (other, length) in pushed {
            queue.push(Candidate::new(id, other, length, &states));
        }
    }

    // Every chart left, flattened (an atom that never merged has not been
    // yet) and refined; one that will not flatten, as its triangles.
    for state in states.into_iter().filter(State::alive) {
        if cancelled(cancel) {
            return None;
        }
        let flat = state
            .flat
            .or_else(|| flatten(mesh, &state.triangles, settings.flattening));
        match flat {
            Some(flat) => {
                let flat = finish(mesh, &state.triangles, flat, settings);
                done.push(flat.into_chart(mesh, state.triangles));
            }
            None => done.extend(state.triangles.into_iter().map(|triangle| {
                match flatten(mesh, &[triangle], settings.flattening) {
                    Some(flat) => flat.into_chart(mesh, vec![triangle]),
                    // No area to flatten.
                    None => Chart {
                        triangles: vec![triangle],
                        uvs: vec![[Vec2::ZERO; 3]],
                    },
                }
            })),
        }
    }
    // In order of first triangle, so the layout's order is the mesh's.
    done.sort_by_key(|chart| chart.triangles[0]);
    Some(done)
}

/// The atoms of every triangle `skip` leaves: each flooded out across edges,
/// in triangle order, from the first triangle not yet in one, taking a
/// neighbour whose normal is within [`ATOM_CONE_DEGREES`] of the seed's (or
/// that has no normal, having no area), up to [`ATOM_TRIANGLES`].
fn grow_atoms(mesh: &CutMesh<'_>, adjacency: &Adjacency, skip: &[bool]) -> Vec<Vec<u32>> {
    let cone = ATOM_CONE_DEGREES.to_radians().cos();
    let normal: Vec<DVec3> = mesh
        .triangles
        .iter()
        .map(|corners| {
            let [a, b, c] = corners.map(|vertex| mesh.positions[vertex as usize].as_dvec3());
            (b - a).cross(c - a).normalize_or_zero()
        })
        .collect();
    let mut taken = skip.to_vec();
    let mut atoms = Vec::new();
    let mut queue = VecDeque::new();
    for seed in 0..mesh.triangles.len() {
        if taken[seed] {
            continue;
        }
        let reference = normal[seed];
        let mut atom = vec![seed as u32];
        taken[seed] = true;
        queue.clear();
        queue.push_back(seed as u32);
        'grow: while let Some(triangle) = queue.pop_front() {
            for &(other, _) in adjacency.of(triangle) {
                if atom.len() >= ATOM_TRIANGLES {
                    break 'grow;
                }
                let at = other as usize;
                if taken[at] {
                    continue;
                }
                let flat = normal[at] == DVec3::ZERO || reference == DVec3::ZERO;
                if flat || normal[at].dot(reference) >= cone {
                    taken[at] = true;
                    atom.push(other);
                    queue.push_back(other);
                }
            }
        }
        atom.sort_unstable();
        atoms.push(atom);
    }
    atoms
}

/// Test merging charts `a` and `b` (`size` triangles together): the merged
/// chart's flattening, if it is valid and within the limit.
fn merge_test(
    mesh: &CutMesh<'_>,
    states: &mut [State],
    a: u32,
    b: u32,
    size: usize,
    settings: Settings,
) -> Option<Flat> {
    if size <= WHOLE_TEST_TRIANGLES {
        let mut merged: Vec<u32> = states[a as usize]
            .triangles
            .iter()
            .chain(&states[b as usize].triangles)
            .copied()
            .collect();
        merged.sort_unstable();
        let flat = flatten(mesh, &merged, settings.flattening)?;
        return settings.accepts(flat.sums).then_some(flat);
    }
    // The larger chart held, the smaller flattened onto it.
    let (large, small) = if states[a as usize].triangles.len() >= states[b as usize].triangles.len()
    {
        (a, b)
    } else {
        (b, a)
    };
    if states[large as usize].flat.is_none() {
        if states[large as usize].unflattenable {
            return None;
        }
        let flat = flatten(mesh, &states[large as usize].triangles, settings.flattening);
        states[large as usize].unflattenable = flat.is_none();
        states[large as usize].flat = Some(flat?);
    }
    let onto = states[large as usize].flat.as_ref()?;
    let attached = attach(
        mesh,
        onto,
        &states[small as usize].triangles,
        settings.flattening,
    )?;
    let sums = onto.sums.plus(attached.sums);
    if !settings.accepts(sums) {
        return None;
    }
    let mut merged = states[large as usize].flat.take()?;
    merged.uvs.extend(attached.uvs);
    for edge in &attached.shared {
        merged.boundary.remove(edge);
    }
    merged.boundary.extend(attached.border);
    merged.sums = sums;
    Some(merged)
}

/// A chart during the merge.
#[derive(Default)]
struct State {
    /// Sorted.
    triangles: Vec<u32>,
    perimeter: f64,
    /// Neighbouring charts, by the length of border shared with each.
    neighbours: BTreeMap<u32, f64>,
    flat: Option<Flat>,
    /// Tried and failed to flatten on its own: nothing can be attached to it.
    unflattenable: bool,
    merged_into: Option<u32>,
}

impl State {
    fn alive(&self) -> bool {
        self.merged_into.is_none()
    }
}

/// A pair of neighbouring charts worth trying as one: the more of the smaller
/// one's border they share, the sooner.
struct Candidate {
    priority: f64,
    a: u32,
    b: u32,
}

impl Candidate {
    fn new(a: u32, b: u32, shared: f64, states: &[State]) -> Self {
        let smaller = states[a as usize]
            .perimeter
            .min(states[b as usize].perimeter)
            .max(1.0e-30);
        Self {
            priority: shared / smaller,
            a: a.min(b),
            b: a.max(b),
        }
    }
}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    /// Highest priority first; between equals, the lowest ids.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .total_cmp(&other.priority)
            .then_with(|| other.a.cmp(&self.a))
            .then_with(|| other.b.cmp(&self.b))
    }
}

/// Which triangle is across each edge of each triangle, in the cut surface.
struct Adjacency {
    /// Per triangle, per edge (corner k to k + 1), the triangle across it.
    across: Vec<[Option<u32>; 3]>,
    /// Per triangle, per edge, its length.
    lengths: Vec<[f64; 3]>,
    /// Per triangle, its neighbours across edges, with the edges' lengths.
    neighbours: Vec<Vec<(u32, f64)>>,
}

impl Adjacency {
    fn new(mesh: &CutMesh<'_>) -> Self {
        let mut users: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
        for (triangle, corners) in mesh.triangles.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (corners[k], corners[(k + 1) % 3]);
                if a != b {
                    users
                        .entry((a.min(b), a.max(b)))
                        .or_default()
                        .push(triangle as u32);
                }
            }
        }
        let mut across = vec![[None; 3]; mesh.triangles.len()];
        let mut lengths = vec![[0.0; 3]; mesh.triangles.len()];
        for (triangle, corners) in mesh.triangles.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (corners[k], corners[(k + 1) % 3]);
                lengths[triangle][k] =
                    f64::from(mesh.positions[a as usize].distance(mesh.positions[b as usize]));
                if let Some(&[x, y]) = users.get(&(a.min(b), a.max(b))).map(Vec::as_slice) {
                    let other = if x == triangle as u32 { y } else { x };
                    if other != triangle as u32 {
                        across[triangle][k] = Some(other);
                    }
                }
            }
        }
        let neighbours = across
            .iter()
            .zip(&lengths)
            .map(|(edges, lengths)| {
                edges
                    .iter()
                    .zip(lengths)
                    .filter_map(|(edge, &length)| edge.map(|other| (other, length)))
                    .collect()
            })
            .collect();
        Self {
            across,
            lengths,
            neighbours,
        }
    }

    fn of(&self, triangle: u32) -> &[(u32, f64)] {
        &self.neighbours[triangle as usize]
    }

    fn edges(&self, triangle: u32) -> &[Option<u32>; 3] {
        &self.across[triangle as usize]
    }

    fn length(&self, triangle: u32, edge: usize) -> f64 {
        self.lengths[triangle as usize][edge]
    }
}

/// A chart's stretch, summed over its triangles: what the mean is taken from.
#[derive(Clone, Copy, Default)]
struct Sums {
    /// Area-weighted stretch.
    stretch: f64,
    /// 3D area.
    area: f64,
    /// The worst triangle's σ₁/σ₂.
    worst: f64,
}

impl Sums {
    fn mean(self) -> f64 {
        if self.area > 0.0 {
            self.stretch / self.area
        } else {
            f64::MAX
        }
    }

    fn plus(self, other: Sums) -> Sums {
        Sums {
            stretch: self.stretch + other.stretch,
            area: self.area + other.area,
            worst: self.worst.max(other.worst),
        }
    }
}

/// A border edge: its two cut vertices (lower first) and their UVs.
type Border = BTreeMap<(u32, u32), [Vec2; 2]>;

/// A chart flattened: its vertices' UVs, its border, its stretch.
struct Flat {
    /// Per cut vertex of the chart, its UV.
    uvs: BTreeMap<u32, Vec2>,
    /// The border's edges, with their ends' UVs.
    boundary: Border,
    sums: Sums,
}

impl Flat {
    fn into_chart(self, mesh: &CutMesh<'_>, triangles: Vec<u32>) -> Chart {
        Chart {
            uvs: triangles
                .iter()
                .map(|&triangle| {
                    mesh.triangles[triangle as usize]
                        .map(|vertex| self.uvs.get(&vertex).copied().unwrap_or_default())
                })
                .collect(),
            triangles,
        }
    }
}

/// A chart's own mesh: its cut vertices, their positions, its triangles over
/// them, and its border.
struct ChartMesh {
    vertices: Vec<u32>,
    points: Vec<Vec3>,
    local: Vec<[u32; 3]>,
    /// The border's edges, in local vertices.
    boundary: Vec<(u32, u32)>,
}

impl ChartMesh {
    /// The chart's mesh, or `None` when it is not a disc: connected through
    /// its edges (as every chart here is), every edge used once or twice, one
    /// border loop through no vertex twice, Euler characteristic 1.
    fn new(mesh: &CutMesh<'_>, triangles: &[u32]) -> Option<ChartMesh> {
        // The chart's own vertices, in order of first corner.
        let mut number: BTreeMap<u32, u32> = BTreeMap::new();
        let mut vertices: Vec<u32> = Vec::new();
        let local: Vec<[u32; 3]> = triangles
            .iter()
            .map(|&triangle| {
                mesh.triangles[triangle as usize].map(|vertex| {
                    *number.entry(vertex).or_insert_with(|| {
                        vertices.push(vertex);
                        vertices.len() as u32 - 1
                    })
                })
            })
            .collect();
        let boundary = disc_boundary(&local, vertices.len())?;
        let points = vertices
            .iter()
            .map(|&vertex| mesh.positions[vertex as usize])
            .collect();
        Some(ChartMesh {
            vertices,
            points,
            local,
            boundary,
        })
    }

    /// A border edge in cut vertices, lower first.
    fn edge(&self, (a, b): (u32, u32)) -> (u32, u32) {
        let (a, b) = (self.vertices[a as usize], self.vertices[b as usize]);
        (a.min(b), a.max(b))
    }

    /// A border edge in cut vertices, lower first, with its ends' UVs from
    /// `uvs` (one per local vertex) in the same order.
    fn border_edge(&self, (a, b): (u32, u32), uvs: &[Vec2]) -> ((u32, u32), [Vec2; 2]) {
        let (x, y) = (self.vertices[a as usize], self.vertices[b as usize]);
        let (p, q) = (uvs[a as usize], uvs[b as usize]);
        if x <= y {
            ((x, y), [p, q])
        } else {
            ((y, x), [q, p])
        }
    }
}

/// Flatten `triangles` as one chart, or `None` where they cannot be: not a
/// disc, or a flattening that folds or overlaps itself.
fn flatten(mesh: &CutMesh<'_>, triangles: &[u32], flattening: Flattening) -> Option<Flat> {
    let chart = ChartMesh::new(mesh, triangles)?;
    // Pin two border vertices far apart, a 3D distance apart along U: the
    // first border vertex's farthest, and that one's farthest.
    let border: BTreeSet<u32> = chart.boundary.iter().flat_map(|&(a, b)| [a, b]).collect();
    let farthest = |from: u32| {
        border
            .iter()
            .copied()
            .max_by(|&x, &y| {
                let p = chart.points[from as usize];
                p.distance_squared(chart.points[x as usize])
                    .total_cmp(&p.distance_squared(chart.points[y as usize]))
                    .then(y.cmp(&x))
            })
            .unwrap_or(from)
    };
    let one = farthest(*border.first()?);
    let two = farthest(one);
    let length = chart.points[one as usize].distance(chart.points[two as usize]);
    let pins = [
        (one as usize, Vec2::ZERO),
        (two as usize, Vec2::new(length, 0.0)),
    ];
    let mut uvs = super::lscm::flatten(&chart.points, &chart.local, &pins)?;
    if flattening == Flattening::MinimumStretch {
        super::arap::refine(&chart.points, &chart.local, &mut uvs, TEST_PASSES, None);
    }
    measure(&chart, uvs)
}

/// A chart flattened for the merge's test, refined the rest of the way: the
/// remaining as-rigid-as-possible passes over the whole chart, kept where they
/// leave it valid and within the limit (or, for a chart that was never within
/// it, no worse), and scaled to its area.
fn finish(mesh: &CutMesh<'_>, triangles: &[u32], flat: Flat, settings: Settings) -> Flat {
    let Some(chart) = ChartMesh::new(mesh, triangles) else {
        return flat;
    };
    let mut uvs: Vec<Vec2> = chart
        .vertices
        .iter()
        .map(|vertex| flat.uvs.get(vertex).copied().unwrap_or_default())
        .collect();
    if settings.flattening != Flattening::MinimumStretch {
        return measure(&chart, uvs).unwrap_or(flat);
    }
    super::arap::refine(
        &chart.points,
        &chart.local,
        &mut uvs,
        super::arap::PASSES - TEST_PASSES,
        None,
    );
    match measure(&chart, uvs) {
        Some(refined)
            if settings.accepts(refined.sums)
                || (!settings.accepts(flat.sums) && refined.sums.mean() <= flat.sums.mean()) =>
        {
            refined
        }
        _ => flat,
    }
}

/// Check a whole chart's flattening, mirror it to keep the winding, scale it
/// to its area and measure its stretch: `None` where it folds or overlaps
/// itself.
fn measure(chart: &ChartMesh, mut uvs: Vec<Vec2>) -> Option<Flat> {
    let elements = elements(&chart.points, &chart.local);
    let area3: f64 = elements.iter().map(|element| element.area).sum();
    let area2: f64 = elements.iter().map(|element| signed(element, &uvs)).sum();
    if area3 <= 0.0 || area2 == 0.0 {
        return None;
    }
    if area2 < 0.0 {
        for uv in &mut uvs {
            uv.x = -uv.x;
        }
    }
    let scale = (area3 / area2.abs()).sqrt();
    for uv in &mut uvs {
        *uv = (uv.as_dvec2() * scale).as_vec2();
    }
    let sums = stretch(&elements, &uvs)?;
    let edges: Vec<Edge> = chart
        .boundary
        .iter()
        .map(|&(a, b)| Edge {
            ends: [uvs[a as usize], uvs[b as usize]],
            vertices: [a, b],
        })
        .collect();
    if crosses(&edges, &edges) {
        return None;
    }
    Some(Flat {
        uvs: chart
            .vertices
            .iter()
            .copied()
            .zip(uvs.iter().copied())
            .collect(),
        boundary: chart
            .boundary
            .iter()
            .map(|&edge| chart.border_edge(edge, &uvs))
            .collect(),
        sums,
    })
}

/// The smaller chart of a merge, flattened onto the larger.
struct Attached {
    /// The UVs of its vertices the larger chart has not.
    uvs: Vec<(u32, Vec2)>,
    /// The border edges the two charts share, which the merge closes.
    shared: Vec<(u32, u32)>,
    /// Its border edges the larger chart has not, with their ends' UVs.
    border: Vec<((u32, u32), [Vec2; 2])>,
    sums: Sums,
}

/// Flatten `triangles` onto `onto`, their shared border pinned where `onto`
/// has it: `None` where the two would not be a disc together, or the result
/// folds or crosses itself.
fn attach(
    mesh: &CutMesh<'_>,
    onto: &Flat,
    triangles: &[u32],
    flattening: Flattening,
) -> Option<Attached> {
    let chart = ChartMesh::new(mesh, triangles)?;
    let own: Vec<(u32, u32)> = chart
        .boundary
        .iter()
        .map(|&edge| chart.edge(edge))
        .collect();
    // One stretch of shared border, and no other vertex in common: two discs
    // glued along it are a disc.
    let shared: Vec<(u32, u32)> = own
        .iter()
        .copied()
        .filter(|edge| onto.boundary.contains_key(edge))
        .collect();
    let common: BTreeSet<u32> = chart
        .vertices
        .iter()
        .copied()
        .filter(|vertex| onto.uvs.contains_key(vertex))
        .collect();
    if shared.is_empty() || !is_path(&shared, &common) {
        return None;
    }
    let pins: Vec<(usize, Vec2)> = chart
        .vertices
        .iter()
        .enumerate()
        .filter_map(|(local, vertex)| onto.uvs.get(vertex).map(|&uv| (local, uv)))
        .collect();
    let mut uvs = super::lscm::flatten(&chart.points, &chart.local, &pins)?;
    if flattening == Flattening::MinimumStretch {
        let mut held = vec![false; chart.vertices.len()];
        for &(local, _) in &pins {
            held[local] = true;
        }
        super::arap::refine(
            &chart.points,
            &chart.local,
            &mut uvs,
            TEST_PASSES,
            Some(&held),
        );
    }
    let elements = elements(&chart.points, &chart.local);
    let sums = stretch(&elements, &uvs)?;

    // The new border edges may cross nothing: not each other, and not the
    // larger chart's border beyond what the two share. Its own edges are
    // known not to cross, and only those near the new ones are compared.
    let shared_set: BTreeSet<(u32, u32)> = shared.iter().copied().collect();
    let new: Vec<Edge> = chart
        .boundary
        .iter()
        .filter(|&&edge| !shared_set.contains(&chart.edge(edge)))
        .map(|&edge| {
            let ((x, y), ends) = chart.border_edge(edge, &uvs);
            Edge {
                ends,
                vertices: [x, y],
            }
        })
        .collect();
    let (low, high) = new.iter().flat_map(|edge| edge.ends).fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(low, high), uv| (low.min(uv), high.max(uv)),
    );
    let near: Vec<Edge> = onto
        .boundary
        .iter()
        .filter(|(edge, _)| !shared_set.contains(edge))
        .filter(|(_, [a, b])| a.min(*b).cmple(high).all() && a.max(*b).cmpge(low).all())
        .map(|(&(x, y), &ends)| Edge {
            ends,
            vertices: [x, y],
        })
        .collect();
    if crosses(&new, &new) || crosses(&new, &near) {
        return None;
    }
    Some(Attached {
        uvs: chart
            .vertices
            .iter()
            .zip(&uvs)
            .filter(|(vertex, _)| !onto.uvs.contains_key(vertex))
            .map(|(&vertex, &uv)| (vertex, uv))
            .collect(),
        shared,
        border: new
            .iter()
            .map(|edge| ((edge.vertices[0], edge.vertices[1]), edge.ends))
            .collect(),
        sums,
    })
}

/// Whether `edges` form one open path whose vertices are exactly `vertices`.
fn is_path(edges: &[(u32, u32)], vertices: &BTreeSet<u32>) -> bool {
    let mut degree: BTreeMap<u32, u32> = BTreeMap::new();
    for &(a, b) in edges {
        *degree.entry(a).or_default() += 1;
        *degree.entry(b).or_default() += 1;
    }
    let ends = degree.values().filter(|&&count| count == 1).count();
    // With every degree at most 2 and two ends, the edges are paths and loops;
    // a path of n edges has n + 1 vertices, so one path and nothing else.
    degree.values().all(|&count| count <= 2)
        && ends == 2
        && degree.len() == edges.len() + 1
        && degree.keys().eq(vertices.iter())
}

/// A triangle's signed UV area.
fn signed(element: &Element, uvs: &[Vec2]) -> f64 {
    let [a, b, c] = element.corners.map(|corner| uvs[corner].as_dvec2());
    0.5 * (b - a).perp_dot(c - a)
}

/// The stretch of a flattening, or `None` where a triangle is folded or
/// collapsed.
fn stretch(elements: &[Element], uvs: &[Vec2]) -> Option<Sums> {
    let mut sums = Sums {
        worst: 1.0,
        ..Sums::default()
    };
    for element in elements {
        if signed(element, uvs) <= COLLAPSED * element.area {
            return None;
        }
        let (mut row_u, mut row_v) = (DVec2::ZERO, DVec2::ZERO);
        for k in 0..3 {
            let uv = uvs[element.corners[k]].as_dvec2();
            row_u += uv.x * element.gradients[k];
            row_v += uv.y * element.gradients[k];
        }
        let (large, small) = singular_values(row_u.x, row_u.y, row_v.x, row_v.y);
        sums.stretch += element.area * large.max(1.0 / small.max(1.0e-12));
        sums.area += element.area;
        sums.worst = sums.worst.max(large / small.max(1.0e-12));
    }
    Some(sums)
}

/// The singular values of `[[a, b], [c, d]]`, larger first.
fn singular_values(a: f64, b: f64, c: f64, d: f64) -> (f64, f64) {
    let energy = 0.5 * (a * a + b * b + c * c + d * d);
    let determinant = a * d - b * c;
    let root = (energy * energy - determinant * determinant)
        .max(0.0)
        .sqrt();
    ((energy + root).sqrt(), (energy - root).max(0.0).sqrt())
}

/// The border edges of a chart that is a disc, or `None` for any other shape.
fn disc_boundary(local: &[[u32; 3]], vertex_count: usize) -> Option<Vec<(u32, u32)>> {
    let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for corners in local {
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            if a == b {
                return None;
            }
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    if edges.values().any(|&users| users > 2) {
        return None;
    }
    let euler = vertex_count as i64 - edges.len() as i64 + local.len() as i64;
    if euler != 1 {
        return None;
    }
    let open: Vec<(u32, u32)> = edges
        .iter()
        .filter(|&(_, &users)| users == 1)
        .map(|(&edge, _)| edge)
        .collect();
    // One loop through every border vertex, each on it once.
    let mut next: Vec<[u32; 2]> = vec![[u32::MAX; 2]; vertex_count];
    let mut degree = vec![0usize; vertex_count];
    for &(a, b) in &open {
        for (from, to) in [(a, b), (b, a)] {
            let slot = degree[from as usize];
            if slot >= 2 {
                return None;
            }
            next[from as usize][slot] = to;
            degree[from as usize] += 1;
        }
    }
    let &(start, _) = open.first()?;
    let (mut previous, mut at, mut walked) = (start, next[start as usize][0], 1);
    while at != start {
        if walked > open.len() {
            return None;
        }
        let [x, y] = next[at as usize];
        let step = if x == previous { y } else { x };
        previous = at;
        at = step;
        walked += 1;
    }
    (walked == open.len()).then_some(open)
}

/// A border edge: its ends' UVs and the vertices they belong to.
#[derive(Clone, Copy)]
struct Edge {
    ends: [Vec2; 2],
    vertices: [u32; 2],
}

/// Whether any edge of `these` crosses any edge of `those` (the same list, to
/// test one border against itself). Edges that share a vertex are neighbours
/// along a border and are not compared. `those` is bucketed on a grid, so a
/// long border costs little more than a short one.
fn crosses(these: &[Edge], those: &[Edge]) -> bool {
    if these.is_empty() || those.is_empty() {
        return false;
    }
    let (low, high) = those.iter().flat_map(|edge| edge.ends).fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(low, high), uv| (low.min(uv.as_dvec2()), high.max(uv.as_dvec2())),
    );
    let side = (those.len() as f64).sqrt().ceil().max(1.0);
    let cell = ((high - low) / side).max(DVec2::splat(1.0e-12));
    let last = side as i64 - 1;
    let cells = |edge: &Edge| {
        let [a, b] = edge.ends.map(|uv| uv.as_dvec2());
        let at = |point: DVec2| ((point - low) / cell).floor();
        let (from, to) = (at(a.min(b)), at(a.max(b)));
        (
            (from.x as i64).clamp(0, last)..=(to.x as i64).clamp(0, last),
            (from.y as i64).clamp(0, last)..=(to.y as i64).clamp(0, last),
        )
    };
    let mut grid: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (index, edge) in those.iter().enumerate() {
        let (xs, ys) = cells(edge);
        for x in xs {
            for y in ys.clone() {
                grid.entry((x, y)).or_default().push(index);
            }
        }
    }
    for edge in these {
        let (xs, ys) = cells(edge);
        for x in xs {
            for y in ys.clone() {
                for &index in grid.get(&(x, y)).map_or(&[][..], Vec::as_slice) {
                    let other = &those[index];
                    if edge
                        .vertices
                        .iter()
                        .any(|vertex| other.vertices.contains(vertex))
                    {
                        continue;
                    }
                    let [a, b] = edge.ends.map(|uv| uv.as_dvec2());
                    let [c, d] = other.ends.map(|uv| uv.as_dvec2());
                    if segments_cross(a, b, c, d) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Whether segments `ab` and `cd` cross, touching included.
fn segments_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let side = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    (d1 * d2 <= 0.0) && (d3 * d4 <= 0.0) && !(d1 == 0.0 && d2 == 0.0 && d3 == 0.0 && d4 == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube's eight corners and twelve triangles, wound outwards.
    fn cube() -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let positions = (0..8)
            .map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32))
            .collect();
        let quads = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let triangles = quads
            .iter()
            .flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]])
            .collect();
        (positions, triangles)
    }

    fn run(positions: &[Vec3], triangles: &[[u32; 3]], max_stretch: f32) -> Vec<Chart> {
        let island = vec![0; triangles.len()];
        let mesh = CutMesh {
            positions,
            triangles,
            island: &island,
        };
        let settings = Settings {
            flattening: Flattening::MinimumStretch,
            max_stretch,
        };
        charts(&mesh, settings, None).expect("not cancelled")
    }

    /// A closed shape cannot be flattened whole: it is cut into discs, every
    /// triangle in one of them once, none folded, the cube's faces unfolded
    /// without stretch.
    #[test]
    fn a_closed_cube_is_cut_into_flat_discs() {
        let (positions, triangles) = cube();
        let charts = run(&positions, &triangles, 1.1);
        assert!(charts.len() >= 2, "a closed shell needs a cut");
        let mut seen = vec![0; triangles.len()];
        for chart in &charts {
            for (&triangle, uv) in chart.triangles.iter().zip(&chart.uvs) {
                seen[triangle as usize] += 1;
                let [a, b, c] = *uv;
                let area = (b - a).perp_dot(c - a) * 0.5;
                assert!(
                    (area - 0.5).abs() < 1.0e-3,
                    "triangle {triangle}: UV area {area}, 3D area 0.5"
                );
            }
        }
        assert!(seen.iter().all(|&count| count == 1), "{seen:?}");
    }

    /// A band all the way round (the cube's four sides) is a ring, never a
    /// disc: no chart holds all of it.
    #[test]
    fn a_ring_is_never_one_chart() {
        let (positions, triangles) = cube();
        let sides: Vec<[u32; 3]> = triangles[4..].to_vec();
        let charts = run(&positions, &sides, 2.0);
        assert!(
            charts
                .iter()
                .all(|chart| chart.triangles.len() < sides.len())
        );
    }

    #[test]
    fn a_path_is_one_open_run_of_edges() {
        let vertices = |list: &[u32]| list.iter().copied().collect::<BTreeSet<u32>>();
        assert!(is_path(&[(1, 2), (2, 3)], &vertices(&[1, 2, 3])));
        assert!(
            !is_path(&[(1, 2), (2, 3), (1, 3)], &vertices(&[1, 2, 3])),
            "a loop"
        );
        assert!(
            !is_path(&[(1, 2), (3, 4)], &vertices(&[1, 2, 3, 4])),
            "two runs"
        );
        assert!(
            !is_path(&[(1, 2), (2, 3)], &vertices(&[1, 2, 3, 9])),
            "a vertex off it"
        );
    }

    #[test]
    fn crossing_segments_cross_and_neighbours_do_not_count() {
        let edge = |a: [f32; 2], b: [f32; 2], vertices: [u32; 2]| Edge {
            ends: [Vec2::from(a), Vec2::from(b)],
            vertices,
        };
        let x = [
            edge([0.0, 0.0], [1.0, 1.0], [0, 1]),
            edge([0.0, 1.0], [1.0, 0.0], [2, 3]),
        ];
        assert!(crosses(&x, &x));
        let corner = [
            edge([0.0, 0.0], [1.0, 0.0], [0, 1]),
            edge([1.0, 0.0], [1.0, 1.0], [1, 2]),
        ];
        assert!(!crosses(&corner, &corner), "neighbours share a vertex");
    }
}
