//! What every operation does without the vendored meshoptimizer.
//!
//! `third_party/meshoptimizer` is optional: delete the tree and the workspace
//! still builds, with every operation reporting [`OptError::Unavailable`]. These
//! are that half of each signature, in one place rather than interleaved with
//! the real implementations — which is also what makes it hard to add an
//! operation with no unavailable arm.

#![cfg(not(has_meshopt))]

use crate::OptError;

use super::*;

#[cfg(not(has_meshopt))]
pub fn generate_vertex_remap(
    _indices: &[u32],
    _vertex_bytes: &[u8],
    _vertex_count: usize,
    _stride: usize,
) -> Result<(Vec<u32>, usize), OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn generate_vertex_remap_custom<F>(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _attributes_match: F,
) -> Result<(Vec<u32>, usize), OptError>
where
    F: Fn(u32, u32) -> bool,
{
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn remap_index_buffer(_indices: &[u32], _remap: &[u32]) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn filter_index_buffer(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn filter_index_buffer_with_rows(
    _indices: &[u32],
    _positions: &[f32],
    _row_ids: &[u32],
    _vertex_count: usize,
) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn simplify_prune(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _target_error: f32,
) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn simplify_scale(_positions: &[f32], _vertex_count: usize) -> Result<f32, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
#[expect(clippy::too_many_arguments, reason = "mirrors the enabled signature")]
pub fn simplify(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _attributes: &SimplifyAttributes,
    _target_index_count: usize,
    _target_error: f32,
    _options: u32,
) -> Result<SimplifyOutcome, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn simplify_sloppy(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _target_index_count: usize,
    _target_error: f32,
) -> Result<SimplifyOutcome, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn optimize_vertex_cache(_indices: &[u32], _vertex_count: usize) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn optimize_overdraw(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _threshold: f32,
) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn optimize_vertex_fetch_remap(
    _indices: &[u32],
    _vertex_count: usize,
) -> Result<(Vec<u32>, usize), OptError> {
    Err(OptError::Unavailable)
}

#[cfg(not(has_meshopt))]
pub fn analyze(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _vertex_size: usize,
) -> Result<AnalysisCounters, OptError> {
    Err(OptError::Unavailable)
}
