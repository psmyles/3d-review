//! Density: how many texture pixels each object gets per meter, and whether
//! its triangles are too small for the size it will be on screen.
//!
//! The triangle-density figure is an estimate. An object seen at screen size
//! `S` — its bounding-sphere diameter `D` over the screen height `H` pixels —
//! is drawn at `k = S·H/D` pixels per meter. A convex surface of area `A`
//! projects on average to `A/4` (Cauchy), and about half its `T` triangles
//! face the camera, so the average visible triangle covers `A·k²/(2T)` pixels.
//! Setting that equal to the profile's minimum `a` gives the screen size below
//! which the object should switch to a LOD:
//!
//! ```text
//! S = (D / H) · √(2·T·a / A)
//! ```
//!
//! When `S > 1` the triangles are too small even with the object filling the
//! screen, and a LOD cannot help — the mesh itself wants reducing.

use review_model::measure::SurfaceMeasure;

use super::{Outcome, count_param, largest};
use crate::context::Context;
use crate::finding::{Measured, Offender, Threshold};
use crate::profile::RuleConfig;

pub(super) fn texel(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let texture_size = count_param(config, "texture_size", 1024) as f32;
    let target = config.number("target").unwrap_or(1024.0);
    let tolerance = config.number("tolerance").unwrap_or(2.0).max(1.0);
    let (low, high) = (target / tolerance, target * tolerance);
    let measure = SurfaceMeasure::new(ctx.model, 0);
    let offenders: Vec<Offender> = ctx
        .node_triangles()
        .iter()
        .enumerate()
        .filter(|(_, triangles)| !triangles.is_empty())
        .filter_map(|(node, triangles)| {
            let density = f64::from(
                measure.texel_density(triangles.iter().map(|&t| t as usize), texture_size)?,
            );
            // An object with no UV area has nothing to judge; `uv.missing`
            // reports it.
            (density > 0.0 && (density < low || density > high))
                .then(|| Offender::node(node, Some(Measured::PxPerMeter(density))))
        })
        .collect();
    let measured = largest(&offenders, |m| match m {
        Measured::PxPerMeter(d) => Some((d / target).ln().abs()),
        _ => None,
    });
    Outcome::judged(
        offenders,
        Threshold::Range {
            min: Measured::PxPerMeter(low),
            max: Measured::PxPerMeter(high),
        },
    )
    .with_measured(measured)
}

/// One object's triangle-density figures.
pub(crate) struct TriangleDensity {
    /// Triangles per square meter of surface.
    pub per_square_meter: f64,
    /// The screen size below which the object should switch to a LOD.
    pub lod_screen_size: f64,
}

/// The triangle-density figures of the triangles `triangles` spanning a
/// sphere of diameter `diameter`, or `None` for a surface with no area.
pub(crate) fn triangle_density(
    measure: &SurfaceMeasure,
    triangles: &[u32],
    diameter: f64,
    min_pixel_area: f64,
    screen_height: f64,
) -> Option<TriangleDensity> {
    let area: f64 = triangles
        .iter()
        .map(|&t| f64::from(measure.world_area.get(t as usize).copied().unwrap_or(0.0)))
        .sum();
    if area <= 0.0 || diameter <= 0.0 || screen_height <= 0.0 {
        return None;
    }
    let count = triangles.len() as f64;
    Some(TriangleDensity {
        per_square_meter: count / area,
        lod_screen_size: (diameter / screen_height) * (2.0 * count * min_pixel_area / area).sqrt(),
    })
}

/// `reduce = false`: objects that should switch to a LOD below some screen
/// size. `reduce = true`: objects too dense even at full screen. Both read the
/// LOD rule's parameters (`config` is that rule's).
pub(super) fn triangle_lod(ctx: &Context<'_>, config: &RuleConfig, reduce: bool) -> Outcome {
    let min_pixel_area = config.number("min_pixel_area").unwrap_or(10.0);
    let screen_height = config.number("screen_height").unwrap_or(1080.0);
    let min_screen_size = config.number("min_screen_size").unwrap_or(0.1);
    let measure = SurfaceMeasure::new(ctx.model, 0);
    let bounds = ctx.node_bounds();
    let offenders: Vec<Offender> = ctx
        .node_triangles()
        .iter()
        .enumerate()
        .filter(|(_, triangles)| !triangles.is_empty())
        .filter_map(|(node, triangles)| {
            let diameter = bounds
                .get(node)
                .filter(|bounds| !bounds.is_empty())
                .map_or(0.0, |bounds| f64::from(bounds.radius()) * 2.0);
            let density =
                triangle_density(&measure, triangles, diameter, min_pixel_area, screen_height)?;
            let size = density.lod_screen_size;
            let offends = if reduce {
                size > 1.0
            } else {
                (min_screen_size..=1.0).contains(&size)
            };
            offends.then(|| {
                let mut offender = Offender::node(node, Some(Measured::ScreenSize(size)));
                offender.detail = Some(Measured::PerSquareMeter(density.per_square_meter));
                offender
            })
        })
        .collect();
    let measured = largest(&offenders, |m| match m {
        Measured::ScreenSize(s) => Some(*s),
        _ => None,
    });
    let threshold = if reduce {
        Threshold::Max(Measured::ScreenSize(1.0))
    } else {
        Threshold::Min(Measured::SquarePixels(min_pixel_area))
    };
    Outcome::judged(offenders, threshold).with_measured(measured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_size_follows_the_formula() {
        // A 1 m² surface of 1000 triangles, 2 m across, at 1080 px with a
        // 10 px² minimum: S = (2/1080)·√(2·1000·10/1) = 0.2619...
        let measure = SurfaceMeasure {
            world_area: vec![0.001; 1000],
            uv_area: vec![0.0; 1000],
        };
        let triangles: Vec<u32> = (0..1000).collect();
        let density = triangle_density(&measure, &triangles, 2.0, 10.0, 1080.0).unwrap();
        assert!((density.per_square_meter - 1000.0).abs() < 1e-2);
        let expected = (2.0 / 1080.0) * (2.0_f64 * 1000.0 * 10.0).sqrt();
        assert!((density.lod_screen_size - expected).abs() < 1e-5);
    }
}
