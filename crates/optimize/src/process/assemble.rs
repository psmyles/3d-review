//! Rebuilding one `ModelData` per level out of the processed submeshes.
//!
//! A level that deforms gets its own `SkinData` / `MorphData` over its own
//! vertices, the source's clips cloned, and bounds measured at rest — the
//! processed mesh has to animate exactly as the source did.
//!
//! ## Two layouts
//!
//! Normally the output is a pure triangle mesh carrying **no** face topology:
//! `faces` and `triangles.to_face` are both left empty. That is not a gap — the
//! original polygon table describes contiguous corner runs in the *source*
//! vertex array, a layout welding and simplification necessarily destroy, and
//! every consumer in `render` already falls back to per-triangle behaviour when
//! the table is absent (the wireframe draws triangle edges, face normals get one
//! slot per triangle, UV islands fall back to the solid fill). Synthesizing a
//! plausible-looking table instead would draw a wireframe that is simply wrong.
//!
//! The exception is a level some operation **rebuilt** the faces of — today only
//! [`crate::remesh`], whose whole output is quads. Those faces describe the
//! level's own geometry, so they can be published; but `TriangleData::to_face`
//! is all-or-nothing, so publishing them means putting the *whole level* into the
//! corner-run layout (one vertex per face corner, contiguous, in face order).
//! That is what [`crate::remesh::layout::corner_run`] does per piece, and what
//! `corner_run` below switches on. In that mode the level's own vertex buffer is
//! corner-split like an import's — which is why `carry.control_point` records
//! each corner's indexed vertex, so the export still writes one control point per
//! real vertex and the stats still report the indexed count (invariant 5).

use review_model::{
    AnimContext, ModelData, ModelStats, MorphData, SkinData, TopologyFace, TriangleData, Vertex,
    anim,
};

use crate::Warnings;
use crate::remesh::layout::{self, CornerRun};
use crate::submesh::{NO_FACE, Submesh, TagPresence};

use super::*;

pub(crate) fn assemble(
    submeshes: &[&Submesh],
    source: &ModelData,
    tags: TagPresence,
    level: usize,
    warnings: &mut Warnings,
) -> (ModelData, LevelCarry) {
    let _z = crate::prof::zone!("Assemble Model");

    // One piece with rebuilt faces puts the whole level in the corner-run
    // layout: `TriangleData::to_face` cannot describe some pieces and not
    // others, so the alternative would be dropping the rebuilt faces entirely.
    let corner_run = submeshes
        .iter()
        .any(|piece| piece.polygons.as_ref().is_some_and(|carry| carry.rebuilt));

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
    let mut faces: Vec<TopologyFace> = Vec::new();
    let mut to_face: Vec<u32> = Vec::new();
    let mut carry = LevelCarry {
        polygons: Vec::new(),
        color_channels: vec![Vec::with_capacity(total_vertices); color_channel_count],
        vertex_crease: Vec::with_capacity(if any_crease { total_vertices } else { 0 }),
        extra_skins: Vec::new(),
        dq_weights: Vec::new(),
        source_corner: Vec::with_capacity(total_vertices),
        control_point: Vec::new(),
    };
    // The deform tables are per **logical** vertex, which outside the corner-run
    // layout is the level vertex and inside it is the indexed vertex a run of
    // corners was expanded from. `indexed_base` numbers those the same way the
    // corners' `control_point` entries do, so the two agree by construction.
    let mut indexed_base = 0u32;
    // Built only when the source had the table: a level of an unskinned model
    // stays `None`.
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
        // In corner-run mode the piece is expanded first and everything below
        // reads the expansion; outside it, `run` is `None` and the piece's own
        // arrays are used exactly as before.
        let run: Option<CornerRun> = corner_run.then(|| layout::corner_run(piece));
        let piece_vertices: &[Vertex] = run.as_ref().map_or(&piece.vertices, |run| &run.vertices);
        let piece_indices: &[u32] = run.as_ref().map_or(&piece.indices, |run| &run.indices);

        let base = vertices.len() as u32;
        let face_base = faces.len() as u32;
        let triangle_first = (indices.len() / 3) as u32;
        vertices.extend_from_slice(piece_vertices);
        indices.extend(piece_indices.iter().map(|&index| index + base));

        if let Some(run) = &run {
            carry.control_point.extend(
                run.control_point
                    .iter()
                    .map(|&vertex| vertex + indexed_base),
            );
            faces.extend(run.faces.iter().map(|face| TopologyFace {
                first_index: face.first_index + base,
                index_count: face.index_count,
            }));
            to_face.extend(run.to_face.iter().map(|&face| face + face_base));
        }

        let source_corner: &[u32] = run
            .as_ref()
            .map_or(&piece.source_corner, |run| &run.source_corner);
        if source_corner.len() == piece_vertices.len() {
            carry.source_corner.extend_from_slice(source_corner);
        } else {
            carry.source_corner.resize(vertices.len(), u32::MAX);
        }

        for (channel, destination) in carry.color_channels.iter_mut().enumerate() {
            let colors = run.as_ref().map_or_else(
                || piece.color_channels.get(channel),
                |run| run.color_channels.get(channel),
            );
            match colors {
                Some(colors) if colors.len() == piece_vertices.len() => {
                    destination.extend_from_slice(colors)
                }
                _ => destination.resize(vertices.len(), glam::Vec4::ONE),
            }
        }
        if any_crease {
            let creases: &[f32] = run
                .as_ref()
                .map_or(&piece.vertex_crease, |run| &run.vertex_crease);
            if creases.len() == piece_vertices.len() {
                carry.vertex_crease.extend_from_slice(creases);
            } else {
                carry.vertex_crease.resize(vertices.len(), 0.0);
            }
        }

        // The deform rows stay per indexed vertex whichever layout the level is
        // in — `corner_to_logical` is what projects them onto the corners.
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
        // The export's extra-skin and DQ layers, on the other hand, address
        // *level* vertices, so in corner-run mode one indexed vertex's row has to
        // be written to each of its corners. Both are rare (a second skin
        // deformer, a dual-quaternion blend weight), so the corner table is built
        // only when one is present rather than on every piece.
        let needs_corners =
            piece.extra_skins.iter().any(|layer| !layer.is_empty()) || !piece.dq_weight.is_empty();
        let corner_table: Vec<Vec<u32>> = match (&run, needs_corners) {
            (Some(run), true) => {
                let mut table = vec![Vec::new(); piece.vertices.len()];
                for (slot, &point) in run.control_point.iter().enumerate() {
                    if let Some(corners) = table.get_mut(point as usize) {
                        corners.push(base + slot as u32);
                    }
                }
                table
            }
            _ => Vec::new(),
        };
        let corners_of = |vertex: usize| -> Vec<u32> {
            match corner_table.get(vertex) {
                Some(corners) => corners.clone(),
                None => vec![base + vertex as u32],
            }
        };
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
                    for corner in corners_of(vertex) {
                        carry.extra_skins[slot]
                            .influences
                            .push((corner, cluster, weight));
                    }
                }
            }
        }
        for vertex in 0..piece.vertices.len() {
            for &weight in piece.dq_weight.row(vertex) {
                for corner in corners_of(vertex) {
                    carry.dq_weights.push((corner, weight));
                }
            }
        }

        match (&run, &piece.polygons) {
            // Corner-run mode: every triangle belongs to a face, so every piece
            // gets a carry entry — including one that had none, whose triangles
            // became three-corner polygons of their own.
            (Some(run), carried) => {
                let source_face: Vec<u32> = (0..run.faces.len())
                    .map(|face| {
                        carried
                            .as_ref()
                            .and_then(|carry| carry.source_face.get(face).copied())
                            .unwrap_or(NO_FACE)
                    })
                    .collect();
                let layer = |values: &[bool]| -> Vec<bool> {
                    if values.is_empty() {
                        Vec::new()
                    } else {
                        (0..run.faces.len())
                            .map(|face| values.get(face).copied().unwrap_or(false))
                            .collect()
                    }
                };
                let mut face_offsets = Vec::with_capacity(run.faces.len() + 1);
                let mut corners = Vec::with_capacity(run.vertices.len());
                face_offsets.push(0);
                for face in &run.faces {
                    corners.extend(
                        (0..face.index_count).map(|corner| base + face.first_index + corner),
                    );
                    face_offsets.push(corners.len() as u32);
                }
                carry.polygons.push(NodePolygons {
                    node: piece.node,
                    material: piece.material,
                    triangle_first,
                    triangle_face: run.to_face.clone(),
                    face_offsets,
                    corners,
                    source_face,
                    face_smoothing: carried
                        .as_ref()
                        .map(|carry| layer(&carry.face_smoothing))
                        .unwrap_or_default(),
                    face_hole: carried
                        .as_ref()
                        .map(|carry| layer(&carry.face_hole))
                        .unwrap_or_default(),
                    face_group: carried
                        .as_ref()
                        .filter(|carry| !carry.face_group.is_empty())
                        .map(|carry| {
                            (0..run.faces.len())
                                .map(|face| carry.face_group.get(face).copied().unwrap_or(0))
                                .collect()
                        })
                        .unwrap_or_default(),
                    // An edge is authored between two *indexed* vertices; in the
                    // run each of them appears once per incident face, so the
                    // first corner of each stands for it.
                    edges: carried
                        .as_ref()
                        .map(|carry| {
                            carry
                                .edges
                                .iter()
                                .map(|edge| {
                                    [
                                        base + run.first_slot(edge[0]),
                                        base + run.first_slot(edge[1]),
                                    ]
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    source_edge: carried
                        .as_ref()
                        .map(|carry| carry.source_edge.clone())
                        .unwrap_or_default(),
                    edge_smoothing: carried
                        .as_ref()
                        .map(|carry| carry.edge_smoothing.clone())
                        .unwrap_or_default(),
                    edge_crease: carried
                        .as_ref()
                        .map(|carry| carry.edge_crease.clone())
                        .unwrap_or_default(),
                    edge_visibility: carried
                        .as_ref()
                        .map(|carry| carry.edge_visibility.clone())
                        .unwrap_or_default(),
                });
            }
            (None, Some(polygons)) => {
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
            (None, None) => {}
        }

        for (channel, destination) in uv_channels.iter_mut().enumerate() {
            let uvs = run.as_ref().map_or_else(
                || piece.uv_channels.get(channel),
                |run| run.uv_channels.get(channel),
            );
            match uvs {
                Some(source_uvs) if source_uvs.len() == piece_vertices.len() => {
                    destination.extend_from_slice(source_uvs)
                }
                // A submesh built before this channel existed can't happen today,
                // but padding keeps the arrays parallel rather than silently short.
                _ => destination.resize(vertices.len(), glam::Vec2::ZERO),
            }
        }

        let triangles = piece_indices.len() / 3;
        if tags.node {
            triangle_node.extend(std::iter::repeat_n(piece.node, triangles));
        }
        if tags.material {
            triangle_material.extend(std::iter::repeat_n(piece.material, triangles));
        }
        indexed_base += piece.vertices.len() as u32;
    }

    let vertex_count = vertices.len();
    let mut model = ModelData {
        name: level_name(&source.name, level),
        vertices,
        indices,
        // Empty outside the corner-run layout — see the module docs.
        faces,
        triangles: TriangleData {
            to_face,
            material: triangle_material,
            node: triangle_node,
        },
        nodes: source.nodes.clone(),
        uv_channels,
        uv_set_names: source.uv_set_names.clone(),
        bounds: None,
        stats: ModelStats::default(),
        materials: source.materials.clone(),
        // Outside the corner-run layout every level vertex is its own logical
        // vertex and the map is the identity; inside it, a corner's logical
        // vertex is the indexed one it was expanded from — which is exactly what
        // `control_point` already records.
        corner_to_logical: if !(any_skin || any_morph) {
            Vec::new()
        } else if corner_run {
            carry.control_point.clone()
        } else {
            (0..vertex_count as u32).collect()
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

    // The stats first: `validate_deform` reads the logical count off them.
    model.stats = measured_stats(&model, source, &carry, indexed_base as usize);
    debug_assert!(
        model
            .triangles
            .validate(
                model.indices.len() / 3,
                model.faces.len(),
                model.materials.len(),
                model.nodes.len(),
            )
            .is_ok(),
        "the assembled level's per-triangle tags drifted: {:?}",
        model.triangles.validate(
            model.indices.len() / 3,
            model.faces.len(),
            model.materials.len(),
            model.nodes.len(),
        )
    );
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
        model.stats = measured_stats(&model, source, &carry, indexed_base as usize);
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
