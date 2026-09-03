//! Checks over the repository's real FBX fixtures that the per-node figures the
//! bridge records line up with the whole-model ones it publishes.
//!
//! The stats overlay's scoped columns are sums of these per-node / per-draw-group
//! numbers, so "the parts add up to the whole" is the property that makes them
//! faithful (invariant 5). The demo cube cannot test it: it has one node, one
//! material and no seams. These run over assets with real hierarchies — a
//! multi-part prop, a skinned character, a scan — where a drifted attribution
//! would actually show.
//!
//! Each test skips itself when its fixture is absent or the build carries no
//! vendored ufbx, so a bare checkout still passes.

use std::path::PathBuf;

use review_model::{ModelData, StatsScope};

/// Every fixture the tests below load. Asserted to exist by
/// [`every_fixture_the_suite_loads_exists`], so a rename cannot quietly turn the
/// whole suite green.
const FIXTURES: [&str; 3] = ["SK_Player_01.fbx", "SM_column04.fbx", "lucy.fbx"];

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test_models")
        .join(name)
}

fn fixture(name: &str) -> Option<ModelData> {
    let path = fixture_path(name);
    if !path.exists() {
        eprintln!("skipping {name}: the fixture is not present");
        return None;
    }
    match review_import::load_model(&path) {
        Ok(model) => Some(model),
        // A checkout without the vendored ufbx sources can't import *anything*;
        // that is the same "this checkout can't run these" situation as a missing
        // fixture. Any other error means the file is there and did not load,
        // which the suite must report rather than skip past.
        Err(review_import::ImportError::UfbxUnavailable) => {
            eprintln!("skipping {name}: FBX import is unavailable in this build");
            None
        }
        Err(error) => panic!("{name} is present but failed to load: {error}"),
    }
}

#[test]
fn every_fixture_the_suite_loads_exists() {
    for name in FIXTURES {
        let path = fixture_path(name);
        assert!(
            path.exists(),
            "fixture {name} is missing; update FIXTURES or restore {}",
            path.display()
        );
    }
}

/// The per-node DCC counts the bridge now records must sum to the file's own
/// `Verts` stat — that identity is the whole reason a scoped Verts figure can be
/// reported at all.
#[test]
fn per_node_vertex_counts_sum_to_the_model_stat() {
    for name in FIXTURES {
        let Some(model) = fixture(name) else { continue };
        let summed: usize = model
            .nodes
            .iter()
            .map(|node| node.source_vertex_count)
            .sum();
        assert_eq!(
            summed, model.stats.vertex_count,
            "{name}: per-node source vertex counts must sum to the model's own"
        );
        // Only mesh-bearing nodes carry one: a group or bone contributes nothing.
        for node in &model.nodes {
            if node.mesh_part.is_none() {
                assert_eq!(
                    node.source_vertex_count, 0,
                    "{name}: node {:?} has no mesh but claims vertices",
                    node.name
                );
            }
        }
    }
}

/// A scope covering every node must reproduce the whole-model figures exactly —
/// which is what lets the overlay show the file's own numbers beside measured
/// subsets and have the two agree when nothing is hidden.
#[test]
fn a_scope_over_every_node_reproduces_the_model_stats() {
    for name in FIXTURES {
        let Some(model) = fixture(name) else { continue };
        let groups = model.mesh_group_stats();
        let everything = vec![true; model.nodes.len()];
        let scope = model.scope_stats(&groups, StatsScope::Nodes(&everything));

        assert_eq!(
            scope.triangle_count,
            model.indices.len() / 3,
            "{name}: triangles"
        );
        assert_eq!(
            scope.gpu_vertex_count, model.stats.gpu_vertex_count,
            "{name}: GPU vertices"
        );
        assert_eq!(scope.draw_count, model.stats.draw_count, "{name}: draws");
        assert_eq!(
            scope.vertex_count,
            Some(model.stats.vertex_count),
            "{name}: authored vertices"
        );
        // Polygons are measured from the faces the triangles reference, so they
        // can fall short of the file's face count only by faces that produced no
        // triangle at all (a stray point or edge in the mesh).
        assert!(
            scope.polygon_count <= model.stats.polygon_count,
            "{name}: measured polygons {} exceed the file's {}",
            scope.polygon_count,
            model.stats.polygon_count
        );
    }
}

/// Every node's own scope, added up, must also reproduce the whole — the groups
/// partition the mesh, so nothing may be counted twice or dropped.
#[test]
fn the_per_node_scopes_partition_the_mesh() {
    for name in FIXTURES {
        let Some(model) = fixture(name) else { continue };
        let groups = model.mesh_group_stats();

        let mut triangles = 0;
        let mut gpu_vertices = 0;
        for index in 0..model.nodes.len() {
            let mut mask = vec![false; model.nodes.len()];
            mask[index] = true;
            let scope = model.scope_stats(&groups, StatsScope::Nodes(&mask));
            triangles += scope.triangle_count;
            gpu_vertices += scope.gpu_vertex_count;
        }
        assert_eq!(triangles, model.indices.len() / 3, "{name}: triangles");
        assert_eq!(
            gpu_vertices, model.stats.gpu_vertex_count,
            "{name}: GPU vertices"
        );
    }
}

/// Selecting one material must account for every triangle wearing it, and no
/// others — and must decline to report an authored vertex count, which a
/// material slot has no honest claim to.
#[test]
fn material_scopes_cover_their_own_triangles_only() {
    for name in FIXTURES {
        let Some(model) = fixture(name) else { continue };
        let groups = model.mesh_group_stats();

        let mut triangles = 0;
        for slot in 0..model.materials.len() {
            let scope = model.scope_stats(&groups, StatsScope::Material(slot as u32));
            assert_eq!(scope.vertex_count, None, "{name}: material {slot}");
            assert!(
                scope.draw_count <= 1,
                "{name}: one material slot is at most one draw"
            );
            triangles += scope.triangle_count;

            let counted = model
                .triangles
                .material
                .iter()
                .filter(|&&material| material as usize == slot)
                .count();
            assert_eq!(
                scope.triangle_count, counted,
                "{name}: material {slot} triangles"
            );
        }
        assert_eq!(
            triangles,
            model.indices.len() / 3,
            "{name}: every triangle carries exactly one of the file's materials"
        );
    }
}
