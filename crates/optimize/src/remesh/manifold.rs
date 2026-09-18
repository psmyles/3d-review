//! What a mesh's edges say about whether it is a surface.
//!
//! The field solve wants a manifold: every edge shared by exactly two triangles,
//! every vertex a single fan. Game assets rarely are — a kitbash is a pile of
//! interpenetrating closed parts, and a wall is often a plane with no thickness
//! at all. Instant Meshes completes on either, it just cannot promise the
//! rebuilt surface closes where the input did not, so this is a **report** the
//! caller turns into a warning rather than a gate.
//!
//! Boundary edges are counted separately because they are not a problem: an open
//! border is a legitimate thing to remesh, and `align_to_boundaries` exists for
//! it.

use std::collections::HashMap;

/// An undirected edge, its endpoints in ascending order so the two triangles
/// that share it agree on the key.
type Edge = (u32, u32);

fn edge_key(a: u32, b: u32) -> Edge {
    if a < b { (a, b) } else { (b, a) }
}

/// How far the proxy is from being a closed manifold surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ManifoldReport {
    /// Edges used by exactly one triangle: an open border.
    pub boundary_edges: usize,
    /// Edges used by three or more: a surface that branches, which no
    /// half-edge structure can describe.
    pub nonmanifold_edges: usize,
    /// Vertices whose incident triangles do not form a single fan — two cones
    /// meeting at a point, or a surface pinched to a vertex. Legal for a field
    /// extraction, fatal for a solver that walks a half-edge structure.
    pub nonmanifold_vertices: usize,
}

impl ManifoldReport {
    /// Whether a half-edge structure can describe this surface. Boundaries are
    /// allowed: an open border is a legitimate thing to remesh, and both engines
    /// have a setting for holding the field against one.
    pub(crate) fn is_manifold(self) -> bool {
        self.nonmanifold_edges == 0 && self.nonmanifold_vertices == 0
    }
}

/// Count `indices`' undirected edges by how many triangles use each, and its
/// vertices by whether their incident triangles form one fan.
///
/// Reads the index buffer alone — the proxy is already welded by position, so
/// two triangles that meet geometrically share an index and their shared edge is
/// one entry here.
pub(crate) fn report(indices: &[u32]) -> ManifoldReport {
    let mut uses: HashMap<Edge, u32> = HashMap::new();
    for triangle in indices.as_chunks::<3>().0 {
        for corner in 0..3 {
            let key = edge_key(triangle[corner], triangle[(corner + 1) % 3]);
            *uses.entry(key).or_insert(0) += 1;
        }
    }

    let mut report = ManifoldReport::default();
    for count in uses.values() {
        match count {
            0 | 1 => report.boundary_edges += 1,
            2 => {}
            _ => report.nonmanifold_edges += 1,
        }
    }
    report.nonmanifold_vertices = count_pinched_vertices(indices, &uses);
    report
}

/// Vertices whose incident triangles fall into more than one fan.
///
/// The test is a union-find over each vertex's own incident triangles, joined
/// when two of them share an edge *at that vertex*. One component means one fan.
/// An edge used by more than two triangles is left out of the join: it is
/// already reported as a non-manifold edge, and treating it as a link would
/// merge fans that are not actually connected through the surface.
fn count_pinched_vertices(indices: &[u32], uses: &HashMap<Edge, u32>) -> usize {
    // Per vertex, the two edges each incident triangle contributes there. The
    // triangle itself is not needed — only whether two of them meet.
    let mut incident: HashMap<u32, Vec<[Edge; 2]>> = HashMap::new();
    for corners in indices.as_chunks::<3>().0 {
        for corner in 0..3 {
            let vertex = corners[corner];
            let previous = corners[(corner + 2) % 3];
            let next = corners[(corner + 1) % 3];
            incident
                .entry(vertex)
                .or_default()
                .push([edge_key(vertex, previous), edge_key(vertex, next)]);
        }
    }

    let mut pinched = 0;
    for fan in incident.values() {
        if fan.len() < 2 {
            continue;
        }
        // Which triangle of this fan first claimed each edge; a second claimer
        // joins with it.
        let mut owner: HashMap<Edge, usize> = HashMap::new();
        let mut parent: Vec<usize> = (0..fan.len()).collect();
        fn find(parent: &mut [usize], mut node: usize) -> usize {
            while parent[node] != node {
                parent[node] = parent[parent[node]];
                node = parent[node];
            }
            node
        }
        for (slot, edges) in fan.iter().enumerate() {
            for edge in edges {
                if uses.get(edge).copied().unwrap_or(0) > 2 {
                    continue;
                }
                match owner.get(edge) {
                    Some(&first) => {
                        let (a, b) = (find(&mut parent, first), find(&mut parent, slot));
                        if a != b {
                            parent[b] = a;
                        }
                    }
                    None => {
                        owner.insert(*edge, slot);
                    }
                }
            }
        }
        let components = (0..fan.len())
            .filter(|&slot| find(&mut parent, slot) == slot)
            .count();
        if components > 1 {
            pinched += 1;
        }
    }
    pinched
}

#[cfg(test)]
mod tests {
    use review_model::demo_cube_model;

    use super::*;
    use crate::submesh::partition;

    #[test]
    fn a_closed_cube_is_manifold() {
        let model = demo_cube_model();
        let (pieces, _) = partition(&model, None);
        let borrowed: Vec<_> = pieces.iter().collect();
        let proxy = super::super::proxy::build(&borrowed);

        let report = report(&proxy.indices);

        assert_eq!(report.boundary_edges, 0, "a cube is closed");
        assert_eq!(report.nonmanifold_edges, 0);
    }

    #[test]
    fn an_open_quad_reports_its_four_border_edges() {
        // Two triangles sharing a diagonal: four border edges, one interior.
        let indices = [0u32, 1, 2, 0, 2, 3];

        let report = report(&indices);

        assert_eq!(report.boundary_edges, 4);
        assert_eq!(report.nonmanifold_edges, 0);
    }

    #[test]
    fn a_third_face_on_one_edge_is_non_manifold() {
        let indices = [0u32, 1, 2, 0, 1, 3, 0, 1, 4];

        let report = report(&indices);

        assert_eq!(report.nonmanifold_edges, 1, "edge 0-1 is used three times");
    }
}
