//! How far a point is from the surface a vertex used to sit on.
//!
//! Garland and Heckbert's error quadric, the standard measure a mesh
//! simplifier chooses collapses by. For one plane it is the squared distance to
//! that plane, written as a symmetric 4x4 so that the *sum* over several planes
//! is again one matrix — which is the whole trick. A vertex accumulates the
//! quadric of every face around it, and from then on "how badly would putting
//! the vertex here distort the surface" is one small matrix product, however
//! many faces the surface had there.
//!
//! Here it answers a narrower question than in a simplifier: not which edge to
//! collapse next — [`super::partition`] has already decided what merges with
//! what — but **where the one vertex a region collapses to should end up**. A
//! region's quadrics summed give a matrix whose minimum is the point that best
//! fits every face the region covered, which on a fillet or a rim is markedly
//! better than any of the vertices that were there.
//!
//! ## Open borders get a plane of their own
//!
//! A border edge has surface on one side only, so nothing in the sum resists a
//! vertex sliding *along* the border or off the end of it. Garland and Heckbert's
//! answer, and ours: add a plane through the border edge perpendicular to its
//! face, weighted by the edge's length. The border then holds its shape for the
//! same reason the surface does.
//!
//! ## Storage
//!
//! Ten `f32`s — the upper triangle of a symmetric 4x4. `f32` because there is
//! one per vertex and a ten-million-triangle object has five million of them:
//! at `f64` that is four hundred megabytes for a term that only ever chooses
//! between candidate positions. Accumulation is in `f64` and so is the solve;
//! only the resting value is narrowed.

/// The squared-distance-to-a-set-of-planes form for one vertex.
///
/// Laid out as the upper triangle of the symmetric matrix, row by row:
/// `aa ab ac ad  bb bc bd  cc cd  dd`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Quadric([f32; 10]);

impl Quadric {
    /// The quadric of the plane with unit normal `normal` through `point`,
    /// scaled by `weight` — which is the face's area, so a big face counts for
    /// the surface it actually covers.
    pub(crate) fn from_plane(normal: [f64; 3], point: [f64; 3], weight: f64) -> Quadric {
        let [a, b, c] = normal;
        let d = -(a * point[0] + b * point[1] + c * point[2]);
        let w = weight;
        Quadric([
            (a * a * w) as f32,
            (a * b * w) as f32,
            (a * c * w) as f32,
            (a * d * w) as f32,
            (b * b * w) as f32,
            (b * c * w) as f32,
            (b * d * w) as f32,
            (c * c * w) as f32,
            (c * d * w) as f32,
            (d * d * w) as f32,
        ])
    }

    /// Add another vertex's quadric into this one.
    pub(crate) fn add(&mut self, other: &Quadric) {
        for (slot, value) in self.0.iter_mut().zip(other.0) {
            *slot += value;
        }
    }

    /// The error of putting a vertex at `point`: the weighted sum of squared
    /// distances to every plane in the sum. Never negative but for rounding,
    /// which is why it is clamped.
    pub(crate) fn error_at(&self, point: [f64; 3]) -> f64 {
        let q: [f64; 10] = std::array::from_fn(|slot| self.0[slot] as f64);
        let [x, y, z] = point;
        let error = q[0] * x * x
            + 2.0 * q[1] * x * y
            + 2.0 * q[2] * x * z
            + 2.0 * q[3] * x
            + q[4] * y * y
            + 2.0 * q[5] * y * z
            + 2.0 * q[6] * y
            + q[7] * z * z
            + 2.0 * q[8] * z
            + q[9];
        error.max(0.0)
    }

    /// The point of least error, or `None` when the planes do not pin one down.
    ///
    /// That happens whenever the surface here is flat or a crease rather than a
    /// corner — a plane leaves a whole plane of equally good answers, and an
    /// edge leaves a line — and it is the *common* case, not an error. The
    /// caller falls back to a vertex that was actually there.
    pub(crate) fn optimal_point(&self) -> Option<[f64; 3]> {
        let q: [f64; 10] = std::array::from_fn(|slot| self.0[slot] as f64);
        // The gradient is zero where `A x = -b`, with A the upper-left 3x3 and
        // b the fourth column.
        let a = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
        let b = [-q[3], -q[6], -q[8]];

        let determinant = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
            - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);

        // Scaled against the matrix's own magnitude rather than an absolute
        // epsilon: the quadrics of a metre-sized object and a centimetre-sized
        // one differ by eight orders of magnitude, and a fixed threshold would
        // mean "always solve" for one and "never solve" for the other.
        let scale = (q[0] + q[4] + q[7]).abs();
        if scale <= 0.0 || determinant.abs() <= 1.0e-9 * scale * scale * scale {
            return None;
        }

        let solve = |column: usize| -> f64 {
            let mut m = a;
            for row in 0..3 {
                m[row][column] = b[row];
            }
            m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
        };
        let point = [
            solve(0) / determinant,
            solve(1) / determinant,
            solve(2) / determinant,
        ];
        point.iter().all(|value| value.is_finite()).then_some(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quadric built from a plane reports zero on that plane and the squared
    /// distance off it.
    #[test]
    fn one_plane_measures_the_distance_to_itself() {
        // The plane z = 0, weight one.
        let q = Quadric::from_plane([0.0, 0.0, 1.0], [0.0, 0.0, 0.0], 1.0);

        assert!(q.error_at([5.0, -3.0, 0.0]).abs() < 1.0e-9, "on the plane");
        assert!(
            (q.error_at([0.0, 0.0, 2.0]) - 4.0).abs() < 1.0e-9,
            "two units off it is four"
        );
    }

    #[test]
    fn a_weight_scales_the_error() {
        let light = Quadric::from_plane([0.0, 1.0, 0.0], [0.0, 0.0, 0.0], 1.0);
        let heavy = Quadric::from_plane([0.0, 1.0, 0.0], [0.0, 0.0, 0.0], 3.0);

        let point = [0.0, 2.0, 0.0];
        assert!((heavy.error_at(point) - 3.0 * light.error_at(point)).abs() < 1.0e-9);
    }

    /// Three planes at right angles pin a corner down exactly.
    #[test]
    fn three_planes_meeting_at_a_corner_solve_to_it() {
        let corner = [1.0, 2.0, 3.0];
        let mut q = Quadric::default();
        q.add(&Quadric::from_plane([1.0, 0.0, 0.0], corner, 1.0));
        q.add(&Quadric::from_plane([0.0, 1.0, 0.0], corner, 1.0));
        q.add(&Quadric::from_plane([0.0, 0.0, 1.0], corner, 1.0));

        let solved = q.optimal_point().expect("a corner is well determined");

        for axis in 0..3 {
            assert!(
                (solved[axis] - corner[axis]).abs() < 1.0e-6,
                "axis {axis}: {solved:?} against {corner:?}"
            );
        }
    }

    /// A flat patch leaves a whole plane of equally good answers, so there is
    /// nothing to solve — and that is the ordinary case, not a failure.
    #[test]
    fn a_flat_surface_has_no_single_best_point() {
        let mut q = Quadric::default();
        for offset in 0..4 {
            q.add(&Quadric::from_plane(
                [0.0, 0.0, 1.0],
                [offset as f64, 0.0, 0.0],
                1.0,
            ));
        }

        assert!(q.optimal_point().is_none());
    }

    #[test]
    fn an_empty_quadric_solves_to_nothing() {
        assert!(Quadric::default().optimal_point().is_none());
        assert_eq!(Quadric::default().error_at([1.0, 2.0, 3.0]), 0.0);
    }

    /// The conditioning guard is relative, so it answers the same way for the
    /// same shape at any size.
    #[test]
    fn the_solve_is_scale_independent() {
        let solve_at = |scale: f64| {
            let corner = [scale, 2.0 * scale, 3.0 * scale];
            let mut q = Quadric::default();
            q.add(&Quadric::from_plane([1.0, 0.0, 0.0], corner, scale * scale));
            q.add(&Quadric::from_plane([0.0, 1.0, 0.0], corner, scale * scale));
            q.add(&Quadric::from_plane([0.0, 0.0, 1.0], corner, scale * scale));
            q.optimal_point()
                .map(|point| [point[0] / scale, point[1] / scale, point[2] / scale])
        };

        let small = solve_at(0.01).expect("a corner solves at centimetre scale");
        let large = solve_at(100.0).expect("and at hundred-metre scale");
        for axis in 0..3 {
            assert!(
                (small[axis] - large[axis]).abs() < 1.0e-4,
                "axis {axis}: {small:?} against {large:?}"
            );
        }
    }

    #[test]
    fn adding_is_the_sum_of_the_errors() {
        let first = Quadric::from_plane([1.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0);
        let second = Quadric::from_plane([0.0, 1.0, 0.0], [0.0, 0.0, 0.0], 1.0);
        let point = [3.0, 4.0, 5.0];

        let mut both = first;
        both.add(&second);

        assert!(
            (both.error_at(point) - (first.error_at(point) + second.error_at(point))).abs()
                < 1.0e-9
        );
    }
}
