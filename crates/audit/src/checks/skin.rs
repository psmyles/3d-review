//! Skinning: influence counts and weight sums per vertex, bones per mesh,
//! bones nothing uses, and bind poses that disagree with the skeleton.
//!
//! The viewer renormalizes weights when it draws, so an unnormalized rig looks
//! fine on screen — which is exactly why these are counted rather than seen.

use std::collections::{BTreeMap, BTreeSet};

use review_model::NodeKind;

use super::{Outcome, count_param, group_by_node, largest};
use crate::context::Context;
use crate::finding::{ElementSet, Measured, Offender, Skip, Threshold};
use crate::profile::RuleConfig;

/// Influences lighter than this do not count toward the influence limit — an
/// engine's import drops them.
const INFLUENCE_EPSILON: f32 = 1.0e-4;

/// Per offending logical vertex, its owning node and representative corner,
/// in node order, for every vertex `offends` accepts.
fn vertex_offenders(ctx: &Context<'_>, offends: impl Fn(usize) -> bool) -> Vec<Offender> {
    let representative = ctx.logical_corner();
    let corner_node = ctx.corner_node();
    let mut items: Vec<(u32, u32)> = representative
        .iter()
        .enumerate()
        .filter(|&(logical, &corner)| corner != u32::MAX && offends(logical))
        .map(|(_, &corner)| {
            let node = corner_node
                .get(corner as usize)
                .copied()
                .unwrap_or(u32::MAX);
            (node, corner)
        })
        .collect();
    items.sort_unstable();
    group_by_node(items, ElementSet::Vertices)
}

pub(super) fn influences(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let Some(skin) = &ctx.model.skin else {
        return Outcome::skip(Skip::NoSkin);
    };
    let max = count_param(config, "max", 4) as usize;
    let count = |logical: usize| {
        skin.weights[skin.influence_range(logical)]
            .iter()
            .filter(|&&weight| weight > INFLUENCE_EPSILON)
            .count()
    };
    let mut offenders = vertex_offenders(ctx, |logical| count(logical) > max);
    let logical = ctx.logical();
    for offender in &mut offenders {
        if let ElementSet::Vertices(corners) = &offender.elements {
            let worst = corners
                .iter()
                .filter_map(|&corner| logical.get(corner as usize))
                .map(|&id| count(id as usize))
                .max()
                .unwrap_or(0);
            offender.measured = Some(Measured::Count(worst as u64));
        }
    }
    let measured = largest(&offenders, |m| match m {
        Measured::Count(c) => Some(*c as f64),
        _ => None,
    });
    Outcome::judged(offenders, Threshold::Max(Measured::Count(max as u64))).with_measured(measured)
}

pub(super) fn unnormalized_weights(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    let Some(skin) = &ctx.model.skin else {
        return Outcome::skip(Skip::NoSkin);
    };
    let tolerance = config.number("tolerance").unwrap_or(0.01) as f32;
    let offenders = vertex_offenders(ctx, |logical| {
        let range = skin.influence_range(logical);
        !range.is_empty() && (skin.weights[range].iter().sum::<f32>() - 1.0).abs() > tolerance
    });
    Outcome::judged(
        offenders,
        Threshold::Max(Measured::Ratio(f64::from(tolerance))),
    )
}

/// The bones that actually move each skinned mesh node's vertices.
fn bones_by_mesh(ctx: &Context<'_>) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut bones: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let Some(skin) = &ctx.model.skin else {
        return bones;
    };
    for (index, &weight) in skin.weights.iter().enumerate() {
        if weight <= 0.0 {
            continue;
        }
        if let Some(cluster) = skin
            .influence_cluster
            .get(index)
            .and_then(|&cluster| skin.clusters.get(cluster as usize))
        {
            bones
                .entry(cluster.mesh_node)
                .or_default()
                .insert(cluster.bone);
        }
    }
    bones
}

pub(super) fn bones_per_mesh(ctx: &Context<'_>, config: &RuleConfig) -> Outcome {
    if ctx.model.skin.is_none() {
        return Outcome::skip(Skip::NoSkin);
    }
    let max = count_param(config, "max", 256);
    let offenders: Vec<Offender> = bones_by_mesh(ctx)
        .into_iter()
        .filter(|(_, bones)| bones.len() as u64 > max)
        .map(|(mesh, bones)| {
            Offender::node(mesh as usize, Some(Measured::Count(bones.len() as u64)))
        })
        .collect();
    let measured = largest(&offenders, |m| match m {
        Measured::Count(c) => Some(*c as f64),
        _ => None,
    });
    Outcome::judged(offenders, Threshold::Max(Measured::Count(max))).with_measured(measured)
}

/// Bones that carry no weight and have no weighted bone below them — dead
/// joints an engine still has to evaluate every frame.
pub(super) fn unused_bones(ctx: &Context<'_>) -> Outcome {
    if ctx.model.skin.is_none() {
        return Outcome::skip(Skip::NoSkin);
    }
    let model = ctx.model;
    let used: BTreeSet<u32> = bones_by_mesh(ctx).into_values().flatten().collect();
    // A bone is needed when it, or anything below it, is used.
    let mut needed = vec![false; model.nodes.len()];
    for &bone in &used {
        let mut cursor = Some(bone as usize);
        let mut steps = 0;
        while let Some(index) = cursor {
            if needed[index] || steps > model.nodes.len() {
                break;
            }
            needed[index] = true;
            cursor = model.nodes[index].parent;
            steps += 1;
        }
    }
    let offenders = (0..model.nodes.len())
        .filter(|&index| model.nodes[index].kind == NodeKind::Bone && !needed[index])
        .map(|index| Offender {
            node: Some(index as u32),
            count: 1,
            measured: None,
            detail: None,
            elements: ElementSet::Points(vec![(ctx.node_position(index).to_array(), index as u32)]),
        })
        .collect();
    Outcome::judged(offenders, Threshold::None)
}

/// Bones whose bind pose (where the mesh was skinned) differs from where the
/// skeleton actually stands at rest. Engines that rebuild the bind pose from
/// the skeleton then deform the mesh on import.
pub(super) fn bind_pose_mismatch(ctx: &Context<'_>) -> Outcome {
    let Some(skin) = &ctx.model.skin else {
        return Outcome::skip(Skip::NoSkin);
    };
    let model = ctx.model;
    let size = model
        .bounds
        .map_or(1.0, |bounds| bounds.radius() * 2.0)
        .max(1.0e-6);
    let mut worst: BTreeMap<u32, f32> = BTreeMap::new();
    for cluster in &skin.clusters {
        let Some(bone) = model.nodes.get(cluster.bone as usize) else {
            continue;
        };
        let (_, bind_rotation, bind_position) =
            cluster.bind_to_world.to_scale_rotation_translation();
        let (_, rest_rotation, rest_position) = bone.transform.to_scale_rotation_translation();
        let distance = bind_position.distance(rest_position);
        let angle = bind_rotation.angle_between(rest_rotation).to_degrees();
        if distance > 1.0e-3 * size || angle > 0.1 {
            let entry = worst.entry(cluster.bone).or_insert(0.0);
            *entry = entry.max(distance);
        }
    }
    let offenders = worst
        .into_iter()
        .map(|(bone, distance)| Offender {
            node: Some(bone),
            count: 1,
            measured: Some(Measured::Meters(f64::from(distance))),
            detail: None,
            elements: ElementSet::Points(vec![(ctx.node_position(bone as usize).to_array(), bone)]),
        })
        .collect();
    Outcome::judged(offenders, Threshold::None)
}
