//! Raw declarations for the remesh bridge (`src/remesh_bridge.cpp`), and the one
//! call site that uses them.
//!
//! The crate's third and last `unsafe` surface, beside [`crate::ffi`]
//! (meshoptimizer) and [`crate::export_ffi`] (ufbx_write). Like the export
//! bridge — and unlike the meshoptimizer bindings — it goes through
//! hand-written C++ rather than calling the library directly: Instant Meshes'
//! API is a dozen stages over Eigen matrices that throw, none of which has any
//! business being named from Rust.
//!
//! The structs below mirror `remesh_bridge.h` field for field, in the same order
//! with the same names; changing one without the other is the single thing the
//! compiler cannot catch here.
//!
//! ## Ownership
//!
//! The run hands back an opaque, bridge-owned result. [`ResultHandle`] is the
//! `Drop` guard for it, so every path out of [`run`] — including a panic in the
//! copying below — releases it exactly once. See `remesh_bridge.h` for why the
//! output is not count-then-filled the way `ufbx_bridge.c` parses a scene.
#![cfg(has_instant_meshes)]

use std::ffi::{c_char, c_int};
use std::ptr::NonNull;

use glam::Vec3;

use crate::OptError;
use crate::remesh::{RemeshInput, RemeshOptions, RemeshOutput};

/// Must equal `RVO_REMESH_ERROR_LENGTH` in `remesh_bridge.h`.
pub const ERROR_LENGTH: usize = 512;

#[repr(C)]
struct RvoRemeshInput {
    positions: *const f32,
    vertex_count: usize,
    indices: *const u32,
    index_count: usize,
}

#[repr(C)]
struct RvoRemeshOptions {
    engine: u32,
    rosy: u32,
    posy: u32,
    face_count: u32,
    crease_angle_deg: f32,
    extrinsic: i32,
    align_to_boundaries: i32,
    smooth_iterations: u32,
    pure_quad: i32,
    deterministic: i32,
    adaptive_strength: f32,
    min_cost_flow: i32,
}

/// The bridge's opaque result type. Never constructed on this side.
#[repr(C)]
struct RvoRemeshResult {
    _opaque: [u8; 0],
}

unsafe extern "C" {
    fn review_remesh_run(
        input: *const RvoRemeshInput,
        options: *const RvoRemeshOptions,
        out: *mut *mut RvoRemeshResult,
        error: *mut c_char,
        error_length: usize,
    ) -> c_int;

    fn review_remesh_positions(
        result: *const RvoRemeshResult,
        vertex_count: *mut usize,
    ) -> *const f32;

    fn review_remesh_corners(
        result: *const RvoRemeshResult,
        corner_count: *mut usize,
    ) -> *const u32;

    fn review_remesh_face_offsets(
        result: *const RvoRemeshResult,
        face_count: *mut usize,
    ) -> *const u32;

    fn review_remesh_free(result: *mut RvoRemeshResult);
}

/// Owns one bridge result and frees it on drop.
struct ResultHandle(NonNull<RvoRemeshResult>);

impl Drop for ResultHandle {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a successful `review_remesh_run` and is
        // freed exactly once — this type is neither `Copy` nor `Clone`, and the
        // only way to build one is from that call.
        unsafe { review_remesh_free(self.0.as_ptr()) }
    }
}

impl ResultHandle {
    /// Copy one of the result's arrays into an owned `Vec`.
    ///
    /// `fetch` is one of the three accessors; it writes a count through the
    /// out-parameter and returns the base pointer, or null for an empty array.
    /// What that count *means* differs per accessor — vertices for the position
    /// stream, faces for the offset table, which holds one more — so `length`
    /// turns it into the number of elements to read. A null pointer with a
    /// non-zero count would be the bridge contradicting itself, so it is read as
    /// empty rather than dereferenced.
    fn copy<T: Copy>(
        &self,
        fetch: unsafe extern "C" fn(*const RvoRemeshResult, *mut usize) -> *const T,
        length: impl Fn(usize) -> usize,
    ) -> Vec<T> {
        let mut count = 0usize;
        // SAFETY: `self.0` is a live result for the whole borrow, and `count` is
        // a valid `usize` to write through. The accessors only read.
        let base = unsafe { fetch(self.0.as_ptr(), &raw mut count) };
        let len = length(count);
        if base.is_null() || len == 0 {
            return Vec::new();
        }
        // SAFETY: the bridge guarantees `len` readable `T` at `base` — the
        // arrays are `std::vector`s it sized itself — and the slice is copied
        // before this function returns, so it never outlives the handle.
        unsafe { std::slice::from_raw_parts(base, len) }.to_vec()
    }
}

/// Run the vendored retopologizer over one proxy mesh.
///
/// Every buffer is **copied** out of the bridge before the handle is dropped, so
/// nothing in the returned value borrows C++ memory. The output is then
/// re-validated on this side: the bridge is trusted to be correct, but a
/// `Vec<u32>` that indexes past the vertex array would panic somewhere far from
/// here, and the whole point of the wrapper is that it cannot.
pub(crate) fn run(
    input: &RemeshInput<'_>,
    options: &RemeshOptions,
) -> Result<RemeshOutput, OptError> {
    let _z = crate::prof::zone!("Remesh Engine");

    if input.positions.len() != input.vertex_count() * 3 {
        return Err(OptError::StreamLength {
            len: input.positions.len(),
            expected: input.vertex_count() * 3,
        });
    }
    if !input.indices.len().is_multiple_of(3) {
        return Err(OptError::IndexCount(input.indices.len()));
    }
    if input.indices.is_empty() || input.positions.is_empty() {
        return Err(OptError::EmptyMesh);
    }

    let raw_input = RvoRemeshInput {
        positions: input.positions.as_ptr(),
        vertex_count: input.vertex_count(),
        indices: input.indices.as_ptr(),
        index_count: input.indices.len(),
    };
    let raw_options = RvoRemeshOptions {
        engine: options.engine,
        rosy: options.rosy,
        posy: options.posy,
        face_count: options.face_count,
        crease_angle_deg: options.crease_angle_deg,
        extrinsic: options.extrinsic as i32,
        align_to_boundaries: options.align_to_boundaries as i32,
        smooth_iterations: options.smooth_iterations,
        pure_quad: options.pure_quad as i32,
        deterministic: options.deterministic as i32,
        adaptive_strength: options.adaptive_strength,
        min_cost_flow: options.min_cost_flow as i32,
    };

    let mut raw_result: *mut RvoRemeshResult = std::ptr::null_mut();
    let mut message = [0 as c_char; ERROR_LENGTH];
    // SAFETY: both descriptors live across the call and point at slices that
    // outlive it (they borrow the caller's buffers). `raw_result` and `message`
    // are valid writable destinations, and `message.len()` is the buffer's real
    // size, so the bridge cannot write past it.
    let ok = unsafe {
        review_remesh_run(
            &raw const raw_input,
            &raw const raw_options,
            &raw mut raw_result,
            message.as_mut_ptr(),
            message.len(),
        )
    };

    let Some(pointer) = NonNull::new(raw_result) else {
        return Err(OptError::Remesh(error_text(&message)));
    };
    let handle = ResultHandle(pointer);
    if ok == 0 {
        return Err(OptError::Remesh(error_text(&message)));
    }

    let positions = handle.copy(review_remesh_positions, |vertices| {
        vertices.saturating_mul(3)
    });
    let corners = handle.copy(review_remesh_corners, |corners| corners);
    // The offset accessor reports the face *count*; the table it points at holds
    // the trailing end offset too, so it is one longer.
    let mut face_offsets = handle.copy(review_remesh_face_offsets, |faces| faces.saturating_add(1));
    drop(handle);

    let vertices: Vec<Vec3> = positions
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[x, y, z]| Vec3::new(x, y, z))
        .collect();
    if face_offsets.is_empty() {
        face_offsets.push(0);
    }

    let output = RemeshOutput {
        positions: vertices,
        face_offsets,
        corners,
    };
    output.validate()?;
    Ok(output)
}

/// The reference density field, for the Rust port to be checked against.
///
/// `remesh/size_field.rs` reimplements `remesh_density.cpp`, and the vendored
/// C++ is about to be deleted — so the one moment the two answers can be
/// compared is while both exist. Test-only, and it goes with the C++ it calls.
///
/// Returns the field, or `None` where the reference reports there is nothing to
/// do (which the port spells as `None` too).
#[cfg(test)]
pub(crate) fn density_field_reference(
    positions: &[f32],
    normals: &[f32],
    areas: Option<&[f32]>,
    indices: &[u32],
    vertex_count: usize,
    strength: f32,
    target_edge: f32,
) -> Option<Vec<f32>> {
    unsafe extern "C" {
        fn review_density_field_probe(
            positions: *const f32,
            normals: *const f32,
            areas: *const f32,
            vertex_count: usize,
            indices: *const u32,
            index_count: usize,
            strength: f32,
            target_edge: f32,
            out: *mut f32,
        ) -> c_int;
    }

    assert!(positions.len() >= vertex_count * 3);
    assert!(normals.len() >= vertex_count * 3);
    if let Some(areas) = areas {
        assert!(areas.len() >= vertex_count);
    }
    let mut out = vec![0.0f32; vertex_count];
    // SAFETY: every pointer is taken from a slice checked above to hold at
    // least what `vertex_count` and `indices.len()` promise, and `out` is
    // exactly `vertex_count` long, which is what the probe fills. The areas
    // pointer is null only when there are no areas, which the C side tests for.
    let varies = unsafe {
        review_density_field_probe(
            positions.as_ptr(),
            normals.as_ptr(),
            areas.map_or(std::ptr::null(), <[f32]>::as_ptr),
            vertex_count,
            indices.as_ptr(),
            indices.len(),
            strength,
            target_edge,
            out.as_mut_ptr(),
        )
    };
    (varies != 0).then_some(out)
}

/// The bridge's NUL-terminated message as a `String`, or a stand-in when it
/// wrote none.
fn error_text(message: &[c_char; ERROR_LENGTH]) -> String {
    let bytes: Vec<u8> = message
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    if bytes.is_empty() {
        return "the remesher failed without reporting why".to_owned();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
