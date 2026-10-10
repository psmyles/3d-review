//! The UV viewport's model-derived geometry: the per-channel UV wireframe traced
//! over the original polygons, the solid/island fill it sits over, and the
//! connected-component island coloring (union-find over UV-continuous faces).

use review_model::ModelData;

use crate::scene::SceneVertex;

use super::deform::NO_DEFORM;
use super::vertex::{push_fill_vertex, push_line};

/// Color of the model's UV edges drawn over the grid (a readable cyan-blue).
const UV_EDGE_COLOR: [f32; 4] = [0.29, 0.64, 0.91, 0.9];
/// Flat fill color for the solid-shaded UV view (a muted steel blue, gamma-space
/// like the line colors). The wireframe is drawn on top of it.
const UV_FILL_SOLID_COLOR: [f32; 4] = [0.24, 0.34, 0.46, 1.0];

/// Which nodes' UVs the UV viewport lays out: the Outliner's node selection when
/// there is one, else every node, and in both cases minus the meshes the Outliner
/// has hidden — so hiding a part hides it in every workspace, and selecting parts
/// isolates their layout.
///
/// Resolved once per rebuild into a per-node inclusion mask; `None` draws
/// everything (nothing selected and nothing hidden, or a model whose per-triangle
/// node info cannot resolve a node at all — the same rule [`super::hidden::HiddenFilter`]
/// follows, so a malformed model still shows its UVs).
pub(crate) struct UvNodeScope<'a> {
    include: Option<Vec<bool>>,
    /// What a node outside the mask's range resolves to: excluded under a
    /// selection (it was not selected), included otherwise (it was not hidden).
    out_of_range: bool,
    /// Per-triangle owning node, present only when it runs parallel to the
    /// triangle list (so a lookup can't index out of range).
    triangle_node: Option<&'a [u32]>,
}

impl<'a> UvNodeScope<'a> {
    /// Resolve `selected_nodes` (sorted mesh/group node indices; a node covers its
    /// whole subtree) and `hidden_nodes` (Outliner-hidden mesh nodes) against
    /// `model`.
    pub(crate) fn new(model: &'a ModelData, selected_nodes: &[u32], hidden_nodes: &[u32]) -> Self {
        let triangle_count = model.indices.len() / 3;
        let triangle_node = (triangle_count != 0 && model.triangles.node.len() == triangle_count)
            .then_some(model.triangles.node.as_slice());
        if (selected_nodes.is_empty() && hidden_nodes.is_empty()) || triangle_node.is_none() {
            return Self {
                include: None,
                out_of_range: true,
                triangle_node: None,
            };
        }

        let node_count = model.nodes.len();
        let mut include = if selected_nodes.is_empty() {
            vec![true; node_count]
        } else {
            subtree_union(model, selected_nodes)
        };
        for &hidden in hidden_nodes {
            if let Some(slot) = include.get_mut(hidden as usize) {
                *slot = false;
            }
        }
        Self {
            include: Some(include),
            out_of_range: selected_nodes.is_empty(),
            triangle_node,
        }
    }

    /// Whether scene-graph node `node` is laid out.
    fn includes_node(&self, node: u32) -> bool {
        match &self.include {
            None => true,
            Some(mask) => mask
                .get(node as usize)
                .copied()
                .unwrap_or(self.out_of_range),
        }
    }

    /// Whether triangle `index` is laid out.
    fn includes_triangle(&self, index: usize) -> bool {
        match self.triangle_node {
            None => true,
            Some(nodes) => nodes
                .get(index)
                .is_none_or(|&node| self.includes_node(node)),
        }
    }

    /// Whether every triangle is laid out, which lets a builder skip the per-face
    /// node map entirely.
    fn includes_everything(&self) -> bool {
        self.include.is_none()
    }
}

/// Per-node mask of the union of the subtrees rooted at `roots`, in one
/// O(nodes × depth) pass however many roots there are (rather than one
/// [`ModelData::node_subtree_mask`] walk per root). A guard bounds a malformed
/// parent cycle.
fn subtree_union(model: &ModelData, roots: &[u32]) -> Vec<bool> {
    let node_count = model.nodes.len();
    let mut is_root = vec![false; node_count];
    for &root in roots {
        if let Some(slot) = is_root.get_mut(root as usize) {
            *slot = true;
        }
    }
    (0..node_count)
        .map(|index| {
            let mut current = Some(index);
            let mut guard = 0;
            while let Some(node) = current {
                if is_root[node] {
                    return true;
                }
                current = model.nodes.get(node).and_then(|node| node.parent);
                guard += 1;
                if guard > node_count || current.is_some_and(|parent| parent >= node_count) {
                    break;
                }
            }
            false
        })
        .collect()
}

/// The model's UV edges for `channel`, traced over each *original* polygon (like
/// the model wireframe, [`super::wireframe_edge_indices`], but in UV space):
/// positions are the per-corner UVs mapped to the UV plane as `(u, v, 0)`. Falls
/// back to triangle edges when the model arrives without face topology. Only the
/// faces `scope` lays out are traced.
pub(crate) fn uv_wireframe_lines(
    model: &ModelData,
    channel: u32,
    scope: &UvNodeScope<'_>,
) -> Vec<SceneVertex> {
    let channel = channel as usize;
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);

    let uv_point = |vertex_index: usize| {
        let uv = model.uv_for_channel(vertex_index, channel);
        [uv.x, uv.y, 0.0]
    };

    if model.faces.is_empty() {
        for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
            if !scope.includes_triangle(triangle_index) {
                continue;
            }
            let [a, b, c] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            push_line(
                &mut vertices,
                uv_point(a),
                uv_point(b),
                UV_EDGE_COLOR,
                NO_DEFORM,
            );
            push_line(
                &mut vertices,
                uv_point(b),
                uv_point(c),
                UV_EDGE_COLOR,
                NO_DEFORM,
            );
            push_line(
                &mut vertices,
                uv_point(c),
                uv_point(a),
                UV_EDGE_COLOR,
                NO_DEFORM,
            );
        }
        return vertices;
    }

    // Faces are filtered through their owning node. A model whose faces can't be
    // mapped to nodes can't resolve the scope, so it traces every face — the same
    // fallback the 3D wireframe takes for a hidden mesh.
    let face_node = (!scope.includes_everything())
        .then(|| model.face_nodes())
        .flatten();
    for (face_index, face) in model.faces.iter().enumerate() {
        if let Some(map) = face_node.as_ref()
            && map
                .get(face_index)
                .is_some_and(|&node| node != u32::MAX && !scope.includes_node(node))
        {
            continue;
        }
        let count = face.index_count as usize;
        if count < 2 {
            continue;
        }
        let first = face.first_index as usize;
        for corner in 0..count {
            let a = first + corner;
            let b = first + (corner + 1) % count;
            if a < model.vertices.len() && b < model.vertices.len() {
                push_line(
                    &mut vertices,
                    uv_point(a),
                    uv_point(b),
                    UV_EDGE_COLOR,
                    NO_DEFORM,
                );
            }
        }
    }

    vertices
}

/// Filled UV-space triangles for `channel`, used as the solid layer the UV
/// wireframe is drawn over. Positions are the triangulated per-corner UVs mapped
/// to the UV plane as `(u, v, 0)`, with a zero normal so the scene shader returns
/// the baked vertex color flat (no lighting). When `per_island` is false every
/// triangle gets [`UV_FILL_SOLID_COLOR`]; when true each connected UV island is
/// tinted a unique color (see [`uv_island_colors`]). Only the triangles `scope`
/// lays out are filled; island colors are assigned over the *whole* model, so a
/// part keeps its color as the selection changes around it. `opacity` scales every
/// color's alpha: below 1 the fill lets a texture behind the layout show through.
pub(crate) fn uv_fill_triangles(
    model: &ModelData,
    channel: u32,
    per_island: bool,
    opacity: f32,
    scope: &UvNodeScope<'_>,
) -> Vec<SceneVertex> {
    let channel = channel as usize;
    if model.indices.is_empty() || model.vertices.is_empty() {
        return Vec::new();
    }

    // Per-face island colors, looked up per triangle via `tri_to_face`.
    let island_colors = per_island.then(|| uv_island_colors(model, channel));
    let uv_point = |vertex_index: usize| {
        let uv = model.uv_for_channel(vertex_index, channel);
        [uv.x, uv.y, 0.0]
    };

    let mut vertices = Vec::with_capacity(model.indices.len());
    for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        if !scope.includes_triangle(triangle_index) {
            continue;
        }
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        if a >= model.vertices.len() || b >= model.vertices.len() || c >= model.vertices.len() {
            continue;
        }
        let color = match &island_colors {
            // All triangles of a face belong to the same island; resolve the
            // owning face for this triangle and use its island color.
            Some(colors) => {
                let face = model
                    .triangles
                    .to_face
                    .get(triangle_index)
                    .copied()
                    .unwrap_or(0) as usize;
                colors.get(face).copied().unwrap_or(UV_FILL_SOLID_COLOR)
            }
            None => UV_FILL_SOLID_COLOR,
        };
        let color = [color[0], color[1], color[2], color[3] * opacity];
        push_fill_vertex(&mut vertices, uv_point(a), color, NO_DEFORM);
        push_fill_vertex(&mut vertices, uv_point(b), color, NO_DEFORM);
        push_fill_vertex(&mut vertices, uv_point(c), color, NO_DEFORM);
    }

    vertices
}

/// A per-*face* fill color assigning each connected UV island its own hue, in
/// first-seen order so colors are stable for a given mesh. The islands
/// themselves come from [`review_model::uv_islands`], which joins faces by 3D
/// adjacency so mirrored and overlapping islands stay apart. Indexed by face
/// ([`ModelData::faces`]); empty for a mesh without face topology, which the
/// caller fills solid.
fn uv_island_colors(model: &ModelData, channel: usize) -> Vec<[f32; 4]> {
    if model.faces.is_empty() {
        return Vec::new();
    }
    review_model::uv_islands::uv_islands(model, channel, |_| false)
        .island
        .into_iter()
        .map(island_color)
        .collect()
}

/// The Aud workspace's offending triangles in UV space: one flat-coloured
/// triangle per entry of `fills[severity]`, coloured `colors[severity]`, worst
/// severity last so it covers the rest.
pub(crate) fn uv_highlight_triangles(
    model: &ModelData,
    channel: u32,
    fills: [&[u32]; 3],
    colors: [[f32; 4]; 3],
) -> Vec<SceneVertex> {
    let channel = channel as usize;
    let uv_point = |vertex_index: u32| {
        let uv = model.uv_for_channel(vertex_index as usize, channel);
        [uv.x, uv.y, 0.0]
    };
    let mut vertices = Vec::new();
    for (severity, triangles) in fills.iter().enumerate() {
        for &triangle in *triangles {
            let Some(corners) = model
                .indices
                .get(triangle as usize * 3..triangle as usize * 3 + 3)
            else {
                continue;
            };
            for &corner in corners {
                push_fill_vertex(&mut vertices, uv_point(corner), colors[severity], NO_DEFORM);
            }
        }
    }
    vertices
}

/// Distinct, evenly-spread island color for the `index`-th UV island. Hues step
/// by the golden-ratio conjugate so successive islands stay far apart on the
/// wheel; saturation/value are fixed for a cohesive, readable palette. Returned
/// in gamma space (the scene shader emits a zero-normal vertex color directly).
fn island_color(index: u32) -> [f32; 4] {
    let hue = (index as f32 * 0.618_034).fract();
    let [r, g, b] = hsv_to_rgb(hue, 0.55, 0.80);
    [r, g, b, 1.0]
}

/// HSV to RGB, each component in `0..=1`; the hue wraps. What the RGB means
/// (gamma or linear) is the caller's: this is only the colour-wheel arithmetic,
/// shared by the UV islands and the Unique material mode's per-part hues.
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let sector = h.floor() as i32;
    let f = h - sector as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match sector.rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3, Vec4};
    use review_model::{SceneNode, TopologyFace, TriangleData, Vertex};

    use super::*;

    /// Two triangles sharing the edge (1,0,0)-(0,1,0), corner-split as import
    /// leaves them, with the UVs each face gives its three corners.
    fn two_faces(first: [Vec2; 3], second: [Vec2; 3]) -> ModelData {
        let positions = [
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            Vec3::X,
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::Y,
        ];
        let uvs = first.into_iter().chain(second);
        let vertices = positions
            .into_iter()
            .zip(uvs)
            .map(|(position, uv)| Vertex {
                position,
                normal: Vec3::Z,
                uv,
                tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
                vertex_color: Vec4::ONE,
            })
            .collect();
        let mut model = ModelData {
            vertices,
            indices: (0..6).collect(),
            faces: vec![
                TopologyFace {
                    first_index: 0,
                    index_count: 3,
                },
                TopologyFace {
                    first_index: 3,
                    index_count: 3,
                },
            ],
            ..Default::default()
        };
        model.recompute_bounds();
        model
    }

    #[test]
    fn faces_whose_uvs_agree_across_their_edge_are_one_island() {
        let model = two_faces(
            [Vec2::ZERO, Vec2::X, Vec2::Y],
            [Vec2::X, Vec2::ONE, Vec2::Y],
        );
        let colors = uv_island_colors(&model, 0);
        assert_eq!(colors.len(), 2);
        assert_eq!(colors[0], colors[1]);
    }

    #[test]
    fn a_seam_splits_the_faces_into_two_islands() {
        // The second face's copy of the shared edge sits elsewhere in UV space.
        let seamed = two_faces(
            [Vec2::ZERO, Vec2::X, Vec2::Y],
            [
                Vec2::new(3.0, 0.0),
                Vec2::new(4.0, 1.0),
                Vec2::new(3.0, 1.0),
            ],
        );
        let colors = uv_island_colors(&seamed, 0);
        assert_ne!(colors[0], colors[1]);
    }

    #[test]
    fn a_mirrored_edge_is_a_seam_even_where_the_uvs_coincide() {
        // Both faces map the shared edge onto the same UV segment, but swapped
        // end for end: welding by UV alone would call this continuous.
        let mirrored = two_faces(
            [Vec2::ZERO, Vec2::X, Vec2::Y],
            [Vec2::Y, Vec2::ONE, Vec2::X],
        );
        let colors = uv_island_colors(&mirrored, 0);
        assert_ne!(colors[0], colors[1]);
    }

    #[test]
    fn a_mesh_without_face_topology_has_no_islands_to_colour() {
        let mut model = two_faces([Vec2::ZERO; 3], [Vec2::ZERO; 3]);
        model.faces.clear();
        assert!(uv_island_colors(&model, 0).is_empty());
    }
    /// [`two_faces`] with each face owned by its own node — node 1 a child of
    /// node 0 — the way import tags a two-part hierarchy.
    fn two_parts() -> ModelData {
        let mut model = two_faces(
            [Vec2::ZERO, Vec2::X, Vec2::Y],
            [Vec2::X, Vec2::ONE, Vec2::Y],
        );
        model.nodes = vec![
            SceneNode::default(),
            SceneNode {
                parent: Some(0),
                ..SceneNode::default()
            },
        ];
        model.triangles = TriangleData {
            to_face: vec![0, 1],
            material: vec![0, 0],
            node: vec![0, 1],
        };
        model
    }

    /// How many faces' edges a UV wireframe traced (three edges, two vertices
    /// each, per triangle face).
    fn traced_faces(model: &ModelData, selected: &[u32], hidden: &[u32]) -> usize {
        let scope = UvNodeScope::new(model, selected, hidden);
        uv_wireframe_lines(model, 0, &scope).len() / 6
    }

    /// How many triangles a UV fill laid out.
    fn filled_triangles(model: &ModelData, selected: &[u32], hidden: &[u32]) -> usize {
        let scope = UvNodeScope::new(model, selected, hidden);
        uv_fill_triangles(model, 0, false, 1.0, &scope).len() / 3
    }

    #[test]
    fn nothing_selected_or_hidden_lays_out_every_face() {
        let model = two_parts();
        assert_eq!(traced_faces(&model, &[], &[]), 2);
        assert_eq!(filled_triangles(&model, &[], &[]), 2);
    }

    #[test]
    fn a_selection_lays_out_only_its_subtree() {
        let model = two_parts();
        // The child alone is its own face.
        assert_eq!(traced_faces(&model, &[1], &[]), 1);
        assert_eq!(filled_triangles(&model, &[1], &[]), 1);
        // The parent covers its child too.
        assert_eq!(traced_faces(&model, &[0], &[]), 2);
        assert_eq!(filled_triangles(&model, &[0], &[]), 2);
    }

    #[test]
    fn a_hidden_mesh_is_never_laid_out_even_when_selected() {
        let model = two_parts();
        assert_eq!(traced_faces(&model, &[], &[0]), 1);
        assert_eq!(filled_triangles(&model, &[], &[0]), 1);
        assert_eq!(traced_faces(&model, &[1], &[1]), 0);
        assert_eq!(filled_triangles(&model, &[1], &[1]), 0);
    }

    /// A model whose triangles carry no node info can't resolve the scope, so
    /// it draws everything rather than nothing — the hidden-mesh filter's rule.
    #[test]
    fn a_model_without_node_info_lays_out_everything() {
        let mut model = two_parts();
        model.triangles.node.clear();
        assert_eq!(traced_faces(&model, &[1], &[0]), 2);
        assert_eq!(filled_triangles(&model, &[1], &[0]), 2);
    }

    /// The triangle fallback (no face topology) honours the scope too.
    #[test]
    fn the_triangle_fallback_honours_the_scope() {
        let mut model = two_parts();
        model.faces.clear();
        assert_eq!(traced_faces(&model, &[1], &[]), 1);
        assert_eq!(traced_faces(&model, &[], &[]), 2);
    }
}
