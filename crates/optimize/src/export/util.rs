//! Leaf helpers shared across the builders.

use std::ffi::CString;

use glam::Mat4;

/// A C string from `text`, falling back to `fallback` when the text is empty or
/// contains an interior NUL (which a name read from a file legitimately might).
pub(crate) fn c_string(text: &str, fallback: &str) -> CString {
    if text.is_empty() {
        return CString::new(fallback).unwrap_or_default();
    }
    CString::new(text.replace('\0', ""))
        .unwrap_or_else(|_| CString::new(fallback).unwrap_or_default())
}

/// A C string from `text`, empty when it is empty.
pub(crate) fn c_string_or_empty(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

/// A world matrix in meters, expressed in the file's unit: ufbx reads the
/// file's `TransformLink` / pose matrices and pre-multiplies its root scale,
/// so writing them back means scaling the whole 3×4 affine part by the units
/// per meter.
pub(crate) fn world_matrix_in_file_units(matrix: Mat4, per_meter: f32) -> [f64; 16] {
    let mut out = matrix.as_dmat4().to_cols_array();
    for column in 0..4 {
        for row in 0..3 {
            out[column * 4 + row] *= f64::from(per_meter);
        }
    }
    out
}
