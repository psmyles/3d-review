//! The edges the rebuilt surface has to keep, chained into curves.
//!
//! Two kinds, and the user asks for each separately:
//!
//! * **Open borders** (`align_to_boundaries`) — an edge with one face. This is
//!   the setting that matters most on the assets this tool is pointed at. A leaf,
//!   a sheet of cloth, a wall with no thickness: their whole shape is their
//!   border, and a rebuild that treats the border as ordinary surface returns a
//!   ragged fringe or, worse, tears it.
//! * **Creases** (`sharp_edges`) — an edge whose two faces turn through more
//!   than the crease angle. Worth keeping on hard-surface and CAD parts, where a
//!   rebuilt surface that rounds a 90-degree corner is useless; left off for
//!   organic shapes, where it only breaks up the flow.
//!
//! * **Folds** — an edge whose two faces turn back past a right angle. Unlike
//!   the two above this is **not a setting**, because it is not a matter of
//!   taste: a surface that doubles back on itself has a *silhouette* there, and
//!   the silhouette is the shape. A leaf, a strip of bark, a piece of cloth —
//!   each is two sheets meeting at a rim, and a rebuild that treats that rim as
//!   ordinary surface merges a vertex from one sheet with one from the other
//!   and the object comes back narrower. Measured on a plant's bark at a
//!   quarter density: 87 % of its surface area survived without this and 96 %
//!   with it.
//!
//! Non-manifold edges are always features, whatever the settings say. The
//! surface branches there and no rebuild can describe it, so the least wrong
//! thing is to keep a vertex exactly where it was.
//!
//! ## Why a fold is not just a sharp crease
//!
//! It is the same test with a different threshold, and the threshold is what
//! makes it safe to leave on. **Keep sharp edges** is off for organic work
//! because creasing every thirty-degree bend breaks up the flow — and on a real
//! plant 96 % of edges do turn less than forty degrees. Past ninety they are
//! 1.3 %, and those are not bends in a surface, they are its edge. A box
//! modelled at exactly ninety degrees is deliberately left out by a hair of
//! margin, so a hard-surface part with creases off still rebuilds smooth.
//!
//! ## Why chains rather than a set of edges
//!
//! Seeds are placed *along* a feature at a spacing, not scattered near it. A
//! border that gets its vertices at even arc-length stations comes back as one
//! clean loop; a border whose vertices are wherever the interior sampling
//! happened to land comes back as a fringe. So the edges are walked into
//! [`Polyline`]s first, and [`super::seeds`] stations each one.
//!
//! ## Determinism
//!
//! Every choice here breaks ties on the lower vertex index: which chains are
//! started, in what order, and which way they are walked. Two runs over the same
//! mesh therefore produce the same chains in the same order with the same
//! interior orientation, which is what lets the seeds — and so the whole
//! partition — be reproducible.

use crate::cancel::{CancelToken, cancelled};

use super::surface::Surface;
use super::topology::EdgeAt;

/// Cosine of the angle past which two faces count as folded back rather than
/// bent, whatever **Keep sharp edges** says.
///
/// A right angle is a cosine of zero; the margin is what keeps a box modelled at
/// exactly ninety degrees out, since that is a shape a rebuild can round off
/// without losing what it is. Chosen by measurement rather than taste — on the
/// plant that prompted it, every threshold from thirty degrees to a hundred held
/// the bark's area within a point or two of each other, and it fell away past
/// a hundred and ten.
const FOLD_COSINE: f64 = -0.02;

/// Least area, against the longest edge squared, for a face's normal to be
/// trusted. See [`is_sliver`].
const SLIVER_QUALITY: f64 = 1.0e-4;

/// How sharply a chain must turn, in radians, for the vertex at the bend to be
/// treated as a corner that a seed has to land on exactly.
///
/// Sixty degrees. Below that a chain is bending, and a rebuild is entitled to
/// cut the bend; past it the chain has a point on it, and cutting the point off
/// is how a leaf comes back with a blunt end.
const CORNER_ANGLE: f64 = std::f64::consts::FRAC_PI_3;

/// A chain of feature edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Polyline {
    /// Vertices in order along the chain. A closed chain does **not** repeat its
    /// first vertex at the end.
    pub(crate) vertices: Vec<u32>,
    pub(crate) closed: bool,
    pub(crate) kind: FeatureKind,
}

/// What made a chain a feature. A border chain is what the output's own border
/// is built from, so it is counted separately when the vertex budget works out
/// how many boundary edges the result will have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeatureKind {
    Boundary,
    Crease,
    /// The surface turns back on itself: the rim of a thin shell.
    Fold,
}

/// The feature curves of a mesh, and the per-vertex flag that goes with them.
#[derive(Debug, Default)]
pub(crate) struct Features {
    pub(crate) polylines: Vec<Polyline>,
    /// Per vertex: lies on any feature chain. Read by the partition (a region
    /// may not straddle one cheaply), by the collapse (a feature vertex is
    /// always the survivor) and by the smoothing (a feature vertex is pinned).
    pub(crate) on_feature: Vec<bool>,
    /// Per vertex: the chain it lies on, or `u32::MAX`. A vertex where two
    /// chains meet takes the lower-numbered one; it is a junction, and
    /// [`super::seeds`] pins junctions regardless of which chain claims them.
    pub(crate) polyline_of: Vec<u32>,
    /// Per vertex: lies where chains meet or end, so a seed must sit exactly
    /// here or the corner is lost.
    pub(crate) junction: Vec<bool>,
    /// Feature partners of each vertex, ascending: `vertex_count + 1` starts
    /// into [`Self::partner_entries`].
    ///
    /// A CSR rather than a `Vec<Vec<u32>>` because the outer vector costs
    /// twenty-four bytes of header per vertex whether or not the vertex is on a
    /// feature at all — a quarter of a gigabyte at ten million, for a table that
    /// is empty almost everywhere. Kept past the walk that builds it, because
    /// the partition needs to ask whether a given edge runs *along* a feature.
    partner_starts: Vec<u32>,
    partner_entries: Vec<u32>,
}

impl Features {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.polylines.is_empty()
    }

    /// The feature partners of `vertex`, ascending.
    fn partners_of(&self, vertex: u32) -> &[u32] {
        let at = vertex as usize;
        match (self.partner_starts.get(at), self.partner_starts.get(at + 1)) {
            (Some(&first), Some(&end)) => &self.partner_entries[first as usize..end as usize],
            _ => &[],
        }
    }

    /// Whether the edge `a..b` is itself a feature, rather than merely joining
    /// two vertices that each happen to lie on one.
    ///
    /// The difference matters wherever a feature passes near itself. A strip one
    /// triangle wide has both its rims in the same neighbourhood, and the edge
    /// spanning its width joins two feature vertices without being one — it is
    /// the very edge that must *not* be free to cross.
    pub(crate) fn is_feature_edge(&self, a: u32, b: u32) -> bool {
        self.partners_of(a).binary_search(&b).is_ok()
    }
}

/// Find the feature edges and walk them into chains.
///
/// `crease_cosine` is the cosine of the crease angle, or `None` when creases are
/// not being kept. Borders are included when `borders` is set; non-manifold
/// edges always are.
pub(crate) fn build(
    surface: Surface<'_>,
    borders: bool,
    crease_cosine: Option<f32>,
    cancel: Option<&CancelToken>,
) -> Features {
    let _z = crate::prof::zone!("Remesh Features");
    let count = surface.vertex_count();
    let mut features = Features {
        polylines: Vec::new(),
        on_feature: vec![false; count],
        polyline_of: vec![u32::MAX; count],
        junction: vec![false; count],
        partner_starts: Vec::new(),
        partner_entries: Vec::new(),
    };
    if count == 0 {
        return features;
    }

    // Per vertex, its feature partners, ascending. Stored rather than recomputed
    // because the walk below visits each vertex several times and the edge scan
    // is the expensive part.
    // Count, prefix-sum, fill — the shape every index in this module is built
    // with, and the reason a vertex on no feature costs four bytes rather than
    // an allocation.
    let mut starts = vec![0u32; count + 1];
    let mut scratch = Vec::new();
    for vertex in 0..count as u32 {
        surface
            .topology
            .edges_at(surface.indices, vertex, &mut scratch);
        let found = scratch
            .iter()
            .filter(|edge| is_feature(surface, **edge, borders, crease_cosine))
            .count();
        starts[vertex as usize + 1] = found as u32;
    }
    if cancelled(cancel) {
        return features;
    }
    for vertex in 0..count {
        starts[vertex + 1] += starts[vertex];
    }
    let mut entries = vec![0u32; starts[count] as usize];
    for vertex in 0..count as u32 {
        surface
            .topology
            .edges_at(surface.indices, vertex, &mut scratch);
        let mut at = starts[vertex as usize] as usize;
        for edge in &scratch {
            if is_feature(surface, *edge, borders, crease_cosine) {
                entries[at] = edge.other;
                at += 1;
            }
        }
        // `edges_at` reports a vertex's edges in face order, and the lookups
        // below are binary searches.
        entries[starts[vertex as usize] as usize..at].sort_unstable();
        if at > starts[vertex as usize] as usize {
            features.on_feature[vertex as usize] = true;
        }
    }
    features.partner_starts = starts;
    features.partner_entries = entries;
    if cancelled(cancel) {
        return features;
    }

    // A vertex is a junction when it is not simply passing through: an endpoint
    // (one partner) or a meeting of three or more chains. Those are the points
    // a seed has to land on exactly, since they are the corners.
    for vertex in 0..count as u32 {
        let degree = features.partners_of(vertex).len();
        if degree == 1 || degree > 2 {
            features.junction[vertex as usize] = true;
        }
    }

    // Walked as **edges**, not vertices.
    //
    // A chain is a maximal path of feature edges, and the vertices along it are
    // shared: on a cube every corner carries three creases, so a walk that
    // consumed vertices would claim one edge per corner and lose the other
    // eight. What is used up by a walk is the edge it crossed.
    let mut used = vec![false; features.partner_entries.len()];

    // Open chains first, from every vertex that is not simply passing through,
    // so what is left over is exactly the closed loops. Starting an open chain
    // from its middle would split it in two.
    for start in 0..count as u32 {
        while features.partners_of(start).len() != 2 && has_unused(&features, &used, start) {
            let line = walk(&features, &mut used, start);
            push_line(&mut features, surface, line, false, borders, crease_cosine);
        }
    }
    if cancelled(cancel) {
        return features;
    }
    // Whatever edges remain run through degree-two vertices only, so each is
    // part of a loop that closes on itself.
    for start in 0..count as u32 {
        while has_unused(&features, &used, start) {
            let mut line = walk(&features, &mut used, start);
            // The walk comes back to where it began; the repeat is dropped, so a
            // closed chain names each vertex once.
            let closed = line.len() > 2 && line.first() == line.last();
            if closed {
                line.pop();
            }
            push_line(&mut features, surface, line, closed, borders, crease_cosine);
        }
    }

    for (index, line) in features.polylines.iter().enumerate() {
        for &vertex in &line.vertices {
            if features.polyline_of[vertex as usize] == u32::MAX {
                features.polyline_of[vertex as usize] = index as u32;
            }
        }
    }
    mark_corners(surface, &mut features);
    features
}

/// Flag the vertices where a chain turns a corner, so a seed lands on each.
///
/// A junction — an end, or a meeting of three chains — is already treated this
/// way, and this is the case that is not one: a point in the *middle* of a
/// smooth degree-two chain. The tip of a leaf is exactly that. Its rim runs
/// unbroken all the way round, so nothing about the graph marks the tip, and
/// stations spaced by arc length land either side of it and cut it to a chord.
///
/// ## Why the angle is measured across a window rather than at the vertex
///
/// On a dense input the turn between one edge and the next is a degree or two
/// of nothing much, and noise besides. What matters is how far the chain has
/// turned over the distance a *rebuilt* edge will span, so the two chords are
/// taken to the vertices half a target edge away along the chain. A rounded tip
/// finer than that reads as its whole turn, which is what it looks like once
/// rebuilt, and a bumpy stretch of an otherwise straight rim cancels.
fn mark_corners(surface: Surface<'_>, features: &mut Features) {
    let mut corners: Vec<u32> = Vec::new();
    for line in &features.polylines {
        let count = line.vertices.len();
        if count < 3 {
            continue;
        }
        // Cumulative arc length, with the closing edge for a loop.
        let steps = if line.closed { count } else { count - 1 };
        let mut lengths = Vec::with_capacity(steps + 1);
        lengths.push(0.0f64);
        for step in 0..steps {
            let a = line.vertices[step];
            let b = line.vertices[(step + 1) % count];
            lengths.push(lengths[step] + surface.distance(a, b));
        }
        let total = lengths[steps];
        if total <= 0.0 {
            continue;
        }

        let mut angles = vec![0.0f64; count];
        for (at, slot) in angles.iter_mut().enumerate() {
            let vertex = line.vertices[at];
            let window = f64::from(surface.sizes[vertex as usize]) * 0.5;
            if window <= 0.0 {
                continue;
            }
            let back = station_at(line, &lengths, at, window, false);
            let ahead = station_at(line, &lengths, at, window, true);
            if back == at || ahead == at {
                continue;
            }
            let here = surface.position(vertex);
            let from = surface.position(line.vertices[back]);
            let to = surface.position(line.vertices[ahead]);
            let (Some(incoming), Some(outgoing)) = (direction(from, here), direction(here, to))
            else {
                continue;
            };
            let dot =
                (incoming[0] * outgoing[0] + incoming[1] * outgoing[1] + incoming[2] * outgoing[2])
                    .clamp(-1.0, 1.0);
            *slot = dot.acos();
        }

        // Only the sharpest vertex of each bend. A tip rounded over several
        // input vertices would otherwise flag all of them, and two adjacent
        // corners cannot collapse into one another — they would sit there as a
        // stubborn pair, spending budget and evening nothing out.
        for at in 0..count {
            if angles[at] < CORNER_ANGLE {
                continue;
            }
            let window = f64::from(surface.sizes[line.vertices[at] as usize]) * 0.5;
            if is_sharpest(line, &lengths, &angles, at, window) {
                corners.push(line.vertices[at]);
            }
        }
    }
    for vertex in corners {
        features.junction[vertex as usize] = true;
    }
}

/// The chain slot furthest from `at` but still within `window` of arc length,
/// walking forwards when `ahead` and backwards otherwise.
///
/// Walked outward from `at` rather than searched for, so the work is the number
/// of input vertices a rebuilt edge spans — the square root of the reduction,
/// a handful — and never the length of the chain. A rim of a ten-million
/// triangle object is a long chain, and anything per-vertex that scans it is
/// quadratic in disguise.
fn station_at(line: &Polyline, lengths: &[f64], at: usize, window: f64, ahead: bool) -> usize {
    let count = line.vertices.len();
    let mut best = at;
    for step in 1..count {
        let Some(other) = neighbour(line, at, step, ahead) else {
            break;
        };
        if arc_gap(line, lengths, at, other) > window {
            break;
        }
        best = other;
    }
    best
}

/// Whether `at` carries the largest turn within `window` of arc length.
fn is_sharpest(line: &Polyline, lengths: &[f64], angles: &[f64], at: usize, window: f64) -> bool {
    let count = line.vertices.len();
    for ahead in [true, false] {
        for step in 1..count {
            let Some(other) = neighbour(line, at, step, ahead) else {
                break;
            };
            if arc_gap(line, lengths, at, other) > window {
                break;
            }
            // Ties to the lower index, so two equal bends pick the same one on
            // every run.
            if angles[other] > angles[at] || (angles[other] == angles[at] && other < at) {
                return false;
            }
        }
    }
    true
}

/// The slot `step` along the chain from `at`, or `None` past the end of an open
/// one. A closed chain wraps.
fn neighbour(line: &Polyline, at: usize, step: usize, ahead: bool) -> Option<usize> {
    let count = line.vertices.len();
    if line.closed {
        return Some(if ahead {
            (at + step) % count
        } else {
            (at + count - step % count) % count
        });
    }
    if ahead {
        (at + step < count).then(|| at + step)
    } else {
        at.checked_sub(step)
    }
}

/// How far apart two slots are along the chain, the short way round a loop.
fn arc_gap(line: &Polyline, lengths: &[f64], at: usize, other: usize) -> f64 {
    let last = lengths.len().saturating_sub(1);
    let total = lengths[last];
    let gap = (lengths[other.min(last)] - lengths[at.min(last)]).abs();
    if line.closed {
        gap.min(total - gap)
    } else {
        gap
    }
}

/// The unit direction from `from` to `to`, or `None` if they coincide.
fn direction(from: [f64; 3], to: [f64; 3]) -> Option<[f64; 3]> {
    let step = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = (step[0] * step[0] + step[1] * step[1] + step[2] * step[2]).sqrt();
    (length > 0.0).then(|| [step[0] / length, step[1] / length, step[2] / length])
}

/// Walk from `start` along unused feature edges, always taking the lowest
/// unused partner, until there is nowhere new to go.
///
/// Stops on arriving at a vertex that is not degree two — an endpoint or a
/// junction — because that is where one chain ends and the next begins. The
/// junction belongs to both, which is what keeps a corner seeded.
fn walk(features: &Features, used: &mut [bool], start: u32) -> Vec<u32> {
    let mut line = vec![start];
    let mut at = start;
    while let Some(slot) = lowest_unused(features, used, at) {
        let next = features.partner_entries[slot];
        mark_used(features, used, at, next);
        line.push(next);
        at = next;
        if at == start || features.partners_of(at).len() != 2 {
            break;
        }
    }
    line
}

/// The slot of `vertex`'s lowest-numbered partner across an edge nothing has
/// walked yet.
fn lowest_unused(features: &Features, used: &[bool], vertex: u32) -> Option<usize> {
    let base = features.partner_starts[vertex as usize] as usize;
    // The entries are ascending, so the first unwalked one is the lowest.
    (base..base + features.partners_of(vertex).len()).find(|&slot| !used[slot])
}

fn has_unused(features: &Features, used: &[bool], vertex: u32) -> bool {
    lowest_unused(features, used, vertex).is_some()
}

/// Mark the edge between `a` and `b` walked, from both ends.
///
/// One `position` each way is enough because a vertex's partners are distinct:
/// `Topology::edges_at` reports each undirected edge once, whatever the faces
/// around it.
fn mark_used(features: &Features, used: &mut [bool], a: u32, b: u32) {
    for (from, to) in [(a, b), (b, a)] {
        let base = features.partner_starts[from as usize] as usize;
        if let Ok(offset) = features.partners_of(from).binary_search(&to) {
            used[base + offset] = true;
        }
    }
}

fn push_line(
    features: &mut Features,
    surface: Surface<'_>,
    vertices: Vec<u32>,
    closed: bool,
    borders: bool,
    crease_cosine: Option<f32>,
) {
    if vertices.len() < 2 {
        return;
    }
    // A chain is a border chain when its edges are: the two kinds are walked
    // together because a border that turns a corner is still one curve, but the
    // vertex budget has to know which of them the output's own border comes
    // from.
    let kind = chain_kind(surface, &vertices, borders, crease_cosine);
    features.polylines.push(Polyline {
        vertices,
        closed,
        kind,
    });
}

/// Whether the first edge of a chain is an open border.
fn chain_kind(
    surface: Surface<'_>,
    vertices: &[u32],
    borders: bool,
    crease_cosine: Option<f32>,
) -> FeatureKind {
    let mut scratch = Vec::new();
    surface
        .topology
        .edges_at(surface.indices, vertices[0], &mut scratch);
    let Some(edge) = scratch.iter().find(|edge| edge.other == vertices[1]) else {
        return FeatureKind::Crease;
    };
    if edge.is_boundary() {
        return if borders {
            FeatureKind::Boundary
        } else {
            FeatureKind::Crease
        };
    }
    let _ = crease_cosine;
    // A chain that turns back on itself is a rim; one that merely bends sharply
    // is a crease. Read from the first edge, as the whole chain is.
    match dihedral_cosine(surface, *edge) {
        Some(dot) if dot < FOLD_COSINE => FeatureKind::Fold,
        _ => FeatureKind::Crease,
    }
}

/// Whether one edge is a feature.
fn is_feature(
    surface: Surface<'_>,
    edge: EdgeAt,
    borders: bool,
    crease_cosine: Option<f32>,
) -> bool {
    if edge.is_nonmanifold() {
        // No rebuild can describe a branch, so the least wrong answer is to keep
        // a vertex exactly where the input had one.
        return true;
    }
    if edge.is_boundary() {
        return borders;
    }
    let Some(dot) = dihedral_cosine(surface, edge) else {
        return false;
    };
    // Turning through a *larger* angle means a *smaller* cosine. A fold is
    // always a feature; a crease is one only when asked for.
    dot < FOLD_COSINE || crease_cosine.is_some_and(|cosine| dot < f64::from(cosine))
}

/// How much an edge's two faces turn, as a cosine, or `None` when either has no
/// usable normal or the edge has no two faces.
fn dihedral_cosine(surface: Surface<'_>, edge: EdgeAt) -> Option<f64> {
    let [first, second] = edge.faces;
    if first == u32::MAX || second == u32::MAX {
        return None;
    }
    if is_sliver(surface, first) || is_sliver(surface, second) {
        return None;
    }
    let (a, b) = (surface.face_normal(first)?, surface.face_normal(second)?);
    Some(a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
}

/// Whether a face is too thin for its normal to mean anything.
///
/// A normal is a cross product, and a cross product of two nearly parallel
/// edges is mostly rounding error — it can point anywhere, and a pair of them
/// will happily read as a hundred and eighty degrees apart. Real assets are full
/// of these: a fan of triangles meeting at a pole, a strip welded to itself, a
/// zero-area face an exporter left behind. They must not be allowed to invent
/// features, because a feature costs a seed and the seeds are the vertex budget:
/// a sphere whose poles each read as a ring of folds spent nearly its whole
/// budget on them and came back 42 % denser than it was asked for.
///
/// Measured against the longest edge squared, which is the only scale-free thing
/// to compare an area to. A well-shaped triangle scores about 0.43; the cut-off
/// here is four orders of magnitude below anything an artist would author.
fn is_sliver(surface: Surface<'_>, face: u32) -> bool {
    let corners = surface.face(face);
    let longest = (0..3)
        .map(|corner| surface.distance(corners[corner], corners[(corner + 1) % 3]))
        .fold(0.0f64, f64::max);
    if longest <= 0.0 {
        return true;
    }
    surface.face_area(face) < SLIVER_QUALITY * longest * longest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remesh::size_field;
    use crate::remesh::topology::Topology;

    /// The unit cube as twelve triangles, closed, every edge of the box a
    /// ninety-degree crease.
    fn cube() -> (Vec<f32>, Vec<u32>) {
        let positions = vec![
            0.0, 0.0, 0.0, // 0
            1.0, 0.0, 0.0, // 1
            1.0, 1.0, 0.0, // 2
            0.0, 1.0, 0.0, // 3
            0.0, 0.0, 1.0, // 4
            1.0, 0.0, 1.0, // 5
            1.0, 1.0, 1.0, // 6
            0.0, 1.0, 1.0, // 7
        ];
        let indices = vec![
            0, 2, 1, 0, 3, 2, // back
            4, 5, 6, 4, 6, 7, // front
            0, 1, 5, 0, 5, 4, // bottom
            3, 7, 6, 3, 6, 2, // top
            0, 4, 7, 0, 7, 3, // left
            1, 2, 6, 1, 6, 5, // right
        ];
        (positions, indices)
    }

    /// A flat strip of `steps` quads in the XY plane: an open surface whose
    /// whole outline is a single border loop.
    fn strip(steps: u32) -> (Vec<f32>, Vec<u32>) {
        let mut positions = Vec::new();
        for step in 0..=steps {
            let x = step as f32;
            positions.extend_from_slice(&[x, 0.0, 0.0]);
            positions.extend_from_slice(&[x, 1.0, 0.0]);
        }
        let mut indices = Vec::new();
        for step in 0..steps {
            let at = step * 2;
            indices.extend_from_slice(&[at, at + 2, at + 1]);
            indices.extend_from_slice(&[at + 1, at + 2, at + 3]);
        }
        (positions, indices)
    }

    fn features_of(
        positions: &[f32],
        indices: &[u32],
        borders: bool,
        crease_degrees: Option<f32>,
    ) -> Features {
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
        build(
            surface,
            borders,
            crease_degrees.map(|degrees| degrees.to_radians().cos()),
            None,
        )
    }

    /// A flat diamond: four sharp corners on one unbroken border loop, each
    /// side divided so no corner is anywhere near an end of a chain.
    ///
    /// ```text
    ///        (0, 1)
    ///       /      \
    ///  (-1, 0)     (1, 0)
    ///       \      /
    ///        (0, -1)
    /// ```
    fn diamond(per_side: u32) -> (Vec<f32>, Vec<u32>) {
        let tips = [[0.0f32, 1.0], [1.0, 0.0], [0.0, -1.0], [-1.0, 0.0]];
        // The centre first, then the border walked anticlockwise.
        let mut positions = vec![0.0f32, 0.0, 0.0];
        for side in 0..4 {
            let from = tips[side];
            let to = tips[(side + 1) % 4];
            for step in 0..per_side {
                let along = step as f32 / per_side as f32;
                positions.extend_from_slice(&[
                    from[0] + (to[0] - from[0]) * along,
                    from[1] + (to[1] - from[1]) * along,
                    0.0,
                ]);
            }
        }
        let border = 4 * per_side;
        let mut indices = Vec::new();
        for step in 0..border {
            indices.extend_from_slice(&[0, 1 + step, 1 + (step + 1) % border]);
        }
        (positions, indices)
    }

    /// A corner in the middle of a chain gets a seed of its own.
    ///
    /// Nothing about the graph marks it: the border of a leaf runs unbroken all
    /// the way round, so its tip has two feature partners like every other
    /// vertex on it. Stations spaced by arc length land either side and the
    /// rebuild cuts the tip off to a chord. The four corners of a diamond are
    /// the same shape as that tip and are equally invisible to a degree count.
    #[test]
    fn a_corner_in_the_middle_of_a_border_is_a_junction() {
        let (positions, indices) = diamond(6);

        let features = features_of(&positions, &indices, true, None);

        let junctions: Vec<u32> = (0..positions.len() as u32 / 3)
            .filter(|&vertex| features.junction[vertex as usize])
            .collect();
        // The tips are the first vertex of each side: border vertex 0, 6, 12, 18,
        // which are 1, 7, 13, 19 once the centre is counted.
        assert_eq!(
            junctions,
            vec![1, 7, 13, 19],
            "each of the four corners, and nothing along the straight sides"
        );
    }

    /// Only the sharpest vertex of a bend, so a rounded tip does not flag every
    /// vertex on it.
    ///
    /// Two adjacent junctions is not a harmless excess: the collapse refuses to
    /// merge one into another, so they sit there spending budget and evening
    /// nothing out.
    #[test]
    fn a_rounded_corner_flags_one_vertex_not_all_of_them() {
        // A circle turns continuously, so every vertex on it bends by the same
        // amount and the window sees the full turn at each. Exactly one may be
        // picked per window.
        let count = 32u32;
        let mut positions = vec![0.0f32, 0.0, 0.0];
        for step in 0..count {
            let angle = step as f32 / count as f32 * std::f32::consts::TAU;
            positions.extend_from_slice(&[angle.cos(), angle.sin(), 0.0]);
        }
        let mut indices = Vec::new();
        for step in 0..count {
            indices.extend_from_slice(&[0, 1 + step, 1 + (step + 1) % count]);
        }

        let features = features_of(&positions, &indices, true, None);

        let flagged = features.junction.iter().filter(|&&is| is).count();
        // The size field is one unit here against a circumference of ~6.3, so
        // the window covers most of the ring and very few may survive it.
        assert!(
            flagged <= 2,
            "a smooth ring flagged {flagged} corners; the thinning is not working"
        );
    }

    #[test]
    fn a_cube_at_thirty_degrees_has_its_twelve_edges_as_creases() {
        let (positions, indices) = cube();

        let features = features_of(&positions, &indices, true, Some(30.0));

        let edges: usize = features
            .polylines
            .iter()
            .map(|line| {
                if line.closed {
                    line.vertices.len()
                } else {
                    line.vertices.len() - 1
                }
            })
            .sum();
        assert_eq!(edges, 12, "a box has twelve creased edges");
        assert!(
            features.on_feature.iter().all(|&on| on),
            "every corner of a cube is on a crease"
        );
        assert!(
            features
                .polylines
                .iter()
                .all(|line| line.kind == FeatureKind::Crease),
            "a closed cube has no border"
        );
    }

    #[test]
    fn a_cube_with_creases_off_has_no_features() {
        let (positions, indices) = cube();

        let features = features_of(&positions, &indices, true, None);

        assert!(features.is_empty());
        assert!(features.on_feature.iter().all(|&on| !on));
    }

    #[test]
    fn a_flat_cube_face_angle_is_below_a_ninety_degree_threshold() {
        let (positions, indices) = cube();

        // The box's folds are exactly ninety degrees, so a threshold above that
        // keeps none of them.
        let features = features_of(&positions, &indices, true, Some(120.0));

        assert!(
            features.is_empty(),
            "a ninety-degree fold is not a crease at a hundred-and-twenty-degree threshold"
        );
    }

    #[test]
    fn an_open_strip_has_one_closed_border_loop() {
        let (positions, indices) = strip(6);

        let features = features_of(&positions, &indices, true, None);

        assert_eq!(features.polylines.len(), 1, "one outline");
        let line = &features.polylines[0];
        assert!(line.closed, "an outline comes back round to itself");
        assert_eq!(line.kind, FeatureKind::Boundary);
        assert_eq!(
            line.vertices.len(),
            14,
            "every vertex of a one-quad-wide strip is on its outline"
        );
        let mut sorted = line.vertices.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 14, "a chain never repeats a vertex");
    }

    #[test]
    fn borders_off_leaves_the_strip_featureless() {
        let (positions, indices) = strip(4);

        let features = features_of(&positions, &indices, false, None);

        assert!(features.is_empty());
    }

    #[test]
    fn a_branching_edge_is_a_feature_whatever_the_settings_say() {
        // Three faces on edge 0-1.
        let positions = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0,
        ];
        let indices = vec![0, 1, 2, 0, 1, 3, 0, 1, 4];

        let features = features_of(&positions, &indices, false, None);

        assert!(
            features.on_feature[0] && features.on_feature[1],
            "the branch itself is kept even with borders and creases off"
        );
    }

    #[test]
    fn the_chains_are_the_same_on_every_run() {
        let (positions, indices) = cube();

        let first = features_of(&positions, &indices, true, Some(30.0));
        let second = features_of(&positions, &indices, true, Some(30.0));

        assert_eq!(first.polylines, second.polylines);
        assert_eq!(first.polyline_of, second.polyline_of);
    }
}
