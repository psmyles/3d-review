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

use common::{compare_digest_across_binaries, fixture, geometry_digest, run};

/// Every fixture this suite loads.
const FIXTURES: [&str; 4] = [
    "monkey.fbx",
    "SM_column04.fbx",
    "SM_Ammo_Crate_01a.fbx",
    "cpg_pedestal_pebbles.fbx",
];

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
        assert!(
            face.index_count >= 3,
            "{label}: face {index} has no corners"
        );
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
    assert_eq!(
        other, 0,
        "a field extraction emits only triangles and quads"
    );
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

    // The same mesh has to come out at any thread count, which one process
    // cannot check on its own: `tests/remesh_single_thread.rs` runs this exact
    // stack with the pool forced to one thread, and whichever binary gets here
    // second compares the two.
    compare_digest_across_binaries("remesh_monkey_quads_1500", geometry_digest(a));
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
    // Rough sanity: a quad mesh has about as many vertices as quads. The ceiling
    // is generous because the ratio climbs as the rebuild gets coarser — the
    // attribute seams the projection splits at are a property of the *source*,
    // so their vertices stay while the interior's shrink. Measured on this
    // fixture, which is all seams: 2.1.
    let ratio = level.model.stats.vertex_count as f32 / level.model.stats.polygon_count as f32;
    assert!(
        (0.5..2.5).contains(&ratio),
        "{} vertices for {} polygons is not a quad mesh",
        level.model.stats.vertex_count,
        level.model.stats.polygon_count
    );
}

/// A closed single-shell asset: the quad solver runs and every face is a quad.
#[cfg(has_quadriflow)]
#[test]
fn only_quads_produces_nothing_but_quads_on_a_closed_shell() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    let target = 1_500u32;
    let params = RemeshParams {
        topology: RemeshTopology::PureQuads,
        ..quad_params(target)
    };
    let result = run(&model, &remesh_stack(params));
    for warning in &result.warnings {
        assert!(
            !warning.contains("fell back"),
            "this fixture is a closed shell, so the solver should run: {warning}"
        );
    }

    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "column, only quads");
    let (triangles, quads, other) = face_degrees(level);
    assert_eq!(
        (triangles, other),
        (0, 0),
        "'Only quads' means only quads: {quads} quads beside {triangles} triangles"
    );
    // The solver targets a count rather than a face size, so it lands much
    // closer than the field extraction does; measured at 1404 for 1500.
    let error = (quads as f32 - target as f32).abs() / target as f32;
    assert!(error <= 0.2, "asked for {target} quads and got {quads}");
}

/// An asset the solver cannot lay out has to come back as a mesh and a warning.
///
/// This one is the regression test for a **crash**: `Eigen::SparseLU`, which
/// `EIGEN_MPL2_ONLY` selects, reports a failed factorization through `info()`
/// and leaves nothing to solve with, and upstream's unchecked `solve()` on that
/// state segfaulted on exactly this fixture (see `third_party/quadriflow`'s
/// NOTICE). A passing run is one that *returns*.
#[cfg(has_quadriflow)]
#[test]
fn an_object_the_quad_solver_gives_up_on_falls_back_rather_than_failing() {
    let Some(model) = fixture("SM_Ammo_Crate_01a.fbx") else {
        return;
    };
    let params = RemeshParams {
        topology: RemeshTopology::PureQuads,
        ..quad_params(1_500)
    };
    let result = run(&model, &remesh_stack(params));

    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "ammo crate, only quads");
    assert!(
        level.model.stats.polygon_count > 0,
        "the fall back still produces a mesh"
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("fell back")),
        "the user has to be told the topology they asked for was not used: {:?}",
        result.warnings
    );
    let (_, quads, other) = face_degrees(level);
    assert_eq!(other, 0);
    assert!(quads > 0, "the fall back is still a quad-dominant mesh");
}

/// Without the vendored solver, "Only quads" stays in the menu and falls back —
/// so a preset naming it does not silently mean something else.
#[cfg(not(has_quadriflow))]
#[test]
fn only_quads_falls_back_when_the_solver_is_not_vendored() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    let params = RemeshParams {
        topology: RemeshTopology::PureQuads,
        ..quad_params(1_500)
    };
    let result = run(&model, &remesh_stack(params));

    let level = result.lod(0).expect("the stack produced a level");
    assert_consistent(&level.model, "column, only quads unavailable");
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("no quad solver")),
        "{:?}",
        result.warnings
    );
}

/// The solver's own seed is pinned, and the tree is compiled serially, so two
/// runs have to agree byte for byte.
#[cfg(has_quadriflow)]
#[test]
fn two_quad_solves_produce_the_same_mesh() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    let params = RemeshParams {
        topology: RemeshTopology::PureQuads,
        ..quad_params(1_200)
    };
    let stack = remesh_stack(params);

    let first = run(&model, &stack);
    let second = run(&model, &stack);

    let a = first.lod(0).expect("a level");
    let b = second.lod(0).expect("a level");
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
    assert_eq!(a.model.faces, b.model.faces, "face tables differ");
}

/// The ratio between the largest and smallest face on one object, measured
/// across the middle of the distribution so a handful of degenerates at either
/// end cannot answer for it.
///
/// In *edge* length rather than area, which is the thing you see: a face twice
/// as long either way is four times the area.
fn face_size_spread(level: &ProcessedLod, node: u32) -> f32 {
    let mut edges: Vec<f32> = Vec::new();
    for (index, face) in level.model.faces.iter().enumerate() {
        let corners = face.index_count as usize;
        if corners < 3 {
            continue;
        }
        let triangle = level
            .model
            .triangles
            .to_face
            .iter()
            .position(|&face| face as usize == index);
        let owner = triangle
            .and_then(|triangle| level.model.triangles.node.get(triangle).copied())
            .unwrap_or(u32::MAX);
        if owner != node {
            continue;
        }
        let corner = |at: usize| level.model.vertices[face.first_index as usize + at].position;
        let mut area = 0.0;
        for at in 1..corners - 1 {
            area += 0.5
                * (corner(at) - corner(0))
                    .cross(corner(at + 1) - corner(0))
                    .length();
        }
        edges.push(area.sqrt());
    }
    assert!(!edges.is_empty(), "node {node} has no faces to measure");
    edges.sort_by(|a, b| a.partial_cmp(b).expect("finite face areas"));
    let at = |share: f32| edges[((edges.len() - 1) as f32 * share) as usize];
    at(0.9) / at(0.1).max(f32::MIN_POSITIVE)
}

/// The node with the most surface area, which on a pedestal-and-pebbles asset is
/// the pedestal: one broad flat face with a rounded rim, i.e. the thing "vary
/// face size" exists for.
fn widest_node(model: &ModelData) -> u32 {
    let mut area: std::collections::BTreeMap<u32, f32> = Default::default();
    for triangle in 0..model.indices.len() / 3 {
        let corner = |at: usize| model.vertices[model.indices[triangle * 3 + at] as usize].position;
        let node = model
            .triangles
            .node
            .get(triangle)
            .copied()
            .unwrap_or_default();
        *area.entry(node).or_default() += 0.5
            * (corner(1) - corner(0))
                .cross(corner(2) - corner(0))
                .length();
    }
    area.into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).expect("finite areas"))
        .map(|(node, _)| node)
        .expect("the fixture has geometry")
}

/// The operation this whole parameter exists for: the same budget, spent where
/// the surface needs it.
#[test]
fn varying_face_size_spends_the_budget_on_the_curved_parts() {
    let Some(model) = fixture("cpg_pedestal_pebbles.fbx") else {
        return;
    };
    let node = widest_node(&model);
    let params = |strength: f32| RemeshParams {
        topology: RemeshTopology::QuadDominant,
        density: RemeshDensity::Ratio,
        ratio: 0.25,
        adaptive_strength: strength,
        ..RemeshParams::default()
    };

    let even = run(&model, &remesh_stack(params(0.0)));
    let varied = run(&model, &remesh_stack(params(1.0)));
    let even = even.lod(0).expect("a level");
    let varied = varied.lod(0).expect("a level");

    // A uniform field puts one face size on the whole object, rim and flat
    // alike — the spread is whatever the extraction's own jitter is.
    let flat = face_size_spread(even, node);
    assert!(
        flat < 1.5,
        "a uniform rebuild should be uniform, and this one spreads {flat:.2}x"
    );
    // A varied one has to be visibly different, not marginally.
    let spread = face_size_spread(varied, node);
    assert!(
        spread > 2.0,
        "varying face size barely varied it: {spread:.2}x against {flat:.2}x even"
    );
}

/// Whatever the field does, the count is the one that was asked for. Every
/// topology, because each reaches it through a different engine or a different
/// symmetry.
#[test]
fn the_face_count_is_the_one_that_was_asked_for() {
    let Some(model) = fixture("cpg_pedestal_pebbles.fbx") else {
        return;
    };
    let wanted = 3_000;
    for topology in RemeshTopology::ALL {
        for strength in [0.0, 1.0] {
            let result = run(
                &model,
                &remesh_stack(RemeshParams {
                    topology,
                    density: RemeshDensity::Absolute,
                    faces: wanted,
                    adaptive_strength: strength,
                    ..RemeshParams::default()
                }),
            );
            let level = result.lod(0).expect("a level");
            let produced = level.model.faces.len() as f32;
            let miss = (produced - wanted as f32).abs() / wanted as f32;
            assert!(
                miss < 0.12,
                "{topology:?} at strength {strength} produced {produced} faces for a \
                 budget of {wanted}, which is {:.0}% out. Warnings: {:?}",
                miss * 100.0,
                result.warnings
            );
        }
    }
}

/// The reproducible path has to stay reproducible *with* a field, which is where
/// it is hardest: the field decides how fine each region is, and the extraction
/// then snaps and collapses at that size.
#[test]
fn a_varied_rebuild_is_reproducible() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let stack = remesh_stack(RemeshParams {
        adaptive_strength: 1.0,
        ..quad_params(1_500)
    });

    let first = run(&model, &stack);
    let second = run(&model, &stack);
    let a = first.lod(0).expect("a level");
    let b = second.lod(0).expect("a level");
    assert_eq!(a.model.vertices, b.model.vertices, "vertices differ");
    assert_eq!(a.model.indices, b.model.indices, "index buffers differ");
}
