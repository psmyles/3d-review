//! The glb writer, checked by reading what it writes back through the glTF
//! reader (`review-import`), which validates every file as the `gltf` crate —
//! the parser Bevy's loader is built on — does.
//!
//! Fixtures: `assets/test_models/hierarchy.glb` (a Blender scene with a node
//! hierarchy, a mirrored node, two materials, a 16-bit palette colour, an
//! integer `_CAD_FACE` and extras on nodes, meshes and materials; made by
//! `make_hierarchy_glb.py` beside it), and the CAD kit's parts as Blender wrote
//! them ([`common::cad_kit`]: not in this repository, so those tests are
//! `#[ignore]`d).

#![cfg(has_meshopt)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use review_model::{ComponentType, ModelData};
use review_optimize::{
    CancelToken, ExportFileType, ExportOptions, GlbTangents, HierarchyMode, LodLevel, LodPackaging,
    LodParams, OpKind, OptError, OptStack, ReduceParams, TargetMode, WeldParams, export,
    export_cancellable,
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

/// A cancelled export gives up before it replaces anything: the file already
/// at the destination keeps its bytes, and no staging file is left beside it.
/// This is what the Working card's Cancel promises ("Nothing on disk was
/// changed").
#[test]
fn a_cancelled_export_replaces_nothing() {
    let model = hierarchy();
    let result = run(&model, &no_op());
    let dir = temp_dir("glb_cancelled");
    let path = dir.join("asset.glb");
    std::fs::write(&path, b"the previous export").expect("seed the destination");

    // Already superseded: the counter has moved past the token's generation.
    let generation = Arc::new(AtomicU64::new(1));
    let cancel = CancelToken::new(Arc::clone(&generation), 0);
    let error = export_cancellable(&result.lods, &model, None, &path, &glb(), Some(&cancel))
        .expect_err("a cancelled export does not finish");

    assert!(matches!(error, OptError::Cancelled), "got {error:?}");
    assert_eq!(
        std::fs::read(&path).expect("the destination is still there"),
        b"the previous export",
        "the previous export is untouched"
    );
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("read the export directory")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["asset.glb".to_owned()],
        "nothing else was written"
    );
}

#[test]
fn a_glb_through_a_no_op_stack_comes_out_byte_for_byte_the_same() {
    assert_no_op_reproduces(
        "glb_idempotent",
        &[repo().join("assets/test_models/hierarchy.glb")],
    );
}

/// The same for CAD parts as Blender wrote them: tens of thousands of vertices,
/// a 16-bit palette colour and two float custom attributes.
#[test]
#[ignore = "needs the CAD kit: set REVIEW_CAD_KIT"]
fn a_cad_kit_glb_through_a_no_op_stack_comes_out_byte_for_byte_the_same() {
    assert_no_op_reproduces("glb_idempotent_cad_kit", &common::cad_kit("glb"));
}

/// Each of `sources` written through a no-op stack, then that file written
/// again: the second pass changes nothing.
fn assert_no_op_reproduces(folder: &str, sources: &[PathBuf]) {
    let dir = temp_dir(folder);
    for source in sources {
        let name = source.file_name().unwrap().to_string_lossy().into_owned();
        let first = write(&load(source), &no_op(), &glb(), &dir.join("first"), &name);
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

/// Every CAD kit part cut to a budget and a quarter of it: both levels keep the
/// face and region ids, as Blender wrote them, and no triangle of either reaches
/// across two values of one.
#[test]
#[ignore = "needs the CAD kit: set REVIEW_CAD_KIT"]
fn cad_kit_attributes_keep_their_seams_through_a_lod_chain() {
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
    let dir = temp_dir("glb_seams");
    let files: Vec<PathBuf> = common::cad_kit("glb")
        .iter()
        .flat_map(|source| {
            let name = source.file_name().unwrap().to_string_lossy().into_owned();
            let written = write(&load(source), &stack, &options, &dir, &name);
            assert_eq!(written.len(), 2, "{name}: a level per file");
            written
        })
        .collect();
    for file in files {
        let level = load(&file);
        let names: Vec<&str> = level
            .custom_attributes
            .iter()
            .map(|attribute| attribute.name.as_str())
            .collect();
        assert_eq!(names, ["_CAD_FACE", "_CAD_REGION"], "{}", file.display());
        for cad in &level.custom_attributes {
            // Blender writes an integer attribute as float; it comes back as it
            // went.
            assert_eq!(cad.format.component, ComponentType::F32);
            // No triangle reaches across a CAD face or region: the line between
            // two was a seam, and the simplifier kept it.
            for triangle in level.indices.as_chunks::<3>().0 {
                let ids = triangle.map(|corner| cad.value(corner as usize)[0]);
                assert!(
                    ids[0] == ids[1] && ids[1] == ids[2],
                    "{}: a triangle straddles {} values {ids:?}",
                    file.display(),
                    cad.name
                );
            }
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

#[test]
fn mikktspace_tangents_are_written_where_asked() {
    let source = hierarchy();
    assert!(
        !source.gltf.as_deref().unwrap().tangents,
        "Blender wrote no tangents"
    );
    let options = ExportOptions {
        glb_tangents: GlbTangents::MikkTSpace,
        ..glb()
    };
    let written = load(
        &write(
            &source,
            &no_op(),
            &options,
            &temp_dir("glb_mikk"),
            "hierarchy.glb",
        )[0],
    );
    assert!(
        written.gltf.as_deref().unwrap().tangents,
        "TANGENT is in the file"
    );
    for vertex in &written.vertices {
        let tangent = vertex.tangent.truncate();
        assert!((tangent.length() - 1.0).abs() < 1e-3, "unit: {tangent}");
        assert!(tangent.dot(vertex.normal).abs() < 1e-2, "perpendicular");
        assert!(vertex.tangent.w.abs() == 1.0, "handedness is a sign");
    }
    // The default leaves a file without tangents without them.
    let plain = load(
        &write(
            &source,
            &no_op(),
            &glb(),
            &temp_dir("glb_plain"),
            "hierarchy.glb",
        )[0],
    );
    assert!(!plain.gltf.as_deref().unwrap().tangents);
}
