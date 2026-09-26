//! Checked safe wrappers over the vendored meshoptimizer C API.
//!
//! Together with [`crate::ffi`] this is the crate's entire `unsafe` surface
//! (invariant 9's meshoptimizer exception). Every function here validates the
//! preconditions the C side assumes *before* the call and re-validates the
//! reported output size after it, so a malformed mesh produces an
//! [`OptError`] rather than an out-of-bounds read:
//!
//! * index buffers are a whole number of triangles and every index addresses a
//!   real vertex,
//! * position/attribute streams are exactly `vertex_count * components` long,
//! * destination buffers are sized with checked arithmetic to the worst case
//!   the C documentation states (never the hoped-for target),
//! * the returned element count is `<=` the destination capacity and, for index
//!   buffers, a multiple of three.
//!
//! When the vendored sources are absent (`cfg(has_meshopt)` unset) every
//! function short-circuits to [`OptError::Unavailable`] and the crate still
//! compiles — the same optional-vendoring contract `crates/import` has.
//!
//! ## Layout
//!
//! The shared validation is here — every wrapper checks its mesh preconditions
//! before the call and re-checks the reported element count after — with the
//! operations beside it: [`remap`] the welds and filters, [`simplify`] the
//! collapses, [`reorder`] the cache / overdraw / fetch reorders, and [`analyze`]
//! the measuring.

#![allow(
    unsafe_code,
    reason = "invariant 9: the checked safe wrappers, one per meshoptimizer call"
)]

use crate::OptError;

// The wrapper modules exist only when the vendored tree was compiled; their
// declarations are gated here rather than by an inner `#![cfg]` alone, since a
// module removed that way still leaves its `pub use` naming nothing.
#[cfg(has_meshopt)]
mod analyze;
pub mod options;
#[cfg(has_meshopt)]
mod remap;
#[cfg(has_meshopt)]
mod reorder;
#[cfg(has_meshopt)]
mod shading;
#[cfg(has_meshopt)]
mod simplify;
#[cfg(not(has_meshopt))]
mod unavailable;
#[cfg(has_meshopt)]
mod voxel;

#[cfg(has_meshopt)]
pub use analyze::*;
#[cfg(has_meshopt)]
pub use remap::*;
#[cfg(has_meshopt)]
pub use reorder::*;
#[cfg(has_meshopt)]
pub use shading::*;
#[cfg(has_meshopt)]
pub use simplify::*;
#[cfg(not(has_meshopt))]
pub use unavailable::*;
#[cfg(has_meshopt)]
pub use voxel::*;

/// Result of a simplification pass: the reduced index buffer (still referencing
/// the *original* vertex buffer) and the error meshoptimizer actually achieved.
///
/// Defined here, outside the `has_meshopt` gate, because the unavailable stubs
/// name it too.
#[derive(Debug, Clone)]
pub struct SimplifyOutcome {
    pub indices: Vec<u32>,
    /// Achieved error, relative to mesh extents unless the caller passed the
    /// absolute-error option.
    pub error: f32,
}

/// Extra per-vertex attributes to preserve during simplification, as a packed
/// stream plus one weight per component. An empty `weights` selects the
/// position-only simplifier.
#[derive(Debug, Clone, Default)]
pub struct SimplifyAttributes {
    pub stream: Vec<f32>,
    pub weights: Vec<f32>,
}

/// Floats per position in a packed position stream.
pub const POSITION_COMPONENTS: usize = 3;

/// Byte stride of a packed position stream (`float3`, tightly packed).
pub const POSITION_STRIDE: usize = POSITION_COMPONENTS * size_of::<f32>();

/// GPU vertex-cache model used for the ACMR/ATVR figures reported in the Opt
/// stats overlay. 16 entries with 32-wide warps is meshoptimizer's own
/// "modern GPU" default, and using one fixed model keeps the numbers
/// comparable between the source and processed meshes.
#[cfg_attr(not(has_meshopt), allow(dead_code))] // read only by the gated wrappers
const CACHE_SIZE: u32 = 16;

#[cfg_attr(not(has_meshopt), allow(dead_code))] // read only by the gated wrappers
const WARP_SIZE: u32 = 32;

#[cfg_attr(not(has_meshopt), allow(dead_code))] // read only by the gated wrappers
const PRIMGROUP_SIZE: u32 = 0;

/// Raw counters behind the analysis ratios, summed across submeshes before the
/// ratios are re-derived. Averaging per-submesh ratios would weight a 10-triangle
/// part the same as a 100k-triangle one and report a number no measurement could
/// reproduce; invariant 5 wants the figure the whole mesh actually exhibits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnalysisCounters {
    pub vertices_transformed: u64,
    pub triangles: u64,
    pub vertices: u64,
    pub pixels_covered: u64,
    pub pixels_shaded: u64,
    pub bytes_fetched: u64,
    pub vertex_buffer_bytes: u64,
}

impl AnalysisCounters {
    pub fn accumulate(&mut self, other: Self) {
        self.vertices_transformed += other.vertices_transformed;
        self.triangles += other.triangles;
        self.vertices += other.vertices;
        self.pixels_covered += other.pixels_covered;
        self.pixels_shaded += other.pixels_shaded;
        self.bytes_fetched += other.bytes_fetched;
        self.vertex_buffer_bytes += other.vertex_buffer_bytes;
    }

    /// Average cache misses per triangle (transformed vertices / triangles).
    pub fn acmr(self) -> f32 {
        ratio(self.vertices_transformed, self.triangles)
    }

    /// Average transformed vertices per vertex; 1.0 means each vertex is
    /// transformed exactly once.
    pub fn atvr(self) -> f32 {
        ratio(self.vertices_transformed, self.vertices)
    }

    /// Shaded pixels / covered pixels; 1.0 means no overdraw.
    pub fn overdraw(self) -> f32 {
        ratio(self.pixels_shaded, self.pixels_covered)
    }

    /// Fetched bytes / vertex buffer size; 1.0 means each byte is fetched once.
    pub fn overfetch(self) -> f32 {
        ratio(self.bytes_fetched, self.vertex_buffer_bytes)
    }
}

fn ratio(numerator: u64, denominator: u64) -> f32 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f32 / denominator as f32
    }
}

/// True when the vendored meshoptimizer sources were compiled into this build.
pub const fn available() -> bool {
    cfg!(has_meshopt)
}

/// Validate an index buffer against a vertex count: whole triangles, non-empty,
/// and every index in range. Every wrapper below calls this first, which is what
/// lets the C calls assume a well-formed mesh.
#[cfg_attr(not(has_meshopt), allow(dead_code))] // read only by the gated wrappers
fn check_indices(indices: &[u32], vertex_count: usize) -> Result<(), OptError> {
    if indices.is_empty() {
        return Err(OptError::EmptyMesh);
    }
    if !indices.len().is_multiple_of(3) {
        return Err(OptError::IndexCount(indices.len()));
    }
    if vertex_count == 0 {
        return Err(OptError::EmptyMesh);
    }
    if let Some(&index) = indices.iter().find(|&&i| i as usize >= vertex_count) {
        return Err(OptError::IndexRange {
            index,
            vertex_count,
        });
    }
    Ok(())
}

/// Validate a packed float stream: exactly `vertex_count * components` values,
/// all finite. Non-finite positions would make the simplifier's error metric and
/// the overdraw rasterizer produce garbage rather than fail, so they are
/// rejected up front.
#[cfg_attr(not(has_meshopt), allow(dead_code))] // read only by the gated wrappers
fn check_stream(stream: &[f32], vertex_count: usize, components: usize) -> Result<(), OptError> {
    let expected = vertex_count
        .checked_mul(components)
        .ok_or(OptError::SizeOverflow)?;
    if stream.len() != expected {
        return Err(OptError::StreamLength {
            len: stream.len(),
            expected,
        });
    }
    if stream.iter().any(|value| !value.is_finite()) {
        return Err(OptError::NonFiniteStream);
    }
    Ok(())
}

/// Check a returned index count against the destination capacity, and that it
/// describes whole triangles.
#[cfg_attr(not(has_meshopt), allow(dead_code))] // read only by the gated wrappers
fn check_index_result(produced: usize, capacity: usize) -> Result<(), OptError> {
    if produced > capacity || !produced.is_multiple_of(3) {
        return Err(OptError::BadResult { produced, capacity });
    }
    Ok(())
}

/// Check a returned vertex count against the input vertex count (a remap can
/// only ever merge vertices, never invent them).
#[cfg_attr(not(has_meshopt), allow(dead_code))] // read only by the gated wrappers
fn check_vertex_result(produced: usize, vertex_count: usize) -> Result<(), OptError> {
    if produced > vertex_count {
        return Err(OptError::BadResult {
            produced,
            capacity: vertex_count,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_indices_rejects_a_partial_triangle() {
        assert!(matches!(
            check_indices(&[0, 1], 3),
            Err(OptError::IndexCount(2))
        ));
    }

    #[test]
    fn check_indices_rejects_an_empty_mesh() {
        assert!(matches!(
            check_indices(&[0, 1, 2], 0),
            Err(OptError::EmptyMesh)
        ));
    }

    #[test]
    fn check_indices_rejects_an_index_past_the_buffer() {
        assert!(matches!(
            check_indices(&[0, 1, 7], 3),
            Err(OptError::IndexRange {
                index: 7,
                vertex_count: 3
            })
        ));
    }

    #[test]
    fn check_indices_accepts_a_whole_in_range_triangle() {
        assert!(check_indices(&[0, 1, 2], 3).is_ok());
    }

    #[test]
    fn check_stream_rejects_a_length_that_does_not_match() {
        assert!(matches!(
            check_stream(&[0.0; 8], 3, 3),
            Err(OptError::StreamLength {
                len: 8,
                expected: 9
            })
        ));
    }

    #[test]
    fn check_stream_rejects_a_non_finite_value() {
        let stream = [0.0, 0.0, f32::NAN, 0.0, 0.0, 0.0];
        assert!(matches!(
            check_stream(&stream, 2, 3),
            Err(OptError::NonFiniteStream)
        ));
    }

    #[test]
    fn check_stream_accepts_an_exact_finite_stream() {
        assert!(check_stream(&[0.0; 6], 2, 3).is_ok());
    }

    #[test]
    fn check_index_result_rejects_a_count_past_the_destination() {
        assert!(matches!(
            check_index_result(9, 6),
            Err(OptError::BadResult {
                produced: 9,
                capacity: 6
            })
        ));
    }

    #[test]
    fn check_index_result_rejects_a_count_that_is_not_whole_triangles() {
        // Reported as `BadResult` rather than `IndexCount`: this is meshoptimizer
        // telling us how many elements it wrote, not a caller handing us a
        // malformed buffer.
        assert!(matches!(
            check_index_result(4, 6),
            Err(OptError::BadResult {
                produced: 4,
                capacity: 6
            })
        ));
    }

    #[test]
    fn check_vertex_result_rejects_a_count_past_the_destination() {
        assert!(matches!(
            check_vertex_result(7, 4),
            Err(OptError::BadResult {
                produced: 7,
                capacity: 4
            })
        ));
    }
}
