//! The Unwrap operation, measured the way a CAD pipeline measures it: stretch
//! (the ratio of a triangle map's singular values) and texel density
//! (`sqrt(UV area / 3D area)`), area-weighted percentiles. The CAD parts are
//! measured against the baseline Blender's minimum-stretch unwrap set; they are
//! the CAD kit ([`common::cad_kit`]), which is not in this repository, so those
//! tests are `#[ignore]`d. The rest run over the committed fixtures.

#![cfg(has_meshopt)]

use std::collections::HashMap;
use std::path::Path;

use glam::{Vec2, Vec3};
use review_model::{ComponentType, ModelData};
use review_optimize::{
    ExistingSeams, LodLevel, LodParams, OpKind, OptStack, ReduceParams, SeamSource, SimplifyFlags,
    SimplifySettings, TargetMode, UnwrapParams, UvLayout,
};

mod common;

fn load(path: &Path) -> ModelData {
    review_import::load_model(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// A part's name: its file's stem, as the kit's `baseline.json` keys it.
fn part_name(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

/// The CAD kit's `baseline.json`: per part, its LOD0 triangle budget and, per
/// level, the baseline row.
fn baseline() -> HashMap<String, (u32, Vec<Row>)> {
    let path = common::cad_kit_dir().join("baseline.json");
    let text = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let json: serde_json::Value = serde_json::from_slice(&text).expect("baseline.json is JSON");
    json["parts"]
        .as_object()
        .expect("baseline.json has parts")
        .iter()
        .map(|(part, entry)| {
            let triangles = entry["triangles"].as_u64().expect("a budget") as u32;
            let rows = entry["levels"]
                .as_array()
                .expect("levels")
                .iter()
                .map(|row| {
                    let value = |at: usize| row[at].as_f64().expect("a figure") as f32;
                    (value(0), value(1), value(2))
                })
                .collect();
            (part.clone(), (triangles, rows))
        })
        .collect()
}

/// The stack the CAD baseline was measured with (`regularize`): a Reduce to
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

/// Every kit part in `extension`, at its budget and unwrapped by `unwrap`,
/// against its baseline rows: stretch p95 no more than 0.05 over Blender's, the
/// density spread no wider.
fn assert_kit_meets_baseline(extension: &str, unwrap: &UnwrapParams) {
    let baseline = baseline();
    for path in common::cad_kit(extension) {
        let part = part_name(&path);
        let (triangles, rows) = baseline
            .get(&part)
            .unwrap_or_else(|| panic!("{part} has no row in baseline.json"));
        let result = common::run(&load(&path), &stack(*triangles, unwrap.clone()));
        assert!(result.warnings.is_empty(), "{part}: {:?}", result.warnings);
        for (lod, &(stretch_p95, density_p5, density_p95)) in result.lods.iter().zip(rows) {
            let (stretch, low, high) = measure(&lod.model);
            assert!(
                stretch <= stretch_p95 + 0.05,
                "{part}.{extension} LOD{}: stretch p95 {stretch} against {stretch_p95}",
                lod.level
            );
            assert!(
                high - low <= density_p95 - density_p5 + 0.002,
                "{part}.{extension} LOD{}: density {low}-{high} against {density_p5}-{density_p95}",
                lod.level
            );
        }
    }
}

/// The kit's glb parts, cut into the CAD regions `_CAD_REGION` carries.
#[test]
#[ignore = "needs the CAD kit: set REVIEW_CAD_KIT"]
fn the_cad_kit_unwraps_at_least_as_well_as_its_baseline() {
    assert_kit_meets_baseline("glb", &UnwrapParams::cad_islands("_CAD_REGION"));
}

/// The FBX route carries the CAD face id as U of a second UV set and the
/// region as U of a third, which the simplifier refits a little: cut along the
/// third, the FBX parts meet the same baseline.
#[test]
#[ignore = "needs the CAD kit: set REVIEW_CAD_KIT"]
fn the_cad_kit_as_fbx_cuts_islands_along_an_id_in_a_uv_set() {
    let unwrap = UnwrapParams {
        seams: SeamSource::UvSetU(2),
        ..UnwrapParams::cad_islands("unused")
    };
    assert_kit_meets_baseline("fbx", &unwrap);
}

/// An atlas is in the 0-1 square, in the set asked for, and adds that set to a
/// model that had none.
#[test]
fn an_atlas_fills_the_unit_square_in_its_own_set() {
    let Some(model) = common::fixture("meter_cube.fbx") else {
        return;
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Unwrap(UnwrapParams {
        layout: UvLayout::Atlas,
        target_set: 1,
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

/// Per triangle of `level`, its island in UV set 0: triangles are joined
/// through a corner with the same position and UV.
fn islands(level: &ModelData) -> Vec<usize> {
    let count = level.indices.len() / 3;
    let key = |corner: u32| {
        let vertex = corner as usize;
        let p = level.vertices[vertex].position;
        let uv = level.uv_for_channel(vertex, 0);
        [p.x, p.y, p.z, uv.x, uv.y].map(f32::to_bits)
    };
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut first: std::collections::HashMap<[u32; 5], usize> = Default::default();
    for triangle in 0..count {
        for &corner in &level.indices[triangle * 3..triangle * 3 + 3] {
            let other = *first.entry(key(corner)).or_insert(triangle);
            let (x, y) = (root(&mut parent, triangle), root(&mut parent, other));
            parent[x] = y;
        }
    }
    (0..count)
        .map(|triangle| root(&mut parent, triangle))
        .collect()
}

/// How many distinct values custom attribute `name` takes over `model`.
fn distinct_values(model: &ModelData, name: &str) -> usize {
    let attribute = model
        .custom_attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .unwrap_or_else(|| panic!("no {name}"));
    let mut values: Vec<u32> = (0..model.vertices.len())
        .map(|vertex| {
            let bits = attribute.value(vertex)[0];
            match attribute.format.component {
                ComponentType::F32 => f32::from_bits(bits).round() as u32,
                _ => bits,
            }
        })
        .collect();
    values.sort_unstable();
    values.dedup();
    values.len()
}

/// The CAD regions (faces joined across straight tangent edges) take the seams
/// off straight fillets: on every kit part where a region joins faces, fewer
/// islands than one per CAD face, and no more stretch or density spread.
/// Measured on a filleted part at LOD0: 41 islands against 56, stretch p95
/// 1.000 and density 1.000-1.000 for both. Joining across *curved* tangent
/// edges too gave fewer still and twisted the texture through the toroidal
/// fillets (`docs/GOTCHAS.md`), which is why the regions stop at straight ones.
#[test]
#[ignore = "needs the CAD kit: set REVIEW_CAD_KIT"]
fn the_cad_kit_regions_join_straight_fillets_without_stretch() {
    let baseline = baseline();
    let per_face = UnwrapParams {
        existing_seams: ExistingSeams::Cut,
        ..UnwrapParams::cad_islands("_CAD_FACE")
    };
    let mut joined = 0;
    for path in common::cad_kit("glb") {
        let part = part_name(&path);
        let model = load(&path);
        // A part whose every region is one face has nothing to compare.
        if distinct_values(&model, "_CAD_REGION") == distinct_values(&model, "_CAD_FACE") {
            continue;
        }
        joined += 1;
        let (triangles, _) = baseline[&part];
        let unwrap = |params: UnwrapParams| {
            let result = common::run(&model, &stack(triangles, params));
            assert!(result.warnings.is_empty(), "{part}: {:?}", result.warnings);
            let level = result.lods[0].model.clone();
            let mut distinct = islands(&level);
            distinct.sort_unstable();
            distinct.dedup();
            (distinct.len(), measure(&level))
        };
        let (faces, (face_stretch, face_low, face_high)) = unwrap(per_face.clone());
        let (regions, (stretch, low, high)) = unwrap(UnwrapParams::cad_islands("_CAD_REGION"));
        assert!(
            regions < faces,
            "{part}: {regions} region islands against {faces} face islands"
        );
        assert!(
            stretch <= face_stretch + 0.01,
            "{part}: stretch p95 {stretch} against {face_stretch} cut at faces"
        );
        assert!(
            high - low <= face_high - face_low + 0.002,
            "{part}: density {low}-{high} against {face_low}-{face_high} cut at faces"
        );
    }
    assert!(joined > 0, "no kit part has a region of several faces");
}

/// Whether two UV triangles overlap: an edge of one crosses an edge of the
/// other, or one's centre is inside the other.
fn triangles_overlap(a: [Vec2; 3], b: [Vec2; 3]) -> bool {
    let side = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    let crosses = |p: Vec2, q: Vec2, r: Vec2, s: Vec2| {
        let eps = 1.0e-9;
        let (d1, d2) = (side(r, s, p), side(r, s, q));
        let (d3, d4) = (side(p, q, r), side(p, q, s));
        ((d1 > eps && d2 < -eps) || (d1 < -eps && d2 > eps))
            && ((d3 > eps && d4 < -eps) || (d3 < -eps && d4 > eps))
    };
    let inside = |p: Vec2, t: [Vec2; 3]| {
        let sign = side(t[0], t[1], t[2]).signum();
        (0..3).all(|k| side(t[k], t[(k + 1) % 3], p) * sign > 1.0e-9)
    };
    (0..3).any(|i| (0..3).any(|j| crosses(a[i], a[(i + 1) % 3], b[j], b[(j + 1) % 3])))
        || inside((a[0] + a[1] + a[2]) / 3.0, b)
        || inside((b[0] + b[1] + b[2]) / 3.0, a)
}

/// The monkey unwrapped from scratch, with its source-property capture (as
/// the viewer runs it: the polygons ride along) or without.
fn unwrapped_monkey(capture: bool) -> Option<ModelData> {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Unwrap(UnwrapParams {
        reference_set: None,
        ..UnwrapParams::default()
    }));
    let result = if capture {
        let (model, extras) = common::fixture_full("monkey.fbx")?;
        common::run_with_extras(&model, &extras, &stack)
    } else {
        common::run(&common::fixture("monkey.fbx")?, &stack)
    };
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    Some(result.lods[0].model.clone())
}

/// An organic model with no seams worth keeping (Blender's Suzanne,
/// flat-shaded, so every face has vertices of its own) unwraps into a few
/// islands — not the hundred-odd slivers xatlas's segmentation leaves — none
/// overlapping, none folded, all wound one way, all but a sliver of it within
/// the stretch the limit allows.
#[test]
fn an_organic_model_unwraps_into_few_clean_islands() {
    if let Some(level) = unwrapped_monkey(false) {
        assert_clean(&level);
    }
}

/// With the polygons riding along, no seam may run inside a quad: the
/// exporter writes one UV per polygon corner, so a quad cut along its diagonal
/// had one triangle take the other's UVs and stretch across the whole atlas.
/// Measured before the fix: 14 such triangles on the monkey, spanning up to
/// the square's full width.
#[test]
fn polygons_stay_whole_when_unwrapped() {
    if let Some(level) = unwrapped_monkey(true) {
        assert_clean(&level);
    }
}

/// The most islands the monkey may come out in. Measured at the default
/// settings: 16, and 18 with the polygons riding along — four of them the
/// faces set apart round the doubled triangles on its centre line. The
/// shatter-and-merge segmentation this replaced left 23 and 28.
const MONKEY_ISLANDS: usize = 20;
/// The monkey's area-weighted stretch, 95th percentile, at most. Measured
/// 1.40 and 1.43 at the default limit of 1.2.
const MONKEY_STRETCH_P95: f32 = 1.5;
/// The most any one triangle of the monkey may stretch. Measured 2.7 and 3.1;
/// a triangle whose corners came from two islands stretches far further.
const MONKEY_WORST: f32 = 4.0;

/// The checks both runs of the monkey must pass.
fn assert_clean(level: &ModelData) {
    let count = level.indices.len() / 3;
    let corners = |triangle: usize| &level.indices[triangle * 3..triangle * 3 + 3];
    let uvs: Vec<[Vec2; 3]> = (0..count)
        .map(|triangle| {
            corners(triangle)
                .iter()
                .map(|&corner| level.uv_for_channel(corner as usize, 0))
                .collect::<Vec<_>>()
                .try_into()
                .unwrap()
        })
        .collect();

    for uv in uvs.iter().flatten() {
        assert!(
            (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y),
            "{uv} is outside the square"
        );
    }
    for (triangle, uv) in uvs.iter().enumerate() {
        let area = (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]);
        assert!(
            area > 0.0,
            "triangle {triangle} is folded or wound the other way"
        );
    }

    let island = islands(level);
    let distinct: std::collections::HashSet<usize> = island.iter().copied().collect();
    assert!(
        distinct.len() <= MONKEY_ISLANDS,
        "{} islands",
        distinct.len()
    );

    for a in 0..count {
        for b in a + 1..count {
            let touching = (0..3).any(|i| (0..3).any(|j| uvs[a][i] == uvs[b][j]));
            if touching && island[a] == island[b] {
                continue;
            }
            assert!(
                !triangles_overlap(uvs[a], uvs[b]),
                "triangles {a} and {b} overlap"
            );
        }
    }

    let (stretch, _, _) = measure(level);
    assert!(stretch < MONKEY_STRETCH_P95, "stretch p95 {stretch}");
    // Every triangle within what a flattening that is one-to-one can do: a
    // triangle whose corners came from two islands is a long sliver across the
    // atlas, far past this.
    for (triangle, uv) in uvs.iter().enumerate() {
        let p = corners(triangle)
            .iter()
            .map(|&corner| level.vertices[corner as usize].position)
            .collect::<Vec<Vec3>>();
        if let Some(stretch) = review_model::measure::uv_stretch([p[0], p[1], p[2]], *uv) {
            assert!(
                stretch < MONKEY_WORST,
                "triangle {triangle} stretches {stretch}"
            );
        }
    }
}
