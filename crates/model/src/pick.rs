//! Picking a posed mesh: the ray query the viewport's click-to-select runs
//! against what is actually on screen.
//!
//! [`SceneBvh`] indexes [`ModelData::vertices`], which are baked in the *bind*
//! pose, while the drawn mesh is deformed on the GPU by the pose's palette
//! (`deform_corner` is the CPU mirror of that vertex shader). Picking the BVH
//! directly is therefore correct only for a model at rest — on anything posed it
//! hits where the limb *was*.
//!
//! This closes that gap without re-deforming the whole mesh per ray. Each part
//! is classified once per pose:
//!
//! * **Rest** — no skin, no morph, an identity node delta. Query the tree as
//!   built.
//! * **Rigid** — the whole part rides one node matrix. Push the *ray* through
//!   the inverse instead of the geometry through the forward map; an affine map
//!   leaves the ray parameter `t` alone (`inv(o + t·d) = inv(o) + t·inv_vec(d)`),
//!   so hits stay directly comparable with every other part's. Costs nothing per
//!   pose.
//! * **Deformed** — skinned or blend-shaped. The tree's *topology* survives a
//!   deform even though its boxes do not, so the boxes are refit from the posed
//!   corners once per pose (`Bvh::refit`) and the triangle test deforms its
//!   three corners on demand. A pick touches a few dozen triangles, so deforming
//!   lazily there is far cheaper than materialising every posed position.

use glam::{Mat4, Vec3};

use crate::ModelData;
use crate::anim::{AnimContext, DeformPose, deform_corner};
use crate::bvh::{SceneBvh, SceneHit};

/// How a part's geometry moves under the current pose.
#[derive(Debug, Clone)]
enum PartPick {
    /// Unmoved: query the rest tree as built.
    Rest,
    /// Rigid on one node — the ray is pushed through this inverse delta.
    Rigid { inverse: Mat4 },
    /// Skinned / blend-shaped: boxes refit for this pose, corners deformed on
    /// demand by the triangle test.
    Deformed { bounds: Vec<(Vec3, Vec3)> },
}

/// A [`SceneBvh`] projected onto one pose, so a pick hits the mesh as drawn.
///
/// Build it for a pose and reuse it across rays (hover casts one per frame);
/// rebuild when the pose or the model changes.
#[derive(Debug, Clone)]
pub struct PosedScene {
    /// Parallel to the [`SceneBvh`]'s parts, so the two are only ever used
    /// together — a mismatch would silently query the wrong tree.
    parts: Vec<PartPick>,
}

impl PosedScene {
    /// Classify every part of `bvh` under `deform`, refitting the boxes of the
    /// ones that actually deform. Cost is proportional to the *deforming*
    /// geometry: a rigid or unmoved part costs nothing beyond its classification.
    pub fn build(
        bvh: &SceneBvh,
        model: &ModelData,
        ctx: &AnimContext,
        deform: &DeformPose,
    ) -> Self {
        let node_count = model.nodes.len();
        let parts = bvh
            .parts()
            .iter()
            .map(|part| {
                if part_deforms(model, part.bvh.triangles()) {
                    let bounds = part
                        .bvh
                        .refit(|tri| deformed_triangle(model, ctx, deform, tri));
                    return PartPick::Deformed { bounds };
                }
                // Rigid on its own node, if it has one and it moved.
                let delta = ((part.node as usize) < node_count)
                    .then(|| deform.palette.get(part.node as usize).copied())
                    .flatten();
                match delta {
                    Some(matrix) if matrix != Mat4::IDENTITY => PartPick::Rigid {
                        inverse: crate::anim::invert_or_identity(matrix),
                    },
                    _ => PartPick::Rest,
                }
            })
            .collect();
        Self { parts }
    }

    /// The nearest triangle the ray hits on the *posed* mesh, across every part
    /// `allow` accepts. `bvh` must be the hierarchy this was built from.
    ///
    /// Mirrors [`SceneBvh::pick`]'s contract exactly, including that each part's
    /// search is bounded by the best hit so far.
    #[expect(
        clippy::too_many_arguments,
        reason = "a ray, its bounds, the pose it is cast against and the tree it                   indexes - each already the smallest thing it could be"
    )]
    pub fn pick(
        &self,
        bvh: &SceneBvh,
        model: &ModelData,
        ctx: &AnimContext,
        deform: &DeformPose,
        origin: Vec3,
        dir: Vec3,
        t_min: f32,
        t_max: f32,
        allow: impl Fn(u32) -> bool,
    ) -> Option<SceneHit> {
        let mut best: Option<SceneHit> = None;
        let mut far = t_max;
        for (part, state) in bvh.parts().iter().zip(&self.parts) {
            if !allow(part.node) {
                continue;
            }
            let hit = match state {
                PartPick::Rest => part.bvh.closest_hit(model, origin, dir, t_min, far),
                PartPick::Rigid { inverse } => {
                    // `t` is invariant under the affine map, so the hit needs no
                    // correction — only the ray is moved into rest space.
                    let rest_origin = inverse.transform_point3(origin);
                    let rest_dir = inverse.transform_vector3(dir);
                    part.bvh
                        .closest_hit(model, rest_origin, rest_dir, t_min, far)
                }
                PartPick::Deformed { bounds } => {
                    part.bvh
                        .closest_hit_with(origin, dir, t_min, far, Some(bounds), |tri| {
                            deformed_triangle(model, ctx, deform, tri)
                        })
                }
            };
            if let Some(hit) = hit {
                far = hit.t;
                best = Some(SceneHit {
                    node: part.node,
                    triangle: hit.triangle,
                    t: hit.t,
                });
            }
        }
        best
    }
}

/// Whether any corner of these triangles is skinned or blend-shaped — i.e.
/// whether the part moves by anything other than its own node matrix.
///
/// Mirrors [`deform_corner`]'s first two sources. The scan exits on the first
/// such corner, so the common case (a skinned character) is O(1); only a part
/// that turns out to be rigid pays for its own length, and then only in a model
/// that carries a skin or morph at all.
fn part_deforms(model: &ModelData, triangles: &[u32]) -> bool {
    if model.skin.is_none() && model.morph.is_none() {
        return false;
    }
    triangles.iter().any(|&tri| {
        let base = tri as usize * 3;
        (0..3).any(|slot| {
            let Some(&corner) = model.indices.get(base + slot) else {
                return false;
            };
            let Some(&logical) = model.corner_to_logical.get(corner as usize) else {
                return false;
            };
            let logical = logical as usize;
            let skinned = model
                .skin
                .as_ref()
                .is_some_and(|skin| !skin.influence_range(logical).is_empty());
            let shaped = model
                .morph
                .as_ref()
                .is_some_and(|morph| !morph.entry_range(logical).is_empty());
            skinned || shaped
        })
    })
}

/// A triangle's three corner positions under the pose, through the same
/// [`deform_corner`] the shader mirrors. A missing index reads as the origin,
/// matching [`crate::triangle_positions`].
fn deformed_triangle(
    model: &ModelData,
    ctx: &AnimContext,
    deform: &DeformPose,
    tri: u32,
) -> [Vec3; 3] {
    let base = tri as usize * 3;
    let position = |slot: usize| {
        model
            .indices
            .get(base + slot)
            .map(|&corner| deform_corner(model, ctx, deform, corner as usize).0)
            .unwrap_or(Vec3::ZERO)
    };
    [position(0), position(1), position(2)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{Pose, build_palette, evaluate_pose};
    use crate::{
        AnimationClip, Key, LocalTransform, ModelStats, NodeKind, NodeTrack, SceneNode,
        SkinCluster, SkinData, TriangleData, Vertex,
    };

    const SWING: f32 = 5.0;

    fn node(name: &str, parent: Option<usize>, kind: NodeKind) -> SceneNode {
        SceneNode {
            name: name.to_string(),
            parent,
            transform: Mat4::IDENTITY,
            rest_local: LocalTransform::IDENTITY,
            kind,
            ..SceneNode::default()
        }
    }

    fn vertex(position: Vec3) -> Vertex {
        Vertex {
            position,
            ..Vertex::default()
        }
    }

    /// A unit quad on the ground plane owned by node 0, with a two-bone chain
    /// beside it and one clip that swings the chain `SWING` metres along +X.
    ///
    /// `skinned` binds every vertex fully to the tip bone, so the quad rides the
    /// clip; otherwise the quad is rigid on its own node, which the clip drives
    /// instead — the two classifications this module makes.
    fn quad_model(skinned: bool) -> ModelData {
        let mut model = ModelData {
            nodes: vec![
                node("mesh", None, NodeKind::Mesh),
                node("root", None, NodeKind::Bone),
                node("tip", Some(1), NodeKind::Bone),
            ],
            vertices: vec![
                vertex(Vec3::new(-0.5, 0.0, -0.5)),
                vertex(Vec3::new(0.5, 0.0, -0.5)),
                vertex(Vec3::new(0.5, 0.0, 0.5)),
                vertex(Vec3::new(-0.5, 0.0, 0.5)),
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            corner_to_logical: vec![0, 1, 2, 3],
            ..ModelData::default()
        };
        model.nodes[0].mesh_part = Some(0);
        model.triangles = TriangleData {
            to_face: Vec::new(),
            material: Vec::new(),
            node: vec![0, 0],
        };
        model.stats = ModelStats {
            vertex_count: 4,
            ..ModelStats::default()
        };
        // The clip drives whatever carries the quad: the tip bone when skinned,
        // the mesh node itself when not.
        let driven = if skinned { 2 } else { 0 };
        model.animations = vec![AnimationClip {
            time_begin: 0.0,
            time_end: 1.0,
            tracks: vec![NodeTrack {
                node: driven,
                translation: vec![
                    Key {
                        time: 0.0,
                        value: Vec3::ZERO,
                    },
                    Key {
                        time: 1.0,
                        value: Vec3::X * SWING,
                    },
                ],
                ..NodeTrack::default()
            }],
            ..AnimationClip::default()
        }];
        if skinned {
            // Rest equals bind, so the bind inverse is the identity.
            model.skin = Some(SkinData {
                offsets: vec![0, 1, 2, 3, 4],
                bones: vec![2, 2, 2, 2],
                weights: vec![1.0, 1.0, 1.0, 1.0],
                influence_cluster: vec![0, 0, 0, 0],
                clusters: vec![SkinCluster {
                    bone: 2,
                    mesh_node: 0,
                    world_to_bone_bind: Mat4::IDENTITY,
                    mesh_node_to_bone: Mat4::IDENTITY,
                    bind_to_world: Mat4::IDENTITY,
                    name: String::new(),
                }],
                deformers: Vec::new(),
            });
            assert_eq!(model.validate_deform(), Ok(()));
        }
        model
    }

    /// Pose `model` at `time` on its clip and hand back what a pick needs.
    fn posed(model: &ModelData, time: f64) -> (AnimContext, DeformPose) {
        let ctx = AnimContext::new(model);
        let mut pose = Pose::new(model);
        evaluate_pose(model, &ctx, model.animations.first(), time, &mut pose);
        let mut deform = DeformPose::default();
        build_palette(model, &ctx, &pose, &mut deform);
        (ctx, deform)
    }

    /// Straight down onto `x` from well above the quad.
    fn ray_at(x: f32) -> (Vec3, Vec3) {
        (Vec3::new(x, 10.0, 0.0), Vec3::NEG_Y)
    }

    fn hit(
        scene: &PosedScene,
        bvh: &SceneBvh,
        model: &ModelData,
        ctx: &AnimContext,
        deform: &DeformPose,
        x: f32,
    ) -> Option<SceneHit> {
        let (origin, dir) = ray_at(x);
        scene.pick(
            bvh,
            model,
            ctx,
            deform,
            origin,
            dir,
            0.0,
            f32::INFINITY,
            |_| true,
        )
    }

    /// The bug this module exists for: with the clip at its end, the quad is
    /// drawn `SWING` along +X, and the pick must follow it rather than the
    /// bind-pose buffers the BVH indexes.
    #[test]
    fn a_skinned_part_is_picked_where_the_pose_put_it() {
        let model = quad_model(true);
        let bvh = SceneBvh::build(&model);
        let (ctx, deform) = posed(&model, 1.0);
        let scene = PosedScene::build(&bvh, &model, &ctx, &deform);
        assert!(matches!(scene.parts[0], PartPick::Deformed { .. }));

        assert!(
            hit(&scene, &bvh, &model, &ctx, &deform, SWING).is_some(),
            "the posed quad must be pickable where it is drawn"
        );
        assert!(
            hit(&scene, &bvh, &model, &ctx, &deform, 0.0).is_none(),
            "the posed pick must not hit where the mesh no longer is"
        );
        // ...which is exactly where the rest-pose tree still answers "hit".
        let (origin, dir) = ray_at(0.0);
        assert!(
            bvh.pick(&model, origin, dir, 0.0, f32::INFINITY, |_| true)
                .is_some(),
            "the stale answer this module replaces"
        );
    }

    /// A part carried by its own node moves through the palette instead, which
    /// the pick handles by pushing the *ray* through the inverse.
    #[test]
    fn a_rigid_part_is_picked_through_its_node_delta() {
        let model = quad_model(false);
        let bvh = SceneBvh::build(&model);
        let (ctx, deform) = posed(&model, 1.0);
        let scene = PosedScene::build(&bvh, &model, &ctx, &deform);
        assert!(matches!(scene.parts[0], PartPick::Rigid { .. }));

        let moved = hit(&scene, &bvh, &model, &ctx, &deform, SWING)
            .expect("the rigid quad must be pickable where its node put it");
        assert!(hit(&scene, &bvh, &model, &ctx, &deform, 0.0).is_none());
        // `t` is the world distance travelled, unchanged by the rest-space
        // detour: the quad sits at y = 0, the ray starts 10 up with a unit
        // direction.
        assert!((moved.t - 10.0).abs() < 1e-4, "{}", moved.t);
        assert_eq!(moved.node, 0);
    }

    /// At rest every classification must agree with the plain rest-pose query.
    #[test]
    fn an_unposed_model_matches_the_plain_query() {
        for skinned in [false, true] {
            let model = quad_model(skinned);
            let bvh = SceneBvh::build(&model);
            let (ctx, deform) = posed(&model, 0.0);
            let scene = PosedScene::build(&bvh, &model, &ctx, &deform);
            let (origin, dir) = ray_at(0.0);
            let plain = bvh.pick(&model, origin, dir, 0.0, f32::INFINITY, |_| true);
            let through_pose = hit(&scene, &bvh, &model, &ctx, &deform, 0.0);
            assert_eq!(plain, through_pose, "skinned = {skinned}");
            assert!(plain.is_some());
        }
    }

    /// A model with neither skin nor morph and no motion classifies as Rest, so
    /// it costs nothing per pose.
    #[test]
    fn a_static_model_classifies_as_rest() {
        let model = crate::demo_cube_model();
        let bvh = SceneBvh::build(&model);
        let ctx = AnimContext::new(&model);
        let mut pose = Pose::new(&model);
        evaluate_pose(&model, &ctx, None, 0.0, &mut pose);
        let mut deform = DeformPose::default();
        build_palette(&model, &ctx, &pose, &mut deform);
        let scene = PosedScene::build(&bvh, &model, &ctx, &deform);
        assert!(
            scene
                .parts
                .iter()
                .all(|part| matches!(part, PartPick::Rest))
        );
    }

    /// The refit boxes must contain every posed corner, or the traversal would
    /// prune away a triangle the ray really crosses.
    #[test]
    fn refit_bounds_contain_every_posed_corner() {
        let model = quad_model(true);
        let bvh = SceneBvh::build(&model);
        let (ctx, deform) = posed(&model, 1.0);

        for part in bvh.parts() {
            let bounds = part
                .bvh
                .refit(|tri| deformed_triangle(&model, &ctx, &deform, tri));
            let (lo, hi) = bounds[0];
            for &tri in part.bvh.triangles() {
                for corner in deformed_triangle(&model, &ctx, &deform, tri) {
                    assert!(
                        corner.cmpge(lo).all() && corner.cmple(hi).all(),
                        "posed corner {corner:?} outside refit root box {lo:?}..{hi:?}"
                    );
                }
            }
        }
    }
}
