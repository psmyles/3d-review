//! Writing a processed LOD chain out as FBX, through the vendored ufbx_write.
//!
//! ## Shape of the work
//!
//! [`export_fbx`] turns one or more [`ProcessedLod`]s into either a single file
//! holding the whole chain as suffixed sibling nodes, or one file per level. The
//! per-level geometry is regrouped back into meshes (one per source node), the
//! scene graph is either rebuilt or flattened, and the whole payload crosses
//! into C once per file as flat arrays — see `export_bridge.h`.
//!
//! ## What is written
//!
//! Everything the source authored that the optimization stack did not change.
//! With the source-property capture ([`SourceExtras`]) in hand, the export
//! writes the **whole** scene graph — every node, not only the mesh-bearing
//! ones — from each node's authored properties (local transform, rotation
//! order, pivots, pre/post rotations, geometric transform, inherit type,
//! visibility, user properties), its node attribute (bone, light, camera, null,
//! LOD group) with the attribute's own properties, every material with its
//! full property set on the shader model the file declared, the textures and
//! embedded videos those materials reference, the scene's settings and
//! metadata, and the authored tangent basis. Without the capture (an export in
//! the moments before it lands) the older computed path is used: the mesh
//! nodes' ancestor chains from their world transforms, and a Phong material
//! carrying the viewer's PBR figures.
//!
//! Materials are the ones the *source file* declared. Material edits made in
//! the viewer are previews of a look, not authored asset data, so writing them
//! into an exported mesh would quietly change the asset.
//!
//! ## Layout
//!
//! This module is the public face — [`export_fbx`], its report, and the path
//! each level is written to. Everything below it is one builder per part of the
//! payload: [`scene`] defines the payload structs, [`build`] sequences the
//! builders, [`nodes`] / [`mesh`] / [`materials`] / [`deform`] / [`anim`] /
//! [`settings`] each fill one part of it, and [`write`] hands the whole to C.

use std::path::{Path, PathBuf};

use review_model::{ModelData, SourceExtras};

use crate::OptError;
use crate::notice::ExportNote;
use crate::process::ProcessedLod;
use crate::replace_file::Staged;
use crate::stack::{ExportOptions, LodPackaging};

mod anim;
mod build;
mod deform;
mod materials;
mod mesh;
mod nodes;
mod scene;
mod settings;
mod unit;
mod util;
mod write;

pub(crate) use anim::*;
pub(crate) use build::*;
pub(crate) use deform::*;
pub(crate) use materials::*;
pub(crate) use mesh::*;
pub(crate) use nodes::*;
pub(crate) use scene::*;
pub(crate) use settings::*;
pub(crate) use unit::*;
pub(crate) use util::*;
pub(crate) use write::*;

/// What an export actually produced, for the confirmation the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// Every file written, in order.
    pub files: Vec<PathBuf>,
    pub mesh_count: usize,
    pub triangle_count: usize,
    /// Things worth saying about the result — a dropped skin, a node whose
    /// transform could not be inverted, properties that had not loaded.
    pub notes: Vec<ExportNote>,
}

/// True when this build has the vendored ufbx_write compiled in.
pub const fn available() -> bool {
    cfg!(has_ufbxw)
}

/// Write `lods` to `path` according to `options`.
///
/// `extras` is the source's property capture, when it has landed; without it
/// the export writes what the viewer shows and says so in the report.
///
/// `path` is the file the user chose; under [`LodPackaging::FilePerLod`] it
/// supplies the stem and each level gets a `_LOD<n>` suffix — unless there is
/// only one level, which is written to `path` itself.
pub fn export_fbx(
    lods: &[ProcessedLod],
    source: &ModelData,
    extras: Option<&SourceExtras>,
    path: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, OptError> {
    let _z = crate::prof::zone!("Export FBX");

    if !available() {
        return Err(OptError::Unavailable);
    }
    if lods.is_empty() {
        return Err(OptError::EmptyMesh);
    }

    let mut report = ExportReport {
        files: Vec::new(),
        mesh_count: 0,
        triangle_count: 0,
        notes: Vec::new(),
    };
    if extras.is_none() {
        report.notes.push(ExportNote::CaptureNotLoaded);
    }

    // Every file is written to a temporary sibling first and the whole set is
    // committed only once all of it is on disk. Two things would otherwise go
    // wrong on a failure: ufbx_write opens the destination with `fopen(…, "wb")`,
    // which truncates the user's previous export before writing a byte, and a
    // chain whose third level failed would already have replaced the first two.
    let mut staged: Vec<Staged> = Vec::with_capacity(lods.len());
    match options.packaging {
        LodPackaging::SingleFileSuffixed => {
            let scene = build_scene(lods, source, extras, options, &mut report)?;
            staged.push(Staged::write(path, |staging| {
                write_scene(&scene, staging, options.format)
            })?);
        }
        LodPackaging::FilePerLod => {
            // A lone level is not a chain: there is nothing for a `_LOD0` suffix to
            // distinguish it from, and a stack that only reduces the mesh in place
            // is meant to stand in for the source asset — so it goes to the path
            // the user actually chose.
            let chain = lods.len() > 1;
            for lod in lods {
                // Each file holds one level, so its meshes keep their plain names
                // rather than a suffix the filename already carries.
                let scene = build_scene(
                    std::slice::from_ref(lod),
                    source,
                    extras,
                    options,
                    &mut report,
                )?;
                let level_path = if chain {
                    level_path(path, lod.level)
                } else {
                    path.to_path_buf()
                };
                staged.push(Staged::write(&level_path, |staging| {
                    write_scene(&scene, staging, options.format)
                })?);
            }
        }
    }

    // Everything is written; put each file in its place. A rename this late is
    // rare — the directory is known writable, since the staging files are in it
    // — but if one does fail the user still has to be told which of their assets
    // were already replaced, so the report travels with the error.
    for entry in staged {
        let destination = entry.destination().to_path_buf();
        match entry.commit() {
            Ok(committed) => report.files.push(committed),
            Err(error) => {
                report.notes.dedup();
                return Err(OptError::ExportIncomplete {
                    replaced: report.files,
                    failed: destination,
                    reason: error.to_string(),
                });
            }
        }
    }

    report.notes.dedup();
    Ok(report)
}

/// `asset.fbx` → `asset_LOD2.fbx`.
fn level_path(path: &Path, level: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mesh".to_owned());
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_else(|| "fbx".to_owned());
    path.with_file_name(format!("{stem}_LOD{level}.{extension}"))
}

/// Unit coverage for the pure helpers. The FFI write path is covered end to end
/// by `tests/export_round_trip.rs`, which reads every written file back.
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::c_char;

    use glam::{Mat4, Vec3};
    use review_model::ModelData;
    use review_model::extras::{AttributeKind, CoordinateAxis, Synthetic};

    use super::*;
    use crate::stack::HierarchyMode;

    #[test]
    fn level_path_suffixes_the_stem_and_keeps_the_extension() {
        assert_eq!(
            level_path(Path::new("out/asset.fbx"), 2),
            PathBuf::from("out/asset_LOD2.fbx")
        );
    }

    #[test]
    fn level_path_supplies_a_name_and_an_extension_when_the_path_has_neither() {
        assert_eq!(
            level_path(Path::new("asset"), 1),
            PathBuf::from("asset_LOD1.fbx")
        );
        assert_eq!(level_path(Path::new(""), 0), PathBuf::from("mesh_LOD0.fbx"));
    }

    #[test]
    fn unit_scale_follows_the_source_and_snaps_to_the_unit_it_meant() {
        // A centimeter file's 0.01 arrives through an f32, so the naive
        // `x * 100.0` is 0.99999998 — a factor no importer names, and one whose
        // reciprocal drifts every coordinate written with it.
        let centimeters = UnitScale::from_source(0.01);
        assert_eq!(centimeters.unit_scale_cm, 1.0);
        assert_eq!(centimeters.per_meter, 100.0);

        let inches = UnitScale::from_source(0.0254);
        assert_eq!(inches.unit_scale_cm, 2.54);

        let meters = UnitScale::from_source(1.0);
        assert_eq!(meters.unit_scale_cm, 100.0);
        assert_eq!(meters.per_meter, 1.0);

        // An unrecognised unit is still the file's own unit and is written as
        // declared; only a missing one falls back to the meters the geometry
        // already is.
        assert_eq!(UnitScale::from_source(0.5).unit_scale_cm, 50.0);
        for missing in [0.0, -1.0, f32::NAN] {
            let fallback = UnitScale::from_source(missing);
            assert_eq!(fallback.unit_scale_cm, DEFAULT_UNIT_SCALE_CM);
            assert_eq!(fallback.per_meter, 1.0);
        }
    }

    #[test]
    fn time_mode_matches_standard_rates_and_leaves_the_rest_custom() {
        assert_eq!(time_mode_for(30.0), Some(6));
        assert_eq!(time_mode_for(24.0), Some(11));
        assert_eq!(time_mode_for(59.94), Some(17));
        assert_eq!(time_mode_for(31.5), None);
    }

    #[test]
    fn c_string_falls_back_for_empty_text() {
        assert_eq!(c_string("", "Mesh").to_bytes(), b"Mesh");
    }

    #[test]
    fn c_string_keeps_a_name_carrying_an_interior_nul() {
        assert_eq!(c_string("Bo\0dy", "Mesh").to_bytes(), b"Body");
    }

    #[cfg(has_ufbxw)]
    #[test]
    fn error_message_reads_the_bridge_text_and_explains_its_absence() {
        let mut buffer = [0; crate::export_ffi::ERROR_LENGTH];
        for (slot, &byte) in buffer.iter_mut().zip(b"could not write the file") {
            *slot = byte as c_char;
        }
        assert_eq!(error_message(&buffer), "could not write the file");
        assert_eq!(
            error_message(&[0; 8]),
            "the FBX writer failed without reporting a reason"
        );
    }

    /// Two nodes each claiming the other as parent — what the ancestor walk has
    /// to survive, since it follows `parent` links to the root.
    fn cyclic_model() -> ModelData {
        let node = |name: &str, parent: usize| review_model::SceneNode {
            name: name.to_owned(),
            parent: Some(parent),
            mesh_part: Some(0),
            source_vertex_count: 0,
            transform: Mat4::IDENTITY,
            rest_local: Default::default(),
            kind: review_model::NodeKind::Mesh,
            bone: None,
        };
        ModelData {
            name: "Looped".to_owned(),
            nodes: vec![node("A", 1), node("B", 0)],
            ..ModelData::default()
        }
    }

    fn empty_scene() -> SceneData {
        SceneData::new(UnitScale::from_source(0.0))
    }

    #[test]
    fn place_node_cuts_a_parent_cycle_and_says_so() {
        let source = cyclic_model();
        let mut scene = empty_scene();
        let group = NodeGroup {
            source_node: Some(0),
            triangles: Vec::new(),
        };

        let (node, notes) = place_node(
            &mut scene,
            &mut HashMap::new(),
            &source,
            &group,
            HierarchyMode::Rebuild,
            0,
        );

        assert_eq!(node, 0, "the mesh still attaches to a real node");
        assert_eq!(
            scene.nodes.len(),
            1,
            "the loop emits one node, not an endless chain"
        );
        assert!(
            notes.contains(&ExportNote::ParentChainLooped),
            "the user is told the branch was flattened: {notes:?}"
        );
    }

    /// A source hierarchy shaped like a game FBX carrying its LOD chain: the
    /// importer's synthetic file root, a group node, and two sibling mesh
    /// leaves under it.
    fn lod_sibling_model() -> ModelData {
        let node =
            |name: &str, parent: Option<usize>, mesh: Option<usize>| review_model::SceneNode {
                name: name.to_owned(),
                parent,
                mesh_part: mesh,
                source_vertex_count: 0,
                transform: Mat4::IDENTITY,
                rest_local: Default::default(),
                kind: if mesh.is_some() {
                    review_model::NodeKind::Mesh
                } else {
                    review_model::NodeKind::Empty
                },
                bone: None,
            };
        ModelData {
            name: "Crate".to_owned(),
            nodes: vec![
                node("", None, None),
                node("SM_Crate", Some(0), None),
                node("SM_Crate_LOD0", Some(1), Some(0)),
                node("SM_Crate_LOD1", Some(1), Some(1)),
            ],
            ..ModelData::default()
        }
    }

    fn placed_names(scene: &SceneData) -> Vec<&str> {
        scene
            .nodes
            .iter()
            .map(|node| node.name.to_str().expect("names are UTF-8"))
            .collect()
    }

    /// Sibling meshes under one group come out under a single shared ancestor
    /// chain — not one copy of it per mesh — and the importer's synthetic file
    /// root is not re-emitted as a wrapper node.
    #[test]
    fn place_node_shares_ancestors_and_drops_the_synthetic_file_root() {
        let source = lod_sibling_model();
        let mut scene = empty_scene();
        let mut placed = HashMap::new();

        for leaf in [2, 3] {
            let group = NodeGroup {
                source_node: Some(leaf),
                triangles: Vec::new(),
            };
            let (node, _) = place_node(
                &mut scene,
                &mut placed,
                &source,
                &group,
                HierarchyMode::Rebuild,
                0,
            );
            assert!(node >= 0, "the mesh attaches to a real node");
        }

        assert_eq!(
            placed_names(&scene),
            ["SM_Crate", "SM_Crate_LOD0", "SM_Crate_LOD1"],
            "one shared chain, no file-root wrapper"
        );
        assert_eq!(scene.nodes[0].parent, NO_PARENT);
        assert_eq!(scene.nodes[1].parent, 0, "first leaf under the group");
        assert_eq!(scene.nodes[2].parent, 0, "second leaf beside the first");
    }

    /// A suffixed chain (several levels in one file) puts each level's leaf as
    /// a sibling under the same unsuffixed ancestors, mirroring how game FBX
    /// files lay out their own LOD chains.
    #[test]
    fn place_node_puts_suffixed_levels_as_siblings_under_one_chain() {
        let source = lod_sibling_model();
        let mut scene = empty_scene();
        let mut placed = HashMap::new();
        let group = NodeGroup {
            source_node: Some(2),
            triangles: Vec::new(),
        };

        for level in 0..2 {
            place_node(
                &mut scene,
                &mut placed,
                &source,
                &group,
                HierarchyMode::Rebuild,
                level,
            );
        }

        assert_eq!(
            placed_names(&scene),
            ["SM_Crate", "SM_Crate_LOD0", "SM_Crate_LOD0_LOD1"],
            "only the leaf is suffixed; the ancestors are shared"
        );
        assert_eq!(scene.nodes[1].parent, 0);
        assert_eq!(scene.nodes[2].parent, 0, "the level-1 leaf is a sibling");
    }

    /// The authored graph: every node except the synthetic root goes out, a
    /// scale helper is skipped with its children re-parented past it, and a
    /// later level's mesh node is a suffixed sibling with the same attribute.
    #[test]
    fn place_graph_writes_every_authored_node_and_skips_the_helpers() {
        use review_model::extras::{
            Application, AttributeExtras, InheritMode, NodeExtras, RotationOrder, SceneExtras,
            SnapMode, TimeMode, TimeProtocol,
        };

        let mut source = lod_sibling_model();
        // A scale helper between the group and its second leaf, and a light.
        source.nodes.push(review_model::SceneNode {
            name: "helper".to_owned(),
            parent: Some(1),
            ..Default::default()
        });
        source.nodes[3].parent = Some(4);
        source.nodes.push(review_model::SceneNode {
            name: "Key".to_owned(),
            parent: Some(1),
            kind: review_model::NodeKind::Light,
            ..Default::default()
        });
        let node_extras = |synthetic, attribute: Option<AttributeKind>| NodeExtras {
            props: Vec::new(),
            rotation_order: RotationOrder::Xyz,
            inherit_mode: InheritMode::Normal,
            original_inherit_mode: InheritMode::Normal,
            geometry_to_node: Mat4::IDENTITY,
            synthetic,
            visible: true,
            attribute: attribute.map(|kind| AttributeExtras {
                kind,
                name: String::new(),
                props: Vec::new(),
                light: None,
                camera: None,
                lod_group: None,
            }),
        };
        let extras = SourceExtras {
            scene: SceneExtras {
                creator: String::new(),
                filename: String::new(),
                original_file_path: String::new(),
                version: 7700,
                ascii: false,
                original_application: Application::default(),
                latest_application: Application::default(),
                scene_props: Vec::new(),
                settings_props: Vec::new(),
                axes: [CoordinateAxis::Unknown; 3],
                original_axis_up: CoordinateAxis::Unknown,
                unit_meters: 1.0,
                original_unit_meters: 1.0,
                frames_per_second: 30.0,
                ambient_color: Vec3::ZERO,
                default_camera: String::new(),
                time_mode: TimeMode::Fps30,
                time_protocol: TimeProtocol::Default,
                snap_mode: SnapMode::None,
            },
            nodes: vec![
                node_extras(Synthetic::Root, None),
                node_extras(Synthetic::None, None),
                node_extras(Synthetic::None, Some(AttributeKind::Mesh)),
                node_extras(Synthetic::None, Some(AttributeKind::Mesh)),
                node_extras(Synthetic::ScaleHelper, None),
                node_extras(Synthetic::None, Some(AttributeKind::Light)),
            ],
            materials: Vec::new(),
            textures: Vec::new(),
            videos: Vec::new(),
            meshes: Vec::new(),
            poses: Vec::new(),
            display_layers: Vec::new(),
            selection_sets: Vec::new(),
            anim_layers: Vec::new(),
            animations: Vec::new(),
        };

        let mut scene = empty_scene();
        let mut placed = HashMap::new();
        place_graph(&mut scene, &mut placed, &source, &extras);

        assert_eq!(
            placed_names(&scene),
            ["SM_Crate", "SM_Crate_LOD0", "SM_Crate_LOD1", "Key"],
            "no root, no helper, and the light is there"
        );
        assert_eq!(scene.nodes[2].parent, 0, "re-parented past the helper");
        assert_eq!(scene.nodes[3].attribute_kind, ATTRIB_LIGHT);
        assert!(scene.nodes.iter().all(|node| node.authored_transform));

        let group = NodeGroup {
            source_node: Some(2),
            triangles: Vec::new(),
        };
        assert_eq!(
            place_level_node(&mut scene, &placed, &source, &extras, &group, 0),
            1
        );
        let copy = place_level_node(&mut scene, &placed, &source, &extras, &group, 1);
        assert_eq!(copy, 4);
        assert_eq!(scene.nodes[4].parent, 0, "the level copy is a sibling");
        assert_eq!(scene.nodes[4].name.to_str().unwrap(), "SM_Crate_LOD0_LOD1");
    }
}
