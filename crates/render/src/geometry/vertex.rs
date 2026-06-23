//! Low-level [`SceneVertex`] emit helpers shared by every geometry builder, plus
//! the normal-length scaling the normal-line views use. This is the only place
//! line and fill vertices are constructed, so a future vertex-layout change
//! touches this file plus the `SceneVertex` definition.

use glam::Vec3;
use review_model::ModelData;

use crate::scene::SceneVertex;

/// Scale a fractional length slider value (UI clamps it to 1%–10%) into world
/// units relative to the model's largest bounding extent (so normals read
/// consistently regardless of scale).
pub(super) fn debug_normal_length(model: &ModelData, scale: f32) -> f32 {
    let size = model
        .bounds
        .map(|bounds| bounds.size())
        .unwrap_or(Vec3::splat(1.0));
    let max_extent = size.max_element().max(1.0);
    max_extent * scale.max(0.001)
}

/// Push one filled-triangle corner: a zero-normal vertex carrying `color` in the
/// `vertex_color` channel, so the scene shader's overlay path returns it flat (the
/// same path the line views use). The mesh sources its color from the material
/// uniform, so the per-vertex color channel now belongs to the overlays/lines.
pub(super) fn push_fill_vertex(
    vertices: &mut Vec<SceneVertex>,
    position: [f32; 3],
    color: [f32; 4],
) {
    vertices.push(SceneVertex {
        position,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: color,
    });
}

pub(super) fn push_line(
    vertices: &mut Vec<SceneVertex>,
    start: [f32; 3],
    end: [f32; 3],
    color: [f32; 4],
) {
    vertices.push(SceneVertex {
        position: start,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: color,
    });
    vertices.push(SceneVertex {
        position: end,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: color,
    });
}
