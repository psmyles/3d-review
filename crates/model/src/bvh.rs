//! A bounding-volume hierarchy over the model's triangles, answering
//! line-of-sight occlusion queries — "does the mesh block the segment from the
//! camera to this point?" — in sub-linear time. Built once per loaded model and
//! reused across frames; the alternative (testing every triangle per query) is
//! linear in the triangle count and stutters on high-poly models.
//!
//! Invariant 1: the hierarchy stores only a permutation of triangle indices and
//! per-node bounds — it never copies the vertex/index geometry. Queries borrow
//! the shared [`ModelData`] to read triangle positions.

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
    /// Build a hierarchy over `model`'s triangles. Cost is `O(n log n)` in the
    /// triangle count; do this once per loaded model and reuse it across frames.
    /// Expects a triangulated model (the importer triangulates on load).
    pub fn build(model: &ModelData) -> Self {
        let tri_count = model.indices.len() / 3;
        let mut tris: Vec<u32> = (0..tri_count as u32).collect();

        if tri_count == 0 {
            return Self {
                nodes: vec![Node::EMPTY],
                tris,
            };
        }

        // Per-triangle bounds + centroid scratch, dropped when the build returns
        // (build acceleration data, not a persistent copy of the geometry).
        let mut tri_min = Vec::with_capacity(tri_count);
        let mut tri_max = Vec::with_capacity(tri_count);
        let mut centroid = Vec::with_capacity(tri_count);
        for t in 0..tri_count {
            let [a, b, c] = triangle_positions(model, t as u32);
            let lo = a.min(b).min(c);
            let hi = a.max(b).max(c);
            tri_min.push(lo);
            tri_max.push(hi);
            centroid.push((lo + hi) * 0.5);
        }

        let mut nodes = Vec::with_capacity(2 * tri_count);
        nodes.push(Node::EMPTY);
        build_node(
            0, 0, tri_count, &mut nodes, &mut tris, &tri_min, &tri_max, &centroid,
        );
        nodes.shrink_to_fit();
        Self { nodes, tris }
    }

    /// Whether the mesh occludes the segment from `origin` to `target`: true when
    /// any triangle is crossed strictly between the endpoints (excluding the thin
    /// [`SEGMENT_SLACK`] margins). Sub-linear in the triangle count; reads
    /// triangle positions from `model` (must be the model this was built from).
    pub fn segment_occluded(&self, model: &ModelData, origin: Vec3, target: Vec3) -> bool {
        let dir = target - origin;
        let inv_dir = dir.recip();
        let t_min = SEGMENT_SLACK;
        let t_max = 1.0 - SEGMENT_SLACK;

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
fn triangle_positions(model: &ModelData, tri: u32) -> [Vec3; 3] {
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
    fn empty_model_never_occludes() {
        let model = ModelData::default();
        let bvh = Bvh::build(&model);
        assert!(!bvh.segment_occluded(&model, Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)));
    }
}
