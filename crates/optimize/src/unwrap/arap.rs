//! As-rigid-as-possible refinement of a flattened chart (Liu, Zhang, Xu,
//! Gotsman and Gortler, "A Local/Global Approach to Mesh Parameterization",
//! 2008).
//!
//! LSCM keeps angles and lets area drift, which on a doubly curved island is a
//! texel density that wanders by a fifth across it. ARAP asks each triangle's
//! map to be a rotation — neither stretched nor scaled — and alternates two
//! exact steps from the LSCM start: per triangle, the rotation nearest its
//! current map (closed form in two dimensions); then, all at once, the UVs
//! that best match those rotations (a sparse linear system, the cotangent
//! Laplacian, solved by conjugate gradients). What comes out balances angle
//! against area much as a minimum-stretch unwrap does.
//!
//! Everything is `f64`, with a fixed number of passes, iterations bounded by
//! the chart's size and only `sqrt`, so the result is the same on every
//! machine.

use glam::{DVec2, Vec2, Vec3};

use super::element::elements;
use super::solve::{Sparse, conjugate_gradients};

/// Local/global passes a chart is refined with, in all.
pub(super) const PASSES: usize = 12;
/// Conjugate-gradient iterations per solve for a chart of `n` vertices, at
/// most: each solve starts from the last pass's answer, so few are needed.
fn iterations(n: usize) -> usize {
    ((8.0 * (n as f64).sqrt()) as usize).clamp(50, 400)
}

/// Refine `uvs` (one per chart vertex) for the chart's `triangles` (indices into
/// `points` and `uvs`, which are parallel) over `passes` local/global passes,
/// the vertices `held` marks held where they are (or, with none given, vertex
/// 0, which fixes the translation the energy cannot see). A chart too small or
/// too degenerate to refine is left as it is.
pub(super) fn refine(
    points: &[Vec3],
    triangles: &[[u32; 3]],
    uvs: &mut [Vec2],
    passes: usize,
    held: Option<&[bool]>,
) {
    let n = uvs.len();
    if n < 3 || triangles.is_empty() {
        return;
    }
    let elements = elements(points, triangles);
    if elements.is_empty() {
        return;
    }

    let mut matrix = Sparse::new(n);
    for element in &elements {
        for i in 0..3 {
            for j in 0..3 {
                let value = element.area * element.gradients[i].dot(element.gradients[j]);
                matrix.add(element.corners[i], element.corners[j], value);
            }
        }
    }
    matrix.finish();
    let fixed: Vec<bool> = match held {
        Some(held) if held.iter().any(|&held| held) => held.to_vec(),
        _ => (0..n).map(|vertex| vertex == 0).collect(),
    };

    let mut current: Vec<DVec2> = uvs.iter().map(|uv| uv.as_dvec2()).collect();
    for _ in 0..passes {
        // Local: each triangle's nearest rotation.
        let rotations: Vec<[DVec2; 2]> = elements
            .iter()
            .map(|element| {
                let mut row_u = DVec2::ZERO;
                let mut row_v = DVec2::ZERO;
                for k in 0..3 {
                    let uv = current[element.corners[k]];
                    row_u += uv.x * element.gradients[k];
                    row_v += uv.y * element.gradients[k];
                }
                // J = [[a, b], [c, d]]: the nearest rotation's (cos, sin) is
                // along (a + d, c - b).
                let (a, b, c, d) = (row_u.x, row_u.y, row_v.x, row_v.y);
                let direction = DVec2::new(a + d, c - b).try_normalize().unwrap_or(DVec2::X);
                let (cos, sin) = (direction.x, direction.y);
                [DVec2::new(cos, -sin), DVec2::new(sin, cos)]
            })
            .collect();

        // Global: the UVs that best match them, one coordinate at a time.
        for axis in 0..2 {
            let mut rhs = vec![0.0f64; n];
            for (element, rotation) in elements.iter().zip(&rotations) {
                for k in 0..3 {
                    rhs[element.corners[k]] +=
                        element.area * element.gradients[k].dot(rotation[axis]);
                }
            }
            let mut x: Vec<f64> = current.iter().map(|uv| uv[axis]).collect();
            conjugate_gradients(&matrix, &rhs, &mut x, &fixed, iterations(n));
            for (uv, value) in current.iter_mut().zip(&x) {
                uv[axis] = *value;
            }
        }
    }
    if current.iter().all(|uv| uv.is_finite()) {
        for (uv, refined) in uvs.iter_mut().zip(&current) {
            *uv = refined.as_vec2();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip of a cylinder, flattened badly (squashed across), comes back
    /// isometric: every triangle the same size as in 3D.
    #[test]
    fn a_developable_strip_unrolls_without_stretch() {
        let segments = 8;
        let mut points = Vec::new();
        let mut uvs = Vec::new();
        for i in 0..=segments {
            let angle = i as f32 / segments as f32 * 1.5;
            for z in [0.0f32, 1.0] {
                points.push(Vec3::new(angle.cos(), angle.sin(), z));
                // A projection onto the XZ plane: squashed where the strip turns.
                uvs.push(Vec2::new(angle.cos(), z));
            }
        }
        let mut triangles = Vec::new();
        for i in 0..segments as u32 {
            let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
        refine(&points, &triangles, &mut uvs, PASSES, None);
        for corners in &triangles {
            let p = corners.map(|corner| points[corner as usize]);
            let q = corners.map(|corner| uvs[corner as usize]);
            let stretch = super::super::layout::stretch(p, q).expect("a real triangle");
            assert!(stretch < 1.01, "stretch {stretch}");
            let area3 = 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
            let area2 = 0.5 * (q[1] - q[0]).perp_dot(q[2] - q[0]).abs();
            assert!((area2 / area3 - 1.0).abs() < 0.01, "{area2} vs {area3}");
        }
    }
}
