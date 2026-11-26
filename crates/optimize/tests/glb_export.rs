//! The glb writer, checked by reading what it writes back through the glTF
//! reader (`review-import`), which validates every file as the `gltf` crate —
//! the parser Bevy's loader is built on — does.
//!
//! Fixtures: `assets/test_models/hierarchy.glb` (a Blender scene with a node
//! hierarchy, a mirrored node, two materials, a 16-bit palette colour, an
//! integer `_CAD_FACE` and extras on nodes, meshes and materials; made by
//! `make_hierarchy_glb.py` beside it) and the CAD kit's parts as glb
//! (`experiments/cad-kit/inputs/*.glb`, by
//! `blender_kit_glb.py` there).

#![cfg(has_meshopt)]

use std::path::{Path, PathBuf};

use review_model::{ComponentType, ModelData};
use review_optimize::{
    ExportFileType, ExportOptions, HierarchyMode, LodLevel, LodPackaging, LodParams, OpKind,
    OptStack, ReduceParams, TargetMode, WeldParams, export,
};

mod common;

use common::{run, temp_dir};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(path: &Path) -> ModelData {
    review_import::load_model(path)
        .unwrap_or_else(|error| panic!("{} reads: {error}", path.display()))
}

fn hierarchy() -> ModelData {
    load(&repo().join("assets/test_models/hierarchy.glb"))
}

fn kit(part: &str) -> PathBuf {
    repo().join(format!(
        "experiments/cad-kit/inputs/{part}.glb"
    ))
}

/// A stack that changes nothing a lossless index has not already: an exact weld.
fn no_op() -> OptStack {
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack
}

fn glb() -> ExportOptions {
    ExportOptions {
        file_type: ExportFileType::Glb,
        ..ExportOptions::default()
    }
}

/// Run `stack` over `model` and write it to `dir/name`; the files written.
fn write(
    model: &ModelData,
    stack: &OptStack,
    options: &ExportOptions,
    dir: &Path,
    name: &str,
) -> Vec<PathBuf> {
    let result = run(model, stack);
    std::fs::create_dir_all(dir).expect("the output folder");
    export(&result.lods, model, None, &dir.join(name), options)
        .expect("the export succeeds")
        .files
}

#[test]
fn a_glb_through_a_no_op_stack_comes_out_byte_for_byte_the_same() {
    let dir = temp_dir("glb_idempotent");
    for source in [
        repo().join("assets/test_models/hierarchy.glb"),
        kit("link"),
        kit("hinge_block"),
    ] {
        let name = source.file_name().unwrap().to_string_lossy().into_owned();
        let first = write(&load(&source), &no_op(), &glb(), &dir.join("first"), &name);
        let second = write(
            &load(&first[0]),
            &no_op(),
            &glb(),
            &dir.join("second"),
            &name,
        );
        assert_eq!(
            std::fs::read(&first[0]).unwrap(),
            std::fs::read(&second[0]).unwrap(),
            "{name}: a second pass changed the file"
        );
    }
}

#[test]
fn the_values_the_file_stored_come_back_as_they_were() {
    let source = hierarchy();
    let dir = temp_dir("glb_values");
    let written = load(&write(&source, &no_op(), &glb(), &dir, "hierarchy.glb")[0]);

    // The same nodes, in the same places, with the same extras.
    let names = |model: &ModelData| {
        model
            .nodes
            .iter()
            .map(|node| node.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&written), names(&source));
    for (before, after) in source.nodes.iter().zip(&written.nodes) {
        assert_eq!(before.parent, after.parent, "{}", before.name);
        assert_eq!(before.rest_local, after.rest_local, "{}", before.name);
    }
    let (before, after) = (
        source.gltf.as_deref().unwrap(),
        written.gltf.as_deref().unwrap(),
    );
    let extras = |gltf: &review_model::GltfSource| {
        gltf.nodes
            .iter()
            .map(|node| {
                (
                    node.extras.clone(),
                    node.mesh_name.clone(),
                    node.mesh_extras.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(extras(after), extras(before));
    assert!(
        before.nodes.iter().any(|node| node.extras.is_some()),
        "the fixture has node extras"
    );
    assert_eq!(
        after.materials, before.materials,
        "materials are written verbatim"
    );
    assert_eq!(after.color_format, before.color_format);

    // Every triangle, every corner, bit for bit: the world positions the reader
    // bakes, and the values as stored.
    let corners = |model: &ModelData| {
        let mut corners: Vec<[u32; 13]> = model
            .indices
            .iter()
            .map(|&index| {
                let vertex = model.vertices[index as usize];
                let mut key = [0u32; 13];
                for (slot, value) in vertex
                    .position
                    .to_array()
                    .into_iter()
                    .chain(vertex.normal.to_array())
                    .chain(vertex.uv.to_array())
                    .chain(vertex.vertex_color.to_array())
                    .enumerate()
                {
                    key[slot] = value.to_bits();
                }
                key[12] = model.custom_attributes[0].value(index as usize)[0];
                key
            })
            .collect();
        corners.sort_unstable();
        corners
    };
    assert_eq!(corners(&written), corners(&source));
}

#[test]
fn custom_attributes_keep_their_seams_through_a_lod_chain() {
    let source = load(&kit("link"));
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
        levels: vec![LodLevel {
            target_ratio: 0.25,
            ..LodLevel::default()
        }],
        ..LodParams::default()
    }));
    let options = ExportOptions {
        packaging: LodPackaging::FilePerLod,
        ..glb()
    };
    let files = write(
        &source,
        &stack,
        &options,
        &temp_dir("glb_seams"),
        "link.glb",
    );
    assert_eq!(files.len(), 2);
    for file in files {
        let level = load(&file);
        let [cad] = level.custom_attributes.as_slice() else {
            panic!("{}: one custom attribute", file.display());
        };
        assert_eq!(cad.name, "_CAD_FACE");
        // Blender writes an integer attribute as float; it comes back as it went.
        assert_eq!(cad.format.component, ComponentType::F32);
        // No triangle reaches across a CAD face: the line between two was a
        // seam, and the simplifier kept it.
        for triangle in level.indices.as_chunks::<3>().0 {
            let ids = triangle.map(|corner| cad.value(corner as usize)[0]);
            assert!(
                ids[0] == ids[1] && ids[1] == ids[2],
                "{}: a triangle straddles CAD faces {ids:?}",
                file.display()
            );
        }
    }
}

#[test]
fn custom_attributes_can_be_left_out() {
    let source = hierarchy();
    let options = ExportOptions {
        strip_custom_attributes: true,
        ..glb()
    };
    let written = load(&write(&source, &no_op(), &options, &temp_dir("glb_strip"), "h.glb")[0]);
    assert!(written.custom_attributes.is_empty());
    assert!(
        written.gltf.as_deref().unwrap().color_format.is_some(),
        "the colour stays"
    );
}

#[test]
fn an_fbx_is_written_in_gltf_axes() {
    let Some(source) = common::fixture("monkey.fbx") else {
        return;
    };
    let options = ExportOptions {
        hierarchy: HierarchyMode::Rebuild,
        ..glb()
    };
    let written = load(
        &write(
            &source,
            &no_op(),
            &options,
            &temp_dir("glb_fbx"),
            "monkey.glb",
        )[0],
    );
    let (Some(before), Some(after)) = (source.bounds, written.bounds) else {
        panic!("both have bounds");
    };
    // The same extent, turned from the file's axes into glTF's.
    let axes = source
        .stats
        .source_axes
        .expect("monkey.fbx declares its axes");
    let rotation = axes
        .rotation_to(review_import::gltf::AXES)
        .expect("a rotation, not a mirror");
    let turned = (rotation * before.size()).abs();
    assert!(
        (turned - after.size()).abs().max_element() < 1.0e-4 * turned.max_element(),
        "{turned} vs {}",
        after.size()
    );
    assert_eq!(written.stats.triangle_count, source.stats.triangle_count);
    assert_eq!(written.materials.len(), source.materials.len());
}

#[test]
fn a_single_file_chain_names_each_level() {
    let source = hierarchy();
    let mut stack = no_op();
    stack.push_op(OpKind::SimplifyLod(LodParams {
        levels: vec![LodLevel {
            target_ratio: 0.5,
            ..LodLevel::default()
        }],
        ..LodParams::default()
    }));
    let written = load(&write(&source, &stack, &glb(), &temp_dir("glb_chain"), "h.glb")[0]);
    let mut names: Vec<&str> = written
        .nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "mirror_LOD0",
            "mirror_LOD1",
            "panel_LOD0",
            "panel_LOD1",
            "rig"
        ],
        "each mesh node and its level copy, beside it under the same parent"
    );
    let rig = written
        .nodes
        .iter()
        .position(|node| node.name == "rig")
        .unwrap();
    assert!(
        written
            .nodes
            .iter()
            .filter(|node| node.name != "rig")
            .all(|node| node.parent == Some(rig))
    );
}
