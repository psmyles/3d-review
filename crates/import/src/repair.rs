//! The import funnel's repair step: values a file got wrong that would otherwise
//! cost the artist the whole model.
//!
//! The bridge copies animation keys, blend-shape offsets and weights, stack
//! ranges and bind matrices straight from the file, and
//! [`ModelData::validate_deform`] — the funnel guard — refuses a model carrying a
//! single non-finite one of them. That is the right guard and it stays the last
//! word, but refusing the *file* for it contradicts the policy the bridge already
//! follows for skin weights and for a stack ufbx cannot bake: one broken piece
//! costs that piece, not the model. So before the guard runs, each piece the
//! guard would refuse is taken out on its own, and a sentence saying so is handed
//! back for the caller to show:
//!
//! * an animation key whose time or value is not a number is dropped (its
//!   neighbours still hold the channel), and a channel whose keys still run
//!   backwards loses its keys altogether;
//! * a clip whose declared range is not a range plays over its keys instead;
//! * a blend shape with any offset that is not a number loses its offsets (the
//!   shape stays, so nothing that indexes shapes moves), a channel's rest weight
//!   that is not a number becomes 0, and an in-between whose target weight is not
//!   one is dropped;
//! * a skin cluster whose bind matrix is not a number stops weighting vertices
//!   (its entry stays, with an identity matrix, so the cluster numbering every
//!   other table uses is unchanged).
//!
//! Nothing is renumbered, which is what keeps this local: the source-property
//! capture refers to clips, channels and clusters by index, and each still means
//! what it did.
//!
//! What is *not* repaired is anything a correct bridge cannot produce — a track
//! naming a node that does not exist, a CSR that does not add up. Those are
//! drifts, and the guard still fails the load on them.

use glam::{Mat4, Quat, Vec3};
use review_model::{AnimationClip, Key, ModelData, MorphData, SkinData};

/// Repair every piece of `model`'s deform data [`ModelData::validate_deform`]
/// would refuse for a value the file got wrong, returning one sentence per
/// repair. Empty for a file that needed none.
pub(crate) fn repair_deform(model: &mut ModelData) -> Vec<String> {
    let mut warnings = Vec::new();
    for clip in &mut model.animations {
        repair_clip(clip, &mut warnings);
    }
    if let Some(morph) = &mut model.morph {
        repair_morph(morph, &mut warnings);
    }
    if let Some(skin) = &mut model.skin {
        repair_skin(skin, &mut warnings);
    }
    warnings
}

/// Drop every key whose time or value is not a number, then — if what is left
/// still runs backwards — every key of the channel. Returns how many went.
fn keep_finite_keys<T>(keys: &mut Vec<Key<T>>, finite: impl Fn(&T) -> bool) -> usize {
    let before = keys.len();
    keys.retain(|key| key.time.is_finite() && finite(&key.value));
    if keys.windows(2).any(|pair| pair[1].time < pair[0].time) {
        keys.clear();
    }
    before - keys.len()
}

fn repair_clip(clip: &mut AnimationClip, warnings: &mut Vec<String>) {
    let mut dropped = 0usize;
    for track in &mut clip.tracks {
        dropped += keep_finite_keys(&mut track.translation, |value: &Vec3| value.is_finite());
        dropped += keep_finite_keys(&mut track.rotation, |value: &Quat| value.is_finite());
        dropped += keep_finite_keys(&mut track.scale, |value: &Vec3| value.is_finite());
    }
    for track in &mut clip.morph_tracks {
        dropped += keep_finite_keys(&mut track.keys, |value: &f32| value.is_finite());
    }
    if dropped > 0 {
        warnings.push(format!(
            "clip '{}': {dropped} animation key(s) whose time or value was not a number were left out",
            clip.name
        ));
    }

    let declared = (clip.time_begin, clip.time_end);
    if !(declared.0.is_finite() && declared.1.is_finite() && declared.1 >= declared.0) {
        // The keys are finite by now, so their extent is a range — or there are
        // none, and the clip is a single frame at zero.
        let (begin, end) = key_range(clip).unwrap_or((0.0, 0.0));
        clip.time_begin = begin;
        clip.time_end = end;
        warnings.push(format!(
            "clip '{}': its declared time range {}..{} is not a range, so it plays over its keys \
             ({begin}..{end}) instead",
            clip.name, declared.0, declared.1
        ));
    }
}

/// The earliest and latest key time anywhere in `clip`.
fn key_range(clip: &AnimationClip) -> Option<(f64, f64)> {
    let times = clip
        .tracks
        .iter()
        .flat_map(|track| {
            track
                .translation
                .iter()
                .map(|key| key.time)
                .chain(track.rotation.iter().map(|key| key.time))
                .chain(track.scale.iter().map(|key| key.time))
        })
        .chain(
            clip.morph_tracks
                .iter()
                .flat_map(|track| track.keys.iter().map(|key| key.time)),
        );
    times.fold(None, |range, time| match range {
        None => Some((time, time)),
        Some((begin, end)) => Some((f64::min(begin, time), f64::max(end, time))),
    })
}

fn repair_morph(morph: &mut MorphData, warnings: &mut Vec<String>) {
    // A shape with any offset that is not a number: every offset of it goes.
    let mut broken = vec![false; morph.shapes.len()];
    for (index, &shape) in morph.shape.iter().enumerate() {
        let finite = morph
            .position
            .get(index)
            .is_some_and(|offset| offset.is_finite())
            && morph
                .normal
                .get(index)
                .is_some_and(|offset| offset.is_finite());
        if !finite && let Some(flag) = broken.get_mut(shape as usize) {
            *flag = true;
        }
    }
    if broken.iter().any(|&flag| flag) {
        let parallel =
            morph.position.len() == morph.shape.len() && morph.normal.len() == morph.shape.len();
        // Only a well-formed CSR is rewritten; a malformed one is a drift the
        // guard reports as it is.
        if parallel && morph.offsets.last().copied().unwrap_or(0) as usize == morph.shape.len() {
            let mut offsets = Vec::with_capacity(morph.offsets.len());
            let mut shape = Vec::with_capacity(morph.shape.len());
            let mut position = Vec::with_capacity(morph.position.len());
            let mut normal = Vec::with_capacity(morph.normal.len());
            offsets.push(0u32);
            for logical in 0..morph.logical_vertex_count() {
                for entry in morph.entry_range(logical) {
                    if broken
                        .get(morph.shape[entry] as usize)
                        .is_some_and(|&flag| flag)
                    {
                        continue;
                    }
                    shape.push(morph.shape[entry]);
                    position.push(morph.position[entry]);
                    normal.push(morph.normal[entry]);
                }
                offsets.push(shape.len() as u32);
            }
            morph.offsets = offsets;
            morph.shape = shape;
            morph.position = position;
            morph.normal = normal;
            for (index, _) in broken.iter().enumerate().filter(|(_, flag)| **flag) {
                warnings.push(format!(
                    "blend shape '{}': offsets that were not numbers, so the shape was left out",
                    morph.shapes[index].name
                ));
            }
        }
    }

    for channel in &mut morph.channels {
        if !channel.rest_weight.is_finite() {
            channel.rest_weight = 0.0;
            warnings.push(format!(
                "blend-shape channel '{}': its weight was not a number and rests at 0",
                channel.name
            ));
        }
        let before = channel.keyframes.len();
        channel
            .keyframes
            .retain(|keyframe| keyframe.target_weight.is_finite());
        if channel.keyframes.len() < before {
            warnings.push(format!(
                "blend-shape channel '{}': {} in-between target(s) whose weight was not a number \
                 were left out",
                channel.name,
                before - channel.keyframes.len()
            ));
        }
    }
}

fn repair_skin(skin: &mut SkinData, warnings: &mut Vec<String>) {
    let broken: Vec<bool> = skin
        .clusters
        .iter()
        .map(|cluster| !cluster.world_to_bone_bind.is_finite())
        .collect();
    if !broken.iter().any(|&flag| flag) {
        return;
    }
    let parallel = skin.weights.len() == skin.bones.len()
        && skin.influence_cluster.len() == skin.bones.len()
        && skin.offsets.last().copied().unwrap_or(0) as usize == skin.bones.len();
    if !parallel {
        return;
    }
    let mut offsets = Vec::with_capacity(skin.offsets.len());
    let mut bones = Vec::with_capacity(skin.bones.len());
    let mut weights = Vec::with_capacity(skin.weights.len());
    let mut influence_cluster = Vec::with_capacity(skin.influence_cluster.len());
    offsets.push(0u32);
    for logical in 0..skin.logical_vertex_count() {
        for influence in skin.influence_range(logical) {
            let cluster = skin.influence_cluster[influence];
            if broken.get(cluster as usize).is_some_and(|&flag| flag) {
                continue;
            }
            bones.push(skin.bones[influence]);
            weights.push(skin.weights[influence]);
            influence_cluster.push(cluster);
        }
        offsets.push(bones.len() as u32);
    }
    skin.offsets = offsets;
    skin.bones = bones;
    skin.weights = weights;
    skin.influence_cluster = influence_cluster;
    for (cluster, _) in skin
        .clusters
        .iter_mut()
        .zip(&broken)
        .filter(|(_, flag)| **flag)
    {
        cluster.world_to_bone_bind = Mat4::IDENTITY;
        warnings.push(format!(
            "skin cluster '{}': its bind matrix was not a number, so its bone no longer moves \
             the mesh",
            cluster.name
        ));
    }
}

#[cfg(test)]
mod tests {
    use review_model::{
        MorphChannel, MorphKeyframe, MorphShape, MorphTrack, NodeTrack, SceneNode, SkinCluster,
    };

    use super::*;

    fn key<T>(time: f64, value: T) -> Key<T> {
        Key { time, value }
    }

    fn model_with(clips: Vec<AnimationClip>) -> ModelData {
        ModelData {
            nodes: vec![SceneNode::default(); 2],
            animations: clips,
            ..ModelData::default()
        }
    }

    #[test]
    fn a_key_that_is_not_a_number_costs_the_key_not_the_file() {
        let clip = AnimationClip {
            name: "Walk".to_owned(),
            time_begin: 0.0,
            time_end: 1.0,
            frame_rate: 30.0,
            tracks: vec![NodeTrack {
                node: 1,
                translation: vec![
                    key(0.0, Vec3::ZERO),
                    key(0.5, Vec3::new(f32::NAN, 0.0, 0.0)),
                    key(f64::NAN, Vec3::ONE),
                    key(1.0, Vec3::ONE),
                ],
                ..NodeTrack::default()
            }],
            morph_tracks: Vec::new(),
        };
        let mut model = model_with(vec![clip]);
        assert!(
            model.validate_deform().is_err(),
            "the guard refuses it as read"
        );

        let warnings = repair_deform(&mut model);
        assert_eq!(model.validate_deform(), Ok(()));
        assert_eq!(model.animations[0].tracks[0].translation.len(), 2);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("Walk") && warnings[0].contains('2'));
    }

    #[test]
    fn a_reversed_or_missing_range_plays_over_the_keys() {
        let clip = |begin: f64, end: f64| AnimationClip {
            name: "Take".to_owned(),
            time_begin: begin,
            time_end: end,
            frame_rate: 30.0,
            tracks: vec![NodeTrack {
                node: 0,
                rotation: vec![key(0.25, Quat::IDENTITY), key(2.0, Quat::IDENTITY)],
                ..NodeTrack::default()
            }],
            morph_tracks: Vec::new(),
        };
        let mut model = model_with(vec![clip(3.0, 1.0), clip(f64::NAN, 1.0), clip(0.0, 1.0)]);
        let warnings = repair_deform(&mut model);
        assert_eq!(model.validate_deform(), Ok(()));
        for repaired in &model.animations[..2] {
            assert_eq!((repaired.time_begin, repaired.time_end), (0.25, 2.0));
        }
        assert_eq!(
            (model.animations[2].time_begin, model.animations[2].time_end),
            (0.0, 1.0),
            "a sound range is left alone"
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn a_blend_shape_that_is_not_a_number_is_left_out_on_its_own() {
        let mut model = ModelData {
            nodes: vec![SceneNode::default()],
            morph: Some(MorphData {
                channels: vec![
                    MorphChannel {
                        name: "smile".to_owned(),
                        mesh_node: 0,
                        rest_weight: f32::NAN,
                        keyframes: vec![MorphKeyframe {
                            shape: 0,
                            target_weight: 1.0,
                        }],
                    },
                    MorphChannel {
                        name: "frown".to_owned(),
                        mesh_node: 0,
                        rest_weight: 0.5,
                        keyframes: vec![
                            MorphKeyframe {
                                shape: 1,
                                target_weight: f32::INFINITY,
                            },
                            MorphKeyframe {
                                shape: 1,
                                target_weight: 1.0,
                            },
                        ],
                    },
                ],
                shapes: vec![
                    MorphShape {
                        name: "smile".to_owned(),
                    },
                    MorphShape {
                        name: "frown".to_owned(),
                    },
                ],
                // Two logical vertices: the first moved by both shapes, the
                // second by the broken one only.
                offsets: vec![0, 2, 3],
                shape: vec![0, 1, 1],
                position: vec![Vec3::X, Vec3::Y, Vec3::Z],
                normal: vec![Vec3::ZERO, Vec3::new(0.0, f32::NAN, 0.0), Vec3::ZERO],
            }),
            corner_to_logical: vec![0, 1],
            vertices: vec![review_model::Vertex::default(); 2],
            ..ModelData::default()
        };
        model.stats.vertex_count = 2;
        assert!(model.validate_deform().is_err());

        let warnings = repair_deform(&mut model);
        assert_eq!(model.validate_deform(), Ok(()));
        let morph = model.morph.as_ref().unwrap();
        assert_eq!(morph.shapes.len(), 2, "no shape is renumbered");
        assert_eq!(morph.offsets, vec![0, 1, 1]);
        assert_eq!(morph.shape, vec![0], "only the intact shape keeps offsets");
        assert_eq!(morph.position, vec![Vec3::X]);
        assert_eq!(morph.channels[0].rest_weight, 0.0);
        assert_eq!(morph.channels[1].keyframes.len(), 1);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn a_bind_matrix_that_is_not_a_number_unbinds_only_its_bone() {
        let cluster = |bone: u32, world_to_bone_bind: Mat4| SkinCluster {
            bone,
            mesh_node: 0,
            world_to_bone_bind,
            mesh_node_to_bone: Mat4::IDENTITY,
            bind_to_world: Mat4::IDENTITY,
            name: format!("bone{bone}"),
        };
        let mut broken = Mat4::IDENTITY;
        broken.x_axis.x = f32::NAN;
        let mut model = ModelData {
            nodes: vec![SceneNode::default(); 3],
            skin: Some(SkinData {
                offsets: vec![0, 2, 3],
                bones: vec![1, 2, 2],
                weights: vec![0.5, 0.5, 1.0],
                influence_cluster: vec![0, 1, 1],
                clusters: vec![cluster(1, Mat4::IDENTITY), cluster(2, broken)],
                deformers: Vec::new(),
            }),
            corner_to_logical: vec![0, 1],
            vertices: vec![review_model::Vertex::default(); 2],
            ..ModelData::default()
        };
        model.stats.vertex_count = 2;
        assert!(model.validate_deform().is_err());

        let warnings = repair_deform(&mut model);
        assert_eq!(model.validate_deform(), Ok(()));
        let skin = model.skin.as_ref().unwrap();
        assert_eq!(skin.clusters.len(), 2, "no cluster is renumbered");
        assert_eq!(skin.offsets, vec![0, 1, 1]);
        assert_eq!(skin.bones, vec![1]);
        assert_eq!(skin.influence_cluster, vec![0]);
        assert_eq!(skin.clusters[1].world_to_bone_bind, Mat4::IDENTITY);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("bone2"));
    }

    #[test]
    fn a_sound_model_needs_no_repair() {
        let mut model = model_with(vec![AnimationClip {
            name: "Idle".to_owned(),
            time_begin: 0.0,
            time_end: 1.0,
            frame_rate: 30.0,
            tracks: Vec::new(),
            morph_tracks: vec![MorphTrack {
                channel: 0,
                keys: vec![key(0.0, 0.0), key(1.0, 1.0)],
            }],
        }]);
        let before = model.clone();
        assert!(repair_deform(&mut model).is_empty());
        assert_eq!(model, before);
    }
}
