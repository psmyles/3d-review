//! Round trips of the source-property capture: the scene graph and the look.
//!
//! Every authored node must come back with its transform, kind and properties;
//! every material with its shading model, properties and textures; the scene's
//! own settings and metadata with it. What `SourceExtras` carries in has to come
//! back out of a written file, read through the vendored ufbx reader and its own
//! capture.

#![cfg(all(has_meshopt, has_ufbxw))]

use review_model::extras::Synthetic;
use review_optimize::{ExportOptions, FbxFormat, export_fbx};

mod common;

use common::extras::{assert_props_survive, node_by_name, passthrough, prop, round_trip};
use common::{fixture_full as fixture, reload_full as reload, temp_dir};

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
    // bring it back. The fixture is a classic Phong, so it has no metalness to
    // carry: its `ReflectionFactor` is Phong reflectivity, which import
    // deliberately does not read (see `review_import_material_metallic`).
    assert!((material.smoothness - 0.447_213_6).abs() < 1e-5);
    assert_eq!(material.metallic, 0.0);
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
    // Metalness is one-way through the Phong fallback: the writer puts it on
    // `ReflectionFactor` as the closest slot the model has, and the reader does
    // not take it back off. For this dielectric fixture the two agree anyway.
    assert_eq!(without_capture.materials[0].metallic, 0.0);
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
