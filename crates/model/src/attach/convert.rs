//! Bringing an animation file's keys into the model's units and axes.
//!
//! Import normalizes every file to meters by scaling its scene **root**, so a
//! node's own translation stays in its file's unit: a clip from a meter file,
//! played under a centimeter rig's root, would move a hundredth as far as it
//! should. Import never converts axes at all, so a Z-up file's top-level nodes
//! are posed in Z-up coordinates. Converting the keys here is what makes a clip
//! play the way an engine that imports both files into one coordinate system
//! would show it.
//!
//! Only the keys move. A channel the clip does not key falls back to the
//! model's rest, which is already in the model's units and axes.

use glam::Quat;

use crate::{AnimationClip, ModelData, SceneNode};

/// Two unit figures within this fraction of each other are the same unit: the
/// figure arrives through an `f32`, so a centimeter is not always exactly 0.01.
const SAME_UNIT: f32 = 1e-4;

/// What converting one file's keys into the model's terms takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Conversion {
    /// Multiplies every translation key: one unit of the file, in the model's.
    pub(crate) scale: f32,
    /// Turns a top-level node's keys from the file's axes into the model's.
    /// `None` when the axes agree, when either file declared none, or when the
    /// two are of opposite handedness and no rotation maps one onto the other.
    pub(crate) rotation: Option<Quat>,
}

impl Conversion {
    pub(crate) fn between(model: &ModelData, source: &ModelData) -> Self {
        Self {
            scale: unit_scale(
                model.stats.source_unit_meters,
                source.stats.source_unit_meters,
            )
            .unwrap_or(1.0),
            rotation: match (source.stats.source_axes, model.stats.source_axes) {
                (Some(file), Some(model)) if file != model => file.rotation_to(model),
                _ => None,
            },
        }
    }

    /// Convert `clip`'s keys, which still index `nodes` — the file's own graph,
    /// not yet re-pointed at the model.
    pub(crate) fn apply(&self, clip: &mut AnimationClip, nodes: &[SceneNode]) {
        if self.scale == 1.0 && self.rotation.is_none() {
            return;
        }
        for track in &mut clip.tracks {
            // A node under another is written in its parent's frame, which the
            // model's own parent supplies; only the nodes hanging off the scene
            // root are written in the file's world axes.
            let rotation = self
                .rotation
                .filter(|_| is_top_level(nodes, track.node as usize));
            for key in &mut track.translation {
                key.value *= self.scale;
                if let Some(rotation) = rotation {
                    key.value = rotation * key.value;
                }
            }
            if let Some(rotation) = rotation {
                for key in &mut track.rotation {
                    key.value = (rotation * key.value).normalize();
                }
            }
        }
    }
}

/// Whether a unit figure is one a file actually declared.
pub(crate) fn declared_unit(meters: f32) -> bool {
    meters.is_finite() && meters > 0.0
}

/// The factor taking a length in `file_meters` units to `model_meters` units, or
/// `None` when the two are the same unit or either file declared none.
pub(crate) fn unit_scale(model_meters: f32, file_meters: f32) -> Option<f32> {
    if !declared_unit(model_meters) || !declared_unit(file_meters) {
        return None;
    }
    let scale = file_meters / model_meters;
    ((scale - 1.0).abs() > SAME_UNIT).then_some(scale)
}

/// Whether `node` hangs straight off a scene root, so that its local transform
/// is written in its file's world axes. ufbx's root is the one parentless node.
pub(crate) fn is_top_level(nodes: &[SceneNode], node: usize) -> bool {
    nodes
        .get(node)
        .and_then(|node| node.parent)
        .and_then(|parent| nodes.get(parent))
        .is_none_or(|parent| parent.parent.is_none())
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::anim::node_with_local;
    use crate::{Axis, CoordinateAxes, Key, LocalTransform, NodeTrack};

    const Y_UP: CoordinateAxes = CoordinateAxes {
        right: Axis::PositiveX,
        up: Axis::PositiveY,
        front: Axis::PositiveZ,
    };
    const Z_UP: CoordinateAxes = CoordinateAxes {
        right: Axis::PositiveX,
        up: Axis::PositiveZ,
        front: Axis::NegativeY,
    };

    fn model(unit: f32, axes: CoordinateAxes) -> ModelData {
        let mut model = ModelData {
            nodes: vec![
                node_with_local("", None, LocalTransform::IDENTITY),
                node_with_local("Hip", Some(0), LocalTransform::IDENTITY),
                node_with_local("Spine", Some(1), LocalTransform::IDENTITY),
            ],
            ..ModelData::default()
        };
        model.stats.source_unit_meters = unit;
        model.stats.source_axes = Some(axes);
        model
    }

    fn clip() -> AnimationClip {
        let track = |node| NodeTrack {
            node,
            translation: vec![Key {
                time: 0.0,
                value: Vec3::new(0.0, 0.0, 1.0),
            }],
            rotation: vec![Key {
                time: 0.0,
                value: Quat::IDENTITY,
            }],
            ..NodeTrack::default()
        };
        AnimationClip {
            tracks: vec![track(1), track(2)],
            ..AnimationClip::default()
        }
    }

    #[test]
    fn a_meter_file_on_a_centimeter_rig_moves_a_hundred_times_as_far() {
        let rig = model(0.01, Y_UP);
        let file = model(1.0, Y_UP);
        let conversion = Conversion::between(&rig, &file);
        assert!((conversion.scale - 100.0).abs() < 1e-3);
        assert_eq!(conversion.rotation, None);
        let mut clip = clip();
        conversion.apply(&mut clip, &file.nodes);
        for track in &clip.tracks {
            assert!((track.translation[0].value.z - 100.0).abs() < 1e-3);
        }
    }

    #[test]
    fn the_same_unit_and_axes_leave_the_keys_alone() {
        let conversion = Conversion::between(&model(0.01, Y_UP), &model(0.01, Y_UP));
        assert_eq!(
            conversion,
            Conversion {
                scale: 1.0,
                rotation: None
            }
        );
        // An undeclared unit is never a reason to rescale.
        let conversion = Conversion::between(&model(0.01, Y_UP), &model(0.0, Y_UP));
        assert_eq!(conversion.scale, 1.0);
    }

    /// A Z-up file's up is the rig's up once converted, and only the node
    /// hanging off the root turns: the spine is written in the hip's frame.
    #[test]
    fn a_z_up_file_turns_its_top_level_nodes_into_the_rigs_axes() {
        let rig = model(0.01, Y_UP);
        let file = model(0.01, Z_UP);
        let conversion = Conversion::between(&rig, &file);
        let rotation = conversion.rotation.expect("both right-handed");
        assert!((rotation * Vec3::Z).abs_diff_eq(Vec3::Y, 1e-6), "up to up");
        assert!(
            (rotation * Vec3::NEG_Y).abs_diff_eq(Vec3::Z, 1e-6),
            "front to front"
        );

        let mut clip = clip();
        conversion.apply(&mut clip, &file.nodes);
        let [hip, spine] = &clip.tracks[..] else {
            panic!("two tracks");
        };
        assert!(hip.translation[0].value.abs_diff_eq(Vec3::Y, 1e-6));
        assert!(
            (hip.rotation[0].value * Vec3::Z).abs_diff_eq(Vec3::Y, 1e-6),
            "the hip's frame turns with it"
        );
        assert_eq!(spine.translation[0].value, Vec3::Z);
        assert_eq!(spine.rotation[0].value, Quat::IDENTITY);
    }

    #[test]
    fn opposite_handedness_is_not_converted() {
        let left_handed = CoordinateAxes {
            front: Axis::NegativeZ,
            ..Y_UP
        };
        assert_eq!(Y_UP.rotation_to(left_handed), None);
        let conversion = Conversion::between(&model(0.01, Y_UP), &model(0.01, left_handed));
        assert_eq!(conversion.rotation, None);
    }
}
