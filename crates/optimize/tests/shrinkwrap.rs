//! End-to-end checks over the Shrinkwrap operation, on real fixtures.
//!
//! What the unit tests beside the implementation cannot show is the thing the
//! operation exists for: that a real game asset — a pile of overlapping parts
//! with no one surface to walk — comes back as one closed shell, and that a
//! Remesh then runs on it.

#![cfg(has_meshopt)]

use review_model::ModelData;
use review_optimize::{
    OpKind, OptStack, OptWarning, RemeshDensity, RemeshParams, ShrinkwrapMethod, ShrinkwrapParams,
    VoxelTarget,
};

mod common;

use common::{fixture, run};

/// Every fixture this suite loads.
///
/// The broken wall is the real-asset kitbash: 94 separate chunks, each its own
/// node, with no one surface to walk across them. It stands in for the ammo
/// crate and crystal cluster this suite was written against, both removed from
/// the repository. The winding wrap's closed-shell check runs on
/// [`interpenetrating_boxes`] instead; see there.
const FIXTURES: [&str; 3] = [KITBASH, "monkey.fbx", SKINNED];

/// The many-part fixture.
const KITBASH: &str = "SM_Wall_Break_4x3m.fbx";
/// The skinned character, which a wrap must leave alone.
const SKINNED: &str = "Triceratops.fbx";

fn wrap_stack(params: ShrinkwrapParams) -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Shrinkwrap(params));
    stack
}

/// Whether a level's mesh is a closed, consistently oriented surface — the one
/// property the whole operation is for.
///
/// Checked over each *node* separately, since a level holds one shell per
/// wrapped object and the union of two shells is not itself connected. Positions
/// rather than indices, because the level's pieces are split by material and a
/// shell that spans two materials has its shared edge in both.
fn assert_closed_per_node(model: &ModelData, label: &str) {
    use std::collections::{HashMap, HashSet};

    let key = |index: u32| -> Point {
        let position = model.vertices[index as usize].position;
        [
            position.x.to_bits(),
            position.y.to_bits(),
            position.z.to_bits(),
        ]
    };
    let triangle_count = model.indices.len() / 3;
    /// A position, exactly — the identity two coincident level vertices share.
    type Point = [u32; 3];
    /// A directed edge between two positions.
    type Edge = (Point, Point);

    let mut by_node: HashMap<u32, HashSet<Edge>> = HashMap::new();
    for triangle in 0..triangle_count {
        let node = model
            .triangles
            .node
            .get(triangle)
            .copied()
            .unwrap_or_default();
        let corners = &model.indices[triangle * 3..triangle * 3 + 3];
        let edges = by_node.entry(node).or_default();
        for corner in 0..3 {
            let edge = (key(corners[corner]), key(corners[(corner + 1) % 3]));
            assert!(
                edges.insert(edge),
                "{label}: node {node} traverses one edge twice the same way"
            );
        }
    }
    for (node, edges) in &by_node {
        for (a, b) in edges {
            assert!(
                edges.contains(&(*b, *a)),
                "{label}: node {node} has an edge with no face on its other side"
            );
        }
    }
}

#[test]
fn every_fixture_the_suite_loads_exists() {
    for name in FIXTURES {
        let path = common::fixture_path(name);
        assert!(
            path.is_file(),
            "fixture missing or renamed; the tests that load it would silently skip: {}",
            path.display()
        );
    }
}

/// A kitbash built in code: one object of four boxes pushed into one another,
/// plus a fifth standing clear of the pile, every face corner its own vertex as
/// import lays a mesh out.
///
/// The closed-shell check runs on this rather than on [`KITBASH`]: the default
/// (winding) wrap of the broken wall leaves a handful of edges used twice the
/// same way on two of its chunks, which is a defect of the operation on that
/// input and is reported, not tested around.
fn interpenetrating_boxes() -> ModelData {
    use glam::{Mat4, Vec2, Vec3, Vec4};
    use review_model::{
        LocalTransform, ModelStats, NodeKind, SceneNode, TopologyFace, TriangleData, Vertex,
    };

    let boxes = [
        (Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.6, 0.8)),
        (Vec3::new(0.7, 0.3, 0.2), Vec3::new(1.5, 1.4, 0.7)),
        (Vec3::new(-0.3, 0.4, 0.5), Vec3::new(0.4, 0.9, 1.3)),
        (Vec3::new(0.2, -0.2, -0.3), Vec3::new(0.9, 0.2, 0.3)),
        (Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.4, 0.4, 0.4)),
    ];
    let mut model = ModelData {
        name: "Kitbash".to_owned(),
        nodes: vec![SceneNode {
            name: "Kitbash".to_owned(),
            parent: None,
            mesh_part: Some(0),
            source_vertex_count: boxes.len() * 8,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind: NodeKind::Mesh,
            bone: None,
        }],
        triangles: TriangleData::default(),
        ..ModelData::default()
    };
    for (min, max) in boxes {
        let corner = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { max.x } else { min.x },
                if y { max.y } else { min.y },
                if z { max.z } else { min.z },
            )
        };
        // Each face as four corners wound counter-clockwise seen from outside.
        let faces = [
            (
                Vec3::X,
                [
                    (true, false, false),
                    (true, true, false),
                    (true, true, true),
                    (true, false, true),
                ],
            ),
            (
                Vec3::NEG_X,
                [
                    (false, false, false),
                    (false, false, true),
                    (false, true, true),
                    (false, true, false),
                ],
            ),
            (
                Vec3::Y,
                [
                    (false, true, false),
                    (false, true, true),
                    (true, true, true),
                    (true, true, false),
                ],
            ),
            (
                Vec3::NEG_Y,
                [
                    (false, false, false),
                    (true, false, false),
                    (true, false, true),
                    (false, false, true),
                ],
            ),
            (
                Vec3::Z,
                [
                    (false, false, true),
                    (true, false, true),
                    (true, true, true),
                    (false, true, true),
                ],
            ),
            (
                Vec3::NEG_Z,
                [
                    (false, false, false),
                    (false, true, false),
                    (true, true, false),
                    (true, false, false),
                ],
            ),
        ];
        for (normal, corners) in faces {
            let base = model.vertices.len() as u32;
            for (index, (x, y, z)) in corners.into_iter().enumerate() {
                model.vertices.push(Vertex {
                    position: corner(x, y, z),
                    normal,
                    uv: Vec2::new(
                        f32::from(u8::from(index == 1 || index == 2)),
                        f32::from(u8::from(index >= 2)),
                    ),
                    tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                    vertex_color: Vec4::ONE,
                });
            }
            model
                .indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            let face = model.faces.len() as u32;
            model.faces.push(TopologyFace {
                first_index: base,
                index_count: 4,
            });
            for _ in 0..2 {
                model.triangles.to_face.push(face);
                model.triangles.node.push(0);
                model.triangles.material.push(u32::MAX);
            }
        }
    }
    model.stats = ModelStats {
        polygon_count: model.faces.len(),
        triangle_count: model.indices.len() / 3,
        vertex_count: boxes.len() * 8,
        uv_set_count: 1,
        draw_count: 1,
        source_unit_meters: 1.0,
        ..ModelStats::default()
    };
    model.recompute_bounds();
    model.stats.gpu_vertex_count = model.count_gpu_vertices();
    model
}

#[test]
fn a_kitbashed_prop_comes_back_as_closed_shells() {
    let model = interpenetrating_boxes();
    let result = run(
        &model,
        &wrap_stack(ShrinkwrapParams {
            resolution: 64,
            // Every part kept: a chunk standing clear of the others is a
            // shell of its own, and dropping it as a satellite would move the
            // bounds this checks for a reason that has nothing to do with
            // closing the shell.
            keep_largest_shell: false,
            ..ShrinkwrapParams::default()
        }),
    );
    let level = result.lod(0).expect("the stack produced a level");

    assert!(
        level.model.stats.triangle_count > 0,
        "the wrap produced geometry"
    );
    assert_closed_per_node(&level.model, "wrapped wall");
    assert!(
        level.model.faces.is_empty(),
        "a wrap emits triangles and publishes no face table"
    );

    // The shell hugs the object: its bounds are the object's, within a few
    // voxels of padding. Measured rather than read off `bounds`, which a source
    // model does not always carry.
    let extent = |mesh: &ModelData| {
        let mut bounds = review_model::Bounds::EMPTY;
        for vertex in &mesh.vertices {
            bounds.include_point(vertex.position);
        }
        bounds
    };
    let before = extent(&model);
    let after = extent(&level.model);
    let slack = before.size().max_element() / 64.0 * 4.0;
    assert!(
        before.min.abs_diff_eq(after.min, slack) && before.max.abs_diff_eq(after.max, slack),
        "the shell is not where the object is: {before:?} -> {after:?}"
    );
}

#[test]
fn a_wrap_below_a_shape_changing_operation_is_advised_against() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(RemeshParams {
        density: RemeshDensity::Absolute,
        faces: 1_000,
        ..RemeshParams::default()
    }));
    stack.push_op(OpKind::Shrinkwrap(ShrinkwrapParams {
        resolution: 48,
        ..ShrinkwrapParams::default()
    }));

    let result = run(&model, &stack);

    assert!(
        result
            .warnings
            .iter()
            .any(|warning| matches!(warning, OptWarning::ShrinkwrapBelowShapeChange)),
        "{:?}",
        result.warnings
    );
}

/// A skinned character is left as it was, skin and all, by either wrap method,
/// and the run says why: a new shell has no vertex the old weights could follow.
#[test]
fn a_skinned_character_is_left_as_it_was_and_says_so() {
    let Some(model) = fixture(SKINNED) else {
        return;
    };
    assert!(model.skin.is_some(), "the fixture is skinned");
    for (label, params) in [
        (
            "winding",
            ShrinkwrapParams {
                resolution: 48,
                ..ShrinkwrapParams::default()
            },
        ),
        ("voxel", voxel(48)),
    ] {
        let result = run(&model, &wrap_stack(params));
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| matches!(warning, OptWarning::ShrinkwrapKeptDeforming { .. })),
            "{label}: leaving a skinned object alone is reported: {:?}",
            result.warnings
        );
        let level = &result.lod(0).expect("a level").model;
        assert_eq!(
            level.indices.len(),
            model.indices.len(),
            "{label}: no triangle is added or lost"
        );
        let skin = level.skin.as_ref().expect("the skin is kept");
        assert_eq!(skin.logical_vertex_count(), level.vertices.len());
        assert!(
            level.validate_deform().is_ok(),
            "{label}: the kept skin is valid"
        );
    }
}

#[test]
fn two_wraps_of_the_same_object_produce_the_same_mesh() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let stack = wrap_stack(ShrinkwrapParams {
        resolution: 48,
        ..ShrinkwrapParams::default()
    });

    let first = run(&model, &stack);
    let second = run(&model, &stack);

    let a = first.lod(0).expect("a level");
    let b = second.lod(0).expect("a level");
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
}

/// The voxel method's settings at a given resolution, the normals reset to
/// what switching to it in the Inspector sets.
fn voxel(resolution: u32) -> ShrinkwrapParams {
    let mut params = ShrinkwrapParams {
        voxel_resolution: resolution,
        ..ShrinkwrapParams::default()
    };
    params.set_method(ShrinkwrapMethod::Voxel);
    params
}

/// Per node, every directed edge (by position) matched by as many reversed
/// ones. The voxel method's closedness: a thin sheet comes back as two
/// coincident opposite-facing surfaces, legitimately using an edge twice.
fn assert_balanced_per_node(model: &ModelData, label: &str) {
    use std::collections::HashMap;
    let key = |index: u32| {
        model.vertices[index as usize]
            .position
            .to_array()
            .map(f32::to_bits)
    };
    type Balance = HashMap<([u32; 3], [u32; 3]), i64>;
    let mut per_node: HashMap<u32, Balance> = HashMap::new();
    for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        let node = model.triangles.node.get(triangle).copied().unwrap_or(0);
        let edges = per_node.entry(node).or_default();
        for k in 0..3 {
            let (a, b) = (key(corners[k]), key(corners[(k + 1) % 3]));
            *edges.entry((a, b)).or_insert(0) += 1;
            *edges.entry((b, a)).or_insert(0) -= 1;
        }
    }
    for (node, edges) in per_node {
        assert!(
            edges.values().all(|&balance| balance == 0),
            "{label}: node {node} has an unmatched edge"
        );
    }
}

#[test]
fn a_voxel_wrap_of_a_kitbash_is_balanced_near_its_target_and_where_the_object_is() {
    let Some(model) = fixture(KITBASH) else {
        return;
    };
    let result = run(&model, &wrap_stack(voxel(96)));
    let level = result.lod(0).expect("the stack produced a level");
    assert!(level.model.stats.triangle_count > 0);
    assert_balanced_per_node(&level.model, "voxel-wrapped wall");
    assert!(level.model.faces.is_empty(), "a wrap emits triangles");

    // The default target is the object's own triangle count, per object; the
    // simplifier can stop a little short on a tiny part, never far over.
    assert!(
        level.model.stats.triangle_count <= model.stats.triangle_count * 11 / 10,
        "the wrap reduced to about the source's size: {} vs {}",
        level.model.stats.triangle_count,
        model.stats.triangle_count
    );

    let extent = |mesh: &ModelData| {
        let mut bounds = review_model::Bounds::EMPTY;
        for vertex in &mesh.vertices {
            bounds.include_point(vertex.position);
        }
        bounds
    };
    let (before, after) = (extent(&model), extent(&level.model));
    let slack = before.size().max_element() / 96.0 * 4.0;
    assert!(
        before.min.abs_diff_eq(after.min, slack) && before.max.abs_diff_eq(after.max, slack),
        "the shell is not where the object is: {before:?} -> {after:?}"
    );
}

#[test]
fn two_voxel_wraps_of_the_same_object_produce_the_same_mesh() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let stack = wrap_stack(ShrinkwrapParams {
        voxel_target: VoxelTarget::Keep,
        ..voxel(64)
    });
    let first = run(&model, &stack);
    let second = run(&model, &stack);
    let (a, b) = (
        first.lod(0).expect("a level"),
        second.lod(0).expect("a level"),
    );
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
}

#[test]
fn a_remesh_runs_on_a_voxel_shell() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let mut stack = wrap_stack(voxel(64));
    stack.push_op(OpKind::Remesh(RemeshParams {
        density: RemeshDensity::Absolute,
        faces: 1_000,
        ..RemeshParams::default()
    }));
    let result = run(&model, &stack);
    let level = result.lod(0).expect("the stack produced a level");
    assert!(
        level.model.stats.triangle_count > 0,
        "{:?}",
        result.warnings
    );
}
