//! Flattening one chart, and judging what comes out.
//!
//! A chart can be flattened when it is a **planar domain**: a disc, or a disc
//! with holes — connected, every edge used by one or two triangles, its border
//! loops passing through no vertex twice, and no handle (genus 0). A disc with
//! holes needs no cut between them: the holes stay holes in the flattening, the
//! way an eye socket does, and the seams placed later are spent on stretch
//! alone.
//!
//! Flattening is LSCM from two border vertices far apart, then (for minimum
//! stretch) as-rigid-as-possible passes. The result is mirrored if it came out
//! wound backwards, scaled to its own area — one UV unit is one model unit —
//! and measured: per triangle its stretch, `max(σ₁, 1/σ₂)` against the chart's
//! mean density, which counts both what the stretch view shows (σ₁/σ₂) and what
//! the texel-density view shows (σ₁σ₂ drifting from 1).
//!
//! It is **valid** when it is one-to-one: no triangle folded or collapsed, no
//! two border edges crossing, and exactly one border loop wound
//! counter-clockwise (the outside; every hole clockwise). With no triangle
//! folded, a point's coverage is the winding number of the border about it;
//! simple loops that do not cross, one counter-clockwise and the rest
//! clockwise, never wind twice — so nothing overlaps.

use std::collections::HashMap;

use glam::{DVec2, Vec2, Vec3};

use super::element::{Element, elements};
use super::surface::{root, union};
use crate::stack::Flattening;

/// As-rigid-as-possible passes a flattening is judged by while seams are
/// placed; a chart that is kept gets the rest of [`super::arap::PASSES`].
pub(super) const TEST_PASSES: usize = 4;

/// A UV triangle smaller than this share of its 3D area (once the chart is
/// scaled to its area) is collapsed, as good as folded.
const COLLAPSED: f64 = 1.0e-6;

/// The stretch a folded or collapsed triangle counts as: past any limit, and
/// large enough that a fold is where a seam goes first.
pub(super) const FOLDED: f64 = 10.0;

/// The share of a chart's area whose own stretch may pass the per-triangle
/// bound ([`super::charts::Settings::tail_limit`]): a few slivers no flattening
/// can straighten must not hold a whole island to ransom.
pub(super) const TAIL_SHARE: f64 = 0.01;

/// A chart to flatten: its own vertices and triangles.
pub(super) struct Domain {
    pub points: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// Per vertex, the surface vertex it was cut from.
    pub origin: Vec<u32>,
}

/// A domain's border, and whether it can lie flat.
pub(super) struct Shape {
    /// Each border loop, its vertices in order, the domain on their left.
    pub loops: Vec<Vec<u32>>,
    /// Edges used by one or two triangles, loops through no vertex twice.
    pub manifold: bool,
    /// Per triangle, whether it is where the domain is not manifold: it uses
    /// an edge more than two triangles do, or has a corner the border passes
    /// through twice.
    pub trouble: Vec<bool>,
    /// Connected pieces.
    pub components: usize,
    /// Euler characteristic, `V − E + F`.
    pub euler: i64,
}

impl Shape {
    /// A disc, or a disc with holes.
    pub(super) fn planar(&self) -> bool {
        self.manifold && self.components == 1 && self.handles() == 0 && !self.loops.is_empty()
    }

    /// Its genus: the handles a cut graph has to open, for a connected
    /// manifold.
    pub(super) fn handles(&self) -> i64 {
        (2 - self.loops.len() as i64 - self.euler) / 2
    }
}

/// The shape of `domain`. Triangles with a corner twice are left out: they
/// have no area and lie along an edge the others already use.
pub(super) fn shape(domain: &Domain) -> Shape {
    let n = domain.points.len();
    let proper = || {
        domain
            .triangles
            .iter()
            .filter(|[a, b, c]| a != b && b != c && a != c)
    };
    let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
    for corners in proper() {
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut trouble = vec![false; domain.triangles.len()];
    let mut twice = vec![false; n];

    // The border, walked: each border edge runs with the domain on its left,
    // and leaves each vertex once.
    let mut next = vec![u32::MAX; n];
    let mut order: Vec<u32> = Vec::new();
    for corners in proper() {
        for k in 0..3 {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            if uses[&(a.min(b), a.max(b))] == 1 {
                if next[a as usize] != u32::MAX {
                    twice[a as usize] = true;
                }
                next[a as usize] = b;
                order.push(a);
            }
        }
    }
    for (triangle, corners) in domain.triangles.iter().enumerate() {
        if corners[0] == corners[1] || corners[1] == corners[2] || corners[0] == corners[2] {
            continue;
        }
        trouble[triangle] = (0..3).any(|k| {
            let (a, b) = (corners[k], corners[(k + 1) % 3]);
            uses[&(a.min(b), a.max(b))] > 2 || twice[a as usize]
        });
    }
    let mut manifold = !trouble.contains(&true);
    let mut on_loop = vec![false; n];
    let mut loops = Vec::new();
    for &start in &order {
        if on_loop[start as usize] {
            continue;
        }
        let mut walk = Vec::new();
        let mut at = start;
        loop {
            if on_loop[at as usize] {
                // Back where it began, or into another loop: the second is a
                // vertex on the border twice.
                if at != start {
                    manifold = false;
                }
                break;
            }
            on_loop[at as usize] = true;
            walk.push(at);
            at = next[at as usize];
            if at == u32::MAX {
                manifold = false;
                break;
            }
        }
        loops.push(walk);
    }

    // Vertices used, and connected pieces through the triangles.
    let mut parent: Vec<u32> = (0..n as u32).collect();
    let mut used = vec![false; n];
    let mut faces = 0i64;
    for corners in proper() {
        faces += 1;
        for &corner in corners {
            used[corner as usize] = true;
        }
        union(&mut parent, corners[0], corners[1]);
        union(&mut parent, corners[0], corners[2]);
    }
    let vertices = used.iter().filter(|&&used| used).count() as i64;
    let components = (0..n as u32)
        .filter(|&vertex| used[vertex as usize] && root(&mut parent, vertex) == vertex)
        .count();
    Shape {
        loops,
        manifold,
        trouble,
        components,
        euler: vertices - uses.len() as i64 + faces,
    }
}

/// A chart flattened and measured.
pub(super) struct Flat {
    /// Per domain vertex, its UV, in model units.
    pub uvs: Vec<Vec2>,
    /// Per domain triangle, its stretch ([`FOLDED`] for a folded one, 1 for
    /// one with no area).
    pub stretch: Vec<f64>,
    /// Per domain triangle, its 3D area.
    pub area: Vec<f64>,
    /// One-to-one: nothing folded, collapsed or overlapping.
    pub valid: bool,
    /// The share of the area folded or collapsed.
    pub folded: f64,
    /// Per domain triangle, whether it is folded or collapsed.
    pub folds: Vec<bool>,
    /// Whether the border crosses itself or winds round twice, nothing
    /// folded: the chart lies flat but over itself, as a strip that wraps
    /// round more than a full turn does.
    pub overlaps: bool,
    /// Where it lies over itself: the triangles of two border edges that
    /// cross, one from each part that overlaps.
    pub overlap_at: Option<[usize; 2]>,
    /// The area-weighted mean stretch.
    pub mean: f64,
    /// The worst σ₁/σ₂, all but the worst [`TAIL_SHARE`] of the area.
    pub tail: f64,
}

/// Flatten `domain` (whose shape is `shape`): `None` where it has nothing to
/// flatten.
pub(super) fn flatten(
    domain: &Domain,
    shape: &Shape,
    flattening: Flattening,
    passes: usize,
) -> Option<Flat> {
    // Pin two border vertices far apart, a 3D distance apart along U: the
    // first border vertex's farthest, and that one's farthest.
    let border: Vec<u32> = {
        let mut border: Vec<u32> = shape.loops.iter().flatten().copied().collect();
        border.sort_unstable();
        border
    };
    let points = &domain.points;
    let farthest = |from: u32| {
        border
            .iter()
            .copied()
            .max_by(|&x, &y| {
                let p = points[from as usize];
                p.distance_squared(points[x as usize])
                    .total_cmp(&p.distance_squared(points[y as usize]))
                    .then(y.cmp(&x))
            })
            .unwrap_or(from)
    };
    let one = farthest(*border.first()?);
    let two = farthest(one);
    if one == two {
        return None;
    }
    let length = points[one as usize].distance(points[two as usize]);
    let pins = [
        (one as usize, Vec2::ZERO),
        (two as usize, Vec2::new(length, 0.0)),
    ];
    let mut uvs = super::lscm::flatten(points, &domain.triangles, &pins)?;
    if flattening == Flattening::MinimumStretch {
        super::arap::refine(points, &domain.triangles, &mut uvs, passes, None);
    }
    measure(domain, shape, uvs)
}

/// `flat` refined by `passes` more as-rigid-as-possible passes over the whole
/// domain (none for a conformal flattening), and measured again.
pub(super) fn refine(
    domain: &Domain,
    shape: &Shape,
    flat: &Flat,
    flattening: Flattening,
    passes: usize,
) -> Option<Flat> {
    let mut uvs = flat.uvs.clone();
    if flattening == Flattening::MinimumStretch {
        super::arap::refine(&domain.points, &domain.triangles, &mut uvs, passes, None);
    }
    measure(domain, shape, uvs)
}

/// Mirror a flattening to keep the winding, scale it to its area and measure
/// it: `None` where it has no area at all.
fn measure(domain: &Domain, shape: &Shape, mut uvs: Vec<Vec2>) -> Option<Flat> {
    let elements = elements(&domain.points, &domain.triangles);
    let area3: f64 = elements.iter().map(|element| element.area).sum();
    let area2: f64 = elements.iter().map(|element| signed(element, &uvs)).sum();
    if area3 <= 0.0 || area2 == 0.0 || !area2.is_finite() {
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

    let count = domain.triangles.len();
    let mut stretch = vec![1.0f64; count];
    let mut area = vec![0.0f64; count];
    let mut anisotropy: Vec<(f64, f64)> = Vec::with_capacity(elements.len());
    let mut folded = 0.0;
    let mut folds = vec![false; count];
    let mut weighted = 0.0;
    for element in &elements {
        let (own, ratio) = if signed(element, &uvs) <= COLLAPSED * element.area {
            folded += element.area;
            folds[element.triangle] = true;
            (FOLDED, FOLDED)
        } else {
            let (row_u, row_v) = jacobian(element, &uvs);
            let (large, small) = singular_values(row_u.x, row_u.y, row_v.x, row_v.y);
            let small = small.max(1.0e-12);
            (
                large.max(1.0 / small).min(FOLDED),
                (large / small).min(FOLDED),
            )
        };
        stretch[element.triangle] = own;
        area[element.triangle] = element.area;
        weighted += element.area * own;
        anisotropy.push((ratio, element.area));
    }
    anisotropy.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut kept = 0.0;
    let mut tail = 1.0;
    for &(ratio, weight) in &anisotropy {
        tail = ratio;
        kept += weight;
        if kept >= (1.0 - TAIL_SHARE) * area3 {
            break;
        }
    }

    // One-to-one: the border crosses nothing, and winds round once.
    let mut overlaps = false;
    let mut overlap_at = None;
    if folded == 0.0 {
        let edges: Vec<Edge> = shape
            .loops
            .iter()
            .flat_map(|walk| (0..walk.len()).map(|k| (walk[k], walk[(k + 1) % walk.len()])))
            .map(|(a, b)| Edge {
                ends: [uvs[a as usize], uvs[b as usize]],
                vertices: [a, b],
            })
            .collect();
        let outer = shape
            .loops
            .iter()
            .filter(|walk| loop_area(walk, &uvs) > 0.0)
            .count();
        let crossing = crosses(&edges, &edges);
        overlaps = outer != 1 || crossing.is_some();
        // The triangles either crossing border edge belongs to.
        overlap_at = crossing.and_then(|(one, two)| {
            let owner = |edge: &Edge| {
                domain.triangles.iter().position(|corners| {
                    (0..3).any(|k| [corners[k], corners[(k + 1) % 3]] == edge.vertices)
                })
            };
            Some([owner(&edges[one])?, owner(&edges[two])?])
        });
    }
    Some(Flat {
        uvs,
        stretch,
        area,
        valid: folded == 0.0 && !overlaps,
        folded: folded / area3,
        folds,
        overlaps,
        overlap_at,
        mean: weighted / area3,
        tail,
    })
}

/// A triangle's signed UV area.
fn signed(element: &Element, uvs: &[Vec2]) -> f64 {
    let [a, b, c] = element.corners.map(|corner| uvs[corner].as_dvec2());
    0.5 * (b - a).perp_dot(c - a)
}

/// A triangle's map from its plane to UV, as the rows of its 2x2 Jacobian.
fn jacobian(element: &Element, uvs: &[Vec2]) -> (DVec2, DVec2) {
    let (mut row_u, mut row_v) = (DVec2::ZERO, DVec2::ZERO);
    for k in 0..3 {
        let uv = uvs[element.corners[k]].as_dvec2();
        row_u += uv.x * element.gradients[k];
        row_v += uv.y * element.gradients[k];
    }
    (row_u, row_v)
}

/// The signed area a border loop encloses in UV: positive counter-clockwise.
fn loop_area(walk: &[u32], uvs: &[Vec2]) -> f64 {
    (0..walk.len())
        .map(|k| {
            let a = uvs[walk[k] as usize].as_dvec2();
            let b = uvs[walk[(k + 1) % walk.len()] as usize].as_dvec2();
            a.perp_dot(b)
        })
        .sum::<f64>()
        * 0.5
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

/// A border edge: its ends' UVs and the vertices they belong to.
#[derive(Clone, Copy)]
struct Edge {
    ends: [Vec2; 2],
    vertices: [u32; 2],
}

/// The first edge of `these` that crosses an edge of `those` (the same list,
/// to test one border against itself), with that edge: an index into each.
/// Edges that share a vertex are neighbours along a border and are not
/// compared. `those` is bucketed on a grid, so a long
/// border costs little more than a short one.
fn crosses(these: &[Edge], those: &[Edge]) -> Option<(usize, usize)> {
    if these.is_empty() || those.is_empty() {
        return None;
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
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (index, edge) in those.iter().enumerate() {
        let (xs, ys) = cells(edge);
        for x in xs {
            for y in ys.clone() {
                grid.entry((x, y)).or_default().push(index);
            }
        }
    }
    for (at, edge) in these.iter().enumerate() {
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
                        return Some((at, index));
                    }
                }
            }
        }
    }
    None
}

/// Whether segments `ab` and `cd` properly cross: each passes from one side of
/// the other to its other side. Touching is not crossing — a chart whose
/// border meets itself at a point (a pinch the fixed seams left) does not
/// overlap itself there — and two borders cannot overlap without crossing
/// somewhere, short of one lying wholly inside the other, which the winding
/// test catches.
fn segments_cross(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> bool {
    let side = |p: DVec2, q: DVec2, r: DVec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(columns: u32, rows: u32, hole: Option<(u32, u32)>) -> Domain {
        let mut points = Vec::new();
        for y in 0..=rows {
            for x in 0..=columns {
                points.push(Vec3::new(x as f32, y as f32, 0.0));
            }
        }
        let at = |x: u32, y: u32| y * (columns + 1) + x;
        let mut triangles = Vec::new();
        for y in 0..rows {
            for x in 0..columns {
                if hole == Some((x, y)) {
                    continue;
                }
                triangles.push([at(x, y), at(x + 1, y), at(x + 1, y + 1)]);
                triangles.push([at(x, y), at(x + 1, y + 1), at(x, y + 1)]);
            }
        }
        let origin = (0..points.len() as u32).collect();
        Domain {
            points,
            triangles,
            origin,
        }
    }

    #[test]
    fn a_disc_and_a_disc_with_a_hole_are_planar() {
        let disc = shape(&grid(3, 3, None));
        assert!(disc.planar());
        assert_eq!(disc.loops.len(), 1);
        let holed = shape(&grid(3, 3, Some((1, 1))));
        assert!(holed.planar(), "a hole needs no cut");
        assert_eq!(holed.loops.len(), 2);
    }

    /// A flat square with a hole in it flattens one-to-one, without stretch:
    /// one loop counter-clockwise, the hole's clockwise.
    #[test]
    fn a_flat_domain_with_a_hole_flattens_validly() {
        let domain = grid(3, 3, Some((1, 1)));
        let shape = shape(&domain);
        let flat = flatten(&domain, &shape, Flattening::MinimumStretch, TEST_PASSES)
            .expect("a flattening");
        assert!(flat.valid);
        assert!((flat.mean - 1.0).abs() < 1.0e-3, "mean {}", flat.mean);
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
        assert!(crosses(&x, &x).is_some());
        let corner = [
            edge([0.0, 0.0], [1.0, 0.0], [0, 1]),
            edge([1.0, 0.0], [1.0, 1.0], [1, 2]),
        ];
        assert!(
            crosses(&corner, &corner).is_none(),
            "neighbours share a vertex"
        );
    }
}
