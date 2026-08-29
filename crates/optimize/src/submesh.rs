//! Partitioning a [`ModelData`] into independently-optimizable submeshes, and
//! the buffer surgery every operation shares.
//!
//! ## Why (node, material) is the unit
//!
//! Import produces one flat triangle soup for the whole scene, with each
//! triangle tagged by its owning node and material slot. meshoptimizer works on
//! a single indexed mesh and — crucially — its simplifier returns a *new* index
//! buffer with no correspondence back to the input triangles. There is
//! therefore no way to carry a per-triangle material tag through a simplify
//! pass. Splitting on `(node, material)` gives every output triangle an
//! unambiguous tag by construction, and has two further benefits: geometry
//! never collapses across an object boundary (two props sitting next to each
//! other stay two props), and it matches the renderer's own draw grouping.
//!
//! The cost is that a collapse can never merge two materials' geometry along
//! their shared seam. That seam is already a hard split in the source data —
//! import gives each material's corners their own vertices — so little is lost.

use std::collections::HashMap;

use glam::Vec2;
use review_model::{ModelData, Vertex};

use crate::meshopt::POSITION_COMPONENTS;

/// The no-material sentinel `ModelData::triangles.material` uses.
pub const NO_MATERIAL: u32 = u32::MAX;

/// One independently-optimizable piece of the scene: all triangles sharing an
/// owning node and material slot, with their vertices compacted into local
/// arrays indexed from zero.
#[derive(Debug, Clone)]
pub struct Submesh {
    /// Owning node, indexing `ModelData::nodes`. `0` when the source model
    /// carried no per-triangle node information.
    pub node: u32,
    /// Material slot, or [`NO_MATERIAL`].
    pub material: u32,
    pub vertices: Vec<Vertex>,
    /// Extra UV sets, one entry per channel, each parallel to `vertices`. Empty
    /// unless the source model carries more than one UV set (matching
    /// [`ModelData::uv_channels`]'s own convention).
    pub uv_channels: Vec<Vec<Vec2>>,
    pub indices: Vec<u32>,
}

impl Submesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() || self.vertices.is_empty()
    }

    /// A tightly packed `float3` position stream — the form every
    /// position-taking meshoptimizer entry point wants. Rebuilt per call rather
    /// than cached: an operation that rewrites the vertex array would otherwise
    /// have to remember to invalidate it, and a stale position stream is a
    /// silent wrong-geometry bug rather than a loud one.
    pub fn positions(&self) -> Vec<f32> {
        let mut positions = Vec::with_capacity(self.vertices.len() * POSITION_COMPONENTS);
        for vertex in &self.vertices {
            positions.extend_from_slice(&[vertex.position.x, vertex.position.y, vertex.position.z]);
        }
        positions
    }

    /// Apply an old→new vertex remap (as produced by meshoptimizer) to every
    /// parallel vertex array. Entries equal to [`u32::MAX`] mark a vertex the
    /// remap dropped, which meshoptimizer's own `remapVertexBuffer` also skips.
    ///
    /// The index buffer is rewritten by the caller — remapping vertices and
    /// remapping indices are separate steps in the C API, and keeping them
    /// separate here lets [`Self::compact_unreferenced`] reuse this.
    pub fn apply_vertex_remap(&mut self, remap: &[u32], unique: usize) {
        debug_assert_eq!(remap.len(), self.vertices.len());

        let mut vertices = vec![Vertex::default(); unique];
        for (old, &new) in remap.iter().enumerate() {
            if new == u32::MAX {
                continue;
            }
            if let (Some(source), Some(destination)) =
                (self.vertices.get(old), vertices.get_mut(new as usize))
            {
                *destination = *source;
            }
        }
        self.vertices = vertices;

        for channel in &mut self.uv_channels {
            let mut remapped = vec![Vec2::ZERO; unique];
            for (old, &new) in remap.iter().enumerate() {
                if new == u32::MAX {
                    continue;
                }
                if let (Some(source), Some(destination)) =
                    (channel.get(old), remapped.get_mut(new as usize))
                {
                    *destination = *source;
                }
            }
            *channel = remapped;
        }
    }

    /// Drop vertices the index buffer no longer references, preserving the
    /// order of those that remain.
    ///
    /// Simplification leaves the vertex array untouched — its output index
    /// buffer just stops mentioning most of it — so without this the processed
    /// mesh would report the *source* vertex count while drawing a fraction of
    /// the triangles. Running it at the end of every level is what makes the
    /// Verts figure in the stats overlay a real measurement (invariant 5)
    /// whether or not the user added a Vertex Fetch operation.
    pub fn compact_unreferenced(&mut self) {
        let mut remap = vec![u32::MAX; self.vertices.len()];
        let mut next = 0u32;
        for &index in &self.indices {
            let slot = index as usize;
            if let Some(entry) = remap.get_mut(slot)
                && *entry == u32::MAX
            {
                *entry = next;
                next += 1;
            }
        }
        if next as usize == self.vertices.len() {
            return;
        }

        for index in &mut self.indices {
            if let Some(&new) = remap.get(*index as usize)
                && new != u32::MAX
            {
                *index = new;
            }
        }
        self.apply_vertex_remap(&remap.clone(), next as usize);
    }
}

/// Split `model` into per-(node, material) submeshes, in first-seen triangle
/// order so the result — and therefore the reassembled mesh — is deterministic.
///
/// Returns the submeshes plus which per-triangle tags the source actually
/// carried, so reassembly can leave a tag array empty rather than fabricating
/// one the source never had.
pub fn partition(model: &ModelData) -> (Vec<Submesh>, TagPresence) {
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
        let mut vertices = Vec::new();
        let mut uv_channels = vec![Vec::new(); channel_count];
        let mut indices = Vec::with_capacity(source_triangles.len() * 3);

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
            for &global in corners {
                // In range, so both lookups hit: `local_of_global` is sized to
                // the model's vertex array.
                let entry = &mut local_of_global[global as usize];
                if *entry == u32::MAX {
                    *entry = vertices.len() as u32;
                    touched.push(global);
                    vertices.push(model.vertices[global as usize]);
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
                }
                indices.push(*entry);
            }
        }

        for global in touched.drain(..) {
            local_of_global[global as usize] = u32::MAX;
        }

        submeshes.push(Submesh {
            node,
            material,
            vertices,
            uv_channels,
            indices,
        });
    }

    (submeshes, tags)
}

/// Which per-triangle tag arrays the source model carried. Reassembly mirrors
/// this rather than always emitting both: a model with no node hierarchy should
/// come out of processing still having none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagPresence {
    pub node: bool,
    pub material: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use review_model::demo_cube_model;

    #[test]
    fn cube_partitions_into_one_submesh_with_all_corners() {
        let model = demo_cube_model();
        let (submeshes, _tags) = partition(&model);

        assert_eq!(
            submeshes.len(),
            1,
            "the demo cube is one node, one material"
        );
        let cube = &submeshes[0];
        assert_eq!(cube.triangle_count(), model.indices.len() / 3);
        assert_eq!(
            cube.vertices.len(),
            model.vertices.len(),
            "every source vertex is referenced, so none is dropped"
        );
    }

    #[test]
    fn positions_stream_is_tightly_packed() {
        let model = demo_cube_model();
        let (submeshes, _) = partition(&model);
        let positions = submeshes[0].positions();

        assert_eq!(positions.len(), submeshes[0].vertices.len() * 3);
        assert_eq!(positions[0], submeshes[0].vertices[0].position.x);
        assert_eq!(positions[2], submeshes[0].vertices[0].position.z);
    }

    #[test]
    fn compaction_drops_vertices_nothing_references() {
        let model = demo_cube_model();
        let (mut submeshes, _) = partition(&model);
        let cube = &mut submeshes[0];

        // Keep only the first triangle; the other 22 vertices become orphans.
        cube.indices.truncate(3);
        let kept: Vec<_> = cube
            .indices
            .iter()
            .map(|&i| cube.vertices[i as usize])
            .collect();
        cube.compact_unreferenced();

        assert_eq!(cube.vertices.len(), 3);
        assert_eq!(cube.indices, vec![0, 1, 2]);
        assert_eq!(cube.vertices, kept, "surviving vertices keep their data");
    }
}
