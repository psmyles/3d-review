//! The `f64` vector arithmetic the rebuild's stages share.
//!
//! One copy of each expression, so every stage computes a face's cross product,
//! normal and area the same way. That matters beyond tidiness: the stages compare
//! their answers against one another (a fold is two stages' normals disagreeing),
//! and a triangle computed with its terms in a different order is a different
//! number in the last bit.
//!
//! Plain arrays rather than `glam`'s `DVec3` because every caller already holds
//! `[f64; 3]` and the conversions would be the only thing added.

pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn scale(a: [f64; 3], by: f64) -> [f64; 3] {
    [a[0] * by, a[1] * by, a[2] * by]
}

pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn length(vector: [f64; 3]) -> f64 {
    dot(vector, vector).sqrt()
}

/// `vector` divided by its length, or `None` when it has none.
pub(crate) fn normalized(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = length(vector);
    (length > 0.0).then(|| [vector[0] / length, vector[1] / length, vector[2] / length])
}

/// Twice the area of the triangle `a b c`, as a vector along its normal.
pub(crate) fn triangle_cross(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    cross(sub(b, a), sub(c, a))
}

/// The area of the triangle `a b c`.
pub(crate) fn triangle_area(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    length(triangle_cross(a, b, c)) * 0.5
}
