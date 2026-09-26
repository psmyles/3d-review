//! A [`Submesh`]: the per-(node, material) piece that can survive a simplify.

use std::collections::HashMap;

use glam::{Vec2, Vec4};
use review_model::Vertex;

use crate::meshopt::POSITION_COMPONENTS;

use super::*;

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
    /// [`ModelData::uv_channels`](review_model::ModelData::uv_channels)'s own
    /// convention).
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
    /// Whether the normals no longer describe the surface: a weld that ignored
    /// normals merged vertices that disagreed, and each survivor kept one of
    /// them arbitrarily. Cleared by whatever writes normals next — the stack's
    /// finishing pass smooths them if nothing else did.
    pub normals_stale: bool,
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
    /// Several old vertices may map to one new slot (a weld). Every slot takes
    /// the vertex *and* all of its carried per-vertex data from one old vertex,
    /// the first that maps there - so after a tolerance weld the vertex's own UV
    /// and the first UV channel still agree, which a last-writer-wins vertex
    /// (meshoptimizer's own remap) beside first-writer channels did not.
    /// Polygon corners and edges are renumbered through the same remap.
    pub fn apply_vertex_remap(&mut self, remap: &[u32], unique: usize) {
        debug_assert_eq!(remap.len(), self.vertices.len());

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
        self.vertices = gather(&self.vertices, &representative, Vertex::default());
        self.gather_rows(&representative);

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

    /// Rebuild every per-vertex array *other than* `vertices` so slot `n` holds
    /// what old vertex `representative[n]` carried ([`u32::MAX`] leaves the
    /// fallback). The half of a remap that has to follow a vertex through the
    /// UV and color channels, creases, source corners and every deform row —
    /// shared by a merge ([`Self::apply_vertex_remap`]) and a split
    /// ([`Self::split_by_corner`]), so a new per-vertex array is added once.
    fn gather_rows(&mut self, representative: &[u32]) {
        for channel in &mut self.uv_channels {
            *channel = gather(channel, representative, Vec2::ZERO);
        }
        for channel in &mut self.color_channels {
            *channel = gather(channel, representative, Vec4::ONE);
        }
        if !self.vertex_crease.is_empty() {
            self.vertex_crease = gather(&self.vertex_crease, representative, 0.0);
        }
        if !self.source_corner.is_empty() {
            self.source_corner = gather(&self.source_corner, representative, u32::MAX);
        }
        self.skin = self.skin.gather(representative);
        for layer in &mut self.extra_skins {
            *layer = layer.gather(representative);
        }
        self.dq_weight = self.dq_weight.gather(representative);
        self.morph = self.morph.gather(representative);
    }

    /// Give every triangle corner the value `corner[i]` (one per index), copying
    /// a vertex wherever its corners disagree. Returns the value each vertex
    /// ended up with — `None` for one no triangle references.
    ///
    /// This is how a *per-corner* result (meshoptimizer's generated normals and
    /// tangents) becomes per-vertex data: corners that agree keep sharing their
    /// vertex, and each disagreement becomes a copy appended after the existing
    /// vertices, carrying everything the original did — channels, creases and
    /// deform rows exactly — so skinning is unaffected. Corners are visited in
    /// index order, which makes the numbering deterministic.
    ///
    /// A carried polygon never needs two copies of one vertex, so the corners of
    /// one face at one vertex are first made to agree (the face's first corner
    /// there wins); then the carry's corners are renumbered per face and each
    /// carried edge is repeated for every distinct pair of copies its faces now
    /// use, so the export can still name each of them.
    pub(crate) fn split_by_corner<T: Copy + PartialEq>(&mut self, corner: &[T]) -> Vec<Option<T>> {
        debug_assert_eq!(corner.len(), self.indices.len());
        let mut values: Vec<T> = corner.to_vec();

        let triangle_face = self
            .polygons
            .as_ref()
            .filter(|carry| carry.triangle_face.len() == self.indices.len() / 3)
            .map(|carry| carry.triangle_face.clone());
        if let Some(triangle_face) = &triangle_face {
            let mut first: HashMap<(u32, u32), T> = HashMap::new();
            for (index, value) in values.iter_mut().enumerate() {
                let face = triangle_face[index / 3];
                if face == NO_FACE {
                    continue;
                }
                let key = (face, self.indices[index]);
                *value = *first.entry(key).or_insert(*value);
            }
        }

        let original = self.vertices.len();
        let before = self.indices.clone();
        let mut assigned: Vec<Option<T>> = vec![None; original];
        let mut next_copy: Vec<u32> = vec![u32::MAX; original];
        let mut representative: Vec<u32> = (0..original as u32).collect();
        for (index, &value) in self.indices.iter_mut().zip(&values) {
            let mut current = *index as usize;
            loop {
                match assigned[current] {
                    None => {
                        assigned[current] = Some(value);
                        break;
                    }
                    Some(existing) if existing == value => break,
                    Some(_) if next_copy[current] != u32::MAX => {
                        current = next_copy[current] as usize;
                    }
                    Some(_) => {
                        let copy = assigned.len();
                        assigned.push(Some(value));
                        next_copy.push(u32::MAX);
                        representative.push(*index);
                        next_copy[current] = copy as u32;
                        current = copy;
                        break;
                    }
                }
            }
            *index = current as u32;
        }

        if representative.len() > original {
            self.vertices = gather(&self.vertices, &representative, Vertex::default());
            self.gather_rows(&representative);
            if let (Some(polygons), Some(_)) = (&mut self.polygons, &triangle_face) {
                polygons.split_corners(&before, &self.indices);
            }
        }
        assigned
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
pub(super) fn gather<T: Copy>(values: &[T], representative: &[u32], fallback: T) -> Vec<T> {
    representative
        .iter()
        .map(|&old| values.get(old as usize).copied().unwrap_or(fallback))
        .collect()
}

/// A triangle's corners rotated so the smallest index leads — the same triangle
/// whichever corner an operation started it at, but a different winding stays
/// different.
pub(super) fn canonical_triangle(corners: &[u32]) -> [u32; 3] {
    let (a, b, c) = (corners[0], corners[1], corners[2]);
    if a <= b && a <= c {
        [a, b, c]
    } else if b <= a && b <= c {
        [b, c, a]
    } else {
        [c, a, b]
    }
}
