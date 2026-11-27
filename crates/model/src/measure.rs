//! Per-triangle and per-object surface measurements: world area, UV area, and
//! each object's bounds.
//!
//! Shared by the audit's density findings and the renderer's density heat maps,
//! so the figure a finding quotes and the colour the view paints come from the
//! same arithmetic.

use glam::Vec3;

use crate::{Bounds, ModelData};

/// The three corner positions of triangle `triangle`, or `None` for an index
/// past the end or a corner out of range.
pub fn triangle_corners(model: &ModelData, triangle: usize) -> Option<[Vec3; 3]> {
    let corners = model.indices.get(triangle * 3..triangle * 3 + 3)?;
    Some([
        model.vertices.get(corners[0] as usize)?.position,
        model.vertices.get(corners[1] as usize)?.position,
        model.vertices.get(corners[2] as usize)?.position,
    ])
}

/// World-space area of triangle `triangle`, in square meters (import normalizes
/// every file to meters). Zero for a malformed index.
pub fn triangle_area(model: &ModelData, triangle: usize) -> f32 {
    triangle_corners(model, triangle).map_or(0.0, |[a, b, c]| (b - a).cross(c - a).length() * 0.5)
}

/// Signed UV-space area of triangle `triangle` in UV set `channel`: positive for
/// counter-clockwise UVs, negative for a mirrored (flipped) triangle.
pub fn triangle_uv_area(model: &ModelData, triangle: usize, channel: usize) -> f32 {
    let Some(corners) = model.indices.get(triangle * 3..triangle * 3 + 3) else {
        return 0.0;
    };
    let uv = |corner: u32| model.uv_for_channel(corner as usize, channel);
    let (a, b, c) = (uv(corners[0]), uv(corners[1]), uv(corners[2]));
    (b - a).perp_dot(c - a) * 0.5
}

/// How much a triangle's UVs stretch it one way more than the other: the
/// ratio of the larger to the smaller singular value of the map from the
/// triangle (in its own plane) to its UVs. 1 where the UVs are a scaled copy of
/// the shape; capped at 50, so one collapsed sliver cannot swamp an area-weighted
/// percentile.
/// `None` for a triangle with no area.
pub fn uv_stretch(points: [glam::Vec3; 3], uvs: [glam::Vec2; 3]) -> Option<f32> {
    let (e1, e2) = (points[1] - points[0], points[2] - points[0]);
    let normal = e1.cross(e2);
    let (Some(x), Some(n)) = (e1.try_normalize(), normal.try_normalize()) else {
        return None;
    };
    let y = n.cross(x);
    let (p00, p01, p10, p11) = (e1.dot(x), e2.dot(x), e1.dot(y), e2.dot(y));
    let det = p00 * p11 - p01 * p10;
    if det.abs() <= 1.0e-20 {
        return None;
    }
    let (q1, q2) = (uvs[1] - uvs[0], uvs[2] - uvs[0]);
    // J = U * P^-1, with U's columns the UV edges.
    let a = (q1.x * p11 - q2.x * p10) / det;
    let b = (-q1.x * p01 + q2.x * p00) / det;
    let c = (q1.y * p11 - q2.y * p10) / det;
    let d = (-q1.y * p01 + q2.y * p00) / det;
    // A 2x2 matrix's singular values, in closed form.
    let energy = 0.5 * (a * a + b * b + c * c + d * d);
    let determinant = a * d - b * c;
    let root = (energy * energy - determinant * determinant)
        .max(0.0)
        .sqrt();
    let larger = (energy + root).sqrt();
    let smaller = (energy - root).max(0.0).sqrt();
    Some(if smaller <= larger / 50.0 {
        50.0
    } else {
        larger / smaller
    })
}

/// [`uv_stretch`] of `model`'s triangle in UV set `channel`.
pub fn triangle_uv_stretch(model: &ModelData, triangle: usize, channel: usize) -> Option<f32> {
    let corners = model.indices.get(triangle * 3..triangle * 3 + 3)?;
    let points = [0, 1, 2].map(|k| {
        model
            .vertices
            .get(corners[k] as usize)
            .map_or(glam::Vec3::ZERO, |vertex| vertex.position)
    });
    let uvs = [0, 1, 2].map(|k| model.uv_for_channel(corners[k] as usize, channel));
    uv_stretch(points, uvs)
}

/// Per-triangle world area and signed UV area, the two numbers every density
/// figure is built from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SurfaceMeasure {
    pub world_area: Vec<f32>,
    pub uv_area: Vec<f32>,
}

impl SurfaceMeasure {
    /// Measure every triangle of `model` in UV set `channel`.
    pub fn new(model: &ModelData, channel: usize) -> Self {
        let triangle_count = model.indices.len() / 3;
        Self {
            world_area: (0..triangle_count)
                .map(|triangle| triangle_area(model, triangle))
                .collect(),
            uv_area: (0..triangle_count)
                .map(|triangle| triangle_uv_area(model, triangle, channel))
                .collect(),
        }
    }

    /// Texel density of a set of triangles in pixels per meter for a square
    /// texture `texture_size` pixels wide: `√(Σ|uv area| / Σ world area) · size`.
    /// `None` when the triangles have no world area.
    pub fn texel_density(
        &self,
        triangles: impl IntoIterator<Item = usize>,
        texture_size: f32,
    ) -> Option<f32> {
        let (mut uv, mut world) = (0.0_f64, 0.0_f64);
        for triangle in triangles {
            uv += f64::from(self.uv_area.get(triangle).copied().unwrap_or(0.0).abs());
            world += f64::from(self.world_area.get(triangle).copied().unwrap_or(0.0));
        }
        (world > 0.0).then(|| ((uv / world).sqrt() * f64::from(texture_size)) as f32)
    }
}

/// Per-node bounds of the triangles each node owns, parallel to
/// [`ModelData::nodes`]. A node owning no triangles gets [`Bounds::EMPTY`].
pub fn node_bounds(model: &ModelData) -> Vec<Bounds> {
    let mut bounds = vec![Bounds::EMPTY; model.nodes.len()];
    let triangle_count = model.indices.len() / 3;
    if model.triangles.node.len() != triangle_count {
        return bounds;
    }
    for (triangle, &node) in model.triangles.node.iter().enumerate() {
        let (Some(slot), Some(corners)) = (
            bounds.get_mut(node as usize),
            triangle_corners(model, triangle),
        ) else {
            continue;
        };
        for corner in corners {
            slot.include_point(corner);
        }
    }
    bounds
}

/// Per-node triangle lists, parallel to [`ModelData::nodes`] — the triangles
/// each node owns, in index order. Empty lists for a model without node tags.
pub fn node_triangles(model: &ModelData) -> Vec<Vec<u32>> {
    let mut lists = vec![Vec::new(); model.nodes.len()];
    let triangle_count = model.indices.len() / 3;
    if model.triangles.node.len() != triangle_count {
        return lists;
    }
    for (triangle, &node) in model.triangles.node.iter().enumerate() {
        if let Some(list) = lists.get_mut(node as usize) {
            list.push(triangle as u32);
        }
    }
    lists
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};

    use super::*;
    use crate::Vertex;

    fn square(uv_scale: f32) -> ModelData {
        let corners = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        ModelData {
            vertices: corners
                .iter()
                .map(|&p| Vertex {
                    position: p,
                    uv: Vec2::new(p.x, p.y) * uv_scale,
                    ..Default::default()
                })
                .collect(),
            indices: vec![0, 1, 2, 0, 2, 3],
            ..Default::default()
        }
    }

    #[test]
    fn a_unit_square_filling_its_uvs_has_the_texture_size_per_meter() {
        let model = square(1.0);
        let measure = SurfaceMeasure::new(&model, 0);
        assert!((measure.world_area.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        let density = measure.texel_density(0..2, 1024.0).unwrap();
        assert!((density - 1024.0).abs() < 1e-3);
        // Half the UV width is a quarter of the area, half the density.
        let half = SurfaceMeasure::new(&square(0.5), 0);
        assert!((half.texel_density(0..2, 1024.0).unwrap() - 512.0).abs() < 1e-3);
    }

    #[test]
    fn mirrored_uvs_have_negative_area() {
        let mut model = square(1.0);
        for vertex in &mut model.vertices {
            vertex.uv.x = -vertex.uv.x;
        }
        assert!(triangle_uv_area(&model, 0, 0) < 0.0);
    }
}
