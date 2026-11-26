//! Baking from the model as it came in (`BakeSource::Original`): every level
//! takes its values from one detailed surface, so a point that is the same in
//! two levels has the same value in both.

#![cfg(has_meshopt)]

use std::collections::HashMap;
use std::path::Path;

use review_model::ModelData;
use review_optimize::{
    AoQuality, AoTarget, BakeAoParams, BakeSource, BakeWearParams, LodLevel, LodParams, OpKind,
    OptStack, ReduceParams, TargetMode,
};

mod common;

fn link() -> ModelData {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/cad-kit/inputs/link.glb");
    review_import::load_model(&path).expect("the kit's link reads")
}

/// A Reduce and a chain, then `bake`.
fn stack(bake: OpKind) -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Reduce(ReduceParams {
        target: LodLevel {
            target_mode: TargetMode::Triangles,
            target_triangles: 1500,
            ..LodLevel::default()
        },
        ..ReduceParams::default()
    }));
    stack.push_op(OpKind::SimplifyLod(LodParams {
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
        ..LodParams::default()
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
    let model = link();
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
    let model = link();
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
