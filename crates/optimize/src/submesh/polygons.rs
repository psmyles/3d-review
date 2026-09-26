//! The source's faces and edges, carried over local vertices.
//!
//! An operation that keeps triangles whole reconciles this by canonical triangle
//! content; a simplify clears it, and the export writes that level as triangles
//! with a note naming the operation.

use std::collections::HashMap;

use review_model::ModelData;

use super::*;

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
    /// Whether an operation *built* these faces rather than carrying the
    /// source's through.
    ///
    /// The distinction matters because a rebuilt carry is the only one the
    /// renderer can be shown: the source's faces describe corner runs in the
    /// *source* vertex array, a layout welding destroys, so a processed level
    /// normally goes out as pure triangles with no `faces` table at all. A
    /// rebuilt one describes the level's own geometry, so `process::assemble`
    /// expands the whole level into the corner-run layout and the quads reach
    /// the viewport, the stats card and the export as quads.
    ///
    /// `false` for every carry `build_polygon_carry` produces; set by
    /// [`crate::remesh`] on the pieces it rebuilds. It survives a vertex remap
    /// and a triangle reconciliation with the rest of the carry, so a Weld or a
    /// Vertex Fetch below a Remesh keeps the polygons.
    pub rebuilt: bool,
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
    pub(super) fn retain_faces(&mut self, keep: &[bool]) {
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

    /// Follow a vertex *split*: the index buffer went from `before` to `after`
    /// with only the vertex numbers changing, and every corner of one face at
    /// one old vertex moved to the same new one. Renumbers each face's corners
    /// through the triangles that face owns, and repeats each carried edge (with
    /// its layers) once per distinct pair of new vertices the faces around it now
    /// use — an edge whose two sides were split apart becomes two edges, as it
    /// would be if the source had been authored that way.
    pub(super) fn split_corners(&mut self, before: &[u32], after: &[u32]) {
        let mut new_of: HashMap<(u32, u32), u32> = HashMap::new();
        for (index, (&old, &new)) in before.iter().zip(after).enumerate() {
            if let Some(&face) = self.triangle_face.get(index / 3)
                && face != NO_FACE
            {
                new_of.entry((face, old)).or_insert(new);
            }
        }

        // Every (old pair → new pair) a face's boundary now uses, keyed by the
        // unordered old pair so an edge finds it whichever way it is stored.
        let mut uses: HashMap<(u32, u32), Vec<[u32; 2]>> = HashMap::new();
        for face in 0..self.face_count() {
            let first = self.face_offsets[face] as usize;
            let end = self.face_offsets[face + 1] as usize;
            let face_id = face as u32;
            let renumber = |old: u32| new_of.get(&(face_id, old)).copied().unwrap_or(old);
            let count = end - first;
            for k in 0..count {
                let a = self.corners[first + k];
                let b = self.corners[first + (k + 1) % count];
                let key = (a.min(b), a.max(b));
                let pair = if a <= b {
                    [renumber(a), renumber(b)]
                } else {
                    [renumber(b), renumber(a)]
                };
                let entry = uses.entry(key).or_default();
                if !entry.contains(&pair) {
                    entry.push(pair);
                }
            }
            for corner in &mut self.corners[first..end] {
                *corner = renumber(*corner);
            }
        }

        let old_edges = std::mem::take(&mut self.edges);
        let source_edge = std::mem::take(&mut self.source_edge);
        let smoothing = std::mem::take(&mut self.edge_smoothing);
        let crease = std::mem::take(&mut self.edge_crease);
        let visibility = std::mem::take(&mut self.edge_visibility);
        for (index, edge) in old_edges.iter().enumerate() {
            let key = (edge[0].min(edge[1]), edge[0].max(edge[1]));
            let reversed = edge[0] > edge[1];
            let pairs = uses.get(&key).map_or(&[][..], Vec::as_slice);
            // An edge no face bounds keeps its old numbering; the export already
            // drops an edge it cannot name.
            let fallback = [[key.0, key.1]];
            let pairs = if pairs.is_empty() {
                &fallback[..]
            } else {
                pairs
            };
            for pair in pairs {
                self.edges
                    .push(if reversed { [pair[1], pair[0]] } else { *pair });
                if let Some(&value) = source_edge.get(index) {
                    self.source_edge.push(value);
                }
                if let Some(&value) = smoothing.get(index) {
                    self.edge_smoothing.push(value);
                }
                if let Some(&value) = crease.get(index) {
                    self.edge_crease.push(value);
                }
                if let Some(&value) = visibility.get(index) {
                    self.edge_visibility.push(value);
                }
            }
        }
    }

    /// Keep only the edges `keep` marks.
    pub(super) fn retain_edges(&mut self, keep: &[bool]) {
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

/// The polygon carry of one submesh: the faces its triangles were cut from,
/// with corners in the local numbering, plus the part's edges and layers.
pub(super) fn build_polygon_carry(
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
