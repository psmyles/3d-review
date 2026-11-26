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
//! Everything is `f64`, with a fixed number of passes and iterations and only
//! `sqrt`, so the result is the same on every machine.

use glam::{DVec2, Vec2, Vec3};

/// Local/global passes.
const PASSES: usize = 12;
/// Conjugate-gradient iterations per solve, at most.
const ITERATIONS: usize = 400;

/// Refine `uvs` (one per chart vertex) for the chart's `triangles` (indices into
/// `points` and `uvs`, which are parallel). A chart too small or too degenerate
/// to refine is left as it is.
pub(super) fn refine(points: &[Vec3], triangles: &[[u32; 3]], uvs: &mut [Vec2]) {
    let n = uvs.len();
    if n < 3 || triangles.is_empty() {
        return;
    }
    // Each triangle in its own plane: area, and the gradients of its three
    // barycentric functions (what turns its UVs into its 2x2 map).
    struct Element {
        corners: [usize; 3],
        area: f64,
        gradients: [DVec2; 3],
    }
    let mut elements = Vec::with_capacity(triangles.len());
    for corners in triangles {
        let [a, b, c] = corners.map(|corner| points[corner as usize].as_dvec3());
        let (e1, e2) = (b - a, c - a);
        let normal = e1.cross(e2);
        let (Some(x), Some(n_hat)) = (e1.try_normalize(), normal.try_normalize()) else {
            continue;
        };
        let y = n_hat.cross(x);
        let (p00, p01, p10, p11) = (e1.dot(x), e2.dot(x), e1.dot(y), e2.dot(y));
        let det = p00 * p11 - p01 * p10;
        if det.abs() <= 1.0e-30 {
            continue;
        }
        // Rows of P^-1, with P's columns the triangle's two edges.
        let m0 = DVec2::new(p11, -p01) / det;
        let m1 = DVec2::new(-p10, p00) / det;
        elements.push(Element {
            corners: corners.map(|corner| corner as usize),
            area: 0.5 * normal.length(),
            gradients: [-(m0 + m1), m0, m1],
        });
    }
    if elements.is_empty() {
        return;
    }

    // The system's matrix, as per-row (column, value) lists; vertex 0 is held
    // where it is, which fixes the translation the energy cannot see.
    let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut add = |i: usize, j: usize, value: f64| match rows[i]
        .iter_mut()
        .find(|(column, _)| *column == j)
    {
        Some((_, existing)) => *existing += value,
        None => rows[i].push((j, value)),
    };
    for element in &elements {
        for i in 0..3 {
            for j in 0..3 {
                let value = element.area * element.gradients[i].dot(element.gradients[j]);
                add(element.corners[i], element.corners[j], value);
            }
        }
    }
    for row in &mut rows {
        row.sort_unstable_by_key(|(column, _)| *column);
    }
    let diagonal: Vec<f64> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            row.iter()
                .find(|(column, _)| *column == i)
                .map_or(1.0, |(_, value)| value.max(1.0e-30))
        })
        .collect();

    let mut current: Vec<DVec2> = uvs.iter().map(|uv| uv.as_dvec2()).collect();
    let pinned = current[0];
    for _ in 0..PASSES {
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
            let pin = pinned[axis];
            solve(&rows, &diagonal, &rhs, &mut x, pin);
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

/// Jacobi-preconditioned conjugate gradients on `rows x = rhs`, with `x[0]`
/// held at `pin`. Starts from `x` as given.
fn solve(rows: &[Vec<(usize, f64)>], diagonal: &[f64], rhs: &[f64], x: &mut [f64], pin: f64) {
    let n = x.len();
    x[0] = pin;
    let multiply = |v: &[f64], out: &mut [f64]| {
        for (i, row) in rows.iter().enumerate() {
            out[i] = if i == 0 {
                0.0
            } else {
                row.iter()
                    .filter(|(column, _)| *column != 0)
                    .map(|&(column, value)| value * v[column])
                    .sum()
            };
        }
    };
    // The pinned vertex's column moves to the right-hand side.
    let mut b = vec![0.0f64; n];
    for (i, row) in rows.iter().enumerate().skip(1) {
        let pinned: f64 = row
            .iter()
            .filter(|(column, _)| *column == 0)
            .map(|&(_, value)| value * pin)
            .sum();
        b[i] = rhs[i] - pinned;
    }
    let mut ax = vec![0.0f64; n];
    multiply(x, &mut ax);
    let mut r: Vec<f64> = (0..n)
        .map(|i| if i == 0 { 0.0 } else { b[i] - ax[i] })
        .collect();
    let mut z: Vec<f64> = (0..n).map(|i| r[i] / diagonal[i]).collect();
    let mut p = z.clone();
    let mut rz: f64 = (0..n).map(|i| r[i] * z[i]).sum();
    let scale: f64 = b
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .max(1.0e-60);
    let mut ap = vec![0.0f64; n];
    for _ in 0..ITERATIONS {
        if rz.abs() <= 1.0e-24 * scale {
            break;
        }
        multiply(&p, &mut ap);
        let pap: f64 = (0..n).map(|i| p[i] * ap[i]).sum();
        if pap <= 0.0 {
            break;
        }
        let alpha = rz / pap;
        for i in 1..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        for i in 1..n {
            z[i] = r[i] / diagonal[i];
        }
        let next: f64 = (0..n).map(|i| r[i] * z[i]).sum();
        let beta = next / rz;
        rz = next;
        for i in 1..n {
            p[i] = z[i] + beta * p[i];
        }
        p[0] = 0.0;
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
        refine(&points, &triangles, &mut uvs);
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
