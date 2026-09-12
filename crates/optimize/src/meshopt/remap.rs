//! Welding and filtering: the operations that rewrite the index buffer without
//! changing what the mesh looks like.

#![cfg(has_meshopt)]

use crate::OptError;

use super::*;

// The `#[cfg(has_meshopt)]` / `#[cfg(not(has_meshopt))]` pairs below keep the
// public signatures identical in both configurations, so the rest of the crate
// (and its tests) compile unchanged when the vendored tree is absent.

/// Byte-equality vertex remap: vertices whose `stride` bytes are identical
/// collapse to one. `vertex_bytes` must be `vertex_count * stride` long and
/// fully initialized — meshoptimizer compares padding too, which is why callers
/// build a tightly packed key rather than casting a Rust struct.
///
/// Returns `(remap, unique_vertex_count)` where `remap[old] = new`.
pub fn generate_vertex_remap(
    indices: &[u32],
    vertex_bytes: &[u8],
    vertex_count: usize,
    stride: usize,
) -> Result<(Vec<u32>, usize), OptError> {
    check_indices(indices, vertex_count)?;
    let expected = vertex_count
        .checked_mul(stride)
        .ok_or(OptError::SizeOverflow)?;
    if stride == 0 || vertex_bytes.len() != expected {
        return Err(OptError::StreamLength {
            len: vertex_bytes.len(),
            expected,
        });
    }

    let mut remap = vec![0u32; vertex_count];
    // SAFETY: `remap` has exactly `vertex_count` elements (the documented
    // destination size); `indices` is a checked whole-triangle buffer whose
    // entries all address a vertex below `vertex_count`; `vertex_bytes` is
    // exactly `vertex_count * stride` initialized bytes, so reads of `stride`
    // bytes at every vertex offset stay in bounds.
    let unique = unsafe {
        crate::ffi::meshopt_generateVertexRemap(
            remap.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_bytes.as_ptr().cast(),
            vertex_count,
            stride,
        )
    };
    check_vertex_result(unique, vertex_count)?;
    Ok((remap, unique))
}

/// Position-equality vertex remap with a caller-supplied attribute test:
/// meshoptimizer groups vertices whose positions are bitwise equal, then asks
/// `attributes_match(a, b)` whether the two are close enough to merge. This is
/// the tolerance weld — it can rejoin a seam split by slightly different normals
/// or UVs, which an exact byte weld leaves apart.
///
/// The predicate runs on the C side's stack, so it must not panic or unwind: it
/// is invoked through an `extern "C"` trampoline that catches nothing. The
/// closures the crate passes only read `Vec` elements through `get`, never index.
pub fn generate_vertex_remap_custom<F>(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    attributes_match: F,
) -> Result<(Vec<u32>, usize), OptError>
where
    F: Fn(u32, u32) -> bool,
{
    use std::ffi::{c_int, c_uint, c_void};

    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;

    /// Trampoline: recovers the predicate from `context` and adapts its `bool`
    /// to meshoptimizer's `int`.
    unsafe extern "C" fn trampoline<F>(context: *mut c_void, a: c_uint, b: c_uint) -> c_int
    where
        F: Fn(u32, u32) -> bool,
    {
        // SAFETY: `context` is the `&F` handed to `meshopt_generateVertexRemapCustom`
        // below, which outlives the call; meshoptimizer passes it back unmodified
        // and never invokes the callback after returning.
        let predicate = unsafe { &*context.cast::<F>() };
        c_int::from(predicate(a, b))
    }

    let mut remap = vec![0u32; vertex_count];
    let context: *const F = &attributes_match;
    // SAFETY: `remap` holds the documented `vertex_count` destination elements;
    // `indices` is checked whole-triangle and in range; `positions` is exactly
    // `vertex_count * 3` finite floats, matching the declared stride of 12 bytes;
    // `context` points at `attributes_match`, alive for the whole call, and is
    // only read back by `trampoline::<F>`, which is instantiated for the same `F`.
    let unique = unsafe {
        crate::ffi::meshopt_generateVertexRemapCustom(
            remap.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            Some(trampoline::<F>),
            context.cast_mut().cast(),
        )
    };
    check_vertex_result(unique, vertex_count)?;
    Ok((remap, unique))
}

/// Rewrite `indices` through a remap produced by one of the remap generators.
pub fn remap_index_buffer(indices: &[u32], remap: &[u32]) -> Result<Vec<u32>, OptError> {
    check_indices(indices, remap.len())?;

    let mut destination = vec![0u32; indices.len()];
    // SAFETY: `destination` has exactly `indices.len()` elements (the documented
    // destination size); every entry of `indices` is `< remap.len()` (checked
    // above), so each `remap[indices[i]]` lookup the C side performs is in bounds.
    unsafe {
        crate::ffi::meshopt_remapIndexBuffer(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            remap.as_ptr(),
        );
    }
    Ok(destination)
}

/// Drop degenerate triangles (two corners at the same position) and duplicate
/// triangles with matching winding. Redundancy is judged on the first
/// [`POSITION_STRIDE`] bytes of each vertex, so this is fed the packed position
/// stream. Triangles that duplicate another with *opposite* winding survive —
/// meshoptimizer keeps those deliberately, since double-sided geometry needs them.
pub fn filter_index_buffer(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
) -> Result<Vec<u32>, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;

    let mut destination = vec![0u32; indices.len()];
    // SAFETY: `destination` is the documented worst-case size (`index_count`);
    // `indices` is checked whole-triangle and in range; `positions` is exactly
    // `vertex_count * 3` floats, so reading `POSITION_STRIDE` bytes at stride
    // `POSITION_STRIDE` for each of `vertex_count` vertices stays in bounds.
    let produced = unsafe {
        crate::ffi::meshopt_filterIndexBuffer(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr().cast(),
            vertex_count,
            POSITION_STRIDE,
            POSITION_STRIDE,
        )
    };
    check_index_result(produced, destination.len())?;
    destination.truncate(produced);
    Ok(destination)
}

/// [`filter_index_buffer`] with a second stream: one `u32` per vertex naming
/// its deform row, so two triangles are duplicates only when their vertices
/// deform alike as well as sit at the same positions.
pub fn filter_index_buffer_with_rows(
    indices: &[u32],
    positions: &[f32],
    row_ids: &[u32],
    vertex_count: usize,
) -> Result<Vec<u32>, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;
    if row_ids.len() != vertex_count {
        return Err(OptError::StreamLength {
            len: row_ids.len(),
            expected: vertex_count,
        });
    }

    let streams = [
        crate::ffi::MeshoptStream {
            data: positions.as_ptr().cast(),
            size: POSITION_STRIDE,
            stride: POSITION_STRIDE,
        },
        crate::ffi::MeshoptStream {
            data: row_ids.as_ptr().cast(),
            size: size_of::<u32>(),
            stride: size_of::<u32>(),
        },
    ];
    let mut destination = vec![0u32; indices.len()];
    // SAFETY: `destination` is the documented worst-case size (`index_count`);
    // `indices` is checked whole-triangle and in range; both streams are
    // exactly `vertex_count` elements of the declared size and stride, so every
    // per-vertex read stays in bounds; two streams is below the limit of 16.
    let produced = unsafe {
        crate::ffi::meshopt_filterIndexBufferMulti(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_count,
            streams.as_ptr(),
            streams.len(),
        )
    };
    check_index_result(produced, destination.len())?;
    destination.truncate(produced);
    Ok(destination)
}
