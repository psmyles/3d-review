//! The collapse-based operations: the three simplifiers and the scale their
//! error targets are expressed in.
//!
//! A simplify that barely removes anything is usually attribute seams, not a
//! bug: import splits every face corner, so a mesh whose normals or UVs differ at
//! every corner presents *every* edge as a discontinuity, and a
//! topology-preserving collapse cannot cross one.

#![cfg(has_meshopt)]

use crate::OptError;

use super::*;

/// The most attributes `meshopt_simplifyWithAttributes` accepts
/// (`kMaxAttributes` in the vendored `simplifier.cpp`).
pub const MAX_SIMPLIFY_ATTRIBUTES: usize = 32;

/// Every `meshopt_Simplify*` option bit this crate knows, i.e. every bit of
/// [`options::simplify`]. Anything else - `meshopt_SimplifySparse`, which would
/// redefine the error scale, or the internal solve bit upstream asserts is
/// never passed here - is refused.
const KNOWN_SIMPLIFY_OPTIONS: u32 = options::simplify::LOCK_BORDER
    | options::simplify::ERROR_ABSOLUTE
    | options::simplify::PRUNE
    | options::simplify::REGULARIZE
    | options::simplify::PERMISSIVE
    | options::simplify::REGULARIZE_LIGHT
    | options::simplify::PRESERVE_FOLDS
    | options::simplify::ERROR_CLAMPED;

fn check_simplify_options(options: u32) -> Result<(), OptError> {
    if options & !KNOWN_SIMPLIFY_OPTIONS != 0 {
        return Err(OptError::InvalidParameter {
            name: "simplify options",
            value: options as f32,
        });
    }
    Ok(())
}

/// Remove small disconnected components whose extent is below `target_error`
/// (relative to the mesh extent, i.e. the same scale as [`simplify`]'s error).
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

/// The mesh's scaling factor, used to convert meshoptimizer's *relative* error
/// (a fraction of the mesh extent) into world units for display.
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

/// Collapse the mesh toward `target_index_count`, stopping early if
/// `target_error` would be exceeded. `options` is a bitmask from
/// [`options::simplify`].
///
/// Note the destination must be sized to `index_count`, *not* the target: the
/// simplifier can stop short of the goal on topology constraints, and the C
/// documentation is explicit that the worst case is the full input size.
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
    check_simplify_options(options)?;

    // meshoptimizer asserts each of these, and its asserts abort the process.
    let attribute_count = attributes.weights.len();
    if attribute_count > MAX_SIMPLIFY_ATTRIBUTES {
        return Err(OptError::InvalidParameter {
            name: "attribute count",
            value: attribute_count as f32,
        });
    }
    if let Some(&weight) = attributes
        .weights
        .iter()
        .find(|weight| !(weight.is_finite() && **weight >= 0.0))
    {
        return Err(OptError::InvalidParameter {
            name: "attribute weight",
            value: weight,
        });
    }
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

/// Collapse the mesh toward `target_index_count` exactly as [`simplify`] does,
/// then **move** each surviving vertex to the point that best fits the surface
/// it now stands for, and re-solve its attributes there — where [`simplify`]
/// leaves every survivor at its own position with its own values.
///
/// That second half is what keeps a simplified surface shading like the
/// original. A survivor's normal described the few faces around it at full
/// detail; once the collapses have stretched its faces across a whole panel, the
/// fitted normal describes the panel, where the original one (from a bevel, say)
/// tilts the shading of everything it now reaches.
///
/// `positions` and `attributes.stream` are rewritten **in place**, and only for
/// vertices the returned index buffer references, which still addresses the
/// input vertex array. meshoptimizer moves only vertices inside a smooth surface
/// and at junctions of several seams: anything on an open border, along a
/// two-sided attribute seam or locked keeps its exact position, and a vertex
/// that moves takes every vertex sharing its position with it, bit for bit — so
/// no seam or material border can open. Attributes with a zero weight are
/// neither considered nor rewritten.
pub fn simplify_with_update(
    indices: &[u32],
    positions: &mut [f32],
    vertex_count: usize,
    attributes: &mut SimplifyAttributes,
    target_index_count: usize,
    target_error: f32,
    options: u32,
) -> Result<SimplifyOutcome, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;
    check_simplify_options(options)?;

    let attribute_count = attributes.weights.len();
    if attribute_count > MAX_SIMPLIFY_ATTRIBUTES {
        return Err(OptError::InvalidParameter {
            name: "attribute count",
            value: attribute_count as f32,
        });
    }
    if let Some(&weight) = attributes
        .weights
        .iter()
        .find(|weight| !(weight.is_finite() && **weight >= 0.0))
    {
        return Err(OptError::InvalidParameter {
            name: "attribute weight",
            value: weight,
        });
    }
    if attribute_count > 0 {
        check_stream(&attributes.stream, vertex_count, attribute_count)?;
    }

    let target = target_index_count.min(indices.len());
    // The call rewrites its index buffer in place, so it works on a copy.
    let mut result = indices.to_vec();
    let mut error = 0.0f32;
    let attribute_stride = attribute_count * size_of::<f32>();
    let attribute_data = if attribute_count == 0 {
        std::ptr::null_mut()
    } else {
        attributes.stream.as_mut_ptr()
    };
    // SAFETY: `result` is a whole-triangle, in-range index buffer of
    // `index_count` entries, which is all the call writes (it only ever shrinks
    // it); `positions` is exactly `vertex_count * 3` finite floats at the declared
    // stride and is written only within that range; `attributes.stream` is
    // exactly `vertex_count * attribute_count` finite floats at the declared
    // stride (or null with a zero count), likewise written in range;
    // `attributes.weights` holds `attribute_count` non-negative finite entries;
    // `vertex_lock` is null ("no locked vertices"); `error` is a live local.
    let produced = unsafe {
        crate::ffi::meshopt_simplifyWithUpdate(
            result.as_mut_ptr(),
            result.len(),
            positions.as_mut_ptr(),
            vertex_count,
            POSITION_STRIDE,
            attribute_data,
            attribute_stride,
            attributes.weights.as_ptr(),
            attribute_count,
            std::ptr::null(),
            target,
            target_error.max(0.0),
            options,
            &raw mut error,
        )
    };
    check_index_result(produced, result.len())?;
    result.truncate(produced);
    // A solve that went singular is rejected upstream rather than written, but
    // a non-finite value reaching the vertex buffer would poison every later
    // step, so it is checked rather than assumed.
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;
    if attribute_count > 0 {
        check_stream(&attributes.stream, vertex_count, attribute_count)?;
    }
    Ok(SimplifyOutcome {
        indices: result,
        error,
    })
}

/// Topology-ignoring simplifier: much faster and hits the target far more
/// reliably than [`simplify`], at the cost of not preserving the mesh's
/// topology (it can close holes and merge nearby shells). The right choice for
/// the smallest LODs, where silhouette matters more than structure.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One triangle, which is all the checks below need to get past the mesh
    /// validation and reach the parameter checks.
    fn triangle() -> ([u32; 3], [f32; 9]) {
        ([0, 1, 2], [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
    }

    fn simplify_with(
        attributes: &SimplifyAttributes,
        options: u32,
    ) -> Result<SimplifyOutcome, OptError> {
        let (indices, positions) = triangle();
        simplify(&indices, &positions, 3, attributes, 3, 0.01, options)
    }

    /// meshoptimizer asserts on each of these, and its asserts abort the
    /// process: the wrapper is where they have to be refused.
    #[test]
    fn a_negative_or_non_finite_weight_is_refused() {
        for weight in [-1.0, f32::NAN, f32::INFINITY] {
            let attributes = SimplifyAttributes {
                stream: vec![0.0; 3],
                weights: vec![weight],
            };
            assert!(matches!(
                simplify_with(&attributes, 0),
                Err(OptError::InvalidParameter {
                    name: "attribute weight",
                    ..
                })
            ));
        }
    }

    #[test]
    fn more_attributes_than_meshoptimizer_takes_are_refused() {
        let count = MAX_SIMPLIFY_ATTRIBUTES + 1;
        let attributes = SimplifyAttributes {
            stream: vec![0.0; 3 * count],
            weights: vec![1.0; count],
        };
        assert!(matches!(
            simplify_with(&attributes, 0),
            Err(OptError::InvalidParameter {
                name: "attribute count",
                ..
            })
        ));
    }

    #[test]
    fn the_updating_simplifier_refuses_what_the_plain_one_does() {
        let (indices, mut positions) = triangle();
        let mut attributes = SimplifyAttributes {
            stream: vec![0.0; 3],
            weights: vec![-1.0],
        };
        assert!(matches!(
            simplify_with_update(&indices, &mut positions, 3, &mut attributes, 3, 0.01, 0),
            Err(OptError::InvalidParameter {
                name: "attribute weight",
                ..
            })
        ));
        attributes.weights = vec![1.0];
        assert!(matches!(
            simplify_with_update(
                &indices,
                &mut positions,
                3,
                &mut attributes,
                3,
                0.01,
                1 << 1
            ),
            Err(OptError::InvalidParameter {
                name: "simplify options",
                ..
            })
        ));
    }

    /// `ops` copies every vertex back after an update, which is exact only
    /// because a vertex the result does not reference comes back untouched.
    #[test]
    fn an_update_leaves_unreferenced_vertices_alone() {
        // A 4x4-vertex grid, plus one stray vertex no triangle uses.
        let mut positions = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                let bump = if (x, y) == (1, 1) { 0.3 } else { 0.0 };
                positions.extend_from_slice(&[x as f32, y as f32, bump]);
            }
        }
        positions.extend_from_slice(&[9.0, 9.0, 9.0]);
        let mut indices = Vec::new();
        for y in 0..3u32 {
            for x in 0..3u32 {
                let a = y * 4 + x;
                indices.extend_from_slice(&[a, a + 1, a + 5, a, a + 5, a + 4]);
            }
        }
        let vertex_count = positions.len() / 3;
        let mut attributes = SimplifyAttributes {
            stream: (0..vertex_count).map(|v| v as f32 * 0.1).collect(),
            weights: vec![0.5],
        };
        let before = (positions.clone(), attributes.stream.clone());
        let outcome = simplify_with_update(
            &indices,
            &mut positions,
            vertex_count,
            &mut attributes,
            6,
            f32::MAX,
            0,
        )
        .unwrap();
        assert!(
            outcome.indices.len() < indices.len(),
            "nothing was simplified"
        );
        let stray = vertex_count - 1;
        assert_eq!(positions[stray * 3..], before.0[stray * 3..]);
        assert_eq!(attributes.stream[stray], before.1[stray]);
        for referenced in 0..stray {
            if !outcome.indices.contains(&(referenced as u32)) {
                assert_eq!(
                    positions[referenced * 3..referenced * 3 + 3],
                    before.0[referenced * 3..referenced * 3 + 3],
                    "vertex {referenced} is no longer used but was moved"
                );
            }
        }
    }

    #[test]
    fn an_unknown_option_bit_is_refused() {
        let attributes = SimplifyAttributes {
            stream: Vec::new(),
            weights: Vec::new(),
        };
        assert!(matches!(
            simplify_with(&attributes, 1 << 1),
            Err(OptError::InvalidParameter {
                name: "simplify options",
                ..
            })
        ));
        assert!(simplify_with(&attributes, options::simplify::LOCK_BORDER).is_ok());
    }
}
