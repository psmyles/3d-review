//! The built-in cube: what the viewer shows before anything is loaded, and the
//! fixture most of this crate's tests are written against.

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::*;

pub fn demo_cube_model() -> ModelData {
    let mut model = ModelData {
        name: "Demo Cube".to_owned(),
        vertices: demo_cube_vertices(),
        indices: demo_cube_indices(),
        faces: (0..6)
            .map(|face_index| TopologyFace {
                first_index: face_index * 4,
                index_count: 4,
            })
            .collect(),
        triangles: TriangleData {
            to_face: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            material: vec![0; 12],
            // The demo cube is a single node, so every triangle belongs to node 0.
            node: vec![0; 12],
        },
        nodes: vec![SceneNode {
            name: "Demo Cube".to_owned(),
            parent: None,
            mesh_part: Some(0),
            // The whole cube is this one node, so its share is the model's own
            // `vertex_count` below.
            source_vertex_count: 24,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Mesh,
            bone: None,
        }],
        uv_set_names: vec!["UVMap".to_owned()],
        stats: ModelStats {
            polygon_count: 6,
            triangle_count: 12,
            vertex_count: 24,
            // Overwritten below by the measured count (each flat-shaded corner
            // is genuinely unique, so it stays 24).
            gpu_vertex_count: 0,
            uv_set_count: 1,
            material_count: 1,
            draw_count: 1,
            bone_count: 0,
            clip_count: 0,
            source_unit_meters: 1.0,
        },
        materials: vec![MaterialImportDefaults {
            name: "Default".to_owned(),
            base_color: Vec3::ONE,
            smoothness: 0.6,
            metallic: 0.0,
            emissive: Vec3::ZERO,
        }],
        ..Default::default()
    };
    model.recompute_bounds();
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
    model
}

fn demo_cube_vertices() -> Vec<Vertex> {
    let s = 0.5;
    let y0 = 0.03;
    let y1 = 1.03;

    let faces = [
        ([[-s, y0, s], [s, y0, s], [s, y1, s], [-s, y1, s]], Vec3::Z),
        (
            [[s, y0, -s], [-s, y0, -s], [-s, y1, -s], [s, y1, -s]],
            Vec3::NEG_Z,
        ),
        (
            [[-s, y0, -s], [-s, y0, s], [-s, y1, s], [-s, y1, -s]],
            Vec3::NEG_X,
        ),
        ([[s, y0, s], [s, y0, -s], [s, y1, -s], [s, y1, s]], Vec3::X),
        (
            [[-s, y1, s], [s, y1, s], [s, y1, -s], [-s, y1, -s]],
            Vec3::Y,
        ),
        (
            [[-s, y0, -s], [s, y0, -s], [s, y0, s], [-s, y0, s]],
            Vec3::NEG_Y,
        ),
    ];

    let uvs = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];

    // Distinct per-corner vertex colors so the vertex-color debug view has
    // something to show on the built-in demo: an R/G/B/yellow ring with a
    // 0.25→1.0 alpha ramp to exercise the alpha / RGB+A modes too.
    let vertex_colors = [
        Vec4::new(1.0, 0.0, 0.0, 0.25),
        Vec4::new(0.0, 1.0, 0.0, 0.50),
        Vec4::new(0.0, 0.0, 1.0, 0.75),
        Vec4::new(1.0, 1.0, 0.0, 1.00),
    ];

    let mut vertices = Vec::with_capacity(24);
    for (positions, normal) in faces {
        for (index, position) in positions.into_iter().enumerate() {
            vertices.push(Vertex {
                position: Vec3::from_array(position),
                normal,
                uv: uvs[index],
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                vertex_color: vertex_colors[index],
            });
        }
    }

    vertices
}

fn demo_cube_indices() -> Vec<u32> {
    let mut indices = Vec::with_capacity(36);
    for face_index in 0..6 {
        let base = face_index * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_cube_carries_nodes_and_per_triangle_material() {
        let model = demo_cube_model();

        let tris = &model.triangles;
        assert!(!model.nodes.is_empty());
        assert_eq!(tris.material.len(), model.stats.triangle_count);
        assert_eq!(tris.material.len(), tris.to_face.len());
        assert!(tris.material.iter().all(|&slot| slot == 0));
        // Per-triangle node index runs parallel and points at the single node.
        assert_eq!(tris.node.len(), model.stats.triangle_count);
        assert!(tris.node.iter().all(|&node| node == 0));

        // The parallel per-triangle arrays stay mutually in lockstep, and every
        // index they carry points at a real face / material / node.
        assert!(
            tris.validate(
                model.stats.triangle_count,
                model.faces.len(),
                model.materials.len(),
                model.nodes.len(),
            )
            .is_ok()
        );
    }

    #[test]
    fn demo_cube_node_is_typed_as_a_mesh() {
        let model = demo_cube_model();
        assert_eq!(model.nodes[0].kind, NodeKind::Mesh);
        assert!(model.nodes[0].bone.is_none());
        assert!(model.skin.is_none());
        assert_eq!(model.stats.bone_count, 0);
    }
}
