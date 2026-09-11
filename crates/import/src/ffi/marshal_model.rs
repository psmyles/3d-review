//! The bridge's flat arrays -> `review_model::ModelData`.
//!
//! This is the funnel invariant 7 names: every FBX the viewer opens comes through
//! here, and any future caller of the C core must produce an identical
//! `ModelData`. No `unsafe` of its own — it reads the checked slices
//! [`super::raw`] hands back and owns nothing the C side allocated.

// Safe code over the checked slices `raw` hands back: no pointer reaches here.
#![deny(unsafe_code)]

use std::path::Path;

use glam::{DMat4, Mat4, Quat, Vec2, Vec3, Vec4};
use review_model::{
    AnimContext, AnimationClip, BoneInfo, Key, LocalTransform, MaterialImportDefaults, ModelData,
    ModelStats, MorphChannel, MorphData, MorphKeyframe, MorphShape, MorphTrack, NodeKind,
    NodeTrack, SceneNode, SkinCluster, SkinData, SkinDeformerInfo, SkinningMethod, TopologyFace,
    TriangleData, Vertex, anim,
};

use crate::ImportError;

use super::raw::{checked_slice, read_optional_c_string};
use super::raw_scene::*;

/// The bridge's flat geometry arrays, marshaled into owned Rust buffers.
pub(super) struct MarshaledGeometry {
    pub(super) vertices: Vec<Vertex>,
    pub(super) indices: Vec<u32>,
    pub(super) faces: Vec<TopologyFace>,
    pub(super) triangles: TriangleData,
}

/// Marshal the bridge's flat geometry arrays (vertices / indices / original
/// faces / per-triangle metadata) into owned Rust buffers.
pub(super) fn marshal_geometry(
    scene: &ReviewImportScene,
) -> Result<MarshaledGeometry, ImportError> {
    let vertices = checked_slice(scene.vertices, scene.vertex_count, "vertices")?
        .iter()
        .map(|vertex| Vertex {
            position: Vec3::from_array(vertex.position),
            normal: Vec3::from_array(vertex.normal),
            uv: Vec2::from_array(vertex.uv),
            tangent: Vec4::from_array(vertex.tangent),
            vertex_color: Vec4::from_array(vertex.vertex_color),
        })
        .collect::<Vec<_>>();
    let indices = checked_slice(scene.indices, scene.index_count, "indices")?.to_vec();
    let faces = checked_slice(scene.faces, scene.face_count, "faces")?
        .iter()
        .map(|face| TopologyFace {
            first_index: face.first_index,
            index_count: face.index_count,
        })
        .collect::<Vec<_>>();
    let triangles = TriangleData {
        to_face: checked_slice(scene.tri_to_face, scene.tri_to_face_count, "tri_to_face")?.to_vec(),
        material: checked_slice(scene.tri_material, scene.tri_material_count, "tri_material")?
            .to_vec(),
        node: checked_slice(scene.tri_node, scene.tri_node_count, "tri_node")?.to_vec(),
    };
    Ok(MarshaledGeometry {
        vertices,
        indices,
        faces,
        triangles,
    })
}

/// The bridge's `review_import_node_kind` codes. An unrecognized code is
/// [`NodeKind::Other`] rather than an error, so the C side can grow a new
/// attribute type without breaking an older Rust build.
pub(super) fn node_kind_from_code(code: u32) -> NodeKind {
    match code {
        1 => NodeKind::Mesh,
        2 => NodeKind::Bone,
        3 => NodeKind::Light,
        4 => NodeKind::Camera,
        5 => NodeKind::Empty,
        _ => NodeKind::Other,
    }
}

/// Marshal the bridge's scene-graph node table for the Outliner.
pub(super) fn marshal_nodes(scene: &ReviewImportScene) -> Result<Vec<SceneNode>, ImportError> {
    Ok(checked_slice(scene.nodes, scene.node_count, "nodes")?
        .iter()
        .map(|node| {
            let kind = node_kind_from_code(node.kind);
            SceneNode {
                name: read_optional_c_string(node.name).unwrap_or_default(),
                parent: (node.parent >= 0).then_some(node.parent as usize),
                mesh_part: (node.mesh_part_index >= 0).then_some(node.mesh_part_index as usize),
                source_vertex_count: node.source_vertex_count as usize,
                transform: Mat4::from_cols_array(&node.transform),
                rest_local: LocalTransform {
                    translation: Vec3::from_array(node.local_translation),
                    rotation: Quat::from_array(node.local_rotation).normalize(),
                    scale: Vec3::from_array(node.local_scale),
                },
                kind,
                bone: (kind == NodeKind::Bone).then_some(BoneInfo {
                    radius: node.bone_radius,
                    relative_length: node.bone_relative_length,
                }),
            }
        })
        .collect())
}

/// The bridge's `review_import_skin_deformer::method` codes (ufbx's
/// `ufbx_skinning_method`). Unknown codes read as linear, which is how the
/// viewer evaluates every skin anyway.
pub(super) fn skinning_method_from_code(code: u32) -> SkinningMethod {
    match code {
        1 => SkinningMethod::Rigid,
        2 => SkinningMethod::DualQuaternion,
        3 => SkinningMethod::BlendedDqLinear,
        _ => SkinningMethod::Linear,
    }
}

/// The per-corner logical-vertex map, marshaled for every model (skinned or
/// not) since blend shapes need it as well as skin weights.
pub(super) fn marshal_corner_map(scene: &ReviewImportScene) -> Result<Vec<u32>, ImportError> {
    Ok(checked_slice(
        scene.corner_source_vertex,
        scene.corner_source_vertex_count,
        "corner_source_vertex",
    )?
    .to_vec())
}

/// Marshal the bridge's CSR skin table + cluster table. Returns `None` for
/// an unskinned scene (the bridge allocates nothing and reports zero
/// influences), so the common non-skeletal model carries no skin payload at
/// all.
pub(super) fn marshal_skin(scene: &ReviewImportScene) -> Result<Option<SkinData>, ImportError> {
    if scene.skin_influence_count == 0 {
        return Ok(None);
    }
    let clusters = checked_slice(
        scene.skin_clusters,
        scene.skin_cluster_count,
        "skin_clusters",
    )?
    .iter()
    .map(|cluster| SkinCluster {
        bone: cluster.bone,
        mesh_node: cluster.mesh_node,
        world_to_bone_bind: Mat4::from_cols_array(&cluster.world_to_bone_bind),
        mesh_node_to_bone: DMat4::from_cols_array(&cluster.mesh_node_to_bone).as_mat4(),
        bind_to_world: DMat4::from_cols_array(&cluster.bind_to_world).as_mat4(),
        name: read_optional_c_string(cluster.name).unwrap_or_default(),
    })
    .collect();
    let deformers = checked_slice(
        scene.skin_deformers,
        scene.skin_deformer_count,
        "skin_deformers",
    )?
    .iter()
    .map(|deformer| SkinDeformerInfo {
        mesh_node: deformer.mesh_node,
        method: skinning_method_from_code(deformer.method),
        max_weights_per_vertex: deformer.max_weights_per_vertex,
    })
    .collect();
    Ok(Some(SkinData {
        offsets: checked_slice(scene.skin_offsets, scene.skin_offset_count, "skin_offsets")?
            .to_vec(),
        bones: checked_slice(scene.skin_bones, scene.skin_influence_count, "skin_bones")?.to_vec(),
        weights: checked_slice(
            scene.skin_weights,
            scene.skin_influence_count,
            "skin_weights",
        )?
        .to_vec(),
        influence_cluster: checked_slice(
            scene.skin_influence_cluster,
            scene.skin_influence_count,
            "skin_influence_cluster",
        )?
        .to_vec(),
        clusters,
        deformers,
    }))
}

/// Marshal the bridge's blend shapes: the sparse per-shape offsets are
/// sorted into the per-logical-vertex CSR the shader and the CPU reference
/// walk. `None` when no mesh carries a channel.
pub(super) fn marshal_morph(
    scene: &ReviewImportScene,
    logical_count: usize,
) -> Result<Option<MorphData>, ImportError> {
    if scene.morph_channel_count == 0 {
        return Ok(None);
    }
    let keyframes = checked_slice(
        scene.morph_keyframes,
        scene.morph_keyframe_count,
        "morph_keyframes",
    )?;
    let channels = checked_slice(
        scene.morph_channels,
        scene.morph_channel_count,
        "morph_channels",
    )?
    .iter()
    .map(|channel| {
        let first = channel.keyframe_first as usize;
        let end = first.saturating_add(channel.keyframe_count as usize);
        let range = keyframes.get(first..end).ok_or_else(|| {
            ImportError::LoadFailed(format!(
                "FBX bridge morph channel keyframes {first}..{end} exceed {}",
                keyframes.len()
            ))
        })?;
        Ok(MorphChannel {
            name: read_optional_c_string(channel.name).unwrap_or_default(),
            mesh_node: channel.mesh_node,
            rest_weight: channel.rest_weight,
            keyframes: range
                .iter()
                .map(|key| MorphKeyframe {
                    shape: key.shape,
                    target_weight: key.target_weight,
                })
                .collect(),
        })
    })
    .collect::<Result<Vec<_>, ImportError>>()?;
    let shapes = checked_slice(scene.morph_shapes, scene.morph_shape_count, "morph_shapes")?
        .iter()
        .map(|shape| MorphShape {
            name: read_optional_c_string(shape.name).unwrap_or_default(),
        })
        .collect();
    let entries = checked_slice(
        scene.morph_entries,
        scene.morph_entry_count,
        "morph_entries",
    )?;

    // Counting sort into CSR rows by logical vertex, preserving the bridge's
    // emit order within a row. An entry naming a vertex past the logical
    // count is a bridge drift and fails the load here.
    let mut offsets = vec![0u32; logical_count + 1];
    for entry in entries {
        let logical = entry.logical_vertex as usize;
        if logical >= logical_count {
            return Err(ImportError::LoadFailed(format!(
                "FBX bridge morph entry references source vertex {logical} of {logical_count}"
            )));
        }
        offsets[logical + 1] += 1;
    }
    for index in 1..offsets.len() {
        offsets[index] += offsets[index - 1];
    }
    let mut cursor = offsets.clone();
    let mut shape = vec![0u32; entries.len()];
    let mut position = vec![Vec3::ZERO; entries.len()];
    let mut normal = vec![Vec3::ZERO; entries.len()];
    for entry in entries {
        let slot = &mut cursor[entry.logical_vertex as usize];
        let index = *slot as usize;
        *slot += 1;
        shape[index] = entry.shape;
        position[index] = Vec3::from_array(entry.position);
        normal[index] = Vec3::from_array(entry.normal);
    }

    Ok(Some(MorphData {
        channels,
        shapes,
        offsets,
        shape,
        position,
        normal,
    }))
}

/// Marshal the bridge's baked animation stacks into clips.
pub(super) fn marshal_animations(
    scene: &ReviewImportScene,
) -> Result<Vec<AnimationClip>, ImportError> {
    if scene.anim_stack_count == 0 {
        return Ok(Vec::new());
    }
    let node_tracks = checked_slice(
        scene.anim_node_tracks,
        scene.anim_node_track_count,
        "anim_node_tracks",
    )?;
    let vec3_keys = checked_slice(
        scene.anim_vec3_keys,
        scene.anim_vec3_key_count,
        "anim_vec3_keys",
    )?;
    let quat_keys = checked_slice(
        scene.anim_quat_keys,
        scene.anim_quat_key_count,
        "anim_quat_keys",
    )?;
    let morph_tracks = checked_slice(
        scene.anim_morph_tracks,
        scene.anim_morph_track_count,
        "anim_morph_tracks",
    )?;
    let scalar_keys = checked_slice(
        scene.anim_scalar_keys,
        scene.anim_scalar_key_count,
        "anim_scalar_keys",
    )?;

    fn range<'a, T>(
        items: &'a [T],
        first: u32,
        count: u32,
        what: &str,
    ) -> Result<&'a [T], ImportError> {
        let first = first as usize;
        let end = first.saturating_add(count as usize);
        items.get(first..end).ok_or_else(|| {
            ImportError::LoadFailed(format!(
                "FBX bridge {what} range {first}..{end} exceeds {}",
                items.len()
            ))
        })
    }
    let vec3 = |first: u32, count: u32| -> Result<Vec<Key<Vec3>>, ImportError> {
        Ok(range(vec3_keys, first, count, "vec3 key")?
            .iter()
            .map(|key| Key {
                time: key.time,
                value: Vec3::from_array(key.value),
            })
            .collect())
    };

    checked_slice(scene.anim_stacks, scene.anim_stack_count, "anim_stacks")?
        .iter()
        .map(|stack| {
            let tracks = range(
                node_tracks,
                stack.node_track_first,
                stack.node_track_count,
                "node track",
            )?
            .iter()
            .map(|track| {
                Ok(NodeTrack {
                    node: track.node,
                    translation: vec3(track.translation_first, track.translation_count)?,
                    rotation: range(
                        quat_keys,
                        track.rotation_first,
                        track.rotation_count,
                        "quat key",
                    )?
                    .iter()
                    .map(|key| Key {
                        time: key.time,
                        value: Quat::from_array(key.value),
                    })
                    .collect(),
                    scale: vec3(track.scale_first, track.scale_count)?,
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;
            let morph_tracks = range(
                morph_tracks,
                stack.morph_track_first,
                stack.morph_track_count,
                "morph track",
            )?
            .iter()
            .map(|track| {
                Ok(MorphTrack {
                    channel: track.channel,
                    keys: range(scalar_keys, track.first, track.count, "scalar key")?
                        .iter()
                        .map(|key| Key {
                            time: key.time,
                            value: key.value,
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>, ImportError>>()?;
            Ok(AnimationClip {
                name: read_optional_c_string(stack.name).unwrap_or_default(),
                time_begin: stack.time_begin,
                time_end: stack.time_end,
                tracks,
                morph_tracks,
            })
        })
        .collect()
}

/// Marshal the bridge's material table into the editable table's import
/// defaults.
pub(super) fn marshal_materials(
    scene: &ReviewImportScene,
) -> Result<Vec<MaterialImportDefaults>, ImportError> {
    Ok(
        checked_slice(scene.materials, scene.material_count, "materials")?
            .iter()
            .map(|material| MaterialImportDefaults {
                name: read_optional_c_string(material.name).unwrap_or_else(|| "Default".to_owned()),
                base_color: Vec3::from_array(material.base_color),
                smoothness: material.smoothness,
                metallic: material.metallic,
                emissive: Vec3::from_array(material.emissive),
            })
            .collect(),
    )
}

pub(super) fn model_from_bridge_scene(
    path: &Path,
    scene: &ReviewImportScene,
    progress: crate::ProgressSink<'_>,
) -> Result<ModelData, ImportError> {
    progress(crate::ImportProgress::stage(crate::ImportStage::Building));
    let MarshaledGeometry {
        vertices,
        indices,
        faces,
        triangles,
    } = marshal_geometry(scene)?;
    let nodes = marshal_nodes(scene)?;
    let corner_to_logical = marshal_corner_map(scene)?;
    let skin = marshal_skin(scene)?;
    let morph = marshal_morph(scene, scene.source_vertex_count)?;
    let animations = marshal_animations(scene)?;
    let bone_count = nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Bone)
        .count();
    let clip_count = animations.len();
    let uv_channels = build_uv_channels(scene)?;
    let uv_set_names = checked_slice(scene.uv_set_names, scene.uv_set_name_count, "uv_set_names")?
        .iter()
        .map(|&name| read_optional_c_string(name).unwrap_or_default())
        .collect::<Vec<_>>();
    let materials = marshal_materials(scene)?;

    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Imported Model")
        .to_owned();

    let mut model = ModelData {
        name,
        vertices,
        indices,
        faces,
        triangles,
        nodes,
        uv_channels,
        uv_set_names,
        bounds: None,
        stats: ModelStats {
            polygon_count: scene.face_count,
            triangle_count: scene.index_count / 3,
            // The source DCC's logical vertex count, not the per-corner
            // expanded render count (invariant 5: faithful stats).
            vertex_count: scene.source_vertex_count,
            // Measured below once the whole model is assembled, like Draws.
            gpu_vertex_count: 0,
            uv_set_count: scene.uv_set_count as usize,
            material_count: scene.material_count,
            // Overwritten below from `material_draw_count` so the Draws stat
            // matches the renderer's per-material grouping (invariant 5).
            draw_count: 0,
            bone_count,
            clip_count,
            source_unit_meters: scene.source_unit_meters,
        },
        materials,
        corner_to_logical,
        skin,
        morph,
        animations,
        frame_rate: scene.frames_per_second,
    };

    // Every index must address a real vertex. The bridge derives each one by
    // subtracting a face's first index from a triangulation result, an
    // unsigned subtraction that would wrap into a huge index rather than
    // fail if either ever drifted — so it is checked here, at the funnel,
    // before the renderer or the optimizer reads the buffer.
    if let Some(&index) = model
        .indices
        .iter()
        .find(|&&index| index as usize >= model.vertices.len())
    {
        return Err(ImportError::LoadFailed(format!(
            "FBX bridge returned index {index} for a mesh of {} vertices",
            model.vertices.len()
        )));
    }

    // Lockstep + range guard at the one import funnel (invariant 7), *before*
    // anything consumes the per-triangle arrays: each array must be empty or
    // exactly `triangle_count` long, and every entry must index a real
    // face / material / node. Catches a bridge marshaling drift here, once,
    // rather than in every renderer-side reader.
    model
        .triangles
        .validate(
            model.stats.triangle_count,
            model.faces.len(),
            model.materials.len(),
            model.nodes.len(),
        )
        .map_err(ImportError::LoadFailed)?;

    // Same funnel guard for everything the deform path reads: the corner map
    // must cover exactly the render mesh, the skin / morph CSR rows must be
    // monotonic and terminate at their entry counts, every influence must
    // name a real node with a finite non-negative weight (and a cluster
    // binding that same bone), and every clip must animate real nodes /
    // channels with ordered finite keys. A drifted bridge fill is caught
    // here, once.
    model.validate_deform().map_err(ImportError::LoadFailed)?;

    // Bounds describe what is on screen. A skinned or morphed model rests in
    // the file's default pose, which the GPU skins into — so its bounds are
    // measured through the same deformation, not from the bind-pose buffer.
    // (Each clip's own motion envelope is *not* measured here: nothing is on
    // screen that needs it until a clip is selected, and on a large scene it
    // costs more than everything above it — see `measure_clip_bounds`.)
    if model.needs_deform() {
        let _z = crate::prof::zone!("Rest Bounds");
        let ctx = AnimContext::new(&model);
        model.bounds = anim::rest_bounds(&model, &ctx);
    } else {
        model.recompute_bounds();
    }
    // The FBX may carry UVs but no tangent layer (common for Maya exports); the
    // bridge then leaves a zero tangent per vertex. Synthesize a real tangent
    // basis from the UVs + normals so normal maps shade correctly — done once
    // here at the single import funnel (invariant 7).
    if model.has_degenerate_tangents() {
        model.generate_tangents();
    }
    // The renderer groups triangles into one draw per distinct material slot;
    // report that count so Draws is the real draw-call count.
    model.stats.draw_count = model.material_draw_count();
    // `stats.gpu_vertex_count` stays 0 — "not measured yet". It is an
    // O(corners) hash walk over the whole mesh, by far the longest item in a
    // large import, and it is a *displayed number*, not something the viewport
    // draws with; the caller measures it once the model is up.

    Ok(model)
}

/// Builds the per-channel UV table for multi-set models. Single-set models
/// (and meshes the bridge left without a `uvs` allocation) return an empty
/// vec, in which case the renderer falls back to [`Vertex::uv`].
pub(super) fn build_uv_channels(scene: &ReviewImportScene) -> Result<Vec<Vec<Vec2>>, ImportError> {
    let channel_count = scene.uv_set_count as usize;
    if channel_count <= 1 || scene.uvs.is_null() {
        return Ok(Vec::new());
    }

    // Checked: both counts are C-provided, so a wrapped multiply here would
    // defeat the length guard below and turn the indexing loop into a panic.
    let vertex_count = scene.vertex_count;
    let expected = channel_count
        .checked_mul(vertex_count)
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(|| {
            ImportError::LoadFailed(format!(
                "FBX bridge UV table overflows: {channel_count} channels x {vertex_count} vertices"
            ))
        })?;
    if scene.uv_value_count < expected {
        return Err(ImportError::LoadFailed(format!(
            "FBX bridge returned {} UV values, expected {expected}",
            scene.uv_value_count
        )));
    }

    let values = checked_slice(scene.uvs, scene.uv_value_count, "uvs")?;
    let channels = (0..channel_count)
        .map(|channel| {
            (0..vertex_count)
                .map(|vertex| {
                    // In bounds: `base + 1 <= expected - 1 < values.len()`,
                    // with `expected` overflow-checked above.
                    let base = (channel * vertex_count + vertex) * 2;
                    Vec2::new(values[base], values[base + 1])
                })
                .collect()
        })
        .collect();

    Ok(channels)
}

// The module's own code is safe; these tests fabricate the bridge's
// `#[repr(C)]` mirrors with `mem::zeroed()` to drive the marshallers without a
// C call, which is what the deny above has to make room for.
#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;

    /// A zeroed bridge scene (every pointer null, every count 0) with just the
    /// UV fields set — the shape `build_uv_channels` consumes.
    // SAFETY (of the test helper): `ReviewImportScene` is a plain `#[repr(C)]`
    // struct of pointers + integers, for which all-zero bytes is a valid value.
    fn uv_scene(uv_set_count: u32, vertex_count: usize, uvs: &[f32]) -> ReviewImportScene {
        let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
        scene.uv_set_count = uv_set_count;
        scene.vertex_count = vertex_count;
        scene.uvs = uvs.as_ptr() as *mut f32;
        scene.uv_value_count = uvs.len();
        scene
    }

    #[test]
    fn build_uv_channels_reads_channel_major_values() {
        // 2 channels × 2 vertices × (u, v): channel-major layout.
        let uvs = [0.0, 0.1, 0.2, 0.3, 1.0, 1.1, 1.2, 1.3];
        let scene = uv_scene(2, 2, &uvs);
        let channels = build_uv_channels(&scene).expect("valid UV table");
        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0], vec![Vec2::new(0.0, 0.1), Vec2::new(0.2, 0.3)]);
        assert_eq!(channels[1], vec![Vec2::new(1.0, 1.1), Vec2::new(1.2, 1.3)]);
    }

    #[test]
    fn build_uv_channels_rejects_a_short_buffer() {
        // 2 channels × 2 vertices needs 8 values; give 6.
        let uvs = [0.0; 6];
        let scene = uv_scene(2, 2, &uvs);
        assert!(matches!(
            build_uv_channels(&scene),
            Err(ImportError::LoadFailed(_))
        ));
    }

    /// A zeroed scene whose CSR skin arrays point at Rust-owned data — the
    /// same trick `uv_scene` uses, so `marshal_skin` can be exercised without
    /// an FBX file.
    fn skin_scene(
        corner_to_logical: &[u32],
        offsets: &[u32],
        bones: &[u32],
        weights: &[f32],
        influence_cluster: &[u32],
        clusters: &[ReviewImportSkinCluster],
    ) -> ReviewImportScene {
        let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
        scene.vertex_count = corner_to_logical.len();
        scene.corner_source_vertex = corner_to_logical.as_ptr() as *mut u32;
        scene.corner_source_vertex_count = corner_to_logical.len();
        scene.skin_offsets = offsets.as_ptr() as *mut u32;
        scene.skin_offset_count = offsets.len();
        scene.skin_bones = bones.as_ptr() as *mut u32;
        scene.skin_weights = weights.as_ptr() as *mut f32;
        scene.skin_influence_count = bones.len();
        scene.skin_influence_cluster = influence_cluster.as_ptr() as *mut u32;
        scene.skin_clusters = clusters.as_ptr() as *mut ReviewImportSkinCluster;
        scene.skin_cluster_count = clusters.len();
        scene
    }

    fn identity_cluster(bone: u32) -> ReviewImportSkinCluster {
        ReviewImportSkinCluster {
            bone,
            mesh_node: 0,
            world_to_bone_bind: Mat4::IDENTITY.to_cols_array(),
            mesh_node_to_bone: DMat4::IDENTITY.to_cols_array(),
            bind_to_world: DMat4::IDENTITY.to_cols_array(),
            name: std::ptr::null_mut(),
        }
    }

    #[test]
    fn marshal_skin_is_none_for_an_unskinned_scene() {
        let scene: ReviewImportScene = unsafe { std::mem::zeroed() };
        assert!(
            marshal_skin(&scene)
                .expect("no skin is not an error")
                .is_none()
        );
    }

    #[test]
    fn marshal_skin_copies_the_csr_table() {
        let clusters = [identity_cluster(1), identity_cluster(2)];
        let scene = skin_scene(
            &[0, 0, 1],
            &[0, 2, 3],
            &[1, 2, 2],
            &[0.75, 0.25, 1.0],
            &[0, 1, 1],
            &clusters,
        );
        let corner_map = marshal_corner_map(&scene).expect("valid corner map");
        let skin = marshal_skin(&scene)
            .expect("valid skin")
            .expect("a skinned scene marshals to Some");
        assert_eq!(corner_map, vec![0, 0, 1]);
        assert_eq!(skin.offsets, vec![0, 2, 3]);
        assert_eq!(skin.bones, vec![1, 2, 2]);
        assert_eq!(skin.weights, vec![0.75, 0.25, 1.0]);
        assert_eq!(skin.influence_cluster, vec![0, 1, 1]);
        assert_eq!(skin.clusters.len(), 2);
        assert_eq!(skin.clusters[1].bone, 2);
        assert_eq!(skin.clusters[1].world_to_bone_bind, Mat4::IDENTITY);
        assert_eq!(skin.influence_range(0), 0..2);
        assert_eq!(skin.validate(2, 3), Ok(()));
    }

    #[test]
    fn marshal_skin_rejects_a_null_array_with_a_nonzero_count() {
        // A count without its pointer must be a clean error naming the field,
        // not a `from_raw_parts` on null.
        let clusters = [identity_cluster(1)];
        let mut scene = skin_scene(&[0], &[0, 1], &[1], &[1.0], &[0], &clusters);
        scene.skin_bones = std::ptr::null_mut();
        assert!(matches!(
            marshal_skin(&scene),
            Err(ImportError::LoadFailed(message)) if message.contains("skin_bones")
        ));
        let mut scene = skin_scene(&[0], &[0, 1], &[1], &[1.0], &[0], &clusters);
        scene.skin_influence_cluster = std::ptr::null_mut();
        assert!(matches!(
            marshal_skin(&scene),
            Err(ImportError::LoadFailed(message)) if message.contains("skin_influence_cluster")
        ));
    }

    #[test]
    fn marshal_morph_sorts_entries_into_logical_rows() {
        // Two shapes over three logical vertices, emitted out of vertex order.
        let keyframes = [
            ReviewImportMorphKeyframe {
                shape: 0,
                target_weight: 1.0,
            },
            ReviewImportMorphKeyframe {
                shape: 1,
                target_weight: 1.0,
            },
        ];
        let channels = [
            ReviewImportMorphChannel {
                name: std::ptr::null_mut(),
                mesh_node: 0,
                rest_weight: 0.25,
                keyframe_first: 0,
                keyframe_count: 1,
            },
            ReviewImportMorphChannel {
                name: std::ptr::null_mut(),
                mesh_node: 0,
                rest_weight: 0.0,
                keyframe_first: 1,
                keyframe_count: 1,
            },
        ];
        let shapes = [
            ReviewImportMorphShape {
                name: std::ptr::null_mut(),
            },
            ReviewImportMorphShape {
                name: std::ptr::null_mut(),
            },
        ];
        let entry = |logical_vertex: u32, shape: u32, x: f32| ReviewImportMorphEntry {
            logical_vertex,
            shape,
            position: [x, 0.0, 0.0],
            normal: [0.0; 3],
        };
        let entries = [entry(2, 0, 2.0), entry(0, 1, 0.5), entry(2, 1, 2.5)];
        let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
        scene.morph_channels = channels.as_ptr() as *mut ReviewImportMorphChannel;
        scene.morph_channel_count = channels.len();
        scene.morph_keyframes = keyframes.as_ptr() as *mut ReviewImportMorphKeyframe;
        scene.morph_keyframe_count = keyframes.len();
        scene.morph_shapes = shapes.as_ptr() as *mut ReviewImportMorphShape;
        scene.morph_shape_count = shapes.len();
        scene.morph_entries = entries.as_ptr() as *mut ReviewImportMorphEntry;
        scene.morph_entry_count = entries.len();

        let morph = marshal_morph(&scene, 3)
            .expect("valid morph")
            .expect("channels marshal to Some");
        assert_eq!(morph.channels.len(), 2);
        assert_eq!(morph.channels[0].rest_weight, 0.25);
        assert_eq!(morph.channels[1].keyframes[0].shape, 1);
        assert_eq!(morph.offsets, vec![0, 1, 1, 3]);
        assert_eq!(morph.shape, vec![1, 0, 1]);
        assert_eq!(morph.position[0].x, 0.5);
        assert_eq!(morph.position[1].x, 2.0);
        assert_eq!(morph.position[2].x, 2.5);
        assert_eq!(morph.validate(3, 1), Ok(()));

        // An entry past the logical range is a bridge drift, not a panic.
        let bad = [entry(7, 0, 1.0)];
        scene.morph_entries = bad.as_ptr() as *mut ReviewImportMorphEntry;
        scene.morph_entry_count = bad.len();
        assert!(matches!(
            marshal_morph(&scene, 3),
            Err(ImportError::LoadFailed(message)) if message.contains("source vertex 7 of 3")
        ));
    }

    #[test]
    fn marshal_animations_reads_tracks_and_key_ranges() {
        let vec3_keys = [
            ReviewImportVec3Key {
                time: 0.0,
                value: [0.0, 1.0, 0.0],
            },
            ReviewImportVec3Key {
                time: 1.0,
                value: [2.0, 1.0, 0.0],
            },
        ];
        let quat_keys = [ReviewImportQuatKey {
            time: 0.5,
            value: [0.0, 0.0, 0.0, 1.0],
        }];
        let tracks = [ReviewImportNodeTrack {
            node: 3,
            translation_first: 0,
            translation_count: 2,
            rotation_first: 0,
            rotation_count: 1,
            scale_first: 2,
            scale_count: 0,
        }];
        let scalar_keys = [ReviewImportScalarKey {
            time: 0.0,
            value: 0.5,
        }];
        let morph_tracks = [ReviewImportMorphTrack {
            channel: 1,
            first: 0,
            count: 1,
        }];
        let stacks = [ReviewImportAnimStack {
            name: std::ptr::null_mut(),
            time_begin: 0.0,
            time_end: 1.0,
            node_track_first: 0,
            node_track_count: 1,
            morph_track_first: 0,
            morph_track_count: 1,
        }];
        let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
        scene.anim_stacks = stacks.as_ptr() as *mut ReviewImportAnimStack;
        scene.anim_stack_count = stacks.len();
        scene.anim_node_tracks = tracks.as_ptr() as *mut ReviewImportNodeTrack;
        scene.anim_node_track_count = tracks.len();
        scene.anim_vec3_keys = vec3_keys.as_ptr() as *mut ReviewImportVec3Key;
        scene.anim_vec3_key_count = vec3_keys.len();
        scene.anim_quat_keys = quat_keys.as_ptr() as *mut ReviewImportQuatKey;
        scene.anim_quat_key_count = quat_keys.len();
        scene.anim_morph_tracks = morph_tracks.as_ptr() as *mut ReviewImportMorphTrack;
        scene.anim_morph_track_count = morph_tracks.len();
        scene.anim_scalar_keys = scalar_keys.as_ptr() as *mut ReviewImportScalarKey;
        scene.anim_scalar_key_count = scalar_keys.len();

        let clips = marshal_animations(&scene).expect("valid animation");
        assert_eq!(clips.len(), 1);
        let clip = &clips[0];
        assert_eq!(clip.time_end, 1.0);
        assert_eq!(clip.tracks.len(), 1);
        assert_eq!(clip.tracks[0].node, 3);
        assert_eq!(clip.tracks[0].translation.len(), 2);
        assert_eq!(
            clip.tracks[0].translation[1].value,
            Vec3::new(2.0, 1.0, 0.0)
        );
        assert_eq!(clip.tracks[0].rotation.len(), 1);
        assert!(clip.tracks[0].scale.is_empty());
        assert_eq!(clip.morph_tracks[0].channel, 1);
        assert_eq!(clip.morph_tracks[0].keys[0].value, 0.5);
        assert_eq!(clip.validate(4, 2), Ok(()));

        // A track range past the key table is a bridge drift.
        let bad = [ReviewImportNodeTrack {
            translation_count: 5,
            ..tracks[0]
        }];
        scene.anim_node_tracks = bad.as_ptr() as *mut ReviewImportNodeTrack;
        assert!(matches!(
            marshal_animations(&scene),
            Err(ImportError::LoadFailed(message)) if message.contains("vec3 key range")
        ));

        // No stacks at all is simply no clips.
        let empty: ReviewImportScene = unsafe { std::mem::zeroed() };
        assert!(marshal_animations(&empty).expect("no animation").is_empty());
    }

    #[test]
    fn node_kind_maps_every_bridge_code_and_falls_back() {
        assert_eq!(node_kind_from_code(1), NodeKind::Mesh);
        assert_eq!(node_kind_from_code(2), NodeKind::Bone);
        assert_eq!(node_kind_from_code(3), NodeKind::Light);
        assert_eq!(node_kind_from_code(4), NodeKind::Camera);
        assert_eq!(node_kind_from_code(5), NodeKind::Empty);
        assert_eq!(node_kind_from_code(0), NodeKind::Other);
        // An unknown code from a newer bridge must degrade, not panic.
        assert_eq!(node_kind_from_code(999), NodeKind::Other);
    }

    /// A zeroed scene carrying only geometry — the shape
    /// `model_from_bridge_scene`'s range guard reads.
    fn geometry_scene(vertices: &[ReviewImportVertex], indices: &[u32]) -> ReviewImportScene {
        let mut scene: ReviewImportScene = unsafe { std::mem::zeroed() };
        scene.vertices = vertices.as_ptr() as *mut ReviewImportVertex;
        scene.vertex_count = vertices.len();
        scene.indices = indices.as_ptr() as *mut u32;
        scene.index_count = indices.len();
        scene.source_vertex_count = vertices.len();
        scene.source_unit_meters = 1.0;
        scene
    }

    fn blank_vertices(count: usize) -> Vec<ReviewImportVertex> {
        (0..count)
            .map(|_| ReviewImportVertex {
                position: [0.0; 3],
                normal: [0.0; 3],
                uv: [0.0; 2],
                tangent: [0.0; 4],
                vertex_color: [0.0; 4],
            })
            .collect()
    }

    #[test]
    fn an_index_addressing_no_vertex_fails_the_load() {
        let vertices = blank_vertices(3);
        let scene = geometry_scene(&vertices, &[0, 1, 3]);
        assert!(matches!(
            model_from_bridge_scene(Path::new("mesh.fbx"), &scene, &|_| {}),
            Err(ImportError::LoadFailed(message))
                if message.contains("index 3") && message.contains("3 vertices")
        ));
    }

    #[test]
    fn indices_within_the_vertex_buffer_load() {
        let vertices = blank_vertices(3);
        let scene = geometry_scene(&vertices, &[0, 1, 2]);
        let model = model_from_bridge_scene(Path::new("mesh.fbx"), &scene, &|_| {})
            .expect("a whole triangle");
        assert_eq!(model.indices, vec![0, 1, 2]);
        assert_eq!(model.name, "mesh");
    }

    #[test]
    fn build_uv_channels_rejects_overflowing_counts() {
        // Malicious/corrupt counts whose product wraps `usize` must be a clean
        // error, not a wrapped guard followed by an index panic.
        let uvs = [0.0; 4];
        let scene = uv_scene(2, usize::MAX / 2 + 1, &uvs);
        assert!(matches!(
            build_uv_channels(&scene),
            Err(ImportError::LoadFailed(_))
        ));
    }
}
