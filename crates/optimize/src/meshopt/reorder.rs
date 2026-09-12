//! The cache / overdraw / fetch reorders.
//!
//! Invisible in the viewport — they change only the order vertices are fetched
//! and triangles rasterized in, which is why the Opt overlay's measured ACMR,
//! ATVR and overdraw figures are what make them worth having.

#![cfg(has_meshopt)]

use crate::OptError;

use super::*;

/// Reorder triangles for the post-transform vertex cache. Vertex data is
/// untouched; only the index order changes.
pub fn optimize_vertex_cache(indices: &[u32], vertex_count: usize) -> Result<Vec<u32>, OptError> {
    check_indices(indices, vertex_count)?;

    let mut destination = vec![0u32; indices.len()];
    // SAFETY: `destination` has exactly `indices.len()` elements (the documented
    // destination size); `indices` is checked whole-triangle with every entry
    // below `vertex_count`, which is what the optimizer uses to size its
    // internal per-vertex tables.
    unsafe {
        crate::ffi::meshopt_optimizeVertexCache(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_count,
        );
    }
    Ok(destination)
}

/// Reorder triangles front-to-back within cache-friendly clusters to cut
/// overdraw. `threshold` bounds how much vertex-cache efficiency may regress
/// (1.05 = up to 5%).
pub fn optimize_overdraw(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    threshold: f32,
) -> Result<Vec<u32>, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;

    let mut destination = vec![0u32; indices.len()];
    // SAFETY: `destination` has exactly `indices.len()` elements; `indices` is
    // checked whole-triangle and in range; `positions` is exactly
    // `vertex_count * 3` finite floats at the declared stride.
    unsafe {
        crate::ffi::meshopt_optimizeOverdraw(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            threshold.max(1.0),
        );
    }
    Ok(destination)
}

/// Vertex-fetch remap: orders vertices by first use so the GPU reads the vertex
/// buffer close to linearly, and drops any vertex the index buffer never
/// references. Returns `(remap, unique_vertex_count)` for the caller to apply to
/// every parallel vertex array.
pub fn optimize_vertex_fetch_remap(
    indices: &[u32],
    vertex_count: usize,
) -> Result<(Vec<u32>, usize), OptError> {
    check_indices(indices, vertex_count)?;

    let mut remap = vec![0u32; vertex_count];
    // SAFETY: `remap` has exactly `vertex_count` elements (the documented
    // destination size); `indices` is checked whole-triangle with every entry
    // below `vertex_count`.
    let unique = unsafe {
        crate::ffi::meshopt_optimizeVertexFetchRemap(
            remap.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_count,
        )
    };
    check_vertex_result(unique, vertex_count)?;
    Ok((remap, unique))
}
