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
