//! The skeleton overlay's CPU geometry: one octahedral bone per parent→child
//! joint pair, plus a 3-axis marker at every joint that terminates a chain.
//!
//! Joint positions come from
//! [`SceneNode::transform`](review_model::SceneNode::transform)'s translation
//! column — the rest-pose `node_to_world` the importer carried through. Geometry
//! is world-baked at import, so these are already in the same space as the mesh
//! and need no further transform. Under animation the overlay follows the pose
//! through the same deform palette as the mesh: every octahedron vertex carries
//! the deform lane of the joint it belongs to (head + ring → the parent, tail →
//! the child), so no CPU rebuild is needed per frame.
//!
//! Both builders emit **zero-normal** vertices (via [`push_line`] /
//! [`push_fill_vertex`]), which is the scene shader's overlay sentinel: the
//! fragment is returned flat in its vertex color, contributes no ambient, and is
//! skipped by the GTAO G-buffer. Nothing here needs a new shader.

use glam::{Vec2, Vec3};
use review_model::{DeformPose, ModelData, NodeKind, ray_triangle_t};

use crate::scene::SceneVertex;

use super::deform::node_deform;
use super::vertex::{push_fill_vertex, push_line, push_line_deformed};

/// An octahedron's cross-section, as a fraction of the bone's own length — the
/// proportion Blender uses, and what makes a bone read as a tapered spike rather
/// than a sliver or a blob.
const BONE_WIDTH_FRACTION: f32 = 0.1;
/// Where along the bone the octahedron's widest ring sits. Near the parent joint,
/// so the taper points at the child and the bone reads directionally.
const BONE_RING_POSITION: f32 = 0.1;
/// Floor / ceiling on an octahedron's half-width, as a fraction of the model's
/// largest bounding extent. The floor keeps a tiny finger bone from collapsing to
/// an invisible sliver; the ceiling keeps a single long root bone from swallowing
/// the character.
const BONE_HALF_WIDTH_MIN_FRACTION: f32 = 0.004;
const BONE_HALF_WIDTH_MAX_FRACTION: f32 = 0.05;
/// A terminal joint's marker half-length, as a fraction of the model extent — used
/// when the bone's authored radius gives nothing usable.
const JOINT_MARKER_FRACTION: f32 = 0.01;

/// A parent→child bone segment, resolved to world positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BoneSegment {
    /// The node index of the parent joint — what a selection matches against, so
    /// selecting a bone highlights the segment *leaving* it (the convention every
    /// DCC uses: the bone belongs to its head joint).
    pub(crate) node: usize,
    /// The child joint the segment points at; the tail vertex follows it.
    pub(crate) child: usize,
    pub(crate) head: Vec3,
    pub(crate) tail: Vec3,
}

/// World position of a node at rest, from its `node_to_world` translation.
fn joint_position(model: &ModelData, node: usize) -> Vec3 {
    model.nodes[node].transform.w_axis.truncate()
}

/// Every node's world position under `deform`, or at rest when there is none.
///
/// The CPU mirror of what the vertex shader does to the overlay's own vertices:
/// they carry `node_deform(node)`, so the GPU moves each by palette entry
/// `node` — and this applies the same matrix to the same rest position. That is
/// what lets a pick hit the bones where they are *drawn* on an animated model
/// rather than where the bind pose left them.
pub fn posed_joint_positions(model: &ModelData, deform: Option<&DeformPose>) -> Vec<Vec3> {
    (0..model.nodes.len())
        .map(|node| {
            let rest = joint_position(model, node);
            match deform.and_then(|deform| deform.palette.get(node)) {
                Some(matrix) => matrix.transform_point3(rest),
                None => rest,
            }
        })
        .collect()
}

/// Every parent→child pair where **both** ends are bones.
///
/// A bone parented to a non-bone (the usual root case) contributes no segment of
/// its own — it gets a joint marker instead — because there is no second joint to
/// draw toward. A bone with several bone children yields one octahedron per child,
/// which is how a shoulder or a hand fans out.
pub(crate) fn bone_segments(model: &ModelData) -> Vec<BoneSegment> {
    let joints = posed_joint_positions(model, None);
    bone_segments_from(model, &joints)
}

/// [`bone_segments`] over caller-supplied joint positions — the posed ones, for
/// a pick that has to match the drawn skeleton rather than the rest one.
fn bone_segments_from(model: &ModelData, joints: &[Vec3]) -> Vec<BoneSegment> {
    let mut segments = Vec::new();
    for (index, node) in model.nodes.iter().enumerate() {
        if node.kind != NodeKind::Bone {
            continue;
        }
        let Some(parent) = node.parent else { continue };
        let Some(parent_node) = model.nodes.get(parent) else {
            continue;
        };
        if parent_node.kind != NodeKind::Bone {
            continue;
        }
        let (Some(&head), Some(&tail)) = (joints.get(parent), joints.get(index)) else {
            continue;
        };
        segments.push(BoneSegment {
            node: parent,
            child: index,
            head,
            tail,
        });
    }
    segments
}

/// Bone nodes that no segment starts from — chain tips, and roots whose parent
/// isn't a bone. They get an axis marker so the chain visibly terminates rather
/// than just stopping.
pub(crate) fn terminal_joints(model: &ModelData) -> Vec<usize> {
    let mut has_outgoing = vec![false; model.nodes.len()];
    for segment in bone_segments(model) {
        has_outgoing[segment.node] = true;
    }
    model
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| node.kind == NodeKind::Bone && !has_outgoing[*index])
        .map(|(index, _)| index)
        .collect()
}

/// The model's largest bounding extent, the yardstick every size below is a
/// fraction of, so a rig reads the same whether it was authored in meters or
/// centimeters.
fn model_extent(model: &ModelData) -> f32 {
    model
        .bounds
        .map(|bounds| bounds.size().max_element())
        .unwrap_or(1.0)
        .max(f32::MIN_POSITIVE)
}

/// An octahedron's half-width for a bone of `length`: a fraction of its own
/// length, clamped into a model-relative band so neither a fingertip nor a root
/// bone escapes a sensible size, then scaled by the user's slider.
fn bone_half_width(length: f32, extent: f32, scale: f32) -> f32 {
    let min = extent * BONE_HALF_WIDTH_MIN_FRACTION;
    let max = extent * BONE_HALF_WIDTH_MAX_FRACTION;
    ((length * BONE_WIDTH_FRACTION).clamp(min, max) * scale.max(0.01)).max(f32::MIN_POSITIVE)
}

/// An orthonormal pair spanning the plane perpendicular to `axis`, for placing the
/// octahedron's ring. The seed is swapped when `axis` is close to +X so the cross
/// product never degenerates on an axis-aligned bone.
fn perpendicular_basis(axis: Vec3) -> (Vec3, Vec3) {
    let seed = if axis.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
    let right = axis.cross(seed).normalize_or_zero();
    let right = if right.length_squared() < 1e-12 {
        // `axis` was parallel to the seed after all (a degenerate bone); any
        // perpendicular will do.
        Vec3::Z
    } else {
        right
    };
    let up = axis.cross(right).normalize_or_zero();
    (right, up)
}

/// The six vertices of a bone's octahedron: head, tail, and the four ring points
/// around the widest cross-section.
fn octahedron_points(head: Vec3, tail: Vec3, extent: f32, scale: f32) -> Option<[Vec3; 6]> {
    let along = tail - head;
    let length = along.length();
    if length <= f32::MIN_POSITIVE {
        // A zero-length bone (a child sharing its parent's position) has no
        // direction to build on; the joint marker still shows where it is.
        return None;
    }
    let axis = along / length;
    let (right, up) = perpendicular_basis(axis);
    let half = bone_half_width(length, extent, scale);
    let ring = head + along * BONE_RING_POSITION;
    Some([
        head,
        tail,
        ring + right * half,
        ring + up * half,
        ring - right * half,
        ring - up * half,
    ])
}

/// The octahedron's eight triangles, as index triples into [`octahedron_points`]'s
/// output: four from the head to each ring edge, four from the ring to the tail.
const OCTAHEDRON_TRIANGLES: [[usize; 3]; 8] = [
    [0, 2, 3],
    [0, 3, 4],
    [0, 4, 5],
    [0, 5, 2],
    [1, 3, 2],
    [1, 4, 3],
    [1, 5, 4],
    [1, 2, 5],
];

/// The octahedron's twelve edges: the ring, plus four spokes to each apex.
const OCTAHEDRON_EDGES: [[usize; 2]; 12] = [
    [2, 3],
    [3, 4],
    [4, 5],
    [5, 2],
    [0, 2],
    [0, 3],
    [0, 4],
    [0, 5],
    [1, 2],
    [1, 3],
    [1, 4],
    [1, 5],
];

/// How a bone is coloured: the base colour, the selected one, and the hover
/// preview — bundled because every builder needs all of them and threading five
/// loose arguments through each was already at the limit.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoneTint<'a> {
    /// Sorted selected-bone set.
    pub(crate) selected: &'a [u32],
    /// The bone under the pointer, previewing what a click would select.
    pub(crate) hovered: Option<u32>,
    pub(crate) color: [f32; 4],
    pub(crate) selected_color: [f32; 4],
    pub(crate) hover_color: [f32; 4],
}

impl BoneTint<'_> {
    /// The colour bone `node` is drawn in. Selection wins over hover: a selected
    /// bone the pointer happens to rest on must not stop looking selected.
    fn color_for(&self, node: usize) -> [f32; 4] {
        if self.selected.binary_search(&(node as u32)).is_ok() {
            self.selected_color
        } else if self.hovered == Some(node as u32) {
            self.hover_color
        } else {
            self.color
        }
    }
}

/// How far from a bone's drawn outline a click still counts as hitting it, in
/// egui points. A bone is often a sliver two or three pixels wide — a finger
/// joint, or any bone seen end-on — and requiring the pointer to land inside
/// that would make exactly the rigs worth inspecting unclickable.
pub const BONE_PICK_TOLERANCE_POINTS: f32 = 6.0;

/// Which bone the pointer is on, given a ray through it and a way to project
/// world points back to the viewport.
///
/// Two tests, in priority order:
///
/// 1. The ray against the octahedra themselves, nearest hit first. This is what
///    picking a thick bone feels like — the shape is solid and the pointer is
///    inside it.
/// 2. Failing that, the screen distance from the pointer to each bone's drawn
///    edges, within [`BONE_PICK_TOLERANCE_POINTS`] scaled into `tolerance`.
///    This is what makes a sliver clickable, and it is measured against the
///    *drawn* edges so what is picked is what can be seen.
///
/// Shapes come from the same builders the overlay draws with, over `joints` —
/// pass the posed positions and the pick follows the animation. Returns the
/// segment's head node, the same node the Outliner selects for that bone.
#[expect(
    clippy::too_many_arguments,
    reason = "a ray, a projection and the overlay's own sizing, all needed to \
              hit-test what was drawn"
)]
pub fn pick_bone_shape(
    model: &ModelData,
    joints: &[Vec3],
    scale: f32,
    origin: Vec3,
    dir: Vec3,
    project: &dyn Fn(Vec3) -> Option<Vec2>,
    pointer: Vec2,
    tolerance: f32,
) -> Option<usize> {
    let extent = model_extent(model);
    let segments = bone_segments_from(model, joints);

    // ── 1. Solid hits ───────────────────────────────────────────────────────
    let mut nearest: Option<(f32, usize)> = None;
    for segment in &segments {
        let Some(points) = octahedron_points(segment.head, segment.tail, extent, scale) else {
            continue;
        };
        for [a, b, c] in OCTAHEDRON_TRIANGLES {
            if let Some(t) = ray_triangle_t(origin, dir, points[a], points[b], points[c])
                && t > 0.0
                && nearest.is_none_or(|(best, _)| t < best)
            {
                nearest = Some((t, segment.node));
            }
        }
    }
    if let Some((_, node)) = nearest {
        return Some(node);
    }

    // ── 2. Near misses, by screen distance to the drawn edges ───────────────
    // Ranked by distance rather than depth: within a few pixels of two bones,
    // the one the pointer is actually nearer to is the one being aimed at.
    let mut closest: Option<(f32, usize)> = None;
    let mut consider = |distance: f32, node: usize| {
        if distance <= tolerance && closest.is_none_or(|(best, _)| distance < best) {
            closest = Some((distance, node));
        }
    };

    for segment in &segments {
        let Some(points) = octahedron_points(segment.head, segment.tail, extent, scale) else {
            continue;
        };
        for [a, b] in OCTAHEDRON_EDGES {
            let (Some(from), Some(to)) = (project(points[a]), project(points[b])) else {
                continue;
            };
            consider(point_to_segment_distance(pointer, from, to), segment.node);
        }
    }

    // A chain's last joint has no segment leaving it, only its axis marker — and
    // it is as legitimate a thing to click as any bone.
    for joint in terminal_joints(model) {
        let Some(&position) = joints.get(joint) else {
            continue;
        };
        let half = joint_marker_half(model, joint, extent, scale);
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let (Some(from), Some(to)) = (
                project(position - axis * half),
                project(position + axis * half),
            ) else {
                continue;
            };
            consider(point_to_segment_distance(pointer, from, to), joint);
        }
    }

    closest.map(|(_, node)| node)
}

/// Shortest distance from `point` to the line segment `from`-`to`, in the same
/// units they are given in.
fn point_to_segment_distance(point: Vec2, from: Vec2, to: Vec2) -> f32 {
    let along = to - from;
    let length_squared = along.length_squared();
    if length_squared <= f32::MIN_POSITIVE {
        return point.distance(from);
    }
    let t = ((point - from).dot(along) / length_squared).clamp(0.0, 1.0);
    point.distance(from + along * t)
}

/// Half-length of a terminal joint's axis marker: what the rig authored, or a
/// model-relative size when it declared nothing usable.
fn joint_marker_half(model: &ModelData, joint: usize, extent: f32, scale: f32) -> f32 {
    let bone = model.nodes[joint].bone.unwrap_or_default();
    let authored = bone.radius * bone.relative_length;
    let base = if authored > 0.0 {
        authored * extent * JOINT_MARKER_FRACTION
    } else {
        extent * JOINT_MARKER_FRACTION
    };
    base * scale.max(0.01)
}

/// The solid octahedron fills, one per bone segment. Drawn with the always-on-top
/// `fill_overlay` pipeline; the color's alpha keeps the mesh readable underneath.
pub(crate) fn skeleton_fill_triangles(
    model: &ModelData,
    tint: BoneTint<'_>,
    scale: f32,
    fill_alpha: f32,
) -> Vec<SceneVertex> {
    let extent = model_extent(model);
    let mut vertices = Vec::new();
    for segment in bone_segments(model) {
        let Some(points) = octahedron_points(segment.head, segment.tail, extent, scale) else {
            continue;
        };
        let base = tint.color_for(segment.node);
        let fill = [base[0], base[1], base[2], base[3] * fill_alpha];
        for [a, b, c] in OCTAHEDRON_TRIANGLES {
            for corner in [a, b, c] {
                push_fill_vertex(
                    &mut vertices,
                    points[corner].to_array(),
                    fill,
                    octahedron_deform(&segment, corner),
                );
            }
        }
    }
    vertices
}

/// The deform lane of octahedron point `point` (an index into
/// [`octahedron_points`]'s output): the tail follows the child joint, everything
/// else rides the parent rigidly.
fn octahedron_deform(segment: &BoneSegment, point: usize) -> [u32; 4] {
    if point == 1 {
        node_deform(segment.child)
    } else {
        node_deform(segment.node)
    }
}

/// The octahedron outlines plus a 3-axis marker at every terminal joint. Opaque,
/// so a bone stays legible against a busy mesh even where the fill is thin.
pub(crate) fn skeleton_lines(
    model: &ModelData,
    tint: BoneTint<'_>,
    scale: f32,
) -> Vec<SceneVertex> {
    let extent = model_extent(model);
    let mut vertices = Vec::new();

    for segment in bone_segments(model) {
        let Some(points) = octahedron_points(segment.head, segment.tail, extent, scale) else {
            continue;
        };
        let line_color = tint.color_for(segment.node);
        for [a, b] in OCTAHEDRON_EDGES {
            push_line_deformed(
                &mut vertices,
                points[a].to_array(),
                points[b].to_array(),
                line_color,
                octahedron_deform(&segment, a),
                octahedron_deform(&segment, b),
            );
        }
    }

    for joint in terminal_joints(model) {
        let position = joint_position(model, joint);
        let half = joint_marker_half(model, joint, extent, scale);
        let line_color = tint.color_for(joint);
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            push_line(
                &mut vertices,
                (position - axis * half).to_array(),
                (position + axis * half).to_array(),
                line_color,
                node_deform(joint),
            );
        }
    }

    vertices
}

#[cfg(test)]
mod tests {
    /// A tint with no hover, which is what every case below but the hover one
    /// wants: the builders' colour arguments used to be loose and are now
    /// bundled.
    fn tint<'a>(selected: &'a [u32], color: [f32; 4], selected_color: [f32; 4]) -> BoneTint<'a> {
        BoneTint {
            selected,
            hovered: None,
            color,
            selected_color,
            hover_color: [0.0; 4],
        }
    }

    use super::*;
    use glam::Mat4;
    use review_model::{BoneInfo, SceneNode};

    fn bone_at(name: &str, parent: Option<usize>, position: Vec3, kind: NodeKind) -> SceneNode {
        SceneNode {
            name: name.to_owned(),
            parent,
            mesh_part: None,
            source_vertex_count: 0,
            transform: Mat4::from_translation(position),
            rest_local: Default::default(),
            kind,
            bone: (kind == NodeKind::Bone).then_some(BoneInfo {
                radius: 1.0,
                relative_length: 1.0,
            }),
        }
    }

    /// A root null, a 3-bone chain hanging off it, and a stray mesh node:
    ///
    /// ```text
    /// 0 root (Empty)   at origin
    /// └── 1 hips  (Bone) y=0
    ///     └── 2 spine (Bone) y=1
    ///         └── 3 head  (Bone) y=2
    /// 4 mesh (Mesh), parented to the root
    /// ```
    fn chain() -> ModelData {
        let mut model = ModelData {
            nodes: vec![
                bone_at("root", None, Vec3::ZERO, NodeKind::Empty),
                bone_at("hips", Some(0), Vec3::new(0.0, 0.0, 0.0), NodeKind::Bone),
                bone_at("spine", Some(1), Vec3::new(0.0, 1.0, 0.0), NodeKind::Bone),
                bone_at("head", Some(2), Vec3::new(0.0, 2.0, 0.0), NodeKind::Bone),
                bone_at("mesh", Some(0), Vec3::ZERO, NodeKind::Mesh),
            ],
            ..Default::default()
        };
        model.bounds = Some(review_model::Bounds {
            min: Vec3::new(-1.0, 0.0, -1.0),
            max: Vec3::new(1.0, 2.0, 1.0),
        });
        model
    }

    #[test]
    fn segments_link_bone_to_bone_only() {
        let segments = bone_segments(&chain());
        // hips->spine and spine->head. The root is not a bone, so hips starts no
        // segment of its own, and the mesh node is ignored entirely.
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].node, 1);
        assert_eq!(segments[0].head, Vec3::ZERO);
        assert_eq!(segments[0].tail, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(segments[1].node, 2);
    }

    #[test]
    fn a_bone_with_several_bone_children_gets_one_segment_each() {
        let mut model = chain();
        // A second child of the spine: a clavicle branching sideways.
        model.nodes.push(bone_at(
            "clavicle",
            Some(2),
            Vec3::new(1.0, 1.5, 0.0),
            NodeKind::Bone,
        ));
        let from_spine = bone_segments(&model)
            .into_iter()
            .filter(|segment| segment.node == 2)
            .count();
        assert_eq!(from_spine, 2, "one octahedron per child");
    }

    #[test]
    fn terminal_joints_are_the_chain_tips() {
        // Only `head` starts no segment; hips and spine both do.
        assert_eq!(terminal_joints(&chain()), vec![3]);
    }

    #[test]
    fn a_lone_bone_is_its_own_terminal_joint() {
        let mut model = chain();
        model.nodes.truncate(2); // root + hips
        assert!(bone_segments(&model).is_empty());
        assert_eq!(terminal_joints(&model), vec![1]);
    }

    #[test]
    fn vertex_counts_match_the_octahedron_and_marker_geometry() {
        let model = chain();
        let fill = skeleton_fill_triangles(&model, tint(&[], [1.0; 4], [1.0; 4]), 1.0, 0.35);
        let lines = skeleton_lines(&model, tint(&[], [1.0; 4], [1.0; 4]), 1.0);
        // 2 segments x 8 triangles x 3 corners.
        assert_eq!(fill.len(), 2 * 8 * 3);
        // 2 segments x 12 edges x 2 ends, plus 1 terminal joint x 3 axes x 2 ends.
        assert_eq!(lines.len(), 2 * 12 * 2 + 3 * 2);
    }

    #[test]
    fn overlay_vertices_carry_the_zero_normal_sentinel() {
        let model = chain();
        let fill = skeleton_fill_triangles(&model, tint(&[], [1.0; 4], [1.0; 4]), 1.0, 0.35);
        let lines = skeleton_lines(&model, tint(&[], [1.0; 4], [1.0; 4]), 1.0);
        // Without this the shader would light the bones and they'd feed GTAO.
        assert!(
            fill.iter()
                .chain(&lines)
                .all(|v| v.normal == [0.0, 0.0, 0.0])
        );
    }

    #[test]
    fn selected_bones_take_the_selection_color() {
        let model = chain();
        let plain = [0.0, 0.0, 1.0, 1.0];
        let selected = [1.0, 0.0, 0.0, 1.0];
        // Bone 1 (hips) selected: its segment recolors, bone 2's does not.
        let lines = skeleton_lines(&model, tint(&[1], plain, selected), 1.0);
        let recolored = lines.iter().filter(|v| v.vertex_color == selected).count();
        assert_eq!(recolored, 12 * 2, "exactly one octahedron's outline");
        assert!(lines.iter().any(|v| v.vertex_color == plain));
    }

    #[test]
    fn the_fill_alpha_scales_only_the_alpha_channel() {
        let model = chain();
        let color = [0.2, 0.4, 0.6, 0.8];
        let fill = skeleton_fill_triangles(&model, tint(&[], color, color), 1.0, 0.5);
        assert_eq!(fill[0].vertex_color, [0.2, 0.4, 0.6, 0.4]);
        // The outlines stay fully opaque at the authored alpha.
        let lines = skeleton_lines(&model, tint(&[], color, color), 1.0);
        assert_eq!(lines[0].vertex_color, color);
    }

    #[test]
    fn bone_width_is_clamped_into_a_model_relative_band() {
        let extent = 2.0;
        // A very short bone is floored, a very long one capped.
        assert_eq!(
            bone_half_width(0.0001, extent, 1.0),
            extent * BONE_HALF_WIDTH_MIN_FRACTION
        );
        assert_eq!(
            bone_half_width(1000.0, extent, 1.0),
            extent * BONE_HALF_WIDTH_MAX_FRACTION
        );
        // In between it tracks the bone's own length.
        let mid = bone_half_width(0.5, extent, 1.0);
        assert!((mid - 0.05).abs() < 1e-6, "got {mid}");
        // The user's slider scales the result.
        assert!(bone_half_width(0.5, extent, 2.0) > mid);
    }

    #[test]
    fn a_zero_length_bone_is_skipped_without_panicking() {
        let mut model = chain();
        // Put the spine exactly on the hips.
        model.nodes[2].transform = Mat4::from_translation(Vec3::ZERO);
        let fill = skeleton_fill_triangles(&model, tint(&[], [1.0; 4], [1.0; 4]), 1.0, 0.35);
        // hips->spine is degenerate and drops out; spine->head still draws.
        assert_eq!(fill.len(), 8 * 3);
    }

    #[test]
    fn an_axis_aligned_bone_builds_a_non_degenerate_ring() {
        // The perpendicular basis must not collapse when the bone runs along an
        // axis - X in particular, which is the natural cross-product seed.
        for axis in [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X] {
            let points = octahedron_points(Vec3::ZERO, axis, 1.0, 1.0).expect("a real bone");
            let ring_span = (points[2] - points[4]).length();
            assert!(ring_span > 1e-6, "{axis:?} collapsed the ring");
            let other_span = (points[3] - points[5]).length();
            assert!(other_span > 1e-6, "{axis:?} collapsed the second ring axis");
        }
    }

    #[test]
    fn a_hovered_bone_takes_the_hover_color() {
        let model = chain();
        let plain = [0.0, 0.0, 1.0, 1.0];
        let selected = [1.0, 0.0, 0.0, 1.0];
        let hover = [0.0, 1.0, 0.0, 1.0];
        let hovered = |selected_set: &[u32], node: Option<u32>| {
            skeleton_lines(
                &model,
                BoneTint {
                    selected: selected_set,
                    hovered: node,
                    color: plain,
                    selected_color: selected,
                    hover_color: hover,
                },
                1.0,
            )
        };

        // Bone 1 (hips) hovered: its segment takes the hover colour.
        let lines = hovered(&[], Some(1));
        assert_eq!(
            lines.iter().filter(|v| v.vertex_color == hover).count(),
            12 * 2,
            "exactly one octahedron's outline"
        );

        // Selection wins over hover, so a selected bone under the pointer does
        // not stop looking selected.
        let lines = hovered(&[1], Some(1));
        assert_eq!(lines.iter().filter(|v| v.vertex_color == hover).count(), 0);
        assert_eq!(
            lines.iter().filter(|v| v.vertex_color == selected).count(),
            12 * 2
        );
    }

    /// The pick must hit the bone the ray passes through, and report the head
    /// joint - the node the Outliner selects for that bone.
    #[test]
    fn pick_bone_shape_hits_the_octahedron_it_passes_through() {
        let model = chain();
        let joints = posed_joint_positions(&model, None);
        // `chain` puts hips at y = 0, spine at y = 1 and head at y = 2, so the
        // *spine* bone is the one spanning y = 1..2. Aim across its middle.
        let origin = Vec3::new(5.0, 1.5, 0.0);
        let dir = Vec3::NEG_X;
        let hit = pick_bone_shape(
            &model,
            &joints,
            1.0,
            origin,
            dir,
            &|_| None,
            Vec2::ZERO,
            0.0,
        );
        assert_eq!(hit, Some(2), "the head joint of the bone crossed");

        // A ray well clear of the skeleton hits nothing, and with no projection
        // and no tolerance there is no near-miss fallback either.
        let miss = pick_bone_shape(
            &model,
            &joints,
            1.0,
            Vec3::new(5.0, 50.0, 0.0),
            dir,
            &|_| None,
            Vec2::ZERO,
            0.0,
        );
        assert_eq!(miss, None);
    }

    /// A bone too thin to hit directly is still clickable, through the
    /// screen-space tolerance - which is the whole reason that second test
    /// exists.
    #[test]
    fn pick_bone_shape_catches_a_near_miss_within_the_tolerance() {
        let model = chain();
        let joints = posed_joint_positions(&model, None);
        // Project along the x axis: y becomes the screen x, z the screen y.
        let project = |world: Vec3| Some(Vec2::new(world.y, world.z));
        // A ray that misses every octahedron entirely, with the pointer a
        // little to the side of the bone's drawn line rather than on it.
        let origin = Vec3::new(5.0, 1.5, 20.0);
        let pointer = Vec2::new(1.5, 0.5);

        let far = pick_bone_shape(
            &model,
            &joints,
            1.0,
            origin,
            Vec3::NEG_X,
            &project,
            pointer,
            0.0,
        );
        assert_eq!(far, None, "nothing is within a zero tolerance");

        let near = pick_bone_shape(
            &model,
            &joints,
            1.0,
            origin,
            Vec3::NEG_X,
            &project,
            pointer,
            1.0,
        );
        assert_eq!(near, Some(2), "the bone the pointer is nearest to");
    }

    /// Under a pose the joints move, and the pick has to follow them.
    #[test]
    fn posed_joint_positions_follow_the_palette() {
        let model = chain();
        let rest = posed_joint_positions(&model, None);
        let mut palette = vec![Mat4::IDENTITY; model.nodes.len()];
        palette[1] = Mat4::from_translation(Vec3::new(3.0, 0.0, 0.0));
        let deform = review_model::DeformPose {
            palette,
            ..Default::default()
        };
        let posed = posed_joint_positions(&model, Some(&deform));

        assert_eq!(
            posed[0], rest[0],
            "an identity entry leaves the joint alone"
        );
        assert_eq!(posed[1], rest[1] + Vec3::new(3.0, 0.0, 0.0));
    }
}
