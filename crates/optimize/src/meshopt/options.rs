//! The option bitmasks meshoptimizer's calls take.
//!
//! Plain numbers, so unlike the wrappers beside them they exist whether or not
//! the vendored tree was compiled: the stack's parameter types build their
//! bitmasks from these, and those types have to exist in either build. The
//! values are pinned against the vendored header by `build.rs`
//! (`MESHOPT_BOUND`).

/// `meshopt_Simplify*`: the `options` of `meshopt_simplify` /
/// `meshopt_simplifyWithAttributes`.
pub mod simplify {
    /// Do not move vertices that are on the topological border.
    pub const LOCK_BORDER: u32 = 1 << 0;
    // Bit 1 is `meshopt_SimplifySparse`, deliberately not exposed. It is a hint
    // that the index buffer addresses only a small subset of the vertex array,
    // and it redefines `target_error` to be relative to that subset's extents
    // rather than the mesh's. Submeshes arrive here with their vertices already
    // compacted, so the hint would never be true and the error-scale change
    // would silently mean something different from what the UI says.
    /// Treat the error limit and resulting error as absolute rather than
    /// relative to mesh extents.
    pub const ERROR_ABSOLUTE: u32 = 1 << 2;
    /// Remove disconnected parts of the mesh during simplification.
    pub const PRUNE: u32 = 1 << 3;
    /// Produce more regular triangle sizes/shapes, at some cost to quality.
    pub const REGULARIZE: u32 = 1 << 4;
    /// Allow collapses across attribute discontinuities.
    pub const PERMISSIVE: u32 = 1 << 5;
    /// Like [`REGULARIZE`] at a smaller cost to quality.
    pub const REGULARIZE_LIGHT: u32 = 1 << 6;
    /// Experimental: keep the fold line between opposite-facing triangles — the
    /// edge of a double-sided sheet — from eroding.
    pub const PRESERVE_FOLDS: u32 = 1 << 7;
    /// Experimental: clamp attribute error to the position error's scale, so a
    /// patch of high attribute variance neither dominates the collapse order nor
    /// inflates the reported error.
    pub const ERROR_CLAMPED: u32 = 1 << 8;
}

/// `meshopt_Remesh*`: the `options` of `meshopt_remesh` (experimental).
pub mod remesh {
    /// A two-sided shell around the surface instead of a filled solid.
    pub const SHELL: u32 = 1 << 0;
    /// Place output vertices to fit the source surface rather than on the grid.
    pub const SOLVE: u32 = 1 << 1;
}

/// `meshopt_Tangent*`: the `options` of `meshopt_generateTangents`.
pub mod tangent {
    // Bit 0 is `meshopt_TangentCompatible` (MikkTSpace-exact weighting),
    // deliberately not exposed: the viewer uses upstream's default weighting.
    /// A corner touching only degenerate triangles gets a zero tangent instead
    /// of an arbitrary fallback, so the caller can tell it apart.
    pub const ZERO_FALLBACK: u32 = 1 << 1;
}
