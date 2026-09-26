//! Which side of the surface a point is on, for a surface that does not
//! reliably have sides.
//!
//! ## Why not a ray cast
//!
//! The usual inside test — count crossings along a ray — needs a closed
//! manifold, which is precisely what an object being shrinkwrapped does not
//! have. On a kitbash of interpenetrating parts, a mesh with holes, or one with
//! an inverted shell, the parity answer flips at random and the extracted shell
//! comes out full of bubbles.
//!
//! The **generalized winding number** (Jacobson, Kavan and Sorkine-Hornung,
//! 2013) has no such precondition. It is the sum of the signed solid angles the
//! triangles subtend at the query point, over 4π: exactly 1 strictly inside a
//! closed surface, 0 strictly outside, and — the useful part — a smooth field in
//! between that degrades gracefully rather than flipping. Two shells that
//! overlap read 2 inside the overlap, a hole makes the value sag rather than
//! invert, and a surface whose winding crosses ½ is the honest boundary of "the
//! solid this soup describes".
//!
//! ## Why it is fast enough
//!
//! Summed triangle by triangle it is O(triangles) per query, and a wrap makes
//! millions of queries. Barill, Dickson, Schmidt, Levin and Jacobson (2018)
//! observe that a cluster of triangles seen from far enough away looks like a
//! single dipole: its area-weighted normal at its area-weighted centroid. This
//! builds that hierarchy once and, per query, descends only into the clusters
//! whose own extent is a significant fraction of their distance — so a point far
//! from the surface costs a handful of dipole evaluations and a point near it
//! costs the exact sum over its own neighbourhood.

use glam::Vec3;

/// How many times its own radius a cluster must be away before its dipole
/// approximation is used instead of descending into it.
///
/// The approximation's error falls off as `(radius / distance)²`, so two gives
/// roughly a quarter of the cluster's own subtended angle — far below the ½
/// threshold the sign is read against, and the sign is all this is used for.
const FAR_FIELD_RATIO: f32 = 2.0;

/// Triangles per leaf. The exact solid angle costs a handful of dot products
/// and one `atan2`, so a leaf this size is cheaper to sum than to describe.
const LEAF_SIZE: usize = 8;

/// `1 / 4π`.
const INVERSE_SOLID_ANGLE: f32 = 0.079_577_47;

#[derive(Debug, Clone, Copy)]
struct Node {
    /// Area-weighted centroid: where the cluster's dipole sits.
    centroid: Vec3,
    /// Area-weighted normal sum — the dipole moment itself. A cluster that folds
    /// back on itself has a small one, and correctly contributes little.
    moment: Vec3,
    /// Distance from `centroid` to the farthest point of any triangle in the
    /// cluster.
    radius: f32,
    /// Leaf (`count > 0`): `first` is the start of its run in [`Tree::order`].
    /// Internal: `first` is the left child, and the right child is `first + 1`.
    first: u32,
    count: u32,
}

/// A hierarchy over one proxy's triangles, for winding-number queries.
pub(super) struct Tree {
    /// Three positions per triangle, in the order the caller gave them.
    triangles: Vec<[Vec3; 3]>,
    nodes: Vec<Node>,
    /// Permutation of `0..triangles.len()`, grouped so each leaf owns a run.
    order: Vec<u32>,
}

impl Tree {
    /// Build over an indexed triangle mesh. Positions are copied, because the
    /// query is the innermost loop of a wrap and an indirection per corner
    /// there costs more than the copy does once.
    pub(super) fn build(positions: &[f32], indices: &[u32]) -> Self {
        let mut triangles = Vec::with_capacity(indices.len() / 3);
        for corners in indices.as_chunks::<3>().0 {
            let at = |index: u32| {
                let base = index as usize * 3;
                match positions.get(base..base + 3) {
                    Some(values) => Vec3::new(values[0], values[1], values[2]),
                    None => Vec3::ZERO,
                }
            };
            triangles.push([at(corners[0]), at(corners[1]), at(corners[2])]);
        }

        let count = triangles.len();
        if count == 0 {
            return Self {
                triangles,
                nodes: Vec::new(),
                order: Vec::new(),
            };
        }
        let centroids: Vec<Vec3> = triangles
            .iter()
            .map(|corners| (corners[0] + corners[1] + corners[2]) / 3.0)
            .collect();
        let mut order: Vec<u32> = (0..count as u32).collect();
        let mut nodes = Vec::with_capacity(2 * count / LEAF_SIZE + 2);
        nodes.push(Node {
            centroid: Vec3::ZERO,
            moment: Vec3::ZERO,
            radius: 0.0,
            first: 0,
            count: 0,
        });
        build_node(0, 0, count, &mut nodes, &mut order, &triangles, &centroids);
        Self {
            triangles,
            nodes,
            order,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The generalized winding number at `query`: ~1 inside, ~0 outside.
    pub(super) fn winding(&self, query: Vec3) -> f32 {
        if self.nodes.is_empty() {
            return 0.0;
        }
        let mut total = 0.0f32;
        // Depth bounded by the tree, which halves at every level; 64 is far
        // more than any realistic triangle count reaches.
        let mut stack = [0u32; 64];
        let mut sp = 1usize;
        stack[0] = 0;
        while sp > 0 {
            sp -= 1;
            let node = self.nodes[stack[sp] as usize];
            let offset = query - node.centroid;
            let distance = offset.length();
            if node.count == 0 && distance > node.radius * FAR_FIELD_RATIO && distance > 0.0 {
                // Far field: one dipole stands in for the whole cluster.
                total += INVERSE_SOLID_ANGLE * node.moment.dot(offset)
                    / (distance * distance * distance);
                continue;
            }
            if node.count > 0 {
                for &triangle in
                    &self.order[node.first as usize..(node.first + node.count) as usize]
                {
                    let [a, b, c] = self.triangles[triangle as usize];
                    total += INVERSE_SOLID_ANGLE * solid_angle(query, a, b, c);
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = node.first;
                stack[sp + 1] = node.first + 1;
                sp += 2;
            }
        }
        total
    }

    /// Whether `query` is inside the solid the surface describes.
    ///
    /// The half-way point, as the generalized winding number's own definition
    /// asks: it is exactly ½ *on* a closed surface, so the test is the same one
    /// a signed distance's sign would make, and stays meaningful where a closed
    /// surface's would not exist.
    pub(super) fn is_inside(&self, query: Vec3) -> bool {
        self.winding(query) > 0.5
    }
}

/// Recursive median split on the widest axis of the cluster's centroids, the
/// same shape [`review_model::Bvh`] is built with.
fn build_node(
    index: usize,
    start: usize,
    end: usize,
    nodes: &mut Vec<Node>,
    order: &mut [u32],
    triangles: &[[Vec3; 3]],
    centroids: &[Vec3],
) {
    // The cluster's dipole and extent, whatever it turns out to be.
    let mut area_total = 0.0f32;
    let mut moment = Vec3::ZERO;
    let mut weighted = Vec3::ZERO;
    for &triangle in &order[start..end] {
        let [a, b, c] = triangles[triangle as usize];
        let cross = (b - a).cross(c - a);
        let area = cross.length() * 0.5;
        area_total += area;
        // `cross` is already `2 * area * normal`, so the dipole moment is half
        // of it — no normalization, and a degenerate triangle contributes
        // nothing rather than a NaN direction.
        moment += cross * 0.5;
        weighted += centroids[triangle as usize] * area;
    }
    let centroid = if area_total > 0.0 {
        weighted / area_total
    } else {
        centroids[order[start] as usize]
    };
    let mut radius = 0.0f32;
    for &triangle in &order[start..end] {
        for corner in triangles[triangle as usize] {
            radius = radius.max((corner - centroid).length());
        }
    }
    nodes[index] = Node {
        centroid,
        moment,
        radius,
        first: start as u32,
        count: (end - start) as u32,
    };

    if end - start <= LEAF_SIZE {
        return;
    }

    // Widest axis of the centroid spread.
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for &triangle in &order[start..end] {
        lo = lo.min(centroids[triangle as usize]);
        hi = hi.max(centroids[triangle as usize]);
    }
    let extent = hi - lo;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let middle = (lo[axis] + hi[axis]) * 0.5;

    let slice = &mut order[start..end];
    let mut split = partition(slice, |triangle| {
        centroids[triangle as usize][axis] < middle
    });
    // Every centroid on one side of the plane (coincident triangles): split down
    // the middle so the recursion still terminates.
    if split == 0 || split == slice.len() {
        split = slice.len() / 2;
    }

    let left = nodes.len();
    nodes.push(nodes[index]);
    nodes.push(nodes[index]);
    nodes[index].first = left as u32;
    nodes[index].count = 0;
    build_node(
        left,
        start,
        start + split,
        nodes,
        order,
        triangles,
        centroids,
    );
    build_node(
        left + 1,
        start + split,
        end,
        nodes,
        order,
        triangles,
        centroids,
    );
}

/// In-place stable-enough partition: everything the predicate accepts moves to
/// the front, and the boundary is returned.
fn partition(slice: &mut [u32], predicate: impl Fn(u32) -> bool) -> usize {
    let mut next = 0;
    for index in 0..slice.len() {
        if predicate(slice[index]) {
            slice.swap(next, index);
            next += 1;
        }
    }
    next
}

/// The signed solid angle triangle `abc` subtends at `query`.
///
/// Van Oosterom and Strackee's formula, which is numerically well behaved right
/// up to the surface — unlike the spherical-excess form, which loses every digit
/// as the triangle flattens out.
fn solid_angle(query: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let a = a - query;
    let b = b - query;
    let c = c - query;
    let (la, lb, lc) = (a.length(), b.length(), c.length());
    if la == 0.0 || lb == 0.0 || lc == 0.0 {
        // The query is *on* a corner: the angle is undefined there, and the
        // caller's band makes the point irrelevant either way.
        return 0.0;
    }
    let numerator = a.dot(b.cross(c));
    let denominator = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
    2.0 * numerator.atan2(denominator)
}

#[cfg(test)]
mod tests {
    use review_model::demo_cube_model;

    use super::*;
    use crate::remesh::proxy;
    use crate::submesh::partition as split;

    fn cube_tree() -> (Tree, glam::Vec3) {
        let model = demo_cube_model();
        let (pieces, _) = split(&model, None);
        let borrowed: Vec<_> = pieces.iter().collect();
        let proxy = proxy::build(&borrowed);
        let centre = proxy.bounds.center();
        (Tree::build(&proxy.positions, &proxy.indices), centre)
    }

    #[test]
    fn winding_number_is_one_inside_the_cube_and_zero_outside() {
        let (tree, centre) = cube_tree();

        assert!(
            (tree.winding(centre) - 1.0).abs() < 1.0e-3,
            "inside a closed surface the winding number is 1, got {}",
            tree.winding(centre)
        );
        for offset in [
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::new(0.0, 0.0, -5.0),
            Vec3::new(3.0, 3.0, 3.0),
        ] {
            let outside = centre + offset;
            assert!(
                tree.winding(outside).abs() < 1.0e-3,
                "outside it is 0, got {} at {outside:?}",
                tree.winding(outside)
            );
        }
    }

    #[test]
    fn the_inside_test_agrees_with_the_cube_it_was_built_from() {
        let (tree, centre) = cube_tree();
        // The demo cube is one unit across in x and z, one in y, centred at
        // `centre`; sample a grid and compare with the box test.
        for x in -4..=4 {
            for y in -4..=4 {
                for z in -4..=4 {
                    let point = centre + Vec3::new(x as f32, y as f32, z as f32) * 0.17;
                    let offset = (point - centre).abs();
                    // Skip points within a hair of a face, where "inside" is a
                    // coin toss for both tests.
                    if (offset - Vec3::splat(0.5)).abs().min_element() < 0.02 {
                        continue;
                    }
                    let expected = offset.cmplt(Vec3::splat(0.5)).all();
                    assert_eq!(
                        tree.is_inside(point),
                        expected,
                        "at {point:?}, winding {}",
                        tree.winding(point)
                    );
                }
            }
        }
    }

    #[test]
    fn two_overlapping_shells_still_read_as_inside() {
        // The kitbash case the whole operation exists for: a winding number of
        // 2 inside the overlap, which the ½ test reads as solid — where a
        // ray-parity test would call it empty.
        let model = demo_cube_model();
        let (pieces, _) = split(&model, None);
        let borrowed: Vec<_> = pieces.iter().collect();
        let proxy = proxy::build(&borrowed);
        let mut positions = proxy.positions.clone();
        let mut indices = proxy.indices.clone();
        let base = (positions.len() / 3) as u32;
        // A second copy of the cube, offset by a third of its width so the two
        // interpenetrate.
        for (slot, value) in proxy.positions.iter().enumerate() {
            positions.push(if slot % 3 == 0 { value + 0.33 } else { *value });
        }
        indices.extend(proxy.indices.iter().map(|&index| index + base));

        let tree = Tree::build(&positions, &indices);
        let overlap = proxy.bounds.center() + Vec3::new(0.2, 0.0, 0.0);

        assert!(
            tree.winding(overlap) > 1.5,
            "inside both shells the winding number is 2, got {}",
            tree.winding(overlap)
        );
        assert!(tree.is_inside(overlap));
    }
}
