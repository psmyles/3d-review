//! What attaching a file found: how much of the model it animates, and whatever
//! does not line up between the two files.
//!
//! Measured once, when the file attaches, from the file's own scene and clips
//! and the [`Binding`] between them — before the clips are converted and
//! re-pointed, so every figure here describes the file as it was authored. A
//! [`BindingReport`] with [`warnings`](BindingReport::has_warnings) is what the
//! viewer tells the user about up front; the rest (coverage, the lists of what
//! did not match) is there for the file's binding report.
//!
//! What is a warning and what is only listed follows from what the user can do
//! about it. A bone of the file's with nothing to drive is most likely a renamed
//! or missing bone, and its keys are being thrown away. A control-rig node with
//! no counterpart is expected — the rig never had one — and so is a model bone
//! the file leaves alone: a face-only file is meant to leave the body at rest.

use super::Binding;
use super::convert::{declared_unit, unit_scale};
use glam::Vec3;

use crate::{AnimationClip, CoordinateAxes, ModelData, NodeKind};

/// A bone offset this close to the model's — in meters — is the same offset.
/// A tenth of a millimetre: the rounding two exporters leave on one skeleton.
const OFFSET_TOLERANCE_METERS: f32 = 1e-4;
/// ...or this close, as a fraction of the model's offset. A percent of a bone is
/// already more than any exporter's rounding and less than a proportion an
/// animator would notice.
const OFFSET_TOLERANCE_RELATIVE: f32 = 0.01;

/// How many of something the file animates, of how many there are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coverage {
    pub animated: usize,
    pub total: usize,
}

/// The two files declare different units. Always converted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitChange {
    /// One unit of each, in meters.
    pub file_meters: f32,
    pub model_meters: f32,
}

/// The two files declare different axis systems.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisChange {
    pub file: CoordinateAxes,
    pub model: CoordinateAxes,
    /// Whether the keys were turned into the model's axes: `false` when the two
    /// systems are of opposite handedness, which only a mirror could map.
    pub converted: bool,
}

/// A bone whose offset from its parent, as the file keys it, is not the
/// model's: the two skeletons are not the same proportions. All in meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneOffset {
    /// The bone, in the file's graph.
    pub node: u32,
    /// The model bone it drives.
    pub model_node: u32,
    /// The length of the offset each file gives the bone.
    pub file_length: f32,
    pub model_length: f32,
    /// How far apart the two offsets are — their lengths *and* directions.
    pub difference: f32,
}

/// A mesh the file carries a copy of, with a different vertex count from the
/// model's: most likely exported from another version of that mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexCount {
    /// The mesh, in the file's graph.
    pub node: u32,
    /// The model's mesh of that name.
    pub model_node: u32,
    /// Each one's logical (DCC) vertex count.
    pub file: usize,
    pub model: usize,
}

/// How one attached file binds to the model. Node indices name the file's own
/// graph ([`super::AttachedAnimation::nodes`]) or, where the field says so, the
/// model's; channel indices are the model's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindingReport {
    /// The model's bones the file animates, of all its bones.
    pub bones: Coverage,
    /// The model's blend-shape channels the file animates, of those on the
    /// meshes the file carries a copy of. Zero of zero for a file with no mesh.
    pub channels: Coverage,
    /// Warned about, and converted.
    pub units: Option<UnitChange>,
    /// Warned about, and converted when a rotation can.
    pub axes: Option<AxisChange>,
    /// Warned about; the keys play as authored. Largest difference first.
    pub bone_offsets: Vec<BoneOffset>,
    /// Warned about.
    pub vertex_counts: Vec<VertexCount>,
    /// The file's bones with no counterpart in the model. Warned about: their
    /// keys have nothing to drive.
    pub unmatched_bones: Vec<u32>,
    /// The file's other named nodes with no counterpart — control rigs,
    /// helpers. Listed only.
    pub unmatched_nodes: Vec<u32>,
    /// The model's bones nothing in the file keys. Listed only.
    pub unanimated_bones: Vec<u32>,
    /// The model's channels, on the meshes the file carries, that nothing in
    /// it keys. Listed only.
    pub unanimated_channels: Vec<u32>,
}

impl BindingReport {
    /// Measure how `source` binds to `model` through `binding`.
    pub fn new(model: &ModelData, source: &ModelData, binding: &Binding) -> Self {
        let model_meters = model.stats.source_unit_meters;
        let file_meters = source.stats.source_unit_meters;
        let units = unit_scale(model_meters, file_meters).map(|_| UnitChange {
            file_meters,
            model_meters,
        });
        let axes = match (source.stats.source_axes, model.stats.source_axes) {
            (Some(file), Some(model)) if file != model => Some(AxisChange {
                file,
                model,
                converted: file.rotation_to(model).is_some(),
            }),
            _ => None,
        };

        let mut unmatched_bones = Vec::new();
        let mut unmatched_nodes = Vec::new();
        for (index, node) in source.nodes.iter().enumerate() {
            if binding.node(index as u32).is_some()
                || super::without_namespace(&node.name).is_empty()
            {
                continue;
            }
            if node.kind == NodeKind::Bone {
                unmatched_bones.push(index as u32);
            } else {
                unmatched_nodes.push(index as u32);
            }
        }

        let (bones, unanimated_bones) = bone_coverage(model, source, binding);
        let (channels, unanimated_channels) = channel_coverage(model, source, binding);
        Self {
            bones,
            channels,
            units,
            axes,
            bone_offsets: bone_offsets(model, source, binding),
            vertex_counts: vertex_counts(model, source, binding),
            unmatched_bones,
            unmatched_nodes,
            unanimated_bones,
            unanimated_channels,
        }
    }

    /// Whether anything here is worth telling the user about before they press
    /// play.
    pub fn has_warnings(&self) -> bool {
        self.units.is_some()
            || self.axes.is_some()
            || !self.bone_offsets.is_empty()
            || !self.vertex_counts.is_empty()
            || !self.unmatched_bones.is_empty()
    }
}

/// Whether `track` keys anything at all.
fn keys_anything(track: &crate::NodeTrack) -> bool {
    !(track.translation.is_empty() && track.rotation.is_empty() && track.scale.is_empty())
}

/// The model's bones the file animates, and the list of those it does not.
fn bone_coverage(model: &ModelData, source: &ModelData, binding: &Binding) -> (Coverage, Vec<u32>) {
    let mut animated = vec![false; model.nodes.len()];
    for track in source.animations.iter().flat_map(|clip| &clip.tracks) {
        if let Some(target) = binding.node(track.node).filter(|_| keys_anything(track))
            && let Some(slot) = animated.get_mut(target as usize)
        {
            *slot = true;
        }
    }
    let bones = model
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == NodeKind::Bone);
    let mut coverage = Coverage::default();
    let mut unanimated = Vec::new();
    for (index, _) in bones {
        coverage.total += 1;
        if animated[index] {
            coverage.animated += 1;
        } else {
            unanimated.push(index as u32);
        }
    }
    (coverage, unanimated)
}

/// The model's channels on the meshes the file carries a copy of that the file
/// animates, and the list of those it does not.
fn channel_coverage(
    model: &ModelData,
    source: &ModelData,
    binding: &Binding,
) -> (Coverage, Vec<u32>) {
    let Some(morph) = &model.morph else {
        return (Coverage::default(), Vec::new());
    };
    let mut carried = vec![false; model.nodes.len()];
    for (index, node) in source.nodes.iter().enumerate() {
        if node.mesh_part.is_some()
            && let Some(target) = binding.node(index as u32)
            && let Some(slot) = carried.get_mut(target as usize)
        {
            *slot = true;
        }
    }
    let mut animated = vec![false; morph.channels.len()];
    for track in source.animations.iter().flat_map(|clip| &clip.morph_tracks) {
        if let Some(target) = binding
            .channel(track.channel)
            .filter(|_| !track.keys.is_empty())
            && let Some(slot) = animated.get_mut(target as usize)
        {
            *slot = true;
        }
    }
    let mut coverage = Coverage::default();
    let mut unanimated = Vec::new();
    for (index, channel) in morph.channels.iter().enumerate() {
        if !carried
            .get(channel.mesh_node as usize)
            .copied()
            .unwrap_or(false)
        {
            continue;
        }
        coverage.total += 1;
        if animated[index] {
            coverage.animated += 1;
        } else {
            unanimated.push(index as u32);
        }
    }
    (coverage, unanimated)
}

/// The bones whose offset from their parent the file keys differently from the
/// model's rest — which, for a bone that only rotates, is the bone's length and
/// direction in its parent's frame, the same in every pose.
///
/// Measured only where it means a proportion:
///
/// - a bone under another bone. One under the scene root or a null is where the
///   skeleton is *placed*, not how it is built;
/// - a bone whose translation the file keys. One it does not plays the model's
///   own offset, so the two cannot disagree;
/// - a bone whose keyed translation holds still. One that moves — a hip
///   carrying the root motion, a jaw that slides — has no one length to compare.
fn bone_offsets(model: &ModelData, source: &ModelData, binding: &Binding) -> Vec<BoneOffset> {
    // Each file's own unit, to meters. Import put the conversion in each root,
    // so a node's local offset is still in its file's unit; a file that
    // declared none is taken to share the other's.
    let model_meters = model.stats.source_unit_meters;
    let file_meters = source.stats.source_unit_meters;
    let (model_meters, file_meters) =
        match (declared_unit(model_meters), declared_unit(file_meters)) {
            (true, true) => (model_meters, file_meters),
            (true, false) => (model_meters, model_meters),
            (false, true) => (file_meters, file_meters),
            (false, false) => (1.0, 1.0),
        };
    let held = held_translations(
        &source.animations,
        source.nodes.len(),
        OFFSET_TOLERANCE_METERS / file_meters,
    );
    let mut offsets = Vec::new();
    for (index, node) in source.nodes.iter().enumerate() {
        let Some(model_node) = binding.node(index as u32) else {
            continue;
        };
        let parent_is_bone = node
            .parent
            .and_then(|parent| source.nodes.get(parent))
            .is_some_and(|parent| parent.kind == NodeKind::Bone);
        if node.kind != NodeKind::Bone || !parent_is_bone {
            continue;
        }
        let Some(target) = model.nodes.get(model_node as usize) else {
            continue;
        };
        let Some(keyed) = held[index] else {
            continue;
        };
        let file_offset = keyed * file_meters;
        let model_offset = target.rest_local.translation * model_meters;
        let difference = file_offset.distance(model_offset);
        let tolerance =
            OFFSET_TOLERANCE_METERS.max(OFFSET_TOLERANCE_RELATIVE * model_offset.length());
        if difference > tolerance {
            offsets.push(BoneOffset {
                node: index as u32,
                model_node,
                file_length: file_offset.length(),
                model_length: model_offset.length(),
                difference,
            });
        }
    }
    offsets.sort_by(|a, b| b.difference.total_cmp(&a.difference));
    offsets
}

/// Per node of the file, the translation every clip keys it at, in the file's
/// unit, when every key is within `still` of the first — `None` for a node no
/// clip keys the translation of, and for one whose translation moves. One pass
/// over every key, however many clips and bones there are.
fn held_translations(clips: &[AnimationClip], nodes: usize, still: f32) -> Vec<Option<Vec3>> {
    #[derive(Clone, Copy)]
    enum Held {
        Unkeyed,
        Still(Vec3),
        Moving,
    }
    let mut held = vec![Held::Unkeyed; nodes];
    for track in clips.iter().flat_map(|clip| &clip.tracks) {
        let Some(slot) = held.get_mut(track.node as usize) else {
            continue;
        };
        for key in &track.translation {
            *slot = match *slot {
                Held::Unkeyed => Held::Still(key.value),
                Held::Still(first) if key.value.distance(first) <= still => Held::Still(first),
                _ => Held::Moving,
            };
        }
    }
    held.into_iter()
        .map(|held| match held {
            Held::Still(value) => Some(value),
            Held::Unkeyed | Held::Moving => None,
        })
        .collect()
}

/// The meshes the file carries a copy of whose vertex count is not the model's.
fn vertex_counts(model: &ModelData, source: &ModelData, binding: &Binding) -> Vec<VertexCount> {
    source
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.mesh_part.is_some() && node.source_vertex_count > 0)
        .filter_map(|(index, node)| {
            let model_node = binding.node(index as u32)?;
            let target = model.nodes.get(model_node as usize)?;
            (target.mesh_part.is_some()
                && target.source_vertex_count > 0
                && target.source_vertex_count != node.source_vertex_count)
                .then_some(VertexCount {
                    node: index as u32,
                    model_node,
                    file: node.source_vertex_count,
                    model: target.source_vertex_count,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::anim::node_with_local;
    use crate::{Key, LocalTransform, MorphChannel, MorphData, MorphTrack, NodeTrack, SceneNode};

    fn node(name: &str, parent: Option<usize>, kind: NodeKind, offset: Vec3) -> SceneNode {
        SceneNode {
            kind,
            ..node_with_local(
                name,
                parent,
                LocalTransform {
                    translation: offset,
                    ..LocalTransform::IDENTITY
                },
            )
        }
    }

    fn mesh(name: &str, vertices: usize) -> SceneNode {
        SceneNode {
            mesh_part: Some(0),
            source_vertex_count: vertices,
            ..node(name, Some(0), NodeKind::Mesh, Vec3::ZERO)
        }
    }

    fn channel(name: &str, mesh_node: u32) -> MorphChannel {
        MorphChannel {
            name: name.to_owned(),
            mesh_node,
            rest_weight: 0.0,
            keyframes: Vec::new(),
        }
    }

    /// A centimeter rig: root, hip, spine 50 cm up the hip, a neck 20 cm up
    /// the spine, and a head mesh with two channels.
    fn rig() -> ModelData {
        let mut rig = ModelData {
            nodes: vec![
                node("", None, NodeKind::Empty, Vec3::ZERO),
                node("Hip", Some(0), NodeKind::Bone, Vec3::new(0.0, 100.0, 0.0)),
                node("Spine", Some(1), NodeKind::Bone, Vec3::new(0.0, 50.0, 0.0)),
                node("Neck", Some(2), NodeKind::Bone, Vec3::new(0.0, 20.0, 0.0)),
                mesh("Head", 400),
            ],
            morph: Some(MorphData {
                channels: vec![channel("Smile", 4), channel("Blink", 4)],
                ..MorphData::default()
            }),
            ..ModelData::default()
        };
        rig.stats.source_unit_meters = 0.01;
        rig
    }

    fn translation(node: u32, values: &[Vec3]) -> NodeTrack {
        NodeTrack {
            node,
            translation: values
                .iter()
                .enumerate()
                .map(|(frame, &value)| Key {
                    time: frame as f64,
                    value,
                })
                .collect(),
            ..NodeTrack::default()
        }
    }

    /// An animation file in meters keying the rig's skeleton, a control-rig
    /// null, and a bone the rig does not have.
    fn body(spine: Vec3) -> ModelData {
        let mut body = ModelData {
            nodes: vec![
                node("", None, NodeKind::Empty, Vec3::ZERO),
                node("Hip", Some(0), NodeKind::Bone, Vec3::ZERO),
                node("Spine", Some(1), NodeKind::Bone, Vec3::ZERO),
                node("Neck", Some(2), NodeKind::Bone, Vec3::ZERO),
                node("Ctrl_Hips", Some(0), NodeKind::Empty, Vec3::ZERO),
                node("Tail", Some(1), NodeKind::Bone, Vec3::ZERO),
            ],
            animations: vec![AnimationClip {
                tracks: vec![
                    // The hip carries the root motion: it moves.
                    translation(1, &[Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.3, 1.0, 0.0)]),
                    translation(2, &[spine, spine]),
                    translation(4, &[Vec3::ZERO]),
                ],
                ..AnimationClip::default()
            }],
            ..ModelData::default()
        };
        body.stats.source_unit_meters = 1.0;
        body
    }

    fn report(model: &ModelData, source: &ModelData) -> BindingReport {
        BindingReport::new(model, source, &Binding::new(model, source))
    }

    #[test]
    fn a_skeleton_of_the_same_proportions_raises_nothing_but_its_unit() {
        let rig = rig();
        // 0.5 m is the rig's 50 cm.
        let report = report(&rig, &body(Vec3::new(0.0, 0.5, 0.0)));
        assert_eq!(report.bone_offsets, []);
        assert_eq!(
            report.units,
            Some(UnitChange {
                file_meters: 1.0,
                model_meters: 0.01
            })
        );
        assert_eq!(report.axes, None);
        assert!(report.has_warnings(), "a unit change is still said");
    }

    /// The spine is keyed 10 cm longer than the rig's. The hip, which moves, and
    /// the neck, which the file does not key, have nothing to compare.
    #[test]
    fn a_bone_keyed_at_another_length_is_a_proportion_warning() {
        let rig = rig();
        let report = report(&rig, &body(Vec3::new(0.0, 0.6, 0.0)));
        let [offset] = report.bone_offsets[..] else {
            panic!("one bone differs: {:?}", report.bone_offsets);
        };
        assert_eq!((offset.node, offset.model_node), (2, 2));
        assert!((offset.difference - 0.1).abs() < 1e-5);
        assert!((offset.file_length - 0.6).abs() < 1e-5);
        assert!((offset.model_length - 0.5).abs() < 1e-5);
    }

    #[test]
    fn rounding_between_two_exporters_is_not_a_proportion() {
        let rig = rig();
        let report = report(&rig, &body(Vec3::new(0.0, 0.500_05, 0.0)));
        assert_eq!(report.bone_offsets, []);
    }

    #[test]
    fn an_unmatched_bone_warns_and_an_unmatched_control_is_only_listed() {
        let rig = rig();
        let report = report(&rig, &body(Vec3::new(0.0, 0.5, 0.0)));
        assert_eq!(report.unmatched_bones, [5], "the tail");
        assert_eq!(
            report.unmatched_nodes,
            [4],
            "the control, and never the root"
        );
        // Hip and spine are keyed; the neck is not.
        assert_eq!(
            report.bones,
            Coverage {
                animated: 2,
                total: 3
            }
        );
        assert_eq!(report.unanimated_bones, [3]);
        assert_eq!(report.channels, Coverage::default(), "the file has no mesh");
    }

    /// The facial shape: a copy of the rig's head keying one of its channels,
    /// exported from a version of the head with another vertex count.
    #[test]
    fn a_copied_mesh_counts_its_channels_and_its_vertices() {
        let rig = rig();
        let mut face = ModelData {
            nodes: vec![
                node("", None, NodeKind::Empty, Vec3::ZERO),
                mesh("Head", 380),
            ],
            morph: Some(MorphData {
                channels: vec![channel("Blink", 1)],
                ..MorphData::default()
            }),
            animations: vec![AnimationClip {
                morph_tracks: vec![MorphTrack {
                    channel: 0,
                    keys: vec![Key {
                        time: 0.0,
                        value: 1.0,
                    }],
                }],
                ..AnimationClip::default()
            }],
            ..ModelData::default()
        };
        face.stats.source_unit_meters = 0.01;
        let report = report(&rig, &face);
        assert_eq!(
            report.channels,
            Coverage {
                animated: 1,
                total: 2
            }
        );
        assert_eq!(report.unanimated_channels, [0], "the smile");
        assert_eq!(
            report.vertex_counts,
            [VertexCount {
                node: 1,
                model_node: 4,
                file: 380,
                model: 400
            }]
        );
        assert_eq!(report.bones.animated, 0);
        assert!(report.has_warnings());

        face.nodes[1].source_vertex_count = 400;
        let report = super::super::AttachedAnimation::new(&rig, face, "face".to_owned()).report;
        assert!(!report.has_warnings(), "{report:?}");
    }
}
