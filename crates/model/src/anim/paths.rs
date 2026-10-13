//! Motion paths: where each animated node travels over a clip, for the viewer's
//! animation-curve overlay.
//!
//! A path follows the node's **pivot** — the origin of its `node_to_world`. For
//! a bone that is its joint, the point the skeleton overlay draws it from; for an
//! object it is the point the animator keyed, which is also what a DCC's motion
//! trail follows. It is sampled once per frame of the clip at the clip's own
//! rate: exactly the frames the transport steps through, and the keys
//! themselves, since `ufbx_bake_anim` bakes at that rate.
//!
//! Only nodes the clip can move are walked ([`super::clip_moved_nodes`]: every
//! node it tracks, and everything under one). Of those, two get no path:
//!
//! - a **skinned mesh's own node**, whose transform does not say where its
//!   surface goes — the skin clusters do, through the bones, and those have
//!   paths of their own;
//! - a node whose pivot **holds still** for the whole clip — a bone that only
//!   turns in place, a piece of rubble that never falls. There is no line to
//!   draw, and an empty path would still take a color the eye has to discount.

use glam::Vec3;

use crate::{AnimationClip, ModelData};

use super::{AnimContext, Pose, clip_moved_nodes, evaluate_pose};

/// How far a pivot has to travel, as a fraction of the model's largest extent,
/// before it counts as moving. Anything under it is rounding in a held pose: the
/// baked keys of a constant channel come back a few ulps apart.
const STILL_FRACTION: f32 = 1.0e-4;

/// One node's path through a clip.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodePath {
    /// The node it follows, indexing [`ModelData::nodes`].
    pub node: u32,
    /// The node's pivot in world space at every frame of the clip, first to
    /// last — `points[i]` is frame `i`, at [`AnimationClip::frame_time`].
    pub points: Vec<Vec3>,
}

/// The path of every node `clip` moves, in node order.
///
/// One pose evaluation per frame, so the cost is the clip's frame count times
/// the scene's node count — the mesh is never touched. Built when the overlay is
/// switched on or the clip changes, never per drawn frame.
pub fn clip_paths(model: &ModelData, ctx: &AnimContext, clip: &AnimationClip) -> Vec<NodePath> {
    let moved = clip_moved_nodes(model, ctx, clip);

    // A skinned mesh is moved by its clusters' bones, not by its own node.
    let mut skinned = vec![false; model.nodes.len()];
    if let Some(skin) = &model.skin {
        for cluster in &skin.clusters {
            if let Some(slot) = skinned.get_mut(cluster.mesh_node as usize) {
                *slot = true;
            }
        }
    }

    let candidates: Vec<usize> = (0..model.nodes.len())
        .filter(|&node| moved[node] && !skinned[node])
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    let frames = clip.frame_count();
    let mut points: Vec<Vec<Vec3>> = candidates
        .iter()
        .map(|_| Vec::with_capacity(frames))
        .collect();
    let mut pose = Pose::new(model);
    for frame in 0..frames {
        evaluate_pose(model, ctx, Some(clip), clip.frame_time(frame), &mut pose);
        for (path, &node) in points.iter_mut().zip(&candidates) {
            path.push(pose.world[node].w_axis.truncate());
        }
    }

    let extent = model
        .bounds
        .map_or(1.0, |bounds| bounds.size().max_element())
        .max(f32::MIN_POSITIVE);
    let still = extent * STILL_FRACTION;
    candidates
        .into_iter()
        .zip(points)
        .filter(|(_, points)| travels(points, still))
        .map(|(node, points)| NodePath {
            node: node as u32,
            points,
        })
        .collect()
}

/// Which of `paths` a selection narrows the curves to, as indices into them.
///
/// Selected **bones** name their own paths exactly; failing any bone, selected
/// **nodes** name the paths of everything under them, as a mesh or group
/// selection covers its subtree everywhere else. Bones come first because they
/// are their own selection: the viewer mirrors the primary bone into its scalar
/// selection too, and reading that as a node would widen it to the bone's whole
/// subtree. When the selection names no path at all — the body mesh of a rigged
/// character, a piece of rubble that never moves — every path is shown, so
/// selecting something unanimated leaves the curves as they were. Paths of
/// `hidden` nodes are left out either way.
///
/// `bones`, `nodes` and `hidden` are sorted node indices.
pub fn selected_paths(
    model: &ModelData,
    paths: &[NodePath],
    bones: &[u32],
    nodes: &[u32],
    hidden: &[u32],
) -> Vec<u32> {
    let visible = || {
        paths
            .iter()
            .enumerate()
            .filter(|(_, path)| hidden.binary_search(&path.node).is_err())
    };
    let named: Vec<u32> = if !bones.is_empty() {
        visible()
            .filter(|(_, path)| bones.binary_search(&path.node).is_ok())
            .map(|(index, _)| index as u32)
            .collect()
    } else if !nodes.is_empty() {
        let covered = model.node_subtree_union(nodes);
        visible()
            .filter(|(_, path)| covered.get(path.node as usize).copied().unwrap_or(false))
            .map(|(index, _)| index as u32)
            .collect()
    } else {
        Vec::new()
    };
    if named.is_empty() {
        visible().map(|(index, _)| index as u32).collect()
    } else {
        named
    }
}

/// Whether `points` span more than `still` along any axis.
fn travels(points: &[Vec3], still: f32) -> bool {
    let Some(&first) = points.first() else {
        return false;
    };
    let (min, max) = points.iter().fold((first, first), |(min, max), &point| {
        (min.min(point), max.max(point))
    });
    (max - min).max_element() > still
}

#[cfg(test)]
mod tests {
    use glam::{Mat4, Quat};

    use super::*;
    use crate::anim::node_with_local;
    use crate::{Bounds, Key, LocalTransform, NodeKind, NodeTrack, SkinCluster, SkinData};

    fn local(translation: Vec3) -> LocalTransform {
        LocalTransform {
            translation,
            ..LocalTransform::IDENTITY
        }
    }

    /// A root bone at the origin, a child one metre up and a grandchild a metre
    /// further, with world transforms filled from the composed locals.
    fn chain() -> ModelData {
        let mut model = ModelData {
            nodes: vec![
                node_with_local("root", None, local(Vec3::ZERO)),
                node_with_local("child", Some(0), local(Vec3::Y)),
                node_with_local("grandchild", Some(1), local(Vec3::Y)),
            ],
            bounds: Some(Bounds {
                min: Vec3::splat(-1.0),
                max: Vec3::splat(2.0),
            }),
            ..ModelData::default()
        };
        model.nodes[1].transform = Mat4::from_translation(Vec3::Y);
        model.nodes[2].transform = Mat4::from_translation(Vec3::Y * 2.0);
        for node in &mut model.nodes {
            node.kind = NodeKind::Bone;
        }
        model
    }

    fn clip(tracks: Vec<NodeTrack>) -> AnimationClip {
        AnimationClip {
            name: "clip".to_owned(),
            time_begin: 0.0,
            time_end: 1.0,
            frame_rate: 10.0,
            tracks,
            ..AnimationClip::default()
        }
    }

    /// A tracked node and the untracked child riding it each get a path, one
    /// point per frame of the clip; the parent they hang from does not move and
    /// gets none.
    #[test]
    fn a_moved_node_and_what_rides_it_each_get_a_path() {
        let model = chain();
        let ctx = AnimContext::new(&model);
        let clip = clip(vec![NodeTrack {
            node: 1,
            translation: vec![
                Key {
                    time: 0.0,
                    value: Vec3::Y,
                },
                Key {
                    time: 1.0,
                    value: Vec3::new(1.0, 1.0, 0.0),
                },
            ],
            ..NodeTrack::default()
        }]);

        let paths = clip_paths(&model, &ctx, &clip);
        let nodes: Vec<u32> = paths.iter().map(|path| path.node).collect();
        assert_eq!(nodes, [1, 2], "the root never moves");

        let frames = clip.frame_count();
        assert_eq!(frames, 11);
        for path in &paths {
            assert_eq!(path.points.len(), frames, "one point per frame");
        }
        // The child's pivot slides from (0,1,0) to (1,1,0); the grandchild keeps
        // its metre above it the whole way.
        assert!(paths[0].points[0].abs_diff_eq(Vec3::Y, 1e-6));
        assert!(paths[0].points[10].abs_diff_eq(Vec3::new(1.0, 1.0, 0.0), 1e-6));
        assert!(paths[0].points[5].abs_diff_eq(Vec3::new(0.5, 1.0, 0.0), 1e-6));
        assert!(paths[1].points[10].abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 1e-6));
    }

    /// A node that only turns in place keeps its pivot where it was, so it has no
    /// path — but the child it swings round does.
    #[test]
    fn a_node_turning_in_place_has_no_path_but_its_child_does() {
        let model = chain();
        let ctx = AnimContext::new(&model);
        let clip = clip(vec![NodeTrack {
            node: 1,
            rotation: vec![
                Key {
                    time: 0.0,
                    value: Quat::IDENTITY,
                },
                Key {
                    time: 1.0,
                    value: Quat::from_rotation_z(1.0),
                },
            ],
            ..NodeTrack::default()
        }]);

        let paths = clip_paths(&model, &ctx, &clip);
        let nodes: Vec<u32> = paths.iter().map(|path| path.node).collect();
        assert_eq!(nodes, [2]);
    }

    /// A skinned mesh is moved by its bones, so its own node gets no path even
    /// when the clip moves it.
    #[test]
    fn a_skinned_mesh_node_has_no_path_of_its_own() {
        let mut model = chain();
        model.nodes[2].kind = NodeKind::Mesh;
        model.skin = Some(SkinData {
            clusters: vec![SkinCluster {
                bone: 1,
                mesh_node: 2,
                world_to_bone_bind: Mat4::IDENTITY,
                mesh_node_to_bone: Mat4::IDENTITY,
                bind_to_world: Mat4::IDENTITY,
                name: "child".to_owned(),
            }],
            ..SkinData::default()
        });
        let ctx = AnimContext::new(&model);
        let clip = clip(vec![NodeTrack {
            node: 1,
            translation: vec![
                Key {
                    time: 0.0,
                    value: Vec3::Y,
                },
                Key {
                    time: 1.0,
                    value: Vec3::X,
                },
            ],
            ..NodeTrack::default()
        }]);

        let paths = clip_paths(&model, &ctx, &clip);
        let nodes: Vec<u32> = paths.iter().map(|path| path.node).collect();
        assert_eq!(nodes, [1]);
    }

    /// A clip that tracks nothing moves nothing.
    #[test]
    fn a_clip_with_no_tracks_has_no_paths() {
        let model = chain();
        let ctx = AnimContext::new(&model);
        assert!(clip_paths(&model, &ctx, &clip(Vec::new())).is_empty());
    }

    /// One path per node of `chain`, so a path's index is its node's.
    fn every_node_path() -> Vec<NodePath> {
        (0..3)
            .map(|node| NodePath {
                node,
                points: vec![Vec3::ZERO, Vec3::X],
            })
            .collect()
    }

    /// Selected bones name their own paths and nothing under them.
    #[test]
    fn selected_bones_show_exactly_their_own_paths() {
        let model = chain();
        let paths = every_node_path();
        assert_eq!(selected_paths(&model, &paths, &[1], &[], &[]), [1]);
        assert_eq!(selected_paths(&model, &paths, &[0, 2], &[], &[]), [0, 2]);
        // The scalar selection the viewer mirrors a bone into is not read as a
        // subtree while bones are selected.
        assert_eq!(selected_paths(&model, &paths, &[1], &[1], &[]), [1]);
    }

    /// A node selection covers its subtree, as it does everywhere else.
    #[test]
    fn a_selected_node_shows_the_paths_under_it() {
        let model = chain();
        let paths = every_node_path();
        assert_eq!(selected_paths(&model, &paths, &[], &[1], &[]), [1, 2]);
        assert_eq!(selected_paths(&model, &paths, &[], &[0], &[]), [0, 1, 2]);
    }

    /// Nothing selected, or a selection naming no path, shows them all; hidden
    /// nodes are left out either way.
    #[test]
    fn a_selection_naming_no_path_shows_every_path() {
        let model = chain();
        let paths = every_node_path();
        assert_eq!(selected_paths(&model, &paths, &[], &[], &[]), [0, 1, 2]);
        // Only node 2 has a path; selecting node 0 alone names none of them.
        let one = vec![paths[2].clone()];
        assert_eq!(selected_paths(&model, &one, &[0], &[], &[]), [0]);
        // Hidden: gone whatever the selection says.
        assert_eq!(selected_paths(&model, &paths, &[], &[], &[1]), [0, 2]);
        assert_eq!(selected_paths(&model, &paths, &[1], &[], &[1]), [0, 2]);
    }
}
