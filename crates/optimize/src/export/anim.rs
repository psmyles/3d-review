//! The authored animation curves, and the layers and sets over them.
//!
//! A curve node gets only the curves the source actually had: a reader treats
//! *any* curve, even an empty one, as a non-constant value, and ufbx then adds a
//! scale helper under every bone whose `Lcl Scaling` was a default-only curve
//! node.

use std::collections::HashMap;
use std::ffi::CString;

use review_model::{ModelData, SourceExtras};

use super::*;

// ---------------------------------------------------------------------------
// Animation, display layers, selection sets
// ---------------------------------------------------------------------------

/// FBX ktime units per second (`UFBXW_KTIME_SECOND`).
pub(crate) const KTIME_SECOND: f64 = 46_186_158_000.0;

/// `ufbxw_keyframe_flags` bits.
pub(crate) const KEY_CONSTANT: u32 = 0x1;

pub(crate) const KEY_CONSTANT_NEXT: u32 = 0x2;

pub(crate) const KEY_LINEAR: u32 = 0x4;

pub(crate) const KEY_CUBIC: u32 = 0x8;

pub(crate) const KEY_TANGENT_USER: u32 = 0x100;

pub(crate) const KEY_TANGENT_BROKEN: u32 = 0x200;

pub(crate) const KEY_WEIGHTED_LEFT: u32 = 0x1000;

pub(crate) const KEY_WEIGHTED_RIGHT: u32 = 0x2000;

/// `RVO_TARGET_*`.
pub(crate) const TARGET_NODE: u32 = 0;

pub(crate) const TARGET_NODE_ATTRIBUTE: u32 = 1;

pub(crate) const TARGET_MATERIAL: u32 = 2;

pub(crate) const TARGET_TEXTURE: u32 = 3;

pub(crate) const TARGET_VIDEO: u32 = 4;

pub(crate) const TARGET_BLEND_CHANNEL: u32 = 5;

pub(crate) const TARGET_DISPLAY_LAYER: u32 = 6;

pub(crate) const TARGET_ANIM_LAYER: u32 = 7;

pub(crate) fn ktime(seconds: f64) -> i64 {
    (seconds * KTIME_SECOND).round() as i64
}

/// One authored curve in the writer's terms. The reader expanded each key's
/// tangents to `(dx, dy)` handles as `dx = weight × interval`, `dy = dx ×
/// slope`; this is the exact inverse, per key against its neighbours.
pub(crate) fn curve_data(curve: &review_model::extras::Curve) -> CurveData {
    let keys = &curve.keys;
    let mut out = Vec::with_capacity(keys.len());
    for (index, key) in keys.iter().enumerate() {
        let mut flags = match key.interpolation {
            review_model::extras::Interpolation::ConstantPrev => KEY_CONSTANT,
            review_model::extras::Interpolation::ConstantNext => KEY_CONSTANT_NEXT,
            review_model::extras::Interpolation::Linear => KEY_LINEAR,
            review_model::extras::Interpolation::Cubic
            | review_model::extras::Interpolation::Unnamed(_) => KEY_CUBIC,
        };
        let mut weight_left = 1.0 / 3.0;
        let mut weight_right = 1.0 / 3.0;
        let mut slope_left = 0.0;
        let mut slope_right = 0.0;
        if flags == KEY_CUBIC {
            flags |= KEY_TANGENT_USER | KEY_TANGENT_BROKEN | KEY_WEIGHTED_LEFT | KEY_WEIGHTED_RIGHT;
            let (dx_left, dy_left) = (f64::from(key.left.0), f64::from(key.left.1));
            let (dx_right, dy_right) = (f64::from(key.right.0), f64::from(key.right.1));
            if index > 0 {
                let interval = key.time - keys[index - 1].time;
                if interval > 0.0 && dx_left > 0.0 {
                    weight_left = dx_left / interval;
                    slope_left = dy_left / dx_left;
                }
            }
            if index + 1 < keys.len() {
                let interval = keys[index + 1].time - key.time;
                if interval > 0.0 && dx_right > 0.0 {
                    weight_right = dx_right / interval;
                    slope_right = dy_right / dx_right;
                }
            }
        }
        out.push(KeyData {
            time: ktime(key.time),
            value: key.value,
            flags,
            weight_left,
            weight_right,
            slope_left,
            slope_right,
        });
    }
    CurveData {
        keys: out,
        pre_mode: curve.pre.mode.code(),
        pre_repeat: curve.pre.repeat_count,
        post_mode: curve.post.mode.code(),
        post_repeat: curve.post.repeat_count,
    }
}

/// The authored animation: every stack, every layer under the first stack
/// that lists it, and every animated property resolved onto the elements
/// written above.
pub(crate) fn build_animation(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    extras: &SourceExtras,
    report: &mut ExportReport,
) {
    if extras.animations.is_empty() {
        return;
    }
    for stack in &extras.animations {
        let props = scene.push_props(&stack.props);
        scene.anim_stacks.push(AnimStackData {
            name: c_string_or_empty(&stack.name),
            props,
            time_begin: ktime(stack.time_begin),
            time_end: ktime(stack.time_end),
        });
    }
    scene.active_stack = 0;

    // Capture layer index → exported layer index (a layer no stack lists has
    // no place in the file).
    let mut layer_map: Vec<i32> = vec![-1; extras.anim_layers.len()];
    let mut orphaned = 0usize;
    for (index, _) in extras.anim_layers.iter().enumerate() {
        let stack = extras
            .animations
            .iter()
            .position(|stack| stack.layers.iter().any(|&layer| layer as usize == index));
        match stack {
            Some(stack) => {
                layer_map[index] = scene.anim_layers.len() as i32;
                scene.anim_layers.push(AnimLayerData {
                    name: CString::default(),
                    stack: stack as i32,
                    weight: 0.0,
                    props: PropRange::default(),
                    anim_props: Vec::new(),
                });
            }
            None => orphaned += 1,
        }
    }
    if orphaned > 0 {
        report
            .notes
            .push(ExportNote::OrphanedAnimationLayers { count: orphaned });
    }

    let mut unmapped = 0usize;
    for (index, layer) in extras.anim_layers.iter().enumerate() {
        let exported = layer_map[index];
        if exported < 0 {
            continue;
        }
        let props = scene.push_props(&layer.props);
        let mut anim_props = Vec::new();
        for anim in &layer.anim {
            let curves = [
                anim.curves[0].as_ref().map(curve_data),
                anim.curves[1].as_ref().map(curve_data),
                anim.curves[2].as_ref().map(curve_data),
            ];
            // A curve node without curves still carries its defaults, and a
            // reader bakes a track for its node — it is written as authored.
            let default = [
                f64::from(anim.default.x),
                f64::from(anim.default.y),
                f64::from(anim.default.z),
            ];
            let mut targets: Vec<(u32, i32, i32)> = Vec::new();
            match &anim.target {
                review_model::extras::ElementRef::Node(node)
                | review_model::extras::ElementRef::NodeAttribute(node) => {
                    let kind = if matches!(anim.target, review_model::extras::ElementRef::Node(_)) {
                        TARGET_NODE
                    } else {
                        TARGET_NODE_ATTRIBUTE
                    };
                    // A later level's copy of the node animates the same way.
                    targets.extend(
                        placed_and_copies(scene, placed, *node as usize)
                            .into_iter()
                            .map(|target| (kind, target, 0)),
                    );
                }
                review_model::extras::ElementRef::Material(material) => {
                    if (*material as usize) < scene.materials.len() {
                        targets.push((TARGET_MATERIAL, *material as i32, 0));
                    }
                }
                review_model::extras::ElementRef::Texture(texture) => {
                    if let Some(&mapped) = scene.texture_map.get(*texture as usize)
                        && mapped >= 0
                    {
                        targets.push((TARGET_TEXTURE, mapped, 0));
                    }
                }
                review_model::extras::ElementRef::Video(video) => {
                    if (*video as usize) < scene.videos.len() {
                        targets.push((TARGET_VIDEO, *video as i32, 0));
                    }
                }
                review_model::extras::ElementRef::BlendChannel(channel) => {
                    for (mesh_index, mesh) in scene.meshes.iter().enumerate() {
                        if let Some(slot) = mesh.channel_sources.iter().position(|&c| c == *channel)
                        {
                            targets.push((TARGET_BLEND_CHANNEL, mesh_index as i32, slot as i32));
                        }
                    }
                }
                review_model::extras::ElementRef::DisplayLayer(layer) => {
                    if (*layer as usize) < scene.display_layers.len() {
                        targets.push((TARGET_DISPLAY_LAYER, *layer as i32, 0));
                    }
                }
                review_model::extras::ElementRef::AnimLayer(layer) => {
                    if let Some(&mapped) = layer_map.get(*layer as usize)
                        && mapped >= 0
                    {
                        targets.push((TARGET_ANIM_LAYER, mapped, 0));
                    }
                }
                review_model::extras::ElementRef::Unmapped { .. } => {}
            }
            if targets.is_empty() {
                unmapped += 1;
                continue;
            }
            for (kind, target, target2) in targets {
                anim_props.push(AnimPropData {
                    target_kind: kind,
                    target,
                    target2,
                    prop_name: c_string_or_empty(&anim.prop_name),
                    default,
                    curves: curves.clone(),
                });
            }
        }
        let data = &mut scene.anim_layers[exported as usize];
        data.name = c_string_or_empty(&layer.name);
        // The capture holds ufbx's reading, the file's `Weight` ÷ 100; the
        // writer's field *is* the FBX `Weight`, a percent (template default
        // 100). Passed through unscaled, every full-strength layer went out at
        // 1 %, which a reader ignores only for the base layer.
        data.weight = layer.weight * 100.0;
        data.props = props;
        data.anim_props = anim_props;
    }
    if unmapped > 0 {
        report
            .notes
            .push(ExportNote::UnmappedAnimatedProperties { count: unmapped });
    }
}

/// Every scene node a source node went out as: the node itself, then each
/// suffixed per-level copy a chain made of it. What a layer, a set or a curve
/// that names the source node applies to — the copies are the same node at
/// another level, and animation already treated them so.
pub(crate) fn placed_and_copies(
    scene: &SceneData,
    placed: &HashMap<usize, i32>,
    node: usize,
) -> Vec<i32> {
    placed
        .get(&node)
        .copied()
        .into_iter()
        .chain(scene.level_copies.get(&node).into_iter().flatten().copied())
        .collect()
}

/// Display layers over the placed nodes, and over every level copy of them.
pub(crate) fn build_display_layers(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    extras: &SourceExtras,
) {
    for layer in &extras.display_layers {
        let props = scene.push_props(&layer.props);
        let nodes = layer
            .nodes
            .iter()
            .flat_map(|node| placed_and_copies(scene, placed, *node as usize))
            .collect();
        scene.display_layers.push(DisplayLayerData {
            name: c_string_or_empty(&layer.name),
            props,
            nodes,
        });
    }
}

/// Selection sets: node membership always — of the node and of every level copy
/// of it, as display layers and animation have it; vertex / edge / face members
/// mapped onto the mesh each of those nodes carries, where its polygons survived.
pub(crate) fn build_selection_sets(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    report: &mut ExportReport,
) {
    let mut components_dropped = 0usize;
    for set in &extras.selection_sets {
        let props = scene.push_props(&set.props);
        let mut nodes = Vec::new();
        for entry in &set.nodes {
            let Some(node_index) = entry.node else {
                continue;
            };
            for node in placed_and_copies(scene, placed, node_index as usize) {
                let (vertices, edges, faces, lost) =
                    selection_components(scene, source, entry, node);
                components_dropped += usize::from(lost);
                nodes.push(SelectionNodeData {
                    node,
                    include_node: entry.include_node,
                    vertices,
                    edges,
                    faces,
                });
            }
        }
        scene.selection_sets.push(SelectionSetData {
            name: c_string_or_empty(&set.name),
            props,
            nodes,
        });
    }
    if components_dropped > 0 {
        report.notes.push(ExportNote::SelectionMembersLost {
            count: components_dropped,
        });
    }
}

/// One selection entry's vertex / edge / face members on the mesh scene node
/// `node` carries, and whether any of them could not be found there.
fn selection_components(
    scene: &SceneData,
    source: &ModelData,
    entry: &review_model::extras::SelectionNodeExtras,
    node: i32,
) -> (Vec<i32>, Vec<i32>, Vec<i32>, bool) {
    let has_components =
        !entry.vertices.is_empty() || !entry.edges.is_empty() || !entry.faces.is_empty();
    if !has_components {
        return (Vec::new(), Vec::new(), Vec::new(), false);
    }
    let Some(mesh) = scene.meshes.iter().find(|mesh| mesh.node == node) else {
        return (Vec::new(), Vec::new(), Vec::new(), true);
    };
    let wanted_vertices: std::collections::HashSet<u32> = entry.vertices.iter().copied().collect();
    let wanted_edges: std::collections::HashSet<u32> = entry.edges.iter().copied().collect();
    let wanted_faces: std::collections::HashSet<u32> = entry.faces.iter().copied().collect();
    let vertices: Vec<i32> = mesh
        .source_corners
        .iter()
        .enumerate()
        .filter(|&(_, &corner)| {
            corner != u32::MAX
                && source
                    .corner_to_logical
                    .get(corner as usize)
                    .is_some_and(|logical| wanted_vertices.contains(logical))
        })
        .map(|(local, _)| local as i32)
        .collect();
    let edges: Vec<i32> = mesh
        .edge_sources
        .iter()
        .enumerate()
        .filter(|&(_, &edge)| edge != u32::MAX && wanted_edges.contains(&edge))
        .map(|(index, _)| index as i32)
        .collect();
    let faces: Vec<i32> = mesh
        .face_sources
        .iter()
        .enumerate()
        .filter(|&(_, &face)| face != u32::MAX && wanted_faces.contains(&face))
        .map(|(index, _)| index as i32)
        .collect();
    let lost = vertices.len() < entry.vertices.len()
        || edges.len() < entry.edges.len()
        || faces.len() < entry.faces.len();
    (vertices, edges, faces, lost)
}
