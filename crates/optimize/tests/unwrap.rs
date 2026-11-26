//! The Unwrap operation over the CAD kit, measured as
//! the pipeline plan (item 3.1) measures it: stretch (the
//! ratio of a triangle map's singular values) and texel density
//! (`sqrt(UV area / 3D area)`), area-weighted percentiles, against the baseline
//! table Blender's minimum-stretch unwrap set.

#![cfg(has_meshopt)]

use std::path::Path;

use glam::{Vec2, Vec3};
use review_model::ModelData;
use review_optimize::{
    ChartGrowth, LodLevel, LodParams, OpKind, OptStack, ReduceParams, SeamSource, SimplifyFlags,
    SimplifySettings, TargetMode, UnwrapParams, UvLayout,
};

mod common;

fn kit(part: &str, extension: &str) -> ModelData {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../experiments/cad-kit/inputs")
        .join(format!("{part}.{extension}"));
    review_import::load_model(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The baseline's stack (`run_review3d.sh`, `regularize`): a Reduce to
/// `triangles` and a chain at a half and a quarter, the error limit lifted;
/// then the unwrap.
fn stack(triangles: u32, unwrap: UnwrapParams) -> OptStack {
    let simplify = SimplifySettings {
        flags: SimplifyFlags {
            regularize: true,
            ..SimplifyFlags::default()
        },
        ..SimplifySettings::default()
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Reduce(ReduceParams {
        simplify,
        target: LodLevel {
            target_mode: TargetMode::Triangles,
            target_triangles: triangles,
            target_error: 1.0,
            ..LodLevel::default()
        },
    }));
    stack.push_op(OpKind::SimplifyLod(LodParams {
        simplify,
        levels: [0.5, 0.25]
            .map(|ratio| LodLevel {
                target_ratio: ratio,
                target_error: 1.0,
                ..LodLevel::default()
            })
            .to_vec(),
    }));
    stack.push_op(OpKind::Unwrap(unwrap));
    stack
}

/// Area-weighted (stretch p95, density p5, density p95) of `model`'s UV set 0.
fn measure(model: &ModelData) -> (f32, f32, f32) {
    let mut stretch = Vec::new();
    let mut density = Vec::new();
    for corners in model.indices.as_chunks::<3>().0 {
        let p: [Vec3; 3] = corners.map(|corner| model.vertices[corner as usize].position);
        let q: [Vec2; 3] = corners.map(|corner| model.uv_for_channel(corner as usize, 0));
        let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
        let normal = e1.cross(e2);
        let area = 0.5 * normal.length();
        let (Some(x), Some(n)) = (e1.try_normalize(), normal.try_normalize()) else {
            continue;
        };
        let y = n.cross(x);
        let (p00, p01, p10, p11) = (e1.dot(x), e2.dot(x), e1.dot(y), e2.dot(y));
        let det = p00 * p11 - p01 * p10;
        if det.abs() < 1e-14 {
            continue;
        }
        let (q1, q2) = (q[1] - q[0], q[2] - q[0]);
        let a = (q1.x * p11 - q2.x * p10) / det;
        let b = (-q1.x * p01 + q2.x * p00) / det;
        let c = (q1.y * p11 - q2.y * p10) / det;
        let d = (-q1.y * p01 + q2.y * p00) / det;
        let energy = 0.5 * (a * a + b * b + c * c + d * d);
        let root = (energy * energy - (a * d - b * c).powi(2)).max(0.0).sqrt();
        let (large, small) = ((energy + root).sqrt(), (energy - root).max(0.0).sqrt());
        stretch.push(((large / small.max(1e-12)).min(50.0), area));
        density.push(((large * small).sqrt(), area));
    }
    let percentile = |values: &mut Vec<(f32, f32)>, share: f32| {
        values.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total: f32 = values.iter().map(|(_, weight)| weight).sum();
        let mut running = 0.0;
        for &(value, weight) in values.iter() {
            running += weight;
            if running / total >= share {
                return value;
            }
        }
        values.last().map_or(0.0, |(value, _)| *value)
    };
    (
        percentile(&mut stretch, 0.95),
        percentile(&mut density, 0.05),
        percentile(&mut density, 0.95),
    )
}

/// One row of the baseline table: stretch p95, density p5, density p95.
type Row = (f32, f32, f32);

/// Each part at the baseline's budget, against its table rows: stretch p95 no
/// more than 0.05 over Blender's, the density spread no wider.
#[test]
fn the_kit_unwraps_at_least_as_well_as_the_baseline() {
    // (part, triangles at LOD0, per level: stretch p95, density p5, density p95)
    let baseline: [(&str, u32, [Row; 3]); 4] = [
        (
            "link",
            1500,
            [
                (1.016, 0.999, 1.001),
                (1.014, 0.999, 1.001),
                (1.023, 0.998, 1.000),
            ],
        ),
        (
            "shell",
            1500,
            [
                (1.259, 0.937, 1.113),
                (1.285, 0.936, 1.113),
                (1.325, 0.931, 1.124),
            ],
        ),
        (
            "hinge_block",
            400,
            [
                (1.015, 0.998, 1.003),
                (1.021, 0.997, 1.002),
                (1.019, 0.998, 1.001),
            ],
        ),
        (
            "hinge_pin",
            250,
            [
                (1.290, 0.998, 1.002),
                (1.034, 0.996, 1.003),
                (1.010, 0.999, 1.001),
            ],
        ),
    ];
    // The glb parts, the CAD face in `_CAD_FACE`.
    for (part, triangles, rows) in baseline {
        let model = kit(part, "glb");
        let result = common::run(
            &model,
            &stack(triangles, UnwrapParams::cad_islands("_CAD_FACE")),
        );
        assert!(result.warnings.is_empty(), "{part}: {:?}", result.warnings);
        for (lod, (stretch_p95, density_p5, density_p95)) in result.lods.iter().zip(rows) {
            let (stretch, low, high) = measure(&lod.model);
            assert!(
                stretch <= stretch_p95 + 0.05,
                "{part} LOD{}: stretch p95 {stretch} against {stretch_p95}",
                lod.level
            );
            assert!(
                high - low <= density_p95 - density_p5 + 0.002,
                "{part} LOD{}: density {low}-{high} against {density_p5}-{density_p95}",
                lod.level
            );
        }
    }
}

/// The FBX route carries the CAD face id as U of a second UV set, which the
/// simplifier refits a little: islands still follow the faces.
#[test]
fn an_id_carried_in_a_uv_set_cuts_islands_too() {
    let model = kit("hinge_pin", "fbx");
    let unwrap = UnwrapParams {
        seams: SeamSource::UvSetU(1),
        ..UnwrapParams::cad_islands("unused")
    };
    let result = common::run(&model, &stack(250, unwrap));
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    let (stretch, low, high) = measure(&result.lods[0].model);
    assert!(stretch < 1.05, "stretch p95 {stretch}");
    assert!(high - low < 0.01, "density {low}-{high}");
}

/// An atlas is in the 0-1 square, in the set asked for, and adds that set to a
/// model that had none.
#[test]
fn an_atlas_fills_the_unit_square_in_its_own_set() {
    let model = kit("link", "glb");
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Unwrap(UnwrapParams {
        layout: UvLayout::Atlas,
        target_set: 1,
        charts: ChartGrowth::Automatic,
        ..UnwrapParams::default()
    }));
    let result = common::run(&model, &stack);
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    let level = &result.lods[0].model;
    assert_eq!(level.stats.uv_set_count, 2, "set 1 added beside set 0");
    let atlas = &level.uv_channels[1];
    assert!(
        atlas
            .iter()
            .all(|uv| (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y))
    );
    let (low, high) = atlas
        .iter()
        .fold((Vec2::MAX, Vec2::MIN), |(low, high), uv| {
            (low.min(*uv), high.max(*uv))
        });
    assert!(
        (high - low).max_element() > 0.9,
        "the atlas spans the square: {low} {high}"
    );
    // Set 0 is untouched.
    assert_eq!(
        level.uv_channels[0],
        level
            .vertices
            .iter()
            .map(|vertex| vertex.uv)
            .collect::<Vec<_>>()
    );
}
