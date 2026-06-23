//! The UV viewport's model-derived geometry: the per-channel UV wireframe traced
//! over the original polygons, the solid/island fill it sits over, and the
//! connected-component island coloring (union-find over UV-continuous faces).

use glam::Vec3;
use review_model::ModelData;

use crate::scene::SceneVertex;

use super::vertex::{push_fill_vertex, push_line};

/// Color of the model's UV edges drawn over the grid (a readable cyan-blue).
const UV_EDGE_COLOR: [f32; 4] = [0.29, 0.64, 0.91, 0.9];
/// Flat fill color for the solid-shaded UV view (a muted steel blue, gamma-space
/// like the line colors). The wireframe is drawn on top of it.
const UV_FILL_SOLID_COLOR: [f32; 4] = [0.24, 0.34, 0.46, 1.0];

/// The model's UV edges for `channel`, traced over each *original* polygon (like
/// [`wireframe_lines`](super::wireframe_lines) but in UV space): positions are the
/// per-corner UVs mapped to the UV plane as `(u, v, 0)`. Falls back to triangle
/// edges when the model arrives without face topology.
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

    // Per-face island colors, looked up per triangle via `tri_to_face`.
    let island_colors = per_island.then(|| uv_island_colors(model, channel));
    let uv_point = |vertex_index: usize| {
        let uv = model.uv_for_channel(vertex_index, channel);
        [uv.x, uv.y, 0.0]
    };

    let mut vertices = Vec::with_capacity(model.indices.len());
    for (triangle_index, triangle) in model.indices.chunks_exact(3).enumerate() {
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
                let face = model.tri_to_face.get(triangle_index).copied().unwrap_or(0) as usize;
                colors.get(face).copied().unwrap_or(UV_FILL_SOLID_COLOR)
            }
            None => UV_FILL_SOLID_COLOR,
        };
        push_fill_vertex(&mut vertices, uv_point(a), color);
        push_fill_vertex(&mut vertices, uv_point(b), color);
        push_fill_vertex(&mut vertices, uv_point(c), color);
    }

    vertices
}

/// A per-*face* fill color assigning each connected UV island its own hue.
///
/// A UV island is a connected component of faces in the UV layout. The importer
/// splits vertices per face-corner, so faces never share a vertex index — island
/// membership can't be read off the index buffer. Instead we connect faces that
/// share a 3D edge whose UVs are continuous across it: two faces meeting along an
/// edge belong to the same island unless that edge is a UV seam (the UVs differ
/// on the two sides). Connecting by 3D adjacency — not by welding UV coordinates —
/// keeps mirrored/overlapping UV islands (common in game characters) separate.
///
/// Edges are keyed by their welded 3D endpoints (quantized so shared control
/// points collapse), carrying the per-endpoint UVs; a union-find over faces finds
/// the components and each root gets a distinct color in first-seen order. The
/// returned vector is indexed by face index ([`ModelData::faces`]).
fn uv_island_colors(model: &ModelData, channel: usize) -> Vec<[f32; 4]> {
    use std::collections::HashMap;

    let face_count = model.faces.len();
    if face_count == 0 {
        // No face topology to walk — let the caller fall back to the solid fill.
        return Vec::new();
    }

    // Quantization steps: positions snap relative to the model's size so shared
    // control points collapse despite float drift; UVs snap on a fixed fine grid.
    let scale = model
        .bounds
        .map(|bounds| bounds.size().max_element())
        .unwrap_or(1.0)
        .max(1.0e-6);
    let pos_step = scale * 1.0e-5;
    let quant_pos = |p: Vec3| -> [i64; 3] {
        [
            (p.x / pos_step).round() as i64,
            (p.y / pos_step).round() as i64,
            (p.z / pos_step).round() as i64,
        ]
    };
    let quant_uv = |uv: glam::Vec2| -> [i64; 2] {
        [
            (uv.x / 1.0e-5).round() as i64,
            (uv.y / 1.0e-5).round() as i64,
        ]
    };

    // Map each 3D edge (welded endpoints, order-independent) to the faces meeting
    // there, each tagged with the UVs at the edge's low/high endpoint.
    type EdgeKey = ([i64; 3], [i64; 3]);
    type EdgeFace = (usize, [i64; 2], [i64; 2]);
    let mut edges: HashMap<EdgeKey, Vec<EdgeFace>> = HashMap::new();

    for (face_index, face) in model.faces.iter().enumerate() {
        let count = face.index_count as usize;
        if count < 2 {
            continue;
        }
        let first = face.first_index as usize;
        for corner in 0..count {
            let ia = first + corner;
            let ib = first + (corner + 1) % count;
            let (Some(va), Some(vb)) = (model.vertices.get(ia), model.vertices.get(ib)) else {
                continue;
            };
            let (pa, pb) = (quant_pos(va.position), quant_pos(vb.position));
            let uva = quant_uv(model.uv_for_channel(ia, channel));
            let uvb = quant_uv(model.uv_for_channel(ib, channel));
            // Order the endpoints by position so an edge keys identically from
            // either adjacent face, carrying the UV that goes with each endpoint.
            let (key, uv_low, uv_high) = if pa <= pb {
                ((pa, pb), uva, uvb)
            } else {
                ((pb, pa), uvb, uva)
            };
            edges
                .entry(key)
                .or_default()
                .push((face_index, uv_low, uv_high));
        }
    }

    let mut parent: Vec<u32> = (0..face_count as u32).collect();
    for faces in edges.values() {
        // Join faces meeting at this 3D edge whose UVs match at both endpoints
        // (continuous, i.e. not a seam). Manifold edges hold two faces, so the
        // pairwise scan is effectively constant.
        for (i, &(fa, la, ha)) in faces.iter().enumerate() {
            for &(fb, lb, hb) in &faces[i + 1..] {
                if la == lb && ha == hb {
                    uf_union(&mut parent, fa as u32, fb as u32);
                }
            }
        }
    }

    // Map each component root to a color index in first-seen order, so island
    // colors are stable for a given mesh rather than scattered by face id.
    let mut root_color_index = vec![u32::MAX; face_count];
    let mut island_count = 0_u32;
    let mut colors = vec![UV_FILL_SOLID_COLOR; face_count];
    for face in 0..face_count as u32 {
        let root = uf_find(&mut parent, face) as usize;
        if root_color_index[root] == u32::MAX {
            root_color_index[root] = island_count;
            island_count += 1;
        }
        colors[face as usize] = island_color(root_color_index[root]);
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
