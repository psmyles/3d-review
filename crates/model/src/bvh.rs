//! A bounding-volume hierarchy over the model's triangles, answering
//! line-of-sight occlusion queries — "does the mesh block the segment from the
//! camera to this point?" — in sub-linear time. Built once per loaded model and
//! reused across frames; the alternative (testing every triangle per query) is
//! linear in the triangle count and stutters on high-poly models.
//!
//! Invariant 1: the hierarchy stores only a permutation of triangle indices and
//! per-node bounds — it never copies the vertex/index geometry. Queries borrow
//! the shared [`ModelData`] to read triangle positions.

use std::collections::BTreeMap;

use glam::Vec3;

use crate::ModelData;

/// Triangles per leaf: the traversal tests every triangle in a leaf linearly, so
/// a small bucket keeps leaf work cheap while bounding the tree's depth.
const LEAF_SIZE: usize = 4;

/// Fraction of the segment length trimmed at each end of an occlusion query: a
/// triangle within this margin of the target point is the queried surface itself
/// (e.g. a box-edge midpoint lying on the mesh) and must not count as an
/// occluder; the start margin avoids the ray self-intersecting at its origin.
const SEGMENT_SLACK: f32 = 1.0e-3;

/// Maximum traversal-stack depth. The stack grows by one entry per level, so
/// [`MAX_DEPTH`] below keeps every tree inside it and the stack on the call frame.
const MAX_STACK: usize = 64;

/// The deepest a node may be built. A midpoint split is not bounded by
/// `log2(triangles)`: on clustered geometry - one distant triangle beside a
/// dense patch, repeated - each split peels a single triangle off, and a tree
/// deeper than the traversal stack had whole subtrees the queries silently never
/// visited (a missed pick, an occluder not found). Past [`MEDIAN_FROM_DEPTH`]
/// every split is a median one, which halves the triangles and so adds at most
/// `log2` more levels; this is the backstop past which a node is a leaf whatever
/// its size.
const MAX_DEPTH: usize = MAX_STACK - 2;

/// The depth from which splits are by median rather than by midpoint - see
/// [`MAX_DEPTH`]. A realistic mesh never gets here; its tree is ~log2(n) deep.
const MEDIAN_FROM_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf (`count > 0`): `left_first` is the start offset into [`Bvh::tris`].
    /// Internal (`count == 0`): `left_first` is the left child's node index, and
    /// the right child is `left_first + 1`.
    left_first: u32,
    count: u32,
}

/// The nearest triangle a ray crossed, and where along the ray it did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Hit parameter along the query direction: the world hit is
    /// `origin + t * dir`.
    pub t: f32,
    /// Global triangle index (`indices[3*t .. 3*t + 3]`).
    pub triangle: u32,
}

/// The nearest point on the mesh to a query, and which triangle carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClosestPoint {
    /// Global triangle index (`indices[3*t .. 3*t + 3]`).
    pub triangle: u32,
    /// The point itself, in the model's own space.
    pub point: Vec3,
    /// Distance from the query to [`Self::point`], squared — comparable without
    /// a square root, which is how the traversal prunes.
    pub distance_squared: f32,
    /// Barycentric coordinates of [`Self::point`] within [`Self::triangle`],
    /// summing to 1. Attributes interpolate straight through these.
    pub barycentric: Vec3,
}

impl Node {
    const EMPTY: Self = Self {
        min: Vec3::ZERO,
        max: Vec3::ZERO,
        left_first: 0,
        count: 0,
    };
}

/// A read-only spatial index over a model's triangles for occlusion queries.
#[derive(Debug, Clone)]
pub struct Bvh {
    nodes: Vec<Node>,
    /// Permutation of `0..triangle_count`, grouped so each leaf owns a contiguous
    /// run. A triangle index `t` refers to `indices[3*t .. 3*t + 3]`.
    tris: Vec<u32>,
}

impl Bvh {
    /// Build a hierarchy over all of `model`'s triangles. Cost is `O(n log n)` in
    /// the triangle count; do this once per loaded model and reuse it across
    /// frames. Expects a triangulated model (the importer triangulates on load).
    pub fn build(model: &ModelData) -> Self {
        let tri_count = model.indices.len() / 3;
        Self::build_from_triangles(model, (0..tri_count as u32).collect())
    }

    /// Build a hierarchy over a chosen subset of *global* triangle indices
    /// (`global_tris`, each `t` referring to `indices[3*t .. 3*t + 3]`). The
    /// stored permutation holds global indices, so queries read positions straight
    /// from the shared model regardless of which subset this covers — that lets a
    /// [`SceneBvh`] hold one hierarchy per mesh part without copying geometry.
    pub(crate) fn build_from_triangles(model: &ModelData, global_tris: Vec<u32>) -> Self {
        let n = global_tris.len();
        if n == 0 {
            return Self {
                nodes: vec![Node::EMPTY],
                tris: Vec::new(),
            };
        }

        // Per-triangle bounds + centroid scratch, indexed by *position* within
        // `global_tris` (a local index), dropped when the build returns (build
        // acceleration data, not a persistent copy of the geometry).
        let mut tri_min = Vec::with_capacity(n);
        let mut tri_max = Vec::with_capacity(n);
        let mut centroid = Vec::with_capacity(n);
        for &g in &global_tris {
            let [a, b, c] = triangle_positions(model, g);
            let lo = a.min(b).min(c);
            let hi = a.max(b).max(c);
            tri_min.push(lo);
            tri_max.push(hi);
            centroid.push((lo + hi) * 0.5);
        }

        // The builder permutes *local* positions; leaves reference these, then we
        // translate them back to global triangle indices for the stored `tris`.
        let mut order: Vec<u32> = (0..n as u32).collect();
        let mut nodes = Vec::with_capacity(2 * n);
        nodes.push(Node::EMPTY);
        build_node(
            0, 0, n, 0, &mut nodes, &mut order, &tri_min, &tri_max, &centroid,
        );
        nodes.shrink_to_fit();

        let tris = order.iter().map(|&p| global_tris[p as usize]).collect();
        Self { nodes, tris }
    }

    /// Whether the mesh occludes the segment from `origin` to `target`: true when
    /// any triangle is crossed strictly between the endpoints (excluding the thin
    /// `SEGMENT_SLACK` margins). Sub-linear in the triangle count; reads
    /// triangle positions from `model` (must be the model this was built from).
    pub fn segment_occluded(&self, model: &ModelData, origin: Vec3, target: Vec3) -> bool {
        let dir = target - origin;
        self.any_hit(model, origin, dir, SEGMENT_SLACK, 1.0 - SEGMENT_SLACK)
    }

    /// Whether any triangle blocks the ray `origin + t * dir` within
    /// `t ∈ (0, t_max)`. `t_max` is in units of `dir`'s length — pass a unit
    /// `dir` so it reads as a world distance, or `f32::INFINITY` for an
    /// unbounded ray. Unlike [`Self::segment_occluded`] there is no start
    /// slack: the caller offsets `origin` off the queried surface itself (a
    /// ray has no natural length to take a fractional slack of).
    pub fn ray_occluded(&self, model: &ModelData, origin: Vec3, dir: Vec3, t_max: f32) -> bool {
        self.any_hit(model, origin, dir, 0.0, t_max)
    }

    /// Shared any-hit traversal behind both occlusion queries: whether any
    /// triangle is crossed strictly within `t ∈ (t_min, t_max)` along `dir`.
    fn any_hit(&self, model: &ModelData, origin: Vec3, dir: Vec3, t_min: f32, t_max: f32) -> bool {
        let inv_dir = dir.recip();

        let mut stack = [0u32; MAX_STACK];
        let mut sp = 1usize; // node 0 (root) seeded below
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let node = self.nodes[stack[sp] as usize];
            if !segment_hits_aabb(origin, inv_dir, node.min, node.max, t_min, t_max) {
                continue;
            }
            if node.count > 0 {
                let leaf =
                    &self.tris[node.left_first as usize..(node.left_first + node.count) as usize];
                for &t in leaf {
                    let [a, b, c] = triangle_positions(model, t);
                    if ray_triangle_t(origin, dir, a, b, c).is_some_and(|t| t > t_min && t < t_max)
                    {
                        return true;
                    }
                }
            } else if sp + 2 <= MAX_STACK {
                stack[sp] = node.left_first;
                stack[sp + 1] = node.left_first + 1;
                sp += 2;
            }
        }
        false
    }

    /// The *nearest* triangle crossed by the ray `origin + t * dir` within
    /// `t` between `t_min` and `t_max`, or `None` when nothing is hit.
    ///
    /// Unlike the occlusion queries this cannot stop at the first hit, so it
    /// descends nearest-child-first and skips any node whose slab entry is
    /// already behind the best hit so far — which recovers most of the early
    /// exit an any-hit traversal gets for free.
    pub fn closest_hit(
        &self,
        model: &ModelData,
        origin: Vec3,
        dir: Vec3,
        t_min: f32,
        t_max: f32,
    ) -> Option<Hit> {
        self.closest_hit_with(origin, dir, t_min, t_max, None, |t| {
            triangle_positions(model, t)
        })
    }

    /// [`Self::closest_hit`] over caller-supplied geometry: `positions` answers
    /// for a global triangle index, and `bounds` (when given) replaces the
    /// stored per-node boxes. Both exist for the posed pick, which tests
    /// CPU-deformed corners against boxes [`Self::refit`] recomputed for the
    /// same pose — the tree's *topology* survives a deform, only its bounds and
    /// positions move.
    pub(crate) fn closest_hit_with(
        &self,
        origin: Vec3,
        dir: Vec3,
        t_min: f32,
        t_max: f32,
        bounds: Option<&[(Vec3, Vec3)]>,
        positions: impl Fn(u32) -> [Vec3; 3],
    ) -> Option<Hit> {
        let inv_dir = dir.recip();
        let mut best: Option<Hit> = None;
        let mut far = t_max;

        let box_of = |index: u32| -> (Vec3, Vec3) {
            match bounds {
                Some(refit) => refit[index as usize],
                None => {
                    let node = self.nodes[index as usize];
                    (node.min, node.max)
                }
            }
        };

        let mut stack = [0u32; MAX_STACK];
        let mut sp = 1usize; // node 0 (root) seeded below
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let index = stack[sp];
            let node = self.nodes[index as usize];
            let (lo, hi) = box_of(index);
            // The box may have been pushed while a nearer hit was still to come.
            if !segment_hits_aabb(origin, inv_dir, lo, hi, t_min, far) {
                continue;
            }
            if node.count > 0 {
                let leaf =
                    &self.tris[node.left_first as usize..(node.left_first + node.count) as usize];
                for &t in leaf {
                    let [a, b, c] = positions(t);
                    if let Some(hit) = ray_triangle_t(origin, dir, a, b, c)
                        && hit > t_min
                        && hit < far
                    {
                        far = hit;
                        best = Some(Hit {
                            t: hit,
                            triangle: t,
                        });
                    }
                }
            } else if sp + 2 <= MAX_STACK {
                // Nearest child last, so it pops first and tightens `far` before
                // the sibling is tested.
                let left = node.left_first;
                let right = left + 1;
                let entry = |child: u32| {
                    let (lo, hi) = box_of(child);
                    slab_entry(origin, inv_dir, lo, hi, t_min)
                };
                let (first, second) = if entry(left) <= entry(right) {
                    (right, left)
                } else {
                    (left, right)
                };
                stack[sp] = first;
                stack[sp + 1] = second;
                sp += 2;
            }
        }
        best
    }

    /// The nearest point on any indexed triangle to `query`, or `None` when the
    /// tree is empty or nothing lies within `max_distance`.
    ///
    /// This is the projection half of the retopology pipeline: a regenerated
    /// surface has no correspondence at all to the source, so every attribute
    /// it carries — material, UVs, colors, normals — is read off the source
    /// surface at the nearest point to each new vertex.
    ///
    /// Descends nearest-child-first by box distance and skips any box already
    /// farther than the best point so far, which is the closest-point analogue
    /// of [`Self::closest_hit`]'s early exit.
    pub fn closest_point(
        &self,
        model: &ModelData,
        query: Vec3,
        max_distance: f32,
    ) -> Option<ClosestPoint> {
        self.closest_point_where(model, query, max_distance, |_| true)
    }

    /// The nearest point on a triangle `allow` accepts.
    ///
    /// The filter is applied to the triangles themselves, not to the answer, so
    /// the search returns the nearest *acceptable* point rather than nothing
    /// when the nearest point happens to be on a rejected triangle. A caller
    /// asking "where is the surface, on the side I am facing" needs that: on a
    /// sheet with two sides a hair apart the nearest triangle of all is very
    /// often the one on the other side.
    ///
    /// Box pruning is unchanged - `allow` cannot be consulted for a box - so a
    /// filter that rejects almost everything costs close to a full traversal.
    pub fn closest_point_where(
        &self,
        model: &ModelData,
        query: Vec3,
        max_distance: f32,
        allow: impl Fn(u32) -> bool,
    ) -> Option<ClosestPoint> {
        // NaN spelled out rather than left to a negated comparison: a NaN range
        // would otherwise square to NaN and reject every box silently.
        if self.tris.is_empty() || max_distance.is_nan() || max_distance < 0.0 {
            return None;
        }
        let mut best: Option<ClosestPoint> = None;
        let mut far = max_distance * max_distance;

        let mut stack = [0u32; MAX_STACK];
        let mut sp = 1usize; // node 0 (root) seeded below
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let index = stack[sp];
            let node = self.nodes[index as usize];
            // The box may have been pushed while a nearer point was still to
            // come, so re-test against the current best rather than the one in
            // force when it was queued.
            if aabb_distance_squared(query, node.min, node.max) > far {
                continue;
            }
            if node.count > 0 {
                let leaf =
                    &self.tris[node.left_first as usize..(node.left_first + node.count) as usize];
                for &t in leaf {
                    if !allow(t) {
                        continue;
                    }
                    let [a, b, c] = triangle_positions(model, t);
                    let (point, barycentric) = closest_point_on_triangle(query, a, b, c);
                    let distance_squared = (point - query).length_squared();
                    if distance_squared <= far {
                        far = distance_squared;
                        best = Some(ClosestPoint {
                            triangle: t,
                            point,
                            distance_squared,
                            barycentric,
                        });
                    }
                }
            } else if sp + 2 <= MAX_STACK {
                // Nearest child last, so it pops first and tightens `far`
                // before the sibling is tested.
                let left = node.left_first;
                let right = left + 1;
                let distance = |child: u32| {
                    let node = self.nodes[child as usize];
                    aabb_distance_squared(query, node.min, node.max)
                };
                let (first, second) = if distance(left) <= distance(right) {
                    (right, left)
                } else {
                    (left, right)
                };
                stack[sp] = first;
                stack[sp + 1] = second;
                sp += 2;
            }
        }
        best
    }

    /// Per-node bounds recomputed from `positions`, in this tree's own node
    /// order, for querying a deformed copy of the geometry it was built over.
    ///
    /// A node's children are always pushed *after* it during the build, so
    /// walking the node list in reverse sees every child before its parent and
    /// one linear pass suffices.
    pub(crate) fn refit(&self, positions: impl Fn(u32) -> [Vec3; 3]) -> Vec<(Vec3, Vec3)> {
        let mut bounds =
            vec![(Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)); self.nodes.len()];
        for index in (0..self.nodes.len()).rev() {
            let node = self.nodes[index];
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            if node.count > 0 {
                let leaf =
                    &self.tris[node.left_first as usize..(node.left_first + node.count) as usize];
                for &t in leaf {
                    for corner in positions(t) {
                        lo = lo.min(corner);
                        hi = hi.max(corner);
                    }
                }
            } else {
                for child in [node.left_first as usize, node.left_first as usize + 1] {
                    // An empty tree's single node is a zero-count "leaf" with no
                    // children, so this only runs for real internal nodes.
                    if let Some(&(clo, chi)) = bounds.get(child) {
                        lo = lo.min(clo);
                        hi = hi.max(chi);
                    }
                }
            }
            bounds[index] = (lo, hi);
        }
        bounds
    }

    /// The triangle permutation this tree indexes — the global triangle indices
    /// it covers, in leaf order. Read by the posed pick to know which corners
    /// need deforming.
    pub(crate) fn triangles(&self) -> &[u32] {
        &self.tris
    }
}

/// Sentinel owning-node id for a model with no per-triangle node info — never
/// matches a real (Outliner) mesh index, so such a part always participates in
/// occlusion.
const NO_NODE: u32 = u32::MAX;

/// One [`Bvh`] per scene mesh part (owning node), so an occlusion query can skip
/// the parts the Outliner has hidden. This is both *correct* — a hidden mesh
/// isn't drawn, so it must not occlude a dimension label — and *fast*: a query
/// buried inside a tight "visible only" bounding box never traverses the hidden
/// geometry packed around it, which a single whole-model hierarchy would force it
/// to. Built once per loaded model and reused across frames.
///
/// Invariant 1: like [`Bvh`], the parts store only triangle-index permutations
/// and node bounds — never a copy of the vertex/index geometry.
#[derive(Debug, Clone)]
pub struct SceneBvh {
    parts: Vec<ScenePart>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScenePart {
    /// The scene node these triangles belong to (matches [`crate::TriangleData::node`]),
    /// or [`NO_NODE`] when the model carries no per-triangle node info.
    pub(crate) node: u32,
    pub(crate) bvh: Bvh,
}

/// The nearest mesh part a pick ray hit: which scene node owns it, which
/// triangle was crossed, and where along the ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneHit {
    /// Owning scene node (an index into [`ModelData::nodes`]), or [`u32::MAX`]
    /// for a model that carries no per-triangle node info.
    pub node: u32,
    /// Global triangle index.
    pub triangle: u32,
    /// Hit parameter along the query direction.
    pub t: f32,
}

impl SceneBvh {
    /// Build one [`Bvh`] per owning node. Total cost is `O(n log n)` in the
    /// triangle count (the same as a single whole-model build, split across
    /// parts); do this once per loaded model and reuse it across frames.
    pub fn build(model: &ModelData) -> Self {
        let tri_count = model.indices.len() / 3;
        if model.triangles.node.len() != tri_count {
            // No per-triangle node info: one part covering the whole model, which
            // visibility filtering can never exclude (it carries [`NO_NODE`]).
            let all = (0..tri_count as u32).collect();
            return Self {
                parts: vec![ScenePart {
                    node: NO_NODE,
                    bvh: Bvh::build_from_triangles(model, all),
                }],
            };
        }

        // Group global triangle indices by owning node. A `BTreeMap` makes the
        // part order deterministic (independent of triangle traversal order).
        let mut by_node: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for t in 0..tri_count as u32 {
            by_node
                .entry(model.triangles.node[t as usize])
                .or_default()
                .push(t);
        }
        let parts = by_node
            .into_iter()
            .map(|(node, tris)| ScenePart {
                node,
                bvh: Bvh::build_from_triangles(model, tris),
            })
            .collect();
        Self { parts }
    }

    /// Whether any *visible* mesh part occludes the segment from `origin` to
    /// `target`: parts whose node is in `hidden_nodes` are skipped, so a hidden
    /// mesh neither blocks a label nor costs a query. `hidden_nodes` is tiny (the
    /// handful of Outliner-hidden meshes), so the linear membership test is
    /// cheaper than building a set. Reads triangle positions from `model` (must be
    /// the model this was built from).
    pub fn segment_occluded(
        &self,
        model: &ModelData,
        origin: Vec3,
        target: Vec3,
        hidden_nodes: &[u32],
    ) -> bool {
        self.parts.iter().any(|part| {
            !hidden_nodes.contains(&part.node) && part.bvh.segment_occluded(model, origin, target)
        })
    }

    /// The nearest triangle the ray `origin + t * dir` hits within `t_max`,
    /// across every part `allow` accepts — the viewport pick.
    ///
    /// `allow` takes the part's owning node, so a caller filters by whatever it
    /// means by "pickable" (hidden meshes are out, and in solo only the isolated
    /// set is in) without materialising a complement list. Each part's search is
    /// bounded by the best hit so far, so the parts behind the front one cost
    /// little more than their root box test.
    pub fn pick(
        &self,
        model: &ModelData,
        origin: Vec3,
        dir: Vec3,
        t_min: f32,
        t_max: f32,
        allow: impl Fn(u32) -> bool,
    ) -> Option<SceneHit> {
        let mut best: Option<SceneHit> = None;
        let mut far = t_max;
        for part in &self.parts {
            if !allow(part.node) {
                continue;
            }
            if let Some(hit) = part.bvh.closest_hit(model, origin, dir, t_min, far) {
                far = hit.t;
                best = Some(SceneHit {
                    node: part.node,
                    triangle: hit.triangle,
                    t: hit.t,
                });
            }
        }
        best
    }

    /// The nearest point on any part `allow` accepts, and which node owns it.
    ///
    /// The closest-point twin of [`Self::pick`], and filtered the same way, so a
    /// caller measuring one object against another asks for that object's node
    /// rather than having to carve a mesh out first. Each part's search is
    /// bounded by the best point so far, so the parts farther away than the
    /// nearest cost little more than their root box test.
    pub fn closest_point(
        &self,
        model: &ModelData,
        query: Vec3,
        max_distance: f32,
        allow: impl Fn(u32) -> bool,
    ) -> Option<(u32, ClosestPoint)> {
        let mut best: Option<(u32, ClosestPoint)> = None;
        let mut far = max_distance;
        for part in &self.parts {
            if !allow(part.node) {
                continue;
            }
            if let Some(hit) = part.bvh.closest_point(model, query, far) {
                // Tightened to the distance itself, not its square: the bound
                // this takes is a distance.
                far = hit.distance_squared.sqrt();
                best = Some((part.node, hit));
            }
        }
        best
    }

    /// The parts, for a caller that queries them itself (the posed pick, which
    /// deforms each part's corners before testing it).
    pub(crate) fn parts(&self) -> &[ScenePart] {
        &self.parts
    }
}

/// Recursively fill node `node_idx` for the triangles `tris[start..end]`, pushing
/// child nodes onto `nodes` as it descends.
#[allow(clippy::too_many_arguments)]
fn build_node(
    node_idx: usize,
    start: usize,
    end: usize,
    depth: usize,
    nodes: &mut Vec<Node>,
    tris: &mut [u32],
    tri_min: &[Vec3],
    tri_max: &[Vec3],
    centroid: &[Vec3],
) {
    // Node bounds = union of its triangles' bounds.
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for &t in &tris[start..end] {
        lo = lo.min(tri_min[t as usize]);
        hi = hi.max(tri_max[t as usize]);
    }
    nodes[node_idx].min = lo;
    nodes[node_idx].max = hi;

    let count = end - start;
    if count <= LEAF_SIZE || depth >= MAX_DEPTH {
        nodes[node_idx].left_first = start as u32;
        nodes[node_idx].count = count as u32;
        return;
    }

    // Split along the widest axis of the centroid bounds, at its midpoint.
    let mut clo = Vec3::splat(f32::INFINITY);
    let mut chi = Vec3::splat(f32::NEG_INFINITY);
    for &t in &tris[start..end] {
        clo = clo.min(centroid[t as usize]);
        chi = chi.max(centroid[t as usize]);
    }
    let extent = chi - clo;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    if axis_value(extent, axis) <= 0.0 {
        // Every centroid coincides — nothing to split on; keep this as a leaf.
        nodes[node_idx].left_first = start as u32;
        nodes[node_idx].count = count as u32;
        return;
    }

    let split = axis_value(clo, axis) + axis_value(extent, axis) * 0.5;
    let mut mid = if depth >= MEDIAN_FROM_DEPTH {
        start
    } else {
        start
            + partition(&mut tris[start..end], |t| {
                axis_value(centroid[t as usize], axis) < split
            })
    };
    if mid == start || mid == end {
        // A spatial split that put everything on one side, or a node deep enough
        // that only a median split keeps the tree inside the traversal stack.
        mid = start + count / 2;
        tris[start..end].select_nth_unstable_by(count / 2, |&p, &q| {
            axis_value(centroid[p as usize], axis)
                .partial_cmp(&axis_value(centroid[q as usize], axis))
                .unwrap_or(core::cmp::Ordering::Equal)
        });
    }

    let left = nodes.len() as u32;
    nodes.push(Node::EMPTY);
    nodes.push(Node::EMPTY);
    nodes[node_idx].left_first = left;
    nodes[node_idx].count = 0;
    build_node(
        left as usize,
        start,
        mid,
        depth + 1,
        nodes,
        tris,
        tri_min,
        tri_max,
        centroid,
    );
    build_node(
        left as usize + 1,
        mid,
        end,
        depth + 1,
        nodes,
        tris,
        tri_min,
        tri_max,
        centroid,
    );
}

/// Component `axis` (0 = x, 1 = y, 2 = z) of `v`.
fn axis_value(v: Vec3, axis: usize) -> f32 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// Stable in-place partition: move every element satisfying `pred` to the front,
/// returning the count moved (the index where the failing partition begins).
fn partition(slice: &mut [u32], pred: impl Fn(u32) -> bool) -> usize {
    let mut i = 0;
    for j in 0..slice.len() {
        if pred(slice[j]) {
            slice.swap(i, j);
            i += 1;
        }
    }
    i
}

/// The three world positions of triangle `tri`, falling back to the origin for
/// any out-of-range index (a malformed mesh yields a degenerate, non-occluding
/// triangle rather than a panic).
/// The three corner positions of a triangle, with a missing index or vertex
/// reading as the origin rather than panicking.
///
/// Shared because `optimize`'s AO bake raycasts against the same buffers this
/// BVH indexes, and had grown an identical copy.
pub fn triangle_positions(model: &ModelData, tri: u32) -> [Vec3; 3] {
    let base = tri as usize * 3;
    let pos = |slot: usize| {
        model
            .indices
            .get(base + slot)
            .and_then(|&i| model.vertices.get(i as usize))
            .map(|v| v.position)
            .unwrap_or(Vec3::ZERO)
    };
    [pos(0), pos(1), pos(2)]
}

/// Slab test: whether the segment `origin + t*dir` (with `inv_dir = dir.recip()`)
/// overlaps the AABB `[lo, hi]` within `t ∈ [t_min, t_max]`.
fn segment_hits_aabb(
    origin: Vec3,
    inv_dir: Vec3,
    lo: Vec3,
    hi: Vec3,
    t_min: f32,
    t_max: f32,
) -> bool {
    let t0 = (lo - origin) * inv_dir;
    let t1 = (hi - origin) * inv_dir;
    let near = t0.min(t1);
    let far = t0.max(t1);
    let enter = near.max_element().max(t_min);
    let exit = far.min_element().min(t_max);
    enter <= exit
}

/// The `t` at which the segment enters the AABB `[lo, hi]` (clamped below by
/// `t_min`), for ordering a nearest-first descent. Meaningless when the box is
/// missed — the caller's own slab test rejects those.
fn slab_entry(origin: Vec3, inv_dir: Vec3, lo: Vec3, hi: Vec3, t_min: f32) -> f32 {
    let t0 = (lo - origin) * inv_dir;
    let t1 = (hi - origin) * inv_dir;
    t0.min(t1).max_element().max(t_min)
}

/// Squared distance from `point` to the AABB `[lo, hi]`; zero inside it. The
/// pruning test behind [`Bvh::closest_point`].
fn aabb_distance_squared(point: Vec3, lo: Vec3, hi: Vec3) -> f32 {
    let outside = (lo - point).max(point - hi).max(Vec3::ZERO);
    outside.length_squared()
}

/// The point of triangle `abc` nearest to `p`, with its barycentric
/// coordinates `(u, v, w)` — `p ≈ u*a + v*b + w*c`, summing to 1.
///
/// Ericson's *Real-Time Collision Detection* §5.1.5: classify `p` against the
/// triangle's seven Voronoi regions (three vertices, three edges, the face) and
/// answer in closed form. A degenerate triangle falls through to its first
/// corner rather than dividing by zero.
///
/// Public for the same reason [`ray_triangle_t`] is: `optimize` projects onto
/// geometry of its own as well as through a [`Bvh`], and a second copy of this
/// would be a second chance to get it subtly wrong.
pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, Vec3) {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, Vec3::X); // vertex region A
    }

    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, Vec3::Y); // vertex region B
    }

    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let denominator = d1 - d3;
        let v = if denominator != 0.0 {
            d1 / denominator
        } else {
            0.0
        };
        return (a + ab * v, Vec3::new(1.0 - v, v, 0.0)); // edge region AB
    }

    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, Vec3::Z); // vertex region C
    }

    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let denominator = d2 - d6;
        let w = if denominator != 0.0 {
            d2 / denominator
        } else {
            0.0
        };
        return (a + ac * w, Vec3::new(1.0 - w, 0.0, w)); // edge region AC
    }

    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let denominator = (d4 - d3) + (d5 - d6);
        let w = if denominator != 0.0 {
            (d4 - d3) / denominator
        } else {
            0.0
        };
        return (b + (c - b) * w, Vec3::new(0.0, 1.0 - w, w)); // edge region BC
    }

    // Face region: the barycentric coordinates are the normalized sub-areas.
    let denominator = va + vb + vc;
    if denominator == 0.0 {
        return (a, Vec3::X); // degenerate triangle
    }
    let inverse = 1.0 / denominator;
    let v = vb * inverse;
    let w = vc * inverse;
    (a + ab * v + ac * w, Vec3::new(1.0 - v - w, v, w))
}

/// Möller–Trumbore ray/triangle intersection. Returns the hit parameter `t`
/// along `dir` (so the world hit is `origin + t * dir`), or `None` when the ray
/// misses or runs parallel to the triangle. Hits from either side count — the
/// model's facing is irrelevant to whether it blocks the line of sight.
///
/// Public because the skeleton overlay's bone pick tests the same maths against
/// octahedra it builds itself, which are geometry no BVH indexes.
pub fn ray_triangle_t(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    const EPS: f32 = 1.0e-8;
    let edge1 = b - a;
    let edge2 = c - a;
    let pvec = dir.cross(edge2);
    let det = edge1.dot(pvec);
    if det.abs() < EPS {
        return None; // ray parallel to the triangle plane
    }
    let inv_det = 1.0 / det;
    let tvec = origin - a;
    let u = tvec.dot(pvec) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qvec = tvec.cross(edge1);
    let v = dir.dot(qvec) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(edge2.dot(qvec) * inv_det)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Vertex, demo_cube_model};

    /// Brute-force reference for the closest-point query: scan every triangle.
    /// The BVH must agree with this everywhere — it only changes the *cost*.
    /// The squared distance alone, not the point: where two faces are
    /// equidistant — a query on the cube's axis is as close to one side as to
    /// the other — the *point* is a legitimate choice between them and the two
    /// implementations need not agree on it.
    fn nearest_point_brute_force(model: &ModelData, query: Vec3) -> Option<f32> {
        (0..(model.indices.len() / 3) as u32)
            .map(|triangle| {
                let [a, b, c] = triangle_positions(model, triangle);
                let (point, _) = closest_point_on_triangle(query, a, b, c);
                (point - query).length_squared()
            })
            .min_by(f32::total_cmp)
    }

    #[test]
    fn closest_point_matches_a_brute_force_scan_around_the_cube() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);

        // A grid straddling the cube: inside it, on its faces, and well outside.
        for x in -3..=3 {
            for y in -3..=3 {
                for z in -3..=3 {
                    let query =
                        Vec3::new(x as f32, y as f32, z as f32) * 0.4 + Vec3::new(0.0, 0.53, 0.0);
                    let expected =
                        nearest_point_brute_force(&model, query).expect("the cube has faces");
                    let hit = bvh
                        .closest_point(&model, query, f32::INFINITY)
                        .expect("the hierarchy must find what the scan found");
                    assert!(
                        (hit.distance_squared - expected).abs() <= 1.0e-6,
                        "at {query:?} the hierarchy found {} and the scan {expected}",
                        hit.distance_squared
                    );
                    assert!(
                        ((hit.point - query).length_squared() - hit.distance_squared).abs()
                            <= 1.0e-6,
                        "at {query:?} the reported point and distance disagree"
                    );
                }
            }
        }
    }

    #[test]
    fn a_query_on_the_surface_lands_on_it_with_interior_barycentrics() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // Inside the +Z face, away from its edges and its diagonal.
        let query = Vec3::new(0.2, 0.3, 0.5);

        let hit = bvh
            .closest_point(&model, query, 1.0)
            .expect("the cube is within range");

        assert!(
            hit.distance_squared <= 1.0e-8,
            "the query is on the surface"
        );
        assert!((hit.point - query).length() <= 1.0e-5);
        let sum = hit.barycentric.x + hit.barycentric.y + hit.barycentric.z;
        assert!((sum - 1.0).abs() <= 1.0e-5, "barycentrics sum to one");
        assert!(
            hit.barycentric.min_element() > 0.0,
            "a point inside a triangle has no zero coordinate: {:?}",
            hit.barycentric
        );
        // The barycentrics must actually reconstruct the point.
        let [a, b, c] = triangle_positions(&model, hit.triangle);
        let rebuilt = a * hit.barycentric.x + b * hit.barycentric.y + c * hit.barycentric.z;
        assert!((rebuilt - hit.point).length() <= 1.0e-5);
    }

    #[test]
    fn a_query_beyond_the_maximum_distance_finds_nothing() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        let far = Vec3::new(0.0, 0.53, 20.0);

        assert!(bvh.closest_point(&model, far, 1.0).is_none());
        assert!(
            bvh.closest_point(&model, far, 25.0).is_some(),
            "the same query inside the range does find the cube"
        );
    }

    #[test]
    fn closest_point_on_an_empty_model_finds_nothing() {
        let model = ModelData::default();
        let bvh = Bvh::build(&model);

        assert!(
            bvh.closest_point(&model, Vec3::ZERO, f32::INFINITY)
                .is_none()
        );
    }

    /// Brute-force reference: scan every triangle, matching the BVH's slack rule.
    /// The BVH must agree with this on every query — it only changes the *cost*.
    fn occluded_brute_force(model: &ModelData, origin: Vec3, target: Vec3) -> bool {
        let dir = target - origin;
        let t_min = SEGMENT_SLACK;
        let t_max = 1.0 - SEGMENT_SLACK;
        (0..model.indices.len() / 3).any(|t| {
            let [a, b, c] = triangle_positions(model, t as u32);
            ray_triangle_t(origin, dir, a, b, c).is_some_and(|t| t > t_min && t < t_max)
        })
    }

    /// Brute-force reference for the nearest hit: scan every triangle and keep
    /// the smallest `t`. The BVH must agree on both the distance and which
    /// triangle won — it only changes the *cost*.
    fn closest_brute_force(
        model: &ModelData,
        origin: Vec3,
        dir: Vec3,
        t_min: f32,
        t_max: f32,
    ) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for t in 0..model.indices.len() as u32 / 3 {
            let [a, b, c] = triangle_positions(model, t);
            if let Some(hit) = ray_triangle_t(origin, dir, a, b, c)
                && hit > t_min
                && hit < t_max
                && best.is_none_or(|b| hit < b.t)
            {
                best = Some(Hit {
                    t: hit,
                    triangle: t,
                });
            }
        }
        best
    }

    #[test]
    fn closest_hit_matches_brute_force_over_a_grid() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        let eyes = [
            Vec3::new(0.0, 0.5, 6.0),
            Vec3::new(6.0, 0.5, 0.0),
            Vec3::new(0.0, 6.0, 0.0),
            Vec3::new(4.0, 4.0, 4.0),
            Vec3::new(-4.0, 0.5, -4.0),
        ];
        for eye in eyes {
            for ix in -3..=3 {
                for iy in -3..=3 {
                    // Jittered off the cube's face planes, for the same reason
                    // `unbounded_ray_matches_a_long_segment` jitters: a ray
                    // exactly grazing a box edge is a measure-zero case where
                    // the AABB and triangle tests can disagree by an ulp.
                    let target =
                        Vec3::new(ix as f32 * 0.26 + 0.005, 0.53 + iy as f32 * 0.26, 0.007);
                    let dir = (target - eye).normalize();
                    let mine = bvh.closest_hit(&model, eye, dir, 0.0, f32::INFINITY);
                    let reference = closest_brute_force(&model, eye, dir, 0.0, f32::INFINITY);
                    match (mine, reference) {
                        (None, None) => {}
                        (Some(mine), Some(reference)) => assert!(
                            (mine.t - reference.t).abs() < 1e-4,
                            "eye {eye:?} dir {dir:?}: {mine:?} vs {reference:?}"
                        ),
                        (mine, reference) => {
                            panic!("eye {eye:?} dir {dir:?}: {mine:?} vs {reference:?}")
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn closest_hit_takes_the_near_face_not_the_far_one() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // The cube spans z in [-0.5, 0.5]; from 5 out the near face is 4.5 away
        // and the far one 5.5. An any-hit traversal is free to report either.
        let hit = bvh
            .closest_hit(
                &model,
                Vec3::new(0.0, 0.5, 5.0),
                Vec3::NEG_Z,
                0.0,
                f32::INFINITY,
            )
            .expect("the ray crosses the cube");
        assert!((hit.t - 4.5).abs() < 1e-4, "{}", hit.t);
    }

    #[test]
    fn closest_hit_respects_its_bounds() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        let origin = Vec3::new(0.0, 0.5, 5.0);
        assert!(
            bvh.closest_hit(&model, origin, Vec3::NEG_Z, 0.0, 4.0)
                .is_none(),
            "a ray stopping short of the near face must not hit"
        );
        assert!(
            bvh.closest_hit(&model, origin, Vec3::NEG_Z, 5.0, f32::INFINITY)
                .is_some_and(|hit| hit.t > 5.0),
            "starting past the near face must find the far one"
        );
    }

    #[test]
    fn pick_returns_the_nearest_part_and_its_node() {
        // Two cubes on the z axis, each its own node, the second further away.
        let mut model = demo_cube_model();
        let corners = model.vertices.len() as u32;
        let triangles = model.indices.len() as u32 / 3;
        model.nodes.push(model.nodes[0].clone());
        let far: Vec<Vertex> = model
            .vertices
            .iter()
            .map(|v| Vertex {
                position: v.position + Vec3::new(0.0, 0.0, -4.0),
                ..*v
            })
            .collect();
        model.vertices.extend(far);
        let shifted: Vec<u32> = (0..model.indices.len())
            .map(|i| model.indices[i] + corners)
            .collect();
        model.indices.extend(shifted);
        model.triangles.node = (0..triangles)
            .map(|_| 0u32)
            .chain((0..triangles).map(|_| 1u32))
            .collect();

        let bvh = SceneBvh::build(&model);
        let origin = Vec3::new(0.0, 0.5, 6.0);
        let hit = bvh
            .pick(&model, origin, Vec3::NEG_Z, 0.0, f32::INFINITY, |_| true)
            .expect("the ray crosses both cubes");
        assert_eq!(hit.node, 0, "the near cube owns the hit");
        assert!((hit.t - 5.5).abs() < 1e-4, "{}", hit.t);

        // Rejecting the near part must fall through to the far one, not miss.
        let behind = bvh
            .pick(&model, origin, Vec3::NEG_Z, 0.0, f32::INFINITY, |node| {
                node != 0
            })
            .expect("the far cube is still there");
        assert_eq!(behind.node, 1);
        assert!(behind.t > hit.t);

        // Rejecting everything picks nothing.
        assert!(
            bvh.pick(&model, origin, Vec3::NEG_Z, 0.0, f32::INFINITY, |_| false)
                .is_none()
        );
    }

    #[test]
    fn segment_through_cube_is_occluded() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // Straight through the cube centre (it spans z ∈ [-0.5, 0.5], y ∈ [0.03, 1.03]).
        let occluded =
            bvh.segment_occluded(&model, Vec3::new(0.0, 0.5, 5.0), Vec3::new(0.0, 0.5, -5.0));
        assert!(occluded);
    }

    #[test]
    fn segment_clear_of_cube_is_not_occluded() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // Well off to the side, never reaching the box.
        let occluded = bvh.segment_occluded(
            &model,
            Vec3::new(10.0, 10.0, 10.0),
            Vec3::new(11.0, 10.0, 10.0),
        );
        assert!(!occluded);
    }

    #[test]
    fn target_on_surface_is_not_self_occluded() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // A point on the +z face, viewed head-on from in front: the face it sits on
        // is the queried surface (within the end slack), not an occluder.
        let occluded =
            bvh.segment_occluded(&model, Vec3::new(0.0, 0.5, 5.0), Vec3::new(0.0, 0.5, 0.5));
        assert!(!occluded);
    }

    #[test]
    fn bvh_matches_brute_force_over_a_grid() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // Eye points circling the cube; targets sweeping a grid across it. Every
        // combination must give the same answer as the linear scan.
        let eyes = [
            Vec3::new(0.0, 0.5, 6.0),
            Vec3::new(6.0, 0.5, 0.0),
            Vec3::new(0.0, 6.0, 0.0),
            Vec3::new(4.0, 4.0, 4.0),
            Vec3::new(-4.0, 0.5, -4.0),
        ];
        for eye in eyes {
            for ix in -3..=3 {
                for iy in -3..=3 {
                    for iz in -3..=3 {
                        let target =
                            Vec3::new(ix as f32 * 0.25, 0.53 + iy as f32 * 0.25, iz as f32 * 0.25);
                        assert_eq!(
                            bvh.segment_occluded(&model, eye, target),
                            occluded_brute_force(&model, eye, target),
                            "mismatch for eye {eye:?} target {target:?}",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ray_toward_cube_is_occluded_and_away_is_not() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        let origin = Vec3::new(0.0, 0.5, 5.0);
        // Toward the cube (its +z face sits at z = 0.5, i.e. 4.5 away).
        assert!(bvh.ray_occluded(&model, origin, Vec3::NEG_Z, f32::INFINITY));
        // Pointing away from it.
        assert!(!bvh.ray_occluded(&model, origin, Vec3::Z, f32::INFINITY));
    }

    #[test]
    fn ray_t_max_clips_the_hit() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        let origin = Vec3::new(0.0, 0.5, 5.0);
        // The nearest face is 4.5 along -z: a shorter ray misses, a longer hits.
        assert!(!bvh.ray_occluded(&model, origin, Vec3::NEG_Z, 4.0));
        assert!(bvh.ray_occluded(&model, origin, Vec3::NEG_Z, 5.0));
    }

    #[test]
    fn unbounded_ray_matches_a_long_segment() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // Rays from an eye through a jittered grid must agree with a segment
        // reaching past the model. The segment is 100 long, not enormous: its
        // slack margins scale with its length, and margins wide enough to
        // swallow the geometry would make the two queries legitimately differ.
        // The grid is jittered off the cube's face planes — a ray exactly
        // grazing a box edge is a measure-zero case where the AABB and
        // triangle tests can disagree by an ulp.
        let eye = Vec3::new(4.0, 4.0, 4.0);
        for ix in -3..=3 {
            for iy in -3..=3 {
                let target = Vec3::new(ix as f32 * 0.26 + 0.005, 0.53 + iy as f32 * 0.26, 0.007);
                let dir = (target - eye).normalize();
                assert_eq!(
                    bvh.ray_occluded(&model, eye, dir, f32::INFINITY),
                    bvh.segment_occluded(&model, eye, eye + dir * 100.0),
                    "mismatch for target {target:?}",
                );
            }
        }
    }

    #[test]
    fn ray_matches_brute_force_over_a_grid() {
        let model = demo_cube_model();
        let bvh = Bvh::build(&model);
        // Brute-force reference for the ray query, mirroring the segment grid
        // test: every triangle scanned linearly with the same (t_min, t_max).
        let brute_force = |origin: Vec3, dir: Vec3, t_max: f32| {
            (0..model.indices.len() / 3).any(|t| {
                let [a, b, c] = triangle_positions(&model, t as u32);
                ray_triangle_t(origin, dir, a, b, c).is_some_and(|t| t > 0.0 && t < t_max)
            })
        };
        let eyes = [
            Vec3::new(0.0, 0.5, 6.0),
            Vec3::new(6.0, 0.5, 0.0),
            Vec3::new(4.0, 4.0, 4.0),
        ];
        for eye in eyes {
            for ix in -3..=3 {
                for iy in -3..=3 {
                    for iz in -3..=3 {
                        // Jittered off the cube's face planes: an exact edge
                        // graze is a measure-zero case where the AABB and
                        // triangle tests may disagree by an ulp.
                        let target = Vec3::new(
                            ix as f32 * 0.26 + 0.005,
                            0.53 + iy as f32 * 0.26,
                            iz as f32 * 0.26 + 0.007,
                        );
                        let dir = (target - eye).normalize();
                        for t_max in [2.0, 6.0, f32::INFINITY] {
                            assert_eq!(
                                bvh.ray_occluded(&model, eye, dir, t_max),
                                brute_force(eye, dir, t_max),
                                "mismatch for eye {eye:?} target {target:?} t_max {t_max}",
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn empty_model_never_occludes() {
        let model = ModelData::default();
        let bvh = Bvh::build(&model);
        assert!(!bvh.segment_occluded(&model, Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)));
    }

    /// A two-part model: node 0's triangle straddles the origin (blocking a
    /// segment through it), node 1's triangle sits far off to the side. Hiding the
    /// blocking part must drop the occlusion — and never test it.
    #[test]
    fn scene_bvh_skips_hidden_parts() {
        use crate::{TriangleData, Vertex};
        use glam::{Vec2, Vec4};

        let positions = [
            // Node 0: a triangle in the z = 0 plane covering the origin.
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            // Node 1: a triangle far away on +x, never on the line of sight.
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::new(101.0, 0.0, 0.0),
            Vec3::new(100.0, 1.0, 0.0),
        ];
        let model = ModelData {
            vertices: positions
                .iter()
                .map(|&position| Vertex {
                    position,
                    normal: Vec3::Y,
                    uv: Vec2::ZERO,
                    tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                    vertex_color: Vec4::ONE,
                })
                .collect(),
            indices: vec![0, 1, 2, 3, 4, 5],
            triangles: TriangleData {
                node: vec![0, 1],
                ..Default::default()
            },
            ..Default::default()
        };
        let bvh = SceneBvh::build(&model);

        let origin = Vec3::new(0.0, 0.0, 5.0);
        let target = Vec3::new(0.0, 0.0, -5.0);

        // Nothing hidden: node 0 blocks the segment.
        assert!(bvh.segment_occluded(&model, origin, target, &[]));
        // Hiding the unrelated node 1 changes nothing.
        assert!(bvh.segment_occluded(&model, origin, target, &[1]));
        // Hiding node 0 removes the only occluder.
        assert!(!bvh.segment_occluded(&model, origin, target, &[0]));
    }

    /// A tree deeper than the traversal stack used to lose whole subtrees to
    /// every query. Triangles at exponentially growing distances are the worst
    /// case for a midpoint split - each one peels a single triangle off - and
    /// every one of them must still be found.
    #[test]
    fn a_degenerately_clustered_mesh_is_still_searched_to_the_bottom() {
        use glam::{Vec2, Vec4};
        let count = 300u32;
        let mut positions = Vec::new();
        for index in 0..count {
            let x = 1.3f32.powi(index as i32);
            positions.extend([
                Vec3::new(x, 0.0, 0.0),
                Vec3::new(x + 0.1, 0.0, 0.0),
                Vec3::new(x, 0.1, 0.0),
            ]);
        }
        let model = ModelData {
            vertices: positions
                .iter()
                .map(|&position| Vertex {
                    position,
                    normal: Vec3::Z,
                    uv: Vec2::ZERO,
                    tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                    vertex_color: Vec4::ONE,
                })
                .collect(),
            indices: (0..count * 3).collect(),
            ..Default::default()
        };
        let bvh = Bvh::build(&model);

        for triangle in 0..count {
            let [a, b, c] = triangle_positions(&model, triangle);
            let centre = (a + b + c) / 3.0;
            let hit = bvh
                .closest_point(&model, centre, f32::INFINITY)
                .expect("the mesh has faces");
            assert!(
                hit.distance_squared <= centre.length_squared() * 1.0e-10,
                "triangle {triangle} was not reached: nearest found at {}",
                hit.distance_squared.sqrt()
            );
        }
    }
}
