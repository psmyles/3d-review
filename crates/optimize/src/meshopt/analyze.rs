//! Measuring a mesh: the ACMR / ATVR / overdraw / overfetch figures the Opt
//! stats card reports.

#![cfg(has_meshopt)]

use crate::OptError;

use super::*;

/// Measure a submesh against the cache / overdraw / fetch models, returning the
/// raw counters so a whole-mesh figure can be summed rather than averaged.
/// `vertex_size` is the size of one *rendered* vertex, so the overfetch figure
/// reflects the GPU's real vertex buffer rather than this crate's layout.
pub fn analyze(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    vertex_size: usize,
) -> Result<AnalysisCounters, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;

    // SAFETY (all three): `indices` is checked whole-triangle with every entry
    // below `vertex_count`, and `positions` is exactly `vertex_count * 3` finite
    // floats at the declared stride. All three calls only read, and return their
    // statistics by value.
    let cache = unsafe {
        crate::ffi::meshopt_analyzeVertexCache(
            indices.as_ptr(),
            indices.len(),
            vertex_count,
            CACHE_SIZE,
            WARP_SIZE,
            PRIMGROUP_SIZE,
        )
    };
    let overdraw = unsafe {
        crate::ffi::meshopt_analyzeOverdraw(
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
        )
    };
    let fetch = unsafe {
        crate::ffi::meshopt_analyzeVertexFetch(
            indices.as_ptr(),
            indices.len(),
            vertex_count,
            vertex_size.max(1),
        )
    };

    Ok(AnalysisCounters {
        vertices_transformed: u64::from(cache.vertices_transformed),
        triangles: (indices.len() / 3) as u64,
        vertices: vertex_count as u64,
        pixels_covered: u64::from(overdraw.pixels_covered),
        pixels_shaded: u64::from(overdraw.pixels_shaded),
        bytes_fetched: u64::from(fetch.bytes_fetched),
        vertex_buffer_bytes: (vertex_count as u64) * (vertex_size.max(1) as u64),
    })
}
