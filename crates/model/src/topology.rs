//! Which render corners are the same point, and which faces share an edge.
//!
//! Import splits every face corner into its own render vertex, so two faces
//! meeting on an edge reference four different [`ModelData::vertices`] entries.
//! Any question about mesh connectivity — a UV seam, a UV island, a
//! non-manifold edge, a hard edge, an inverted face — has to be asked of the
//! **logical** vertices those corners came from. This module is the one place
//! that answers it, for the renderer's seam and island views and for the audit
//! alike.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::ModelData;

/// One use of one mesh edge by one face, keyed by the pair of logical vertices
/// it connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeUse {
    /// `min(logical) << 32 | max(logical)`, so both winding directions of one
    /// edge sort together and a single pass over the sorted list groups them.
    pub key: u64,
    /// The render corner this use saw the edge's *lower*-numbered logical vertex
    /// at. Ordered to match `key`, so two uses of an edge compare attributes end
    /// for end whichever way each face winds around it.
    pub low: u32,
    /// The render corner this use saw the higher-numbered logical vertex at.
    pub high: u32,
    /// The polygon (indexing [`ModelData::faces`]) that uses the edge — or the
    /// triangle, for a model that carries no face topology.
    pub face: u32,
    /// Whether the face walks the edge from `low` to `high`. Two faces sharing a
    /// manifold edge with consistent winding walk it in opposite directions.
    pub forward: bool,
}

impl EdgeUse {
    /// The two logical vertices the edge connects, lower first.
    pub fn logical_pair(self) -> (u32, u32) {
        ((self.key >> 32) as u32, self.key as u32)
    }
}

/// The identity two render corners meet on: [`ModelData::corner_to_logical`]
/// when import supplied it, otherwise [`welded_logical_ids`].
pub fn logical_ids(model: &ModelData) -> Cow<'_, [u32]> {
    if model.corner_to_logical.len() == model.vertices.len() {
        Cow::Borrowed(&model.corner_to_logical)
    } else {
        Cow::Owned(welded_logical_ids(model))
    }
}

/// A stand-in for [`ModelData::corner_to_logical`] on a mesh that carries none —
/// the Opt workspace's processed levels, whose rebuilt vertex buffer has no
/// source vertices left to map back to. Two corners get the same id when they
/// sit at the **same position on the same object**.
///
/// This is exact rather than tolerant, and that is not a simplification:
/// `optimize` never synthesizes a position — every operation either drops
/// vertices or copies whole ones through a remap — so a processed vertex's
/// position is bit for bit one the source mesh had. Welding within a tolerance
/// instead would risk merging genuinely separate surfaces that pass close by.
///
/// The owning node is part of the key because a weld is otherwise happy to join
/// two objects that touch, which would hide the border each of them really has
/// there. A vertex no triangle references keeps `u32::MAX`.
pub fn welded_logical_ids(model: &ModelData) -> Vec<u32> {
    let triangle_count = model.indices.len() / 3;
    let node_tags =
        (model.triangles.node.len() == triangle_count).then_some(model.triangles.node.as_slice());

    let mut ids = vec![u32::MAX; model.vertices.len()];
    let mut welded: HashMap<(u32, [u32; 3]), u32> = HashMap::new();
    for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
        let node = node_tags.map_or(0, |nodes| nodes[triangle_index]);
        for &corner in triangle {
            let corner = corner as usize;
            // Skips an out-of-range index and an already-assigned corner in one
            // test.
            if ids.get(corner).copied() != Some(u32::MAX) {
                continue;
            }
            let Some(vertex) = model.vertices.get(corner) else {
                continue;
            };
            let key = (
                node,
                [
                    position_bits(vertex.position.x),
                    position_bits(vertex.position.y),
                    position_bits(vertex.position.z),
                ],
            );
            let next = welded.len() as u32;
            ids[corner] = *welded.entry(key).or_insert(next);
        }
    }
    ids
}

/// One position component as a hash key. `-0.0` folds onto `0.0` and every NaN
/// onto one canonical NaN, because the comparison is bitwise: without that, two
/// vertices at the same visible point could miss each other over a sign bit.
fn position_bits(value: f32) -> u32 {
    if value == 0.0 {
        0.0f32.to_bits()
    } else if value.is_nan() {
        f32::NAN.to_bits()
    } else {
        value.to_bits()
    }
}

impl ModelData {
    /// A per-face owning scene-graph node index, parallel to
    /// [`ModelData::faces`]: each triangle stamps its node onto its face. `None`
    /// when the model carries no per-triangle node/face info, so a face's owner
    /// can't be resolved.
    pub fn face_nodes(&self) -> Option<Vec<u32>> {
        let triangle_count = self.indices.len() / 3;
        if triangle_count == 0
            || self.triangles.node.len() != triangle_count
            || self.triangles.to_face.len() != triangle_count
            || self.faces.is_empty()
        {
            return None;
        }
        let mut map = vec![u32::MAX; self.faces.len()];
        for triangle in 0..triangle_count {
            let face = self.triangles.to_face[triangle] as usize;
            if let Some(slot) = map.get_mut(face) {
                *slot = self.triangles.node[triangle];
            }
        }
        Some(map)
    }
}

/// Every edge of every face `skip_node` keeps, one [`EdgeUse`] per (face, edge)
/// pair, with `logical` giving each render corner the identity two faces meet
/// on.
///
/// Walks the original polygons wherever the model carries face topology — so a
/// quad's four real edges are considered and never its triangulation diagonal —
/// and falls back to triangle edges only for a mesh that arrived without it.
/// `skip_node` is asked about each face's owning node (`u32::MAX` when the owner
/// can't be resolved); returning `true` leaves the face out.
///
/// An edge whose ends share a logical vertex is degenerate and recorded nowhere.
/// The list is in walk order; sort it by `key` (see [`group_edge_uses`]) to
/// gather the uses of each edge.
pub fn edge_uses(
    model: &ModelData,
    logical: &[u32],
    skip_node: impl Fn(u32) -> bool,
) -> Vec<EdgeUse> {
    let mut edges = Vec::with_capacity(model.indices.len());

    if model.faces.is_empty() {
        let triangle_count = model.indices.len() / 3;
        let tags = (model.triangles.node.len() == triangle_count).then_some(&model.triangles.node);
        for (triangle_index, triangle) in model.indices.as_chunks::<3>().0.iter().enumerate() {
            let node = tags.map_or(u32::MAX, |tags| tags[triangle_index]);
            if skip_node(node) {
                continue;
            }
            for corner in 0..3 {
                push_edge_use(
                    &mut edges,
                    logical,
                    triangle[corner] as usize,
                    triangle[(corner + 1) % 3] as usize,
                    triangle_index as u32,
                );
            }
        }
        return edges;
    }

    let face_node = model.face_nodes();
    for (face_index, face) in model.faces.iter().enumerate() {
        let node = face_node
            .as_ref()
            .and_then(|nodes| nodes.get(face_index).copied())
            .unwrap_or(u32::MAX);
        if skip_node(node) {
            continue;
        }
        let count = face.index_count as usize;
        if count < 2 {
            continue;
        }
        let first = face.first_index as usize;
        for corner in 0..count {
            push_edge_use(
                &mut edges,
                logical,
                first + corner,
                first + (corner + 1) % count,
                face_index as u32,
            );
        }
    }

    edges
}

/// Sort `edges` by key and yield each edge's uses as one slice — length 1 for an
/// open border, 2 for a manifold edge, more for a non-manifold one.
pub fn group_edge_uses(edges: &mut [EdgeUse]) -> impl Iterator<Item = &[EdgeUse]> {
    edges.sort_unstable_by_key(|edge| (edge.key, edge.face));
    edges.chunk_by(|a, b| a.key == b.key)
}

/// Record one use of the edge from render corner `a` to `b`, on the canonical
/// (lower logical vertex first) form.
fn push_edge_use(edges: &mut Vec<EdgeUse>, logical: &[u32], a: usize, b: usize, face: u32) {
    let (Some(&logical_a), Some(&logical_b)) = (logical.get(a), logical.get(b)) else {
        return;
    };
    if logical_a == logical_b || logical_a == u32::MAX || logical_b == u32::MAX {
        return;
    }
    let use_ = if logical_a < logical_b {
        EdgeUse {
            key: (u64::from(logical_a) << 32) | u64::from(logical_b),
            low: a as u32,
            high: b as u32,
            face,
            forward: true,
        }
    } else {
        EdgeUse {
            key: (u64::from(logical_b) << 32) | u64::from(logical_a),
            low: b as u32,
            high: a as u32,
            face,
            forward: false,
        }
    };
    edges.push(use_);
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::{TopologyFace, TriangleData, Vertex};

    /// Two quads sharing an edge plus a third face hanging off the same edge — a
    /// fin, the textbook non-manifold case.
    fn fin_model() -> ModelData {
        // Logical vertices: 0..=5. Face A = 0,1,2,3; face B = 1,4,5,2 (shares
        // edge 1-2 with A, walked the other way); face C = 1,2,? — a triangle
        // 2,1,0' reusing the shared edge a third time.
        let logical = [0u32, 1, 2, 3, 1, 4, 5, 2, 2, 1, 0];
        let vertices = logical
            .iter()
            .map(|&id| Vertex {
                position: Vec3::new(id as f32, 0.0, 0.0),
                ..Default::default()
            })
            .collect();
        ModelData {
            vertices,
            corner_to_logical: logical.to_vec(),
            faces: vec![
                TopologyFace {
                    first_index: 0,
                    index_count: 4,
                },
                TopologyFace {
                    first_index: 4,
                    index_count: 4,
                },
                TopologyFace {
                    first_index: 8,
                    index_count: 3,
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10],
            triangles: TriangleData {
                to_face: vec![0, 0, 1, 1, 2],
                material: vec![0; 5],
                node: vec![0, 0, 0, 0, 1],
            },
            ..Default::default()
        }
    }

    #[test]
    fn groups_count_faces_per_edge() {
        let model = fin_model();
        let logical = logical_ids(&model);
        let mut edges = edge_uses(&model, &logical, |_| false);
        let shared: Vec<usize> = group_edge_uses(&mut edges)
            .filter(|group| group[0].logical_pair() == (1, 2))
            .map(<[EdgeUse]>::len)
            .collect();
        assert_eq!(shared, vec![3], "edge 1-2 is used by all three faces");
    }

    #[test]
    fn direction_records_how_each_face_walks_the_edge() {
        let model = fin_model();
        let logical = logical_ids(&model);
        let mut edges = edge_uses(&model, &logical, |_| false);
        let group = group_edge_uses(&mut edges)
            .find(|group| group[0].logical_pair() == (1, 2))
            .unwrap()
            .to_vec();
        // Face A walks 1→2 (forward), face B walks 2→1, face C walks 2→1.
        let directions: Vec<(u32, bool)> = group.iter().map(|u| (u.face, u.forward)).collect();
        assert_eq!(directions, vec![(0, true), (1, false), (2, false)]);
    }

    #[test]
    fn skipped_nodes_contribute_nothing() {
        let model = fin_model();
        let logical = logical_ids(&model);
        let mut edges = edge_uses(&model, &logical, |node| node == 1);
        let shared = group_edge_uses(&mut edges)
            .find(|group| group[0].logical_pair() == (1, 2))
            .map(<[EdgeUse]>::len);
        assert_eq!(shared, Some(2));
    }

    #[test]
    fn face_nodes_projects_triangle_tags() {
        assert_eq!(fin_model().face_nodes(), Some(vec![0, 0, 1]));
    }
}
