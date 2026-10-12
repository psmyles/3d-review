//! UV islands: the connected pieces of a UV layout.
//!
//! An island is a set of faces joined by edges whose UVs are continuous across
//! them. Joining by **3D adjacency** rather than by welding UV coordinates is
//! what keeps mirrored and overlapping islands — common in game characters —
//! apart: two faces stacked on the same UVs but on opposite sides of a model
//! share no edge, so they are two islands.

use crate::ModelData;
use crate::topology::{EdgeUse, edge_uses, group_edge_uses, logical_ids};

/// The UV grid two corners are compared on. A non-seam edge's two sides are one
/// source UV element copied into two corners, so they normally agree bit for
/// bit; this only absorbs the last-place noise of a file that stored them
/// separately.
const UV_STEP: f32 = 1.0e-5;

/// Which island each face belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UvIslands {
    /// Per element (a face of [`ModelData::faces`], or a triangle when
    /// `by_triangle`), its island id in `0..count`, numbered in first-seen
    /// element order so ids are stable for a given mesh.
    pub island: Vec<u32>,
    pub count: u32,
    /// True when the model carried no face topology and the elements are
    /// triangles.
    pub by_triangle: bool,
}

impl UvIslands {
    /// The island of triangle `triangle`, resolving through the face map when
    /// the islands are per face.
    pub fn of_triangle(&self, model: &ModelData, triangle: usize) -> Option<u32> {
        if self.by_triangle {
            self.island.get(triangle).copied()
        } else {
            let face = *model.triangles.to_face.get(triangle)?;
            self.island.get(face as usize).copied()
        }
    }
}

/// Find the islands of UV set `channel`. Faces owned by a node `skip_node`
/// accepts are left out of the joining, though they still get an island id
/// (their own).
pub fn uv_islands(model: &ModelData, channel: usize, skip_node: impl Fn(u32) -> bool) -> UvIslands {
    let by_triangle = model.faces.is_empty();
    let element_count = if by_triangle {
        model.indices.len() / 3
    } else {
        model.faces.len()
    };
    if element_count == 0 {
        return UvIslands::default();
    }

    let logical = logical_ids(model);
    let mut edges = edge_uses(model, &logical, skip_node);
    let quantize = |corner: u32| {
        let uv = model.uv_for_channel(corner as usize, channel);
        [
            (uv.x / UV_STEP).round() as i64,
            (uv.y / UV_STEP).round() as i64,
        ]
    };
    let continuous = |a: &EdgeUse, b: &EdgeUse| {
        quantize(a.low) == quantize(b.low) && quantize(a.high) == quantize(b.high)
    };

    let mut parent: Vec<u32> = (0..element_count as u32).collect();
    for group in group_edge_uses(&mut edges) {
        // Manifold edges hold two uses, so the pairwise scan is effectively
        // constant.
        for (i, a) in group.iter().enumerate() {
            for b in &group[i + 1..] {
                if continuous(a, b) {
                    union(&mut parent, a.face, b.face);
                }
            }
        }
    }

    let mut root_island = vec![u32::MAX; element_count];
    let mut island = vec![0; element_count];
    let mut count = 0;
    for element in 0..element_count as u32 {
        let root = find(&mut parent, element) as usize;
        if root_island[root] == u32::MAX {
            root_island[root] = count;
            count += 1;
        }
        island[element as usize] = root_island[root];
    }
    UvIslands {
        island,
        count,
        by_triangle,
    }
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        let next = parent[parent[x as usize] as usize];
        parent[x as usize] = next;
        x = next;
    }
    x
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        // Lower root wins so the result does not depend on edge order.
        let (keep, merge) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[merge as usize] = keep;
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};

    use super::*;
    use crate::{TopologyFace, TriangleData, Vertex};

    /// Two triangles sharing logical edge (1,2). `seam` moves one side's UV so
    /// the edge is a UV cut.
    fn pair(seam: bool) -> ModelData {
        let logical = [0u32, 1, 2, 1, 3, 2];
        let uvs = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            if seam {
                Vec2::new(5.0, 0.0)
            } else {
                Vec2::new(1.0, 0.0)
            },
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        ModelData {
            vertices: (0..6)
                .map(|i| Vertex {
                    position: Vec3::new(logical[i] as f32, 0.0, 0.0),
                    uv: uvs[i],
                    ..Default::default()
                })
                .collect(),
            corner_to_logical: logical.to_vec(),
            indices: (0..6).collect(),
            faces: vec![
                TopologyFace {
                    first_index: 0,
                    index_count: 3,
                },
                TopologyFace {
                    first_index: 3,
                    index_count: 3,
                },
            ],
            triangles: TriangleData {
                to_face: vec![0, 1],
                material: vec![0, 0],
                node: vec![0, 0],
            },
            ..Default::default()
        }
    }

    #[test]
    fn continuous_faces_form_one_island() {
        let islands = uv_islands(&pair(false), 0, |_| false);
        assert_eq!(islands.count, 1);
        assert_eq!(islands.island, vec![0, 0]);
    }

    #[test]
    fn a_seam_splits_the_islands() {
        let islands = uv_islands(&pair(true), 0, |_| false);
        assert_eq!(islands.count, 2);
        assert_eq!(islands.island, vec![0, 1]);
    }
}
