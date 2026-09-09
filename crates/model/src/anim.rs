//! Pose evaluation for animation clips, and the deform palette the renderer
//! skins with.
//!
//! Everything here is pure `glam` math over [`ModelData`] (invariant 10) and is
//! the single definition of what a pose *means*: the GPU vertex shader, the
//! per-clip bounds measured at import and the tests all reproduce
//! [`deform_corner`]. A pose is a world matrix per node plus a weight per blend
//! shape; the palette derived from it is a delta over the *baked* vertex — node
//! entries move rigid geometry (and the skeleton overlay), cluster entries skin.

use glam::{Mat4, Quat, Vec3};

use crate::{
    AnimationClip, Bounds, Key, ModelData, MorphChannel, MorphKeyframe, SceneNode,
    frame_rate_or_default,
};

/// Everything about a model that pose evaluation needs but that never changes
/// between poses: the parents-first node order, the inverse rest world matrices,
/// and the corner / node lookups that project a pose onto the mesh. Build once
/// per model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnimContext {
    /// Node indices in an order where every parent precedes its children.
    order: Vec<usize>,
    /// `inverse(SceneNode::transform)` per node — what turns a posed world
    /// matrix into the delta a baked vertex needs.
    rest_world_inverse: Vec<Mat4>,
    /// Per corner, the node whose mesh it belongs to (`u32::MAX` when the model
    /// carries no per-triangle node info, or the corner is unreferenced).
    corner_node: Vec<u32>,
    /// Per logical vertex, one corner expanded from it (`u32::MAX` when none).
    logical_corner: Vec<u32>,
}

impl AnimContext {
    pub fn new(model: &ModelData) -> Self {
        let node_count = model.nodes.len();

        // Parents first: repeatedly emit nodes whose parent is already emitted.
        // Any node whose parent chain is cyclic or dangling is appended at the
        // end and evaluated as a root, so a malformed file still poses.
        let mut order = Vec::with_capacity(node_count);
        let mut placed = vec![false; node_count];
        let mut progressed = true;
        while progressed && order.len() < node_count {
            progressed = false;
            for (index, node) in model.nodes.iter().enumerate() {
                if placed[index] {
                    continue;
                }
                let ready = match node.parent {
                    None => true,
                    Some(parent) => parent < node_count && parent != index && placed[parent],
                };
                if ready {
                    placed[index] = true;
                    order.push(index);
                    progressed = true;
                }
            }
        }
        for (index, done) in placed.iter().enumerate() {
            if !done {
                order.push(index);
            }
        }

        let rest_world_inverse = model
            .nodes
            .iter()
            .map(|node| invert_affine(node.transform))
            .collect();

        let mut corner_node = vec![u32::MAX; model.vertices.len()];
        let triangle_count = model.indices.len() / 3;
        if model.triangles.node.len() == triangle_count {
            for (triangle, corners) in model.indices.as_chunks::<3>().0.iter().enumerate() {
                for &corner in corners {
                    if let Some(slot) = corner_node.get_mut(corner as usize) {
                        *slot = model.triangles.node[triangle];
                    }
                }
            }
        }

        let mut logical_corner = vec![u32::MAX; model.stats.vertex_count];
        for (corner, &logical) in model.corner_to_logical.iter().enumerate() {
            if let Some(slot) = logical_corner.get_mut(logical as usize)
                && *slot == u32::MAX
            {
                *slot = corner as u32;
            }
        }

        Self {
            order,
            rest_world_inverse,
            corner_node,
            logical_corner,
        }
    }

    /// Node indices, parents before children.
    pub fn order(&self) -> &[usize] {
        &self.order
    }
}

/// A singular rest matrix (a zero-scaled node) has no inverse; fall back to the
/// identity so the node simply doesn't move rather than poisoning the palette
/// with NaNs.
fn invert_affine(matrix: Mat4) -> Mat4 {
    let determinant = matrix.determinant();
    if determinant.abs() > 1e-20 && determinant.is_finite() {
        let inverse = matrix.inverse();
        if inverse.is_finite() {
            return inverse;
        }
    }
    Mat4::IDENTITY
}

/// One evaluated pose: a world matrix per node and a weight per blend shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pose {
    /// `node_to_world` per node, parallel to [`ModelData::nodes`].
    pub world: Vec<Mat4>,
    /// Per blend-shape channel, its evaluated weight (parallel to
    /// [`crate::MorphData::channels`]).
    pub channel_weights: Vec<f32>,
    /// Per blend shape, the summed effective weight every channel contributes
    /// (parallel to [`crate::MorphData::shapes`]).
    pub shape_weights: Vec<f32>,
}

impl Pose {
    pub fn new(model: &ModelData) -> Self {
        let (channels, shapes) = model
            .morph
            .as_ref()
            .map_or((0, 0), |morph| (morph.channels.len(), morph.shapes.len()));
        Self {
            world: vec![Mat4::IDENTITY; model.nodes.len()],
            channel_weights: vec![0.0; channels],
            shape_weights: vec![0.0; shapes],
        }
    }
}

/// Evaluate `clip` at `time` (or the rest pose when `clip` is `None`) into
/// `out`, resizing it to the model. Every node's world is recomposed from its
/// rest local transform with the clip's keys substituted channel by channel —
/// a node the clip never touches still moves with an animated ancestor.
pub fn evaluate_pose(
    model: &ModelData,
    ctx: &AnimContext,
    clip: Option<&AnimationClip>,
    time: f64,
    out: &mut Pose,
) {
    let node_count = model.nodes.len();
    out.world.clear();
    out.world.resize(node_count, Mat4::IDENTITY);

    // Track lookup per node for this clip (`None` = keep the rest transform).
    let mut track_of = vec![None; node_count];
    if let Some(clip) = clip {
        for track in &clip.tracks {
            if let Some(slot) = track_of.get_mut(track.node as usize) {
                *slot = Some(track);
            }
        }
    }

    for &index in &ctx.order {
        let Some(node) = model.nodes.get(index) else {
            continue;
        };
        let local = match track_of[index] {
            Some(track) => {
                let rest = node.rest_local;
                let translation = sample_vec3(&track.translation, time, rest.translation);
                let rotation = sample_quat(&track.rotation, time, rest.rotation);
                let scale = sample_vec3(&track.scale, time, rest.scale);
                Mat4::from_scale_rotation_translation(scale, rotation, translation)
            }
            None => node.rest_local.to_mat4(),
        };
        let parent_world = node
            .parent
            .filter(|&parent| parent < node_count && parent != index)
            .map_or(Mat4::IDENTITY, |parent| out.world[parent]);
        out.world[index] = parent_world * local;
    }

    // Blend shapes: the channel weight (keyed or rest) → per-keyframe effective
    // weights → summed per shape.
    let (channel_count, shape_count) = model
        .morph
        .as_ref()
        .map_or((0, 0), |morph| (morph.channels.len(), morph.shapes.len()));
    out.channel_weights.clear();
    out.channel_weights.resize(channel_count, 0.0);
    out.shape_weights.clear();
    out.shape_weights.resize(shape_count, 0.0);
    if let Some(morph) = &model.morph {
        for (index, channel) in morph.channels.iter().enumerate() {
            out.channel_weights[index] = channel.rest_weight;
        }
        if let Some(clip) = clip {
            for track in &clip.morph_tracks {
                if let Some(slot) = out.channel_weights.get_mut(track.channel as usize) {
                    *slot = sample_scalar(&track.keys, time, *slot);
                }
            }
        }
        let mut effective = Vec::new();
        for (channel, &weight) in morph.channels.iter().zip(&out.channel_weights) {
            channel_effective_weights(channel, weight, &mut effective);
            for (key, &effective_weight) in channel.keyframes.iter().zip(&effective) {
                if effective_weight == 0.0 {
                    continue;
                }
                if let Some(slot) = out.shape_weights.get_mut(key.shape as usize) {
                    *slot += effective_weight;
                }
            }
        }
    }
}

/// The file's default pose: every node at its rest transform, every channel at
/// its rest weight.
pub fn rest_pose(model: &ModelData, ctx: &AnimContext, out: &mut Pose) {
    evaluate_pose(model, ctx, None, 0.0, out);
}

/// Sample a piecewise-linear channel at `time`, holding the end values outside
/// the key range; `rest` when the channel has no keys.
fn sample_vec3(keys: &[Key<Vec3>], time: f64, rest: Vec3) -> Vec3 {
    match segment(keys, time) {
        Segment::None => rest,
        Segment::At(index) => keys[index].value,
        Segment::Between(a, b, t) => keys[a].value.lerp(keys[b].value, t),
    }
}

fn sample_scalar(keys: &[Key<f32>], time: f64, rest: f32) -> f32 {
    match segment(keys, time) {
        Segment::None => rest,
        Segment::At(index) => keys[index].value,
        Segment::Between(a, b, t) => keys[a].value + (keys[b].value - keys[a].value) * t,
    }
}

/// Rotations interpolate spherically along the shorter arc.
fn sample_quat(keys: &[Key<Quat>], time: f64, rest: Quat) -> Quat {
    match segment(keys, time) {
        Segment::None => rest,
        Segment::At(index) => keys[index].value.normalize(),
        Segment::Between(a, b, t) => {
            let from = keys[a].value.normalize();
            let mut to = keys[b].value.normalize();
            if from.dot(to) < 0.0 {
                to = -to;
            }
            from.slerp(to, t).normalize()
        }
    }
}

enum Segment {
    None,
    At(usize),
    Between(usize, usize, f32),
}

/// Locate `time` among `keys` (sorted by time): the exact key, or the pair it
/// falls between with the interpolation factor. Outside the range → the end key.
fn segment<T>(keys: &[Key<T>], time: f64) -> Segment {
    if keys.is_empty() {
        return Segment::None;
    }
    // First key strictly after `time`.
    let next = keys.partition_point(|key| key.time <= time);
    if next == 0 {
        return Segment::At(0);
    }
    if next >= keys.len() {
        return Segment::At(keys.len() - 1);
    }
    let prev = next - 1;
    let span = keys[next].time - keys[prev].time;
    if span <= 0.0 {
        return Segment::At(prev);
    }
    let t = ((time - keys[prev].time) / span).clamp(0.0, 1.0) as f32;
    Segment::Between(prev, next, t)
}

/// The per-keyframe effective weights of `channel` at channel weight `weight`
/// — a verbatim port of ufbx's `ufbxi_update_blend_channel`, so in-between
/// targets blend exactly as ufbx (and the DCC) blend them. `out` is resized to
/// the channel's keyframe count. With a single keyframe at target weight 1 the
/// result is simply `[weight]`.
pub fn channel_effective_weights(channel: &MorphChannel, weight: f32, out: &mut Vec<f32>) {
    let keys: &[MorphKeyframe] = &channel.keyframes;
    out.clear();
    out.resize(keys.len(), 0.0);
    if keys.is_empty() {
        return;
    }

    // The split around zero: the last keyframe with a negative target.
    let last_negative: isize = keys
        .iter()
        .rposition(|key| key.target_weight < 0.0)
        .map_or(-1, |index| index as isize);

    // `None` stands for ufbx's implicit zero key (target weight 0).
    let mut prev: Option<usize> = None;
    let mut next: Option<usize> = None;
    if weight > 0.0 {
        if last_negative >= 0 {
            prev = Some(last_negative as usize);
        }
        for (index, key) in keys.iter().enumerate().skip((last_negative + 1) as usize) {
            prev = next;
            next = Some(index);
            if key.target_weight > weight {
                break;
            }
        }
    } else {
        if ((last_negative + 1) as usize) < keys.len() {
            prev = Some((last_negative + 1) as usize);
        }
        let mut index = last_negative;
        while index >= 0 {
            prev = next;
            next = Some(index as usize);
            if keys[index as usize].target_weight < weight {
                break;
            }
            index -= 1;
        }
    }

    let target = |key: Option<usize>| key.map_or(0.0, |index| keys[index].target_weight);
    let delta = target(next) - target(prev);
    if delta != 0.0 {
        let t = (weight - target(prev)) / delta;
        if let Some(index) = prev {
            out[index] = 1.0 - t;
        }
        if let Some(index) = next {
            out[index] = t;
        }
    }
}

/// The GPU-ready form of a pose: one delta matrix per palette entry and one
/// weight per blend shape. Palette layout — entries `[0, node_count)` are the
/// nodes' rigid deltas (`world × inverse(rest_world)`, identity at rest),
/// followed by one entry per skin cluster (`bone_world × world_to_bone_bind`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeformPose {
    pub palette: Vec<Mat4>,
    pub shape_weights: Vec<f32>,
}

/// Number of palette entries `model` needs: its nodes plus its skin clusters.
pub fn palette_len(model: &ModelData) -> usize {
    model.nodes.len() + model.skin.as_ref().map_or(0, |skin| skin.clusters.len())
}

/// Turn a pose into the palette the shader multiplies by.
pub fn build_palette(model: &ModelData, ctx: &AnimContext, pose: &Pose, out: &mut DeformPose) {
    out.palette.clear();
    out.palette.reserve(palette_len(model));
    for (index, _) in model.nodes.iter().enumerate() {
        let world = pose.world.get(index).copied().unwrap_or(Mat4::IDENTITY);
        let inverse = ctx
            .rest_world_inverse
            .get(index)
            .copied()
            .unwrap_or(Mat4::IDENTITY);
        out.palette.push(world * inverse);
    }
    if let Some(skin) = &model.skin {
        for cluster in &skin.clusters {
            let bone_world = pose
                .world
                .get(cluster.bone as usize)
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            out.palette.push(bone_world * cluster.world_to_bone_bind);
        }
    }
    out.shape_weights.clear();
    out.shape_weights.extend_from_slice(&pose.shape_weights);
}

/// ufbx skips the normalisation when the weights already sum to one within
/// this epsilon (`UFBX_EPSILON`).
const WEIGHT_SUM_EPSILON: f32 = 1.0e-6;

/// The CPU reference for what the vertex shader does to render vertex `corner`
/// under `deform`: blend-shape offsets first, then the weighted palette blend
/// (normalised by the summed weight as ufbx does), falling back to the node's
/// rigid entry for a corner with no skin influences. Returns the deformed
/// position and (unnormalised-input, normalised-output) normal.
pub fn deform_corner(
    model: &ModelData,
    ctx: &AnimContext,
    deform: &DeformPose,
    corner: usize,
) -> (Vec3, Vec3) {
    let Some(vertex) = model.vertices.get(corner) else {
        return (Vec3::ZERO, Vec3::Y);
    };
    let mut position = vertex.position;
    let mut normal = vertex.normal;
    let logical = model.corner_to_logical.get(corner).map(|&v| v as usize);

    if let (Some(morph), Some(logical)) = (&model.morph, logical) {
        for entry in morph.entry_range(logical) {
            let weight = deform
                .shape_weights
                .get(morph.shape[entry] as usize)
                .copied()
                .unwrap_or(0.0);
            if weight != 0.0 {
                position += morph.position[entry] * weight;
                normal += morph.normal[entry] * weight;
            }
        }
    }

    let node_count = model.nodes.len();
    let mut blended_position = Vec3::ZERO;
    let mut blended_normal = Vec3::ZERO;
    let mut total = 0.0_f32;
    if let (Some(skin), Some(logical)) = (&model.skin, logical) {
        for influence in skin.influence_range(logical) {
            let weight = skin.weights[influence];
            let entry = node_count + skin.influence_cluster[influence] as usize;
            let Some(matrix) = deform.palette.get(entry) else {
                continue;
            };
            blended_position += matrix.transform_point3(position) * weight;
            blended_normal += matrix.transform_vector3(normal) * weight;
            total += weight;
        }
    }
    if total > 0.0 {
        if (total - 1.0).abs() > WEIGHT_SUM_EPSILON {
            blended_position /= total;
            blended_normal /= total;
        }
        return (blended_position, blended_normal.normalize_or_zero());
    }

    // Unskinned (or a skinned vertex with no influences): ride the node rigidly.
    let node = ctx.corner_node.get(corner).copied().unwrap_or(u32::MAX);
    match deform.palette.get(node as usize) {
        Some(matrix) if (node as usize) < node_count => (
            matrix.transform_point3(position),
            matrix.transform_vector3(normal).normalize_or_zero(),
        ),
        _ => (position, normal.normalize_or_zero()),
    }
}

/// [`deform_corner`]'s position for logical vertex `logical`, through one of
/// the corners expanded from it. `None` when no corner maps to it.
pub fn deform_logical_position(
    model: &ModelData,
    ctx: &AnimContext,
    deform: &DeformPose,
    logical: usize,
) -> Option<Vec3> {
    let corner = ctx.logical_corner.get(logical).copied()?;
    if corner == u32::MAX {
        return None;
    }
    Some(deform_corner(model, ctx, deform, corner as usize).0)
}

/// Bounds of the mesh under `deform`. Walks the logical vertices (one corner
/// each) when the corner map is present — positions are shared across a logical
/// vertex's corners, so that is the whole set — else every corner.
pub fn posed_bounds(model: &ModelData, ctx: &AnimContext, deform: &DeformPose) -> Option<Bounds> {
    let mut bounds = Bounds::EMPTY;
    for corner in bounds_corners(model, ctx) {
        bounds.include_point(deform_corner(model, ctx, deform, corner as usize).0);
    }
    (!bounds.is_empty()).then_some(bounds)
}

/// The corners a bounds measurement walks: one per logical vertex when the corner
/// map is present — positions are shared across a logical vertex's corners, so
/// that is the whole set — else every corner. Shared by [`posed_bounds`] and
/// [`clip_bounds`] so the two can never disagree on what "the mesh" is.
fn bounds_corners<'a>(model: &'a ModelData, ctx: &'a AnimContext) -> impl Iterator<Item = u32> {
    match (!ctx.logical_corner.is_empty()).then_some(ctx.logical_corner.as_slice()) {
        Some(corners) => Either::Left(corners.iter().copied().filter(|&c| c != u32::MAX)),
        None => Either::Right(0..model.vertices.len() as u32),
    }
}

/// A two-armed iterator, so [`bounds_corners`] can return either walk without
/// boxing it.
enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R, T> Iterator for Either<L, R>
where
    L: Iterator<Item = T>,
    R: Iterator<Item = T>,
{
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self {
            Self::Left(left) => left.next(),
            Self::Right(right) => right.next(),
        }
    }
}

/// The bounds of the file's default pose — what a skinned or morphed model's
/// [`ModelData::bounds`] is, since that is the pose it rests in on screen.
pub fn rest_bounds(model: &ModelData, ctx: &AnimContext) -> Option<Bounds> {
    let mut pose = Pose::new(model);
    let mut deform = DeformPose::default();
    rest_pose(model, ctx, &mut pose);
    build_palette(model, ctx, &pose, &mut deform);
    posed_bounds(model, ctx, &deform)
}

/// Which nodes `clip` can pose away from their rest world transform: every node
/// it tracks, plus every descendant of one, since a parent's motion carries its
/// whole subtree.
///
/// Every other node poses at *exactly* its rest world matrix in every frame —
/// [`evaluate_pose`] falls back to `rest_local` for an untracked node, which is
/// what [`rest_pose`] gives it too — so anything riding it holds one position for
/// the whole clip. That is what [`clip_bounds`] exploits.
fn clip_moved_nodes(model: &ModelData, ctx: &AnimContext, clip: &AnimationClip) -> Vec<bool> {
    let mut moved = vec![false; model.nodes.len()];
    for track in &clip.tracks {
        if let Some(slot) = moved.get_mut(track.node as usize) {
            *slot = true;
        }
    }
    // Parents first, so a parent's flag is already final when its children read it.
    for &index in &ctx.order {
        if let Some(node) = model.nodes.get(index)
            && let Some(parent) = node.parent
            && parent < moved.len()
            && parent != index
            && moved[parent]
        {
            moved[index] = true;
        }
    }
    moved
}

/// Can `clip` move render corner `corner`? Mirrors [`deform_corner`]'s three
/// sources exactly — blend-shape offsets, the skin blend, then the rigid node
/// fallback — so a corner this rejects is one every frame leaves where the rest
/// pose put it.
fn corner_moves(
    model: &ModelData,
    ctx: &AnimContext,
    clip: &AnimationClip,
    moved_nodes: &[bool],
    corner: usize,
) -> bool {
    let logical = model.corner_to_logical.get(corner).map(|&v| v as usize);

    // A blend shape only moves a vertex it carries an offset for, and only while
    // some channel's weight is animated. (A channel held at a constant weight
    // holds the same one in the rest pose, so it moves nothing across frames.)
    if !clip.morph_tracks.is_empty()
        && let (Some(morph), Some(logical)) = (&model.morph, logical)
        && !morph.entry_range(logical).is_empty()
    {
        return true;
    }

    // Skinned corners ride their clusters' bones, not their own node.
    if let (Some(skin), Some(logical)) = (&model.skin, logical) {
        let range = skin.influence_range(logical);
        if !range.is_empty() {
            return skin.bones[range]
                .iter()
                .any(|&bone| moved_nodes.get(bone as usize).copied().unwrap_or(false));
        }
    }

    // Unskinned (or a skinned corner with no influences): rigid on its own node.
    let node = ctx.corner_node.get(corner).copied().unwrap_or(u32::MAX);
    moved_nodes.get(node as usize).copied().unwrap_or(false)
}

/// The union of the posed bounds over every frame of `clip` at `fps` — measured
/// once at import so framing and the bounding-box overlay can describe the whole
/// motion without ever re-skinning on the redraw path.
///
/// Only the corners `clip` can actually move are re-measured per frame; the rest
/// hold their rest-pose position in every frame and so are measured once. That
/// makes the cost proportional to the *animated* part of the scene rather than to
/// the whole mesh, which is the difference between a usable load and an unusable
/// one on the shape a game FBX actually takes: a large static set with a small
/// animated prop in it, over a long take. (A 2.8M-triangle exterior with a
/// 2401-frame clip reaching 1.5% of its triangles walked all 2.07M logical
/// vertices 2401 times.) The measured bounds are unchanged — a static corner's
/// palette entry is the same matrix at rest as in every frame.
pub fn clip_bounds(
    model: &ModelData,
    ctx: &AnimContext,
    clip: &AnimationClip,
    fps: f64,
) -> Option<Bounds> {
    let fps = frame_rate_or_default(fps);
    let mut pose = Pose::new(model);
    let mut deform = DeformPose::default();

    // Split the corners `posed_bounds` would walk into the ones this clip can
    // move and the ones it cannot, measuring the latter as we go — in the rest
    // pose, which is the pose they hold in every frame of the clip.
    let moved_nodes = clip_moved_nodes(model, ctx, clip);
    rest_pose(model, ctx, &mut pose);
    build_palette(model, ctx, &pose, &mut deform);
    let mut bounds = Bounds::EMPTY;
    let mut moving: Vec<u32> = Vec::new();
    for corner in bounds_corners(model, ctx) {
        if corner_moves(model, ctx, clip, &moved_nodes, corner as usize) {
            moving.push(corner);
        } else {
            bounds.include_point(deform_corner(model, ctx, &deform, corner as usize).0);
        }
    }

    if !moving.is_empty() {
        for frame in 0..clip.frame_count(fps) {
            evaluate_pose(
                model,
                ctx,
                Some(clip),
                clip.frame_time(frame, fps),
                &mut pose,
            );
            build_palette(model, ctx, &pose, &mut deform);
            for &corner in &moving {
                bounds.include_point(deform_corner(model, ctx, &deform, corner as usize).0);
            }
        }
    }

    (!bounds.is_empty()).then_some(bounds)
}

/// True when composing every node's rest local transform down its parent chain
/// reproduces its stored world transform within `tolerance` — the check that
/// the importer's local transforms and world matrices agree, and therefore that
/// a pose recomposed from them lands where the file says.
pub fn rest_locals_recompose(model: &ModelData, ctx: &AnimContext, tolerance: f32) -> bool {
    let mut pose = Pose::new(model);
    rest_pose(model, ctx, &mut pose);
    model
        .nodes
        .iter()
        .zip(&pose.world)
        .all(|(node, world)| matrices_close(node.transform, *world, tolerance))
}

fn matrices_close(a: Mat4, b: Mat4, tolerance: f32) -> bool {
    a.to_cols_array()
        .iter()
        .zip(b.to_cols_array())
        .all(|(x, y)| (x - y).abs() <= tolerance)
}

/// Convenience for tests and importers: a node with only a rest local transform.
pub fn node_with_local(
    name: &str,
    parent: Option<usize>,
    local: crate::LocalTransform,
) -> SceneNode {
    SceneNode {
        name: name.to_owned(),
        parent,
        rest_local: local,
        ..SceneNode::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        LocalTransform, ModelStats, MorphData, MorphShape, MorphTrack, NodeKind, NodeTrack,
        SkinCluster, SkinData, TriangleData, Vertex,
    };

    fn local(translation: Vec3) -> LocalTransform {
        LocalTransform {
            translation,
            ..LocalTransform::IDENTITY
        }
    }

    /// Root at the origin, a child 1 m up, a grandchild 1 m further up; world
    /// transforms filled from the composed locals.
    fn chain() -> ModelData {
        let mut model = ModelData {
            nodes: vec![
                node_with_local("root", None, local(Vec3::ZERO)),
                node_with_local("child", Some(0), local(Vec3::Y)),
                node_with_local("grandchild", Some(1), local(Vec3::Y)),
            ],
            ..ModelData::default()
        };
        model.nodes[1].transform = Mat4::from_translation(Vec3::Y);
        model.nodes[2].transform = Mat4::from_translation(Vec3::Y * 2.0);
        for node in &mut model.nodes {
            node.kind = NodeKind::Bone;
        }
        model
    }

    fn key<T>(time: f64, value: T) -> Key<T> {
        Key { time, value }
    }

    #[test]
    fn rest_pose_recomposes_the_stored_world_transforms() {
        let model = chain();
        let ctx = AnimContext::new(&model);
        assert_eq!(ctx.order(), &[0, 1, 2]);
        assert!(rest_locals_recompose(&model, &ctx, 1e-6));
    }

    #[test]
    fn order_puts_parents_first_even_when_children_come_first_in_the_table() {
        let mut model = chain();
        // Reverse the table: grandchild, child, root.
        model.nodes.reverse();
        model.nodes[0].parent = Some(1);
        model.nodes[1].parent = Some(2);
        model.nodes[2].parent = None;
        let ctx = AnimContext::new(&model);
        assert_eq!(ctx.order(), &[2, 1, 0]);
        assert!(rest_locals_recompose(&model, &ctx, 1e-6));
    }

    #[test]
    fn a_translation_key_moves_the_whole_subtree() {
        let model = chain();
        let ctx = AnimContext::new(&model);
        let clip = AnimationClip {
            name: "shift".to_owned(),
            time_begin: 0.0,
            time_end: 1.0,
            tracks: vec![NodeTrack {
                node: 1,
                translation: vec![key(0.0, Vec3::Y), key(1.0, Vec3::new(2.0, 1.0, 0.0))],
                ..NodeTrack::default()
            }],
            ..AnimationClip::default()
        };
        let mut pose = Pose::new(&model);
        evaluate_pose(&model, &ctx, Some(&clip), 0.5, &mut pose);
        assert!(
            pose.world[1]
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(1.0, 1.0, 0.0), 1e-6)
        );
        // The grandchild is untracked but rides its parent.
        assert!(
            pose.world[2]
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(1.0, 2.0, 0.0), 1e-6)
        );
        // Outside the key range the end values hold.
        evaluate_pose(&model, &ctx, Some(&clip), 5.0, &mut pose);
        assert!(
            pose.world[1]
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(2.0, 1.0, 0.0), 1e-6)
        );
        evaluate_pose(&model, &ctx, Some(&clip), -5.0, &mut pose);
        assert!(pose.world[1].w_axis.truncate().abs_diff_eq(Vec3::Y, 1e-6));
    }

    #[test]
    fn rotation_keys_take_the_shorter_arc() {
        let a = Quat::from_rotation_y(0.1);
        // The same orientation as +170°, expressed with the opposite sign so a
        // naive lerp would swing the long way round.
        let b = -Quat::from_rotation_y(170.0_f32.to_radians());
        let keys = vec![key(0.0, a), key(1.0, b)];
        let mid = sample_quat(&keys, 0.5, Quat::IDENTITY);
        let expected = Quat::from_rotation_y((0.1 + 170.0_f32.to_radians()) * 0.5);
        assert!(mid.abs_diff_eq(expected, 1e-4) || mid.abs_diff_eq(-expected, 1e-4));
    }

    #[test]
    fn clip_frame_helpers_round_trip() {
        let clip = AnimationClip {
            time_begin: 1.0,
            time_end: 2.0,
            ..AnimationClip::default()
        };
        assert_eq!(clip.frame_count(30.0), 31);
        assert_eq!(clip.frame_count(0.0), 31);
        assert_eq!(clip.frame_time(0, 30.0), 1.0);
        assert!((clip.frame_time(30, 30.0) - 2.0).abs() < 1e-9);
        assert_eq!(clip.frame_time(99, 30.0), 2.0);
        assert_eq!(clip.frame_at(1.5, 30.0), 15);
        assert_eq!(clip.frame_at(9.0, 30.0), 30);
        assert!((clip.wrap_time(2.25) - 1.25).abs() < 1e-9);
        assert!((clip.wrap_time(0.75) - 1.75).abs() < 1e-9);
        assert_eq!(clip.clamp_time(7.0), 2.0);
        let empty = AnimationClip::default();
        assert_eq!(empty.frame_count(30.0), 1);
        assert_eq!(empty.wrap_time(3.0), 0.0);
    }

    /// One triangle skinned to the chain: corner 0 rides the child (node 1)
    /// fully, corners 1 and 2 are split between root and child.
    fn skinned_triangle() -> ModelData {
        let mut model = chain();
        model.vertices = vec![
            Vertex {
                position: Vec3::new(0.0, 1.0, 0.0),
                ..Vertex::default()
            },
            Vertex {
                position: Vec3::new(1.0, 0.5, 0.0),
                ..Vertex::default()
            },
            Vertex {
                position: Vec3::new(0.0, 0.5, 1.0),
                ..Vertex::default()
            },
        ];
        model.indices = vec![0, 1, 2];
        model.triangles = TriangleData {
            to_face: Vec::new(),
            material: Vec::new(),
            node: vec![0],
        };
        model.corner_to_logical = vec![0, 1, 2];
        model.stats = ModelStats {
            vertex_count: 3,
            ..ModelStats::default()
        };
        // The bind pose equals the rest pose, so `world_to_bone_bind` is the
        // inverse of each bone's rest world.
        let bind = |bone: u32| SkinCluster {
            bone,
            mesh_node: 0,
            world_to_bone_bind: model.nodes[bone as usize].transform.inverse(),
        };
        model.skin = Some(SkinData {
            offsets: vec![0, 1, 3, 5],
            bones: vec![1, 0, 1, 0, 1],
            weights: vec![1.0, 0.5, 0.5, 0.5, 0.5],
            influence_cluster: vec![1, 0, 1, 0, 1],
            clusters: vec![bind(0), bind(1)],
            deformers: Vec::new(),
        });
        model
    }

    #[test]
    fn rest_palette_is_identity_and_leaves_the_mesh_alone() {
        let model = skinned_triangle();
        let ctx = AnimContext::new(&model);
        let mut pose = Pose::new(&model);
        let mut deform = DeformPose::default();
        rest_pose(&model, &ctx, &mut pose);
        build_palette(&model, &ctx, &pose, &mut deform);
        assert_eq!(deform.palette.len(), palette_len(&model));
        for entry in &deform.palette {
            assert!(entry.abs_diff_eq(Mat4::IDENTITY, 1e-6), "{entry}");
        }
        for corner in 0..3 {
            let (position, _) = deform_corner(&model, &ctx, &deform, corner);
            assert!(position.abs_diff_eq(model.vertices[corner].position, 1e-6));
        }
        assert_eq!(model.validate_deform(), Ok(()));
    }

    #[test]
    fn a_moved_bone_carries_its_vertices_by_their_weights() {
        let model = skinned_triangle();
        let ctx = AnimContext::new(&model);
        let clip = AnimationClip {
            time_begin: 0.0,
            time_end: 1.0,
            tracks: vec![NodeTrack {
                node: 1,
                // Rest at the start, +3 X at the end.
                translation: vec![key(0.0, Vec3::Y), key(1.0, Vec3::new(3.0, 1.0, 0.0))],
                ..NodeTrack::default()
            }],
            ..AnimationClip::default()
        };
        let mut pose = Pose::new(&model);
        let mut deform = DeformPose::default();
        evaluate_pose(&model, &ctx, Some(&clip), 1.0, &mut pose);
        build_palette(&model, &ctx, &pose, &mut deform);
        // Fully on the child: shifted by the full +3 X.
        let (p0, _) = deform_corner(&model, &ctx, &deform, 0);
        assert!(p0.abs_diff_eq(Vec3::new(3.0, 1.0, 0.0), 1e-5), "{p0}");
        // Half root (unmoved) / half child: shifted by +1.5 X.
        let (p1, _) = deform_corner(&model, &ctx, &deform, 1);
        assert!(p1.abs_diff_eq(Vec3::new(2.5, 0.5, 0.0), 1e-5), "{p1}");
        // The clip envelope spans both poses.
        let bounds = clip_bounds(&model, &ctx, &clip, 30.0).unwrap();
        assert!((bounds.max.x - 3.0).abs() < 1e-5, "{bounds:?}");
        assert!(bounds.min.x.abs() < 1e-5, "{bounds:?}");
    }

    #[test]
    fn an_unskinned_corner_rides_its_node_rigidly() {
        let mut model = skinned_triangle();
        model.skin = None;
        // The triangle belongs to node 1; move node 1.
        model.triangles.node = vec![1];
        let ctx = AnimContext::new(&model);
        let clip = AnimationClip {
            time_end: 1.0,
            tracks: vec![NodeTrack {
                node: 1,
                translation: vec![key(0.0, Vec3::new(0.0, 1.0, 4.0))],
                ..NodeTrack::default()
            }],
            ..AnimationClip::default()
        };
        let mut pose = Pose::new(&model);
        let mut deform = DeformPose::default();
        evaluate_pose(&model, &ctx, Some(&clip), 0.0, &mut pose);
        build_palette(&model, &ctx, &pose, &mut deform);
        let (p1, _) = deform_corner(&model, &ctx, &deform, 1);
        assert!(p1.abs_diff_eq(Vec3::new(1.0, 0.5, 4.0), 1e-5), "{p1}");
    }

    #[test]
    fn morph_offsets_add_before_skinning() {
        let mut model = skinned_triangle();
        model.morph = Some(MorphData {
            channels: vec![MorphChannel {
                name: "puff".to_owned(),
                mesh_node: 0,
                rest_weight: 0.0,
                keyframes: vec![MorphKeyframe {
                    shape: 0,
                    target_weight: 1.0,
                }],
            }],
            shapes: vec![MorphShape {
                name: "puff".to_owned(),
            }],
            offsets: vec![0, 1, 1, 1],
            shape: vec![0],
            position: vec![Vec3::Z],
            normal: vec![Vec3::ZERO],
        });
        let ctx = AnimContext::new(&model);
        let clip = AnimationClip {
            time_end: 1.0,
            morph_tracks: vec![MorphTrack {
                channel: 0,
                keys: vec![key(0.0, 0.0), key(1.0, 1.0)],
            }],
            ..AnimationClip::default()
        };
        let mut pose = Pose::new(&model);
        let mut deform = DeformPose::default();
        evaluate_pose(&model, &ctx, Some(&clip), 0.5, &mut pose);
        assert_eq!(pose.shape_weights, vec![0.5]);
        build_palette(&model, &ctx, &pose, &mut deform);
        let (p0, _) = deform_corner(&model, &ctx, &deform, 0);
        assert!(p0.abs_diff_eq(Vec3::new(0.0, 1.0, 0.5), 1e-5), "{p0}");
        assert_eq!(model.validate_deform(), Ok(()));
    }

    #[test]
    fn in_between_keyframes_blend_like_ufbx() {
        let channel = MorphChannel {
            name: "c".to_owned(),
            mesh_node: 0,
            rest_weight: 0.0,
            keyframes: vec![
                MorphKeyframe {
                    shape: 0,
                    target_weight: 0.5,
                },
                MorphKeyframe {
                    shape: 1,
                    target_weight: 1.0,
                },
            ],
        };
        let mut out = Vec::new();
        channel_effective_weights(&channel, 0.25, &mut out);
        assert_eq!(out, vec![0.5, 0.0]);
        channel_effective_weights(&channel, 0.5, &mut out);
        assert_eq!(out, vec![1.0, 0.0]);
        channel_effective_weights(&channel, 0.75, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
        channel_effective_weights(&channel, 1.0, &mut out);
        assert_eq!(out, vec![0.0, 1.0]);
        // Past the last target the last segment extrapolates linearly, as ufbx.
        channel_effective_weights(&channel, 1.5, &mut out);
        assert_eq!(out, vec![-1.0, 2.0]);
        channel_effective_weights(&channel, 0.0, &mut out);
        assert_eq!(out, vec![0.0, 0.0]);

        // The common single-target channel is just the weight itself.
        let simple = MorphChannel {
            keyframes: vec![MorphKeyframe {
                shape: 0,
                target_weight: 1.0,
            }],
            ..channel
        };
        channel_effective_weights(&simple, 0.3, &mut out);
        assert_eq!(out, vec![0.3]);
    }

    #[test]
    fn unnormalized_weights_are_normalized_by_their_sum() {
        let mut model = skinned_triangle();
        {
            let skin = model.skin.as_mut().unwrap();
            // Corner 0's single influence at weight 2.0 must still land exactly
            // on the bone, not twice as far.
            skin.weights[0] = 2.0;
        }
        let ctx = AnimContext::new(&model);
        let clip = AnimationClip {
            time_end: 1.0,
            tracks: vec![NodeTrack {
                node: 1,
                translation: vec![key(0.0, Vec3::new(3.0, 1.0, 0.0))],
                ..NodeTrack::default()
            }],
            ..AnimationClip::default()
        };
        let mut pose = Pose::new(&model);
        let mut deform = DeformPose::default();
        evaluate_pose(&model, &ctx, Some(&clip), 0.0, &mut pose);
        build_palette(&model, &ctx, &pose, &mut deform);
        let (p0, _) = deform_corner(&model, &ctx, &deform, 0);
        assert!(p0.abs_diff_eq(Vec3::new(3.0, 1.0, 0.0), 1e-5), "{p0}");
    }
}
