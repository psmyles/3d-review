//! Rebuilding one `ModelData` per level out of the processed submeshes.
//!
//! A level that deforms gets its own `SkinData` / `MorphData` over its own
//! vertices, the source's clips cloned, and bounds measured at rest — the
//! processed mesh has to animate exactly as the source did.

use review_model::{
    AnimContext, ModelData, ModelStats, MorphData, SkinData, TriangleData, Vertex, anim,
};

use crate::Warnings;
use crate::submesh::{Submesh, TagPresence};

use super::*;

/// Rebuild a [`ModelData`] from processed submeshes.
///
/// The output is a pure triangle mesh, so it carries **no** face topology:
/// `faces` and `triangles.to_face` are both left empty. That is not a gap — the
/// original polygon table describes contiguous corner runs in the *source*
/// vertex array, a layout welding and simplification necessarily destroy, and
/// every consumer in `render` already falls back to per-triangle behaviour when
/// the table is absent (the wireframe draws triangle edges, face normals get one
/// slot per triangle, UV islands fall back to the solid fill). Synthesizing a
/// plausible-looking table instead would draw a wireframe that is simply wrong.
/// Which derived per-vertex bases the assembled mesh has to rebuild. Both are
/// computed from the geometry, so recomputing one that is still valid would just
/// overwrite the source file's authored values with synthesized ones.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rebuild {
    pub(crate) normals: bool,
    pub(crate) tangents: bool,
}

pub(crate) fn assemble(
    submeshes: &[Submesh],
    source: &ModelData,
    tags: TagPresence,
    level: usize,
    rebuild: Rebuild,
    warnings: &mut Warnings,
) -> (ModelData, LevelCarry) {
    let _z = crate::prof::zone!("Assemble Model");

    let total_vertices: usize = submeshes.iter().map(|piece| piece.vertices.len()).sum();
    let total_indices: usize = submeshes.iter().map(|piece| piece.indices.len()).sum();
    let channel_count = source.uv_channels.len();
    let color_channel_count = submeshes
        .iter()
        .map(|piece| piece.color_channels.len())
        .max()
        .unwrap_or(0);
    let any_crease = submeshes
        .iter()
        .any(|piece| !piece.vertex_crease.is_empty());

    let mut vertices: Vec<Vertex> = Vec::with_capacity(total_vertices);
    let mut indices: Vec<u32> = Vec::with_capacity(total_indices);
    let mut uv_channels: Vec<Vec<glam::Vec2>> =
        vec![Vec::with_capacity(total_vertices); channel_count];
    let mut triangle_node: Vec<u32> = Vec::new();
    let mut triangle_material: Vec<u32> = Vec::new();
    let vertex_count: usize = submeshes
        .iter()
        .filter(|piece| !piece.is_empty())
        .map(|piece| piece.vertices.len())
        .sum();
    let mut carry = LevelCarry {
        polygons: Vec::new(),
        color_channels: vec![Vec::with_capacity(total_vertices); color_channel_count],
        vertex_crease: Vec::with_capacity(if any_crease { total_vertices } else { 0 }),
        extra_skins: Vec::new(),
        dq_weights: Vec::new(),
        source_corner: Vec::with_capacity(total_vertices),
    };
    // The deform tables over the level's own vertices, each of which is its own
    // logical vertex now (`corner_to_logical` is the identity). Built only when
    // the source had the table: a level of an unskinned model stays `None`.
    let any_skin = source.skin.is_some() && submeshes.iter().any(|piece| !piece.skin.is_empty());
    let any_morph = source.morph.is_some() && submeshes.iter().any(|piece| !piece.morph.is_empty());
    let mut skin_offsets: Vec<u32> = Vec::new();
    let mut skin_bones: Vec<u32> = Vec::new();
    let mut skin_weights: Vec<f32> = Vec::new();
    let mut skin_clusters: Vec<u32> = Vec::new();
    let mut morph_offsets: Vec<u32> = Vec::new();
    let mut morph_shape: Vec<u32> = Vec::new();
    let mut morph_position: Vec<glam::Vec3> = Vec::new();
    let mut morph_normal: Vec<glam::Vec3> = Vec::new();
    if any_skin {
        skin_offsets.push(0);
    }
    if any_morph {
        morph_offsets.push(0);
    }

    for piece in submeshes {
        if piece.is_empty() {
            continue;
        }
        let base = vertices.len() as u32;
        let triangle_first = (indices.len() / 3) as u32;
        vertices.extend_from_slice(&piece.vertices);
        indices.extend(piece.indices.iter().map(|&index| index + base));
        if piece.source_corner.len() == piece.vertices.len() {
            carry.source_corner.extend_from_slice(&piece.source_corner);
        } else {
            carry.source_corner.resize(vertices.len(), u32::MAX);
        }

        for (channel, destination) in carry.color_channels.iter_mut().enumerate() {
            match piece.color_channels.get(channel) {
                Some(colors) => destination.extend_from_slice(colors),
                None => destination.resize(vertices.len(), glam::Vec4::ONE),
            }
        }
        if any_crease {
            if piece.vertex_crease.is_empty() {
                carry.vertex_crease.resize(vertices.len(), 0.0);
            } else {
                carry.vertex_crease.extend_from_slice(&piece.vertex_crease);
            }
        }
        if any_skin {
            for vertex in 0..piece.vertices.len() {
                for &(cluster, weight) in piece.skin.row(vertex) {
                    let bone = source
                        .skin
                        .as_ref()
                        .and_then(|skin| skin.clusters.get(cluster as usize))
                        .map_or(u32::MAX, |entry| entry.bone);
                    skin_bones.push(bone);
                    skin_weights.push(weight);
                    skin_clusters.push(cluster);
                }
                skin_offsets.push(skin_bones.len() as u32);
            }
        }
        if any_morph {
            for vertex in 0..piece.vertices.len() {
                for entry in piece.morph.row(vertex) {
                    morph_shape.push(entry.shape);
                    morph_position.push(entry.position);
                    morph_normal.push(entry.normal);
                }
                morph_offsets.push(morph_shape.len() as u32);
            }
        }
        for (layer, rows) in piece.extra_skins.iter().enumerate() {
            if rows.is_empty() {
                continue;
            }
            let slot = match carry
                .extra_skins
                .iter()
                .position(|entry| entry.node == piece.node && entry.layer == layer)
            {
                Some(slot) => slot,
                None => {
                    carry.extra_skins.push(LevelSkinLayer {
                        node: piece.node,
                        layer,
                        influences: Vec::new(),
                    });
                    carry.extra_skins.len() - 1
                }
            };
            for vertex in 0..piece.vertices.len() {
                for &(cluster, weight) in rows.row(vertex) {
                    carry.extra_skins[slot].influences.push((
                        base + vertex as u32,
                        cluster,
                        weight,
                    ));
                }
            }
        }
        for vertex in 0..piece.vertices.len() {
            for &weight in piece.dq_weight.row(vertex) {
                carry.dq_weights.push((base + vertex as u32, weight));
            }
        }
        if let Some(polygons) = &piece.polygons {
            carry.polygons.push(NodePolygons {
                node: piece.node,
                material: piece.material,
                triangle_first,
                triangle_face: polygons.triangle_face.clone(),
                face_offsets: polygons.face_offsets.clone(),
                corners: polygons
                    .corners
                    .iter()
                    .map(|&corner| corner + base)
                    .collect(),
                source_face: polygons.source_face.clone(),
                face_smoothing: polygons.face_smoothing.clone(),
                face_hole: polygons.face_hole.clone(),
                face_group: polygons.face_group.clone(),
                edges: polygons
                    .edges
                    .iter()
                    .map(|edge| [edge[0] + base, edge[1] + base])
                    .collect(),
                source_edge: polygons.source_edge.clone(),
                edge_smoothing: polygons.edge_smoothing.clone(),
                edge_crease: polygons.edge_crease.clone(),
                edge_visibility: polygons.edge_visibility.clone(),
            });
        }

        for (channel, destination) in uv_channels.iter_mut().enumerate() {
            match piece.uv_channels.get(channel) {
                Some(source_uvs) => destination.extend_from_slice(source_uvs),
                // A submesh built before this channel existed can't happen today,
                // but padding keeps the arrays parallel rather than silently short.
                None => destination.resize(vertices.len(), glam::Vec2::ZERO),
            }
        }

        let triangles = piece.triangle_count();
        if tags.node {
            triangle_node.extend(std::iter::repeat_n(piece.node, triangles));
        }
        if tags.material {
            triangle_material.extend(std::iter::repeat_n(piece.material, triangles));
        }
    }

    let mut model = ModelData {
        name: level_name(&source.name, level),
        vertices,
        indices,
        // Deliberately empty — see the doc comment above.
        faces: Vec::new(),
        triangles: TriangleData {
            to_face: Vec::new(),
            material: triangle_material,
            node: triangle_node,
        },
        nodes: source.nodes.clone(),
        uv_channels,
        uv_set_names: source.uv_set_names.clone(),
        bounds: None,
        stats: ModelStats::default(),
        materials: source.materials.clone(),
        // Every level vertex is its own logical vertex: the rows the pieces
        // carried per vertex become the tables, and the map is the identity.
        corner_to_logical: if any_skin || any_morph {
            (0..vertex_count as u32).collect()
        } else {
            Vec::new()
        },
        skin: any_skin.then(|| SkinData {
            offsets: skin_offsets,
            bones: skin_bones,
            weights: skin_weights,
            influence_cluster: skin_clusters,
            clusters: source
                .skin
                .as_ref()
                .map(|skin| skin.clusters.clone())
                .unwrap_or_default(),
            deformers: source
                .skin
                .as_ref()
                .map(|skin| skin.deformers.clone())
                .unwrap_or_default(),
        }),
        morph: any_morph.then(|| MorphData {
            channels: source
                .morph
                .as_ref()
                .map(|morph| morph.channels.clone())
                .unwrap_or_default(),
            shapes: source
                .morph
                .as_ref()
                .map(|morph| morph.shapes.clone())
                .unwrap_or_default(),
            offsets: morph_offsets,
            shape: morph_shape,
            position: morph_position,
            normal: morph_normal,
        }),
        // The clips address nodes and channels, both carried unchanged.
        animations: source.animations.clone(),
        frame_rate: source.frame_rate,
    };

    // Normals first: tangents are orthonormalized against them, so rebuilding
    // tangents from stale normals would bake the staleness into both.
    if rebuild.normals {
        model.generate_normals();
    }
    // Tangents are derived from positions, UVs and normals, so any geometry
    // change invalidates them. Regenerating unconditionally would be wasted work
    // on a reorder-only stack, and would also overwrite the source file's
    // authored tangents with synthesized ones for no reason.
    if rebuild.tangents {
        model.generate_tangents();
    }

    // The stats first: `validate_deform` reads the logical count off them.
    model.stats = measured_stats(&model, source, &carry);
    if let Err(error) = model.validate_deform() {
        // Never publish a table the funnel guard rejects; the level draws as
        // static geometry and the user hears why.
        warnings.push(&format!(
            "The processed mesh's skin / blend-shape data did not reconcile ({error}); it was \
             dropped from this level and its export."
        ));
        model.corner_to_logical = Vec::new();
        model.skin = None;
        model.morph = None;
        model.animations = Vec::new();
        model.stats = measured_stats(&model, source, &carry);
    }
    // A deforming level rests in the file's default pose like the source does;
    // its bounds are measured through the same deformation.
    if model.needs_deform() {
        let ctx = AnimContext::new(&model);
        model.bounds = anim::rest_bounds(&model, &ctx);
    } else {
        model.recompute_bounds();
    }
    (model, carry)
}
