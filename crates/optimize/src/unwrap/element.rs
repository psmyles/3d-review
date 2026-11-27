//! A triangle in its own plane, as both flattenings see it.

use glam::{DVec2, Vec3};

/// One triangle of a chart: its corners (indices into the chart's vertices),
/// its area, and the gradients of its three barycentric functions in its own
/// plane — what turns the UVs at its corners into its 2x2 map. The plane's
/// axes are the first edge and the normal crossed with it, so a triangle
/// wound counter-clockwise about its normal stays counter-clockwise.
pub(super) struct Element {
    pub corners: [usize; 3],
    pub area: f64,
    pub gradients: [DVec2; 3],
}

/// The chart's triangles that have an area; the rest are left out.
pub(super) fn elements(points: &[Vec3], triangles: &[[u32; 3]]) -> Vec<Element> {
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
    elements
}
