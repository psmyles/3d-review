//! Least-squares conformal maps (Lévy, Petitjean, Ray and Maillot, 2002): the
//! flattening every chart starts from.
//!
//! A map is conformal where each triangle's Jacobian is a similarity, which in
//! two dimensions is two linear conditions on the gradients of U and V:
//! `∇U = rot(∇V)`. LSCM asks for that in the least-squares sense, weighted by
//! area, with two vertices pinned (a conformal map is free to move, turn and
//! scale): one sparse symmetric system, solved by conjugate gradients from a
//! projection onto the chart's own plane.
//!
//! Everything is `f64` with a fixed iteration cap, so the result is the same on
//! every machine.

use glam::{DVec2, DVec3, Vec2, Vec3};

use super::element::elements;
use super::solve::{Sparse, conjugate_gradients};

/// Conjugate-gradient iterations for a chart of `n` vertices, at most: enough
/// for the system's condition, which grows with the chart's size, from a start
/// that is already close wherever the chart is nearly flat.
fn iterations(n: usize) -> usize {
    ((20.0 * (n as f64).sqrt()) as usize).clamp(150, 2000)
}

/// Flatten a chart: one UV per vertex of `points` (indices in `triangles`),
/// each of `pins` (at least two vertices, in a fixed order) held at its UV.
/// `None` when the chart has nothing to flatten.
pub(super) fn flatten(
    points: &[Vec3],
    triangles: &[[u32; 3]],
    pins: &[(usize, Vec2)],
) -> Option<Vec<Vec2>> {
    let n = points.len();
    let elements = elements(points, triangles);
    if n < 3 || elements.is_empty() || pins.len() < 2 {
        return None;
    }

    // Unknowns: U of every vertex, then V of every vertex. Per triangle the
    // residuals are `Σ uₖ gₖ.x − vₖ gₖ.y` and `Σ uₖ gₖ.y + vₖ gₖ.x`; the
    // matrix is their weighted sum of squares.
    let mut matrix = Sparse::new(2 * n);
    for element in &elements {
        for (k, &row) in element.corners.iter().enumerate() {
            let gk = element.gradients[k];
            for (l, &column) in element.corners.iter().enumerate() {
                let gl = element.gradients[l];
                let same = element.area * gk.dot(gl);
                let cross = element.area * (gk.y * gl.x - gk.x * gl.y);
                matrix.add(row, column, same);
                matrix.add(n + row, n + column, same);
                matrix.add(row, n + column, cross);
                matrix.add(n + row, column, -cross);
            }
        }
    }
    matrix.finish();

    // Start from the projection onto the chart's plane, moved, turned and
    // scaled to best fit the pins: close to the answer wherever the chart is
    // nearly flat.
    let at = |vertex: usize| points[vertex].as_dvec3();
    let normal = elements
        .iter()
        .map(|element| {
            let [a, b, c] = element.corners.map(at);
            (b - a).cross(c - a)
        })
        .sum::<DVec3>()
        .try_normalize()
        .unwrap_or(DVec3::Z);
    let u_axis = (at(pins[1].0) - at(pins[0].0))
        .reject_from(normal)
        .try_normalize()
        .unwrap_or_else(|| normal.any_orthonormal_vector());
    let v_axis = normal.cross(u_axis);
    let projected: Vec<DVec2> = (0..n)
        .map(|vertex| DVec2::new(at(vertex).dot(u_axis), at(vertex).dot(v_axis)))
        .collect();
    let fit = similarity(
        pins.iter()
            .map(|&(vertex, uv)| (projected[vertex], uv.as_dvec2())),
    );
    let mut x = vec![0.0f64; 2 * n];
    for (vertex, &point) in projected.iter().enumerate() {
        let uv = fit(point);
        x[vertex] = uv.x;
        x[n + vertex] = uv.y;
    }
    let mut fixed = vec![false; 2 * n];
    for &(vertex, uv) in pins {
        x[vertex] = f64::from(uv.x);
        x[n + vertex] = f64::from(uv.y);
        fixed[vertex] = true;
        fixed[n + vertex] = true;
    }
    conjugate_gradients(&matrix, &vec![0.0; 2 * n], &mut x, &fixed, iterations(n));
    let uvs: Vec<DVec2> = (0..n)
        .map(|vertex| DVec2::new(x[vertex], x[n + vertex]))
        .collect();
    uvs.iter()
        .all(|uv| uv.is_finite())
        .then(|| uvs.iter().map(|uv| uv.as_vec2()).collect())
}

/// The similarity (a move, a turn and a scale, never a mirror) that best maps
/// each pair's first point onto its second, in the least-squares sense: as
/// complex numbers, `w ≈ a z + b`, solved in closed form.
fn similarity(pairs: impl Iterator<Item = (DVec2, DVec2)> + Clone) -> impl Fn(DVec2) -> DVec2 {
    let count = pairs.clone().count().max(1) as f64;
    let (from_sum, to_sum) = pairs
        .clone()
        .fold((DVec2::ZERO, DVec2::ZERO), |(from, to), (z, w)| {
            (from + z, to + w)
        });
    let (from_mean, to_mean) = (from_sum / count, to_sum / count);
    // a = Σ (w - w̄) conj(z - z̄) / Σ |z - z̄|².
    let (mut a, mut norm) = (DVec2::ZERO, 0.0);
    for (z, w) in pairs {
        let (z, w) = (z - from_mean, w - to_mean);
        a += DVec2::new(w.x * z.x + w.y * z.y, w.y * z.x - w.x * z.y);
        norm += z.length_squared();
    }
    let a = if norm > 0.0 { a / norm } else { DVec2::X };
    move |point: DVec2| {
        let z = point - from_mean;
        DVec2::new(a.x * z.x - a.y * z.y, a.x * z.y + a.y * z.x) + to_mean
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bent strip of a cylinder unrolls without stretch: LSCM is exact on a
    /// developable surface.
    #[test]
    fn a_developable_strip_unrolls_exactly() {
        let segments = 8;
        let mut points = Vec::new();
        for i in 0..=segments {
            let angle = i as f32 / segments as f32 * 1.5;
            for z in [0.0f32, 1.0] {
                points.push(Vec3::new(angle.cos(), angle.sin(), z));
            }
        }
        let mut triangles = Vec::new();
        for i in 0..segments as u32 {
            let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            triangles.push([a, c, b]);
            triangles.push([b, c, d]);
        }
        let pins = [(0, Vec2::ZERO), (2 * segments, Vec2::new(1.5, 0.0))];
        let uvs = flatten(&points, &triangles, &pins).expect("a chart");
        for corners in &triangles {
            let p = corners.map(|corner| points[corner as usize]);
            let q = corners.map(|corner| uvs[corner as usize]);
            let stretch = super::super::layout::stretch(p, q).expect("a real triangle");
            assert!(stretch < 1.01, "stretch {stretch}");
            let area3 = 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
            let area2 = 0.5 * (q[1] - q[0]).perp_dot(q[2] - q[0]);
            assert!(area2 > 0.0, "the winding is kept");
            // Pinned an arc's length apart, the strip unrolls at its own size.
            assert!((area2 / area3 - 1.0).abs() < 0.01, "{area2} vs {area3}");
        }
    }
}
