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

use crate::OptError;

/// Floats per position in a packed position stream.
pub const POSITION_COMPONENTS: usize = 3;
/// Byte stride of a packed position stream (`float3`, tightly packed).
pub const POSITION_STRIDE: usize = POSITION_COMPONENTS * size_of::<f32>();

/// GPU vertex-cache model used for the ACMR/ATVR figures reported in the Opt
/// stats overlay. 16 entries with 32-wide warps is meshoptimizer's own
/// "modern GPU" default, and using one fixed model keeps the numbers
/// comparable between the source and processed meshes.
const CACHE_SIZE: u32 = 16;
const WARP_SIZE: u32 = 32;
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
fn check_index_result(produced: usize, capacity: usize) -> Result<(), OptError> {
    if produced > capacity || !produced.is_multiple_of(3) {
        return Err(OptError::BadResult { produced, capacity });
    }
    Ok(())
}

/// Check a returned vertex count against the input vertex count (a remap can
/// only ever merge vertices, never invent them).
fn check_vertex_result(produced: usize, vertex_count: usize) -> Result<(), OptError> {
    if produced > vertex_count {
        return Err(OptError::BadResult {
            produced,
            capacity: vertex_count,
        });
    }
    Ok(())
}

// The `#[cfg(has_meshopt)]` / `#[cfg(not(has_meshopt))]` pairs below keep the
// public signatures identical in both configurations, so the rest of the crate
// (and its tests) compile unchanged when the vendored tree is absent.

/// Byte-equality vertex remap: vertices whose `stride` bytes are identical
/// collapse to one. `vertex_bytes` must be `vertex_count * stride` long and
/// fully initialized — meshoptimizer compares padding too, which is why callers
/// build a tightly packed key rather than casting a Rust struct.
///
/// Returns `(remap, unique_vertex_count)` where `remap[old] = new`.
#[cfg(has_meshopt)]
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

#[cfg(not(has_meshopt))]
pub fn generate_vertex_remap(
    _indices: &[u32],
    _vertex_bytes: &[u8],
    _vertex_count: usize,
    _stride: usize,
) -> Result<(Vec<u32>, usize), OptError> {
    Err(OptError::Unavailable)
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
#[cfg(has_meshopt)]
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

/// Rewrite `indices` through a remap produced by one of the remap generators.
#[cfg(has_meshopt)]
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

#[cfg(not(has_meshopt))]
pub fn remap_index_buffer(_indices: &[u32], _remap: &[u32]) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

/// Drop degenerate triangles (two corners at the same position) and duplicate
/// triangles with matching winding. Redundancy is judged on the first
/// [`POSITION_STRIDE`] bytes of each vertex, so this is fed the packed position
/// stream. Triangles that duplicate another with *opposite* winding survive —
/// meshoptimizer keeps those deliberately, since double-sided geometry needs them.
#[cfg(has_meshopt)]
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

#[cfg(not(has_meshopt))]
pub fn filter_index_buffer(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

/// Remove small disconnected components whose extent is below `target_error`
/// (relative to the mesh extent, i.e. the same scale as [`simplify`]'s error).
#[cfg(has_meshopt)]
pub fn simplify_prune(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    target_error: f32,
) -> Result<Vec<u32>, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;

    let mut destination = vec![0u32; indices.len()];
    // SAFETY: `destination` is the documented worst case (`index_count`);
    // `indices` is checked whole-triangle and in range; `positions` is exactly
    // `vertex_count * 3` finite floats matching the declared stride.
    let produced = unsafe {
        crate::ffi::meshopt_simplifyPrune(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            target_error.max(0.0),
        )
    };
    check_index_result(produced, destination.len())?;
    destination.truncate(produced);
    Ok(destination)
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

/// The mesh's scaling factor, used to convert meshoptimizer's *relative* error
/// (a fraction of the mesh extent) into world units for display.
#[cfg(has_meshopt)]
pub fn simplify_scale(positions: &[f32], vertex_count: usize) -> Result<f32, OptError> {
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;
    if vertex_count == 0 {
        return Ok(0.0);
    }
    // SAFETY: `positions` is exactly `vertex_count * 3` finite floats, matching
    // the declared `POSITION_STRIDE`; the call only reads.
    Ok(unsafe {
        crate::ffi::meshopt_simplifyScale(positions.as_ptr(), vertex_count, POSITION_STRIDE)
    })
}

#[cfg(not(has_meshopt))]
pub fn simplify_scale(_positions: &[f32], _vertex_count: usize) -> Result<f32, OptError> {
    Err(OptError::Unavailable)
}

/// Result of a simplification pass: the reduced index buffer (still referencing
/// the *original* vertex buffer) and the error meshoptimizer actually achieved.
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

/// Collapse the mesh toward `target_index_count`, stopping early if
/// `target_error` would be exceeded. `options` is a bitmask from
/// [`crate::ffi::simplify_options`].
///
/// Note the destination must be sized to `index_count`, *not* the target: the
/// simplifier can stop short of the goal on topology constraints, and the C
/// documentation is explicit that the worst case is the full input size.
#[cfg(has_meshopt)]
pub fn simplify(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    attributes: &SimplifyAttributes,
    target_index_count: usize,
    target_error: f32,
    options: u32,
) -> Result<SimplifyOutcome, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;

    let attribute_count = attributes.weights.len();
    if attribute_count > 0 {
        check_stream(&attributes.stream, vertex_count, attribute_count)?;
    }

    let target = target_index_count.min(indices.len());
    let mut destination = vec![0u32; indices.len()];
    let mut error = 0.0f32;

    let produced = if attribute_count == 0 {
        // SAFETY: `destination` is the documented worst case (`index_count`);
        // `indices` is checked whole-triangle and in range; `positions` is
        // exactly `vertex_count * 3` finite floats at the declared stride;
        // `error` is a live local written at most once.
        unsafe {
            crate::ffi::meshopt_simplify(
                destination.as_mut_ptr(),
                indices.as_ptr(),
                indices.len(),
                positions.as_ptr(),
                vertex_count,
                POSITION_STRIDE,
                target,
                target_error.max(0.0),
                options,
                &raw mut error,
            )
        }
    } else {
        let attribute_stride = attribute_count * size_of::<f32>();
        // SAFETY: as above, plus `attributes.stream` is exactly
        // `vertex_count * attribute_count` finite floats, matching the declared
        // `attribute_stride`, and `attributes.weights` holds exactly
        // `attribute_count` entries. `vertex_lock` is null, which the C API
        // documents as "no locked vertices".
        unsafe {
            crate::ffi::meshopt_simplifyWithAttributes(
                destination.as_mut_ptr(),
                indices.as_ptr(),
                indices.len(),
                positions.as_ptr(),
                vertex_count,
                POSITION_STRIDE,
                attributes.stream.as_ptr(),
                attribute_stride,
                attributes.weights.as_ptr(),
                attribute_count,
                std::ptr::null(),
                target,
                target_error.max(0.0),
                options,
                &raw mut error,
            )
        }
    };

    check_index_result(produced, destination.len())?;
    destination.truncate(produced);
    Ok(SimplifyOutcome {
        indices: destination,
        error,
    })
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

/// Topology-ignoring simplifier: much faster and hits the target far more
/// reliably than [`simplify`], at the cost of not preserving the mesh's
/// topology (it can close holes and merge nearby shells). The right choice for
/// the smallest LODs, where silhouette matters more than structure.
#[cfg(has_meshopt)]
pub fn simplify_sloppy(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    target_index_count: usize,
    target_error: f32,
) -> Result<SimplifyOutcome, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;

    let target = target_index_count.min(indices.len());
    let mut destination = vec![0u32; indices.len()];
    let mut error = 0.0f32;
    // SAFETY: `destination` is the documented worst case (`index_count`);
    // `indices` is checked whole-triangle and in range; `positions` is exactly
    // `vertex_count * 3` finite floats at the declared stride; `vertex_lock` is
    // null ("no locked vertices"); `error` is a live local.
    let produced = unsafe {
        crate::ffi::meshopt_simplifySloppy(
            destination.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            std::ptr::null(),
            target,
            target_error.max(0.0),
            &raw mut error,
        )
    };
    check_index_result(produced, destination.len())?;
    destination.truncate(produced);
    Ok(SimplifyOutcome {
        indices: destination,
        error,
    })
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

/// Reorder triangles for the post-transform vertex cache. Vertex data is
/// untouched; only the index order changes.
#[cfg(has_meshopt)]
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

#[cfg(not(has_meshopt))]
pub fn optimize_vertex_cache(_indices: &[u32], _vertex_count: usize) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

/// Reorder triangles front-to-back within cache-friendly clusters to cut
/// overdraw. `threshold` bounds how much vertex-cache efficiency may regress
/// (1.05 = up to 5%).
#[cfg(has_meshopt)]
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

#[cfg(not(has_meshopt))]
pub fn optimize_overdraw(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _threshold: f32,
) -> Result<Vec<u32>, OptError> {
    Err(OptError::Unavailable)
}

/// Vertex-fetch remap: orders vertices by first use so the GPU reads the vertex
/// buffer close to linearly, and drops any vertex the index buffer never
/// references. Returns `(remap, unique_vertex_count)` for the caller to apply to
/// every parallel vertex array.
#[cfg(has_meshopt)]
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

#[cfg(not(has_meshopt))]
pub fn optimize_vertex_fetch_remap(
    _indices: &[u32],
    _vertex_count: usize,
) -> Result<(Vec<u32>, usize), OptError> {
    Err(OptError::Unavailable)
}

/// Measure a submesh against the cache / overdraw / fetch models, returning the
/// raw counters so a whole-mesh figure can be summed rather than averaged.
/// `vertex_size` is the size of one *rendered* vertex, so the overfetch figure
/// reflects the GPU's real vertex buffer rather than this crate's layout.
#[cfg(has_meshopt)]
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

#[cfg(not(has_meshopt))]
pub fn analyze(
    _indices: &[u32],
    _positions: &[f32],
    _vertex_count: usize,
    _vertex_size: usize,
) -> Result<AnalysisCounters, OptError> {
    Err(OptError::Unavailable)
}
