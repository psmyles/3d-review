//! Round trips of the source-property capture: what `SourceExtras` carries in
//! must come back out of a written file, read through the vendored ufbx reader
//! and its own capture.
//!
//! The fixtures cover real game assets (a skinned, textured character; a Phong
//! rock); the vendored writer's patch probe (`src/ufbxw_probe.c`) supplies the
//! things no fixture authors — a `Null`, a LOD group, a camera, user properties
//! of every kind — by being imported and re-exported like any other file.

#![cfg(all(has_meshopt, has_ufbxw))]

use std::path::{Path, PathBuf};

use review_model::extras::{AttributeKind, ElementRef, Prop, PropType, Synthetic};
use review_model::{ModelData, NodeKind, SourceExtras};
use review_optimize::{
    BakeAoParams, ExportOptions, FbxFormat, HierarchyMode, LodLevel, OpKind, OptStack,
    ProcessInput, ProcessedResult, ReduceParams, SimplifySettings, WeldParams, export_fbx, process,
};

const VERTEX_SIZE: usize = 64;

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp directory");
    dir
}

/// A fixture with its capture, or `None` when this checkout cannot run the test.
fn fixture(name: &str) -> Option<(ModelData, SourceExtras)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test_models")
        .join(name);
    if !path.exists() {
        eprintln!("skipping {name}: the fixture is not present");
        return None;
    }
    load(&path)
}

fn load(path: &Path) -> Option<(ModelData, SourceExtras)> {
    match review_import::load_model_full(path) {
        Ok((model, Some(extras))) => Some((model, extras)),
        Ok((_, None)) => {
            eprintln!("skipping {}: no capture in this build", path.display());
            None
        }
        Err(review_import::ImportError::UfbxUnavailable) => {
            eprintln!("skipping {}: FBX import is unavailable", path.display());
            None
        }
        Err(error) => panic!("{} is present but failed to load: {error}", path.display()),
    }
}

fn reload(path: &Path) -> (ModelData, SourceExtras) {
    load(path).unwrap_or_else(|| panic!("could not read back {}", path.display()))
}

fn run(model: &ModelData, extras: &SourceExtras, stack: &OptStack) -> ProcessedResult {
    process(ProcessInput {
        model,
        stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: Some(extras),
    })
    .expect("processing succeeds")
}

fn passthrough(model: &ModelData, extras: &SourceExtras) -> ProcessedResult {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::FilterTriangles);
    process(ProcessInput {
        model,
        stack: &stack,
        render_vertex_size: VERTEX_SIZE,
        hidden_nodes: &[],
        extras: Some(extras),
    })
    .expect("processing succeeds")
}

/// Export the passthrough of a fixture with its capture and read it back.
fn round_trip(
    model: &ModelData,
    extras: &SourceExtras,
    dir: &Path,
    format: FbxFormat,
) -> (PathBuf, ModelData, SourceExtras) {
    let result = passthrough(model, extras);
    let path = dir.join(match format {
        FbxFormat::Binary => "round_trip.fbx",
        FbxFormat::Ascii => "round_trip_ascii.fbx",
    });
    let report = export_fbx(
        &result.lods,
        model,
        Some(extras),
        &path,
        &ExportOptions {
            format,
            hierarchy: HierarchyMode::Rebuild,
            ..ExportOptions::default()
        },
    )
    .expect("export succeeds");
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.contains("had not finished loading")),
        "the capture was used: {:?}",
        report.notes
    );
    let (loaded, loaded_extras) = reload(&path);
    (path, loaded, loaded_extras)
}

fn node_by_name<'a>(
    model: &'a ModelData,
    name: &str,
) -> Option<(usize, &'a review_model::SceneNode)> {
    model
        .nodes
        .iter()
        .enumerate()
        .find(|(_, node)| node.name == name)
}

fn prop<'a>(props: &'a [Prop], name: &str) -> &'a Prop {
    props
        .iter()
        .find(|prop| prop.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no property {name:?} in {:?}",
                props.iter().map(|p| &p.name).collect::<Vec<_>>()
            )
        })
}

fn assert_prop_eq(source: &Prop, loaded: &Prop, context: &str) {
    assert_eq!(source.name, loaded.name, "{context}");
    let reals = source.flags.real_count();
    for component in 0..reals {
        assert!(
            (source.value_real[component] - loaded.value_real[component]).abs() < 1e-6,
            "{context} {}[{component}]: {} vs {}",
            source.name,
            source.value_real[component],
            loaded.value_real[component]
        );
    }
    if source.flags.has_int() && !source.flags.has_string() {
        assert_eq!(
            source.value_int, loaded.value_int,
            "{context} {}",
            source.name
        );
    }
    if source.flags.has_string() && source.kind == PropType::Text {
        assert_eq!(
            source.value_str, loaded.value_str,
            "{context} {}",
            source.name
        );
    }
    assert_eq!(
        source.flags.user_defined(),
        loaded.flags.user_defined(),
        "{context} {} user flag",
        source.name
    );
}

/// Every authored property of `source` is present in `loaded` with the same
/// value. Compound headers have no value and are skipped.
fn assert_props_survive(source: &[Prop], loaded: &[Prop], context: &str) {
    for authored in source {
        if authored.kind == PropType::Compound || !authored.flags.has_value() {
            continue;
        }
        let Some(back) = loaded.iter().find(|prop| prop.name == authored.name) else {
            panic!(
                "{context}: property {:?} was not written back",
                authored.name
            );
        };
        assert_prop_eq(authored, back, context);
    }
}

#[test]
fn every_authored_node_survives_with_its_transform_and_kind() {
    let Some((model, extras)) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let dir = temp_dir("extras_nodes");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);

    let authored: Vec<usize> = (0..model.nodes.len())
        .filter(|&index| extras.nodes[index].synthetic == Synthetic::None)
        .collect();
    let reimported = (0..loaded.nodes.len())
        .filter(|&index| loaded_extras.nodes[index].synthetic == Synthetic::None)
        .count();
    assert_eq!(
        reimported,
        authored.len(),
        "every authored node, and nothing else"
    );

    for index in authored {
        let node = &model.nodes[index];
        let (back_index, back) = node_by_name(&loaded, &node.name)
            .unwrap_or_else(|| panic!("node {:?} was not written", node.name));
        assert_eq!(back.kind, node.kind, "{}", node.name);
        // The authored properties recompose ufbx's own local transform.
        let local = node.rest_local;
        let back_local = back.rest_local;
        assert!(
            local.translation.distance(back_local.translation) < 1e-4,
            "{} translation {:?} vs {:?}",
            node.name,
            local.translation,
            back_local.translation
        );
        assert!(
            local.rotation.dot(back_local.rotation).abs() > 1.0 - 1e-5,
            "{} rotation {:?} vs {:?}",
            node.name,
            local.rotation,
            back_local.rotation
        );
        assert!(
            local.scale.distance(back_local.scale) < 1e-4,
            "{} scale {:?} vs {:?}",
            node.name,
            local.scale,
            back_local.scale
        );
        // The world transform follows, so the hierarchy came back whole.
        let delta = (node.transform - back.transform).abs();
        let max = delta.to_cols_array().into_iter().fold(0f32, f32::max);
        assert!(max < 1e-3, "{} world drifts by {max}", node.name);
        // And every authored property is there, same value, same user flag.
        assert_props_survive(
            &extras.nodes[index].props,
            &loaded_extras.nodes[back_index].props,
            &node.name,
        );
        assert_eq!(
            extras.nodes[index].rotation_order, loaded_extras.nodes[back_index].rotation_order,
            "{} rotation order",
            node.name
        );
    }
}

#[test]
fn material_properties_shading_model_and_textures_survive() {
    let Some((model, extras)) = fixture("SK_Player_01.fbx") else {
        return;
    };
    assert!(!extras.textures.is_empty(), "the fixture carries textures");
    let dir = temp_dir("extras_materials");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);

    assert_eq!(loaded.materials.len(), model.materials.len());
    assert_eq!(
        loaded_extras.textures.len(),
        extras.textures.len(),
        "every texture is written"
    );
    for (index, material) in model.materials.iter().enumerate() {
        let back_index = loaded
            .materials
            .iter()
            .position(|back| back.name == material.name)
            .unwrap_or_else(|| panic!("material {:?} was not written", material.name));
        let authored = &extras.materials[index];
        let back = &loaded_extras.materials[back_index];
        assert_eq!(authored.shader_type, back.shader_type, "{}", material.name);
        assert_props_survive(&authored.props, &back.props, &material.name);
        // The viewer's own reading of the material is unchanged too.
        let loaded_material = &loaded.materials[back_index];
        assert!(
            (loaded_material.smoothness - material.smoothness).abs() < 1e-4,
            "{} smoothness",
            material.name
        );
        assert!(
            (loaded_material.metallic - material.metallic).abs() < 1e-4,
            "{} metallic",
            material.name
        );

        // Texture connections: same properties fed by textures of the same name.
        let mut connections: Vec<(String, String)> = authored
            .textures
            .iter()
            .map(|t| {
                (
                    t.material_prop.clone(),
                    extras.textures[t.texture as usize].name.clone(),
                )
            })
            .collect();
        let mut back_connections: Vec<(String, String)> = back
            .textures
            .iter()
            .map(|t| {
                (
                    t.material_prop.clone(),
                    loaded_extras.textures[t.texture as usize].name.clone(),
                )
            })
            .collect();
        connections.sort();
        back_connections.sort();
        assert_eq!(connections, back_connections, "{} textures", material.name);
    }

    for texture in &extras.textures {
        let back = loaded_extras
            .textures
            .iter()
            .find(|back| back.name == texture.name)
            .unwrap_or_else(|| panic!("texture {:?} was not written", texture.name));
        assert_eq!(texture.kind, back.kind, "{}", texture.name);
        assert_eq!(
            texture.absolute_filename, back.absolute_filename,
            "{}",
            texture.name
        );
        assert_eq!(
            texture.relative_filename, back.relative_filename,
            "{}",
            texture.name
        );
        assert_eq!(texture.uv_set, back.uv_set, "{}", texture.name);
        assert_eq!(texture.wrap_u, back.wrap_u, "{}", texture.name);
        assert_eq!(texture.wrap_v, back.wrap_v, "{}", texture.name);
        assert_eq!(
            texture.content.len(),
            back.content.len(),
            "{} content",
            texture.name
        );
        assert_eq!(
            texture.content, back.content,
            "{} content bytes",
            texture.name
        );
        assert_props_survive(&texture.props, &back.props, &texture.name);
    }
}

#[test]
fn phong_figures_survive_with_and_without_the_capture() {
    let Some((model, extras)) = fixture("rock_pillar_03.fbx") else {
        return;
    };
    let material = &model.materials[0];
    // A shininess exponent of 20 reads as this smoothness; both paths must
    // bring it back, and the metallic written to `ReflectionFactor` too.
    assert!((material.smoothness - 0.447_213_6).abs() < 1e-5);
    let dir = temp_dir("extras_phong");

    let (_, with_capture, with_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    assert!((with_capture.materials[0].smoothness - material.smoothness).abs() < 1e-5);
    // The fixture authors the classic `Shininess` name; it comes back as such.
    let exponent = prop(&with_extras.materials[0].props, "Shininess");
    assert!((exponent.value_real[0] - 20.0).abs() < 1e-6, "{exponent:?}");

    let result = passthrough(&model, &extras);
    let path = dir.join("viewer_path.fbx");
    export_fbx(&result.lods, &model, None, &path, &ExportOptions::default()).expect("export");
    let (without_capture, _) = reload(&path);
    assert!((without_capture.materials[0].smoothness - material.smoothness).abs() < 1e-5);
    assert!((without_capture.materials[0].metallic - material.metallic).abs() < 1e-5);
}

#[test]
fn scene_settings_and_metadata_survive() {
    let Some((model, extras)) = fixture("AN_ZombiedogLocomotion.fbx") else {
        return;
    };
    let dir = temp_dir("extras_settings");
    let (path, _, loaded) = round_trip(&model, &extras, &dir, FbxFormat::Ascii);

    let source = &extras.scene;
    let back = &loaded.scene;
    assert_eq!(back.version, 7700, "always written as FBX 2020");
    assert_eq!(back.axes, source.axes);
    assert!((back.unit_meters - source.unit_meters).abs() < 1e-9);
    assert!((back.frames_per_second - source.frames_per_second).abs() < 1e-6);
    assert_eq!(back.time_mode, source.time_mode);
    assert_eq!(back.time_protocol, source.time_protocol);
    assert_eq!(back.snap_mode, source.snap_mode);
    assert!(back.ambient_color.distance(source.ambient_color) < 1e-6);
    assert_eq!(back.default_camera, source.default_camera);
    assert_eq!(
        back.original_application.name,
        source.original_application.name
    );
    assert_eq!(back.latest_application.name, "3D Review");
    for name in ["TimeSpanStart", "TimeSpanStop"] {
        assert_eq!(
            prop(&source.settings_props, name).value_int,
            prop(&back.settings_props, name).value_int,
            "{name}"
        );
    }
    // Written as the newest revision, stated in the file itself.
    let text = std::fs::read_to_string(&path).expect("read the ASCII export");
    assert!(
        text.lines()
            .any(|line| line.contains("FBXVersion:") && line.contains("7700"))
    );
}

#[test]
fn authored_tangents_are_written_and_synthesized_ones_are_not() {
    let Some((model, extras)) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let dir = temp_dir("extras_tangents");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    for part in &extras.meshes {
        let name = &model.nodes[part.node as usize].name;
        let back = loaded_extras
            .mesh_of_node(node_by_name(&loaded, name).expect("node written").0 as u32)
            .expect("mesh written");
        assert_eq!(part.tangents_authored, back.tangents_authored, "{name}");
    }
    if extras.tangents_authored() {
        // A real basis came back: unit handedness, orthogonal to the normal.
        let mut checked = 0;
        for vertex in &loaded.vertices {
            assert!(
                vertex.tangent.w.abs() == 1.0,
                "handedness {:?}",
                vertex.tangent
            );
            assert!(vertex.tangent.truncate().dot(vertex.normal).abs() < 0.05);
            checked += 1;
        }
        assert!(checked > 0);
    }
}

/// The patch probe imported and re-exported: the attributes and user
/// properties no fixture authors.
#[cfg(has_ufbxw_probe)]
#[test]
fn nulls_lod_groups_cameras_and_user_properties_survive() {
    let dir = temp_dir("extras_probe");
    let probe_path = dir.join("probe.fbx");
    review_optimize::probe::write_patch_probe(&probe_path, false).expect("probe written");
    let Some((model, extras)) = load(&probe_path) else {
        return;
    };
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Ascii);

    // The Null and its Size.
    let (index, node) = node_by_name(&loaded, "Locator").expect("Locator written");
    assert_eq!(node.kind, NodeKind::Empty);
    let attribute = loaded_extras.nodes[index]
        .attribute
        .as_ref()
        .expect("attribute");
    assert_eq!(attribute.kind, AttributeKind::Empty);
    assert!((prop(&attribute.props, "Size").value_real[0] - 42.5).abs() < 1e-9);
    // The same place the probe's own import put it, in the same declared unit.
    let expected = node_by_name(&model, "Locator")
        .expect("source")
        .1
        .rest_local
        .translation;
    assert!(
        node.rest_local.translation.distance(expected) < 1e-6,
        "{:?} vs {expected:?}",
        node.rest_local.translation
    );
    assert_eq!(
        loaded.stats.source_unit_meters,
        model.stats.source_unit_meters
    );

    // User properties of every kind, still flagged as the user's.
    let (index, _) = node_by_name(&loaded, "Props").expect("Props written");
    let props = &loaded_extras.nodes[index].props;
    let int = prop(props, "MyInt");
    assert_eq!(int.value_int, 7);
    assert!(int.user_defined());
    let number = prop(props, "MyNumber");
    assert!((number.value_real[0] - 2.5).abs() < 1e-9);
    assert!(number.user_defined() && number.flags.animatable());
    let note = prop(props, "MyNote");
    assert_eq!(note.value_str, "hello");
    assert!(note.user_defined());
    let vector = prop(props, "MyVector");
    assert_eq!(&vector.value_real[..3], &[1.0, 2.0, 3.0]);
    let blob = prop(props, "MyBlob");
    assert_eq!(blob.value_blob, b"blob!");
    assert!(prop(props, "MyHidden").flags.hidden());

    // The LOD group with its levels and limits.
    let (index, _) = node_by_name(&loaded, "LodRoot").expect("LodRoot written");
    let attribute = loaded_extras.nodes[index]
        .attribute
        .as_ref()
        .expect("attribute");
    assert_eq!(attribute.kind, AttributeKind::LodGroup);
    let group = attribute.lod_group.as_ref().expect("levels");
    let distances: Vec<f64> = group.levels.iter().map(|level| level.distance).collect();
    assert_eq!(distances, [0.0, 120.0, 480.0]);
    let displays: Vec<u32> = group
        .levels
        .iter()
        .map(|level| level.display.code())
        .collect();
    assert_eq!(displays, [0, 1, 2]);
    assert!(group.use_distance_limit);
    assert_eq!(
        (group.distance_limit_min, group.distance_limit_max),
        (5.0, 600.0)
    );
    assert!(!group.ignore_parent_transform, "WorldSpace was set");

    // The camera keeps its focal length.
    let (index, node) = node_by_name(&loaded, "Cam").expect("Cam written");
    assert_eq!(node.kind, NodeKind::Camera);
    let attribute = loaded_extras.nodes[index]
        .attribute
        .as_ref()
        .expect("attribute");
    assert_eq!(attribute.kind, AttributeKind::Camera);
    let camera = attribute.camera.as_ref().expect("camera");
    assert!((camera.focal_length_mm - 35.0).abs() < 1e-9);

    // The layered texture, its layers, and the material connection.
    let layered = loaded_extras
        .textures
        .iter()
        .find(|texture| texture.name == "Layered")
        .expect("layered texture written");
    let layer_names: Vec<&str> = layered
        .layers
        .iter()
        .map(|layer| loaded_extras.textures[layer.texture as usize].name.as_str())
        .collect();
    assert_eq!(layer_names, ["Base", "Detail"]);
    assert_eq!(layered.layers[1].blend_mode.code(), 1);
    assert!((layered.layers[1].alpha - 0.5).abs() < 1e-9);
    let material = &loaded_extras.materials[0];
    assert!(
        material
            .textures
            .iter()
            .any(|t| t.material_prop == "DiffuseColor"
                && loaded_extras.textures[t.texture as usize].name == "Layered")
    );

    // The probe's own animation is covered by
    // `synthetic_animation_selection_and_layer_survive`.
    let _ = ElementRef::Node(0);
}

/// Export one processed result with the capture and read it back.
fn export_result(
    result: &ProcessedResult,
    model: &ModelData,
    extras: &SourceExtras,
    path: &Path,
) -> (ModelData, SourceExtras) {
    export_fbx(
        &result.lods,
        model,
        Some(extras),
        path,
        &ExportOptions::default(),
    )
    .expect("export");
    reload(path)
}

/// The source's quads must come back as quads through every operation that
/// keeps triangles whole, and as triangles only after a simplify rebuilt them.
#[test]
fn polygons_survive_every_operation_that_keeps_triangles_whole() {
    let Some((model, extras)) = fixture("rock_pillar_03.fbx") else {
        return;
    };
    assert!(
        model.stats.polygon_count < model.stats.triangle_count,
        "the fixture is authored in quads"
    );
    let dir = temp_dir("extras_polygons");

    let mut preserving: Vec<(&str, OptStack)> = Vec::new();
    let mut stack = OptStack::default();
    stack.push_op(OpKind::BakeAo(BakeAoParams::default()));
    preserving.push(("bake", stack));
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    preserving.push(("weld", stack));
    let mut stack = OptStack::default();
    stack.push_op(OpKind::FilterTriangles);
    preserving.push(("filter", stack));
    let mut stack = OptStack::default();
    stack.push_op(OpKind::VertexCache);
    stack.push_op(OpKind::VertexFetch);
    preserving.push(("reorder", stack));
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Overdraw { threshold: 1.05 });
    preserving.push(("overdraw", stack));

    for (label, stack) in &preserving {
        let result = run(&model, &extras, stack);
        let level = &result.lods[0];
        assert_eq!(
            level.model.stats.polygon_count, model.stats.polygon_count,
            "{label}: the processed level still counts the source's polygons"
        );
        let (loaded, _) =
            export_result(&result, &model, &extras, &dir.join(format!("{label}.fbx")));
        assert_eq!(
            loaded.stats.polygon_count, model.stats.polygon_count,
            "{label}: polygons"
        );
        assert_eq!(
            loaded.stats.triangle_count, model.stats.triangle_count,
            "{label}: triangles"
        );
    }

    let mut stack = OptStack::default();
    stack.push_op(OpKind::Reduce(ReduceParams {
        simplify: SimplifySettings::default(),
        target: LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        },
    }));
    let result = run(&model, &extras, &stack);
    let level = &result.lods[0];
    assert_eq!(
        level.model.stats.polygon_count, level.model.stats.triangle_count,
        "a simplified level is triangles"
    );
    let (loaded, _) = export_result(&result, &model, &extras, &dir.join("reduce.fbx"));
    assert_eq!(loaded.stats.polygon_count, loaded.stats.triangle_count);
    assert!(loaded.stats.triangle_count < model.stats.triangle_count);
}

/// The probe's topology layers and its second color set, through the pipeline.
#[cfg(has_ufbxw_probe)]
#[test]
fn topology_layers_and_extra_color_sets_survive() {
    let dir = temp_dir("extras_layers");
    let probe_path = dir.join("probe.fbx");
    review_optimize::probe::write_patch_probe(&probe_path, false).expect("probe written");
    let Some((model, extras)) = load(&probe_path) else {
        return;
    };
    let source = extras
        .mesh_of_node(node_by_name(&model, "Quad").unwrap().0 as u32)
        .unwrap();
    assert_eq!(source.edges.len(), 7);
    assert_eq!(source.color_sets.len(), 2);

    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    assert_eq!(loaded.stats.polygon_count, 2, "two quads");
    let back = loaded_extras
        .mesh_of_node(node_by_name(&loaded, "Quad").unwrap().0 as u32)
        .unwrap();
    assert_eq!(back.edges.len(), 7);
    assert_eq!(back.edge_crease, source.edge_crease);
    assert_eq!(back.edge_smoothing, source.edge_smoothing);
    assert_eq!(back.edge_visibility, source.edge_visibility);
    assert_eq!(back.face_hole, vec![false, true]);
    // The reader tables the groups: each face names an entry, the entry the id.
    let group_ids: Vec<i32> = back
        .face_group
        .iter()
        .map(|&group| back.face_groups[group as usize].id)
        .collect();
    assert_eq!(group_ids, vec![3, 7]);
    // The probe shares control points between corners of different colors, so
    // the export splits them (vertices are attribute-unique); every position
    // still carries its authored crease.
    let crease_by_position = |model: &ModelData, part: &review_model::extras::MeshExtras| {
        let mut creases: Vec<([i32; 3], i32)> = model
            .corner_to_logical
            .iter()
            .enumerate()
            .map(|(corner, &logical)| {
                // Quantized so the pairs sort and compare exactly.
                let position = (model.vertices[corner].position * 1000.0).round();
                let local = (logical - part.logical_first) as usize;
                (
                    [position.x as i32, position.y as i32, position.z as i32],
                    (part.vertex_crease[local] * 1000.0).round() as i32,
                )
            })
            .collect();
        creases.sort_unstable();
        creases.dedup();
        creases
    };
    assert_eq!(
        crease_by_position(&loaded, back),
        crease_by_position(&model, source)
    );

    let names: Vec<&str> = back
        .color_sets
        .iter()
        .map(|set| set.name.as_str())
        .collect();
    assert_eq!(names, ["Base", "Mask"]);
    assert_eq!(back.color_sets[1].values, source.color_sets[1].values);
    // Set 0 rides the geometry itself.
    let base: Vec<glam::Vec4> = loaded
        .vertices
        .iter()
        .map(|vertex| vertex.vertex_color)
        .collect();
    let source_base: Vec<glam::Vec4> = model
        .vertices
        .iter()
        .map(|vertex| vertex.vertex_color)
        .collect();
    assert_eq!(base, source_base);
}

/// A cluster identified by the names of its bone and its mesh node.
fn cluster_key(model: &ModelData, cluster: &review_model::SkinCluster) -> (String, String) {
    (
        model.nodes[cluster.bone as usize].name.clone(),
        model.nodes[cluster.mesh_node as usize].name.clone(),
    )
}

fn assert_matrix_close(a: glam::Mat4, b: glam::Mat4, tolerance: f32, context: &str) {
    let delta = (a - b).abs();
    let max = delta.to_cols_array().into_iter().fold(0f32, f32::max);
    assert!(
        max < tolerance,
        "{context}: drifts by {max}\n{a:?}\nvs\n{b:?}"
    );
}

/// The character's skin comes back with the same clusters — matched by bone and
/// mesh — the same authored matrices, and the same rest pose, through a
/// passthrough and a halving LOD alike.
#[test]
fn skin_survives_with_its_authored_cluster_matrices() {
    let Some((model, extras)) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let source_skin = model.skin.as_ref().expect("the fixture is skinned");
    let dir = temp_dir("extras_skin");

    let mut stacks: Vec<(&str, OptStack)> = Vec::new();
    let mut stack = OptStack::default();
    stack.push_op(OpKind::FilterTriangles);
    stacks.push(("passthrough", stack));
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::Reduce(ReduceParams {
        simplify: SimplifySettings::default(),
        target: LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        },
    }));
    stacks.push(("halved", stack));

    for (label, stack) in &stacks {
        let result = run(&model, &extras, stack);
        assert!(
            result.lods[0].model.skin.is_some(),
            "{label}: the level keeps its skin"
        );
        let (loaded, loaded_extras) =
            export_result(&result, &model, &extras, &dir.join(format!("{label}.fbx")));
        let skin = loaded
            .skin
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: skin written"));
        assert_eq!(
            skin.clusters.len(),
            source_skin.clusters.len(),
            "{label}: cluster count"
        );
        assert_eq!(
            skin.deformers.len(),
            source_skin.deformers.len(),
            "{label}: deformer count"
        );
        for cluster in &source_skin.clusters {
            let key = cluster_key(&model, cluster);
            let back = skin
                .clusters
                .iter()
                .find(|candidate| cluster_key(&loaded, candidate) == key)
                .unwrap_or_else(|| panic!("{label}: cluster {key:?} was not written"));
            let context = format!("{label} {key:?}");
            assert_matrix_close(
                back.mesh_node_to_bone,
                cluster.mesh_node_to_bone,
                1e-3,
                &context,
            );
            assert_matrix_close(back.bind_to_world, cluster.bind_to_world, 1e-4, &context);
            assert_matrix_close(
                back.world_to_bone_bind,
                cluster.world_to_bone_bind,
                1e-3,
                &context,
            );
        }
        for deformer in &source_skin.deformers {
            let node = &model.nodes[deformer.mesh_node as usize].name;
            let back = skin
                .deformers
                .iter()
                .find(|candidate| &loaded.nodes[candidate.mesh_node as usize].name == node)
                .unwrap_or_else(|| panic!("{label}: deformer of {node:?} was not written"));
            assert_eq!(
                back.method, deformer.method,
                "{label}: {node} skinning method"
            );
        }
        // Every vertex is skinned, and the skinned rest pose sits where the
        // source's does.
        assert!(
            (0..loaded.vertices.len()).all(|corner| {
                let logical = loaded.corner_to_logical[corner] as usize;
                !skin.influence_range(logical).is_empty()
            }),
            "{label}: every vertex has an influence"
        );
        if *label == "passthrough" {
            let source_bounds = model.bounds.expect("bounds");
            let back_bounds = loaded.bounds.expect("bounds");
            assert!(
                source_bounds.min.distance(back_bounds.min) < 1e-3,
                "{label}: rest bounds min"
            );
            assert!(
                source_bounds.max.distance(back_bounds.max) < 1e-3,
                "{label}: rest bounds max"
            );
        }
        assert!(
            !loaded_extras.meshes.is_empty(),
            "{label}: the capture reads the mesh parts back"
        );
    }
}

/// Authored bind poses come back node for node.
#[test]
fn bind_poses_survive() {
    let Some((model, extras)) = fixture("AN_ZombiedogLocomotion.fbx") else {
        return;
    };
    let bind_poses: Vec<_> = extras
        .poses
        .iter()
        .filter(|pose| pose.is_bind_pose)
        .collect();
    assert!(!bind_poses.is_empty(), "the fixture carries bind poses");
    let dir = temp_dir("extras_poses");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    let back_poses: Vec<_> = loaded_extras
        .poses
        .iter()
        .filter(|pose| pose.is_bind_pose)
        .collect();
    assert_eq!(back_poses.len(), bind_poses.len());
    for (pose, back) in bind_poses.iter().zip(&back_poses) {
        assert_eq!(pose.entries.len(), back.entries.len(), "{}", pose.name);
        for entry in &pose.entries {
            let node = &model.nodes[entry.node as usize].name;
            let matched = back
                .entries
                .iter()
                .find(|candidate| &loaded.nodes[candidate.node as usize].name == node)
                .unwrap_or_else(|| panic!("pose entry for {node:?} was not written"));
            assert_matrix_close(matched.bone_to_world, entry.bone_to_world, 1e-3, node);
        }
    }
}

/// The probe's synthetic deform data: a blend-type skin with dual-quaternion
/// weights, a second skin layer, and a blend shape with normal deltas.
#[cfg(has_ufbxw_probe)]
#[test]
fn dual_quaternion_weights_skin_layers_and_blend_shapes_survive() {
    let dir = temp_dir("extras_deform");
    let probe_path = dir.join("probe.fbx");
    review_optimize::probe::write_patch_probe(&probe_path, false).expect("probe written");
    let Some((model, extras)) = load(&probe_path) else {
        return;
    };
    let quad = node_by_name(&model, "Quad").unwrap().0 as u32;
    let source_part = extras.mesh_of_node(quad).unwrap();
    assert_eq!(source_part.dq_weights.len(), 2);
    assert_eq!(source_part.extra_skins.len(), 1);
    let source_morph = model.morph.as_ref().expect("the probe has a blend shape");

    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    let back_quad = node_by_name(&loaded, "Quad").unwrap().0 as u32;
    let back_part = loaded_extras.mesh_of_node(back_quad).unwrap();

    // Positions are the stable identity across the export's vertex split.
    let position_of = |model: &ModelData, logical: u32, part: &review_model::extras::MeshExtras| {
        let corner = model
            .corner_to_logical
            .iter()
            .position(|&candidate| candidate == logical)
            .expect("logical vertex has a corner");
        let _ = part;
        (model.vertices[corner].position * 1000.0).round()
    };

    // The blend-type skin and its dual-quaternion weights.
    let skin = loaded.skin.as_ref().expect("skin written");
    assert_eq!(
        skin.deformers[0].method,
        review_model::SkinningMethod::BlendedDqLinear
    );
    let mut dq: Vec<(glam::Vec3, f32)> = back_part
        .dq_weights
        .iter()
        .map(|&(vertex, weight)| (position_of(&loaded, vertex, back_part), weight))
        .collect();
    dq.sort_by(|a, b| a.0.to_array().partial_cmp(&b.0.to_array()).unwrap());
    dq.dedup();
    let mut source_dq: Vec<(glam::Vec3, f32)> = source_part
        .dq_weights
        .iter()
        .map(|&(vertex, weight)| (position_of(&model, vertex, source_part), weight))
        .collect();
    source_dq.sort_by(|a, b| a.0.to_array().partial_cmp(&b.0.to_array()).unwrap());
    assert_eq!(dq, source_dq);

    // The second skin layer with its one weight.
    assert_eq!(back_part.extra_skins.len(), 1);
    let layer = &back_part.extra_skins[0];
    assert_eq!(layer.clusters.len(), 1);
    assert_eq!(layer.clusters[0].name, "BoneLayer");
    let weighted: Vec<f32> = layer.influences.iter().map(|&(_, weight)| weight).collect();
    assert!(
        weighted.iter().all(|&weight| (weight - 0.25).abs() < 1e-6),
        "{weighted:?}"
    );
    assert!(!weighted.is_empty());

    // The blend shape: same channel, same rest weight, same offsets by position.
    let morph = loaded.morph.as_ref().expect("blend shape written");
    assert_eq!(morph.channels.len(), 1);
    assert_eq!(morph.channels[0].name, "Smile");
    assert!((morph.channels[0].rest_weight - 0.25).abs() < 1e-6);
    assert!((morph.channels[0].keyframes[0].target_weight - 1.0).abs() < 1e-6);
    let offsets_by_position = |model: &ModelData, morph: &review_model::MorphData| {
        let mut out: Vec<([i32; 3], [i32; 3], [i32; 3])> = Vec::new();
        for (corner, &logical) in model.corner_to_logical.iter().enumerate() {
            let range = morph.offsets[logical as usize] as usize
                ..morph.offsets[logical as usize + 1] as usize;
            for index in range {
                let quantize = |v: glam::Vec3| {
                    let q = (v * 1000.0).round();
                    [q.x as i32, q.y as i32, q.z as i32]
                };
                out.push((
                    quantize(model.vertices[corner].position),
                    quantize(morph.position[index]),
                    quantize(morph.normal[index]),
                ));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    };
    assert_eq!(
        offsets_by_position(&loaded, morph),
        offsets_by_position(&model, source_morph)
    );
}

/// The layer's animated properties keyed by target name and property, for a
/// comparison that does not depend on element order.
fn curves_by_target<'a>(
    model: &'a ModelData,
    extras: &'a SourceExtras,
) -> std::collections::BTreeMap<(String, String), &'a review_model::extras::AnimPropCurves> {
    let mut out = std::collections::BTreeMap::new();
    for layer in &extras.anim_layers {
        for anim in &layer.anim {
            let target = match &anim.target {
                ElementRef::Node(node) => format!("node:{}", model.nodes[*node as usize].name),
                ElementRef::NodeAttribute(node) => {
                    format!("attr:{}", model.nodes[*node as usize].name)
                }
                ElementRef::Material(material) => {
                    format!("material:{}", model.materials[*material as usize].name)
                }
                ElementRef::Texture(texture) => {
                    format!("texture:{}", extras.textures[*texture as usize].name)
                }
                ElementRef::Video(video) => {
                    format!("video:{}", extras.videos[*video as usize].name)
                }
                ElementRef::BlendChannel(channel) => format!(
                    "channel:{}",
                    model
                        .morph
                        .as_ref()
                        .map_or("", |morph| morph.channels[*channel as usize].name.as_str())
                ),
                ElementRef::DisplayLayer(layer) => {
                    format!("layer:{}", extras.display_layers[*layer as usize].name)
                }
                ElementRef::AnimLayer(layer) => {
                    format!("animlayer:{}", extras.anim_layers[*layer as usize].name)
                }
                ElementRef::Unmapped { name, .. } => format!("unmapped:{name}"),
            };
            out.insert((target, anim.prop_name.clone()), anim);
        }
    }
    out
}

fn assert_curves_equal(
    source: &review_model::extras::AnimPropCurves,
    back: &review_model::extras::AnimPropCurves,
    context: &str,
) {
    for component in 0..3 {
        // A component the source never keyed comes back as an empty curve (the
        // writer creates one per component); both mean "the default".
        fn present(
            curve: &Option<review_model::extras::Curve>,
        ) -> Option<&review_model::extras::Curve> {
            curve.as_ref().filter(|c| !c.keys.is_empty())
        }
        match (
            present(&source.curves[component]),
            present(&back.curves[component]),
        ) {
            (None, None) => {}
            (Some(a), Some(b)) => {
                assert_eq!(
                    a.keys.len(),
                    b.keys.len(),
                    "{context}[{component}] key count"
                );
                assert_eq!(a.pre, b.pre, "{context}[{component}] pre-extrapolation");
                assert_eq!(a.post, b.post, "{context}[{component}] post-extrapolation");
                for (index, (ka, kb)) in a.keys.iter().zip(&b.keys).enumerate() {
                    assert!(
                        (ka.time - kb.time).abs() < 1e-6,
                        "{context}[{component}] key {index} time"
                    );
                    assert!(
                        (ka.value - kb.value).abs() < 1e-5,
                        "{context}[{component}] key {index} value {} vs {}",
                        ka.value,
                        kb.value
                    );
                    assert_eq!(
                        ka.interpolation, kb.interpolation,
                        "{context}[{component}] key {index} interpolation"
                    );
                    if ka.interpolation == review_model::extras::Interpolation::Cubic {
                        // A first key's left handle has no previous key to hang on
                        // (the reader derives it from the previous key's attribute),
                        // so it cannot come back — and nothing evaluates it.
                        type Side<'a> = (&'a str, (f32, f32), (f32, f32));
                        let sides: &[Side] = if index == 0 {
                            &[("right", ka.right, kb.right)]
                        } else {
                            &[("left", ka.left, kb.left), ("right", ka.right, kb.right)]
                        };
                        for &(name, sa, sb) in sides {
                            assert!(
                                (sa.0 - sb.0).abs() < 1e-4
                                    && (sa.1 - sb.1).abs() < 1e-3 * (1.0 + sa.1.abs()),
                                "{context}[{component}] key {index} {name} tangent {sa:?} vs {sb:?}"
                            );
                        }
                    }
                }
            }
            _ => panic!("{context}[{component}]: one side lacks the curve"),
        }
    }
}

/// The dog's clips come back as the same stacks, layers and authored curves,
/// and bake to the same tracks.
#[test]
fn animation_curves_and_baked_clips_survive() {
    let Some((model, extras)) = fixture("AN_ZombiedogLocomotion.fbx") else {
        return;
    };
    assert!(!model.animations.is_empty());
    let dir = temp_dir("extras_animation");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);

    assert_eq!(
        loaded.animations.len(),
        model.animations.len(),
        "clip count"
    );
    assert_eq!(
        loaded_extras.animations.len(),
        extras.animations.len(),
        "stack count"
    );
    for (source, back) in extras.animations.iter().zip(&loaded_extras.animations) {
        assert_eq!(source.name, back.name);
        assert!(
            (source.time_begin - back.time_begin).abs() < 1e-6,
            "{}",
            source.name
        );
        assert!(
            (source.time_end - back.time_end).abs() < 1e-6,
            "{}",
            source.name
        );
        assert_eq!(
            source.layers.len(),
            back.layers.len(),
            "{} layers",
            source.name
        );
    }

    let source_curves = curves_by_target(&model, &extras);
    let back_curves = curves_by_target(&loaded, &loaded_extras);
    let mut compared = 0;
    for (key, anim) in &source_curves {
        let back = back_curves
            .get(key)
            .unwrap_or_else(|| panic!("animated property {key:?} was not written"));
        assert_curves_equal(anim, back, &format!("{key:?}"));
        compared += 1;
    }
    assert!(compared > 0, "the fixture animates something");

    // The reader bakes both files the same way, so the baked tracks agree.
    for (clip, back) in model.animations.iter().zip(&loaded.animations) {
        assert_eq!(clip.name, back.name);
        assert_eq!(clip.tracks.len(), back.tracks.len(), "{} tracks", clip.name);
        for track in &clip.tracks {
            let node = &model.nodes[track.node as usize].name;
            let back_track = back
                .tracks
                .iter()
                .find(|candidate| &loaded.nodes[candidate.node as usize].name == node)
                .unwrap_or_else(|| panic!("{}: no track for {node}", clip.name));
            assert_eq!(
                track.translation.len(),
                back_track.translation.len(),
                "{node} translation keys"
            );
            for (a, b) in track.translation.iter().zip(&back_track.translation) {
                assert!(
                    a.value.distance(b.value) < 1e-4,
                    "{node} translation {a:?} vs {b:?}"
                );
            }
            for (a, b) in track.rotation.iter().zip(&back_track.rotation) {
                assert!(
                    a.value.dot(b.value).abs() > 1.0 - 1e-4,
                    "{node} rotation {a:?} vs {b:?}"
                );
            }
            for (a, b) in track.scale.iter().zip(&back_track.scale) {
                assert!(
                    a.value.distance(b.value) < 1e-4,
                    "{node} scale {a:?} vs {b:?}"
                );
            }
        }
    }
}

/// Display layers keep their members and colors.
#[test]
fn display_layers_survive() {
    let Some((model, extras)) = fixture("AN_ZombiedogLocomotion.fbx") else {
        return;
    };
    assert!(
        !extras.display_layers.is_empty(),
        "the fixture carries display layers"
    );
    let dir = temp_dir("extras_display_layers");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    assert_eq!(
        loaded_extras.display_layers.len(),
        extras.display_layers.len()
    );
    for layer in &extras.display_layers {
        let back = loaded_extras
            .display_layers
            .iter()
            .find(|candidate| candidate.name == layer.name)
            .unwrap_or_else(|| panic!("display layer {:?} was not written", layer.name));
        assert_eq!(back.visible, layer.visible, "{}", layer.name);
        assert_eq!(back.frozen, layer.frozen, "{}", layer.name);
        assert!(
            back.ui_color.distance(layer.ui_color) < 1e-6,
            "{}",
            layer.name
        );
        let mut members: Vec<&str> = layer
            .nodes
            .iter()
            .map(|&n| model.nodes[n as usize].name.as_str())
            .collect();
        let mut back_members: Vec<&str> = back
            .nodes
            .iter()
            .map(|&n| loaded.nodes[n as usize].name.as_str())
            .collect();
        members.sort_unstable();
        back_members.sort_unstable();
        assert_eq!(members, back_members, "{}", layer.name);
    }
}

/// The probe's animation (a translation with weighted user tangents, a linear
/// user property, a material color, a stepped visibility), its display layer
/// and its selection set with vertex and face members.
#[cfg(has_ufbxw_probe)]
#[test]
fn synthetic_animation_selection_and_layer_survive() {
    let dir = temp_dir("extras_probe_anim");
    let probe_path = dir.join("probe.fbx");
    review_optimize::probe::write_patch_probe(&probe_path, false).expect("probe written");
    let Some((model, extras)) = load(&probe_path) else {
        return;
    };
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Ascii);

    let source_curves = curves_by_target(&model, &extras);
    let back_curves = curves_by_target(&loaded, &loaded_extras);
    for key in [
        ("node:Locator", "Lcl Translation"),
        ("node:Props", "MyNumber"),
        ("material:Mat", "DiffuseColor"),
        ("node:Quad", "Visibility"),
    ] {
        let key = (key.0.to_owned(), key.1.to_owned());
        let source = source_curves
            .get(&key)
            .unwrap_or_else(|| panic!("probe lacks {key:?}"));
        let back = back_curves
            .get(&key)
            .unwrap_or_else(|| panic!("{key:?} was not written"));
        assert_curves_equal(source, back, &format!("{key:?}"));
    }
    let translation = &back_curves[&("node:Locator".to_owned(), "Lcl Translation".to_owned())];
    let curve = translation.curves[0].as_ref().unwrap();
    assert_eq!(
        curve.post.mode,
        review_model::extras::ExtrapolationMode::Repeat
    );
    assert_eq!(curve.post.repeat_count, 3);
    let visibility = &back_curves[&("node:Quad".to_owned(), "Visibility".to_owned())];
    let steps: Vec<_> = visibility.curves[0]
        .as_ref()
        .unwrap()
        .keys
        .iter()
        .map(|k| k.interpolation)
        .collect();
    assert_eq!(
        steps,
        [
            review_model::extras::Interpolation::ConstantPrev,
            review_model::extras::Interpolation::ConstantNext,
            review_model::extras::Interpolation::ConstantPrev
        ]
    );
    assert_eq!(loaded.animations.len(), 1);
    assert_eq!(loaded.animations[0].name, "Take");

    let layer = loaded_extras
        .display_layers
        .iter()
        .find(|layer| layer.name == "Layer1")
        .expect("display layer written");
    let mut members: Vec<&str> = layer
        .nodes
        .iter()
        .map(|&n| loaded.nodes[n as usize].name.as_str())
        .collect();
    members.sort_unstable();
    assert_eq!(members, ["Locator", "Quad"]);
    assert!(layer.ui_color.distance(glam::Vec3::new(0.25, 0.5, 0.75)) < 1e-6);

    let set = loaded_extras
        .selection_sets
        .iter()
        .find(|set| set.name == "Sel")
        .expect("selection set written");
    assert_eq!(set.nodes.len(), 1);
    let entry = &set.nodes[0];
    assert_eq!(loaded.nodes[entry.node.unwrap() as usize].name, "Quad");
    assert!(entry.include_node);
    // The selected source vertices 0 and 2 are the positions (0,0,0) and (1,1,0);
    // the export split vertex 2 (two corners, two colors), so both copies are in.
    let part = loaded_extras.mesh_of_node(entry.node.unwrap()).unwrap();
    let mut selected: Vec<[i32; 3]> = entry
        .vertices
        .iter()
        .map(|&logical| {
            let corner = loaded
                .corner_to_logical
                .iter()
                .position(|&c| c == logical)
                .unwrap();
            let p = (loaded.vertices[corner].position * 1000.0).round();
            [p.x as i32, p.y as i32, p.z as i32]
        })
        .collect();
    selected.sort_unstable();
    selected.dedup();
    let unit = (1.0f32 * loaded.stats.source_unit_meters * 1000.0).round() as i32;
    assert_eq!(selected, [[0, 0, 0], [unit, unit, 0]]);
    let _ = part;
    assert_eq!(entry.faces.len(), 1);
    assert_eq!(
        entry.faces[0] as usize,
        part.face_first as usize + 1,
        "the second quad"
    );
}
