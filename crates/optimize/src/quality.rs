//! How far a simplified mesh is from the one it was made from, measured on the
//! CPU: the change in the volume it encloses, the distance between the two
//! surfaces sampled both ways, and how far the shading normals turned.
//!
//! It is what lets a LOD level be judged by more than its triangle count (plan item
//! 1.5): the report gives every simplified level these figures against the source,
//! and the optional limits in [`QualityLimits`] stop a level early rather than let
//! it lose more than they allow. Deterministic, as every output is (D27): samples
//! are placed by a fixed low-discrepancy sequence over the triangles in order, and
//! nothing depends on how work fell across threads.

use std::borrow::Cow;

use glam::Vec3;
use review_model::{Bvh, ModelData, exact};
use serde::{Deserialize, Serialize};

use crate::submesh::Submesh;

/// Samples per direction for a level's report.
pub(crate) const REPORT_SAMPLES: usize = 8192;
/// Samples per direction for each candidate the limits' search measures: fewer,
/// because the search measures several, and it compares against a threshold
/// rather than printing a figure.
pub(crate) const LIMIT_SAMPLES: usize = 2048;

/// How far a simplified mesh is from the one it was made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelQuality {
    /// The change in enclosed volume as a share of the reference's: `-0.25` lost a
    /// quarter of it. `None` when the reference encloses none to speak of — an open
    /// sheet, whose "volume" depends on where the origin is.
    pub volume_change: Option<f32>,
    /// The distance between the two surfaces, sampled both ways, as a share of
    /// the reference's size: the largest side of its bounding box, the yardstick
    /// the simplifier's error limit is a share of.
    pub distance_mean: f32,
    /// The largest of those distances.
    pub distance_max: f32,
    /// The angle between the two surfaces' shading normals at matching points,
    /// in degrees.
    pub normal_deviation_mean: f32,
    /// The angle 95% of matching points stay within, in degrees.
    pub normal_deviation_p95: f32,
}

/// Limits a simplify may not pass, whatever its triangle target: a level stops
/// above its target rather than break one (plan item 1.5). Both off unless set.
/// Measured against the mesh the operation starts from, as its target is, and per
/// object, as its budget is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QualityLimits {
    /// The most an object's enclosed volume may change, as a share of it
    /// (`0.05` = 5%). Read only for an object that encloses a volume.
    pub max_volume_change: Option<f32>,
    /// The farthest the simplified surface may lie from the original anywhere, as
    /// a share of the object's size.
    pub max_distance: Option<f32>,
}

/// Which of [`QualityLimits`] a measurement broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Broken {
    Volume,
    Distance,
}

impl QualityLimits {
    /// Whether either limit is set.
    pub fn any(&self) -> bool {
        self.max_volume_change.is_some() || self.max_distance.is_some()
    }

    /// The first limit `quality` breaks, or `None` when it keeps to both.
    pub(crate) fn broken_by(&self, quality: &LevelQuality) -> Option<Broken> {
        if let (Some(limit), Some(change)) = (self.max_volume_change, quality.volume_change)
            && change.abs() > limit
        {
            return Some(Broken::Volume);
        }
        if let Some(limit) = self.max_distance
            && quality.distance_max > limit
        {
            return Some(Broken::Distance);
        }
        None
    }
}

/// A mesh prepared for measuring: its triangles under a closest-point hierarchy,
/// their areas for placing samples, and the volume and size the figures are read
/// against.
pub(crate) struct Surface<'a> {
    model: Cow<'a, ModelData>,
    bvh: Bvh,
    /// Each triangle's facing, for matching a point to the nearest surface that
    /// faces the same way (see [`compare`]).
    face_normals: Vec<Vec3>,
    /// Running total of the triangles' areas, for placing samples by area.
    cumulative_area: Vec<f64>,
    volume: f64,
    size: f32,
}

impl<'a> Surface<'a> {
    /// The surface of a model as it stands, borrowed.
    #[cfg(test)]
    pub(crate) fn of_model(model: &'a ModelData) -> Self {
        Self::new(Cow::Borrowed(model))
    }

    /// The surface of some pieces, joined: an object, or a whole level.
    pub(crate) fn of_pieces<'p>(pieces: impl IntoIterator<Item = &'p Submesh>) -> Surface<'static> {
        let mut model = ModelData::default();
        for piece in pieces {
            let base = model.vertices.len() as u32;
            model.vertices.extend_from_slice(&piece.vertices);
            model
                .indices
                .extend(piece.indices.iter().map(|&index| index + base));
        }
        Surface::new(Cow::Owned(model))
    }

    /// One object's surface in a model, by its triangles' node tags: what a
    /// quality limit measures that object against — the object as the run found
    /// it, whatever operations came before.
    pub(crate) fn of_node(model: &ModelData, node: u32) -> Surface<'static> {
        let tagged = model.triangles.node.len() == model.indices.len() / 3;
        let mut object = ModelData::default();
        for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
            if tagged && model.triangles.node[triangle] != node {
                continue;
            }
            for &corner in corners {
                object.indices.push(object.vertices.len() as u32);
                object.vertices.push(model.vertices[corner as usize]);
            }
        }
        Surface::new(Cow::Owned(object))
    }

    fn new(model: Cow<'a, ModelData>) -> Self {
        let bvh = Bvh::build(&model);
        let mut face_normals = Vec::with_capacity(model.indices.len() / 3);
        let mut cumulative_area = Vec::with_capacity(model.indices.len() / 3);
        let mut total = 0.0f64;
        let mut volume = 0.0f64;
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for triangle in model.indices.as_chunks::<3>().0 {
            let [a, b, c] = corners(&model, triangle);
            let cross = (b - a).cross(c - a);
            face_normals.push(cross.normalize_or_zero());
            total += f64::from(0.5 * cross.length());
            cumulative_area.push(total);
            // The divergence theorem: each triangle's signed tetrahedron to the
            // origin. Summed in f64, since a large closed mesh's terms cancel.
            volume += f64::from(a.dot(b.cross(c))) / 6.0;
            for corner in [a, b, c] {
                min = min.min(corner);
                max = max.max(corner);
            }
        }
        let size = if min.x <= max.x {
            (max - min).max_element()
        } else {
            0.0
        };
        Self {
            model,
            bvh,
            face_normals,
            cumulative_area,
            volume,
            size,
        }
    }

    fn total_area(&self) -> f64 {
        self.cumulative_area.last().copied().unwrap_or(0.0)
    }

    /// `count` points spread over the surface by area, with the shading normal at
    /// each: the R2 low-discrepancy sequence, in triangle order, so the same mesh
    /// is always sampled at the same points.
    fn samples(&self, count: usize) -> Vec<(Vec3, Vec3)> {
        let total = self.total_area();
        if total <= 0.0 || count == 0 {
            return Vec::new();
        }
        // The plastic number's reciprocals: R2's two step sizes.
        const A1: f64 = 0.754_877_666_246_692_7;
        const A2: f64 = 0.569_840_290_998_053_2;
        (0..count)
            .map(|i| {
                let along = (i as f64 + 0.5) / count as f64 * total;
                let triangle = self
                    .cumulative_area
                    .partition_point(|&area| area < along)
                    .min(self.cumulative_area.len() - 1);
                let (mut u, mut v) = (
                    (0.5 + i as f64 * A1).fract() as f32,
                    (0.5 + i as f64 * A2).fract() as f32,
                );
                if u + v > 1.0 {
                    (u, v) = (1.0 - u, 1.0 - v);
                }
                let weights = Vec3::new(1.0 - u - v, u, v);
                let indices = &self.model.indices[triangle * 3..triangle * 3 + 3];
                let [a, b, c] = corners(&self.model, indices);
                let point = a * weights.x + b * weights.y + c * weights.z;
                (point, normal_at(&self.model, indices, weights))
            })
            .collect()
    }
}

/// How far `simplified` is from `reference`, with `samples` points each way.
pub(crate) fn compare(
    reference: &Surface<'_>,
    simplified: &Surface<'_>,
    samples: usize,
) -> LevelQuality {
    let mut distances = Vec::with_capacity(samples * 2);
    let mut angles = Vec::with_capacity(samples * 2);
    for (from, to) in [(reference, simplified), (simplified, reference)] {
        for (point, normal) in from.samples(samples) {
            let Some(hit) = to.bvh.closest_point(&to.model, point, f32::INFINITY) else {
                continue;
            };
            distances.push(hit.distance_squared.sqrt());
            // The angle is read off the nearest surface facing the same way: on a
            // thin wall the nearest of all is the far side, half a turn away, and
            // would report the wall as inside out.
            let facing = to
                .bvh
                .closest_point_where(&to.model, point, f32::INFINITY, |triangle| {
                    to.face_normals[triangle as usize].dot(normal) > 0.0
                })
                .unwrap_or(hit);
            let indices =
                &to.model.indices[facing.triangle as usize * 3..facing.triangle as usize * 3 + 3];
            let other = normal_at(&to.model, indices, facing.barycentric);
            angles.push(exact::acos(normal.dot(other).clamp(-1.0, 1.0)).to_degrees());
        }
    }
    let size = reference.size.max(f32::MIN_POSITIVE);
    let mean = |values: &[f32]| {
        if values.is_empty() {
            0.0
        } else {
            (values.iter().map(|&value| f64::from(value)).sum::<f64>() / values.len() as f64) as f32
        }
    };
    angles.sort_unstable_by(f32::total_cmp);
    let p95 = angles
        .get(((angles.len() as f64 * 0.95) as usize).min(angles.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0);
    // A volume small against the bounding box's is an open sheet's, which
    // depends on where the origin happens to be rather than on the shape.
    let encloses = reference.volume.abs() > 1.0e-6 * f64::from(reference.size).powi(3);
    LevelQuality {
        volume_change: encloses
            .then(|| ((simplified.volume - reference.volume) / reference.volume.abs()) as f32),
        distance_mean: mean(&distances) / size,
        distance_max: distances.iter().copied().fold(0.0, f32::max) / size,
        normal_deviation_mean: mean(&angles),
        normal_deviation_p95: p95,
    }
}

fn corners(model: &ModelData, triangle: &[u32]) -> [Vec3; 3] {
    [0, 1, 2].map(|corner| model.vertices[triangle[corner] as usize].position)
}

/// The shading normal at `weights` within a triangle: its corners' normals
/// interpolated, or the face's own where those cancel out.
fn normal_at(model: &ModelData, triangle: &[u32], weights: Vec3) -> Vec3 {
    let normal = [0, 1, 2]
        .iter()
        .map(|&corner| model.vertices[triangle[corner] as usize].normal * weights[corner])
        .sum::<Vec3>();
    normal.try_normalize().unwrap_or_else(|| {
        let [a, b, c] = corners(model, triangle);
        (b - a).cross(c - a).normalize_or_zero()
    })
}

#[cfg(test)]
mod tests {
    use review_model::Vertex;

    use super::*;

    /// A closed axis-aligned box from `min` to `max`, outward-wound, with face
    /// normals.
    fn cuboid(min: Vec3, max: Vec3) -> ModelData {
        let mut model = ModelData::default();
        let faces: [(Vec3, [Vec3; 4]); 6] = [
            (
                Vec3::X,
                [
                    Vec3::new(1., 0., 0.),
                    Vec3::new(1., 1., 0.),
                    Vec3::new(1., 1., 1.),
                    Vec3::new(1., 0., 1.),
                ],
            ),
            (
                Vec3::NEG_X,
                [
                    Vec3::new(0., 0., 0.),
                    Vec3::new(0., 0., 1.),
                    Vec3::new(0., 1., 1.),
                    Vec3::new(0., 1., 0.),
                ],
            ),
            (
                Vec3::Y,
                [
                    Vec3::new(0., 1., 0.),
                    Vec3::new(0., 1., 1.),
                    Vec3::new(1., 1., 1.),
                    Vec3::new(1., 1., 0.),
                ],
            ),
            (
                Vec3::NEG_Y,
                [
                    Vec3::new(0., 0., 0.),
                    Vec3::new(1., 0., 0.),
                    Vec3::new(1., 0., 1.),
                    Vec3::new(0., 0., 1.),
                ],
            ),
            (
                Vec3::Z,
                [
                    Vec3::new(0., 0., 1.),
                    Vec3::new(1., 0., 1.),
                    Vec3::new(1., 1., 1.),
                    Vec3::new(0., 1., 1.),
                ],
            ),
            (
                Vec3::NEG_Z,
                [
                    Vec3::new(0., 0., 0.),
                    Vec3::new(0., 1., 0.),
                    Vec3::new(1., 1., 0.),
                    Vec3::new(1., 0., 0.),
                ],
            ),
        ];
        for (normal, quad) in faces {
            let base = model.vertices.len() as u32;
            for unit in quad {
                model.vertices.push(Vertex {
                    position: min + unit * (max - min),
                    normal,
                    ..Vertex::default()
                });
            }
            model
                .indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        model
    }

    #[test]
    fn a_mesh_against_itself_has_changed_nothing() {
        let model = cuboid(Vec3::ZERO, Vec3::ONE);
        let surface = Surface::of_model(&model);
        let quality = compare(&surface, &Surface::of_model(&model), 512);
        assert_eq!(quality.volume_change, Some(0.0));
        assert!(quality.distance_max < 1.0e-6, "{quality:?}");
        assert!(quality.normal_deviation_p95 < 1.0e-3, "{quality:?}");
    }

    #[test]
    fn a_shrunken_box_reports_its_lost_volume_and_how_far_it_moved() {
        let reference = cuboid(Vec3::ZERO, Vec3::ONE);
        // Half as deep: half the volume, the far face 0.5 (half the size) away.
        let shrunk = cuboid(Vec3::ZERO, Vec3::new(1.0, 1.0, 0.5));
        let quality = compare(
            &Surface::of_model(&reference),
            &Surface::of_model(&shrunk),
            2048,
        );
        let change = quality.volume_change.expect("a box encloses a volume");
        assert!((change + 0.5).abs() < 1.0e-5, "{change}");
        assert!((quality.distance_max - 0.5).abs() < 0.01, "{quality:?}");
        assert!(quality.distance_mean > 0.0 && quality.distance_mean < quality.distance_max);

        let limits = QualityLimits {
            max_volume_change: Some(0.4),
            ..QualityLimits::default()
        };
        assert_eq!(limits.broken_by(&quality), Some(Broken::Volume));
        let limits = QualityLimits {
            max_distance: Some(0.6),
            ..QualityLimits::default()
        };
        assert_eq!(limits.broken_by(&quality), None);
    }

    #[test]
    fn the_same_mesh_is_always_sampled_at_the_same_points() {
        let model = cuboid(Vec3::ZERO, Vec3::new(2.0, 1.0, 3.0));
        let surface = Surface::of_model(&model);
        assert_eq!(surface.samples(300), surface.samples(300));
    }
}
