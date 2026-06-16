//! CPU-side vertex generation for the scene's static grid and the derived debug
//! views (wireframe, face/vertex normal lines).
//!
//! These index *into* the shared [`ModelData`] buffers (invariant 1) and produce
//! flat [`SceneVertex`] arrays for upload; they never copy or re-own geometry.
//! Each builder takes its visual parameters explicitly so the renderer can
//! rebuild a single view live when its slider/color changes.

use glam::Vec3;
use review_model::ModelData;

use crate::scene::SceneVertex;

/// The static reference grid + colored X/Z axis lines, in world space. A 2 m
/// square floor (`±GRID_HALF_EXTENT`) ruled in 10 cm cells, with a stronger line
/// every 0.5 m. Iterates integer cell indices to avoid float drift.
pub(crate) fn scene_lines() -> Vec<SceneVertex> {
    let mut vertices = Vec::new();
    let extent = crate::GRID_HALF_EXTENT;
    // 10 cm cells across the half-extent, so ±10 lines for a ±1 m grid.
    const CELL: f32 = 0.1;
    let lines_per_half = (extent / CELL).round() as i32;

    for line in -lines_per_half..=lines_per_half {
        let coord = line as f32 * CELL;
        // Emphasize every 0.5 m (every 5th 10 cm line).
        let strong = line % 5 == 0;
        let color = if strong {
            [0.42, 0.49, 0.54, 0.46]
        } else {
            [0.33, 0.38, 0.42, 0.28]
        };
        push_line(
            &mut vertices,
            [coord, 0.0, -extent],
            [coord, 0.0, extent],
            color,
        );
        push_line(
            &mut vertices,
            [-extent, 0.0, coord],
            [extent, 0.0, coord],
            color,
        );
    }

    push_line(
        &mut vertices,
        [-extent, 0.002, 0.0],
        [extent, 0.002, 0.0],
        [0.94, 0.23, 0.28, 1.0],
    );
    push_line(
        &mut vertices,
        [0.0, 0.004, -extent],
        [0.0, 0.004, extent],
        [0.18, 0.53, 1.0, 1.0],
    );
    vertices
}

/// Color of the UV unit-square border (the 0..1 outline), gamma-space (the line
/// shader returns the vertex color directly into the gamma framebuffer).
const UV_GRID_BORDER_COLOR: [f32; 4] = [0.42, 0.49, 0.54, 0.7];
/// Color of the interior UV grid subdivisions (every 0.1 of the unit square).
const UV_GRID_CELL_COLOR: [f32; 4] = [0.33, 0.38, 0.42, 0.32];
/// Color of the model's UV edges drawn over the grid (a readable cyan-blue).
const UV_EDGE_COLOR: [f32; 4] = [0.29, 0.64, 0.91, 0.9];
/// Flat fill color for the solid-shaded UV view (a muted steel blue, gamma-space
/// like the line colors). The wireframe is drawn on top of it.
const UV_FILL_SOLID_COLOR: [f32; 4] = [0.24, 0.34, 0.46, 1.0];

/// The 0..1 reference grid for the UV viewport, in UV-plane coordinates
/// (positions are `(u, v, 0)`). Ten subdivisions per axis plus a stronger
/// border at 0 and 1, matching the look of the 3D floor grid.
pub(crate) fn uv_grid_lines() -> Vec<SceneVertex> {
    const CELLS: i32 = 10;
    let mut vertices = Vec::with_capacity(((CELLS + 1) * 2 * 2) as usize);

    for line in 0..=CELLS {
        let coord = line as f32 / CELLS as f32;
        // The 0 and 1 lines form the unit-square border; the rest are cells.
        let color = if line == 0 || line == CELLS {
            UV_GRID_BORDER_COLOR
        } else {
            UV_GRID_CELL_COLOR
        };
        // Vertical line (constant u) and horizontal line (constant v).
        push_line(&mut vertices, [coord, 0.0, 0.0], [coord, 1.0, 0.0], color);
        push_line(&mut vertices, [0.0, coord, 0.0], [1.0, coord, 0.0], color);
    }

    vertices
}

/// The model's UV edges for `channel`, traced over each *original* polygon (like
/// [`wireframe_lines`] but in UV space): positions are the per-corner UVs mapped
/// to the UV plane as `(u, v, 0)`. Falls back to triangle edges when the model
/// arrives without face topology.
pub(crate) fn uv_wireframe_lines(model: &ModelData, channel: u32) -> Vec<SceneVertex> {
    let channel = channel as usize;
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);

    let uv_point = |vertex_index: usize| {
        let uv = model.uv_for_channel(vertex_index, channel);
        [uv.x, uv.y, 0.0]
    };

    if model.faces.is_empty() {
        for triangle in model.indices.chunks_exact(3) {
            let [a, b, c] = [
                triangle[0] as usize,
                triangle[1] as usize,
                triangle[2] as usize,
            ];
            push_line(&mut vertices, uv_point(a), uv_point(b), UV_EDGE_COLOR);
            push_line(&mut vertices, uv_point(b), uv_point(c), UV_EDGE_COLOR);
            push_line(&mut vertices, uv_point(c), uv_point(a), UV_EDGE_COLOR);
        }
        return vertices;
    }

    for face in &model.faces {
        let count = face.index_count as usize;
        if count < 2 {
            continue;
        }
        let first = face.first_index as usize;
        for corner in 0..count {
            let a = first + corner;
            let b = first + (corner + 1) % count;
            if a < model.vertices.len() && b < model.vertices.len() {
                push_line(&mut vertices, uv_point(a), uv_point(b), UV_EDGE_COLOR);
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
/// tinted a unique color (see [`uv_island_colors`]).
pub(crate) fn uv_fill_triangles(
    model: &ModelData,
    channel: u32,
    per_island: bool,
) -> Vec<SceneVertex> {
    let channel = channel as usize;
    if model.indices.is_empty() || model.vertices.is_empty() {
        return Vec::new();
    }

    let island_colors = per_island.then(|| uv_island_colors(model));
    let uv_point = |vertex_index: usize| {
        let uv = model.uv_for_channel(vertex_index, channel);
        [uv.x, uv.y, 0.0]
    };

    let mut vertices = Vec::with_capacity(model.indices.len());
    for triangle in model.indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        if a >= model.vertices.len() || b >= model.vertices.len() || c >= model.vertices.len() {
            continue;
        }
        let color = match &island_colors {
            // All three corners share an island root, so corner `a` decides.
            Some(colors) => colors.get(a).copied().unwrap_or(UV_FILL_SOLID_COLOR),
            None => UV_FILL_SOLID_COLOR,
        };
        push_fill_vertex(&mut vertices, uv_point(a), color);
        push_fill_vertex(&mut vertices, uv_point(b), color);
        push_fill_vertex(&mut vertices, uv_point(c), color);
    }

    vertices
}

/// A per-vertex fill color assigning each connected UV island its own hue.
///
/// Islands are the connected components of the mesh as indexed: triangles that
/// share a vertex index share that vertex's UV exactly, so they're joined; a UV
/// seam duplicates the vertex (same position, different UV, distinct index), so
/// the two sides fall into different components — which is precisely a UV island.
/// A union-find over triangle membership finds the components; each root is
/// assigned a distinct color in first-seen order.
fn uv_island_colors(model: &ModelData) -> Vec<[f32; 4]> {
    let vertex_count = model.vertices.len();
    let mut parent: Vec<u32> = (0..vertex_count as u32).collect();

    for triangle in model.indices.chunks_exact(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        if (a as usize) < vertex_count && (b as usize) < vertex_count && (c as usize) < vertex_count
        {
            uf_union(&mut parent, a, b);
            uf_union(&mut parent, a, c);
        }
    }

    // Map each component root to a color index in first-seen order, so island
    // colors are stable for a given mesh rather than scattered by vertex id.
    let mut root_color_index = vec![u32::MAX; vertex_count];
    let mut island_count = 0_u32;
    let mut colors = vec![UV_FILL_SOLID_COLOR; vertex_count];
    for vertex in 0..vertex_count as u32 {
        let root = uf_find(&mut parent, vertex) as usize;
        if root_color_index[root] == u32::MAX {
            root_color_index[root] = island_count;
            island_count += 1;
        }
        colors[vertex as usize] = island_color(root_color_index[root]);
    }

    colors
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

/// HSV→RGB for `h`, `s`, `v` in 0..1, returning 0..1 RGB.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match (i as i32).rem_euclid(6) {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Union-find root with path halving.
fn uf_find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

/// Union the components containing `a` and `b`.
fn uf_union(parent: &mut [u32], a: u32, b: u32) {
    let root_a = uf_find(parent, a);
    let root_b = uf_find(parent, b);
    if root_a != root_b {
        parent[root_a as usize] = root_b;
    }
}

/// The shaded mesh vertices + indices for `uv_channel`.
pub(crate) fn model_mesh(model: &ModelData, uv_channel: u32) -> (Vec<SceneVertex>, Vec<u32>) {
    let channel = uv_channel as usize;
    let vertices = model
        .vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| SceneVertex {
            position: vertex.position.to_array(),
            normal: vertex.normal.to_array(),
            uv: model.uv_for_channel(index, channel).to_array(),
            color: vertex.color.to_array(),
            vertex_color: vertex.vertex_color.to_array(),
            smoothness: vertex.smoothness,
        })
        .collect();
    (vertices, model.indices.clone())
}

/// Wireframe line segments tracing each *original* polygon's edges (quads stay
/// quads, n-gons stay n-gons) in the given color — not the triangulated
/// diagonals, which a viewer must not show as real edges (Maya/Blender don't).
///
/// The import bridge lays out each face's corners as a contiguous run of
/// vertices, so a `TopologyFace` is the closed loop over the `index_count`
/// vertices starting at `first_index`. Falls back to triangle edges only if a
/// model somehow arrives without face topology.
pub(crate) fn wireframe_lines(model: &ModelData, color: [f32; 4]) -> Vec<SceneVertex> {
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);

    if model.faces.is_empty() {
        return triangulated_wireframe_lines(model, color);
    }

    for face in &model.faces {
        let count = face.index_count as usize;
        if count < 2 {
            continue;
        }
        let first = face.first_index as usize;

        for corner in 0..count {
            let a = first + corner;
            let b = first + (corner + 1) % count;
            let (Some(start), Some(end)) = (
                model.vertices.get(a).map(|vertex| vertex.position),
                model.vertices.get(b).map(|vertex| vertex.position),
            ) else {
                continue;
            };
            push_line(&mut vertices, start.to_array(), end.to_array(), color);
        }
    }

    vertices
}

/// Triangle-edge fallback for [`wireframe_lines`] when face topology is absent.
fn triangulated_wireframe_lines(model: &ModelData, color: [f32; 4]) -> Vec<SceneVertex> {
    let mut vertices = Vec::with_capacity(model.indices.len() * 2);

    for triangle in model.indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let positions = [
            model.vertices.get(a).map(|vertex| vertex.position),
            model.vertices.get(b).map(|vertex| vertex.position),
            model.vertices.get(c).map(|vertex| vertex.position),
        ];
        let [Some(a), Some(b), Some(c)] = positions else {
            continue;
        };

        push_line(&mut vertices, a.to_array(), b.to_array(), color);
        push_line(&mut vertices, b.to_array(), c.to_array(), color);
        push_line(&mut vertices, c.to_array(), a.to_array(), color);
    }

    vertices
}

/// The 12 edges of the model's axis-aligned bounding box, in the given color.
/// Empty when the model has no computed bounds (e.g. an empty mesh).
pub(crate) fn bounding_box_lines(model: &ModelData, color: [f32; 4]) -> Vec<SceneVertex> {
    let Some(bounds) = model.bounds else {
        return Vec::new();
    };
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
        push_line(&mut vertices, corners[a], corners[b], color);
    }
    vertices
}

/// One line per original face, from the face centroid along its averaged normal.
/// `length_scale` is relative to the model's largest extent; `color` is baked in.
pub(crate) fn face_normal_lines(
    model: &ModelData,
    length_scale: f32,
    color: [f32; 4],
) -> Vec<SceneVertex> {
    let triangle_count = model.indices.len() / 3;
    if triangle_count == 0 {
        return Vec::new();
    }

    let face_count = model
        .tri_to_face
        .iter()
        .copied()
        .max()
        .map(|max_face| max_face as usize + 1)
        .unwrap_or(triangle_count);
    let mut accum_centers = vec![Vec3::ZERO; face_count];
    let mut accum_normals = vec![Vec3::ZERO; face_count];
    let mut counts = vec![0_u32; face_count];
    let normal_length = debug_normal_length(model, length_scale);

    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
        let face_index = model
            .tri_to_face
            .get(triangle_index)
            .copied()
            .unwrap_or(triangle_index as u32) as usize;
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let positions = [
            model.vertices.get(a).map(|vertex| vertex.position),
            model.vertices.get(b).map(|vertex| vertex.position),
            model.vertices.get(c).map(|vertex| vertex.position),
        ];
        let [Some(a), Some(b), Some(c)] = positions else {
            continue;
        };

        let ab = b - a;
        let ac = c - a;
        // Normalize before the degeneracy test: the raw cross product scales with
        // triangle area, so a fixed `length_squared` threshold wrongly rejects the
        // tiny (but valid) triangles of dense regions — hands, head, boots — while
        // keeping the low-poly torso. `normalize_or_zero` only yields zero for a
        // genuinely degenerate (zero-area / non-finite) triangle.
        let normal = ab.cross(ac).normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }

        if let (Some(center_accum), Some(normal_accum), Some(count)) = (
            accum_centers.get_mut(face_index),
            accum_normals.get_mut(face_index),
            counts.get_mut(face_index),
        ) {
            *center_accum += (a + b + c) / 3.0;
            *normal_accum += normal;
            *count += 1;
        }
    }

    let mut vertices = Vec::with_capacity(face_count * 2);
    for face_index in 0..face_count {
        let count = counts[face_index];
        if count == 0 {
            continue;
        }

        let center = accum_centers[face_index] / count as f32;
        let normal = accum_normals[face_index];
        if normal.length_squared() <= f32::EPSILON {
            continue;
        }

        let end = center + normal.normalize() * normal_length;
        push_line(&mut vertices, center.to_array(), end.to_array(), color);
    }

    vertices
}

/// One line per vertex, from the vertex along its normal. `length_scale` is
/// relative to the model's largest extent; `color` is baked in.
pub(crate) fn vertex_normal_lines(
    model: &ModelData,
    length_scale: f32,
    color: [f32; 4],
) -> Vec<SceneVertex> {
    let normal_length = debug_normal_length(model, length_scale);
    let mut vertices = Vec::with_capacity(model.vertices.len() * 2);

    for vertex in &model.vertices {
        if vertex.normal.length_squared() <= f32::EPSILON {
            continue;
        }

        let start = vertex.position;
        let end = start + vertex.normal.normalize() * normal_length;
        push_line(&mut vertices, start.to_array(), end.to_array(), color);
    }

    vertices
}

/// Scale a fractional length slider value (UI clamps it to 1%–10%) into world
/// units relative to the model's largest bounding extent (so normals read
/// consistently regardless of scale).
fn debug_normal_length(model: &ModelData, scale: f32) -> f32 {
    let size = model
        .bounds
        .map(|bounds| bounds.size())
        .unwrap_or(Vec3::splat(1.0));
    let max_extent = size.max_element().max(1.0);
    max_extent * scale.max(0.001)
}

/// Push one filled-triangle corner: a zero-normal vertex carrying `color`, so the
/// scene shader returns it flat (the same path the line views use).
fn push_fill_vertex(vertices: &mut Vec<SceneVertex>, position: [f32; 3], color: [f32; 4]) {
    vertices.push(SceneVertex {
        position,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color,
        vertex_color: [0.0, 0.0, 0.0, 0.0],
        smoothness: 0.0,
    });
}

fn push_line(vertices: &mut Vec<SceneVertex>, start: [f32; 3], end: [f32; 3], color: [f32; 4]) {
    vertices.push(SceneVertex {
        position: start,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color,
        vertex_color: [0.0, 0.0, 0.0, 0.0],
        smoothness: 0.0,
    });
    vertices.push(SceneVertex {
        position: end,
        normal: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color,
        vertex_color: [0.0, 0.0, 0.0, 0.0],
        smoothness: 0.0,
    });
}
