//! Skins, blend shapes and bind poses.
//!
//! A skin cluster's file `Transform` is `mesh_node_to_bone` verbatim, while
//! `TransformLink` is `bind_to_world` and is the only one in scene units — which
//! is why one is scaled by `per_meter` here and the other is left alone. Getting
//! either wrong passes every bounds round trip; the skinned pose is what breaks.

use std::collections::HashMap;

use glam::{Mat3, Mat4};
use review_model::{ModelData, SourceExtras};

use crate::notice::ExportNote;
use crate::process::ProcessedLod;
use crate::stack::HierarchyMode;

use super::*;

// ---------------------------------------------------------------------------
// Skins, blend shapes, poses
// ---------------------------------------------------------------------------

/// One shape's sparse offsets over a mesh: `(vertices, xyz offsets, xyz normal deltas)`.
#[derive(Clone, Default)]
pub(crate) struct ShapeOffsets(Vec<i32>, Vec<f64>, Vec<f64>);

/// `ufbxw_skinning_type` for a source skinning method.
pub(crate) fn skinning_type_code(method: review_model::SkinningMethod) -> u32 {
    match method {
        review_model::SkinningMethod::Rigid => 0,
        review_model::SkinningMethod::Linear => 1,
        review_model::SkinningMethod::DualQuaternion => 2,
        review_model::SkinningMethod::BlendedDqLinear => 3,
    }
}

/// The scene node a source node was placed at, placing its ancestor chain on
/// the computed path if nothing has yet (bones are rarely mesh ancestors).
pub(crate) fn placed_or_place(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    node: usize,
    extras: Option<&SourceExtras>,
) -> Option<i32> {
    if let Some(&existing) = placed.get(&node) {
        return Some(existing);
    }
    match extras {
        Some(extras) => emit_authored_node(scene, placed, source, extras, node, 0),
        None => {
            let group = NodeGroup {
                source_node: Some(node),
                triangles: Vec::new(),
            };
            let (index, _) = place_node(scene, placed, source, &group, HierarchyMode::Rebuild, 0);
            (index >= 0).then_some(index)
        }
    }
}

/// The authored bind poses, placed. Only the file's *bind* poses have a writer
/// (a rest pose is a different element the vendored writer does not emit).
pub(crate) fn build_poses(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    report: &mut ExportReport,
) {
    let mut skipped = 0usize;
    for pose in &extras.poses {
        if !pose.is_bind_pose {
            skipped += 1;
            continue;
        }
        let per_meter = scene.unit.per_meter;
        let mut nodes = Vec::with_capacity(pose.entries.len());
        for entry in &pose.entries {
            let Some(node) =
                placed_or_place(scene, placed, source, entry.node as usize, Some(extras))
            else {
                continue;
            };
            nodes.push((
                node,
                world_matrix_in_file_units(entry.bone_to_world, per_meter),
            ));
        }
        scene.poses.push(PoseData {
            name: c_string_or_empty(&pose.name),
            nodes,
        });
    }
    if skipped > 0 {
        report
            .notes
            .push(ExportNote::NonBindPosesSkipped { count: skipped });
    }
}

/// The skins and blend shapes of one exported mesh, from the level's own deform
/// tables (whose logical vertices are the level's vertices) and the carry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_deform(
    scene: &mut SceneData,
    mesh: &mut MeshData,
    placed: &mut HashMap<usize, i32>,
    lod: &ProcessedLod,
    group: &NodeGroup,
    source: &ModelData,
    extras: Option<&SourceExtras>,
    report: &mut ExportReport,
) {
    let Some(node_index) = group.source_node else {
        return;
    };
    let level = &lod.model;
    let has_skin = level.skin.is_some()
        || lod
            .carry
            .extra_skins
            .iter()
            .any(|layer| layer.node as usize == node_index);
    let has_morph = level.morph.as_ref().is_some_and(|morph| {
        morph
            .channels
            .iter()
            .any(|channel| channel.mesh_node as usize == node_index)
    });
    if !has_skin && !has_morph {
        return;
    }
    let Some(node) = source.nodes.get(node_index) else {
        return;
    };
    let per_meter = scene.unit.per_meter;
    // Level vertex → this mesh's local vertex.
    let mut local_of_level: HashMap<u32, i32> = HashMap::with_capacity(mesh.level_vertices.len());
    for (local, &level_vertex) in mesh.level_vertices.iter().enumerate() {
        local_of_level.insert(level_vertex, local as i32);
    }
    // The skin and blend-shape tables are per *logical* vertex, which is the
    // level vertex itself in the ordinary layout and the indexed vertex a run of
    // corners was expanded from in the corner-run one. `corner_to_logical` is the
    // identity in the first case, so this is the same lookup either way.
    let logical_of = |level_vertex: u32| -> usize {
        level
            .corner_to_logical
            .get(level_vertex as usize)
            .map_or(level_vertex as usize, |&logical| logical as usize)
    };
    // Deltas move back into geometry space like the positions did.
    let geometry_to_node = extras
        .and_then(|extras| extras.nodes.get(node_index))
        .map_or(Mat4::IDENTITY, |authored| authored.geometry_to_node);
    let world = node.transform * geometry_to_node;
    let to_local = world.inverse();
    let linear = Mat3::from_mat4(world);
    let determinant = linear.determinant().abs();

    // ---- Skins
    if let Some(skin) = &level.skin {
        let mut clusters: Vec<ClusterData> = Vec::new();
        let mut cluster_slot: HashMap<u32, usize> = HashMap::new();
        let mut unplaced = 0usize;
        for (index, cluster) in skin.clusters.iter().enumerate() {
            if cluster.mesh_node as usize != node_index {
                continue;
            }
            let Some(bone) = placed_or_place(scene, placed, source, cluster.bone as usize, extras)
            else {
                unplaced += 1;
                continue;
            };
            cluster_slot.insert(index as u32, clusters.len());
            clusters.push(ClusterData {
                bone,
                name: c_string_or_empty(&cluster.name),
                // `Transform` is authored in the file's units and read verbatim;
                // `TransformLink` is a world matrix ufbx normalized to meters.
                transform: cluster.mesh_node_to_bone.as_dmat4().to_cols_array(),
                transform_link: world_matrix_in_file_units(cluster.bind_to_world, per_meter),
                vertices: Vec::new(),
                weights: Vec::new(),
            });
        }
        if unplaced > 0 {
            report.notes.push(ExportNote::UnplacedClusters {
                mesh: node.name.clone(),
                count: unplaced,
            });
        }
        if !clusters.is_empty() {
            for (local, &level_vertex) in mesh.level_vertices.iter().enumerate() {
                let range = skin.influence_range(logical_of(level_vertex));
                for (&cluster, &weight) in skin.influence_cluster[range.clone()]
                    .iter()
                    .zip(&skin.weights[range])
                {
                    if let Some(&slot) = cluster_slot.get(&cluster) {
                        clusters[slot].vertices.push(local as i32);
                        clusters[slot].weights.push(f64::from(weight));
                    }
                }
            }
            let method = skin
                .deformers
                .iter()
                .find(|deformer| deformer.mesh_node as usize == node_index)
                .map_or(review_model::SkinningMethod::Linear, |deformer| {
                    deformer.method
                });
            let mut dq_vertices = Vec::new();
            let mut dq_weights = Vec::new();
            for &(level_vertex, weight) in &lod.carry.dq_weights {
                if let Some(&local) = local_of_level.get(&level_vertex) {
                    dq_vertices.push(local);
                    dq_weights.push(f64::from(weight));
                }
            }
            let bind_pose = scene
                .poses
                .iter()
                .position(|pose| {
                    pose.nodes
                        .iter()
                        .any(|(placed_node, _)| *placed_node == mesh.node)
                })
                .map_or(-1, |index| index as i32);
            mesh.skins.push(SkinExportData {
                skinning_type: skinning_type_code(method),
                clusters,
                dq_vertices,
                dq_weights,
                bind_pose,
            });
        }
    }
    // Further skin layers, from the carry and the capture's cluster tables.
    if let Some(part) = extras.and_then(|extras| extras.mesh_of_node(node_index as u32)) {
        for layer in lod
            .carry
            .extra_skins
            .iter()
            .filter(|layer| layer.node as usize == node_index)
        {
            let Some(authored) = part.extra_skins.get(layer.layer) else {
                continue;
            };
            let mut clusters: Vec<ClusterData> = Vec::new();
            let mut slot_of: Vec<Option<usize>> = Vec::with_capacity(authored.clusters.len());
            for cluster in &authored.clusters {
                let placed_bone =
                    placed_or_place(scene, placed, source, cluster.bone as usize, extras);
                slot_of.push(placed_bone.map(|bone| {
                    clusters.push(ClusterData {
                        bone,
                        name: c_string_or_empty(&cluster.name),
                        transform: cluster.mesh_node_to_bone.as_dmat4().to_cols_array(),
                        transform_link: world_matrix_in_file_units(
                            cluster.bind_to_world,
                            per_meter,
                        ),
                        vertices: Vec::new(),
                        weights: Vec::new(),
                    });
                    clusters.len() - 1
                }));
            }
            for &(level_vertex, cluster, weight) in &layer.influences {
                let (Some(&local), Some(Some(slot))) = (
                    local_of_level.get(&level_vertex),
                    slot_of.get(cluster as usize),
                ) else {
                    continue;
                };
                clusters[*slot].vertices.push(local);
                clusters[*slot].weights.push(f64::from(weight));
            }
            mesh.skins.push(SkinExportData {
                skinning_type: skinning_type_code(authored.method),
                clusters,
                dq_vertices: Vec::new(),
                dq_weights: Vec::new(),
                bind_pose: -1,
            });
        }
    }

    // ---- Blend shapes
    if let Some(morph) = &level.morph {
        // Every shape's offsets over this mesh, gathered in one walk.
        let mut by_shape: HashMap<u32, ShapeOffsets> = HashMap::new();
        for (local, &level_vertex) in mesh.level_vertices.iter().enumerate() {
            let logical = logical_of(level_vertex);
            let range = match (morph.offsets.get(logical), morph.offsets.get(logical + 1)) {
                (Some(&start), Some(&end)) if end >= start => start as usize..end as usize,
                _ => 0..0,
            };
            for index in range {
                let entry = by_shape.entry(morph.shape[index]).or_default();
                let position = to_local.transform_vector3(morph.position[index]);
                // The import rotated normal deltas by the cofactor matrix
                // (|det|·L⁻ᵀ); undoing it is Lᵀ / |det|, and the result stays
                // unnormalized as the file authored it.
                let normal = if determinant > 1e-12 {
                    linear.transpose() * morph.normal[index] / determinant
                } else {
                    morph.normal[index]
                };
                entry.0.push(local as i32);
                entry.1.extend_from_slice(&[
                    f64::from(position.x),
                    f64::from(position.y),
                    f64::from(position.z),
                ]);
                entry.2.extend_from_slice(&[
                    f64::from(normal.x),
                    f64::from(normal.y),
                    f64::from(normal.z),
                ]);
            }
        }
        for (channel_index, channel) in morph
            .channels
            .iter()
            .enumerate()
            .filter(|(_, channel)| channel.mesh_node as usize == node_index)
        {
            mesh.channel_sources.push(channel_index as u32);
            let shapes = channel
                .keyframes
                .iter()
                .map(|key| {
                    let ShapeOffsets(vertices, offsets, normals) =
                        by_shape.get(&key.shape).cloned().unwrap_or_default();
                    let has_normals = normals.iter().any(|&value| value != 0.0);
                    BlendShapeData {
                        name: c_string_or_empty(
                            morph
                                .shapes
                                .get(key.shape as usize)
                                .map_or("", |shape| shape.name.as_str()),
                        ),
                        vertices,
                        offsets,
                        normals: if has_normals { normals } else { Vec::new() },
                        target_weight: f64::from(key.target_weight) * 100.0,
                    }
                })
                .collect();
            mesh.blend_channels.push(BlendChannelData {
                name: c_string_or_empty(&channel.name),
                weight: f64::from(channel.rest_weight) * 100.0,
                shapes,
            });
        }
    }
}
