//! The voxel remesher (experimental upstream): a new closed surface pulled out
//! of a voxelization of the input, sharing none of its topology.
//!
//! Its output is an *unindexed* soup — three positions per triangle — and its
//! size is only known after the fact, so the call is count-then-fill: once with
//! no destination for an upper bound, then with a buffer that large.
//!
//! meshoptimizer's `assert`s abort the process in every build profile, so each
//! one on this entry point is satisfied here before the call: a resolution in
//! `4..=256`, a whole number of triangles, a 12-byte position stride, and a
//! destination that is present whenever a capacity is given.

#![cfg(has_meshopt)]

use crate::OptError;
use crate::meshopt::options;

use super::*;

/// Floats per output triangle (three corners of three floats).
const TRIANGLE_FLOATS: usize = 9;

/// Smallest and largest voxel resolution the library accepts: grid steps
/// across the input's longest side.
pub const VOXEL_RESOLUTION_MIN: u32 = 4;
pub const VOXEL_RESOLUTION_MAX: u32 = 256;

fn check_remesh_arguments(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    resolution: u32,
    flags: u32,
) -> Result<(), OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;
    if !(VOXEL_RESOLUTION_MIN..=VOXEL_RESOLUTION_MAX).contains(&resolution) {
        return Err(OptError::InvalidParameter {
            name: "voxel resolution",
            value: resolution as f32,
        });
    }
    // Upstream keeps a private debug bit high in the mask; only the two
    // documented options may reach it from here.
    if flags & !(options::remesh::SHELL | options::remesh::SOLVE) != 0 {
        return Err(OptError::InvalidParameter {
            name: "voxel remesh options",
            value: flags as f32,
        });
    }
    Ok(())
}

/// An upper bound on the triangles [`remesh`] will produce for these
/// arguments — the counting pass, which skips the vertex solve and is much
/// cheaper than the real one.
pub fn remesh_bound(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    resolution: u32,
    flags: u32,
) -> Result<usize, OptError> {
    check_remesh_arguments(indices, positions, vertex_count, resolution, flags)?;
    // SAFETY: a null destination with a zero capacity is the documented
    // counting call (upstream asserts exactly `destination || max == 0`);
    // `indices` is non-empty, whole-triangle and in range; `positions` is
    // exactly `vertex_count * 3` finite floats at the 12-byte stride upstream
    // asserts on; `resolution` is in 4..=256 (checked above, and it fits a C
    // int); `flags` holds only documented bits.
    Ok(unsafe {
        crate::ffi::meshopt_remesh(
            std::ptr::null_mut(),
            0,
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            resolution as std::ffi::c_int,
            flags,
        )
    })
}

/// Remesh into a triangle soup of at most `capacity` triangles: nine floats per
/// triangle, wound so each faces out of the solid. `capacity` should come from
/// [`remesh_bound`] with the same arguments, which upstream guarantees is
/// enough; a result that would not fit is reported rather than truncated.
pub fn remesh(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    resolution: u32,
    flags: u32,
    capacity: usize,
) -> Result<Vec<f32>, OptError> {
    check_remesh_arguments(indices, positions, vertex_count, resolution, flags)?;
    if capacity == 0 {
        return Ok(Vec::new());
    }
    let length = capacity
        .checked_mul(TRIANGLE_FLOATS)
        .ok_or(OptError::SizeOverflow)?;
    let mut destination = vec![0.0f32; length];
    // SAFETY: `destination` holds exactly `capacity * 9` floats, the size the
    // declared `max_triangle_count` promises, and is non-null; the remaining
    // arguments are validated exactly as in `remesh_bound`.
    let produced = unsafe {
        crate::ffi::meshopt_remesh(
            destination.as_mut_ptr(),
            capacity,
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            resolution as std::ffi::c_int,
            flags,
        )
    };
    // With too small a buffer the call writes what fits and returns a *bound*;
    // a count past the capacity means the result is incomplete.
    if produced > capacity {
        return Err(OptError::BadResult { produced, capacity });
    }
    destination.truncate(produced * TRIANGLE_FLOATS);
    if destination.iter().any(|value| !value.is_finite()) {
        return Err(OptError::NonFiniteStream);
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube, eight shared corners, wound outward.
    fn cube() -> (Vec<u32>, Vec<f32>) {
        let positions = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0,
        ];
        let indices = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, //
            3, 6, 2, 3, 7, 6, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5,
        ];
        (indices, positions)
    }

    #[test]
    fn a_resolution_outside_the_library_range_is_refused() {
        let (indices, positions) = cube();
        for resolution in [0, 3, 257, 10_000] {
            assert!(matches!(
                remesh_bound(&indices, &positions, 8, resolution, 0),
                Err(OptError::InvalidParameter { .. })
            ));
        }
    }

    #[test]
    fn an_undocumented_option_bit_is_refused() {
        let (indices, positions) = cube();
        assert!(matches!(
            remesh_bound(&indices, &positions, 8, 16, 1 << 30),
            Err(OptError::InvalidParameter { .. })
        ));
    }

    #[test]
    fn the_bound_covers_the_result_and_the_result_is_repeatable() {
        let (indices, positions) = cube();
        let flags = options::remesh::SOLVE;
        let bound = remesh_bound(&indices, &positions, 8, 16, flags).expect("counts");
        let first = remesh(&indices, &positions, 8, 16, flags, bound).expect("remeshes");
        assert!(!first.is_empty());
        assert!(first.len() / TRIANGLE_FLOATS <= bound);
        let second = remesh(&indices, &positions, 8, 16, flags, bound).expect("remeshes");
        assert_eq!(
            first.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            second.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "same input, same bytes"
        );
    }

    #[test]
    fn too_small_a_buffer_is_reported_not_truncated() {
        let (indices, positions) = cube();
        let bound = remesh_bound(&indices, &positions, 8, 16, 0).expect("counts");
        assert!(bound > 1);
        assert!(matches!(
            remesh(&indices, &positions, 8, 16, 0, 1),
            Err(OptError::BadResult { .. })
        ));
    }
}
