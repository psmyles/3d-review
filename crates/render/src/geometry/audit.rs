//! The Aud workspace's offender lines and dots, built from an [`AuditOverlay`].
//!
//! Every vertex copies the deform lane of the corner it stands on (or, for a
//! point with no corner, the lane of the node it belongs to), so the overlay
//! bends with a skinned or animated mesh exactly as the wireframe does. A dot is
//! a zero-length line: the line program draws one end-on as a square of the
//! draw's width, so a wider draw is all a dot needs.

use review_model::ModelData;

use super::deform::{corner_deform, node_deform};
use super::vertex::{push_line, push_line_deformed};
use crate::AuditOverlay;
use crate::scene::SceneVertex;

/// The overlay's edges and its dots, each as a line list in which every colour's
/// alpha is scaled by `alpha` — 1 for the solid pass, the overlay's hidden alpha
/// for the faint always-on-top one.
pub(crate) fn audit_lines(
    model: &ModelData,
    lanes: &[[u32; 4]],
    overlay: &AuditOverlay<'_>,
    alpha: f32,
) -> (Vec<SceneVertex>, Vec<SceneVertex>) {
    let position = |corner: u32| {
        model
            .vertices
            .get(corner as usize)
            .map(|vertex| vertex.position.to_array())
    };
    let mut edges = Vec::new();
    let mut dots = Vec::new();
    for severity in 0..3 {
        let [r, g, b, _] = overlay.colors[severity];
        let color = [r, g, b, alpha];
        for &[a, b] in overlay.edges[severity] {
            if let (Some(start), Some(end)) = (position(a), position(b)) {
                push_line_deformed(
                    &mut edges,
                    start,
                    end,
                    color,
                    corner_deform(lanes, a as usize),
                    corner_deform(lanes, b as usize),
                );
            }
        }
        for &corner in overlay.corner_dots[severity] {
            if let Some(point) = position(corner) {
                push_line(
                    &mut dots,
                    point,
                    point,
                    color,
                    corner_deform(lanes, corner as usize),
                );
            }
        }
        for &(point, node) in overlay.point_dots[severity] {
            // A model that never deforms has no palette for a node lane to read.
            let lane = if lanes.is_empty() {
                super::deform::NO_DEFORM
            } else {
                node_deform(node as usize)
            };
            push_line(&mut dots, point, point, color, lane);
        }
    }
    (edges, dots)
}
