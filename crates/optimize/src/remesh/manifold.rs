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

/// How far the proxy is from being a closed manifold surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ManifoldReport {
    /// Edges used by exactly one triangle: an open border.
    pub boundary_edges: usize,
    /// Edges used by three or more: a surface that branches, which no
    /// half-edge structure can describe.
    pub nonmanifold_edges: usize,
}

/// Count `indices`' undirected edges by how many triangles use each.
///
/// Reads the index buffer alone — the proxy is already welded by position, so
/// two triangles that meet geometrically share an index and their shared edge is
/// one entry here.
pub(super) fn report(indices: &[u32]) -> ManifoldReport {
    let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
    for triangle in indices.chunks_exact(3) {
        for corner in 0..3 {
            let a = triangle[corner];
            let b = triangle[(corner + 1) % 3];
            let key = if a < b { (a, b) } else { (b, a) };
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
    report
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
