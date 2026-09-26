//! The crossing into C.
//!
//! [`write_scene`] is one long function on purpose: it is a strict lifetime
//! chain, where each `Vec` of bridge structs borrows the storage of the one
//! above it (keys -> curves -> anim props -> layers -> stacks; the UV and color
//! pointer tables -> meshes). Every local has to outlive the call, so they all
//! have to live in one stack frame. Its tiers are marked by comment.

use std::ffi::{CString, c_char, c_int};
use std::path::Path;

use crate::OptError;
use crate::stack::FbxFormat;

use super::*;

// ---------------------------------------------------------------------------
// The FFI call
// ---------------------------------------------------------------------------

/// Hand one assembled scene to the bridge.
#[cfg(has_ufbxw)]
pub(crate) fn write_scene(
    scene: &SceneData,
    path: &Path,
    format: FbxFormat,
) -> Result<(), OptError> {
    use crate::export_ffi::{
        RvoExportAnimLayer, RvoExportAnimProp, RvoExportAnimStack, RvoExportBlendChannel,
        RvoExportBlendShape, RvoExportCluster, RvoExportColorSet, RvoExportCurve,
        RvoExportDisplayLayer, RvoExportKey, RvoExportMaterial, RvoExportMaterialTexture,
        RvoExportMesh, RvoExportNode, RvoExportPose, RvoExportPoseNode, RvoExportProp,
        RvoExportPropRange, RvoExportScene, RvoExportSelectionNode, RvoExportSelectionSet,
        RvoExportSkin, RvoExportTexture, RvoExportTextureLayer, RvoExportVideo,
    };

    // The bridge takes UTF-8 and widens it for `_wfopen` on Windows. A path that
    // is not valid Unicode is refused outright: `to_string_lossy` would have
    // quietly written the file under a different name.
    check_lengths(scene).map_err(OptError::Export)?;

    let path_utf8 = path.to_str().ok_or_else(|| {
        OptError::Export(format!(
            "the output path is not valid Unicode: {}",
            path.display()
        ))
    })?;
    let c_path = CString::new(path_utf8)
        .map_err(|_| OptError::Export("the output path contains a NUL byte".to_owned()))?;

    let range = |range: PropRange| RvoExportPropRange {
        first: range.first,
        count: range.count,
    };
    let bytes = |bytes: &[u8]| {
        if bytes.is_empty() {
            std::ptr::null()
        } else {
            bytes.as_ptr()
        }
    };

    // The `repr(C)` mirrors borrow every buffer in `scene`, which outlives this
    // function; the bridge in turn only borrows them for the duration of the
    // call, so nothing here escapes.
    let props: Vec<RvoExportProp> = scene
        .props
        .iter()
        .map(|prop| RvoExportProp {
            name: prop.name.as_ptr(),
            kind: prop.kind,
            flags: prop.flags,
            value_int: prop.value_int,
            value_real: prop.value_real,
            value_str: prop.value_str.as_ptr(),
            blob: bytes(&prop.blob),
            blob_length: prop.blob.len(),
        })
        .collect();
    let nodes: Vec<RvoExportNode> = scene
        .nodes
        .iter()
        .map(|node| RvoExportNode {
            name: node.name.as_ptr(),
            parent: node.parent,
            translation: node.translation,
            rotation: node.rotation,
            scaling: node.scaling,
            authored_transform: i32::from(node.authored_transform),
            props: range(node.props),
            attribute_kind: node.attribute_kind,
            attribute_name: node.attribute_name.as_ptr(),
            attribute_props: range(node.attribute_props),
        })
        .collect();
    let material_textures: Vec<Vec<RvoExportMaterialTexture>> = scene
        .materials
        .iter()
        .map(|material| {
            material
                .textures
                .iter()
                .map(|(prop, texture)| RvoExportMaterialTexture {
                    prop: prop.as_ptr(),
                    texture: *texture,
                })
                .collect()
        })
        .collect();
    let materials: Vec<RvoExportMaterial> = scene
        .materials
        .iter()
        .zip(&material_textures)
        .map(|(material, textures)| RvoExportMaterial {
            name: material.name.as_ptr(),
            shader: material.shader,
            shading_model: material.shading_model.as_ptr(),
            props: range(material.props),
            base_color: material.base_color,
            emissive: material.emissive,
            shininess_exponent: material.shininess_exponent,
            reflection_factor: material.reflection_factor,
            textures: if textures.is_empty() {
                std::ptr::null()
            } else {
                textures.as_ptr()
            },
            texture_count: textures.len(),
        })
        .collect();
    let texture_layers: Vec<Vec<RvoExportTextureLayer>> = scene
        .textures
        .iter()
        .map(|texture| {
            texture
                .layers
                .iter()
                .map(|&(texture, blend_mode, alpha)| RvoExportTextureLayer {
                    texture,
                    blend_mode,
                    alpha,
                })
                .collect()
        })
        .collect();
    let textures: Vec<RvoExportTexture> = scene
        .textures
        .iter()
        .zip(&texture_layers)
        .map(|(texture, layers)| RvoExportTexture {
            name: texture.name.as_ptr(),
            layered: i32::from(texture.layered),
            filename: texture.filename.as_ptr(),
            relative_filename: texture.relative_filename.as_ptr(),
            content: bytes(&texture.content),
            content_length: texture.content.len(),
            video: texture.video,
            props: range(texture.props),
            layers: if layers.is_empty() {
                std::ptr::null()
            } else {
                layers.as_ptr()
            },
            layer_count: layers.len(),
        })
        .collect();
    let videos: Vec<RvoExportVideo> = scene
        .videos
        .iter()
        .map(|video| RvoExportVideo {
            name: video.name.as_ptr(),
            filename: video.filename.as_ptr(),
            relative_filename: video.relative_filename.as_ptr(),
            content: bytes(&video.content),
            content_length: video.content.len(),
            props: range(video.props),
        })
        .collect();

    // Pointer tables for the per-mesh UV and color sets, kept alive alongside
    // the mesh mirrors below.
    let uv_pointers: Vec<(Vec<*const f64>, Vec<*const c_char>, Vec<RvoExportColorSet>)> = scene
        .meshes
        .iter()
        .map(|mesh| {
            (
                mesh.uv_sets.iter().map(|set| set.as_ptr()).collect(),
                mesh.uv_set_names.iter().map(|name| name.as_ptr()).collect(),
                mesh.color_sets
                    .iter()
                    .map(|(name, values)| RvoExportColorSet {
                        name: name.as_ptr(),
                        values: values.as_ptr(),
                    })
                    .collect(),
            )
        })
        .collect();
    let opt_u8 = |values: &[u8]| {
        if values.is_empty() {
            std::ptr::null()
        } else {
            values.as_ptr()
        }
    };
    let opt_i32 = |values: &[i32]| {
        if values.is_empty() {
            std::ptr::null()
        } else {
            values.as_ptr()
        }
    };

    // The deform mirrors, owned beside the meshes so every pointer stays live.
    let cluster_mirrors: Vec<Vec<Vec<RvoExportCluster>>> = scene
        .meshes
        .iter()
        .map(|mesh| {
            mesh.skins
                .iter()
                .map(|skin| {
                    skin.clusters
                        .iter()
                        .map(|cluster| RvoExportCluster {
                            bone: cluster.bone,
                            name: cluster.name.as_ptr(),
                            transform: cluster.transform,
                            transform_link: cluster.transform_link,
                            vertices: opt_i32(&cluster.vertices),
                            weights: optional(&cluster.weights),
                            weight_count: cluster.weights.len(),
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let skin_mirrors: Vec<Vec<RvoExportSkin>> = scene
        .meshes
        .iter()
        .zip(&cluster_mirrors)
        .map(|(mesh, clusters)| {
            mesh.skins
                .iter()
                .zip(clusters)
                .map(|(skin, clusters)| RvoExportSkin {
                    skinning_type: skin.skinning_type,
                    clusters: if clusters.is_empty() {
                        std::ptr::null()
                    } else {
                        clusters.as_ptr()
                    },
                    cluster_count: clusters.len(),
                    dq_vertices: opt_i32(&skin.dq_vertices),
                    dq_weights: optional(&skin.dq_weights),
                    dq_count: skin.dq_weights.len(),
                    bind_pose: skin.bind_pose,
                })
                .collect()
        })
        .collect();
    let shape_mirrors: Vec<Vec<Vec<RvoExportBlendShape>>> = scene
        .meshes
        .iter()
        .map(|mesh| {
            mesh.blend_channels
                .iter()
                .map(|channel| {
                    channel
                        .shapes
                        .iter()
                        .map(|shape| RvoExportBlendShape {
                            name: shape.name.as_ptr(),
                            vertices: opt_i32(&shape.vertices),
                            offsets: optional(&shape.offsets),
                            normals: optional(&shape.normals),
                            offset_count: shape.vertices.len(),
                            target_weight: shape.target_weight,
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let channel_mirrors: Vec<Vec<RvoExportBlendChannel>> = scene
        .meshes
        .iter()
        .zip(&shape_mirrors)
        .map(|(mesh, shapes)| {
            mesh.blend_channels
                .iter()
                .zip(shapes)
                .map(|(channel, shapes)| RvoExportBlendChannel {
                    name: channel.name.as_ptr(),
                    weight: channel.weight,
                    shapes: if shapes.is_empty() {
                        std::ptr::null()
                    } else {
                        shapes.as_ptr()
                    },
                    shape_count: shapes.len(),
                })
                .collect()
        })
        .collect();
    let pose_nodes: Vec<Vec<RvoExportPoseNode>> = scene
        .poses
        .iter()
        .map(|pose| {
            pose.nodes
                .iter()
                .map(|&(node, matrix)| RvoExportPoseNode { node, matrix })
                .collect()
        })
        .collect();
    let poses: Vec<RvoExportPose> = scene
        .poses
        .iter()
        .zip(&pose_nodes)
        .map(|(pose, nodes)| RvoExportPose {
            name: pose.name.as_ptr(),
            nodes: if nodes.is_empty() {
                std::ptr::null()
            } else {
                nodes.as_ptr()
            },
            node_count: nodes.len(),
        })
        .collect();

    let meshes: Vec<RvoExportMesh> = scene
        .meshes
        .iter()
        .zip(&uv_pointers)
        .zip(skin_mirrors.iter().zip(&channel_mirrors))
        .map(
            |((mesh, (sets, names, color_sets)), (skins, channels))| RvoExportMesh {
                name: mesh.name.as_ptr(),
                node: mesh.node,
                positions: mesh.positions.as_ptr(),
                vertex_count: mesh.vertex_count,
                indices: mesh.indices.as_ptr(),
                index_count: mesh.indices.len(),
                face_offsets: mesh.face_offsets.as_ptr(),
                face_count: mesh.face_offsets.len().saturating_sub(1),
                normals: optional(&mesh.normals),
                colors: optional(&mesh.colors),
                tangents: optional(&mesh.tangents),
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
                color_set_name: mesh
                    .color_set_name
                    .as_ref()
                    .map_or(std::ptr::null(), |name| name.as_ptr()),
                color_sets: if color_sets.is_empty() {
                    std::ptr::null()
                } else {
                    color_sets.as_ptr()
                },
                color_set_count: color_sets.len(),
                face_smoothing: opt_u8(&mesh.face_smoothing),
                face_hole: opt_u8(&mesh.face_hole),
                face_group: opt_i32(&mesh.face_group),
                edges: opt_i32(&mesh.edges),
                edge_count: mesh.edges.len(),
                edge_smoothing: opt_u8(&mesh.edge_smoothing),
                edge_crease: optional(&mesh.edge_crease),
                edge_visibility: opt_u8(&mesh.edge_visibility),
                vertex_crease: optional(&mesh.vertex_crease),
                props: range(mesh.props),
                skins: if skins.is_empty() {
                    std::ptr::null()
                } else {
                    skins.as_ptr()
                },
                skin_count: skins.len(),
                blend_channels: if channels.is_empty() {
                    std::ptr::null()
                } else {
                    channels.as_ptr()
                },
                blend_channel_count: channels.len(),
            },
        )
        .collect();

    // Animation mirrors: keys, then curves, then the properties pointing at
    // them, then the layers — each level owning the storage the next borrows.
    let key_mirrors: Vec<Vec<[Vec<RvoExportKey>; 3]>> = scene
        .anim_layers
        .iter()
        .map(|layer| {
            layer
                .anim_props
                .iter()
                .map(|prop| {
                    let keys = |curve: &Option<CurveData>| -> Vec<RvoExportKey> {
                        curve
                            .as_ref()
                            .map(|curve| {
                                curve
                                    .keys
                                    .iter()
                                    .map(|key| RvoExportKey {
                                        time: key.time,
                                        value: key.value,
                                        flags: key.flags,
                                        weight_left: key.weight_left,
                                        weight_right: key.weight_right,
                                        slope_left: key.slope_left,
                                        slope_right: key.slope_right,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    [
                        keys(&prop.curves[0]),
                        keys(&prop.curves[1]),
                        keys(&prop.curves[2]),
                    ]
                })
                .collect()
        })
        .collect();
    let curve_mirrors: Vec<Vec<[Option<RvoExportCurve>; 3]>> = scene
        .anim_layers
        .iter()
        .zip(&key_mirrors)
        .map(|(layer, keys)| {
            layer
                .anim_props
                .iter()
                .zip(keys)
                .map(|(prop, keys)| {
                    let curve = |index: usize| -> Option<RvoExportCurve> {
                        prop.curves[index].as_ref().map(|curve| RvoExportCurve {
                            keys: if keys[index].is_empty() {
                                std::ptr::null()
                            } else {
                                keys[index].as_ptr()
                            },
                            key_count: keys[index].len(),
                            pre_mode: curve.pre_mode,
                            pre_repeat: curve.pre_repeat,
                            post_mode: curve.post_mode,
                            post_repeat: curve.post_repeat,
                        })
                    };
                    [curve(0), curve(1), curve(2)]
                })
                .collect()
        })
        .collect();
    let anim_prop_mirrors: Vec<Vec<RvoExportAnimProp>> = scene
        .anim_layers
        .iter()
        .zip(&curve_mirrors)
        .map(|(layer, curves)| {
            layer
                .anim_props
                .iter()
                .zip(curves)
                .map(|(prop, curves)| RvoExportAnimProp {
                    target_kind: prop.target_kind,
                    target: prop.target,
                    target2: prop.target2,
                    prop_name: prop.prop_name.as_ptr(),
                    default_value: prop.default,
                    curves: [
                        curves[0]
                            .as_ref()
                            .map_or(std::ptr::null(), |curve| curve as *const _),
                        curves[1]
                            .as_ref()
                            .map_or(std::ptr::null(), |curve| curve as *const _),
                        curves[2]
                            .as_ref()
                            .map_or(std::ptr::null(), |curve| curve as *const _),
                    ],
                })
                .collect()
        })
        .collect();
    let anim_layers: Vec<RvoExportAnimLayer> = scene
        .anim_layers
        .iter()
        .zip(&anim_prop_mirrors)
        .map(|(layer, props)| RvoExportAnimLayer {
            name: layer.name.as_ptr(),
            stack: layer.stack,
            weight: layer.weight,
            props: range(layer.props),
            anim_props: if props.is_empty() {
                std::ptr::null()
            } else {
                props.as_ptr()
            },
            anim_prop_count: props.len(),
        })
        .collect();
    let anim_stacks: Vec<RvoExportAnimStack> = scene
        .anim_stacks
        .iter()
        .map(|stack| RvoExportAnimStack {
            name: stack.name.as_ptr(),
            props: range(stack.props),
            time_begin: stack.time_begin,
            time_end: stack.time_end,
        })
        .collect();
    let display_layers: Vec<RvoExportDisplayLayer> = scene
        .display_layers
        .iter()
        .map(|layer| RvoExportDisplayLayer {
            name: layer.name.as_ptr(),
            props: range(layer.props),
            nodes: opt_i32(&layer.nodes),
            node_count: layer.nodes.len(),
        })
        .collect();
    let selection_node_mirrors: Vec<Vec<RvoExportSelectionNode>> = scene
        .selection_sets
        .iter()
        .map(|set| {
            set.nodes
                .iter()
                .map(|node| RvoExportSelectionNode {
                    node: node.node,
                    include_node: i32::from(node.include_node),
                    vertices: opt_i32(&node.vertices),
                    vertex_count: node.vertices.len(),
                    edges: opt_i32(&node.edges),
                    edge_count: node.edges.len(),
                    faces: opt_i32(&node.faces),
                    face_count: node.faces.len(),
                })
                .collect()
        })
        .collect();
    let selection_sets: Vec<RvoExportSelectionSet> = scene
        .selection_sets
        .iter()
        .zip(&selection_node_mirrors)
        .map(|(set, nodes)| RvoExportSelectionSet {
            name: set.name.as_ptr(),
            props: range(set.props),
            nodes: if nodes.is_empty() {
                std::ptr::null()
            } else {
                nodes.as_ptr()
            },
            node_count: nodes.len(),
        })
        .collect();

    let settings = &scene.settings;
    let payload = RvoExportScene {
        unit_scale_cm: scene.unit.unit_scale_cm,
        props: if props.is_empty() {
            std::ptr::null()
        } else {
            props.as_ptr()
        },
        prop_count: props.len(),
        axis_right: settings.axes[0],
        axis_up: settings.axes[1],
        axis_front: settings.axes[2],
        time_mode: settings.time_mode,
        frame_rate: settings.frame_rate,
        settings_props: range(settings.settings_props),
        scene_info_props: range(settings.scene_info_props),
        original_application_vendor: settings.original_application[0].as_ptr(),
        original_application_name: settings.original_application[1].as_ptr(),
        original_application_version: settings.original_application[2].as_ptr(),
        original_filename: settings.original_filename.as_ptr(),
        application_name: settings.application_name.as_ptr(),
        application_version: settings.application_version.as_ptr(),
        nodes: nodes.as_ptr(),
        node_count: nodes.len(),
        materials: if materials.is_empty() {
            std::ptr::null()
        } else {
            materials.as_ptr()
        },
        material_count: materials.len(),
        textures: if textures.is_empty() {
            std::ptr::null()
        } else {
            textures.as_ptr()
        },
        texture_count: textures.len(),
        videos: if videos.is_empty() {
            std::ptr::null()
        } else {
            videos.as_ptr()
        },
        video_count: videos.len(),
        meshes: meshes.as_ptr(),
        mesh_count: meshes.len(),
        poses: if poses.is_empty() {
            std::ptr::null()
        } else {
            poses.as_ptr()
        },
        pose_count: poses.len(),
        anim_stacks: if anim_stacks.is_empty() {
            std::ptr::null()
        } else {
            anim_stacks.as_ptr()
        },
        anim_stack_count: anim_stacks.len(),
        anim_layers: if anim_layers.is_empty() {
            std::ptr::null()
        } else {
            anim_layers.as_ptr()
        },
        anim_layer_count: anim_layers.len(),
        active_stack: scene.active_stack,
        display_layers: if display_layers.is_empty() {
            std::ptr::null()
        } else {
            display_layers.as_ptr()
        },
        display_layer_count: display_layers.len(),
        selection_sets: if selection_sets.is_empty() {
            std::ptr::null()
        } else {
            selection_sets.as_ptr()
        },
        selection_set_count: selection_sets.len(),
    };

    // Element type inferred as `c_char` from the call below, whose signedness is
    // the platform's rather than a fixed `i8`.
    let mut error = [0; crate::export_ffi::ERROR_LENGTH];
    // SAFETY: `payload` and every array it points at are live for this call and
    // sized exactly by the counts beside them; `c_path` is a valid NUL-terminated
    // string; `error` is a live buffer of exactly `ERROR_LENGTH` bytes, which is
    // what the length argument declares. The C side validates the payload's
    // internal consistency (index ranges, parent ordering, property ranges)
    // before using it, and frees everything it allocates on every path.
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
pub(crate) fn write_scene(
    _scene: &SceneData,
    _path: &Path,
    _format: FbxFormat,
) -> Result<(), OptError> {
    Err(OptError::Unavailable)
}

/// Check every stream the bridge reads by a count it is *given* rather than
/// the stream's own length, before any of them crosses into C.
///
/// The bridge validates what it can see - index ranges, parent order, property
/// ranges - but it is handed a vertex count and a pointer, so a normal stream
/// one vertex short is an out-of-bounds read it has no way to notice. The
/// builders produce these lengths correctly today; this is what keeps a builder
/// that drifts from turning into a read past a `Vec` rather than an error.
pub(crate) fn check_lengths(scene: &SceneData) -> Result<(), String> {
    for (index, mesh) in scene.meshes.iter().enumerate() {
        check_mesh_lengths(mesh).map_err(|what| format!("mesh {index}: {what}"))?;
    }
    Ok(())
}

fn check_mesh_lengths(mesh: &MeshData) -> Result<(), String> {
    let vertices = mesh.vertex_count;
    let faces = mesh.face_offsets.len().saturating_sub(1);
    let edges = mesh.edges.len();
    // Exactly `per * count` values, or none at all where the layer is optional.
    let sized = |what: &str, len: usize, per: usize, count: usize, optional: bool| {
        let expected = per.checked_mul(count);
        if (optional && len == 0) || expected == Some(len) {
            Ok(())
        } else {
            Err(format!("{what} has {len} values, expected {per} x {count}"))
        }
    };
    sized("positions", mesh.positions.len(), 3, vertices, false)?;
    sized("normals", mesh.normals.len(), 3, vertices, true)?;
    sized("colors", mesh.colors.len(), 4, vertices, true)?;
    sized("tangents", mesh.tangents.len(), 4, vertices, true)?;
    if mesh.uv_set_names.len() != mesh.uv_sets.len() {
        return Err("a UV set has no name".to_owned());
    }
    for set in &mesh.uv_sets {
        sized("a UV set", set.len(), 2, vertices, false)?;
    }
    for (_, values) in &mesh.color_sets {
        sized("a color set", values.len(), 4, vertices, false)?;
    }
    sized(
        "vertex creases",
        mesh.vertex_crease.len(),
        1,
        vertices,
        true,
    )?;
    let per_face_materials = mesh.material_slots.len() > 1;
    sized(
        "face materials",
        mesh.face_materials.len(),
        1,
        faces,
        !per_face_materials,
    )?;
    sized("face smoothing", mesh.face_smoothing.len(), 1, faces, true)?;
    sized("face holes", mesh.face_hole.len(), 1, faces, true)?;
    sized("face groups", mesh.face_group.len(), 1, faces, true)?;
    sized("edge smoothing", mesh.edge_smoothing.len(), 1, edges, true)?;
    sized("edge creases", mesh.edge_crease.len(), 1, edges, true)?;
    sized(
        "edge visibility",
        mesh.edge_visibility.len(),
        1,
        edges,
        true,
    )?;
    for skin in &mesh.skins {
        for cluster in &skin.clusters {
            sized(
                "a cluster's vertices",
                cluster.vertices.len(),
                1,
                cluster.weights.len(),
                false,
            )?;
        }
        sized(
            "blend-weight vertices",
            skin.dq_vertices.len(),
            1,
            skin.dq_weights.len(),
            false,
        )?;
    }
    for channel in &mesh.blend_channels {
        for shape in &channel.shapes {
            let offsets = shape.vertices.len();
            sized(
                "a blend shape's offsets",
                shape.offsets.len(),
                3,
                offsets,
                false,
            )?;
            sized(
                "a blend shape's normals",
                shape.normals.len(),
                3,
                offsets,
                true,
            )?;
        }
    }
    Ok(())
}

/// A null pointer for an empty buffer — the bridge reads "attribute absent".
#[cfg(has_ufbxw)]
pub(crate) fn optional(values: &[f64]) -> *const f64 {
    if values.is_empty() {
        std::ptr::null()
    } else {
        values.as_ptr()
    }
}

/// Read the bridge's NUL-terminated message out of its fixed buffer.
#[cfg(has_ufbxw)]
pub(crate) fn error_message(buffer: &[c_char]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-triangle mesh whose every stream is the length the bridge reads.
    fn triangle() -> MeshData {
        MeshData {
            positions: vec![0.0; 9],
            indices: vec![0, 1, 2],
            face_offsets: vec![0, 3],
            vertex_count: 3,
            normals: vec![0.0; 9],
            uv_sets: vec![vec![0.0; 6]],
            uv_set_names: vec![CString::new("map1").expect("no NUL")],
            ..MeshData::default()
        }
    }

    #[test]
    fn a_consistent_mesh_passes() {
        assert_eq!(check_mesh_lengths(&triangle()), Ok(()));
    }

    /// Each of these is a count the bridge is handed separately from the
    /// stream, so a short stream would be read past its end in C.
    #[test]
    fn a_stream_shorter_than_its_count_is_refused() {
        type Breaks = fn(&mut MeshData);
        let cases: [(&str, Breaks); 6] = [
            ("normals", |mesh| {
                mesh.normals.pop();
            }),
            ("UV set", |mesh| mesh.uv_sets[0].truncate(4)),
            ("tangents", |mesh| mesh.tangents = vec![0.0; 8]),
            ("vertex creases", |mesh| mesh.vertex_crease = vec![0.0; 2]),
            ("face smoothing", |mesh| mesh.face_smoothing = vec![0; 2]),
            ("cluster", |mesh| {
                mesh.skins = vec![SkinExportData {
                    clusters: vec![ClusterData {
                        vertices: vec![0, 1],
                        weights: vec![1.0],
                        ..ClusterData::default()
                    }],
                    ..SkinExportData::default()
                }];
            }),
        ];
        for (what, break_it) in cases {
            let mut mesh = triangle();
            break_it(&mut mesh);
            assert!(check_mesh_lengths(&mesh).is_err(), "{what} was not caught");
        }
    }

    #[test]
    fn per_face_materials_are_required_once_there_are_several_slots() {
        let mut mesh = triangle();
        mesh.material_slots = vec![0, 1];
        assert!(check_mesh_lengths(&mesh).is_err());
        mesh.face_materials = vec![1];
        assert_eq!(check_mesh_lengths(&mesh), Ok(()));
    }
}
