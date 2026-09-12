//! Round trips of the source-property capture: what the *mesh* carries.
//!
//! Nulls, LOD groups, cameras and user properties (supplied by the writer's own
//! patch probe, since no fixture authors them), the source's quads and n-gons
//! through every operation that keeps triangles whole, and the topology layers
//! and extra color sets beside them.

#![cfg(all(has_meshopt, has_ufbxw))]

use review_model::extras::{AttributeKind, ElementRef};
use review_model::{ModelData, NodeKind};
use review_optimize::{
    BakeAoParams, FbxFormat, LodLevel, OpKind, OptStack, ReduceParams, SimplifySettings, WeldParams,
};

mod common;

use common::extras::{export_result, node_by_name, prop, round_trip};
use common::{fixture_full as fixture, load_full as load, run_with_extras as run, temp_dir};

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
