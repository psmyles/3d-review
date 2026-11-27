//! Writing a processed LOD chain as glb: binary glTF 2.0, in safe Rust.
//!
//! ## What is written
//!
//! The scene graph — every node, with its own transform and `extras` — and one
//! mesh per mesh-bearing node, one primitive per material, each with its own
//! compacted vertices: `POSITION`, `NORMAL`, every UV set, `COLOR_0` in the type
//! the source stored it in (never narrower), `TANGENT` where the source had
//! tangents, and the source's custom attributes (`_CAD_FACE`) in their own
//! types unless the export is told to leave them out. A glTF source's
//! materials, textures, samplers and images are written back verbatim
//! (`GltfSource`); any other source's materials are written from their PBR
//! factors.
//!
//! ## Exactly as it was, where it was not changed
//!
//! Import bakes every vertex into world space and turns V up; neither is
//! exact once undone. So a vertex the stack left bit-for-bit as the import
//! made it is written with the values the file stored (`GltfVertices`), and
//! only one an operation moved is transformed back. With the vertices
//! compacted in first-use order, a glb this writer made, read and written by a
//! no-op stack, comes out byte for byte the same.
//!
//! ## LOD chains
//!
//! As the FBX export lays them out (`nodes::place_level_node`): one file per
//! level puts each level's meshes on the source nodes; one file for the chain
//! names each level's root `<name>_LOD<n>`, level 0 on the source nodes and each
//! further level on a copy of the path from that root down to the mesh, beside
//! the root. A generated chain replaces the file's own lower LOD levels.
//!
//! The output depends only on its input: no clock, no hash order, and JSON
//! written from structs (field order fixed by their declaration) and
//! `BTreeMap`s.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
use review_model::naming::{self, LodRole};
use review_model::{AttributeFormat, ComponentType, GltfSource, ModelData};
use serde::Serialize;
use serde_json::value::RawValue;

use super::{ExportReport, check_cancel, dedup_notes, planned_files};
use crate::notice::ExportNote;
use crate::process::ProcessedLod;
use crate::replace_file::Staged;
use crate::stack::{ExportOptions, GlbTangents, HierarchyMode, LodPackaging};
use crate::{CancelToken, OptError};

/// glTF's own axes: +X right, +Y up, +Z toward the viewer.
const GLTF_AXES: review_model::CoordinateAxes = review_model::CoordinateAxes {
    right: review_model::Axis::PositiveX,
    up: review_model::Axis::PositiveY,
    front: review_model::Axis::PositiveZ,
};

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

/// Write `lods` to `path` as glb, laid out as `options` says. The same
/// staging and commit as [`super::export_fbx`]: every file is complete on disk
/// before any is put in place.
pub fn export_glb(
    lods: &[ProcessedLod],
    source: &ModelData,
    path: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, OptError> {
    glb(lods, source, path, options, None)
}

/// [`export_glb`], checking `cancel` as [`super::export_cancellable`] describes.
pub(super) fn glb(
    lods: &[ProcessedLod],
    source: &ModelData,
    path: &Path,
    options: &ExportOptions,
    cancel: Option<&CancelToken>,
) -> Result<ExportReport, OptError> {
    let _z = crate::prof::zone!("Export glb");
    if lods.is_empty() {
        return Err(OptError::EmptyMesh);
    }
    let mut report = ExportReport {
        files: Vec::new(),
        mesh_count: 0,
        triangle_count: 0,
        notes: Vec::new(),
    };
    if source.gltf.is_none() {
        report.notes.push(ExportNote::FbxIntoGlb);
    }
    if source.needs_deform() {
        report.notes.push(ExportNote::GlbNoDeform);
    }
    if lods.iter().any(|lod| !lod.carry.color_channels.is_empty()) {
        report.notes.push(ExportNote::GlbExtraColorSets);
    }

    let generated = lods.len() > 1;
    let paths = planned_files(lods.iter().map(|lod| lod.level), path, options);
    let mut staged = Vec::with_capacity(paths.len());
    let files: Vec<Vec<&ProcessedLod>> = match options.packaging {
        LodPackaging::SingleFileSuffixed => vec![lods.iter().collect()],
        LodPackaging::FilePerLod => lods.iter().map(|lod| vec![lod]).collect(),
    };
    for (levels, file) in files.iter().zip(&paths) {
        check_cancel(cancel)?;
        let bytes = build_file(levels, generated, source, options, &mut report)?;
        staged.push(Staged::write(file, |staging| {
            std::fs::write(staging, &bytes).map_err(OptError::from)
        })?);
    }
    check_cancel(cancel)?;
    for entry in staged {
        let destination = entry.destination().to_path_buf();
        match entry.commit() {
            Ok(committed) => report.files.push(committed),
            Err(error) => {
                dedup_notes(&mut report.notes);
                return Err(OptError::ExportIncomplete {
                    replaced: report.files,
                    failed: destination,
                    reason: error.to_string(),
                });
            }
        }
    }
    dedup_notes(&mut report.notes);
    Ok(report)
}

/// One file's bytes: the levels in `levels`, `generated` saying whether the
/// export as a whole is a generated chain (which replaces authored levels).
pub(crate) fn build_file(
    levels: &[&ProcessedLod],
    generated: bool,
    source: &ModelData,
    options: &ExportOptions,
    report: &mut ExportReport,
) -> Result<Vec<u8>, OptError> {
    let mut writer = Writer::new(source, options, generated, levels.len() > 1);
    writer.place_source_graph();
    for lod in levels {
        writer.write_level(lod);
    }
    report.mesh_count += writer.meshes.len();
    report.triangle_count += writer.triangles;
    report.notes.append(&mut writer.notes);
    Ok(writer.finish())
}

// ---------------------------------------------------------------------------
// The JSON, as structs: their declaration order is the file's field order.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Document {
    asset: Asset,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    extensions_used: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    extensions_required: Vec<String>,
    scene: u32,
    scenes: Vec<Scene>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nodes: Vec<Node>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    meshes: Vec<Mesh>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    materials: Vec<Box<RawValue>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    textures: Vec<Box<RawValue>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    samplers: Vec<Box<RawValue>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<Box<RawValue>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    accessors: Vec<Accessor>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    buffer_views: Vec<BufferView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    buffers: Vec<Buffer>,
}

#[derive(Serialize)]
struct Asset {
    version: &'static str,
    generator: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    copyright: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extras: Option<Box<RawValue>>,
}

#[derive(Serialize)]
struct Scene {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    nodes: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extras: Option<Box<RawValue>>,
}

#[derive(Serialize, Clone)]
struct Node {
    #[serde(skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mesh: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matrix: Option<[f32; 16]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotation: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    translation: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extras: Option<Box<RawValue>>,
    /// Not written: the parent, for placing children and finding the roots.
    #[serde(skip)]
    parent: Option<u32>,
}

#[derive(Serialize)]
struct Mesh {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    primitives: Vec<Primitive>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extras: Option<Box<RawValue>>,
}

#[derive(Serialize)]
struct Primitive {
    attributes: BTreeMap<String, u32>,
    indices: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    material: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Accessor {
    buffer_view: u32,
    component_type: u32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    normalized: bool,
    count: usize,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<Vec<f32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<Vec<f32>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BufferView {
    buffer: u32,
    byte_length: usize,
    byte_offset: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_stride: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Buffer {
    byte_length: usize,
}

fn raw(text: Option<&str>) -> Option<Box<RawValue>> {
    RawValue::from_string(text?.to_owned()).ok()
}

fn kind_of(components: u8) -> &'static str {
    match components {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        _ => "VEC4",
    }
}

// ---------------------------------------------------------------------------
// The binary chunk.

#[derive(Default)]
struct Bin {
    bytes: Vec<u8>,
    views: Vec<BufferView>,
    accessors: Vec<Accessor>,
}

impl Bin {
    fn align(&mut self) {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
    }

    /// A vertex attribute: `data` holds `count` packed elements of `format`.
    /// Each element starts on a four-byte boundary, as glTF requires of vertex
    /// data, so a narrow one (a `u16` scalar, a `u8x3`) is padded and its view
    /// given a stride.
    fn attribute(
        &mut self,
        data: &[u8],
        format: AttributeFormat,
        count: usize,
        bounds: Option<(Vec<f32>, Vec<f32>)>,
    ) -> u32 {
        let element = format.size();
        let stride = element.next_multiple_of(4);
        self.align();
        let offset = self.bytes.len();
        if stride == element {
            self.bytes.extend_from_slice(data);
        } else {
            for value in data.chunks_exact(element) {
                self.bytes.extend_from_slice(value);
                self.bytes.resize(self.bytes.len() + stride - element, 0);
            }
        }
        self.views.push(BufferView {
            buffer: 0,
            byte_length: self.bytes.len() - offset,
            byte_offset: offset,
            byte_stride: (stride != element).then_some(stride),
            target: Some(ARRAY_BUFFER),
        });
        let (min, max) = bounds.unzip();
        self.accessors.push(Accessor {
            buffer_view: (self.views.len() - 1) as u32,
            component_type: format.component.code(),
            normalized: format.normalized,
            count,
            kind: kind_of(format.components),
            max,
            min,
        });
        (self.accessors.len() - 1) as u32
    }

    /// An index buffer, in the narrowest type that holds it. glTF reserves each
    /// type's largest value (primitive restart), so 16 bits hold up to 65534.
    fn indices(&mut self, indices: &[u32]) -> u32 {
        let largest = indices.iter().copied().max().unwrap_or(0);
        self.align();
        let offset = self.bytes.len();
        let component = if largest < u32::from(u16::MAX) {
            for &index in indices {
                self.bytes.extend_from_slice(&(index as u16).to_le_bytes());
            }
            ComponentType::U16
        } else {
            for &index in indices {
                self.bytes.extend_from_slice(&index.to_le_bytes());
            }
            ComponentType::U32
        };
        self.views.push(BufferView {
            buffer: 0,
            byte_length: self.bytes.len() - offset,
            byte_offset: offset,
            byte_stride: None,
            target: Some(ELEMENT_ARRAY_BUFFER),
        });
        self.accessors.push(Accessor {
            buffer_view: (self.views.len() - 1) as u32,
            component_type: component.code(),
            normalized: false,
            count: indices.len(),
            kind: "SCALAR",
            max: None,
            min: None,
        });
        (self.accessors.len() - 1) as u32
    }

    /// An image's bytes: a plain view, no target.
    fn image(&mut self, bytes: &[u8]) -> u32 {
        self.align();
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(bytes);
        self.views.push(BufferView {
            buffer: 0,
            byte_length: bytes.len(),
            byte_offset: offset,
            byte_stride: None,
            target: None,
        });
        (self.views.len() - 1) as u32
    }
}

// ---------------------------------------------------------------------------
// The writer.

struct Writer<'a> {
    source: &'a ModelData,
    gltf: Option<&'a GltfSource>,
    options: &'a ExportOptions,
    roles: Vec<LodRole>,
    /// Several levels share this file: each level's root carries its suffix.
    chain: bool,
    /// The rotation from the source's axes to glTF's, applied at the roots;
    /// `None` when they are glTF's already (or cannot be rotated into them).
    axis_fix: Option<Quat>,
    nodes: Vec<Node>,
    /// Source node -> the node written for it, when it was written.
    placed: Vec<Option<u32>>,
    /// (source node, level) -> its copy in that level's branch.
    copies: HashMap<(usize, usize), u32>,
    meshes: Vec<Mesh>,
    bin: Bin,
    triangles: usize,
    notes: Vec<ExportNote>,
}

/// One node's share of a level: its triangles, grouped by material, each group
/// in the level's triangle order, the groups in first-seen order.
struct NodeGroup {
    node: usize,
    materials: Vec<(u32, Vec<usize>)>,
}

impl<'a> Writer<'a> {
    fn new(
        source: &'a ModelData,
        options: &'a ExportOptions,
        generated: bool,
        chain: bool,
    ) -> Self {
        let roles = if generated {
            naming::lod_roles(source)
        } else {
            vec![LodRole::Plain; source.nodes.len()]
        };
        let mut notes = Vec::new();
        let axis_fix = match source.stats.source_axes {
            Some(axes) if axes != GLTF_AXES => match axes.rotation_to(GLTF_AXES) {
                Some(rotation) => Some(rotation),
                None => {
                    notes.push(ExportNote::GlbAxesMirrored);
                    None
                }
            },
            _ => None,
        };
        Self {
            source,
            gltf: source.gltf.as_deref(),
            options,
            roles,
            chain,
            axis_fix,
            nodes: Vec::new(),
            placed: vec![None; source.nodes.len()],
            copies: HashMap::new(),
            meshes: Vec::new(),
            bin: Bin::default(),
            triangles: 0,
            notes,
        }
    }

    fn rebuild(&self) -> bool {
        self.options.hierarchy == HierarchyMode::Rebuild
    }

    /// Add `node` under `parent` and return its index; a root takes the axis
    /// rotation.
    fn push_node(&mut self, mut node: Node, parent: Option<u32>) -> u32 {
        if parent.is_none()
            && let Some(fix) = self.axis_fix
        {
            match node.matrix {
                Some(matrix) => {
                    node.matrix = Some(
                        (Mat4::from_quat(fix) * Mat4::from_cols_array(&matrix)).to_cols_array(),
                    );
                }
                None => {
                    let rotation = node.rotation.map_or(Quat::IDENTITY, Quat::from_array);
                    let translation = node.translation.map_or(Vec3::ZERO, Vec3::from_array);
                    node.rotation = Some((fix * rotation).to_array());
                    node.translation = Some((fix * translation).to_array());
                }
            }
        }
        node.parent = parent;
        let index = self.nodes.len() as u32;
        self.nodes.push(node);
        if let Some(parent) = parent {
            self.nodes[parent as usize].children.push(index);
        }
        index
    }

    /// The written form of source node `index`, named `name`, with no mesh.
    fn node_data(&self, index: usize, name: String) -> Node {
        let node = &self.source.nodes[index];
        let gltf = self.gltf.and_then(|gltf| gltf.nodes.get(index));
        let matrix = gltf.and_then(|gltf| gltf.matrix);
        let local = node.rest_local;
        let (rotation, scale, translation) = if matrix.is_some() {
            (None, None, None)
        } else {
            (
                (local.rotation != Quat::IDENTITY).then(|| local.rotation.to_array()),
                (local.scale != Vec3::ONE).then(|| local.scale.to_array()),
                (local.translation != Vec3::ZERO).then(|| local.translation.to_array()),
            )
        };
        Node {
            name,
            children: Vec::new(),
            mesh: None,
            matrix,
            rotation,
            scale,
            translation,
            extras: raw(gltf.and_then(|gltf| gltf.extras.as_deref())),
            parent: None,
        }
    }

    /// Write the source's scene graph, every node but those of an authored
    /// level the generated chain replaces. Rebuild only.
    fn place_source_graph(&mut self) {
        if !self.rebuild() {
            return;
        }
        for index in 0..self.source.nodes.len() {
            if self.roles[index] == LodRole::Replaced {
                continue;
            }
            let parent = match self.source.nodes[index].parent {
                Some(parent) => match self.placed.get(parent).copied().flatten() {
                    Some(placed) => Some(placed),
                    // Under a node that was not written: so is this.
                    None => continue,
                },
                None => None,
            };
            let data = self.node_data(index, self.source.nodes[index].name.clone());
            self.placed[index] = Some(self.push_node(data, parent));
        }
    }

    /// The node `node`'s mesh for `level` goes on, created as the chain's
    /// layout needs it.
    fn target_node(&mut self, node: usize, level: usize) -> Option<u32> {
        let name = &self.source.nodes[node].name;
        let (root, renumber) = match self.roles.get(node) {
            Some(LodRole::Source { root }) => (*root, true),
            _ => (node, false),
        };
        if !self.rebuild() {
            // One root node per mesh, in world space.
            let name = if self.chain || renumber {
                naming::with_lod(name, level)
            } else {
                name.clone()
            };
            let mut data = self.node_data(node, name);
            data.matrix = None;
            data.rotation = None;
            data.scale = None;
            data.translation = None;
            // The axis rotation is in the geometry already.
            let fix = self.axis_fix.take();
            let index = self.push_node(data, None);
            self.axis_fix = fix;
            return Some(index);
        }
        if !self.chain || level == 0 {
            if (self.chain || renumber)
                && let Some(at) = self.placed.get(root).copied().flatten()
            {
                self.nodes[at as usize].name =
                    naming::with_lod(&self.source.nodes[root].name, level);
            }
            return self.placed.get(node).copied().flatten();
        }
        // A further level of a chain: a copy of the path from the level's root
        // down to the mesh, beside the root.
        let mut path = vec![node];
        while *path.last()? != root {
            if path.len() > self.source.nodes.len() {
                path = vec![node];
                break;
            }
            match self.source.nodes[*path.last()?].parent {
                Some(parent) => path.push(parent),
                None => {
                    path = vec![node];
                    break;
                }
            }
        }
        path.reverse();
        let root = path[0];
        let mut parent = self
            .placed
            .get(root)
            .copied()
            .flatten()
            .and_then(|at| self.nodes[at as usize].parent);
        for &index in &path {
            if let Some(&copy) = self.copies.get(&(index, level)) {
                parent = Some(copy);
                continue;
            }
            let name = if index == root {
                naming::with_lod(&self.source.nodes[index].name, level)
            } else {
                self.source.nodes[index].name.clone()
            };
            let data = self.node_data(index, name);
            let copy = self.push_node(data, parent);
            self.copies.insert((index, level), copy);
            parent = Some(copy);
        }
        parent
    }

    fn write_level(&mut self, lod: &ProcessedLod) {
        for group in groups(&lod.model) {
            if group.node >= self.source.nodes.len() && !self.source.nodes.is_empty() {
                continue;
            }
            let Some(target) = (if self.source.nodes.is_empty() {
                // A model with no node table: one root for its mesh.
                let node = Node {
                    name: self.source.name.clone(),
                    children: Vec::new(),
                    mesh: None,
                    matrix: None,
                    rotation: None,
                    scale: None,
                    translation: None,
                    extras: None,
                    parent: None,
                };
                Some(self.push_node(node, None))
            } else {
                self.target_node(group.node, lod.level)
            }) else {
                continue;
            };
            let mesh = self.write_mesh(lod, &group);
            if let Some(mesh) = mesh {
                self.nodes[target as usize].mesh = Some(mesh);
            }
        }
    }

    /// The node's world transform, which its geometry is taken back out of.
    fn world_of(&self, node: usize) -> Mat4 {
        self.source
            .nodes
            .get(node)
            .map_or(Mat4::IDENTITY, |node| node.transform)
    }

    fn write_mesh(&mut self, lod: &ProcessedLod, group: &NodeGroup) -> Option<u32> {
        let space = if self.rebuild() {
            let world = self.world_of(group.node);
            let inverse = world.inverse();
            if inverse.is_finite() {
                Space::Local {
                    inverse,
                    linear: Mat3::from_mat4(world),
                    mirrored: Mat3::from_mat4(world).determinant() < 0.0,
                }
            } else {
                self.notes.push(ExportNote::NonInvertibleTransform {
                    node: self.source.nodes[group.node].name.clone(),
                });
                Space::World(Quat::IDENTITY)
            }
        } else {
            Space::World(self.axis_fix.unwrap_or(Quat::IDENTITY))
        };
        let mut primitives = Vec::with_capacity(group.materials.len());
        for (material, triangles) in &group.materials {
            if triangles.is_empty() {
                continue;
            }
            self.triangles += triangles.len();
            primitives.push(self.write_primitive(lod, *material, triangles, &space));
        }
        if primitives.is_empty() {
            return None;
        }
        let gltf_node = self.gltf.and_then(|gltf| gltf.nodes.get(group.node));
        let name = gltf_node
            .and_then(|node| node.mesh_name.clone())
            .or_else(|| {
                self.source
                    .nodes
                    .get(group.node)
                    .map(|node| node.name.clone())
            })
            .filter(|name| !name.is_empty());
        self.meshes.push(Mesh {
            name,
            primitives,
            extras: raw(gltf_node.and_then(|node| node.mesh_extras.as_deref())),
        });
        Some((self.meshes.len() - 1) as u32)
    }

    fn write_primitive(
        &mut self,
        lod: &ProcessedLod,
        material: u32,
        triangles: &[usize],
        space: &Space,
    ) -> Primitive {
        let level = &lod.model;
        // The primitive's vertices, in first-use order.
        let mut local_of: HashMap<u32, u32> = HashMap::new();
        let mut order: Vec<u32> = Vec::new();
        let mut indices: Vec<u32> = Vec::with_capacity(triangles.len() * 3);
        for &triangle in triangles {
            let corners = &level.indices[triangle * 3..triangle * 3 + 3];
            let corners = if space.mirrored() {
                // Undo the import's reversal (`import::gltf`, a negative
                // determinant).
                [corners[0], corners[2], corners[1]]
            } else {
                [corners[0], corners[1], corners[2]]
            };
            for vertex in corners {
                let next = order.len() as u32;
                let local = *local_of.entry(vertex).or_insert_with(|| {
                    order.push(vertex);
                    next
                });
                indices.push(local);
            }
        }

        let source = self.source;
        let originals = self.gltf.map(|gltf| &gltf.vertices);
        // Per written vertex: the source corner it is still bit-for-bit, and
        // that corner's glTF vertex — the values the file stored.
        let mut unchanged: Vec<Option<(usize, usize)>> = order
            .iter()
            .map(|&vertex| {
                let corner = *lod.carry.source_corner.get(vertex as usize)?;
                let corner = (corner != u32::MAX).then_some(corner as usize)?;
                let logical = *source.corner_to_logical.get(corner)? as usize;
                originals?;
                Some((corner, logical))
            })
            .collect();
        let local_space = matches!(space, Space::Local { .. });
        let view = Written {
            level,
            source,
            originals,
            space,
            local_space,
        };

        // MikkTSpace tangents, when asked for: worked out per corner from what
        // the file will hold — local positions and normals, the first UV set as
        // stored — and a vertex split wherever its corners disagree (a UV mirror).
        let mut mikktspace: Option<Vec<Vec4>> = None;
        let has_uvs = level.stats.uv_set_count > 0 || !level.uv_channels.is_empty();
        if self.options.glb_tangents == GlbTangents::MikkTSpace && has_uvs {
            let positions: Vec<f32> = order
                .iter()
                .zip(&unchanged)
                .flat_map(|(&vertex, &known)| view.position(vertex, known).to_array())
                .collect();
            let normals: Vec<f32> = order
                .iter()
                .zip(&unchanged)
                .flat_map(|(&vertex, &known)| view.normal(vertex, known).to_array())
                .collect();
            let uvs: Vec<f32> = order
                .iter()
                .zip(&unchanged)
                .flat_map(|(&vertex, &known)| view.uv(vertex, known, 0).to_array())
                .collect();
            if let Ok(corner_tangents) = crate::meshopt::generate_tangents_mikktspace(
                &indices,
                &positions,
                &normals,
                &uvs,
                order.len(),
            ) {
                let mut slot_of: HashMap<(u32, [u32; 4]), u32> = HashMap::new();
                let mut split_order = Vec::with_capacity(order.len());
                let mut split_unchanged = Vec::with_capacity(order.len());
                let mut computed = Vec::with_capacity(order.len());
                for (corner, index) in indices.iter_mut().enumerate() {
                    let tangent = Vec4::from_slice(&corner_tangents[corner * 4..corner * 4 + 4]);
                    let key = (*index, tangent.to_array().map(f32::to_bits));
                    let next = split_order.len() as u32;
                    *index = *slot_of.entry(key).or_insert_with(|| {
                        split_order.push(order[*index as usize]);
                        split_unchanged.push(unchanged[*index as usize]);
                        computed.push(tangent);
                        next
                    });
                }
                order = split_order;
                unchanged = split_unchanged;
                mikktspace = Some(computed);
            }
        }

        // Whether the primitive's source vertices had something: any of them,
        // when any is known; `otherwise` when none is (all made by an operation,
        // or a source that is not glTF). A primitive that had no UVs, tangents or
        // colours in the file gets none back.
        let had = |has: &dyn Fn(usize) -> bool, otherwise: bool| -> bool {
            let mut known = unchanged.iter().flatten().peekable();
            if known.peek().is_none() {
                return otherwise;
            }
            known.any(|&(_, logical)| has(logical))
        };
        let mut attributes = BTreeMap::new();

        // POSITION.
        let mut positions = Vec::with_capacity(order.len() * 12);
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (slot, &vertex) in order.iter().enumerate() {
            let written = view.position(vertex, unchanged[slot]);
            for axis in 0..3 {
                min[axis] = min[axis].min(written[axis]);
                max[axis] = max[axis].max(written[axis]);
            }
            positions.extend(
                written
                    .to_array()
                    .iter()
                    .flat_map(|value| value.to_le_bytes()),
            );
        }
        let f32x = |components| AttributeFormat {
            component: ComponentType::F32,
            components,
            normalized: false,
        };
        let position = self.bin.attribute(
            &positions,
            f32x(3),
            order.len(),
            Some((min.to_vec(), max.to_vec())),
        );
        attributes.insert("POSITION".to_owned(), position);

        // NORMAL.
        let mut normals = Vec::with_capacity(order.len() * 12);
        for (slot, &vertex) in order.iter().enumerate() {
            let written = view.normal(vertex, unchanged[slot]);
            normals.extend(
                written
                    .to_array()
                    .iter()
                    .flat_map(|value| value.to_le_bytes()),
            );
        }
        let normal = self.bin.attribute(&normals, f32x(3), order.len(), None);
        attributes.insert("NORMAL".to_owned(), normal);

        // TANGENT: where the source had tangents — every vertex of a non-glTF
        // source, each glTF vertex whose primitive carried them.
        let wants_tangents = match (self.options.glb_tangents, originals) {
            (GlbTangents::Omit, _) => false,
            (GlbTangents::MikkTSpace, _) => mikktspace.is_some(),
            (GlbTangents::AsSource, None) => true,
            (GlbTangents::AsSource, Some(originals)) => had(
                &|logical| originals.tangents[logical].is_some(),
                self.gltf.is_some_and(|gltf| gltf.tangents),
            ),
        };
        if wants_tangents {
            let mut tangents = Vec::with_capacity(order.len() * 16);
            for (slot, &vertex) in order.iter().enumerate() {
                let written = match &mikktspace {
                    Some(computed) => computed[slot],
                    None => view.tangent(vertex, unchanged[slot]),
                };
                tangents.extend(
                    written
                        .to_array()
                        .iter()
                        .flat_map(|value| value.to_le_bytes()),
                );
            }
            let tangent = self.bin.attribute(&tangents, f32x(4), order.len(), None);
            attributes.insert("TANGENT".to_owned(), tangent);
        }

        // TEXCOORD_n, V turned back down.
        let uv_sets = if level.uv_channels.is_empty() {
            level.stats.uv_set_count.min(1)
        } else {
            level.uv_channels.len()
        };
        for set in 0..uv_sets {
            // A set the primitive's source vertices did not have is left out,
            // unless an operation (an Unwrap) has given them one since.
            let changed =
                order
                    .iter()
                    .zip(&unchanged)
                    .any(|(&vertex, unchanged)| match unchanged {
                        Some((corner, _)) => {
                            level
                                .uv_for_channel(vertex as usize, set)
                                .to_array()
                                .map(f32::to_bits)
                                != source
                                    .uv_for_channel(*corner, set)
                                    .to_array()
                                    .map(f32::to_bits)
                        }
                        None => true,
                    });
            if let Some(originals) = originals
                && !changed
                && !had(
                    &|logical| {
                        originals
                            .uvs
                            .get(set)
                            .is_some_and(|uvs| uvs[logical].is_some())
                    },
                    true,
                )
            {
                continue;
            }
            let mut uvs = Vec::with_capacity(order.len() * 8);
            for (slot, &vertex) in order.iter().enumerate() {
                let written = view.uv(vertex, unchanged[slot], set);
                uvs.extend(
                    written
                        .to_array()
                        .iter()
                        .flat_map(|value| value.to_le_bytes()),
                );
            }
            let accessor = self.bin.attribute(&uvs, f32x(2), order.len(), None);
            attributes.insert(format!("TEXCOORD_{set}"), accessor);
        }

        // COLOR_0, in the source's type. A primitive whose source vertices had
        // none, and whose colours nothing has changed since, gets none.
        let colored = order
            .iter()
            .any(|&vertex| level.vertices[vertex as usize].vertex_color != Vec4::ONE)
            || originals.is_some_and(|originals| had(&|logical| originals.colored[logical], false));
        let color_format = self
            .gltf
            .and_then(|gltf| gltf.color_format)
            .unwrap_or(AttributeFormat::RGBA_F32);
        if colored {
            let mut colors = Vec::with_capacity(order.len() * color_format.size());
            for &vertex in &order {
                let color = level.vertices[vertex as usize].vertex_color;
                for component in color
                    .to_array()
                    .iter()
                    .take(usize::from(color_format.components))
                {
                    push_float(&mut colors, *component, color_format);
                }
            }
            let accessor = self.bin.attribute(&colors, color_format, order.len(), None);
            attributes.insert("COLOR_0".to_owned(), accessor);
        }

        // The custom attributes, in their own types.
        if !self.options.strip_custom_attributes {
            for attribute in &level.custom_attributes {
                if attribute.vertex_count() != level.vertices.len() {
                    continue;
                }
                let mut bytes = Vec::with_capacity(order.len() * attribute.format.size());
                for &vertex in &order {
                    for &word in attribute.value(vertex as usize) {
                        push_word(&mut bytes, word, attribute.format.component);
                    }
                }
                let accessor = self
                    .bin
                    .attribute(&bytes, attribute.format, order.len(), None);
                attributes.insert(attribute.name.clone(), accessor);
            }
        }

        let indices = self.bin.indices(&indices);
        let material_count = self.material_count();
        Primitive {
            attributes,
            indices,
            material: (material != u32::MAX && (material as usize) < material_count)
                .then_some(material),
        }
    }

    fn material_count(&self) -> usize {
        match self.gltf {
            Some(gltf) => gltf.materials.len(),
            None => self.source.materials.len(),
        }
    }

    /// The materials, textures, samplers and images, and the extensions they use.
    #[allow(clippy::type_complexity)]
    fn materials(
        &mut self,
    ) -> (
        Vec<Box<RawValue>>,
        Vec<Box<RawValue>>,
        Vec<Box<RawValue>>,
        Vec<Box<RawValue>>,
        BTreeSet<String>,
    ) {
        let mut used = BTreeSet::new();
        let Some(gltf) = self.gltf else {
            let materials = self
                .source
                .materials
                .iter()
                .filter_map(|material| {
                    RawValue::from_string(pbr_material(material, &mut used)).ok()
                })
                .collect();
            return (materials, Vec::new(), Vec::new(), Vec::new(), used);
        };
        let verbatim = |texts: &[String], used: &mut BTreeSet<String>| -> Vec<Box<RawValue>> {
            texts
                .iter()
                .filter_map(|text| {
                    collect_extensions(text, used);
                    RawValue::from_string(text.clone()).ok()
                })
                .collect()
        };
        let materials = verbatim(&gltf.materials, &mut used);
        let textures = verbatim(&gltf.textures, &mut used);
        let samplers = verbatim(&gltf.samplers, &mut used);
        let mut images = Vec::with_capacity(gltf.images.len());
        for image in &gltf.images {
            collect_extensions(&image.json, &mut used);
            let text = match &image.bytes {
                Some(bytes) => {
                    let view = self.bin.image(bytes);
                    match serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
                        &image.json,
                    ) {
                        Ok(mut object) => {
                            object.insert("bufferView".to_owned(), view.into());
                            serde_json::to_string(&object).unwrap_or_else(|_| image.json.clone())
                        }
                        Err(_) => image.json.clone(),
                    }
                }
                None => image.json.clone(),
            };
            if let Ok(value) = RawValue::from_string(text) {
                images.push(value);
            }
        }
        (materials, textures, samplers, images, used)
    }

    fn finish(mut self) -> Vec<u8> {
        let (materials, textures, samplers, images, used) = self.materials();
        let gltf = self.gltf;
        // The source's declared extensions that something written still uses,
        // in its order; one this writer adds itself (emissive strength) after.
        let mut extensions_used: Vec<String> = gltf
            .map(|gltf| {
                gltf.extensions_used
                    .iter()
                    .filter(|name| used.contains(*name))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for name in &used {
            if !extensions_used.contains(name) {
                extensions_used.push(name.clone());
            }
        }
        let extensions_required: Vec<String> = gltf
            .map(|gltf| {
                gltf.extensions_required
                    .iter()
                    .filter(|name| used.contains(*name))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let roots: Vec<u32> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.parent.is_none())
            .map(|(index, _)| index as u32)
            .collect();
        self.bin.align();
        let bin = std::mem::take(&mut self.bin);
        let document = Document {
            asset: Asset {
                version: "2.0",
                generator: format!("{} {}", env!("REVIEW_PRODUCT"), env!("REVIEW_VERSION")),
                copyright: gltf.and_then(|gltf| gltf.copyright.clone()),
                extras: raw(gltf.and_then(|gltf| gltf.asset_extras.as_deref())),
            },
            extensions_used,
            extensions_required,
            scene: 0,
            scenes: vec![Scene {
                name: gltf.and_then(|gltf| gltf.scene_name.clone()),
                nodes: roots,
                extras: raw(gltf.and_then(|gltf| gltf.scene_extras.as_deref())),
            }],
            nodes: std::mem::take(&mut self.nodes),
            meshes: std::mem::take(&mut self.meshes),
            materials,
            textures,
            samplers,
            images,
            accessors: bin.accessors,
            buffer_views: bin.views,
            buffers: if bin.bytes.is_empty() {
                Vec::new()
            } else {
                vec![Buffer {
                    byte_length: bin.bytes.len(),
                }]
            },
        };
        container(
            &serde_json::to_vec(&document).expect("the document serializes"),
            &bin.bytes,
        )
    }
}

/// What the file holds for a level vertex: the value the source file stored,
/// where the vertex is still bit-for-bit its source corner (`unchanged`), or
/// the level's value taken back out of world space (and V turned down).
struct Written<'a> {
    level: &'a ModelData,
    source: &'a ModelData,
    originals: Option<&'a review_model::GltfVertices>,
    space: &'a Space,
    local_space: bool,
}

impl Written<'_> {
    fn same(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
    }

    fn position(&self, vertex: u32, unchanged: Option<(usize, usize)>) -> Vec3 {
        let value = self.level.vertices[vertex as usize].position;
        match (unchanged, self.originals) {
            (Some((corner, logical)), Some(originals))
                if self.local_space
                    && Self::same(
                        &value.to_array(),
                        &self.source.vertices[corner].position.to_array(),
                    ) =>
            {
                originals.positions[logical]
            }
            _ => self.space.point(value),
        }
    }

    fn normal(&self, vertex: u32, unchanged: Option<(usize, usize)>) -> Vec3 {
        let value = self.level.vertices[vertex as usize].normal;
        match (unchanged, self.originals) {
            (Some((corner, logical)), Some(originals))
                if self.local_space
                    && Self::same(
                        &value.to_array(),
                        &self.source.vertices[corner].normal.to_array(),
                    ) =>
            {
                originals.normals[logical].unwrap_or_else(|| self.space.normal(value))
            }
            _ => self.space.normal(value),
        }
    }

    fn tangent(&self, vertex: u32, unchanged: Option<(usize, usize)>) -> Vec4 {
        let value = self.level.vertices[vertex as usize].tangent;
        match (unchanged, self.originals) {
            (Some((corner, logical)), Some(originals))
                if self.local_space
                    && Self::same(
                        &value.to_array(),
                        &self.source.vertices[corner].tangent.to_array(),
                    ) =>
            {
                originals.tangents[logical].unwrap_or_else(|| self.space.tangent(value))
            }
            _ => self.space.tangent(value),
        }
    }

    fn uv(&self, vertex: u32, unchanged: Option<(usize, usize)>, set: usize) -> Vec2 {
        let value = self.level.uv_for_channel(vertex as usize, set);
        match (unchanged, self.originals) {
            (Some((corner, logical)), Some(originals))
                if Self::same(
                    &value.to_array(),
                    &self.source.uv_for_channel(corner, set).to_array(),
                ) =>
            {
                originals
                    .uvs
                    .get(set)
                    .and_then(|uvs| uvs[logical])
                    .unwrap_or_else(|| flip_v(value))
            }
            _ => flip_v(value),
        }
    }
}

/// Where a mesh's vertices are written: back in its node's own space, or in
/// world space (a flat export) turned into glTF's axes.
enum Space {
    Local {
        inverse: Mat4,
        linear: Mat3,
        mirrored: bool,
    },
    World(Quat),
}

impl Space {
    fn mirrored(&self) -> bool {
        matches!(self, Space::Local { mirrored: true, .. })
    }

    fn point(&self, world: Vec3) -> Vec3 {
        match self {
            Space::Local { inverse, .. } => inverse.transform_point3(world),
            Space::World(fix) => *fix * world,
        }
    }

    /// A world normal `M^-T n` taken back: `M^T` it, and renormalize.
    fn normal(&self, world: Vec3) -> Vec3 {
        match self {
            Space::Local { linear, .. } => (linear.transpose() * world).normalize_or_zero(),
            Space::World(fix) => *fix * world,
        }
    }

    /// A world tangent `M t` taken back, its handedness turned over again by a
    /// mirroring transform (as import turned it).
    fn tangent(&self, world: Vec4) -> Vec4 {
        match self {
            Space::Local {
                inverse, mirrored, ..
            } => {
                let direction = Mat3::from_mat4(*inverse) * world.truncate();
                let sign = if *mirrored { -world.w } else { world.w };
                direction.normalize_or_zero().extend(sign)
            }
            Space::World(fix) => (*fix * world.truncate()).extend(world.w),
        }
    }
}

fn flip_v(uv: Vec2) -> Vec2 {
    Vec2::new(uv.x, 1.0 - uv.y)
}

/// Every node's triangles in `level`, by material — see [`NodeGroup`].
fn groups(level: &ModelData) -> Vec<NodeGroup> {
    let triangle_count = level.indices.len() / 3;
    let nodes = (level.triangles.node.len() == triangle_count).then_some(&level.triangles.node);
    let materials =
        (level.triangles.material.len() == triangle_count).then_some(&level.triangles.material);
    let mut groups: Vec<NodeGroup> = Vec::new();
    let mut group_of: HashMap<u32, usize> = HashMap::new();
    for triangle in 0..triangle_count {
        let node = nodes.map_or(0, |nodes| nodes[triangle]);
        let material = materials.map_or(u32::MAX, |materials| materials[triangle]);
        let at = *group_of.entry(node).or_insert_with(|| {
            groups.push(NodeGroup {
                node: node as usize,
                materials: Vec::new(),
            });
            groups.len() - 1
        });
        let group = &mut groups[at];
        match group
            .materials
            .iter_mut()
            .find(|(known, _)| *known == material)
        {
            Some((_, triangles)) => triangles.push(triangle),
            None => group.materials.push((material, vec![triangle])),
        }
    }
    groups
}

/// A float in `format`'s component type: its bits, or a normalized integer
/// (rounded to nearest — exact for any value that came in as one).
fn push_float(bytes: &mut Vec<u8>, value: f32, format: AttributeFormat) {
    let scaled = |max: f32| (value.clamp(0.0, 1.0) * max).round();
    match format.component {
        ComponentType::F32 => bytes.extend_from_slice(&value.to_le_bytes()),
        ComponentType::U16 => bytes.extend_from_slice(&(scaled(65535.0) as u16).to_le_bytes()),
        ComponentType::U8 => bytes.push(scaled(255.0) as u8),
        // glTF allows no other colour type; were one read, it is written back
        // in the nearest it does.
        ComponentType::I16 => bytes
            .extend_from_slice(&((value.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()),
        ComponentType::I8 => bytes.push((value.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8),
        ComponentType::U32 => bytes.extend_from_slice(
            &((f64::from(value.clamp(0.0, 1.0)) * 4_294_967_295.0).round() as u32).to_le_bytes(),
        ),
    }
}

/// A raw word in its component type: the low bytes it came from.
fn push_word(bytes: &mut Vec<u8>, word: u32, component: ComponentType) {
    match component {
        ComponentType::I8 | ComponentType::U8 => bytes.push(word as u8),
        ComponentType::I16 | ComponentType::U16 => {
            bytes.extend_from_slice(&(word as u16).to_le_bytes())
        }
        ComponentType::U32 | ComponentType::F32 => bytes.extend_from_slice(&word.to_le_bytes()),
    }
}

/// A material from its PBR factors, for a source with no glTF of its own.
fn pbr_material(
    material: &review_model::MaterialImportDefaults,
    used: &mut BTreeSet<String>,
) -> String {
    let color = material.base_color;
    let emissive = material.emissive;
    let peak = emissive.max_element();
    let mut object = serde_json::Map::new();
    if !material.name.is_empty() {
        object.insert("name".to_owned(), material.name.clone().into());
    }
    let mut pbr = serde_json::Map::new();
    pbr.insert(
        "baseColorFactor".to_owned(),
        serde_json::json!([color.x, color.y, color.z, 1.0]),
    );
    pbr.insert(
        "metallicFactor".to_owned(),
        serde_json::json!(material.metallic),
    );
    pbr.insert(
        "roughnessFactor".to_owned(),
        serde_json::json!(1.0 - material.smoothness),
    );
    object.insert("pbrMetallicRoughness".to_owned(), pbr.into());
    if peak > 0.0 {
        // glTF caps an emissive factor at 1; a brighter one keeps its colour
        // there and carries its strength in the extension.
        let (factor, strength) = if peak > 1.0 {
            (emissive / peak, Some(peak))
        } else {
            (emissive, None)
        };
        object.insert(
            "emissiveFactor".to_owned(),
            serde_json::json!([factor.x, factor.y, factor.z]),
        );
        if let Some(strength) = strength {
            const EXTENSION: &str = "KHR_materials_emissive_strength";
            used.insert(EXTENSION.to_owned());
            object.insert(
                "extensions".to_owned(),
                serde_json::json!({ EXTENSION: { "emissiveStrength": strength } }),
            );
        }
    }
    serde_json::to_string(&object).unwrap_or_else(|_| "{}".to_owned())
}

/// The names of every extension `text` (a JSON object) uses, at any depth.
fn collect_extensions(text: &str, used: &mut BTreeSet<String>) {
    fn walk(value: &serde_json::Value, used: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(serde_json::Value::Object(extensions)) = object.get("extensions") {
                    used.extend(extensions.keys().cloned());
                }
                for child in object.values() {
                    walk(child, used);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, used);
                }
            }
            _ => {}
        }
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        walk(&value, used);
    }
}

/// The GLB container: header, the JSON chunk padded with spaces, the binary
/// chunk (when there is one) padded with zeros.
fn container(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let json_padded = json.len().next_multiple_of(4);
    let bin_padded = bin.len().next_multiple_of(4);
    let total = 12 + 8 + json_padded + if bin.is_empty() { 0 } else { 8 + bin_padded };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_padded as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json);
    out.resize(out.len() + json_padded - json.len(), b' ');
    if !bin.is_empty() {
        out.extend_from_slice(&(bin_padded as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(bin);
        out.resize(out.len() + bin_padded - bin.len(), 0);
    }
    out
}
