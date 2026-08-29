//! Raw declarations for the vendored meshoptimizer C API.
//!
//! This module and [`crate::meshopt`] are the *only* `unsafe` in the crate
//! (invariant 9's meshoptimizer exception). Nothing here is public outside the
//! crate: every call goes through a checked wrapper in [`crate::meshopt`] that
//! validates the buffer/index preconditions the C side assumes, so the rest of
//! the crate — and all of `app` / `ui` / `render` — stays safe.
//!
//! Signatures mirror `third_party/meshoptimizer/meshoptimizer.h` (v1.2)
//! verbatim. `MESHOPTIMIZER_API` expands to nothing and the header wraps the
//! whole surface in `extern "C"`, so the platform default calling convention
//! applies on both sides. `size_t` maps to `usize`, `unsigned int` to `u32`.
//!
//! Keep this file in lockstep with the vendored header when refreshing it —
//! a silently changed parameter list is the one thing the compiler cannot
//! catch here.
#![cfg(has_meshopt)]

use std::ffi::{c_float, c_int, c_uint, c_void};

/// `struct meshopt_VertexCacheStatistics` — vertex transform cache analysis.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VertexCacheStatistics {
    pub vertices_transformed: c_uint,
    pub warps_executed: c_uint,
    /// Transformed vertices / triangle count.
    pub acmr: c_float,
    /// Transformed vertices / vertex count.
    pub atvr: c_float,
}

/// `struct meshopt_VertexFetchStatistics` — vertex fetch analysis.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VertexFetchStatistics {
    pub bytes_fetched: c_uint,
    /// Fetched bytes / vertex buffer size.
    pub overfetch: c_float,
}

/// `struct meshopt_OverdrawStatistics` — software-rasterizer overdraw analysis.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct OverdrawStatistics {
    pub pixels_covered: c_uint,
    pub pixels_shaded: c_uint,
    /// Shaded pixels / covered pixels.
    pub overdraw: c_float,
}

/// Simplification option flags (`meshopt_Simplify*`), as a bitmask passed in
/// the `options` parameter of [`meshopt_simplify`] /
/// [`meshopt_simplifyWithAttributes`].
pub mod simplify_options {
    use std::ffi::c_uint;

    /// Do not move vertices that are on the topological border.
    pub const LOCK_BORDER: c_uint = 1 << 0;
    // Bit 1 is `meshopt_SimplifySparse`, deliberately not exposed. It is a hint
    // that the index buffer addresses only a small subset of the vertex array,
    // and it redefines `target_error` to be relative to that subset's extents
    // rather than the mesh's. Submeshes arrive here with their vertices already
    // compacted, so the hint would never be true and the error-scale change
    // would silently mean something different from what the UI says.
    /// Treat the error limit and resulting error as absolute rather than
    /// relative to mesh extents.
    pub const ERROR_ABSOLUTE: c_uint = 1 << 2;
    /// Remove disconnected parts of the mesh during simplification.
    pub const PRUNE: c_uint = 1 << 3;
    /// Produce more regular triangle sizes/shapes, at some cost to quality.
    pub const REGULARIZE: c_uint = 1 << 4;
    /// Allow collapses across attribute discontinuities.
    pub const PERMISSIVE: c_uint = 1 << 5;
    /// Like [`REGULARIZE`] at a smaller cost to quality.
    pub const REGULARIZE_LIGHT: c_uint = 1 << 6;
}

unsafe extern "C" {
    pub fn meshopt_generateVertexRemap(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
    ) -> usize;

    pub fn meshopt_generateVertexRemapCustom(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        callback: Option<unsafe extern "C" fn(*mut c_void, c_uint, c_uint) -> c_int>,
        context: *mut c_void,
    ) -> usize;

    // `meshopt_remapVertexBuffer` is deliberately not declared. It rewrites one
    // interleaved stream, whereas a submesh's vertex data lives in several
    // parallel arrays (`Vertex` plus one UV array per extra channel) and `Vertex`
    // has no guaranteed layout to hand to C. `Submesh::apply_vertex_remap`
    // performs the identical remap in safe Rust across every array instead.
    pub fn meshopt_remapIndexBuffer(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        remap: *const c_uint,
    );

    pub fn meshopt_filterIndexBuffer(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
        vertex_stride: usize,
    ) -> usize;

    pub fn meshopt_optimizeVertexCache(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
    );

    pub fn meshopt_optimizeOverdraw(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        threshold: c_float,
    );

    /// Multi-stream vertex-fetch reorder: produces a remap instead of rewriting
    /// one interleaved buffer. This is the variant the crate uses, because a
    /// submesh's vertex data lives in several parallel arrays (the `Vertex`
    /// array plus one UV array per extra channel) rather than a single stream.
    pub fn meshopt_optimizeVertexFetchRemap(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
    ) -> usize;

    pub fn meshopt_simplify(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        target_index_count: usize,
        target_error: c_float,
        options: c_uint,
        result_error: *mut c_float,
    ) -> usize;

    pub fn meshopt_simplifyWithAttributes(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        vertex_attributes: *const c_float,
        vertex_attributes_stride: usize,
        attribute_weights: *const c_float,
        attribute_count: usize,
        vertex_lock: *const u8,
        target_index_count: usize,
        target_error: c_float,
        options: c_uint,
        result_error: *mut c_float,
    ) -> usize;

    pub fn meshopt_simplifySloppy(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        vertex_lock: *const u8,
        target_index_count: usize,
        target_error: c_float,
        result_error: *mut c_float,
    ) -> usize;

    pub fn meshopt_simplifyPrune(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        target_error: c_float,
    ) -> usize;

    pub fn meshopt_simplifyScale(
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
    ) -> c_float;

    pub fn meshopt_analyzeVertexCache(
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        cache_size: c_uint,
        warp_size: c_uint,
        primgroup_size: c_uint,
    ) -> VertexCacheStatistics;

    pub fn meshopt_analyzeVertexFetch(
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
        vertex_size: usize,
    ) -> VertexFetchStatistics;

    pub fn meshopt_analyzeOverdraw(
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
    ) -> OverdrawStatistics;
}
