//! The density heat maps' CPU geometry: a vertex buffer parallel to the mesh's
//! own (one entry per render vertex, same order, like the skin-weight map), with
//! each face's ramp colour baked into its corners.
//!
//! Values are per *face*: import gives every face corner its own render vertex,
//! so a polygon's corners carry exactly one colour. An Opt level shares its
//! vertices between faces, so there a vertex takes the mean of the faces around
//! it. The figures come from `review_model::measure`, the same arithmetic the
//! audit's findings quote.

use review_model::ModelData;
use review_model::measure::{SurfaceMeasure, node_bounds};

use crate::HeatMap;
use crate::scene::SceneVertex;

use super::deform::corner_deform;
use super::vertex::push_shaded_vertex;

/// Blend the three ramp stops at `x` in `-1..=1` (low → on target → high).
pub(crate) fn heat_ramp(stops: [[f32; 3]; 3], x: f32) -> [f32; 3] {
    let x = if x.is_finite() {
        x.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let (from, to, t) = if x < 0.0 {
        (stops[1], stops[0], -x)
    } else {
        (stops[1], stops[2], x)
    };
    [
        from[0] + (to[0] - from[0]) * t,
        from[1] + (to[1] - from[1]) * t,
        from[2] + (to[2] - from[2]) * t,
    ]
}

/// Each face's position on the ramp (`-1..=1`), or `None` for a face the map
/// has nothing to say about (no UV area, no surface), indexed like the faces —
/// or the triangles, for a model without face topology.
fn face_values(model: &ModelData, heat: HeatMap) -> Vec<Option<f32>> {
    let triangle_count = model.indices.len() / 3;
    let by_triangle = model.faces.is_empty() || model.triangles.to_face.len() != triangle_count;
    let face_count = if by_triangle {
        triangle_count
    } else {
        model.faces.len()
    };
    let face_of = |triangle: usize| {
        if by_triangle {
            triangle
        } else {
            model.triangles.to_face[triangle] as usize
        }
    };
    let measure = SurfaceMeasure::new(model, 0);
    // Per face: world area, |UV area|, triangle count, owning node.
    let mut world = vec![0.0_f64; face_count];
    let mut uv = vec![0.0_f64; face_count];
    let mut triangles = vec![0_u32; face_count];
    let mut node = vec![u32::MAX; face_count];
    for triangle in 0..triangle_count {
        let face = face_of(triangle);
        if face >= face_count {
            continue;
        }
        world[face] += f64::from(measure.world_area[triangle]);
        uv[face] += f64::from(measure.uv_area[triangle].abs());
        triangles[face] += 1;
        if let Some(&owner) = model.triangles.node.get(triangle) {
            node[face] = owner;
        }
    }
    match heat {
        HeatMap::None => vec![None; face_count],
        HeatMap::TexelDensity {
            texture_size,
            target,
            tolerance,
        } => {
            let span = f64::from(tolerance.max(1.0001)).log2();
            (0..face_count)
                .map(|face| {
                    (world[face] > 0.0 && uv[face] > 0.0).then(|| {
                        let density = (uv[face] / world[face]).sqrt() * f64::from(texture_size);
                        ((density / f64::from(target)).log2() / span) as f32
                    })
                })
                .collect()
        }
        HeatMap::TriangleDensity {
            min_pixel_area,
            screen_height,
        } => {
            let bounds = node_bounds(model);
            let diameter = |node: u32| {
                bounds
                    .get(node as usize)
                    .filter(|bounds| !bounds.is_empty())
                    .map(|bounds| f64::from(bounds.radius()) * 2.0)
                    .or_else(|| model.bounds.map(|bounds| f64::from(bounds.radius()) * 2.0))
                    .unwrap_or(1.0)
                    .max(1.0e-6)
            };
            (0..face_count)
                .map(|face| {
                    (world[face] > 0.0 && triangles[face] > 0).then(|| {
                        // The pixels one of this face's triangles covers with its
                        // object filling the screen (half facing away, A/4 mean
                        // projection — see the audit's density check).
                        let k = f64::from(screen_height) / diameter(node[face]);
                        let pixels = world[face] / f64::from(triangles[face]) * k * k / 2.0;
                        // Dense (too few pixels) reads high, like a hot spot.
                        (-(pixels / f64::from(min_pixel_area)).log2() / 3.0) as f32
                    })
                })
                .collect()
        }
    }
}

/// Build the heat-map vertex buffer: the mesh's own positions and normals, with
/// the ramp colour in `rgb` and, in `a`, how fully the ramp replaces the neutral
/// shaded base (1 for a measured face, 0 for one the map says nothing about).
pub(crate) fn heat_vertices(
    model: &ModelData,
    lanes: &[[u32; 4]],
    heat: HeatMap,
    stops: [[f32; 3]; 3],
) -> Vec<SceneVertex> {
    if !heat.is_active() {
        return Vec::new();
    }
    let values = face_values(model, heat);
    // Per corner, the mean of the measured faces that use it. Import gives every
    // face corner its own render vertex, so there that is just its face's value; an
    // Opt level is indexed, its vertices shared between faces, and the mean blends
    // them across a vertex rather than letting whichever triangle came last colour
    // it.
    let mut corner_sum = vec![0.0_f32; model.vertices.len()];
    let mut corner_faces = vec![0_u32; model.vertices.len()];
    let triangle_count = model.indices.len() / 3;
    let by_triangle = model.faces.is_empty() || model.triangles.to_face.len() != triangle_count;
    for triangle in 0..triangle_count {
        let face = if by_triangle {
            triangle
        } else {
            model.triangles.to_face[triangle] as usize
        };
        let Some(value) = values.get(face).copied().flatten() else {
            continue;
        };
        for &corner in &model.indices[triangle * 3..triangle * 3 + 3] {
            if let (Some(sum), Some(faces)) = (
                corner_sum.get_mut(corner as usize),
                corner_faces.get_mut(corner as usize),
            ) {
                *sum += value;
                *faces += 1;
            }
        }
    }
    let mut vertices = Vec::with_capacity(model.vertices.len());
    for (index, vertex) in model.vertices.iter().enumerate() {
        let (rgb, alpha) = match corner_faces[index] {
            0 => (stops[1], 0.0),
            faces => (heat_ramp(stops, corner_sum[index] / faces as f32), 1.0),
        };
        push_shaded_vertex(
            &mut vertices,
            vertex.position.to_array(),
            vertex.normal.to_array(),
            [rgb[0], rgb[1], rgb[2], alpha],
            corner_deform(lanes, index),
        );
    }
    vertices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ramp_runs_low_through_target_to_high() {
        let stops = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
        assert_eq!(heat_ramp(stops, -1.0), [0.0, 0.0, 1.0]);
        assert_eq!(heat_ramp(stops, 0.0), [0.0, 1.0, 0.0]);
        assert_eq!(heat_ramp(stops, 1.0), [1.0, 0.0, 0.0]);
        assert_eq!(heat_ramp(stops, 7.0), [1.0, 0.0, 0.0], "clamped");
        assert_eq!(heat_ramp(stops, f32::NAN), [0.0, 1.0, 0.0]);
    }

    /// An indexed mesh (an Opt level) shares a vertex between faces, and that
    /// vertex takes their mean rather than whichever face was walked last.
    #[test]
    fn a_shared_vertex_takes_the_mean_of_its_faces() {
        use glam::{Vec2, Vec3};
        use review_model::Vertex;

        let vertex = |x: f32, y: f32, u: f32, v: f32| Vertex {
            position: Vec3::new(x, y, 0.0),
            uv: Vec2::new(u, v),
            ..Default::default()
        };
        // Two triangles of equal world area over the shared edge 1-2; the second
        // has three times the UV area, so it reads denser.
        let model = ModelData {
            vertices: vec![
                vertex(0.0, 0.0, 0.0, 0.0),
                vertex(1.0, 0.0, 1.0, 0.0),
                vertex(0.0, 1.0, 0.0, 1.0),
                vertex(1.0, 1.0, 2.0, 2.0),
            ],
            indices: vec![0, 1, 2, 1, 3, 2],
            ..Default::default()
        };
        let heat = HeatMap::TexelDensity {
            texture_size: 1024.0,
            target: 1024.0,
            tolerance: 2.0,
        };
        let stops = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
        let values = face_values(&model, heat);
        let (first, second) = (values[0].expect("measured"), values[1].expect("measured"));
        assert!(second > first, "the second face is the denser");

        let vertices = heat_vertices(&model, &[], heat, stops);
        let color = |index: usize| {
            let [r, g, b, _] = vertices[index].vertex_color;
            [r, g, b]
        };
        assert_eq!(color(0), heat_ramp(stops, first), "only the first face");
        assert_eq!(color(3), heat_ramp(stops, second), "only the second face");
        let mean = heat_ramp(stops, (first + second) * 0.5);
        assert_eq!(color(1), mean, "on the shared edge");
        assert_eq!(color(2), mean, "on the shared edge");
    }
}
