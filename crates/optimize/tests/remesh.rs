//! End-to-end checks over the Remesh operation, on real fixtures.
//!
//! The unit tests beside the implementation cover the pieces in isolation — the
//! density split, the corner-run expansion, the projection's seam behaviour.
//! These run the whole thing: a real asset in, a rebuilt quad mesh out, through
//! the same `process` the workspace calls.
//!
//! Each test skips itself when its fixture or a vendored tree is absent, so a
//! checkout without them still passes. `REVIEW_REQUIRE_FIXTURES=1` turns every
//! skip into a failure.

#![cfg(all(has_meshopt, has_instant_meshes))]

use review_model::ModelData;
use review_optimize::{
    NodeOverride, OpKind, OptStack, ProcessedLod, RemeshDensity, RemeshParams, RemeshTopology,
    WeldParams,
};

mod common;

use common::{fixture, run};

/// Every fixture this suite loads.
const FIXTURES: [&str; 2] = ["monkey.fbx", "SM_column04.fbx"];

fn remesh_stack(params: RemeshParams) -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Remesh(params));
    stack
}

fn quad_params(faces: u32) -> RemeshParams {
    RemeshParams {
        topology: RemeshTopology::QuadDominant,
        density: RemeshDensity::Absolute,
        faces,
        ..RemeshParams::default()
    }
}

/// How many of a level's polygons have each corner count.
fn face_degrees(level: &ProcessedLod) -> (usize, usize, usize) {
    let mut triangles = 0;
    let mut quads = 0;
    let mut other = 0;
    for face in &level.model.faces {
        match face.index_count {
            3 => triangles += 1,
            4 => quads += 1,
            _ => other += 1,
        }
    }
    (triangles, quads, other)
}

/// The structural guarantees a rebuilt level must satisfy on top of the ones
/// every processed mesh does.
fn assert_consistent(model: &ModelData, label: &str) {
    let triangle_count = model.indices.len() / 3;
    assert_eq!(
        model.indices.len() % 3,
        0,
        "{label}: the index buffer is not whole triangles"
    );
    for &index in &model.indices {
        assert!(
            (index as usize) < model.vertices.len(),
            "{label}: index {index} addresses no vertex of {}",
            model.vertices.len()
        );
    }
    assert!(
        model
            .triangles
            .validate(
                triangle_count,
                model.faces.len(),
                model.materials.len(),
                model.nodes.len()
            )
            .is_ok(),
        "{label}: the per-triangle tags drifted: {:?}",
        model.triangles.validate(
            triangle_count,
            model.faces.len(),
            model.materials.len(),
            model.nodes.len()
        )
    );
    // Corner runs: every face owns a contiguous, in-range slice of the vertex
    // array, and every triangle's corners lie inside its own face's run.
    for (index, face) in model.faces.iter().enumerate() {
        assert!(face.index_count >= 3, "{label}: face {index} has no corners");
        let end = face.first_index as usize + face.index_count as usize;
        assert!(
            end <= model.vertices.len(),
            "{label}: face {index} runs past the vertex array"
        );
    }
    for triangle in 0..triangle_count {
        let face = model.faces[model.triangles.to_face[triangle] as usize];
        for corner in 0..3 {
            let index = model.indices[triangle * 3 + corner];
            assert!(
                index >= face.first_index && index < face.first_index + face.index_count,
                "{label}: triangle {triangle} reaches outside its own face's run"
            );
        }
    }
    for vertex in &model.vertices {
        assert!(
            vertex.position.is_finite(),
            "{label}: a rebuilt vertex is not at a finite position"
        );
        assert!(
            vertex.normal.is_finite() && vertex.normal.length() > 0.5,
            "{label}: a rebuilt vertex has no usable normal"
        );
    }
}

fn assert_no_skip_warning(result: &review_optimize::ProcessedResult) {
    for warning in &result.warnings {
        assert!(
            !warning.contains("left as it is"),
            "the remesh skipped an object: {warning}"
        );
    }
}

#[test]
fn every_fixture_the_suite_loads_exists() {
    for name in FIXTURES {
        let path = common::fixture_path(name);
        if !path.exists() {
            common::skip(&format!("{name} is not present"));
            return;
        }
    }
}

#[test]
fn mostly_quads_hits_its_face_budget_and_is_mostly_quads() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let target = 2_000u32;
    let result = run(&model, &remesh_stack(quad_params(target)));
    assert_no_skip_warning(&result);

    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "monkey, mostly quads");

    let polygons = level.model.stats.polygon_count;
    let (triangles, quads, other) = face_degrees(level);
    assert_eq!(other, 0, "a field extraction emits only triangles and quads");
    assert_eq!(
        triangles + quads,
        polygons,
        "the stats' polygon count is the face table's length"
    );
    // The engine targets a face *area* rather than a count, and overshoots it
    // by a consistent fifth or so; measured here at 2460 for a 2000 target.
    let error = (polygons as f32 - target as f32).abs() / target as f32;
    assert!(
        error <= 0.3,
        "asked for {target} faces and got {polygons}, which is {:.0}% off",
        error * 100.0
    );
    // Quad share rises with density — the triangles are the field's
    // singularities, and there are about as many of them whatever the target.
    // Measured on this fixture: 67% at 500 faces, 75% at 2000, 83% at 8000.
    // Suzanne is a hard case (creases, open eyes, one non-manifold edge); the
    // game-asset fixtures sit higher.
    let quad_share = quads as f32 / polygons as f32;
    assert!(
        quad_share >= 0.7,
        "only {:.0}% of the faces are quads",
        quad_share * 100.0
    );
    assert!(
        level.model.stats.triangle_count > polygons,
        "a quad mesh has more triangles than polygons"
    );
}

#[test]
fn triangles_topology_produces_no_polygons_beyond_triangles() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let params = RemeshParams {
        topology: RemeshTopology::Triangles,
        ..quad_params(2_000)
    };
    let result = run(&model, &remesh_stack(params));
    assert_no_skip_warning(&result);

    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "monkey, triangles");
    let (_, quads, other) = face_degrees(level);
    assert_eq!((quads, other), (0, 0));
    assert_eq!(
        level.model.stats.polygon_count, level.model.stats.triangle_count,
        "an all-triangle mesh counts one polygon per triangle"
    );
}

#[test]
fn every_rebuilt_face_carries_a_material_the_source_authored() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    let result = run(&model, &remesh_stack(quad_params(3_000)));
    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "column, mostly quads");

    let source_materials = model.materials.len();
    assert!(source_materials > 0, "the fixture has materials to keep");
    for &material in &level.model.triangles.material {
        assert!(
            material == u32::MAX || (material as usize) < source_materials,
            "a rebuilt triangle carries material {material}, which the source does not have"
        );
    }
    // The projection must find *some* real material for the surface, not leave
    // the whole mesh on the no-material sentinel.
    assert!(
        level
            .model
            .triangles
            .material
            .iter()
            .any(|&material| material != u32::MAX),
        "the rebuilt mesh lost every material"
    );
}

#[test]
fn two_deterministic_runs_produce_the_same_mesh() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let stack = remesh_stack(quad_params(1_500));

    let first = run(&model, &stack);
    let second = run(&model, &stack);

    let a = first.lod(0).expect("a level");
    let b = second.lod(0).expect("a level");
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
    assert_eq!(a.model.faces, b.model.faces, "face tables differ");
    assert_eq!(a.model.triangles, b.model.triangles, "triangle tags differ");
}

#[test]
fn an_excluded_object_is_left_exactly_as_it_was() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    // Exclude every node: the whole mesh must come back untouched, which is the
    // only exclusion assertion that does not depend on which node is which.
    let mut stack = remesh_stack(quad_params(500));
    stack.overrides = (0..model.nodes.len())
        .map(|node| NodeOverride {
            node,
            exclude: true,
            ..NodeOverride::default()
        })
        .collect();

    let result = run(&model, &stack);
    let level = result.lod(0).expect("the stack produced a level");

    assert!(
        level.model.faces.is_empty(),
        "nothing was rebuilt, so the level carries no face table"
    );
    assert_eq!(
        level.model.stats.triangle_count,
        model.indices.len() / 3,
        "an excluded object keeps every triangle"
    );
}

#[test]
fn a_weld_below_a_remesh_keeps_the_quads() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let mut stack = remesh_stack(quad_params(1_500));
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::VertexFetch);

    let result = run(&model, &stack);
    assert_no_skip_warning(&result);
    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "monkey, remesh + weld + fetch");

    let (_, quads, other) = face_degrees(level);
    assert_eq!(other, 0);
    assert!(
        quads as f32 / level.model.stats.polygon_count as f32 >= 0.7,
        "a weld and a vertex-fetch reorder below the remesh lost its quads"
    );
}

#[test]
fn a_remesh_reports_fewer_vertices_than_its_corner_run_holds() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &remesh_stack(quad_params(1_500)));
    let level = result.lod(0).expect("the stack produced a level");

    // Invariant 5: the panel shows the indexed count the export writes, never
    // the corner-run buffer's own length.
    assert!(
        level.model.stats.vertex_count < level.model.vertices.len(),
        "the corner-run buffer should be larger than the indexed mesh it holds"
    );
    assert!(
        level.model.stats.vertex_count > 0,
        "the indexed count is a real measurement"
    );
    // Rough sanity: a quad mesh has about as many vertices as quads.
    let ratio = level.model.stats.vertex_count as f32 / level.model.stats.polygon_count as f32;
    assert!(
        (0.5..2.0).contains(&ratio),
        "{} vertices for {} polygons is not a quad mesh",
        level.model.stats.vertex_count,
        level.model.stats.polygon_count
    );
}
