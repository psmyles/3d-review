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

use review_model::ModelData;
use review_optimize::{
    AoQuality, AoTarget, BakeAoParams, LodLevel, LodParams, NormalParams, OpKind, OptStack,
    OptWarning, ReduceParams, SimplifyAlgorithm, SimplifyFlags, SimplifySettings, WeldParams,
};

mod common;

use common::{fixture, fixture_path, run};

/// Every fixture the tests below load. Asserted to exist by
/// [`every_fixture_the_suite_loads_exists`], so a rename cannot quietly turn the
/// whole suite green.
const FIXTURES: [&str; 5] = [
    "SK_Player_01.fbx",
    "SM_column04.fbx",
    "lucy.fbx",
    "monkey.fbx",
    "xyzrgb_dragon.fbx",
];

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

/// Reduce runs the same simplifier the LOD chain does, but against the mesh
/// itself: the run produces exactly one output, at the target, and it is that
/// mesh a later operation and the export see. On a real asset it has to hit the
/// target as squarely as a LOD level does — the two share the code that aims it.
#[test]
fn a_reduce_over_a_game_asset_hits_its_target_in_place() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let source_triangles = model.indices.len() / 3;

    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::Reduce(ReduceParams {
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        target: LodLevel {
            target_ratio: 0.5,
            target_error: 0.02,
        },
    }));

    let result = run(&model, &stack);
    assert_eq!(
        result.lods.len(),
        1,
        "a reduce replaces the mesh rather than adding a level beside it"
    );
    let reduced = &result.lods[0].model;
    assert_consistent(reduced, "reduced");
    assert_eq!(
        reduced.name, model.name,
        "the mesh keeps the source name: nothing about it is a LOD"
    );

    let triangles = reduced.stats.triangle_count;
    assert!(
        triangles < source_triangles * 6 / 10 && triangles > source_triangles * 4 / 10,
        "the reduce lands near its 50% target: {triangles} of {source_triangles}"
    );
    assert!(
        result.lods[0].metrics.simplify_error > 0.0,
        "level 0 now carries the error the reduce cost: {:?}",
        result.lods[0].metrics
    );
    assert_eq!(
        result.source.triangles, source_triangles,
        "the baseline is still the unreduced mesh, so the overlay's delta is real"
    );
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
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        }],
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
            .any(|warning| matches!(warning, OptWarning::SimplifierStalledOnSeams)),
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
        simplify: SimplifySettings {
            flags: SimplifyFlags {
                permissive: true,
                ..SimplifyFlags::default()
            },
            ..half.simplify
        },
        ..half.clone()
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
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
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
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        }],
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

    // The fixture is skinned; every level keeps a skin over its own vertices,
    // valid by the same guard the import runs, with the clips beside it.
    if let Some(source_skin) = &model.skin {
        for lod in &result.lods {
            let skin = lod
                .model
                .skin
                .as_ref()
                .expect("skinning is kept on every level");
            assert_eq!(skin.logical_vertex_count(), lod.model.vertices.len());
            assert_eq!(skin.clusters.len(), source_skin.clusters.len());
            assert!(
                lod.model.validate_deform().is_ok(),
                "LOD {} deform data",
                lod.level
            );
            assert_eq!(lod.model.animations.len(), model.animations.len());
        }
        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.to_string().contains("skin")),
            "nothing about the skin was dropped: {:?}",
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
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        levels: vec![LodLevel {
            target_ratio: 0.25,
            target_error: 0.2,
        }],
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

/// Sanity: every file the tests above load is where they expect it. Checking the
/// directory alone was not enough — renaming one fixture turned four tests into
/// silent green passes.
#[test]
fn every_fixture_the_suite_loads_exists() {
    for name in FIXTURES {
        let path = fixture_path(name);
        assert!(
            path.is_file(),
            "fixture missing or renamed; the tests that load it would silently skip: {}",
            path.display()
        );
    }
}

/// A LOD operation on its own has to work.
///
/// It did not: import splits every face corner, so the mesh reaches the
/// simplifier with no shared vertices and every edge reading as an attribute
/// seam, and a topology-preserving collapse removed *nothing* — the user added a
/// LOD operation, moved its sliders, and watched the triangle count sit still.
/// A run now indexes the mesh losslessly first, which is the precondition every
/// meshoptimizer operation assumes.
#[test]
fn a_lod_operation_alone_simplifies_a_corner_split_import() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };
    let source_triangles = model.indices.len() / 3;

    let mut stack = OptStack::default();
    stack.push_op(OpKind::SimplifyLod(LodParams {
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.01,
        }],
    }));
    let result = run(&model, &stack);

    let base = &result.lods[0].model;
    assert_eq!(
        base.indices.len() / 3,
        source_triangles,
        "level 0 keeps every triangle"
    );
    assert!(
        base.vertices.len() < model.vertices.len() / 2,
        "the lossless index pass shares vertices: {} -> {}",
        model.vertices.len(),
        base.vertices.len()
    );

    let simplified = result.lods[1].model.indices.len() / 3;
    assert!(
        simplified <= source_triangles * 55 / 100,
        "a 50% target should be reached: {source_triangles} -> {simplified}"
    );
    assert_consistent(&result.lods[1].model, "column LOD1");
}

/// Indexing changes what the mesh *is* made of, never what it looks like: the
/// merged vertices were identical, so every triangle still spans the same three
/// positions it did before.
#[test]
fn indexing_preserves_every_triangle_of_the_source() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };

    let mut stack = OptStack::default();
    stack.push_op(OpKind::VertexCache);
    let processed = &run(&model, &stack).lods[0].model;

    assert_eq!(
        processed.indices.len(),
        model.indices.len(),
        "no triangle is added or lost"
    );
    let corners = |mesh: &ModelData| {
        let mut all: Vec<[[u32; 3]; 3]> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| {
                let mut corner = [[0u32; 3]; 3];
                for (slot, &index) in corner.iter_mut().zip(triangle) {
                    *slot = mesh.vertices[index as usize]
                        .position
                        .to_array()
                        .map(f32::to_bits);
                }
                corner.sort_unstable();
                corner
            })
            .collect();
        all.sort_unstable();
        all
    };
    assert_eq!(
        corners(processed),
        corners(&model),
        "the same triangles, over the same positions"
    );
}

/// The stats panel's "GPU Verts" (measured at import) and the Opt card's
/// baseline (measured after the run's lossless index pass) are presented as the
/// same figure, so they must *be* the same figure — a user who sees them
/// disagree has no way to know which to trust.
#[test]
fn the_import_gpu_vertex_count_matches_the_opt_baseline() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };

    assert!(
        model.stats.gpu_vertex_count > 0 && model.stats.gpu_vertex_count < model.vertices.len(),
        "the GPU cost is measured and is below the corner-split count: {} of {}",
        model.stats.gpu_vertex_count,
        model.vertices.len()
    );

    // An empty stack still measures: its baseline is the indexed mesh.
    let result = run(&model, &OptStack::default());
    assert!(result.lods.is_empty());
    assert_eq!(
        result.source.vertices, model.stats.gpu_vertex_count,
        "the two GPU-vertex figures the UI shows must agree"
    );
    assert_eq!(result.source.triangles, model.indices.len() / 3);
    assert!(
        result.source_metrics.acmr < 3.0,
        "the baseline ACMR describes the indexed asset, not the corner-split \
         buffer (which is always exactly 3.0): {}",
        result.source_metrics.acmr
    );
}

/// The AO bake over a real asset: every written value is a real color, the
/// asset's own concavities produce actual variation, and two runs are
/// bit-identical — which is what makes the bake safe to re-run on every stack
/// edit and stable across a preset round-trip.
#[test]
fn bake_ao_writes_bounded_deterministic_colors() {
    let Some(model) = fixture("SM_column04.fbx") else {
        return;
    };

    let mut stack = OptStack::default();
    stack.push_op(OpKind::BakeAo(BakeAoParams {
        // Low quality keeps the suite quick; determinism and bounds don't
        // depend on the ray count.
        quality: AoQuality::Low,
        target: AoTarget::Rgb,
        ..BakeAoParams::default()
    }));

    let result = run(&model, &stack);
    let baked = &result.lods[0].model;
    assert_consistent(baked, "bake_ao");

    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    for vertex in &baked.vertices {
        for component in vertex.vertex_color.to_array() {
            assert!(
                component.is_finite() && (0.0..=1.0).contains(&component),
                "every baked component is a real color value, got {component}"
            );
        }
        lowest = lowest.min(vertex.vertex_color.x);
        highest = highest.max(vertex.vertex_color.x);
    }
    assert!(
        highest - lowest > 0.1,
        "a column's crevices and open faces bake differently: {lowest}..{highest}"
    );

    let again = run(&model, &stack);
    for (a, b) in baked.vertices.iter().zip(&again.lods[0].model.vertices) {
        assert_eq!(
            a.vertex_color.to_array(),
            b.vertex_color.to_array(),
            "two runs are bit-identical"
        );
    }
}

/// Recalculating a skinned character's normals splits vertices at its hard
/// edges, and every copy must keep the skin its original had — checked by the
/// same guard import runs, over the level's own vertices.
#[test]
fn recalculating_a_skinned_characters_normals_keeps_its_skin() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::RecalculateNormals(NormalParams::default()));
    let result = run(&model, &stack);
    let level = &result.lods[0].model;
    assert_consistent(level, "recalculated character");
    assert_eq!(
        level.indices.len(),
        model.indices.len(),
        "no triangle changed"
    );
    if model.skin.is_some() {
        let skin = level.skin.as_ref().expect("the skin is kept");
        assert_eq!(skin.logical_vertex_count(), level.vertices.len());
        assert!(level.validate_deform().is_ok());
    }
    assert!(
        level
            .vertices
            .iter()
            .all(|vertex| (vertex.normal.length() - 1.0).abs() < 1e-3),
        "every normal is unit length"
    );
}

/// The tangent rebuild splits a vertex only at a mirrored-UV seam, so on a real
/// asset it adds a handful of vertices, not a multiple of them. A regression to
/// splitting on every tangent difference would show up here as a jump.
#[test]
fn the_tangent_rebuild_adds_only_a_few_vertices_on_a_real_asset() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let mut indexed = OptStack::default();
    indexed.push_op(OpKind::VertexFetch);
    let baseline = run(&model, &indexed).lods[0].model.vertices.len();

    let mut welded = OptStack::default();
    welded.push_op(OpKind::FilterTriangles);
    let split = run(&model, &welded).lods[0].model.vertices.len();
    assert!(
        split >= baseline && split * 100 <= baseline * 105,
        "the rebuild grew {baseline} vertices to {split}"
    );
}
