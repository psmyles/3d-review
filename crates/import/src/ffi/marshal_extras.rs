//! The capture -> `review_model::SourceExtras`.
//!
//! Everything the viewer never reads but the exporter must give back. Like
//! [`super::marshal_model`] this is safe code over slices borrowed from the
//! capture, and it runs as an import stage of its own, after the mesh is already
//! on screen.

// Safe code over borrowed slices: a raw pointer genuinely never reaches here,
// since [`super::raw`]'s accessors take the capture rather than a pointer out of
// it.
#![deny(unsafe_code)]

use glam::{DMat4, Mat4, Quat, Vec3, Vec4};
use review_model::{ExtrasCounts, ModelData, SourceExtras, extras};

use crate::ImportError;

use super::bridge::ExtrasHandle;
use super::marshal_model::skinning_method_from_code;
use super::raw_extras::*;

// ---- Marshaling the capture into `SourceExtras` ----

impl ExtrasHandle {
    pub(crate) fn marshal(self, model: &ModelData) -> Result<SourceExtras, ImportError> {
        let extras = marshal_extras(&self.raw)?;
        extras
            .validate(&ExtrasCounts::of(model))
            .map_err(|message| ImportError::LoadFailed(format!("source properties: {message}")))?;
        Ok(extras)
    }
}

/// Borrowed views over the capture's arenas and tables, with the range
/// checks every reference goes through.
pub(super) struct ExtrasView<'a> {
    pub(super) strings: &'a [u8],
    pub(super) bytes: &'a [u8],
    pub(super) props: &'a [XProp],
}

impl ExtrasView<'_> {
    fn str(&self, s: XStr) -> String {
        let first = s.offset as usize;
        let end = first.saturating_add(s.length as usize);
        self.strings
            .get(first..end)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default()
    }

    fn bytes(&self, b: XBytes) -> Vec<u8> {
        let end = b.offset.saturating_add(b.length);
        self.bytes
            .get(b.offset..end)
            .map(<[u8]>::to_vec)
            .unwrap_or_default()
    }

    fn props(&self, range: XPropRange) -> Result<Vec<extras::Prop>, ImportError> {
        slice_range(self.props, range.first, range.count, "props")?
            .iter()
            .map(|prop| {
                Ok(extras::Prop {
                    name: self.str(prop.name),
                    kind: extras::PropType::from_code(prop.kind),
                    flags: extras::PropFlags(prop.flags),
                    value_int: prop.value_int,
                    value_real: prop.value_real,
                    value_str: self.str(prop.value_str),
                    value_blob: self.bytes(prop.value_blob),
                })
            })
            .collect()
    }
}

pub(super) fn slice_range<'a, T>(
    items: &'a [T],
    first: u32,
    count: u32,
    what: &str,
) -> Result<&'a [T], ImportError> {
    let first = first as usize;
    let end = first.saturating_add(count as usize);
    items.get(first..end).ok_or_else(|| {
        ImportError::LoadFailed(format!(
            "source properties: {what} range {first}..{end} exceeds {}",
            items.len()
        ))
    })
}

pub(super) fn dmat(m: &[f64; 16]) -> Mat4 {
    DMat4::from_cols_array(m).as_mat4()
}

pub(super) fn v3(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32)
}

/// Marshal the capture the bridge wrote into owned Rust data.
///
/// Each element kind is its own function below. The capture mirrors ufbx's whole
/// element vocabulary, so one body doing all of it ran past seven hundred lines
/// with no seam a reader could hold; between them they share only the arena
/// views and the range checks in [`ExtrasView`].
pub(super) fn marshal_extras(raw: &ReviewImportExtras) -> Result<SourceExtras, ImportError> {
    let view = ExtrasView {
        strings: raw.strings()?,
        bytes: raw.bytes()?,
        props: raw.props()?,
    };
    let (anim_layers, animations) = marshal_animation_extras(&view, raw)?;
    Ok(SourceExtras {
        scene: marshal_scene_extras(&view, raw)?,
        nodes: marshal_node_extras(&view, raw)?,
        materials: marshal_material_extras(&view, raw)?,
        textures: marshal_texture_extras(&view, raw)?,
        videos: marshal_video_extras(&view, raw)?,
        meshes: marshal_mesh_extras(&view, raw)?,
        poses: marshal_pose_extras(&view, raw)?,
        display_layers: marshal_display_layers(&view, raw)?,
        selection_sets: marshal_selection_sets(&view, raw)?,
        anim_layers,
        animations,
    })
}

/// The scene's own settings and metadata.
pub(super) fn marshal_scene_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<extras::SceneExtras, ImportError> {
    let scene = {
        let s = &raw.scene;
        extras::SceneExtras {
            creator: view.str(s.creator),
            filename: view.str(s.filename),
            original_file_path: view.str(s.original_file_path),
            version: s.version,
            ascii: s.ascii != 0,
            original_application: extras::Application {
                vendor: view.str(s.original_vendor),
                name: view.str(s.original_name),
                version: view.str(s.original_version),
            },
            latest_application: extras::Application {
                vendor: view.str(s.latest_vendor),
                name: view.str(s.latest_name),
                version: view.str(s.latest_version),
            },
            scene_props: view.props(s.scene_props)?,
            settings_props: view.props(s.settings_props)?,
            axes: [
                extras::CoordinateAxis::from_code(s.axis_right),
                extras::CoordinateAxis::from_code(s.axis_up),
                extras::CoordinateAxis::from_code(s.axis_front),
            ],
            original_axis_up: extras::CoordinateAxis::from_code(s.original_axis_up),
            unit_meters: s.unit_meters,
            original_unit_meters: s.original_unit_meters,
            frames_per_second: s.frames_per_second,
            ambient_color: v3(s.ambient_color),
            default_camera: view.str(s.default_camera),
            time_mode: extras::TimeMode::from_code(s.time_mode),
            time_protocol: extras::TimeProtocol::from_code(s.time_protocol),
            snap_mode: extras::SnapMode::from_code(s.snap_mode),
        }
    };
    Ok(scene)
}

/// Every authored node, with the light / camera / LOD-group parameters of
/// whatever attribute it carries.
pub(super) fn marshal_node_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::NodeExtras>, ImportError> {
    let lights = raw.lights()?;
    let cameras = raw.cameras()?;
    let lod_groups = raw.lod_groups()?;
    let lod_levels = raw.lod_levels()?;

    let nodes = raw
        .nodes()?
        .iter()
        .map(|node| {
            let kind = extras::AttributeKind::from_code(node.attribute_kind);
            let attribute = if kind == extras::AttributeKind::None {
                None
            } else {
                let typed = |what: &str| -> Result<usize, ImportError> {
                    usize::try_from(node.attribute_index).map_err(|_| {
                        ImportError::LoadFailed(format!(
                            "source properties: a {what} has no parameters"
                        ))
                    })
                };
                let light = if kind == extras::AttributeKind::Light {
                    let light = lights.get(typed("light")?).ok_or_else(|| {
                        ImportError::LoadFailed(
                            "source properties: light index out of range".to_owned(),
                        )
                    })?;
                    Some(extras::LightExtras {
                        color: v3(light.color),
                        intensity: light.intensity,
                        local_direction: v3(light.local_direction),
                        kind: extras::LightType::from_code(light.kind),
                        decay: extras::LightDecay::from_code(light.decay),
                        area_shape: extras::LightAreaShape::from_code(light.area_shape),
                        inner_angle: light.inner_angle,
                        outer_angle: light.outer_angle,
                        cast_light: light.cast_light != 0,
                        cast_shadows: light.cast_shadows != 0,
                    })
                } else {
                    None
                };
                let camera = if kind == extras::AttributeKind::Camera {
                    let camera = cameras.get(typed("camera")?).ok_or_else(|| {
                        ImportError::LoadFailed(
                            "source properties: camera index out of range".to_owned(),
                        )
                    })?;
                    Some(extras::CameraExtras {
                        projection_mode: extras::ProjectionMode::from_code(camera.projection_mode),
                        resolution_is_pixels: camera.resolution_is_pixels != 0,
                        resolution: camera.resolution,
                        field_of_view_deg: camera.field_of_view_deg,
                        orthographic_extent: camera.orthographic_extent,
                        aspect_ratio: camera.aspect_ratio,
                        near_plane: camera.near_plane,
                        far_plane: camera.far_plane,
                        aspect_mode: extras::AspectMode::from_code(camera.aspect_mode),
                        aperture_mode: extras::ApertureMode::from_code(camera.aperture_mode),
                        gate_fit: extras::GateFit::from_code(camera.gate_fit),
                        aperture_format: extras::ApertureFormat::from_code(camera.aperture_format),
                        focal_length_mm: camera.focal_length_mm,
                        film_size_inch: camera.film_size_inch,
                        aperture_size_inch: camera.aperture_size_inch,
                        squeeze_ratio: camera.squeeze_ratio,
                    })
                } else {
                    None
                };
                let lod_group = if kind == extras::AttributeKind::LodGroup {
                    let group = lod_groups.get(typed("LOD group")?).ok_or_else(|| {
                        ImportError::LoadFailed(
                            "source properties: LOD group index out of range".to_owned(),
                        )
                    })?;
                    Some(extras::LodGroupExtras {
                        relative_distances: group.relative_distances != 0,
                        ignore_parent_transform: group.ignore_parent_transform != 0,
                        use_distance_limit: group.use_distance_limit != 0,
                        distance_limit_min: group.distance_limit_min,
                        distance_limit_max: group.distance_limit_max,
                        levels: slice_range(
                            lod_levels,
                            group.level_first,
                            group.level_count,
                            "LOD levels",
                        )?
                        .iter()
                        .map(|level| extras::LodLevel {
                            distance: level.distance,
                            display: extras::LodDisplay::from_code(level.display),
                        })
                        .collect(),
                    })
                } else {
                    None
                };
                Some(extras::AttributeExtras {
                    kind,
                    name: view.str(node.attribute_name),
                    props: view.props(node.attribute_props)?,
                    light,
                    camera,
                    lod_group,
                })
            };
            Ok(extras::NodeExtras {
                props: view.props(node.props)?,
                rotation_order: extras::RotationOrder::from_code(node.rotation_order),
                inherit_mode: extras::InheritMode::from_code(node.inherit_mode),
                original_inherit_mode: extras::InheritMode::from_code(node.original_inherit_mode),
                geometry_to_node: dmat(&node.geometry_to_node),
                synthetic: extras::Synthetic::from_code(node.synthetic),
                visible: node.visible != 0,
                attribute,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(nodes)
}

/// Every material, with the textures bound to its channels.
pub(super) fn marshal_material_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::MaterialExtras>, ImportError> {
    let material_textures = raw.material_textures()?;
    let materials = raw
        .materials()?
        .iter()
        .map(|material| {
            Ok(extras::MaterialExtras {
                shader_type: extras::ShaderType::from_code(material.shader_type),
                shading_model: view.str(material.shading_model),
                props: view.props(material.props)?,
                textures: slice_range(
                    material_textures,
                    material.texture_first,
                    material.texture_count,
                    "material textures",
                )?
                .iter()
                .map(|texture| extras::MaterialTexture {
                    material_prop: view.str(texture.material_prop),
                    shader_prop: view.str(texture.shader_prop),
                    texture: texture.texture,
                })
                .collect(),
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(materials)
}

/// Every texture, with the layers a layered texture composites.
pub(super) fn marshal_texture_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::TextureExtras>, ImportError> {
    let texture_layers = raw.texture_layers()?;
    let textures = raw
        .textures()?
        .iter()
        .map(|texture| {
            Ok(extras::TextureExtras {
                name: view.str(texture.name),
                kind: extras::TextureKind::from_code(texture.kind),
                filename: view.str(texture.filename),
                absolute_filename: view.str(texture.absolute_filename),
                relative_filename: view.str(texture.relative_filename),
                uv_set: view.str(texture.uv_set),
                wrap_u: extras::WrapMode::from_code(texture.wrap_u),
                wrap_v: extras::WrapMode::from_code(texture.wrap_v),
                uv_transform: (texture.has_uv_transform != 0).then(|| {
                    (
                        v3(texture.uv_translation),
                        Quat::from_xyzw(
                            texture.uv_rotation[0] as f32,
                            texture.uv_rotation[1] as f32,
                            texture.uv_rotation[2] as f32,
                            texture.uv_rotation[3] as f32,
                        )
                        .normalize(),
                        v3(texture.uv_scale),
                    )
                }),
                content: view.bytes(texture.content),
                video: u32::try_from(texture.video).ok(),
                layers: slice_range(
                    texture_layers,
                    texture.layer_first,
                    texture.layer_count,
                    "texture layers",
                )?
                .iter()
                .map(|layer| extras::TextureLayer {
                    texture: layer.texture,
                    blend_mode: extras::BlendMode::from_code(layer.blend_mode),
                    alpha: layer.alpha,
                })
                .collect(),
                props: view.props(texture.props)?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(textures)
}

/// Every video: an image file reference, with any embedded content.
pub(super) fn marshal_video_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::VideoExtras>, ImportError> {
    let videos = raw
        .videos()?
        .iter()
        .map(|video| {
            Ok(extras::VideoExtras {
                name: view.str(video.name),
                filename: view.str(video.filename),
                absolute_filename: view.str(video.absolute_filename),
                relative_filename: view.str(video.relative_filename),
                content: view.bytes(video.content),
                props: view.props(video.props)?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(videos)
}

/// Everything a mesh carries that the viewer never reads: extra color sets,
/// the edge and face topology layers, vertex creases, subdivision settings,
/// and the second and later skin deformers with their DQ weights.
pub(super) fn marshal_mesh_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::MeshExtras>, ImportError> {
    let color_sets = raw.color_sets()?;
    let color_values = raw.color_values()?;
    let edges = raw.edges()?;
    let edge_smoothing = raw.edge_smoothing()?;
    let edge_crease = raw.edge_crease()?;
    let edge_visibility = raw.edge_visibility()?;
    let face_smoothing = raw.face_smoothing()?;
    let face_hole = raw.face_hole()?;
    let face_group = raw.face_group()?;
    let vertex_crease = raw.vertex_crease()?;
    let face_groups = raw.face_groups()?;
    let extra_skins = raw.extra_skins()?;
    let extra_clusters = raw.extra_clusters()?;
    let extra_skin_offsets = raw.extra_skin_offsets()?;
    let extra_influences = raw.extra_influences()?;
    let dq_weights = raw.dq_weights()?;
    let uv_sets = raw.uv_sets()?;
    let unused_vertices = raw.unused_vertices()?;

    let meshes = raw
        .meshes()?
        .iter()
        .map(|mesh| {
            let corners = mesh.corner_count as usize;
            // Every face layer is `face_count` long, whichever the file carried.
            slice_range(
                face_smoothing,
                mesh.face_first,
                mesh.face_count,
                "face layers",
            )?;
            let logical = mesh.logical_count as usize;
            let color_sets = slice_range(
                color_sets,
                mesh.color_set_first,
                mesh.color_set_count,
                "color sets",
            )?
            .iter()
            .map(|set| {
                let values = if set.value_first == u32::MAX {
                    Vec::new()
                } else {
                    let count = corners.checked_mul(4).ok_or_else(|| {
                        ImportError::LoadFailed("source properties: color set overflows".to_owned())
                    })?;
                    let first = set.value_first as usize;
                    color_values
                        .get(first..first.saturating_add(count))
                        .ok_or_else(|| {
                            ImportError::LoadFailed(
                                "source properties: color set values out of range".to_owned(),
                            )
                        })?
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32))
                        .collect()
                };
                Ok(extras::ColorSetExtras {
                    name: view.str(set.name),
                    index: set.index,
                    values,
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;
            let edge_range = |items: &[u8]| -> Result<Vec<bool>, ImportError> {
                Ok(
                    slice_range(items, mesh.edge_first, mesh.edge_count, "edge layer")?
                        .iter()
                        .map(|&v| v != 0)
                        .collect(),
                )
            };
            let face_range = |items: &[u8]| -> Result<Vec<bool>, ImportError> {
                Ok(
                    slice_range(items, mesh.face_first, mesh.face_count, "face layer")?
                        .iter()
                        .map(|&v| v != 0)
                        .collect(),
                )
            };
            let pair_first = mesh.edge_first.saturating_mul(2);
            let pair_count = mesh.edge_count.saturating_mul(2);
            let extra_skins = slice_range(
                extra_skins,
                mesh.extra_skin_first,
                mesh.extra_skin_count,
                "skin layers",
            )?
            .iter()
            .map(|skin| {
                let offsets = slice_range(
                    extra_skin_offsets,
                    skin.offset_first,
                    (logical as u32).saturating_add(1),
                    "skin layer rows",
                )?
                .to_vec();
                Ok(extras::SkinLayerExtras {
                    method: skinning_method_from_code(skin.method),
                    max_weights_per_vertex: skin.max_weights_per_vertex,
                    clusters: slice_range(
                        extra_clusters,
                        skin.cluster_first,
                        skin.cluster_count,
                        "skin layer clusters",
                    )?
                    .iter()
                    .map(|cluster| extras::ExtraCluster {
                        bone: cluster.bone,
                        name: view.str(cluster.name),
                        mesh_node_to_bone: dmat(&cluster.mesh_node_to_bone),
                        bind_to_world: dmat(&cluster.bind_to_world),
                    })
                    .collect(),
                    offsets,
                    influences: slice_range(
                        extra_influences,
                        skin.influence_first,
                        skin.influence_count,
                        "skin layer influences",
                    )?
                    .iter()
                    .map(|influence| (influence.cluster, influence.weight))
                    .collect(),
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;
            Ok(extras::MeshExtras {
                node: mesh.node,
                name: view.str(mesh.name),
                props: view.props(mesh.props)?,
                corner_first: mesh.corner_first,
                corner_count: mesh.corner_count,
                logical_first: mesh.logical_first,
                logical_count: mesh.logical_count,
                face_first: mesh.face_first,
                face_count: mesh.face_count,
                tangents_authored: mesh.tangents_authored != 0,
                normals_authored: mesh.normals_authored != 0,
                uv_sets: slice_range(uv_sets, mesh.uv_set_first, mesh.uv_set_count, "uv sets")?
                    .iter()
                    .map(|set| extras::UvSetExtras {
                        name: view.str(set.name),
                        index: set.index,
                        has_values: set.has_values != 0,
                    })
                    .collect(),
                unused_vertices: slice_range(
                    unused_vertices,
                    mesh.unused_vertex_first,
                    mesh.unused_vertex_count,
                    "unused vertices",
                )?
                .iter()
                .map(|vertex| (vertex.logical, Vec3::from_array(vertex.position)))
                .collect(),
                reversed_winding: mesh.reversed_winding != 0,
                color_sets,
                edges: slice_range(edges, pair_first, pair_count, "edges")?
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| [pair[0], pair[1]])
                    .collect(),
                edge_smoothing: if mesh.has_edge_smoothing != 0 {
                    edge_range(edge_smoothing)?
                } else {
                    Vec::new()
                },
                edge_crease: if mesh.has_edge_crease != 0 {
                    slice_range(edge_crease, mesh.edge_first, mesh.edge_count, "edge crease")?
                        .iter()
                        .map(|&v| v as f32)
                        .collect()
                } else {
                    Vec::new()
                },
                edge_visibility: if mesh.has_edge_visibility != 0 {
                    edge_range(edge_visibility)?
                } else {
                    Vec::new()
                },
                face_smoothing: if mesh.has_face_smoothing != 0 {
                    face_range(face_smoothing)?
                } else {
                    Vec::new()
                },
                face_hole: if mesh.has_face_hole != 0 {
                    face_range(face_hole)?
                } else {
                    Vec::new()
                },
                face_group: if mesh.has_face_group != 0 {
                    slice_range(face_group, mesh.face_first, mesh.face_count, "face group")?
                        .to_vec()
                } else {
                    Vec::new()
                },
                face_groups: slice_range(
                    face_groups,
                    mesh.face_group_first,
                    mesh.face_group_count,
                    "face groups",
                )?
                .iter()
                .map(|group| extras::FaceGroup {
                    id: group.id,
                    name: view.str(group.name),
                })
                .collect(),
                vertex_crease: if mesh.has_vertex_crease != 0 {
                    slice_range(
                        vertex_crease,
                        mesh.logical_first,
                        mesh.logical_count,
                        "vertex crease",
                    )?
                    .iter()
                    .map(|&v| v as f32)
                    .collect()
                } else {
                    Vec::new()
                },
                subdivision: extras::SubdivisionExtras {
                    preview_levels: mesh.subdivision_preview_levels,
                    render_levels: mesh.subdivision_render_levels,
                    display_mode: extras::SubdivisionDisplayMode::from_code(
                        mesh.subdivision_display_mode,
                    ),
                    boundary: extras::SubdivisionBoundary::from_code(mesh.subdivision_boundary),
                    uv_boundary: extras::SubdivisionBoundary::from_code(
                        mesh.subdivision_uv_boundary,
                    ),
                },
                extra_skins,
                dq_weights: slice_range(
                    dq_weights,
                    mesh.dq_first,
                    mesh.dq_count,
                    "dual-quaternion weights",
                )?
                .iter()
                .map(|w| (w.logical_vertex, w.weight as f32))
                .collect(),
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(meshes)
}

/// The authored bind poses.
pub(super) fn marshal_pose_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::PoseExtras>, ImportError> {
    let pose_entries = raw.pose_entries()?;
    let poses = raw
        .poses()?
        .iter()
        .map(|pose| {
            Ok(extras::PoseExtras {
                name: view.str(pose.name),
                is_bind_pose: pose.is_bind_pose != 0,
                entries: slice_range(
                    pose_entries,
                    pose.entry_first,
                    pose.entry_count,
                    "pose entries",
                )?
                .iter()
                .map(|entry| extras::PoseEntry {
                    node: entry.node,
                    bone_to_world: dmat(&entry.bone_to_world),
                })
                .collect(),
                props: view.props(pose.props)?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(poses)
}

/// The display layers, and which nodes are on each.
pub(super) fn marshal_display_layers(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::DisplayLayerExtras>, ImportError> {
    let layer_nodes = raw.layer_nodes()?;
    let display_layers = raw
        .display_layers()?
        .iter()
        .map(|layer| {
            Ok(extras::DisplayLayerExtras {
                name: view.str(layer.name),
                visible: layer.visible != 0,
                frozen: layer.frozen != 0,
                ui_color: v3(layer.ui_color),
                nodes: slice_range(
                    layer_nodes,
                    layer.node_first,
                    layer.node_count,
                    "display layer nodes",
                )?
                .to_vec(),
                props: view.props(layer.props)?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(display_layers)
}

/// The selection sets, and what each one selects.
pub(super) fn marshal_selection_sets(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<Vec<extras::SelectionSetExtras>, ImportError> {
    let selection_nodes = raw.selection_nodes()?;
    let selection_indices = raw.selection_indices()?;
    let selection_sets = raw
        .selection_sets()?
        .iter()
        .map(|set| {
            Ok(extras::SelectionSetExtras {
                name: view.str(set.name),
                props: view.props(set.props)?,
                nodes: slice_range(
                    selection_nodes,
                    set.node_first,
                    set.node_count,
                    "selection nodes",
                )?
                .iter()
                .map(|node| {
                    Ok(extras::SelectionNodeExtras {
                        node: u32::try_from(node.node).ok(),
                        include_node: node.include_node != 0,
                        vertices: slice_range(
                            selection_indices,
                            node.vertex_first,
                            node.vertex_count,
                            "selection vertices",
                        )?
                        .to_vec(),
                        edges: slice_range(
                            selection_indices,
                            node.edge_first,
                            node.edge_count,
                            "selection edges",
                        )?
                        .to_vec(),
                        faces: slice_range(
                            selection_indices,
                            node.face_first,
                            node.face_count,
                            "selection faces",
                        )?
                        .to_vec(),
                    })
                })
                .collect::<Result<Vec<_>, ImportError>>()?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok(selection_sets)
}

/// The authored animation curves: the layers that carry them, and the stacks
/// those layers belong to.
///
/// One function rather than two because both halves resolve a curve index
/// through the same `curve_of` lookup over the shared key and curve arenas.
#[allow(clippy::type_complexity)]
pub(super) fn marshal_animation_extras(
    view: &ExtrasView<'_>,
    raw: &ReviewImportExtras,
) -> Result<(Vec<extras::LayerCurves>, Vec<extras::ClipCurves>), ImportError> {
    let anim_keys = raw.anim_keys()?;
    let anim_curves = raw.anim_curves()?;
    let anim_props = raw.anim_props()?;
    let curve_of = |index: i32| -> Result<Option<extras::Curve>, ImportError> {
        let Ok(index) = usize::try_from(index) else {
            return Ok(None);
        };
        let curve = anim_curves.get(index).ok_or_else(|| {
            ImportError::LoadFailed("source properties: curve index out of range".to_owned())
        })?;
        Ok(Some(extras::Curve {
            keys: slice_range(anim_keys, curve.key_first, curve.key_count, "keys")?
                .iter()
                .map(|key| extras::RawKey {
                    time: key.time,
                    value: key.value,
                    interpolation: extras::Interpolation::from_code(key.interpolation),
                    left: (key.left_dx, key.left_dy),
                    right: (key.right_dx, key.right_dy),
                })
                .collect(),
            pre: extras::Extrapolation {
                mode: extras::ExtrapolationMode::from_code(curve.pre_mode),
                repeat_count: curve.pre_repeat,
            },
            post: extras::Extrapolation {
                mode: extras::ExtrapolationMode::from_code(curve.post_mode),
                repeat_count: curve.post_repeat,
            },
        }))
    };
    let anim_layers = raw
        .anim_layers()?
        .iter()
        .map(|layer| {
            Ok(extras::LayerCurves {
                name: view.str(layer.name),
                weight: layer.weight,
                weight_is_animated: layer.weight_is_animated != 0,
                blended: layer.blended != 0,
                additive: layer.additive != 0,
                compose_rotation: layer.compose_rotation != 0,
                compose_scale: layer.compose_scale != 0,
                props: view.props(layer.props)?,
                anim: slice_range(
                    anim_props,
                    layer.anim_prop_first,
                    layer.anim_prop_count,
                    "animated properties",
                )?
                .iter()
                .map(|prop| {
                    let target = match prop.target_kind {
                        1 => extras::ElementRef::Node(prop.target),
                        2 => extras::ElementRef::NodeAttribute(prop.target),
                        3 => extras::ElementRef::Material(prop.target),
                        4 => extras::ElementRef::Texture(prop.target),
                        5 => extras::ElementRef::Video(prop.target),
                        6 => extras::ElementRef::BlendChannel(prop.target),
                        7 => extras::ElementRef::DisplayLayer(prop.target),
                        8 => extras::ElementRef::AnimLayer(prop.target),
                        _ => extras::ElementRef::Unmapped {
                            element_type: prop.element_type,
                            name: view.str(prop.element_name),
                        },
                    };
                    Ok(extras::AnimPropCurves {
                        target,
                        prop_name: view.str(prop.prop_name),
                        default: v3(prop.default_value),
                        curves: [
                            curve_of(prop.curves[0])?,
                            curve_of(prop.curves[1])?,
                            curve_of(prop.curves[2])?,
                        ],
                    })
                })
                .collect::<Result<Vec<_>, ImportError>>()?,
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;

    let stack_layers = raw.stack_layers()?;
    let animations = raw
        .anim_stacks()?
        .iter()
        .map(|stack| {
            Ok(extras::ClipCurves {
                name: view.str(stack.name),
                props: view.props(stack.props)?,
                clip: u32::try_from(stack.clip).ok(),
                time_begin: stack.time_begin,
                time_end: stack.time_end,
                layers: slice_range(
                    stack_layers,
                    stack.layer_first,
                    stack.layer_count,
                    "stack layers",
                )?
                .to_vec(),
            })
        })
        .collect::<Result<Vec<_>, ImportError>>()?;
    Ok((anim_layers, animations))
}
