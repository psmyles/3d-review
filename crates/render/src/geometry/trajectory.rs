//! The animation-curve overlay's CPU geometry: each animated node's path through
//! the selected clip, a dot on every frame of it, and a marker riding the node to
//! show where on the path the current frame is.
//!
//! The paths are `review_model::anim::clip_paths` — world-space points sampled
//! once — so the line and its dots never deform: a path already *is* the whole
//! motion. The marker is the one part that moves, and it moves without a CPU
//! rebuild: it sits at the node's rest pivot carrying the node's own deform lane,
//! so the vertex shader puts it wherever the pose puts the node. That is the
//! skeleton overlay's trick for its joints, and the palette entry it reads maps a
//! node's rest pivot onto its posed one exactly (`world × inverse(rest_world)`),
//! so the marker lands on the path at every frame and slides along it between
//! them.
//!
//! The line between the dots is either straight, frame to frame, or a smooth
//! curve through them ([`curve_points`]). Either way the dots are the measured
//! positions and the line between them is drawing: the pose between two frames
//! is interpolated by the player, not sampled here.
//!
//! Zero-normal vertices throughout, the scene shader's overlay sentinel: nothing
//! here is lit, and nothing feeds ambient occlusion.

use glam::Vec3;

use crate::TrajectoryOverlay;
use crate::scene::SceneVertex;

use super::deform::{NO_DEFORM, node_deform};
use super::vertex::push_line;
use review_model::ModelData;
use review_model::color::{linear_to_srgb, oklch_to_linear_srgb};

/// How far, in radians, a smooth curve may turn from one of its segments to the
/// next: about six degrees, below which a polyline a few pixels wide already
/// reads as a curve. Where a path turns less than this per frame — most of any
/// clip sampled at 24 fps or more — it spends one segment per frame, exactly
/// as the straight style does.
const SMOOTH_TURN: f32 = 0.1;
/// The most segments a smooth curve spends between two frames, however sharply
/// it turns there. Bounds the buffer at this many times the straight style's.
const SMOOTH_STEPS_MAX: u32 = 8;

/// Successive hues step by the golden-ratio conjugate, so nodes next to each
/// other in [`hierarchy_ranks`]' order — a bone and its child — land far apart on
/// the wheel. The same step the UV islands and the Unique material mode use.
const HUE_STEP: f32 = 0.618_034;

/// The OKLab lightness every path is drawn at: one value, so no hue reads
/// darker than another (an HSV wheel's blues did), and high enough to stand out
/// over both a dark viewport and a light mesh.
const LIGHTNESS: f32 = 0.78;

/// The OKLab chromas successive nodes cycle through. Hue alone stops telling
/// paths apart well before a full rig: sixty bones are six degrees apart on the
/// wheel. Cycling a second, coarser axis keeps neighbours apart in colourfulness
/// as well, without giving up the shared lightness. Where the gamut cannot hold
/// a chroma at [`LIGHTNESS`] (the blues), it gives way rather than the lightness.
const CHROMAS: [f32; 3] = [0.13, 0.075, 0.10];

/// The color of the node at `rank` in [`hierarchy_ranks`]' order — its path's,
/// and its bone's with it. Keyed on the node rather than on the path's place in
/// the list, so a bone keeps its color from one clip to the next and when a
/// selection hides the paths around it. Gamma space, as every line color is.
pub(crate) fn trajectory_color(rank: u32) -> [f32; 4] {
    let hue = (rank as f32 * HUE_STEP).fract();
    let chroma = CHROMAS[rank as usize % CHROMAS.len()];
    let [r, g, b] = oklch_to_linear_srgb(LIGHTNESS, chroma, hue).map(linear_to_srgb);
    [r, g, b, 1.0]
}

/// Each node's place in a depth-first walk of the hierarchy (children in table
/// order), which is what a path's color is keyed on.
///
/// Depth-first puts a chain's bones next to one another, so the golden-ratio
/// step sends each one to the far side of the wheel from its parent. Keyed on
/// the table index instead, a rig the importer laid out breadth-first spaces a
/// chain's bones by the number of chains beside it — 5 and 8 entries apart on
/// the dog fixture, which are Fibonacci numbers, and those are exactly the
/// distances the golden ratio maps to *nearly the same* hue: its knee and foot
/// came out the same yellow-green. A node a malformed parent cycle leaves
/// unreached is ranked after the rest.
pub(crate) fn hierarchy_ranks(model: &ModelData) -> Vec<u32> {
    let count = model.nodes.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut roots = Vec::new();
    for (index, node) in model.nodes.iter().enumerate() {
        match node
            .parent
            .filter(|&parent| parent < count && parent != index)
        {
            Some(parent) => children[parent].push(index),
            None => roots.push(index),
        }
    }
    let mut ranks = vec![u32::MAX; count];
    let mut next = 0;
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(node) = stack.pop() {
        if ranks[node] != u32::MAX {
            continue;
        }
        ranks[node] = next;
        next += 1;
        stack.extend(children[node].iter().rev());
    }
    for rank in &mut ranks {
        if *rank == u32::MAX {
            *rank = next;
            next += 1;
        }
    }
    ranks
}

/// The color of node `node`, given the model's [`hierarchy_ranks`].
fn node_color(ranks: &[u32], node: u32) -> [f32; 4] {
    trajectory_color(ranks.get(node as usize).copied().unwrap_or(node))
}

/// The overlay's three line lists. Separate because each is drawn at its own
/// width: the dots and markers are zero-length lines, which the line shader
/// draws as squares of the width it is given.
#[derive(Default)]
pub(crate) struct TrajectoryGeometry {
    /// Each shown path, frame to frame.
    pub(crate) lines: Vec<SceneVertex>,
    /// A dot on every frame of each shown path, so the spacing reads as speed.
    pub(crate) dots: Vec<SceneVertex>,
    /// One per shown path, riding its node to the current frame.
    pub(crate) markers: Vec<SceneVertex>,
}

/// Build the shown paths of `overlay`. `deforms` says whether the model has a
/// deform layout — without one there is no palette for the marker's node lane to
/// read, so it stays at the rest pivot.
pub(crate) fn trajectory_geometry(
    model: &ModelData,
    overlay: &TrajectoryOverlay<'_>,
    deforms: bool,
) -> TrajectoryGeometry {
    let ranks = hierarchy_ranks(model);
    let mut geometry = TrajectoryGeometry::default();
    for &index in overlay.shown {
        let Some(path) = overlay.paths.get(index as usize) else {
            continue;
        };
        let Some(node) = model.nodes.get(path.node as usize) else {
            continue;
        };
        let color = node_color(&ranks, path.node);
        for pair in curve_points(&path.points, overlay.style.smooth).windows(2) {
            push_line(
                &mut geometry.lines,
                pair[0].to_array(),
                pair[1].to_array(),
                color,
                NO_DEFORM,
            );
        }
        for point in &path.points {
            let point = point.to_array();
            push_line(&mut geometry.dots, point, point, color, NO_DEFORM);
        }
        let rest = node.transform.w_axis.truncate().to_array();
        let lane = if deforms {
            node_deform(path.node as usize)
        } else {
            NO_DEFORM
        };
        push_line(&mut geometry.markers, rest, rest, color, lane);
    }
    geometry
}

/// The points a path's line is drawn through: its frames, with a held pose's
/// repeats dropped (a zero-length segment would only draw a square under the
/// frame's own dot), and with `smooth`, a centripetal Catmull-Rom spline through
/// them.
///
/// Centripetal — knots spaced by the square root of each chord — because it is
/// the one parametrisation of the family that never loops or cusps within a
/// segment, which a uniform spline does wherever a slow frame sits beside a fast
/// one: exactly what an animation's ease-in and ease-out are. Each segment is cut
/// into as many pieces as the path turns at its two ends calls for
/// ([`SMOOTH_TURN`]), so a straight run costs nothing extra.
fn curve_points(points: &[Vec3], smooth: bool) -> Vec<Vec3> {
    let mut frames: Vec<Vec3> = Vec::with_capacity(points.len());
    for &point in points {
        if frames.last() != Some(&point) {
            frames.push(point);
        }
    }
    if !smooth || frames.len() < 3 {
        return frames;
    }
    let last = frames.len() - 1;
    // Past either end, a point mirrored through the end one, so the end segments
    // leave their frames heading the way the path does.
    let at = |index: isize| -> Vec3 {
        match index {
            -1 => 2.0 * frames[0] - frames[1],
            i if i as usize > last => 2.0 * frames[last] - frames[last - 1],
            i => frames[i as usize],
        }
    };
    let mut curve = Vec::with_capacity(frames.len() * 2);
    curve.push(frames[0]);
    for segment in 0..last as isize {
        let [p0, p1, p2, p3] = [-1, 0, 1, 2].map(|offset| at(segment + offset));
        let turn = (p1 - p0).angle_between(p2 - p1) + (p2 - p1).angle_between(p3 - p2);
        let steps = ((turn * 0.5 / SMOOTH_TURN).ceil() as u32).clamp(1, SMOOTH_STEPS_MAX);
        for step in 1..steps {
            curve.push(centripetal(p0, p1, p2, p3, step as f32 / steps as f32));
        }
        curve.push(p2);
    }
    curve
}

/// The centripetal Catmull-Rom spline through `p1` and `p2`, `fraction` of the
/// way from one to the other (Barry and Goldman's pyramid). Every chord must
/// have a length, which [`curve_points`] guarantees by dropping repeats.
fn centripetal(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, fraction: f32) -> Vec3 {
    let t1 = p0.distance(p1).sqrt();
    let t2 = t1 + p1.distance(p2).sqrt();
    let t3 = t2 + p2.distance(p3).sqrt();
    let t = t1 + (t2 - t1) * fraction;
    let lerp = |a: Vec3, b: Vec3, from: f32, to: f32| a + (b - a) * ((t - from) / (to - from));
    let a1 = lerp(p0, p1, 0.0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, 0.0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

/// Per node, the color its bone takes while its path is shown — `None` for every
/// node with no shown path. Sized to the model's node table.
pub(crate) fn trajectory_node_colors(
    model: &ModelData,
    overlay: &TrajectoryOverlay<'_>,
) -> Vec<Option<[f32; 4]>> {
    let ranks = hierarchy_ranks(model);
    let mut colors = vec![None; model.nodes.len()];
    for &index in overlay.shown {
        if let Some(path) = overlay.paths.get(index as usize)
            && let Some(slot) = colors.get_mut(path.node as usize)
        {
            *slot = Some(node_color(&ranks, path.node));
        }
    }
    colors
}

#[cfg(test)]
mod tests {
    use glam::Mat4;
    use review_model::{NodePath, SceneNode};

    use super::*;
    use crate::TrajectoryStyle;

    fn model() -> ModelData {
        ModelData {
            nodes: (0..3)
                .map(|index| SceneNode {
                    name: format!("node{index}"),
                    transform: Mat4::from_translation(Vec3::X * index as f32),
                    ..SceneNode::default()
                })
                .collect(),
            ..ModelData::default()
        }
    }

    fn paths() -> Vec<NodePath> {
        vec![
            NodePath {
                node: 1,
                points: vec![Vec3::ZERO, Vec3::Y, Vec3::Y, Vec3::Y * 2.0],
            },
            NodePath {
                node: 2,
                points: vec![Vec3::ZERO, Vec3::Z],
            },
        ]
    }

    /// One segment per pair of frames that differ, a dot on every frame, and one
    /// marker per path - over the shown paths only.
    #[test]
    fn only_the_shown_paths_are_built() {
        let model = model();
        let paths = paths();
        let overlay = TrajectoryOverlay {
            revision: 1,
            paths: &paths,
            shown: &[0],
            style: TrajectoryStyle::default(),
        };
        let geometry = trajectory_geometry(&model, &overlay, true);
        // Four frames, one of them a repeat: two segments, four dots.
        assert_eq!(geometry.lines.len(), 2 * 2);
        assert_eq!(geometry.dots.len(), 4 * 2);
        assert_eq!(geometry.markers.len(), 2);

        let both = TrajectoryOverlay {
            shown: &[0, 1],
            ..overlay
        };
        let geometry = trajectory_geometry(&model, &both, true);
        assert_eq!(geometry.lines.len(), 3 * 2);
        assert_eq!(geometry.markers.len(), 2 * 2);
    }

    /// The path is a fixed record of the whole motion; the marker is what
    /// follows the pose, from the node's rest pivot on its own lane.
    #[test]
    fn the_marker_rides_its_node_and_the_path_stays_put() {
        let model = model();
        let paths = paths();
        let overlay = TrajectoryOverlay {
            revision: 1,
            paths: &paths,
            shown: &[1],
            style: TrajectoryStyle::default(),
        };
        let geometry = trajectory_geometry(&model, &overlay, true);
        assert!(
            geometry
                .lines
                .iter()
                .chain(&geometry.dots)
                .all(|vertex| vertex.deform == NO_DEFORM)
        );
        assert!(
            geometry
                .markers
                .iter()
                .all(|vertex| vertex.deform == node_deform(2)
                    && vertex.position == [2.0, 0.0, 0.0])
        );

        // A model with no deform layout has no palette to read.
        let still = trajectory_geometry(&model, &overlay, false);
        assert!(
            still
                .markers
                .iter()
                .all(|vertex| vertex.deform == NO_DEFORM)
        );
    }

    /// A path and its bone share a color, every vertex of the path carries it,
    /// and neighbouring nodes do not share one.
    #[test]
    fn a_path_and_its_bone_share_a_color() {
        let model = model();
        let paths = paths();
        let overlay = TrajectoryOverlay {
            revision: 1,
            paths: &paths,
            shown: &[0, 1],
            style: TrajectoryStyle::default(),
        };
        let colors = trajectory_node_colors(&model, &overlay);
        assert_eq!(colors[0], None, "node 0 has no path");
        let ranks = hierarchy_ranks(&model);
        assert_eq!(colors[1], Some(node_color(&ranks, 1)));
        assert_ne!(colors[1], colors[2]);

        let geometry = trajectory_geometry(&model, &overlay, true);
        assert_eq!(geometry.lines[0].vertex_color, node_color(&ranks, 1));
        assert!(
            geometry
                .lines
                .iter()
                .all(|vertex| vertex.normal == [0.0, 0.0, 0.0]),
            "the overlay sentinel: unlit, and out of the AO"
        );
    }

    /// A hidden path takes no color, so its bone goes back to the skeleton's own.
    #[test]
    fn a_hidden_path_leaves_its_bone_uncolored() {
        let model = model();
        let paths = paths();
        let overlay = TrajectoryOverlay {
            revision: 1,
            paths: &paths,
            shown: &[1],
            style: TrajectoryStyle::default(),
        };
        let colors = trajectory_node_colors(&model, &overlay);
        assert_eq!(colors[1], None);
        assert_eq!(colors[2], Some(node_color(&hierarchy_ranks(&model), 2)));
    }

    /// Two three-bone chains laid out breadth-first, as the importer lays out a
    /// rig: depth-first ranks put each chain's bones next to one another.
    #[test]
    fn ranks_walk_each_chain_before_the_next() {
        let parents = [None, Some(0), Some(0), Some(1), Some(2), Some(3), Some(4)];
        let model = ModelData {
            nodes: parents
                .iter()
                .map(|&parent| SceneNode {
                    parent,
                    ..SceneNode::default()
                })
                .collect(),
            ..ModelData::default()
        };
        // root, then 1-3-5 down one chain, then 2-4-6 down the other.
        assert_eq!(hierarchy_ranks(&model), [0, 1, 4, 2, 5, 3, 6]);
    }

    /// A bone and its child never share a hue, on a real rig's layout: the dog
    /// fixture's front-left knee, ankle and foot sit 5 and 8 table entries apart,
    /// and keyed on the table index the golden-ratio step gave them nearly the
    /// same color.
    #[test]
    fn a_chain_laid_out_breadth_first_gets_distinct_hues() {
        // Five chains side by side, eight bones deep, breadth-first.
        let chains = 5;
        let depth = 8;
        let mut parents = vec![None];
        for level in 0..depth {
            for chain in 0..chains {
                parents.push(Some(if level == 0 {
                    0
                } else {
                    1 + (level - 1) * chains + chain
                }));
            }
        }
        let model = ModelData {
            nodes: parents
                .iter()
                .map(|&parent| SceneNode {
                    parent,
                    ..SceneNode::default()
                })
                .collect(),
            ..ModelData::default()
        };
        let ranks = hierarchy_ranks(&model);
        let hue = |node: usize| (ranks[node] as f32 * HUE_STEP).fract();
        for (child, parent) in parents.iter().enumerate().skip(1) {
            let parent = parent.expect("every node but the root has one");
            let apart = (hue(child) - hue(parent)).abs();
            let apart = apart.min(1.0 - apart);
            assert!(
                apart > 0.3,
                "node {child} is only {apart:.3} of the wheel from its parent {parent}"
            );
        }
    }

    /// Every path reads equally light, whatever its hue: the old HSV tones put
    /// the blues far below the yellows.
    #[test]
    fn every_path_shares_one_lightness() {
        use review_model::color::{linear_srgb_to_oklab, srgb_to_linear};
        for rank in 0..68 {
            let [r, g, b, _] = trajectory_color(rank);
            let [lightness, ..] = linear_srgb_to_oklab([r, g, b].map(srgb_to_linear));
            assert!(
                (lightness - LIGHTNESS).abs() < 2e-3,
                "rank {rank} lightness {lightness}"
            );
        }
    }

    /// Sixty-odd bones, every color its own.
    #[test]
    fn a_full_rig_gets_distinct_colors() {
        let colors: Vec<[f32; 4]> = (0..68).map(trajectory_color).collect();
        for (a, first) in colors.iter().enumerate() {
            for second in &colors[a + 1..] {
                assert_ne!(first, second);
            }
        }
    }
    /// Straight, a path is its frames with the held repeats dropped; smooth, it
    /// still passes through every one of them, in order.
    #[test]
    fn a_smooth_curve_passes_through_every_frame() {
        // A quarter circle, a frame every fifteen degrees, holding at the end.
        let mut frames: Vec<Vec3> = (0..=6)
            .map(|step| {
                let angle = step as f32 * 15f32.to_radians();
                Vec3::new(angle.cos(), angle.sin(), 0.0)
            })
            .collect();
        frames.push(frames[6]);

        let straight = curve_points(&frames, false);
        assert_eq!(straight, frames[..7]);

        let smooth = curve_points(&frames, true);
        assert!(
            smooth.len() > straight.len(),
            "it turns, so it is cut finer"
        );
        let mut from = 0;
        for frame in &straight {
            let found = smooth[from..]
                .iter()
                .position(|point| point == frame)
                .expect("every frame is on the curve");
            from += found + 1;
        }
        // And it follows the circle it was sampled from more closely than the
        // straight chords do, which fall 1 - cos(7.5 degrees), 0.0086, inside it
        // midway between frames. The end segments are the loosest: past the
        // last frame the curve can only guess which way the path was heading.
        let chord_sag = 1.0 - 7.5f32.to_radians().cos();
        for point in &smooth {
            assert!((point.length() - 1.0).abs() < chord_sag * 0.6, "{point}");
        }
    }

    /// A path that runs straight costs the smooth style nothing extra.
    #[test]
    fn a_straight_run_is_not_subdivided() {
        let frames: Vec<Vec3> = (0..10).map(|step| Vec3::X * step as f32).collect();
        assert_eq!(curve_points(&frames, true), frames);
    }

    /// However sharp the turn, a segment is cut into a bounded number of pieces.
    #[test]
    fn a_sharp_turn_is_cut_into_a_bounded_number_of_pieces() {
        let frames = [
            Vec3::ZERO,
            Vec3::X,
            Vec3::ZERO + Vec3::Y * 1e-3,
            Vec3::NEG_X,
        ];
        let smooth = curve_points(&frames, true);
        assert!(smooth.len() <= 1 + 3 * SMOOTH_STEPS_MAX as usize);
        assert!(smooth.iter().all(|point| point.is_finite()));
    }
}
