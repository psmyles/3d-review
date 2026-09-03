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
//! ## What is written, and what is not
//!
//! Output is always triangulated: the optimizer's meshes are triangle soups and
//! the original polygon topology cannot survive welding or simplification.
//!
//! Materials are the ones the *source file* declared. Material edits made in the
//! viewer are previews of a look, not authored asset data, so writing them into
//! an exported mesh would quietly change the asset. Source materials carry no
//! texture paths through import either, so exported materials are untextured and
//! the report says so.
//!
//! Skinning is dropped, as everywhere else in this crate.

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int};
use std::path::{Path, PathBuf};

use glam::{Mat4, Quat, Vec3};
use review_model::ModelData;

use crate::OptError;
use crate::process::ProcessedLod;
use crate::stack::{ExportOptions, FbxFormat, HierarchyMode, LodPackaging};

/// A root node's parent index — `RVO_NO_PARENT` in `export_bridge.h`.
const NO_PARENT: i32 = -1;

/// Centimeters per exported unit, written as FBX's `UnitScaleFactor`.
///
/// Import normalizes every file to meters, so that is what the geometry reaching
/// the writer is in. FBX does not fix a unit — it declares one, conventionally
/// centimeters — so writing metric coordinates without saying so makes the model
/// read back a hundred times too small.
const UNIT_SCALE_CM: f64 = 100.0;

/// What an export actually produced, for the confirmation the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// Every file written, in order.
    pub files: Vec<PathBuf>,
    pub mesh_count: usize,
    pub triangle_count: usize,
    /// Things worth saying about the result — untextured materials, a dropped
    /// skin, a node whose transform could not be inverted.
    pub notes: Vec<String>,
}

/// True when this build has the vendored ufbx_write compiled in.
pub const fn available() -> bool {
    cfg!(has_ufbxw)
}

/// Write `lods` to `path` according to `options`.
///
/// `path` is the file the user chose; under [`LodPackaging::FilePerLod`] it
/// supplies the stem and each level gets a `_LOD<n>` suffix — unless there is
/// only one level, which is written to `path` itself.
pub fn export_fbx(
    lods: &[ProcessedLod],
    source: &ModelData,
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
    if !source.materials.is_empty() {
        report.notes.push(
            "Materials are written as the source file declared them, without textures — \
             import carries no texture paths, and viewer material edits stay previews."
                .to_owned(),
        );
    }
    if source.skin.is_some() {
        report
            .notes
            .push("Skinning was dropped: this tool processes static geometry.".to_owned());
    }

    match options.packaging {
        LodPackaging::SingleFileSuffixed => {
            let scene = build_scene(lods, source, options, &mut report)?;
            write_scene(&scene, path, options.format)?;
            report.files.push(path.to_path_buf());
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
                let scene = build_scene(std::slice::from_ref(lod), source, options, &mut report)?;
                let level_path = if chain {
                    level_path(path, lod.level)
                } else {
                    path.to_path_buf()
                };
                write_scene(&scene, &level_path, options.format)?;
                report.files.push(level_path);
            }
        }
    }

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

// ---------------------------------------------------------------------------
// Scene assembly
// ---------------------------------------------------------------------------

/// The owned Rust side of a scene payload. Every buffer the bridge borrows lives
/// here, so the whole thing must outlive the `review_export_fbx` call — which it
/// does, since [`write_scene`] takes it by reference.
struct SceneData {
    nodes: Vec<NodeData>,
    materials: Vec<MaterialData>,
    meshes: Vec<MeshData>,
}

struct NodeData {
    name: CString,
    parent: i32,
    translation: [f64; 3],
    rotation: [f64; 4],
    scaling: [f64; 3],
}

struct MaterialData {
    name: CString,
    base_color: [f64; 3],
    emissive: [f64; 3],
}

struct MeshData {
    name: CString,
    node: i32,
    positions: Vec<f64>,
    indices: Vec<i32>,
    triangle_count: usize,
    vertex_count: usize,
    normals: Vec<f64>,
    colors: Vec<f64>,
    uv_sets: Vec<Vec<f64>>,
    uv_set_names: Vec<CString>,
    material_slots: Vec<i32>,
    face_materials: Vec<i32>,
}

/// Build the payload for one output file.
fn build_scene(
    lods: &[ProcessedLod],
    source: &ModelData,
    options: &ExportOptions,
    report: &mut ExportReport,
) -> Result<SceneData, OptError> {
    let mut scene = SceneData {
        nodes: Vec::new(),
        materials: Vec::new(),
        meshes: Vec::new(),
    };

    for (index, material) in source.materials.iter().enumerate() {
        scene.materials.push(MaterialData {
            name: c_string(&material.name, &format!("Material{index}")),
            base_color: [
                f64::from(material.base_color.x),
                f64::from(material.base_color.y),
                f64::from(material.base_color.z),
            ],
            emissive: [
                f64::from(material.emissive.x),
                f64::from(material.emissive.y),
                f64::from(material.emissive.z),
            ],
        });
    }

    // Whether more than one level shares this file decides whether meshes need a
    // `_LOD<n>` suffix to stay distinguishable.
    let suffix_levels = lods.len() > 1;

    // Source node index → scene node index for every node already emitted into
    // this file, so meshes sharing ancestors share one chain (see `place_node`).
    let mut placed = HashMap::new();

    for lod in lods {
        let groups = group_by_node(&lod.model);
        for group in groups {
            let (node_index, notes) = place_node(
                &mut scene,
                &mut placed,
                source,
                &group,
                options.hierarchy,
                lod.level,
            );
            report.notes.extend(notes);

            let mesh = build_mesh(
                &lod.model,
                &group,
                node_index,
                suffix_levels.then_some(lod.level),
                options.hierarchy,
                source,
            );
            report.triangle_count += mesh.triangle_count;
            scene.meshes.push(mesh);
        }
    }

    scene.mesh_count_into(report);
    if scene.meshes.is_empty() {
        return Err(OptError::EmptyMesh);
    }
    Ok(scene)
}

impl SceneData {
    fn mesh_count_into(&self, report: &mut ExportReport) {
        report.mesh_count += self.meshes.len();
    }
}

/// One mesh's worth of a processed level: the triangles owned by a single source
/// node, and the compacted vertex set they reference.
struct NodeGroup {
    /// Index into `ModelData::nodes`, or `None` when the model carries no node
    /// tags (everything then becomes one mesh).
    source_node: Option<usize>,
    /// Triangle indices into the level's index buffer.
    triangles: Vec<usize>,
}

/// Split a processed level into per-source-node groups, in first-seen order.
fn group_by_node(model: &ModelData) -> Vec<NodeGroup> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return Vec::new();
    }
    if model.triangles.node.len() != triangle_count {
        return vec![NodeGroup {
            source_node: None,
            triangles: (0..triangle_count).collect(),
        }];
    }

    let mut order: Vec<u32> = Vec::new();
    let mut groups: Vec<NodeGroup> = Vec::new();
    for triangle in 0..triangle_count {
        let node = model.triangles.node[triangle];
        let slot = match order.iter().position(|&seen| seen == node) {
            Some(slot) => slot,
            None => {
                order.push(node);
                groups.push(NodeGroup {
                    source_node: Some(node as usize),
                    triangles: Vec::new(),
                });
                groups.len() - 1
            }
        };
        groups[slot].triangles.push(triangle);
    }
    groups
}

/// Create (or reuse) the scene node this group's mesh attaches to, returning its
/// index plus any notes raised while working out its transform.
///
/// Under [`HierarchyMode::Rebuild`] the source node's ancestors are emitted too,
/// each with the local transform implied by the world transforms import
/// recorded. `placed` remembers every node already emitted into this scene, so
/// meshes with common ancestors — sibling LODs under one group node, or one
/// node's levels in a suffixed chain — hang off a single shared chain rather
/// than each duplicating it from the root. Only the mesh-bearing leaf carries
/// the level suffix, which puts a chain's levels beside each other as siblings —
/// the layout game FBX files use for their own LOD chains. The importer's
/// synthetic file root (nameless, meshless, parentless) is not re-emitted at
/// all: the written file has its own root, so an explicit copy would wrap every
/// re-import in one extra level.
///
/// Under [`HierarchyMode::FlatBaked`] a single identity root node is created per
/// mesh instead.
fn place_node(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    group: &NodeGroup,
    hierarchy: HierarchyMode,
    level: usize,
) -> (i32, Vec<String>) {
    let mut notes = Vec::new();

    let Some(source_index) = group
        .source_node
        .filter(|_| hierarchy == HierarchyMode::Rebuild)
    else {
        // Flat: one identity root per mesh, holding world-space geometry.
        let name = group
            .source_node
            .and_then(|index| source.nodes.get(index))
            .map(|node| node.name.clone())
            .unwrap_or_else(|| source.name.clone());
        scene.nodes.push(NodeData {
            name: c_string(&suffixed(&name, level), "Mesh"),
            parent: NO_PARENT,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scaling: [1.0; 3],
        });
        return ((scene.nodes.len() - 1) as i32, notes);
    };

    // Emit the ancestor chain root-first, so every parent exists (and precedes
    // its child in the array) before the child is created — which is exactly the
    // ordering the bridge validates.
    let mut chain = Vec::new();
    let mut cursor = Some(source_index);
    while let Some(index) = cursor {
        chain.push(index);
        cursor = source.nodes.get(index).and_then(|node| node.parent);
        // A cycle in the imported hierarchy would loop forever; the chain can
        // never legitimately be longer than the node table.
        if chain.len() > source.nodes.len() {
            notes.push("A node's parent chain looped; that branch was exported flat.".to_owned());
            chain.truncate(1);
            break;
        }
    }
    chain.reverse();

    let mut parent = NO_PARENT;
    let mut parent_world = Mat4::IDENTITY;
    for &index in &chain {
        let Some(node) = source.nodes.get(index) else {
            continue;
        };
        let leaf = index == source_index;
        // The synthetic file root — skipped, per above. (Never the leaf, so the
        // mesh always has a real node to attach to.) `parent_world` is left
        // alone so its transform — import parks the unit normalization there —
        // folds into its children's locals instead of vanishing.
        if !leaf && node.parent.is_none() && node.name.is_empty() && node.mesh_part.is_none() {
            continue;
        }
        // Reuse a node an earlier group already emitted. A suffixed leaf is a
        // per-level variant of its source node, so it is never shared.
        let suffix = leaf && level != 0;
        if !suffix && let Some(&existing) = placed.get(&index) {
            parent = existing;
            parent_world = node.transform;
            continue;
        }
        // Local = inverse(parent world) * world. A non-invertible parent (a zero
        // scale axis) leaves the child at the parent's origin rather than
        // producing NaNs.
        let inverse = parent_world.inverse();
        let local = if inverse.is_finite() {
            inverse * node.transform
        } else {
            notes.push(format!(
                "'{}' has a non-invertible transform; its children were exported \
                 relative to it without it.",
                node.name
            ));
            node.transform
        };
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        let rotation = if rotation.is_finite() {
            rotation
        } else {
            Quat::IDENTITY
        };
        let scale = if scale.is_finite() { scale } else { Vec3::ONE };

        let name = if suffix {
            suffixed(&node.name, level)
        } else {
            node.name.clone()
        };
        scene.nodes.push(NodeData {
            name: c_string(&name, "Node"),
            parent,
            translation: [
                f64::from(translation.x),
                f64::from(translation.y),
                f64::from(translation.z),
            ],
            rotation: [
                f64::from(rotation.x),
                f64::from(rotation.y),
                f64::from(rotation.z),
                f64::from(rotation.w),
            ],
            scaling: [f64::from(scale.x), f64::from(scale.y), f64::from(scale.z)],
        });
        parent = (scene.nodes.len() - 1) as i32;
        if !suffix {
            placed.insert(index, parent);
        }
        parent_world = node.transform;
    }

    (parent, notes)
}

/// `"Body"` at level 0, `"Body_LOD2"` above it. Level 0 keeps the plain name so
/// a chain-less export round-trips with the source's own names.
fn suffixed(name: &str, level: usize) -> String {
    if level == 0 {
        name.to_owned()
    } else {
        format!("{name}_LOD{level}")
    }
}

/// Build one mesh's flat arrays from `group`'s triangles.
fn build_mesh(
    model: &ModelData,
    group: &NodeGroup,
    node: i32,
    suffix_level: Option<usize>,
    hierarchy: HierarchyMode,
    source: &ModelData,
) -> MeshData {
    // Geometry is world-baked; under `Rebuild` it has to move back into the
    // owning node's local space, or it would be transformed twice on import.
    let to_local = (hierarchy == HierarchyMode::Rebuild)
        .then(|| {
            group
                .source_node
                .and_then(|index| source.nodes.get(index))
                .map(|node| node.transform.inverse())
                .filter(|inverse| inverse.is_finite())
        })
        .flatten();

    let channel_count = model.uv_channels.len();
    let has_uv = channel_count > 0 || !model.vertices.is_empty();

    let mut local_of_global: Vec<i32> = vec![-1; model.vertices.len()];
    let mut positions: Vec<f64> = Vec::new();
    let mut normals: Vec<f64> = Vec::new();
    let mut colors: Vec<f64> = Vec::new();
    let mut uv_sets: Vec<Vec<f64>> = vec![Vec::new(); channel_count.max(usize::from(has_uv))];
    let mut indices: Vec<i32> = Vec::with_capacity(group.triangles.len() * 3);

    // An out-of-range index means a malformed mesh, and the whole triangle has
    // to go: emitting the corners that are in range would leave an index buffer
    // that is no longer a multiple of three and shift every later triangle by
    // one. Both loops below walk `group.triangles`, so both consult this.
    let whole_triangle = |triangle: usize| {
        model.indices[triangle * 3..triangle * 3 + 3]
            .iter()
            .all(|&index| (index as usize) < model.vertices.len())
    };

    for &triangle in &group.triangles {
        if !whole_triangle(triangle) {
            continue;
        }
        for corner in 0..3 {
            let global = model.indices[triangle * 3 + corner] as usize;
            // In range, so both lookups hit: `local_of_global` is sized to the
            // model's vertex array.
            let slot = &mut local_of_global[global];
            if *slot < 0 {
                let vertex = model.vertices[global];
                *slot = (positions.len() / 3) as i32;

                let position = match to_local {
                    Some(inverse) => inverse.transform_point3(vertex.position),
                    None => vertex.position,
                };
                positions.extend_from_slice(&[
                    f64::from(position.x),
                    f64::from(position.y),
                    f64::from(position.z),
                ]);

                // Normals are directions: they transform by the inverse
                // transpose, and must be renormalized after a non-uniform scale.
                let normal = match to_local {
                    Some(inverse) => inverse
                        .transpose()
                        .transform_vector3(vertex.normal)
                        .normalize_or(vertex.normal),
                    None => vertex.normal,
                };
                normals.extend_from_slice(&[
                    f64::from(normal.x),
                    f64::from(normal.y),
                    f64::from(normal.z),
                ]);

                colors.extend_from_slice(&[
                    f64::from(vertex.vertex_color.x),
                    f64::from(vertex.vertex_color.y),
                    f64::from(vertex.vertex_color.z),
                    f64::from(vertex.vertex_color.w),
                ]);

                if channel_count == 0 {
                    if let Some(set) = uv_sets.first_mut() {
                        set.extend_from_slice(&[f64::from(vertex.uv.x), f64::from(vertex.uv.y)]);
                    }
                } else {
                    for (channel, set) in uv_sets.iter_mut().enumerate() {
                        let uv = model
                            .uv_channels
                            .get(channel)
                            .and_then(|uvs| uvs.get(global))
                            .copied()
                            .unwrap_or_default();
                        set.extend_from_slice(&[f64::from(uv.x), f64::from(uv.y)]);
                    }
                }
            }
            indices.push(*slot);
        }
    }

    // Per-face material, expressed as indices into this mesh's own slot list —
    // which is the order the bridge connects them to the node in.
    let mut material_slots: Vec<i32> = Vec::new();
    let mut face_materials: Vec<i32> = Vec::with_capacity(group.triangles.len());
    let triangle_count = model.indices.len() / 3;
    if model.triangles.material.len() == triangle_count {
        for &triangle in &group.triangles {
            if !whole_triangle(triangle) {
                continue;
            }
            let global = model.triangles.material[triangle];
            // The no-material sentinel maps to slot 0 of an empty list, which the
            // bridge writes as "no material".
            if global == u32::MAX || global as usize >= source.materials.len() {
                face_materials.push(0);
                continue;
            }
            let slot = match material_slots
                .iter()
                .position(|&seen| seen == global as i32)
            {
                Some(slot) => slot,
                None => {
                    material_slots.push(global as i32);
                    material_slots.len() - 1
                }
            };
            face_materials.push(slot as i32);
        }
    }
    if material_slots.len() <= 1 {
        // The bridge only reads per-face assignment for a multi-material mesh.
        face_materials.clear();
    }

    let name = group
        .source_node
        .and_then(|index| source.nodes.get(index))
        .map(|node| node.name.clone())
        .unwrap_or_else(|| model.name.clone());
    let name = match suffix_level {
        Some(level) => suffixed(&name, level),
        None => name,
    };

    let uv_set_names: Vec<CString> = (0..uv_sets.len())
        .map(|channel| {
            let label = source
                .uv_set_names
                .get(channel)
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("UVMap{channel}"));
            c_string(&label, "UVMap")
        })
        .collect();

    let vertex_count = positions.len() / 3;
    MeshData {
        name: c_string(&name, "Mesh"),
        node,
        positions,
        triangle_count: indices.len() / 3,
        indices,
        vertex_count,
        normals,
        colors,
        uv_sets,
        uv_set_names,
        material_slots,
        face_materials,
    }
}

/// A C string from `text`, falling back to `fallback` when the text is empty or
/// contains an interior NUL (which a name read from a file legitimately might).
fn c_string(text: &str, fallback: &str) -> CString {
    if text.is_empty() {
        return CString::new(fallback).unwrap_or_default();
    }
    CString::new(text.replace('\0', ""))
        .unwrap_or_else(|_| CString::new(fallback).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// The FFI call
// ---------------------------------------------------------------------------

/// Hand one assembled scene to the bridge.
#[cfg(has_ufbxw)]
fn write_scene(scene: &SceneData, path: &Path, format: FbxFormat) -> Result<(), OptError> {
    use crate::export_ffi::{RvoExportMaterial, RvoExportMesh, RvoExportNode, RvoExportScene};

    let path_string = path.to_string_lossy().into_owned();
    let c_path = CString::new(path_string)
        .map_err(|_| OptError::Export("the output path contains a NUL byte".to_owned()))?;

    // The `repr(C)` mirrors borrow every buffer in `scene`, which outlives this
    // function; the bridge in turn only borrows them for the duration of the
    // call, so nothing here escapes.
    let nodes: Vec<RvoExportNode> = scene
        .nodes
        .iter()
        .map(|node| RvoExportNode {
            name: node.name.as_ptr(),
            parent: node.parent,
            translation: node.translation,
            rotation: node.rotation,
            scaling: node.scaling,
        })
        .collect();
    let materials: Vec<RvoExportMaterial> = scene
        .materials
        .iter()
        .map(|material| RvoExportMaterial {
            name: material.name.as_ptr(),
            base_color: material.base_color,
            emissive: material.emissive,
        })
        .collect();

    // Pointer tables for the per-mesh UV sets, kept alive alongside the mesh
    // mirrors below.
    let uv_pointers: Vec<(Vec<*const f64>, Vec<*const c_char>)> = scene
        .meshes
        .iter()
        .map(|mesh| {
            (
                mesh.uv_sets.iter().map(|set| set.as_ptr()).collect(),
                mesh.uv_set_names.iter().map(|name| name.as_ptr()).collect(),
            )
        })
        .collect();

    let meshes: Vec<RvoExportMesh> = scene
        .meshes
        .iter()
        .zip(&uv_pointers)
        .map(|(mesh, (sets, names))| RvoExportMesh {
            name: mesh.name.as_ptr(),
            node: mesh.node,
            positions: mesh.positions.as_ptr(),
            vertex_count: mesh.vertex_count,
            indices: mesh.indices.as_ptr(),
            triangle_count: mesh.triangle_count,
            normals: optional(&mesh.normals),
            colors: optional(&mesh.colors),
            uv_sets: if sets.is_empty() {
                std::ptr::null()
            } else {
                sets.as_ptr()
            },
            uv_set_names: if names.is_empty() {
                std::ptr::null()
            } else {
                names.as_ptr()
            },
            uv_set_count: sets.len(),
            material_slots: if mesh.material_slots.is_empty() {
                std::ptr::null()
            } else {
                mesh.material_slots.as_ptr()
            },
            material_slot_count: mesh.material_slots.len(),
            face_materials: if mesh.face_materials.is_empty() {
                std::ptr::null()
            } else {
                mesh.face_materials.as_ptr()
            },
        })
        .collect();

    let payload = RvoExportScene {
        unit_scale_cm: UNIT_SCALE_CM,
        nodes: nodes.as_ptr(),
        node_count: nodes.len(),
        materials: if materials.is_empty() {
            std::ptr::null()
        } else {
            materials.as_ptr()
        },
        material_count: materials.len(),
        meshes: meshes.as_ptr(),
        mesh_count: meshes.len(),
    };

    // Element type inferred as `c_char` from the call below, whose signedness is
    // the platform's rather than a fixed `i8`.
    let mut error = [0; crate::export_ffi::ERROR_LENGTH];
    // SAFETY: `payload` and every array it points at are live for this call and
    // sized exactly by the counts beside them; `c_path` is a valid NUL-terminated
    // string; `error` is a live buffer of exactly `ERROR_LENGTH` bytes, which is
    // what the length argument declares. The C side validates the payload's
    // internal consistency (index ranges, parent ordering) before using it, and
    // frees everything it allocates on every path.
    let status = unsafe {
        crate::export_ffi::review_export_fbx(
            &raw const payload,
            c_path.as_ptr(),
            c_int::from(format == FbxFormat::Ascii),
            error.as_mut_ptr(),
            error.len(),
        )
    };

    if status != 0 {
        return Err(OptError::Export(error_message(&error)));
    }
    Ok(())
}

#[cfg(not(has_ufbxw))]
fn write_scene(_scene: &SceneData, _path: &Path, _format: FbxFormat) -> Result<(), OptError> {
    Err(OptError::Unavailable)
}

/// A null pointer for an empty buffer — the bridge reads "attribute absent".
#[cfg(has_ufbxw)]
fn optional(values: &[f64]) -> *const f64 {
    if values.is_empty() {
        std::ptr::null()
    } else {
        values.as_ptr()
    }
}

/// Read the bridge's NUL-terminated message out of its fixed buffer.
#[cfg(has_ufbxw)]
fn error_message(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    let message = String::from_utf8_lossy(&bytes).into_owned();
    if message.is_empty() {
        "the FBX writer failed without reporting a reason".to_owned()
    } else {
        message
    }
}

/// Unit coverage for the pure helpers. The FFI write path is covered end to end
/// by `tests/export_round_trip.rs`, which reads every written file back.
#[cfg(test)]
mod tests {
    use super::*;

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
            kind: review_model::NodeKind::Mesh,
            bone: None,
        };
        ModelData {
            name: "Looped".to_owned(),
            nodes: vec![node("A", 1), node("B", 0)],
            ..ModelData::default()
        }
    }

    #[test]
    fn place_node_cuts_a_parent_cycle_and_says_so() {
        let source = cyclic_model();
        let mut scene = SceneData {
            nodes: Vec::new(),
            materials: Vec::new(),
            meshes: Vec::new(),
        };
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
            notes.iter().any(|note| note.contains("looped")),
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

    fn empty_scene() -> SceneData {
        SceneData {
            nodes: Vec::new(),
            materials: Vec::new(),
            meshes: Vec::new(),
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
        assert_eq!(
            scene.nodes[2].parent, 0,
            "the suffixed level sits beside level 0, not under a duplicate chain"
        );
    }
}
