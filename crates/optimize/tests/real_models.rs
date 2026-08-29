//! End-to-end checks over the repository's real FBX fixtures.
//!
//! The unit tests in `process.rs` use the synthetic demo cube, which is small
//! enough to assert exact counts on but is not representative: it has one node,
//! one material, twelve triangles and no seams worth speaking of. These run the
//! same pipeline over actual scanned and modelled assets — hundreds of thousands
//! of triangles, real UV seams, and (for the character) multiple materials and a
//! skin deformer — where the interesting failures live.
//!
//! Each test skips itself when its fixture or the vendored meshoptimizer sources
//! are absent, so a checkout without them still passes.

#![cfg(has_meshopt)]

use std::path::{Path, PathBuf};

use review_model::ModelData;
use review_optimize::{
    LodLevel, LodParams, OpKind, OptStack, ProcessInput, ProcessedResult, SimplifyAlgorithm,
    SimplifyFlags, WeldParams, process,
};

/// Stand-in for the renderer's `SceneVertex` size (position, normal, uv, tangent,
/// color). Only the overfetch figure depends on it.
const VERTEX_SIZE: usize = 64;

fn fixture(name: &str) -> Option<ModelData> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test_models")
        .join(name);
    if !path.exists() {
        return None;
    }
    match review_import::load_model(&path) {
        Ok(model) => Some(model),
        // A checkout without the vendored ufbx sources can't import anything;
        // that is a missing-fixture situation, not a failure of this crate.
        Err(error) => {
            eprintln!("skipping {name}: {error}");
            None
        }
    }
}

fn run(model: &ModelData, stack: &OptStack) -> ProcessedResult {
    process(ProcessInput {
        model,
        stack,
        render_vertex_size: VERTEX_SIZE,
    })
    .expect("a real fixture always processes")
}

/// Assert the structural guarantees every processed mesh must satisfy, whatever
/// the stack did to it.
fn assert_consistent(model: &ModelData, label: &str) {
    let triangle_count = model.indices.len() / 3;
    assert_eq!(
        model.indices.len() % 3,
        0,
        "{label}: index buffer is whole triangles"
    );
    model
        .triangles
        .validate(
            triangle_count,
            model.faces.len(),
            model.materials.len(),
            model.nodes.len(),
        )
        .unwrap_or_else(|error| panic!("{label}: per-triangle arrays drifted: {error}"));
    assert!(
        model
            .indices
            .iter()
            .all(|&index| (index as usize) < model.vertices.len()),
        "{label}: every index addresses a real vertex"
    );
    for (channel, uvs) in model.uv_channels.iter().enumerate() {
        assert_eq!(
            uvs.len(),
            model.vertices.len(),
            "{label}: UV channel {channel} runs parallel to the vertex array"
        );
    }
    assert_eq!(
        model.stats.triangle_count, triangle_count,
        "{label}: the reported triangle count is the measured one"
    );
    assert_eq!(
        model.stats.vertex_count,
        model.vertices.len(),
        "{label}: the reported vertex count is the measured one"
    );
    if !model.vertices.is_empty() {
        assert!(model.bounds.is_some(), "{label}: bounds are recomputed");
    }
}

/// Position welding, which is what the option exists for.
const POSITION_WELD: WeldParams = WeldParams {
    attribute_tolerance: 0.0,
    compare_normals: false,
    compare_uvs: false,
    compare_colors: false,
};

/// The default weld only removes *exact* duplicates, so on a game asset — whose
/// interior corners share a normal and a UV — it collapses the import's corner
/// expansion down to something near the authored vertex count.
#[test]
fn welding_a_game_asset_removes_most_of_the_duplicate_corners() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let before = model.vertices.len();

    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    let result = run(&model, &stack);

    let after = result.lods[0].model.vertices.len();
    assert!(
        after < before / 2,
        "welding should more than halve a game asset's vertices: {before} -> {after}"
    );
    assert_eq!(
        result.lods[0].model.indices.len(),
        model.indices.len(),
        "welding removes no triangles"
    );
    assert_consistent(&result.lods[0].model, "welded character");
}

/// A scanned mesh imported without authored normals gets per-face ones, so every
/// corner's normal differs and the default (attribute-preserving) weld correctly
/// merges nothing — the corners are genuinely distinct. Position welding is the
/// option for that case, and it must both collapse the mesh *and* leave it with
/// usable normals, since merging discards all but one of the originals.
#[test]
fn position_welding_a_scan_collapses_it_and_rebuilds_usable_normals() {
    let Some(model) = fixture("lucy.fbx") else {
        return;
    };
    let before = model.vertices.len();

    let mut attribute_weld = OptStack::default();
    attribute_weld.push_op(OpKind::Weld(WeldParams::default()));
    assert_eq!(
        run(&model, &attribute_weld).lods[0].model.vertices.len(),
        before,
        "every corner of a flat-shaded scan has a distinct normal, so nothing merges"
    );

    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(POSITION_WELD));
    let result = run(&model, &stack);
    let welded = &result.lods[0].model;

    assert!(
        welded.vertices.len() < before / 2,
        "position welding should more than halve the vertices: {before} -> {}",
        welded.vertices.len()
    );
    assert_consistent(welded, "position-welded lucy");

    // The surviving normal after a merge is an arbitrary one of the originals, so
    // the mesh must have been given fresh ones — every normal unit length and
    // pointing somewhere real.
    for (index, vertex) in welded.vertices.iter().enumerate() {
        let length = vertex.normal.length();
        assert!(
            (length - 1.0).abs() < 1.0e-3,
            "vertex {index} normal is not unit length after welding: {length}"
        );
    }
}

/// The scans block the topology-preserving simplifier: with per-face normals
/// every edge is an attribute seam, and collapsing across one is exactly what the
/// default settings forbid. Both documented ways out must work — position welding
/// first, or letting the simplifier cross seams.
#[test]
fn a_flat_shaded_scan_simplifies_once_its_attribute_seams_are_dealt_with() {
    let Some(model) = fixture("xyzrgb_dragon.fbx") else {
        return;
    };
    let source_triangles = model.indices.len() / 3;
    let half = LodParams {
        algorithm: SimplifyAlgorithm::Standard,
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        }],
        ..LodParams::default()
    };

    let mut blocked = OptStack::default();
    blocked.push_op(OpKind::SimplifyLod(half.clone()));
    let blocked_result = run(&model, &blocked);
    let blocked_triangles = blocked_result.lods[1].model.stats.triangle_count;
    assert!(
        blocked_triangles > source_triangles * 9 / 10,
        "with every edge an attribute seam the simplifier is expected to stall, \
         got {blocked_triangles} of {source_triangles}"
    );
    // Stalling looks identical to the tool being broken, so it must be explained.
    assert!(
        blocked_result
            .warnings
            .iter()
            .any(|warning| warning.contains("attribute seam")),
        "a stalled simplify explains itself: {:?}",
        blocked_result.warnings
    );

    let mut welded = OptStack::default();
    welded.push_op(OpKind::Weld(POSITION_WELD));
    welded.push_op(OpKind::SimplifyLod(half.clone()));
    let welded_triangles = run(&model, &welded).lods[1].model.stats.triangle_count;
    assert!(
        welded_triangles < source_triangles * 6 / 10,
        "position welding should unblock the simplifier: {welded_triangles} of {source_triangles}"
    );

    let mut permissive = OptStack::default();
    permissive.push_op(OpKind::SimplifyLod(LodParams {
        flags: SimplifyFlags {
            permissive: true,
            ..SimplifyFlags::default()
        },
        ..half
    }));
    let permissive_triangles = run(&model, &permissive).lods[1].model.stats.triangle_count;
    assert!(
        permissive_triangles < source_triangles * 6 / 10,
        "collapsing across seams should also unblock it: \
         {permissive_triangles} of {source_triangles}"
    );
}

/// The headline case: a LOD chain over a real game asset should actually hit its
/// targets, keep shrinking, and stay structurally sound at every level.
#[test]
fn a_lod_chain_over_a_game_asset_hits_its_targets() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let source_triangles = model.indices.len() / 3;

    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::SimplifyLod(LodParams {
        algorithm: SimplifyAlgorithm::Standard,
        levels: vec![
            LodLevel {
                target_ratio: 0.5,
                target_error: 0.02,
            },
            LodLevel {
                target_ratio: 0.25,
                target_error: 0.05,
            },
            LodLevel {
                target_ratio: 0.1,
                target_error: 0.1,
            },
        ],
        ..LodParams::default()
    }));

    let result = run(&model, &stack);
    assert_eq!(result.lods.len(), 4, "base plus three levels");

    let counts: Vec<usize> = result
        .lods
        .iter()
        .map(|lod| lod.model.stats.triangle_count)
        .collect();
    assert_eq!(counts[0], source_triangles, "level 0 is not simplified");
    assert!(
        counts.windows(2).all(|pair| pair[1] < pair[0]),
        "each level is strictly smaller than the last: {counts:?}"
    );
    // A clean closed surface with a generous error budget should get close to the
    // requested ratio; allow slack for the topology constraints the simplifier
    // legitimately stops on.
    assert!(
        counts[1] < source_triangles * 3 / 4,
        "the 50% level should get well under three quarters: {counts:?}"
    );

    for lod in &result.lods {
        assert_consistent(&lod.model, &format!("character LOD {}", lod.level));
        assert!(
            lod.metrics.acmr > 0.0 && lod.metrics.atvr > 0.0,
            "LOD {} carries measured cache figures",
            lod.level
        );
    }
}

/// Vertex-cache optimization must improve (or at worst not worsen) ACMR on a real
/// mesh, and change nothing about the geometry. This is the check that makes the
/// reorder operations meaningful — they are invisible in the viewport, so the
/// measurement is the only evidence they did anything.
#[test]
fn vertex_cache_optimization_improves_acmr_without_touching_geometry() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };

    let mut welded = OptStack::default();
    welded.push_op(OpKind::Weld(WeldParams::default()));
    let plain = run(&model, &welded);

    let mut optimized_stack = welded.clone();
    optimized_stack.push_op(OpKind::VertexCache);
    let optimized = run(&model, &optimized_stack);

    assert_eq!(
        optimized.lods[0].model.indices.len(),
        plain.lods[0].model.indices.len(),
        "reordering removes no triangles"
    );
    assert_eq!(
        optimized.lods[0].model.vertices.len(),
        plain.lods[0].model.vertices.len(),
        "reordering removes no vertices"
    );
    assert!(
        optimized.lods[0].metrics.acmr <= plain.lods[0].metrics.acmr,
        "ACMR should not regress: {} -> {}",
        plain.lods[0].metrics.acmr,
        optimized.lods[0].metrics.acmr
    );
}

/// A multi-material, multi-node character. Every triangle must keep a material
/// and node tag that still addresses a real slot — the property that forced
/// processing to work per (node, material) submesh in the first place.
#[test]
fn a_multi_material_character_keeps_every_triangle_tagged() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    assert!(
        model.materials.len() > 1,
        "this fixture is expected to carry several materials"
    );

    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::SimplifyLod(LodParams {
        algorithm: SimplifyAlgorithm::Standard,
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        }],
        ..LodParams::default()
    }));

    let result = run(&model, &stack);
    for lod in &result.lods {
        let label = format!("character LOD {}", lod.level);
        assert_consistent(&lod.model, &label);

        let triangle_count = lod.model.indices.len() / 3;
        assert_eq!(
            lod.model.triangles.material.len(),
            triangle_count,
            "{label}: every triangle keeps a material tag"
        );
        assert_eq!(
            lod.model.triangles.node.len(),
            triangle_count,
            "{label}: every triangle keeps a node tag"
        );
        // Materials survive a simplify only because each (node, material) group is
        // processed as its own mesh; a regression there shows up as tags collapsing
        // to a single slot.
        let distinct: std::collections::HashSet<u32> =
            lod.model.triangles.material.iter().copied().collect();
        assert!(
            distinct.len() > 1,
            "{label}: the mesh still uses several materials, found {distinct:?}"
        );
    }

    // The fixture is skinned, which this tool drops — and says so.
    if model.skin.is_some() {
        assert!(
            result.lods.iter().all(|lod| lod.model.skin.is_none()),
            "skinning is dropped from every level"
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("skinned")),
            "the user is told skinning was dropped: {:?}",
            result.warnings
        );
    }
}

/// An excluded object must come through byte-identical while its neighbours are
/// optimized around it.
#[test]
fn excluding_a_node_leaves_its_geometry_untouched() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    // Pick the node owning the most triangles, so the assertion has something
    // substantial to check.
    let Some(node) = most_common_node(&model) else {
        return;
    };
    let source_triangles = model
        .triangles
        .node
        .iter()
        .filter(|&&owner| owner == node)
        .count();

    let mut stack = OptStack::default();
    stack.push_op(OpKind::SimplifyLod(LodParams {
        algorithm: SimplifyAlgorithm::Standard,
        levels: vec![LodLevel {
            target_ratio: 0.25,
            target_error: 0.2,
        }],
        ..LodParams::default()
    }));
    stack.node_override_mut(node as usize).exclude = true;

    let result = run(&model, &stack);
    let lod1 = &result.lods[1].model;
    let kept = lod1
        .triangles
        .node
        .iter()
        .filter(|&&owner| owner == node)
        .count();

    assert_eq!(
        kept, source_triangles,
        "an excluded node keeps every triangle even in the smallest level"
    );
    assert!(
        lod1.stats.triangle_count < model.indices.len() / 3,
        "the rest of the model was still simplified"
    );
}

/// The node owning the most triangles, or `None` for a model with no node tags.
fn most_common_node(model: &ModelData) -> Option<u32> {
    use std::collections::HashMap;
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for &node in &model.triangles.node {
        *counts.entry(node).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(node, _)| node)
}

/// Sanity: the fixture directory is where the tests expect it, so a rename turns
/// into a visible failure rather than silently skipping every test above.
#[test]
fn the_fixture_directory_exists() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/test_models");
    assert!(
        dir.is_dir(),
        "fixture directory moved; the tests above would silently skip: {}",
        dir.display()
    );
}
