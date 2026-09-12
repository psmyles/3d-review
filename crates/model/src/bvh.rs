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

/// Maximum traversal-stack depth. A median-split tree over any realistic mesh is
/// far shallower than this (≈ log2(triangles) + leaf depth), so the stack never
/// overflows; the bound just keeps the stack on the call frame.
const MAX_STACK: usize = 64;

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
            0, 0, n, &mut nodes, &mut order, &tri_min, &tri_max, &centroid,
        );
        nodes.shrink_to_fit();

        let tris = order.iter().map(|&p| global_tris[p as usize]).collect();
        Self { nodes, tris }
    }

    /// Whether the mesh occludes the segment from `origin` to `target`: true when
    /// any triangle is crossed strictly between the endpoints (excluding the thin
    /// [`SEGMENT_SLACK`] margins). Sub-linear in the triangle count; reads
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
struct ScenePart {
    /// The scene node these triangles belong to (matches [`crate::TriangleData::node`]),
    /// or [`NO_NODE`] when the model carries no per-triangle node info.
    node: u32,
    bvh: Bvh,
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
}

/// Recursively fill node `node_idx` for the triangles `tris[start..end]`, pushing
/// child nodes onto `nodes` as it descends.
#[allow(clippy::too_many_arguments)]
fn build_node(
    node_idx: usize,
    start: usize,
    end: usize,
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
    if count <= LEAF_SIZE {
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
    let mut mid = start
        + partition(&mut tris[start..end], |t| {
            axis_value(centroid[t as usize], axis) < split
        });
    if mid == start || mid == end {
        // A spatial split that put everything on one side; fall back to a median
        // split so the recursion still makes progress.
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

/// Möller–Trumbore ray/triangle intersection. Returns the hit parameter `t`
/// along `dir` (so the world hit is `origin + t * dir`), or `None` when the ray
/// misses or runs parallel to the triangle. Hits from either side count — the
/// model's facing is irrelevant to whether it blocks the line of sight.
fn ray_triangle_t(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
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
    use crate::demo_cube_model;

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
}
