//! Export tests that check the written file by *reading it back* through the
//! vendored ufbx importer.
//!
//! Asserting on the writer's own return value would only prove it did not
//! error. ufbx_write is upstream work-in-progress, so what matters is whether a
//! real FBX reader recovers the mesh — which is also what catches a breaking
//! change when the vendored copy is refreshed.

#![cfg(all(has_meshopt, has_ufbxw))]

use std::path::{Path, PathBuf};

use review_model::ModelData;
use review_optimize::{
    ExportOptions, FbxFormat, HierarchyMode, LodLevel, LodPackaging, LodParams, OpKind, OptStack,
    ProcessInput, ProcessedResult, SimplifyAlgorithm, WeldParams, export_fbx, process,
};

const VERTEX_SIZE: usize = 64;

/// A per-test temporary directory under the target dir, removed and recreated so
/// a rerun never sees the previous run's files.
fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp directory");
    dir
}

fn fixture(name: &str) -> Option<ModelData> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test_models")
        .join(name);
    if !path.exists() {
        eprintln!("skipping {name}: the fixture is not present");
        return None;
    }
    match review_import::load_model(&path) {
        Ok(model) => Some(model),
        // No vendored ufbx means nothing can be imported at all — the same "this
        // checkout can't run these" situation as a missing fixture. Anything else
        // is a file that is present and did not load, which must be reported.
        Err(review_import::ImportError::UfbxUnavailable) => {
            eprintln!("skipping {name}: FBX import is unavailable in this build");
            None
        }
        Err(error) => panic!("{name} is present but failed to load: {error}"),
    }
}

fn run(model: &ModelData, stack: &OptStack) -> ProcessedResult {
    process(ProcessInput {
        model,
        stack,
        render_vertex_size: VERTEX_SIZE,
    })
    .expect("processing succeeds")
}

/// Re-import a written file, failing with the reader's own message.
fn reimport(path: &Path) -> ModelData {
    review_import::load_model(path)
        .unwrap_or_else(|error| panic!("could not read back {}: {error}", path.display()))
}

/// The smallest stack that still produces a chain to export: a run with nothing
/// enabled measures the source and produces no levels, so the export tests need
/// *an* operation, and filtering leaves a clean mesh's triangles alone.
fn passthrough() -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::FilterTriangles);
    stack
}

/// A stack that welds and then halves the triangle count.
fn weld_and_halve() -> OptStack {
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
    stack
}

#[test]
fn a_written_file_reads_back_with_the_same_geometry() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &passthrough());
    let expected = &result.lods[0].model;

    let dir = temp_dir("round_trip_geometry");
    let path = dir.join("monkey.fbx");
    let report = export_fbx(&result.lods, &model, &path, &ExportOptions::default())
        .expect("export succeeds");

    assert_eq!(report.files, vec![path.clone()]);
    assert!(path.exists(), "the file was written");

    let loaded = reimport(&path);
    assert_eq!(
        loaded.indices.len() / 3,
        expected.indices.len() / 3,
        "the triangle count survives the round trip"
    );
    // The reader corner-expands on import, so vertex counts are not comparable;
    // the geometry's extent is, and it is what would drift if positions were
    // mangled or transformed twice.
    let (Some(before), Some(after)) = (expected.bounds, loaded.bounds) else {
        panic!("both meshes should have bounds");
    };
    let tolerance = before.size().max_element() * 1.0e-3;
    assert!(
        before.min.abs_diff_eq(after.min, tolerance)
            && before.max.abs_diff_eq(after.max, tolerance),
        "bounds drifted: {before:?} -> {after:?}"
    );
}

#[test]
fn a_lod_chain_writes_suffixed_sibling_nodes_into_one_file() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &weld_and_halve());
    assert_eq!(result.lods.len(), 2);

    let dir = temp_dir("round_trip_suffixed");
    let path = dir.join("asset.fbx");
    export_fbx(
        &result.lods,
        &model,
        &path,
        &ExportOptions {
            packaging: LodPackaging::SingleFileSuffixed,
            ..ExportOptions::default()
        },
    )
    .expect("export succeeds");

    let loaded = reimport(&path);
    let names: Vec<&str> = loaded.nodes.iter().map(|node| node.name.as_str()).collect();
    assert!(
        names.iter().any(|name| name.ends_with("_LOD1")),
        "the second level is suffixed: {names:?}"
    );
    assert_eq!(
        loaded.indices.len() / 3,
        result
            .lods
            .iter()
            .map(|lod| lod.model.stats.triangle_count)
            .sum::<usize>(),
        "both levels' triangles are present"
    );
}

#[test]
fn one_file_per_lod_writes_a_numbered_file_for_each_level() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &weld_and_halve());

    let dir = temp_dir("round_trip_per_lod");
    let path = dir.join("asset.fbx");
    let report = export_fbx(
        &result.lods,
        &model,
        &path,
        &ExportOptions {
            packaging: LodPackaging::FilePerLod,
            ..ExportOptions::default()
        },
    )
    .expect("export succeeds");

    assert_eq!(report.files.len(), 2);
    assert_eq!(report.files[0], dir.join("asset_LOD0.fbx"));
    assert_eq!(report.files[1], dir.join("asset_LOD1.fbx"));

    for (level, file) in report.files.iter().enumerate() {
        let loaded = reimport(file);
        assert_eq!(
            loaded.indices.len() / 3,
            result.lods[level].model.stats.triangle_count,
            "{} holds exactly its own level",
            file.display()
        );
    }
}

#[test]
fn rebuilt_and_flat_hierarchies_place_the_geometry_identically() {
    // The two modes differ in where the transform lives — on the node, or baked
    // into the vertices — so a correctly written file puts the geometry in the
    // same world position either way. This is the check that catches un-baking
    // applied in the wrong direction, which is otherwise invisible until an
    // artist opens the file.
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let result = run(&model, &passthrough());
    let dir = temp_dir("round_trip_hierarchy");

    let mut bounds = Vec::new();
    for (label, hierarchy) in [
        ("rebuild", HierarchyMode::Rebuild),
        ("flat", HierarchyMode::FlatBaked),
    ] {
        let path = dir.join(format!("{label}.fbx"));
        export_fbx(
            &result.lods,
            &model,
            &path,
            &ExportOptions {
                hierarchy,
                ..ExportOptions::default()
            },
        )
        .expect("export succeeds");
        bounds.push((label, reimport(&path).bounds.expect("bounds")));
    }

    let source = result.lods[0].model.bounds.expect("source bounds");
    let tolerance = source.size().max_element() * 1.0e-2;
    for (label, written) in &bounds {
        assert!(
            source.min.abs_diff_eq(written.min, tolerance)
                && source.max.abs_diff_eq(written.max, tolerance),
            "{label}: geometry landed in the wrong place: {source:?} -> {written:?}"
        );
    }
}

#[test]
fn a_multi_material_mesh_keeps_its_materials() {
    let Some(model) = fixture("SK_Player_01.fbx") else {
        return;
    };
    assert!(model.materials.len() > 1);
    let result = run(&model, &passthrough());

    let dir = temp_dir("round_trip_materials");
    let path = dir.join("materials.fbx");
    export_fbx(&result.lods, &model, &path, &ExportOptions::default()).expect("export succeeds");

    let loaded = reimport(&path);
    assert!(
        loaded.materials.len() > 1,
        "several materials survive, found {}",
        loaded.materials.len()
    );
    let distinct: std::collections::HashSet<u32> =
        loaded.triangles.material.iter().copied().collect();
    assert!(
        distinct.len() > 1,
        "faces are still assigned to different materials: {distinct:?}"
    );
}

#[test]
fn ascii_output_reads_back_too() {
    let Some(model) = fixture("meter_cube.fbx") else {
        return;
    };
    let result = run(&model, &passthrough());

    let dir = temp_dir("round_trip_ascii");
    let path = dir.join("cube.fbx");
    export_fbx(
        &result.lods,
        &model,
        &path,
        &ExportOptions {
            format: FbxFormat::Ascii,
            ..ExportOptions::default()
        },
    )
    .expect("export succeeds");

    let text = std::fs::read_to_string(&path).expect("ASCII output is text");
    assert!(text.contains("Objects:"), "the file looks like ASCII FBX");
    assert_eq!(
        reimport(&path).indices.len() / 3,
        12,
        "a cube is 12 triangles"
    );
}

#[test]
fn uv_sets_survive_the_round_trip() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let source_sets = model.stats.uv_set_count;
    if source_sets == 0 {
        return;
    }
    let result = run(&model, &passthrough());

    let dir = temp_dir("round_trip_uvs");
    let path = dir.join("uvs.fbx");
    export_fbx(&result.lods, &model, &path, &ExportOptions::default()).expect("export succeeds");

    let loaded = reimport(&path);
    assert_eq!(
        loaded.stats.uv_set_count, source_sets,
        "every UV set is written and read back"
    );
    assert!(
        loaded
            .vertices
            .iter()
            .any(|vertex| vertex.uv.x != 0.0 || vertex.uv.y != 0.0),
        "the UVs carry real values, not zeros"
    );
}

#[test]
fn exporting_nothing_is_an_error_rather_than_an_empty_file() {
    let dir = temp_dir("round_trip_empty");
    let path = dir.join("empty.fbx");
    let error = export_fbx(&[], &ModelData::default(), &path, &ExportOptions::default())
        .expect_err("an empty chain cannot be exported");
    assert!(
        matches!(error, review_optimize::OptError::EmptyMesh),
        "got {error:?}"
    );
    assert!(!path.exists(), "no file is left behind");
}
