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
//!
//! ## What rides along
//!
//! Beyond the vertices and the triangle list, a submesh carries what the source
//! authored *about* them and the export has to write back: extra vertex-color
//! sets and vertex creases (per vertex, remapped with the vertices), and the
//! polygon topology — the faces the triangles were cut from, the edge list and
//! the per-face / per-edge layers — as a [`PolygonCarry`]. The carry follows
//! three rules, one per class of operation:
//!
//! * a **vertex remap** (weld, vertex fetch, compaction) renumbers its corners
//!   and edge endpoints, dropping a face that collapses below three distinct
//!   vertices;
//! * an **index-buffer rewrite that keeps triangles whole** (filter, prune, the
//!   cache and overdraw reorders) is reconciled by content — every triangle is
//!   found again in the new buffer, a face survives only when all its triangles
//!   did ([`Submesh::reconcile_triangles`]);
//! * a **simplify** produces triangles with no correspondence to the input, so
//!   the carry is cleared ([`Submesh::clear_polygons`]) and that piece goes out
//!   as triangles.

use std::collections::HashMap;

use glam::{Vec2, Vec3, Vec4};
use review_model::{ModelData, SourceExtras, Vertex};

use crate::meshopt::POSITION_COMPONENTS;

/// A compressed-sparse-row table of per-vertex entries: vertex `v`'s entries
/// are `data[offsets[v]..offsets[v + 1]]`. Empty (no offsets at all) when the
/// submesh carries none of this kind, so an unskinned model pays nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct VertexRows<T> {
    /// `vertex_count + 1` starts, or empty.
    pub offsets: Vec<u32>,
    pub data: Vec<T>,
}

// Hand-written so the empty table exists for every `T`, not only defaultable ones.
impl<T> Default for VertexRows<T> {
    fn default() -> Self {
        Self {
            offsets: Vec::new(),
            data: Vec::new(),
        }
    }
}

impl<T: Clone> VertexRows<T> {
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn row(&self, vertex: usize) -> &[T] {
        match (self.offsets.get(vertex), self.offsets.get(vertex + 1)) {
            (Some(&start), Some(&end)) if end >= start => &self.data[start as usize..end as usize],
            _ => &[],
        }
    }

    fn push_row(&mut self, entries: &[T]) {
        if self.offsets.is_empty() {
            self.offsets.push(0);
        }
        self.data.extend_from_slice(entries);
        self.offsets.push(self.data.len() as u32);
    }

    /// The rows of `representative[slot]` for every new slot, in order.
    fn gather(&self, representative: &[u32]) -> Self {
        let mut out = Self::default();
        if self.is_empty() {
            return out;
        }
        out.offsets.push(0);
        for &old in representative {
            let row = if old == u32::MAX {
                &[]
            } else {
                self.row(old as usize)
            };
            out.data.extend_from_slice(row);
            out.offsets.push(out.data.len() as u32);
        }
        out
    }
}

/// One blend-shape offset of a vertex: the shape and its world-oriented
/// position / normal deltas, as the model carries them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphEntry {
    pub shape: u32,
    pub position: Vec3,
    pub normal: Vec3,
}

/// The no-material sentinel `ModelData::triangles.material` uses.
pub const NO_MATERIAL: u32 = u32::MAX;

/// A triangle that belongs to no carried face.
pub const NO_FACE: u32 = u32::MAX;

/// The source polygon topology of one submesh, in its local vertex numbering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PolygonCarry {
    /// `face_count + 1` starts into `corners`.
    pub face_offsets: Vec<u32>,
    /// Local vertex indices, one per polygon corner.
    pub corners: Vec<u32>,
    /// Per face, the index of the source face (`ModelData::faces`) it came from.
    pub source_face: Vec<u32>,
    /// Per triangle of the submesh's index buffer, the local face it belongs to
    /// or [`NO_FACE`].
    pub triangle_face: Vec<u32>,
    /// Per face, present only when the source carried the layer.
    pub face_smoothing: Vec<bool>,
    pub face_hole: Vec<bool>,
    /// The authored polygon-group *id* of each face (the reader reports group
    /// table indices; those are resolved to ids here, which is what the FBX
    /// `PolygonGroup` layer holds).
    pub face_group: Vec<u32>,
    /// Edges as local vertex pairs.
    pub edges: Vec<[u32; 2]>,
    /// Per edge, the index of the source edge (`MeshExtras::edges` of the part).
    pub source_edge: Vec<u32>,
    /// Per edge, present only when the source carried the layer.
    pub edge_smoothing: Vec<bool>,
    pub edge_crease: Vec<f32>,
    pub edge_visibility: Vec<bool>,
}

impl PolygonCarry {
    pub fn face_count(&self) -> usize {
        self.face_offsets.len().saturating_sub(1)
    }

    /// The corners of face `face`.
    pub fn face(&self, face: usize) -> &[u32] {
        let first = self.face_offsets[face] as usize;
        let end = self.face_offsets[face + 1] as usize;
        &self.corners[first..end]
    }

    /// Keep only the faces `keep` marks, dropping their layer entries with them
    /// and renumbering `triangle_face` (whose entries the caller has already
    /// rewritten to the *old* face numbering).
    fn retain_faces(&mut self, keep: &[bool]) {
        let mut new_of_old = vec![NO_FACE; keep.len()];
        let mut offsets = vec![0u32];
        let mut corners = Vec::with_capacity(self.corners.len());
        let mut source_face = Vec::new();
        let mut smoothing = Vec::new();
        let mut hole = Vec::new();
        let mut group = Vec::new();
        for (face, &kept) in keep.iter().enumerate() {
            if !kept {
                continue;
            }
            new_of_old[face] = source_face.len() as u32;
            corners.extend_from_slice(self.face(face));
            offsets.push(corners.len() as u32);
            source_face.push(self.source_face[face]);
            if let Some(&value) = self.face_smoothing.get(face) {
                smoothing.push(value);
            }
            if let Some(&value) = self.face_hole.get(face) {
                hole.push(value);
            }
            if let Some(&value) = self.face_group.get(face) {
                group.push(value);
            }
        }
        for entry in &mut self.triangle_face {
            if *entry != NO_FACE {
                *entry = new_of_old.get(*entry as usize).copied().unwrap_or(NO_FACE);
            }
        }
        self.face_offsets = offsets;
        self.corners = corners;
        self.source_face = source_face;
        self.face_smoothing = smoothing;
        self.face_hole = hole;
        self.face_group = group;
    }

    /// Keep only the edges `keep` marks.
    fn retain_edges(&mut self, keep: &[bool]) {
        let mut edges = Vec::new();
        let mut source_edge = Vec::new();
        let mut smoothing = Vec::new();
        let mut crease = Vec::new();
        let mut visibility = Vec::new();
        for (index, &kept) in keep.iter().enumerate() {
            if !kept {
                continue;
            }
            edges.push(self.edges[index]);
            if let Some(&value) = self.source_edge.get(index) {
                source_edge.push(value);
            }
            if let Some(&value) = self.edge_smoothing.get(index) {
                smoothing.push(value);
            }
            if let Some(&value) = self.edge_crease.get(index) {
                crease.push(value);
            }
            if let Some(&value) = self.edge_visibility.get(index) {
                visibility.push(value);
            }
        }
        self.edges = edges;
        self.source_edge = source_edge;
        self.edge_smoothing = smoothing;
        self.edge_crease = crease;
        self.edge_visibility = visibility;
    }
}

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
    /// Vertex-color sets beyond the first (which lives in `Vertex::vertex_color`),
    /// each parallel to `vertices`.
    pub color_channels: Vec<Vec<Vec4>>,
    /// Per-vertex subdivision creases, parallel to `vertices`; empty when the
    /// source carried none.
    pub vertex_crease: Vec<f32>,
    pub indices: Vec<u32>,
    /// The source polygon topology, while an operation has not rebuilt the
    /// triangles from scratch.
    pub polygons: Option<PolygonCarry>,
    /// The primary skin's influences per vertex: `(cluster, weight)`, the
    /// cluster indexing the source `SkinData::clusters`.
    pub skin: VertexRows<(u32, f32)>,
    /// Further skin layers of the part, each `(cluster, weight)` with the
    /// cluster indexing that layer's own cluster list.
    pub extra_skins: Vec<VertexRows<(u32, f32)>>,
    /// The primary skin's dual-quaternion blend weight, at most one per vertex.
    pub dq_weight: VertexRows<f32>,
    /// Blend-shape offsets per vertex.
    pub morph: VertexRows<MorphEntry>,
    /// Per vertex, a source corner (`ModelData::vertices` of the source) it
    /// came from — the representative one after a weld. Empty when unknown.
    pub source_corner: Vec<u32>,
}

impl Submesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() || self.vertices.is_empty()
    }

    /// Whether any deform data rides on the vertices.
    pub fn has_rows(&self) -> bool {
        !self.skin.is_empty()
            || self.extra_skins.iter().any(|layer| !layer.is_empty())
            || !self.dq_weight.is_empty()
            || !self.morph.is_empty()
    }

    /// One id per vertex naming its whole deform row (skin, extra skins,
    /// dual-quaternion weight, blend-shape offsets): vertices with identical
    /// rows share an id. A weld or a filter compares this beside the geometry,
    /// so two vertices that only *deform* differently are never merged or
    /// treated as duplicates — which is what keeps a gathered row exact.
    /// Empty when the submesh carries no rows.
    pub fn row_ids(&self) -> Vec<u32> {
        if !self.has_rows() {
            return Vec::new();
        }
        let mut ids: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut key = Vec::new();
        (0..self.vertices.len())
            .map(|vertex| {
                key.clear();
                for &(cluster, weight) in self.skin.row(vertex) {
                    key.extend_from_slice(&cluster.to_ne_bytes());
                    key.extend_from_slice(&weight.to_bits().to_ne_bytes());
                }
                key.push(0xFF);
                for layer in &self.extra_skins {
                    for &(cluster, weight) in layer.row(vertex) {
                        key.extend_from_slice(&cluster.to_ne_bytes());
                        key.extend_from_slice(&weight.to_bits().to_ne_bytes());
                    }
                    key.push(0xFE);
                }
                for &weight in self.dq_weight.row(vertex) {
                    key.extend_from_slice(&weight.to_bits().to_ne_bytes());
                }
                key.push(0xFD);
                for entry in self.morph.row(vertex) {
                    key.extend_from_slice(&entry.shape.to_ne_bytes());
                    for value in entry
                        .position
                        .to_array()
                        .into_iter()
                        .chain(entry.normal.to_array())
                    {
                        key.extend_from_slice(&value.to_bits().to_ne_bytes());
                    }
                }
                let next = ids.len() as u32;
                *ids.entry(key.clone()).or_insert(next)
            })
            .collect()
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
    ///
    /// Several old vertices may map to one new slot (a weld). The vertex itself
    /// is written last-writer-wins, as meshoptimizer's remap does; the carried
    /// per-vertex data is *gathered* from the first old vertex of each slot
    /// instead, so every slot's extras come from one consistent source vertex.
    /// Polygon corners and edges are renumbered through the same remap.
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

        // The representative old vertex of every new slot: the first one that
        // maps there.
        let mut representative = vec![u32::MAX; unique];
        for (old, &new) in remap.iter().enumerate() {
            if let Some(slot) = representative.get_mut(new as usize)
                && *slot == u32::MAX
            {
                *slot = old as u32;
            }
        }
        for channel in &mut self.uv_channels {
            *channel = gather(channel, &representative, Vec2::ZERO);
        }
        for channel in &mut self.color_channels {
            *channel = gather(channel, &representative, Vec4::ONE);
        }
        if !self.vertex_crease.is_empty() {
            self.vertex_crease = gather(&self.vertex_crease, &representative, 0.0);
        }
        if !self.source_corner.is_empty() {
            self.source_corner = gather(&self.source_corner, &representative, u32::MAX);
        }
        self.skin = self.skin.gather(&representative);
        for layer in &mut self.extra_skins {
            *layer = layer.gather(&representative);
        }
        self.dq_weight = self.dq_weight.gather(&representative);
        self.morph = self.morph.gather(&representative);

        if let Some(polygons) = &mut self.polygons {
            let mapped = |index: u32| remap.get(index as usize).copied().unwrap_or(u32::MAX);
            for corner in &mut polygons.corners {
                *corner = mapped(*corner);
            }
            // A face whose corners fold onto fewer than three distinct vertices
            // is no polygon any more.
            let keep: Vec<bool> = (0..polygons.face_count())
                .map(|face| {
                    let corners = polygons.face(face);
                    let mut distinct: Vec<u32> = corners.to_vec();
                    distinct.sort_unstable();
                    distinct.dedup();
                    !corners.contains(&u32::MAX) && distinct.len() >= 3
                })
                .collect();
            if keep.iter().any(|&kept| !kept) {
                polygons.retain_faces(&keep);
            }
            for edge in &mut polygons.edges {
                *edge = [mapped(edge[0]), mapped(edge[1])];
            }
            let keep_edges: Vec<bool> = polygons
                .edges
                .iter()
                .map(|edge| edge[0] != u32::MAX && edge[1] != u32::MAX && edge[0] != edge[1])
                .collect();
            if keep_edges.iter().any(|&kept| !kept) {
                polygons.retain_edges(&keep_edges);
            }
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

    /// Re-establish the polygon carry after an operation rewrote the index
    /// buffer without renumbering vertices: every new triangle is found again in
    /// `before` by content (the same three corners in the same winding), so a
    /// filter that dropped triangles, a prune, or a cache reorder all resolve.
    /// A face survives only when every one of its triangles did; the rest of
    /// its triangles are kept as face-less triangles.
    ///
    /// A triangle that cannot be found — the operation invented geometry — voids
    /// the carry for the whole piece, which then goes out as triangles.
    pub fn reconcile_triangles(&mut self, before: &[u32]) {
        let Some(polygons) = &mut self.polygons else {
            return;
        };
        let old_count = before.len() / 3;
        if polygons.triangle_face.len() != old_count {
            self.polygons = None;
            return;
        }
        // Old triangle by rotation-normalized content; duplicates queue up so
        // each is claimed once.
        let mut by_content: HashMap<[u32; 3], Vec<u32>> = HashMap::new();
        for triangle in 0..old_count {
            let key = canonical_triangle(&before[triangle * 3..triangle * 3 + 3]);
            by_content.entry(key).or_default().push(triangle as u32);
        }
        for queue in by_content.values_mut() {
            queue.reverse();
        }

        let new_count = self.indices.len() / 3;
        let mut new_face = Vec::with_capacity(new_count);
        let mut survived = vec![0u32; polygons.face_count()];
        let mut triangles_of_face = vec![0u32; polygons.face_count()];
        for &face in &polygons.triangle_face {
            if face != NO_FACE {
                triangles_of_face[face as usize] += 1;
            }
        }
        for triangle in 0..new_count {
            let key = canonical_triangle(&self.indices[triangle * 3..triangle * 3 + 3]);
            let Some(old) = by_content.get_mut(&key).and_then(Vec::pop) else {
                self.polygons = None;
                return;
            };
            let face = polygons.triangle_face[old as usize];
            if face != NO_FACE {
                survived[face as usize] += 1;
            }
            new_face.push(face);
        }
        let keep: Vec<bool> = survived
            .iter()
            .zip(&triangles_of_face)
            .map(|(&survived, &total)| survived == total)
            .collect();
        // A triangle of a broken face is face-less from here.
        for entry in &mut new_face {
            if *entry != NO_FACE && !keep[*entry as usize] {
                *entry = NO_FACE;
            }
        }
        polygons.triangle_face = new_face;
        if keep.iter().any(|&kept| !kept) {
            polygons.retain_faces(&keep);
        }
    }

    /// Forget the polygon carry: the triangles no longer correspond to any
    /// source face (a simplify rebuilt them).
    pub fn clear_polygons(&mut self) {
        self.polygons = None;
    }
}

/// `values[representative[slot]]` for every new slot, `fallback` where a slot
/// has no representative.
fn gather<T: Copy>(values: &[T], representative: &[u32], fallback: T) -> Vec<T> {
    representative
        .iter()
        .map(|&old| values.get(old as usize).copied().unwrap_or(fallback))
        .collect()
}

/// A triangle's corners rotated so the smallest index leads — the same triangle
/// whichever corner an operation started it at, but a different winding stays
/// different.
fn canonical_triangle(corners: &[u32]) -> [u32; 3] {
    let (a, b, c) = (corners[0], corners[1], corners[2]);
    if a <= b && a <= c {
        [a, b, c]
    } else if b <= a && b <= c {
        [b, c, a]
    } else {
        [c, a, b]
    }
}

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
        });
    }

    (submeshes, tags)
}

/// The polygon carry of one submesh: the faces its triangles were cut from,
/// with corners in the local numbering, plus the part's edges and layers.
fn build_polygon_carry(
    model: &ModelData,
    part: Option<&review_model::extras::MeshExtras>,
    triangles: &[usize],
    local_of_global: &[u32],
) -> Option<PolygonCarry> {
    let mut carry = PolygonCarry {
        face_offsets: vec![0],
        ..PolygonCarry::default()
    };
    let mut local_face_of: HashMap<u32, u32> = HashMap::new();
    let mut triangle_face = Vec::with_capacity(triangles.len());

    for &triangle in triangles {
        let source_face = model.triangles.to_face[triangle];
        let face = match local_face_of.get(&source_face) {
            Some(&face) => face,
            None => {
                let topology = model.faces.get(source_face as usize)?;
                let first = topology.first_index as usize;
                let end = first.checked_add(topology.index_count as usize)?;
                if topology.index_count < 3 || end > local_of_global.len() {
                    return None;
                }
                let corners = &local_of_global[first..end];
                // A corner the triangle walk did not reach belongs to another
                // submesh's face — a face split across materials, which the
                // source cannot author. Treat it as a drift and write triangles.
                if corners.contains(&u32::MAX) {
                    return None;
                }
                let face = carry.face_offsets.len() as u32 - 1;
                carry.corners.extend_from_slice(corners);
                carry.face_offsets.push(carry.corners.len() as u32);
                carry.source_face.push(source_face);
                if let Some(part) = part {
                    let local = (source_face as usize).wrapping_sub(part.face_first as usize);
                    if let Some(&value) = part.face_smoothing.get(local) {
                        carry.face_smoothing.push(value);
                    }
                    if let Some(&value) = part.face_hole.get(local) {
                        carry.face_hole.push(value);
                    }
                    if let Some(&group) = part.face_group.get(local) {
                        let id = part
                            .face_groups
                            .get(group as usize)
                            .map_or(group, |entry| entry.id as u32);
                        carry.face_group.push(id);
                    }
                }
                local_face_of.insert(source_face, face);
                face
            }
        };
        triangle_face.push(face);
    }
    carry.triangle_face = triangle_face;

    if let Some(part) = part {
        for (index, edge) in part.edges.iter().enumerate() {
            let a = edge[0];
            let b = edge[1];
            if a == u32::MAX || b == u32::MAX {
                continue;
            }
            let (Some(&la), Some(&lb)) = (
                local_of_global.get(a as usize),
                local_of_global.get(b as usize),
            ) else {
                continue;
            };
            // An edge whose corners belong to another submesh is that
            // submesh's.
            if la == u32::MAX || lb == u32::MAX {
                continue;
            }
            carry.edges.push([la, lb]);
            carry.source_edge.push(index as u32);
            if let Some(&value) = part.edge_smoothing.get(index) {
                carry.edge_smoothing.push(value);
            }
            if let Some(&value) = part.edge_crease.get(index) {
                carry.edge_crease.push(value);
            }
            if let Some(&value) = part.edge_visibility.get(index) {
                carry.edge_visibility.push(value);
            }
        }
    }
    Some(carry)
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
        let (submeshes, _tags) = partition(&model, None);

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
        let (submeshes, _) = partition(&model, None);
        let positions = submeshes[0].positions();

        assert_eq!(positions.len(), submeshes[0].vertices.len() * 3);
        assert_eq!(positions[0], submeshes[0].vertices[0].position.x);
        assert_eq!(positions[2], submeshes[0].vertices[0].position.z);
    }

    #[test]
    fn compaction_drops_vertices_nothing_references() {
        let model = demo_cube_model();
        let (mut submeshes, _) = partition(&model, None);
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

    /// A quad cut into two triangles: the carry names the face, and survives a
    /// weld, a reorder and a compaction — but not the loss of one triangle.
    fn quad() -> Submesh {
        let vertex = |x: f32, y: f32| Vertex {
            position: glam::Vec3::new(x, y, 0.0),
            ..Vertex::default()
        };
        Submesh {
            node: 0,
            material: NO_MATERIAL,
            vertices: vec![
                vertex(0.0, 0.0),
                vertex(1.0, 0.0),
                vertex(1.0, 1.0),
                vertex(0.0, 1.0),
                // A duplicate of vertex 2, as a corner-split source would have.
                vertex(1.0, 1.0),
            ],
            uv_channels: Vec::new(),
            color_channels: vec![vec![Vec4::splat(0.5); 5]],
            vertex_crease: vec![0.0, 0.1, 0.2, 0.3, 0.2],
            indices: vec![0, 1, 2, 0, 4, 3],
            skin: VertexRows::default(),
            extra_skins: Vec::new(),
            dq_weight: VertexRows::default(),
            morph: VertexRows::default(),
            source_corner: Vec::new(),
            polygons: Some(PolygonCarry {
                face_offsets: vec![0, 4],
                corners: vec![0, 1, 2, 3],
                source_face: vec![7],
                triangle_face: vec![0, 0],
                face_smoothing: vec![true],
                face_hole: Vec::new(),
                face_group: vec![3],
                edges: vec![[0, 1], [1, 2], [2, 3], [3, 0]],
                source_edge: vec![0, 1, 2, 3],
                edge_smoothing: Vec::new(),
                edge_crease: vec![0.0, 0.5, 0.0, 0.5],
                edge_visibility: Vec::new(),
            }),
        }
    }

    #[test]
    fn a_vertex_remap_renumbers_the_carry_and_gathers_per_vertex_data() {
        let mut piece = quad();
        // Weld vertex 4 onto vertex 2.
        let remap = [0, 1, 2, 3, 2];
        piece.indices = vec![0, 1, 2, 0, 2, 3];
        piece.apply_vertex_remap(&remap, 4);

        assert_eq!(piece.vertices.len(), 4);
        assert_eq!(piece.vertex_crease, vec![0.0, 0.1, 0.2, 0.3]);
        assert_eq!(piece.color_channels[0].len(), 4);
        let polygons = piece.polygons.as_ref().unwrap();
        assert_eq!(polygons.corners, vec![0, 1, 2, 3]);
        assert_eq!(polygons.edges.len(), 4);
        assert_eq!(polygons.edge_crease, vec![0.0, 0.5, 0.0, 0.5]);
    }

    #[test]
    fn a_reorder_keeps_the_face_and_a_dropped_triangle_breaks_it() {
        let mut piece = quad();
        let before = piece.indices.clone();
        // The two triangles swapped, one of them rotated.
        piece.indices = vec![4, 3, 0, 0, 1, 2];
        piece.reconcile_triangles(&before);
        let polygons = piece.polygons.as_ref().unwrap();
        assert_eq!(polygons.face_count(), 1);
        assert_eq!(polygons.triangle_face, vec![0, 0]);

        let before = piece.indices.clone();
        piece.indices.truncate(3);
        piece.reconcile_triangles(&before);
        let polygons = piece.polygons.as_ref().unwrap();
        assert_eq!(
            polygons.face_count(),
            0,
            "a face missing a triangle is gone"
        );
        assert_eq!(polygons.triangle_face, vec![NO_FACE]);
        assert!(polygons.face_smoothing.is_empty() && polygons.face_group.is_empty());
    }

    #[test]
    fn an_invented_triangle_voids_the_carry() {
        let mut piece = quad();
        let before = piece.indices.clone();
        piece.indices = vec![0, 2, 1, 0, 4, 3];
        piece.reconcile_triangles(&before);
        assert!(
            piece.polygons.is_none(),
            "reversed winding is not the same triangle"
        );
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;

    #[test]
    fn row_ids_name_equal_rows_and_survive_a_gather() {
        let mut rows = VertexRows::default();
        rows.push_row(&[(0u32, 1.0f32)]);
        rows.push_row(&[(1, 0.5), (2, 0.5)]);
        rows.push_row(&[(0, 1.0)]);
        let piece = Submesh {
            node: 0,
            material: NO_MATERIAL,
            vertices: vec![Vertex::default(); 3],
            uv_channels: Vec::new(),
            color_channels: Vec::new(),
            vertex_crease: Vec::new(),
            indices: vec![0, 1, 2],
            polygons: None,
            skin: rows.clone(),
            extra_skins: Vec::new(),
            dq_weight: VertexRows::default(),
            morph: VertexRows::default(),
            source_corner: Vec::new(),
        };
        let ids = piece.row_ids();
        assert_eq!(ids[0], ids[2], "identical rows share an id");
        assert_ne!(ids[0], ids[1]);

        let gathered = rows.gather(&[2, 1]);
        assert_eq!(gathered.row(0), &[(0, 1.0)]);
        assert_eq!(gathered.row(1), &[(1, 0.5), (2, 0.5)]);
        assert_eq!(gathered.offsets, vec![0, 1, 3]);
    }
}
