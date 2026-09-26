//! Export tests that check the written file by *reading it back* through the
//! vendored ufbx importer.
//!
//! Asserting on the writer's own return value would only prove it did not
//! error. ufbx_write is upstream work-in-progress, so what matters is whether a
//! real FBX reader recovers the mesh — which is also what catches a breaking
//! change when the vendored copy is refreshed.

#![cfg(all(has_meshopt, has_ufbxw))]

use review_model::ModelData;
use review_optimize::{
    AoQuality, AoTarget, BakeAoParams, ExportOptions, FbxFormat, HierarchyMode, LodLevel,
    LodPackaging, LodParams, OpKind, OptStack, SimplifyAlgorithm, SimplifySettings, WeldParams,
    export_fbx,
};

mod common;

use common::{array_size, fixture, reimport, run, scene_property, temp_dir};

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
        simplify: SimplifySettings {
            algorithm: SimplifyAlgorithm::Standard,
            ..SimplifySettings::default()
        },
        levels: vec![LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        }],
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
    let report = export_fbx(&result.lods, &model, None, &path, &ExportOptions::default())
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
fn a_path_with_non_ascii_characters_is_written_where_it_says() {
    // ufbx_write's own file writer opens with the narrow `fopen`, which on Windows
    // reads the path in the ANSI code page: a folder named for its user (`Zoë`) or
    // a file named in any other script failed to open, although the reader opens
    // the same path fine. The bridge now opens the file itself.
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &passthrough());

    let dir = temp_dir("round_trip_Zoë_日本語");
    let path = dir.join("modèle_テスト.fbx");
    let report = export_fbx(&result.lods, &model, None, &path, &ExportOptions::default())
        .expect("export succeeds");

    assert_eq!(report.files, vec![path.clone()]);
    assert!(path.exists(), "the file was written under its own name");
    let loaded = reimport(&path);
    assert_eq!(
        loaded.indices.len(),
        result.lods[0].model.indices.len(),
        "the triangle count survives the round trip"
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
        None,
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
    // "Sibling" is literal: the levels share one parent, not per-level copies
    // of the ancestor chain.
    let mesh_parents: std::collections::HashSet<Option<usize>> = loaded
        .nodes
        .iter()
        .filter(|node| node.mesh_part.is_some())
        .map(|node| node.parent)
        .collect();
    assert_eq!(
        mesh_parents.len(),
        1,
        "both levels hang off the same parent node: {names:?}"
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
        None,
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
            None,
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
    export_fbx(&result.lods, &model, None, &path, &ExportOptions::default())
        .expect("export succeeds");

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
        None,
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

/// Export `name` as ASCII and hand back the text, or `None` when the fixture is
/// absent. ASCII because these two tests are about what the *file* says, which a
/// re-import normalizes away: a reader resolves the declared unit and flattens
/// an indexed layer, so neither a wrong `UnitScaleFactor` paired with matching
/// coordinates nor an unreadable-but-well-formed layer shape shows up in the
/// model that comes back.
fn ascii_export(name: &str, dir_name: &str) -> Option<String> {
    let model = fixture(name)?;
    let result = run(&model, &passthrough());
    let dir = temp_dir(dir_name);
    let path = dir.join("out.fbx");
    export_fbx(
        &result.lods,
        &model,
        None,
        &path,
        &ExportOptions {
            format: FbxFormat::Ascii,
            ..ExportOptions::default()
        },
    )
    .expect("export succeeds");
    Some(std::fs::read_to_string(&path).expect("ASCII output is text"))
}

#[test]
fn the_written_file_declares_the_sources_own_unit() {
    // FBX declares its unit rather than fixing one, and import normalizes every
    // file to meters — so an export has to say which unit its coordinates are
    // in. Writing the source's own unit is what keeps a centimeter asset a
    // centimeter asset through the tool. Both fixtures are checked: a wrong
    // hard-coded factor would still pass over one of them.
    for (name, expected) in [("SM_Ammo_Crate_01a.fbx", "1"), ("SM_column04.fbx", "2.54")] {
        let Some(text) = ascii_export(name, "round_trip_unit") else {
            continue;
        };
        assert_eq!(
            scene_property(&text, "UnitScaleFactor"),
            expected,
            "{name} should be written in the unit it was authored in"
        );
    }
    // The declared factor and the coordinates have to agree, which is what the
    // bounds round trips above measure: they re-import (back into meters) and
    // compare against the source's metric bounds, so declaring a unit without
    // scaling the geometry — or the reverse — moves them by 100x.
}

#[test]
fn uv_and_color_layers_are_polygon_vertex_indexed() {
    // `ByVertice` + `IndexToDirect` — what ufbx_write's non-indexed UV/color
    // setters produce, since they dedup the values and emit an index array as
    // long as the value array — is legal FBX that readers assuming a
    // per-polygon-vertex index cannot load. Unity is one: it rejects the mesh
    // with "has invalid UV coordinates" and blames the exporting tool. Assert
    // the shape every DCC writes instead, including the index array length that
    // makes it unambiguous.
    let Some(text) = ascii_export("SM_Ammo_Crate_01a.fbx", "round_trip_layers") else {
        return;
    };

    let mut polygon_indices = 0usize;
    let mut layer: Option<&str> = None;
    let mut checked = 0usize;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("PolygonVertexIndex: *") {
            polygon_indices = array_size(rest);
        } else if line.starts_with("LayerElementUV:") {
            layer = Some("UV");
        } else if line.starts_with("LayerElementColor:") {
            layer = Some("Color");
        } else if line.starts_with("LayerElement") {
            layer = None;
        } else if let Some(which) = layer {
            if let Some(mapping) = line.strip_prefix("MappingInformationType: ") {
                assert_eq!(mapping, "\"ByPolygonVertex\"", "{which} layer mapping");
            } else if let Some(reference) = line.strip_prefix("ReferenceInformationType: ") {
                assert_eq!(
                    reference, "\"IndexToDirect\"",
                    "{which} layer reference mode"
                );
            } else if let Some(rest) = line
                .strip_prefix("UVIndex: *")
                .or_else(|| line.strip_prefix("ColorIndex: *"))
            {
                assert_eq!(
                    array_size(rest),
                    polygon_indices,
                    "{which} layer indexes every polygon vertex exactly once"
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 2,
        "the fixture should have contributed UV and color layers to check, saw {checked}"
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
    export_fbx(&result.lods, &model, None, &path, &ExportOptions::default())
        .expect("export succeeds");

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

/// Baked AO must survive the file: the writer already emits the vertex-color
/// set, so what this pins is that the *baked* values — not a default white
/// layer — are what a real FBX reader recovers.
#[test]
fn baked_ao_survives_an_export_round_trip() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let mut stack = OptStack::default();
    stack.push_op(OpKind::BakeAo(BakeAoParams {
        quality: AoQuality::Low,
        target: AoTarget::Rgb,
        ..BakeAoParams::default()
    }));
    let result = run(&model, &stack);

    let dir = temp_dir("round_trip_bake_ao");
    let path = dir.join("baked.fbx");
    export_fbx(&result.lods, &model, None, &path, &ExportOptions::default())
        .expect("export succeeds");

    let loaded = reimport(&path);
    let (mut lowest, mut highest) = (f32::INFINITY, f32::NEG_INFINITY);
    for vertex in &loaded.vertices {
        assert!(
            (0.0..=1.0).contains(&vertex.vertex_color.x),
            "read-back colors stay real values, got {}",
            vertex.vertex_color.x
        );
        lowest = lowest.min(vertex.vertex_color.x);
        highest = highest.max(vertex.vertex_color.x);
    }
    assert!(
        highest - lowest > 0.1,
        "the read-back color set carries the bake's variation, not a uniform \
         layer: {lowest}..{highest}"
    );
}

/// Shading has to survive the file, and the way it stops doing so is silent:
/// ufbx falls a zero-length normal back to a constant `(0, 1, 0)`, which reads
/// as a lit but featureless mesh rather than as an error. Two things sank it,
/// both only visible on a non-metric source — its node transforms carry the
/// unit normalization import parked there, so applying `per_meter` to the
/// *local* coordinates as well shrank the whole hierarchy by a second factor of
/// 100 and, on re-import, the cofactor matrix with it, until every normal
/// underflowed the reader's epsilon.
#[test]
fn normals_survive_the_round_trip() {
    // A centimeter fixture: at a metric source the two factors are both 1 and
    // the bug cannot appear at all.
    let Some(model) = fixture("rock_pillar_03.fbx") else {
        return;
    };
    let result = run(&model, &passthrough());

    let dir = temp_dir("round_trip_normals");
    let path = dir.join("normals.fbx");
    export_fbx(&result.lods, &model, None, &path, &ExportOptions::default())
        .expect("export succeeds");

    let loaded = reimport(&path);
    // Every normal against the geometric normal of a face it belongs to. The
    // comparison is deliberately not source-against-export vertex by vertex:
    // the indexing pass renumbers the buffer, so what is checkable is that the
    // normals still agree with the surface they sit on — which a constant
    // fallback, or a normal transformed the wrong way round, does not.
    let agreement = |mesh: &ModelData| {
        let (mut total, mut corners) = (0.0f64, 0usize);
        for triangle in 0..mesh.indices.len() / 3 {
            let corner: Vec<_> = (0..3)
                .map(|offset| mesh.indices[triangle * 3 + offset] as usize)
                .collect();
            let position: Vec<_> = corner.iter().map(|&i| mesh.vertices[i].position).collect();
            let face = (position[1] - position[0])
                .cross(position[2] - position[0])
                .normalize_or_zero();
            if face == glam::Vec3::ZERO {
                continue;
            }
            for &i in &corner {
                total += f64::from(face.dot(mesh.vertices[i].normal));
                corners += 1;
            }
        }
        total / corners as f64
    };

    let source = agreement(&model);
    let written = agreement(&loaded);
    assert!(
        source > 0.5,
        "the fixture's own normals face its surface: {source}"
    );
    assert!(
        written > source - 0.05,
        "the written normals agree with the surface as well as the source's do:          {written} against {source}"
    );
}

/// ufbx_write seeds every scene it creates with a "Take 001" stack and its
/// BaseLayer, so a static mesh came back out of the tool carrying an empty
/// animation — listed in the Outliner's Animations tab and shown as a take by
/// every DCC. The export writes no animation at all, so neither has anything to
/// hold.
#[test]
fn no_animation_is_written_for_a_static_mesh() {
    let Some(text) = ascii_export("rock_pillar_03.fbx", "round_trip_no_anim") else {
        return;
    };
    assert!(
        !text.contains("AnimationStack"),
        "a static export declares no animation stack"
    );
    assert!(
        !text.contains("AnimationLayer"),
        "a static export declares no animation layer"
    );
}

#[test]
fn exporting_nothing_is_an_error_rather_than_an_empty_file() {
    let dir = temp_dir("round_trip_empty");
    let path = dir.join("empty.fbx");
    let error = export_fbx(
        &[],
        &ModelData::default(),
        None,
        &path,
        &ExportOptions::default(),
    )
    .expect_err("an empty chain cannot be exported");
    assert!(
        matches!(error, review_optimize::OptError::EmptyMesh),
        "got {error:?}"
    );
    assert!(!path.exists(), "no file is left behind");
}

#[test]
fn a_successful_export_leaves_no_staging_file_behind() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &passthrough());

    let dir = temp_dir("round_trip_replaces");
    let path = dir.join("asset.fbx");
    // A previous export, which this one replaces.
    std::fs::write(&path, b"the previous export").expect("seed the destination");

    export_fbx(&result.lods, &model, None, &path, &ExportOptions::default())
        .expect("export succeeds");

    assert!(
        reimport(&path).indices.len() > 3,
        "the destination holds the new export"
    );
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read the export directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".partial-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "staging files left behind: {leftovers:?}"
    );
}

#[test]
fn a_chain_that_cannot_replace_a_later_file_reports_what_it_did_replace() {
    let Some(model) = fixture("monkey.fbx") else {
        return;
    };
    let result = run(&model, &weld_and_halve());

    let dir = temp_dir("round_trip_incomplete");
    let path = dir.join("asset.fbx");
    // LOD 1's destination is a directory, so its rename fails while LOD 0's has
    // already gone through — the partial-replacement case the user has to be
    // told about by name.
    std::fs::create_dir(dir.join("asset_LOD1.fbx")).expect("block the second destination");

    let error = export_fbx(
        &result.lods,
        &model,
        None,
        &path,
        &ExportOptions {
            packaging: LodPackaging::FilePerLod,
            ..ExportOptions::default()
        },
    )
    .expect_err("the second level cannot be put in place");

    match error {
        review_optimize::OptError::ExportIncomplete {
            replaced, failed, ..
        } => {
            assert_eq!(replaced, vec![dir.join("asset_LOD0.fbx")]);
            assert_eq!(failed, dir.join("asset_LOD1.fbx"));
        }
        other => panic!("expected ExportIncomplete, got {other:?}"),
    }

    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("read the export directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".partial-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "an abandoned level's staging file must not survive: {leftovers:?}"
    );
}
