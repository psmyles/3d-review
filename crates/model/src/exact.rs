//! Transcendental functions that give the same bits on every OS.
//!
//! `f32::sin` and its kin call the platform's maths library: glibc on Linux, the
//! CRT on Windows, Apple's libm on macOS. Those are free to round the last bit
//! differently, and a ray direction one ulp apart is an occlusion hit on one
//! machine and a miss on another: a different byte in the written file
//! (`docs/ARCHITECTURE.md`, D27). These are the `libm` crate's pure-Rust ports
//! of musl's, the same instructions everywhere, and Rust never fuses their
//! multiply-adds.
//!
//! Use them for anything whose value reaches an output — a written file or a
//! report. Display-only maths (the viewer, a test's fixture) can keep `std`'s.
//! `sqrt` is not here: IEEE 754 makes it exact everywhere already.

/// `x.sin()`, the same on every OS.
#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

/// `x.cos()`, the same on every OS.
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

/// `x.acos()`, the same on every OS.
#[inline]
pub fn acos(x: f32) -> f32 {
    libm::acosf(x)
}

/// `y.atan2(x)`, the same on every OS.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

/// `x.powf(y)`, the same on every OS.
#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// `x.ln()`, the same on every OS.
#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

/// `x.exp()`, the same on every OS.
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

/// `x.cbrt()`, the same on every OS.
#[inline]
pub fn cbrt(x: f32) -> f32 {
    libm::cbrtf(x)
}

/// `x.acos()` in `f64`, the same on every OS.
#[inline]
pub fn acos_f64(x: f64) -> f64 {
    libm::acos(x)
}

/// `x.ln()` in `f64`, the same on every OS.
#[inline]
pub fn ln_f64(x: f64) -> f64 {
    libm::log(x)
}

/// `x.exp()` in `f64`, the same on every OS.
#[inline]
pub fn exp_f64(x: f64) -> f64 {
    libm::exp(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// They are the functions they stand in for: the platform's own agrees to
    /// within a few ulps, which is all the platforms promise each other.
    #[test]
    fn they_agree_with_the_platform_to_a_few_ulps() {
        let ulps = |a: f32, b: f32| (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs();
        for step in 0..1000 {
            let x = step as f32 * 0.00731 - 3.0;
            assert!(ulps(sin(x), x.sin()) <= 2, "sin {x}");
            assert!(ulps(cos(x), x.cos()) <= 2, "cos {x}");
            assert!(ulps(atan2(x, 0.7), x.atan2(0.7)) <= 2, "atan2 {x}");
            assert!(ulps(cbrt(x), x.cbrt()) <= 2, "cbrt {x}");
            assert!(ulps(exp(x), x.exp()) <= 2, "exp {x}");
            let unit = (x / 3.0).clamp(-1.0, 1.0);
            assert!(ulps(acos(unit), unit.acos()) <= 2, "acos {unit}");
            let positive = x + 3.5;
            assert!(ulps(ln(positive), positive.ln()) <= 2, "ln {positive}");
            assert!(
                ulps(powf(positive, 2.4), positive.powf(2.4)) <= 2,
                "powf {positive}"
            );
            let wide_ulps =
                |a: f64, b: f64| (a.to_bits() as i128 - b.to_bits() as i128).unsigned_abs();
            let (wide, wide_unit) = (f64::from(positive), f64::from(unit));
            assert!(
                wide_ulps(acos_f64(wide_unit), wide_unit.acos()) <= 2,
                "acos {unit}"
            );
            assert!(wide_ulps(ln_f64(wide), wide.ln()) <= 2, "ln {wide}");
            assert!(wide_ulps(exp_f64(wide), wide.exp()) <= 2, "exp {wide}");
        }
    }
}
