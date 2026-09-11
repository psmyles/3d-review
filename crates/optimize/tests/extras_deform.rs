//! Round trips of the source-property capture: the rig.
//!
//! A skin's authored cluster matrices, its bind poses, the second and later skin
//! deformers, dual-quaternion weights and blend shapes. These are the parts a
//! bounds check cannot see: the geometry round-trips either way, and it is the
//! posed mesh that breaks, so the matrices are compared numerically.

#![cfg(all(has_meshopt, has_ufbxw))]

use review_model::ModelData;
use review_optimize::{
    FbxFormat, LodLevel, OpKind, OptStack, ReduceParams, SimplifySettings, WeldParams,
};

mod common;

use common::extras::{assert_matrix_close, export_result, node_by_name, round_trip};
use common::{fixture_full as fixture, load_full as load, run_with_extras as run, temp_dir};

/// A cluster identified by the names of its bone and its mesh node.
fn cluster_key(model: &ModelData, cluster: &review_model::SkinCluster) -> (String, String) {
    (
        model.nodes[cluster.bone as usize].name.clone(),
        model.nodes[cluster.mesh_node as usize].name.clone(),
    )
}

/// The character's skin comes back with the same clusters — matched by bone and
/// mesh — the same authored matrices, and the same rest pose, through a
/// passthrough and a halving LOD alike.
#[test]
fn skin_survives_with_its_authored_cluster_matrices() {
    let Some((model, extras)) = fixture("SK_Player_01.fbx") else {
        return;
    };
    let source_skin = model.skin.as_ref().expect("the fixture is skinned");
    let dir = temp_dir("extras_skin");

    let mut stacks: Vec<(&str, OptStack)> = Vec::new();
    let mut stack = OptStack::default();
    stack.push_op(OpKind::FilterTriangles);
    stacks.push(("passthrough", stack));
    let mut stack = OptStack::default();
    stack.push_op(OpKind::Weld(WeldParams::default()));
    stack.push_op(OpKind::Reduce(ReduceParams {
        simplify: SimplifySettings::default(),
        target: LodLevel {
            target_ratio: 0.5,
            target_error: 0.05,
        },
    }));
    stacks.push(("halved", stack));

    for (label, stack) in &stacks {
        let result = run(&model, &extras, stack);
        assert!(
            result.lods[0].model.skin.is_some(),
            "{label}: the level keeps its skin"
        );
        let (loaded, loaded_extras) =
            export_result(&result, &model, &extras, &dir.join(format!("{label}.fbx")));
        let skin = loaded
            .skin
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: skin written"));
        assert_eq!(
            skin.clusters.len(),
            source_skin.clusters.len(),
            "{label}: cluster count"
        );
        assert_eq!(
            skin.deformers.len(),
            source_skin.deformers.len(),
            "{label}: deformer count"
        );
        for cluster in &source_skin.clusters {
            let key = cluster_key(&model, cluster);
            let back = skin
                .clusters
                .iter()
                .find(|candidate| cluster_key(&loaded, candidate) == key)
                .unwrap_or_else(|| panic!("{label}: cluster {key:?} was not written"));
            let context = format!("{label} {key:?}");
            assert_matrix_close(
                back.mesh_node_to_bone,
                cluster.mesh_node_to_bone,
                1e-3,
                &context,
            );
            assert_matrix_close(back.bind_to_world, cluster.bind_to_world, 1e-4, &context);
            assert_matrix_close(
                back.world_to_bone_bind,
                cluster.world_to_bone_bind,
                1e-3,
                &context,
            );
        }
        for deformer in &source_skin.deformers {
            let node = &model.nodes[deformer.mesh_node as usize].name;
            let back = skin
                .deformers
                .iter()
                .find(|candidate| &loaded.nodes[candidate.mesh_node as usize].name == node)
                .unwrap_or_else(|| panic!("{label}: deformer of {node:?} was not written"));
            assert_eq!(
                back.method, deformer.method,
                "{label}: {node} skinning method"
            );
        }
        // Every vertex is skinned, and the skinned rest pose sits where the
        // source's does.
        assert!(
            (0..loaded.vertices.len()).all(|corner| {
                let logical = loaded.corner_to_logical[corner] as usize;
                !skin.influence_range(logical).is_empty()
            }),
            "{label}: every vertex has an influence"
        );
        if *label == "passthrough" {
            let source_bounds = model.bounds.expect("bounds");
            let back_bounds = loaded.bounds.expect("bounds");
            assert!(
                source_bounds.min.distance(back_bounds.min) < 1e-3,
                "{label}: rest bounds min"
            );
            assert!(
                source_bounds.max.distance(back_bounds.max) < 1e-3,
                "{label}: rest bounds max"
            );
        }
        assert!(
            !loaded_extras.meshes.is_empty(),
            "{label}: the capture reads the mesh parts back"
        );
    }
}

/// Authored bind poses come back node for node.
#[test]
fn bind_poses_survive() {
    let Some((model, extras)) = fixture("AN_ZombiedogLocomotion.fbx") else {
        return;
    };
    let bind_poses: Vec<_> = extras
        .poses
        .iter()
        .filter(|pose| pose.is_bind_pose)
        .collect();
    assert!(!bind_poses.is_empty(), "the fixture carries bind poses");
    let dir = temp_dir("extras_poses");
    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    let back_poses: Vec<_> = loaded_extras
        .poses
        .iter()
        .filter(|pose| pose.is_bind_pose)
        .collect();
    assert_eq!(back_poses.len(), bind_poses.len());
    for (pose, back) in bind_poses.iter().zip(&back_poses) {
        assert_eq!(pose.entries.len(), back.entries.len(), "{}", pose.name);
        for entry in &pose.entries {
            let node = &model.nodes[entry.node as usize].name;
            let matched = back
                .entries
                .iter()
                .find(|candidate| &loaded.nodes[candidate.node as usize].name == node)
                .unwrap_or_else(|| panic!("pose entry for {node:?} was not written"));
            assert_matrix_close(matched.bone_to_world, entry.bone_to_world, 1e-3, node);
        }
    }
}

/// The probe's synthetic deform data: a blend-type skin with dual-quaternion
/// weights, a second skin layer, and a blend shape with normal deltas.
#[cfg(has_ufbxw_probe)]
#[test]
fn dual_quaternion_weights_skin_layers_and_blend_shapes_survive() {
    let dir = temp_dir("extras_deform");
    let probe_path = dir.join("probe.fbx");
    review_optimize::probe::write_patch_probe(&probe_path, false).expect("probe written");
    let Some((model, extras)) = load(&probe_path) else {
        return;
    };
    let quad = node_by_name(&model, "Quad").unwrap().0 as u32;
    let source_part = extras.mesh_of_node(quad).unwrap();
    assert_eq!(source_part.dq_weights.len(), 2);
    assert_eq!(source_part.extra_skins.len(), 1);
    let source_morph = model.morph.as_ref().expect("the probe has a blend shape");

    let (_, loaded, loaded_extras) = round_trip(&model, &extras, &dir, FbxFormat::Binary);
    let back_quad = node_by_name(&loaded, "Quad").unwrap().0 as u32;
    let back_part = loaded_extras.mesh_of_node(back_quad).unwrap();

    // Positions are the stable identity across the export's vertex split.
    let position_of = |model: &ModelData, logical: u32, part: &review_model::extras::MeshExtras| {
        let corner = model
            .corner_to_logical
            .iter()
            .position(|&candidate| candidate == logical)
            .expect("logical vertex has a corner");
        let _ = part;
        (model.vertices[corner].position * 1000.0).round()
    };

    // The blend-type skin and its dual-quaternion weights.
    let skin = loaded.skin.as_ref().expect("skin written");
    assert_eq!(
        skin.deformers[0].method,
        review_model::SkinningMethod::BlendedDqLinear
    );
    let mut dq: Vec<(glam::Vec3, f32)> = back_part
        .dq_weights
        .iter()
        .map(|&(vertex, weight)| (position_of(&loaded, vertex, back_part), weight))
        .collect();
    dq.sort_by(|a, b| a.0.to_array().partial_cmp(&b.0.to_array()).unwrap());
    dq.dedup();
    let mut source_dq: Vec<(glam::Vec3, f32)> = source_part
        .dq_weights
        .iter()
        .map(|&(vertex, weight)| (position_of(&model, vertex, source_part), weight))
        .collect();
    source_dq.sort_by(|a, b| a.0.to_array().partial_cmp(&b.0.to_array()).unwrap());
    assert_eq!(dq, source_dq);

    // The second skin layer with its one weight.
    assert_eq!(back_part.extra_skins.len(), 1);
    let layer = &back_part.extra_skins[0];
    assert_eq!(layer.clusters.len(), 1);
    assert_eq!(layer.clusters[0].name, "BoneLayer");
    let weighted: Vec<f32> = layer.influences.iter().map(|&(_, weight)| weight).collect();
    assert!(
        weighted.iter().all(|&weight| (weight - 0.25).abs() < 1e-6),
        "{weighted:?}"
    );
    assert!(!weighted.is_empty());

    // The blend shape: same channel, same rest weight, same offsets by position.
    let morph = loaded.morph.as_ref().expect("blend shape written");
    assert_eq!(morph.channels.len(), 1);
    assert_eq!(morph.channels[0].name, "Smile");
    assert!((morph.channels[0].rest_weight - 0.25).abs() < 1e-6);
    assert!((morph.channels[0].keyframes[0].target_weight - 1.0).abs() < 1e-6);
    let offsets_by_position = |model: &ModelData, morph: &review_model::MorphData| {
        let mut out: Vec<([i32; 3], [i32; 3], [i32; 3])> = Vec::new();
        for (corner, &logical) in model.corner_to_logical.iter().enumerate() {
            let range = morph.offsets[logical as usize] as usize
                ..morph.offsets[logical as usize + 1] as usize;
            for index in range {
                let quantize = |v: glam::Vec3| {
                    let q = (v * 1000.0).round();
                    [q.x as i32, q.y as i32, q.z as i32]
                };
                out.push((
                    quantize(model.vertices[corner].position),
                    quantize(morph.position[index]),
                    quantize(morph.normal[index]),
                ));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    };
    assert_eq!(
        offsets_by_position(&loaded, morph),
        offsets_by_position(&model, source_morph)
    );
}
