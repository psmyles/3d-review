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
