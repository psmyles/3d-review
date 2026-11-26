//! glTF 2.0 import (`.gltf` and `.glb`): the second way into `ModelData`,
//! beside ufbx, ending in the same funnel checks (invariant 7).
//!
//! What is read: the default scene's node hierarchy with each node's own
//! transform; every triangle primitive's positions, normals, tangents, UV sets
//! and `COLOR_0`; the custom attributes a pipeline adds (names starting with
//! `_`), exact and in their own types; materials, by name and PBR factors. The
//! rest of what the file says about them — `extras` on the asset, the scene, the
//! nodes, the meshes, every material with its textures — is kept verbatim in
//! [`GltfSource`] for a glb export to write back.
//!
//! Like an FBX import, the model is corner-split (one render vertex per
//! triangle corner, `corner_to_logical` naming the glTF vertex it came from),
//! baked into world space, and V-up: glTF stores UVs with V pointing down the
//! image, so V is flipped once here and flipped back by the glb writer.
//!
//! glTF's own conventions: metres, +Y up, +Z toward the viewer, right-handed.
//! Not read yet, and said so as a warning: skins, animations, morph targets,
//! cameras' and lights' own data, and colour sets past `COLOR_0`.

mod accessor;
mod buffers;

use std::path::Path;

use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
use gltf::Semantic;
use gltf::mesh::Mode;
use review_model::{
    AttributeFormat, Axis, CoordinateAxes, CustomAttribute, GltfImage, GltfNode, GltfSource,
    LocalTransform, MaterialImportDefaults, MaterialMap, ModelData, ModelStats, NodeKind,
    SceneNode, TopologyFace, TriangleData, Vertex,
};

use crate::{CancelToken, ImportError, ImportProgress, ImportStage, ProgressSink};

/// The axes glTF fixes for every file.
pub const AXES: CoordinateAxes = CoordinateAxes {
    right: Axis::PositiveX,
    up: Axis::PositiveY,
    front: Axis::PositiveZ,
};

/// Whether `bytes` look like glTF: a binary container, or JSON.
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"glTF")
        || bytes
            .iter()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|&byte| byte == b'{')
}

/// Read the file at `path`. The model plus a sentence for each thing in the
/// file it could not carry.
pub(crate) fn load_path(
    path: &Path,
    progress: ProgressSink<'_>,
    cancel: Option<&CancelToken>,
) -> Result<(ModelData, Vec<String>), ImportError> {
    let _z = crate::prof::zone!("Load glTF");
    progress(ImportProgress::stage(ImportStage::Reading));
    let bytes = std::fs::read(path)
        .map_err(|error| ImportError::LoadFailed(format!("{}: {error}", path.display())))?;
    load_bytes(&bytes, path, path.parent(), progress, cancel)
}

/// Read a file already in memory. `name` names the model; `directory` is
/// where a `.gltf`'s separate buffers are, `None` when there is nowhere to look.
pub(crate) fn load_bytes(
    bytes: &[u8],
    name: &Path,
    directory: Option<&Path>,
    progress: ProgressSink<'_>,
    cancel: Option<&CancelToken>,
) -> Result<(ModelData, Vec<String>), ImportError> {
    let failed = |reason: String| ImportError::LoadFailed(reason);
    let gltf::Gltf { document, blob } =
        gltf::Gltf::from_slice(bytes).map_err(|error| failed(format!("glTF: {error}")))?;
    let buffers = buffers::load(&document, blob, directory).map_err(failed)?;
    if cancel.is_some_and(CancelToken::is_cancelled) {
        return Err(ImportError::Cancelled);
    }
    progress(ImportProgress::stage(ImportStage::Building));
    let model_name = name
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Imported Model")
        .to_owned();
    let (mut model, warnings) = Reader::new(&document, &buffers, cancel).read(model_name)?;
    finish(&mut model)?;
    Ok((model, warnings))
}

/// The funnel's own checks and the measurements a drawable model needs, as the
/// FBX path makes them (`ffi::marshal_model::model_from_bridge_scene`).
fn finish(model: &mut ModelData) -> Result<(), ImportError> {
    if let Some(&index) = model
        .indices
        .iter()
        .find(|&&index| index as usize >= model.vertices.len())
    {
        return Err(ImportError::LoadFailed(format!(
            "glTF import produced index {index} for {} vertices",
            model.vertices.len()
        )));
    }
    model
        .triangles
        .validate(
            model.stats.triangle_count,
            model.faces.len(),
            model.materials.len(),
            model.nodes.len(),
        )
        .map_err(ImportError::LoadFailed)?;
    model.validate_deform().map_err(ImportError::LoadFailed)?;
    model.recompute_bounds();
    // A primitive without `TANGENT` leaves a zero tangent per corner, which is
    // the funnel's signal to derive one from the UVs (only for those corners).
    if model.has_degenerate_tangents() {
        model.generate_missing_tangents();
    }
    model.stats.draw_count = model.material_draw_count();
    Ok(())
}

/// One import's working state.
struct Reader<'a> {
    document: &'a gltf::Document,
    buffers: &'a [Vec<u8>],
    cancel: Option<&'a CancelToken>,
    warnings: Vec<String>,

    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    faces: Vec<TopologyFace>,
    triangles: TriangleData,
    /// Per UV set past the first, per render vertex (set 0 lives on the vertex;
    /// it is mirrored in here once a second set turns up).
    uv_sets: Vec<Vec<Vec2>>,
    corner_to_logical: Vec<u32>,
    /// The most UV sets any primitive had.
    uv_set_count: usize,
    custom: Vec<CustomAttribute>,
    source: GltfSource,
}

/// One primitive's values, read and checked, in the node's own space.
struct Primitive {
    count: usize,
    positions: Vec<f32>,
    normals: Option<Vec<f32>>,
    tangents: Option<Vec<f32>>,
    uvs: Vec<Vec<f32>>,
    colors: Option<(Vec<f32>, AttributeFormat)>,
    custom: Vec<(String, AttributeFormat, Vec<u32>)>,
    triangles: Vec<[u32; 3]>,
    material: u32,
}

impl<'a> Reader<'a> {
    fn new(
        document: &'a gltf::Document,
        buffers: &'a [Vec<u8>],
        cancel: Option<&'a CancelToken>,
    ) -> Self {
        Self {
            document,
            buffers,
            cancel,
            warnings: Vec::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
            faces: Vec::new(),
            triangles: TriangleData::default(),
            uv_sets: Vec::new(),
            corner_to_logical: Vec::new(),
            uv_set_count: 0,
            custom: Vec::new(),
            source: GltfSource::default(),
        }
    }

    fn warn(&mut self, warning: String) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn read(mut self, name: String) -> Result<(ModelData, Vec<String>), ImportError> {
        let json = self.document.as_json();
        self.source.copyright = json.asset.copyright.clone();
        self.source.asset_extras = raw(&json.asset.extras);
        self.source.extensions_used = json.extensions_used.clone();
        self.source.extensions_required = json.extensions_required.clone();
        self.source.materials = json
            .materials
            .iter()
            .map(|material| serde_json::to_string(material).unwrap_or_else(|_| "{}".to_owned()))
            .collect();
        self.source.textures = json
            .textures
            .iter()
            .map(|texture| serde_json::to_string(texture).unwrap_or_else(|_| "{}".to_owned()))
            .collect();
        self.source.samplers = json
            .samplers
            .iter()
            .map(|sampler| serde_json::to_string(sampler).unwrap_or_else(|_| "{}".to_owned()))
            .collect();
        self.source.images = self.images();
        if !json.animations.is_empty() {
            self.warn(format!(
                "{} animation(s) were not read: glTF animation is not supported yet",
                json.animations.len()
            ));
        }

        let materials: Vec<MaterialImportDefaults> = self
            .document
            .materials()
            .map(|material| material_defaults(&material))
            .collect();
        let joints: std::collections::HashSet<usize> = self
            .document
            .skins()
            .flat_map(|skin| skin.joints().map(|joint| joint.index()).collect::<Vec<_>>())
            .collect();
        if self.document.skins().next().is_some() {
            self.warn("skins were not read: glTF skinning is not supported yet".to_owned());
        }

        // The scene's nodes, parents first, children in the order they list them.
        let scene = self
            .document
            .default_scene()
            .or_else(|| self.document.scenes().next());
        let roots: Vec<gltf::Node<'_>> = match &scene {
            Some(scene) => {
                self.source.scene_name = scene.name().map(str::to_owned);
                self.source.scene_extras = raw(scene.extras());
                scene.nodes().collect()
            }
            // No scene: every node no other node lists as a child.
            None => {
                let mut child = vec![false; self.document.nodes().len()];
                for node in self.document.nodes() {
                    for kid in node.children() {
                        child[kid.index()] = true;
                    }
                }
                self.document
                    .nodes()
                    .filter(|node| !child[node.index()])
                    .collect()
            }
        };
        let mut placed = vec![false; self.document.nodes().len()];
        let mut stack: Vec<(gltf::Node<'_>, Option<usize>, Mat4)> = roots
            .into_iter()
            .rev()
            .map(|node| (node, None, Mat4::IDENTITY))
            .collect();
        let mut nodes: Vec<SceneNode> = Vec::new();
        let mut mesh_parts = 0usize;
        while let Some((node, parent, parent_world)) = stack.pop() {
            if std::mem::replace(&mut placed[node.index()], true) {
                self.warn(format!(
                    "node {} is listed under more than one parent; it was read once",
                    node.index()
                ));
                continue;
            }
            if self.cancel.is_some_and(CancelToken::is_cancelled) {
                return Err(ImportError::Cancelled);
            }
            let (rest_local, matrix) = local_transform(&node);
            let local = match matrix {
                Some(matrix) => Mat4::from_cols_array(&matrix),
                None => rest_local.to_mat4(),
            };
            let world = parent_world * local;
            let index = nodes.len();
            let kind = if node.mesh().is_some() {
                NodeKind::Mesh
            } else if node.camera().is_some() {
                NodeKind::Camera
            } else if joints.contains(&node.index()) {
                NodeKind::Bone
            } else {
                NodeKind::Empty
            };
            let mut scene_node = SceneNode {
                name: node.name().unwrap_or_default().to_owned(),
                parent,
                transform: world,
                rest_local,
                kind,
                ..SceneNode::default()
            };
            let mut gltf_node = GltfNode {
                extras: raw(node.extras()),
                matrix,
                ..GltfNode::default()
            };
            if let Some(mesh) = node.mesh() {
                gltf_node.mesh_name = mesh.name().map(str::to_owned);
                gltf_node.mesh_extras = raw(mesh.extras());
                scene_node.mesh_part = Some(mesh_parts);
                mesh_parts += 1;
                scene_node.source_vertex_count =
                    self.read_mesh(&mesh, index as u32, world, materials.len())?;
            }
            nodes.push(scene_node);
            self.source.nodes.push(gltf_node);
            for child in node.children().collect::<Vec<_>>().into_iter().rev() {
                stack.push((child, Some(index), world));
            }
        }
        let unplaced = placed.iter().filter(|&&placed| !placed).count();
        if unplaced > 0 && scene.is_some() {
            self.warn(format!(
                "{unplaced} node(s) outside the file's default scene were not read"
            ));
        }

        let triangle_count = self.indices.len() / 3;
        let uv_set_count = self.uv_set_count;
        let bone_count = nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Bone)
            .count();
        let logical = self.source.vertices.positions.len();
        let stats = ModelStats {
            polygon_count: triangle_count,
            triangle_count,
            vertex_count: logical,
            gpu_vertex_count: 0,
            uv_set_count,
            material_count: materials.len(),
            draw_count: 0,
            bone_count,
            clip_count: 0,
            source_unit_meters: 1.0,
            source_axes: Some(AXES),
        };
        // Set 0 lives on the vertex; the channel table is kept only when a
        // second set exists, with set 0 mirrored in (`ModelData::uv_channels`).
        let uv_channels = if self.uv_sets.len() > 1 {
            std::mem::take(&mut self.uv_sets)
        } else {
            Vec::new()
        };
        let uv_set_names = (0..uv_set_count)
            .map(|set| format!("TEXCOORD_{set}"))
            .collect();
        let model = ModelData {
            name,
            vertices: self.vertices,
            indices: self.indices,
            faces: self.faces,
            triangles: self.triangles,
            nodes,
            uv_channels,
            uv_set_names,
            bounds: None,
            stats,
            materials,
            corner_to_logical: self.corner_to_logical,
            skin: None,
            morph: None,
            animations: Vec::new(),
            frame_rate: 0.0,
            custom_attributes: self.custom,
            gltf: Some(Box::new(self.source)),
        };
        Ok((model, self.warnings))
    }

    /// Every image as its JSON without `bufferView`, plus the bytes that view held.
    fn images(&mut self) -> Vec<GltfImage> {
        let json = self.document.as_json();
        let mut images = Vec::with_capacity(json.images.len());
        for (index, image) in json.images.iter().enumerate() {
            let mut image = image.clone();
            let bytes = image.buffer_view.take().and_then(|view| {
                let view = self.document.views().nth(view.value())?;
                let buffer = self.buffers.get(view.buffer().index())?;
                buffer
                    .get(view.offset()..view.offset() + view.length())
                    .map(<[u8]>::to_vec)
            });
            if bytes.is_none() && image.uri.is_none() {
                self.warn(format!("image {index}'s bytes could not be read"));
            }
            images.push(GltfImage {
                json: serde_json::to_string(&image).unwrap_or_else(|_| "{}".to_owned()),
                bytes,
            });
        }
        images
    }

    /// Bake every triangle primitive of `mesh` into the model under `node`, at
    /// `world`. Returns how many glTF vertices it had.
    fn read_mesh(
        &mut self,
        mesh: &gltf::Mesh<'_>,
        node: u32,
        world: Mat4,
        material_count: usize,
    ) -> Result<usize, ImportError> {
        let linear = Mat3::from_mat4(world);
        let mirrored = linear.determinant() < 0.0;
        let normal_matrix = {
            let inverse = linear.inverse().transpose();
            if inverse.is_finite() { inverse } else { linear }
        };
        let mut vertex_total = 0;
        for primitive in mesh.primitives() {
            if self.cancel.is_some_and(CancelToken::is_cancelled) {
                return Err(ImportError::Cancelled);
            }
            let primitive = match self.read_primitive(&primitive, mesh, material_count) {
                Ok(Some(primitive)) => primitive,
                Ok(None) => continue,
                Err(reason) => {
                    self.warn(format!(
                        "a primitive of mesh '{}' was not read: {reason}",
                        mesh.name().unwrap_or_default()
                    ));
                    continue;
                }
            };
            vertex_total += primitive.count;
            self.bake(&primitive, node, world, normal_matrix, linear, mirrored);
        }
        Ok(vertex_total)
    }

    fn read_primitive(
        &mut self,
        primitive: &gltf::Primitive<'_>,
        mesh: &gltf::Mesh<'_>,
        material_count: usize,
    ) -> Result<Option<Primitive>, String> {
        let corners_of = match primitive.mode() {
            Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan => primitive.mode(),
            other => {
                self.warn(format!(
                    "mesh '{}' has {other:?} primitives, which are not triangles and were not read",
                    mesh.name().unwrap_or_default()
                ));
                return Ok(None);
            }
        };
        let Some(position) = primitive.get(&Semantic::Positions) else {
            return Ok(None);
        };
        let count = position.count();
        let buffers = self.buffers;
        let floats =
            |accessor: gltf::Accessor<'_>, width: usize, what: &str| -> Result<Vec<f32>, String> {
                let values = accessor::floats(&accessor, buffers)?;
                if values.len() != count * width {
                    return Err(format!(
                        "its {what} has {} values for {count} vertices",
                        values.len() / width.max(1)
                    ));
                }
                Ok(values)
            };
        let positions = floats(position, 3, "POSITION")?;
        let normals = primitive
            .get(&Semantic::Normals)
            .map(|accessor| floats(accessor, 3, "NORMAL"))
            .transpose()?;
        let tangents = primitive
            .get(&Semantic::Tangents)
            .map(|accessor| floats(accessor, 4, "TANGENT"))
            .transpose()?;
        let mut uvs = Vec::new();
        while let Some(accessor) = primitive.get(&Semantic::TexCoords(uvs.len() as u32)) {
            uvs.push(floats(accessor, 2, "TEXCOORD")?);
        }
        let colors = match primitive.get(&Semantic::Colors(0)) {
            Some(accessor) => {
                let format = accessor::format(&accessor)?;
                let width = usize::from(format.components);
                if width < 3 {
                    return Err(format!("its COLOR_0 has {width} components"));
                }
                Some((floats(accessor, width, "COLOR_0")?, format))
            }
            None => None,
        };
        let mut custom = Vec::new();
        for (semantic, accessor) in primitive.attributes() {
            match semantic {
                Semantic::Extras(name) => {
                    let format = accessor::format(&accessor)?;
                    let words = accessor::words(&accessor, buffers)?;
                    if words.len() != count * usize::from(format.components) {
                        return Err(format!("its _{name} does not cover its vertices"));
                    }
                    custom.push((format!("_{name}"), format, words));
                }
                Semantic::Colors(set) if set > 0 => {
                    self.warn("colour sets past COLOR_0 were not read".to_owned())
                }
                Semantic::Joints(_) | Semantic::Weights(_) => {}
                _ => {}
            }
        }
        // A stable order, whatever order the file listed them in.
        custom.sort_by(|a, b| a.0.cmp(&b.0));
        if primitive.morph_targets().next().is_some() {
            self.warn("morph targets were not read: glTF morphing is not supported yet".to_owned());
        }

        let indices: Vec<u32> = match primitive.indices() {
            Some(accessor) => accessor::words(&accessor, buffers)?,
            None => (0..count as u32).collect(),
        };
        let mut triangles = Vec::with_capacity(indices.len() / 3);
        let mut push = |a: u32, b: u32, c: u32| {
            if [a, b, c].iter().all(|&index| (index as usize) < count) {
                triangles.push([a, b, c]);
            }
        };
        match corners_of {
            Mode::Triangles => {
                for triangle in indices.as_chunks::<3>().0 {
                    push(triangle[0], triangle[1], triangle[2]);
                }
            }
            Mode::TriangleStrip => {
                for at in 0..indices.len().saturating_sub(2) {
                    let [a, b, c] = [indices[at], indices[at + 1], indices[at + 2]];
                    if at % 2 == 0 {
                        push(a, b, c)
                    } else {
                        push(b, a, c)
                    }
                }
            }
            _ => {
                for at in 1..indices.len().saturating_sub(1) {
                    push(indices[0], indices[at], indices[at + 1]);
                }
            }
        }
        let material = match primitive.material().index() {
            Some(index) if index < material_count => index as u32,
            _ => u32::MAX,
        };
        Ok(Some(Primitive {
            count,
            positions,
            normals,
            tangents,
            uvs,
            colors,
            custom,
            triangles,
            material,
        }))
    }

    /// Append `primitive`'s triangles, one render vertex per corner, in world
    /// space; and its vertices, as stored, to the side table.
    fn bake(
        &mut self,
        primitive: &Primitive,
        node: u32,
        world: Mat4,
        normal_matrix: Mat3,
        linear: Mat3,
        mirrored: bool,
    ) {
        let logical_base = self.source.vertices.positions.len() as u32;
        let vec3 = |values: &[f32], at: usize| {
            Vec3::new(values[at * 3], values[at * 3 + 1], values[at * 3 + 2])
        };
        let vec4 = |values: &[f32], at: usize| {
            Vec4::new(
                values[at * 4],
                values[at * 4 + 1],
                values[at * 4 + 2],
                values[at * 4 + 3],
            )
        };
        let vec2 = |values: &[f32], at: usize| Vec2::new(values[at * 2], values[at * 2 + 1]);

        // The side table: every vertex as stored.
        let originals = &mut self.source.vertices;
        while originals.uvs.len() < primitive.uvs.len() {
            originals.uvs.push(vec![None; originals.positions.len()]);
        }
        for at in 0..primitive.count {
            originals.positions.push(vec3(&primitive.positions, at));
            originals
                .normals
                .push(primitive.normals.as_deref().map(|values| vec3(values, at)));
            originals
                .tangents
                .push(primitive.tangents.as_deref().map(|values| vec4(values, at)));
            originals.colored.push(primitive.colors.is_some());
            for (set, values) in originals.uvs.iter_mut().enumerate() {
                values.push(primitive.uvs.get(set).map(|uvs| vec2(uvs, at)));
            }
        }
        if primitive.tangents.is_some() {
            self.source.tangents = true;
        }
        if let Some((_, format)) = &primitive.colors {
            self.source.color_format = Some(match self.source.color_format {
                Some(known) => wider_color(known, *format),
                None => *format,
            });
        }

        // UV sets this primitive brings that no earlier one had.
        self.uv_set_count = self.uv_set_count.max(primitive.uvs.len());
        let had_sets = self.uv_sets.len();
        if primitive.uvs.len() > had_sets.max(1) {
            if had_sets == 0 {
                // Set 0 joins the table, mirrored from the vertices so far.
                self.uv_sets
                    .push(self.vertices.iter().map(|vertex| vertex.uv).collect());
            }
            while self.uv_sets.len() < primitive.uvs.len() {
                self.uv_sets.push(vec![Vec2::ZERO; self.vertices.len()]);
            }
        }
        // Custom attributes this primitive brings that no earlier one had, and
        // where each of its own goes.
        let mut slots = Vec::with_capacity(primitive.custom.len());
        for (name, format, _) in &primitive.custom {
            let slot = match self.custom.iter().position(|known| &known.name == name) {
                Some(slot) => slot,
                None => {
                    self.custom.push(CustomAttribute {
                        name: name.clone(),
                        format: *format,
                        values: vec![0; self.vertices.len() * usize::from(format.components)],
                    });
                    self.custom.len() - 1
                }
            };
            if self.custom[slot].format != *format {
                let known = self.custom[slot].format.label();
                self.warnings.push(format!(
                    "{name} is {} in one primitive and {known} in another; the other's values were left out",
                    format.label()
                ));
                slots.push(None);
            } else {
                slots.push(Some(slot));
            }
        }

        for corners in &primitive.triangles {
            // A mirroring transform turns the winding over; reversing it keeps
            // the front faces in front (the glTF spec's rule for a negative
            // determinant).
            let corners = if mirrored {
                [corners[0], corners[2], corners[1]]
            } else {
                *corners
            };
            let positions =
                corners.map(|at| world.transform_point3(vec3(&primitive.positions, at as usize)));
            let flat = (positions[1] - positions[0])
                .cross(positions[2] - positions[0])
                .normalize_or_zero();
            let face = self.faces.len() as u32;
            self.faces.push(TopologyFace {
                first_index: self.indices.len() as u32,
                index_count: 3,
            });
            self.triangles.to_face.push(face);
            self.triangles.material.push(primitive.material);
            self.triangles.node.push(node);
            for (&at, position) in corners.iter().zip(positions) {
                let at = at as usize;
                let normal = match &primitive.normals {
                    Some(values) => (normal_matrix * vec3(values, at)).normalize_or_zero(),
                    None => flat,
                };
                let tangent = match &primitive.tangents {
                    Some(values) => {
                        let tangent = vec4(values, at);
                        let direction = (linear * tangent.truncate()).normalize_or_zero();
                        let sign = if mirrored { -tangent.w } else { tangent.w };
                        direction.extend(sign)
                    }
                    // Zero is the funnel's "derive one" signal (`finish`).
                    None => Vec4::ZERO,
                };
                let uv_of = |set: usize| {
                    primitive
                        .uvs
                        .get(set)
                        .map_or(Vec2::ZERO, |uvs| flip_v(vec2(uvs, at)))
                };
                let vertex_color = match &primitive.colors {
                    Some((values, format)) if format.components == 4 => vec4(values, at),
                    Some((values, _)) => {
                        Vec3::new(values[at * 3], values[at * 3 + 1], values[at * 3 + 2])
                            .extend(1.0)
                    }
                    None => Vec4::ONE,
                };
                self.indices.push(self.vertices.len() as u32);
                self.vertices.push(Vertex {
                    position,
                    normal,
                    uv: uv_of(0),
                    tangent,
                    vertex_color,
                });
                self.corner_to_logical.push(logical_base + at as u32);
                for (set, values) in self.uv_sets.iter_mut().enumerate() {
                    values.push(uv_of(set));
                }
                for (slot, attribute) in self.custom.iter_mut().enumerate() {
                    let width = attribute.width();
                    let own = slots
                        .iter()
                        .position(|&taken| taken == Some(slot))
                        .map(|index| &primitive.custom[index].2[at * width..(at + 1) * width]);
                    match own {
                        Some(words) => attribute.values.extend_from_slice(words),
                        None => attribute.values.resize(attribute.values.len() + width, 0),
                    }
                }
            }
        }
    }
}

/// glTF's V runs down the image; `ModelData`'s runs up, as FBX's does.
fn flip_v(uv: Vec2) -> Vec2 {
    Vec2::new(uv.x, 1.0 - uv.y)
}

/// The node's own transform, and its matrix when it gave one rather than
/// translation, rotation and scale.
fn local_transform(node: &gltf::Node<'_>) -> (LocalTransform, Option<[f32; 16]>) {
    match node.transform() {
        gltf::scene::Transform::Decomposed {
            translation,
            rotation,
            scale,
        } => (
            LocalTransform {
                translation: Vec3::from_array(translation),
                // As stored: a writer that keeps it unchanged must not see it
                // renormalized.
                rotation: Quat::from_array(rotation),
                scale: Vec3::from_array(scale),
            },
            None,
        ),
        transform @ gltf::scene::Transform::Matrix { .. } => {
            let (translation, rotation, scale) = transform.clone().decomposed();
            let matrix = transform.matrix();
            let mut flat = [0.0f32; 16];
            for (column, values) in matrix.iter().enumerate() {
                flat[column * 4..column * 4 + 4].copy_from_slice(values);
            }
            (
                LocalTransform {
                    translation: Vec3::from_array(translation),
                    rotation: Quat::from_array(rotation).normalize(),
                    scale: Vec3::from_array(scale),
                },
                Some(flat),
            )
        }
    }
}

/// A material's PBR factors as the viewer's import defaults. Textures are kept
/// in [`GltfSource`] for the writer; the viewer does not bind them yet.
fn material_defaults(material: &gltf::Material<'_>) -> MaterialImportDefaults {
    let pbr = material.pbr_metallic_roughness();
    let [r, g, b, _] = pbr.base_color_factor();
    let strength = material.emissive_strength().unwrap_or(1.0);
    MaterialImportDefaults {
        name: material.name().unwrap_or_default().to_owned(),
        base_color: Vec3::new(r, g, b),
        smoothness: 1.0 - pbr.roughness_factor().clamp(0.0, 1.0),
        metallic: pbr.metallic_factor().clamp(0.0, 1.0),
        emissive: Vec3::from_array(material.emissive_factor()) * strength,
        emission_factor: strength,
        maps: [None; MaterialMap::COUNT],
    }
}

/// The colour format to write back when primitives disagree: the one that
/// loses nothing of either.
fn wider_color(a: AttributeFormat, b: AttributeFormat) -> AttributeFormat {
    let rank = |format: AttributeFormat| match format.component {
        review_model::ComponentType::F32 => 3,
        review_model::ComponentType::U16 | review_model::ComponentType::I16 => 2,
        _ => 1,
    };
    let mut wider = if rank(b) > rank(a) { b } else { a };
    wider.components = a.components.max(b.components);
    wider
}

/// `extras` as the file's own JSON text.
fn raw(extras: &gltf::json::Extras) -> Option<String> {
    extras.as_ref().map(|value| value.get().to_owned())
}

#[cfg(test)]
mod tests;
