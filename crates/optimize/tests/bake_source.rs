//! Baking from the model as it came in (`BakeSource::Original`): every level
//! takes its values from one detailed surface, so a point that is the same in
//! two levels has the same value in both.

#![cfg(has_meshopt)]

use std::collections::HashMap;

use review_model::ModelData;
use review_optimize::{
    AoQuality, AoTarget, BakeAoParams, BakeSource, BakeWearParams, LodLevel, LodParams, OpKind,
    OptStack, ReduceParams, SimplifyAlgorithm, SimplifySettings, TargetMode, WeldParams,
};

mod common;

/// A surface with ridges for the wear to find and parts that occlude one
/// another: the monkey, or `None` when this checkout cannot run the test.
fn part() -> Option<ModelData> {
    common::fixture("monkey.fbx")
}

/// A position weld, a Reduce and a chain, then `bake`. The monkey is shaded
/// flat, so its normals split it at every corner, which no simplifier may
/// cross; welding by position first is what lets the levels shrink at all.
/// The standard simplifier keeps each survivor where it was, so the levels
/// share points to compare at (the default refits them to new positions).
fn stack(bake: OpKind) -> OptStack {
    let simplify = SimplifySettings {
        algorithm: SimplifyAlgorithm::Standard,
        ..SimplifySettings::default()
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams {
        compare_normals: false,
        compare_uvs: false,
        ..WeldParams::default()
    }));
    stack.push_op(OpKind::Reduce(ReduceParams {
        simplify,
        target: LodLevel {
            target_mode: TargetMode::Triangles,
            target_triangles: 1500,
            ..LodLevel::default()
        },
    }));
    stack.push_op(OpKind::SimplifyLod(LodParams {
        simplify,
        levels: vec![
            LodLevel {
                target_ratio: 0.5,
                target_error: 1.0,
                ..LodLevel::default()
            },
            LodLevel {
                target_ratio: 0.25,
                target_error: 1.0,
                ..LodLevel::default()
            },
        ],
    }));
    stack.push_op(bake);
    stack
}

/// Per level, each position's value in `channel` of the vertex colour.
fn values(model: &ModelData, channel: usize) -> HashMap<[u32; 3], u32> {
    model
        .vertices
        .iter()
        .map(|vertex| {
            (
                vertex.position.to_array().map(f32::to_bits),
                vertex.vertex_color[channel].to_bits(),
            )
        })
        .collect()
}

/// How many positions two levels share, and how many of those disagree.
fn agreement(result: &review_optimize::ProcessedResult, channel: usize) -> (usize, usize) {
    let base = values(&result.lods[0].model, channel);
    let (mut shared, mut differ) = (0, 0);
    for lod in &result.lods[1..] {
        for (position, value) in values(&lod.model, channel) {
            if let Some(&own) = base.get(&position) {
                shared += 1;
                if own != value {
                    differ += 1;
                }
            }
        }
    }
    (shared, differ)
}

#[test]
fn wear_from_the_original_is_the_same_wherever_the_levels_meet() {
    let Some(model) = part() else {
        return;
    };
    let original = common::run(
        &model,
        &stack(OpKind::BakeWear(BakeWearParams {
            source: BakeSource::Original,
            ..BakeWearParams::default()
        })),
    );
    assert!(original.warnings.is_empty(), "{:?}", original.warnings);
    let (shared, differ) = agreement(&original, 2);
    assert!(shared > 100, "the levels share points: {shared}");
    assert_eq!(differ, 0, "{differ} of {shared} shared points disagree");
    // Something was written.
    assert!(
        original.lods[2]
            .model
            .vertices
            .iter()
            .any(|vertex| vertex.vertex_color.z > 0.5)
    );
}

#[test]
fn ao_from_the_original_is_the_same_wherever_the_levels_meet() {
    let Some(model) = part() else {
        return;
    };
    let params = BakeAoParams {
        quality: AoQuality::Low,
        target: AoTarget::Alpha,
        source: BakeSource::Original,
        ..BakeAoParams::default()
    };
    let original = common::run(&model, &stack(OpKind::BakeAo(params)));
    assert!(original.warnings.is_empty(), "{:?}", original.warnings);
    let (shared, differ) = agreement(&original, 3);
    assert!(shared > 100, "the levels share points: {shared}");
    assert_eq!(differ, 0, "{differ} of {shared} shared points disagree");

    // Baked on each level, the same points differ: each level occludes itself.
    let own = common::run(
        &model,
        &stack(OpKind::BakeAo(BakeAoParams {
            source: BakeSource::Level,
            ..params
        })),
    );
    let (_, differ_own) = agreement(&own, 3);
    assert!(
        differ_own > 0,
        "a level-by-level bake differs between levels"
    );
}
