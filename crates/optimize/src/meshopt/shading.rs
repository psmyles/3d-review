//! Shading bases: per-corner normals and tangents generated from the geometry.
//!
//! Both return one value per index-buffer *corner*, not per vertex — two
//! corners of one vertex can legitimately want different values (either side of
//! a hard edge, either side of a mirrored-UV seam). Turning that into vertices
//! is [`crate::submesh::Submesh::split_by_corner`]'s job.
//!
//! meshoptimizer's `assert`s are compiled into every build profile (nothing
//! defines `NDEBUG`), and a failed one aborts the process. So beyond the usual
//! mesh checks, every argument upstream asserts on is validated here first: the
//! crease angle in `[0, π]`, a non-negative smoothing amount, and the strides
//! (fixed below, all within upstream's `12..=256` / `8..=256` byte windows).

#![cfg(has_meshopt)]

use crate::OptError;
use crate::meshopt::options;

use super::*;

/// Floats per normal in a packed normal stream.
const NORMAL_COMPONENTS: usize = 3;
/// Floats per UV in a packed UV stream.
const UV_COMPONENTS: usize = 2;
/// Floats per generated tangent (xyz + handedness).
pub const TANGENT_COMPONENTS: usize = 4;

/// The largest crease angle upstream accepts, bit-for-bit its `3.1415927f`
/// (`f32::consts::PI` rounds to the same float).
const MAX_CREASE: f32 = std::f32::consts::PI;
/// Upstream runs `ceil(smoothing)` passes and stops at 10; anything above only
/// risks an overflowing float-to-int conversion on the C side.
pub const MAX_SMOOTHING: f32 = 10.0;

/// Per-corner normals: `indices.len() * 3` floats. Edges whose faces meet at
/// less than `crease_radians` are smoothed across; sharper ones stay hard.
/// Corners are grouped by *position*, so a UV seam does not break smoothing.
/// `smoothing` (0 = none) relaxes the result over neighbouring triangles.
pub fn generate_normals(
    indices: &[u32],
    positions: &[f32],
    vertex_count: usize,
    crease_radians: f32,
    smoothing: f32,
) -> Result<Vec<f32>, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;
    if !crease_radians.is_finite() {
        return Err(OptError::InvalidParameter {
            name: "crease angle",
            value: crease_radians,
        });
    }
    if !smoothing.is_finite() {
        return Err(OptError::InvalidParameter {
            name: "normal smoothing",
            value: smoothing,
        });
    }
    let crease = crease_radians.clamp(0.0, MAX_CREASE);
    let smoothing = smoothing.clamp(0.0, MAX_SMOOTHING);

    let length = indices
        .len()
        .checked_mul(NORMAL_COMPONENTS)
        .ok_or(OptError::SizeOverflow)?;
    let mut result = vec![0.0f32; length];
    // SAFETY: `result` holds exactly `index_count * 3` floats, the documented
    // output size; `indices` is non-null, a whole number of triangles and in
    // range (upstream's `index_count % 3 == 0` assert); `positions` is exactly
    // `vertex_count * 3` finite floats at `POSITION_STRIDE` (12 bytes, inside the
    // asserted 12..=256 window and a multiple of 4); `crease` is in [0, π] and
    // `smoothing` in [0, 10], satisfying the remaining two asserts.
    unsafe {
        crate::ffi::meshopt_generateNormals(
            result.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            crease,
            smoothing,
        );
    }
    if result.iter().any(|value| !value.is_finite()) {
        return Err(OptError::NonFiniteStream);
    }
    Ok(result)
}

/// Per-corner tangents: `indices.len() * 4` floats, xyz plus the bitangent
/// sign in `w`, with upstream's default weighting (not MikkTSpace-exact). A
/// corner touching only degenerate UV triangles comes back as all zeros rather
/// than an arbitrary basis, so the caller can let its neighbours decide.
pub fn generate_tangents(
    indices: &[u32],
    positions: &[f32],
    normals: &[f32],
    uvs: &[f32],
    vertex_count: usize,
) -> Result<Vec<f32>, OptError> {
    check_indices(indices, vertex_count)?;
    check_stream(positions, vertex_count, POSITION_COMPONENTS)?;
    check_stream(normals, vertex_count, NORMAL_COMPONENTS)?;
    check_stream(uvs, vertex_count, UV_COMPONENTS)?;

    let length = indices
        .len()
        .checked_mul(TANGENT_COMPONENTS)
        .ok_or(OptError::SizeOverflow)?;
    let mut result = vec![0.0f32; length];
    // SAFETY: `result` holds exactly `index_count * 4` floats, the documented
    // output size; `indices` is non-null, whole-triangle and in range; the three
    // streams are exactly `vertex_count` tightly packed finite float3 / float3 /
    // float2 values, at strides of 12 / 12 / 8 bytes — inside upstream's
    // asserted 12..=256 and 8..=256 windows and multiples of 4.
    unsafe {
        crate::ffi::meshopt_generateTangents(
            result.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr(),
            vertex_count,
            POSITION_STRIDE,
            normals.as_ptr(),
            NORMAL_COMPONENTS * size_of::<f32>(),
            uvs.as_ptr(),
            UV_COMPONENTS * size_of::<f32>(),
            options::tangent::ZERO_FALLBACK,
        );
    }
    if result.iter().any(|value| !value.is_finite()) {
        return Err(OptError::NonFiniteStream);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube with its eight corners shared, CCW outward.
    fn welded_cube() -> (Vec<u32>, Vec<f32>) {
        let positions = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0,
        ];
        let indices = vec![
            0, 2, 1, 0, 3, 2, // -z
            4, 5, 6, 4, 6, 7, // +z
            0, 1, 5, 0, 5, 4, // -y
            3, 6, 2, 3, 7, 6, // +y
            0, 4, 7, 0, 7, 3, // -x
            1, 2, 6, 1, 6, 5, // +x
        ];
        (indices, positions)
    }

    fn distinct_normals(normals: &[f32]) -> usize {
        let mut seen: Vec<[u32; 3]> = normals
            .as_chunks::<3>()
            .0
            .iter()
            .map(|n| [n[0].to_bits(), n[1].to_bits(), n[2].to_bits()])
            .collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    }

    #[test]
    fn a_sharp_crease_keeps_a_cube_faceted_and_a_wide_one_smooths_it() {
        let (indices, positions) = welded_cube();
        let faceted =
            generate_normals(&indices, &positions, 8, 45f32.to_radians(), 0.0).expect("generates");
        assert_eq!(faceted.len(), indices.len() * 3);
        assert_eq!(distinct_normals(&faceted), 6, "one normal per face");

        let smooth =
            generate_normals(&indices, &positions, 8, 179f32.to_radians(), 0.0).expect("generates");
        assert_eq!(
            distinct_normals(&smooth),
            8,
            "one averaged normal per corner"
        );
    }

    #[test]
    fn a_non_finite_crease_or_smoothing_is_refused_before_the_call() {
        let (indices, positions) = welded_cube();
        assert!(matches!(
            generate_normals(&indices, &positions, 8, f32::NAN, 0.0),
            Err(OptError::InvalidParameter { .. })
        ));
        assert!(matches!(
            generate_normals(&indices, &positions, 8, 1.0, f32::INFINITY),
            Err(OptError::InvalidParameter { .. })
        ));
        // Out-of-range but finite values are clamped rather than refused.
        assert!(generate_normals(&indices, &positions, 8, 10.0, 100.0).is_ok());
        assert!(generate_normals(&indices, &positions, 8, -1.0, -1.0).is_ok());
    }

    #[test]
    fn mirrored_uvs_give_opposite_handedness() {
        // Two quads side by side in the XY plane, the right one's U mirrored.
        let positions = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, // left
            1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 2.0, 1.0, 0.0, 1.0, 1.0, 0.0, // right
        ];
        let normals = [0.0, 0.0, 1.0].repeat(8);
        let uvs = vec![
            0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, // left
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, // right, mirrored
        ];
        let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        let tangents =
            generate_tangents(&indices, &positions, &normals, &uvs, 8).expect("generates");
        let w: Vec<f32> = tangents.as_chunks::<4>().0.iter().map(|t| t[3]).collect();
        assert!(
            w[..6].iter().all(|&s| s == w[0]),
            "the left quad agrees: {w:?}"
        );
        assert!(
            w[6..].iter().all(|&s| s == -w[0]),
            "the right quad is mirrored: {w:?}"
        );
    }
}
