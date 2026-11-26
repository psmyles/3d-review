//! The edge-wear bake: how sharply the surface turns at each point, from the
//! dihedral angles of the edges around it, mapped from an angle range to 0-1
//! and written into the vertex colour.
//!
//! Per object, so a material border is no edge: an object's pieces are joined
//! by position, every edge two faces share is measured (the angle between
//! their normals, and whether it sticks out — the second face bends away from
//! the first's normal — or in), and each point takes the strongest wear of the
//! edges that meet there. Every copy of a point (a hard edge, a UV seam) gets
//! the same value. Only `sqrt`, `acos` (through `libm`) and comparisons: the
//! same bits everywhere.

use std::collections::HashMap;

use glam::Vec3;
use review_model::ModelData;

use crate::process::{RunContext, is_excluded, resolve_op};
use crate::stack::{BakeWearParams, OpInstance, OpKind, OptStack};
use crate::submesh::Submesh;

/// Bake wear into every eligible object's pieces.
pub(crate) fn bake_wear_submeshes(
    submeshes: &mut [Submesh],
    op: &OpInstance,
    stack: &OptStack,
    _model: &ModelData,
    run: &RunContext<'_>,
) {
    let _z = crate::prof::zone!("Bake Wear");
    let OpKind::BakeWear(global) = &op.kind else {
        return;
    };
    let mut nodes: Vec<u32> = Vec::new();
    for piece in submeshes.iter() {
        if !nodes.contains(&piece.node) {
            nodes.push(piece.node);
        }
    }
    for node in nodes {
        if run.cancelled() {
            return;
        }
        if is_excluded(stack, node) {
            continue;
        }
        let params = match resolve_op(stack, op, node) {
            OpKind::BakeWear(params) => *params,
            _ => *global,
        };
        let mut pieces: Vec<&mut Submesh> = submeshes
            .iter_mut()
            .filter(|piece| piece.node == node && !piece.is_empty())
            .collect();
        bake_node(&mut pieces, &params);
    }
}

/// `-0.0` and `+0.0` as one position.
fn key(position: Vec3) -> [u32; 3] {
    position
        .to_array()
        .map(|value| if value == 0.0 { 0 } else { value.to_bits() })
}

fn bake_node(pieces: &mut [&mut Submesh], params: &BakeWearParams) {
    // Every vertex's point, across the object's pieces.
    let mut points: HashMap<[u32; 3], u32> = HashMap::new();
    let point_of: Vec<Vec<u32>> = pieces
        .iter()
        .map(|piece| {
            piece
                .vertices
                .iter()
                .map(|vertex| {
                    let next = points.len() as u32;
                    *points.entry(key(vertex.position)).or_insert(next)
                })
                .collect()
        })
        .collect();

    // Every face: its points, its unit normal and its centre.
    struct Face {
        points: [u32; 3],
        normal: Vec3,
        centre: Vec3,
    }
    let mut faces = Vec::new();
    for (piece, own) in pieces.iter().zip(&point_of) {
        for corners in piece.indices.as_chunks::<3>().0 {
            let [a, b, c] = corners.map(|corner| piece.vertices[corner as usize].position);
            let Some(normal) = (b - a).cross(c - a).try_normalize() else {
                continue;
            };
            faces.push(Face {
                points: corners.map(|corner| own[corner as usize]),
                normal,
                centre: (a + b + c) / 3.0,
            });
        }
    }

    // Edges by their two points, with the faces on them.
    let mut edges: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (index, face) in faces.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (face.points[k], face.points[(k + 1) % 3]);
            if a != b {
                edges
                    .entry((a.min(b), a.max(b)))
                    .or_default()
                    .push(index as u32);
            }
        }
    }

    // Each point's strongest wear. Walking the faces' own edges, in face order,
    // keeps the visit order fixed (the map is only looked up).
    let span = (params.angle_full - params.angle_start).max(1.0e-3);
    let mut wear = vec![0.0f32; points.len()];
    for face in &faces {
        for k in 0..3 {
            let (a, b) = (face.points[k], face.points[(k + 1) % 3]);
            // A border, or a non-manifold edge, has no one angle.
            let Some(&[first, second]) = edges.get(&(a.min(b), a.max(b))).map(Vec::as_slice) else {
                continue;
            };
            let (one, other) = (&faces[first as usize], &faces[second as usize]);
            let turn = review_model::exact::acos(one.normal.dot(other.normal).clamp(-1.0, 1.0))
                .to_degrees();
            // Convex when the second face bends away from the first's normal.
            let convex = (other.centre - one.centre).dot(one.normal) <= 0.0;
            if !convex && !params.concave {
                continue;
            }
            let ramp = ((turn - params.angle_start) / span).clamp(0.0, 1.0);
            let value = review_model::exact::powf(ramp, params.falloff.max(0.01));
            for point in [a, b] {
                let slot = &mut wear[point as usize];
                *slot = slot.max(value);
            }
        }
    }

    for (piece, own) in pieces.iter_mut().zip(&point_of) {
        for (vertex, &point) in piece.vertices.iter_mut().zip(own) {
            crate::ao::write_target(
                &mut vertex.vertex_color,
                wear[point as usize],
                params.target,
                params.srgb,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use review_model::Vertex;

    use super::*;

    /// A unit cube's six faces as one piece, every face's corners its own.
    fn cube() -> Submesh {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (axis, sign) in [
            (0, 1.0),
            (0, -1.0),
            (1, 1.0),
            (1, -1.0),
            (2, 1.0),
            (2, -1.0),
        ] {
            let normal =
                Vec3::from_array(std::array::from_fn(|k| if k == axis { sign } else { 0.0 }));
            let (u, v) = (
                normal.any_orthonormal_vector(),
                normal.cross(normal.any_orthonormal_vector()),
            );
            let base = vertices.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                vertices.push(Vertex {
                    position: normal + u * a + v * b,
                    normal,
                    ..Vertex::default()
                });
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Submesh {
            node: 0,
            material: crate::submesh::NO_MATERIAL,
            vertices,
            uv_channels: Vec::new(),
            color_channels: Vec::new(),
            vertex_crease: Vec::new(),
            indices,
            polygons: None,
            skin: Default::default(),
            extra_skins: Vec::new(),
            dq_weight: Default::default(),
            morph: Default::default(),
            custom: Default::default(),
            source_corner: Vec::new(),
            normals_stale: false,
        }
    }

    #[test]
    fn a_cube_wears_fully_at_its_corners_and_not_across_its_faces() {
        let mut piece = cube();
        bake_node(&mut [&mut piece], &BakeWearParams::default());
        // Every corner of a cube is on a 90-degree convex edge.
        assert!(
            piece
                .vertices
                .iter()
                .all(|vertex| vertex.vertex_color.z == 1.0)
        );

        // A flat grid wears nothing: its diagonals are no edges.
        let mut flat = cube();
        flat.indices.truncate(6);
        for vertex in &mut flat.vertices {
            vertex.vertex_color = glam::Vec4::ONE;
        }
        bake_node(&mut [&mut flat], &BakeWearParams::default());
        assert!(
            flat.vertices[..4]
                .iter()
                .all(|vertex| vertex.vertex_color.z == 0.0)
        );
    }

    #[test]
    fn concave_edges_wear_only_when_asked() {
        // The cube turned inside out: every edge is concave.
        let mut inside = cube();
        for triangle in inside.indices.as_chunks_mut::<3>().0 {
            triangle.swap(1, 2);
        }
        bake_node(&mut [&mut inside], &BakeWearParams::default());
        assert!(
            inside
                .vertices
                .iter()
                .all(|vertex| vertex.vertex_color.z == 0.0)
        );
        let concave = BakeWearParams {
            concave: true,
            ..BakeWearParams::default()
        };
        bake_node(&mut [&mut inside], &concave);
        assert!(
            inside
                .vertices
                .iter()
                .all(|vertex| vertex.vertex_color.z == 1.0)
        );
    }
}
