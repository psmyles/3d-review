//! Synthetic subjects for the Remesh suites, standing in for the shape kinds the
//! proprietary fixtures used to supply.
//!
//! Most of what those suites pin is a property of a *kind* of surface rather
//! than of one asset: a thin closed shell whose silhouette is a fold (a leaf), a
//! chunky closed object with nothing to lose (a stone), a broad flat face with a
//! rounded rim (a pedestal), several objects with several materials. The three
//! fixtures that remain - a cube, Suzanne and a wall - have none of those, so
//! the suites build them here instead, analytically and deterministically.
//!
//! Every shape is an ellipsoid with exact single-vertex poles: a pole ring of
//! coincident vertices would be a fan of zero-area slivers, which is a fault in
//! the input rather than anything the rebuild should be judged on. Vertices are
//! shared within an object and normals are the ellipsoid's own, so the import's
//! corner split is not reproduced - `process` indexes either layout to the same
//! mesh.

#![allow(dead_code)]

use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
use review_model::{
    LocalTransform, MaterialImportDefaults, ModelData, ModelStats, NodeKind, SceneNode,
    TriangleData, Vertex,
};

/// One object of a synthetic model.
pub struct Object {
    pub name: &'static str,
    /// Radii along the object's own axes before `rotation`.
    pub radii: Vec3,
    pub rotation: Quat,
    pub center: Vec3,
    /// Latitude bands, pole to pole.
    pub rings: u32,
    /// Longitude steps around.
    pub segments: u32,
    /// Material for the upper half (`y >= 0` in the object's own frame) and
    /// the lower half. The same index twice gives a one-material object.
    pub materials: [u32; 2],
}

/// Assemble objects into one model, one node per object.
pub fn model(name: &str, objects: &[Object], material_count: usize) -> ModelData {
    let mut model = ModelData {
        name: name.to_owned(),
        uv_set_names: vec!["UVMap".to_owned()],
        materials: (0..material_count)
            .map(|at| MaterialImportDefaults {
                name: format!("Material{at}"),
                base_color: Vec3::ONE,
                smoothness: 0.5,
                metallic: 0.0,
                emissive: Vec3::ZERO,
                emission_factor: 1.0,
                maps: [None; review_model::MaterialMap::COUNT],
            })
            .collect(),
        ..Default::default()
    };
    for (node, object) in objects.iter().enumerate() {
        let first = model.vertices.len() as u32;
        let rotate = Mat3::from_quat(object.rotation);
        let (rings, segments) = (object.rings.max(2), object.segments.max(3));
        // The poles, then each interior ring of `segments` vertices.
        let mut push = |theta: f32, phi: f32| {
            let unit = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let local = unit * object.radii;
            // An ellipsoid's normal is the gradient of (x/a)^2 + ... = 1.
            let normal = rotate * (local / (object.radii * object.radii)).normalize();
            model.vertices.push(Vertex {
                position: object.center + rotate * local,
                normal,
                uv: Vec2::new(phi / std::f32::consts::TAU, theta / std::f32::consts::PI),
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                vertex_color: Vec4::ONE,
            });
        };
        push(0.0, 0.0);
        for ring in 1..rings {
            let theta = std::f32::consts::PI * ring as f32 / rings as f32;
            for step in 0..segments {
                push(theta, std::f32::consts::TAU * step as f32 / segments as f32);
            }
        }
        push(std::f32::consts::PI, 0.0);
        let north = first;
        let south = model.vertices.len() as u32 - 1;
        let at = |ring: u32, step: u32| first + 1 + (ring - 1) * segments + step % segments;

        let triangle = |model: &mut ModelData, corners: [u32; 3], upper: bool| {
            model.indices.extend_from_slice(&corners);
            model
                .triangles
                .material
                .push(object.materials[usize::from(!upper)]);
            model.triangles.node.push(node as u32);
        };
        // Wound counter-clockwise seen from outside, so the geometric normal
        // agrees with the stored one.
        for step in 0..segments {
            triangle(&mut model, [north, at(1, step + 1), at(1, step)], true);
        }
        for ring in 1..rings - 1 {
            let upper = ring < rings / 2;
            for step in 0..segments {
                let (a, b) = (at(ring, step), at(ring, step + 1));
                let (c, d) = (at(ring + 1, step), at(ring + 1, step + 1));
                triangle(&mut model, [a, b, d], upper);
                triangle(&mut model, [a, d, c], upper);
            }
        }
        for step in 0..segments {
            triangle(
                &mut model,
                [south, at(rings - 1, step), at(rings - 1, step + 1)],
                false,
            );
        }
        model.nodes.push(SceneNode {
            name: object.name.to_owned(),
            parent: None,
            mesh_part: Some(node),
            source_vertex_count: model.vertices.len() - first as usize,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Mesh,
            bone: None,
        });
    }
    let triangles = model.indices.len() / 3;
    model.triangles = TriangleData {
        to_face: Vec::new(),
        ..std::mem::take(&mut model.triangles)
    };
    model.stats = ModelStats {
        polygon_count: triangles,
        triangle_count: triangles,
        vertex_count: model.vertices.len(),
        gpu_vertex_count: model.vertices.len(),
        uv_set_count: 1,
        material_count,
        draw_count: material_count.max(1),
        bone_count: 0,
        clip_count: 0,
        source_unit_meters: 1.0,
        source_axes: None,
    };
    model.recompute_bounds();
    model
}

/// A plant's worth of leaves: six thin closed shells, each its own object, set
/// apart and turned every which way.
///
/// A leaf is an ellipsoid 2 % as thick as it is long, so its edge is a rim
/// where the surface turns through 180 degrees over a few rings rather than at
/// one edge - the bevelled silhouette a real leaf or strip of bark has, and the
/// case the shape measures exist for. Two materials each, split across the
/// shell, so the material seams run over the rim.
pub fn leaves(rings: u32, segments: u32) -> ModelData {
    let objects: Vec<Object> = (0..6u32)
        .map(|at| {
            let angle = at as f32 * 1.1;
            Object {
                name: ["LeafA", "LeafB", "LeafC", "LeafD", "LeafE", "LeafF"][at as usize],
                radii: Vec3::new(1.0, 0.02, 0.35),
                rotation: Quat::from_euler(glam::EulerRot::YXZ, angle, 0.3 * at as f32, 0.5),
                center: Vec3::new((at % 3) as f32 * 2.5, (at / 3) as f32 * 2.5, 0.0),
                rings,
                segments,
                materials: [0, 1],
            }
        })
        .collect();
    model("leaves", &objects, 2)
}

/// A broad pedestal with a handful of stones beside it: closed, chunky,
/// nothing touching anything else.
///
/// The pedestal (node 0) is a slab - one broad flat face with a rounded rim -
/// which is what varying the face size is for: big faces on the flat, small
/// ones round the rim.
pub fn pedestal_and_stones(rings: u32, segments: u32) -> ModelData {
    let mut objects = vec![Object {
        name: "Pedestal",
        radii: Vec3::new(3.0, 0.35, 3.0),
        rotation: Quat::IDENTITY,
        center: Vec3::ZERO,
        rings: rings * 2,
        segments: segments * 2,
        materials: [0, 1],
    }];
    for at in 0..4u32 {
        let angle = at as f32 * std::f32::consts::FRAC_PI_2;
        objects.push(Object {
            name: ["StoneA", "StoneB", "StoneC", "StoneD"][at as usize],
            radii: Vec3::new(0.6 + 0.1 * at as f32, 0.35, 0.45),
            rotation: Quat::from_euler(glam::EulerRot::YXZ, angle * 0.7, 0.2, 0.1 * at as f32),
            center: Vec3::new(angle.cos() * 4.5, 0.0, angle.sin() * 4.5),
            rings,
            segments,
            materials: [1, 1],
        });
    }
    model("pedestal and stones", &objects, 2)
}
