//! One level's geometry, regrouped into a mesh per source node.
//!
//! [`build_mesh`] runs seven phases over each group, each its own function
//! below: rebake into the node's local space, validate the indices, emit the
//! vertices and their layers, map the source faces, assign a material per face,
//! carry the polygon edges, and name the geometry.

use std::collections::HashMap;
use std::ffi::CString;

use glam::{Mat3, Mat4};
use review_model::{ModelData, SourceExtras};

use crate::process::{LevelCarry, NodePolygons};
use crate::stack::HierarchyMode;
use crate::submesh::NO_FACE;

use super::*;

/// Build one mesh's arrays from `group`'s triangles: the vertices they reach,
/// and the polygon-vertex stream cut into the source faces the level's carry
/// preserved — a triangle no face survived for goes out as a triangle.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_mesh(
    model: &ModelData,
    carry: &LevelCarry,
    group: &NodeGroup,
    node: i32,
    suffix_level: Option<usize>,
    hierarchy: HierarchyMode,
    source: &ModelData,
    extras: Option<&SourceExtras>,
    unit: UnitScale,
) -> (MeshData, MeshNotes) {
    let source_node = group.source_node.and_then(|index| source.nodes.get(index));
    let part = group
        .source_node
        .zip(extras)
        .and_then(|(index, extras)| extras.mesh_of_node(index as u32));

    let LocalFrame {
        to_local,
        normal_to_local,
        direction_to_local,
    } = local_frame(hierarchy, source_node, group, extras);

    let channel_count = model.uv_channels.len();
    let has_uv = channel_count > 0 || !model.vertices.is_empty();
    let write_tangents = part.is_some_and(|part| part.tangents_authored);
    let color_set_count = carry.color_channels.len();
    let has_crease = !carry.vertex_crease.is_empty();

    let mut local_of_global: Vec<i32> = vec![-1; model.vertices.len()];
    let mut positions: Vec<f64> = Vec::new();
    let mut normals: Vec<f64> = Vec::new();
    let mut colors: Vec<f64> = Vec::new();
    let mut tangents: Vec<f64> = Vec::new();
    let mut uv_sets: Vec<Vec<f64>> = vec![Vec::new(); channel_count.max(usize::from(has_uv))];
    let mut color_values: Vec<Vec<f64>> = vec![Vec::new(); color_set_count];
    let mut vertex_crease: Vec<f64> = Vec::new();

    // An out-of-range index means a malformed mesh, and the whole triangle has
    // to go: emitting the corners that are in range would leave an index buffer
    // that is no longer a multiple of three and shift every later triangle by
    // one. Every loop below walks `group.triangles`, so every one consults this.
    let whole_triangle = |triangle: usize| {
        model.indices[triangle * 3..triangle * 3 + 3]
            .iter()
            .all(|&index| (index as usize) < model.vertices.len())
    };

    // The local vertex for a level vertex, emitted on first use.
    let mut local_vertex = |global: usize| -> i32 {
        let slot = local_of_global[global];
        if slot >= 0 {
            return slot;
        }
        let vertex = model.vertices[global];
        let slot = (positions.len() / 3) as i32;
        local_of_global[global] = slot;

        // Under `Rebuild` the node's own inverse hands back the source file's
        // unit already — import parks the unit normalization in the node
        // transforms, and the chain reproduces it at the top of the exported
        // hierarchy. Flat geometry keeps world meters and is the one place the
        // factor is applied directly (see `UnitScale::per_meter`).
        let position = match to_local {
            Some(inverse) => inverse.transform_point3(vertex.position),
            None => unit.per_meter * vertex.position,
        };
        positions.extend_from_slice(&[
            f64::from(position.x),
            f64::from(position.y),
            f64::from(position.z),
        ]);

        // Directions, so the unit factor never touches them; they do need
        // renormalizing after a non-uniform scale.
        let normal = match normal_to_local {
            Some(matrix) => (matrix * vertex.normal).normalize_or(vertex.normal),
            None => vertex.normal,
        };
        normals.extend_from_slice(&[
            f64::from(normal.x),
            f64::from(normal.y),
            f64::from(normal.z),
        ]);

        colors.extend_from_slice(&[
            f64::from(vertex.vertex_color.x),
            f64::from(vertex.vertex_color.y),
            f64::from(vertex.vertex_color.z),
            f64::from(vertex.vertex_color.w),
        ]);
        for (channel, values) in color_values.iter_mut().enumerate() {
            let color = carry.color_channels[channel]
                .get(global)
                .copied()
                .unwrap_or(glam::Vec4::ONE);
            values.extend_from_slice(&[
                f64::from(color.x),
                f64::from(color.y),
                f64::from(color.z),
                f64::from(color.w),
            ]);
        }
        if has_crease {
            vertex_crease.push(f64::from(
                carry.vertex_crease.get(global).copied().unwrap_or(0.0),
            ));
        }

        if write_tangents {
            let tangent = vertex.tangent.truncate();
            let tangent = match direction_to_local {
                Some(matrix) => (matrix * tangent).normalize_or(tangent),
                None => tangent,
            };
            tangents.extend_from_slice(&[
                f64::from(tangent.x),
                f64::from(tangent.y),
                f64::from(tangent.z),
                f64::from(vertex.tangent.w),
            ]);
        }

        if channel_count == 0 {
            if let Some(set) = uv_sets.first_mut() {
                set.extend_from_slice(&[f64::from(vertex.uv.x), f64::from(vertex.uv.y)]);
            }
        } else {
            for (channel, set) in uv_sets.iter_mut().enumerate() {
                let uv = model
                    .uv_channels
                    .get(channel)
                    .and_then(|uvs| uvs.get(global))
                    .copied()
                    .unwrap_or_default();
                set.extend_from_slice(&[f64::from(uv.x), f64::from(uv.y)]);
            }
        }
        slot
    };

    let CarriedFaces {
        pieces,
        face_of_triangle,
        any_face_smoothing,
        any_face_hole,
        any_face_group,
        any_edge_smoothing,
        any_edge_crease,
        any_edge_visibility,
    } = carried_faces(carry, group);

    // Per-face material, expressed as indices into this mesh's own slot list —
    // which is the order the bridge connects them to the node in.
    let triangle_count = model.indices.len() / 3;
    let has_material = model.triangles.material.len() == triangle_count;
    let mut material_slots: Vec<i32> = Vec::new();
    let slot_of = |global: u32, material_slots: &mut Vec<i32>| -> i32 {
        // The no-material sentinel maps to slot 0 of an empty list, which the
        // bridge writes as "no material".
        if global == u32::MAX || global as usize >= source.materials.len() {
            return 0;
        }
        match material_slots
            .iter()
            .position(|&seen| seen == global as i32)
        {
            Some(slot) => slot as i32,
            None => {
                material_slots.push(global as i32);
                (material_slots.len() - 1) as i32
            }
        }
    };

    let mut indices: Vec<i32> = Vec::with_capacity(group.triangles.len() * 3);
    let mut face_offsets: Vec<i32> = vec![0];
    let mut face_sources: Vec<u32> = Vec::new();
    let mut face_materials: Vec<i32> = Vec::new();
    let mut face_smoothing: Vec<u8> = Vec::new();
    let mut face_hole: Vec<u8> = Vec::new();
    let mut face_group: Vec<i32> = Vec::new();
    // (local a, local b) of every polygon edge -> the corner it starts at.
    let mut corner_of_edge: HashMap<(i32, i32), i32> = HashMap::new();
    let mut emitted_faces: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::new();
    let mut triangles_as_triangles = 0usize;
    let mut written_triangles = 0usize;

    let mut push_face = |corners: &[i32],
                         material: i32,
                         layers: (bool, bool, i32),
                         indices: &mut Vec<i32>,
                         face_offsets: &mut Vec<i32>| {
        let first = indices.len();
        indices.extend_from_slice(corners);
        face_offsets.push(indices.len() as i32);
        face_materials.push(material);
        if any_face_smoothing {
            face_smoothing.push(u8::from(layers.0));
        }
        if any_face_hole {
            face_hole.push(u8::from(layers.1));
        }
        if any_face_group {
            face_group.push(layers.2);
        }
        for (offset, &corner) in corners.iter().enumerate() {
            let next = corners[(offset + 1) % corners.len()];
            corner_of_edge
                .entry((corner, next))
                .or_insert((first + offset) as i32);
        }
        written_triangles += corners.len().saturating_sub(2);
    };

    for &triangle in &group.triangles {
        if !whole_triangle(triangle) {
            continue;
        }
        let material = if has_material {
            slot_of(model.triangles.material[triangle], &mut material_slots)
        } else {
            0
        };
        match face_of_triangle.get(&triangle) {
            Some(&(piece_index, face)) => {
                if !emitted_faces.insert((piece_index, face)) {
                    continue;
                }
                let piece = pieces[piece_index];
                // Every corner of a carried face is a vertex some triangle of
                // this group reaches, so all resolve.
                let corners: Vec<i32> = piece
                    .face(face)
                    .iter()
                    .map(|&global| local_vertex(global as usize))
                    .collect();
                let layers = (
                    piece.face_smoothing.get(face).copied().unwrap_or(false),
                    piece.face_hole.get(face).copied().unwrap_or(false),
                    piece.face_group.get(face).copied().unwrap_or(0) as i32,
                );
                push_face(&corners, material, layers, &mut indices, &mut face_offsets);
                face_sources.push(piece.source_face.get(face).copied().unwrap_or(u32::MAX));
            }
            None => {
                let corners: Vec<i32> = (0..3)
                    .map(|corner| local_vertex(model.indices[triangle * 3 + corner] as usize))
                    .collect();
                push_face(
                    &corners,
                    material,
                    (false, false, 0),
                    &mut indices,
                    &mut face_offsets,
                );
                face_sources.push(u32::MAX);
                if !pieces.is_empty() {
                    triangles_as_triangles += 1;
                }
            }
        }
    }
    if material_slots.len() <= 1 {
        // The bridge only reads per-face assignment for a multi-material mesh.
        face_materials.clear();
    }

    let EdgeStreams {
        edges,
        edge_sources,
        edge_smoothing,
        edge_crease,
        edge_visibility,
    } = edge_streams(
        &pieces,
        &local_of_global,
        &corner_of_edge,
        (any_edge_smoothing, any_edge_crease, any_edge_visibility),
    );

    // The geometry element keeps its own authored name when the capture has
    // it; a node and its mesh are named separately in FBX.
    let name = part
        .filter(|part| !part.name.is_empty())
        .map(|part| part.name.clone())
        .or_else(|| source_node.map(|node| node.name.clone()))
        .unwrap_or_else(|| model.name.clone());
    let name = match suffix_level {
        Some(level) => suffixed(&name, level),
        None => name,
    };

    let uv_set_names: Vec<CString> = (0..uv_sets.len())
        .map(|channel| {
            let label = source
                .uv_set_names
                .get(channel)
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("UVMap{channel}"));
            c_string(&label, "UVMap")
        })
        .collect();
    let color_set_name = part
        .and_then(|part| part.color_sets.first())
        .filter(|set| !set.name.is_empty())
        .map(|set| c_string(&set.name, "Col"));
    let color_sets: Vec<(CString, Vec<f64>)> = color_values
        .into_iter()
        .enumerate()
        .map(|(channel, values)| {
            let label = part
                .and_then(|part| part.color_sets.get(channel + 1))
                .map(|set| set.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("Col{}", channel + 1));
            (c_string(&label, "Col"), values)
        })
        .collect();

    let notes = MeshNotes {
        triangles_rebuilt: triangles_as_triangles,
        polygons_lost: pieces.is_empty() && part.is_some() && extras.is_some(),
    };
    let vertex_count = positions.len() / 3;
    let mut level_vertices = vec![0u32; vertex_count];
    for (level_vertex, &local) in local_of_global.iter().enumerate() {
        if local >= 0 {
            level_vertices[local as usize] = level_vertex as u32;
        }
    }
    let source_corners: Vec<u32> = level_vertices
        .iter()
        .map(|&level_vertex| {
            carry
                .source_corner
                .get(level_vertex as usize)
                .copied()
                .unwrap_or(u32::MAX)
        })
        .collect();
    let mesh = MeshData {
        name: c_string(&name, "Mesh"),
        node,
        positions,
        indices,
        face_offsets,
        triangle_count: written_triangles,
        vertex_count,
        normals,
        colors,
        tangents,
        uv_sets,
        uv_set_names,
        material_slots,
        face_materials,
        color_set_name,
        color_sets,
        face_smoothing,
        face_hole,
        face_group,
        edges,
        edge_smoothing,
        edge_crease,
        edge_visibility,
        vertex_crease,
        props: PropRange::default(),
        level_vertices,
        source_corners,
        face_sources,
        edge_sources,
        channel_sources: Vec::new(),
        source_node: group.source_node,
        level: suffix_level.unwrap_or(0),
        skins: Vec::new(),
        blend_channels: Vec::new(),
    };
    (mesh, notes)
}

/// What a mesh had to give up, for the report.
pub(crate) struct MeshNotes {
    /// Triangles written as triangles inside a mesh that otherwise kept its
    /// polygons.
    pub(crate) triangles_rebuilt: usize,
    /// The whole mesh went out as triangles although the source had polygons.
    pub(crate) polygons_lost: bool,
}

/// The maps that take world-baked geometry back into its node's local space.
///
/// All three are `None` when the geometry stays in world space — a flattened
/// hierarchy, a node with no inverse, or no source node at all.
pub(crate) struct LocalFrame {
    /// Positions.
    pub(crate) to_local: Option<Mat4>,
    /// Normals.
    pub(crate) normal_to_local: Option<Mat3>,
    /// Tangents and other surface directions.
    pub(crate) direction_to_local: Option<Mat3>,
}

/// Build the [`LocalFrame`] for one node's geometry.
///
/// Geometry is world-baked; under `Rebuild` it has to move back into the owning
/// node's local space, or it would be transformed twice on import. With the
/// capture that space is the *geometry* space — the node's geometric transform
/// (written back as its `Geometric*` properties) sits between the two.
pub(crate) fn local_frame(
    hierarchy: HierarchyMode,
    source_node: Option<&review_model::SceneNode>,
    group: &NodeGroup,
    extras: Option<&SourceExtras>,
) -> LocalFrame {
    let node_world = (hierarchy == HierarchyMode::Rebuild)
        .then(|| {
            let node = source_node?;
            let geometry_to_node = group
                .source_node
                .zip(extras)
                .and_then(|(index, extras)| extras.nodes.get(index))
                .map_or(Mat4::IDENTITY, |authored| authored.geometry_to_node);
            let world = node.transform * geometry_to_node;
            world.inverse().is_finite().then_some(world)
        })
        .flatten();
    let to_local = node_world.map(|world| world.inverse());
    LocalFrame {
        to_local,
        // A normal rides the *inverse transpose* of the map its positions take,
        // and that map is `to_local` — so the normal's matrix is the transpose
        // of the node's own world transform, not of its inverse. The two agree
        // for a uniform scale, which is why only a rotated node shows the
        // difference: it rotates every normal the wrong way round.
        normal_to_local: node_world.map(|world| Mat3::from_mat4(world).transpose()),
        // Tangents are directions along the surface, so they take the same map
        // as the positions (without the translation), not the normal's.
        direction_to_local: to_local.map(Mat3::from_mat4),
    }
}

/// The source faces this node's geometry still belongs to, and which of their
/// layers anything authored.
pub(crate) struct CarriedFaces<'a> {
    /// The carry pieces that belong to this node.
    pub(crate) pieces: Vec<&'a NodePolygons>,
    /// Level triangle -> (piece, face within it).
    pub(crate) face_of_triangle: HashMap<usize, (usize, usize)>,
    pub(crate) any_face_smoothing: bool,
    pub(crate) any_face_hole: bool,
    pub(crate) any_face_group: bool,
    pub(crate) any_edge_smoothing: bool,
    pub(crate) any_edge_crease: bool,
    pub(crate) any_edge_visibility: bool,
}

/// Which carried face each level triangle belongs to, for this node's pieces.
pub(crate) fn carried_faces<'a>(carry: &'a LevelCarry, group: &NodeGroup) -> CarriedFaces<'a> {
    let pieces: Vec<&NodePolygons> = carry
        .polygons
        .iter()
        .filter(|piece| group.source_node == Some(piece.node as usize))
        .collect();
    let mut face_of_triangle: HashMap<usize, (usize, usize)> = HashMap::new();
    for (piece_index, piece) in pieces.iter().enumerate() {
        for (offset, &face) in piece.triangle_face.iter().enumerate() {
            if face != NO_FACE {
                face_of_triangle.insert(
                    piece.triangle_first as usize + offset,
                    (piece_index, face as usize),
                );
            }
        }
    }
    CarriedFaces {
        any_face_smoothing: pieces.iter().any(|piece| !piece.face_smoothing.is_empty()),
        any_face_hole: pieces.iter().any(|piece| !piece.face_hole.is_empty()),
        any_face_group: pieces.iter().any(|piece| !piece.face_group.is_empty()),
        any_edge_smoothing: pieces.iter().any(|piece| !piece.edge_smoothing.is_empty()),
        any_edge_crease: pieces.iter().any(|piece| !piece.edge_crease.is_empty()),
        any_edge_visibility: pieces.iter().any(|piece| !piece.edge_visibility.is_empty()),
        pieces,
        face_of_triangle,
    }
}

/// The polygon edges of one mesh, and the layers over them.
pub(crate) struct EdgeStreams {
    /// Each edge as the corner it starts at.
    pub(crate) edges: Vec<i32>,
    pub(crate) edge_sources: Vec<u32>,
    pub(crate) edge_smoothing: Vec<u8>,
    pub(crate) edge_crease: Vec<f64>,
    pub(crate) edge_visibility: Vec<u8>,
}

/// Name each carried edge by the corner it starts at in the stream just built.
///
/// An edge whose face the level no longer has cannot be named, and is dropped.
pub(crate) fn edge_streams(
    pieces: &[&NodePolygons],
    local_of_global: &[i32],
    corner_of_edge: &HashMap<(i32, i32), i32>,
    layers: (bool, bool, bool),
) -> EdgeStreams {
    let (any_smoothing, any_crease, any_visibility) = layers;
    let mut out = EdgeStreams {
        edges: Vec::new(),
        edge_sources: Vec::new(),
        edge_smoothing: Vec::new(),
        edge_crease: Vec::new(),
        edge_visibility: Vec::new(),
    };
    for piece in pieces {
        for (index, edge) in piece.edges.iter().enumerate() {
            let a = local_of_global[edge[0] as usize];
            let b = local_of_global[edge[1] as usize];
            if a < 0 || b < 0 {
                continue;
            }
            let Some(&corner) = corner_of_edge
                .get(&(a, b))
                .or_else(|| corner_of_edge.get(&(b, a)))
            else {
                continue;
            };
            out.edges.push(corner);
            out.edge_sources
                .push(piece.source_edge.get(index).copied().unwrap_or(u32::MAX));
            if any_smoothing {
                out.edge_smoothing.push(u8::from(
                    piece.edge_smoothing.get(index).copied().unwrap_or(false),
                ));
            }
            if any_crease {
                out.edge_crease.push(f64::from(
                    piece.edge_crease.get(index).copied().unwrap_or(0.0),
                ));
            }
            if any_visibility {
                out.edge_visibility.push(u8::from(
                    piece.edge_visibility.get(index).copied().unwrap_or(true),
                ));
            }
        }
    }
    out
}
