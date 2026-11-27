//! The sparse symmetric systems both flattenings solve, and the solve:
//! Jacobi-preconditioned conjugate gradients with some unknowns held fixed.
//!
//! Everything is `f64` and summed in a fixed order, so the result is the same
//! on every machine.

/// A symmetric matrix as per-row `(column, value)` lists, sorted by column once
/// [`Sparse::finish`] has run.
pub(super) struct Sparse {
    rows: Vec<Vec<(usize, f64)>>,
}

impl Sparse {
    pub(super) fn new(size: usize) -> Self {
        Self {
            rows: vec![Vec::new(); size],
        }
    }

    /// Add `value` at `(row, column)`.
    pub(super) fn add(&mut self, row: usize, column: usize, value: f64) {
        match self.rows[row].iter_mut().find(|(at, _)| *at == column) {
            Some((_, existing)) => *existing += value,
            None => self.rows[row].push((column, value)),
        }
    }

    /// Sort every row, so a product sums in column order.
    pub(super) fn finish(&mut self) {
        for row in &mut self.rows {
            row.sort_unstable_by_key(|(column, _)| *column);
        }
    }
}

/// Solve `matrix x = rhs` for the unknowns `fixed` leaves free, the fixed ones
/// held at the value `x` gives them; `x` is also the starting point. At most
/// `iterations` steps; stops early once the residual is negligible.
pub(super) fn conjugate_gradients(
    matrix: &Sparse,
    rhs: &[f64],
    x: &mut [f64],
    fixed: &[bool],
    iterations: usize,
) {
    let n = x.len();
    let free = |i: usize| !fixed[i];
    let diagonal: Vec<f64> = matrix
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            row.iter()
                .find(|(column, _)| *column == i)
                .map_or(1.0, |(_, value)| value.max(1.0e-30))
        })
        .collect();
    // The fixed unknowns' columns move to the right-hand side.
    let mut b = vec![0.0f64; n];
    for (i, row) in matrix.rows.iter().enumerate() {
        if free(i) {
            let known: f64 = row
                .iter()
                .filter(|&&(column, _)| fixed[column])
                .map(|&(column, value)| value * x[column])
                .sum();
            b[i] = rhs[i] - known;
        }
    }
    let multiply = |v: &[f64], out: &mut [f64]| {
        for (i, row) in matrix.rows.iter().enumerate() {
            out[i] = if free(i) {
                row.iter()
                    .filter(|&&(column, _)| free(column))
                    .map(|&(column, value)| value * v[column])
                    .sum()
            } else {
                0.0
            };
        }
    };
    let mut ax = vec![0.0f64; n];
    multiply(x, &mut ax);
    let mut r: Vec<f64> = (0..n)
        .map(|i| if free(i) { b[i] - ax[i] } else { 0.0 })
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
    for _ in 0..iterations {
        if rz.abs() <= 1.0e-24 * scale {
            break;
        }
        multiply(&p, &mut ap);
        let pap: f64 = (0..n).map(|i| p[i] * ap[i]).sum();
        if pap <= 0.0 {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            if free(i) {
                x[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
                z[i] = r[i] / diagonal[i];
            }
        }
        let next: f64 = (0..n).map(|i| r[i] * z[i]).sum();
        let beta = next / rz;
        rz = next;
        for i in 0..n {
            p[i] = if free(i) { z[i] + beta * p[i] } else { 0.0 };
        }
    }
}
