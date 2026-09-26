//! Splitting a `ModelData` into submeshes.

use std::collections::HashMap;

use glam::{Vec2, Vec4};
use review_model::{ModelData, SourceExtras};

use super::*;

/// Split `model` into per-(node, material) submeshes, in first-seen triangle
/// order so the result — and therefore the reassembled mesh — is deterministic.
///
/// `extras` supplies what rides along beyond the geometry: the polygon carry
/// (also needing the model's own face table and triangle→face map), extra
/// color sets and vertex creases. Without it the pieces carry geometry only.
///
/// Returns the submeshes plus which per-triangle tags the source actually
/// carried, so reassembly can leave a tag array empty rather than fabricating
/// one the source never had.
pub fn partition(model: &ModelData, extras: Option<&SourceExtras>) -> (Vec<Submesh>, TagPresence) {
    let _z = crate::prof::zone!("Partition Submeshes");

    let triangle_count = model.indices.len() / 3;
    let tags = TagPresence {
        node: model.triangles.node.len() == triangle_count && triangle_count > 0,
        material: model.triangles.material.len() == triangle_count && triangle_count > 0,
    };
    if triangle_count == 0 || model.vertices.is_empty() {
        return (Vec::new(), tags);
    }

    // Extra UV channels only exist on multi-set models; channel 0 is mirrored
    // into `uv_channels[0]` there, and lives only in `Vertex::uv` otherwise.
    let channel_count = model.uv_channels.len();
    // The polygon table applies only while the model still has the corner-split
    // layout it describes (a processed model has none).
    let faces_known = extras.is_some()
        && model.triangles.to_face.len() == triangle_count
        && !model.faces.is_empty();

    let mut order: Vec<(u32, u32)> = Vec::new();
    let mut slots: HashMap<(u32, u32), usize> = HashMap::new();
    let mut triangles_per_slot: Vec<Vec<usize>> = Vec::new();

    for triangle in 0..triangle_count {
        let node = if tags.node {
            model.triangles.node[triangle]
        } else {
            0
        };
        let material = if tags.material {
            model.triangles.material[triangle]
        } else {
            NO_MATERIAL
        };
        let key = (node, material);
        let slot = *slots.entry(key).or_insert_with(|| {
            order.push(key);
            triangles_per_slot.push(Vec::new());
            order.len() - 1
        });
        triangles_per_slot[slot].push(triangle);
    }

    // One scratch map reused across submeshes, cleared through a touched list so
    // the cost stays proportional to the vertices a submesh actually uses rather
    // than to the whole model per submesh.
    let mut local_of_global = vec![u32::MAX; model.vertices.len()];
    let mut touched: Vec<u32> = Vec::new();

    let mut submeshes = Vec::with_capacity(order.len());
    for (slot, &(node, material)) in order.iter().enumerate() {
        let source_triangles = &triangles_per_slot[slot];
        let part = extras
            .filter(|_| tags.node)
            .and_then(|extras| extras.mesh_of_node(node));
        let mut vertices = Vec::new();
        let mut uv_channels = vec![Vec::new(); channel_count];
        let mut color_channels: Vec<Vec<Vec4>> = part
            .map(|part| {
                part.color_sets
                    .iter()
                    .filter(|set| !set.values.is_empty())
                    .map(|_| Vec::new())
                    .collect()
            })
            .unwrap_or_default();
        let color_sets: Vec<&[Vec4]> = part
            .map(|part| {
                part.color_sets
                    .iter()
                    .filter(|set| !set.values.is_empty())
                    .map(|set| set.values.as_slice())
                    .collect()
            })
            .unwrap_or_default();
        let has_crease = part.is_some_and(|part| !part.vertex_crease.is_empty());
        let mut vertex_crease = Vec::new();
        let mut source_corner: Vec<u32> = Vec::new();
        let mut indices = Vec::with_capacity(source_triangles.len() * 3);
        let mut kept_triangles: Vec<usize> = Vec::with_capacity(source_triangles.len());
        let corner_map = (model.corner_to_logical.len() == model.vertices.len())
            .then_some(model.corner_to_logical.as_slice());
        let skin = model.skin.as_ref().filter(|_| corner_map.is_some());
        let morph = model.morph.as_ref().filter(|_| corner_map.is_some());
        let extra_layers: Vec<&review_model::extras::SkinLayerExtras> = part
            .filter(|_| corner_map.is_some())
            .map(|part| part.extra_skins.iter().collect())
            .unwrap_or_default();
        let dq_of_logical: HashMap<u32, f32> = part
            .map(|part| part.dq_weights.iter().copied().collect())
            .unwrap_or_default();
        let mut skin_rows = VertexRows::default();
        let mut extra_rows: Vec<VertexRows<(u32, f32)>> =
            vec![VertexRows::default(); extra_layers.len()];
        let mut dq_rows = VertexRows::default();
        let mut morph_rows = VertexRows::default();
        let mut row_scratch: Vec<(u32, f32)> = Vec::new();

        for &triangle in source_triangles {
            let corners = &model.indices[triangle * 3..triangle * 3 + 3];
            // An out-of-range index means a malformed model. The whole triangle
            // goes: dropping only the offending corner would leave an index
            // buffer that is no longer a multiple of three, shifting every later
            // triangle by one corner into plausible-looking garbage.
            if corners
                .iter()
                .any(|&global| global as usize >= model.vertices.len())
            {
                continue;
            }
            kept_triangles.push(triangle);
            for &global in corners {
                // In range, so both lookups hit: `local_of_global` is sized to
                // the model's vertex array.
                let entry = &mut local_of_global[global as usize];
                if *entry == u32::MAX {
                    *entry = vertices.len() as u32;
                    touched.push(global);
                    vertices.push(model.vertices[global as usize]);
                    source_corner.push(global);
                    for (channel, destination) in uv_channels.iter_mut().enumerate() {
                        destination.push(
                            model
                                .uv_channels
                                .get(channel)
                                .and_then(|uvs| uvs.get(global as usize))
                                .copied()
                                .unwrap_or(Vec2::ZERO),
                        );
                    }
                    if let Some(part) = part {
                        let local_corner =
                            (global as usize).wrapping_sub(part.corner_first as usize);
                        for (channel, destination) in color_channels.iter_mut().enumerate() {
                            destination.push(
                                color_sets[channel]
                                    .get(local_corner)
                                    .copied()
                                    .unwrap_or(Vec4::ONE),
                            );
                        }
                        if has_crease {
                            let logical =
                                model
                                    .corner_to_logical
                                    .get(global as usize)
                                    .map(|&logical| {
                                        (logical as usize).wrapping_sub(part.logical_first as usize)
                                    });
                            vertex_crease.push(
                                logical
                                    .and_then(|logical| part.vertex_crease.get(logical))
                                    .copied()
                                    .unwrap_or(0.0),
                            );
                        }
                    }
                    if let Some(map) = corner_map {
                        let logical = map[global as usize] as usize;
                        if let Some(skin) = skin {
                            let range = skin.influence_range(logical);
                            row_scratch.clear();
                            row_scratch.extend(
                                skin.influence_cluster[range.clone()]
                                    .iter()
                                    .zip(&skin.weights[range])
                                    .map(|(&cluster, &weight)| (cluster, weight)),
                            );
                            skin_rows.push_row(&row_scratch);
                        }
                        for (layer, rows) in extra_layers.iter().zip(&mut extra_rows) {
                            let local = logical
                                .wrapping_sub(part.map_or(0, |part| part.logical_first as usize));
                            let (start, end) =
                                match (layer.offsets.get(local), layer.offsets.get(local + 1)) {
                                    (Some(&start), Some(&end)) if end >= start => {
                                        (start as usize, end as usize)
                                    }
                                    _ => (0, 0),
                                };
                            rows.push_row(layer.influences.get(start..end).unwrap_or(&[]));
                        }
                        if !dq_of_logical.is_empty() {
                            match dq_of_logical.get(&(logical as u32)) {
                                Some(&weight) => dq_rows.push_row(&[weight]),
                                None => dq_rows.push_row(&[]),
                            }
                        }
                        if let Some(morph) = morph {
                            let (start, end) = match (
                                morph.offsets.get(logical),
                                morph.offsets.get(logical + 1),
                            ) {
                                (Some(&start), Some(&end)) if end >= start => {
                                    (start as usize, end as usize)
                                }
                                _ => (0, 0),
                            };
                            let entries: Vec<MorphEntry> = (start..end)
                                .map(|index| MorphEntry {
                                    shape: morph.shape[index],
                                    position: morph.position[index],
                                    normal: morph.normal[index],
                                })
                                .collect();
                            morph_rows.push_row(&entries);
                        }
                    }
                }
                indices.push(*entry);
            }
        }

        let polygons = if faces_known {
            build_polygon_carry(model, part, &kept_triangles, &local_of_global)
        } else {
            None
        };

        for global in touched.drain(..) {
            local_of_global[global as usize] = u32::MAX;
        }

        submeshes.push(Submesh {
            node,
            material,
            vertices,
            uv_channels,
            color_channels,
            vertex_crease,
            indices,
            polygons,
            skin: skin_rows,
            extra_skins: extra_rows,
            dq_weight: dq_rows,
            morph: morph_rows,
            source_corner,
            normals_stale: false,
        });
    }

    (submeshes, tags)
}
