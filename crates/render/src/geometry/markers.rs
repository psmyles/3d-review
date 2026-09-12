//! The pivot cross and the bounding box: the two overlays that mark a place
//! rather than follow the mesh.

use glam::Vec3;
use review_model::{Bounds, ModelData};

use crate::scene::SceneVertex;

use super::deform::NO_DEFORM;
use super::vertex::push_line;

/// Half-length of each pivot-marker axis line, as a fraction of the model's
/// largest bounding extent — so the marker reads at any model scale.
const PIVOT_LENGTH_FRACTION: f32 = 0.1;

/// Floor for the pivot half-length, in world units, so a degenerate or empty
/// model still shows a visible marker.
const PIVOT_MIN_HALF: f32 = 0.05;

/// World-space position of the model's pivot: the translation of its root scene
/// node (the first parentless node), i.e. where the object's local origin lands
/// after the importer world-bakes the geometry (invariant 1). The world origin
/// when the model carries no node hierarchy.
pub(crate) fn model_pivot(model: &ModelData) -> [f32; 3] {
    model
        .nodes
        .iter()
        .find(|node| node.parent.is_none())
        .map(|node| node.transform.w_axis.truncate().to_array())
        .unwrap_or([0.0, 0.0, 0.0])
}

/// Half-length of each pivot axis line: [`PIVOT_LENGTH_FRACTION`] of the model's
/// largest extent, floored at [`PIVOT_MIN_HALF`].
pub(crate) fn pivot_half_extent(model: &ModelData) -> f32 {
    let size = model.bounds.map(Bounds::size).unwrap_or(Vec3::splat(1.0));
    (size.max_element() * PIVOT_LENGTH_FRACTION).max(PIVOT_MIN_HALF)
}

/// A 3-axis pivot marker centered on `pivot`: three colored line segments running
/// through the point along the world axes (X red, Y green, Z blue), each extending
/// `half` in both directions, so their intersection marks the object's pivot. Drawn
/// with the depth-tested line pipeline like the other overlays, so it is occluded
/// where it passes inside the mesh.
pub(crate) fn pivot_lines(pivot: [f32; 3], half: f32) -> Vec<SceneVertex> {
    // Gamma-space axis colors (the line shader returns the vertex color directly),
    // matching the floor grid's X/Z hues with a green Y.
    const X_COLOR: [f32; 4] = [0.94, 0.23, 0.28, 1.0];
    const Y_COLOR: [f32; 4] = [0.45, 0.80, 0.30, 1.0];
    const Z_COLOR: [f32; 4] = [0.18, 0.53, 1.0, 1.0];

    let pivot = Vec3::from_array(pivot);
    let mut vertices = Vec::with_capacity(6);
    for (axis, color) in [(Vec3::X, X_COLOR), (Vec3::Y, Y_COLOR), (Vec3::Z, Z_COLOR)] {
        push_line(
            &mut vertices,
            (pivot - axis * half).to_array(),
            (pivot + axis * half).to_array(),
            color,
            NO_DEFORM,
        );
    }
    vertices
}

/// The 12 edges of an axis-aligned bounding box, in the given color. The caller
/// passes whichever box it wants drawn — the whole model's bounds, or just its
/// currently-visible geometry (see [`ModelData::visible_bounds`]).
pub(crate) fn bounding_box_lines(bounds: Bounds, color: [f32; 4]) -> Vec<SceneVertex> {
    let (min, max) = (bounds.min, bounds.max);
    let corners = [
        [min.x, min.y, min.z],
        [max.x, min.y, min.z],
        [max.x, max.y, min.z],
        [min.x, max.y, min.z],
        [min.x, min.y, max.z],
        [max.x, min.y, max.z],
        [max.x, max.y, max.z],
        [min.x, max.y, max.z],
    ];
    // Pairs of corner indices: the bottom (z=min) loop, the top (z=max) loop,
    // then the four verticals connecting them.
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];

    let mut vertices = Vec::with_capacity(EDGES.len() * 2);
    for (a, b) in EDGES {
        push_line(&mut vertices, corners[a], corners[b], color, NO_DEFORM);
    }
    vertices
}
