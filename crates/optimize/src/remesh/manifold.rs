//! What a mesh's edges say about whether it is a surface.
//!
//! The rebuild wants a manifold: every edge shared by exactly two triangles,
//! every vertex a single fan. Game assets rarely are — a kitbash is a pile of
//! interpenetrating closed parts, and a wall is often a plane with no thickness
//! at all. The rebuild completes on either, it just cannot promise the surface
//! closes where the input did not, so this is a **report** the caller turns into
//! a warning rather than a gate.
//!
//! Boundary edges are counted separately because they are not a problem: an open
//! border is a legitimate thing to remesh, and `align_to_boundaries` exists for
//! it.
//!
//! The counting itself lives in [`super::topology`], which answers this and
//! every other connectivity question from one index. This used to build a
//! `HashMap` of every edge plus a second one of every vertex's incident fan,
//! which at the sizes the operation now takes is a quarter of a gigabyte of hash
//! entries to answer three integers.

/// How far the proxy is from being a closed manifold surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ManifoldReport {
    /// Edges used by exactly one triangle: an open border.
    pub boundary_edges: usize,
    /// Edges used by three or more: a surface that branches, which no
    /// half-edge structure can describe.
    pub nonmanifold_edges: usize,
    /// Vertices whose incident triangles do not form a single fan — two cones
    /// meeting at a point, or a surface pinched to a vertex.
    pub nonmanifold_vertices: usize,
}

impl ManifoldReport {
    /// Whether a half-edge structure can describe this surface. Boundaries are
    /// allowed: an open border is a legitimate thing to remesh, and the rebuild
    /// has a setting for holding its edges against one.
    pub(crate) fn is_manifold(self) -> bool {
        self.nonmanifold_edges == 0 && self.nonmanifold_vertices == 0
    }
}
