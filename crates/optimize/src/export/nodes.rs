//! The scene graph: which nodes are emitted, how they are parented, and where
//! the unit scale lands.
//!
//! The unit factor is applied **once**, at the top of the hierarchy — the chain
//! is built against a root frame rather than the identity, so it lands in the
//! topmost emitted node's scale and the whole subtree rides it. Applying it to
//! each local value instead double-counts, which cancels in the positions and
//! destroys the normals.

use std::collections::HashMap;
use std::ffi::CString;

use glam::{Mat4, Quat, Vec3};
use review_model::extras::AttributeKind;
use review_model::{ModelData, SourceExtras};

use crate::notice::ExportNote;
use crate::stack::HierarchyMode;

use super::*;

/// One mesh's worth of a processed level: the triangles owned by a single source
/// node, and the compacted vertex set they reference.
pub(crate) struct NodeGroup {
    /// Index into `ModelData::nodes`, or `None` when the model carries no node
    /// tags (everything then becomes one mesh).
    pub(crate) source_node: Option<usize>,
    /// Triangle indices into the level's index buffer.
    pub(crate) triangles: Vec<usize>,
}

/// Split a processed level into per-source-node groups, in first-seen order.
pub(crate) fn group_by_node(model: &ModelData) -> Vec<NodeGroup> {
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

    // In first-appearance order, found through a map: a linear search of the
    // groups seen so far is triangles x nodes on a scene of many objects.
    let mut slot_of: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut groups: Vec<NodeGroup> = Vec::new();
    for triangle in 0..triangle_count {
        let node = model.triangles.node[triangle];
        let slot = *slot_of.entry(node).or_insert_with(|| {
            groups.push(NodeGroup {
                source_node: Some(node as usize),
                triangles: Vec::new(),
            });
            groups.len() - 1
        });
        groups[slot].triangles.push(triangle);
    }
    groups
}

/// The `RVO_ATTRIB_*` kind for a captured attribute.
pub(crate) fn attribute_code(kind: AttributeKind) -> u32 {
    match kind {
        AttributeKind::Bone => ATTRIB_BONE,
        AttributeKind::Light => ATTRIB_LIGHT,
        AttributeKind::Camera => ATTRIB_CAMERA,
        AttributeKind::Empty => ATTRIB_NULL,
        AttributeKind::LodGroup => ATTRIB_LOD_GROUP,
        // A mesh attribute is the geometry itself; anything else has no writer.
        _ => ATTRIB_NONE,
    }
}

/// Whether a source node is one the importer made up rather than the file
/// authored. Neither the synthetic root nor a scale helper is written; the
/// authored `InheritType` lets a reader re-derive the helper.
pub(crate) fn is_synthetic(source: &ModelData, extras: &SourceExtras, index: usize) -> bool {
    review_model::hierarchy::is_synthetic(source, Some(extras), index)
}

/// The nearest authored ancestor of `index`, skipping synthetic nodes.
pub(crate) fn authored_parent(
    source: &ModelData,
    extras: &SourceExtras,
    index: usize,
) -> Option<usize> {
    review_model::hierarchy::authored_parent(source, Some(extras), index)
}

/// Emit one authored node (after its authored parent) with the properties the
/// file gave it, returning its scene index. `placed` memoizes the walk so
/// shared ancestors are emitted once, in parent-first order.
pub(crate) fn emit_authored_node(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    index: usize,
    depth: usize,
) -> Option<i32> {
    if let Some(&existing) = placed.get(&index) {
        return Some(existing);
    }
    // A parent cycle would recurse forever; the chain can never legitimately be
    // deeper than the node table.
    if depth > source.nodes.len() {
        return None;
    }
    let node = source.nodes.get(index)?;
    let authored = extras.nodes.get(index)?;
    let parent = match authored_parent(source, extras, index) {
        Some(parent) => emit_authored_node(scene, placed, source, extras, parent, depth + 1)
            .unwrap_or(NO_PARENT),
        None => NO_PARENT,
    };
    let data = authored_node_data(scene, node, authored, parent, &node.name);
    scene.nodes.push(data);
    let placed_index = (scene.nodes.len() - 1) as i32;
    placed.insert(index, placed_index);
    Some(placed_index)
}

/// The node payload for `node` written from its authored properties.
pub(crate) fn authored_node_data(
    scene: &mut SceneData,
    node: &review_model::SceneNode,
    authored: &review_model::extras::NodeExtras,
    parent: i32,
    name: &str,
) -> NodeData {
    let props = scene.push_props(&authored.props);
    let (attribute_kind, attribute_name, attribute_props) = match &authored.attribute {
        Some(attribute) => (
            attribute_code(attribute.kind),
            c_string_or_empty(&attribute.name),
            scene.push_props(&attribute.props),
        ),
        None => (ATTRIB_NONE, CString::default(), PropRange::default()),
    };
    // The computed transform is kept beside the authored properties only as
    // the fallback the bridge never reads when `authored_transform` is set.
    let local = node.rest_local;
    NodeData {
        name: c_string(name, "Node"),
        parent,
        translation: [
            f64::from(local.translation.x),
            f64::from(local.translation.y),
            f64::from(local.translation.z),
        ],
        rotation: [
            f64::from(local.rotation.x),
            f64::from(local.rotation.y),
            f64::from(local.rotation.z),
            f64::from(local.rotation.w),
        ],
        scaling: [
            f64::from(local.scale.x),
            f64::from(local.scale.y),
            f64::from(local.scale.z),
        ],
        authored_transform: true,
        props,
        attribute_kind,
        attribute_name,
        attribute_props,
    }
}

/// Emit the whole authored scene graph, parent-first, into `placed`.
pub(crate) fn place_graph(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
) {
    for index in 0..source.nodes.len() {
        if is_synthetic(source, extras, index) {
            continue;
        }
        emit_authored_node(scene, placed, source, extras, index, 0);
    }
}

/// The scene node a level's mesh attaches to, with the whole graph already
/// placed. In a chain (`in_chain`: the file holds several levels) level 0 uses
/// the source node itself, renamed `_LOD0`, and a later level gets a suffixed
/// sibling copy — same authored properties and attribute — beside it. A file
/// holding one level (one file per LOD, or a lone level) puts it on the source
/// node itself under its plain name, whatever the level's number: the file
/// name carries the suffix, and a copy beside the source node would leave that
/// node behind empty, with every display layer, pose and set still bound to it.
pub(crate) fn place_level_node(
    scene: &mut SceneData,
    placed: &HashMap<usize, i32>,
    source: &ModelData,
    extras: &SourceExtras,
    group: &NodeGroup,
    level: usize,
    in_chain: bool,
) -> i32 {
    let Some(index) = group.source_node else {
        // No node tags at all: one root holds the whole level.
        scene.nodes.push(NodeData {
            name: c_string(&level_name(&source.name, level, in_chain), "Mesh"),
            parent: NO_PARENT,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scaling: [1.0; 3],
            authored_transform: false,
            props: PropRange::default(),
            attribute_kind: ATTRIB_NONE,
            attribute_name: CString::default(),
            attribute_props: PropRange::default(),
        });
        return (scene.nodes.len() - 1) as i32;
    };
    let base = placed.get(&index).copied().unwrap_or(NO_PARENT);
    if base == NO_PARENT {
        return base;
    }
    if !in_chain {
        return base;
    }
    if level == 0 {
        if let Some(node) = source.nodes.get(index) {
            scene.nodes[base as usize].name = c_string(&suffixed(&node.name, 0), "Node");
        }
        return base;
    }
    let (Some(node), Some(authored)) = (source.nodes.get(index), extras.nodes.get(index)) else {
        return base;
    };
    let parent = scene.nodes[base as usize].parent;
    let data = authored_node_data(scene, node, authored, parent, &suffixed(&node.name, level));
    scene.nodes.push(data);
    let copy = (scene.nodes.len() - 1) as i32;
    scene.level_copies.entry(index).or_default().push(copy);
    copy
}

/// Create (or reuse) the scene node this group's mesh attaches to, returning its
/// index plus any notes raised while working out its transform. The computed
/// path, used without the source-property capture.
///
/// Under [`HierarchyMode::Rebuild`] the source node's ancestors are emitted too,
/// each with the local transform implied by the world transforms import
/// recorded. `placed` remembers every node already emitted into this scene, so
/// meshes with common ancestors — sibling LODs under one group node, or one
/// node's levels in a suffixed chain — hang off a single shared chain rather
/// than each duplicating it from the root. Only the mesh-bearing leaf carries
/// the level suffix, which puts a chain's levels beside each other as siblings —
/// the layout game FBX files use for their own LOD chains; with `in_chain` set the
/// level-0 leaf is suffixed `_LOD0` too, since it is one of them. The importer's
/// synthetic file root (nameless, meshless, parentless) is not re-emitted at
/// all: the written file has its own root, so an explicit copy would wrap every
/// re-import in one extra level.
///
/// Under [`HierarchyMode::FlatBaked`] a single identity root node is created per
/// mesh instead.
pub(crate) fn place_node(
    scene: &mut SceneData,
    placed: &mut HashMap<usize, i32>,
    source: &ModelData,
    group: &NodeGroup,
    hierarchy: HierarchyMode,
    level: usize,
    in_chain: bool,
) -> (i32, Vec<ExportNote>) {
    let mut notes = Vec::new();

    let plain = |name: CString, parent: i32, local: Mat4| -> NodeData {
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        let rotation = if rotation.is_finite() {
            rotation
        } else {
            Quat::IDENTITY
        };
        let scale = if scale.is_finite() { scale } else { Vec3::ONE };
        NodeData {
            name,
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
            authored_transform: false,
            props: PropRange::default(),
            attribute_kind: ATTRIB_NONE,
            attribute_name: CString::default(),
            attribute_props: PropRange::default(),
        }
    };

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
        scene.nodes.push(plain(
            c_string(&level_name(&name, level, in_chain), "Mesh"),
            NO_PARENT,
            Mat4::IDENTITY,
        ));
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
            notes.push(ExportNote::ParentChainLooped);
            chain.truncate(1);
            break;
        }
    }
    chain.reverse();

    let mut parent = NO_PARENT;
    // The frame the chain hangs off is the *written file's* root, whose unit is
    // the source's — not the meter import normalized everything to. Starting
    // the walk at `1/per_meter` instead of the identity is what puts the unit
    // conversion into the topmost emitted node's scale, once, for the whole
    // subtree. Everything below it then computes `parent⁻¹ × child` in the
    // source's own unit, because that is the unit import's node transforms are
    // already expressed in (it parks the normalization at the synthetic root,
    // which the skip below folds in here).
    let mut parent_world = Mat4::from_scale(Vec3::splat(1.0 / scene.unit.per_meter));
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
        // Reuse a node an earlier group already emitted. A later level's leaf in
        // a chain is a per-level variant of its source node, so it is never
        // shared. A level-0 leaf an earlier group emitted as its ancestor still
        // takes the chain's `_LOD0` name. Outside a chain the file holds one
        // level, whose leaf *is* the source node, as `place_level_node` has it.
        let level_copy = leaf && in_chain && level != 0;
        if !level_copy && let Some(&existing) = placed.get(&index) {
            if leaf && in_chain {
                scene.nodes[existing as usize].name = c_string(&suffixed(&node.name, 0), "Node");
            }
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
            notes.push(ExportNote::NonInvertibleTransform {
                node: node.name.clone(),
            });
            node.transform
        };

        let name = if leaf {
            level_name(&node.name, level, in_chain)
        } else {
            node.name.clone()
        };
        scene
            .nodes
            .push(plain(c_string(&name, "Node"), parent, local));
        parent = (scene.nodes.len() - 1) as i32;
        if !level_copy {
            placed.insert(index, parent);
        }
        parent_world = node.transform;
    }

    (parent, notes)
}

/// `"Body_LOD2"`.
pub(crate) fn suffixed(name: &str, level: usize) -> String {
    format!("{name}_LOD{level}")
}

/// The name a level's mesh node goes out under. A file holding a chain names
/// every level, `_LOD0` included: an engine groups the siblings into one LOD
/// group by that suffix, so a plain-named level 0 is left out of the group.
/// Outside a chain — a file holding one level, which is what one file per LOD
/// writes — the node keeps the source's plain name whatever the level: the
/// file name already carries the suffix, and the file round-trips with the
/// source's own names.
pub(crate) fn level_name(name: &str, level: usize, in_chain: bool) -> String {
    if in_chain {
        suffixed(name, level)
    } else {
        name.to_owned()
    }
}
