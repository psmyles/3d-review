//! Measuring how far one subject's shading normals are from another's, in the
//! pixels both of them cover.
//!
//! Both images come from the `normals` look — the interpolated vertex normal
//! written as `n * 0.5 + 0.5` with no lighting, tone map or occlusion in the way —
//! rendered from the same camera, so a pixel compares the normal each mesh would
//! shade that point of the screen with. That is the quantity a viewer reads as
//! "the shading looks different", which no vertex-to-vertex distance measures.

use glam::Vec3;
use image::{Rgba, RgbaImage};

/// Angles at or past this are drawn at the top of the heat ramp.
const HEAT_RANGE_DEGREES: f32 = 30.0;

/// A decoded normal is a surface pixel when its length is near one. The normals
/// look is rendered over black, which decodes to `(-1, -1, -1)` — length 1.73 —
/// and an antialiased edge blends towards it, so both fall outside this band.
const UNIT_BAND: std::ops::RangeInclusive<f32> = 0.9..=1.1;

/// What one comparison measured, over every view it was given.
#[derive(Default)]
pub struct NormalError {
    angles: Vec<f32>,
    /// Pixels exactly one of the two images covers.
    silhouette_only_one: usize,
    /// Pixels either image covers.
    silhouette_either: usize,
}

impl NormalError {
    /// Fold one view's pair of `normals` images in, returning its heat map.
    pub fn add_view(&mut self, reference: &RgbaImage, candidate: &RgbaImage) -> RgbaImage {
        let mut heat = RgbaImage::new(reference.width(), reference.height());
        for ((a, b), out) in reference
            .pixels()
            .zip(candidate.pixels())
            .zip(heat.pixels_mut())
        {
            let (a, b) = (decode(*a), decode(*b));
            *out = match (a, b) {
                (Some(a), Some(b)) => {
                    let angle = a
                        .normalize()
                        .dot(b.normalize())
                        .clamp(-1.0, 1.0)
                        .acos()
                        .to_degrees();
                    self.angles.push(angle);
                    self.silhouette_either += 1;
                    ramp(angle / HEAT_RANGE_DEGREES)
                }
                (Some(_), None) | (None, Some(_)) => {
                    self.silhouette_only_one += 1;
                    self.silhouette_either += 1;
                    Rgba([200, 0, 200, 255])
                }
                (None, None) => Rgba([24, 24, 24, 255]),
            };
        }
        heat
    }

    /// One line: mean / median / p90 angle, the share past 10 and 20 degrees,
    /// and the silhouette mismatch.
    pub fn summary(&self) -> String {
        if self.angles.is_empty() {
            return "no pixel covered by both".into();
        }
        let mut sorted = self.angles.clone();
        sorted.sort_by(f32::total_cmp);
        let at = |q: f32| sorted[((sorted.len() - 1) as f32 * q) as usize];
        let mean = sorted.iter().sum::<f32>() / sorted.len() as f32;
        let past = |limit: f32| {
            100.0 * sorted.iter().filter(|&&angle| angle > limit).count() as f32
                / sorted.len() as f32
        };
        format!(
            "mean {mean:5.2} deg  median {:5.2}  p90 {:5.2}  >10deg {:5.2}%  >20deg {:5.2}%  silhouette {:5.2}%",
            at(0.5),
            at(0.9),
            past(10.0),
            past(20.0),
            self.silhouette_mismatch_percent()
        )
    }

    pub fn mean(&self) -> f32 {
        if self.angles.is_empty() {
            return 0.0;
        }
        self.angles.iter().sum::<f32>() / self.angles.len() as f32
    }

    pub fn silhouette_mismatch_percent(&self) -> f32 {
        100.0 * self.silhouette_only_one as f32 / self.silhouette_either.max(1) as f32
    }
}

fn decode(pixel: Rgba<u8>) -> Option<Vec3> {
    let normal = Vec3::new(
        f32::from(pixel[0]) / 255.0 * 2.0 - 1.0,
        f32::from(pixel[1]) / 255.0 * 2.0 - 1.0,
        f32::from(pixel[2]) / 255.0 * 2.0 - 1.0,
    );
    UNIT_BAND.contains(&normal.length()).then_some(normal)
}

/// Dark blue for agreement through orange to red at the top of the range.
fn ramp(t: f32) -> Rgba<u8> {
    let t = t.clamp(0.0, 1.0);
    let stops = [
        (0.0, [16.0, 20.0, 48.0]),
        (0.33, [40.0, 120.0, 200.0]),
        (0.66, [240.0, 170.0, 40.0]),
        (1.0, [255.0, 40.0, 40.0]),
    ];
    let mut color = stops[stops.len() - 1].1;
    for pair in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            let f = (t - t0) / (t1 - t0);
            color = [
                c0[0] + (c1[0] - c0[0]) * f,
                c0[1] + (c1[1] - c0[1]) * f,
                c0[2] + (c1[2] - c0[2]) * f,
            ];
            break;
        }
    }
    Rgba([color[0] as u8, color[1] as u8, color[2] as u8, 255])
}
