//! Playing one file's clips on another file's model.
//!
//! Characters are commonly delivered as a **rig** file (meshes, skeleton, skins,
//! blend shapes, no clips) beside any number of **animation** files. Attaching an
//! animation file to the model on screen **re-points** its clips at that model:
//! every track is matched to a node or blend-shape channel of the model by name
//! and rewritten to its index there, and a track with nothing to drive is
//! dropped. Nothing is resampled and no key is copied — the clip's own vectors are
//! filtered in place — so pose evaluation, the GPU deform, the clip envelope, the
//! motion paths and the transport all play an attached clip exactly as they play
//! the model's own, against the model's `ModelData`.
//!
//! What matches:
//!
//! - **A node** by name, with any Maya namespace removed (`Eve:CC_Base_Hip` is
//!   `CC_Base_Hip`). When the model has several nodes of that name, an exact
//!   name match wins, then one whose parent's name also agrees. Each model node
//!   is driven by at most one source node, so two tracks never fight over it. An
//!   unnamed node never matches: ufbx's scene root is unnamed in every file.
//! - **A blend-shape channel** by its mesh node's name plus its own name, both
//!   without namespaces. ufbx names a channel after its deformer
//!   (`Morpher_CC_Base_Body.Brow_Down_L`), so the pair is specific.
//!
//! The channels a clip does not key keep the model's rest values, never the
//! animation file's default pose, which is whatever frame its exporter was parked
//! on: [`crate::anim::evaluate_pose`] falls back to the *model's* rest for an
//! untracked node, which is exactly that.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{AnimationClip, ModelData, NodeKind, SceneNode};

/// Which clip, of every clip the viewer holds: `file` 0 is the model's own file,
/// `file` `n` is its `n`-th attached animation file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipId {
    pub file: usize,
    pub clip: usize,
}

/// An animation file attached to a model: its clips, re-pointed at the model,
/// and what is needed to say how it was matched. Its meshes, if it carried any,
/// are gone: a file that hosts blend-shape curves on a copy of the model's meshes
/// is only ever played on the model's own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttachedAnimation {
    /// The file's name, as the interface shows it.
    pub name: String,
    /// The file's declared frame rate, `0.0` when it declared none. Each clip
    /// carries its own as well, which is what it is timed by.
    pub frame_rate: f64,
    /// The file's own scene graph — what is shown and counted for it.
    pub nodes: Vec<SceneNode>,
    /// Per node of [`Self::nodes`], the model node it drives.
    pub node_map: Vec<Option<u32>>,
    /// The file's clips, re-pointed at the model.
    pub clips: Vec<AnimationClip>,
}

impl AttachedAnimation {
    /// Attach `source` to `model`: re-point its clips and keep its scene graph,
    /// dropping everything else it carried. Consumes `source` so the clips move
    /// rather than copy.
    pub fn new(model: &ModelData, source: ModelData, name: String) -> Self {
        let binding = Binding::new(model, &source);
        let ModelData {
            nodes,
            animations,
            frame_rate,
            ..
        } = source;
        let clips = animations
            .into_iter()
            .map(|clip| binding.repoint(clip))
            .collect();
        Self {
            name,
            frame_rate,
            nodes,
            node_map: binding.nodes,
            clips,
        }
    }
}

/// How one file's scene maps onto a model's: per node and per blend-shape
/// channel of the file, the model's counterpart.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Binding {
    /// Per source node, the model node it drives.
    pub nodes: Vec<Option<u32>>,
    /// Per source blend-shape channel, the model channel it drives.
    pub channels: Vec<Option<u32>>,
}

impl Binding {
    /// Match `source`'s nodes and channels to `model`'s by name (see the module
    /// docs for the rules).
    pub fn new(model: &ModelData, source: &ModelData) -> Self {
        Self {
            nodes: match_nodes(model, source),
            channels: match_channels(model, source),
        }
    }

    /// Whether any of `source`'s clips has a track this binding resolves — the
    /// test for "this file animates that model".
    pub fn binds(&self, source: &ModelData) -> bool {
        source.animations.iter().any(|clip| {
            clip.tracks
                .iter()
                .any(|track| self.node(track.node).is_some())
                || clip
                    .morph_tracks
                    .iter()
                    .any(|track| self.channel(track.channel).is_some())
        })
    }

    /// How many of `source`'s meshes have no mesh of the same name in the model.
    /// Zero for a file that carries meshes only to host curves on copies of the
    /// model's own — the facial-animation shape — and for a file with none.
    pub fn foreign_meshes(&self, model: &ModelData, source: &ModelData) -> usize {
        source
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.mesh_part.is_some())
            .filter(|&(index, _)| {
                self.node(index as u32)
                    .and_then(|target| model.nodes.get(target as usize))
                    .is_none_or(|target| target.mesh_part.is_none())
            })
            .count()
    }

    fn node(&self, node: u32) -> Option<u32> {
        self.nodes.get(node as usize).copied().flatten()
    }

    fn channel(&self, channel: u32) -> Option<u32> {
        self.channels.get(channel as usize).copied().flatten()
    }

    /// `clip` with every track rewritten to the model's index and every track
    /// with no counterpart removed — in place, so no key is copied.
    pub fn repoint(&self, mut clip: AnimationClip) -> AnimationClip {
        clip.tracks.retain_mut(|track| match self.node(track.node) {
            Some(target) => {
                track.node = target;
                true
            }
            None => false,
        });
        clip.morph_tracks
            .retain_mut(|track| match self.channel(track.channel) {
                Some(target) => {
                    track.channel = target;
                    true
                }
                None => false,
            });
        clip
    }
}

/// `name` without its Maya namespaces: everything after the last `:`.
pub fn without_namespace(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The name of `node`'s parent without namespaces, or `""` for a root.
fn parent_name<'a>(nodes: &'a [SceneNode], node: &SceneNode) -> &'a str {
    node.parent
        .and_then(|parent| nodes.get(parent))
        .map_or("", |parent| without_namespace(&parent.name))
}

fn match_nodes(model: &ModelData, source: &ModelData) -> Vec<Option<u32>> {
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, node) in model.nodes.iter().enumerate() {
        let name = without_namespace(&node.name);
        if !name.is_empty() {
            by_name.entry(name).or_default().push(index);
        }
    }
    let mut claimed = vec![false; model.nodes.len()];
    let mut map = vec![None; source.nodes.len()];
    // Exact names first, across the whole file, so a namespaced copy of a bone
    // can never take the model node from the bone of that very name; then what
    // is left, without namespaces. Within a pass a parent that agrees wins.
    for exact in [true, false] {
        for (index, node) in source.nodes.iter().enumerate() {
            if map[index].is_some() {
                continue;
            }
            let Some(candidates) = by_name.get(without_namespace(&node.name)) else {
                continue;
            };
            let parent = parent_name(&source.nodes, node);
            let best = candidates
                .iter()
                .copied()
                .filter(|&candidate| {
                    !claimed[candidate] && (!exact || model.nodes[candidate].name == node.name)
                })
                .min_by_key(|&candidate| {
                    parent_name(&model.nodes, &model.nodes[candidate]) != parent
                });
            if let Some(target) = best {
                claimed[target] = true;
                map[index] = Some(target as u32);
            }
        }
    }
    map
}

fn match_channels(model: &ModelData, source: &ModelData) -> Vec<Option<u32>> {
    let (Some(model_morph), Some(source_morph)) = (&model.morph, &source.morph) else {
        return source
            .morph
            .as_ref()
            .map_or_else(Vec::new, |morph| vec![None; morph.channels.len()]);
    };
    let key = |nodes: &[SceneNode], mesh_node: u32, name: &str| -> Option<(String, String)> {
        let mesh = nodes.get(mesh_node as usize)?;
        Some((
            without_namespace(&mesh.name).to_owned(),
            without_namespace(name).to_owned(),
        ))
    };
    let mut by_key: HashMap<(String, String), u32> = HashMap::new();
    for (index, channel) in model_morph.channels.iter().enumerate() {
        if let Some(key) = key(&model.nodes, channel.mesh_node, &channel.name) {
            by_key.entry(key).or_insert(index as u32);
        }
    }
    source_morph
        .channels
        .iter()
        .map(|channel| {
            let key = key(&source.nodes, channel.mesh_node, &channel.name)?;
            by_key.get(&key).copied()
        })
        .collect()
}

/// Every clip the viewer holds — the model's own and every attached file's —
/// addressed by [`ClipId`].
#[derive(Debug, Clone, Copy)]
pub struct Clips<'a> {
    pub model: &'a ModelData,
    pub attached: &'a [Arc<AttachedAnimation>],
}

impl<'a> Clips<'a> {
    pub fn new(model: &'a ModelData, attached: &'a [Arc<AttachedAnimation>]) -> Self {
        Self { model, attached }
    }

    /// The clips of file `file` (0 is the model's own).
    pub fn of_file(&self, file: usize) -> &'a [AnimationClip] {
        match file {
            0 => &self.model.animations,
            n => self
                .attached
                .get(n - 1)
                .map_or(&[], |attached| attached.clips.as_slice()),
        }
    }

    pub fn get(&self, id: ClipId) -> Option<&'a AnimationClip> {
        self.of_file(id.file).get(id.clip)
    }

    /// True when no file holds a clip.
    pub fn is_empty(&self) -> bool {
        (0..=self.attached.len()).all(|file| self.of_file(file).is_empty())
    }
}

/// Whether `model` has a skeleton — at least one bone — for an animation file's
/// own to stand in as a rig.
pub fn has_skeleton(model: &ModelData) -> bool {
    model.nodes.iter().any(|node| node.kind == NodeKind::Bone)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::anim::node_with_local;
    use crate::{
        Key, LocalTransform, MorphChannel, MorphData, MorphKeyframe, MorphShape, MorphTrack,
        NodeTrack,
    };

    fn node(name: &str, parent: Option<usize>, kind: NodeKind) -> SceneNode {
        SceneNode {
            kind,
            ..node_with_local(name, parent, LocalTransform::IDENTITY)
        }
    }

    /// A rig: an unnamed root, a hip and a spine under it, and a head mesh with
    /// two blend-shape channels.
    fn rig() -> ModelData {
        let mut head = node("Head", Some(0), NodeKind::Mesh);
        head.mesh_part = Some(0);
        ModelData {
            nodes: vec![
                node("", None, NodeKind::Other),
                node("Hip", Some(0), NodeKind::Bone),
                node("Spine", Some(1), NodeKind::Bone),
                head,
            ],
            morph: Some(MorphData {
                channels: vec![channel("Morpher.Smile", 3), channel("Morpher.Blink", 3)],
                shapes: vec![MorphShape::default()],
                ..MorphData::default()
            }),
            ..ModelData::default()
        }
    }

    fn channel(name: &str, mesh_node: u32) -> MorphChannel {
        MorphChannel {
            name: name.to_owned(),
            mesh_node,
            rest_weight: 0.0,
            keyframes: vec![MorphKeyframe {
                shape: 0,
                target_weight: 1.0,
            }],
        }
    }

    fn track(node: u32) -> NodeTrack {
        NodeTrack {
            node,
            translation: vec![Key {
                time: 0.0,
                value: Vec3::X,
            }],
            ..NodeTrack::default()
        }
    }

    fn clip(tracks: Vec<NodeTrack>, morph_tracks: Vec<MorphTrack>) -> AnimationClip {
        AnimationClip {
            name: "clip".to_owned(),
            time_end: 1.0,
            frame_rate: 30.0,
            tracks,
            morph_tracks,
            ..AnimationClip::default()
        }
    }

    /// An animation file in the body's shape: the rig's skeleton under a
    /// namespace, a control-rig null the rig lacks, and a clip keying all three.
    fn body_animation() -> ModelData {
        ModelData {
            nodes: vec![
                node("", None, NodeKind::Other),
                node("Ctrl_Reference", Some(0), NodeKind::Empty),
                node("Eve:Hip", Some(0), NodeKind::Bone),
                node("Eve:Spine", Some(2), NodeKind::Bone),
            ],
            animations: vec![clip(vec![track(1), track(2), track(3)], Vec::new())],
            frame_rate: 30.0,
            ..ModelData::default()
        }
    }

    #[test]
    fn nodes_match_by_name_without_namespaces_and_the_root_never_does() {
        let binding = Binding::new(&rig(), &body_animation());
        assert_eq!(binding.nodes, [None, None, Some(1), Some(2)]);
        assert!(binding.binds(&body_animation()));
    }

    #[test]
    fn attaching_repoints_the_tracks_and_drops_those_with_nothing_to_drive() {
        let rig = rig();
        let attached = AttachedAnimation::new(&rig, body_animation(), "AN.fbx".to_owned());
        let nodes: Vec<u32> = attached.clips[0]
            .tracks
            .iter()
            .map(|track| track.node)
            .collect();
        assert_eq!(nodes, [1, 2], "the control-rig track is gone");
        assert_eq!(attached.nodes.len(), 4, "the file's own graph is kept");
        assert_eq!(attached.clips[0].frame_rate, 30.0);
        let channels = rig.morph.as_ref().map_or(0, |morph| morph.channels.len());
        assert_eq!(
            attached.clips[0].validate(rig.nodes.len(), channels),
            Ok(())
        );
    }

    #[test]
    fn a_model_node_is_driven_by_one_source_node_at_most() {
        // Two namespaces carrying the same bone: the exact name wins it, the
        // other gets nothing rather than a second track on the same node.
        let mut source = body_animation();
        source.nodes.push(node("Hip", Some(0), NodeKind::Bone));
        let binding = Binding::new(&rig(), &source);
        assert_eq!(binding.nodes[4], Some(1), "the exact name");
        assert_eq!(binding.nodes[2], None, "the namespaced one comes second");
    }

    #[test]
    fn a_same_named_node_under_the_matching_parent_is_preferred() {
        let mut rig = rig();
        // A second "Spine" under the root, before the one under the hip.
        rig.nodes.insert(2, node("Spine", Some(0), NodeKind::Bone));
        rig.nodes[3].parent = Some(1);
        rig.nodes[4].parent = Some(0);
        rig.morph = None;
        let binding = Binding::new(&rig, &body_animation());
        assert_eq!(binding.nodes[3], Some(3), "the spine under the hip");
    }

    #[test]
    fn blend_shape_weights_match_by_mesh_and_channel_name() {
        // The facial shape: a copy of the rig's head carrying one of its channels
        // and one the rig does not have.
        let mut head = node("Head", Some(0), NodeKind::Mesh);
        head.mesh_part = Some(0);
        let face = ModelData {
            nodes: vec![node("", None, NodeKind::Other), head],
            morph: Some(MorphData {
                channels: vec![channel("Morpher.Blink", 1), channel("Morpher.Pout", 1)],
                ..MorphData::default()
            }),
            animations: vec![clip(
                Vec::new(),
                vec![
                    MorphTrack {
                        channel: 0,
                        keys: vec![Key {
                            time: 0.0,
                            value: 1.0,
                        }],
                    },
                    MorphTrack {
                        channel: 1,
                        keys: Vec::new(),
                    },
                ],
            )],
            ..ModelData::default()
        };
        let rig = rig();
        let binding = Binding::new(&rig, &face);
        assert_eq!(binding.channels, [Some(1), None]);
        assert!(binding.binds(&face));
        assert_eq!(
            binding.foreign_meshes(&rig, &face),
            0,
            "the head is the rig's"
        );

        let attached = AttachedAnimation::new(&rig, face, "AN_VO.fbx".to_owned());
        let channels: Vec<u32> = attached.clips[0]
            .morph_tracks
            .iter()
            .map(|track| track.channel)
            .collect();
        assert_eq!(channels, [1]);
    }

    #[test]
    fn a_file_with_a_mesh_the_model_lacks_counts_it_as_foreign() {
        let mut other = body_animation();
        let mut sword = node("Sword", Some(0), NodeKind::Mesh);
        sword.mesh_part = Some(0);
        other.nodes.push(sword);
        let rig = rig();
        let binding = Binding::new(&rig, &other);
        assert!(binding.binds(&other));
        assert_eq!(binding.foreign_meshes(&rig, &other), 1);
    }

    #[test]
    fn an_unrelated_file_does_not_bind() {
        let unrelated = ModelData {
            nodes: vec![
                node("", None, NodeKind::Other),
                node("Pelvis", Some(0), NodeKind::Bone),
            ],
            animations: vec![clip(vec![track(0), track(1)], Vec::new())],
            ..ModelData::default()
        };
        let binding = Binding::new(&rig(), &unrelated);
        assert!(!binding.binds(&unrelated), "the unnamed roots do not count");
    }

    #[test]
    fn clips_are_addressed_by_file_then_index() {
        let mut model = rig();
        model.animations = vec![clip(Vec::new(), Vec::new())];
        let attached = vec![Arc::new(AttachedAnimation::new(
            &model,
            body_animation(),
            "AN.fbx".to_owned(),
        ))];
        let clips = Clips::new(&model, &attached);
        assert!(clips.get(ClipId { file: 0, clip: 0 }).is_some());
        assert_eq!(
            clips
                .get(ClipId { file: 1, clip: 0 })
                .map(|clip| clip.tracks.len()),
            Some(2)
        );
        assert!(clips.get(ClipId { file: 2, clip: 0 }).is_none());
        assert!(!clips.is_empty());
        assert!(Clips::new(&rig(), &[]).is_empty());
    }
}
