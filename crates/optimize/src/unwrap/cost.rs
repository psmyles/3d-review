//! What a seam costs along each edge: its length, times how much it would
//! show there.
//!
//! A seam shows least where the surface already breaks the eye: along a crease
//! (the shading changes there anyway), in a concave valley (where an ear meets
//! a head), and in a crevice the light barely reaches. It shows most across a
//! smooth, open, convex stretch. So an edge's weight is lowered by how sharply
//! its two faces meet — twice over when they meet in a valley — by how
//! occluded its ends are, and by the reference UVs being cut along it already
//! (with [`crate::stack::ExistingSeams::Prefer`]). The weights multiply and are
//! floored at [`LEAST_WEIGHT`], so length always counts for something.
//!
//! Occlusion is the share of a cosine-weighted hemisphere of
//! [`OCCLUSION_RAYS`] rays from each vertex that meet the object within
//! [`OCCLUSION_REACH`] of its size: local, so a valley reads as one whatever
//! stands around the object. The rays are a fixed set turned into each
//! vertex's frame, so the result is a function of the mesh alone.

use glam::Vec3;
use review_model::{Bvh, ModelData, Vertex, exact};

use super::surface::{NONE, Surface};

/// How much a crease lowers an edge's weight: at a right angle (and past it),
/// a convex edge weighs `1 / (1 + CREASE_WEIGHT)`, a concave one
/// `1 / (1 + 2 CREASE_WEIGHT)`.
const CREASE_WEIGHT: f64 = 4.0;

/// How much occlusion lowers an edge's weight: fully occluded, it weighs
/// `1 / (1 + OCCLUSION_WEIGHT)`.
const OCCLUSION_WEIGHT: f64 = 4.0;

/// The weight of an edge the reference UVs are cut along, before anything
/// else lowers it.
const PREFERRED_WEIGHT: f64 = 0.25;

/// No edge weighs less than this: a seam along the cheapest edges still
/// prefers the shorter way.
const LEAST_WEIGHT: f64 = 0.02;

/// Rays per vertex.
const OCCLUSION_RAYS: u32 = 16;

/// How far the rays reach, as a share of the object's bounding diagonal.
const OCCLUSION_REACH: f32 = 0.15;

/// How far off the surface the rays start, as a share of the diagonal.
const OCCLUSION_LIFT: f32 = 1.0e-4;

/// An object's surface, for occlusion: built once per object, shared by every
/// island in it.
pub(super) struct Occluders {
    model: ModelData,
    bvh: Bvh,
    reach: f32,
    lift: f32,
    /// The rays, in a frame whose Z is the normal.
    rays: Vec<Vec3>,
}

impl Occluders {
    pub(super) fn new(positions: &[Vec3], triangles: &[[u32; 3]]) -> Self {
        let model = ModelData {
            vertices: positions
                .iter()
                .map(|&position| Vertex {
                    position,
                    ..Vertex::default()
                })
                .collect(),
            indices: triangles.iter().flatten().copied().collect(),
            ..Default::default()
        };
        let bvh = Bvh::build(&model);
        let (low, high) = positions
            .iter()
            .fold((Vec3::MAX, Vec3::MIN), |(low, high), &point| {
                (low.min(point), high.max(point))
            });
        let diagonal = if positions.is_empty() {
            0.0
        } else {
            low.distance(high)
        };
        // Cosine-weighted: Hammersley points, the second coordinate an angle.
        let rays = (0..OCCLUSION_RAYS)
            .map(|i| {
                let u = (i as f32 + 0.5) / OCCLUSION_RAYS as f32;
                let angle = i.reverse_bits() as f32 * 2.328_306_4e-10 * std::f32::consts::TAU;
                let radius = u.sqrt();
                Vec3::new(
                    radius * exact::cos(angle),
                    radius * exact::sin(angle),
                    (1.0 - u).max(0.0).sqrt(),
                )
            })
            .collect();
        Self {
            model,
            bvh,
            reach: OCCLUSION_REACH * diagonal,
            lift: (OCCLUSION_LIFT * diagonal).max(1.0e-7),
            rays,
        }
    }

    /// The share of rays from `point` about `normal` that meet the object.
    fn occlusion(&self, point: Vec3, normal: Vec3) -> f64 {
        if normal == Vec3::ZERO || self.reach <= 0.0 {
            return 0.0;
        }
        let (x, y) = basis(normal);
        let origin = point + normal * self.lift;
        let hits = self
            .rays
            .iter()
            .filter(|ray| {
                let direction = x * ray.x + y * ray.y + normal * ray.z;
                self.bvh
                    .ray_occluded(&self.model, origin, direction, self.reach)
            })
            .count();
        hits as f64 / self.rays.len() as f64
    }
}

/// Per edge of `surface`, what a seam along it costs.
pub(super) fn edge_costs(surface: &Surface, occluders: &Occluders) -> Vec<f64> {
    let normals = surface.normals();
    let occlusion: Vec<f64> = surface
        .points
        .iter()
        .zip(&normals)
        .map(|(&point, &normal)| occluders.occlusion(point, normal))
        .collect();
    let face_normal = |triangle: u32| {
        let [a, b, c] = surface.corners[triangle as usize]
            .map(|vertex| surface.points[vertex as usize].as_dvec3());
        (b - a).cross(c - a).normalize_or_zero()
    };
    surface
        .edges
        .iter()
        .map(|edge| {
            let mut weight = 1.0;
            if edge.sides[1] != NONE {
                let (first, second) = (face_normal(edge.sides[0]), face_normal(edge.sides[1]));
                let bend = (1.0 - first.dot(second)).clamp(0.0, 1.0);
                // Concave where the second face's far corner rises off the
                // first face's plane.
                let far = surface.corners[edge.sides[1] as usize]
                    .into_iter()
                    .find(|vertex| !edge.ends.contains(vertex))
                    .unwrap_or(edge.ends[0]);
                let rise = first.dot(
                    surface.points[far as usize].as_dvec3()
                        - surface.points[edge.ends[0] as usize].as_dvec3(),
                );
                let valley = if rise > 0.0 { 2.0 } else { 1.0 };
                weight /= 1.0 + CREASE_WEIGHT * bend * valley;
            }
            let occluded =
                0.5 * (occlusion[edge.ends[0] as usize] + occlusion[edge.ends[1] as usize]);
            weight /= 1.0 + OCCLUSION_WEIGHT * occluded;
            if edge.preferred {
                weight *= PREFERRED_WEIGHT;
            }
            edge.length * weight.max(LEAST_WEIGHT)
        })
        .collect()
}

/// An orthonormal pair perpendicular to unit `n` (Duff et al., "Building an
/// Orthonormal Basis, Revisited").
fn basis(n: Vec3) -> (Vec3, Vec3) {
    let sign = 1.0_f32.copysign(n.z);
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    (
        Vec3::new(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x),
        Vec3::new(b, sign + n.y * n.y * a, -n.y),
    )
}
