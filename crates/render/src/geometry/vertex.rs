//! Low-level [`SceneVertex`] emit helpers shared by every geometry builder, plus
//! the normal-length scaling the normal-line views use. This is the only place
//! line and fill vertices are constructed, so a future vertex-layout change
//! touches this file plus the `SceneVertex` definition.
//!
//! Every helper takes the vertex's `deform` lane: geometry derived from the mesh
//! passes its source corner's lane (see [`super::deform`]) so it deforms with the
//! mesh; static geometry passes [`super::deform::NO_DEFORM`].

use review_model::ModelData;

use crate::scene::SceneVertex;

/// The extent a normal line is measured against when a model has none worth the
/// name — a tenth of a millimetre, the scale the camera lets a zoom reach, so a
/// model that small is still given lines in proportion to it.
const MIN_NORMAL_EXTENT: f32 = 1.0e-4;

/// Scale a fractional length slider value (UI clamps it to 1%–10%) into world
/// units relative to the model's largest bounding extent (so normals read
/// consistently regardless of scale).
///
/// The real extent, at every size: it used to be floored at a metre, so a 5 cm
/// bolt at 10 % grew 10 cm normals, twice the size of the bolt. A model without
/// bounds, or whose bounds are a point or not finite, measures against one world
/// unit, since it has no size of its own to be relative to.
pub(super) fn debug_normal_length(model: &ModelData, scale: f32) -> f32 {
    let max_extent = model
        .bounds
        .map(|bounds| bounds.size().max_element())
        .filter(|extent| extent.is_finite() && *extent > 0.0)
        .map_or(1.0, |extent| extent.max(MIN_NORMAL_EXTENT));
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
    deform: [u32; 4],
) {
    vertices.push(SceneVertex {
        position,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: color,
        deform,
    });
}

/// Push one *shaded* vertex: keeps the source normal (so the fragment is lit
/// rather than taking the shader's zero-normal overlay path) while carrying a
/// caller-chosen color in the `vertex_color` channel. Used by the skin-weight heat
/// map, which needs Lambert shading *and* a per-vertex data color.
pub(super) fn push_shaded_vertex(
    vertices: &mut Vec<SceneVertex>,
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    deform: [u32; 4],
) {
    vertices.push(SceneVertex {
        position,
        normal,
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: color,
        deform,
    });
}

/// Push one line whose two ends share a deform lane.
pub(super) fn push_line(
    vertices: &mut Vec<SceneVertex>,
    start: [f32; 3],
    end: [f32; 3],
    color: [f32; 4],
    deform: [u32; 4],
) {
    push_line_deformed(vertices, start, end, color, deform, deform);
}

/// Push one line whose ends deform independently — a skeleton bone whose head
/// follows the parent joint and whose tail follows the child.
pub(super) fn push_line_deformed(
    vertices: &mut Vec<SceneVertex>,
    start: [f32; 3],
    end: [f32; 3],
    color: [f32; 4],
    start_deform: [u32; 4],
    end_deform: [u32; 4],
) {
    vertices.push(SceneVertex {
        position: start,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: color,
        deform: start_deform,
    });
    vertices.push(SceneVertex {
        position: end,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
        vertex_color: color,
        deform: end_deform,
    });
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use review_model::Bounds;

    use super::*;

    fn sized(size: Option<Vec3>) -> ModelData {
        ModelData {
            bounds: size.map(|size| Bounds {
                min: Vec3::ZERO,
                max: size,
            }),
            ..Default::default()
        }
    }

    /// The slider is a fraction of the model's own largest extent at every
    /// scale: a 5 cm bolt at 10 % gets 5 mm normals, not the 10 cm a metre floor
    /// gave it, and a 20 m building 2 m.
    #[test]
    fn normal_length_is_a_fraction_of_the_real_extent() {
        let bolt = sized(Some(Vec3::new(0.01, 0.05, 0.01)));
        assert!((debug_normal_length(&bolt, 0.1) - 0.005).abs() < 1e-7);
        let building = sized(Some(Vec3::new(20.0, 8.0, 12.0)));
        assert!((debug_normal_length(&building, 0.1) - 2.0).abs() < 1e-5);
    }

    /// A model with no size of its own still gets lines: one world unit without
    /// bounds or with a point for them, and the floor for one far below a rivet.
    #[test]
    fn a_model_without_a_size_still_gets_normals() {
        assert_eq!(debug_normal_length(&sized(None), 0.1), 0.1);
        assert_eq!(debug_normal_length(&sized(Some(Vec3::ZERO)), 0.1), 0.1);
        let nan = sized(Some(Vec3::splat(f32::NAN)));
        assert_eq!(debug_normal_length(&nan, 0.1), 0.1);
        let speck = sized(Some(Vec3::splat(1.0e-7)));
        assert!((debug_normal_length(&speck, 0.1) - MIN_NORMAL_EXTENT * 0.1).abs() < 1e-9);
    }
}
